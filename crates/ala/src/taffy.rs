use crate::collections::{FxHashMap, FxHashSet};
use crate::{
    AbsoluteLength, App, Bounds, DefiniteLength, Edges, GridTemplate, Length, Pixels, Point, Size,
    Style, Window, size,
    util::{
        ceil_to_device_pixel, round_half_toward_zero, round_stroke_to_device_pixel,
        round_to_device_pixel,
    },
};
use smallvec::SmallVec;
use std::{
    any::Any,
    fmt::Debug,
    hash::{Hash as _, Hasher as _},
    mem,
    ops::Range,
    rc::Rc,
};
use taffy::{
    TaffyTree, TraversePartialTree as _,
    geometry::{Point as TaffyPoint, Rect as TaffyRect, Size as TaffySize},
    prelude::{max_content, min_content},
    style::AvailableSpace as TaffyAvailableSpace,
    tree::NodeId,
};

#[cfg(feature = "stacker")]
type StackSafe<T> = stacksafe::StackSafe<T>;
#[cfg(not(feature = "stacker"))]
type StackSafe<T> = T;

type MeasureFn =
    dyn FnMut(Size<Option<Pixels>>, Size<AvailableSpace>, &mut Window, &mut App) -> Size<Pixels>;
type NodeMeasureFn = StackSafe<Box<MeasureFn>>;

struct NodeContext {
    measure: NodeMeasureFn,
    state: Option<Rc<dyn Any>>,
}

pub struct TaffyLayoutEngine {
    taffy: TaffyTree<NodeContext>,
    absolute_layout_bounds: FxHashMap<LayoutId, Bounds<Pixels>>,
    absolute_outer_origins: FxHashMap<LayoutId, Point<f32>>,
    computed_layouts: FxHashSet<LayoutId>,
    layout_bounds_scratch_space: Vec<LayoutId>,
    retained: RetainedNodes,
}

#[derive(Default)]
struct RetainedNodes {
    candidates: FxHashMap<u64, SmallVec<[NodeId; 1]>>,
    next_candidates: FxHashMap<u64, SmallVec<[NodeId; 1]>>,
    keys: FxHashMap<NodeId, u64>,
    used: FxHashSet<NodeId>,
    frame: Vec<NodeId>,
    last_frame: Vec<NodeId>,
}

impl RetainedNodes {
    fn key(
        &self,
        style: &taffy::Style,
        children: &[LayoutId],
        measured: Option<u64>,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> Option<u64> {
        let mut hasher = crate::collections::FxHasher::default();
        rem_size.0.to_bits().hash(&mut hasher);
        scale_factor.to_bits().hash(&mut hasher);
        style_fingerprint(style, &mut hasher);
        measured.hash(&mut hasher);
        for child in children {
            self.keys.get(&child.0)?.hash(&mut hasher);
        }
        Some(hasher.finish())
    }

    fn record(&mut self, node: NodeId, key: Option<u64>) {
        self.frame.push(node);
        self.used.insert(node);
        if let Some(key) = key {
            self.keys.insert(node, key);
            self.next_candidates.entry(key).or_default().push(node);
        }
    }
}

fn style_fingerprint(style: &taffy::Style, hasher: &mut crate::collections::FxHasher) {
    mem::discriminant(&style.display).hash(hasher);
    mem::discriminant(&style.position).hash(hasher);
    mem::discriminant(&style.flex_direction).hash(hasher);
    for dimension in [style.size.width, style.size.height] {
        dimension.tag().hash(hasher);
        dimension.value().to_bits().hash(hasher);
    }
    style.flex_grow.to_bits().hash(hasher);
    style.flex_shrink.to_bits().hash(hasher);
}

const EXPECT_MESSAGE: &str = "we should avoid taffy layout errors by construction if possible";

impl TaffyLayoutEngine {
    pub fn new() -> Self {
        let mut taffy = TaffyTree::new();
        taffy.disable_rounding();
        TaffyLayoutEngine {
            taffy,
            absolute_layout_bounds: FxHashMap::default(),
            absolute_outer_origins: FxHashMap::default(),
            computed_layouts: FxHashSet::default(),
            layout_bounds_scratch_space: Vec::new(),
            retained: RetainedNodes::default(),
        }
    }

