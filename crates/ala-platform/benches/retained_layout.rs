//! Retained Taffy kernel experiment. Stable identities are supplied by the fixture, not ALA.
#[path = "retained_layout/kernel.rs"]
mod layout;
use serde_json::{Value, json};
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

fn main() {
    RECORD_ALLOCATIONS.store(
        std::env::args().any(|arg| arg == "--count-allocations"),
        Ordering::Relaxed,
    );
    let result = if std::env::args().any(|arg| arg == "--verify") {
        json!({"case":"verification","layout":layout::verify()})
    } else {
        layout::run()
    };
    println!("{result}");
}
