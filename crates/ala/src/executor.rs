use crate::scheduler::Instant;
use crate::scheduler::Scheduler;
use crate::util::{TryFutureExt, TryFutureExtBacktrace};
use crate::{ActivityGuard, App, PlatformDispatcher, PlatformScheduler};
#[cfg(not(target_family = "wasm"))]
use futures::channel::mpsc;
use futures::prelude::*;
use std::{future::Future, marker::PhantomData, rc::Rc, sync::Arc, time::Duration};
#[cfg(not(target_family = "wasm"))]
use std::{mem, pin::Pin};

pub use crate::scheduler::{
    DedicatedExecutor, FallibleTask, LocalExecutor as SchedulerLocalExecutor, Priority, Task, Timer,
};

#[derive(Clone)]
pub struct BackgroundExecutor {
    inner: crate::scheduler::BackgroundExecutor,
    dispatcher: Arc<dyn PlatformDispatcher>,
}

#[derive(Clone)]
pub struct ForegroundExecutor {
    inner: crate::scheduler::LocalExecutor,
    dispatcher: Arc<dyn PlatformDispatcher>,
    #[cfg(feature = "profiler")]
    foreground_runnables: Option<crate::profiler::journal::ForegroundRunnableCounter>,
    not_send: PhantomData<Rc<()>>,
}

pub trait TaskExt<T, E> {
    fn detach_and_log_err(self, cx: &App);
    fn detach_and_log_err_with_backtrace(self, cx: &App);
}

impl<T, E> TaskExt<T, E> for Task<Result<T, E>>
where
    T: 'static,
    E: 'static + std::fmt::Display + std::fmt::Debug,
{
    #[track_caller]
    fn detach_and_log_err(self, cx: &App) {
        let location = core::panic::Location::caller();
        cx.foreground_executor()
            .spawn(self.log_tracked_err(*location))
            .detach();
    }

    #[track_caller]
    fn detach_and_log_err_with_backtrace(self, cx: &App) {
        let location = *core::panic::Location::caller();
        cx.foreground_executor()
            .spawn(self.log_tracked_err_with_backtrace(location))
            .detach();
    }
}

impl BackgroundExecutor {
    pub fn new(dispatcher: Arc<dyn PlatformDispatcher>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        let scheduler: Arc<dyn Scheduler> = if let Some(test_dispatcher) = dispatcher.as_test() {
            test_dispatcher.scheduler().clone()
        } else {
            Arc::new(PlatformScheduler::new(dispatcher.clone()))
        };

        #[cfg(not(any(test, feature = "test-support")))]
        let scheduler: Arc<dyn Scheduler> = Arc::new(PlatformScheduler::new(dispatcher.clone()));

        Self {
            inner: crate::scheduler::BackgroundExecutor::new(scheduler),
            dispatcher,
        }
    }

    pub fn scheduler_executor(&self) -> crate::scheduler::BackgroundExecutor {
        self.inner.clone()
    }

    pub fn prevent_app_nap(&self, reason: &str) -> ActivityGuard {
        self.dispatcher.prevent_app_nap(reason)
    }

