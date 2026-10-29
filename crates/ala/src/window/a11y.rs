use crate::*;

pub(crate) mod debug;

use crate::collections::{FxHashMap, FxHashSet};
use crate::{App, Bounds, FocusId, Pixels, SharedString, Window};
use accesskit::{Action, NodeId, TreeUpdate};
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::mem;
use std::ops::Range;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(crate) const ROOT_NODE_ID: NodeId = NodeId(0);

pub(crate) type A11yActionListener =
    Box<dyn FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static>;

pub(crate) struct A11y {
    force_disabled: bool,
    forced: bool,
    active_flag: Arc<AtomicBool>,
    active_this_frame: bool,
    pub(crate) nodes: A11yNodeBuilder,
    pub(crate) focus_ids: FxHashMap<NodeId, FocusId>,
    pub(crate) node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) node_visible_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    pub(crate) action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    previous_focus_ids: FxHashMap<NodeId, FocusId>,
    previous_node_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    previous_node_visible_bounds: FxHashMap<NodeId, Bounds<Pixels>>,
    previous_action_listeners: FxHashMap<NodeId, Vec<(Action, A11yActionListener)>>,
    built_last_frame: bool,
    window_title: Option<SharedString>,
    last_focus_without_node: Option<FocusId>,
    debug: debug::A11yDebug,
    #[cfg(debug_assertions)]
    pub(crate) view_type_names: FxHashMap<EntityId, &'static str>,
}

impl A11y {
    pub(crate) fn new(
        active_flag: Arc<AtomicBool>,
        force_disabled: bool,
        forced: bool,
        window_title: Option<SharedString>,
    ) -> Self {
        Self {
            force_disabled,
            forced,
            active_flag,
            active_this_frame: false,
            nodes: A11yNodeBuilder::new(),
            focus_ids: FxHashMap::default(),
            node_bounds: FxHashMap::default(),
            node_visible_bounds: FxHashMap::default(),
            action_listeners: FxHashMap::default(),
            previous_focus_ids: FxHashMap::default(),
            previous_node_bounds: FxHashMap::default(),
            previous_node_visible_bounds: FxHashMap::default(),
            previous_action_listeners: FxHashMap::default(),
            built_last_frame: false,
            window_title,
            last_focus_without_node: None,
            debug: debug::A11yDebug::default(),
            #[cfg(debug_assertions)]
            view_type_names: FxHashMap::default(),
        }
    }

    pub(crate) fn note_focus_without_node(&mut self, focus_id: FocusId, reason: &str) {
        if self.last_focus_without_node != Some(focus_id) {
            self.last_focus_without_node = Some(focus_id);
            log::info!(
                "a11y: focused element ({focus_id:?}) has no accessibility node \
                 ({reason}); assistive technology will announce the whole window \
                 instead. Give it both an `.id(...)` and a `.role(...)` to expose it."
            );
        }
    }

    pub(crate) fn set_window_title(&mut self, title: impl Into<SharedString>) {
        self.window_title = Some(title.into());
    }

    pub(crate) fn sync_active_flag(&mut self) {
        self.active_this_frame =
            self.is_enabled() && (self.forced || self.active_flag.load(Ordering::SeqCst));
    }

    pub(crate) fn set_forced(&mut self, forced: bool) {
        self.forced = forced;
    }

    pub(crate) fn is_enabled(&self) -> bool {
        !self.force_disabled
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active_this_frame
    }

    pub(crate) fn set_focusable(&mut self, node_id: NodeId, focus_id: FocusId) {
        self.focus_ids.insert(node_id, focus_id);
    }

    pub(crate) fn set_focus(&mut self, node_id: NodeId) {
        if !self.focus_ids.contains_key(&node_id) {
            if cfg!(debug_assertions) {
                panic!("set_focus called for a node that was not registered with set_focusable");
            } else {
                log::warn!(
                    "a11y: set_focus called for a node that was not registered with \
                     set_focusable ({node_id:?})"
                );
            }
        }
        if self.nodes.has_node(node_id) {
            self.last_focus_without_node = None;
            self.nodes.set_focus(node_id);
        } else {
            if let Some(focus_id) = self.focus_ids.get(&node_id).copied() {
                self.note_focus_without_node(focus_id, "it has an id but no role");
            }
        }
    }

