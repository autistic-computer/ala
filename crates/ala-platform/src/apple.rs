#![allow(deprecated)]

#[cfg(target_os = "macos")]
mod metal_atlas;
#[cfg(target_os = "macos")]
mod metal_glass;
#[cfg(target_os = "macos")]
pub mod metal_renderer;
pub(crate) mod system_notifications;
