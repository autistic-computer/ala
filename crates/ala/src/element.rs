#[cfg(any(feature = "inspector", debug_assertions))]
use crate::InspectorElementPath;
use crate::collections::FxHashMap;
use crate::{
    A11ySubtreeBuilder, App, ArenaBox, AvailableSpace, Bounds, Context, DispatchNodeId, ElementId,
    FocusHandle, InspectorElementId, LayoutId, Pixels, Point, SharedString, Size, Style, Window,
    util::FluentBuilder, window::with_element_arena,
};
use derive_more::Deref;
use std::{
    any::Any,
    fmt::{self, Debug, Display},
    mem,
    ops::DerefMut,
    panic,
    sync::Arc,
};

pub trait Element: 'static + IntoElement {
    type RequestLayoutState: 'static;

    type PrepaintState: 'static;

    fn id(&self) -> Option<ElementId>;

    fn source_location(&self) -> Option<&'static panic::Location<'static>>;

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState);

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState;

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    );

    fn a11y_role(&self) -> Option<accesskit::Role> {
        None
    }

    fn write_a11y_info(&self, _node: &mut accesskit::Node) {}

    fn a11y_text(&self) -> Option<SharedString> {
        None
    }

    fn a11y_synthetic_children(
        &mut self,
        _prepaint: &mut Self::PrepaintState,
        _builder: &mut A11ySubtreeBuilder,
    ) {
    }

    fn into_any(self) -> AnyElement {
        AnyElement::new(self)
    }
}

pub trait IntoElement: Sized {
    type Element: Element;

    fn into_element(self) -> Self::Element;

    fn into_any_element(self) -> AnyElement {
        self.into_element().into_any()
    }
}

impl<T: IntoElement> FluentBuilder for T {}

pub trait Render: 'static + Sized {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement;
}

impl Render for Empty {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub trait RenderOnce: 'static {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement;
}

pub trait ParentElement {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>);

    fn child(mut self, child: impl IntoElement) -> Self
    where
        Self: Sized,
    {
        self.extend(std::iter::once(child.into_any_element()));
        self
    }

    fn children(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self
    where
        Self: Sized,
    {
        self.extend(children.into_iter().map(|child| child.into_any_element()));
        self
    }
}

#[derive(Deref, Clone, Debug)]
pub struct GlobalElementId(#[deref] pub(crate) Arc<[ElementId]>, u64);

impl GlobalElementId {
    pub(crate) fn new(path: Arc<[ElementId]>) -> Self {
        let hash = Self::hash_path(&path);
        Self(path, hash)
    }

    fn hash_path(path: &[ElementId]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = crate::collections::FxHasher::default();
        path.hash(&mut hasher);
        hasher.finish()
    }

    fn path_hash(&self) -> u64 {
        if self.1 == 0 {
            Self::hash_path(&self.0)
        } else {
            self.1
        }
    }
}

impl Default for GlobalElementId {
    fn default() -> Self {
        Self::new(Arc::from([]))
    }
}

impl DerefMut for GlobalElementId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.1 = 0;
        &mut self.0
    }
}

impl PartialEq for GlobalElementId {
    fn eq(&self, other: &Self) -> bool {
        self.path_hash() == other.path_hash() && self.0 == other.0
    }
}

impl Eq for GlobalElementId {}

impl std::hash::Hash for GlobalElementId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.path_hash());
    }
}

#[derive(Default)]
pub(crate) struct GlobalIdCache {
    previous: FxHashMap<u64, GlobalElementId>,
    current: FxHashMap<u64, GlobalElementId>,
}

impl GlobalIdCache {
    fn path_hash(path: &[ElementId]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = crate::collections::FxHasher::default();
        path.hash(&mut hasher);
        hasher.finish()
    }

    pub(crate) fn get(&mut self, path: &[ElementId]) -> GlobalElementId {
        let hash = Self::path_hash(path);
        if let Some(id) = self.current.get(&hash)
            && *id.0 == *path
        {
            return id.clone();
        }
        let id = match self.previous.get(&hash) {
            Some(id) if *id.0 == *path => id.clone(),
            _ => GlobalElementId::new(Arc::from(path)),
        };
        self.current.insert(hash, id.clone());
        id
    }

