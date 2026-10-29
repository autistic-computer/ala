#[path = "example_support/fonts.rs"]
mod example_support;

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use ala::{
    AnyWindowHandle, App, AppContext as _, AsyncApp, Bounds, Context, Modifiers, PlatformInput,
    Render, ScrollDelta, ScrollWheelEvent, Task, TouchPhase, Window, WindowBounds, WindowOptions,
    canvas, div, point, prelude::*, px, rgb, size,
};
use ala_platform::application;

const ROWS: usize = 48;
const COLUMNS: usize = 120;

#[derive(Default)]
struct Paints {
    stamps: Vec<(Instant, bool)>,
}

struct Stream {
    tick: u64,
    paints: Rc<RefCell<Paints>>,
    _task: Option<Task<()>>,
}

impl Render for Stream {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let paints = self.paints.clone();
        let tick = self.tick;
        div()
            .id("stream")
            .on_scroll_wheel(cx.listener(|this, _, _, cx| {
                this.tick += 1;
                cx.notify();
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .text_sm()
            .children((0..ROWS).map(move |row| {
                let seed = tick.wrapping_mul(31).wrapping_add(row as u64);
                let line: String = (0..COLUMNS)
                    .map(|column| {
                        let value = seed
                            .wrapping_mul(2654435761)
                            .wrapping_add(column as u64 * 97);
                        char::from(b'!' + (value % 90) as u8)
                    })
                    .collect();
                div().child(line)
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        paints
                            .borrow_mut()
                            .stamps
                            .push((Instant::now(), window.is_window_active()));
                    },
                )
                .size(px(1.)),
            )
    }
}

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0x303446))
    }
}

fn env_ms(name: &str, default: u64) -> Duration {
    Duration::from_millis(
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(default),
    )
}

struct Sample {
    at: Instant,
    voluntary: u64,
    nonvoluntary: u64,
    main_cpu_ns: u64,
    process_cpu_ns: u64,
    gpu_ns: u64,
}

fn status_field(status: &str, name: &str) -> u64 {
    status
        .lines()
        .find_map(|line| line.strip_prefix(name))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn schedstat_ns(path: &str) -> u64 {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|stat| stat.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

fn sample() -> Sample {
    let status = std::fs::read_to_string("/proc/thread-self/status").unwrap_or_default();
    let process_cpu_ns = std::fs::read_dir("/proc/self/task")
        .map(|tasks| {
            tasks
                .flatten()
                .map(|task| {
                    schedstat_ns(&format!(
                        "/proc/self/task/{}/schedstat",
                        task.file_name().to_string_lossy()
                    ))
                })
                .sum()
        })
        .unwrap_or(0);
    let gpu_ns = drm_memory()
        .as_object()
        .map(|clients| {
            clients
                .values()
                .filter_map(|client| client.get("drm-engine-render")?.as_str())
                .filter_map(|value| value.split_whitespace().next()?.parse::<u64>().ok())
                .sum()
        })
        .unwrap_or(0);
    Sample {
        at: Instant::now(),
        voluntary: status_field(&status, "voluntary_ctxt_switches:"),
        nonvoluntary: status_field(&status, "nonvoluntary_ctxt_switches:"),
        main_cpu_ns: schedstat_ns("/proc/thread-self/schedstat"),
        process_cpu_ns,
        gpu_ns,
    }
}

fn drm_memory() -> serde_json::Value {
    let mut clients = std::collections::BTreeMap::new();
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return serde_json::Value::Null;
    };
    for entry in entries.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        if !target.starts_with("/dev/dri") {
            continue;
        }
        let Ok(info) = std::fs::read_to_string(format!(
            "/proc/self/fdinfo/{}",
            entry.file_name().to_string_lossy()
        )) else {
            continue;
        };
        let mut fields = serde_json::Map::new();
        let mut client = None;
        for line in info.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim().to_string();
            if key == "drm-client-id" {
                client = Some(value.clone());
            }
            if key.starts_with("drm-") {
                fields.insert(key.to_string(), serde_json::Value::String(value));
            }
        }
        if let Some(client) = client {
            clients.insert(client, serde_json::Value::Object(fields));
        }
    }
    serde_json::json!(clients)
}

fn summarize(
    name: &str,
    stamps: &[(Instant, bool)],
    from: &Sample,
    to: &Sample,
) -> serde_json::Value {
    let window: Vec<_> = stamps
        .iter()
        .filter(|(stamp, _)| *stamp >= from.at && *stamp < to.at)
        .collect();
    let mut intervals: Vec<f64> = window
        .windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).as_secs_f64() * 1000.)
        .collect();
    intervals.sort_by(f64::total_cmp);
    let percentile = |p: f64| {
        if intervals.is_empty() {
            return 0.;
        }
        intervals[((intervals.len() - 1) as f64 * p).round() as usize]
    };
    let seconds = (to.at - from.at).as_secs_f64();
    let paints = window.len().max(1) as f64;
    let switches = (to.voluntary - from.voluntary) + (to.nonvoluntary - from.nonvoluntary);
    serde_json::json!({
        "phase": name,
        "seconds": seconds,
        "paints": window.len(),
        "fps": window.len() as f64 / seconds,
        "active_paints": window.iter().filter(|(_, active)| *active).count(),
        "interval_ms_p50": percentile(0.5),
        "interval_ms_p95": percentile(0.95),
        "interval_ms_max": intervals.last().copied().unwrap_or(0.),
        "main_switches_per_s": switches as f64 / seconds,
        "main_switches_per_paint": switches as f64 / paints,
        "main_cpu_ms_per_s": (to.main_cpu_ns - from.main_cpu_ns) as f64 / 1e6 / seconds,
        "process_cpu_ms_per_s": (to.process_cpu_ns - from.process_cpu_ns) as f64 / 1e6 / seconds,
        "gpu_ms_per_s": (to.gpu_ns - from.gpu_ns) as f64 / 1e6 / seconds,
    })
}

