use std::time::Duration;

use ala::{
    AnyElement, BenchAppContext, BenchWindowContext, Context, Div, FocusHandle,
    InteractiveElement as _, IntoElement, KeyBinding, KeyContext, Keymap, Keystroke, ListAlignment,
    ListState, MouseButton, ParentElement as _, PathBuilder, Pixels, Render, SharedString,
    Styled as _, Window, actions, canvas, div, list, point, px, rgb, uniform_list,
};

actions!(sweep, [Noop]);

type Build = Box<dyn Fn(&mut Window, &mut Context<Root>) -> AnyElement>;

struct Root {
    build: Build,
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.build)(window, cx)
    }
}

fn open<'a, 'm>(cx: &mut BenchAppContext<'a, 'm>) -> BenchWindowContext<'a, 'm> {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    cx.add_empty_window()
}

fn frames(
    mut window: BenchWindowContext<'_, '_>,
    build: impl Fn(&mut Window, &mut Context<Root>) -> AnyElement + 'static,
) {
    let root = window.update(|window, cx| {
        window
            .replace_root(cx, |_, _| Root {
                build: Box::new(build),
            })
            .entity_id()
    });
    window.run_until_idle();
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        bench_window.update(|_, cx| cx.notify(root));
        bench_window.run_until_idle();
    });
}

fn label(index: usize) -> SharedString {
    SharedString::from(format!("session-{index:04} workspace main branch"))
}

fn row(index: usize) -> Div {
    div()
        .h(px(24.))
        .w_full()
        .px_2()
        .flex()
        .items_center()
        .gap_2()
        .child(div().size(px(12.)).rounded_sm().bg(rgb(0x4caf50)))
        .child(label(index))
}

fn uniform(cx: &mut BenchAppContext, height: Option<Pixels>) {
    let window = open(cx);
    frames(window, move |_, _| {
        let rows = uniform_list("rows", 5000, |range, _, _| range.map(row).collect());
        match height {
            Some(height) => rows.w_full().h(height),
            None => rows.size_full(),
        }
        .into_any_element()
    });
}

#[ala::bench(fps = 60)]
fn uniform_rows_window(cx: &mut BenchAppContext) {
    uniform(cx, None);
}

#[ala::bench(fps = 60)]
fn uniform_rows_menu(cx: &mut BenchAppContext) {
    uniform(cx, Some(px(240.)));
}

#[ala::bench(fps = 60)]
fn list_rows(cx: &mut BenchAppContext) {
    let window = open(cx);
    let state = ListState::new(500, ListAlignment::Top, px(200.));
    frames(window, move |_, _| {
        list(state.clone(), |index, _, _| row(index).into_any_element())
            .size_full()
            .into_any_element()
    });
}

fn grid(cells: impl Iterator<Item = Div>) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_wrap()
        .children(cells)
        .into_any_element()
}

#[ala::bench(fps = 60)]
fn plain_divs(cx: &mut BenchAppContext) {
    let window = open(cx);
    frames(window, |_, _| {
        grid((0..3000).map(|_| div().size(px(12.)).bg(rgb(0x262b31))))
    });
}

#[ala::bench(fps = 60)]
fn cursor_divs(cx: &mut BenchAppContext) {
    let window = open(cx);
    frames(window, |_, _| {
        grid((0..3000).map(|_| div().size(px(12.)).bg(rgb(0x262b31)).cursor_pointer()))
    });
}

#[ala::bench(fps = 60)]
fn listeners(cx: &mut BenchAppContext) {
    let window = open(cx);
    frames(window, |_, cx| {
        grid((0..3000).map(|_| {
            div()
                .size(px(12.))
                .on_mouse_down(MouseButton::Left, cx.listener(|_, _, _, _| {}))
        }))
    });
}

#[ala::bench(fps = 60)]
fn text_cells(cx: &mut BenchAppContext) {
    let window = open(cx);
    let labels: Vec<SharedString> = (0..1500).map(label).collect();
    frames(window, move |_, _| {
        grid(labels.iter().map(|label| div().child(label.clone())))
    });
}

#[ala::bench(fps = 60)]
fn tab_stops(cx: &mut BenchAppContext) {
    let mut window = open(cx);
    let handles: Vec<FocusHandle> = window.update(|_, cx| {
        (0..400)
            .map(|index| cx.focus_handle().tab_index(index).tab_stop(true))
            .collect()
    });
    frames(window, move |_, _| {
        grid(
            handles
                .iter()
                .map(|handle| div().size(px(12.)).track_focus(handle)),
        )
    });
}

