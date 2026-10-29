use bitflags::bitflags;
use thiserror::Error;

use crate::{AnyWindowHandle, Bounds, Pixels, Point};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupOptions {
    pub parent: AnyWindowHandle,

    pub anchor_rect: Bounds<Pixels>,

    pub anchor: PopupAnchor,

    pub gravity: PopupGravity,

    pub constraint_adjustment: PopupConstraintAdjustment,

    pub offset: Point<Pixels>,

    pub grab: bool,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PopupAnchor {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum PopupGravity {
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    BottomLeft,
    TopRight,
    BottomRight,
}

bitflags! {
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct PopupConstraintAdjustment: u32 {
        const SLIDE_X = 1;
        const SLIDE_Y = 2;
        const FLIP_X = 4;
        const FLIP_Y = 8;
        const RESIZE_X = 16;
        const RESIZE_Y = 32;
    }
}

#[derive(Debug, Error)]
#[error("popups are not supported on this platform")]
pub struct PopupNotSupportedError;
