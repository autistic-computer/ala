use std::{
    cell::RefCell,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const QUERIES: u32 = 256;
const READBACKS: usize = 4;
const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ALA_GPU_TIMES").is_ok_and(|value| !value.is_empty() && value != "0")
    })
}

pub(crate) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
    if enabled() && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        wgpu::Features::TIMESTAMP_QUERY
    } else {
        wgpu::Features::empty()
    }
}

struct Readback {
    buffer: wgpu::Buffer,
    passes: u32,
    frame: Frame,
    mapped: Arc<AtomicU8>,
    busy: bool,
}

#[derive(Clone, Copy, Default)]
struct Frame {
    at: f64,
    shaders: u64,
}

enum Clock {
    Timestamps {
        queries: wgpu::QuerySet,
        resolve: wgpu::Buffer,
        readbacks: Vec<Readback>,
        period: f64,
        submitted: Option<usize>,
    },
    Wait,
}

struct Timer {
    device: wgpu::Device,
    clock: Clock,
    passes: u32,
    frame: Frame,
    submitted: Instant,
}

thread_local! {
    static TIMER: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

fn unix_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |time| time.as_secs_f64() * 1000.0)
}

fn timer(device: &wgpu::Device, queue: &wgpu::Queue) -> Timer {
    let clock = if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        let size = u64::from(QUERIES) * 8;
        let buffer = |usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_times"),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Clock::Timestamps {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu_times"),
                ty: wgpu::QueryType::Timestamp,
                count: QUERIES,
            }),
            resolve: buffer(wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC),
            readbacks: (0..READBACKS)
                .map(|_| Readback {
                    buffer: buffer(wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST),
                    passes: 0,
                    frame: Frame::default(),
                    mapped: Arc::new(AtomicU8::new(PENDING)),
                    busy: false,
                })
                .collect(),
            period: f64::from(queue.get_timestamp_period()),
            submitted: None,
        }
    } else {
        Clock::Wait
    };
    eprintln!(
        "[ala-gpu] {}",
        match clock {
            Clock::Timestamps { .. } => "timing frames with GPU timestamps",
            Clock::Wait => "no GPU timestamps; waiting for each frame instead",
        }
    );
    Timer {
        device: device.clone(),
        clock,
        passes: 0,
        frame: Frame::default(),
        submitted: Instant::now(),
    }
}

pub(crate) fn begin_frame(device: &wgpu::Device, queue: &wgpu::Queue, shaders: u64) {
    if !enabled() {
        return;
    }
    TIMER.with_borrow_mut(|slot| {
        let timer = slot.get_or_insert_with(|| timer(device, queue));
        timer.passes = 0;
        timer.frame = Frame {
            at: unix_ms(),
            shaders,
        };
        let Clock::Timestamps {
            readbacks, period, ..
        } = &mut timer.clock
        else {
            return;
        };
        let _ = timer.device.poll(wgpu::PollType::Poll);
        for readback in readbacks.iter_mut() {
            if !readback.busy {
                continue;
            }
            let state = readback.mapped.swap(PENDING, Ordering::AcqRel);
            if state == FAILED {
                readback.busy = false;
                continue;
            }
            if state != MAPPED {
                continue;
            }
            if let Ok(view) = readback.buffer.slice(..).get_mapped_range() {
                let stamps: Vec<u64> = view
                    .chunks_exact(8)
                    .take(readback.passes as usize * 2)
                    .filter_map(|bytes| bytes.try_into().ok().map(u64::from_ne_bytes))
                    .collect();
                drop(view);
                let first = stamps.iter().step_by(2).copied().filter(|&t| t > 0).min();
                let last = stamps.iter().skip(1).step_by(2).copied().max();
                let gpu = match (first, last) {
                    (Some(first), Some(last)) if last > first && readback.passes > 1 => {
                        (last - first) as f64 * *period / 1_000_000.0
                    }
                    _ => f64::NAN,
                };
                report(readback.frame, &format!("gpu={gpu:.3}"));
            }
            readback.buffer.unmap();
            readback.busy = false;
        }
    });
}

fn report(frame: Frame, time: &str) {
    eprintln!(
        "[ala-gpu] at={:.1} {time} shaders={}",
        frame.at, frame.shaders
    );
}

pub(crate) fn pass_writes(
    stamp: &Option<(wgpu::QuerySet, u32)>,
) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    stamp
        .as_ref()
        .map(|(queries, index)| wgpu::RenderPassTimestampWrites {
            query_set: queries,
            beginning_of_pass_write_index: Some(*index),
            end_of_pass_write_index: Some(index + 1),
        })
}

pub(crate) fn next_pass() -> Option<(wgpu::QuerySet, u32)> {
    if !enabled() {
        return None;
    }
    TIMER.with_borrow_mut(|slot| {
        let timer = slot.as_mut()?;
        let Clock::Timestamps { queries, .. } = &timer.clock else {
            return None;
        };
        let index = timer.passes * 2;
        if index + 2 > QUERIES {
            return None;
        }
        timer.passes += 1;
        Some((queries.clone(), index))
    })
}

pub(crate) fn resolve(encoder: &mut wgpu::CommandEncoder, shaders: u64) {
    if !enabled() {
        return;
    }
    TIMER.with_borrow_mut(|slot| {
        let Some(timer) = slot else {
            return;
        };
        timer.frame.shaders = shaders.saturating_sub(timer.frame.shaders);
        let passes = timer.passes;
        let frame = timer.frame;
        let Clock::Timestamps {
            queries,
            resolve,
            readbacks,
            submitted,
            ..
        } = &mut timer.clock
        else {
            return;
        };
        *submitted = None;
        let Some(index) = readbacks.iter().position(|readback| !readback.busy) else {
            return;
        };
        if passes == 0 {
            return;
        }
        let readback = &mut readbacks[index];
        encoder.resolve_query_set(queries, 0..passes * 2, resolve, 0);
        encoder.copy_buffer_to_buffer(resolve, 0, &readback.buffer, 0, u64::from(passes) * 16);
        readback.passes = passes;
        readback.frame = frame;
        readback.busy = true;
        *submitted = Some(index);
    });
}

pub(crate) fn submitted() {
    if !enabled() {
        return;
    }
    TIMER.with_borrow_mut(|slot| {
        let Some(timer) = slot else {
            return;
        };
        timer.submitted = Instant::now();
        match &mut timer.clock {
            Clock::Timestamps {
                readbacks,
                submitted,
                ..
            } => {
                if let Some(index) = submitted.take() {
                    let mapped = readbacks[index].mapped.clone();
                    readbacks[index].buffer.slice(..).map_async(
                        wgpu::MapMode::Read,
                        move |result| {
                            let state = if result.is_ok() { MAPPED } else { FAILED };
                            mapped.store(state, Ordering::Release);
                        },
                    );
                }
            }
            Clock::Wait => {
                let _ = timer.device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: None,
                });
                let wait = timer.submitted.elapsed().as_secs_f64() * 1000.0;
                report(timer.frame, &format!("wait={wait:.3}"));
            }
        }
    });
}