fn scroll(handle: AnyWindowHandle, cx: &mut AsyncApp) {
    handle
        .update(cx, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::ScrollWheel(ScrollWheelEvent {
                    position: point(px(200.), px(200.)),
                    delta: ScrollDelta::Pixels(point(px(0.), px(4.))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                }),
                cx,
            );
        })
        .ok();
}

async fn run_bursts(handle: AnyWindowHandle, paints: Rc<RefCell<Paints>>, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    let bursts = std::env::var("FRAME_PACING_BURSTS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    let burst_span = env_ms("FRAME_PACING_BURST_MS", 500);
    let tail_span = env_ms("FRAME_PACING_TAIL_MS", 1500);
    let event_interval = env_ms("FRAME_PACING_EVENT_MS", 8);
    executor.timer(env_ms("FRAME_PACING_WARMUP_MS", 1500)).await;
    for burst in 0..bursts {
        eprintln!("frame_pacing_mark {burst} idle");
        let idle_from = sample();
        executor.timer(tail_span).await;
        let idle_to = sample();
        eprintln!("frame_pacing_mark {burst} burst");
        let burst_from = sample();
        let mut events = 0;
        while burst_from.at.elapsed() < burst_span {
            scroll(handle, cx);
            events += 1;
            executor.timer(event_interval).await;
        }
        let burst_to = sample();
        eprintln!("frame_pacing_mark {burst} tail");
        executor.timer(tail_span).await;
        let tail_to = sample();
        eprintln!("frame_pacing_mark {burst} end");
        let stamps = paints.borrow().stamps.clone();
        let report = serde_json::json!({
            "burst": burst,
            "events": events,
            "phases": [
                summarize("idle", &stamps, &idle_from, &idle_to),
                summarize("burst", &stamps, &burst_from, &burst_to),
                summarize("tail", &stamps, &burst_to, &tail_to),
            ],
        });
        println!("{report}");
    }
    cx.update(|cx| cx.quit());
}

fn run_example() {
    application().run(|cx: &mut App| {
        if !example_support::load_fonts(cx) {
            return;
        }
        let tick = env_ms("FRAME_PACING_TICK_MS", 8);
        let warmup = env_ms("FRAME_PACING_WARMUP_MS", 1500);
        let active_span = env_ms("FRAME_PACING_ACTIVE_MS", 4000);
        let settle = env_ms("FRAME_PACING_SETTLE_MS", 500);
        let inactive_span = env_ms("FRAME_PACING_INACTIVE_MS", 6000);
        let input_mode = std::env::var("FRAME_PACING_MODE").as_deref() == Ok("input");
        let paints = Rc::new(RefCell::new(Paints::default()));
        let bounds = Bounds::centered(None, size(px(1400.), px(900.)), cx);
        let mut options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        if let Ok(interval) = std::env::var("FRAME_PACING_INACTIVE_INTERVAL_US") {
            options.inactive_frame_interval = interval.parse().ok().map(Duration::from_micros);
        }
        let handle = cx
            .open_window(options, |window, cx| {
                cx.new(|cx| {
                    let task = (!input_mode).then(|| {
                        cx.spawn_in(window, async move |this, cx| {
                            loop {
                                cx.background_executor().timer(tick).await;
                                if this
                                    .update(cx, |this: &mut Stream, cx| {
                                        this.tick += 1;
                                        cx.notify();
                                    })
                                    .is_err()
                                {
                                    return;
                                }
                            }
                        })
                    });
                    Stream {
                        tick: 0,
                        paints: paints.clone(),
                        _task: task,
                    }
                })
            })
            .unwrap();
        cx.activate(true);
        if input_mode {
            cx.spawn(async move |cx| run_bursts(handle.into(), paints, cx).await)
                .detach();
            return;
        }
        cx.spawn(async move |cx| {
            let executor = cx.background_executor().clone();
            executor.timer(warmup).await;
            let active_from = sample();
            executor.timer(active_span).await;
            let active_to = sample();
            let active_memory = drm_memory();
            cx.update(|cx| {
                let bounds = Bounds::centered(None, size(px(400.), px(300.)), cx);
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Blank),
                )
                .unwrap();
            });
            executor.timer(settle).await;
            let inactive_from = sample();
            executor.timer(inactive_span).await;
            let inactive_to = sample();
            let stamps = paints.borrow().stamps.clone();
            let report = serde_json::json!({
                "phases": [
                    summarize("active", &stamps, &active_from, &active_to),
                    summarize("inactive", &stamps, &inactive_from, &inactive_to),
                ],
                "drm": active_memory,
            });
            println!("{report}");
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

fn main() {
    run_example();
}
