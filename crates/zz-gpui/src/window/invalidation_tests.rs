use std::{cell::Cell, ops::Range, rc::Rc, time::Duration};

use crate::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent,
    Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, Pixels, PlatformInput, Point, Render, StatefulInteractiveElement as _,
    StyleRefinement, Styled as _, TestAppContext, UTF16Selection, Window, canvas, div, hsla, point,
    px, util::FluentBuilder as _,
};

const SIDE: f32 = 100.;

fn color(hue: f32) -> Hsla {
    hsla(hue, 0.5, 0.5, 1.)
}

fn idle() -> Hsla {
    color(0.1)
}

fn active() -> Hsla {
    color(0.2)
}

fn hovered() -> Hsla {
    color(0.3)
}

fn focused() -> Hsla {
    color(0.4)
}

fn drop_target() -> Hsla {
    color(0.5)
}

#[derive(Default)]
struct Leaf {
    renders: Rc<Cell<usize>>,
    hover: bool,
    focus: Option<FocusHandle>,
    watch: Option<FocusHandle>,
    saw_watched_focus: Rc<Cell<Option<bool>>>,
    input: bool,
}

impl Render for Leaf {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        if let Some(watch) = &self.watch {
            self.saw_watched_focus.set(Some(watch.is_focused(window)));
        }
        let mut leaf = div()
            .id("leaf")
            .size_full()
            .bg(idle())
            .when(self.hover, |leaf| leaf.hover(|style| style.bg(hovered())))
            .active(|style| style.bg(active()))
            .drag_over::<Dragged>(|style, _, _, _| style.bg(drop_target()))
            .on_click(|_, _, _| {});
        if let Some(focus) = &self.focus {
            leaf = leaf.track_focus(focus).focus(|style| style.bg(focused()));
        }
        if self.input
            && let Some(focus) = self.focus.clone()
        {
            let view = cx.entity();
            leaf = leaf.child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx);
                    },
                )
                .size_full(),
            );
        }
        leaf
    }
}

impl EntityInputHandler for Leaf {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        None
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        _: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        None
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

struct Dragged;

struct DragPreview;

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(4.))
    }
}

struct Row {
    leaves: Vec<Entity<Leaf>>,
    drag_source: bool,
}

impl Render for Row {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .children(self.leaves.iter().map(|leaf| {
                leaf.clone()
                    .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE)))
            }))
            .when(self.drag_source, |row| {
                row.child(
                    div()
                        .id("drag-source")
                        .w(px(SIDE))
                        .h(px(SIDE))
                        .on_drag(Dragged, |_, _, _, cx| cx.new(|_| DragPreview)),
                )
            })
    }
}

struct Fixture {
    window: AnyWindowHandle,
    renders: Vec<Rc<Cell<usize>>>,
}

impl Fixture {
    fn new(
        cx: &mut TestAppContext,
        drag_source: bool,
        build: impl FnOnce(&mut App) -> Vec<Leaf>,
    ) -> (Self, Vec<Entity<Leaf>>) {
        let leaves = cx.update(build);
        let renders = leaves.iter().map(|leaf| leaf.renders.clone()).collect();
        let mut entities = Vec::new();
        let window = cx.add_window(|_, cx| {
            entities = leaves.into_iter().map(|leaf| cx.new(|_| leaf)).collect();
            Row {
                leaves: entities.clone(),
                drag_source,
            }
        });
        let fixture = Self {
            window: window.into(),
            renders,
        };
        fixture.update(cx, |window, cx| {
            window.dispatch_event(mouse_move(point(px(SIDE * 6.), px(SIDE * 3.)), None), cx);
            window.refresh();
            window.draw(cx).clear(cx);
        });
        (fixture, entities)
    }

    fn update<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        cx.update_window(self.window, |_, window, cx| f(window, cx))
            .unwrap()
    }

    fn frame(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App)) -> Vec<usize> {
        for renders in &self.renders {
            renders.set(0);
        }
        self.update(cx, |window, cx| {
            f(window, cx);
            window.draw(cx).clear(cx);
        });
        self.renders.iter().map(|renders| renders.get()).collect()
    }

    fn colors(&self, cx: &mut TestAppContext) -> Vec<Hsla> {
        self.update(cx, |window, _| {
            (0..self.renders.len())
                .map(|index| color_at(window, leaf_center(index)).unwrap_or_default())
                .collect()
        })
    }
}

