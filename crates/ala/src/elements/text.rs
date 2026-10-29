use crate::util::ResultExt;
use crate::{
    ActiveTooltip, AnyView, App, Bounds, DispatchPhase, Element, ElementId, GlobalElementId,
    HighlightStyle, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, Size, TextOverflow,
    TextRun, TextStyle, TooltipId, TruncateFrom, WhiteSpace, Window, WrappedLine,
    WrappedLineLayout, register_tooltip_mouse_handlers, set_tooltip_on_window,
};
use anyhow::Context as _;
use itertools::Itertools;
use smallvec::SmallVec;
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    mem,
    ops::{Deref, DerefMut, Range},
    rc::Rc,
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct Text {
    id: Option<ElementId>,
    text: SharedString,
}

impl Text {
    #[inline]
    pub const fn new(id: ElementId, text: SharedString) -> Self {
        Self { id: Some(id), text }
    }

    #[inline]
    pub const fn new_inaccessible(text: SharedString) -> Self {
        Self { id: None, text }
    }

    #[inline]
    pub const fn id(&self) -> Option<&ElementId> {
        self.id.as_ref()
    }

    pub fn with_id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    #[inline]
    pub const fn text(&self) -> &SharedString {
        &self.text
    }
}

impl Deref for Text {
    type Target = SharedString;
    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

impl DerefMut for Text {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.text
    }
}

#[doc(hidden)]
pub const fn __hash_text_macro_location_unstable_do_not_use(s: &'static str) -> u64 {
    const BASIS: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x100000001b3;

    let bytes = s.as_bytes();
    let mut hash = BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

#[macro_export]
macro_rules! text {
    (id = $id:expr, $text:expr) => {{ $crate::Text::new($id.into(), $text.into()) }};
    ($text:expr) => {{
        const ID: &'static str = concat!(file!(), "/", line!(), ":", column!());
        const HASH: u64 = $crate::__hash_text_macro_location_unstable_do_not_use(ID);
        $crate::Text::new($crate::ElementId::Integer(HASH), $text.into())
    }};
}

impl IntoElement for Text {
    type Element = Self;
    #[inline]
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Text {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        self.id.clone()
    }

    fn a11y_text(&self) -> Option<SharedString> {
        self.id.is_none().then(|| self.text.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        if self.id.is_some() {
            Some(accesskit::Role::Label)
        } else {
            None
        }
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.to_string());
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        <SharedString as Element>::request_layout(&mut self.text, id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        <SharedString as Element>::prepaint(
            &mut self.text,
            id,
            inspector_id,
            bounds,
            request_layout,
            window,
            cx,
        )
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        <SharedString as Element>::paint(
            &mut self.text,
            id,
            inspector_id,
            bounds,
            request_layout,
            prepaint,
            window,
            cx,
        );
    }
}

impl Element for &'static str {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn a11y_text(&self) -> Option<SharedString> {
        Some(SharedString::new_static(self))
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut state = TextLayout::default();
        let layout_id = state.layout(SharedString::from(*self), None, window, cx);
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        text_layout.prepaint(bounds, self)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        text_layout: &mut TextLayout,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        text_layout.paint(self, window, cx)
    }
}

impl IntoElement for &'static str {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl IntoElement for String {
    type Element = SharedString;

    fn into_element(self) -> Self::Element {
        self.into()
    }
}

impl IntoElement for Cow<'static, str> {
    type Element = SharedString;

    fn into_element(self) -> Self::Element {
        self.into()
    }
}

impl Element for SharedString {
    type RequestLayoutState = TextLayout;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn a11y_text(&self) -> Option<SharedString> {
        Some(self.clone())
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut state = TextLayout::default();
        let layout_id = state.layout(self.clone(), None, window, cx);
        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        text_layout.prepaint(bounds, self.as_ref())
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        text_layout: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        text_layout.paint(self.as_ref(), window, cx)
    }
}

impl IntoElement for SharedString {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub struct StyledText {
    text: SharedString,
    runs: Option<Vec<TextRun>>,
    delayed_highlights: Option<Vec<(Range<usize>, HighlightStyle)>>,
    delayed_font_family_overrides: Option<Vec<(Range<usize>, SharedString)>>,
    layout: TextLayout,
}

impl StyledText {
    pub fn new(text: impl Into<SharedString>) -> Self {
        StyledText {
            text: text.into(),
            runs: None,
            delayed_highlights: None,
            delayed_font_family_overrides: None,
            layout: TextLayout::default(),
        }
    }

