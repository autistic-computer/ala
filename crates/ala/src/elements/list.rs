use crate::collections::VecDeque;
use crate::refineable::Refineable as _;
use crate::sum_tree::{Bias, Dimensions, SumTree};
use crate::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, DispatchPhase, Edges, Element, EntityId,
    FocusHandle, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    Overflow, Overscroll, Pixels, Point, RubberBand, ScrollDelta, ScrollKey, ScrollWheelEvent,
    Size, Style, StyleRefinement, Styled, TouchPhase, Window, point, px, size,
};
use std::{cell::RefCell, ops::Range, rc::Rc};

type RenderItemFn = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static;

pub fn list(
    state: ListState,
    render_item: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> List {
    List {
        state,
        render_item: Box::new(render_item),
        style: StyleRefinement::default(),
        sizing_behavior: ListSizingBehavior::default(),
    }
}

pub struct List {
    state: ListState,
    render_item: Box<RenderItemFn>,
    style: StyleRefinement,
    sizing_behavior: ListSizingBehavior,
}

impl List {
    pub fn with_sizing_behavior(mut self, behavior: ListSizingBehavior) -> Self {
        self.sizing_behavior = behavior;
        self
    }
}

#[derive(Clone)]
pub struct ListState(Rc<RefCell<StateInner>>);

impl std::fmt::Debug for ListState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ListState")
    }
}

struct StateInner {
    last_layout_bounds: Option<Bounds<Pixels>>,
    last_padding: Option<Edges<Pixels>>,
    items: SumTree<ListItem>,
    logical_scroll_top: Option<ListOffset>,
    alignment: ListAlignment,
    overdraw: Pixels,
    reset: bool,
    #[allow(clippy::type_complexity)]
    scroll_handler: Option<Box<dyn FnMut(&ListScrollEvent, &mut Window, &mut App)>>,
    scrollbar_drag_start_height: Option<Pixels>,
    measuring_behavior: ListMeasuringBehavior,
    pending_scroll: Option<PendingScroll>,
    follow_state: FollowState,
    overscroll: RubberBand,
}

#[derive(Clone)]
enum PendingScroll {
    Absolute { item_ix: usize, offset: Pixels },
    Proportional(PendingScrollFraction),
}

#[derive(Clone)]
struct PendingScrollFraction {
    item_ix: usize,
    fraction: f32,
}