    pub fn clear(&mut self) {
        let retained = &mut self.retained;
        for node in retained.last_frame.drain(..) {
            if !retained.used.contains(&node) {
                self.taffy.remove(node).expect(EXPECT_MESSAGE);
            }
        }
        mem::swap(&mut retained.last_frame, &mut retained.frame);
        retained.candidates = mem::take(&mut retained.next_candidates);
        retained.keys.clear();
        retained.used.clear();
        self.absolute_layout_bounds.clear();
        self.absolute_outer_origins.clear();
        self.computed_layouts.clear();
    }

    fn take_candidate(
        &mut self,
        key: u64,
        style: &taffy::Style,
        children: &[LayoutId],
        measured: bool,
    ) -> Option<NodeId> {
        let taffy = &self.taffy;
        let retained = &mut self.retained;
        let candidates = retained.candidates.get_mut(&key)?;
        let position = candidates.iter().position(|&node| {
            !retained.used.contains(&node)
                && taffy.get_node_context(node).is_some() == measured
                && taffy.style(node).is_ok_and(|old| old == style)
                && taffy
                    .children(node)
                    .is_ok_and(|old| old.as_slice() == LayoutId::to_taffy_slice(children))
        })?;
        Some(candidates.remove(position))
    }

    fn adopt(&mut self, children: &[LayoutId]) {
        for child in children {
            if let Some(parent) = self.taffy.parent(child.0) {
                self.taffy
                    .remove_child(parent, child.0)
                    .expect(EXPECT_MESSAGE);
            }
        }
    }

    pub fn request_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        children: &[LayoutId],
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        let key = self
            .retained
            .key(&taffy_style, children, None, rem_size, scale_factor);
        if let Some(key) = key
            && let Some(node) = self.take_candidate(key, &taffy_style, children, false)
        {
            self.retained.record(node, Some(key));
            return node.into();
        }

        let node = if children.is_empty() {
            self.taffy.new_leaf(taffy_style).expect(EXPECT_MESSAGE)
        } else {
            self.adopt(children);
            self.taffy
                .new_with_children(taffy_style, LayoutId::to_taffy_slice(children))
                .expect(EXPECT_MESSAGE)
        };
        self.retained.record(node, key);
        node.into()
    }