    pub fn layout(&self) -> &TextLayout {
        &self.layout
    }

    pub fn with_default_highlights(
        mut self,
        default_style: &TextStyle,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        debug_assert!(
            self.delayed_highlights.is_none(),
            "Can't use `with_default_highlights` and `with_highlights`"
        );
        let runs = Self::compute_runs(&self.text, default_style, highlights);
        self.with_runs(runs)
    }

    pub fn with_highlights(
        mut self,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        debug_assert!(
            self.runs.is_none(),
            "Can't use `with_highlights` and `with_default_highlights`"
        );
        self.delayed_highlights = Some(
            highlights
                .into_iter()
                .inspect(|(run, _)| {
                    debug_assert!(self.text.is_char_boundary(run.start));
                    debug_assert!(self.text.is_char_boundary(run.end));
                })
                .collect::<Vec<_>>(),
        );
        self
    }

    fn compute_runs(
        text: &str,
        default_style: &TextStyle,
        highlights: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Vec<TextRun> {
        let mut runs = Vec::new();
        let mut ix = 0;
        for (range, highlight) in highlights {
            if ix < range.start {
                debug_assert!(text.is_char_boundary(range.start));
                runs.push(default_style.clone().to_run(range.start - ix));
            }
            debug_assert!(text.is_char_boundary(range.end));
            runs.push(
                default_style
                    .clone()
                    .highlight(highlight)
                    .to_run(range.len()),
            );
            ix = range.end;
        }
        if ix < text.len() {
            runs.push(default_style.to_run(text.len() - ix));
        }
        runs
    }

    pub fn with_font_family_overrides(
        mut self,
        overrides: impl IntoIterator<Item = (Range<usize>, SharedString)>,
    ) -> Self {
        self.delayed_font_family_overrides = Some(
            overrides
                .into_iter()
                .inspect(|(range, _)| {
                    debug_assert!(self.text.is_char_boundary(range.start));
                    debug_assert!(self.text.is_char_boundary(range.end));
                })
                .collect(),
        );
        self
    }

    fn apply_font_family_overrides(
        runs: &mut [TextRun],
        overrides: &[(Range<usize>, SharedString)],
    ) {
        let mut byte_offset = 0;
        let mut override_idx = 0;
        for run in runs.iter_mut() {
            let run_end = byte_offset + run.len;
            while override_idx < overrides.len() && overrides[override_idx].0.end <= byte_offset {
                override_idx += 1;
            }
            if override_idx < overrides.len() {
                let (ref range, ref family) = overrides[override_idx];
                if byte_offset >= range.start && run_end <= range.end {
                    run.font.family = family.clone();
                }
            }
            byte_offset = run_end;
        }
    }

    pub fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
        let mut text = &*self.text;
        for run in &runs {
            text = text.get(run.len..).unwrap_or_else(|| {
                #[cfg(debug_assertions)]
                panic!("invalid text run. Text: '{text}', run: {run:?}");
                #[cfg(not(debug_assertions))]
                panic!("invalid text run");
            });
        }
        assert!(text.is_empty(), "invalid text run");
        self.runs = Some(runs);
        self
    }
}

impl Element for StyledText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn a11y_text(&self) -> Option<SharedString> {
        Some(self.text.clone())
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let font_family_overrides = self.delayed_font_family_overrides.take();
        let mut runs = self.runs.take().or_else(|| {
            self.delayed_highlights.take().map(|delayed_highlights| {
                Self::compute_runs(&self.text, &window.text_style(), delayed_highlights)
            })
        });

        if let Some(ref overrides) = font_family_overrides {
            let runs =
                runs.get_or_insert_with(|| vec![window.text_style().to_run(self.text.len())]);
            Self::apply_font_family_overrides(runs, overrides);
        }