fn leaf_center(index: usize) -> Point<Pixels> {
    point(px(SIDE * index as f32 + SIDE / 2.), px(SIDE / 2.))
}

fn mouse_move(position: Point<Pixels>, pressed_button: Option<MouseButton>) -> PlatformInput {
    PlatformInput::MouseMove(MouseMoveEvent {
        position,
        pressed_button,
        modifiers: Modifiers::default(),
    })
}

fn mouse_down(position: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseDown(MouseDownEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
        first_mouse: false,
    })
}

fn mouse_up(position: Point<Pixels>) -> PlatformInput {
    PlatformInput::MouseUp(MouseUpEvent {
        position,
        button: MouseButton::Left,
        modifiers: Modifiers::default(),
        click_count: 1,
    })
}

fn key_down(key: &str) -> PlatformInput {
    PlatformInput::KeyDown(KeyDownEvent {
        keystroke: Keystroke::parse(key).unwrap(),
        is_held: false,
        prefer_character_input: false,
    })
}

#[crate::test]
fn a_click_rerenders_only_the_view_it_lands_in(cx: &mut TestAppContext) {
    let (fixture, _) = Fixture::new(cx, false, |_| {
        (0..3)
            .map(|_| Leaf {
                hover: true,
                ..Leaf::default()
            })
            .collect()
    });
    let target = leaf_center(1);
    fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(target, None), cx);
    });
    assert_eq!(fixture.colors(cx), vec![idle(), hovered(), idle()]);

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_down(target), cx);
    });
    assert_eq!(renders, vec![0, 1, 0]);
    assert_eq!(fixture.colors(cx), vec![idle(), active(), idle()]);

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_up(target), cx);
    });
    assert_eq!(renders, vec![0, 1, 0]);
    assert_eq!(fixture.colors(cx), vec![idle(), hovered(), idle()]);
}

#[crate::test]
fn a_focus_change_rerenders_only_views_that_read_focus(cx: &mut TestAppContext) {
    let (fixture, leaves) = Fixture::new(cx, false, |cx| {
        let first = cx.focus_handle();
        let second = cx.focus_handle();
        vec![
            Leaf {
                focus: Some(first.clone()),
                ..Leaf::default()
            },
            Leaf {
                focus: Some(second),
                ..Leaf::default()
            },
            Leaf {
                watch: Some(first),
                ..Leaf::default()
            },
            Leaf::default(),
        ]
    });
    let handles: Vec<FocusHandle> = cx.update(|cx| {
        leaves[..2]
            .iter()
            .map(|leaf| leaf.read(cx).focus.clone().unwrap())
            .collect()
    });
    let watcher = cx.update(|cx| leaves[2].read(cx).saw_watched_focus.clone());

    let renders = fixture.frame(cx, |window, cx| window.focus(&handles[0], cx));
    assert_eq!(renders, vec![1, 0, 1, 0]);
    assert_eq!(watcher.get(), Some(true));
    assert_eq!(fixture.colors(cx), vec![focused(), idle(), idle(), idle()]);

    let renders = fixture.frame(cx, |window, cx| window.focus(&handles[1], cx));
    assert_eq!(renders, vec![1, 1, 1, 0]);
    assert_eq!(watcher.get(), Some(false));
    assert_eq!(fixture.colors(cx), vec![idle(), focused(), idle(), idle()]);

    let renders = fixture.frame(cx, |window, cx| window.blur(cx));
    assert_eq!(renders, vec![0, 1, 0, 0]);
    assert_eq!(fixture.colors(cx), vec![idle(); 4]);
}

#[crate::test]
fn focusing_a_cached_view_registers_its_input_handler(cx: &mut TestAppContext) {
    let (fixture, leaves) = Fixture::new(cx, false, |cx| {
        vec![
            Leaf {
                focus: Some(cx.focus_handle()),
                input: true,
                ..Leaf::default()
            },
            Leaf::default(),
        ]
    });
    let handle = cx.update(|cx| leaves[0].read(cx).focus.clone().unwrap());
    fixture.update(cx, |window, _| {
        assert!(window.platform_window.take_input_handler().is_none());
    });
    let renders = fixture.frame(cx, |window, cx| window.focus(&handle, cx));
    assert_eq!(renders, vec![1, 0]);
    fixture.update(cx, |window, _| {
        assert!(window.platform_window.take_input_handler().is_some());
    });
}