    #[track_caller]
    pub fn spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.spawn_with_priority(Priority::default(), future.boxed())
    }

    #[track_caller]
    pub fn spawn_with_priority<R>(
        &self,
        priority: Priority,
        future: impl Future<Output = R> + Send + 'static,
    ) -> Task<R>
    where
        R: Send + 'static,
    {
        if priority == Priority::RealtimeAudio {
            self.inner.spawn_realtime(future)
        } else {
            self.inner.spawn_with_priority(priority, future)
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub async fn scoped<'scope, F>(&self, scheduler: F)
    where
        F: FnOnce(&mut Scope<'scope>),
    {
        let mut scope = Scope::new(self.clone(), Priority::default());
        (scheduler)(&mut scope);
        let spawned = mem::take(&mut scope.futures)
            .into_iter()
            .map(|f| self.spawn_with_priority(scope.priority, f))
            .collect::<Vec<_>>();
        for task in spawned {
            task.await;
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub async fn scoped_priority<'scope, F>(&self, priority: Priority, scheduler: F)
    where
        F: FnOnce(&mut Scope<'scope>),
    {
        let mut scope = Scope::new(self.clone(), priority);
        (scheduler)(&mut scope);
        let spawned = mem::take(&mut scope.futures)
            .into_iter()
            .map(|f| self.spawn_with_priority(scope.priority, f))
            .collect::<Vec<_>>();
        for task in spawned {
            task.await;
        }
    }

    pub fn now(&self) -> Instant {
        self.inner.scheduler().clock().now()
    }

    #[track_caller]
    pub fn timer(&self, duration: Duration) -> Timer {
        if duration.is_zero() {
            return Timer::ready();
        }
        self.inner.scheduler().timer(duration)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn simulate_random_delay(&self) -> impl Future<Output = ()> + use<> {
        self.dispatcher.as_test().unwrap().simulate_random_delay()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn advance_clock(&self, duration: Duration) {
        self.dispatcher.as_test().unwrap().advance_clock(duration)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn tick(&self) -> bool {
        self.dispatcher.as_test().unwrap().scheduler().tick()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn run_until_parked(&self) {
        let scheduler = self.dispatcher.as_test().unwrap().scheduler();
        scheduler.run();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn allow_parking(&self) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .allow_parking();

        if std::env::var("ALA_RUN_UNTIL_PARKED_LOG").ok().as_deref() == Some("1") {
            log::warn!("[ala::executor] allow_parking: enabled");
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_block_on_ticks(&self, range: std::ops::RangeInclusive<usize>) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .set_timeout_ticks(range);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn forbid_parking(&self) {
        self.dispatcher
            .as_test()
            .unwrap()
            .scheduler()
            .forbid_parking();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn rng(&self) -> crate::scheduler::SharedRng {
        self.dispatcher.as_test().unwrap().scheduler().rng()
    }

    pub fn num_cpus(&self) -> usize {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(test) = self.dispatcher.as_test() {
            return test.num_cpus_override().unwrap_or(4);
        }
        num_cpus::get()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_num_cpus(&self, count: usize) {
        self.dispatcher
            .as_test()
            .expect("set_num_cpus can only be called on a test executor")
            .set_num_cpus(count);
    }

    pub fn is_main_thread(&self) -> bool {
        self.dispatcher.is_main_thread()
    }

    #[doc(hidden)]
    pub fn dispatcher(&self) -> &Arc<dyn PlatformDispatcher> {
        &self.dispatcher
    }
}

impl ForegroundExecutor {
    pub fn new(dispatcher: Arc<dyn PlatformDispatcher>) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        let (scheduler, session_id): (Arc<dyn Scheduler>, _) =
            if let Some(test_dispatcher) = dispatcher.as_test() {
                (
                    test_dispatcher.scheduler().clone(),
                    test_dispatcher.session_id(),
                )
            } else {
                let platform_scheduler = Arc::new(PlatformScheduler::new(dispatcher.clone()));
                let inner = platform_scheduler.foreground_executor();
                return Self {
                    inner,
                    dispatcher,
                    #[cfg(feature = "profiler")]
                    foreground_runnables: Some(platform_scheduler.foreground_runnable_counter()),
                    not_send: PhantomData,
                };
            };

        #[cfg(not(any(test, feature = "test-support")))]
        let platform_scheduler = Arc::new(PlatformScheduler::new(dispatcher.clone()));
        #[cfg(not(any(test, feature = "test-support")))]
        let inner = platform_scheduler.foreground_executor();
        #[cfg(all(not(any(test, feature = "test-support")), feature = "profiler"))]
        let foreground_runnables = Some(platform_scheduler.foreground_runnable_counter());

        #[cfg(any(test, feature = "test-support"))]
        let inner = {
            let scheduler_for_dispatch = Arc::downgrade(&scheduler);
            crate::scheduler::LocalExecutor::new(session_id, scheduler, move |runnable| {
                if let Some(scheduler) = scheduler_for_dispatch.upgrade() {
                    scheduler.schedule_local(session_id, runnable);
                }
            })
        };

        #[cfg(all(any(test, feature = "test-support"), feature = "profiler"))]
        let foreground_runnables = None;

        Self {
            inner,
            dispatcher,
            #[cfg(feature = "profiler")]
            foreground_runnables,
            not_send: PhantomData,
        }
    }

    #[track_caller]
    pub fn spawn<R>(&self, future: impl Future<Output = R> + 'static) -> Task<R>
    where
        R: 'static,
    {
        self.inner.spawn(future.boxed_local())
    }

    #[track_caller]
    pub fn spawn_with_priority<R>(
        &self,
        _priority: Priority,
        future: impl Future<Output = R> + 'static,
    ) -> Task<R>
    where
        R: 'static,
    {
        self.inner.spawn(future)
    }

    #[track_caller]
    pub fn spawn_when_idle<R>(
        &self,
        timeout: Option<Duration>,
        future: impl Future<Output = R> + 'static,
    ) -> Task<R>
    where
        R: 'static,
    {
        let dispatcher = self.dispatcher.clone();
        #[cfg(feature = "profiler")]
        let foreground_runnables = self.foreground_runnables.clone();
        self.inner
            .spawn_with_dispatch(future.boxed_local(), move |runnable| {
                #[cfg(feature = "profiler")]
                if let Some(foreground_runnables) = &foreground_runnables {
                    foreground_runnables.queued();
                }
                dispatcher.dispatch_on_main_thread_when_idle(runnable, timeout);
            })
    }

    pub fn idle_time_remaining(&self) -> Option<Duration> {
        self.dispatcher.idle_time_remaining()
    }

    #[cfg(all(not(target_family = "wasm"), any(test, feature = "test-support")))]
    #[track_caller]
    pub fn block_test<R>(&self, future: impl Future<Output = R>) -> R {
        use std::cell::Cell;

        let scheduler = self.inner.scheduler();

        let output = Cell::new(None);
        let future = async {
            output.set(Some(future.await));
        };
        let mut future = std::pin::pin!(future);

        scheduler.block(None, future.as_mut(), None);

        output.take().expect("block_test future did not complete")
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn block_on<R>(&self, future: impl Future<Output = R>) -> R {
        self.inner.block_on(future)
    }

    #[cfg(not(target_family = "wasm"))]
    pub fn block_with_timeout<R, Fut: Future<Output = R>>(
        &self,
        duration: Duration,
        future: Fut,
    ) -> Result<R, impl Future<Output = R> + use<R, Fut>> {
        self.inner.block_with_timeout(duration, future)
    }

    #[doc(hidden)]
    pub fn dispatcher(&self) -> &Arc<dyn PlatformDispatcher> {
        &self.dispatcher
    }

    #[doc(hidden)]
    pub fn scheduler_executor(&self) -> SchedulerLocalExecutor {
        self.inner.clone()
    }
}

#[cfg(not(target_family = "wasm"))]
pub struct Scope<'a> {
    executor: BackgroundExecutor,
    priority: Priority,
    futures: Vec<Pin<Box<dyn Future<Output = ()> + Send + 'static>>>,
    tx: Option<mpsc::Sender<()>>,
    rx: mpsc::Receiver<()>,
    lifetime: PhantomData<&'a ()>,
}

#[cfg(not(target_family = "wasm"))]
impl<'a> Scope<'a> {
    fn new(executor: BackgroundExecutor, priority: Priority) -> Self {
        let (tx, rx) = mpsc::channel(1);
        Self {
            executor,
            priority,
            tx: Some(tx),
            rx,
            futures: Default::default(),
            lifetime: PhantomData,
        }
    }

    pub fn num_cpus(&self) -> usize {
        self.executor.num_cpus()
    }

    #[track_caller]
    pub fn spawn<F>(&mut self, f: F)
    where
        F: Future<Output = ()> + Send + 'a,
    {
        let tx = self.tx.clone().unwrap();

        let f = unsafe {
            mem::transmute::<
                Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
                Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
            >(Box::pin(async move {
                f.await;
                drop(tx);
            }))
        };
        self.futures.push(f);
    }
}

#[cfg(not(target_family = "wasm"))]
impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.tx.take().unwrap();

        let future = async {
            self.rx.next().await;
        };
        let mut future = std::pin::pin!(future);
        self.executor
            .inner
            .scheduler()
            .block(None, future.as_mut(), None);
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{App, TestDispatcher, TestPlatform};
    use std::cell::RefCell;

    fn create_test_app() -> (TestDispatcher, BackgroundExecutor, Rc<crate::AppCell>) {
        let dispatcher = TestDispatcher::new(0);
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);

        let platform = TestPlatform::new(background_executor.clone(), foreground_executor);
        let asset_source = Arc::new(());
        let http_client = crate::http_client::FakeHttpClient::with_404_response();

        let app = App::new_app(platform, asset_source, http_client);
        (dispatcher, background_executor, app)
    }

    #[test]
    fn sanity_test_tasks_run() {
        let (dispatcher, _background_executor, app) = create_test_app();
        let foreground_executor = app.borrow().foreground_executor.clone();

        let task_ran = Rc::new(RefCell::new(false));

        foreground_executor
            .spawn({
                let task_ran = Rc::clone(&task_ran);
                async move {
                    *task_ran.borrow_mut() = true;
                }
            })
            .detach();

        dispatcher.run_until_parked();

        assert!(
            *task_ran.borrow(),
            "Task should run normally when app is alive"
        );
    }
}