        let layout_id = self.layout.layout(self.text.clone(), runs, window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
        self.layout.prepaint(bounds, &self.text)
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.layout.paint(&self.text, window, cx)
    }
}

impl IntoElement for StyledText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[derive(Default, Clone)]
pub struct TextLayout(Rc<RefCell<Option<TextLayoutInner>>>);

struct TextLayoutInner {
    len: usize,
    lines: SmallVec<[WrappedLine; 1]>,
    line_height: Pixels,
    wrap_width: Option<Pixels>,
    truncate_width: Option<Pixels>,
    fade: Option<Pixels>,
    size: Option<Size<Pixels>>,
    bounds: Option<Bounds<Pixels>>,
}

impl TextLayout {
    fn layout(
        &self,
        text: SharedString,
        runs: Option<Vec<TextRun>>,
        window: &mut Window,
        _: &mut App,
    ) -> LayoutId {
        let text_style = window.text_style();
        let font_size = text_style.font_size.to_pixels(window.rem_size());
        let line_height = window.pixel_snap(
            text_style
                .line_height
                .to_pixels(font_size.into(), window.rem_size()),
        );

        let runs = if let Some(runs) = runs {
            runs
        } else {
            vec![text_style.to_run(text.len())]
        };
        window.request_measured_layout(Default::default(), {
            let element_state = self.clone();

            move |known_dimensions, available_space, window, cx| {
                let wrap_width = if text_style.white_space == WhiteSpace::Normal {
                    known_dimensions.width.or(match available_space.width {
                        crate::AvailableSpace::Definite(x) => Some(x),
                        _ => None,
                    })
                } else {
                    None
                };

                let (truncate_width, truncation_affix, truncate_from) =
                    if let Some(text_overflow) = text_style.text_overflow.clone() {
                        let width = known_dimensions.width.or(match available_space.width {
                            crate::AvailableSpace::Definite(x) => match text_style.line_clamp {
                                Some(max_lines) => Some(x * max_lines),
                                None => Some(x),
                            },
                            _ => None,
                        });

                        match text_overflow {
                            TextOverflow::Truncate(s) => (width, s, TruncateFrom::End),
                            TextOverflow::TruncateStart(s) => (width, s, TruncateFrom::Start),
                            TextOverflow::TruncateMiddle(s) => (width, s, TruncateFrom::Middle),
                            TextOverflow::Fade(_) => (None, "".into(), TruncateFrom::End),
                        }
                    } else {
                        (None, "".into(), TruncateFrom::End)
                    };

                let fade = match text_style.text_overflow {
                    Some(TextOverflow::Fade(width)) => Some(width),
                    _ => None,
                };
                let fit_width = fade.and(known_dimensions.width.or(match available_space.width {
                    crate::AvailableSpace::Definite(x) => Some(x),
                    _ => None,
                }));
                let fit = |mut size: Size<Pixels>| {
                    if let Some(width) = fit_width {
                        size.width = size.width.min(width);
                    }
                    size
                };

                if let Some(text_layout) = element_state.0.borrow().as_ref()
                    && let Some(size) = text_layout.size
                    && (wrap_width == text_layout.wrap_width
                        || (text_layout.wrap_width.is_none()
                            && wrap_width.is_some_and(|wrap_width| size.width <= wrap_width)))
                    && truncate_width.is_none()
                    && text_layout.truncate_width.is_none()
                {
                    return fit(size);
                }

                let mut line_wrapper = cx.text_system().line_wrapper(text_style.font(), font_size);
                let (text, runs) = if let Some(truncate_width) = truncate_width {
                    if let Some(max_lines) = text_style.line_clamp
                        && let Some(wrap_width) = wrap_width
                    {
                        line_wrapper.truncate_wrapped_line(
                            text.clone(),
                            wrap_width,
                            max_lines,
                            &truncation_affix,
                            &runs,
                            truncate_from,
                        )
                    } else if let Some(unclipped) = window
                        .text_system()
                        .shape_text(text.clone(), font_size, &runs, None, None)
                        .log_err()
                        && unclipped
                            .iter()
                            .all(|line| line.size(line_height).width <= truncate_width)
                    {
                        (text.clone(), Cow::Borrowed(&*runs))
                    } else {
                        line_wrapper.truncate_line(
                            text.clone(),
                            truncate_width,
                            &truncation_affix,
                            &runs,
                            truncate_from,
                        )
                    }
                } else {
                    (text.clone(), Cow::Borrowed(&*runs))
                };
                let len = text.len();

                let Some(lines) = window
                    .text_system()
                    .shape_text(text, font_size, &runs, wrap_width, text_style.line_clamp)
                    .log_err()
                else {
                    element_state.0.borrow_mut().replace(TextLayoutInner {
                        lines: Default::default(),
                        len: 0,
                        line_height,
                        wrap_width,
                        truncate_width,
                        fade,
                        size: Some(Size::default()),
                        bounds: None,
                    });
                    return Size::default();
                };

                let mut size: Size<Pixels> = Size::default();
                for line in &lines {
                    let line_size = line.size(line_height);
                    size.height += line_size.height;
                    size.width = size.width.max(line_size.width).ceil();
                }

                element_state.0.borrow_mut().replace(TextLayoutInner {
                    lines,
                    len,
                    line_height,
                    wrap_width,
                    truncate_width,
                    fade,
                    size: Some(size),
                    bounds: None,
                });

                fit(size)
            }
        })
    }

