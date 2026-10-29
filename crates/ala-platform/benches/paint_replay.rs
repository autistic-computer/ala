//! Manually driven opacity replay: CPU timing, scene/input checks, and native pixel checks where available.
use ala::{prelude::*, *};
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
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

fn native_text_system() -> Arc<dyn PlatformTextSystem> {
    #[cfg(target_os = "linux")]
    let system: Arc<dyn PlatformTextSystem> =
        Arc::new(ala_platform::wgpu::CosmicTextSystem::new_without_system_fonts("IBM Plex Sans"));
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let system = ala_platform::current_platform(true).text_system();
    system
        .add_fonts(vec![
            std::borrow::Cow::Borrowed(include_bytes!(
                "../../ala/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
            )),
            std::borrow::Cow::Borrowed(include_bytes!(
                "../../ala/assets/fonts/lilex/Lilex-Regular.ttf"
            )),
        ])
        .unwrap();
    system
}
fn context(pixels: bool) -> HeadlessAppContext {
    if pixels {
        HeadlessAppContext::with_platform(
            native_text_system(),
            Arc::new(()),
            ala_platform::current_headless_renderer,
        )
    } else {
        HeadlessAppContext::new(native_text_system())
    }
}
struct OpacityElement<E> {
    element: Option<E>,
    value: Rc<Cell<f32>>,
}
impl<E: IntoElement + 'static> IntoElement for OpacityElement<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl<E: IntoElement + 'static> Element for OpacityElement<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some("experimental-opacity".into())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let mut element = self.element.take().unwrap().into_any_element();
        (element.request_layout(window, cx), element)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_experimental_paint_opacity(self.value.clone(), |window| {
            element.paint(window, cx)
        });
    }
}

struct Pane {
    rows: Vec<SharedString>,
    opacity: Rc<Cell<f32>>,
    renders: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
    replay: bool,
    mixed: bool,
    nested: bool,
}
impl Render for Pane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let spinner =
            div()
                .size(px(16.))
                .rounded_full()
                .bg(rgb(0x4080ff))
                .when(self.mixed, |spinner| {
                    spinner
                        .w(px(140.))
                        .h(px(24.))
                        .border_1()
                        .border_color(rgb(0xffa020))
                        .text_color(rgb(0xffffff))
                        .child("Animated Aa 123")
                });
        let spinner = if self.replay {
            let spinner = if self.nested {
                OpacityElement {
                    element: Some(spinner),
                    value: self.opacity.clone(),
                }
                .into_any_element()
            } else {
                spinner.into_any_element()
            };
            OpacityElement {
                element: Some(spinner),
                value: self.opacity.clone(),
            }
            .into_any_element()
        } else {
            let opacity = self.opacity.get();
            spinner
                .opacity(if self.nested {
                    opacity * opacity
                } else {
                    opacity
                })
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x101010))
            .text_color(rgb(0xdddddd))
            .child(spinner)
            .children(self.rows.iter().enumerate().map(|(ix, text)| {
                let clicks = self.clicks.clone();
                div()
                    .id(ix)
                    .h(px(24.))
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(0x303030))
                    .on_click(move |_, _, _| clicks.set(clicks.get() + 1))
                    .child(text.clone())
            }))
    }
}
struct Root {
    pane: Entity<Pane>,
    extra: bool,
}
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(self.extra, |root| {
                root.child(div().h(px(19.)).bg(rgb(0x223344)))
            })
            .child(
                self.pane
                    .clone()
                    .cached(StyleRefinement::default().w_full().flex_1()),
            )
    }
}
struct AnimationFixture {
    window: WindowHandle<Root>,
    pane: Entity<Pane>,
    opacity: Rc<Cell<f32>>,
    renders: Rc<Cell<usize>>,
    clicks: Rc<Cell<usize>>,
    cx: HeadlessAppContext,
}
impl AnimationFixture {
    fn new(replay: bool, rows: usize, pixels: bool) -> Self {
        let mut cx = context(pixels);
        let opacity = Rc::new(Cell::new(1.0));
        let renders = Rc::new(Cell::new(0));
        let clicks = Rc::new(Cell::new(0));
        let pane = cx.new(|_| Pane {
            rows: (0..rows)
                .map(|ix| format!("Setting {ix}: render the same text and controls").into())
                .collect(),
            opacity: opacity.clone(),
            renders: renders.clone(),
            clicks: clicks.clone(),
            replay,
            mixed: false,
            nested: false,
        });
        let window = cx
            .open_window(size(px(800.), px(600.)), {
                let pane = pane.clone();
                move |_, cx| cx.new(|_| Root { pane, extra: false })
            })
            .unwrap();
        cx.run_until_parked();
        let mut fixture = Self {
            cx,
            window,
            pane,
            opacity,
            renders,
            clicks,
        };
        fixture.draw(false);
        fixture
    }
    fn draw(&mut self, rebuild: bool) {
        if rebuild {
            self.pane.update(&mut self.cx, |_, cx| cx.notify());
        }
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                window.draw(cx).clear(cx)
            })
            .unwrap();
    }
    fn step(&mut self, frame: usize, rebuild: bool) {
        self.opacity.set(0.1 + (frame % 91) as f32 / 100.);
        self.draw(rebuild);
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn screenshot(&mut self) -> image::RgbaImage {
        self.cx.capture_screenshot(self.window.into()).unwrap()
    }
}

