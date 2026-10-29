mod clock;
mod executor;
mod test_scheduler;
#[cfg(test)]
mod tests;

pub use clock::*;
pub use executor::*;
pub use test_scheduler::*;

use async_task::Runnable;
use futures::channel::oneshot;
use std::{
    any::Any,
    future::Future,
    panic::Location,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Priority {
    RealtimeAudio,
    High,
    #[default]
    Medium,
    Low,
}

impl Priority {
    pub const fn weight(self) -> u32 {
        match self {
            Priority::High => 60,
            Priority::Medium => 30,
            Priority::Low => 10,
            Priority::RealtimeAudio => 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpawnTime(pub Instant);

#[derive(Clone, Debug)]
pub struct RunnableMeta {
    pub location: &'static Location<'static>,
    pub spawned: SpawnTime,
}

impl RunnableMeta {
    #[track_caller]
    pub fn new_with_callers_location() -> Self {
        Self {
            location: core::panic::Location::caller(),
            spawned: SpawnTime(Instant::now()),
        }
    }
}

pub trait Scheduler: Send + Sync {
    #[cfg(not(target_family = "wasm"))]
    fn block(
        &self,
        session_id: Option<SessionId>,
        future: Pin<&mut dyn Future<Output = ()>>,
        timeout: Option<Duration>,
    ) -> bool;

    fn schedule_local(&self, session_id: SessionId, runnable: Runnable<RunnableMeta>);

    fn schedule_background_with_priority(
        &self,
        runnable: Runnable<RunnableMeta>,
        priority: Priority,
    );

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>);

    fn schedule_background(&self, runnable: Runnable<RunnableMeta>) {
        self.schedule_background_with_priority(runnable, Priority::default());
    }

    #[track_caller]
    fn timer(&self, timeout: Duration) -> Timer;
    fn clock(&self) -> Arc<dyn Clock>;

    fn spawn_dedicated(
        self: Arc<Self>,
        f: Box<
            dyn FnOnce(
                    LocalExecutor,
                )
                    -> Pin<Box<dyn Future<Output = Box<dyn Any + Send + Sync>> + 'static>>
                + Send
                + 'static,
        >,
    ) -> Task<Box<dyn Any + Send + Sync>>;

    fn as_test(&self) -> Option<&TestScheduler> {
        None
    }
}

pub fn spawn_dedicated_thread<F, Fut>(
    session_id: SessionId,
    scheduler: Arc<dyn Scheduler>,
    f: F,
) -> Task<Fut::Output>
where
    F: FnOnce(LocalExecutor) -> Fut + Send + 'static,
    Fut: Future + 'static,
    Fut::Output: Send + 'static,
{
    let (task, delivery) = Task::rendezvous();
    let thread_body = move || {
        let (runnable_sender, runnable_receiver) = flume::unbounded::<Runnable<RunnableMeta>>();
        let dispatch = move |runnable: Runnable<RunnableMeta>| {
            let _ = runnable_sender.send(runnable);
        };
        let executor = LocalExecutor::new(session_id, scheduler, dispatch);
        let root_task = executor.spawn(f(executor.clone()));
        delivery.deliver(root_task);
        drop(executor);

        while let Ok(runnable) = runnable_receiver.recv() {
            runnable.run();
        }
    };
    spawn_dedicated_os_thread(session_id, thread_body);
    task
}

fn spawn_dedicated_os_thread(session_id: SessionId, thread_body: impl FnOnce() + Send + 'static) {
    let thread_name = format!("spawn_dedicated session {:?}", session_id);
    #[cfg(not(target_family = "wasm"))]
    std::thread::Builder::new()
        .name(thread_name)
        .spawn(thread_body)
        .expect("failed to spawn dedicated thread");
    #[cfg(all(target_family = "wasm", feature = "wasm-threads"))]
    wasm_thread::Builder::new()
        .name(thread_name)
        .spawn(thread_body)
        .expect("failed to spawn dedicated thread");
    #[cfg(all(target_family = "wasm", not(feature = "wasm-threads")))]
    {
        let _ = (thread_name, thread_body);
        panic!("spawn_dedicated on wasm requires the scheduler crate's `wasm-threads` feature");
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SessionId(u16);

impl SessionId {
    pub fn new(id: u16) -> Self {
        SessionId(id)
    }
}

#[derive(Clone)]
pub struct MainThreadBusy {
    epoch: Instant,
    running_since: Arc<AtomicU64>,
    queued_since: Arc<AtomicU64>,
}

impl MainThreadBusy {
    pub fn new() -> Self {
        let busy = Self {
            epoch: Instant::now(),
            running_since: Arc::default(),
            queued_since: Arc::default(),
        };
        busy.enter();
        busy
    }

    fn stamp(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_micros())
            .map_or(u64::MAX, |now| now.saturating_add(1))
    }

    pub fn enter(&self) {
        self.running_since.store(self.stamp(), Ordering::Relaxed);
    }

    pub fn leave(&self) {
        self.running_since.store(0, Ordering::Relaxed);
    }

    pub fn queue(&self) -> bool {
        self.queued_since.load(Ordering::Acquire) == 0
            && self
                .queued_since
                .compare_exchange(0, self.stamp(), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    pub fn dequeue(&self) {
        self.queued_since.store(0, Ordering::Release);
    }

    pub fn busy_for(&self) -> Duration {
        let now = self.epoch.elapsed();
        [&self.running_since, &self.queued_since]
            .into_iter()
            .map(|since| since.load(Ordering::Relaxed))
            .filter(|&since| since != 0)
            .min()
            .map_or(Duration::ZERO, |since| {
                now.saturating_sub(Duration::from_micros(since - 1))
            })
    }
}

impl Default for MainThreadBusy {
    fn default() -> Self {
        Self::new()
    }
}

#[must_use]
pub struct Timer(oneshot::Receiver<()>);

impl Timer {
    pub fn new(rx: oneshot::Receiver<()>) -> Self {
        Timer(rx)
    }

    pub fn ready() -> Self {
        let (tx, rx) = oneshot::channel();
        tx.send(()).ok();
        Timer(rx)
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
        }
    }
}