    fn prepaint(&self, bounds: Bounds<Pixels>, text: &str) {
        let mut element_state = self.0.borrow_mut();
        let element_state = element_state
            .as_mut()
            .with_context(|| format!("measurement has not been performed on {text}"))
            .unwrap();
        element_state.bounds = Some(bounds);
    }

    fn paint(&self, text: &str, window: &mut Window, cx: &mut App) {
        let element_state = self.0.borrow();
        let element_state = element_state
            .as_ref()
            .with_context(|| format!("measurement has not been performed on {text}"))
            .unwrap();
        let bounds = element_state
            .bounds
            .with_context(|| format!("prepaint has not been performed on {text}"))
            .unwrap();

        let line_height = element_state.line_height;
        let mut line_origin = bounds.origin;
        let text_align = window.text_align();
        let fade = element_state
            .fade
            .filter(|_| {
                element_state
                    .lines
                    .iter()
                    .any(|line| line.size(line_height).width > bounds.size.width + crate::px(0.5))
            })
            .map(|width| (bounds.right() - width, bounds.right()));
        window.with_text_fade(fade, |window| {
            for line in &element_state.lines {
                line.paint_background(
                    line_origin,
                    line_height,
                    text_align,
                    Some(bounds),
                    window,
                    cx,
                )
                .log_err();
                line.paint(
                    line_origin,
                    line_height,
                    text_align,
                    Some(bounds),
                    window,
                    cx,
                )
                .log_err();
                line_origin.y += line.size(line_height).height;
            }
        });
    }

    pub fn index_for_position(&self, mut position: Point<Pixels>) -> Result<usize, usize> {
        let element_state = self.0.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let bounds = element_state
            .bounds
            .expect("prepaint has not been performed");

        if position.y < bounds.top() {
            return Err(0);
        }

        let line_height = element_state.line_height;
        let mut line_origin = bounds.origin;
        let mut line_start_ix = 0;
        for line in &element_state.lines {
            let line_bottom = line_origin.y + line.size(line_height).height;
            if position.y > line_bottom {
                line_origin.y = line_bottom;
                line_start_ix += line.len() + 1;
            } else {
                let position_within_line = position - line_origin;
                match line.index_for_position(position_within_line, line_height) {
                    Ok(index_within_line) => return Ok(line_start_ix + index_within_line),
                    Err(index_within_line) => return Err(line_start_ix + index_within_line),
                }
            }
        }

        Err(line_start_ix.saturating_sub(1))
    }

    pub fn position_for_index(&self, index: usize) -> Option<Point<Pixels>> {
        let element_state = self.0.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let bounds = element_state
            .bounds
            .expect("prepaint has not been performed");
        let line_height = element_state.line_height;

        let mut line_origin = bounds.origin;
        let mut line_start_ix = 0;

        for line in &element_state.lines {
            let line_end_ix = line_start_ix + line.len();
            if index < line_start_ix {
                break;
            } else if index > line_end_ix {
                line_origin.y += line.size(line_height).height;
                line_start_ix = line_end_ix + 1;
                continue;
            } else {
                let ix_within_line = index - line_start_ix;
                return Some(line_origin + line.position_for_index(ix_within_line, line_height)?);
            }
        }

        None
    }

    pub fn line_layout_for_index(&self, index: usize) -> Option<Arc<WrappedLineLayout>> {
        let element_state = self.0.borrow();
        let element_state = element_state
            .as_ref()
            .expect("measurement has not been performed");
        let mut line_start_ix = 0;

        for line in &element_state.lines {
            let line_end_ix = line_start_ix + line.len();
            if index < line_start_ix {
                break;
            } else if index > line_end_ix {
                line_start_ix = line_end_ix + 1;
                continue;
            } else {
                return Some(line.layout.clone());
            }
        }

        None
    }

    pub fn line_layouts(&self) -> SmallVec<[Arc<WrappedLineLayout>; 1]> {
        self.0
            .borrow()
            .as_ref()
            .expect("measurement has not been performed")
            .lines
            .iter()
            .map(|line| line.layout.clone())
            .collect()
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().as_ref().unwrap().bounds.unwrap()
    }

