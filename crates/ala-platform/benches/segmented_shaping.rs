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
    let mut verified_fallback_cases = 0;
    let mut first_layout_difference = None;
    let mut first_wrap_difference = None;
    let mut max_drift_case = None;
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
        let exact_layout = format!("{a:?}") == format!("{b:?}");
        layouts_equal &= exact_layout;
        if seed % 8 == 0 {
            assert!(experimental_text::segment_cuts(&text).is_none());
            assert!(exact_layout, "fallback layout differs for seed {seed}");
            verified_fallback_cases += 1;
        }
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
        let mut first_position_difference = None;
        for (ordinal, ((af, a), (bf, b))) in a_glyphs.iter().zip(&b_glyphs).enumerate() {
            identities_equal &= (af, a.id, a.index, a.is_emoji) == (bf, b.id, b.index, b.is_emoji);
            let drift = f32::from(a.position.x - b.position.x).abs();
            if drift > max_drift {
                max_drift = drift;
                max_drift_case = Some(json!({"seed":seed, "generator_seed":seed+1,
                    "text_bytes":text.len(), "glyph_ordinal":ordinal, "byte_index":a.index,
                    "baseline_x_px":f32::from(a.position.x), "segmented_x_px":f32::from(b.position.x)}));
            }
            if a.position != b.position && first_position_difference.is_none() {
                first_position_difference = Some(json!({"glyph_ordinal":ordinal,
                    "baseline_byte_index":a.index, "segmented_byte_index":b.index,
                    "baseline_font_id":format!("{af:?}"), "segmented_font_id":format!("{bf:?}"),
                    "baseline_glyph_id":format!("{:?}",a.id), "segmented_glyph_id":format!("{:?}",b.id),
                    "baseline_x_px":f32::from(a.position.x), "segmented_x_px":f32::from(b.position.x),
                    "baseline_x_bits":f32::from(a.position.x).to_bits(), "segmented_x_bits":f32::from(b.position.x).to_bits(),
                    "baseline_y_px":f32::from(a.position.y), "segmented_y_px":f32::from(b.position.y)}));
            }
            max_y_drift = max_y_drift.max(f32::from(a.position.y - b.position.y).abs());
        }
        if !exact_layout && first_layout_difference.is_none() {
            let mut cut_diagnostics = Vec::new();
            let mut start = 0;
            let mut summed_width = px(0.);
            for cut in experimental_text::segment_cuts(&text).unwrap_or_default() {
                let part = baseline.layout_line(
                    &text[start..cut],
                    px(16.),
                    &[FontRun {
                        len: cut - start,
                        font_id: font,
                    }],
                );
                let prefix = baseline.layout_line(
                    &text[..cut],
                    px(16.),
                    &[FontRun {
                        len: cut,
                        font_id: font,
                    }],
                );
                summed_width += part.width;
                cut_diagnostics.push(json!({"cut_byte":cut,
                    "native_prefix_width_px":f32::from(prefix.width), "summed_segment_width_px":f32::from(summed_width),
                    "baseline_next_glyph_x_px":a_glyphs.iter().find(|(_,g)|g.index>=cut).map(|(_,g)|f32::from(g.position.x)),
                    "segmented_next_glyph_x_px":b_glyphs.iter().find(|(_,g)|g.index>=cut).map(|(_,g)|f32::from(g.position.x))}));
                start = cut;
            }
            first_layout_difference = Some(json!({"seed":seed, "generator_seed":seed+1,
                "text":text, "text_bytes":text.len(), "font":"IBM Plex Sans", "font_size_px":16,
                "baseline_width_px":f32::from(a.width), "segmented_width_px":f32::from(b.width),
                "first_position_difference":first_position_difference, "segment_cuts":cut_diagnostics}));
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
            let a_indices = indices(&a_window);
            let b_indices = indices(&b_window);
            if a_indices != b_indices {
                wrap_differences += 1;
                if first_wrap_difference.is_none() {
                    let line = (0..a_indices.len().max(b_indices.len()))
                        .find(|&i| a_indices.get(i) != b_indices.get(i))
                        .unwrap();
                    let a_breaks = a_indices.get(line).map(Vec::as_slice).unwrap_or_default();
                    let b_breaks = b_indices.get(line).map(Vec::as_slice).unwrap_or_default();
                    let ordinal = (0..a_breaks.len().max(b_breaks.len()))
                        .find(|&i| a_breaks.get(i) != b_breaks.get(i))
                        .unwrap();
                    let a_index = a_breaks.get(ordinal).copied();
                    let b_index = b_breaks.get(ordinal).copied();
                    first_wrap_difference = Some(
                        json!({"seed":seed, "generator_seed":seed+1, "text":text,
                        "text_bytes":text.len(), "font":"IBM Plex Sans", "font_size_px":16, "wrap_width_px":width,
                        "line_ordinal":line, "boundary_ordinal":ordinal,
                        "baseline_byte_index":a_index, "segmented_byte_index":b_index,
                        "baseline_boundary_x_px":a_index.and_then(|i|a_glyphs.iter().find(|(_,g)|g.index==i).map(|(_,g)|f32::from(g.position.x))),
                        "segmented_boundary_x_px":b_index.and_then(|i|b_glyphs.iter().find(|(_,g)|g.index==i).map(|(_,g)|f32::from(g.position.x))),
                        "baseline_wrap_byte_indices":a_indices, "segmented_wrap_byte_indices":b_indices,
                        "segment_cuts":experimental_text::segment_cuts(&text)}),
                    );
                }
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
    let probe = "hello ".repeat(700);
    let runs = [FontRun {
        len: probe.len(),
        font_id: font,
    }];
    assert!(experimental_text::segment_cuts(&probe).is_some());
    system.layout_line(&probe, px(16.), &runs);
    system.reset_counts();
    system.layout_line(&probe, px(16.), &runs);
    assert_eq!(system.calls.load(Ordering::Relaxed), 0);
    let generation = system.font_generation();
    baseline
        .add_fonts(vec![std::borrow::Cow::Borrowed(include_bytes!(
            "../../ala/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
        ))])
        .unwrap();
    assert_ne!(system.font_generation(), generation);
    system.layout_line(&probe, px(16.), &runs);
    assert!(system.calls.load(Ordering::Relaxed) > 0);
    json!({"paragraphs":80, "wrap_comparisons":compared, "wrap_differences":wrap_differences,
        "max_glyph_x_drift_px":max_drift, "max_glyph_y_drift_px":max_y_drift,
        "exact_line_layouts_equivalent":layouts_equal, "fallback_cases":fallback_cases,
        "verified_fallback_cases":verified_fallback_cases, "font_invalidation":true,
        "external_font_invalidation":true, "glyph_identity_equivalent":identities_equal,
        "first_layout_difference":first_layout_difference, "first_wrap_difference":first_wrap_difference,
        "max_drift_case":max_drift_case,
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