    pub(crate) fn set_active_descendant(&mut self, node_id: NodeId) {
        if self.nodes.node_is_focused(node_id) {
            if cfg!(debug_assertions) {
                panic!("set_active_descendant called on the focused node");
            } else {
                log::warn!("a11y: set_active_descendant called on the focused node ({node_id:?})");
            }
            return;
        }
        if self.nodes.has_node(node_id) && self.nodes.focus_is_ancestor_of_current() {
            self.nodes.set_active_descendant(node_id);
        }
    }

    pub(crate) fn begin_frame(&mut self) -> bool {
        mem::swap(&mut self.focus_ids, &mut self.previous_focus_ids);
        mem::swap(&mut self.node_bounds, &mut self.previous_node_bounds);
        mem::swap(
            &mut self.node_visible_bounds,
            &mut self.previous_node_visible_bounds,
        );
        mem::swap(
            &mut self.action_listeners,
            &mut self.previous_action_listeners,
        );
        self.focus_ids.clear();
        self.node_bounds.clear();
        self.node_visible_bounds.clear();
        self.action_listeners.clear();
        self.nodes.begin_frame(self.window_title.as_ref());
        mem::replace(&mut self.built_last_frame, true)
    }

    pub(crate) fn skip_frame(&mut self) {
        self.built_last_frame = false;
    }

    pub(crate) fn reuse(&mut self, range: Range<A11yIndex>, focus: Option<FocusId>) {
        if !self.active_this_frame {
            return;
        }
        let Some(nodes) = self
            .debug
            .last_tree_update()
            .and_then(|update| update.nodes.get(range.start.nodes..range.end.nodes))
        else {
            return;
        };
        let children: FxHashSet<NodeId> = nodes
            .iter()
            .flat_map(|(_, node)| node.children().iter().copied())
            .collect();
        for (id, node) in nodes {
            if !self.nodes.seen_ids.insert(*id) {
                continue;
            }
            if !children.contains(id)
                && let Some(parent) = self.nodes.nodes_stack.last_mut()
            {
                parent.push_child(*id);
            }
            self.nodes.all_nodes.push((*id, node.clone()));
            if let Some(focus_id) = self.previous_focus_ids.get(id) {
                self.focus_ids.insert(*id, *focus_id);
                if focus == Some(*focus_id) && self.nodes.focus.is_none() {
                    self.nodes.focus = Some(*id);
                }
            }
            if let Some(bounds) = self.previous_node_bounds.get(id) {
                self.node_bounds.insert(*id, *bounds);
            }
            if let Some(bounds) = self.previous_node_visible_bounds.get(id) {
                self.node_visible_bounds.insert(*id, *bounds);
            }
            if let Some(listeners) = self.previous_action_listeners.remove(id) {
                self.action_listeners.insert(*id, listeners);
            }
        }
        if range.end.active_descendant != range.start.active_descendant
            && self.nodes.active_descendant.is_none()
            && range
                .end
                .active_descendant
                .is_some_and(|node| nodes.iter().any(|(id, _)| *id == node))
            && self.nodes.focus.is_some_and(|focus| {
                self.nodes.ids_stack.contains(&focus) || nodes.iter().any(|(id, _)| *id == focus)
            })
        {
            self.nodes.active_descendant = range.end.active_descendant;
        }
    }

    pub(crate) fn end_frame(&mut self, frame: debug::FrameDebugInfo) -> TreeUpdate {
        let update = self.nodes.finalize();
        self.debug.capture(
            &update,
            self.nodes.focus,
            self.nodes.active_descendant,
            self.window_title.as_ref(),
            frame,
            &self.node_bounds,
        );
        #[cfg(debug_assertions)]
        self.debug.capture_node_info(&self.nodes.node_info);
        update
    }

    pub(crate) fn debug_tree_json(&self) -> Option<String> {
        self.debug.to_json()
    }

    pub(crate) fn last_tree_update(&self) -> Option<&TreeUpdate> {
        self.debug.last_tree_update()
    }

    pub(crate) fn last_node_bounds(&self, id: NodeId) -> Option<Bounds<Pixels>> {
        self.debug.node_bounds(id)
    }

    pub(crate) fn frame_number(&self) -> u64 {
        self.debug.frame_number()
    }

    pub(crate) fn index(&self) -> A11yIndex {
        A11yIndex {
            nodes: self.nodes.all_nodes.len(),
            children: self
                .nodes
                .nodes_stack
                .last()
                .map_or(0, |node| node.children().len()),
            focus: self.nodes.focus,
            active_descendant: self.nodes.active_descendant,
        }
    }

