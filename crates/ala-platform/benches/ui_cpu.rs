//! CPU-side UI work: no renderer, GPU submission, readback, or display server.
//! Compare hardware rendering separately with the existing headless benchmarks.
use std::{cell::Cell, rc::Rc, time::Duration};

use ala::{
    BenchAppContext, BenchReport, Context, IntoElement, ParentElement as _, Render, Styled as _,
    Window, div, px, rgb,
};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

struct Grid {
    nodes: usize,
    alternate: bool,
    renders: Rc<Cell<u64>>,
}

impl Render for Grid {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let color = rgb(if self.alternate { 0x334455 } else { 0x445566 });
        div()
            .flex()
            .flex_col()
            .children((0..self.nodes / 100).map(|_| {
                div()
                    .flex()
                    .children((0..100).map(|_| div().size(px(6.)).bg(color)))
            }))
    }
}

struct TextRows {
    rows: usize,
    revision: u64,
    churn: bool,
    renders: Rc<Cell<u64>>,
}

impl Render for TextRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        div()
            .flex()
            .flex_col()
            .text_sm()
            .children((0..self.rows).map(|row| {
                let text = if self.churn {
                    format!(
                        "row {row:04}: revision {:016} — text shaping",
                        self.revision
                    )
                } else {
                    format!("row {row:04}: terminal output — text layout and paint")
                };
                div()
                    .h(px(18.))
                    .bg(rgb(if row as u64 == self.revision % self.rows as u64 {
                        0x334455
                    } else {
                        0x112233
                    }))
                    .child(text)
            }))
    }
}

fn benchmarks(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("ui_cpu");
    for nodes in [100, 1_000, 10_000] {
        let report = BenchReport::with_fps(120);
        group.bench_with_input(
            BenchmarkId::new("grid", nodes),
            &nodes,
            |bencher, &nodes| {
                let mut cx = context(bencher, report.clone());
                let renders = Rc::new(Cell::new(0));
                let mut window = cx.add_empty_window();
                let view = window.update(|window, cx| {
                    window.replace_root(cx, |_, _| Grid {
                        nodes,
                        alternate: false,
                        renders: renders.clone(),
                    })
                });
                cx.run_until_idle();
                let before = renders.get();
                let mut iterations = 0;
                cx.bench_renderer(view, |view, _, cx| {
                    iterations += 1;
                    view.alternate = !view.alternate;
                    cx.notify();
                });
                assert_eq!(renders.get() - before, iterations);
                cx.teardown();
            },
        );
        report.print(Some("ui_cpu/grid"));
    }
    for (name, rows, churn) in [
        ("text_warm", 50, false),
        ("text_warm", 500, false),
        ("text_churn", 50, true),
    ] {
        let report = BenchReport::with_fps(120);
        group.bench_with_input(BenchmarkId::new(name, rows), &rows, |bencher, &rows| {
            let mut cx = context(bencher, report.clone());
            let renders = Rc::new(Cell::new(0));
            let mut window = cx.add_empty_window();
            let view = window.update(|window, cx| {
                window.replace_root(cx, |_, _| TextRows {
                    rows,
                    revision: 0,
                    churn,
                    renders: renders.clone(),
                })
            });
            cx.run_until_idle();
            let before = renders.get();
            let mut iterations = 0;
            cx.bench_renderer(view, |view, _, cx| {
                iterations += 1;
                view.revision += 1;
                cx.notify();
            });
            assert_eq!(renders.get() - before, iterations);
            cx.teardown();
        });
        report.print(Some("ui_cpu/text"));
    }
    group.bench_function("idle_no_redraw", |bencher| {
        let mut cx = context(bencher, BenchReport::with_fps(120));
        let renders = Rc::new(Cell::new(0));
        let mut window = cx.add_empty_window();
        window.update(|window, cx| {
            window.replace_root(cx, |_, _| Grid {
                nodes: 100,
                alternate: false,
                renders: renders.clone(),
            })
        });
        cx.run_until_idle();
        let before = renders.get();
        cx.bench_iter(|cx| {
            cx.update(|_| ());
            cx.run_until_idle();
        });
        assert_eq!(renders.get(), before, "idle work must not redraw");
        cx.teardown();
    });
    group.finish();
}

fn context<'a, 'measurement>(
    bencher: &'a mut criterion::Bencher<'measurement>,
    report: BenchReport,
) -> BenchAppContext<'a, 'measurement> {
    BenchAppContext::new_with_platform_and_report(
        ala::bench_platform(None, ala_platform::current_platform(true).text_system()),
        Some("ui_cpu"),
        bencher,
        report,
    )
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30).warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3)).without_plots();
    targets = benchmarks
}
criterion_main!(benches);