#[crate::test]
fn switching_to_the_keyboard_rerenders_only_hovered_views(cx: &mut TestAppContext) {
    let (fixture, _) = Fixture::new(cx, false, |_| {
        (0..3)
            .map(|_| Leaf {
                hover: true,
                ..Leaf::default()
            })
            .collect()
    });
    let target = leaf_center(2);
    fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(target, None), cx);
    });
    assert_eq!(fixture.colors(cx), vec![idle(), idle(), hovered()]);

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(key_down("a"), cx);
    });
    assert_eq!(renders, vec![0, 0, 1]);
    assert_eq!(fixture.colors(cx), vec![idle(); 3]);

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(target, None), cx);
    });
    assert_eq!(renders, vec![0, 0, 1]);
    assert_eq!(fixture.colors(cx), vec![idle(), idle(), hovered()]);
}

#[crate::test]
fn a_drag_rerenders_only_the_drop_targets_it_enters_and_leaves(cx: &mut TestAppContext) {
    let (fixture, _) = Fixture::new(cx, true, |_| (0..3).map(|_| Leaf::default()).collect());
    let source = leaf_center(3);
    fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(source, None), cx);
        window.dispatch_event(mouse_down(source), cx);
    });
    let start = source - point(px(20.), px(0.));
    fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(start, Some(MouseButton::Left)), cx);
    });
    fixture.update(cx, |_, cx| assert!(cx.has_active_drag()));

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(leaf_center(1), Some(MouseButton::Left)), cx);
    });
    assert_eq!(renders, vec![0, 1, 0]);
    assert_eq!(fixture.colors(cx), vec![idle(), drop_target(), idle()]);

    let nudge = leaf_center(1) + point(px(5.), px(5.));
    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(nudge, Some(MouseButton::Left)), cx);
    });
    assert_eq!(renders, vec![0, 0, 0]);
    assert_eq!(fixture.colors(cx), vec![idle(), drop_target(), idle()]);

    let renders = fixture.frame(cx, |window, cx| {
        window.dispatch_event(mouse_move(leaf_center(0), Some(MouseButton::Left)), cx);
    });
    assert_eq!(renders, vec![1, 1, 0]);
    assert_eq!(fixture.colors(cx), vec![drop_target(), idle(), idle()]);
}

struct Tip;

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size(px(10.)).bg(drop_target())
    }
}

struct Tipped {
    renders: Rc<Cell<usize>>,
}

impl Render for Tipped {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .id("tipped")
            .size_full()
            .bg(idle())
            .tooltip(|_, cx| cx.new(|_| Tip).into())
    }
}

struct TippedRoot {
    tipped: Entity<Tipped>,
}

impl Render for TippedRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.tipped
                .clone()
                .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE))),
        )
    }
}

#[crate::test]
fn a_tooltip_over_a_cached_view_shows_and_hides(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, cx| TippedRoot {
        tipped: cx.new(|_| Tipped {
            renders: renders.clone(),
        }),
    });
    let shows_tip = |cx: &mut TestAppContext, event: Option<PlatformInput>| {
        cx.update_window(window.into(), |_, window, cx| {
            if let Some(event) = event {
                window.dispatch_event(event, cx);
            }
            window.draw(cx).clear(cx);
            window
                .painted_quads()
                .iter()
                .any(|quad| quad.background.as_solid() == Some(drop_target()))
        })
        .unwrap()
    };
    assert!(!shows_tip(cx, Some(mouse_move(leaf_center(0), None))));

    renders.set(0);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert!(shows_tip(cx, None));
    assert_eq!(renders.get(), 1);

    let away = point(px(SIDE * 3.), px(SIDE * 3.));
    assert!(!shows_tip(cx, Some(mouse_move(away, None))));
}

struct Pressable {
    renders: Rc<Cell<usize>>,
}

impl Render for Pressable {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().id("pressable").size_full().bg(idle())
    }
}