    pub fn request_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        measure: impl FnMut(
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &mut App,
        ) -> Size<Pixels>
        + 'static,
    ) -> LayoutId {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        let measure = Box::new(measure) as Box<MeasureFn>;
        #[cfg(feature = "stacker")]
        let measure = StackSafe::new(measure);

        let node = self
            .taffy
            .new_leaf_with_context(
                taffy_style,
                NodeContext {
                    measure,
                    state: None,
                },
            )
            .expect(EXPECT_MESSAGE);
        self.retained.record(node, None);
        node.into()
    }

    pub fn request_keyed_measured_layout(
        &mut self,
        style: Style,
        rem_size: Pixels,
        scale_factor: f32,
        content_key: u64,
        state: Rc<dyn Any>,
        measure: Box<MeasureFn>,
    ) -> (LayoutId, Option<Rc<dyn Any>>) {
        let taffy_style = style.to_taffy(rem_size, scale_factor);
        #[cfg(feature = "stacker")]
        let measure = StackSafe::new(measure);
        let key = self
            .retained
            .key(&taffy_style, &[], Some(content_key), rem_size, scale_factor);
        if let Some(key) = key
            && let Some(node) = self.take_candidate(key, &taffy_style, &[], true)
        {
            let context = self.taffy.get_node_context_mut(node).expect(EXPECT_MESSAGE);
            context.measure = measure;
            let previous = context.state.replace(state);
            self.retained.record(node, Some(key));
            return (node.into(), previous);
        }

        let node = self
            .taffy
            .new_leaf_with_context(
                taffy_style,
                NodeContext {
                    measure,
                    state: Some(state),
                },
            )
            .expect(EXPECT_MESSAGE);
        self.retained.record(node, key);
        (node.into(), None)
    }

    pub fn stretch_auto_size_to_fill(
        &mut self,
        id: LayoutId,
        size: Size<Pixels>,
        scale_factor: f32,
    ) {
        let style = self.taffy.style(id.0).expect(EXPECT_MESSAGE);
        let stretch_width = style.size.width.is_auto();
        let stretch_height = style.size.height.is_auto();
        if !stretch_width && !stretch_height {
            return;
        }
        let mut style = style.clone();
        if stretch_width {
            style.size.width =
                taffy::style::Dimension::length(round_to_device_pixel(size.width.0, scale_factor));
        }
        if stretch_height {
            style.size.height =
                taffy::style::Dimension::length(round_to_device_pixel(size.height.0, scale_factor));
        }
        self.taffy.set_style(id.0, style).expect(EXPECT_MESSAGE);
    }

    #[allow(dead_code)]
    fn count_all_children(&self, parent: LayoutId) -> anyhow::Result<u32> {
        let mut count = 0;

        for child in self.taffy.children(parent.0)? {
            count += 1;

            count += self.count_all_children(LayoutId(child))?
        }

        Ok(count)
    }

    #[allow(dead_code)]
    fn max_depth(&self, depth: u32, parent: LayoutId) -> anyhow::Result<u32> {
        println!(
            "{parent:?} at depth {depth} has {} children",
            self.taffy.child_count(parent.0)
        );

        let mut max_child_depth = 0;

        for child in self.taffy.children(parent.0)? {
            max_child_depth = std::cmp::max(max_child_depth, self.max_depth(0, LayoutId(child))?);
        }

        Ok(depth + 1 + max_child_depth)
    }

    #[allow(dead_code)]
    fn get_edges(&self, parent: LayoutId) -> anyhow::Result<Vec<(LayoutId, LayoutId)>> {
        let mut edges = Vec::new();

        for child in self.taffy.children(parent.0)? {
            edges.push((parent, LayoutId(child)));

            edges.extend(self.get_edges(LayoutId(child))?);
        }

        Ok(edges)
    }

    #[cfg_attr(feature = "stacker", stacksafe::stacksafe)]
    pub fn compute_layout(
        &mut self,
        id: LayoutId,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(parent) = self.taffy.parent(id.0) {
            self.taffy.remove_child(parent, id.0).expect(EXPECT_MESSAGE);
        }
        if !self.computed_layouts.insert(id) {
            let stack = &mut self.layout_bounds_scratch_space;
            stack.push(id);
            while let Some(id) = stack.pop() {
                self.absolute_layout_bounds.remove(&id);
                self.absolute_outer_origins.remove(&id);
                stack.extend(
                    self.taffy
                        .children(id.into())
                        .expect(EXPECT_MESSAGE)
                        .into_iter()
                        .map(LayoutId::from),
                );
            }
        }

        let scale_factor = window.scale_factor();

        let transform = |v: AvailableSpace| match v {
            AvailableSpace::Definite(pixels) => {
                AvailableSpace::Definite(Pixels(pixels.0 * scale_factor))
            }
            AvailableSpace::MinContent => AvailableSpace::MinContent,
            AvailableSpace::MaxContent => AvailableSpace::MaxContent,
        };
        let available_space = size(
            transform(available_space.width),
            transform(available_space.height),
        );

        let out_of_flow_root_style = match (available_space.width, available_space.height) {
            (AvailableSpace::Definite(_), AvailableSpace::Definite(_)) => {
                let style = self.taffy.style(id.0).expect(EXPECT_MESSAGE);
                (style.position == taffy::style::Position::Absolute).then(|| style.clone())
            }
            _ => None,
        };
        if let Some(style) = &out_of_flow_root_style {
            let in_flow_style = taffy::style::Style {
                position: taffy::style::Position::Relative,
                ..style.clone()
            };
            self.taffy
                .set_style(id.0, in_flow_style)
                .expect(EXPECT_MESSAGE);
        }

        self.taffy
            .compute_layout_with_measure(
                id.into(),
                available_space.into(),
                |inputs, _id, node_context, style| {
                    taffy::compute_leaf_layout(
                        inputs,
                        style,
                        |_, _| 0.0,
                        |known_dimensions, available_space| {
                            let Some(node_context) = node_context else {
                                return taffy::geometry::Size::default();
                            };

                            let known_dimensions = Size {
                                width: known_dimensions.width.map(|e| Pixels(e / scale_factor)),
                                height: known_dimensions.height.map(|e| Pixels(e / scale_factor)),
                            };

                            let available_space: Size<AvailableSpace> = available_space.into();
                            let untransform = |ev: AvailableSpace| match ev {
                                AvailableSpace::Definite(pixels) => {
                                    AvailableSpace::Definite(Pixels(pixels.0 / scale_factor))
                                }
                                AvailableSpace::MinContent => AvailableSpace::MinContent,
                                AvailableSpace::MaxContent => AvailableSpace::MaxContent,
                            };
                            let available_space = size(
                                untransform(available_space.width),
                                untransform(available_space.height),
                            );

                            let measured_size: Size<Pixels> = (node_context.measure)(
                                known_dimensions,
                                available_space,
                                window,
                                cx,
                            );
                            snap_measured_size_to_device_pixels(measured_size, scale_factor).into()
                        },
                    )
                },
            )
            .expect(EXPECT_MESSAGE);

        if let Some(style) = out_of_flow_root_style {
            self.taffy.set_style(id.0, style).expect(EXPECT_MESSAGE);
        }
    }

    pub fn layout_bounds(&mut self, id: LayoutId, scale_factor: f32) -> Bounds<Pixels> {
        if let Some(layout) = self.absolute_layout_bounds.get(&id).cloned() {
            return layout;
        }

        let layout = self.taffy.layout(id.into()).expect(EXPECT_MESSAGE);
        let layout_location = layout.location;
        let layout_size = layout.size;
        let parent = self.taffy.parent(id.0);

        let absolute_outer_origin = match parent {
            Some(parent_id) => {
                let parent_id = LayoutId::from(parent_id);
                self.layout_bounds(parent_id, scale_factor);
                let parent_origin = *self
                    .absolute_outer_origins
                    .get(&parent_id)
                    .expect("parent absolute outer origin should be cached");
                parent_origin + Point::from(layout_location)
            }
            None => Point::default(),
        };
        self.absolute_outer_origins
            .insert(id, absolute_outer_origin);

        let absolute_far = absolute_outer_origin + Point::from(Size::from(layout_size));
        let snapped_bounds = Bounds::from_corners(
            absolute_outer_origin.map(round_half_toward_zero),
            absolute_far.map(round_half_toward_zero),
        );

        let bounds = (snapped_bounds / scale_factor).map(Pixels);
        self.absolute_layout_bounds.insert(id, bounds);
        bounds
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
#[repr(transparent)]
pub struct LayoutId(NodeId);

impl LayoutId {
    fn to_taffy_slice(node_ids: &[Self]) -> &[taffy::NodeId] {
        unsafe { std::mem::transmute::<&[LayoutId], &[taffy::NodeId]>(node_ids) }
    }
}

impl std::hash::Hash for LayoutId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        u64::from(self.0).hash(state);
    }
}

