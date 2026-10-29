use crate::scheduler::Instant;
use std::{
    any::{TypeId, type_name},
    cell::{BorrowMutError, Cell, Ref, RefCell, RefMut},
    ffi::OsString,
    marker::PhantomData,
    mem,
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering, Ordering::SeqCst},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow};
use derive_more::{Deref, DerefMut};
use futures::{Future, FutureExt, channel::oneshot, future::LocalBoxFuture};
use itertools::Itertools;
#[cfg(any(test, feature = "leak-detection"))]
use parking_lot::RwLock;
use slotmap::SlotMap;

use crate::collections::{FxHashMap, FxHashSet, HashMap, TypeIdHashMap, TypeIdHashSet, VecDeque};
use crate::http_client::{HttpClient, Url};
use crate::{debug_panic, util::ResultExt};
pub use async_context::*;
#[cfg(feature = "bench-support")]
pub use bench_context::{BenchAppContext, BenchReport, BenchWindowContext, bench_platform};
pub use context::*;
pub use entity_map::*;
#[cfg(any(test, feature = "test-support"))]
pub use headless_app_context::*;
use smallvec::SmallVec;
#[cfg(any(test, feature = "test-support"))]
pub use test_app::*;
#[cfg(any(test, feature = "test-support"))]
pub use test_context::*;
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub use visual_test_context::*;

#[cfg(any(feature = "inspector", debug_assertions))]
use crate::InspectorElementRegistry;
use crate::asset_cache::CachedLoad;
use crate::{
    Action, ActionBuildError, ActionRegistry, ActivityGuard, Any, AnyView, AnyWindowHandle,
    AppContext, Arena, ArenaBox, Asset, AssetSource, BackgroundExecutor, Bounds, ClipboardItem,
    ClipboardReadError, CursorStyle, DispatchPhase, DisplayId, EventEmitter, ExternalDragPayload,
    FocusHandle, FocusMap, ForegroundExecutor, Global, KeyBinding, KeyContext, Keymap, Keystroke,
    LayoutId, Menu, MenuItem, MissingGlyph, OwnedMenu, PathPromptOptions, Pixels, Platform,
    PlatformDisplay, PlatformKeyboardLayout, PlatformKeyboardMapper, Point, Priority,
    PromptBuilder, PromptButton, PromptHandle, PromptLevel, Render, RenderImage,
    RenderablePromptHandle, Reservation, ScreenCaptureSource, SharedString, SubscriberSet,
    Subscription, SvgRenderer, SystemNotification, SystemNotificationResponse, Task,
    TextRenderingMode, TextSystem, ThermalState, Window, WindowAppearance, WindowButtonLayout,
    WindowHandle, WindowId, WindowInvalidator,
    colors::{Colors, GlobalColors},
    hash, init_app_menus,
};

mod async_context;
#[cfg(feature = "bench-support")]
mod bench_context;
mod context;
mod entity_map;
#[cfg(any(test, feature = "test-support"))]
mod headless_app_context;
#[cfg(any(test, feature = "test-support"))]
mod test_app;
#[cfg(any(test, feature = "test-support"))]
mod test_context;
#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
mod visual_test_context;

pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(200);

const MAX_NOTIFIES_PER_FLUSH: usize = 1024;

#[doc(hidden)]
pub struct AppCell {
    app: RefCell<App>,
}

impl AppCell {
    #[doc(hidden)]
    #[track_caller]
    pub fn borrow(&self) -> AppRef<'_> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        AppRef(self.app.borrow())
    }

    #[doc(hidden)]
    #[track_caller]
    pub fn borrow_mut(&self) -> AppRefMut<'_> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        AppRefMut(self.app.borrow_mut())
    }

    #[doc(hidden)]
    #[track_caller]
    pub fn try_borrow_mut(&self) -> Result<AppRefMut<'_>, BorrowMutError> {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("borrowed {thread_id:?}");
        }
        Ok(AppRefMut(self.app.try_borrow_mut()?))
    }
}

#[doc(hidden)]
#[derive(Deref, DerefMut)]
pub struct AppRef<'a>(Ref<'a, App>);

impl Drop for AppRef<'_> {
    fn drop(&mut self) {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("dropped borrow from {thread_id:?}");
        }
    }
}

#[doc(hidden)]
#[derive(Deref, DerefMut)]
pub struct AppRefMut<'a>(RefMut<'a, App>);

impl Drop for AppRefMut<'_> {
    fn drop(&mut self) {
        if option_env!("TRACK_THREAD_BORROWS").is_some() {
            let thread_id = std::thread::current().id();
            eprintln!("dropped {thread_id:?}");
        }
    }
}

pub struct Application(Rc<AppCell>);

pub struct ApplicationHandle {
    app: Rc<AppCell>,
}

impl ApplicationHandle {
    pub fn update<R>(&self, f: impl FnOnce(&mut App) -> R) -> R {
        let cx = &mut *self.app.borrow_mut();
        f(cx)
    }

    pub fn to_async(&self) -> AsyncApp {
        self.update(|cx| cx.to_async())
    }
}

impl Application {
    pub fn with_platform(platform: Rc<dyn Platform>) -> Self {
        Self(App::new_app(
            platform,
            Arc::new(()),
            Arc::new(NullHttpClient),
        ))
    }

    pub fn new_inaccessible(platform: Rc<dyn Platform>) -> Self {
        let this = Self::with_platform(platform);
        this.0.borrow_mut().accessibility_force_disabled = true;
        this
    }

    pub fn with_accessibility_forced(self) -> Self {
        self.0.borrow_mut().accessibility_forced = true;
        self
    }

    pub fn with_assets(self, asset_source: impl AssetSource) -> Self {
        let mut context_lock = self.0.borrow_mut();
        let asset_source = Arc::new(asset_source);
        context_lock.asset_source = asset_source.clone();
        context_lock.svg_renderer = SvgRenderer::new(asset_source);
        drop(context_lock);
        self
    }

    pub fn with_restart_arguments(self, arguments: Vec<OsString>) -> Self {
        self.0.borrow_mut().restart_arguments = arguments;
        self
    }

    pub fn with_http_client(self, http_client: Arc<dyn HttpClient>) -> Self {
        let mut context_lock = self.0.borrow_mut();
        context_lock.http_client = http_client;
        drop(context_lock);
        self
    }

    pub fn with_quit_mode(self, mode: QuitMode) -> Self {
        self.0.borrow_mut().quit_mode = mode;
        self
    }

    pub fn run<F>(self, on_finish_launching: F)
    where
        F: 'static + FnOnce(&mut App),
    {
        let this = self.0.clone();
        let platform = self.0.borrow().platform.clone();
        platform.run(Box::new(move || {
            let cx = &mut *this.borrow_mut();
            on_finish_launching(cx);
        }));

        #[cfg(target_family = "wasm")]
        std::mem::forget(self);
    }

    pub fn run_embedded<F>(self, on_finish_launching: F) -> ApplicationHandle
    where
        F: 'static + FnOnce(&mut App),
    {
        let this = self.0.clone();
        let platform = self.0.borrow().platform.clone();
        platform.run(Box::new(move || {
            let cx = &mut *this.borrow_mut();
            on_finish_launching(cx);
        }));
        ApplicationHandle { app: self.0 }
    }

    pub fn on_open_urls<F>(&self, mut callback: F) -> &Self
    where
        F: 'static + FnMut(Vec<String>),
    {
        self.0.borrow().platform.on_open_urls(Box::new(callback));
        self
    }

    pub fn on_reopen<F>(&self, mut callback: F) -> &Self
    where
        F: 'static + FnMut(&mut App),
    {
        let this = Rc::downgrade(&self.0);
        self.0.borrow_mut().platform.on_reopen(Box::new(move || {
            if let Some(app) = this.upgrade() {
                callback(&mut app.borrow_mut());
            }
        }));
        self
    }

    pub fn background_executor(&self) -> BackgroundExecutor {
        self.0.borrow().background_executor.clone()
    }

    pub fn foreground_executor(&self) -> ForegroundExecutor {
        self.0.borrow().foreground_executor.clone()
    }

    pub fn text_system(&self) -> Arc<TextSystem> {
        self.0.borrow().text_system.clone()
    }

    pub fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.0.borrow().path_for_auxiliary_executable(name)
    }
}

type Handler = Box<dyn FnMut(&mut App) -> bool + 'static>;
type Listener = Box<dyn FnMut(&dyn Any, &mut App) -> bool + 'static>;
type MissingGlyphCallback = Box<dyn FnMut(&[MissingGlyph], &mut App) + 'static>;
pub(crate) type KeystrokeObserver =
    Box<dyn FnMut(&KeystrokeEvent, &mut Window, &mut App) -> bool + 'static>;
type QuitHandler = Box<dyn FnOnce(&mut App) -> LocalBoxFuture<'static, ()> + 'static>;
type WindowClosedHandler = Box<dyn FnMut(&mut App, WindowId)>;
type ReleaseListener = Box<dyn FnOnce(&mut dyn Any, &mut App) + 'static>;
type NewEntityListener = Box<dyn FnMut(AnyEntity, &mut Option<&mut Window>, &mut App) + 'static>;

struct MissingGlyphCallbackEntry {
    registration: Rc<()>,
    callback: Option<MissingGlyphCallback>,
}

#[derive(Default)]
struct MissingGlyphCallbackSlot {
    entry: RefCell<Option<MissingGlyphCallbackEntry>>,
}

impl MissingGlyphCallbackSlot {
    fn replace(&self, callback: MissingGlyphCallback) -> Rc<()> {
        let registration = Rc::new(());
        self.entry.borrow_mut().replace(MissingGlyphCallbackEntry {
            registration: registration.clone(),
            callback: Some(callback),
        });
        registration
    }

    fn invoke(&self, missing_glyphs: &[MissingGlyph], cx: &mut App) {
        let Some((registration, mut callback)) =
            self.entry.borrow_mut().as_mut().and_then(|entry| {
                entry
                    .callback
                    .take()
                    .map(|callback| (entry.registration.clone(), callback))
            })
        else {
            return;
        };
        callback(missing_glyphs, cx);

        let mut entry = self.entry.borrow_mut();
        let is_current = entry
            .as_ref()
            .is_some_and(|entry| Rc::ptr_eq(&entry.registration, &registration));
        if is_current {
            let Some(entry) = entry.as_mut() else {
                return;
            };
            entry.callback = Some(callback);
        }
    }