    pub fn line_height(&self) -> Pixels {
        self.0.borrow().as_ref().unwrap().line_height
    }

    pub fn len(&self) -> usize {
        self.0.borrow().as_ref().unwrap().len
    }

    pub fn text(&self) -> String {
        self.0
            .borrow()
            .as_ref()
            .unwrap()
            .lines
            .iter()
            .map(|s| &s.text)
            .join("\n")
    }

    pub fn wrapped_text(&self) -> String {
        let mut accumulator = String::new();

        for wrapped in self.0.borrow().as_ref().unwrap().lines.iter() {
            let mut seen = 0;
            for boundary in wrapped.layout.wrap_boundaries.iter() {
                let index = wrapped.layout.unwrapped_layout.runs[boundary.run_ix].glyphs
                    [boundary.glyph_ix]
                    .index;

                accumulator.push_str(&wrapped.text[seen..index]);
                accumulator.push('\n');
                seen = index;
            }
            accumulator.push_str(&wrapped.text[seen..]);
            accumulator.push('\n');
        }
        accumulator.pop();
        accumulator
    }
}

pub struct InteractiveText {
    element_id: ElementId,
    text: StyledText,
    click_listener:
        Option<Box<dyn Fn(&[Range<usize>], InteractiveTextClickEvent, &mut Window, &mut App)>>,
    hover_listener: Option<Box<dyn Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App)>>,
    tooltip_builder: Option<Rc<dyn Fn(usize, &mut Window, &mut App) -> Option<AnyView>>>,
    tooltip_id: Option<TooltipId>,
    clickable_ranges: Vec<Range<usize>>,
}

struct InteractiveTextClickEvent {
    mouse_down_index: usize,
    mouse_up_index: usize,
}

#[doc(hidden)]
#[derive(Default)]
pub struct InteractiveTextState {
    mouse_down_index: Rc<Cell<Option<usize>>>,
    hovered_index: Rc<Cell<Option<usize>>>,
    active_tooltip: Rc<RefCell<Option<ActiveTooltip>>>,
    long_press_tooltip_active: Rc<Cell<bool>>,
}

impl InteractiveText {
    pub fn new(id: impl Into<ElementId>, text: StyledText) -> Self {
        Self {
            element_id: id.into(),
            text,
            click_listener: None,
            hover_listener: None,
            tooltip_builder: None,
            tooltip_id: None,
            clickable_ranges: Vec::new(),
        }
    }

    pub fn on_click(
        mut self,
        ranges: Vec<Range<usize>>,
        listener: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.click_listener = Some(Box::new(move |ranges, event, window, cx| {
            for (range_ix, range) in ranges.iter().enumerate() {
                if range.contains(&event.mouse_down_index) && range.contains(&event.mouse_up_index)
                {
                    listener(range_ix, window, cx);
                }
            }
        }));
        self.clickable_ranges = ranges;
        self
    }

