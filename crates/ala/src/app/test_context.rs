use crate::{
    Action, AnyView, AnyWindowHandle, App, AppCell, AppContext, AsyncApp, AvailableSpace,
    BackgroundExecutor, BorrowAppContext, Bounds, Capslock, ClipboardItem, DrawPhase, Drawable,
    Element, Empty, EntityId, EventEmitter, ForegroundExecutor, Global, InputEvent, Keystroke,
    Modifiers, ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Platform, PlatformTextSystem, Point, Render, Result, SharedString, Size,
    SystemNotification, SystemNotificationResponse, Task, TestDispatcher, TestPlatform,
    TestScreenCaptureSource, TestWindow, TextSystem, VisualContext, Window, WindowBounds,
    WindowHandle, WindowOptions, WindowVisibility, app::AlaMode, window::ElementArenaScope,
};
use anyhow::{anyhow, bail};
use futures::{Stream, StreamExt, channel::oneshot};

use std::{
    cell::RefCell, future::Future, ops::Deref, path::PathBuf, rc::Rc, sync::Arc, time::Duration,
};

#[derive(Clone)]
pub struct TestAppContext {
    #[doc(hidden)]
    pub background_executor: BackgroundExecutor,
    #[doc(hidden)]
    pub foreground_executor: ForegroundExecutor,
    #[doc(hidden)]
    pub dispatcher: TestDispatcher,
    test_platform: Rc<TestPlatform>,
    text_system: Arc<TextSystem>,
    fn_name: Option<&'static str>,
    on_quit: Rc<RefCell<Vec<Box<dyn FnOnce() + 'static>>>>,
    #[doc(hidden)]
    pub app: Rc<AppCell>,
}

impl AppContext for TestAppContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        let mut app = self.app.borrow_mut();
        app.new(build_entity)
    }

    fn reserve_entity<T: 'static>(&mut self) -> crate::Reservation<T> {
        let mut app = self.app.borrow_mut();
        app.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: crate::Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        let mut app = self.app.borrow_mut();
        app.insert_entity(reservation, build_entity)
    }

    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        let mut app = self.app.borrow_mut();
        app.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, _: &Entity<T>) -> super::AlaBorrow<'a, T>
    where
        T: 'static,
    {
        panic!("Cannot use as_mut with a test app context. Try calling update() first")
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let app = self.app.borrow();
        app.read_entity(handle, read)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        let mut lock = self.app.borrow_mut();
        lock.update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        let mut lock = self.app.borrow_mut();
        lock.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let app = self.app.borrow();
        app.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.background_executor.spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        let app = self.app.borrow();
        app.read_global(callback)
    }
}