struct PressableRoot {
    pressable: Entity<Pressable>,
}

impl Render for PressableRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.pressable
                .clone()
                .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE))),
        )
    }
}

#[crate::test]
fn pressing_an_element_without_active_styles_rerenders_nothing(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, cx| PressableRoot {
        pressable: cx.new(|_| Pressable {
            renders: renders.clone(),
        }),
    });
    let target = leaf_center(0);
    cx.update_window(window.into(), |_, window, cx| {
        window.dispatch_event(mouse_move(target, None), cx);
        window.draw(cx).clear(cx);
    })
    .unwrap();
    renders.set(0);
    for event in [mouse_down(target), mouse_up(target)] {
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(event, cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
    }
    assert_eq!(renders.get(), 0);
}

fn color_at(window: &Window, position: Point<Pixels>) -> Option<Hsla> {
    let position = position.scale(window.scale_factor());
    window
        .painted_quads()
        .iter()
        .rev()
        .find(|quad| quad.bounds.contains(&position))
        .and_then(|quad| quad.background.as_solid())
}

fn draw<R>(
    cx: &mut TestAppContext,
    window: impl Into<AnyWindowHandle>,
    read: impl FnOnce(&Window) -> R,
) -> R {
    cx.update_window(window.into(), |_, window, cx| {
        window.draw(cx).clear(cx);
        read(window)
    })
    .unwrap()
}

struct Faded {
    opacity: f32,
    child: Entity<Leaf>,
}

impl Render for Faded {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div().opacity(self.opacity).child(
                self.child
                    .clone()
                    .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE))),
            ),
        )
    }
}

#[crate::test]
fn a_cached_view_follows_the_opacity_of_its_parent(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, cx| Faded {
        opacity: 0.25,
        child: cx.new(|_| Leaf {
            renders: renders.clone(),
            ..Leaf::default()
        }),
    });
    let alpha = |window: &Window| color_at(window, leaf_center(0)).map(|color| color.a);
    assert_eq!(draw(cx, window, alpha), Some(0.25));

    window
        .update(cx, |faded, _, cx| {
            faded.opacity = 1.;
            cx.notify();
        })
        .unwrap();
    assert_eq!(draw(cx, window, alpha), Some(1.));
    assert_eq!(renders.get(), 2);

    window.update(cx, |_, _, cx| cx.notify()).unwrap();
    assert_eq!(draw(cx, window, alpha), Some(1.));
    assert_eq!(renders.get(), 2);
}

struct RemSized;

impl Render for RemSized {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(div().size(crate::rems(1.)).bg(focused()))
    }
}

struct RemRoot {
    child: Entity<RemSized>,
}

impl Render for RemRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.child
                .clone()
                .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE))),
        )
    }
}

#[crate::test]
fn a_cached_view_follows_the_rem_size(cx: &mut TestAppContext) {
    let window = cx.add_window(|_, cx| RemRoot {
        child: cx.new(|_| RemSized),
    });
    for rem in [16., 32.] {
        window
            .update(cx, |_, window, cx| {
                window.set_rem_size(px(rem));
                cx.notify();
            })
            .unwrap();
        let painted = draw(cx, window, |window| {
            color_at(window, point(px(rem - 1.), px(rem - 1.)))
        });
        assert_eq!(painted, Some(focused()));
    }
}

struct Outer {
    renders: Rc<Cell<usize>>,
    inner: Entity<Leaf>,
}

impl Render for Outer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div().size_full().bg(hovered()).child(
            self.inner
                .clone()
                .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE))),
        )
    }
}

struct OuterRoot {
    outer: Entity<Outer>,
}

impl Render for OuterRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            self.outer
                .clone()
                .cached(StyleRefinement::default().w(px(SIDE * 2.)).h(px(SIDE * 2.))),
        )
    }
}

