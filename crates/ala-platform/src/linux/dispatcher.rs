use ala::util::ResultExt;
use calloop::{
    EventLoop, LoopHandle, PostAction,
    channel::{self, Sender},
    timer::TimeoutAction,
};

use std::{
    cell::RefCell,
    mem::MaybeUninit,
    rc::Rc,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

use ala::{
    PlatformDispatcher, Priority, PriorityQueueReceiver, PriorityQueueSender, RunnableVariant,
    profiler, scheduler::MainThreadBusy,
};

struct TimerAfter {
    duration: Duration,
    runnable: RunnableVariant,
}

pub(crate) struct LinuxDispatcher {
    id: u64,
    main_sender: PriorityQueueCalloopSender<RunnableVariant>,
    timer_sender: Sender<TimerAfter>,
    background_sender: PriorityQueueSender<RunnableVariant>,
    _background_threads: Vec<thread::JoinHandle<()>>,
    main_thread_id: thread::ThreadId,
}

const MIN_THREADS: usize = 2;

static NEXT_DISPATCHER_ID: AtomicU64 = AtomicU64::new(0);

struct MainLoopTimers {
    dispatcher: u64,
    arm: Rc<dyn Fn(Duration, RunnableVariant)>,
}

thread_local! {
    static MAIN_LOOP_TIMERS: RefCell<Option<MainLoopTimers>> = const { RefCell::new(None) };
}

pub(crate) fn stop_main_thread_timers() {
    MAIN_LOOP_TIMERS.set(None);
}

fn run_runnable(runnable: RunnableVariant) {
    let location = runnable.metadata().location;
    let spawned = runnable.metadata().spawned;
    profiler::update_running_task(spawned, location);
    runnable.run();
    profiler::save_task_timing();
}

fn arm_timer<Data>(
    handle: &LoopHandle<'static, Data>,
    duration: Duration,
    runnable: RunnableVariant,
) {
    let mut runnable = Some(runnable);
    handle
        .insert_source(
            calloop::timer::Timer::from_duration(duration),
            move |_, _, _| {
                if let Some(runnable) = runnable.take() {
                    run_runnable(runnable);
                }
                TimeoutAction::Drop
            },
        )
        .expect("Failed to start timer");
}

impl LinuxDispatcher {
    pub fn new(main_sender: PriorityQueueCalloopSender<RunnableVariant>) -> Self {
        let (background_sender, background_receiver) = PriorityQueueReceiver::new();
        let thread_count =
            std::thread::available_parallelism().map_or(MIN_THREADS, |i| i.get().max(MIN_THREADS));

        let mut background_threads = (0..thread_count)
            .map(|i| {
                let receiver: PriorityQueueReceiver<RunnableVariant> = background_receiver.clone();
                std::thread::Builder::new()
                    .name(format!("Worker-{i}"))
                    .spawn(move || {
                        for runnable in receiver.iter() {
                            run_runnable(runnable);
                        }
                    })
                    .unwrap()
            })
            .collect::<Vec<_>>();

        let (timer_sender, timer_channel) = calloop::channel::channel::<TimerAfter>();
        let timer_thread = std::thread::Builder::new()
            .name("Timer".to_owned())
            .spawn(move || {
                let mut event_loop: EventLoop<()> =
                    EventLoop::try_new().expect("Failed to initialize timer loop!");

                let handle = event_loop.handle();
                let timer_handle = event_loop.handle();
                let signal = event_loop.get_signal();
                handle
                    .insert_source(timer_channel, move |e, _, _| {
                        if let channel::Event::Closed = e {
                            signal.stop();
                        }
                        if let channel::Event::Msg(timer) = e {
                            arm_timer(&timer_handle, timer.duration, timer.runnable);
                        }
                    })
                    .expect("Failed to start timer thread");

                event_loop.run(None, &mut (), |_| {}).log_err();
            })
            .unwrap();

        background_threads.push(timer_thread);

        Self {
            id: NEXT_DISPATCHER_ID.fetch_add(1, Ordering::Relaxed),
            main_sender,
            timer_sender,
            background_sender,
            _background_threads: background_threads,
            main_thread_id: thread::current().id(),
        }
    }

    pub(crate) fn run_main_thread_timers_on<Data: 'static>(
        &self,
        handle: &LoopHandle<'static, Data>,
    ) {
        let handle = handle.clone();
        MAIN_LOOP_TIMERS.set(Some(MainLoopTimers {
            dispatcher: self.id,
            arm: Rc::new(move |duration, runnable| arm_timer(&handle, duration, runnable)),
        }));
    }

    fn arm_on_main_loop(
        &self,
        duration: Duration,
        runnable: RunnableVariant,
    ) -> Option<RunnableVariant> {
        if !self.is_main_thread() {
            return Some(runnable);
        }
        let arm = MAIN_LOOP_TIMERS
            .try_with(|timers| {
                timers
                    .borrow()
                    .as_ref()
                    .filter(|timers| timers.dispatcher == self.id)
                    .map(|timers| timers.arm.clone())
            })
            .ok()
            .flatten();
        match arm {
            Some(arm) => {
                arm(duration, runnable);
                None
            }
            None => Some(runnable),
        }
    }
}

