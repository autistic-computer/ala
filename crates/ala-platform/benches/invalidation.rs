use std::{cell::Cell, rc::Rc, time::Duration};

use ala::{
    App, AppContext as _, BenchAppContext, BenchWindowContext, Bounds, Context, Div, Entity,
    FocusHandle, InteractiveElement as _, IntoElement, Modifiers, MouseButton, MouseDownEvent,
    MouseUpEvent, ParentElement as _, Pixels, PlatformInput, Point, Render, SharedString,
    StatefulInteractiveElement as _, StyleRefinement, Styled as _, Window, div, point,
    prelude::FluentBuilder as _, px, rgb,
};

const PANES: usize = 9;
const PANE_ROWS: usize = 24;
const PANE_COLUMNS: usize = 80;
const SIDEBAR_ROWS: usize = 25;
const SIDEBAR_WIDTH: f32 = 240.0;
const ROW_HEIGHT: f32 = 28.0;

#[derive(Default)]
struct Renders {
    views: Cell<usize>,
    terminals: Cell<usize>,
}

impl Renders {
    fn view(&self) {
        self.views.set(self.views.get() + 1);
    }
}

struct Workspace {
    sidebar: Entity<Sidebar>,
    headers: Vec<Entity<Header>>,
    terminals: Vec<Entity<Terminal>>,
    frames: Vec<Entity<PaneFrame>>,
    renders: Rc<Renders>,
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.view();
        let pane = |index: usize| {
            if let Some(frame) = self.frames.get(index) {
                return frame
                    .clone()
                    .cached(StyleRefinement::default().flex_1().min_w_0())
                    .into_any_element();
            }
            pane_frame(&self.headers[index], &self.terminals[index]).into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x15181c))
            .text_color(rgb(0xd0d4da))
            .text_sm()
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        self.sidebar
                            .clone()
                            .cached(StyleRefinement::default().w(px(SIDEBAR_WIDTH)).h_full()),
                    )
                    .child(div().flex().flex_col().flex_1().children((0..3).map(|row| {
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .children((0..3).map(|column| pane(row * 3 + column)))
                    }))),
            )
            .child(
                div()
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .px_2()
                    .child("session main  window 1  9 panes"),
            )
    }
}

fn pane_frame(header: &Entity<Header>, terminal: &Entity<Terminal>) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .border_1()
        .border_color(rgb(0x2a2f36))
        .child(
            header
                .clone()
                .cached(StyleRefinement::default().w_full().h(px(24.))),
        )
        .child(
            terminal
                .clone()
                .cached(StyleRefinement::default().w_full().flex_1()),
        )
}

struct PaneFrame {
    header: Entity<Header>,
    terminal: Entity<Terminal>,
    renders: Rc<Renders>,
}

impl Render for PaneFrame {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.view();
        pane_frame(&self.header, &self.terminal).size_full()
    }
}

struct Sidebar {
    labels: Vec<SharedString>,
    renders: Rc<Renders>,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.view();
        div()
            .id("sidebar")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1f24))
            .children(self.labels.iter().enumerate().map(|(index, label)| {
                div()
                    .id(("session", index))
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .px_2()
                    .gap_2()
                    .hover(|style| style.bg(rgb(0x262b31)))
                    .active(|style| style.bg(rgb(0x30363d)))
                    .on_click(|_, _, _| {})
                    .child(div().size(px(8.)).rounded_full().bg(rgb(0x4caf50)))
                    .child(div().flex_1().child(label.clone()))
                    .child(div().text_xs().child("3"))
            }))
    }
}

struct Header {
    title: SharedString,
    terminal_focus: FocusHandle,
    renders: Rc<Renders>,
}

impl Render for Header {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.view();
        let active = self.terminal_focus.is_focused(window);
        div()
            .id("header")
            .size_full()
            .flex()
            .items_center()
            .px_2()
            .bg(rgb(if active { 0x2d3440 } else { 0x1f242b }))
            .child(div().flex_1().child(self.title.clone()))
            .child(
                div()
                    .id("close")
                    .size(px(16.))
                    .hover(|style| style.bg(rgb(0x3a414b)))
                    .on_click(|_, _, _| {})
                    .child("x"),
            )
    }
}

