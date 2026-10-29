//! Experimental segmented shaping. CoreText/DirectWrite compatibility is unproven; --verify reports differences.
#[path = "segmented_shaping/text.rs"]
mod experimental_text;
use ala::{prelude::*, *};
use experimental_text::{ExperimentalTextSystem, paragraph};
use serde_json::{Value, json};
use std::sync::Arc;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
struct CountAlloc;
static COUNTING: AtomicBool = AtomicBool::new(false);
static RECORD_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);
#[global_allocator]
static ALLOCATOR: CountAlloc = CountAlloc;

// SAFETY: all allocation and deallocation retain System's layout and pointer contracts.
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(size as u64, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, size) }
    }
}

pub fn measure<T>(f: impl FnOnce() -> T) -> (T, Value) {
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    COUNTING.store(
        RECORD_ALLOCATIONS.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
    let cpu_start = thread_cpu_ns();
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed().as_nanos() as u64;
    let cpu_ns = thread_cpu_ns() - cpu_start;
    COUNTING.store(false, Ordering::Relaxed);
    (
        result,
        json!({
            "ns": elapsed, "cpu_ns":cpu_ns, "allocations": ALLOCS.load(Ordering::Relaxed),
            "allocated_bytes": BYTES.load(Ordering::Relaxed),
        }),
    )
}

#[cfg(unix)]
fn thread_cpu_ns() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is writable throughout this call and this clock id is supported on Linux.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) },
        0
    );
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

#[cfg(windows)]
fn thread_cpu_ns() -> u64 {
    use windows::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: valid pseudo-handle and four writable FILETIME values for this call.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
        .unwrap();
    }
    let ticks =
        |value: FILETIME| (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

pub fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args();
    while let Some(value) = args.next() {
        if value == name {
            return args.next().expect("flag needs a value");
        }
    }
    default.to_owned()
}
pub fn number(name: &str, default: usize) -> usize {
    arg(name, &default.to_string()).parse().unwrap()
}

struct Paragraph {
    text: SharedString,
}
impl Render for Paragraph {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x101010))
            .text_color(rgb(0xeeeeee))
            .child(
                div()
                    .w(px(700.))
                    .text_size(px(16.))
                    .child(self.text.clone()),
            )
    }
}
fn text() -> Value {
    let bytes = number("--bytes", 20_000);
    let workload = arg("--workload", "append");
    let segmented = arg("--mode", "baseline") == "segmented";
    let system = ExperimentalTextSystem::new(segmented);
    let mut cx = HeadlessAppContext::new(system.clone());
    let base = paragraph(bytes, 1);
    let view = cx.new(|_| Paragraph {
        text: base.clone().into(),
    });
    let window = cx
        .open_window(size(px(800.), px(600.)), {
            let view = view.clone();
            move |_, _| view
        })
        .unwrap();
    cx.run_until_parked();
    let warmup = number("--warmup", 20);
    let count = number("--samples", 120);
    let mut growing = base.clone();
    let inputs: Vec<SharedString> = (0..warmup + count)
        .map(|frame| match workload.as_str() {
            "cold" => paragraph(bytes, frame + 100).into(),
            "unchanged" => base.clone().into(),
            "middle" => {
                let mut current = base.clone();
                let mut at = current.len() / 2;
                while !current.is_char_boundary(at) {
                    at += 1;
                }
                current.insert_str(at, &format!(" edited{frame} "));
                current.into()
            }
            _ => {
                growing.push_str(&format!(" token{frame}"));
                growing.clone().into()
            }
        })
        .collect();
    let mut samples = Vec::new();
    for (frame, input) in inputs.into_iter().enumerate() {
        if frame == warmup {
            system.reset_counts();
        }
        let (_, timing) = measure(|| {
            view.update(&mut cx, |view, cx| {
                view.text = input;
                cx.notify();
            });
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        });
        if frame >= warmup {
            samples.push(timing);
        }
    }
    json!({"case":"text", "mode":if segmented {"segmented"} else {"baseline"},
        "bytes":bytes, "workload":workload, "shape_calls":system.calls.load(Ordering::Relaxed),
        "bytes_shaped":system.bytes_shaped.load(Ordering::Relaxed),
        "retained_glyphs":system.retained_glyphs(), "samples":samples})
}

