#![cfg(all(target_os = "linux", feature = "test-support"))]

use ala::{
    GlyphRasterData, GlyphRenderOptions, HeadlessAppContext, ShapedLine, TextAlign, TextRun,
    Window, blue, canvas, div, font, hsla, point, prelude::*, px, red, size, white,
};
use ala_platform::{current_headless_renderer, wgpu::CosmicTextSystem};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

struct Stripes;

impl Render for Stripes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .pl(px(4.))
            .child(div().w(px(8.)).h_full().bg(red()))
            .child(div().w(px(8.)).h_full().bg(white()))
            .child(div().w(px(8.)).h_full().bg(blue()))
    }
}

#[test]
fn window_screenshot_shows_the_laid_out_scene() -> anyhow::Result<()> {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(CosmicTextSystem::new_without_system_fonts("IBM Plex Sans")),
        Arc::new(()),
        current_headless_renderer,
    );
    let window = cx.open_window(size(px(32.), px(8.)), |_, cx| cx.new(|_| Stripes))?;
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;

    let image = cx.capture_screenshot(window.into())?;
    assert_eq!(image.dimensions(), (64, 16));
    for (x, expected) in [
        (3, [0, 0, 0, 255]),
        (12, [255, 0, 0, 255]),
        (23, [255, 0, 0, 255]),
        (24, [255, 255, 255, 255]),
        (39, [255, 255, 255, 255]),
        (40, [0, 0, 255, 255]),
        (55, [0, 0, 255, 255]),
        (60, [0, 0, 0, 255]),
    ] {
        assert_eq!(image.get_pixel(x, 8).0, expected, "pixel at x = {x}");
    }
    Ok(())
}

#[derive(Default)]
struct GlyphPaintTimes {
    ui_paint: Vec<Duration>,
    raster_compute: Vec<Duration>,
    raster_paint: Vec<Duration>,
    glyphs: usize,
}

struct GlyphGrid {
    times: Rc<RefCell<GlyphPaintTimes>>,
    lines: Rc<RefCell<Vec<ShapedLine>>>,
}

const GRID_COLUMNS: usize = 200;
const GRID_ROWS: usize = 60;
const GRID_LINE_HEIGHT: f32 = 18.0;

fn grid_text(row: usize) -> String {
    const ALPHABET: &[u8] =
        b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,;:-_=+()[]{}<>/|\\'\"!?@#$%^&*";
    (0..GRID_COLUMNS)
        .map(|column| ALPHABET[(row * 31 + column * 7 + column / 5) % ALPHABET.len()] as char)
        .collect()
}

impl Render for GlyphGrid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let times = self.times.clone();
        let lines = self.lines.clone();
        canvas(
            move |_, window, _| {
                let mut lines = lines.borrow_mut();
                if lines.is_empty() {
                    let mono = font("DejaVu Sans Mono");
                    for row in 0..GRID_ROWS {
                        let text = grid_text(row);
                        let runs = (0..4)
                            .map(|run| TextRun {
                                len: GRID_COLUMNS / 4,
                                font: mono.clone(),
                                color: hsla(run as f32 * 0.2, 0.6, 0.7, 1.0),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            })
                            .collect::<Vec<_>>();
                        lines.push(window.text_system().shape_line(
                            text.into(),
                            px(13.0),
                            &runs,
                            None,
                        ));
                    }
                }
                lines.clone()
            },
            move |bounds, lines: Vec<ShapedLine>, window, cx| {
                let origin =
                    |row: usize| bounds.origin + point(px(4.0), px(row as f32 * GRID_LINE_HEIGHT));
                let line_height = px(GRID_LINE_HEIGHT);
                let start = Instant::now();
                for (row, line) in lines.iter().enumerate() {
                    line.paint(origin(row), line_height, TextAlign::Left, None, window, cx)
                        .unwrap();
                }
                let ui_paint = start.elapsed();

                let start = Instant::now();
                let raster: Vec<GlyphRasterData> = lines
                    .iter()
                    .enumerate()
                    .map(|(row, line)| {
                        line.compute_glyph_raster_data(
                            origin(row),
                            line_height,
                            GlyphRenderOptions::default(),
                            window,
                            cx,
                        )
                        .unwrap()
                        .unwrap()
                    })
                    .collect();
                let raster_compute = start.elapsed();

                let start = Instant::now();
                for (row, (line, data)) in lines.iter().zip(&raster).enumerate() {
                    assert!(line.paint_with_raster_data(data, origin(row), line_height, window));
                }
                let raster_paint = start.elapsed();

                let mut times = times.borrow_mut();
                times.ui_paint.push(ui_paint);
                times.raster_compute.push(raster_compute);
                times.raster_paint.push(raster_paint);
                times.glyphs = lines
                    .iter()
                    .map(|line| line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>())
                    .sum();
            },
        )
        .size_full()
    }
}

fn per_glyph(samples: &mut [Duration], glyphs: usize) -> String {
    samples.sort_unstable();
    let at = |fraction: f64| {
        samples[((samples.len() - 1) as f64 * fraction).round() as usize].as_secs_f64() * 1e9
            / glyphs as f64
    };
    format!("p10={:.1} p50={:.1} p90={:.1}", at(0.1), at(0.5), at(0.9))
}

#[test]
#[ignore = "benchmark: cargo test -p ala-platform --all-features --release --test headless bench_glyph_paint -- --ignored --nocapture"]
fn bench_glyph_paint() -> anyhow::Result<()> {
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(CosmicTextSystem::new("DejaVu Sans")),
        Arc::new(()),
        current_headless_renderer,
    );
    let times = Rc::new(RefCell::new(GlyphPaintTimes::default()));
    let grid = {
        let times = times.clone();
        move || GlyphGrid {
            times,
            lines: Rc::default(),
        }
    };
    let window = cx.open_window(size(px(1720.), px(1100.)), |_, cx| cx.new(|_| grid()))?;
    let frames: usize = std::env::var("ALA_BENCH_FRAMES")
        .ok()
        .and_then(|frames| frames.parse().ok())
        .unwrap_or(400);
    for _ in 0..frames + 20 {
        cx.update_window(window.into(), |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        })?;
    }
    let mut times = times.borrow_mut();
    let glyphs = times.glyphs;
    let skip = 20;
    let GlyphPaintTimes {
        ui_paint,
        raster_compute,
        raster_paint,
        ..
    } = &mut *times;
    println!(
        "bench_glyph_paint glyphs={glyphs} frames={frames} ns_per_glyph ui_paint {} | raster_compute {} | raster_paint {}",
        per_glyph(&mut ui_paint[skip..], glyphs),
        per_glyph(&mut raster_compute[skip..], glyphs),
        per_glyph(&mut raster_paint[skip..], glyphs),
    );
    Ok(())
}