impl From<NodeId> for LayoutId {
    fn from(node_id: NodeId) -> Self {
        Self(node_id)
    }
}

impl From<LayoutId> for NodeId {
    fn from(layout_id: LayoutId) -> NodeId {
        layout_id.0
    }
}

fn snap_measured_size_to_device_pixels(size: Size<Pixels>, scale_factor: f32) -> Size<f32> {
    size.map(|d| ceil_to_device_pixel(d.0.max(0.0), scale_factor))
}

fn border_widths_to_taffy(
    widths: &Edges<AbsoluteLength>,
    rem_size: Pixels,
    scale_factor: f32,
) -> TaffyRect<taffy::style::LengthPercentage> {
    let snap = |w: &AbsoluteLength| {
        taffy::style::LengthPercentage::length(round_stroke_to_device_pixel(
            w.to_pixels(rem_size).0,
            scale_factor,
        ))
    };
    TaffyRect {
        top: snap(&widths.top),
        right: snap(&widths.right),
        bottom: snap(&widths.bottom),
        left: snap(&widths.left),
    }
}

trait ToTaffy<Output> {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> Output;
}

impl ToTaffy<taffy::style::Style> for Style {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Style {
        use taffy::style_helpers::{fr, length, minmax, repeat};

        fn to_grid_line(
            placement: &Range<crate::GridPlacement>,
        ) -> taffy::Line<taffy::GridPlacement> {
            taffy::Line {
                start: placement.start.into(),
                end: placement.end.into(),
            }
        }