    fn remove(&self, registration: &Rc<()>) -> bool {
        let mut entry = self.entry.borrow_mut();
        let is_current = entry
            .as_ref()
            .is_some_and(|entry| Rc::ptr_eq(&entry.registration, registration));
        if is_current {
            entry.take();
        }
        is_current
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QuitMode {
    #[default]
    Default,
    LastWindowClosed,
    Explicit,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorHideMode {
    Never,
    OnTyping,
    #[default]
    OnTypingAndAction,
}

#[doc(hidden)]
#[derive(Clone, PartialEq, Eq)]
pub struct SystemWindowTab {
    pub id: WindowId,
    pub title: SharedString,
    pub handle: AnyWindowHandle,
    pub last_active_at: Instant,
}

impl SystemWindowTab {
    pub fn new(title: SharedString, handle: AnyWindowHandle) -> Self {
        Self {
            id: handle.id,
            title,
            handle,
            last_active_at: Instant::now(),
        }
    }
}

#[derive(Default)]
pub struct SystemWindowTabController {
    visible: Option<bool>,
    tab_groups: FxHashMap<usize, Vec<SystemWindowTab>>,
}

impl Global for SystemWindowTabController {}

impl SystemWindowTabController {
    pub fn new() -> Self {
        Self {
            visible: None,
            tab_groups: FxHashMap::default(),
        }
    }

    pub fn init(cx: &mut App) {
        cx.set_global(SystemWindowTabController::new());
    }

    pub fn tab_groups(&self) -> &FxHashMap<usize, Vec<SystemWindowTab>> {
        &self.tab_groups
    }

    pub fn get_next_tab_group_window(cx: &mut App, id: WindowId) -> Option<&AnyWindowHandle> {
        let controller = cx.global::<SystemWindowTabController>();
        let current_group = controller
            .tab_groups
            .iter()
            .find_map(|(group, tabs)| tabs.iter().find(|tab| tab.id == id).map(|_| group));

        let current_group = current_group?;
        let mut group_ids: Vec<_> = controller.tab_groups.keys().collect();
        let idx = group_ids.iter().position(|g| *g == current_group)?;
        let next_idx = (idx + 1) % group_ids.len();

        controller
            .tab_groups
            .get(group_ids[next_idx])
            .and_then(|tabs| {
                tabs.iter()
                    .max_by_key(|tab| tab.last_active_at)
                    .or_else(|| tabs.first())
                    .map(|tab| &tab.handle)
            })
    }

    pub fn get_prev_tab_group_window(cx: &mut App, id: WindowId) -> Option<&AnyWindowHandle> {
        let controller = cx.global::<SystemWindowTabController>();
        let current_group = controller
            .tab_groups
            .iter()
            .find_map(|(group, tabs)| tabs.iter().find(|tab| tab.id == id).map(|_| group));

        let current_group = current_group?;
        let mut group_ids: Vec<_> = controller.tab_groups.keys().collect();
        let idx = group_ids.iter().position(|g| *g == current_group)?;
        let prev_idx = if idx == 0 {
            group_ids.len() - 1
        } else {
            idx - 1
        };

        controller
            .tab_groups
            .get(group_ids[prev_idx])
            .and_then(|tabs| {
                tabs.iter()
                    .max_by_key(|tab| tab.last_active_at)
                    .or_else(|| tabs.first())
                    .map(|tab| &tab.handle)
            })
    }

    pub fn tabs(&self, id: WindowId) -> Option<&Vec<SystemWindowTab>> {
        self.tab_groups
            .values()
            .find(|tabs| tabs.iter().any(|tab| tab.id == id))
    }

    pub fn init_visible(cx: &mut App, visible: bool) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        if controller.visible.is_none() {
            controller.visible = Some(visible);
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible.unwrap_or(false)
    }

    pub fn set_visible(cx: &mut App, visible: bool) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        controller.visible = Some(visible);
    }

    pub fn update_last_active(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for windows in controller.tab_groups.values_mut() {
            for tab in windows.iter_mut() {
                if tab.id == id {
                    tab.last_active_at = Instant::now();
                }
            }
        }
    }

    pub fn update_tab_position(cx: &mut App, id: WindowId, ix: usize) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for (_, windows) in controller.tab_groups.iter_mut() {
            if let Some(current_pos) = windows.iter().position(|tab| tab.id == id) {
                if ix < windows.len() && current_pos != ix {
                    let window_tab = windows.remove(current_pos);
                    windows.insert(ix, window_tab);
                }
                break;
            }
        }
    }

    pub fn update_tab_title(cx: &mut App, id: WindowId, title: SharedString) {
        let controller = cx.global::<SystemWindowTabController>();
        let tab = controller
            .tab_groups
            .values()
            .flat_map(|windows| windows.iter())
            .find(|tab| tab.id == id);

        if tab.map_or(true, |t| t.title == title) {
            return;
        }

        let mut controller = cx.global_mut::<SystemWindowTabController>();
        for windows in controller.tab_groups.values_mut() {
            for tab in windows.iter_mut() {
                if tab.id == id {
                    tab.title = title;
                    return;
                }
            }
        }
    }

    pub fn add_tab(cx: &mut App, id: WindowId, tabs: Vec<SystemWindowTab>) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tab) = tabs.iter().find(|tab| tab.id == id).cloned() else {
            return;
        };

        let mut expected_tab_ids: Vec<_> = tabs
            .iter()
            .filter(|tab| tab.id != id)
            .map(|tab| tab.id)
            .sorted()
            .collect();

        let mut tab_group_id = None;
        for (group_id, group_tabs) in &controller.tab_groups {
            let tab_ids: Vec<_> = group_tabs.iter().map(|tab| tab.id).sorted().collect();
            if tab_ids == expected_tab_ids {
                tab_group_id = Some(*group_id);
                break;
            }
        }

        if let Some(tab_group_id) = tab_group_id {
            if let Some(tabs) = controller.tab_groups.get_mut(&tab_group_id) {
                tabs.push(tab);
            }
        } else {
            let new_group_id = controller.tab_groups.len();
            controller.tab_groups.insert(new_group_id, tabs);
        }
    }

    pub fn remove_tab(cx: &mut App, id: WindowId) -> Option<SystemWindowTab> {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let mut removed_tab = None;

        controller.tab_groups.retain(|_, tabs| {
            if let Some(pos) = tabs.iter().position(|tab| tab.id == id) {
                removed_tab = Some(tabs.remove(pos));
            }
            !tabs.is_empty()
        });

        removed_tab
    }

    pub fn move_tab_to_new_window(cx: &mut App, id: WindowId) {
        let mut removed_tab = Self::remove_tab(cx, id);
        let mut controller = cx.global_mut::<SystemWindowTabController>();

        if let Some(tab) = removed_tab {
            let new_group_id = controller.tab_groups.keys().max().map_or(0, |k| k + 1);
            controller.tab_groups.insert(new_group_id, vec![tab]);
        }
    }

    pub fn merge_all_windows(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(initial_tabs) = controller.tabs(id) else {
            return;
        };

        let initial_tabs_len = initial_tabs.len();
        let mut all_tabs = initial_tabs.clone();

        for (_, mut tabs) in controller.tab_groups.drain() {
            tabs.retain(|tab| !all_tabs[..initial_tabs_len].contains(tab));
            all_tabs.extend(tabs);
        }

        controller.tab_groups.insert(0, all_tabs);
    }

    pub fn select_next_tab(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tabs) = controller.tabs(id) else {
            return;
        };

        let current_index = tabs.iter().position(|tab| tab.id == id).unwrap();
        let next_index = (current_index + 1) % tabs.len();

        let _ = &tabs[next_index].handle.update(cx, |_, window, _| {
            window.activate_window();
        });
    }

    pub fn select_previous_tab(cx: &mut App, id: WindowId) {
        let mut controller = cx.global_mut::<SystemWindowTabController>();
        let Some(tabs) = controller.tabs(id) else {
            return;
        };

        let current_index = tabs.iter().position(|tab| tab.id == id).unwrap();
        let previous_index = if current_index == 0 {
            tabs.len() - 1
        } else {
            current_index - 1
        };

        let _ = &tabs[previous_index].handle.update(cx, |_, window, _| {
            window.activate_window();
        });
    }
}

pub(crate) enum AlaMode {
    #[cfg(any(test, feature = "test-support"))]
    Test {
        skip_drawing: bool,
    },
    Production,
}

impl AlaMode {
    #[cfg(any(test, feature = "test-support"))]
    pub fn test() -> Self {
        AlaMode::Test {
            skip_drawing: false,
        }
    }

    #[inline]
    pub(crate) fn skip_drawing(&self) -> bool {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            AlaMode::Test { skip_drawing } => *skip_drawing,
            AlaMode::Production => false,
        }
    }
}

struct PlatformOwnedDrag {
    source_window: WindowId,
    state: PlatformOwnedDragState,
}

enum PlatformOwnedDragState {
    Suspended(AnyDrag),
    RestoredInSourceWindow,
}

static ANIMATION_SPEED: AtomicU32 = AtomicU32::new(0x3F80_0000);

pub fn animation_speed() -> f32 {
    f32::from_bits(ANIMATION_SPEED.load(Ordering::Relaxed))
}

pub fn animation_time(elapsed: Duration) -> Duration {
    elapsed.mul_f32(animation_speed())
}

pub struct App {
    pub(crate) this: Weak<AppCell>,
    pub(crate) platform: Rc<dyn Platform>,
    text_system: Arc<TextSystem>,

    pub(crate) actions: Rc<ActionRegistry>,
    pub(crate) active_drag: Option<AnyDrag>,
    platform_owned_drag: Option<PlatformOwnedDrag>,
    pub(crate) background_executor: BackgroundExecutor,
    pub(crate) foreground_executor: ForegroundExecutor,
    #[cfg(feature = "profiler")]
    foreground_journal: crate::profiler::journal::ForegroundJournal,
    pub(crate) entities: EntityMap,
    pub(crate) new_entity_observers: SubscriberSet<TypeId, NewEntityListener>,
    pub(crate) windows: SlotMap<WindowId, Option<Box<Window>>>,
    pub(crate) window_handles: FxHashMap<WindowId, AnyWindowHandle>,
    pub(crate) focus_handles: Arc<FocusMap>,
    pub(crate) keymap: Rc<RefCell<Keymap>>,
    pub(crate) keyboard_layout: Box<dyn PlatformKeyboardLayout>,
    pub(crate) keyboard_mapper: Rc<dyn PlatformKeyboardMapper>,
    pub(crate) global_action_listeners:
        TypeIdHashMap<Vec<Rc<dyn Fn(&dyn Any, DispatchPhase, &mut Self)>>>,
    pending_effects: VecDeque<Effect>,

    pub(crate) observers: SubscriberSet<EntityId, Handler>,
    pub(crate) event_listeners: SubscriberSet<EntityId, (TypeId, Listener)>,
    pub(crate) keystroke_observers: SubscriberSet<(), KeystrokeObserver>,
    pub(crate) keystroke_interceptors: SubscriberSet<(), KeystrokeObserver>,
    pub(crate) keyboard_layout_observers: SubscriberSet<(), Handler>,
    missing_glyph_callback: Rc<MissingGlyphCallbackSlot>,
    pub(crate) thermal_state_observers: SubscriberSet<(), Handler>,
    pub(crate) system_sleep_observers: SubscriberSet<(), Handler>,
    pub(crate) system_wake_observers: SubscriberSet<(), Handler>,
    pub(crate) release_listeners: SubscriberSet<EntityId, ReleaseListener>,
    pub(crate) global_observers: SubscriberSet<TypeId, Handler>,
    pub(crate) quit_observers: SubscriberSet<(), QuitHandler>,
    pub(crate) restart_observers: SubscriberSet<(), Handler>,
    pub(crate) window_closed_observers: SubscriberSet<(), WindowClosedHandler>,