impl TestAppContext {
    pub fn build(dispatcher: TestDispatcher, fn_name: Option<&'static str>) -> Self {
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);
        let platform = TestPlatform::new(background_executor.clone(), foreground_executor.clone());
        Self::build_with_platform(
            dispatcher,
            fn_name,
            background_executor,
            foreground_executor,
            platform,
        )
    }

    pub fn build_with_text_system(
        dispatcher: TestDispatcher,
        fn_name: Option<&'static str>,
        platform_text_system: Arc<dyn PlatformTextSystem>,
    ) -> Self {
        let arc_dispatcher = Arc::new(dispatcher.clone());
        let background_executor = BackgroundExecutor::new(arc_dispatcher.clone());
        let foreground_executor = ForegroundExecutor::new(arc_dispatcher);
        let platform = TestPlatform::with_text_system(
            background_executor.clone(),
            foreground_executor.clone(),
            platform_text_system,
        );
        Self::build_with_platform(
            dispatcher,
            fn_name,
            background_executor,
            foreground_executor,
            platform,
        )
    }

    fn build_with_platform(
        dispatcher: TestDispatcher,
        fn_name: Option<&'static str>,
        background_executor: BackgroundExecutor,
        foreground_executor: ForegroundExecutor,
        platform: Rc<TestPlatform>,
    ) -> Self {
        let asset_source = Arc::new(());
        let http_client = crate::http_client::FakeHttpClient::with_404_response();
        let text_system = Arc::new(TextSystem::new(platform.text_system()));

        let app = App::new_app(platform.clone(), asset_source, http_client);
        app.borrow_mut().mode = AlaMode::test();

        Self {
            app,
            background_executor,
            foreground_executor,
            dispatcher,
            test_platform: platform,
            text_system,
            fn_name,
            on_quit: Rc::new(RefCell::new(Vec::default())),
        }
    }

    pub fn disable_accessibility(&mut self) {
        self.update(|cx| cx.accessibility_force_disabled = true);
    }

    pub fn skip_drawing(&mut self) {
        self.app.borrow_mut().mode = AlaMode::Test { skip_drawing: true };
    }

    pub fn single() -> Self {
        let dispatcher = TestDispatcher::new(0);
        Self::build(dispatcher, None)
    }

    pub fn test_function_name(&self) -> Option<&'static str> {
        self.fn_name
    }

    pub fn did_prompt_for_new_path(&self) -> bool {
        self.test_platform.did_prompt_for_new_path()
    }

    pub fn active_idle_sleep_preventions(&self) -> usize {
        self.test_platform.active_idle_sleep_preventions()
    }

    pub fn set_idle_sleep_prevention_delay(&self, delay: Duration) {
        self.test_platform.set_idle_sleep_prevention_delay(delay);
    }

    pub fn set_idle_sleep_prevention_fails(&self, fails: bool) {
        self.test_platform.set_idle_sleep_prevention_fails(fails);
    }

    pub fn new_app(&self) -> TestAppContext {
        Self::build(self.dispatcher.clone(), self.fn_name)
    }

    pub fn quit(&self) {
        self.on_quit.borrow_mut().drain(..).for_each(|f| f());
        self.app.borrow_mut().shutdown();
    }

    pub fn on_quit(&mut self, f: impl FnOnce() + 'static) {
        self.on_quit.borrow_mut().push(Box::new(f));
    }

    pub fn refresh(&mut self) -> Result<()> {
        let mut app = self.app.borrow_mut();
        app.refresh_windows();
        Ok(())
    }

    pub fn executor(&self) -> BackgroundExecutor {
        self.background_executor.clone()
    }

    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        &self.foreground_executor
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let mut cx = self.app.borrow_mut();
        cx.update(f)
    }

    pub fn read<R>(&self, f: impl FnOnce(&App) -> R) -> R {
        let cx = self.app.borrow();
        f(&cx)
    }

    pub fn add_window<F, V>(&mut self, build_window: F) -> WindowHandle<V>
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();

        let bounds = Bounds::maximized(None, &cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| build_window(window, cx)),
        )
        .unwrap()
    }

    pub fn open_window<F, V>(
        &mut self,
        window_size: Size<Pixels>,
        build_window: F,
    ) -> WindowHandle<V>
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: window_size,
                })),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| build_window(window, cx)),
        )
        .unwrap()
    }

    pub fn add_empty_window(&mut self) -> &mut VisualTestContext {
        let mut cx = self.app.borrow_mut();
        let bounds = Bounds::maximized(None, &cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(|_| Empty),
            )
            .unwrap();
        drop(cx);
        let cx = VisualTestContext::from_window(*window.deref(), self).into_mut();
        cx.run_until_parked();
        cx
    }

    pub fn add_window_view<F, V>(
        &mut self,
        build_root_view: F,
    ) -> (Entity<V>, &mut VisualTestContext)
    where
        F: FnOnce(&mut Window, &mut Context<V>) -> V,
        V: 'static + Render,
    {
        let mut cx = self.app.borrow_mut();
        let bounds = Bounds::maximized(None, &cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| build_root_view(window, cx)),
            )
            .unwrap();
        drop(cx);
        let view = window.root(self).unwrap();
        let cx = VisualTestContext::from_window(*window.deref(), self).into_mut();
        cx.run_until_parked();

        (view, cx)
    }

    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.test_platform.write_to_clipboard(item)
    }

    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.test_platform.read_from_clipboard()
    }

    pub fn simulate_new_path_selection(
        &self,
        select_path: impl FnOnce(&std::path::Path) -> Option<std::path::PathBuf>,
    ) {
        self.test_platform.simulate_new_path_selection(select_path);
    }

    pub fn simulate_path_prompt_response(
        &self,
        select_paths: impl FnOnce(&crate::PathPromptOptions) -> Option<Vec<std::path::PathBuf>>,
    ) {
        self.test_platform
            .simulate_path_prompt_response(select_paths);
    }

    pub fn did_prompt_for_paths(&self) -> bool {
        self.test_platform.did_prompt_for_paths()
    }

    #[track_caller]
    pub fn simulate_prompt_answer(&self, button: &str) {
        self.test_platform.simulate_prompt_answer(button);
    }

    pub fn has_pending_prompt(&self) -> bool {
        self.test_platform.has_pending_prompt()
    }

    pub fn pending_prompt(&self) -> Option<(String, String)> {
        self.test_platform.pending_prompt()
    }

    pub fn opened_url(&self) -> Option<String> {
        self.test_platform.opened_url.borrow().clone()
    }

    pub fn app_identity(&self) -> Option<(SharedString, SharedString)> {
        self.test_platform.app_identity()
    }

    pub fn shown_system_notifications(&self) -> Vec<SystemNotification> {
        self.test_platform.shown_system_notifications()
    }

    pub fn delivered_system_notifications(&self) -> Vec<SystemNotification> {
        self.test_platform.delivered_system_notifications()
    }

    pub fn dismissed_system_notifications(&self) -> Vec<SharedString> {
        self.test_platform.dismissed_system_notifications()
    }

    pub fn simulate_system_notification_response(&self, response: SystemNotificationResponse) {
        self.test_platform
            .simulate_system_notification_response(response);
    }

    pub fn simulate_window_resize(&self, window_handle: AnyWindowHandle, size: Size<Pixels>) {
        self.test_window(window_handle).simulate_resize(size);
    }

    pub fn simulate_window_move(&self, window_handle: AnyWindowHandle, origin: Point<Pixels>) {
        let mut test_window = self.test_window(window_handle);
        let mouse_position = crate::PlatformWindow::mouse_position(&test_window);
        test_window.simulate_move(origin, mouse_position);
    }

    pub fn simulate_window_visibility_change(
        &self,
        window_handle: AnyWindowHandle,
        visibility: WindowVisibility,
    ) {
        self.test_window(window_handle)
            .simulate_visibility_change(visibility);
    }

    pub fn simulate_window_visual_viewport_change(
        &self,
        window_handle: AnyWindowHandle,
        bounds: Bounds<Pixels>,
    ) {
        self.test_window(window_handle)
            .simulate_visual_viewport_change(bounds);
    }

    pub fn simulate_window_scale_factor_change(
        &self,
        window_handle: AnyWindowHandle,
        scale_factor: f32,
    ) {
        self.test_window(window_handle)
            .simulate_scale_factor_change(scale_factor);
    }

    pub fn expect_restart(&self) -> oneshot::Receiver<(Option<PathBuf>, Vec<std::ffi::OsString>)> {
        let (tx, rx) = futures::channel::oneshot::channel();
        self.test_platform.expect_restart.borrow_mut().replace(tx);
        rx
    }

    pub fn set_screen_capture_sources(&self, sources: Vec<TestScreenCaptureSource>) {
        self.test_platform.set_screen_capture_sources(sources);
    }

    pub fn windows(&self) -> Vec<AnyWindowHandle> {
        self.app.borrow().windows()
    }

    #[track_caller]
    pub fn spawn<Fut, R>(&self, f: impl FnOnce(AsyncApp) -> Fut) -> Task<R>
    where
        Fut: Future<Output = R> + 'static,
        R: 'static,
    {
        self.foreground_executor.spawn(f(self.to_async()))
    }

    pub fn has_global<G: Global>(&self) -> bool {
        let app = self.app.borrow();
        app.has_global::<G>()
    }

    pub fn read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> R {
        let app = self.app.borrow();
        read(app.global(), &app)
    }

    pub fn try_read_global<G: Global, R>(&self, read: impl FnOnce(&G, &App) -> R) -> Option<R> {
        let lock = self.app.borrow();
        Some(read(lock.try_global()?, &lock))
    }

    pub fn set_global<G: Global>(&mut self, global: G) {
        let mut lock = self.app.borrow_mut();
        lock.update(|cx| cx.set_global(global))
    }

    pub fn update_global<G: Global, R>(&mut self, update: impl FnOnce(&mut G, &mut App) -> R) -> R {
        let mut lock = self.app.borrow_mut();
        lock.update(|cx| cx.update_global(update))
    }

    pub fn to_async(&self) -> AsyncApp {
        AsyncApp {
            app: Rc::downgrade(&self.app),
            background_executor: self.background_executor.clone(),
            foreground_executor: self.foreground_executor.clone(),
        }
    }

    pub fn run_until_parked(&self) {
        self.dispatcher.run_until_parked();
    }

    pub fn dispatch_action<A>(&mut self, window: AnyWindowHandle, action: A)
    where
        A: Action,
    {
        window
            .update(self, |_, window, cx| {
                window.dispatch_action(action.boxed_clone(), cx)
            })
            .unwrap();

        self.background_executor.run_until_parked()
    }

    pub fn simulate_keystrokes(&mut self, window: AnyWindowHandle, keystrokes: &str) {
        for keystroke in keystrokes
            .split(' ')
            .map(Keystroke::parse)
            .map(Result::unwrap)
        {
            self.dispatch_keystroke(window, keystroke);
        }

        self.background_executor.run_until_parked()
    }

    pub fn simulate_input(&mut self, window: AnyWindowHandle, input: &str) {
        for keystroke in input.split("").map(Keystroke::parse).map(Result::unwrap) {
            self.dispatch_keystroke(window, keystroke);
        }

        self.background_executor.run_until_parked()
    }

    pub fn dispatch_keystroke(&mut self, window: AnyWindowHandle, keystroke: Keystroke) {
        self.update_window(window, |_, window, cx| {
            window.dispatch_keystroke(keystroke, cx)
        })
        .unwrap();
    }

    pub(crate) fn test_window(&self, window: AnyWindowHandle) -> TestWindow {
        self.app
            .borrow_mut()
            .windows
            .get_mut(window.id)
            .unwrap()
            .as_deref_mut()
            .unwrap()
            .platform_window
            .as_test()
            .unwrap()
            .clone()
    }

    pub fn notifications<T: 'static>(
        &mut self,
        entity: &Entity<T>,
    ) -> impl Stream<Item = ()> + use<T> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        self.update(|cx| {
            cx.observe(entity, {
                let tx = tx.clone();
                move |_, _| {
                    let _ = tx.unbounded_send(());
                }
            })
            .detach();
            cx.observe_release(entity, move |_, _| tx.close_channel())
                .detach()
        });
        rx
    }

    pub fn events<Evt, T: 'static + EventEmitter<Evt>>(
        &mut self,
        entity: &Entity<T>,
    ) -> futures::channel::mpsc::UnboundedReceiver<Evt>
    where
        Evt: 'static + Clone,
    {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        entity
            .update(self, |_, cx: &mut Context<T>| {
                cx.subscribe(entity, move |_entity, _handle, event, _cx| {
                    let _ = tx.unbounded_send(event.clone());
                })
            })
            .detach();
        rx
    }

    pub async fn condition<T: 'static>(
        &mut self,
        entity: &Entity<T>,
        mut predicate: impl FnMut(&mut T, &mut Context<T>) -> bool,
    ) {
        let timer = self.executor().timer(Duration::from_secs(3));
        let mut notifications = self.notifications(entity);

        use futures::FutureExt as _;
        use futures_concurrency::future::Race as _;

        (
            async {
                loop {
                    if entity.update(self, &mut predicate) {
                        return Ok(());
                    }

                    if notifications.next().await.is_none() {
                        bail!("entity dropped")
                    }
                }
            },
            timer.map(|_| Err(anyhow!("condition timed out"))),
        )
            .race()
            .await
            .unwrap();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_name(&mut self, name: &'static str) {
        self.update(|cx| cx.name = Some(name))
    }
}