struct Terminal {
    focus: FocusHandle,
    rows: Vec<SharedString>,
    renders: Rc<Renders>,
}

impl Render for Terminal {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.renders.view();
        self.renders.terminals.set(self.renders.terminals.get() + 1);
        let focused = self.focus.is_focused(window);
        div()
            .id("terminal")
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .relative()
            .text_xs()
            .children(
                self.rows
                    .iter()
                    .map(|row| div().h(px(14.)).whitespace_nowrap().child(row.clone())),
            )
            .child(
                div()
                    .absolute()
                    .top(px(14. * 3.))
                    .left(px(7. * 12.))
                    .w(px(7.))
                    .h(px(14.))
                    .when(focused, |cursor| cursor.bg(rgb(0xd0d4da)))
                    .when(!focused, |cursor| {
                        cursor.border_1().border_color(rgb(0xd0d4da))
                    }),
            )
    }
}

fn terminal_rows(pane: usize) -> Vec<SharedString> {
    (0..PANE_ROWS)
        .map(|row| {
            let line: String = (0..PANE_COLUMNS)
                .map(|column| {
                    let code = (pane * 7 + row * 13 + column * 3) % 62;
                    match code {
                        0..10 => (b'0' + code as u8) as char,
                        10..36 => (b'a' + (code - 10) as u8) as char,
                        _ => (b'A' + (code - 36) as u8) as char,
                    }
                })
                .collect();
            SharedString::from(line)
        })
        .collect()
}

struct Fixture {
    workspace: Entity<Workspace>,
    renders: Rc<Renders>,
}

fn build(window: &mut BenchWindowContext, a11y: bool, framed: bool) -> Fixture {
    let renders = Rc::new(Renders::default());
    let workspace = window.update(|window, cx| {
        window.set_a11y_forced(a11y);
        let renders = renders.clone();
        window.replace_root(cx, |_, cx| {
            let sidebar = cx.new(|_| Sidebar {
                labels: (0..SIDEBAR_ROWS)
                    .map(|index| SharedString::from(format!("session-{index:02} ~/dev/project")))
                    .collect(),
                renders: renders.clone(),
            });
            let terminals: Vec<_> = (0..PANES)
                .map(|pane| {
                    cx.new(|cx| Terminal {
                        focus: cx.focus_handle(),
                        rows: terminal_rows(pane),
                        renders: renders.clone(),
                    })
                })
                .collect();
            let headers: Vec<_> = terminals
                .iter()
                .enumerate()
                .map(|(pane, terminal)| {
                    let terminal_focus = terminal.read(cx).focus.clone();
                    cx.new(|_| Header {
                        title: SharedString::from(format!("{pane}: zsh ~/dev/project")),
                        terminal_focus,
                        renders: renders.clone(),
                    })
                })
                .collect();
            let frames = if framed {
                headers
                    .iter()
                    .zip(&terminals)
                    .map(|(header, terminal)| {
                        cx.new(|_| PaneFrame {
                            header: header.clone(),
                            terminal: terminal.clone(),
                            renders: renders.clone(),
                        })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            Workspace {
                sidebar,
                headers,
                terminals,
                frames,
                renders,
            }
        })
    });
    window.run_until_idle();
    Fixture { workspace, renders }
}

fn pane_point(window: &mut BenchWindowContext, pane: usize) -> Point<Pixels> {
    window.update(|window, _| {
        let viewport = window.viewport_size();
        let grid = Bounds::new(
            point(px(SIDEBAR_WIDTH), px(0.)),
            ala::size(
                viewport.width - px(SIDEBAR_WIDTH),
                viewport.height - px(22.),
            ),
        );
        let width = grid.size.width / 3.;
        let height = grid.size.height / 3.;
        point(
            grid.origin.x + width * ((pane % 3) as f32 + 0.5),
            grid.origin.y + height * ((pane / 3) as f32 + 0.5),
        )
    })
}

fn sidebar_point(row: usize) -> Point<Pixels> {
    point(px(SIDEBAR_WIDTH / 2.), px(ROW_HEIGHT * (row as f32 + 0.5)))
}

fn press(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    window.simulate_mouse_move(position, cx);
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
}

fn release(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

fn click(window: &mut BenchWindowContext, position: Point<Pixels>) {
    window.update(|window, cx| press(position, window, cx));
    window.run_until_idle();
    window.update(|window, cx| release(position, window, cx));
    window.run_until_idle();
}

fn report(name: &str, fixture: &Fixture, iterations: usize) {
    let iterations = iterations.max(1) as f64;
    eprintln!(
        "invalidation {name}: views_rendered_per_iteration={:.2} terminals_rendered_per_iteration={:.2} iterations={}",
        fixture.renders.views.get() as f64 / iterations,
        fixture.renders.terminals.get() as f64 / iterations,
        iterations,
    );
}

fn reset(fixture: &Fixture) {
    fixture.renders.views.set(0);
    fixture.renders.terminals.set(0);
}

#[ala::bench(fps = 60)]
fn click_sidebar_row(cx: &mut BenchAppContext) {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    let mut window = cx.add_empty_window();
    let fixture = build(&mut window, false, false);
    let position = sidebar_point(5);
    click(&mut window, position);
    reset(&fixture);
    let mut iterations = 0;
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        iterations += 1;
        click(&mut bench_window, position);
    });
    report("click_sidebar_row", &fixture, iterations);
    drop(fixture.workspace);
}

fn focus_between_panes(cx: &mut BenchAppContext, a11y: bool, name: &str) {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    let mut window = cx.add_empty_window();
    let fixture = build(&mut window, a11y, false);
    let targets = [pane_point(&mut window, 0), pane_point(&mut window, 4)];
    click(&mut window, targets[0]);
    click(&mut window, targets[1]);
    reset(&fixture);
    let mut iterations = 0;
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        let target = targets[iterations % 2];
        iterations += 1;
        click(&mut bench_window, target);
    });
    report(name, &fixture, iterations);
    drop(fixture.workspace);
}