    pub(crate) fn finish_frame(&mut self) {
        mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }
}

impl Display for GlobalElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, element_id) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ".")?;
            }
            write!(f, "{}", element_id)?;
        }
        Ok(())
    }
}

impl GlobalElementId {
    pub(crate) fn accesskit_node_id(&self) -> accesskit::NodeId {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::default();
        self.0.hash(&mut hasher);
        accesskit::NodeId(hasher.finish())
    }
}

trait ElementObject {
    fn inner_element(&mut self) -> &mut dyn Any;

    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId;

    fn prepaint(&mut self, window: &mut Window, cx: &mut App);

    fn paint(&mut self, window: &mut Window, cx: &mut App);

    fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels>;
}

pub struct Drawable<E: Element> {
    pub element: E,
    phase: ElementDrawPhase<E::RequestLayoutState, E::PrepaintState>,
}

#[derive(Default)]
enum ElementDrawPhase<RequestLayoutState, PrepaintState> {
    #[default]
    Start,
    RequestLayout {
        layout_id: LayoutId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        request_layout: RequestLayoutState,
    },
    LayoutComputed {
        layout_id: LayoutId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        available_space: Size<AvailableSpace>,
        request_layout: RequestLayoutState,
    },
    Prepaint {
        node_id: DispatchNodeId,
        global_id: Option<GlobalElementId>,
        inspector_id: Option<InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: RequestLayoutState,
        prepaint: PrepaintState,
    },
    Painted,
}