impl<T: 'static> Entity<T> {
    pub fn next_event<Event>(&self, cx: &mut TestAppContext) -> impl Future<Output = Event>
    where
        Event: Send + Clone + 'static,
        T: EventEmitter<Event>,
    {
        let (tx, mut rx) = oneshot::channel();
        let mut tx = Some(tx);
        let subscription = self.update(cx, |_, cx| {
            cx.subscribe(self, move |_, _, event, _| {
                if let Some(tx) = tx.take() {
                    _ = tx.send(event.clone());
                }
            })
        });

        async move {
            let event = rx.await.expect("no event emitted");
            drop(subscription);
            event
        }
    }
}

impl<V: 'static> Entity<V> {
    pub fn next_notification(
        &self,
        advance_clock_by: Duration,
        cx: &TestAppContext,
    ) -> impl Future<Output = ()> {
        use postage::prelude::{Sink as _, Stream as _};

        let (mut tx, mut rx) = postage::mpsc::channel(1);
        let subscription = cx.app.borrow_mut().observe(self, move |_, _| {
            tx.try_send(()).ok();
        });

        cx.executor().advance_clock(advance_clock_by);

        async move {
            rx.recv()
                .await
                .expect("entity dropped while test was waiting for its next notification");
            drop(subscription);
        }
    }
}