    pub(crate) element_arena: RefCell<Arena>,
    pub(crate) event_arena: Arena,

    pub(crate) globals_by_type: TypeIdHashMap<Box<dyn Any>>,

    pub(crate) loading_assets: FxHashMap<(TypeId, u64), Box<dyn Any>>,
    asset_source: Arc<dyn AssetSource>,
    pub(crate) svg_renderer: SvgRenderer,
    http_client: Arc<dyn HttpClient>,

    pub(crate) pending_notifications: FxHashSet<EntityId>,
    pub(crate) pending_global_notifications: TypeIdHashSet,
    pub(crate) restart_path: Option<PathBuf>,
    pub(crate) restart_arguments: Vec<OsString>,
    pub(crate) layout_id_buffer: Vec<LayoutId>,
    pub(crate) propagate_event: bool,
    pub(crate) prompt_builder: Option<PromptBuilder>,
    pub(crate) window_invalidators_by_entity:
        FxHashMap<EntityId, FxHashMap<WindowId, WindowInvalidator>>,
    pub(crate) tracked_entities: FxHashMap<WindowId, FxHashSet<EntityId>>,
    pub(crate) current_window_by_entity: FxHashMap<EntityId, WindowId>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) inspector_renderer: Option<crate::InspectorRenderer>,
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub(crate) inspector_element_registry: InspectorElementRegistry,
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    pub(crate) name: Option<&'static str>,
    pub(crate) text_rendering_mode: Rc<Cell<TextRenderingMode>>,

    pub(crate) window_update_stack: Vec<WindowId>,
    pub(crate) mode: AlaMode,
    pub(crate) cursor_hide_mode: CursorHideMode,
    pub(crate) reduce_motion: bool,
    pub(crate) synced_animation_epoch: Instant,
    pub(crate) accessibility_force_disabled: bool,
    pub(crate) accessibility_forced: bool,
    flushing_effects: bool,
    pending_updates: usize,
    quit_mode: QuitMode,
    quitting: bool,

    #[cfg(any(test, feature = "leak-detection"))]
    _ref_counts: Arc<RwLock<EntityRefCounts>>,
}

impl App {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new_app(
        platform: Rc<dyn Platform>,
        asset_source: Arc<dyn AssetSource>,
        http_client: Arc<dyn HttpClient>,
    ) -> Rc<AppCell> {
        let background_executor = platform.background_executor();
        let foreground_executor = platform.foreground_executor();
        assert!(
            background_executor.is_main_thread(),
            "must construct App on main thread"
        );
        #[cfg(feature = "profiler")]
        let foreground_journal = crate::profiler::journal::install_foreground_journal();
        let synced_animation_epoch = background_executor.now();

        let text_system = Arc::new(TextSystem::new(platform.text_system()));
        let entities = EntityMap::new();
        let keyboard_layout = platform.keyboard_layout();
        let keyboard_mapper = platform.keyboard_mapper();

        #[cfg(any(test, feature = "leak-detection"))]
        let _ref_counts = entities.ref_counts_drop_handle();

        let app = Rc::new_cyclic(|this| AppCell {
            app: RefCell::new(App {
                this: this.clone(),
                platform: platform.clone(),
                text_system,
                text_rendering_mode: Rc::new(Cell::new(TextRenderingMode::default())),
                mode: AlaMode::Production,
                actions: Rc::new(ActionRegistry::default()),
                flushing_effects: false,
                pending_updates: 0,
                active_drag: None,
                platform_owned_drag: None,
                background_executor,
                foreground_executor,
                #[cfg(feature = "profiler")]
                foreground_journal,
                svg_renderer: SvgRenderer::new(asset_source.clone()),
                loading_assets: Default::default(),
                asset_source,
                http_client,
                globals_by_type: Default::default(),
                entities,
                new_entity_observers: SubscriberSet::new(),
                windows: SlotMap::with_key(),
                window_update_stack: Vec::new(),
                window_handles: FxHashMap::default(),
                focus_handles: Arc::default(),
                keymap: Rc::new(RefCell::new(Keymap::default())),
                keyboard_layout,
                keyboard_mapper,
                global_action_listeners: Default::default(),
                pending_effects: VecDeque::new(),
                pending_notifications: FxHashSet::default(),
                pending_global_notifications: Default::default(),
                observers: SubscriberSet::new(),
                tracked_entities: FxHashMap::default(),
                window_invalidators_by_entity: FxHashMap::default(),
                current_window_by_entity: FxHashMap::default(),
                event_listeners: SubscriberSet::new(),
                release_listeners: SubscriberSet::new(),
                keystroke_observers: SubscriberSet::new(),
                keystroke_interceptors: SubscriberSet::new(),
                keyboard_layout_observers: SubscriberSet::new(),
                missing_glyph_callback: Rc::default(),
                thermal_state_observers: SubscriberSet::new(),
                system_sleep_observers: SubscriberSet::new(),
                system_wake_observers: SubscriberSet::new(),
                global_observers: SubscriberSet::new(),
                quit_observers: SubscriberSet::new(),
                restart_observers: SubscriberSet::new(),
                restart_path: None,
                restart_arguments: Vec::new(),
                window_closed_observers: SubscriberSet::new(),
                layout_id_buffer: Default::default(),
                propagate_event: true,
                prompt_builder: Some(PromptBuilder::Default),
                #[cfg(any(feature = "inspector", debug_assertions))]
                inspector_renderer: None,
                #[cfg(any(feature = "inspector", debug_assertions))]
                inspector_element_registry: InspectorElementRegistry::default(),
                quit_mode: QuitMode::default(),
                quitting: false,
                cursor_hide_mode: CursorHideMode::default(),
                reduce_motion: false,
                synced_animation_epoch,
                accessibility_force_disabled: false,
                accessibility_forced: false,

                #[cfg(any(test, feature = "test-support", debug_assertions))]
                name: None,
                element_arena: RefCell::new(Arena::new(1024 * 1024)),
                event_arena: Arena::new(1024 * 1024),

                #[cfg(any(test, feature = "leak-detection"))]
                _ref_counts,
            }),
        });

        init_app_menus(platform.as_ref(), &app.borrow());
        SystemWindowTabController::init(&mut app.borrow_mut());
        #[cfg(feature = "profiler")]
        crate::profiler::journal::observe_power(&app.borrow());

        platform.on_keyboard_layout_change(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.keyboard_layout = cx.platform.keyboard_layout();
                    cx.keyboard_mapper = cx.platform.keyboard_mapper();
                    cx.keyboard_layout_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_thermal_state_change(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.thermal_state_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_system_sleep(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.system_sleep_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_system_wake(Box::new({
            let app = Rc::downgrade(&app);
            move || {
                if let Some(app) = app.upgrade() {
                    let cx = &mut app.borrow_mut();
                    cx.system_wake_observers
                        .clone()
                        .retain(&(), move |callback| (callback)(cx));
                }
            }
        }));

        platform.on_quit(Box::new({
            let cx = Rc::downgrade(&app);
            move || {
                let Some(cx) = cx.upgrade() else {
                    return true;
                };
                match cx.try_borrow_mut() {
                    Ok(mut cx) => {
                        cx.shutdown();
                        true
                    }
                    Err(_) => false,
                }
            }
        }));

        app
    }

    #[doc(hidden)]
    pub fn ref_counts_drop_handle(&self) -> impl Sized + use<> {
        self.entities.ref_counts_drop_handle()
    }

    #[cfg(any(test, feature = "leak-detection"))]
    pub fn leak_detector_snapshot(&self) -> LeakDetectorSnapshot {
        self.entities.leak_detector_snapshot()
    }

    #[cfg(any(test, feature = "leak-detection"))]
    pub fn assert_no_new_leaks(&self, snapshot: &LeakDetectorSnapshot) {
        self.entities.assert_no_new_leaks(snapshot)
    }

    pub fn shutdown(&mut self) {
        let mut futures = Vec::new();

        for observer in self.quit_observers.remove(&()) {
            futures.push(observer(self));
        }

        self.windows.clear();
        self.window_handles.clear();
        self.flush_effects();
        self.quitting = true;

        let futures = futures::future::join_all(futures);
        #[cfg(not(target_family = "wasm"))]
        if self
            .foreground_executor
            .block_with_timeout(SHUTDOWN_TIMEOUT, futures)
            .is_err()
        {
            log::error!("timed out waiting on app_will_quit");
        }
        #[cfg(target_family = "wasm")]
        self.foreground_executor.spawn(futures).detach();

        self.quitting = false;
    }

    pub fn keyboard_layout(&self) -> &dyn PlatformKeyboardLayout {
        self.keyboard_layout.as_ref()
    }

    pub fn keyboard_mapper(&self) -> &Rc<dyn PlatformKeyboardMapper> {
        &self.keyboard_mapper
    }

    pub fn on_keyboard_layout_change<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.keyboard_layout_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    pub fn quit(&self) {
        self.platform.quit();
    }

    pub fn cursor_hide_mode(&self) -> CursorHideMode {
        self.cursor_hide_mode
    }

    pub fn set_cursor_hide_mode(&mut self, mode: CursorHideMode) {
        self.cursor_hide_mode = mode;
    }

    pub fn is_cursor_visible(&self) -> bool {
        self.platform.is_cursor_visible()
    }

    pub fn reduce_motion(&self) -> bool {
        self.reduce_motion
    }

    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        if self.reduce_motion != reduce_motion {
            self.reduce_motion = reduce_motion;
            self.refresh_windows();
        }
    }

    pub fn animation_speed(&self) -> f32 {
        animation_speed()
    }

    pub fn set_animation_speed(&mut self, speed: f32) {
        let speed = if speed.is_finite() && speed > 0.0 {
            speed
        } else {
            1.0
        };
        if ANIMATION_SPEED.swap(speed.to_bits(), Ordering::Relaxed) != speed.to_bits() {
            self.refresh_windows();
        }
    }

    pub fn refresh_windows(&mut self) {
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    #[inline(always)]
    pub(crate) fn update<R>(&mut self, update: impl FnOnce(&mut Self) -> R) -> R {
        self.start_update();
        let result = update(self);
        self.finish_update();
        result
    }

    pub(crate) fn start_update(&mut self) {
        self.pending_updates += 1;
    }

    #[inline(never)]
    pub(crate) fn finish_update(&mut self) {
        if !self.flushing_effects && self.pending_updates == 1 {
            self.flushing_effects = true;
            self.flush_effects();
            self.flushing_effects = false;
        }
        self.pending_updates -= 1;
    }

    pub fn observe<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(Entity<W>, &mut App) + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        self.observe_internal(entity, move |e, cx| {
            on_notify(e, cx);
            true
        })
    }

    pub(crate) fn detect_accessed_entities<R>(
        &mut self,
        callback: impl FnOnce(&mut App) -> R,
    ) -> (R, FxHashSet<EntityId>) {
        let accessed_entities_start = self.entities.accessed_entities.get_mut().clone();
        let result = callback(self);
        let entities_accessed_in_callback = self
            .entities
            .accessed_entities
            .get_mut()
            .difference(&accessed_entities_start)
            .copied()
            .collect::<FxHashSet<EntityId>>();
        (result, entities_accessed_in_callback)
    }

    pub(crate) fn record_entities_accessed(
        &mut self,
        window_handle: AnyWindowHandle,
        invalidator: WindowInvalidator,
        entities: &FxHashSet<EntityId>,
    ) {
        let mut tracked_entities =
            std::mem::take(self.tracked_entities.entry(window_handle.id).or_default());
        for entity in tracked_entities.iter() {
            self.window_invalidators_by_entity
                .entry(*entity)
                .and_modify(|windows| {
                    windows.remove(&window_handle.id);
                });
        }
        for entity in entities.iter() {
            self.window_invalidators_by_entity
                .entry(*entity)
                .or_default()
                .insert(window_handle.id, invalidator.clone());
            self.current_window_by_entity
                .insert(*entity, window_handle.id);
        }
        tracked_entities.clear();
        tracked_entities.extend(entities.iter().copied());
        self.tracked_entities
            .insert(window_handle.id, tracked_entities);
    }

    pub(crate) fn new_observer(&mut self, key: EntityId, value: Handler) -> Subscription {
        let (subscription, activate) = self.observers.insert(key, value);
        self.defer(move |_| activate());
        subscription
    }

    pub(crate) fn observe_internal<W>(
        &mut self,
        entity: &Entity<W>,
        mut on_notify: impl FnMut(Entity<W>, &mut App) -> bool + 'static,
    ) -> Subscription
    where
        W: 'static,
    {
        let entity_id = entity.entity_id();
        let handle = entity.downgrade();
        self.new_observer(
            entity_id,
            Box::new(move |cx| {
                if let Some(entity) = handle.upgrade() {
                    on_notify(entity, cx)
                } else {
                    false
                }
            }),
        )
    }

    pub fn subscribe<T, Event>(
        &mut self,
        entity: &Entity<T>,
        mut on_event: impl FnMut(Entity<T>, &Event, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Event>,
        Event: 'static,
    {
        self.subscribe_internal(entity, move |entity, event, cx| {
            on_event(entity, event, cx);
            true
        })
    }

    pub(crate) fn new_subscription(
        &mut self,
        key: EntityId,
        value: (TypeId, Listener),
    ) -> Subscription {
        let (subscription, activate) = self.event_listeners.insert(key, value);
        self.defer(move |_| activate());
        subscription
    }
    pub(crate) fn subscribe_internal<T, Evt>(
        &mut self,
        entity: &Entity<T>,
        mut on_event: impl FnMut(Entity<T>, &Evt, &mut App) -> bool + 'static,
    ) -> Subscription
    where
        T: 'static + EventEmitter<Evt>,
        Evt: 'static,
    {
        let entity_id = entity.entity_id();
        let handle = entity.downgrade();
        self.new_subscription(
            entity_id,
            (
                TypeId::of::<Evt>(),
                Box::new(move |event, cx| {
                    let event: &Evt = event.downcast_ref().expect("invalid event type");
                    if let Some(entity) = handle.upgrade() {
                        on_event(entity, event, cx)
                    } else {
                        false
                    }
                }),
            ),
        )
    }

    pub fn windows(&self) -> Vec<AnyWindowHandle> {
        self.windows
            .keys()
            .flat_map(|window_id| self.window_handles.get(&window_id).copied())
            .collect()
    }

    pub fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        self.platform.window_stack()
    }

    pub fn active_window(&self) -> Option<AnyWindowHandle> {
        self.platform.active_window()
    }

    pub fn open_window<V: 'static + Render>(
        &mut self,
        options: crate::WindowOptions,
        build_root_view: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
    ) -> anyhow::Result<WindowHandle<V>> {
        self.update(|cx| {
            let id = cx.windows.insert(None);
            let handle = WindowHandle::new(id);
            match Window::new(handle.into(), options, cx) {
                Ok(mut window) => {
                    cx.window_update_stack.push(id);
                    let root_view = build_root_view(&mut window, cx);
                    cx.window_update_stack.pop();
                    window.root.replace(root_view.into());
                    window.defer(cx, |window: &mut Window, cx| window.appearance_changed(cx));

                    let clear = window.draw(cx);
                    clear.clear(cx);

                    cx.window_handles.insert(id, window.handle);
                    cx.windows.get_mut(id).unwrap().replace(Box::new(window));
                    Ok(handle)
                }
                Err(e) => {
                    cx.windows.remove(id);
                    Err(e)
                }
            }
        })
    }

    pub fn activate(&self, ignoring_other_apps: bool) {
        self.platform.activate(ignoring_other_apps);
    }

    pub fn hide(&self) {
        self.platform.hide();
    }

    pub fn hide_other_apps(&self) {
        self.platform.hide_other_apps();
    }

    pub fn unhide_other_apps(&self) {
        self.platform.unhide_other_apps();
    }

    pub fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.platform.displays()
    }

    pub fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        self.platform.primary_display()
    }