    pub fn on_hover(
        mut self,
        listener: impl Fn(Option<usize>, MouseMoveEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.hover_listener = Some(Box::new(listener));
        self
    }

    pub fn tooltip(
        mut self,
        builder: impl Fn(usize, &mut Window, &mut App) -> Option<AnyView> + 'static,
    ) -> Self {
        self.tooltip_builder = Some(Rc::new(builder));
        self
    }
}

impl Element for InteractiveText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        Some(self.element_id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Label)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.text.to_string());
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        window.with_optional_element_state::<InteractiveTextState, _>(
            global_id,
            |interactive_state, window| {
                let mut interactive_state = interactive_state
                    .map(|interactive_state| interactive_state.unwrap_or_default());

                if let Some(interactive_state) = interactive_state.as_mut() {
                    if self.tooltip_builder.is_some() {
                        self.tooltip_id =
                            set_tooltip_on_window(&interactive_state.active_tooltip, window);
                    } else {
                        interactive_state.long_press_tooltip_active.set(false);
                        interactive_state.active_tooltip.take();
                    }
                }

                self.text
                    .prepaint(None, inspector_id, bounds, state, window, cx);
                let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                (hitbox, interactive_state)
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let text_layout = self.text.layout().clone();
        window.with_element_state::<InteractiveTextState, _>(
            global_id.unwrap(),
            |interactive_state, window| {
                let mut interactive_state = interactive_state.unwrap_or_default();
                if let Some(click_listener) = self.click_listener.take() {
                    let mouse_position = window.mouse_position();
                    if let Ok(ix) = text_layout.index_for_position(mouse_position)
                        && self
                            .clickable_ranges
                            .iter()
                            .any(|range| range.contains(&ix))
                    {
                        window.set_cursor_style(crate::CursorStyle::PointingHand, hitbox)
                    }

                    let text_layout = text_layout.clone();
                    let mouse_down = interactive_state.mouse_down_index.clone();
                    if let Some(mouse_down_index) = mouse_down.get() {
                        let hitbox = hitbox.clone();
                        let clickable_ranges = mem::take(&mut self.clickable_ranges);
                        window.on_mouse_event(
                            move |event: &MouseUpEvent, phase, window: &mut Window, cx| {
                                if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                                    if let Ok(mouse_up_index) =
                                        text_layout.index_for_position(event.position)
                                    {
                                        click_listener(
                                            &clickable_ranges,
                                            InteractiveTextClickEvent {
                                                mouse_down_index,
                                                mouse_up_index,
                                            },
                                            window,
                                            cx,
                                        )
                                    }

                                    mouse_down.take();
                                    window.invalidate_view(current_view);
                                }
                            },
                        );
                    } else {
                        let hitbox = hitbox.clone();
                        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                            if phase == DispatchPhase::Bubble
                                && hitbox.is_hovered(window)
                                && let Ok(mouse_down_index) =
                                    text_layout.index_for_position(event.position)
                            {
                                mouse_down.set(Some(mouse_down_index));
                                window.invalidate_view(current_view);
                            }
                        });
                    }
                }

                window.on_mouse_event({
                    let mut hover_listener = self.hover_listener.take();
                    let hitbox = hitbox.clone();
                    let text_layout = text_layout.clone();
                    let hovered_index = interactive_state.hovered_index.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                            let current = hovered_index.get();
                            let updated = text_layout.index_for_position(event.position).ok();
                            if current != updated {
                                hovered_index.set(updated);
                                if let Some(hover_listener) = hover_listener.as_ref() {
                                    hover_listener(updated, event.clone(), window, cx);
                                }
                                cx.notify(current_view);
                            }
                        }
                    }
                });

                if let Some(tooltip_builder) = self.tooltip_builder.clone() {
                    let active_tooltip = interactive_state.active_tooltip.clone();
                    let build_tooltip = Rc::new({
                        let tooltip_is_hoverable = false;
                        let text_layout = text_layout.clone();
                        move |window: &mut Window, cx: &mut App| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .ok()
                                .and_then(|position| tooltip_builder(position, window, cx))
                                .map(|view| (view, tooltip_is_hoverable))
                        }
                    });

                    let check_is_hovered_during_prepaint = Rc::new({
                        let source_bounds = hitbox.bounds;
                        let text_layout = text_layout.clone();
                        let pending_mouse_down = interactive_state.mouse_down_index.clone();
                        move |window: &Window| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .is_ok()
                                && source_bounds.contains(&window.mouse_position())
                                && pending_mouse_down.get().is_none()
                        }
                    });

                    let check_is_hovered = Rc::new({
                        let hitbox = hitbox.clone();
                        let text_layout = text_layout.clone();
                        let pending_mouse_down = interactive_state.mouse_down_index.clone();
                        move |window: &Window| {
                            text_layout
                                .index_for_position(window.mouse_position())
                                .is_ok()
                                && hitbox.is_hovered(window)
                                && pending_mouse_down.get().is_none()
                        }
                    });

                    register_tooltip_mouse_handlers(
                        &active_tooltip,
                        self.tooltip_id,
                        build_tooltip,
                        check_is_hovered,
                        check_is_hovered_during_prepaint,
                        interactive_state.long_press_tooltip_active.clone(),
                        None,
                        window,
                    );
                }

                self.text
                    .paint(None, inspector_id, bounds, &mut (), &mut (), window, cx);

                ((), interactive_state)
            },
        );
    }
}

impl IntoElement for InteractiveText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

#[derive(Default)]
pub struct PartTooltip {
    active: Rc<RefCell<Option<ActiveTooltip>>>,
    long_press: Rc<Cell<bool>>,
}

impl PartTooltip {
    pub fn prepaint(&self, window: &mut Window) -> Option<TooltipId> {
        set_tooltip_on_window(&self.active, window)
    }