impl<V> Entity<V> {
    pub fn condition<Evt>(
        &self,
        cx: &TestAppContext,
        mut predicate: impl FnMut(&V, &App) -> bool,
    ) -> impl Future<Output = ()>
    where
        Evt: 'static,
        V: EventEmitter<Evt>,
    {
        use postage::prelude::{Sink as _, Stream as _};

        let (tx, mut rx) = postage::mpsc::channel(1024);

        let mut cx = cx.app.borrow_mut();
        let subscriptions = (
            cx.observe(self, {
                let mut tx = tx.clone();
                move |_, _| {
                    tx.blocking_send(()).ok();
                }
            }),
            cx.subscribe(self, {
                let mut tx = tx;
                move |_, _: &Evt, _| {
                    tx.blocking_send(()).ok();
                }
            }),
        );

        let cx = cx.this.upgrade().unwrap();
        let handle = self.downgrade();

        async move {
            loop {
                {
                    let cx = cx.borrow();
                    let cx = &*cx;
                    if predicate(
                        handle
                            .upgrade()
                            .expect("view dropped with pending condition")
                            .read(cx),
                        cx,
                    ) {
                        break;
                    }
                }

                rx.recv()
                    .await
                    .expect("view dropped with pending condition");
            }
            drop(subscriptions);
        }
    }
}

