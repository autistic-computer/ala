mod app_menu;
mod keyboard;
mod keystroke;

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub mod layer_shell;

pub mod popup;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
mod threaded_dispatcher;

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
mod test;

#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
mod visual_test;

pub(crate) type PlatformScreenCaptureFrame = ();

use crate::collections::FxHashMap;
use crate::scheduler::Instant;
pub use crate::scheduler::RunnableMeta;
use crate::{
    Action, AnyWindowHandle, App, AsyncWindowContext, BackgroundExecutor, Bounds,
    DEFAULT_WINDOW_SIZE, DevicePixels, DispatchEventResult, Edges, ExternalDragPayload, Font,
    FontId, FontMetrics, FontRun, ForegroundExecutor, GlyphId, GpuSpecs, Hsla, ImageSource, Keymap,
    LineLayout, MissingGlyphSink, Pixels, PlatformGestures, PlatformInput, Point, Priority,
    RenderGlyphParams, RenderImage, RenderImageParams, RenderSvgParams, Scene, ShapedGlyph,
    ShapedRun, SharedString, Size, SvgRenderer, SystemWindowTab, Task, Window, WindowControlArea,
    hash, point, px, size,
};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use anyhow::bail;
use anyhow::{Context as _, Result};
use async_task::Runnable;
use futures::channel::oneshot;
#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
use image::RgbaImage;
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder as _, DynamicImage, Frame};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use schemars::JsonSchema;
use seahash::SeaHasher;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::ops;
use std::time::Duration;
use std::{
    ffi::OsString,
    fmt::{self, Debug},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};
use strum::EnumIter;
use uuid::Uuid;

pub use app_menu::*;
pub use keyboard::*;
pub use keystroke::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowVisibility {
    Visible,
    Hidden,
}

impl WindowVisibility {
    pub fn is_visible(self) -> bool {
        self == Self::Visible
    }
}

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub(crate) use test::*;

#[cfg(any(test, feature = "test-support"))]
pub use test::{TestDispatcher, TestScreenCaptureSource, TestScreenCaptureStream};

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub use threaded_dispatcher::ThreadedDispatcher;

#[cfg(all(target_os = "macos", any(test, feature = "test-support")))]
pub use visual_test::VisualTestPlatform;

pub struct ActivityGuard {
    _release: crate::util::Deferred<Box<dyn FnOnce() + Send>>,
}

impl ActivityGuard {
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        Self {
            _release: crate::util::defer(Box::new(release)),
        }
    }

    pub fn noop() -> Self {
        Self::new(|| {})
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[inline]
pub fn guess_compositor() -> &'static str {
    if std::env::var_os("ZED_HEADLESS").is_some() {
        return "Headless";
    }

    #[cfg(feature = "wayland")]
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY");
    #[cfg(not(feature = "wayland"))]
    let wayland_display: Option<std::ffi::OsString> = None;

    #[cfg(feature = "x11")]
    let x11_display = std::env::var_os("DISPLAY");
    #[cfg(not(feature = "x11"))]
    let x11_display: Option<std::ffi::OsString> = None;

    let use_wayland = wayland_display.is_some_and(|display| !display.is_empty());
    let use_x11 = x11_display.is_some_and(|display| !display.is_empty());

    if use_wayland {
        "Wayland"
    } else if use_x11 {
        "X11"
    } else {
        "Headless"
    }
}

pub trait Platform: 'static {
    fn background_executor(&self) -> BackgroundExecutor;
    fn foreground_executor(&self) -> ForegroundExecutor;
    fn text_system(&self) -> Arc<dyn PlatformTextSystem>;

    fn run(&self, on_finish_launching: Box<dyn 'static + FnOnce()>);
    fn quit(&self);
    fn restart(&self, binary_path: Option<PathBuf>, arguments: Vec<OsString>);
    fn activate(&self, ignoring_other_apps: bool);
    fn hide(&self);
    fn hide_other_apps(&self);
    fn unhide_other_apps(&self);

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>>;
    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>>;
    fn active_window(&self) -> Option<AnyWindowHandle>;
    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        None
    }

    fn is_screen_capture_supported(&self) -> bool {
        false
    }

    fn screen_capture_sources(
        &self,
    ) -> oneshot::Receiver<anyhow::Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
        let (sources_tx, sources_rx) = oneshot::channel();
        sources_tx
            .send(Err(anyhow::anyhow!(
                "ala was compiled without the screen-capture feature"
            )))
            .ok();
        sources_rx
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        options: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>>;

    fn window_appearance(&self) -> WindowAppearance;

    fn set_window_appearance(&self, _appearance: Option<WindowAppearance>) {}

    fn button_layout(&self) -> Option<WindowButtonLayout> {
        None
    }

    fn open_url(&self, url: &str);
    fn on_open_urls(&self, callback: Box<dyn FnMut(Vec<String>)>);
    fn register_url_scheme(&self, url: &str) -> Task<Result<()>>;

    fn prompt_for_paths(
        &self,
        options: PathPromptOptions,
    ) -> oneshot::Receiver<Result<Option<Vec<PathBuf>>>>;
    fn prompt_for_new_path(
        &self,
        directory: &Path,
        suggested_name: Option<&str>,
    ) -> oneshot::Receiver<Result<Option<PathBuf>>>;
    fn can_select_mixed_files_and_dirs(&self) -> bool;
    fn reveal_path(&self, path: &Path);
    fn open_with_system(&self, path: &Path);

    fn on_quit(&self, callback: Box<dyn FnMut() -> bool>);
    fn on_reopen(&self, callback: Box<dyn FnMut()>);
    fn on_system_sleep(&self, callback: Box<dyn FnMut()>);
    fn on_system_wake(&self, callback: Box<dyn FnMut()>);

    fn on_app_lifecycle(&self, _callback: Box<dyn FnMut(AppLifecyclePhase)>) {}

    fn on_memory_warning(&self, _callback: Box<dyn FnMut()>) {}

    fn gestures(&self) -> Option<Rc<dyn PlatformGestures>> {
        None
    }

    fn set_menus(&self, menus: Vec<Menu>, keymap: &Keymap);
    fn get_menus(&self) -> Option<Vec<OwnedMenu>> {
        None
    }

    fn set_dock_menu(&self, menu: Vec<MenuItem>, keymap: &Keymap);
    fn perform_dock_menu_action(&self, _action: usize) {}
    fn add_recent_document(&self, _path: &Path) {}
    fn update_jump_list(
        &self,
        _menus: Vec<MenuItem>,
        _entries: Vec<SmallVec<[PathBuf; 2]>>,
    ) -> Task<Vec<SmallVec<[PathBuf; 2]>>> {
        Task::ready(Vec::new())
    }
    fn on_app_menu_action(&self, callback: Box<dyn FnMut(&dyn Action)>);
    fn on_will_open_app_menu(&self, callback: Box<dyn FnMut()>);
    fn on_validate_app_menu_command(&self, callback: Box<dyn FnMut(&dyn Action) -> bool>);

    fn thermal_state(&self) -> ThermalState;
    fn on_thermal_state_change(&self, callback: Box<dyn FnMut()>);
    fn main_thread_busy(&self) -> Option<crate::scheduler::MainThreadBusy> {
        None
    }
    fn prevent_idle_sleep(&self, reason: &str) -> Task<Result<ActivityGuard>>;

    fn set_app_identity(&self, identifier: &str, name: &str) {
        _ = (identifier, name);
    }

    fn show_system_notification(&self, notification: SystemNotification) {
        _ = notification;
    }

    fn dismiss_system_notification(&self, tag: &str) {
        _ = tag;
    }

    fn on_system_notification_response(
        &self,
        callback: Box<dyn FnMut(SystemNotificationResponse)>,
    ) {
        _ = callback;
    }

    fn compositor_name(&self) -> &'static str {
        ""
    }
    fn app_path(&self) -> Result<PathBuf>;
    fn path_for_auxiliary_executable(&self, name: &str) -> Result<PathBuf>;

    fn set_cursor_style(&self, style: CursorStyle);

    fn hide_cursor_until_mouse_moves(&self);

    fn is_cursor_visible(&self) -> bool;

    fn should_auto_hide_scrollbars(&self) -> bool;

    fn read_from_clipboard(&self) -> Option<ClipboardItem>;
    fn write_to_clipboard(&self, item: ClipboardItem);

    fn read_from_clipboard_async(&self) -> Task<Result<Option<ClipboardItem>, ClipboardReadError>> {
        Task::ready(Ok(self.read_from_clipboard()))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn read_from_primary(&self) -> Option<ClipboardItem>;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn write_to_primary(&self, item: ClipboardItem);

    #[cfg(target_os = "macos")]
    fn read_from_find_pasteboard(&self) -> Option<ClipboardItem>;
    #[cfg(target_os = "macos")]
    fn write_to_find_pasteboard(&self, item: ClipboardItem);

    fn write_credentials(&self, url: &str, username: &str, password: &[u8]) -> Task<Result<()>>;
    fn read_credentials(&self, url: &str) -> Task<Result<Option<(String, Vec<u8>)>>>;
    fn delete_credentials(&self, url: &str) -> Task<Result<()>>;

    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout>;
    fn keyboard_mapper(&self) -> Rc<dyn PlatformKeyboardMapper>;
    fn on_keyboard_layout_change(&self, callback: Box<dyn FnMut()>);
}

