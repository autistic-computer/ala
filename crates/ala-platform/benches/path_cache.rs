//! Repeated, cold, unique and cheap path controls; no GPU submission in timing.
#[cfg(any(target_os = "linux", target_os = "macos"))]
use ala::prelude::*;
use ala::*;
use serde_json::{Value, json};
#[cfg(any(target_os = "linux", target_os = "macos"))]
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

fn shape(seed: usize, variant: usize, workload: &str) -> PathBuilder {
    let mut builder = if workload == "dashed" || seed % 2 == 1 {
        PathBuilder::stroke(px(0.75 + (seed % 5) as f32 * 0.3))
    } else {
        PathBuilder::fill()
    };
    if workload == "dashed" {
        builder = builder.dash_array(&[px(3.), px(2.), px(1.)]);
    }
    let salt = variant as f32 * 0.00037;
    builder.move_to(point(px(2.25 + salt), px(3.125)));
    if workload == "triangle" {
        builder.line_to(point(px(31.5), px(5.125)));
        builder.line_to(point(px(15.5), px(20.25)));
    } else {
        for i in 0..16 {
            let x = 4.0 + i as f32 * 1.7;
            let y = 12.0 + ((i + seed) as f32 * 0.7).sin() * 8.0;
            builder.cubic_bezier_to(
                point(px(x + salt), px(y)),
                point(px(x - 1.2), px(y + 6.1)),
                point(px(x + 0.4), px(y - 4.3)),
            );
        }
        builder.arc_to(
            point(px(8.25), px(4.5)),
            px(27.),
            false,
            true,
            point(px(2.25), px(3.125)),
        );
    }
    if seed % 3 != 1 {
        builder.close();
    }
    if seed.is_multiple_of(7) {
        builder.rotate(seed as f32 * 0.125);
    }
    builder.scale(0.75 + (seed % 4) as f32 * 0.125);
    builder.translate(point(
        px((seed % 16) as f32 * 47. + 0.375),
        px((seed / 16 % 16) as f32 * 32. + 0.125),
    ));
    builder
}

fn batch(cache: &mut PathCache, cached: bool, workload: &str, frame: usize, count: usize) -> usize {
    if workload == "cold" {
        cache.clear();
    }
    (0..count)
        .map(|seed| {
            let builder = shape(
                seed,
                if workload == "changing" { frame } else { 0 },
                workload,
            );
            let path = if cached {
                builder.build_with_cache(cache)
            } else {
                builder.build()
            }
            .unwrap();
            let len = path.vertices.len();
            std::hint::black_box(path);
            len
        })
        .sum()
}

fn run() -> Value {
    let workload = arg("--workload", "curves");
    let cached = arg("--mode", "baseline") == "cached";
    let count = number("--paths", 128);
    let mut cache = PathCache::default();
    for frame in 0..20 {
        std::hint::black_box(batch(&mut cache, cached, &workload, frame, count));
    }
    let before = (cache.hits(), cache.misses());
    let samples: Vec<_> = (0..number("--samples", 120))
        .map(|frame| measure(|| batch(&mut cache, cached, &workload, frame + 20, count)).1)
        .collect();
    json!({"case":"paths","workload":workload,"mode":if cached{"cached"}else{"baseline"},"paths_per_sample":count,
        "hits":cache.hits()-before.0,"misses":cache.misses()-before.1,"retained_payload_bytes":cache.retained_payload_bytes(),"samples":samples})
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct Picture {
    paths: Vec<Path<Pixels>>,
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
impl Render for Picture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let paths = self.paths.clone();
        div().size_full().bg(rgb(0x182030)).child(
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    for (i, path) in paths.into_iter().enumerate() {
                        window.paint_path(
                            path,
                            rgb(if i.is_multiple_of(2) {
                                0xa0e030
                            } else {
                                0x50a0ff
                            }),
                        );
                    }
                },
            )
            .size_full(),
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn pixels(cached: bool) -> Vec<Vec<u8>> {
    #[cfg(target_os = "linux")]
    let text: Arc<dyn PlatformTextSystem> =
        Arc::new(ala_platform::wgpu::CosmicTextSystem::new_without_system_fonts("IBM Plex Sans"));
    #[cfg(target_os = "macos")]
    let text = ala_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(
        text,
        Arc::new(()),
        ala_platform::current_headless_renderer,
    );
    let window = cx
        .open_window(size(px(800.), px(600.)), |_, cx| {
            cx.new(|_| Picture { paths: vec![] })
        })
        .unwrap();
    cx.run_until_parked();
    let mut cache = PathCache::default();
    (0usize..12)
        .map(|frame| {
            let paths = (0..128)
                .map(|seed| {
                    let builder = shape(
                        seed,
                        0,
                        if frame.is_multiple_of(3) {
                            "triangle"
                        } else if frame % 3 == 1 {
                            "curves"
                        } else {
                            "dashed"
                        },
                    );
                    if cached {
                        builder.build_with_cache(&mut cache)
                    } else {
                        builder.build()
                    }
                    .unwrap()
                })
                .collect();
            window
                .update(&mut cx, |picture, _, cx| {
                    picture.paths = paths;
                    cx.notify();
                })
                .unwrap();
            cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
            cx.capture_screenshot(window.into()).unwrap().into_raw()
        })
        .collect()
}

fn verify() -> Value {
    let mut cache = PathCache::default();
    let mut comparisons = 0;
    for workload in ["triangle", "curves", "dashed"] {
        for seed in 0..256 {
            let expected = format!("{:?}", shape(seed, 0, workload).build().unwrap());
            for _ in 0..2 {
                let actual = shape(seed, 0, workload)
                    .build_with_cache(&mut cache)
                    .unwrap();
                assert_eq!(expected, format!("{actual:?}"));
                comparisons += 1;
            }
        }
    }
    assert!(cache.hits() >= 768);
    assert!(cache.retained_payload_bytes() <= 8 * 1024 * 1024);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let baseline = pixels(false);
        let candidate = pixels(true);
        assert_eq!(baseline, candidate, "native path pixels differ");
        let distinct: std::collections::BTreeSet<_> = baseline.iter().collect();
        assert_eq!(
            distinct.len(),
            3,
            "all three path scenes must render differently"
        );
    }
    json!({"case":"verification","exact_mesh_comparisons":comparisons,"cache_hits":cache.hits(),
        "retained_payload_bytes":cache.retained_payload_bytes(),"pixel_checks_available":cfg!(any(target_os="linux",target_os="macos")),
        "equal_pixels":if cfg!(any(target_os="linux",target_os="macos")){12*800*600}else{0},"distinct_scenes":3})
}
fn main() {
    RECORD_ALLOCATIONS.store(
        std::env::args().any(|a| a == "--count-allocations"),
        Ordering::Relaxed,
    );
    let value = if std::env::args().any(|a| a == "--verify") {
        verify()
    } else {
        run()
    };
    println!("{}", serde_json::to_string(&value).unwrap());
}