use derive_more::{Deref, DerefMut};

use super::{Context, Entity};
#[derive(Deref, DerefMut, Clone)]
pub struct VisualTestContext {
    #[deref]
    #[deref_mut]
    pub cx: TestAppContext,
    window: AnyWindowHandle,
}

impl VisualTestContext {
    pub fn update<R>(&mut self, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.cx
            .update_window(self.window, |_, window, cx| f(window, cx))
            .unwrap()
    }

    pub fn from_window(window: AnyWindowHandle, cx: &TestAppContext) -> Self {
        Self {
            cx: cx.clone(),
            window,
        }
    }

    pub fn run_until_parked(&self) {
        self.cx.background_executor.run_until_parked();
    }

    pub fn dispatch_action<A>(&mut self, action: A)
    where
        A: Action,
    {
        self.cx.dispatch_action(self.window, action)
    }

    pub fn window_title(&mut self) -> Option<String> {
        self.cx.test_window(self.window).0.lock().title.clone()
    }

    pub fn document_path(&mut self) -> Option<std::path::PathBuf> {
        self.cx
            .test_window(self.window)
            .0
            .lock()
            .document_path
            .clone()
    }

    pub fn simulate_keystrokes(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(self.window, keystrokes)
    }

    pub fn simulate_input(&mut self, input: &str) {
        self.cx.simulate_input(self.window, input)
    }