impl<E: Element> Drawable<E> {
    pub(crate) fn new(element: E) -> Self {
        Drawable {
            element,
            phase: ElementDrawPhase::Start,
        }
    }

    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        let _frame_stats =
            crate::frame_stats::element_phase(crate::frame_stats::Phase::RequestLayout);
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Start => {
                let global_id = self
                    .element
                    .id()
                    .map(|element_id| prepare_element_id(element_id, window));

                let inspector_id;
                #[cfg(any(feature = "inspector", debug_assertions))]
                {
                    inspector_id = if window.inspector_enabled() {
                        self.element
                            .source_location()
                            .map(|source| prepare_inspector_id(source, window))
                    } else {
                        None
                    };
                }
                #[cfg(not(any(feature = "inspector", debug_assertions)))]
                {
                    inspector_id = None;
                }

                let (layout_id, request_layout) = self.element.request_layout(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                self.phase = ElementDrawPhase::RequestLayout {
                    layout_id,
                    global_id,
                    inspector_id,
                    request_layout,
                };
                layout_id
            }
            _ => panic!("must call request_layout only once"),
        }
    }

    pub(crate) fn prepaint(&mut self, window: &mut Window, cx: &mut App) {
        let _frame_stats = crate::frame_stats::element_phase(crate::frame_stats::Phase::Prepaint);
        match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                global_id,
                inspector_id,
                mut request_layout,
            }
            | ElementDrawPhase::LayoutComputed {
                layout_id,
                global_id,
                inspector_id,
                mut request_layout,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    window.element_id_stack.push(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*window.element_id_stack);
                }

                let bounds = window.layout_bounds(layout_id);
                let mut pushed_a11y_node = false;
                if window.a11y.is_active() {
                    if let Some(global_id) = global_id.as_ref() {
                        if let Some(role) = self.element.a11y_role() {
                            let node_id = global_id.accesskit_node_id();
                            let mut node = accesskit::Node::new(role);
                            let scale = window.scale_factor();
                            node.set_bounds(accesskit::Rect {
                                x0: (bounds.origin.x.0 * scale) as f64,
                                y0: (bounds.origin.y.0 * scale) as f64,
                                x1: ((bounds.origin.x.0 + bounds.size.width.0) * scale) as f64,
                                y1: ((bounds.origin.y.0 + bounds.size.height.0) * scale) as f64,
                            });
                            self.element.write_a11y_info(&mut node);
                            window.a11y.node_bounds.insert(node_id, bounds);
                            let visible_bounds = bounds.intersect(&window.content_mask().bounds);
                            window
                                .a11y
                                .node_visible_bounds
                                .insert(node_id, visible_bounds);
                            pushed_a11y_node = window.a11y.nodes.push(node_id, node);
                            #[cfg(debug_assertions)]
                            if pushed_a11y_node {
                                let view = window
                                    .a11y
                                    .view_type_names
                                    .get(&window.current_view())
                                    .copied();
                                let source_location = self.element.source_location();
                                window.a11y.nodes.record_node_info(
                                    node_id,
                                    crate::window::a11y::debug::NodeDebugInfo {
                                        synthetic: false,
                                        view,
                                        element_id: global_id.0.last().map(|id| format!("{id:?}")),
                                        source_location,
                                    },
                                );
                            }
                        }
                    } else if let Some(text) = self.element.a11y_text()
                        && !text.trim().is_empty()
                    {
                        let mut node = accesskit::Node::new(accesskit::Role::Label);
                        let scale = window.scale_factor();
                        node.set_bounds(accesskit::Rect {
                            x0: (bounds.origin.x.0 * scale) as f64,
                            y0: (bounds.origin.y.0 * scale) as f64,
                            x1: ((bounds.origin.x.0 + bounds.size.width.0) * scale) as f64,
                            y1: ((bounds.origin.y.0 + bounds.size.height.0) * scale) as f64,
                        });
                        node.set_value(text.to_string());
                        window.a11y.nodes.push_anonymous_leaf(node);
                    }
                }

                let node_id = window.next_frame.dispatch_tree.push_node();
                let mut prepaint = self.element.prepaint(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    window,
                    cx,
                );
                window.next_frame.dispatch_tree.pop_node();

                if pushed_a11y_node {
                    if let Some(global_id) = global_id.as_ref() {
                        #[cfg(debug_assertions)]
                        let creator = crate::window::a11y::debug::NodeCreator {
                            view: window
                                .a11y
                                .view_type_names
                                .get(&window.current_view())
                                .copied(),
                            element_id: global_id.0.last().map(|id| format!("{id:?}")),
                            source_location: self.element.source_location(),
                        };
                        let mut builder = A11ySubtreeBuilder::new(
                            global_id.accesskit_node_id(),
                            &mut window.a11y.nodes,
                        );
                        #[cfg(debug_assertions)]
                        {
                            builder = builder.with_creator(creator);
                        }
                        self.element
                            .a11y_synthetic_children(&mut prepaint, &mut builder);
                    }
                    window.a11y.nodes.pop();
                }

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                self.phase = ElementDrawPhase::Prepaint {
                    node_id,
                    global_id,
                    inspector_id,
                    bounds,
                    request_layout,
                    prepaint,
                };
            }
            _ => panic!("must call request_layout before prepaint"),
        }
    }

    pub(crate) fn paint(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> (E::RequestLayoutState, E::PrepaintState) {
        let _frame_stats = crate::frame_stats::element_phase(crate::frame_stats::Phase::Paint);
        match mem::take(&mut self.phase) {
            ElementDrawPhase::Prepaint {
                node_id,
                global_id,
                inspector_id,
                bounds,
                mut request_layout,
                mut prepaint,
                ..
            } => {
                if let Some(element_id) = self.element.id() {
                    window.element_id_stack.push(element_id);
                    debug_assert_eq!(&*global_id.as_ref().unwrap().0, &*window.element_id_stack);
                }

                window.next_frame.dispatch_tree.set_active_node(node_id);
                self.element.paint(
                    global_id.as_ref(),
                    inspector_id.as_ref(),
                    bounds,
                    &mut request_layout,
                    &mut prepaint,
                    window,
                    cx,
                );

                if global_id.is_some() {
                    window.element_id_stack.pop();
                }

                self.phase = ElementDrawPhase::Painted;
                (request_layout, prepaint)
            }
            _ => panic!("must call prepaint before paint"),
        }
    }

    pub(crate) fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        if matches!(&self.phase, ElementDrawPhase::Start) {
            self.request_layout(window, cx);
        }

        let layout_id = match mem::take(&mut self.phase) {
            ElementDrawPhase::RequestLayout {
                layout_id,
                global_id,
                inspector_id,
                request_layout,
            } => {
                window.compute_layout(layout_id, available_space, cx);
                self.phase = ElementDrawPhase::LayoutComputed {
                    layout_id,
                    global_id,
                    inspector_id,
                    available_space,
                    request_layout,
                };
                layout_id
            }
            ElementDrawPhase::LayoutComputed {
                layout_id,
                global_id,
                inspector_id,
                available_space: prev_available_space,
                request_layout,
            } => {
                if available_space != prev_available_space {
                    window.compute_layout(layout_id, available_space, cx);
                }
                self.phase = ElementDrawPhase::LayoutComputed {
                    layout_id,
                    global_id,
                    inspector_id,
                    available_space,
                    request_layout,
                };
                layout_id
            }
            _ => panic!("cannot measure after painting"),
        };

        window.layout_bounds(layout_id).size
    }
}

