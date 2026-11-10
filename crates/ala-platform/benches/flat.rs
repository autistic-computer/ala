use std::time::Duration;

use ala::{
    AppContext as _, BenchAppContext, Bounds, Context, CursorStyle, Entity, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div,
    flat::{
        self, Align, BoxStyle, BoxStyleId, Extent, Flat, Flow, Insets, TextStyle, TextStyleId, Ui,
        Wrap,
    },
    px, rgb,
};

const ROWS: usize = 36;
const ROW_HEIGHT: f32 = 24.0;
const PARAGRAPHS: usize = 24;
const PARAGRAPH_WORDS: usize = 60;
const STREAM_LINES: usize = 40;
const STREAM_STEPS: usize = 240;
const LINE_WORDS: usize = 12;
const INSET: f32 = 8.0;
const BACKGROUND: u32 = 0x1b1f24;
const HOVER: u32 = 0x262b31;
const DOT: u32 = 0x4caf50;
const TEXT: u32 = 0xd0d4da;
const WORDS: [&str; 19] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    "lambda", "mu", "nu", "xi", "omicron", "pi", "rho", "sigma", "tau",
];

fn word(seed: usize, index: usize) -> &'static str {
    WORDS[(seed * 7 + index * 3) % WORDS.len()]
}

fn words(seed: usize, count: usize) -> String {
    (0..count)
        .map(|index| word(seed, index))
        .collect::<Vec<_>>()
        .join(" ")
}

struct Content {
    rows: Vec<SharedString>,
    paragraphs: Vec<SharedString>,
    version: u64,
}

fn rows() -> Content {
    Content {
        rows: (0..ROWS)
            .map(|index| SharedString::from(format!("session-{index:02} {}", words(index, 3))))
            .collect(),
        paragraphs: Vec::new(),
        version: 0,
    }
}

fn prose() -> Content {
    Content {
        rows: Vec::new(),
        paragraphs: (0..PARAGRAPHS)
            .map(|index| SharedString::from(words(index, PARAGRAPH_WORDS)))
            .collect(),
        version: 0,
    }
}

fn stream_steps() -> Vec<SharedString> {
    let mut text = (0..STREAM_LINES)
        .map(|line| words(line, LINE_WORDS))
        .collect::<Vec<_>>()
        .join("\n");
    (0..STREAM_STEPS)
        .map(|step| {
            text.push(if step % LINE_WORDS == 0 { '\n' } else { ' ' });
            text.push_str(word(STREAM_LINES, step));
            SharedString::from(text.clone())
        })
        .collect()
}

struct Tree {
    content: Entity<Content>,
}

impl Render for Tree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.read(cx);
        div()
            .id("tree")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(TEXT))
            .text_sm()
            .children(content.rows.iter().enumerate().map(|(index, label)| {
                div()
                    .id(("row", index))
                    .h(px(ROW_HEIGHT))
                    .flex()
                    .items_center()
                    .px(px(INSET))
                    .gap(px(INSET))
                    .hover(|style| style.bg(rgb(HOVER)))
                    .on_click(|_, _, _| {})
                    .child(div().size(px(INSET)).rounded_full().bg(rgb(DOT)))
                    .child(div().flex_1().child(label.clone()))
                    .child(div().text_xs().child("3"))
            }))
            .children(
                content
                    .paragraphs
                    .iter()
                    .map(|text| div().px(px(INSET)).child(text.clone())),
            )
    }
}

struct Styles {
    column: BoxStyleId,
    row: BoxStyleId,
    hovered_row: BoxStyleId,
    dot: BoxStyleId,
    label: BoxStyleId,
    paragraph: BoxStyleId,
    name: TextStyleId,
    count: TextStyleId,
    prose: TextStyleId,
}

struct Flattened {
    ui: Ui,
    styles: Styles,
    content: Entity<Content>,
    count: SharedString,
}

impl Flattened {
    fn new(content: Entity<Content>, window: &Window) -> Self {
        let mut ui = Ui::new(window);
        let font = window.text_style().font();
        let color = Hsla::from(rgb(TEXT));
        let sides = Insets {
            left: px(INSET),
            right: px(INSET),
            ..Insets::default()
        };
        let row = BoxStyle {
            flow: Flow::Row,
            width: Extent::Grow,
            height: Extent::Fixed(px(ROW_HEIGHT)),
            padding: sides,
            gap: px(INSET),
            align: Align::Center,
            ..BoxStyle::default()
        };
        let styles = Styles {
            column: ui.box_style(BoxStyle {
                width: Extent::Grow,
                height: Extent::Grow,
                fill: Some(rgb(BACKGROUND).into()),
                ..BoxStyle::default()
            }),
            row: ui.box_style(row),
            hovered_row: ui.box_style(BoxStyle {
                fill: Some(rgb(HOVER).into()),
                ..row
            }),
            dot: ui.box_style(BoxStyle {
                width: Extent::Fixed(px(INSET)),
                height: Extent::Fixed(px(INSET)),
                fill: Some(rgb(DOT).into()),
                radius: px(INSET / 2.),
                ..BoxStyle::default()
            }),
            label: ui.box_style(BoxStyle {
                flow: Flow::Row,
                width: Extent::Grow,
                ..BoxStyle::default()
            }),
            paragraph: ui.box_style(BoxStyle {
                width: Extent::Grow,
                padding: sides,
                ..BoxStyle::default()
            }),
            name: ui.text_style(TextStyle {
                font: font.clone(),
                size: px(14.),
                color,
                wrap: Wrap::Never,
            }),
            count: ui.text_style(TextStyle {
                font: font.clone(),
                size: px(12.),
                color,
                wrap: Wrap::Never,
            }),
            prose: ui.text_style(TextStyle {
                font,
                size: px(14.),
                color,
                wrap: Wrap::Words,
            }),
        };
        Self {
            ui,
            styles,
            content,
            count: SharedString::new_static("3"),
        }
    }
}

