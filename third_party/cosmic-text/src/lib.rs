#![allow(clippy::new_without_default)]
#![allow(clippy::arithmetic_side_effects)]
#![allow(clippy::indexing_slicing)]
#![deny(clippy::cast_ptr_alignment)]
#![deny(clippy::unwrap_used)]
#![deny(missing_debug_implementations)]
#![deny(unreachable_patterns)]
#![deny(unused_must_use)]
#![warn(clippy::doc_markdown)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::unreadable_literal)]
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;

#[cfg(not(any(feature = "std", feature = "no_std")))]
compile_error!("Either the `std` or `no_std` feature must be enabled");

pub use self::attrs::*;
mod attrs;

pub use self::bidi_para::*;
mod bidi_para;

pub use self::buffer::*;
mod buffer;

pub use self::buffer_line::*;
mod buffer_line;

pub use self::cached::*;
mod cached;

pub use self::glyph_cache::*;
mod glyph_cache;

pub use self::cursor::*;
mod cursor;

pub use self::edit::*;
mod edit;

pub use self::font::*;
mod font;

pub use self::layout::*;
mod layout;

pub use self::line_ending::*;
mod line_ending;

pub use self::render::*;
mod render;

pub use self::shape::*;
mod shape;

pub use self::shape_run_cache::*;
mod shape_run_cache;

#[cfg(feature = "swash")]
pub use self::swash::*;
#[cfg(feature = "swash")]
mod swash;

mod math;

type BuildHasher = core::hash::BuildHasherDefault<rustc_hash::FxHasher>;

#[cfg(feature = "std")]
type HashMap<K, V> = std::collections::HashMap<K, V, BuildHasher>;
#[cfg(not(feature = "std"))]
type HashMap<K, V> = hashbrown::HashMap<K, V, BuildHasher>;