    pub fn simulate_mouse_move(
        &mut self,
        position: Point<Pixels>,
        button: impl Into<Option<MouseButton>>,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseMoveEvent {
            position,
            modifiers,
            pressed_button: button.into(),
        })
    }

    pub fn simulate_mouse_down(
        &mut self,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button,
            click_count: 1,
            first_mouse: false,
        })
    }

    pub fn simulate_mouse_up(
        &mut self,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: Modifiers,
    ) {
        self.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button,
            click_count: 1,
        })
    }

    pub fn simulate_click(&mut self, position: Point<Pixels>, modifiers: Modifiers) {
        self.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        self.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 1,
        });
    }

    pub fn simulate_modifiers_change(&mut self, modifiers: Modifiers) {
        self.simulate_event(ModifiersChangedEvent {
            modifiers,
            capslock: Capslock { on: false },
        })
    }

    pub fn simulate_capslock_change(&mut self, on: bool) {
        self.simulate_event(ModifiersChangedEvent {
            modifiers: Modifiers::none(),
            capslock: Capslock { on },
        })
    }

    pub fn simulate_resize(&self, size: Size<Pixels>) {
        self.simulate_window_resize(self.window, size)
    }

    pub fn simulate_move(&self, origin: Point<Pixels>) {
        self.simulate_window_move(self.window, origin)
    }

    pub fn simulate_visibility_change(&self, visibility: WindowVisibility) {
        self.simulate_window_visibility_change(self.window, visibility);
    }

    pub fn simulate_scale_factor_change(&self, scale_factor: f32) {
        self.simulate_window_scale_factor_change(self.window, scale_factor)
    }

    pub fn debug_bounds(&mut self, selector: &'static str) -> Option<Bounds<Pixels>> {
        self.update(|window, _| window.rendered_frame.debug_bounds.get(selector).copied())
    }

    pub fn draw<E>(
        &mut self,
        origin: Point<Pixels>,
        space: impl Into<Size<AvailableSpace>>,
        f: impl FnOnce(&mut Window, &mut App) -> E,
    ) -> (E::RequestLayoutState, E::PrepaintState)
    where
        E: Element,
    {
        self.update(|window, cx| {
            let arena_scope = ElementArenaScope::enter(&cx.element_arena);

            window.invalidator.set_phase(DrawPhase::Prepaint);
            let mut element = Drawable::new(f(window, cx));
            element.layout_as_root(space.into(), window, cx);
            window.with_absolute_element_offset(origin, |window| element.prepaint(window, cx));

            window.invalidator.set_phase(DrawPhase::Paint);
            let (request_layout_state, prepaint_state) = element.paint(window, cx);

            window.invalidator.set_phase(DrawPhase::None);
            window.refresh();

            drop(element);
            arena_scope.exit(&cx.element_arena).clear(cx);

            (request_layout_state, prepaint_state)
        })
    }

    pub fn simulate_event<E: InputEvent>(&mut self, event: E) {
        self.test_window(self.window)
            .simulate_input(event.to_platform_input());
        self.background_executor.run_until_parked();
    }

    pub fn deactivate_window(&mut self) {
        if Some(self.window) == self.test_platform.active_window() {
            self.test_platform.set_active_window(None)
        }
        self.background_executor.run_until_parked();
    }

    pub fn simulate_close(&mut self) -> bool {
        let handler = self
            .cx
            .update_window(self.window, |_, window, _| {
                window
                    .platform_window
                    .as_test()
                    .unwrap()
                    .0
                    .lock()
                    .should_close_handler
                    .take()
            })
            .unwrap();
        if let Some(mut handler) = handler {
            let should_close = handler();
            self.cx
                .update_window(self.window, |_, window, _| {
                    window.platform_window.on_should_close(handler);
                })
                .unwrap();
            should_close
        } else {
            false
        }
    }

    pub fn into_mut(self) -> &'static mut Self {
        let ptr = Box::into_raw(Box::new(self));
        let cx = unsafe { &mut *ptr };
        cx.on_quit(move || unsafe {
            drop(Box::from_raw(ptr));
        });
        cx
    }
}