impl PlatformDispatcher for LinuxDispatcher {
    fn is_main_thread(&self) -> bool {
        thread::current().id() == self.main_thread_id
    }

    fn dispatch(&self, runnable: RunnableVariant, priority: Priority) {
        self.background_sender
            .send(priority, runnable)
            .unwrap_or_else(|_| panic!("blocking sender returned without value"));
    }

    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority) {
        self.main_sender
            .send(priority, runnable)
            .unwrap_or_else(|runnable| {
                std::mem::forget(runnable);
            });
    }

    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant) {
        let Some(runnable) = self.arm_on_main_loop(duration, runnable) else {
            return;
        };
        if let Err(err) = self.timer_sender.send(TimerAfter { duration, runnable }) {
            std::mem::forget(err);
        }
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            let thread_id = unsafe { libc::pthread_self() };

            let policy = libc::SCHED_FIFO;
            let sched_priority = 65;

            let mut sched_param =
                unsafe { MaybeUninit::<libc::sched_param>::zeroed().assume_init() };
            sched_param.sched_priority = sched_priority;
            let result = unsafe { libc::pthread_setschedparam(thread_id, policy, &sched_param) };
            if result != 0 {
                log::warn!("failed to set realtime thread priority");
            }

            f();
        });
    }
}

pub(crate) struct BusyMarker(MainThreadBusy);

impl BusyMarker {
    pub(crate) fn new(busy: MainThreadBusy) -> Self {
        Self(busy)
    }
}

impl calloop::EventSource for BusyMarker {
    type Event = ();
    type Metadata = ();
    type Ret = ();
    type Error = std::convert::Infallible;

    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;

    fn process_events<F>(
        &mut self,
        _readiness: calloop::Readiness,
        _token: calloop::Token,
        _callback: F,
    ) -> Result<PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        Ok(PostAction::Continue)
    }

    fn register(
        &mut self,
        _poll: &mut calloop::Poll,
        _token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        Ok(())
    }

    fn reregister(
        &mut self,
        _poll: &mut calloop::Poll,
        _token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        Ok(())
    }

    fn unregister(&mut self, _poll: &mut calloop::Poll) -> calloop::Result<()> {
        Ok(())
    }

    fn before_sleep(&mut self) -> calloop::Result<Option<(calloop::Readiness, calloop::Token)>> {
        self.0.leave();
        Ok(None)
    }

    fn before_handle_events(&mut self, _events: calloop::EventIterator<'_>) {
        self.0.enter();
    }
}

pub struct PriorityQueueCalloopSender<T> {
    sender: PriorityQueueSender<T>,
    ping: calloop::ping::Ping,
    busy: MainThreadBusy,
}