    pub fn is_screen_capture_supported(&self) -> bool {
        self.platform.is_screen_capture_supported()
    }

    pub fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        self.platform.screen_capture_sources()
    }

    pub fn find_display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        self.displays()
            .iter()
            .find(|display| display.id() == id)
            .cloned()
    }

    pub fn thermal_state(&self) -> ThermalState {
        self.platform.thermal_state()
    }

    pub fn main_thread_busy(&self) -> Option<crate::scheduler::MainThreadBusy> {
        self.platform.main_thread_busy()
    }

    pub fn prevent_idle_sleep(&self, reason: &str) -> Task<Result<ActivityGuard>> {
        self.platform.prevent_idle_sleep(reason)
    }

    pub fn on_thermal_state_change<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.thermal_state_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    pub fn on_system_sleep<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.system_sleep_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    pub fn on_system_wake<F>(&self, mut callback: F) -> Subscription
    where
        F: 'static + FnMut(&mut App),
    {
        let (subscription, activate) = self.system_wake_observers.insert(
            (),
            Box::new(move |cx| {
                callback(cx);
                true
            }),
        );
        activate();
        subscription
    }

    pub fn window_appearance(&self) -> WindowAppearance {
        self.platform.window_appearance()
    }

    pub fn set_window_appearance(&self, appearance: Option<WindowAppearance>) {
        self.platform.set_window_appearance(appearance);
    }

    pub fn button_layout(&self) -> Option<WindowButtonLayout> {
        self.platform.button_layout()
    }

    pub fn read_from_clipboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_clipboard()
    }

    pub fn read_from_clipboard_async(
        &self,
    ) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        self.platform.read_from_clipboard_async()
    }

    pub fn set_text_rendering_mode(&mut self, mode: TextRenderingMode) {
        self.text_rendering_mode.set(mode);
    }

    pub fn text_rendering_mode(&self) -> TextRenderingMode {
        self.text_rendering_mode.get()
    }

    pub fn write_to_clipboard(&self, item: ClipboardItem) {
        self.platform.write_to_clipboard(item)
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn read_from_primary(&self) -> Option<ClipboardItem> {
        self.platform.read_from_primary()
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub fn write_to_primary(&self, item: ClipboardItem) {
        self.platform.write_to_primary(item)
    }

    #[cfg(target_os = "macos")]
    pub fn read_from_find_pasteboard(&self) -> Option<ClipboardItem> {
        self.platform.read_from_find_pasteboard()
    }

    #[cfg(target_os = "macos")]
    pub fn write_to_find_pasteboard(&self, item: ClipboardItem) {
        self.platform.write_to_find_pasteboard(item)
    }

    pub fn write_credentials(
        &self,
        url: &str,
        username: &str,
        password: &[u8],
    ) -> Task<Result<()>> {
        self.platform.write_credentials(url, username, password)
    }

    pub fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>> {
        self.platform.read_credentials(url)
    }

    pub fn delete_credentials(&self, url: &str) -> Task<Result<()>> {
        self.platform.delete_credentials(url)
    }

    pub fn open_url(&self, url: &str) {
        self.platform.open_url(url);
    }

    pub fn register_url_scheme(&self, scheme: &str) -> Task<Result<()>> {
        self.platform.register_url_scheme(scheme)
    }

    pub fn set_app_identity(&self, identifier: &str, name: &str) {
        self.platform.set_app_identity(identifier, name);
    }

    pub fn show_system_notification(&self, notification: SystemNotification) {
        self.platform.show_system_notification(notification);
    }

    pub fn dismiss_system_notification(&self, tag: &str) {
        self.platform.dismiss_system_notification(tag);
    }

    pub fn on_system_notification_response<F>(&self, mut callback: F)
    where
        F: 'static + FnMut(SystemNotificationResponse, &mut App),
    {
        let this = self.this.clone();
        self.platform
            .on_system_notification_response(Box::new(move |response| {
                if let Some(app) = this.upgrade() {
                    callback(response, &mut app.borrow_mut());
                }
            }));
    }

    pub fn app_path(&self) -> Result<PathBuf> {
        self.platform.app_path()
    }

    pub fn compositor_name(&self) -> &'static str {
        self.platform.compositor_name()
    }

    pub fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf> {
        self.platform.path_for_auxiliary_executable(name)
    }

    pub fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>> {
        self.platform.prompt_for_paths(options)
    }

    pub fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>> {
        self.platform.prompt_for_new_path(directory, suggested_name)
    }

    pub fn reveal_path(&self, path: &Path) {
        self.platform.reveal_path(path)
    }

    pub fn open_with_system(&self, path: &Path) {
        self.platform.open_with_system(path)
    }

    pub fn should_auto_hide_scrollbars(&self) -> bool {
        self.platform.should_auto_hide_scrollbars()
    }

    pub fn restart(&mut self) {
        self.restart_observers
            .clone()
            .retain(&(), |observer| observer(self));
        self.platform.restart(
            self.restart_path.take(),
            std::mem::take(&mut self.restart_arguments),
        )
    }

    pub fn set_restart_path(&mut self, path: PathBuf) {
        self.restart_path = Some(path);
    }

    pub fn http_client(&self) -> Arc<dyn HttpClient> {
        self.http_client.clone()
    }

    pub fn set_http_client(&mut self, new_client: Arc<dyn HttpClient>) {
        self.http_client = new_client;
    }

    pub fn set_quit_mode(&mut self, mode: QuitMode) {
        self.quit_mode = mode;
    }

    pub fn svg_renderer(&self) -> SvgRenderer {
        self.svg_renderer.clone()
    }

    pub(crate) fn push_effect(&mut self, effect: Effect) {
        match &effect {
            Effect::Notify { emitter } => {
                if !self.pending_notifications.insert(*emitter) {
                    return;
                }
            }
            Effect::NotifyGlobalObservers { global_type } => {
                if !self.pending_global_notifications.insert(*global_type) {
                    return;
                }
            }
            _ => {}
        };

        self.pending_effects.push_back(effect);
    }

    fn flush_effects(&mut self) {
        let mut notifications = FxHashMap::<EntityId, usize>::default();
        loop {
            self.release_dropped_entities();
            self.release_dropped_focus_handles();
            if let Some(effect) = self.pending_effects.pop_front() {
                match effect {
                    Effect::Notify { emitter } => {
                        let count = notifications.entry(emitter).or_default();
                        *count += 1;
                        if *count <= MAX_NOTIFIES_PER_FLUSH {
                            self.apply_notify_effect(emitter);
                        } else {
                            if *count == MAX_NOTIFIES_PER_FLUSH + 1 {
                                log::error!(
                                    "{emitter:?} was notified {MAX_NOTIFIES_PER_FLUSH} times while flushing one update, so its observers notify it back in a cycle; dropping its notifications until this flush ends"
                                );
                            }
                            self.pending_notifications.remove(&emitter);
                        }
                    }

                    Effect::Emit {
                        emitter,
                        event_type,
                        event,
                    } => self.apply_emit_effect(emitter, event_type, &*event),

                    Effect::RefreshWindows => {
                        self.apply_refresh_effect();
                    }

                    Effect::NotifyGlobalObservers { global_type } => {
                        self.apply_notify_global_observers_effect(global_type);
                    }

                    Effect::Defer { callback } => {
                        self.apply_defer_effect(callback);
                    }
                    Effect::EntityCreated {
                        entity,
                        tid,
                        window,
                    } => {
                        self.apply_entity_created_effect(entity, tid, window);
                    }
                }
            } else {
                #[cfg(any(test, feature = "test-support"))]
                if matches!(self.mode, AlaMode::Test { .. }) {
                    for window in self
                        .windows
                        .values()
                        .filter_map(|window| {
                            let window = window.as_deref()?;
                            window.invalidator.is_dirty().then_some(window.handle)
                        })
                        .collect::<Vec<_>>()
                    {
                        self.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
                            .unwrap();
                    }
                }

                if self.pending_effects.is_empty() {
                    for window in self.windows.values().filter_map(|window| window.as_deref()) {
                        if window.invalidator.is_dirty()
                            || window.needs_present.get()
                            || !window.next_frame_callbacks.borrow().is_empty()
                        {
                            window.platform_window.schedule_frame();
                        }
                    }

                    self.event_arena.clear();
                    break;
                }
            }
        }
    }

    fn release_dropped_entities(&mut self) {
        loop {
            let dropped = self.entities.take_dropped();
            if dropped.is_empty() {
                break;
            }

            for (entity_id, mut entity) in dropped {
                self.observers.remove(&entity_id);
                self.event_listeners.remove(&entity_id);
                self.window_invalidators_by_entity.remove(&entity_id);
                self.current_window_by_entity.remove(&entity_id);
                for release_callback in self.release_listeners.remove(&entity_id) {
                    release_callback(entity.as_mut(), self);
                }
            }
        }
    }

    fn release_dropped_focus_handles(&mut self) {
        if !self.focus_handles.take_released() {
            return;
        }
        self.focus_handles
            .clone()
            .write()
            .retain(|handle_id, focus| {
                if focus.ref_count.load(SeqCst) == 0 {
                    for window_handle in self.windows() {
                        window_handle
                            .update(self, |_, window, cx| {
                                if window.focus == Some(handle_id) {
                                    window.blur(cx);
                                }
                            })
                            .unwrap();
                    }
                    false
                } else {
                    true
                }
            });
    }

    fn apply_notify_effect(&mut self, emitter: EntityId) {
        self.pending_notifications.remove(&emitter);

        self.observers
            .clone()
            .retain(&emitter, |handler| handler(self));
    }

    fn apply_emit_effect(&mut self, emitter: EntityId, event_type: TypeId, event: &dyn Any) {
        self.event_listeners
            .clone()
            .retain(&emitter, |(stored_type, handler)| {
                if *stored_type == event_type {
                    handler(event, self)
                } else {
                    true
                }
            });
    }

    fn apply_refresh_effect(&mut self) {
        for window in self.windows.values_mut() {
            if let Some(window) = window.as_deref_mut() {
                window.refreshing = true;
                window.invalidator.set_dirty(true);
            }
        }
    }

    fn apply_notify_global_observers_effect(&mut self, type_id: TypeId) {
        self.pending_global_notifications.remove(&type_id);
        self.global_observers
            .clone()
            .retain(&type_id, |observer| observer(self));
    }

    fn apply_defer_effect(&mut self, callback: Box<dyn FnOnce(&mut Self) + 'static>) {
        callback(self);
    }

    fn apply_entity_created_effect(
        &mut self,
        entity: AnyEntity,
        tid: TypeId,
        window: Option<WindowId>,
    ) {
        if let Some(id) = window {
            self.current_window_by_entity.insert(entity.entity_id(), id);
        }

        self.new_entity_observers.clone().retain(&tid, |observer| {
            if let Some(id) = window {
                self.update_window_id(id, {
                    let entity = entity.clone();
                    |_, window, cx| (observer)(entity, &mut Some(window), cx)
                })
                .expect("All windows should be off the stack when flushing effects");
            } else {
                (observer)(entity.clone(), &mut None, self)
            }
            true
        });
    }

    #[inline(always)]
    pub fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        let window_id = *self.current_window_by_entity.get(&entity_id)?;
        self.update_window_id(window_id, |_, window, cx| f(window, cx))
            .ok()
    }

    fn ensure_window(&mut self, entity_id: EntityId, window: WindowId) {
        self.current_window_by_entity
            .entry(entity_id)
            .or_insert(window);
    }

    #[inline(always)]
    pub(crate) fn update_window_id<T, F>(&mut self, id: WindowId, update: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        let mut update = Some(update);
        let mut result = None;
        self.update_window_erased(id, &mut |arguments| {
            if let Some((root_view, window, cx)) = arguments {
                result = Some(update.take().unwrap()(root_view, window, cx));
            } else {
                drop(update.take());
                drop(result.take());
            }
        });
        result.context("window not found")
    }

    pub fn to_async(&self) -> AsyncApp {
        AsyncApp {
            app: self.this.clone(),
            background_executor: self.background_executor.clone(),
            foreground_executor: self.foreground_executor.clone(),
        }
    }

    pub fn background_executor(&self) -> &BackgroundExecutor {
        &self.background_executor
    }

    pub fn foreground_executor(&self) -> &ForegroundExecutor {
        if self.quitting {
            panic!("Can't spawn on main thread after on_app_quit")
        };
        &self.foreground_executor
    }

    #[cfg(feature = "profiler")]
    pub fn foreground_journal(&self) -> crate::profiler::journal::ForegroundJournal {
        self.foreground_journal.clone()
    }

    #[track_caller]
    #[inline(always)]
    pub fn spawn<AsyncFn, R>(&self, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        let mut cx = self.prepare_spawn();

        self.foreground_executor
            .spawn(async move { f(&mut cx).await }.boxed_local())
    }

    pub fn spawn_with_priority<AsyncFn, R>(&self, priority: Priority, f: AsyncFn) -> Task<R>
    where
        AsyncFn: AsyncFnOnce(&mut AsyncApp) -> R + 'static,
        R: 'static,
    {
        if self.quitting {
            debug_panic!("Can't spawn on main thread after on_app_quit")
        };

        let mut cx = self.to_async();

        self.foreground_executor
            .spawn_with_priority(priority, async move { f(&mut cx).await }.boxed_local())
    }

    pub fn defer(&mut self, f: impl FnOnce(&mut App) + 'static) {
        self.push_effect(Effect::Defer {
            callback: Box::new(f),
        });
    }

    pub fn asset_source(&self) -> &Arc<dyn AssetSource> {
        &self.asset_source
    }

    pub fn text_system(&self) -> &Arc<TextSystem> {
        &self.text_system
    }

    pub fn on_missing_glyphs(
        &self,
        callback: impl FnMut(&[MissingGlyph], &mut App) + 'static,
    ) -> Subscription {
        let registration = self.missing_glyph_callback.replace(Box::new(callback));

        if let Some(mut receiver) = self.text_system.take_missing_glyph_receiver() {
            let callback = self.missing_glyph_callback.clone();
            self.spawn(async move |cx| {
                while let Ok(missing_glyphs) = receiver.recv().await {
                    cx.update(|cx| callback.invoke(&missing_glyphs, cx));
                }
            })
            .detach();
        }
        self.text_system.enable_missing_glyph_reporting();

        let callback = self.missing_glyph_callback.clone();
        let text_system = self.text_system.clone();
        Subscription::new(move || {
            if callback.remove(&registration) {
                text_system.disable_missing_glyph_reporting();
            }
        })
    }

    pub fn has_global<G: Global>(&self) -> bool {
        self.globals_by_type.contains_key(&TypeId::of::<G>())
    }

    #[track_caller]
    pub fn global<G: Global>(&self) -> &G {
        self.globals_by_type
            .get(&TypeId::of::<G>())
            .map(|any_state| any_state.downcast_ref::<G>().unwrap())
            .unwrap_or_else(|| panic!("no state of type {} exists", type_name::<G>()))
    }

    pub fn try_global<G: Global>(&self) -> Option<&G> {
        self.globals_by_type
            .get(&TypeId::of::<G>())
            .map(|any_state| any_state.downcast_ref::<G>().unwrap())
    }

    #[track_caller]
    pub fn global_mut<G: Global>(&mut self) -> &mut G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type
            .get_mut(&global_type)
            .and_then(|any_state| any_state.downcast_mut::<G>())
            .unwrap_or_else(|| panic!("no state of type {} exists", type_name::<G>()))
    }

    pub fn default_global<G: Global + Default>(&mut self) -> &mut G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type
            .entry(global_type)
            .or_insert_with(|| Box::<G>::default())
            .downcast_mut::<G>()
            .unwrap()
    }

    pub fn set_global<G: Global>(&mut self, global: G) {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type.insert(global_type, Box::new(global));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn clear_globals(&mut self) {
        self.globals_by_type.drain();
    }

    pub fn remove_global<G: Global>(&mut self) -> G {
        let global_type = TypeId::of::<G>();
        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        *self
            .globals_by_type
            .remove(&global_type)
            .unwrap_or_else(|| panic!("no global added for {}", type_name::<G>()))
            .downcast()
            .unwrap()
    }

    pub fn observe_global<G: Global>(
        &mut self,
        mut f: impl FnMut(&mut Self) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.global_observers.insert(
            TypeId::of::<G>(),
            Box::new(move |cx| {
                f(cx);
                true
            }),
        );
        self.defer(move |_| activate());
        subscription
    }

    #[track_caller]
    pub(crate) fn lease_global<G: Global>(&mut self) -> GlobalLease<G> {
        GlobalLease::new(
            self.globals_by_type
                .remove(&TypeId::of::<G>())
                .with_context(|| format!("no global registered of type {}", type_name::<G>()))
                .unwrap(),
        )
    }

    pub(crate) fn end_global_lease<G: Global>(&mut self, lease: GlobalLease<G>) {
        let global_type = TypeId::of::<G>();

        self.push_effect(Effect::NotifyGlobalObservers { global_type });
        self.globals_by_type.insert(global_type, lease.global);
    }

    pub(crate) fn new_entity_observer(
        &self,
        key: TypeId,
        value: NewEntityListener,
    ) -> Subscription {
        let (subscription, activate) = self.new_entity_observers.insert(key, value);
        activate();
        subscription
    }

    pub fn observe_new<T: 'static>(
        &self,
        on_new: impl 'static + Fn(&mut T, Option<&mut Window>, &mut Context<T>),
    ) -> Subscription {
        self.new_entity_observer(
            TypeId::of::<T>(),
            Box::new(
                move |any_entity: AnyEntity, window: &mut Option<&mut Window>, cx: &mut App| {
                    any_entity
                        .downcast::<T>()
                        .unwrap()
                        .update(cx, |entity_state, cx| {
                            on_new(entity_state, window.as_deref_mut(), cx)
                        })
                },
            ),
        )
    }

    pub fn observe_release<T>(
        &self,
        handle: &Entity<T>,
        on_release: impl FnOnce(&mut T, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let (subscription, activate) = self.release_listeners.insert(
            handle.entity_id(),
            Box::new(move |entity, cx| {
                let entity = entity.downcast_mut().expect("invalid entity type");
                on_release(entity, cx)
            }),
        );
        activate();
        subscription
    }

    pub fn observe_release_in<T>(
        &self,
        handle: &Entity<T>,
        window: &Window,
        on_release: impl FnOnce(&mut T, &mut Window, &mut App) + 'static,
    ) -> Subscription
    where
        T: 'static,
    {
        let window_handle = window.handle;
        self.observe_release(handle, move |entity, cx| {
            let _ = window_handle.update(cx, |_, window, cx| on_release(entity, window, cx));
        })
    }

    pub fn observe_keystrokes(
        &mut self,
        mut f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        fn inner(
            keystroke_observers: &SubscriberSet<(), KeystrokeObserver>,
            handler: KeystrokeObserver,
        ) -> Subscription {
            let (subscription, activate) = keystroke_observers.insert((), handler);
            activate();
            subscription
        }

        inner(
            &self.keystroke_observers,
            Box::new(move |event, window, cx| {
                f(event, window, cx);
                true
            }),
        )
    }

    pub fn intercept_keystrokes(
        &mut self,
        mut f: impl FnMut(&KeystrokeEvent, &mut Window, &mut App) + 'static,
    ) -> Subscription {
        fn inner(
            keystroke_interceptors: &SubscriberSet<(), KeystrokeObserver>,
            handler: KeystrokeObserver,
        ) -> Subscription {
            let (subscription, activate) = keystroke_interceptors.insert((), handler);
            activate();
            subscription
        }

        inner(
            &self.keystroke_interceptors,
            Box::new(move |event, window, cx| {
                f(event, window, cx);
                true
            }),
        )
    }

    pub fn bind_keys(&mut self, bindings: impl IntoIterator<Item = KeyBinding>) {
        self.keymap.borrow_mut().add_bindings(bindings);
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    pub fn clear_key_bindings(&mut self) {
        self.keymap.borrow_mut().clear();
        self.pending_effects.push_back(Effect::RefreshWindows);
    }

    pub fn key_bindings(&self) -> Rc<RefCell<Keymap>> {
        self.keymap.clone()
    }

    pub fn on_action<A: Action>(
        &mut self,
        listener: impl Fn(&A, &mut Self) + 'static,
    ) -> &mut Self {
        self.global_action_listeners
            .entry(TypeId::of::<A>())
            .or_default()
            .push(Rc::new(move |action, phase, cx| {
                if phase == DispatchPhase::Bubble {
                    let action = action.downcast_ref().unwrap();
                    listener(action, cx)
                }
            }));
        self
    }

    pub fn stop_propagation(&mut self) {
        self.propagate_event = false;
    }

    pub fn propagate(&mut self) {
        self.propagate_event = true;
    }

    pub fn build_action(
        &self,
        name: &str,
        data: Option<serde_json::Value>,
    ) -> std::result::Result<Box<dyn Action>, ActionBuildError> {
        self.actions.build_action(name, data)
    }

    pub fn all_action_names(&self) -> &[&'static str] {
        self.actions.all_action_names()
    }

    pub fn all_bindings_for_input(&self, input: &[Keystroke]) -> Vec<KeyBinding> {
        RefCell::borrow(&self.keymap).all_bindings_for_input(input)
    }

    pub fn action_schemas(
        &self,
        generator: &mut schemars::SchemaGenerator,
    ) -> Vec<(&'static str, Option<schemars::Schema>)> {
        self.actions.action_schemas(generator)
    }

    pub fn action_schema_by_name(
        &self,
        name: &str,
        generator: &mut schemars::SchemaGenerator,
    ) -> Option<Option<schemars::Schema>> {
        self.actions.action_schema_by_name(name, generator)
    }

    pub fn deprecated_actions_to_preferred_actions(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.deprecated_aliases()
    }

    pub fn action_deprecation_messages(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.deprecation_messages()
    }

    pub fn action_documentation(&self) -> &HashMap<&'static str, &'static str> {
        self.actions.documentation()
    }

    pub fn on_app_quit<Fut>(
        &self,
        mut on_quit: impl FnMut(&mut App) -> Fut + 'static,
    ) -> Subscription
    where
        Fut: 'static + Future<Output = ()>,
    {
        let (subscription, activate) = self.quit_observers.insert(
            (),
            Box::new(move |cx| {
                let future = on_quit(cx);
                future.boxed_local()
            }),
        );
        activate();
        subscription
    }

    pub fn on_app_restart(&self, mut on_restart: impl 'static + FnMut(&mut App)) -> Subscription {
        let (subscription, activate) = self.restart_observers.insert(
            (),
            Box::new(move |cx| {
                on_restart(cx);
                true
            }),
        );
        activate();
        subscription
    }

    pub fn on_window_closed(
        &self,
        mut on_closed: impl FnMut(&mut App, WindowId) + 'static,
    ) -> Subscription {
        let (subscription, activate) = self.window_closed_observers.insert((), Box::new(on_closed));
        activate();
        subscription
    }

    pub(crate) fn clear_pending_keystrokes(&mut self) {
        for window in self.windows() {
            window
                .update(self, |_, window, cx| {
                    window.clear_pending_keystrokes(cx);
                })
                .ok();
        }
    }

    pub fn is_action_available(&mut self, action: &dyn Action) -> bool {
        let mut action_available = false;
        if let Some(window) = self.active_window()
            && let Ok(window_action_available) =
                window.update(self, |_, window, cx| window.is_action_available(action, cx))
        {
            action_available = window_action_available;
        }

        action_available
            || self
                .global_action_listeners
                .contains_key(&action.as_any().type_id())
    }

    pub fn set_menus(&self, menus: impl IntoIterator<Item = Menu>) {
        let menus: Vec<Menu> = menus.into_iter().collect();
        self.platform.set_menus(menus, &self.keymap.borrow());
    }

    pub fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        self.platform.get_menus()
    }

    pub fn set_dock_menu(&self, menus: Vec<MenuItem>) {
        self.platform.set_dock_menu(menus, &self.keymap.borrow())
    }

    pub fn perform_dock_menu_action(&self, action: usize) {
        self.platform.perform_dock_menu_action(action);
    }

    pub fn add_recent_document(&self, path: &Path) {
        self.platform.add_recent_document(path);
    }

    pub fn update_jump_list(
        &self,
        menus: Vec<MenuItem>,
        entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        self.platform.update_jump_list(menus, entries)
    }

    pub fn dispatch_action(&mut self, action: &dyn Action) {
        if let Some(active_window) = self.active_window() {
            active_window
                .update(self, |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx)
                })
                .log_err();
        } else {
            self.dispatch_global_action(action);
        }
    }

    fn dispatch_global_action(&mut self, action: &dyn Action) {
        self.propagate_event = true;

        if let Some(mut global_listeners) = self
            .global_action_listeners
            .remove(&action.as_any().type_id())
        {
            for listener in &global_listeners {
                listener(action.as_any(), DispatchPhase::Capture, self);
                if !self.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                self.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            self.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }

        if self.propagate_event
            && let Some(mut global_listeners) = self
                .global_action_listeners
                .remove(&action.as_any().type_id())
        {
            for listener in global_listeners.iter().rev() {
                listener(action.as_any(), DispatchPhase::Bubble, self);
                if !self.propagate_event {
                    break;
                }
            }

            global_listeners.extend(
                self.global_action_listeners
                    .remove(&action.as_any().type_id())
                    .unwrap_or_default(),
            );

            self.global_action_listeners
                .insert(action.as_any().type_id(), global_listeners);
        }
    }

    pub fn has_active_drag(&self) -> bool {
        self.active_drag.is_some()
    }

    pub fn active_drag_cursor_style(&self) -> Option<CursorStyle> {
        self.active_drag.as_ref().and_then(|drag| drag.cursor_style)
    }

    pub fn stop_active_drag(&mut self, window: &mut Window) -> bool {
        if self.active_drag.is_some() {
            self.active_drag = None;
            if self.platform_owned_drag.as_ref().is_some_and(|drag| {
                drag.source_window == window.window_handle().window_id()
                    && matches!(&drag.state, PlatformOwnedDragState::RestoredInSourceWindow)
            }) {
                self.platform_owned_drag = None;
            }
            window.refresh();
            true
        } else {
            false
        }
    }

    pub(crate) fn hand_active_drag_to_platform(&mut self, source_window: WindowId) -> bool {
        let Some(drag) = self.active_drag.take() else {
            return false;
        };
        self.platform_owned_drag = Some(PlatformOwnedDrag {
            source_window,
            state: PlatformOwnedDragState::Suspended(drag),
        });
        true
    }

    pub(crate) fn restore_platform_drag(&mut self, source_window: WindowId) -> bool {
        let Some(platform_drag) = self
            .platform_owned_drag
            .as_mut()
            .filter(|drag| drag.source_window == source_window)
        else {
            return false;
        };
        let state = std::mem::replace(
            &mut platform_drag.state,
            PlatformOwnedDragState::RestoredInSourceWindow,
        );
        let PlatformOwnedDragState::Suspended(drag) = state else {
            return false;
        };
        self.active_drag = Some(drag);
        true
    }

    pub(crate) fn hand_restored_drag_to_platform(&mut self, source_window: WindowId) -> bool {
        let Some(platform_drag) = self.platform_owned_drag.as_mut().filter(|drag| {
            drag.source_window == source_window
                && matches!(&drag.state, PlatformOwnedDragState::RestoredInSourceWindow)
        }) else {
            return false;
        };
        let Some(drag) = self.active_drag.take() else {
            return false;
        };
        platform_drag.state = PlatformOwnedDragState::Suspended(drag);
        true
    }

    pub(crate) fn end_platform_drag(&mut self, source_window: WindowId) -> bool {
        if !self
            .platform_owned_drag
            .as_ref()
            .is_some_and(|drag| drag.source_window == source_window)
        {
            return false;
        }
        self.platform_owned_drag = None;
        self.active_drag = None;
        true
    }

    pub fn set_active_drag_cursor_style(
        &mut self,
        cursor_style: CursorStyle,
        window: &mut Window,
    ) -> bool {
        if let Some(ref mut drag) = self.active_drag {
            drag.cursor_style = Some(cursor_style);
            window.refresh();
            true
        } else {
            false
        }
    }

    pub fn set_prompt_builder(
        &mut self,
        renderer: impl Fn(
            PromptLevel,
            &str,
            Option<&str>,
            &[PromptButton],
            PromptHandle,
            &mut Window,
            &mut App,
        ) -> RenderablePromptHandle
        + 'static,
    ) {
        self.prompt_builder = Some(PromptBuilder::Custom(Box::new(renderer)));
    }

    pub fn reset_prompt_builder(&mut self) {
        self.prompt_builder = Some(PromptBuilder::Default);
    }

    pub fn remove_asset<A: Asset>(&mut self, source: &A::Source) {
        let asset_id = (TypeId::of::<A>(), hash(source));
        self.loading_assets.remove(&asset_id);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_asset<A: Asset>(&self, source: &A::Source) -> bool {
        let asset_id = (TypeId::of::<A>(), hash(source));
        self.loading_assets.contains_key(&asset_id)
    }

    pub fn fetch_asset<A: Asset>(&mut self, source: &A::Source) -> Option<A::Output> {
        self.asset_entry::<A>(source).get()
    }

    pub(crate) fn asset_entry<A: Asset>(&mut self, source: &A::Source) -> &CachedLoad<A::Output> {
        let asset_id = (TypeId::of::<A>(), hash(source));
        if !self.loading_assets.contains_key(&asset_id) {
            let future = A::load(source.clone(), self);
            let entry = CachedLoad::new(future, self);
            self.loading_assets.insert(asset_id, Box::new(entry));
        }
        self.loading_assets
            .get(&asset_id)
            .and_then(|entry| entry.downcast_ref())
            .expect("asset cache entries are keyed by their asset type")
    }

    #[track_caller]
    pub fn focus_handle(&self) -> FocusHandle {
        FocusHandle::new(&self.focus_handles)
    }

    pub fn notify(&mut self, entity_id: EntityId) {
        let window_invalidators = mem::take(
            self.window_invalidators_by_entity
                .entry(entity_id)
                .or_default(),
        );

        let live_invalidators: SmallVec<[WindowInvalidator; 2]> = window_invalidators
            .iter()
            .filter(|(window_id, _)| {
                self.tracked_entities
                    .get(window_id)
                    .is_some_and(|set| set.contains(&entity_id))
            })
            .map(|(_, invalidator)| invalidator.clone())
            .collect();

        if live_invalidators.is_empty() {
            if self.pending_notifications.insert(entity_id) {
                self.pending_effects
                    .push_back(Effect::Notify { emitter: entity_id });
            }
        } else {
            for invalidator in &live_invalidators {
                invalidator.invalidate_view(entity_id, self);
            }
        }

        self.window_invalidators_by_entity
            .insert(entity_id, window_invalidators);
    }

    #[cfg(any(test, feature = "test-support", debug_assertions))]
    pub fn get_name(&self) -> Option<&'static str> {
        self.name
    }

    pub fn can_select_mixed_files_and_dirs(&self) -> bool {
        self.platform.can_select_mixed_files_and_dirs()
    }

    pub fn drop_image(&mut self, image: Arc<RenderImage>, current_window: Option<&mut Window>) {
        for window in self.windows.values_mut().flatten() {
            _ = window.drop_image(image.clone());
        }

        if let Some(window) = current_window {
            _ = window.drop_image(image);
        }
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn set_inspector_renderer(&mut self, f: crate::InspectorRenderer) {
        self.inspector_renderer = Some(f);
    }

    #[cfg(any(feature = "inspector", debug_assertions))]
    pub fn register_inspector_element<T: 'static, R: crate::IntoElement, F>(
        &mut self,
        factory: impl 'static + Fn(&mut Window, &mut App) -> F,
    ) where
        F: 'static + FnMut(crate::InspectorElementId, &T, &mut Window, &mut App) -> R,
    {
        self.inspector_element_registry.register(factory);
    }

    pub fn init_colors(&mut self) {
        self.set_global(GlobalColors(Arc::new(Colors::default())));
    }

    #[inline(never)]
    fn update_window_erased(
        &mut self,
        window_id: WindowId,
        update: &mut dyn FnMut(Option<(AnyView, &mut Window, &mut App)>),
    ) {
        self.update(|cx| {
            let Some(mut window) = cx.windows.get_mut(window_id).and_then(Option::take) else {
                update(None);
                return;
            };

            let root_view = window.root.clone().unwrap();

            cx.window_update_stack.push(window.handle.id);
            update(Some((root_view, &mut window, cx)));
            fn trail(window_id: WindowId, window: Box<Window>, cx: &mut App) -> Option<()> {
                cx.window_update_stack.pop();

                if window.removed {
                    cx.end_platform_drag(window_id);
                    cx.window_handles.remove(&window_id);
                    cx.windows.remove(window_id);
                    if let Some(tracked) = cx.tracked_entities.remove(&window_id) {
                        for entity_id in tracked {
                            if let Some(windows) =
                                cx.window_invalidators_by_entity.get_mut(&entity_id)
                            {
                                windows.remove(&window_id);
                            }
                            if cx.current_window_by_entity.get(&entity_id) == Some(&window_id) {
                                cx.current_window_by_entity.remove(&entity_id);
                            }
                        }
                    }

                    cx.window_closed_observers.clone().retain(&(), |callback| {
                        callback(cx, window_id);
                        true
                    });

                    let quit_on_empty = match cx.quit_mode {
                        QuitMode::Explicit => false,
                        QuitMode::LastWindowClosed => true,
                        QuitMode::Default => cfg!(not(target_os = "macos")),
                    };

                    if quit_on_empty && cx.windows.is_empty() {
                        cx.quit();
                    }
                } else {
                    cx.windows.get_mut(window_id)?.replace(window);
                }
                Some(())
            }
            if trail(window_id, window, cx).is_none() {
                update(None);
            }
        });
    }

    #[inline(never)]
    fn update_entity_erased(
        &mut self,
        handle: &AnyEntity,
        entity_type: &str,
        update: &mut dyn FnMut(&mut dyn Any, &mut App),
    ) {
        self.update(|cx| {
            let mut lease = cx.entities.lease_erased(handle, entity_type);
            update(lease.entity.as_deref_mut().unwrap(), cx);
            cx.entities.end_lease_erased(handle.entity_id, lease);
        });
    }

    #[inline(never)]
    #[track_caller]
    fn prepare_spawn(&self) -> AsyncApp {
        if self.quitting {
            debug_panic!("Can't spawn on main thread after on_app_quit")
        };
        self.to_async()
    }
}