    pub(crate) fn truncate(&mut self, index: A11yIndex) {
        for (id, _) in self.nodes.all_nodes.drain(index.nodes..) {
            self.nodes.seen_ids.remove(&id);
            self.focus_ids.remove(&id);
            self.node_bounds.remove(&id);
            self.node_visible_bounds.remove(&id);
            #[cfg(debug_assertions)]
            self.nodes.node_info.remove(&id);
        }
        if let Some(parent) = self.nodes.nodes_stack.last_mut()
            && parent.children().len() > index.children
        {
            let children = parent.children()[..index.children].to_vec();
            parent.set_children(children);
        }
        self.nodes.focus = index.focus;
        self.nodes.active_descendant = index.active_descendant;
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct A11yIndex {
    nodes: usize,
    children: usize,
    focus: Option<NodeId>,
    active_descendant: Option<NodeId>,
}

pub struct A11ySubtreeBuilder<'a> {
    parent_id: NodeId,
    nodes: &'a mut A11yNodeBuilder,
    #[cfg(debug_assertions)]
    creator: debug::NodeCreator,
}

impl<'a> A11ySubtreeBuilder<'a> {
    pub(crate) fn new(parent_id: NodeId, nodes: &'a mut A11yNodeBuilder) -> Self {
        Self {
            parent_id,
            nodes,
            #[cfg(debug_assertions)]
            creator: debug::NodeCreator::default(),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn with_creator(mut self, creator: debug::NodeCreator) -> Self {
        self.creator = creator;
        self
    }

    pub fn synthetic_node_id(&self, key: impl Hash) -> NodeId {
        let mut hasher = std::hash::DefaultHasher::default();
        self.parent_id.0.hash(&mut hasher);
        key.hash(&mut hasher);
        NodeId(hasher.finish())
    }

    pub fn push_child(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        let pushed = self.nodes.push_leaf(id, node);
        #[cfg(debug_assertions)]
        if pushed {
            self.nodes.record_node_info(
                id,
                debug::NodeDebugInfo {
                    synthetic: true,
                    view: self.creator.view,
                    element_id: self.creator.element_id.clone(),
                    source_location: self.creator.source_location,
                },
            );
        }
        pushed
    }

    pub fn parent_node(&mut self) -> &mut accesskit::Node {
        self.nodes
            .current_node_mut()
            .expect("A11ySubtreeBuilder exists only while its element's node is on the stack")
    }
}

pub(crate) struct A11yNodeBuilder {
    ids_stack: SmallVec<[NodeId; 16]>,
    nodes_stack: SmallVec<[accesskit::Node; 16]>,
    all_nodes: Vec<(NodeId, accesskit::Node)>,
    seen_ids: FxHashSet<NodeId>,
    focus: Option<NodeId>,
    active_descendant: Option<NodeId>,
    #[cfg(debug_assertions)]
    node_info: FxHashMap<NodeId, debug::NodeDebugInfo>,
}

impl A11yNodeBuilder {
    fn new() -> Self {
        Self {
            ids_stack: SmallVec::new(),
            nodes_stack: SmallVec::new(),
            all_nodes: Vec::new(),
            seen_ids: FxHashSet::default(),
            focus: None,
            active_descendant: None,
            #[cfg(debug_assertions)]
            node_info: FxHashMap::default(),
        }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn record_node_info(&mut self, id: NodeId, info: debug::NodeDebugInfo) {
        self.node_info.insert(id, info);
    }

    #[must_use]
    fn can_push(&mut self, id: NodeId) -> bool {
        debug_assert!(!self.ids_stack.is_empty(), "node pushed before push_root");

        if !self.seen_ids.insert(id) {
            debug_assert!(
                false,
                "Duplicate a11y node id: {id:?}. In a release build, this node would be silently discarded from the a11y tree."
            );
            return false;
        }

        true
    }

    pub(crate) fn push(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.ids_stack.push(id);
        self.nodes_stack.push(node);
        true
    }

    pub(crate) fn push_leaf(&mut self, id: NodeId, node: accesskit::Node) -> bool {
        if !self.can_push(id) {
            return false;
        }

        if let Some(parent) = self.nodes_stack.last_mut() {
            parent.push_child(id);
        }
        self.all_nodes.push((id, node));
        true
    }

    pub(crate) fn push_anonymous_leaf(&mut self, node: accesskit::Node) -> bool {
        let Some(parent) = self.ids_stack.last().copied() else {
            return false;
        };
        let position = self
            .nodes_stack
            .last()
            .map_or(0, |parent| parent.children().len());
        let mut hasher = std::hash::DefaultHasher::default();
        parent.0.hash(&mut hasher);
        "anonymous".hash(&mut hasher);
        position.hash(&mut hasher);
        let mut id = NodeId(hasher.finish());
        while self.seen_ids.contains(&id) {
            let mut hasher = std::hash::DefaultHasher::default();
            id.0.hash(&mut hasher);
            "anonymous".hash(&mut hasher);
            id = NodeId(hasher.finish());
        }
        self.push_leaf(id, node)
    }

    pub(crate) fn current_node_mut(&mut self) -> Option<&mut accesskit::Node> {
        self.nodes_stack.last_mut()
    }

    pub(crate) fn pop(&mut self) {
        debug_assert!(self.ids_stack.len() > 1, "pop would remove the root node");

        if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
            self.all_nodes.push((id, node));
        }
    }

    fn begin_frame(&mut self, window_title: Option<&SharedString>) {
        self.all_nodes.clear();
        self.ids_stack.clear();
        self.nodes_stack.clear();
        self.seen_ids.clear();
        #[cfg(debug_assertions)]
        self.node_info.clear();
        let mut root_node = accesskit::Node::new(accesskit::Role::Window);
        if let Some(title) = window_title {
            root_node.set_label(title.to_string());
        }

        self.ids_stack.push(ROOT_NODE_ID);
        self.nodes_stack.push(root_node);
        self.focus = None;
        self.active_descendant = None;
    }

    pub(crate) fn has_node(&self, id: NodeId) -> bool {
        id == ROOT_NODE_ID || self.seen_ids.contains(&id)
    }

    pub(crate) fn node_is_focused(&self, id: NodeId) -> bool {
        self.focus == Some(id)
    }

    pub(crate) fn focus_is_ancestor_of_current(&self) -> bool {
        let Some(focus) = self.focus else {
            return false;
        };

        let ancestor_count = self.ids_stack.len().saturating_sub(1);
        self.ids_stack[..ancestor_count].contains(&focus)
    }

    pub(crate) fn set_active_descendant(&mut self, id: NodeId) {
        if self
            .active_descendant
            .is_some_and(|existing| existing != id)
        {
            if cfg!(debug_assertions) {
                panic!("active descendant claimed by multiple nodes in one frame");
            } else {
                log::warn!(
                    "a11y: multiple nodes claimed the active descendant this frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.active_descendant = Some(id);
    }

    pub(crate) fn set_focus(&mut self, id: NodeId) {
        if self.focus.is_some() {
            if cfg!(debug_assertions) {
                panic!("set_focus called more than once in a single frame");
            } else {
                log::warn!(
                    "a11y: set_focus called more than once in a single frame; \
                     using last-wins ({id:?})"
                );
            }
        }
        self.focus = Some(id);
    }

    fn finalize(&mut self) -> TreeUpdate {
        debug_assert_eq!(self.ids_stack.len(), 1);
        debug_assert_eq!(self.ids_stack[0], ROOT_NODE_ID);

        if self.ids_stack.len() != 1 {
            log::error!(
                "a11y: Stack imbalance at end of frame: expected 1 (root), got {}. \
                 Some elements may have pushed without popping.",
                self.ids_stack.len()
            );
        }

        while !self.ids_stack.is_empty() {
            if let (Some(id), Some(node)) = (self.ids_stack.pop(), self.nodes_stack.pop()) {
                self.all_nodes.push((id, node));
            }
        }

        let focus = match self.active_descendant {
            Some(id) if self.has_node(id) => id,
            Some(id) => {
                if cfg!(debug_assertions) {
                    panic!("active_descendant set to {id:?}, which is not in the tree");
                } else {
                    log::warn!("active_descendant set to {id:?}, which is not in the tree");
                    self.focus.unwrap_or(ROOT_NODE_ID)
                }
            }

            _ => self.focus.unwrap_or(ROOT_NODE_ID),
        };

        let nodes = std::mem::take(&mut self.all_nodes);
        let update = TreeUpdate {
            nodes,
            tree: Some(accesskit::Tree::new(ROOT_NODE_ID)),
            tree_id: accesskit::TreeId::ROOT,
            focus,
        };

        Self::repair_tree_update(update)
    }

    fn repair_tree_update(mut update: TreeUpdate) -> TreeUpdate {
        let node_ids: FxHashSet<NodeId> = update.nodes.iter().map(|(id, _)| *id).collect();

        if !node_ids.contains(&update.focus) {
            log::error!(
                "a11y: Focused node {:?} is not in the tree ({} nodes). \
                 Falling back to root. This is a bug in the a11y tree builder.",
                update.focus,
                update.nodes.len()
            );
            update.focus = ROOT_NODE_ID;
        }

        for (id, node) in &mut update.nodes {
            let has_invalid_child = node
                .children()
                .iter()
                .any(|child_id| !node_ids.contains(child_id));
            if has_invalid_child {
                let children = node.children();
                let invalid_count = children
                    .iter()
                    .filter(|child_id| !node_ids.contains(child_id))
                    .count();
                log::error!(
                    "a11y: Node {:?} references {} children not present in the tree. \
                     Stripping invalid child references.",
                    id,
                    invalid_count
                );
                let valid: Vec<NodeId> = children
                    .iter()
                    .copied()
                    .filter(|child_id| node_ids.contains(child_id))
                    .collect();
                node.set_children(valid);
            }
        }

        update
    }
}

#[cfg(test)]
mod tests {
    use super::{A11y, A11yNodeBuilder, ROOT_NODE_ID};
    use crate::{Bounds, FocusId, point, px, size};
    use accesskit::{NodeId, Role};
    use std::sync::{Arc, atomic::AtomicBool};

    fn test_node() -> accesskit::Node {
        accesskit::Node::new(Role::GenericContainer)
    }

    fn new_builder() -> A11yNodeBuilder {
        let mut builder = A11yNodeBuilder::new();
        builder.begin_frame(None);
        builder
    }

    fn new_a11y() -> A11y {
        let mut a11y = A11y::new(Arc::new(AtomicBool::new(true)), false, false, None);
        a11y.begin_frame();
        a11y
    }

    #[test]
    fn accessibility_enabled_is_independent_of_activation() {
        for force_disabled in [false, true] {
            for active in [false, true] {
                for forced in [false, true] {
                    let mut a11y = A11y::new(
                        Arc::new(AtomicBool::new(active)),
                        force_disabled,
                        forced,
                        None,
                    );
                    a11y.sync_active_flag();

                    assert_eq!(a11y.is_enabled(), !force_disabled);
                    assert_eq!(a11y.is_active(), !force_disabled && (active || forced));
                }
            }
        }
    }

    #[test]
    fn disabling_forced_tree_preserves_adapter_activation() {
        let active_flag = Arc::new(AtomicBool::new(true));
        let mut a11y = A11y::new(active_flag.clone(), false, true, None);
        a11y.sync_active_flag();
        assert!(a11y.is_active());

        a11y.set_forced(false);
        a11y.sync_active_flag();
        assert!(a11y.is_active());

        active_flag.store(false, std::sync::atomic::Ordering::SeqCst);
        a11y.sync_active_flag();
        assert!(!a11y.is_active());

        a11y.set_forced(true);
        a11y.sync_active_flag();
        assert!(a11y.is_active());
    }

    #[test]
    fn retained_tree_and_bounds_survive_begin_frame_and_replace_on_capture() {
        let mut a11y = new_a11y();
        let first = NodeId(1);
        let first_bounds = Bounds::new(point(px(12.), px(34.)), size(px(56.), px(78.)));
        assert!(a11y.last_tree_update().is_none());
        assert_eq!(a11y.frame_number(), 0);
        assert!(a11y.nodes.push(first, test_node()));
        a11y.node_bounds.insert(first, first_bounds);
        a11y.nodes.pop();
        a11y.end_frame(Default::default());

        a11y.begin_frame();
        assert!(a11y.node_bounds.is_empty());
        assert_eq!(a11y.last_node_bounds(first), Some(first_bounds));
        assert!(
            a11y.last_tree_update()
                .unwrap()
                .nodes
                .iter()
                .any(|(id, _)| *id == first)
        );
        assert_eq!(a11y.frame_number(), 1);

        let second = NodeId(2);
        let second_bounds = Bounds::new(point(px(98.), px(76.)), size(px(54.), px(32.)));
        assert!(a11y.nodes.push(second, test_node()));
        a11y.node_bounds.insert(second, second_bounds);
        a11y.nodes.pop();
        a11y.end_frame(Default::default());

        assert_eq!(a11y.last_node_bounds(first), None);
        assert_eq!(a11y.last_node_bounds(second), Some(second_bounds));
        let update = a11y.last_tree_update().unwrap();
        assert!(!update.nodes.iter().any(|(id, _)| *id == first));
        assert!(update.nodes.iter().any(|(id, _)| *id == second));
        assert_eq!(a11y.frame_number(), 2);
    }

    #[test]
    fn active_descendant_honored_when_container_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(item, test_node()));

        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop();
        builder.pop();
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_honored_for_deep_descendant() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let group = NodeId(2);
        let item = NodeId(3);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);
        assert!(builder.push(group, test_node()));
        assert!(builder.push(item, test_node()));

        assert!(builder.focus_is_ancestor_of_current());
        builder.set_active_descendant(item);

        builder.pop();
        builder.pop();
        builder.pop();
        let update = builder.finalize();
        assert_eq!(update.focus, item);
    }

    #[test]
    fn active_descendant_ignored_when_focus_in_other_subtree() {
        let mut builder = new_builder();
        let focused_container = NodeId(1);
        let focused_leaf = NodeId(2);
        let other_container = NodeId(3);
        let other_item = NodeId(4);

        assert!(builder.push(focused_container, test_node()));
        assert!(builder.push(focused_leaf, test_node()));
        builder.set_focus(focused_leaf);
        builder.pop();
        builder.pop();

        assert!(builder.push(other_container, test_node()));
        assert!(builder.push(other_item, test_node()));
        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, focused_leaf);
    }

    #[test]
    fn active_descendant_ignored_when_nothing_focused() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        assert!(builder.push(item, test_node()));

        assert!(!builder.focus_is_ancestor_of_current());
        builder.pop();
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, ROOT_NODE_ID);
    }