impl<T> PriorityQueueCalloopSender<T> {
    fn send(&self, priority: Priority, item: T) -> Result<(), ala::queue::SendError<T>> {
        let res = self.sender.send(priority, item);
        if res.is_ok() && self.busy.queue() {
            self.ping.ping();
        }
        res
    }
}

impl<T> Drop for PriorityQueueCalloopSender<T> {
    fn drop(&mut self) {
        self.ping.ping();
    }
}

pub struct PriorityQueueCalloopReceiver<T> {
    receiver: PriorityQueueReceiver<T>,
    source: calloop::ping::PingSource,
    busy: MainThreadBusy,
}

impl<T> PriorityQueueCalloopReceiver<T> {
    pub fn new() -> (PriorityQueueCalloopSender<T>, Self) {
        let (ping, source) = calloop::ping::make_ping().expect("Failed to create a Ping.");
        let busy = MainThreadBusy::new();

        let (tx, rx) = PriorityQueueReceiver::new();

        (
            PriorityQueueCalloopSender {
                sender: tx,
                ping,
                busy: busy.clone(),
            },
            Self {
                receiver: rx,
                source,
                busy,
            },
        )
    }

    pub(crate) fn main_thread_busy(&self) -> MainThreadBusy {
        self.busy.clone()
    }
}

use calloop::channel::Event;

#[derive(Debug)]
pub struct ChannelError(calloop::ping::PingError);

impl std::fmt::Display for ChannelError {
    #[cfg_attr(feature = "nightly_coverage", coverage(off))]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::error::Error for ChannelError {
    #[cfg_attr(feature = "nightly_coverage", coverage(off))]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl<T> calloop::EventSource for PriorityQueueCalloopReceiver<T> {
    type Event = Event<T>;
    type Metadata = ();
    type Ret = ();
    type Error = ChannelError;

    fn process_events<F>(
        &mut self,
        readiness: calloop::Readiness,
        token: calloop::Token,
        mut callback: F,
    ) -> Result<calloop::PostAction, Self::Error>
    where
        F: FnMut(Self::Event, &mut Self::Metadata) -> Self::Ret,
    {
        let mut disconnected = false;

        let action = self
            .source
            .process_events(readiness, token, |(), &mut ()| {
                self.busy.dequeue();
                let receiver = self.receiver.clone();
                for runnable in receiver.try_iter() {
                    match runnable {
                        Ok(r) => callback(Event::Msg(r), &mut ()),
                        Err(_) => disconnected = true,
                    }
                }

                if disconnected {
                    callback(Event::Closed, &mut ());
                }
            })
            .map_err(ChannelError)?;

        if disconnected {
            Ok(PostAction::Remove)
        } else {
            Ok(action)
        }
    }

    fn register(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.register(poll, token_factory)
    }

    fn reregister(
        &mut self,
        poll: &mut calloop::Poll,
        token_factory: &mut calloop::TokenFactory,
    ) -> calloop::Result<()> {
        self.source.reregister(poll, token_factory)
    }