impl AppContext for App {
    fn new<T: 'static>(&mut self, build_entity: impl FnOnce(&mut Context<T>) -> T) -> Entity<T> {
        self.update(|cx| {
            let slot = cx.entities.reserve();
            let handle = slot.clone();
            let entity = build_entity(&mut Context::new_context(cx, slot.downgrade()));

            cx.push_effect(Effect::EntityCreated {
                entity: handle.into_any(),
                tid: TypeId::of::<T>(),
                window: cx.window_update_stack.last().cloned(),
            });

            cx.entities.insert(slot, entity)
        })
    }

    fn reserve_entity<T: 'static>(&mut self) -> Reservation<T> {
        Reservation(self.entities.reserve())
    }

    fn insert_entity<T: 'static>(
        &mut self,
        reservation: Reservation<T>,
        build_entity: impl FnOnce(&mut Context<T>) -> T,
    ) -> Entity<T> {
        self.update(|cx| {
            let slot = reservation.0;
            let entity = build_entity(&mut Context::new_context(cx, slot.downgrade()));
            cx.entities.insert(slot, entity)
        })
    }

    #[inline(always)]
    fn update_entity<T: 'static, R>(
        &mut self,
        handle: &Entity<T>,
        update: impl FnOnce(&mut T, &mut Context<T>) -> R,
    ) -> R {
        let mut update = Some(update);
        let mut result = None;
        self.update_entity_erased(handle, type_name::<T>(), &mut |entity, cx| {
            let value = update.take().unwrap()(
                entity.downcast_mut::<T>().unwrap(),
                &mut Context::new_context(cx, handle.downgrade()),
            );
            result = Some(value);
        });
        result.unwrap()
    }

    fn as_mut<'a, T>(&'a mut self, handle: &Entity<T>) -> AlaBorrow<'a, T>
    where
        T: 'static,
    {
        AlaBorrow::new(handle.clone(), self)
    }

    #[inline(always)]
    fn read_entity<T, R>(&self, handle: &Entity<T>, read: impl FnOnce(&T, &App) -> R) -> R
    where
        T: 'static,
    {
        let entity = self.entities.read(handle);
        read(entity, self)
    }

    fn update_window<T, F>(&mut self, handle: AnyWindowHandle, update: F) -> Result<T>
    where
        F: FnOnce(AnyView, &mut Window, &mut App) -> T,
    {
        self.update_window_id(handle.id, update)
    }

    fn with_window<R>(
        &mut self,
        entity_id: EntityId,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        App::with_window(self, entity_id, f)
    }

    fn read_window<T, R>(
        &self,
        window: &WindowHandle<T>,
        read: impl FnOnce(Entity<T>, &App) -> R,
    ) -> Result<R>
    where
        T: 'static,
    {
        let window = self
            .windows
            .get(window.id)
            .context("window not found")?
            .as_deref()
            .expect("attempted to read a window that is already on the stack");

        let root_view = window.root.clone().unwrap();
        let view = root_view
            .downcast::<T>()
            .map_err(|_| anyhow!("root view's type has changed"))?;

        Ok(read(view, self))
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
        let mut g = self.global::<G>();
        callback(g, self)
    }
}

