use bitflags::bitflags;
use thiserror::Error;

use crate::Pixels;

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Layer {
    Background,

    Bottom,

    Top,

    #[default]
    Overlay,
}

bitflags! {
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct Anchor: u32 {
        const TOP = 1;
        const BOTTOM = 2;
        const LEFT = 4;
        const RIGHT = 8;
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum KeyboardInteractivity {
    None,

    Exclusive,

    #[default]
    OnDemand,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerShellOptions {
    pub namespace: String,
    pub layer: Layer,
    pub anchor: Anchor,
    pub exclusive_zone: Option<Pixels>,
    pub exclusive_edge: Option<Anchor>,
    pub margin: Option<(Pixels, Pixels, Pixels, Pixels)>,
    pub keyboard_interactivity: KeyboardInteractivity,
}

#[derive(Debug, Error)]
#[error("Compositor doesn't support zwlr_layer_shell_v1")]
pub struct LayerShellNotSupportedError;