    pub fn paint(
        &self,
        id: Option<TooltipId>,
        hitbox: &Hitbox,
        over: Rc<dyn Fn(Point<Pixels>) -> bool>,
        build: Rc<dyn Fn(&mut Window, &mut App) -> Option<AnyView>>,
        window: &mut Window,
    ) {
        let build_tooltip = Rc::new(move |window: &mut Window, cx: &mut App| {
            build(window, cx).map(|view| (view, false))
        });
        let check_is_hovered_during_prepaint = Rc::new({
            let bounds = hitbox.bounds;
            let over = over.clone();
            move |window: &Window| {
                let position = window.mouse_position();
                bounds.contains(&position) && over(position)
            }
        });
        let check_is_hovered = Rc::new({
            let hitbox = hitbox.clone();
            move |window: &Window| hitbox.is_hovered(window) && over(window.mouse_position())
        });
        register_tooltip_mouse_handlers(
            &self.active,
            id,
            build_tooltip,
            check_is_hovered,
            check_is_hovered_during_prepaint,
            self.long_press.clone(),
            None,
            window,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_into_element_for() {
        use crate::{ParentElement as _, SharedString, div};
        use std::borrow::Cow;

        let _ = div().child("static str");
        let _ = div().child("String".to_string());
        let _ = div().child(Cow::Borrowed("Cow"));
        let _ = div().child(SharedString::from("SharedString"));
    }

    #[test]
    fn text_macro_id() {
        fn make_text_stable_id(happy: bool) -> Text {
            text!(if happy { "happy" } else { "sad" })
        }

        fn make_text_unstable_id(happy: bool) -> Text {
            if happy { text!("happy") } else { text!("sad") }
        }

        assert_eq!(make_text_stable_id(false).id, make_text_stable_id(true).id);
        assert_ne!(
            make_text_unstable_id(false).id,
            make_text_unstable_id(true).id
        );
    }
}

#[cfg(test)]
mod measurement_regression_tests {
    use super::TextLayout;
    use crate::{
        App, AppContext as _, AvailableSpace, DrawPhase, Empty, Pixels, Size, TestAppContext,
        TextOverflow, TextStyleRefinement, WhiteSpace, Window, px, size,
    };

    const TEXT: &str = "alpha beta gamma";

    fn text_style(
        white_space: WhiteSpace,
        text_overflow: Option<TextOverflow>,
        line_clamp: Option<usize>,
    ) -> TextStyleRefinement {
        TextStyleRefinement {
            font_size: Some(px(10.).into()),
            line_height: Some(px(10.).into()),
            white_space: Some(white_space),
            text_overflow,
            line_clamp,
            ..Default::default()
        }
    }

    fn measure_probe(
        text_layout: &TextLayout,
        style: TextStyleRefinement,
        available_width: AvailableSpace,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        let layout_id = window.with_text_style(Some(style), |window| {
            text_layout.layout(TEXT.into(), None, window, cx)
        });
        window.compute_layout(
            layout_id,
            size(available_width, AvailableSpace::MaxContent),
            cx,
        );
        let outer_size = window.layout_bounds(layout_id).size;
        let measured_size = text_layout
            .0
            .borrow()
            .as_ref()
            .and_then(|layout| layout.size)
            .expect("measurement should record its size");
        println!(
            "text measurement: offered={available_width:?} cached={measured_size:?} outer={outer_size:?}"
        );
        measured_size
    }

    #[ala::test]
    fn text_measurement_fit_width_preserves_geometry(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Empty);
        cx.update_window(window.into(), |_, window, cx| {
            window.invalidator.set_phase(DrawPhase::Prepaint);
            let layout = TextLayout::default();
            let intrinsic = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::MaxContent,
                window,
                cx,
            );
            assert_eq!(intrinsic, size(px(96.), px(10.)));
            assert_eq!(layout.wrapped_text(), TEXT);

            let wider = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::Definite(px(120.)),
                window,
                cx,
            );
            assert_eq!(wider, intrinsic);
            assert_eq!(layout.wrapped_text(), TEXT);

            let narrow = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::Definite(px(36.)),
                window,
                cx,
            );
            assert_eq!(narrow.width, px(36.));
            assert!(narrow.height > intrinsic.height);
            assert!(layout.wrapped_text().contains('\n'));

            let wider_again = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::Definite(px(120.)),
                window,
                cx,
            );
            assert_eq!(wider_again, intrinsic);
            assert_eq!(layout.wrapped_text(), TEXT);
            window.invalidator.set_phase(DrawPhase::None);
        })
        .unwrap();
    }

    #[ala::test]
    fn text_measurement_truncation_preserves_intrinsic_geometry(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Empty);
        cx.update_window(window.into(), |_, window, cx| {
            window.invalidator.set_phase(DrawPhase::Prepaint);
            for (white_space, overflow, line_clamp) in [
                (WhiteSpace::Nowrap, TextOverflow::Truncate("…".into()), None),
                (
                    WhiteSpace::Nowrap,
                    TextOverflow::TruncateStart("…".into()),
                    None,
                ),
                (
                    WhiteSpace::Nowrap,
                    TextOverflow::TruncateMiddle("…".into()),
                    None,
                ),
                (
                    WhiteSpace::Normal,
                    TextOverflow::Truncate("…".into()),
                    Some(1),
                ),
            ] {
                let layout = TextLayout::default();
                let style = || text_style(white_space, Some(overflow.clone()), line_clamp);
                let intrinsic =
                    measure_probe(&layout, style(), AvailableSpace::MaxContent, window, cx);
                assert_eq!(intrinsic, size(px(96.), px(10.)));
                assert_eq!(layout.text(), TEXT);

                let narrow = measure_probe(
                    &layout,
                    style(),
                    AvailableSpace::Definite(px(36.)),
                    window,
                    cx,
                );
                assert!(narrow.width <= px(36.));
                assert_eq!(narrow.height, px(10.));
                assert_ne!(layout.text(), TEXT);
                assert!(layout.text().contains('…'));

                let restored =
                    measure_probe(&layout, style(), AvailableSpace::MaxContent, window, cx);
                assert_eq!(restored, intrinsic);
                assert_eq!(layout.text(), TEXT);
                assert_eq!(layout.wrapped_text(), TEXT);

                let wider = measure_probe(
                    &layout,
                    style(),
                    AvailableSpace::Definite(px(120.)),
                    window,
                    cx,
                );
                assert_eq!(wider, intrinsic);
                assert_eq!(layout.text(), TEXT);
            }
            window.invalidator.set_phase(DrawPhase::None);
        })
        .unwrap();
    }

    #[ala::test]
    fn text_measurement_wrapped_probe_restores_intrinsic_geometry(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Empty);
        cx.update_window(window.into(), |_, window, cx| {
            window.invalidator.set_phase(DrawPhase::Prepaint);
            let layout = TextLayout::default();
            let narrow = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::Definite(px(36.)),
                window,
                cx,
            );
            assert_eq!(narrow.width, px(36.));
            assert!(narrow.height > px(10.));
            assert!(layout.wrapped_text().contains('\n'));

            let intrinsic = measure_probe(
                &layout,
                text_style(WhiteSpace::Normal, None, None),
                AvailableSpace::MaxContent,
                window,
                cx,
            );
            assert_eq!(intrinsic, size(px(96.), px(10.)));
            assert_eq!(layout.wrapped_text(), TEXT);
            window.invalidator.set_phase(DrawPhase::None);
        })
        .unwrap();
    }
}