enum ScrollAnchor {
    Absolute,
    Proportional,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FollowMode {
    #[default]
    Normal,
    Tail,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FollowState {
    #[default]
    Normal,
    Tail {
        is_following: bool,
    },
}

impl FollowState {
    fn is_following(&self) -> bool {
        matches!(self, FollowState::Tail { is_following: true })
    }

    fn has_stopped_following(&self) -> bool {
        matches!(
            self,
            FollowState::Tail {
                is_following: false
            }
        )
    }

    fn start_following(&mut self) {
        if let FollowState::Tail {
            is_following: false,
        } = self
        {
            *self = FollowState::Tail { is_following: true };
        }
    }

    fn stop_following(&mut self) {
        if let FollowState::Tail { is_following: true } = self {
            *self = FollowState::Tail {
                is_following: false,
            };
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListAlignment {
    Top,
    Bottom,
}

pub struct ListScrollEvent {
    pub visible_range: Range<usize>,

    pub count: usize,

    pub is_scrolled: bool,

    pub is_following_tail: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListSizingBehavior {
    Infer,
    #[default]
    Auto,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListMeasuringBehavior {
    Measure(bool),
    #[default]
    Visible,
}

impl ListMeasuringBehavior {
    fn reset(&mut self) {
        match self {
            ListMeasuringBehavior::Measure(has_measured) => *has_measured = false,
            ListMeasuringBehavior::Visible => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListHorizontalSizingBehavior {
    #[default]
    FitList,
    Unconstrained,
}

struct LayoutItemsResponse {
    max_item_width: Pixels,
    scroll_top: ListOffset,
    item_layouts: VecDeque<ItemLayout>,
}

struct ItemLayout {
    index: usize,
    element: AnyElement,
    size: Size<Pixels>,
}

pub struct ListPrepaintState {
    hitbox: Hitbox,
    layout: LayoutItemsResponse,
}

#[derive(Clone)]
enum ListItem {
    Unmeasured {
        size_hint: Option<Size<Pixels>>,
        focus_handle: Option<FocusHandle>,
    },
    Measured {
        size: Size<Pixels>,
        focus_handle: Option<FocusHandle>,
    },
}

impl ListItem {
    fn size(&self) -> Option<Size<Pixels>> {
        if let ListItem::Measured { size, .. } = self {
            Some(*size)
        } else {
            None
        }
    }

    fn size_hint(&self) -> Option<Size<Pixels>> {
        match self {
            ListItem::Measured { size, .. } => Some(*size),
            ListItem::Unmeasured { size_hint, .. } => *size_hint,
        }
    }

    fn focus_handle(&self) -> Option<FocusHandle> {
        match self {
            ListItem::Unmeasured { focus_handle, .. } | ListItem::Measured { focus_handle, .. } => {
                focus_handle.clone()
            }
        }
    }

    fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        match self {
            ListItem::Unmeasured { focus_handle, .. } | ListItem::Measured { focus_handle, .. } => {
                focus_handle
                    .as_ref()
                    .is_some_and(|handle| handle.contains_focused(window, cx))
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ListItemSummary {
    count: usize,
    rendered_count: usize,
    unrendered_count: usize,
    height: Pixels,
    has_focus_handles: bool,
    has_unknown_height: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Count(usize);

#[derive(Clone, Debug, Default)]
struct Height(Pixels);

impl ListState {
    pub fn new(item_count: usize, alignment: ListAlignment, overdraw: Pixels) -> Self {
        let this = Self(Rc::new(RefCell::new(StateInner {
            last_layout_bounds: None,
            last_padding: None,
            items: SumTree::default(),
            logical_scroll_top: None,
            alignment,
            overdraw,
            scroll_handler: None,
            reset: false,
            scrollbar_drag_start_height: None,
            measuring_behavior: ListMeasuringBehavior::default(),
            pending_scroll: None,
            follow_state: FollowState::default(),
            overscroll: RubberBand::default(),
        })));
        this.splice(0..0, item_count);
        this
    }

    pub fn measure_all(self) -> Self {
        self.0.borrow_mut().measuring_behavior = ListMeasuringBehavior::Measure(false);
        self
    }

    pub fn with_uniform_item_height(self, height: Pixels) -> Self {
        self.apply_uniform_item_height(height);
        self
    }

    pub fn reset(&self, element_count: usize) {
        let old_count = {
            let state = &mut *self.0.borrow_mut();
            state.reset = true;
            state.measuring_behavior.reset();
            state.logical_scroll_top = None;
            state.pending_scroll = None;
            state.scrollbar_drag_start_height = None;
            state.items.summary().count
        };

        self.splice(0..old_count, element_count);
    }

    pub fn reset_with_uniform_height(&self, element_count: usize, height: Pixels) {
        self.reset(element_count);
        self.apply_uniform_item_height(height);
    }

    fn apply_uniform_item_height(&self, height: Pixels) {
        let size_hint = Size {
            width: px(0.),
            height,
        };
        let mut state = self.0.borrow_mut();
        let new_items = state
            .items
            .iter()
            .map(|item| ListItem::Unmeasured {
                size_hint: Some(item.size_hint().unwrap_or(size_hint)),
                focus_handle: item.focus_handle(),
            })
            .collect::<Vec<_>>();
        let mut tree = SumTree::default();
        tree.extend(new_items, ());
        state.items = tree;
    }

    pub fn remeasure(&self) {
        let count = self.item_count();
        self.remeasure_items_with_scroll_anchor(0..count, ScrollAnchor::Proportional);
    }

    pub fn remeasure_items(&self, range: Range<usize>) {
        self.remeasure_items_with_scroll_anchor(range, ScrollAnchor::Absolute);
    }

    fn remeasure_items_with_scroll_anchor(&self, range: Range<usize>, scroll_anchor: ScrollAnchor) {
        let state = &mut *self.0.borrow_mut();

        if let Some(scroll_top) = state.logical_scroll_top {
            if range.contains(&scroll_top.item_ix) {
                state.pending_scroll = match scroll_anchor {
                    ScrollAnchor::Absolute => Some(PendingScroll::Absolute {
                        item_ix: scroll_top.item_ix,
                        offset: scroll_top.offset_in_item,
                    }),
                    ScrollAnchor::Proportional => {
                        let mut cursor = state.items.cursor::<Count>(());
                        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

                        cursor
                            .item()
                            .and_then(|item| {
                                item.size().map(|size| {
                                    let fraction = if size.height.0 > 0.0 {
                                        (scroll_top.offset_in_item.0 / size.height.0)
                                            .clamp(0.0, 1.0)
                                    } else {
                                        0.0
                                    };

                                    PendingScroll::Proportional(PendingScrollFraction {
                                        item_ix: scroll_top.item_ix,
                                        fraction,
                                    })
                                })
                            })
                            .or_else(|| state.pending_scroll.clone())
                    }
                };
            }
        }

        let new_items = {
            let mut cursor = state.items.cursor::<Count>(());
            let mut new_items = cursor.slice(&Count(range.start), Bias::Right);
            let invalidated = cursor.slice(&Count(range.end), Bias::Right);
            new_items.extend(
                invalidated.iter().map(|item| ListItem::Unmeasured {
                    size_hint: item.size_hint(),
                    focus_handle: item.focus_handle(),
                }),
                (),
            );
            new_items.append(cursor.suffix(), ());
            new_items
        };
        state.items = new_items;
        state.measuring_behavior.reset();
    }

    pub fn item_count(&self) -> usize {
        self.0.borrow().items.summary().count
    }

    pub fn is_scrolled_to_end(&self) -> Option<bool> {
        let state = self.0.borrow();
        let bounds = state.last_layout_bounds?;
        let summary = state.items.summary();
        if summary.has_unknown_height {
            return None;
        }
        let padding = state.last_padding.unwrap_or_default();
        let content_height = summary.height + padding.top + padding.bottom;
        let scroll_max = (content_height - bounds.size.height).max(px(0.));
        if scroll_max <= px(0.) {
            return None;
        }
        let scroll_top = state.scroll_top(&state.logical_scroll_top());
        Some(scroll_top >= scroll_max)
    }

    pub fn splice(&self, old_range: Range<usize>, count: usize) {
        self.splice_focusable(old_range, (0..count).map(|_| None))
    }

    pub fn splice_focusable(
        &self,
        old_range: Range<usize>,
        focus_handles: impl IntoIterator<Item = Option<FocusHandle>>,
    ) {
        let state = &mut *self.0.borrow_mut();

        let mut old_items = state.items.cursor::<Count>(());
        let mut new_items = old_items.slice(&Count(old_range.start), Bias::Right);
        old_items.seek_forward(&Count(old_range.end), Bias::Right);

        let mut spliced_count = 0;
        new_items.extend(
            focus_handles.into_iter().map(|focus_handle| {
                spliced_count += 1;
                ListItem::Unmeasured {
                    size_hint: None,
                    focus_handle,
                }
            }),
            (),
        );
        new_items.append(old_items.suffix(), ());
        drop(old_items);
        state.items = new_items;

        if let Some(ListOffset {
            item_ix,
            offset_in_item,
        }) = state.logical_scroll_top.as_mut()
        {
            if old_range.contains(item_ix) {
                *item_ix = old_range.start;
                *offset_in_item = px(0.);
            } else if old_range.end <= *item_ix {
                *item_ix = *item_ix - (old_range.end - old_range.start) + spliced_count;
            }
        }
    }

    pub fn set_scroll_handler(
        &self,
        handler: impl FnMut(&ListScrollEvent, &mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().scroll_handler = Some(Box::new(handler))
    }

    pub fn logical_scroll_top(&self) -> ListOffset {
        self.0.borrow().logical_scroll_top()
    }

    pub fn scroll_by(&self, distance: Pixels) {
        if distance == px(0.) {
            return;
        }

        let current_offset = self.logical_scroll_top();
        let state = &mut *self.0.borrow_mut();

        if distance < px(0.) {
            state.follow_state.stop_following();
        }

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(current_offset.item_ix), Bias::Right);

        let start_pixel_offset = cursor.start().height + current_offset.offset_in_item;
        let new_pixel_offset = (start_pixel_offset + distance).max(px(0.));
        if new_pixel_offset > start_pixel_offset {
            cursor.seek_forward(&Height(new_pixel_offset), Bias::Right);
        } else {
            cursor.seek(&Height(new_pixel_offset), Bias::Right);
        }

        let scroll_top = ListOffset {
            item_ix: cursor.start().count,
            offset_in_item: new_pixel_offset - cursor.start().height,
        };
        drop(cursor);
        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    pub fn scroll_to_end(&self) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        state.pending_scroll = None;
        state.logical_scroll_top = Some(ListOffset {
            item_ix: item_count,
            offset_in_item: px(0.),
        });
    }

    pub fn set_follow_mode(&self, mode: FollowMode) {
        let state = &mut *self.0.borrow_mut();

        match mode {
            FollowMode::Normal => {
                state.follow_state = FollowState::Normal;
            }
            FollowMode::Tail => {
                state.follow_state = FollowState::Tail { is_following: true };
                if matches!(mode, FollowMode::Tail) {
                    let item_count = state.items.summary().count;
                    state.logical_scroll_top = Some(ListOffset {
                        item_ix: item_count,
                        offset_in_item: px(0.),
                    });
                }
            }
        }
    }

    pub fn pause_following_tail(&self) {
        self.0.borrow_mut().follow_state.stop_following();
    }

    pub fn is_following_tail(&self) -> bool {
        matches!(
            self.0.borrow().follow_state,
            FollowState::Tail { is_following: true }
        )
    }

    pub fn scroll_to(&self, mut scroll_top: ListOffset) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        if scroll_top.item_ix >= item_count {
            scroll_top.item_ix = item_count;
            scroll_top.offset_in_item = px(0.);
        }

        if scroll_top.item_ix < item_count {
            state.follow_state.stop_following();
        }

        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    pub fn scroll_to_reveal_item(&self, ix: usize) {
        let state = &mut *self.0.borrow_mut();

        let mut scroll_top = state.logical_scroll_top();
        let height = state
            .last_layout_bounds
            .map_or(px(0.), |bounds| bounds.size.height);
        let padding = state.last_padding.unwrap_or_default();

        if ix <= scroll_top.item_ix {
            scroll_top.item_ix = ix;
            scroll_top.offset_in_item = px(0.);
        } else {
            let mut cursor = state.items.cursor::<ListItemSummary>(());
            cursor.seek(&Count(ix + 1), Bias::Right);
            let bottom = cursor.start().height + padding.top;
            let goal_top = px(0.).max(bottom - height + padding.bottom);

            cursor.seek(&Height(goal_top), Bias::Left);
            let start_ix = cursor.start().count;
            let start_item_top = cursor.start().height;

            if start_ix >= scroll_top.item_ix {
                scroll_top.item_ix = start_ix;
                scroll_top.offset_in_item = goal_top - start_item_top;
            }
        }

        state.rebase_pending_scroll(scroll_top);
        state.logical_scroll_top = Some(scroll_top);
    }

    pub fn bounds_for_item(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let state = &*self.0.borrow();

        let bounds = state.last_layout_bounds.unwrap_or_default();
        let padding = state.last_padding.unwrap_or_default();
        let scroll_top = state.logical_scroll_top();
        if ix < scroll_top.item_ix {
            return None;
        }

        let mut cursor = state.items.cursor::<Dimensions<Count, Height>>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

        let scroll_top = cursor.start().1.0 + scroll_top.offset_in_item;

        cursor.seek_forward(&Count(ix), Bias::Right);
        if let Some(&ListItem::Measured { size, .. }) = cursor.item() {
            let &Dimensions(Count(count), Height(top), _) = cursor.start();
            if count == ix {
                let top = bounds.top() + padding.top + top - scroll_top;
                return Some(Bounds::from_corners(
                    point(bounds.left(), top),
                    point(bounds.right(), top + size.height),
                ));
            }
        }
        None
    }

    pub fn scrollbar_drag_started(&self) {
        let mut state = self.0.borrow_mut();
        state.scrollbar_drag_start_height = Some(state.items.summary().height);
    }

    pub fn scrollbar_drag_ended(&self) {
        self.0.borrow_mut().scrollbar_drag_start_height.take();
    }

    pub fn is_scrollbar_dragging(&self) -> bool {
        self.0.borrow().scrollbar_drag_start_height.is_some()
    }

    pub fn set_offset_from_scrollbar(&self, point: Point<Pixels>) {
        self.0.borrow_mut().set_offset_from_scrollbar(point);
    }

    pub fn max_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = self.0.borrow();
        point(Pixels::ZERO, state.max_scroll_offset())
    }

    pub fn scroll_px_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = &self.0.borrow();

        if state.logical_scroll_top.is_none() && state.alignment == ListAlignment::Bottom {
            return Point::new(px(0.), -state.max_scroll_offset());
        }

        let logical_scroll_top = state.logical_scroll_top();

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        let summary: ListItemSummary =
            cursor.summary(&Count(logical_scroll_top.item_ix), Bias::Right);
        let offset = summary.height + logical_scroll_top.offset_in_item;

        Point::new(px(0.), -offset)
    }

    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().last_layout_bounds.unwrap_or_default()
    }

    pub fn item_is_above_viewport(&self, ix: usize) -> Option<bool> {
        let viewport_bounds = self.0.borrow().last_layout_bounds?;

        let scroll_top = self.logical_scroll_top();
        if ix < scroll_top.item_ix {
            return Some(true);
        }

        let item_bounds = self.bounds_for_item(ix)?;
        Some(item_bounds.bottom() <= viewport_bounds.top())
    }

    pub fn item_is_below_viewport(&self, ix: usize) -> Option<bool> {
        let viewport_bounds = self.0.borrow().last_layout_bounds?;

        let scroll_top = self.logical_scroll_top();
        if ix < scroll_top.item_ix {
            return Some(false);
        }

        let item_bounds = self.bounds_for_item(ix)?;
        Some(item_bounds.top() >= viewport_bounds.bottom())
    }
}

impl StateInner {
    fn rebase_pending_scroll(&mut self, scroll_top: ListOffset) {
        let Some(pending) = self.pending_scroll.take() else {
            return;
        };
        if scroll_top.item_ix >= self.items.summary().count {
            return;
        }

        self.pending_scroll = match pending {
            PendingScroll::Absolute { .. } => Some(PendingScroll::Absolute {
                item_ix: scroll_top.item_ix,
                offset: scroll_top.offset_in_item,
            }),
            PendingScroll::Proportional(_) => {
                let mut cursor = self.items.cursor::<Count>(());
                cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
                cursor
                    .item()
                    .and_then(|item| item.size_hint())
                    .filter(|size| size.height.0 > 0.0)
                    .map(|size| {
                        PendingScroll::Proportional(PendingScrollFraction {
                            item_ix: scroll_top.item_ix,
                            fraction: (scroll_top.offset_in_item.0 / size.height.0).clamp(0.0, 1.0),
                        })
                    })
            }
        };
    }

    fn max_scroll_offset(&self) -> Pixels {
        let bounds = self.last_layout_bounds.unwrap_or_default();
        let height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        (height - bounds.size.height).max(px(0.))
    }

    fn visible_range(
        items: &SumTree<ListItem>,
        height: Pixels,
        scroll_top: &ListOffset,
    ) -> Range<usize> {
        let mut cursor = items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
        let start_y = cursor.start().height + scroll_top.offset_in_item;
        cursor.seek_forward(&Height(start_y + height), Bias::Left);
        scroll_top.item_ix..cursor.start().count + 1
    }

    #[allow(clippy::too_many_arguments)]
    fn scroll(
        &mut self,
        scroll_top: &ListOffset,
        height: Pixels,
        delta: Point<Pixels>,
        bounce: Option<TouchPhase>,
        key: ScrollKey,
        current_view: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.reset {
            return;
        }

        let padding = self.last_padding.unwrap_or_default();
        let scroll_max =
            (self.items.summary().height + padding.top + padding.bottom - height).max(px(0.));
        let old_scroll_top = self.scroll_top(scroll_top);
        let new_scroll_top = match bounce {
            Some(phase) => {
                let current = self
                    .logical_scroll_top
                    .map_or_else(|| self.scroll_top(scroll_top), |top| self.scroll_top(&top))
                    .min(scroll_max);
                -self
                    .overscroll
                    .scroll(-current, delta.y, -scroll_max, px(0.), height, phase)
            }
            None => (old_scroll_top - delta.y).max(px(0.)).min(scroll_max),
        };
        if new_scroll_top != old_scroll_top {
            window.claim_scroll(key);
        } else if delta.y != px(0.) {
            window.pin_scroll();
        }

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.pending_scroll = None;
            self.logical_scroll_top = None;
        } else {
            let (start, ..) =
                self.items
                    .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);
            let scroll_top = ListOffset {
                item_ix: start.count,
                offset_in_item: new_scroll_top - start.height,
            };
            self.rebase_pending_scroll(scroll_top);
            self.logical_scroll_top = Some(scroll_top);
        }

        if delta.y > px(0.) {
            self.follow_state.stop_following();
        }

        if let Some(handler) = self.scroll_handler.as_mut() {
            let visible_range = Self::visible_range(&self.items, height, scroll_top);
            handler(
                &ListScrollEvent {
                    visible_range,
                    count: self.items.summary().count,
                    is_scrolled: self.logical_scroll_top.is_some(),
                    is_following_tail: matches!(
                        self.follow_state,
                        FollowState::Tail { is_following: true }
                    ),
                },
                window,
                cx,
            );
        }

        cx.notify(current_view);
    }

    fn logical_scroll_top(&self) -> ListOffset {
        self.logical_scroll_top
            .unwrap_or_else(|| match self.alignment {
                ListAlignment::Top => ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                },
                ListAlignment::Bottom => ListOffset {
                    item_ix: self.items.summary().count,
                    offset_in_item: px(0.),
                },
            })
    }

    fn scroll_top(&self, logical_scroll_top: &ListOffset) -> Pixels {
        let (start, ..) = self.items.find::<ListItemSummary, _>(
            (),
            &Count(logical_scroll_top.item_ix),
            Bias::Right,
        );
        start.height + logical_scroll_top.offset_in_item
    }

    fn layout_all_items(
        &mut self,
        available_width: Pixels,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) {
        match &mut self.measuring_behavior {
            ListMeasuringBehavior::Visible => {
                return;
            }
            ListMeasuringBehavior::Measure(has_measured) => {
                if *has_measured {
                    return;
                }
                *has_measured = true;
            }
        }

        let mut cursor = self.items.cursor::<Count>(());
        let available_item_space = size(
            AvailableSpace::Definite(available_width),
            AvailableSpace::MinContent,
        );

        let mut measured_items = Vec::default();

        for (ix, item) in cursor.enumerate() {
            let size = item.size().unwrap_or_else(|| {
                let mut element = render_item(ix, window, cx);
                element.layout_as_root(available_item_space, window, cx)
            });

            measured_items.push(ListItem::Measured {
                size,
                focus_handle: item.focus_handle(),
            });
        }

        self.items = SumTree::from_iter(measured_items, ());
    }

    fn layout_items(
        &mut self,
        available_width: Option<Pixels>,
        available_height: Pixels,
        padding: &Edges<Pixels>,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) -> LayoutItemsResponse {
        let old_items = self.items.clone();
        let mut measured_items = VecDeque::new();
        let mut item_layouts = VecDeque::new();
        let mut rendered_height = padding.top;
        let mut max_item_width = px(0.);
        let mut scroll_top = self.logical_scroll_top();

        if self.follow_state.is_following() {
            scroll_top = ListOffset {
                item_ix: self.items.summary().count,
                offset_in_item: px(0.),
            };
            self.logical_scroll_top = Some(scroll_top);
        }

        let mut rendered_focused_item = false;

        let available_item_space = size(
            available_width.map_or(AvailableSpace::MaxContent, |width| {
                AvailableSpace::Definite(width)
            }),
            AvailableSpace::MinContent,
        );

        let mut cursor = old_items.cursor::<Count>(());

        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
        for (ix, item) in cursor.by_ref().enumerate() {
            let visible_height = rendered_height - scroll_top.offset_in_item;
            if visible_height >= available_height + self.overdraw {
                break;
            }

            let mut size = item.size();

            if visible_height < available_height || size.is_none() {
                let item_index = scroll_top.item_ix + ix;
                let mut element = render_item(item_index, window, cx);
                let element_size = element.layout_as_root(available_item_space, window, cx);
                size = Some(element_size);

                if ix == 0 {
                    if let Some(pending_scroll) = self.pending_scroll.take() {
                        match pending_scroll {
                            PendingScroll::Absolute { item_ix, offset }
                                if item_ix == scroll_top.item_ix =>
                            {
                                scroll_top.offset_in_item = offset.min(element_size.height);
                                self.logical_scroll_top = Some(scroll_top);
                            }
                            PendingScroll::Proportional(pending_scroll)
                                if pending_scroll.item_ix == scroll_top.item_ix =>
                            {
                                scroll_top.offset_in_item =
                                    Pixels(pending_scroll.fraction * element_size.height.0);
                                self.logical_scroll_top = Some(scroll_top);
                            }
                            _ => {}
                        }
                    }
                }

                if visible_height < available_height {
                    item_layouts.push_back(ItemLayout {
                        index: item_index,
                        element,
                        size: element_size,
                    });
                    if item.contains_focused(window, cx) {
                        rendered_focused_item = true;
                    }
                }
            }

            let size = size.unwrap();
            rendered_height += size.height;
            max_item_width = max_item_width.max(size.width);
            measured_items.push_back(ListItem::Measured {
                size,
                focus_handle: item.focus_handle(),
            });
        }
        rendered_height += padding.bottom;

        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

        if rendered_height - scroll_top.offset_in_item < available_height {
            while rendered_height < available_height {
                cursor.prev();
                if let Some(item) = cursor.item() {
                    let item_index = cursor.start().0;
                    let mut element = render_item(item_index, window, cx);
                    let element_size = element.layout_as_root(available_item_space, window, cx);
                    let focus_handle = item.focus_handle();
                    rendered_height += element_size.height;
                    measured_items.push_front(ListItem::Measured {
                        size: element_size,
                        focus_handle,
                    });
                    item_layouts.push_front(ItemLayout {
                        index: item_index,
                        element,
                        size: element_size,
                    });
                    if item.contains_focused(window, cx) {
                        rendered_focused_item = true;
                    }
                } else {
                    break;
                }
            }

            scroll_top = ListOffset {
                item_ix: cursor.start().0,
                offset_in_item: rendered_height - available_height,
            };

            match self.alignment {
                ListAlignment::Top => {
                    scroll_top.offset_in_item = scroll_top.offset_in_item.max(px(0.));
                    self.logical_scroll_top = Some(scroll_top);
                }
                ListAlignment::Bottom => {
                    scroll_top = ListOffset {
                        item_ix: cursor.start().0,
                        offset_in_item: rendered_height - available_height,
                    };
                    self.logical_scroll_top = None;
                }
            };
        }

        let mut leading_overdraw = scroll_top.offset_in_item;
        while leading_overdraw < self.overdraw {
            cursor.prev();
            if let Some(item) = cursor.item() {
                let size = if let ListItem::Measured { size, .. } = item {
                    *size
                } else {
                    let mut element = render_item(cursor.start().0, window, cx);
                    element.layout_as_root(available_item_space, window, cx)
                };

                leading_overdraw += size.height;
                measured_items.push_front(ListItem::Measured {
                    size,
                    focus_handle: item.focus_handle(),
                });
            } else {
                break;
            }
        }

        let measured_range = cursor.start().0..(cursor.start().0 + measured_items.len());
        let mut cursor = old_items.cursor::<Count>(());
        let mut new_items = cursor.slice(&Count(measured_range.start), Bias::Right);
        new_items.extend(measured_items, ());
        cursor.seek(&Count(measured_range.end), Bias::Right);
        new_items.append(cursor.suffix(), ());
        self.items = new_items;

        if self.follow_state.has_stopped_following() {
            let padding = self.last_padding.unwrap_or_default();
            let total_height = self.items.summary().height + padding.top + padding.bottom;
            let scroll_offset = self.scroll_top(&scroll_top);
            if scroll_offset + available_height >= total_height - px(1.0) {
                self.follow_state.start_following();
            }
        }

        if !rendered_focused_item {
            let mut cursor = self
                .items
                .filter::<_, Count>((), |summary| summary.has_focus_handles);
            cursor.next();
            while let Some(item) = cursor.item() {
                if item.contains_focused(window, cx) {
                    let item_index = cursor.start().0;
                    let mut element = render_item(cursor.start().0, window, cx);
                    let size = element.layout_as_root(available_item_space, window, cx);
                    item_layouts.push_back(ItemLayout {
                        index: item_index,
                        element,
                        size,
                    });
                    break;
                }
                cursor.next();
            }
        }

        LayoutItemsResponse {
            max_item_width,
            scroll_top,
            item_layouts,
        }
    }

    fn prepaint_items(
        &mut self,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        autoscroll: bool,
        render_item: &mut RenderItemFn,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<LayoutItemsResponse, ListOffset> {
        window.transact(|window| {
            match self.measuring_behavior {
                ListMeasuringBehavior::Measure(has_measured) if !has_measured => {
                    self.layout_all_items(bounds.size.width, render_item, window, cx);
                }
                _ => {}
            }

            let mut layout_response = self.layout_items(
                Some(bounds.size.width),
                bounds.size.height,
                &padding,
                render_item,
                window,
                cx,
            );

            window.take_autoscroll();

            if bounds.size.height > padding.top + padding.bottom {
                let shift = Point::new(px(0.), self.overscroll.displacement(bounds.size.height));
                if self.overscroll.is_animating() {
                    window.request_animation_frame();
                }
                let mut item_origin = bounds.origin + Point::new(px(0.), padding.top);
                item_origin.y -= layout_response.scroll_top.offset_in_item;
                for item in &mut layout_response.item_layouts {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        item.element.prepaint_at(item_origin + shift, window, cx);
                    });

                    if let Some(autoscroll_bounds) = window
                        .take_autoscroll()
                        .map(|bounds| Bounds::new(bounds.origin - shift, bounds.size))
                        && autoscroll
                    {
                        if autoscroll_bounds.top() < bounds.top() {
                            let mut item_ix = item.index;
                            let mut offset_in_item = autoscroll_bounds.top() - item_origin.y;

                            if offset_in_item < Pixels::ZERO {
                                let mut cursor = self.items.cursor::<Count>(());
                                cursor.seek(&Count(item_ix), Bias::Right);
                                while offset_in_item < Pixels::ZERO {
                                    cursor.prev();
                                    let Some(prev_item) = cursor.item() else {
                                        offset_in_item = Pixels::ZERO;
                                        break;
                                    };
                                    let size = prev_item.size().unwrap_or_else(|| {
                                        let mut element = render_item(cursor.start().0, window, cx);
                                        let item_available_size = size(
                                            bounds.size.width.into(),
                                            AvailableSpace::MinContent,
                                        );
                                        element.layout_as_root(item_available_size, window, cx)
                                    });
                                    item_ix = cursor.start().0;
                                    offset_in_item += size.height;
                                }
                            }

                            return Err(ListOffset {
                                item_ix,
                                offset_in_item,
                            });
                        } else if autoscroll_bounds.bottom() > bounds.bottom() {
                            let mut cursor = self.items.cursor::<Count>(());
                            cursor.seek(&Count(item.index), Bias::Right);
                            let mut height = bounds.size.height - padding.top - padding.bottom;

                            height -= autoscroll_bounds.bottom() - item_origin.y;

                            while height > Pixels::ZERO {
                                cursor.prev();
                                let Some(item) = cursor.item() else { break };

                                let size = item.size().unwrap_or_else(|| {
                                    let mut item = render_item(cursor.start().0, window, cx);
                                    let item_available_size =
                                        size(bounds.size.width.into(), AvailableSpace::MinContent);
                                    item.layout_as_root(item_available_size, window, cx)
                                });
                                height -= size.height;
                            }

                            return Err(ListOffset {
                                item_ix: cursor.start().0,
                                offset_in_item: if height < Pixels::ZERO {
                                    -height
                                } else {
                                    Pixels::ZERO
                                },
                            });
                        }
                    }

                    item_origin.y += item.size.height;
                }
            } else {
                layout_response.item_layouts.clear();
            }

            Ok(layout_response)
        })
    }

    fn set_offset_from_scrollbar(&mut self, point: Point<Pixels>) {
        let Some(bounds) = self.last_layout_bounds else {
            return;
        };
        let height = bounds.size.height;

        let padding = self.last_padding.unwrap_or_default();
        let content_height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        let scroll_max = (content_height + padding.top + padding.bottom - height).max(px(0.));
        let new_scroll_top = (-point.y).max(px(0.)).min(scroll_max);

        let dragged_to_end =
            scroll_max > px(0.) && new_scroll_top >= (scroll_max - px(1.0)).max(px(0.));
        if dragged_to_end && matches!(self.follow_state, FollowState::Tail { .. }) {
            self.follow_state = FollowState::Tail { is_following: true };
            let item_count = self.items.summary().count;
            self.pending_scroll = None;
            self.logical_scroll_top = Some(ListOffset {
                item_ix: item_count,
                offset_in_item: px(0.),
            });
            return;
        }

        self.follow_state.stop_following();

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.pending_scroll = None;
            self.logical_scroll_top = None;
        } else {
            let (start, _, _) =
                self.items
                    .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);

            let scroll_top = ListOffset {
                item_ix: start.count,
                offset_in_item: new_scroll_top - start.height,
            };
            self.rebase_pending_scroll(scroll_top);
            self.logical_scroll_top = Some(scroll_top);
        }
    }
}

impl std::fmt::Debug for ListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unmeasured { .. } => write!(f, "Unrendered"),
            Self::Measured { size, .. } => f.debug_struct("Rendered").field("size", size).finish(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ListOffset {
    pub item_ix: usize,
    pub offset_in_item: Pixels,
}

impl Element for List {
    type RequestLayoutState = ();
    type PrepaintState = ListPrepaintState;

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let layout_id = match self.sizing_behavior {
            ListSizingBehavior::Infer => {
                let mut style = Style::default();
                style.overflow.y = Overflow::Scroll;
                style.refine(&self.style);
                window.with_text_style(style.text_style().cloned(), |window| {
                    let state = &mut *self.state.0.borrow_mut();

                    let available_height = if let Some(last_bounds) = state.last_layout_bounds {
                        last_bounds.size.height
                    } else {
                        state.overdraw
                    };
                    let padding = style.padding.to_pixels(
                        state.last_layout_bounds.unwrap_or_default().size.into(),
                        window.rem_size(),
                    );

                    let layout_response = state.layout_items(
                        None,
                        available_height,
                        &padding,
                        &mut self.render_item,
                        window,
                        cx,
                    );
                    let max_element_width = layout_response.max_item_width;

                    let summary = state.items.summary();
                    let total_height = summary.height;

                    window.request_measured_layout(
                        style,
                        move |known_dimensions, available_space, _window, _cx| {
                            let width =
                                known_dimensions
                                    .width
                                    .unwrap_or(match available_space.width {
                                        AvailableSpace::Definite(x) => x,
                                        AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                            max_element_width
                                        }
                                    });
                            let height = match available_space.height {
                                AvailableSpace::Definite(height) => total_height.min(height),
                                AvailableSpace::MinContent | AvailableSpace::MaxContent => {
                                    total_height
                                }
                            };
                            size(width, height)
                        },
                    )
                })
            }
            ListSizingBehavior::Auto => {
                let mut style = Style::default();
                style.refine(&self.style);
                window.with_text_style(style.text_style().cloned(), |window| {
                    window.request_layout(style, None, cx)
                })
            }
        };
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> ListPrepaintState {
        let state = &mut *self.state.0.borrow_mut();
        state.reset = false;

        let mut style = Style::default();
        style.refine(&self.style);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        if state
            .last_layout_bounds
            .is_none_or(|last_bounds| last_bounds.size.width != bounds.size.width)
        {
            let new_items = SumTree::from_iter(
                state.items.iter().map(|item| ListItem::Unmeasured {
                    size_hint: None,
                    focus_handle: item.focus_handle(),
                }),
                (),
            );

            state.items = new_items;
            state.measuring_behavior.reset();
        }

        let padding = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());
        let layout =
            match state.prepaint_items(bounds, padding, true, &mut self.render_item, window, cx) {
                Ok(layout) => layout,
                Err(autoscroll_request) => {
                    state.logical_scroll_top = Some(autoscroll_request);
                    state
                        .prepaint_items(bounds, padding, false, &mut self.render_item, window, cx)
                        .unwrap()
                }
            };

        state.last_layout_bounds = Some(bounds);
        state.last_padding = Some(padding);
        ListPrepaintState { hitbox, layout }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();

        let list_state = self.state.clone();
        let height = bounds.size.height;
        let scroll_top = prepaint.layout.scroll_top;
        let hitbox_id = prepaint.hitbox.id;
        let bounce = window.gesture_tuning().overscroll == Overscroll::Bounce;
        let mut accumulated_scroll_delta = ScrollDelta::default();
        let key = ScrollKey::of(&list_state.0);
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && window.should_take_scroll(key, hitbox_id.should_handle_scroll(window))
            {
                let pixel_delta = if bounce {
                    event.delta.pixel_delta(px(20.))
                } else {
                    accumulated_scroll_delta = accumulated_scroll_delta.coalesce(event.delta);
                    accumulated_scroll_delta.pixel_delta(px(20.))
                };
                list_state.0.borrow_mut().scroll(
                    &scroll_top,
                    height,
                    pixel_delta,
                    bounce.then_some(event.touch_phase),
                    key,
                    current_view,
                    window,
                    cx,
                )
            }
        });

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for item in &mut prepaint.layout.item_layouts {
                item.element.paint(window, cx);
            }
        });
    }
}