impl<E> ElementObject for Drawable<E>
where
    E: Element,
    E::RequestLayoutState: 'static,
{
    fn inner_element(&mut self) -> &mut dyn Any {
        &mut self.element
    }

    #[inline]
    fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        Drawable::request_layout(self, window, cx)
    }

    #[inline]
    fn prepaint(&mut self, window: &mut Window, cx: &mut App) {
        Drawable::prepaint(self, window, cx);
    }

    #[inline]
    fn paint(&mut self, window: &mut Window, cx: &mut App) {
        Drawable::paint(self, window, cx);
    }

    #[inline]
    fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        Drawable::layout_as_root(self, available_space, window, cx)
    }
}

pub struct AnyElement(ArenaBox<dyn ElementObject>);

impl AnyElement {
    pub(crate) fn new<E>(element: E) -> Self
    where
        E: 'static + Element,
        E::RequestLayoutState: Any,
    {
        let element = with_element_arena(|arena| arena.alloc(|| Drawable::new(element)))
            .map(|element| element as &mut dyn ElementObject);
        AnyElement(element)
    }

    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.0.inner_element().downcast_mut::<T>()
    }

    pub fn request_layout(&mut self, window: &mut Window, cx: &mut App) -> LayoutId {
        self.0.request_layout(window, cx)
    }

    pub fn prepaint(&mut self, window: &mut Window, cx: &mut App) -> Option<FocusHandle> {
        let focus_assigned = window.next_frame.focus.is_some();

        self.0.prepaint(window, cx);

        if !focus_assigned && let Some(focus_id) = window.next_frame.focus {
            return FocusHandle::for_id(focus_id, &cx.focus_handles);
        }

        None
    }

    pub fn paint(&mut self, window: &mut Window, cx: &mut App) {
        self.0.paint(window, cx);
    }

    pub fn layout_as_root(
        &mut self,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Size<Pixels> {
        self.0.layout_as_root(available_space, window, cx)
    }

    pub fn prepaint_at(
        &mut self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FocusHandle> {
        window.with_absolute_element_offset(origin, |window| self.prepaint(window, cx))
    }

    pub fn prepaint_as_root(
        &mut self,
        origin: Point<Pixels>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<FocusHandle> {
        self.layout_as_root(available_space, window, cx);
        window.with_absolute_element_offset(origin, |window| self.prepaint(window, cx))
    }
}

impl Element for AnyElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout_id = self.request_layout(window, cx);
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.paint(window, cx);
    }
}

impl IntoElement for AnyElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }

    fn into_any_element(self) -> AnyElement {
        self
    }
}

pub struct Empty;