impl Flat for Flattened {
    fn ui(&mut self) -> &mut Ui {
        &mut self.ui
    }

    fn build(&mut self, bounds: Bounds<Pixels>, _: &mut Window, cx: &mut Context<Self>) {
        let Self {
            ui,
            styles,
            content,
            count,
        } = self;
        let content = content.read(cx);
        let hovered = ui.hovered();
        ui.begin();
        ui.open(styles.column);
        for (index, label) in content.rows.iter().enumerate() {
            let key = index as u64;
            let row = ui.open(if hovered == Some(key) {
                styles.hovered_row
            } else {
                styles.row
            });
            ui.target(row, key, CursorStyle::Arrow);
            ui.open(styles.dot);
            ui.close();
            ui.open(styles.label);
            ui.text(key, 0, styles.name, label);
            ui.close();
            ui.text(key, 0, styles.count, count);
            ui.close();
        }
        for (index, text) in content.paragraphs.iter().enumerate() {
            ui.open(styles.paragraph);
            ui.text(index as u64, content.version, styles.prose, text);
            ui.close();
        }
        ui.close();
        ui.layout(bounds.size);
    }
}

struct Host {
    view: Entity<Flattened>,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        flat::surface("flat", &self.view).size_full()
    }
}

#[derive(Clone, Copy)]
enum Layer {
    Tree,
    Flat,
}

fn rerender(
    cx: &mut BenchAppContext,
    name: &str,
    layer: Layer,
    content: Content,
    steps: &[SharedString],
) {
    assert!(
        !cfg!(debug_assertions),
        "build the bench with optimizations"
    );
    let mut window = cx.add_empty_window();
    let (content, root) = window.update(|window, cx| {
        let content = cx.new(|_| content);
        let root = match layer {
            Layer::Tree => window
                .replace_root(cx, |_, _| Tree {
                    content: content.clone(),
                })
                .entity_id(),
            Layer::Flat => window
                .replace_root(cx, |window, cx| Host {
                    view: cx.new(|_| Flattened::new(content.clone(), window)),
                })
                .entity_id(),
        };
        (content, root)
    });
    window.run_until_idle();
    let (quads, glyphs) = window.update(|window, _| {
        let scene = window.rendered_scene();
        (
            scene.quads.len(),
            scene.monochrome_sprites.len() + scene.subpixel_sprites.len(),
        )
    });
    eprintln!("flat {name}: quads={quads} glyphs={glyphs}");
    assert!(
        glyphs > 0,
        "no glyphs painted: add the font-kit feature on macOS or wayland on Linux"
    );
    let mut step = 0;
    let mut bench_window = window.clone();
    window.app_context().bench_iter(|_| {
        bench_window.update(|_, cx| {
            if !steps.is_empty() {
                let text = steps[step % steps.len()].clone();
                content.update(cx, |content, _| {
                    content.paragraphs[0] = text;
                    content.version += 1;
                });
            }
            cx.notify(root);
        });
        step += 1;
        bench_window.run_until_idle();
    });
}

fn stream(cx: &mut BenchAppContext, name: &str, layer: Layer) {
    let steps = stream_steps();
    let content = Content {
        rows: Vec::new(),
        paragraphs: vec![steps[0].clone()],
        version: 0,
    };
    rerender(cx, name, layer, content, &steps);
}

#[ala::bench(fps = 60)]
fn rows_tree(cx: &mut BenchAppContext) {
    rerender(cx, "rows_tree", Layer::Tree, rows(), &[]);
}

#[ala::bench(fps = 60)]
fn rows_flat(cx: &mut BenchAppContext) {
    rerender(cx, "rows_flat", Layer::Flat, rows(), &[]);
}

#[ala::bench(fps = 60)]
fn prose_tree(cx: &mut BenchAppContext) {
    rerender(cx, "prose_tree", Layer::Tree, prose(), &[]);
}

#[ala::bench(fps = 60)]
fn prose_flat(cx: &mut BenchAppContext) {
    rerender(cx, "prose_flat", Layer::Flat, prose(), &[]);
}

#[ala::bench(fps = 60)]
fn stream_tree(cx: &mut BenchAppContext) {
    stream(cx, "stream_tree", Layer::Tree);
}

#[ala::bench(fps = 60)]
fn stream_flat(cx: &mut BenchAppContext) {
    stream(cx, "stream_flat", Layer::Flat);
}

ala::bench_group! {
    name = benches;
    config = criterion::Criterion::default()
        .sample_size(30)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(4))
        .without_plots();
    targets = rows_tree, rows_flat, prose_tree, prose_flat, stream_tree, stream_flat
}
ala::bench_main!(benches);
