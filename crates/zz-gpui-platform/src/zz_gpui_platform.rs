#[cfg(target_os = "macos")]
pub mod apple;
pub mod ios;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(any(target_family = "wasm", test))]
pub mod web;
#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_family = "wasm",
    target_os = "ios"
))]
pub mod wgpu;
#[cfg(target_os = "windows")]
pub mod windows;

pub use zz_gpui::Platform;

use std::rc::Rc;

pub fn background_executor() -> zz_gpui::BackgroundExecutor {
    current_platform(true).background_executor()
}

pub fn application() -> zz_gpui::Application {
    #[cfg(target_family = "wasm")]
    {
        application_with_web_backend(crate::web::WebBackendPreference::Auto)
    }

    #[cfg(not(target_family = "wasm"))]
    zz_gpui::Application::with_platform(current_platform(false))
}

pub fn headless() -> zz_gpui::Application {
    zz_gpui::Application::with_platform(current_platform(true))
}

#[cfg(target_family = "wasm")]
pub use crate::web::WebBackendPreference;

#[cfg(target_family = "wasm")]
pub fn application_with_web_backend(
    backend_preference: WebBackendPreference,
) -> zz_gpui::Application {
    let platform = Rc::new(crate::web::WebPlatform::new_with_backend(
        true,
        backend_preference,
    ));
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    zz_gpui::Application::with_platform(platform).with_http_client(http_client)
}

#[cfg(target_family = "wasm")]
pub fn single_threaded_web() -> zz_gpui::Application {
    let platform = Rc::new(crate::web::WebPlatform::new(false));
    let http_client = std::sync::Arc::new(platform.fetch_http_client());
    zz_gpui::Application::with_platform(platform).with_http_client(http_client)
}

#[cfg(target_family = "wasm")]
pub fn web_init() {
    console_error_panic_hook::set_once();
    crate::web::init_logging();
}

pub fn current_platform(headless: bool) -> Rc<dyn Platform> {
    #[cfg(target_os = "macos")]
    {
        Rc::new(crate::macos::MacPlatform::new(headless))
    }

    #[cfg(target_os = "windows")]
    {
        Rc::new(
            crate::windows::WindowsPlatform::new(headless)
                .expect("failed to initialize Windows platform"),
        )
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        crate::linux::current_platform(headless)
    }

    #[cfg(target_family = "wasm")]
    {
        let _ = headless;
        Rc::new(crate::web::WebPlatform::new(true))
    }

    #[cfg(target_os = "ios")]
    {
        let _ = headless;
        Rc::new(crate::ios::IosPlatform::new())
    }
}

#[cfg(any(feature = "bench-support", feature = "test-support"))]
pub fn current_headless_renderer()
-> anyhow::Result<Option<Box<dyn zz_gpui::PlatformHeadlessRenderer>>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Some(Box::new(
            crate::macos::metal_renderer::MetalHeadlessRenderer::new(),
        )))
    }

    #[cfg(target_os = "linux")]
    {
        crate::wgpu::WgpuHeadlessRenderer::new()
            .map(|renderer| Some(Box::new(renderer) as Box<dyn zz_gpui::PlatformHeadlessRenderer>))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Ok(None)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Duration;
    use zz_gpui::{AppContext, Empty, VisualTestAppContext};

    #[test]
    #[ignore]
    fn test_foreground_tasks_run_with_run_until_parked() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        {
            let task_ran = task_ran.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        assert!(!*task_ran.borrow());

        cx.run_until_parked();

        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore]
    fn test_advance_clock_triggers_delayed_tasks() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        {
            let task_ran = task_ran.clone();
            let executor = cx.background_executor.clone();
            cx.update(|cx| {
                cx.spawn(async move |_| {
                    executor.timer(Duration::from_millis(500)).await;
                    *task_ran.borrow_mut() = true;
                })
                .detach();
            });
        }

        cx.run_until_parked();
        assert!(!*task_ran.borrow());

        cx.advance_clock(Duration::from_millis(600));

        assert!(*task_ran.borrow());
    }

    #[test]
    #[ignore]
    fn test_window_spawn_uses_test_dispatcher() {
        let mut cx = VisualTestAppContext::new(current_platform(false));

        let task_ran = Rc::new(RefCell::new(false));

        let window = cx
            .open_offscreen_window_default(|_, cx| cx.new(|_| Empty))
            .expect("Failed to open window");

        {
            let task_ran = task_ran.clone();
            cx.update_window(window.into(), |_, window, cx| {
                window
                    .spawn(cx, async move |_| {
                        *task_ran.borrow_mut() = true;
                    })
                    .detach();
            })
            .ok();
        }

        assert!(!*task_ran.borrow());

        cx.run_until_parked();

        assert!(*task_ran.borrow());
    }
}