    fn unregister(&mut self, poll: &mut calloop::Poll) -> calloop::Result<()> {
        self.source.unregister(poll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ala::{BackgroundExecutor, ForegroundExecutor};
    use std::{
        collections::BTreeMap,
        sync::Arc,
        time::{Duration, Instant},
    };

    type Switches = BTreeMap<String, (u64, u64)>;

    fn context_switches() -> Switches {
        let main_tid = unsafe { libc::gettid() };
        let mut by_name = Switches::new();
        for task in std::fs::read_dir("/proc/self/task").unwrap().flatten() {
            let path = task.path();
            let Ok(status) = std::fs::read_to_string(path.join("status")) else {
                continue;
            };
            let tid: i32 = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse().ok())
                .unwrap_or_default();
            let name = if tid == main_tid {
                "main".to_owned()
            } else {
                let comm = std::fs::read_to_string(path.join("comm")).unwrap_or_default();
                comm.trim()
                    .trim_end_matches(|c: char| c.is_ascii_digit())
                    .trim_end_matches('-')
                    .to_owned()
            };
            let field = |key: &str| -> u64 {
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or_default()
            };
            let entry = by_name.entry(name).or_default();
            entry.0 += field("voluntary_ctxt_switches:");
            entry.1 += field("nonvoluntary_ctxt_switches:");
        }
        by_name
    }

    fn process_cpu() -> Duration {
        let mut usage = MaybeUninit::<libc::rusage>::zeroed();
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
        let usage = unsafe { usage.assume_init() };
        let micros = |time: libc::timeval| {
            Duration::from_secs(time.tv_sec as u64) + Duration::from_micros(time.tv_usec as u64)
        };
        micros(usage.ru_utime) + micros(usage.ru_stime)
    }

    fn report(label: &str, ops: u32, wall: Duration, cpu: Duration, before: &Switches) {
        let after = context_switches();
        let mut total = 0;
        let mut rows = Vec::new();
        for (name, (voluntary, involuntary)) in &after {
            let (voluntary0, involuntary0) = before.get(name).copied().unwrap_or_default();
            let delta = voluntary - voluntary0 + involuntary - involuntary0;
            if delta > 0 {
                total += delta;
                rows.push(format!("{name}={:.2}", delta as f64 / f64::from(ops)));
            }
        }
        println!(
            "{label}: ops={ops} wall_ns_per_op={:.0} cpu_ns_per_op={:.0} switches_per_op={:.2} [{}]",
            wall.as_nanos() as f64 / f64::from(ops),
            cpu.as_nanos() as f64 / f64::from(ops),
            total as f64 / f64::from(ops),
            rows.join(" ")
        );
    }

    fn main_loop() -> (EventLoop<'static, ()>, Arc<LinuxDispatcher>) {
        let event_loop = EventLoop::<()>::try_new().unwrap();
        let handle = event_loop.handle();
        let (sender, receiver) = PriorityQueueCalloopReceiver::new();
        let dispatcher = Arc::new(LinuxDispatcher::new(sender));
        dispatcher.run_main_thread_timers_on(&handle);
        handle
            .insert_source(BusyMarker::new(receiver.main_thread_busy()), |_, _, _| {})
            .unwrap();
        handle
            .insert_source(receiver, {
                let handle = handle.clone();
                move |event, _, _: &mut ()| {
                    if let Event::Msg(runnable) = event {
                        handle.insert_idle(|_| {
                            runnable.run();
                        });
                    }
                }
            })
            .unwrap();
        (event_loop, dispatcher)
    }

    fn run_bench(
        label: &str,
        ops: u32,
        body: impl AsyncFnOnce(ForegroundExecutor, BackgroundExecutor) + 'static,
    ) {
        let (mut event_loop, dispatcher) = main_loop();
        let foreground = ForegroundExecutor::new(dispatcher.clone());
        let background = BackgroundExecutor::new(dispatcher.clone());
        let signal = event_loop.get_signal();
        thread::sleep(Duration::from_millis(50));
        let before = context_switches();
        let cpu = process_cpu();
        let started = Instant::now();
        foreground
            .spawn({
                let foreground = foreground.clone();
                async move {
                    body(foreground, background).await;
                    signal.stop();
                }
            })
            .detach();
        event_loop.run(None, &mut (), |_| {}).unwrap();
        report(label, ops, started.elapsed(), process_cpu() - cpu, &before);
        stop_main_thread_timers();
        drop(dispatcher);
    }

    #[test]
    #[ignore = "benchmark: cargo test -p ala-platform --all-features --lib bench_dispatch -- --ignored --nocapture --test-threads=1"]
    fn bench_dispatch_timer_ticks() {
        const TICKS: u32 = 200;
        run_bench(
            "main_thread_timer_tick",
            TICKS,
            async move |_, background: BackgroundExecutor| {
                for _ in 0..TICKS {
                    background.timer(Duration::from_millis(2)).await;
                }
            },
        );
    }

    #[test]
    #[ignore = "benchmark: cargo test -p ala-platform --all-features --lib bench_dispatch -- --ignored --nocapture --test-threads=1"]
    fn bench_dispatch_runnable_round_trips() {
        const OPS: u32 = 20_000;
        run_bench(
            "main_thread_spawn_await",
            OPS,
            async move |foreground: ForegroundExecutor, _| {
                for _ in 0..OPS {
                    foreground.spawn(async {}).await;
                }
            },
        );
        run_bench(
            "background_round_trip",
            OPS,
            async move |_, background: BackgroundExecutor| {
                for _ in 0..OPS {
                    background.spawn(async {}).await;
                }
            },
        );
    }

    fn thread_recording_runnable(
        sender: std::sync::mpsc::Sender<thread::ThreadId>,
    ) -> RunnableVariant {
        let (runnable, task) = async_task::Builder::new()
            .metadata(ala::RunnableMeta::new_with_callers_location())
            .spawn(
                move |_| async move {
                    sender.send(thread::current().id()).ok();
                },
                |_| {},
            );
        task.detach();
        runnable
    }

    #[test]
    fn main_thread_timers_fire_on_the_main_loop() {
        let mut event_loop = EventLoop::<()>::try_new().unwrap();
        let (sender, _receiver) = PriorityQueueCalloopReceiver::new();
        let dispatcher = LinuxDispatcher::new(sender);
        dispatcher.run_main_thread_timers_on(&event_loop.handle());
        let (fired, fired_on) = std::sync::mpsc::channel();
        dispatcher.dispatch_after(Duration::from_millis(1), thread_recording_runnable(fired));
        assert!(fired_on.recv_timeout(Duration::from_millis(50)).is_err());
        event_loop
            .dispatch(Some(Duration::from_secs(5)), &mut ())
            .unwrap();
        assert_eq!(fired_on.try_recv(), Ok(thread::current().id()));
        stop_main_thread_timers();
    }

    #[test]
    fn timers_off_a_running_main_loop_use_the_timer_thread() {
        let event_loop = EventLoop::<()>::try_new().unwrap();
        let (sender, _receiver) = PriorityQueueCalloopReceiver::new();
        let attached = Arc::new(LinuxDispatcher::new(sender));
        attached.run_main_thread_timers_on(&event_loop.handle());
        let (sender, _receiver) = PriorityQueueCalloopReceiver::new();
        let detached = LinuxDispatcher::new(sender);
        let (fired, fired_on) = std::sync::mpsc::channel();
        let (fired_after_stop, fired_after_stop_on) = std::sync::mpsc::channel();

        detached.dispatch_after(
            Duration::from_millis(1),
            thread_recording_runnable(fired.clone()),
        );
        let timer_thread = fired_on.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(timer_thread, thread::current().id());

        thread::scope(|scope| {
            scope.spawn(|| {
                attached.dispatch_after(Duration::from_millis(1), thread_recording_runnable(fired));
            });
        });
        let timer_thread = fired_on.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(timer_thread, thread::current().id());

        stop_main_thread_timers();
        attached.dispatch_after(
            Duration::from_millis(1),
            thread_recording_runnable(fired_after_stop),
        );
        let timer_thread = fired_after_stop_on
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_ne!(timer_thread, thread::current().id());
    }

    #[test]
    fn busy_marker_is_idle_only_while_the_loop_polls() {
        let mut event_loop = EventLoop::<()>::try_new().unwrap();
        let busy = MainThreadBusy::new();
        event_loop
            .handle()
            .insert_source(BusyMarker::new(busy.clone()), |_, _, _| {})
            .unwrap();
        let in_callback = Rc::new(std::cell::Cell::new(Duration::ZERO));
        event_loop
            .handle()
            .insert_source(
                calloop::timer::Timer::from_duration(Duration::from_millis(1)),
                {
                    let busy = busy.clone();
                    let in_callback = in_callback.clone();
                    move |_, _, _| {
                        thread::sleep(Duration::from_millis(30));
                        in_callback.set(busy.busy_for());
                        TimeoutAction::Drop
                    }
                },
            )
            .unwrap();
        event_loop
            .dispatch(Some(Duration::from_secs(5)), &mut ())
            .unwrap();
        assert!(in_callback.get() >= Duration::from_millis(30));

        let watcher = thread::spawn({
            let busy = busy.clone();
            move || {
                thread::sleep(Duration::from_millis(50));
                busy.busy_for()
            }
        });
        event_loop
            .dispatch(Some(Duration::from_millis(200)), &mut ())
            .unwrap();
        assert_eq!(watcher.join().unwrap(), Duration::ZERO);
        assert!(busy.busy_for() < Duration::from_millis(50));
    }

    #[test]
    fn main_thread_work_waiting_in_the_queue_counts_as_busy() {
        let mut event_loop = EventLoop::<()>::try_new().unwrap();
        let (sender, receiver) = PriorityQueueCalloopReceiver::new();
        let busy = receiver.main_thread_busy();
        event_loop
            .handle()
            .insert_source(receiver, |_, _, _| {})
            .unwrap();
        busy.leave();
        assert_eq!(busy.busy_for(), Duration::ZERO);

        sender.send(Priority::Medium, ()).unwrap();
        thread::sleep(Duration::from_millis(30));
        assert!(busy.busy_for() >= Duration::from_millis(30));

        event_loop
            .dispatch(Some(Duration::from_secs(5)), &mut ())
            .unwrap();
        assert_eq!(busy.busy_for(), Duration::ZERO);
    }

    #[test]
    fn a_drained_batch_leaves_the_loop_asleep() {
        let mut event_loop = EventLoop::<u32>::try_new().unwrap();
        let (tx, rx) = PriorityQueueCalloopReceiver::new();
        event_loop
            .handle()
            .insert_source(rx, |event, _, received: &mut u32| {
                if let Event::Msg(()) = event {
                    *received += 1;
                }
            })
            .unwrap();
        for _ in 0..3 {
            tx.send(Priority::Medium, ()).unwrap();
        }
        let mut received = 0;
        event_loop
            .dispatch(Some(Duration::from_secs(5)), &mut received)
            .unwrap();
        assert_eq!(received, 3);

        let started = Instant::now();
        event_loop
            .dispatch(Some(Duration::from_millis(50)), &mut received)
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(40));

        tx.send(Priority::Medium, ()).unwrap();
        event_loop
            .dispatch(Some(Duration::from_secs(5)), &mut received)
            .unwrap();
        assert_eq!(received, 4);
    }