pub(crate) enum Effect {
    Notify {
        emitter: EntityId,
    },
    Emit {
        emitter: EntityId,
        event_type: TypeId,
        event: ArenaBox<dyn Any>,
    },
    RefreshWindows,
    NotifyGlobalObservers {
        global_type: TypeId,
    },
    Defer {
        callback: Box<dyn FnOnce(&mut App) + 'static>,
    },
    EntityCreated {
        entity: AnyEntity,
        tid: TypeId,
        window: Option<WindowId>,
    },
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Effect::Notify { emitter } => write!(f, "Notify({})", emitter),
            Effect::Emit { emitter, .. } => write!(f, "Emit({:?})", emitter),
            Effect::RefreshWindows => write!(f, "RefreshWindows"),
            Effect::NotifyGlobalObservers { global_type } => {
                write!(f, "NotifyGlobalObservers({:?})", global_type)
            }
            Effect::Defer { .. } => write!(f, "Defer(..)"),
            Effect::EntityCreated { entity, .. } => write!(f, "EntityCreated({:?})", entity),
        }
    }
}

pub(crate) struct GlobalLease<G: Global> {
    global: Box<dyn Any>,
    global_type: PhantomData<G>,
}

impl<G: Global> GlobalLease<G> {
    fn new(global: Box<dyn Any>) -> Self {
        GlobalLease {
            global,
            global_type: PhantomData,
        }
    }
}