impl IntoElement for Empty {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Empty {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (
            window.request_layout(
                Style {
                    display: crate::Display::None,
                    ..Default::default()
                },
                None,
                cx,
            ),
            (),
        )
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _state: &mut Self::RequestLayoutState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

#[inline(never)]
fn prepare_element_id(element_id: ElementId, window: &mut Window) -> GlobalElementId {
    window.element_id_stack.push(element_id);
    window.global_ids.get(&window.element_id_stack)
}

#[cfg(any(feature = "inspector", debug_assertions))]
#[inline(never)]
fn prepare_inspector_id(
    source: &'static panic::Location<'static>,
    window: &mut Window,
) -> InspectorElementId {
    let path = InspectorElementPath {
        global_id: GlobalElementId::new(Arc::from(&*window.element_id_stack)),
        source_location: source,
    };
    window.build_inspector_element_id(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

    fn path(ids: &[&'static str]) -> GlobalElementId {
        GlobalElementId::new(ids.iter().map(|id| ElementId::from(*id)).collect())
    }

    fn hash(id: &GlobalElementId) -> u64 {
        BuildHasherDefault::<crate::collections::FxHasher>::default().hash_one(id)
    }

    #[test]
    fn global_ids_compare_and_hash_by_path() {
        let a = path(&["root", "table", "row"]);
        let b = path(&["root", "table", "row"]);
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));

        let c = path(&["root", "table", "cell"]);
        assert_ne!(a, c);

        let mut forged = c;
        forged.1 = a.1;
        assert_ne!(a, forged);
        assert_eq!(GlobalElementId::default(), path(&[]));
    }

    #[test]
    fn mutable_global_id_paths_invalidate_the_cached_hash() {
        let mut id = path(&["root", "table", "row"]);
        let replacement = path(&["root", "table", "cell"]);
        *id = replacement.0.clone();
        assert_eq!(id, replacement);
        assert_eq!(hash(&id), hash(&replacement));
        assert_eq!(id.accesskit_node_id(), replacement.accesskit_node_id());

        let mut uncached = replacement.clone();
        uncached.1 = 0;
        assert_eq!(uncached, replacement);
        assert_eq!(hash(&uncached), hash(&replacement));
    }

    #[test]
    fn global_id_accessibility_ids_keep_the_original_path_hash() {
        let first = path(&["root", "table", "row"]);
        let next_frame = path(&["root", "table", "row"]);
        let mut hasher = std::hash::DefaultHasher::default();
        first.0.hash(&mut hasher);
        assert_eq!(
            first.accesskit_node_id(),
            accesskit::NodeId(hasher.finish())
        );
        assert_eq!(first.accesskit_node_id(), next_frame.accesskit_node_id());
    }
}

#[cfg(test)]
mod global_id_cache_tests {
    use super::*;

    fn path(ids: &[&'static str]) -> Vec<ElementId> {
        ids.iter().map(|id| ElementId::from(*id)).collect()
    }

    #[test]
    fn global_ids_are_reused_while_their_path_is_in_use() {
        let row = path(&["root", "table", "row"]);
        let mut cache = GlobalIdCache::default();

        let first = cache.get(&row);
        assert!(Arc::ptr_eq(&first.0, &cache.get(&row).0));

        cache.finish_frame();
        let next = cache.get(&row);
        assert!(Arc::ptr_eq(&first.0, &next.0));
        assert_eq!(first.accesskit_node_id(), next.accesskit_node_id());

        cache.finish_frame();
        cache.finish_frame();
        let later = cache.get(&row);
        assert!(!Arc::ptr_eq(&first.0, &later.0));
        assert_eq!(first, later);
    }

    #[test]
    fn unused_global_ids_are_released_after_one_unused_frame() {
        let row = path(&["root", "table", "row"]);
        let mut cache = GlobalIdCache::default();
        let id = cache.get(&row);
        let weak = Arc::downgrade(&id.0);
        drop(id);
        cache.finish_frame();
        assert!(weak.upgrade().is_some());
        cache.finish_frame();
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn global_id_cache_checks_paths_on_hash_collisions() {
        let row = path(&["root", "table", "row"]);
        let cell = path(&["root", "table", "cell"]);
        let hash = GlobalIdCache::path_hash(&row);
        let mut cache = GlobalIdCache::default();
        let collision = GlobalElementId::new(Arc::from(&*cell));
        cache.current.insert(hash, collision.clone());
        assert_eq!(&*cache.get(&row).0, &*row);
        cache.current.clear();
        cache.previous.insert(hash, collision);
        assert_eq!(&*cache.get(&row).0, &*row);
    }
}