impl AppContext for VisualTestContext {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, _, cx| cx.new(build_entity))
            .expect("window was unexpectedly closed")
    }

    fn reserve_entity<T: 'static>(&mut self) -> crate::Reservation<T> {
        self.cx.reserve_entity()
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: crate::Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, _, cx| {
                cx.insert_entity(reservation, build_entity)
            })
            .expect("window was unexpectedly closed")
    }

    fn update_entity<T, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R
    where
        T: 'static,
    {
        self.cx.update_entity(handle, update)
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> super::AlaBorrow<'a, T>
    where
        T: 'static,
    {
        self.cx.as_mut(handle)
    }

    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        self.cx.read_entity(handle, read)
    }

    fn update_window<T, F>(&mut self, window: AnyWindowHandle, f: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.cx.update_window(window, f)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        self.cx.with_window(entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        self.cx.read_window(window, read)
    }

    fn background_spawn<R>(&self, future: impl Future<Output = R> + Send + 'static) -> Task<R>
    where
        R: Send + 'static,
    {
        self.cx.background_spawn(future)
    }

    fn read_global<G, R>(&self, callback: impl FnOnce(&G, &App) -> R) -> R
    where
        G: Global,
    {
        self.cx.read_global(callback)
    }
}

impl VisualContext for VisualTestContext {
    type Result<T> = T;

    fn window_handle(&self) -> AnyWindowHandle {
        self.window
    }

    fn new_window_entity<T: 'static>(
        &mut self,
        build_entity: impl FnOnce(&mut Window, &mut Context<T>) -> T,
    ) -> Entity<T> {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                cx.new(|cx| build_entity(window, cx))
            })
            .expect("window was unexpectedly closed")
    }

    fn update_window_entity<V: 'static, R>(
        &mut self,
        view: &Entity<V>,
        update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
    ) -> R {
        let view = view.clone();
        self.cx
            .app
            .borrow_mut()
            .with_window(view.entity_id(), |window, app| {
                view.update(app, |v, cx| update(v, window, cx))
            })
            .expect("entity has no current window; use `update` instead of `update_in`")
    }

    fn replace_root_view<V>(
        &mut self,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Entity<V>
    where
        V: 'static + Render,
    {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                window.replace_root(cx, build_view)
            })
            .expect("window was unexpectedly closed")
    }

    fn focus<V: crate::Focusable>(&mut self, view: &Entity<V>) {
        self.window
            .update(&mut self.cx, |_, window, cx| {
                view.read(cx).focus_handle(cx).focus(window, cx)
            })
            .expect("window was unexpectedly closed")
    }
}