        fn to_grid_repeat<T: taffy::style::CheapCloneStr>(
            unit: &Option<GridTemplate>,
        ) -> Vec<taffy::GridTemplateComponent<T>> {
            unit.map(|template| match template.min_size {
                crate::GridTemplateMinSize::Zero => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(length(0.0_f32), fr(1.0_f32))],
                    )]
                }
                crate::GridTemplateMinSize::MinContent => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(min_content(), fr(1.0_f32))],
                    )]
                }
                crate::GridTemplateMinSize::MaxContent => {
                    vec![repeat(
                        template.repeat,
                        vec![minmax(length(0.0_f32), max_content())],
                    )]
                }
            })
            .unwrap_or_default()
        }

        taffy::style::Style {
            display: self.display.into(),
            overflow: self.overflow.into(),
            scrollbar_width: self.scrollbar_width.to_taffy(rem_size, scale_factor),
            position: self.position.into(),
            inset: self.inset.to_taffy(rem_size, scale_factor),
            size: self.size.to_taffy(rem_size, scale_factor),
            min_size: self.min_size.to_taffy(rem_size, scale_factor),
            max_size: self.max_size.to_taffy(rem_size, scale_factor),
            aspect_ratio: self.aspect_ratio,
            margin: self.margin.to_taffy(rem_size, scale_factor),
            padding: self.padding.to_taffy(rem_size, scale_factor),
            border: border_widths_to_taffy(&self.border_widths, rem_size, scale_factor),
            align_items: self
                .align_items
                .map_or(taffy::style::AlignItems::NORMAL, |x| x.into()),
            align_self: self.align_self.map(|x| x.into()),
            align_content: self
                .align_content
                .map_or(taffy::style::AlignContent::NORMAL, |x| x.into()),
            justify_content: self
                .justify_content
                .map_or(taffy::style::JustifyContent::NORMAL, |x| x.into()),
            gap: self.gap.to_taffy(rem_size, scale_factor),
            flex_direction: self.flex_direction.into(),
            flex_wrap: self.flex_wrap.into(),
            flex_basis: self.flex_basis.to_taffy(rem_size, scale_factor),
            flex_grow: self.flex_grow,
            flex_shrink: self.flex_shrink,
            grid_template_rows: to_grid_repeat(&self.grid_rows),
            grid_template_columns: to_grid_repeat(&self.grid_cols),
            grid_row: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.row))
                .unwrap_or_default(),
            grid_column: self
                .grid_location
                .as_ref()
                .map(|location| to_grid_line(&location.column))
                .unwrap_or_default(),
            ..Default::default()
        }
    }
}