#[crate::test]
fn a_clean_cached_view_survives_its_cached_parent_rerendering(cx: &mut TestAppContext) {
    let outer_renders = Rc::new(Cell::new(0));
    let inner_renders = Rc::new(Cell::new(0));
    let window = cx.add_window(|_, cx| {
        let inner = cx.new(|_| Leaf {
            renders: inner_renders.clone(),
            ..Leaf::default()
        });
        OuterRoot {
            outer: cx.new(|_| Outer {
                renders: outer_renders.clone(),
                inner,
            }),
        }
    });
    let quads = |window: &Window| format!("{:?}", window.painted_quads());
    let first = draw(cx, window, quads);
    let (outer, inner) = window
        .update(cx, |root, _, cx| {
            (
                root.outer.entity_id(),
                root.outer.read(cx).inner.entity_id(),
            )
        })
        .unwrap();

    for (notified, expected) in [(outer, (1, 0)), (inner, (1, 1))] {
        outer_renders.set(0);
        inner_renders.set(0);
        cx.update(|cx| cx.notify(notified));
        assert_eq!(draw(cx, window, quads), first);
        assert_eq!((outer_renders.get(), inner_renders.get()), expected);
    }
}

struct Button {
    renders: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
    focus: FocusHandle,
}

impl Render for Button {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let clicks = self.clicks.clone();
        div()
            .id("button")
            .role(accesskit::Role::Button)
            .aria_label("Run")
            .track_focus(&self.focus)
            .size_full()
            .child("Run")
            .on_a11y_action(accesskit::Action::Click, move |_, _, _| {
                clicks.set(clicks.get() + 1)
            })
    }
}

struct Buttons {
    buttons: Vec<Entity<Button>>,
}

impl Render for Buttons {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("buttons")
            .role(accesskit::Role::Group)
            .flex()
            .size_full()
            .children(self.buttons.iter().map(|button| {
                button
                    .clone()
                    .cached(StyleRefinement::default().w(px(SIDE)).h(px(SIDE)))
            }))
    }
}

fn reachable_buttons(tree: &accesskit::TreeUpdate) -> Vec<accesskit::NodeId> {
    let nodes: std::collections::HashMap<_, _> =
        tree.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let mut stack = vec![tree.tree.as_ref().unwrap().root];
    let mut buttons = Vec::new();
    while let Some(id) = stack.pop() {
        let node = nodes[&id];
        if node.role() == accesskit::Role::Button {
            buttons.push(id);
        }
        stack.extend(node.children().iter().rev().copied());
    }
    buttons
}

#[crate::test]
fn a_reused_cached_view_keeps_its_accessibility_nodes(cx: &mut TestAppContext) {
    let renders = Rc::new(Cell::new(0));
    let clicks = Rc::new(Cell::new(0));
    let focuses: Vec<FocusHandle> = cx.update(|cx| (0..2).map(|_| cx.focus_handle()).collect());
    let focus = focuses[0].clone();
    let window = cx.add_window(|_, cx| Buttons {
        buttons: focuses
            .into_iter()
            .map(|focus| {
                cx.new(|_| Button {
                    renders: renders.clone(),
                    clicks: clicks.clone(),
                    focus,
                })
            })
            .collect(),
    });
    window
        .update(cx, |_, window, _| window.set_a11y_forced(true))
        .unwrap();
    let frame = |cx: &mut TestAppContext| {
        draw(cx, window, |window| {
            let tree = window.a11y_tree().unwrap();
            (reachable_buttons(tree), tree.focus)
        })
    };
    let (first, _) = frame(cx);
    assert_eq!(first.len(), 2);
    renders.set(0);

    let (reused, _) = frame(cx);
    assert_eq!(renders.get(), 0);
    assert_eq!(reused, first);

    cx.update_window(window.into(), |_, window, cx| {
        window.handle_a11y_action(
            accesskit::ActionRequest {
                action: accesskit::Action::Click,
                target_tree: accesskit::TreeId::ROOT,
                target_node: first[1],
                data: None,
            },
            cx,
        );
        window.handle_a11y_action(
            accesskit::ActionRequest {
                action: accesskit::Action::Focus,
                target_tree: accesskit::TreeId::ROOT,
                target_node: first[0],
                data: None,
            },
            cx,
        );
        assert!(focus.is_focused(window));
    })
    .unwrap();
    assert_eq!(clicks.get(), 1);

    let (_, focused_node) = frame(cx);
    assert_eq!(focused_node, first[0]);
    renders.set(0);
    let (reused, focused_node) = frame(cx);
    assert_eq!(renders.get(), 0);
    assert_eq!(reused, first);
    assert_eq!(focused_node, first[0]);
}