impl<G: Global> Deref for GlobalLease<G> {
    type Target = G;

    fn deref(&self) -> &Self::Target {
        self.global.downcast_ref().unwrap()
    }
}

impl<G: Global> DerefMut for GlobalLease<G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.global.downcast_mut().unwrap()
    }
}

pub struct AnyDrag {
    pub view: AnyView,

    pub value: Arc<dyn Any>,

    pub cursor_offset: Point<Pixels>,

    pub cursor_style: Option<CursorStyle>,

    pub external_payload_source: Option<ExternalDragPayloadSource>,
}

pub type ExternalDragPayloadSource =
    Box<dyn FnOnce(&mut Window, &mut App) -> Option<ExternalDragPayload> + 'static>;

#[derive(Clone)]
pub struct AnyTooltip {
    pub view: AnyView,

    pub mouse_position: Point<Pixels>,

    pub check_visible_and_update: Rc<dyn Fn(Bounds<Pixels>, &mut Window, &mut App) -> bool>,
}

#[derive(Debug)]
pub struct KeystrokeEvent {
    pub keystroke: Keystroke,

    pub action: Option<Box<dyn Action>>,

    pub context_stack: Vec<KeyContext>,

    pub is_held: bool,
}

struct NullHttpClient;

impl HttpClient for NullHttpClient {
    fn send(
        &self,
        _req: crate::http_client::Request<crate::http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<
        'static,
        anyhow::Result<crate::http_client::Response<crate::http_client::AsyncBody>>,
    > {
        async move {
            anyhow::bail!("No HttpClient available");
        }
        .boxed()
    }

    fn user_agent(&self) -> Option<&crate::http_client::http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}

pub struct AlaBorrow<'a, T> {
    inner: Option<Lease<T>>,
    app: &'a mut App,
}