#[cfg(test)]
mod a11y_tests {
    use crate::{
        AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
        StatefulInteractiveElement as _, Styled as _, TestAppContext, Window, div, px,
    };

    struct Labels;

    impl Render for Labels {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("group")
                .role(accesskit::Role::Group)
                .w(px(200.))
                .child("first")
                .child(div().child("second"))
                .child("  ")
        }
    }

    #[crate::test]
    fn text_without_an_id_becomes_a_stable_label_leaf(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, _| Labels);
        let labels_in_frame = |cx: &mut TestAppContext| {
            cx.update_window(window.into(), |_, window, cx| {
                window.set_a11y_forced(true);
                window.refresh();
                window.draw(cx).clear(cx);
                let tree = window.a11y_tree().expect("forced a11y builds a tree");
                let (group_id, group) = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == accesskit::Role::Group)
                    .expect("the group is in the tree");
                let labels = group
                    .children()
                    .iter()
                    .filter_map(|child| tree.nodes.iter().find(|(id, _)| id == child))
                    .map(|(id, node)| {
                        assert_eq!(node.role(), accesskit::Role::Label);
                        assert!(node.bounds().is_some());
                        (*id, node.value().unwrap_or_default().to_owned())
                    })
                    .collect::<Vec<_>>();
                (*group_id, labels)
            })
            .unwrap()
        };

        let (group, first) = labels_in_frame(cx);
        let values = first
            .iter()
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, ["first", "second"]);
        let (same_group, second) = labels_in_frame(cx);
        assert_eq!(group, same_group);
        assert_eq!(first, second);
    }
}