    #[test]
    fn regular_focus_used_when_no_active_descendant() {
        let mut builder = new_builder();
        let focused = NodeId(1);

        assert!(builder.push(focused, test_node()));
        builder.set_focus(focused);
        builder.pop();

        let update = builder.finalize();
        assert_eq!(update.focus, focused);
    }

    #[test]
    fn focus_is_ancestor_excludes_self_and_non_ancestors() {
        let mut builder = new_builder();
        let container = NodeId(1);
        let item = NodeId(2);

        assert!(builder.push(container, test_node()));
        builder.set_focus(container);

        assert!(!builder.focus_is_ancestor_of_current());

        assert!(builder.push(item, test_node()));
        assert!(builder.focus_is_ancestor_of_current());

        builder.pop();
        builder.pop();
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn multiple_active_descendant_claims_panic_in_debug() {
        let mut builder = new_builder();
        builder.set_active_descendant(NodeId(1));
        builder.set_active_descendant(NodeId(2));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "set_focus called more than once")
    )]
    fn setting_focus_twice_panics_in_debug() {
        let mut builder = new_builder();
        builder.set_focus(NodeId(1));
        builder.set_focus(NodeId(2));
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "was not registered with set_focusable")
    )]
    fn set_focus_without_set_focusable() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        a11y.set_focus(node);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "on the focused node"))]
    fn set_active_descendant_on_focused_node() {
        let mut a11y = new_a11y();
        let node = NodeId(1);
        assert!(a11y.nodes.push(node, test_node()));
        a11y.set_focusable(node, FocusId::default());
        a11y.set_focus(node);
        a11y.set_active_descendant(node);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "active descendant claimed by multiple nodes")
    )]
    fn two_siblings_claiming_active_descendant() {
        let mut a11y = new_a11y();
        let container = NodeId(1);
        let first = NodeId(2);
        let second = NodeId(3);

        assert!(a11y.nodes.push(container, test_node()));
        a11y.set_focusable(container, FocusId::default());
        a11y.set_focus(container);

        assert!(a11y.nodes.push(first, test_node()));
        a11y.set_active_descendant(first);
        a11y.nodes.pop();

        assert!(a11y.nodes.push(second, test_node()));
        a11y.set_active_descendant(second);
        a11y.nodes.pop();

        a11y.nodes.pop();
    }

    #[test]
    fn active_descendant_in_unfocused_subtree_keeps_real_focus() {
        let mut a11y = new_a11y();
        let a = NodeId(1);
        let b = NodeId(2);
        let c = NodeId(3);

        assert!(a11y.nodes.push(a, test_node()));
        a11y.set_focusable(a, FocusId::default());
        a11y.set_focus(a);
        a11y.nodes.pop();

        assert!(a11y.nodes.push(b, test_node()));
        assert!(a11y.nodes.push(c, test_node()));
        a11y.set_active_descendant(c);
        a11y.nodes.pop();
        a11y.nodes.pop();

        let update = a11y.end_frame(Default::default());
        assert_eq!(update.focus, a);
    }
}