pub trait PlatformDisplay: Debug {
    fn id(&self) -> DisplayId;

    fn uuid(&self) -> Result<Uuid>;

    fn bounds(&self) -> Bounds<Pixels>;

    fn visible_bounds(&self) -> Bounds<Pixels> {
        self.bounds()
    }

    fn refresh_rate(&self) -> Option<f32> {
        None
    }

    fn default_bounds(&self) -> Bounds<Pixels> {
        let bounds = self.bounds();
        let center = bounds.center();
        let clipped_window_size = DEFAULT_WINDOW_SIZE.min(&bounds.size);

        let offset = clipped_window_size / 2.0;
        let origin = point(center.x - offset.width, center.y - offset.height);
        Bounds::new(origin, clipped_window_size)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemNotification {
    pub tag: SharedString,
    pub title: SharedString,
    pub body: SharedString,
    pub actions: Vec<SystemNotificationAction>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SystemNotificationAction {
    pub id: SharedString,
    pub label: SharedString,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemNotificationResponse {
    pub tag: SharedString,
    pub action_id: Option<SharedString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(Clone)]
pub struct SourceMetadata {
    pub id: u64,
    pub label: Option<SharedString>,
    pub is_main: Option<bool>,
    pub resolution: Size<DevicePixels>,
}

pub trait ScreenCaptureSource {
    fn metadata(&self) -> Result<SourceMetadata>;

    fn stream(
        &self,
        foreground_executor: &ForegroundExecutor,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>>;
}

pub trait ScreenCaptureStream {
    fn metadata(&self) -> Result<SourceMetadata>;
}

pub struct ScreenCaptureFrame(pub PlatformScreenCaptureFrame);

#[derive(PartialEq, Eq, Hash, Copy, Clone)]
pub struct DisplayId(pub(crate) u64);

impl DisplayId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

impl From<u64> for DisplayId {
    fn from(id: u64) -> Self {
        Self(id)
    }
}

impl From<DisplayId> for u64 {
    fn from(id: DisplayId) -> Self {
        id.0
    }
}

impl Debug for DisplayId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DisplayId({})", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
    TopLeft,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum WindowDecorations {
    #[default]
    Server,
    Client,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub enum Decorations {
    #[default]
    Server,
    Client {
        tiling: Tiling,
    },
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct WindowControls {
    pub fullscreen: bool,
    pub maximize: bool,
    pub minimize: bool,
    pub window_menu: bool,
}

impl Default for WindowControls {
    fn default() -> Self {
        Self {
            fullscreen: true,
            maximize: true,
            minimize: true,
            window_menu: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

impl WindowButton {
    pub fn id(&self) -> &'static str {
        match self {
            WindowButton::Minimize => "minimize",
            WindowButton::Maximize => "maximize",
            WindowButton::Close => "close",
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn index(&self) -> usize {
        match self {
            WindowButton::Minimize => 0,
            WindowButton::Maximize => 1,
            WindowButton::Close => 2,
        }
    }
}

pub const MAX_BUTTONS_PER_SIDE: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowButtonLayout {
    pub left: [Option<WindowButton>; MAX_BUTTONS_PER_SIDE],
    pub right: [Option<WindowButton>; MAX_BUTTONS_PER_SIDE],
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
impl WindowButtonLayout {
    pub fn linux_default() -> Self {
        Self {
            left: [None; MAX_BUTTONS_PER_SIDE],
            right: [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close),
            ],
        }
    }

    pub fn parse(layout_string: &str) -> Result<Self> {
        fn parse_side(
            s: &str,
            seen_buttons: &mut [bool; MAX_BUTTONS_PER_SIDE],
            unrecognized: &mut Vec<String>,
        ) -> [Option<WindowButton>; MAX_BUTTONS_PER_SIDE] {
            let mut result = [None; MAX_BUTTONS_PER_SIDE];
            let mut i = 0;
            for name in s.split(',') {
                let trimmed = name.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let button = match trimmed {
                    "minimize" => Some(WindowButton::Minimize),
                    "maximize" => Some(WindowButton::Maximize),
                    "close" => Some(WindowButton::Close),
                    other => {
                        unrecognized.push(other.to_string());
                        None
                    }
                };
                if let Some(button) = button {
                    if seen_buttons[button.index()] {
                        continue;
                    }
                    if let Some(slot) = result.get_mut(i) {
                        *slot = Some(button);
                        seen_buttons[button.index()] = true;
                        i += 1;
                    }
                }
            }
            result
        }

        let (left_str, right_str) = layout_string.split_once(':').unwrap_or(("", layout_string));
        let mut unrecognized = Vec::new();
        let mut seen_buttons = [false; MAX_BUTTONS_PER_SIDE];
        let layout = Self {
            left: parse_side(left_str, &mut seen_buttons, &mut unrecognized),
            right: parse_side(right_str, &mut seen_buttons, &mut unrecognized),
        };

        if !unrecognized.is_empty()
            && layout.left.iter().all(Option::is_none)
            && layout.right.iter().all(Option::is_none)
        {
            bail!(
                "button layout string {:?} contains no valid buttons (unrecognized: {})",
                layout_string,
                unrecognized.join(", ")
            );
        }

        Ok(layout)
    }

    #[cfg(test)]
    pub fn format(&self) -> String {
        fn format_side(buttons: &[Option<WindowButton>; MAX_BUTTONS_PER_SIDE]) -> String {
            buttons
                .iter()
                .flatten()
                .map(|button| match button {
                    WindowButton::Minimize => "minimize",
                    WindowButton::Maximize => "maximize",
                    WindowButton::Close => "close",
                })
                .collect::<Vec<_>>()
                .join(",")
        }

        format!("{}:{}", format_side(&self.left), format_side(&self.right))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Default)]
pub struct Tiling {
    pub top: bool,
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Tiling {
    pub fn tiled() -> Self {
        Self {
            top: true,
            left: true,
            right: true,
            bottom: true,
        }
    }

    pub fn is_tiled(&self) -> bool {
        self.top || self.left || self.right || self.bottom
    }
}

pub struct A11yCallbacks {
    pub activation: Box<dyn Fn() -> Option<accesskit::TreeUpdate> + Send + 'static>,
    pub action: Box<dyn Fn(accesskit::ActionRequest) + Send + 'static>,
    pub deactivation: Box<dyn Fn() + Send + 'static>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Default)]
pub struct RequestFrameOptions {
    pub require_presentation: bool,
    pub force_render: bool,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(Clone, Debug)]
pub struct WgpuDeviceContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
}

#[cfg(target_os = "windows")]
#[derive(Clone, Debug)]
pub struct DirectXDeviceContext {
    pub device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    pub device_context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum AppLifecyclePhase {
    Active,
    Inactive,
    Background,
    Foreground,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowInsets {
    pub safe_area: Edges<Pixels>,
    pub ime: Edges<Pixels>,
}

impl WindowInsets {
    pub fn effective(&self) -> Edges<Pixels> {
        Edges {
            top: self.safe_area.top.max(self.ime.top),
            right: self.safe_area.right.max(self.ime.right),
            bottom: self.safe_area.bottom.max(self.ime.bottom),
            left: self.safe_area.left.max(self.ime.left),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum TextInputStateChange {
    FocusGained,
    FocusLost,
    SelectionChanged,
    ContentChanged,
}

pub trait PlatformWindow: HasWindowHandle + HasDisplayHandle {
    fn bounds(&self) -> Bounds<Pixels>;
    fn is_maximized(&self) -> bool;
    fn window_bounds(&self) -> WindowBounds;
    fn content_size(&self) -> Size<Pixels>;
    fn visual_viewport_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(Point::default(), self.content_size())
    }
    fn on_visual_viewport_changed(&self, _callback: Box<dyn FnMut()>) {}
    fn target_visual_viewport_bounds(&self) -> Bounds<Pixels> {
        self.visual_viewport_bounds()
    }
    fn prepare_frame(&self) -> bool {
        false
    }
    fn resize(&mut self, size: Size<Pixels>);
    fn scale_factor(&self) -> f32;
    fn appearance(&self) -> WindowAppearance;
    fn display(&self) -> Option<Rc<dyn PlatformDisplay>>;
    fn mouse_position(&self) -> Point<Pixels>;
    fn modifiers(&self) -> Modifiers;
    fn capslock(&self) -> Capslock;
    fn set_input_handler(&mut self, input_handler: PlatformInputHandler);
    fn take_input_handler(&mut self) -> Option<PlatformInputHandler>;
    fn set_text_input_configuration(&mut self, _configuration: TextInputConfiguration) {}
    fn prompt(
        &self,
        level: PromptLevel,
        msg: &str,
        detail: Option<&str>,
        answers: &[PromptButton],
    ) -> Option<oneshot::Receiver<usize>>;
    fn activate(&self);
    fn activate_with_token(&self, _token: String) {
        self.activate();
    }
    fn request_attention(&self) {}
    fn is_active(&self) -> bool;
    fn visibility(&self) -> WindowVisibility;
    fn is_hovered(&self) -> bool;
    fn background_appearance(&self) -> WindowBackgroundAppearance;
    fn set_title(&mut self, title: &str);
    fn set_background_appearance(&self, background_appearance: WindowBackgroundAppearance);
    fn set_underlay_active(&self, _active: bool) {}
    fn minimize(&self);
    fn set_visible(&self, _visible: bool) {}
    fn is_visible(&self) -> bool {
        true
    }
    fn zoom(&self);
    fn toggle_fullscreen(&self);
    fn is_fullscreen(&self) -> bool;
    fn frame_waker(&self) -> Option<Rc<dyn Fn()>> {
        None
    }
    fn on_request_frame(&self, callback: Box<dyn FnMut(RequestFrameOptions)>);
    fn on_input(&self, callback: Box<dyn FnMut(PlatformInput) -> DispatchEventResult>);
    fn on_active_status_change(&self, callback: Box<dyn FnMut(bool)>);
    fn on_visibility_change(&self, callback: Box<dyn FnMut(WindowVisibility)>);
    fn on_hover_status_change(&self, callback: Box<dyn FnMut(bool)>);
    fn on_resize(&self, callback: Box<dyn FnMut(Size<Pixels>, f32)>);
    fn on_moved(&self, callback: Box<dyn FnMut()>);
    fn on_should_close(&self, callback: Box<dyn FnMut() -> bool>);
    fn on_hit_test_window_control(&self, callback: Box<dyn FnMut() -> Option<WindowControlArea>>);
    fn on_close(&self, callback: Box<dyn FnOnce()>);
    fn on_appearance_changed(&self, callback: Box<dyn FnMut()>);
    fn on_button_layout_changed(&self, _callback: Box<dyn FnMut()>) {}
    fn draw(&self, scene: &Scene);
    fn schedule_frame(&self) {}
    fn schedule_frame_after(&self, _delay: Duration) {
        self.schedule_frame();
    }
    fn keeps_presenting_after_input(&self) -> bool {
        true
    }
    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
    fn is_subpixel_rendering_supported(&self) -> bool;
    fn supports_backdrop_sampling(&self) -> bool {
        false
    }

    fn get_title(&self) -> String {
        String::new()
    }
    fn tabbed_windows(&self) -> Option<Vec<SystemWindowTab>> {
        None
    }
    fn tab_bar_visible(&self) -> bool {
        false
    }
    fn set_edited(&mut self, _edited: bool) {}
    fn set_document_path(&self, _path: Option<&std::path::Path>) {}
    fn toggle_simple_fullscreen(&self) {}
    fn is_simple_fullscreen(&self) -> bool {
        false
    }
    #[cfg(target_os = "macos")]
    fn set_traffic_light_position(&self, _position: Point<Pixels>) {}
    fn show_character_palette(&self) {}
    fn titlebar_double_click(&self, _is_resizable: bool, _is_minimizable: bool) {}
    fn on_move_tab_to_new_window(&self, _callback: Box<dyn FnMut()>) {}
    fn on_merge_all_windows(&self, _callback: Box<dyn FnMut()>) {}
    fn on_select_previous_tab(&self, _callback: Box<dyn FnMut()>) {}
    fn on_select_next_tab(&self, _callback: Box<dyn FnMut()>) {}
    fn on_toggle_tab_bar(&self, _callback: Box<dyn FnMut()>) {}
    fn merge_all_windows(&self) {}
    fn move_tab_to_new_window(&self) {}
    fn toggle_window_tab_overview(&self) {}
    fn set_tabbing_identifier(&self, _identifier: Option<String>) {}

    fn native_window_state(&self) -> Option<Vec<u8>> {
        None
    }
    fn restore_native_window_state(&self, _state: &[u8]) {}

    #[cfg(target_os = "windows")]
    fn get_raw_handle(&self) -> windows::Win32::Foundation::HWND;

    fn inner_window_bounds(&self) -> WindowBounds {
        self.window_bounds()
    }
    fn request_decorations(&self, _decorations: WindowDecorations) {}
    fn show_window_menu(&self, _position: Point<Pixels>) {}
    fn start_window_move(&self) {}
    fn can_start_external_drag(&self) -> bool {
        false
    }
    fn start_external_drag(&self, _payload: &ExternalDragPayload) -> bool {
        false
    }
    fn start_window_resize(&self, _edge: ResizeEdge) {}
    fn set_exclusive_zone(&self, _zone: Pixels) {}
    #[cfg(all(target_os = "linux", feature = "wayland"))]
    fn set_exclusive_edge(&self, _edge: layer_shell::Anchor) {}
    fn set_input_region(&self, _region: Option<&[Bounds<Pixels>]>) {}
    fn window_decorations(&self) -> Decorations {
        Decorations::Server
    }
    fn set_app_id(&mut self, _app_id: &str) {}
    fn map_window(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
    fn window_controls(&self) -> WindowControls {
        WindowControls::default()
    }
    fn set_client_inset(&self, _inset: Pixels) {}
    fn gpu_specs(&self) -> Option<GpuSpecs>;

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    fn wgpu_device_context(&self) -> Option<WgpuDeviceContext> {
        None
    }

    #[cfg(target_os = "windows")]
    fn directx_device_context(&self) -> Option<DirectXDeviceContext> {
        None
    }

    fn update_ime_position(&self, _bounds: Bounds<Pixels>);

    fn insets(&self) -> WindowInsets {
        WindowInsets::default()
    }

    fn on_insets_changed(&self, _callback: Box<dyn FnMut(WindowInsets)>) {}

    fn set_back_handler(&self, _callback: Box<dyn FnMut()>) {}

    fn set_back_enabled(&self, _enabled: bool) {}

    fn show_soft_keyboard(&self) {}

    fn hide_soft_keyboard(&self) {}

    fn text_input_state_changed(&self, _change: TextInputStateChange) {}

    fn play_system_bell(&self) {}

    fn a11y_init(&self, _callbacks: A11yCallbacks) {}

    fn a11y_tree_update(&self, _tree_update: accesskit::TreeUpdate) {}

    fn a11y_update_window_bounds(&self) {}

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_test(&mut self) -> Option<&mut TestWindow> {
        None
    }

    #[cfg(any(test, feature = "test-support"))]
    fn render_to_image(&self, _scene: &Scene) -> Result<RgbaImage> {
        anyhow::bail!("render_to_image not implemented for this platform")
    }
}

#[cfg(any(test, feature = "test-support", feature = "bench-support"))]
pub trait PlatformHeadlessRenderer {
    fn render_scene_to_image(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
    ) -> Result<RgbaImage>;

    fn render_scene(&mut self, scene: &Scene, size: Size<DevicePixels>) -> Result<()>;

    fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas>;
}

#[doc(hidden)]
pub type RunnableVariant = Runnable<RunnableMeta>;

#[doc(hidden)]
pub type TimerResolutionGuard = crate::util::Deferred<Box<dyn FnOnce() + Send>>;

#[doc(hidden)]
pub enum TasksIncluded {
    OnlyCompleted,
    CompletedAndRunning,
}

#[doc(hidden)]
pub trait PlatformDispatcher: Send + Sync {
    fn is_main_thread(&self) -> bool;
    fn dispatch(&self, runnable: RunnableVariant, priority: Priority);
    fn dispatch_on_main_thread(&self, runnable: RunnableVariant, priority: Priority);
    fn dispatch_after(&self, duration: Duration, runnable: RunnableVariant);

    fn dispatch_on_main_thread_when_idle(
        &self,
        runnable: RunnableVariant,
        timeout: Option<Duration>,
    ) {
        let _ = timeout;
        self.dispatch_on_main_thread(runnable, Priority::Low);
    }

    fn idle_time_remaining(&self) -> Option<Duration> {
        None
    }

    fn spawn_realtime(&self, f: Box<dyn FnOnce() + Send>);

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn increase_timer_resolution(&self) -> TimerResolutionGuard {
        crate::util::defer(Box::new(|| {}))
    }

    fn prevent_app_nap(&self, _reason: &str) -> ActivityGuard {
        ActivityGuard::noop()
    }

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_test(&self) -> Option<&TestDispatcher> {
        None
    }

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn as_threaded(&self) -> Option<&ThreadedDispatcher> {
        None
    }
}

pub trait PlatformTextSystem: Send + Sync {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()>;
    fn set_missing_glyph_sink(&self, _sink: Option<Arc<dyn MissingGlyphSink>>) {}
    fn all_font_names(&self) -> Vec<String>;
    fn font_id(&self, descriptor: &Font) -> Result<FontId>;
    fn prewarm_fonts(&self, _font_ids: &[FontId]) {}
    fn font_generation(&self) -> u64 {
        0
    }
    fn font_metrics(&self, font_id: FontId) -> FontMetrics;
    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>>;
    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>>;
    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId>;
    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>>;
    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)>;
    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout;
    fn recommended_rendering_mode(&self, _font_id: FontId, _font_size: Pixels)
    -> TextRenderingMode;
    fn glyph_dilation_for_color(&self, _color: Hsla) -> u8 {
        0
    }
}

pub struct NoopTextSystem;

impl NoopTextSystem {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self
    }
}

impl PlatformTextSystem for NoopTextSystem {
    fn add_fonts(&self, _fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        Ok(())
    }

    fn all_font_names(&self) -> Vec<String> {
        Vec::new()
    }

    fn font_id(&self, _descriptor: &Font) -> Result<FontId> {
        Ok(FontId(1))
    }

    fn font_metrics(&self, _font_id: FontId) -> FontMetrics {
        FontMetrics {
            units_per_em: 1000,
            ascent: 1025.0,
            descent: -275.0,
            line_gap: 0.0,
            underline_position: -95.0,
            underline_thickness: 60.0,
            cap_height: 698.0,
            x_height: 516.0,
            bounding_box: Bounds {
                origin: Point {
                    x: -260.0,
                    y: -245.0,
                },
                size: Size {
                    width: 1501.0,
                    height: 1364.0,
                },
            },
        }
    }

    fn typographic_bounds(&self, _font_id: FontId, _glyph_id: GlyphId) -> Result<Bounds<f32>> {
        Ok(Bounds {
            origin: Point { x: 54.0, y: 0.0 },
            size: size(392.0, 528.0),
        })
    }

    fn advance(&self, _font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        Ok(size(600.0 * glyph_id.0 as f32, 0.0))
    }

    fn glyph_for_char(&self, _font_id: FontId, ch: char) -> Option<GlyphId> {
        Some(GlyphId(ch.len_utf16() as u32))
    }

    fn glyph_raster_bounds(&self, _params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        Ok(Default::default())
    }

    fn rasterize_glyph(
        &self,
        _params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        Ok((raster_bounds.size, Vec::new()))
    }

    fn layout_line(&self, text: &str, font_size: Pixels, _runs: &[FontRun]) -> LineLayout {
        let mut position = px(0.);
        let metrics = self.font_metrics(FontId(0));
        let em_width = font_size
            * self
                .advance(FontId(0), self.glyph_for_char(FontId(0), 'm').unwrap())
                .unwrap()
                .width
            / metrics.units_per_em as f32;
        let mut glyphs = Vec::new();
        for (ix, c) in text.char_indices() {
            if let Some(glyph) = self.glyph_for_char(FontId(0), c) {
                glyphs.push(ShapedGlyph {
                    id: glyph,
                    position: point(position, px(0.)),
                    index: ix,
                    is_emoji: glyph.0 == 2,
                });
                if glyph.0 == 2 {
                    position += em_width * 2.0;
                } else {
                    position += em_width;
                }
            } else {
                position += em_width
            }
        }
        let mut runs = Vec::default();
        if !glyphs.is_empty() {
            runs.push(ShapedRun {
                font_id: FontId(0),
                glyphs,
            });
        } else {
            position = px(0.);
        }

        LineLayout {
            font_size,
            width: position,
            ascent: font_size * (metrics.ascent / metrics.units_per_em as f32),
            descent: font_size * (metrics.descent / metrics.units_per_em as f32),
            runs,
            len: text.len(),
        }
    }

    fn recommended_rendering_mode(
        &self,
        _font_id: FontId,
        _font_size: Pixels,
    ) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
}

#[allow(dead_code)]
pub fn get_gamma_correction_ratios(gamma: f32) -> [f32; 4] {
    const GAMMA_INCORRECT_TARGET_RATIOS: [[f32; 4]; 13] = [
        [0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0, 0.0000 / 4.0],
        [0.0166 / 4.0, -0.0807 / 4.0, 0.2227 / 4.0, -0.0751 / 4.0],
        [0.0350 / 4.0, -0.1760 / 4.0, 0.4325 / 4.0, -0.1370 / 4.0],
        [0.0543 / 4.0, -0.2821 / 4.0, 0.6302 / 4.0, -0.1876 / 4.0],
        [0.0739 / 4.0, -0.3963 / 4.0, 0.8167 / 4.0, -0.2287 / 4.0],
        [0.0933 / 4.0, -0.5161 / 4.0, 0.9926 / 4.0, -0.2616 / 4.0],
        [0.1121 / 4.0, -0.6395 / 4.0, 1.1588 / 4.0, -0.2877 / 4.0],
        [0.1300 / 4.0, -0.7649 / 4.0, 1.3159 / 4.0, -0.3080 / 4.0],
        [0.1469 / 4.0, -0.8911 / 4.0, 1.4644 / 4.0, -0.3234 / 4.0],
        [0.1627 / 4.0, -1.0170 / 4.0, 1.6051 / 4.0, -0.3347 / 4.0],
        [0.1773 / 4.0, -1.1420 / 4.0, 1.7385 / 4.0, -0.3426 / 4.0],
        [0.1908 / 4.0, -1.2652 / 4.0, 1.8650 / 4.0, -0.3476 / 4.0],
        [0.2031 / 4.0, -1.3864 / 4.0, 1.9851 / 4.0, -0.3501 / 4.0],
    ];

    const NORM13: f32 = ((0x10000 as f64) / (255.0 * 255.0) * 4.0) as f32;
    const NORM24: f32 = ((0x100 as f64) / (255.0) * 4.0) as f32;

    let index = ((gamma * 10.0).round() as usize).clamp(10, 22) - 10;
    let ratios = GAMMA_INCORRECT_TARGET_RATIOS[index];

    [
        ratios[0] * NORM13,
        ratios[1] * NORM24,
        ratios[2] * NORM13,
        ratios[3] * NORM24,
    ]
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub enum AtlasKey {
    Glyph(RenderGlyphParams),
    Svg(RenderSvgParams),
    Image(RenderImageParams),
}

impl AtlasKey {
    pub fn texture_kind(&self) -> AtlasTextureKind {
        match self {
            AtlasKey::Glyph(params) => {
                if params.is_emoji {
                    AtlasTextureKind::Polychrome
                } else if params.subpixel_rendering {
                    AtlasTextureKind::Subpixel
                } else {
                    AtlasTextureKind::Monochrome
                }
            }
            AtlasKey::Svg(_) => AtlasTextureKind::Monochrome,
            AtlasKey::Image(_) => AtlasTextureKind::Polychrome,
        }
    }
}

impl From<RenderGlyphParams> for AtlasKey {
    fn from(params: RenderGlyphParams) -> Self {
        Self::Glyph(params)
    }
}

impl From<RenderSvgParams> for AtlasKey {
    fn from(params: RenderSvgParams) -> Self {
        Self::Svg(params)
    }
}

impl From<RenderImageParams> for AtlasKey {
    fn from(params: RenderImageParams) -> Self {
        Self::Image(params)
    }
}

pub trait PlatformAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>>;
    fn remove(&self, key: &AtlasKey);
    fn generation(&self) -> u64;

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn contains(&self, _key: &AtlasKey) -> bool {
        false
    }
}

#[doc(hidden)]
pub trait AtlasBackend {
    fn insert(
        &mut self,
        kind: AtlasTextureKind,
        size: Size<DevicePixels>,
        bytes: &[u8],
    ) -> Result<AtlasTile>;

    fn remove(&mut self, tile: AtlasTile);
}

#[doc(hidden)]
pub struct AtlasState<Backend> {
    tiles_by_key: FxHashMap<AtlasKey, AtlasTile>,
    generation: u64,
    pub backend: Backend,
}

impl<Backend> AtlasState<Backend> {
    pub fn new(backend: Backend) -> Self {
        Self {
            tiles_by_key: FxHashMap::default(),
            generation: 0,
            backend,
        }
    }

    pub fn contains(&self, key: &AtlasKey) -> bool {
        self.tiles_by_key.contains_key(key)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn clear(&mut self, reset_backend: impl FnOnce(&mut Backend)) {
        self.tiles_by_key.clear();
        self.generation += 1;
        reset_backend(&mut self.backend);
    }
}

impl<Backend: Default> Default for AtlasState<Backend> {
    fn default() -> Self {
        Self::new(Backend::default())
    }
}

impl<Backend: AtlasBackend> AtlasState<Backend> {
    pub fn get_or_insert_with<'a>(
        &mut self,
        key: AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        match self.tiles_by_key.entry(key) {
            Entry::Occupied(entry) => Ok(Some(*entry.get())),
            Entry::Vacant(entry) => {
                profiling::scope!("new tile");
                let Some((size, bytes)) = build()? else {
                    return Ok(None);
                };
                let tile = self
                    .backend
                    .insert(entry.key().texture_kind(), size, &bytes)?;
                entry.insert(tile);
                Ok(Some(tile))
            }
        }
    }

    pub fn remove(&mut self, key: &AtlasKey) {
        if let Some(tile) = self.tiles_by_key.remove(key) {
            if matches!(key, AtlasKey::Glyph(_)) {
                self.generation += 1;
            }
            self.backend.remove(tile);
        }
    }
}

#[derive(Default)]
pub struct HeadlessAtlas(parking_lot::Mutex<AtlasState<HeadlessAtlasBackend>>);

#[doc(hidden)]
#[derive(Default)]
pub struct HeadlessAtlasBackend {
    next_id: u32,
}

impl AtlasBackend for HeadlessAtlasBackend {
    fn insert(
        &mut self,
        kind: AtlasTextureKind,
        size: Size<DevicePixels>,
        _bytes: &[u8],
    ) -> Result<AtlasTile> {
        self.next_id += 1;
        let texture_id = self.next_id;
        self.next_id += 1;
        let tile_id = self.next_id;
        Ok(AtlasTile {
            texture_id: AtlasTextureId {
                index: texture_id,
                kind,
            },
            tile_id: TileId(tile_id),
            padding: 0,
            bounds: Bounds {
                origin: Point::default(),
                size,
            },
        })
    }

    fn remove(&mut self, _tile: AtlasTile) {}
}

impl PlatformAtlas for HeadlessAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        self.0.lock().get_or_insert_with(key, build)
    }

    fn remove(&self, key: &AtlasKey) {
        self.0.lock().remove(key);
    }

    fn generation(&self) -> u64 {
        self.0.lock().generation()
    }

    #[cfg(any(test, feature = "test-support", feature = "bench-support"))]
    fn contains(&self, key: &AtlasKey) -> bool {
        self.0.lock().contains(key)
    }
}

#[doc(hidden)]
pub struct AtlasTextureList<T> {
    pub textures: Vec<Option<T>>,
    pub free_list: Vec<usize>,
}

impl<T> Default for AtlasTextureList<T> {
    fn default() -> Self {
        Self {
            textures: Vec::default(),
            free_list: Vec::default(),
        }
    }
}

impl<T> ops::Index<usize> for AtlasTextureList<T> {
    type Output = Option<T>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.textures[index]
    }
}

impl<T> AtlasTextureList<T> {
    #[allow(unused)]
    pub fn drain(&mut self) -> std::vec::Drain<'_, Option<T>> {
        self.free_list.clear();
        self.textures.drain(..)
    }

    #[allow(dead_code)]
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut T> {
        self.textures.iter_mut().flatten()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct AtlasTile {
    pub texture_id: AtlasTextureId,
    pub tile_id: TileId,
    pub padding: u32,
    pub bounds: Bounds<DevicePixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct AtlasTextureId {
    pub index: u32,
    pub kind: AtlasTextureKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub enum AtlasTextureKind {
    Monochrome = 0,
    Polychrome = 1,
    Subpixel = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct TileId(pub u32);

impl From<etagere::AllocId> for TileId {
    fn from(id: etagere::AllocId) -> Self {
        Self(id.serialize())
    }
}

impl From<TileId> for etagere::AllocId {
    fn from(id: TileId) -> Self {
        Self::deserialize(id.0)
    }
}

pub struct PlatformInputHandler {
    cx: AsyncWindowContext,
    handler: Box<dyn InputHandler>,
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
impl PlatformInputHandler {
    pub fn new(cx: AsyncWindowContext, handler: Box<dyn InputHandler>) -> Self {
        Self { cx, handler }
    }

    pub fn selected_text_range(&mut self, ignore_disabled_input: bool) -> Option<UTF16Selection> {
        self.cx
            .update(|window, cx| {
                self.handler
                    .selected_text_range(ignore_disabled_input, window, cx)
            })
            .ok()
            .flatten()
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn marked_text_range(&mut self) -> Option<Range<usize>> {
        self.cx
            .update(|window, cx| self.handler.marked_text_range(window, cx))
            .ok()
            .flatten()
    }

    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    pub fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
    ) -> Option<String> {
        self.cx
            .update(|window, cx| {
                self.handler
                    .text_for_range(range_utf16, adjusted, window, cx)
            })
            .ok()
            .flatten()
    }

    pub fn replace_text_in_range(&mut self, replacement_range: Option<Range<usize>>, text: &str) {
        self.cx
            .update(|window, cx| {
                self.handler
                    .replace_text_in_range(replacement_range, text, window, cx);
            })
            .ok();
    }

    pub fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
    ) {
        self.cx
            .update(|window, cx| {
                self.handler.replace_and_mark_text_in_range(
                    range_utf16,
                    new_text,
                    new_selected_range,
                    window,
                    cx,
                )
            })
            .ok();
    }

    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn unmark_text(&mut self) {
        self.cx
            .update(|window, cx| self.handler.unmark_text(window, cx))
            .ok();
    }

    pub fn paste(&mut self, item: ClipboardItem) {
        self.cx
            .update(|window, cx| self.handler.paste(item, window, cx))
            .ok();
    }

    pub fn bounds_for_range(&mut self, range_utf16: Range<usize>) -> Option<Bounds<Pixels>> {
        self.cx
            .update(|window, cx| {
                let bounds = self.handler.bounds_for_range(range_utf16, window, cx)?;
                Some(bounds.map(|length| length * window.zoom()))
            })
            .ok()
            .flatten()
    }

    #[allow(dead_code)]
    pub fn apple_press_and_hold_enabled(&mut self) -> bool {
        self.handler.apple_press_and_hold_enabled()
    }

    pub fn dispatch_input(&mut self, input: &str, window: &mut Window, cx: &mut App) {
        self.handler.replace_text_in_range(None, input, window, cx);
    }

    pub fn compute_ime_candidate_bounds(
        marked_range: Option<Range<usize>>,
        selection: &UTF16Selection,
        mut bounds_for_range: impl FnMut(Range<usize>) -> Option<Bounds<Pixels>>,
    ) -> Option<Bounds<Pixels>> {
        if let Some(marked_range) = marked_range {
            let mut line_start = marked_range.start;

            let caret = selection.range.end;
            if let Some(caret_bounds) = bounds_for_range(caret..caret) {
                for i in (marked_range.start..caret).rev() {
                    if let Some(b) = bounds_for_range(i..i) {
                        if (b.origin.y - caret_bounds.origin.y).abs() > px(0.1) {
                            line_start = i + 1;
                            break;
                        }
                    }
                }
            }
            bounds_for_range(line_start..line_start)
        } else {
            let offset = if selection.reversed {
                selection.range.start
            } else {
                selection.range.end
            };
            bounds_for_range(offset..offset)
        }
    }

    pub fn selected_bounds(&mut self, window: &mut Window, cx: &mut App) -> Option<Bounds<Pixels>> {
        let marked_range = self.handler.marked_text_range(window, cx);
        let selection = self.handler.selected_text_range(true, window, cx)?;
        let bounds = Self::compute_ime_candidate_bounds(marked_range, &selection, |range| {
            self.handler.bounds_for_range(range, window, cx)
        })?;
        Some(bounds.map(|length| length * window.zoom()))
    }

    pub fn ime_candidate_bounds(&mut self) -> Option<Bounds<Pixels>> {
        let marked_range = self.marked_text_range();
        let selection = self.selected_text_range(true)?;
        Self::compute_ime_candidate_bounds(marked_range, &selection, |range| {
            self.bounds_for_range(range)
        })
    }

    #[allow(unused)]
    pub fn character_index_for_point(&mut self, point: Point<Pixels>) -> Option<usize> {
        self.cx
            .update(|window, cx| {
                let point = point / window.zoom();
                self.handler.character_index_for_point(point, window, cx)
            })
            .ok()
            .flatten()
    }

    pub fn set_selected_text_range(&mut self, range_utf16: Range<usize>) {
        self.cx
            .update(|window, cx| {
                self.handler
                    .set_selected_text_range(range_utf16, window, cx)
            })
            .ok();
    }

    pub fn element_bounds(&mut self) -> Option<Bounds<Pixels>> {
        self.cx
            .update(|window, cx| self.handler.element_bounds(window, cx))
            .ok()
            .flatten()
    }

    pub fn text_length_utf16(&mut self) -> Option<usize> {
        self.cx
            .update(|window, cx| self.handler.text_length_utf16(window, cx))
            .ok()
            .flatten()
    }

    #[allow(dead_code)]
    pub fn accepts_text_input(&mut self, window: &mut Window, cx: &mut App) -> bool {
        self.handler.accepts_text_input(window, cx)
    }

    #[allow(dead_code)]
    pub fn query_accepts_text_input(&mut self) -> bool {
        self.cx
            .update(|window, cx| self.handler.accepts_text_input(window, cx))
            .unwrap_or(true)
    }

    pub fn query_prefers_ime_for_printable_keys(&mut self) -> bool {
        self.cx
            .update(|window, cx| {
                !window.has_pending_keystrokes()
                    && self.handler.prefers_ime_for_printable_keys(window, cx)
            })
            .unwrap_or(false)
    }

    pub fn text_input_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> TextInputConfiguration {
        self.handler.text_input_configuration(window, cx)
    }

    pub fn text_input_editable_range(&mut self) -> Option<Range<usize>> {
        self.cx
            .update(|window, cx| self.handler.text_input_editable_range(window, cx))
            .ok()
            .flatten()
    }
}

#[derive(Debug)]
pub struct UTF16Selection {
    pub range: Range<usize>,
    pub reversed: bool,
}

pub trait InputHandler: 'static {
    fn selected_text_range(
        &mut self,
        ignore_disabled_input: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection>;

    fn marked_text_range(&mut self, window: &mut Window, cx: &mut App) -> Option<Range<usize>>;

    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<String>;

    fn replace_text_in_range(
        &mut self,
        replacement_range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    );

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut App,
    );

    fn unmark_text(&mut self, window: &mut Window, cx: &mut App);

    fn paste(&mut self, item: ClipboardItem, window: &mut Window, cx: &mut App) {
        if let Some(text) = item.text() {
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>>;

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<usize>;

    fn set_selected_text_range(
        &mut self,
        _range_utf16: Range<usize>,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn element_bounds(&mut self, _window: &mut Window, _cx: &mut App) -> Option<Bounds<Pixels>> {
        None
    }

    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut App) -> Option<usize> {
        None
    }

    #[allow(dead_code)]
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        true
    }

    fn accepts_text_input(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        true
    }

    fn text_input_editable_range(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Option<Range<usize>> {
        None
    }

    fn prefers_ime_for_printable_keys(&mut self, _window: &mut Window, _cx: &mut App) -> bool {
        false
    }

    fn text_input_configuration(
        &mut self,
        _window: &mut Window,
        _cx: &mut App,
    ) -> TextInputConfiguration {
        TextInputConfiguration::default()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextInputConfiguration {
    pub autocorrect: bool,
    pub autocapitalize: Autocapitalize,
    pub suggestions: bool,
    pub input_action: TextInputAction,
    pub input_mode: TextInputMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextInputMode {
    #[default]
    Text,
    None,
    Decimal,
    Numeric,
    Tel,
    Search,
    Email,
    Url,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Autocapitalize {
    #[default]
    None,
    Words,
    Sentences,
    Characters,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextInputAction {
    #[default]
    Unspecified,
    Enter,
    Done,
    Go,
    Next,
    Previous,
    Search,
    Send,
}

#[derive(Debug)]
pub struct WindowOptions {
    pub window_bounds: Option<WindowBounds>,

    pub titlebar: Option<TitlebarOptions>,

    pub focus: bool,

    pub show: bool,

    pub kind: WindowKind,

    pub is_movable: bool,

    pub app_owns_titlebar_drag: bool,

    pub inactive_frame_interval: Option<Duration>,

    pub is_resizable: bool,

    pub is_minimizable: bool,

    pub display_id: Option<DisplayId>,

    pub window_background: WindowBackgroundAppearance,

    pub app_id: Option<String>,

    pub window_min_size: Option<Size<Pixels>>,

    pub window_decorations: Option<WindowDecorations>,

    pub icon: Option<Arc<image::RgbaImage>>,

    pub tabbing_identifier: Option<String>,

    pub mount: Option<SharedString>,
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub struct WindowParams {
    pub bounds: Bounds<Pixels>,

    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub titlebar: Option<TitlebarOptions>,

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub kind: WindowKind,

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_movable: bool,

    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    pub app_owns_titlebar_drag: bool,

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_resizable: bool,

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub is_minimizable: bool,

    #[cfg_attr(
        any(target_os = "linux", target_os = "freebsd", target_os = "windows"),
        allow(dead_code)
    )]
    pub focus: bool,

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub show: bool,

    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub icon: Option<Arc<image::RgbaImage>>,

    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub display_id: Option<DisplayId>,

    #[cfg_attr(feature = "wayland", allow(dead_code))]
    pub app_id: Option<String>,

    pub window_min_size: Option<Size<Pixels>>,

    #[cfg(target_os = "macos")]
    pub tabbing_identifier: Option<String>,

    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    pub mount: Option<SharedString>,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum WindowBounds {
    Windowed(Bounds<Pixels>),
    Maximized(Bounds<Pixels>),
    Fullscreen(Bounds<Pixels>),
}

impl Default for WindowBounds {
    fn default() -> Self {
        WindowBounds::Windowed(Bounds::default())
    }
}

impl WindowBounds {
    pub fn get_bounds(&self) -> Bounds<Pixels> {
        match self {
            WindowBounds::Windowed(bounds) => *bounds,
            WindowBounds::Maximized(bounds) => *bounds,
            WindowBounds::Fullscreen(bounds) => *bounds,
        }
    }

    pub fn centered(size: Size<Pixels>, cx: &App) -> Self {
        WindowBounds::Windowed(Bounds::centered(None, size, cx))
    }
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            window_bounds: None,
            titlebar: Some(TitlebarOptions {
                title: Default::default(),
                appears_transparent: Default::default(),
                traffic_light_position: Default::default(),
                traffic_light_scale: Default::default(),
            }),
            focus: true,
            show: true,
            kind: WindowKind::Normal,
            is_movable: true,
            app_owns_titlebar_drag: false,
            inactive_frame_interval: Some(Duration::from_micros(33_333)),
            is_resizable: true,
            is_minimizable: true,
            display_id: None,
            window_background: WindowBackgroundAppearance::default(),
            icon: None,
            app_id: None,
            window_min_size: None,
            window_decorations: None,
            tabbing_identifier: None,
            mount: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct TitlebarOptions {
    pub title: Option<SharedString>,

    pub appears_transparent: bool,

    pub traffic_light_position: Option<Point<Pixels>>,

    pub traffic_light_scale: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Normal,

    PopUp,

    AnchoredPopup(popup::PopupOptions),

    Floating,

    #[cfg(all(target_os = "linux", feature = "wayland"))]
    LayerShell(layer_shell::LayerShellOptions),

    Dialog,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum WindowAppearance {
    #[default]
    Light,

    VibrantLight,

    Dark,

    VibrantDark,
}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub enum WindowBackgroundAppearance {
    #[default]
    Opaque,
    Transparent,
    Blurred,
    MicaBackdrop,
    MicaAltBackdrop,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum TextRenderingMode {
    #[default]
    PlatformDefault,
    Subpixel,
    Grayscale,
}

#[derive(Clone, Debug)]
pub struct PathPromptOptions {
    pub files: bool,
    pub directories: bool,
    pub multiple: bool,
    pub prompt: Option<SharedString>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PromptLevel {
    Info,

    Warning,

    Critical,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PromptButton {
    Ok(SharedString),
    Cancel(SharedString),
    Other(SharedString),
}

impl PromptButton {
    pub fn new(label: impl Into<SharedString>) -> Self {
        PromptButton::Other(label.into())
    }

    pub fn ok(label: impl Into<SharedString>) -> Self {
        PromptButton::Ok(label.into())
    }

    pub fn cancel(label: impl Into<SharedString>) -> Self {
        PromptButton::Cancel(label.into())
    }

    #[allow(dead_code)]
    pub fn is_cancel(&self) -> bool {
        matches!(self, PromptButton::Cancel(_))
    }

    pub fn label(&self) -> &SharedString {
        match self {
            PromptButton::Ok(label) => label,
            PromptButton::Cancel(label) => label,
            PromptButton::Other(label) => label,
        }
    }
}

impl From<&str> for PromptButton {
    fn from(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "ok" => PromptButton::Ok("OK".into()),
            "cancel" => PromptButton::Cancel("Cancel".into()),
            _ => PromptButton::Other(SharedString::from(value.to_owned())),
        }
    }
}

#[derive(Copy, Clone, Default, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum CursorStyle {
    #[default]
    Arrow,

    IBeam,

    Crosshair,

    ClosedHand,

    OpenHand,

    PointingHand,

    ResizeLeft,

    ResizeRight,

    ResizeLeftRight,

    ResizeUp,

    ResizeDown,

    ResizeUpDown,

    ResizeUpLeftDownRight,

    ResizeUpRightDownLeft,

    ResizeColumn,

    ResizeRow,

    IBeamCursorForVerticalLayout,

    OperationNotAllowed,

    DragLink,

    DragCopy,

    ContextualMenu,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardItem {
    pub entries: Vec<ClipboardEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardReadError {
    Unavailable,
    Denied(String),
    UnsupportedContent,
}

impl std::fmt::Display for ClipboardReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("the clipboard is unavailable"),
            Self::Denied(message) => {
                write!(formatter, "clipboard access was denied: {message}")
            }
            Self::UnsupportedContent => {
                formatter.write_str("the clipboard contents are unsupported")
            }
        }
    }
}

impl std::error::Error for ClipboardReadError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClipboardEntry {
    String(ClipboardString),
    Image(Image),
    ExternalPaths(crate::ExternalPaths),
}

impl ClipboardItem {
    pub fn new_string(text: String) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(ClipboardString::new(text))],
        }
    }

    pub fn new_string_with_metadata(text: String, metadata: String) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(ClipboardString {
                text,
                metadata: Some(metadata),
            })],
        }
    }

    pub fn new_string_with_json_metadata<T: Serialize>(text: String, metadata: T) -> Self {
        Self {
            entries: vec![ClipboardEntry::String(
                ClipboardString::new(text).with_json_metadata(metadata),
            )],
        }
    }

    pub fn new_image(image: &Image) -> Self {
        Self {
            entries: vec![ClipboardEntry::Image(image.clone())],
        }
    }

    pub fn text(&self) -> Option<String> {
        let mut answer = String::new();

        for entry in self.entries.iter() {
            if let ClipboardEntry::String(ClipboardString { text, metadata: _ }) = entry {
                answer.push_str(text);
            }
        }

        if answer.is_empty() {
            for entry in self.entries.iter() {
                if let ClipboardEntry::ExternalPaths(paths) = entry {
                    for path in &paths.0 {
                        use std::fmt::Write as _;
                        _ = write!(answer, "{}", path.display());
                    }
                }
            }
        }

        if !answer.is_empty() {
            Some(answer)
        } else {
            None
        }
    }

    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub fn metadata(&self) -> Option<&String> {
        match self.entries().first() {
            Some(ClipboardEntry::String(clipboard_string)) if self.entries.len() == 1 => {
                clipboard_string.metadata.as_ref()
            }
            _ => None,
        }
    }

    pub fn entries(&self) -> &[ClipboardEntry] {
        &self.entries
    }

    pub fn into_entries(self) -> impl Iterator<Item = ClipboardEntry> {
        self.entries.into_iter()
    }
}

impl From<ClipboardString> for ClipboardEntry {
    fn from(value: ClipboardString) -> Self {
        Self::String(value)
    }
}

impl From<String> for ClipboardEntry {
    fn from(value: String) -> Self {
        Self::from(ClipboardString::from(value))
    }
}

impl From<Image> for ClipboardEntry {
    fn from(value: Image) -> Self {
        Self::Image(value)
    }
}

impl From<ClipboardEntry> for ClipboardItem {
    fn from(value: ClipboardEntry) -> Self {
        Self {
            entries: vec![value],
        }
    }
}

impl From<String> for ClipboardItem {
    fn from(value: String) -> Self {
        Self::from(ClipboardEntry::from(value))
    }
}

impl From<Image> for ClipboardItem {
    fn from(value: Image) -> Self {
        Self::from(ClipboardEntry::from(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, EnumIter, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Webp,
    Gif,
    Svg,
    Bmp,
    Tiff,
    Ico,
    Pnm,
}

impl ImageFormat {
    pub const fn mime_type(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Svg => "image/svg+xml",
            ImageFormat::Bmp => "image/bmp",
            ImageFormat::Tiff => "image/tiff",
            ImageFormat::Ico => "image/ico",
            ImageFormat::Pnm => "image/x-portable-anymap",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Ico => "ico",
            ImageFormat::Pnm => "pnm",
        }
    }

    pub fn from_mime_type(mime_type: &str) -> Option<Self> {
        use strum::IntoEnumIterator;
        Self::iter()
            .find(|format| format.mime_type() == mime_type)
            .or_else(|| Self::from_mime_type_alias(mime_type))
    }

    fn from_mime_type_alias(mime_type: &str) -> Option<Self> {
        match mime_type {
            "image/jpg" => Some(Self::Jpeg),
            "image/tif" => Some(Self::Tiff),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub format: ImageFormat,
    pub bytes: Vec<u8>,
    pub id: u64,
}

pub(crate) fn decode_static_image(
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<SmallVec<[Frame; 1]>> {
    let decoder = image::ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .context("creating image decoder")?;
    decode_static_image_from_decoder(decoder)
}

pub(crate) fn decode_static_image_from_decoder(
    mut decoder: impl image::ImageDecoder,
) -> Result<SmallVec<[Frame; 1]>> {
    let orientation = decoder
        .orientation()
        .context("reading decoder's orientation")?;
    let mut image = DynamicImage::from_decoder(decoder).context("decoding image")?;
    image.apply_orientation(orientation);

    let mut data = image.into_rgba8();
    for pixel in data.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    Ok(SmallVec::from_elem(Frame::new(data), 1))
}

impl Hash for Image {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.id);
    }
}

impl Image {
    pub fn empty() -> Self {
        Self::from_bytes(ImageFormat::Png, Vec::new())
    }

    pub fn from_bytes(format: ImageFormat, bytes: Vec<u8>) -> Self {
        Self {
            id: hash(&bytes),
            format,
            bytes,
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn use_render_image(
        self: Arc<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        ImageSource::Image(self)
            .use_data(None, window, cx)
            .and_then(|result| result.ok())
    }

    pub fn get_render_image(
        self: Arc<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        ImageSource::Image(self)
            .get_data(None, window, cx)
            .and_then(|result| result.ok())
    }

    pub fn remove_asset(self: Arc<Self>, cx: &mut App) {
        ImageSource::Image(self).remove_asset(cx);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn is_asset_cached(self: &Arc<Self>, cx: &App) -> bool {
        ImageSource::Image(self.clone()).is_asset_cached(cx)
    }

    pub fn to_image_data(&self, svg_renderer: SvgRenderer) -> Result<Arc<RenderImage>> {
        let frames = match self.format {
            ImageFormat::Gif => {
                let decoder = GifDecoder::new(Cursor::new(&self.bytes))?;
                let mut frames = SmallVec::new();

                for frame in decoder.into_frames() {
                    match frame {
                        Ok(mut frame) => {
                            for pixel in frame.buffer_mut().chunks_exact_mut(4) {
                                pixel.swap(0, 2);
                            }
                            frames.push(frame);
                        }
                        Err(err) => {
                            log::debug!("Skipping GIF frame due to decode error: {err}");
                        }
                    }
                }

                if frames.is_empty() {
                    anyhow::bail!("GIF could not be decoded: all frames failed");
                }

                frames
            }
            ImageFormat::Png => decode_static_image(&self.bytes, image::ImageFormat::Png)?,
            ImageFormat::Jpeg => decode_static_image(&self.bytes, image::ImageFormat::Jpeg)?,
            ImageFormat::Webp => decode_static_image(&self.bytes, image::ImageFormat::WebP)?,
            ImageFormat::Bmp => decode_static_image(&self.bytes, image::ImageFormat::Bmp)?,
            ImageFormat::Tiff => decode_static_image(&self.bytes, image::ImageFormat::Tiff)?,
            ImageFormat::Ico => decode_static_image(&self.bytes, image::ImageFormat::Ico)?,
            ImageFormat::Svg => {
                return svg_renderer
                    .render_single_frame(&self.bytes, 1.0)
                    .map_err(Into::into);
            }
            ImageFormat::Pnm => decode_static_image(&self.bytes, image::ImageFormat::Pnm)?,
        };

        Ok(Arc::new(RenderImage::new(frames)))
    }

    pub fn format(&self) -> ImageFormat {
        self.format
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardString {
    pub text: String,
    pub metadata: Option<String>,
}

impl ClipboardString {
    pub fn new(text: String) -> Self {
        Self {
            text,
            metadata: None,
        }
    }

    pub fn with_json_metadata<T: Serialize>(mut self, metadata: T) -> Self {
        self.metadata = Some(serde_json::to_string(&metadata).unwrap());
        self
    }

    pub fn text(&self) -> &String {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn metadata_json<T>(&self) -> Option<T>
    where
        T: for<'a> Deserialize<'a>,
    {
        self.metadata
            .as_ref()
            .and_then(|m| serde_json::from_str(m).ok())
    }

    #[cfg_attr(any(target_os = "linux", target_os = "freebsd"), allow(dead_code))]
    pub fn text_hash(text: &str) -> u64 {
        let mut hasher = SeaHasher::new();
        text.hash(&mut hasher);
        hasher.finish()
    }
}

impl From<String> for ClipboardString {
    fn from(value: String) -> Self {
        Self {
            text: value,
            metadata: None,
        }
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_image_to_image_data_applies_exif_orientation() {
        let image = Image::from_bytes(
            ImageFormat::Jpeg,
            include_bytes!("../examples/image/exif-orientation-rotate-180.jpg").to_vec(),
        );

        let render_image = image.to_image_data(SvgRenderer::new(Arc::new(()))).unwrap();

        assert_eq!(render_image.size(0), size(16.into(), 32.into()));

        let bytes = render_image.as_bytes(0).unwrap();
        assert_eq!(&bytes[..4], &[255, 255, 255, 255]);
        assert_eq!(&bytes[(16 * 32 - 1) * 4..], &[0, 0, 0, 255]);
    }

    #[test]
    fn test_svg_image_to_image_data_converts_to_bgra() {
        let image = Image::from_bytes(
            ImageFormat::Svg,
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1">
<rect width="1" height="1" fill="#38BDF8"/>
</svg>"##
                .to_vec(),
        );

        let render_image = image.to_image_data(SvgRenderer::new(Arc::new(()))).unwrap();
        let bytes = render_image.as_bytes(0).unwrap();

        for pixel in bytes.chunks_exact(4) {
            assert_eq!(pixel, &[0xF8, 0xBD, 0x38, 0xFF]);
        }
    }
}

#[cfg(test)]
mod atlas_tests {
    use super::*;

    const TILE_SIZE: Size<DevicePixels> = Size {
        width: DevicePixels(1),
        height: DevicePixels(1),
    };

    #[derive(Default)]
    struct RecordingAtlasBackend {
        insert_calls: u32,
        fail_next_insert: bool,
        removed_tiles: Vec<AtlasTile>,
    }

    impl AtlasBackend for RecordingAtlasBackend {
        fn insert(
            &mut self,
            kind: AtlasTextureKind,
            size: Size<DevicePixels>,
            _bytes: &[u8],
        ) -> Result<AtlasTile> {
            self.insert_calls += 1;
            if std::mem::take(&mut self.fail_next_insert) {
                anyhow::bail!("backend failed");
            }
            Ok(AtlasTile {
                texture_id: AtlasTextureId { index: 0, kind },
                tile_id: TileId(self.insert_calls),
                padding: 0,
                bounds: Bounds {
                    origin: Point::default(),
                    size,
                },
            })
        }

        fn remove(&mut self, tile: AtlasTile) {
            self.removed_tiles.push(tile);
        }
    }

    fn image_key(image_id: usize) -> AtlasKey {
        AtlasKey::Image(RenderImageParams {
            image_id: crate::ImageId(image_id),
            frame_index: 0,
        })
    }

    fn build_tile() -> Result<Option<(Size<DevicePixels>, Cow<'static, [u8]>)>> {
        Ok(Some((TILE_SIZE, Cow::Borrowed(&[0, 0, 0, 255]))))
    }

    #[test]
    fn only_successful_inserts_are_cached() -> Result<()> {
        let mut state = AtlasState::new(RecordingAtlasBackend::default());
        let key = image_key(1);

        assert_eq!(
            state.get_or_insert_with(key.clone(), &mut || Ok(None))?,
            None
        );
        state
            .get_or_insert_with(key.clone(), &mut || anyhow::bail!("builder failed"))
            .expect_err("builder error should propagate");
        assert!(!state.contains(&key));
        assert_eq!(state.backend.insert_calls, 0);

        state.backend.fail_next_insert = true;
        state
            .get_or_insert_with(key.clone(), &mut build_tile)
            .expect_err("backend error should propagate");
        assert!(!state.contains(&key));
        assert_eq!(state.backend.insert_calls, 1);

        let tile = state
            .get_or_insert_with(key.clone(), &mut build_tile)?
            .context("builder should produce a tile")?;
        assert_eq!(tile.texture_id.kind, key.texture_kind());
        assert_eq!(
            state.get_or_insert_with(key.clone(), &mut || {
                anyhow::bail!("cache hit must not call the builder")
            })?,
            Some(tile)
        );
        assert!(state.contains(&key));
        assert_eq!(state.backend.insert_calls, 2);
        Ok(())
    }

    #[test]
    fn remove_and_clear_invalidate_keys() -> Result<()> {
        let mut state = AtlasState::new(RecordingAtlasBackend::default());
        let key = image_key(1);
        let other_key = image_key(2);
        let tile = state
            .get_or_insert_with(key.clone(), &mut build_tile)?
            .context("builder should produce a tile")?;
        state
            .get_or_insert_with(other_key.clone(), &mut build_tile)?
            .context("builder should produce another tile")?;

        state.remove(&key);
        state.remove(&key);
        assert!(!state.contains(&key));
        assert!(state.contains(&other_key));
        assert_eq!(state.backend.removed_tiles, vec![tile]);

        let mut reset_calls = 0;
        state.clear(|_| reset_calls += 1);
        assert_eq!(reset_calls, 1);
        assert!(!state.contains(&other_key));
        assert_eq!(state.backend.removed_tiles, vec![tile]);
        Ok(())
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_window_button_layout_parse_standard() {
        let layout = WindowButtonLayout::parse("close,minimize:maximize").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_right_only() {
        let layout = WindowButtonLayout::parse("minimize,maximize,close").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close)
            ]
        );
    }

    #[test]
    fn test_window_button_layout_parse_left_only() {
        let layout = WindowButtonLayout::parse("close,minimize,maximize:").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize)
            ]
        );
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_with_whitespace() {
        let layout = WindowButtonLayout::parse(" close , minimize : maximize ").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_empty() {
        let layout = WindowButtonLayout::parse("").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_intentionally_empty() {
        let layout = WindowButtonLayout::parse(":").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [None, None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_invalid_buttons() {
        let layout = WindowButtonLayout::parse("close,invalid,minimize:maximize,foo").unwrap();
        assert_eq!(
            layout.left,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_deduplicates_same_side_buttons() {
        let layout = WindowButtonLayout::parse("close,close,minimize").unwrap();
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Close),
                Some(WindowButton::Minimize),
                None
            ]
        );
        assert_eq!(layout.format(), ":close,minimize");
    }

    #[test]
    fn test_window_button_layout_parse_deduplicates_buttons_across_sides() {
        let layout = WindowButtonLayout::parse("close:maximize,close,minimize").unwrap();
        assert_eq!(layout.left, [Some(WindowButton::Close), None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Maximize),
                Some(WindowButton::Minimize),
                None
            ]
        );

        let button_ids: Vec<_> = layout
            .left
            .iter()
            .chain(layout.right.iter())
            .flatten()
            .map(WindowButton::id)
            .collect();
        let unique_button_ids = button_ids.iter().copied().collect::<HashSet<_>>();
        assert_eq!(unique_button_ids.len(), button_ids.len());
        assert_eq!(layout.format(), "close:maximize,minimize");
    }

    #[test]
    fn test_window_button_layout_parse_gnome_style() {
        let layout = WindowButtonLayout::parse("close").unwrap();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(layout.right, [Some(WindowButton::Close), None, None]);
    }

    #[test]
    fn test_window_button_layout_parse_elementary_style() {
        let layout = WindowButtonLayout::parse("close:maximize").unwrap();
        assert_eq!(layout.left, [Some(WindowButton::Close), None, None]);
        assert_eq!(layout.right, [Some(WindowButton::Maximize), None, None]);
    }

    #[test]
    fn test_window_button_layout_round_trip() {
        let cases = [
            "close:minimize,maximize",
            "minimize,maximize,close:",
            ":close",
            "close:",
            "close:maximize",
            ":",
        ];

        for case in cases {
            let layout = WindowButtonLayout::parse(case).unwrap();
            assert_eq!(layout.format(), case, "Round-trip failed for: {}", case);
        }
    }

    #[test]
    fn test_window_button_layout_linux_default() {
        let layout = WindowButtonLayout::linux_default();
        assert_eq!(layout.left, [None, None, None]);
        assert_eq!(
            layout.right,
            [
                Some(WindowButton::Minimize),
                Some(WindowButton::Maximize),
                Some(WindowButton::Close)
            ]
        );

        let round_tripped = WindowButtonLayout::parse(&layout.format()).unwrap();
        assert_eq!(round_tripped, layout);
    }

    #[test]
    fn test_window_button_layout_parse_all_invalid() {
        assert!(WindowButtonLayout::parse("asdfghjkl").is_err());
    }
}