impl<'a, T: 'static> AlaBorrow<'a, T> {
    fn new(inner: Entity<T>, app: &'a mut App) -> Self {
        app.start_update();
        let lease = app.entities.lease(&inner);
        Self {
            inner: Some(lease),
            app,
        }
    }
}

impl<'a, T: 'static> std::borrow::Borrow<T> for AlaBorrow<'a, T> {
    fn borrow(&self) -> &T {
        self.inner.as_ref().unwrap().borrow()
    }
}

impl<'a, T: 'static> std::borrow::BorrowMut<T> for AlaBorrow<'a, T> {
    fn borrow_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap().borrow_mut()
    }
}

impl<'a, T: 'static> std::ops::Deref for AlaBorrow<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().unwrap()
    }
}

impl<'a, T: 'static> std::ops::DerefMut for AlaBorrow<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.inner.as_mut().unwrap()
    }
}

impl<'a, T> Drop for AlaBorrow<'a, T> {
    fn drop(&mut self) {
        let lease = self.inner.take().unwrap();
        self.app.notify(lease.id);
        self.app.entities.end_lease(lease);
        self.app.finish_update();
    }
}

#[cfg(test)]
mod test {
    use std::{
        cell::{Cell, RefCell},
        ffi::OsString,
        path::PathBuf,
        rc::Rc,
    };

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    use crate::{
        AppContext, Context, Empty, FallbackFontClass, FocusHandle, InteractiveElement as _,
        IntoElement, MissingGlyph, ParentElement as _, Render, Styled as _, TestAppContext, Window,
        div, px,
    };

    struct RenderCounter(Rc<Cell<usize>>);

    impl Render for RenderCounter {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.set(self.0.get() + 1);
            Empty
        }
    }

    #[ala::test]
    fn async_app_refresh_flushes_refresh_effect(cx: &mut TestAppContext) {
        let render_count = Rc::new(Cell::new(0));

        let _window = cx.add_window({
            let render_count = render_count.clone();
            move |_, _| RenderCounter(render_count)
        });

        cx.run_until_parked();
        let render_count_before_refresh = render_count.get();

        cx.to_async().refresh();

        assert_eq!(render_count.get(), render_count_before_refresh + 1);
    }

    #[ala::test]
    fn missing_glyph_callbacks_follow_subscription_lifetime(cx: &mut TestAppContext) {
        let observed = Rc::new(RefCell::new(Vec::new()));
        let subscription = cx.update(|cx| {
            let observed = observed.clone();
            cx.on_missing_glyphs(move |missing_glyphs, _| {
                observed.borrow_mut().extend_from_slice(missing_glyphs);
            })
        });
        cx.update(|cx| {
            cx.text_system()
                .report_missing_glyphs_in_test(vec![missing_glyph("active")]);
        });
        cx.run_until_parked();
        assert_eq!(observed.borrow().as_slice(), &[missing_glyph("active")]);

        let second_observed = Rc::new(RefCell::new(Vec::new()));
        let second_subscription = cx.update(|cx| {
            let second_observed = second_observed.clone();
            cx.on_missing_glyphs(move |missing_glyphs, _| {
                second_observed
                    .borrow_mut()
                    .extend_from_slice(missing_glyphs);
            })
        });
        cx.update(|cx| {
            cx.text_system()
                .report_missing_glyphs_in_test(vec![missing_glyph("replacement")]);
        });
        cx.run_until_parked();
        assert_eq!(observed.borrow().as_slice(), &[missing_glyph("active")]);
        assert_eq!(
            second_observed.borrow().as_slice(),
            &[missing_glyph("replacement")]
        );

        drop(subscription);
        cx.update(|cx| {
            cx.text_system()
                .report_missing_glyphs_in_test(vec![missing_glyph("after old drop")]);
        });
        cx.run_until_parked();
        assert_eq!(
            second_observed.borrow().as_slice(),
            &[
                missing_glyph("replacement"),
                missing_glyph("after old drop")
            ]
        );

        drop(second_subscription);
        cx.update(|cx| {
            cx.text_system()
                .report_missing_glyphs_in_test(vec![missing_glyph("inactive")]);
        });
        cx.run_until_parked();
        assert_eq!(second_observed.borrow().len(), 2);
    }

    #[test]
    fn test_ala_borrow() {
        let cx = TestAppContext::single();
        let observation_count = Rc::new(RefCell::new(0));

        let state = cx.update(|cx| {
            let state = cx.new(|_| false);
            cx.observe(&state, {
                let observation_count = observation_count.clone();
                move |_, _| {
                    let mut count = observation_count.borrow_mut();
                    *count += 1;
                }
            })
            .detach();

            state
        });

        cx.update(|cx| {
            *std::borrow::BorrowMut::borrow_mut(&mut state.as_mut(cx)) = true;
        });

        cx.update(|cx| {
            state.write(cx, false);
        });

        assert_eq!(*observation_count.borrow(), 2);
    }

    #[ala::test]
    async fn test_restart_preserves_path_and_arguments(cx: &mut TestAppContext) {
        #[cfg(unix)]
        let user_data_dir = OsString::from_vec(b"/tmp/zed data/\xff".to_vec());
        #[cfg(not(unix))]
        let user_data_dir = OsString::from("C:\\zed data");
        let arguments = vec![OsString::from("--user-data-dir"), user_data_dir];
        let restart_path = PathBuf::from("updated-zed");
        let _application =
            super::Application(cx.app.clone()).with_restart_arguments(arguments.clone());
        let restart = cx.expect_restart();

        cx.update(|cx| {
            cx.set_restart_path(restart_path.clone());
            cx.restart();
        });

        let (path, restart_arguments) = restart.await.expect("restart was not requested");
        assert_eq!(path, Some(restart_path));
        assert_eq!(restart_arguments, arguments);
    }

    struct FocusOwner {
        outer: FocusHandle,
        inner: Option<FocusHandle>,
    }

    impl Render for FocusOwner {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().track_focus(&self.outer).children(
                self.inner
                    .as_ref()
                    .map(|inner| div().size(px(10.)).track_focus(inner)),
            )
        }
    }

    #[ala::test]
    fn dropped_focus_handles_are_released_and_blurred(cx: &mut TestAppContext) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let events = events.clone();
            move |window, cx| {
                let outer = cx.focus_handle();
                let inner = cx.focus_handle();
                cx.on_blur(&inner, window, {
                    let events = events.clone();
                    move |_, _, _| events.borrow_mut().push("blur")
                })
                .detach();
                cx.on_focus_out(&outer, window, {
                    let events = events.clone();
                    move |_, _, _, _| events.borrow_mut().push("focus out")
                })
                .detach();
                FocusOwner {
                    outer,
                    inner: Some(inner),
                }
            }
        });
        window
            .update(cx, |_, window, _| window.activate_window())
            .unwrap();
        cx.run_until_parked();

        let inner_id = window
            .update(cx, |this, window, cx| {
                let inner = this.inner.clone().unwrap();
                window.focus(&inner, cx);
                inner.id
            })
            .unwrap();
        cx.run_until_parked();
        let shared = cx.update(|cx| {
            let shared = cx.focus_handle();
            drop(shared.clone());
            shared
        });
        assert!(events.borrow().is_empty());

        window
            .update(cx, |this, _, cx| {
                this.inner = None;
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window.update(cx, |_, window, _| window.refresh()).unwrap();
        cx.run_until_parked();

        assert_eq!(*events.borrow(), ["blur", "focus out"]);
        window
            .update(cx, |this, window, cx| {
                assert!(window.focused(cx).is_none());
                let handles = cx.focus_handles.read();
                assert!(handles.get(inner_id).is_none());
                assert!(handles.get(this.outer.id).is_some());
                assert!(handles.get(shared.id).is_some());
            })
            .unwrap();
    }

    #[ala::test]
    fn notify_cycle_ends_within_one_flush(cx: &mut TestAppContext) {
        struct Cycle;
        let observed = Rc::new(Cell::new(0));
        let first = cx.update(|cx| {
            let first = cx.new(|_| Cycle);
            let second = cx.new(|_| Cycle);
            cx.observe(&first, {
                let second = second.clone();
                let observed = observed.clone();
                move |_, cx| {
                    observed.set(observed.get() + 1);
                    second.update(cx, |_, cx| cx.notify());
                }
            })
            .detach();
            cx.observe(&second, {
                let first = first.clone();
                let observed = observed.clone();
                move |_, cx| {
                    observed.set(observed.get() + 1);
                    first.update(cx, |_, cx| cx.notify());
                }
            })
            .detach();
            first.update(cx, |_, cx| cx.notify());
            first
        });
        let cut_off = observed.get();
        assert!(
            (2 * super::MAX_NOTIFIES_PER_FLUSH - 1..=2 * super::MAX_NOTIFIES_PER_FLUSH)
                .contains(&cut_off)
        );

        first.update(cx, |_, cx| cx.notify());
        assert_eq!(observed.get(), 2 * cut_off);
    }

    fn missing_glyph(grapheme: &'static str) -> MissingGlyph {
        MissingGlyph::new(grapheme.into(), FallbackFontClass::Proportional)
    }
}