#[ala::bench(fps = 60)]
fn click_to_focus_pane(cx: &mut BenchAppContext) {
    focus_between_panes(cx, false, "click_to_focus_pane");
}

#[ala::bench(fps = 60)]
fn click_to_focus_pane_a11y(cx: &mut BenchAppContext) {
    focus_between_panes(cx, true, "click_to_focus_pane_a11y");
}

#[ala::bench(fps = 60)]
fn keyboard_focus_pane(cx: &mut BenchAppContext) {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    let mut window = cx.add_empty_window();
    let fixture = build(&mut window, false, false);
    let handles: Vec<FocusHandle> = window.update(|_, cx| {
        let workspace = fixture.workspace.read(cx);
        [0, 4]
            .into_iter()
            .map(|pane| workspace.terminals[pane].read(cx).focus.clone())
            .collect()
    });
    window.update(|window, cx| window.focus(&handles[0], cx));
    window.run_until_idle();
    reset(&fixture);
    let mut iterations = 0;
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        let handle = handles[(iterations + 1) % 2].clone();
        iterations += 1;
        bench_window.update(|window, cx| window.focus(&handle, cx));
        bench_window.run_until_idle();
    });
    report("keyboard_focus_pane", &fixture, iterations);
    drop(fixture.workspace);
}

#[ala::bench(fps = 60)]
fn rerender_pane_frame(cx: &mut BenchAppContext) {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    let mut window = cx.add_empty_window();
    let fixture = build(&mut window, false, true);
    let frame = window.update(|_, cx| fixture.workspace.read(cx).frames[4].entity_id());
    reset(&fixture);
    let mut iterations = 0;
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        iterations += 1;
        bench_window.update(|_, cx| cx.notify(frame));
        bench_window.run_until_idle();
    });
    report("rerender_pane_frame", &fixture, iterations);
    drop(fixture.workspace);
}

ala::bench_group! {
    name = benches;
    config = criterion::Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(4))
        .without_plots();
    targets = click_sidebar_row, click_to_focus_pane, click_to_focus_pane_a11y, keyboard_focus_pane, rerender_pane_frame
}
ala::bench_main!(benches);