impl IntoElement for List {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for List {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl crate::sum_tree::Item for ListItem {
    type Summary = ListItemSummary;

    fn summary(&self, _: ()) -> Self::Summary {
        match self {
            ListItem::Unmeasured {
                size_hint,
                focus_handle,
            } => ListItemSummary {
                count: 1,
                rendered_count: 0,
                unrendered_count: 1,
                height: if let Some(size) = size_hint {
                    size.height
                } else {
                    px(0.)
                },
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: size_hint.is_none(),
            },
            ListItem::Measured {
                size, focus_handle, ..
            } => ListItemSummary {
                count: 1,
                rendered_count: 1,
                unrendered_count: 0,
                height: size.height,
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: false,
            },
        }
    }
}

impl crate::sum_tree::ContextLessSummary for ListItemSummary {
    fn zero() -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &Self) {
        self.count += summary.count;
        self.rendered_count += summary.rendered_count;
        self.unrendered_count += summary.unrendered_count;
        self.height += summary.height;
        self.has_focus_handles |= summary.has_focus_handles;
        self.has_unknown_height |= summary.has_unknown_height;
    }
}

impl<'a> crate::sum_tree::Dimension<'a, ListItemSummary> for Count {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.count;
    }
}

impl<'a> crate::sum_tree::Dimension<'a, ListItemSummary> for Height {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.height;
    }
}