impl AnyWindowHandle {
    pub fn build_entity<V: Render + 'static>(
        &self,
        cx: &mut TestAppContext,
        build_view: impl FnOnce(&mut Window, &mut Context<V>) -> V,
    ) -> Entity<V> {
        self.update(cx, |_, window, cx| cx.new(|cx| build_view(window, cx)))
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        PathPromptOptions, SystemNotification, SystemNotificationAction,
        SystemNotificationResponse, TestAppContext,
    };
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    #[ala::test]
    async fn test_system_notifications_require_identity_and_replace_matching_tags(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task started".into(),
                body: "Running tests".into(),
                actions: Vec::new(),
            });
        });
        assert!(cx.shown_system_notifications().is_empty());
        assert!(cx.delivered_system_notifications().is_empty());

        cx.update(|cx| {
            cx.set_app_identity("com.example.tasks", "Tasks");
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task started".into(),
                body: "Running tests".into(),
                actions: Vec::new(),
            });
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: vec![SystemNotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                }],
            });
        });

        assert_eq!(
            cx.app_identity(),
            Some(("com.example.tasks".into(), "Tasks".into()))
        );
        assert_eq!(cx.shown_system_notifications().len(), 2);
        assert_eq!(
            cx.delivered_system_notifications(),
            [SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: vec![SystemNotificationAction {
                    id: "open".into(),
                    label: "Open".into(),
                }],
            }]
        );

        cx.update(|cx| cx.dismiss_system_notification("thread-1"));
        assert!(cx.delivered_system_notifications().is_empty());
        assert_eq!(cx.dismissed_system_notifications(), ["thread-1"]);
    }

    #[ala::test]
    async fn test_system_notification_body_and_action_responses(cx: &mut TestAppContext) {
        let responses = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            cx.on_system_notification_response({
                let responses = responses.clone();
                move |response, _cx| responses.borrow_mut().push(response)
            });
        });

        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        });
        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: Some("default".into()),
        });

        assert_eq!(
            responses.borrow().as_slice(),
            &[
                SystemNotificationResponse {
                    tag: "thread-1".into(),
                    action_id: None,
                },
                SystemNotificationResponse {
                    tag: "thread-1".into(),
                    action_id: Some("default".into()),
                },
            ]
        );
    }

    #[ala::test]
    async fn test_system_notification_response_handler_can_be_replaced(cx: &mut TestAppContext) {
        let first_responses = Rc::new(RefCell::new(Vec::new()));
        let second_responses = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            cx.on_system_notification_response({
                let first_responses = first_responses.clone();
                move |response, _cx| first_responses.borrow_mut().push(response)
            });
            cx.on_system_notification_response({
                let second_responses = second_responses.clone();
                move |response, _cx| second_responses.borrow_mut().push(response)
            });
        });

        let response = SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        };
        cx.simulate_system_notification_response(response.clone());

        assert!(first_responses.borrow().is_empty());
        assert_eq!(second_responses.borrow().as_slice(), &[response]);
    }

    #[ala::test]
    async fn test_system_notification_response_handler_can_reenter_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_app_identity("com.example.tasks", "Tasks");
            cx.show_system_notification(SystemNotification {
                tag: "thread-1".into(),
                title: "Task finished".into(),
                body: "All tests passed".into(),
                actions: Vec::new(),
            });
            cx.on_system_notification_response(|response, cx| {
                cx.dismiss_system_notification(&response.tag);
            });
        });

        cx.simulate_system_notification_response(SystemNotificationResponse {
            tag: "thread-1".into(),
            action_id: None,
        });

        assert!(cx.delivered_system_notifications().is_empty());
        assert_eq!(cx.dismissed_system_notifications(), ["thread-1"]);
    }

    #[ala::test]
    async fn test_simulate_path_prompt_response(cx: &mut TestAppContext) {
        assert!(!cx.did_prompt_for_paths());

        let receiver = cx.update(|cx| {
            cx.prompt_for_paths(PathPromptOptions {
                files: false,
                directories: true,
                multiple: true,
                prompt: None,
            })
        });
        assert!(cx.did_prompt_for_paths());

        let selected = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        cx.simulate_path_prompt_response({
            let selected = selected.clone();
            move |options| {
                assert!(options.multiple);
                Some(selected)
            }
        });
        assert!(!cx.did_prompt_for_paths());

        let response = receiver.await.unwrap().unwrap();
        assert_eq!(response, Some(selected));
    }

    #[ala::test]
    async fn test_simulate_path_prompt_cancellation(cx: &mut TestAppContext) {
        let receiver = cx.update(|cx| {
            cx.prompt_for_paths(PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: None,
            })
        });

        cx.simulate_path_prompt_response(|_options| None);

        let response = receiver.await.unwrap().unwrap();
        assert_eq!(response, None);
    }
}