impl ToTaffy<f32> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> f32 {
        round_to_device_pixel(self.to_pixels(rem_size).0, scale_factor)
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for Length {
    fn to_taffy(
        &self,
        rem_size: Pixels,
        scale_factor: f32,
    ) -> taffy::prelude::LengthPercentageAuto {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::LengthPercentageAuto::auto(),
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for Length {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::prelude::Dimension {
        match self {
            Length::Definite(length) => length.to_taffy(rem_size, scale_factor),
            Length::Auto => taffy::prelude::Dimension::auto(),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentage::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => {
                taffy::style::LengthPercentageAuto::percent(*fraction)
            }
        }
    }
}

impl ToTaffy<taffy::style::Dimension> for DefiniteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        match self {
            DefiniteLength::Absolute(length) => length.to_taffy(rem_size, scale_factor),
            DefiniteLength::Fraction(fraction) => taffy::style::Dimension::percent(*fraction),
        }
    }
}

impl ToTaffy<taffy::style::LengthPercentage> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentage {
        taffy::style::LengthPercentage::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::LengthPercentageAuto> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::LengthPercentageAuto {
        taffy::style::LengthPercentageAuto::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl ToTaffy<taffy::style::Dimension> for AbsoluteLength {
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> taffy::style::Dimension {
        taffy::style::Dimension::length(self.to_taffy(rem_size, scale_factor))
    }
}

impl<T, T2> From<TaffyPoint<T>> for Point<T2>
where
    T: Into<T2>,
    T2: Clone + Debug + Default + PartialEq,
{
    fn from(point: TaffyPoint<T>) -> Point<T2> {
        Point {
            x: point.x.into(),
            y: point.y.into(),
        }
    }
}

impl<T, T2> From<Point<T>> for TaffyPoint<T2>
where
    T: Into<T2> + Clone + Debug + Default + PartialEq,
{
    fn from(val: Point<T>) -> Self {
        TaffyPoint {
            x: val.x.into(),
            y: val.y.into(),
        }
    }
}

impl<T, U> ToTaffy<TaffySize<U>> for Size<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffySize<U> {
        TaffySize {
            width: self.width.to_taffy(rem_size, scale_factor),
            height: self.height.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> ToTaffy<TaffyRect<U>> for Edges<T>
where
    T: ToTaffy<U> + Clone + Debug + Default + PartialEq,
{
    fn to_taffy(&self, rem_size: Pixels, scale_factor: f32) -> TaffyRect<U> {
        TaffyRect {
            top: self.top.to_taffy(rem_size, scale_factor),
            right: self.right.to_taffy(rem_size, scale_factor),
            bottom: self.bottom.to_taffy(rem_size, scale_factor),
            left: self.left.to_taffy(rem_size, scale_factor),
        }
    }
}

impl<T, U> From<TaffySize<T>> for Size<U>
where
    T: Into<U>,
    U: Clone + Debug + Default + PartialEq,
{
    fn from(taffy_size: TaffySize<T>) -> Self {
        Size {
            width: taffy_size.width.into(),
            height: taffy_size.height.into(),
        }
    }
}

impl<T, U> From<Size<T>> for TaffySize<U>
where
    T: Into<U> + Clone + Debug + Default + PartialEq,
{
    fn from(size: Size<T>) -> Self {
        TaffySize {
            width: size.width.into(),
            height: size.height.into(),
        }
    }
}

#[derive(Copy, Clone, Default, Debug, Eq, PartialEq)]
pub enum AvailableSpace {
    Definite(Pixels),
    #[default]
    MinContent,
    MaxContent,
}

impl AvailableSpace {
    pub const fn min_size() -> Size<Self> {
        Size {
            width: Self::MinContent,
            height: Self::MinContent,
        }
    }
}

impl From<AvailableSpace> for TaffyAvailableSpace {
    fn from(space: AvailableSpace) -> TaffyAvailableSpace {
        match space {
            AvailableSpace::Definite(Pixels(value)) => TaffyAvailableSpace::Definite(value),
            AvailableSpace::MinContent => TaffyAvailableSpace::MinContent,
            AvailableSpace::MaxContent => TaffyAvailableSpace::MaxContent,
        }
    }
}

impl From<TaffyAvailableSpace> for AvailableSpace {
    fn from(space: TaffyAvailableSpace) -> AvailableSpace {
        match space {
            TaffyAvailableSpace::Definite(value) => AvailableSpace::Definite(Pixels(value)),
            TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
            TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent,
        }
    }
}

impl From<Pixels> for AvailableSpace {
    fn from(pixels: Pixels) -> Self {
        AvailableSpace::Definite(pixels)
    }
}

impl From<Size<Pixels>> for Size<AvailableSpace> {
    fn from(size: Size<Pixels>) -> Self {
        Size {
            width: AvailableSpace::Definite(size.width),
            height: AvailableSpace::Definite(size.height),
        }
    }
}

impl TaffyLayoutEngine {
    pub(crate) fn frame_stats_layout_node_count(&self) -> usize {
        self.retained.frame.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_widths_to_taffy_use_stroke_snapping() {
        let border_widths = Edges {
            top: Pixels(0.0).into(),
            right: Pixels(0.4).into(),
            bottom: Pixels(0.5).into(),
            left: Pixels(1.6).into(),
        };
        let taffy_border = border_widths_to_taffy(&border_widths, Pixels(16.0), 1.0);

        assert_eq!(
            taffy_border.top,
            taffy::style::LengthPercentage::length(0.0)
        );
        assert_eq!(
            taffy_border.right,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.bottom,
            taffy::style::LengthPercentage::length(1.0)
        );
        assert_eq!(
            taffy_border.left,
            taffy::style::LengthPercentage::length(2.0)
        );
    }

    #[crate::test]
    fn root_layout_ignores_its_own_margin_and_position(cx: &mut crate::TestAppContext) {
        use crate::{
            Bounds, IntoElement as _, ParentElement as _, Styled as _, canvas, div, point, px, size,
        };
        use std::{cell::Cell, rc::Rc};

        let cx = cx.add_empty_window();
        let roots: [fn() -> crate::Div; 4] = [
            || div().m(px(3.)).size(px(20.)),
            || div().m(px(3.)).w_full(),
            || div().absolute().left(px(5.)).top(px(7.)).right(px(5.)),
            || div().absolute().inset_0().p(px(2.)),
        ];
        let mut laid_out = Vec::new();
        for root in roots {
            let bounds = Rc::new(Cell::new(Bounds::default()));
            let sink = bounds.clone();
            cx.draw(
                point(px(0.), px(0.)),
                size(px(100.), px(50.)),
                move |_, _| {
                    root()
                        .child(
                            canvas(move |bounds, _, _| sink.set(bounds), |_, _, _, _| {})
                                .w_full()
                                .h(px(10.)),
                        )
                        .into_any_element()
                },
            );
            laid_out.push(bounds.get());
        }
        assert_eq!(
            laid_out,
            [
                Bounds::new(point(px(0.), px(0.)), size(px(20.), px(10.))),
                Bounds::new(point(px(0.), px(0.)), size(px(100.), px(10.))),
                Bounds::new(point(px(0.), px(0.)), size(px(100.), px(10.))),
                Bounds::new(point(px(2.), px(2.)), size(px(96.), px(10.))),
            ]
        );
    }
}