impl crate::sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Count {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.count).unwrap()
    }
}

impl crate::sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Height {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.height).unwrap()
    }
}

#[cfg(test)]
mod test {

    use ala::{ScrollDelta, ScrollWheelEvent};
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::{
        self as ala, AppContext, Bounds, Context, Element, FollowMode, InteractiveElement,
        IntoElement, ListState, ParentElement, Render, ScrollHandle, StatefulInteractiveElement,
        Styled, TestAppContext, TouchPhase, VisualTestContext, Window, canvas, div, list, point,
        px, size,
    };

    #[ala::test]
    fn test_autoscroll_above_item_top_renders_items_above(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));
        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |ix, _, _| {
                    if ix == 2 {
                        canvas(
                            |bounds, window, _| {
                                window.request_autoscroll(Bounds::from_corners(
                                    point(bounds.left(), bounds.top() - px(30.)),
                                    point(bounds.right(), bounds.top() + px(5.)),
                                ));
                            },
                            |_, _, _, _| {},
                        )
                        .h(px(20.))
                        .w_full()
                        .into_any()
                    } else {
                        div().h(px(20.)).w_full().into_any()
                    }
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(60.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        let scroll_top = state.logical_scroll_top();
        assert!(
            scroll_top.offset_in_item >= px(0.),
            "offset_in_item must never be negative (would leave blank space above), got {:?}",
            scroll_top.offset_in_item,
        );
        assert_eq!(scroll_top.item_ix, 0);
        assert_eq!(scroll_top.offset_in_item, px(10.));
    }

    #[ala::test]
    fn test_reset_after_paint_before_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        state.scroll_to(ala::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(10.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        state.reset(5);

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(1.), px(1.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-500.))),
            ..Default::default()
        });

        assert_eq!(state.logical_scroll_top().item_ix, 0);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
    }

    #[ala::test]
    fn test_scroll_by_positive_and_negative_distance(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(20.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        state.scroll_by(px(30.));

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 1);
        assert_eq!(offset.offset_in_item, px(10.));

        state.scroll_by(px(-30.));

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));

        state.scroll_by(px(0.));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[ala::test]
    fn test_child_scroll_handler_can_stop_list_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));
        let child_saw_event = Rc::new(Cell::new(false));

        struct TestView {
            state: ListState,
            child_saw_event: Rc<Cell<bool>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let child_saw_event = self.child_saw_event.clone();
                list(self.state.clone(), move |_, _, _| {
                    let child_saw_event = child_saw_event.clone();
                    div()
                        .h(px(20.))
                        .w_full()
                        .on_scroll_wheel(move |_, _, cx| {
                            child_saw_event.set(true);
                            cx.stop_propagation();
                        })
                        .into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                child_saw_event: child_saw_event.clone(),
            })
            .into_any_element()
        });

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(10.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        assert!(
            child_saw_event.get(),
            "the child's scroll-wheel handler should run"
        );
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    struct TestListView(ListState);
    impl Render for TestListView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            list(self.0.clone(), |_, _, _| {
                div().h(px(20.)).w_full().into_any()
            })
            .w_full()
            .h_full()
        }
    }

    #[ala::test]
    fn test_item_viewport_queries_return_none_before_layout(_cx: &mut TestAppContext) {
        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        assert_eq!(state.item_is_above_viewport(0), None);
        assert_eq!(state.item_is_below_viewport(0), None);
    }

    #[ala::test]
    fn test_item_viewport_queries_before_logical_scroll_top(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(1), Some(true));
        assert_eq!(state.item_is_below_viewport(1), Some(false));
    }

    #[ala::test]
    fn test_item_viewport_queries_measured_item_inside_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(2), Some(false));
        assert_eq!(state.item_is_below_viewport(2), Some(false));
    }

    #[ala::test]
    fn test_item_viewport_queries_measured_item_above_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(20.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(2), Some(true));
        assert_eq!(state.item_is_below_viewport(2), Some(false));
    }

    #[ala::test]
    fn test_item_viewport_queries_measured_item_below_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));
    }

    #[ala::test]
    fn test_item_viewport_queries_remain_stable_with_zero_height_viewport(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(0.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(0.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        assert_eq!(state.item_is_above_viewport(1), Some(true));
        assert_eq!(state.item_is_below_viewport(1), Some(false));
        assert_eq!(state.item_is_above_viewport(3), Some(false));
        assert_eq!(state.item_is_below_viewport(3), Some(true));
    }

    #[ala::test]
    fn test_item_viewport_queries_after_scroll_to_end_before_layout(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestListView(state.clone())).into_any_element()
        });

        state.scroll_to_end();

        assert_eq!(state.logical_scroll_top().item_ix, state.item_count());
        assert_eq!(state.item_is_above_viewport(0), Some(true));
        assert_eq!(state.item_is_below_viewport(0), Some(false));
    }

    #[ala::test]
    fn test_bounds_for_item_includes_top_padding(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.)).measure_all();
        let painted = Rc::new(std::cell::RefCell::new(Vec::new()));

        struct PaddedListView(
            ListState,
            Rc<std::cell::RefCell<Vec<Bounds<crate::Pixels>>>>,
        );
        impl Render for PaddedListView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let painted = self.1.clone();
                list(self.0.clone(), move |_, _, _| {
                    let painted = painted.clone();
                    canvas(
                        move |bounds, _, _| painted.borrow_mut().push(bounds),
                        |_, _, _, _| {},
                    )
                    .h(px(20.))
                    .w_full()
                    .into_any()
                })
                .pt(px(30.))
                .w_full()
                .h_full()
            }
        }

        state.scroll_to(ala::ListOffset {
            item_ix: 1,
            offset_in_item: px(5.),
        });
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| PaddedListView(state.clone(), painted.clone()))
                .into_any_element()
        });

        let painted = painted.borrow();
        let first = painted.first().copied().expect("rows were prepainted");
        assert_eq!(first.top(), px(25.));
        assert_eq!(
            state.bounds_for_item(1).map(|bounds| bounds.top()),
            Some(first.top())
        );
        assert_eq!(
            state.bounds_for_item(2).map(|bounds| bounds.top()),
            Some(px(45.))
        );
    }

    #[ala::test]
    fn test_measure_all_after_width_change(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));

        cx.draw(point(px(0.), px(0.)), size(px(200.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));
    }

    #[ala::test]
    fn test_remeasure(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.scroll_to(ala::ListOffset {
            item_ix: 2,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(40.));

        item_height.set(50);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(20.));
    }

    #[ala::test]
    fn test_remeasure_item_preserves_scroll_offset(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 5 { height } else { 100 };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.scroll_to(ala::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        item_height.set(200);
        state.remeasure_items(5..6);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(40.));
    }

    #[ala::test]
    fn test_remeasure_then_scroll_does_not_revert_scroll_position(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(100.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = {
            let state = state.clone();
            cx.update(|_, cx| cx.new(|_| TestView(state)))
        };

        state.scroll_to(ala::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.remeasure_items(5..6);

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(70.));

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(
            offset.offset_in_item,
            px(70.),
            "scrolling after a remeasure should not be reverted by the stale pending scroll"
        );
    }

    #[ala::test]
    fn test_scroll_after_remeasure_clamps_to_shrunk_item_height(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 5 { height } else { 100 };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = {
            let state = state.clone();
            let item_height = item_height.clone();
            cx.update(|_, cx| cx.new(|_| TestView { state, item_height }))
        };

        state.scroll_to(ala::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        item_height.set(50);
        state.remeasure_items(5..6);

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-30.))),
            ..Default::default()
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(50.));
    }

    #[ala::test]
    fn test_follow_tail_stays_at_bottom_as_items_grow(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());

        item_height.set(80);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 7);
        assert_eq!(offset.offset_in_item, px(40.));
        assert!(state.is_following_tail());
    }

    #[ala::test]
    fn test_pause_following_tail_reengages_when_still_at_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));
        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.pause_following_tail();
        assert!(!state.is_following_tail());

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "pausing while at the bottom must re-engage follow-tail on the next layout"
        );
    }

    #[ala::test]
    fn test_pause_following_tail_freezes_off_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                item_height: item_height.clone(),
            })
        });
        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert_eq!(state.logical_scroll_top().item_ix, 6);
        assert!(state.is_following_tail());

        state.pause_following_tail();
        item_height.set(80);
        state.remeasure();
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(
            !state.is_following_tail(),
            "a paused list must not re-engage while the frozen top is off the bottom"
        );

        item_height.set(50);
        state.remeasure();
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "returning to the bottom must restore follow-tail"
        );
    }

    #[ala::test]
    fn test_follow_tail_disengages_on_user_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });
        assert!(state.is_following_tail());

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
            ..Default::default()
        });

        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the user scrolls toward the start"
        );
    }

    #[ala::test]
    fn test_follow_tail_disengages_on_scrollbar_reposition(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the scrollbar manually repositions the list"
        );

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[ala::test]
    fn test_scrollbar_drag_with_growing_content(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let last_item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView {
            state: ListState,
            last_item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let last_item_height = self.last_item_height.clone();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 9 {
                        last_item_height.get()
                    } else {
                        50
                    };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                last_item_height: last_item_height.clone(),
            })
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.scrollbar_drag_started();

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let scrollbar_offset_before_growth = state.scroll_px_offset_for_scrollbar();

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));

        last_item_height.set(550);
        state.remeasure_items(9..10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));
        assert_eq!(
            state.scroll_px_offset_for_scrollbar(),
            scrollbar_offset_before_growth
        );

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[ala::test]
    fn test_set_follow_tail_snaps_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.scroll_to(ala::ListOffset {
            item_ix: 3,
            offset_in_item: px(0.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(!state.is_following_tail());

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());
    }

    #[ala::test]
    fn test_bottom_aligned_scrollbar_offset_at_end(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        const ITEMS: usize = 10;
        const ITEM_SIZE: f32 = 50.0;

        let state = ListState::new(
            ITEMS,
            crate::ListAlignment::Bottom,
            px(ITEMS as f32 * ITEM_SIZE),
        );

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(ITEM_SIZE)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        assert_eq!(state.logical_scroll_top().item_ix, ITEMS);

        let max_offset = state.max_offset_for_scrollbar();
        let scroll_offset = state.scroll_px_offset_for_scrollbar();

        assert_eq!(
            -scroll_offset.y, max_offset.y,
            "scrollbar offset ({}) should equal max offset ({}) when list is pinned to bottom",
            -scroll_offset.y, max_offset.y,
        );
    }

    #[ala::test]
    fn test_follow_tail_reengages_when_scrolled_back_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(50.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            ..Default::default()
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom"
        );
    }

    #[ala::test]
    fn test_follow_tail_reengagement_not_fooled_by_unmeasured_items(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(20, crate::ListAlignment::Top, px(1000.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(200.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        state.remeasure_items(19..20);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            !state.is_following_tail(),
            "follow_tail should not falsely re-engage due to an unmeasured item \
             reducing items.summary().height"
        );
    }

    #[ala::test]
    fn test_follow_tail_reengages_after_scrollbar_disengagement(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        assert!(!state.is_following_tail());

        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom via the scrollbar"
        );
    }

    #[ala::test]
    fn test_follow_tail_reengages_after_scrollbar_drag_to_bottom_while_growing(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.)).measure_all();

        struct TestView(ListState);
        impl Render for TestView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.scrollbar_drag_started();

        state.splice(10..10, 10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        state.scrollbar_drag_ended();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage when the user drags the scrollbar to \
             the bottom of its track, even when content has grown during the drag \
             (so frozen_bottom < live_bottom)"
        );
    }

    #[ala::test]
    fn test_autoscroll_retry_keeps_one_a11y_node_per_row(cx: &mut TestAppContext) {
        struct A11yRowsView {
            state: ListState,
            focus: crate::FocusHandle,
        }
        impl Render for A11yRowsView {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let focus = self.focus.clone();
                list(self.state.clone(), move |ix, _, _| {
                    let row = div()
                        .id(("row", ix))
                        .role(accesskit::Role::ListItem)
                        .h(px(20.))
                        .w_full();
                    if ix == 1 {
                        row.track_focus(&focus)
                            .child(
                                canvas(
                                    |bounds, window, _| window.request_autoscroll(bounds),
                                    |_, _, _, _| {},
                                )
                                .size_full(),
                            )
                            .into_any()
                    } else {
                        row.into_any()
                    }
                })
                .w_full()
                .h(px(30.))
            }
        }

        let state = ListState::new(3, crate::ListAlignment::Top, px(10.));
        let focus = cx.update(|cx| cx.focus_handle());
        let window = cx.add_window(|_, _| A11yRowsView {
            state: state.clone(),
            focus: focus.clone(),
        });
        cx.update_window(window.into(), |_, window, cx| {
            window.set_a11y_forced(true);
            window.focus(&focus, cx);
            state.scroll_to(ala::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.),
            });
            window.draw(cx).clear(cx);

            assert_eq!(state.logical_scroll_top().offset_in_item, px(10.));
            let tree = window.a11y_tree().expect("forced a11y builds a tree");
            let mut rows = tree
                .nodes
                .iter()
                .filter(|(_, node)| node.role() == accesskit::Role::ListItem)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            let row_count = rows.len();
            rows.sort_unstable_by_key(|id| id.0);
            rows.dedup();
            assert_eq!(rows.len(), row_count);
            assert!(rows.contains(&tree.focus));
            let parents = tree
                .nodes
                .iter()
                .flat_map(|(_, node)| node.children())
                .filter(|child| rows.contains(child))
                .count();
            assert_eq!(parents, row_count);
        })
        .unwrap();
    }

    fn scroll_at(cx: &mut VisualTestContext, y: f32, dy: f32, touch_phase: TouchPhase) {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(y)),
            delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
            touch_phase,
            ..Default::default()
        });
    }

    #[ala::test]
    fn test_nested_scroll_areas_do_not_scroll_together(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let outer = ScrollHandle::new();
        let inner = ScrollHandle::new();

        struct Nested {
            outer: ScrollHandle,
            inner: ScrollHandle,
        }
        impl Render for Nested {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .id("outer")
                    .w(px(100.))
                    .h(px(100.))
                    .overflow_y_scroll()
                    .track_scroll(&self.outer)
                    .child(
                        div()
                            .id("inner")
                            .w_full()
                            .h(px(50.))
                            .overflow_y_scroll()
                            .track_scroll(&self.inner)
                            .child(div().w_full().h(px(80.))),
                    )
                    .child(div().w_full().h(px(300.)))
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| Nested {
                outer: outer.clone(),
                inner: inner.clone(),
            })
            .into_any_element()
        });

        scroll_at(cx, 10., -20., TouchPhase::Started);
        assert_eq!(inner.offset().y, px(-20.));
        assert_eq!(outer.offset().y, px(0.));

        scroll_at(cx, 10., -40., TouchPhase::Moved);
        assert_eq!(inner.offset().y, px(-30.));
        assert_eq!(outer.offset().y, px(0.));

        scroll_at(cx, 10., -40., TouchPhase::Moved);
        assert_eq!(
            outer.offset().y,
            px(0.),
            "the rest of a gesture stays with the area it started in"
        );

        scroll_at(cx, 10., -20., TouchPhase::Started);
        assert_eq!(inner.offset().y, px(-30.));
        assert_eq!(
            outer.offset().y,
            px(-20.),
            "a new gesture passes an area at its end to the one around it"
        );

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| Nested {
                outer: outer.clone(),
                inner: inner.clone(),
            })
            .into_any_element()
        });
        scroll_at(cx, 40., 20., TouchPhase::Started);
        assert_eq!(outer.offset().y, px(0.));
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| Nested {
                outer: outer.clone(),
                inner: inner.clone(),
            })
            .into_any_element()
        });
        scroll_at(cx, 40., 20., TouchPhase::Moved);
        assert_eq!(
            inner.offset().y,
            px(-30.),
            "an area that slides under the pointer mid-gesture leaves the gesture alone"
        );
    }

    #[ala::test]
    fn test_scroll_area_in_list_row_keeps_the_list_still(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();
        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));
        let inner = ScrollHandle::new();

        struct Rows {
            state: ListState,
            inner: ScrollHandle,
        }
        impl Render for Rows {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let inner = self.inner.clone();
                list(self.state.clone(), move |ix, _, _| {
                    if ix == 0 {
                        div()
                            .id("inner")
                            .w_full()
                            .h(px(40.))
                            .overflow_y_scroll()
                            .track_scroll(&inner)
                            .child(div().w_full().h(px(100.)))
                            .into_any()
                    } else {
                        div().w_full().h(px(40.)).into_any()
                    }
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| Rows {
                state: state.clone(),
                inner: inner.clone(),
            })
            .into_any_element()
        });

        scroll_at(cx, 10., -30., TouchPhase::Started);
        assert_eq!(inner.offset().y, px(-30.));
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
        assert_eq!(state.logical_scroll_top().item_ix, 0);

        scroll_at(cx, 10., -30., TouchPhase::Moved);
        scroll_at(cx, 10., -30., TouchPhase::Moved);
        assert_eq!(inner.offset().y, px(-60.));
        assert_eq!(state.logical_scroll_top().item_ix, 0);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));

        scroll_at(cx, 10., -10., TouchPhase::Started);
        assert_eq!(inner.offset().y, px(-60.));
        assert_eq!(state.logical_scroll_top().offset_in_item, px(10.));
    }
    #[ala::test]
    fn raw_touch_can_switch_between_scroll_areas(cx: &mut TestAppContext) {
        let first = ScrollHandle::new();
        let second = ScrollHandle::new();
        struct Areas {
            first: ScrollHandle,
            second: ScrollHandle,
        }
        impl Render for Areas {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .flex()
                    .w(px(200.))
                    .h(px(100.))
                    .child(
                        div()
                            .id("first")
                            .w(px(100.))
                            .h(px(100.))
                            .overflow_y_scroll()
                            .track_scroll(&self.first)
                            .child(div().h(px(500.)).w_full()),
                    )
                    .child(
                        div()
                            .id("second")
                            .w(px(100.))
                            .h(px(100.))
                            .overflow_y_scroll()
                            .track_scroll(&self.second)
                            .child(div().h(px(500.)).w_full()),
                    )
            }
        }
        let (_, cx) = cx.add_window_view(|_, _| Areas {
            first: first.clone(),
            second: second.clone(),
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let touch = |cx: &mut VisualTestContext, id, phase, x, y| {
            cx.simulate_event(crate::TouchEvent {
                id: crate::TouchId(id),
                phase,
                position: point(px(x), px(y)),
                predicted_position: None,
                force: None,
            });
        };
        touch(cx, 1, TouchPhase::Started, 50., 80.);
        touch(cx, 1, TouchPhase::Moved, 50., 40.);
        touch(cx, 1, TouchPhase::Moved, 50., 20.);
        touch(cx, 1, TouchPhase::Ended, 50., 20.);
        let first_after = first.offset().y;
        assert!(first_after < px(0.), "first swipe must scroll its own area");
        assert_eq!(second.offset().y, px(0.));
        touch(cx, 2, TouchPhase::Started, 150., 80.);
        touch(cx, 2, TouchPhase::Moved, 150., 40.);
        touch(cx, 2, TouchPhase::Moved, 150., 20.);
        touch(cx, 2, TouchPhase::Ended, 150., 20.);
        assert!(
            second.offset().y < px(0.),
            "second swipe must scroll the newly touched area; first={:?}, second={:?}",
            first.offset(),
            second.offset()
        );
        assert_eq!(
            first.offset().y,
            first_after,
            "the first area's gesture has ended"
        );
    }
}