fn animation() -> Value {
    let rows = number("--rows", 480);
    let replay = arg("--mode", "baseline") == "replay";
    let mut fixture = AnimationFixture::new(replay, rows, false);
    for frame in 0..number("--warmup", 30) {
        fixture.step(frame, !replay);
    }
    fixture.renders.set(0);
    let samples: Vec<_> = (0..number("--samples", 300))
        .map(|frame| measure(|| fixture.step(frame + 30, !replay)).1)
        .collect();
    json!({"case":"animation", "mode":if replay {"replay"} else {"baseline"},
        "rows":rows, "pane_renders":fixture.renders.get(), "samples":samples})
}

fn verify() -> Value {
    let mut baseline = AnimationFixture::new(false, 120, true);
    let mut replay = AnimationFixture::new(true, 120, true);
    #[allow(unused_mut)]
    let mut compared = 0u64;
    let mut scene_comparisons = 0;
    let mut distinct_scenes = std::collections::BTreeSet::new();
    #[allow(unused_mut)]
    let mut distinct_frames: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for frame in 0..24 {
        if [4, 12, 20].contains(&frame) {
            for fixture in [&mut baseline, &mut replay] {
                fixture.pane.update(&mut fixture.cx, |pane, cx| {
                    pane.mixed = frame < 20;
                    pane.nested = frame == 12;
                    cx.notify();
                });
            }
        }
        if frame == 8 || frame == 16 {
            for fixture in [&mut baseline, &mut replay] {
                fixture
                    .window
                    .update(&mut fixture.cx, |root, _, cx| {
                        root.extra = !root.extra;
                        cx.notify();
                    })
                    .unwrap();
            }
        }
        baseline.step(frame * 7, true);
        replay.step(frame * 7, false);
        let a_scene = baseline
            .cx
            .update_window(baseline.window.into(), |_, window, _| {
                window.experimental_frame_snapshot()
            })
            .unwrap();
        let b_scene = replay
            .cx
            .update_window(replay.window.into(), |_, window, _| {
                window.experimental_frame_snapshot()
            })
            .unwrap();
        assert_eq!(
            a_scene, b_scene,
            "painted scene or hitboxes differ at frame {frame}"
        );
        {
            use std::hash::{Hash, Hasher};
            let mut hash = ala::collections::FxHasher::default();
            a_scene.hash(&mut hash);
            distinct_scenes.insert(hash.finish());
        }
        scene_comparisons += 1;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let a = baseline.screenshot();
            let b = replay.screenshot();
            assert_eq!(a.dimensions(), b.dimensions());
            assert_eq!(
                a.as_raw(),
                b.as_raw(),
                "animation pixels differ at frame {frame}"
            );
            {
                use std::hash::{Hash, Hasher};
                let mut hash = ala::collections::FxHasher::default();
                a.as_raw().hash(&mut hash);
                distinct_frames.insert(hash.finish());
            }
            compared += a.width() as u64 * a.height() as u64;
        }
    }
    assert!(
        distinct_scenes.len() >= 12,
        "scene must change while the animation runs"
    );
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    assert!(
        distinct_frames.len() >= 12,
        "the animation must actually change the rendered pixels"
    );
    for fixture in [&mut baseline, &mut replay] {
        fixture
            .cx
            .update_window(fixture.window.into(), |_, window, cx| {
                window.dispatch_event(
                    PlatformInput::MouseDown(MouseDownEvent {
                        button: MouseButton::Left,
                        position: point(px(40.), px(28.)),
                        modifiers: Modifiers::default(),
                        click_count: 1,
                        first_mouse: false,
                    }),
                    cx,
                );
                window.dispatch_event(
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position: point(px(40.), px(28.)),
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    }),
                    cx,
                );
            })
            .unwrap();
        assert_eq!(
            fixture.clicks.get(),
            1,
            "cached row click listener must survive replay"
        );
    }
    json!({"case":"verification", "animation_frames":24, "equal_pixels":compared,
        "distinct_animation_images":distinct_frames.len(),
        "scene_comparisons":scene_comparisons, "distinct_scenes":distinct_scenes.len(),
        "pixel_checks_available":cfg!(any(target_os="macos",target_os="linux")), "cached_input_checks":2})
}