    #[test]
    fn dispatcher_threads_exit_when_owner_drops() {
        let (sender, _receiver) = PriorityQueueCalloopReceiver::new();
        let mut dispatcher = LinuxDispatcher::new(sender);
        let threads = std::mem::take(&mut dispatcher._background_threads);
        drop(dispatcher);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while threads.iter().any(|thread| !thread.is_finished())
            && std::time::Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(threads.iter().all(thread::JoinHandle::is_finished));
        for thread in threads {
            thread
                .join()
                .expect("dispatcher thread should exit cleanly");
        }
    }

    #[test]
    fn calloop_works() {
        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        let handle = event_loop.handle();

        let (tx, rx) = PriorityQueueCalloopReceiver::new();

        struct Data {
            got_msg: bool,
            got_closed: bool,
        }

        let mut data = Data {
            got_msg: false,
            got_closed: false,
        };

        let _channel_token = handle
            .insert_source(rx, move |evt, &mut (), data: &mut Data| match evt {
                Event::Msg(()) => {
                    data.got_msg = true;
                }

                Event::Closed => {
                    data.got_closed = true;
                }
            })
            .unwrap();

        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(!data.got_msg);
        assert!(!data.got_closed);

        tx.send(Priority::Medium, ()).unwrap();
        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(data.got_msg);
        assert!(!data.got_closed);

        drop(tx);
        event_loop
            .dispatch(Some(::std::time::Duration::ZERO), &mut data)
            .unwrap();

        assert!(data.got_msg);
        assert!(data.got_closed);
    }
}