fn verify_shaping() -> Value {
    let system = ExperimentalTextSystem::new(true);
    // Share the native font registry so cross-platform font IDs are comparable.
    let baseline = system.baseline();
    let font = system.font_id(&font("IBM Plex Sans")).unwrap();
    let baseline_font = baseline.font_id(&ala::font("IBM Plex Sans")).unwrap();
    assert_eq!(font, baseline_font);
    let a_window = WindowTextSystem::new(Arc::new(TextSystem::new(baseline.clone())));
    let b_window = WindowTextSystem::new(Arc::new(TextSystem::new(system.clone())));
    let mut identities_equal = true;
    let mut layouts_equal = true;
    let mut max_drift = 0.0f32;
    let mut max_y_drift = 0.0f32;
    let mut wrap_differences = 0;
    let mut compared = 0;
    let mut fallback_cases = 0;
    for seed in 0..80 {
        let mut text = paragraph(2000 + seed * 37, seed + 1);
        if seed % 8 == 0 {
            text.push_str(match (seed / 8) % 4 {
                0 => " שלום العربية",
                1 => "\tlast",
                2 => "\u{202e}RTL\u{202c}",
                _ => "\nnext",
            });
            fallback_cases += 1;
        }
        let runs = [FontRun {
            len: text.len(),
            font_id: font,
        }];
        let a = baseline.layout_line(&text, px(16.), &runs);
        let b = system.layout_line(&text, px(16.), &runs);
        layouts_equal &= format!("{a:?}") == format!("{b:?}");
        assert_eq!(a.len, b.len);
        let a_glyphs: Vec<_> = a
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(move |g| (r.font_id, g)))
            .collect();
        let b_glyphs: Vec<_> = b
            .runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(move |g| (r.font_id, g)))
            .collect();
        identities_equal &= a_glyphs.len() == b_glyphs.len();
        for ((af, a), (bf, b)) in a_glyphs.iter().zip(&b_glyphs) {
            identities_equal &= (af, a.id, a.index, a.is_emoji) == (bf, b.id, b.index, b.is_emoji);
            max_drift = max_drift.max(f32::from(a.position.x - b.position.x).abs());
            max_y_drift = max_y_drift.max(f32::from(a.position.y - b.position.y).abs());
        }
        for width in [200., 300., 500., 700., 1000.] {
            let text_runs = [TextRun {
                len: text.len(),
                font: ala::font("IBM Plex Sans"),
                color: rgb(0xffffff).into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let indices = |window: &WindowTextSystem| {
                window
                    .shape_text(
                        text.clone().into(),
                        px(16.),
                        &text_runs,
                        Some(px(width)),
                        None,
                    )
                    .unwrap()
                    .iter()
                    .map(|line| {
                        line.wrap_boundaries()
                            .iter()
                            .map(|boundary| {
                                line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            if indices(&a_window) != indices(&b_window) {
                wrap_differences += 1;
            }
            compared += 1;
        }
    }
    // The prototype must release all cached geometry when fonts change.
    assert!(system.retained_glyphs() > 0);
    system
        .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
            "../../ala/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
        ))])
        .unwrap();
    assert_eq!(system.retained_glyphs(), 0);
    json!({"paragraphs":80, "wrap_comparisons":compared, "wrap_differences":wrap_differences,
        "max_glyph_x_drift_px":max_drift, "max_glyph_y_drift_px":max_y_drift,
        "exact_line_layouts_equivalent":layouts_equal, "fallback_cases":fallback_cases,
        "font_invalidation":true, "glyph_identity_equivalent":identities_equal,
        "layout_equivalent":wrap_differences == 0 && layouts_equal,
        "pixel_equivalence":"not established for segmented text"})
}

fn main() {
    RECORD_ALLOCATIONS.store(
        std::env::args().any(|arg| arg == "--count-allocations"),
        Ordering::Relaxed,
    );
    let result = if std::env::args().any(|arg| arg == "--verify") {
        json!({"case":"verification","text":verify_shaping()})
    } else {
        text()
    };
    println!("{result}");
}