struct LiveRoot {
    pane: Entity<Pane>,
    opacity: Rc<Cell<f32>>,
    mode: String,
    started: Instant,
    seconds: f32,
}

impl Render for LiveRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.started.elapsed().as_secs_f32() >= self.seconds {
            cx.quit();
        } else if self.mode != "static" {
            let phase = self.opacity.clone();
            let pane = self.pane.clone();
            let started = self.started;
            let rebuild = self.mode == "baseline";
            window.on_next_frame(move |_, cx| {
                phase.set(0.1 + (started.elapsed().as_secs_f32() % 1.0) * 0.9);
                if rebuild {
                    pane.update(cx, |_, cx| cx.notify());
                }
            });
            window.request_animation_frame();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x101010))
            .text_color(rgb(0xeeeeee))
            .child(
                div()
                    .h(px(28.))
                    .child(format!("Opacity replay experiment: {}", self.mode)),
            )
            .child(
                self.pane
                    .clone()
                    .cached(StyleRefinement::default().w_full().flex_1()),
            )
    }
}

fn live() {
    let mode = arg("--mode", "baseline");
    let seconds = number("--seconds", 45) as f32;
    let rows = number("--rows", 480);
    ala_platform::application().run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../../ala/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
                )),
                std::borrow::Cow::Borrowed(include_bytes!(
                    "../../ala/assets/fonts/lilex/Lilex-Regular.ttf"
                )),
            ])
            .unwrap();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let lifetime = std::time::Duration::from_secs_f32(seconds);
        cx.spawn(async move |cx| {
            cx.background_executor().timer(lifetime).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(800.), px(600.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..WindowOptions::default()
            },
            move |_, cx| {
                let opacity = Rc::new(Cell::new(1.0));
                let pane = cx.new(|_| Pane {
                    rows: (0..rows)
                        .map(|row| {
                            format!("Setting {row}: render the same text and controls").into()
                        })
                        .collect(),
                    opacity: opacity.clone(),
                    renders: Rc::new(Cell::new(0)),
                    clicks: Rc::new(Cell::new(0)),
                    replay: mode == "replay",
                    mixed: true,
                    nested: false,
                });
                cx.new(|_| LiveRoot {
                    pane,
                    opacity,
                    mode,
                    started: Instant::now(),
                    seconds,
                })
            },
        )
        .unwrap();
    });
}

fn main() {
    if std::env::args().any(|arg| arg == "--live") {
        live();
        return;
    }
    RECORD_ALLOCATIONS.store(
        std::env::args().any(|arg| arg == "--count-allocations"),
        Ordering::Relaxed,
    );
    let result = if std::env::args().any(|arg| arg == "--verify") {
        verify()
    } else {
        animation()
    };
    println!("{result}");
}