#[ala::bench(fps = 60)]
fn shape_lines(cx: &mut BenchAppContext) {
    let window = open(cx);
    let lines: Vec<SharedString> = (0..2000)
        .map(|index| SharedString::from(format!("{index:06} {}", "terminal output ".repeat(4))))
        .collect();
    frames(window, move |_, _| {
        let lines = lines.clone();
        canvas(
            |_, _, _| {},
            move |_, (), window, _| {
                let font_size = px(13.);
                let style = window.text_style();
                for line in lines {
                    let run = style.to_run(line.len());
                    let shaped = window
                        .text_system()
                        .shape_line(line, font_size, &[run], None);
                    std::hint::black_box(shaped.width());
                }
            },
        )
        .size_full()
        .into_any_element()
    });
}

fn polygon(index: usize) -> ala::Path<Pixels> {
    let origin = point(px((index % 30) as f32 * 40.), px((index / 30) as f32 * 40.));
    let mut builder = PathBuilder::fill();
    builder.move_to(origin + point(px(18.), px(0.)));
    for step in 1..24 {
        let angle = step as f32 * std::f32::consts::TAU / 24.;
        builder.line_to(origin + point(px(18. + 18. * angle.cos()), px(18. * angle.sin())));
    }
    builder.close();
    builder.build().expect("polygon tessellates")
}

fn paths(cx: &mut BenchAppContext, prebuilt: bool) {
    let window = open(cx);
    let built: Vec<_> = (0..300).map(polygon).collect();
    frames(window, move |_, _| {
        let built = built.clone();
        canvas(
            |_, _, _| {},
            move |_, (), window, _| {
                for (index, path) in built.into_iter().enumerate() {
                    let path = if prebuilt { path } else { polygon(index) };
                    window.paint_path(path, rgb(0x4caf50));
                }
            },
        )
        .size_full()
        .into_any_element()
    });
}

#[ala::bench(fps = 60)]
fn paths_built_per_frame(cx: &mut BenchAppContext) {
    paths(cx, false);
}

#[ala::bench(fps = 60)]
fn paths_prebuilt(cx: &mut BenchAppContext) {
    paths(cx, true);
}

const CONTEXTS: [&str; 8] = [
    "Workspace",
    "Pane",
    "Terminal",
    "Editor && mode == insert",
    "Workspace > Pane > Editor",
    "!Terminal && Pane",
    "Picker || Palette",
    "Editor && !(mode == insert) && Pane",
];

fn keymap() -> Keymap {
    let modifiers = ["cmd", "ctrl", "alt", "cmd-shift", "ctrl-shift", "ctrl-alt"];
    let keys = "abcdefghijklmnopqrstuvwxyz0123456789";
    let bindings = (0..360).map(|index| {
        let key = keys.as_bytes()[index % keys.len()] as char;
        let keystroke = format!("{}-{key}", modifiers[index % modifiers.len()]);
        KeyBinding::new(&keystroke, Noop, Some(CONTEXTS[index % CONTEXTS.len()]))
    });
    Keymap::new(bindings.collect())
}

fn context_stack() -> Vec<KeyContext> {
    [
        "Workspace",
        "Dock",
        "Pane",
        "Tabs",
        "Editor mode=insert",
        "Completion",
    ]
    .into_iter()
    .map(|source| KeyContext::parse(source).expect("context parses"))
    .collect()
}

#[ala::bench(fps = 60)]
fn keymap_lookup(cx: &mut BenchAppContext) {
    let keymap = keymap();
    let stack = context_stack();
    let input = [Keystroke::parse("ctrl-shift-k").expect("keystroke parses")];
    cx.bench_iter(|_| {
        std::hint::black_box(keymap.bindings_for_input(&input, &stack));
    });
}

ala::bench_group! {
    name = benches;
    config = criterion::Criterion::default()
        .sample_size(40)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(4))
        .without_plots();
    targets = uniform_rows_window, uniform_rows_menu, list_rows, plain_divs, cursor_divs,
        listeners, text_cells, tab_stops, shape_lines, paths_built_per_frame, paths_prebuilt,
        keymap_lookup
}
ala::bench_main!(benches);
