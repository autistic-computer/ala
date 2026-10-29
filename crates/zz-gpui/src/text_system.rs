mod font_fallbacks;
mod font_features;
mod line;
mod line_layout;
mod line_wrapper;
mod recent_shapes;

pub use font_fallbacks::*;
pub use font_features::*;
pub use line::*;
pub use line_layout::*;
pub use line_wrapper::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::collections::{FxHashMap, FxHashSet};
use crate::{
    Bounds, DevicePixels, Hsla, Pixels, PlatformTextSystem, Point, Result, SharedString, Size,
    StrikethroughStyle, TextRenderingMode, UnderlineStyle, px,
};
use anyhow::{Context as _, anyhow};
use core::fmt;
use derive_more::{Add, Deref, FromStr, Sub};
use itertools::Itertools;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};
use smallvec::{SmallVec, smallvec};
use std::{
    borrow::Cow,
    cmp,
    collections::VecDeque,
    fmt::{Debug, Display, Formatter},
    hash::{Hash, Hasher},
    ops::{Deref, DerefMut, Range},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(C)]
pub struct FontId(pub usize);

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub struct FontFamilyId(pub usize);

pub const SUBPIXEL_VARIANTS_X: u8 = 4;

pub const SUBPIXEL_VARIANTS_Y: u8 = 1;

const UNDERLINE_DESCENT_OFFSET_FACTOR: f32 = 0.618;

pub fn underline_y_offset(line_height: Pixels, ascent: Pixels, descent: Pixels) -> Pixels {
    let padding_top = (line_height - ascent - descent) / 2.;
    padding_top + ascent + descent * UNDERLINE_DESCENT_OFFSET_FACTOR
}

const MAX_REPORTED_MISSING_GLYPHS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FallbackFontClass {
    Proportional,
    Monospace,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MissingGlyph {
    grapheme: SharedString,
    font_class: FallbackFontClass,
}

impl MissingGlyph {
    pub fn new(grapheme: SharedString, font_class: FallbackFontClass) -> Self {
        Self {
            grapheme,
            font_class,
        }
    }

    pub fn grapheme(&self) -> &str {
        &self.grapheme
    }

    pub fn font_class(&self) -> FallbackFontClass {
        self.font_class
    }
}

pub trait MissingGlyphSink: Send + Sync {
    fn report(&self, missing_glyphs: Vec<MissingGlyph>);
}

#[derive(Default)]
struct MissingGlyphState {
    reported: FxHashSet<MissingGlyph>,
    reported_order: VecDeque<MissingGlyph>,
    generation: usize,
}

impl MissingGlyphState {
    fn reset(&mut self, generation: usize) {
        self.reported.clear();
        self.reported_order.clear();
        self.generation = generation;
    }
}

struct QueuedMissingGlyph {
    generation: usize,
    missing_glyph: MissingGlyph,
}

struct MissingGlyphReporter {
    generation: Arc<AtomicUsize>,
    sender: async_channel::Sender<QueuedMissingGlyph>,
}

impl MissingGlyphSink for MissingGlyphReporter {
    fn report(&self, missing_glyphs: Vec<MissingGlyph>) {
        if self.sender.is_closed() {
            return;
        }

        let generation = self.generation.load(Ordering::Acquire);
        for missing_glyph in missing_glyphs.into_iter().unique() {
            let queued = QueuedMissingGlyph {
                generation,
                missing_glyph,
            };
            if self.sender.try_send(queued).is_err() {
                break;
            }
        }
    }
}

impl MissingGlyphReporter {
    fn reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

pub(crate) struct MissingGlyphReceiver {
    state: MissingGlyphState,
    generation: Arc<AtomicUsize>,
    receiver: async_channel::Receiver<QueuedMissingGlyph>,
}

impl MissingGlyphReceiver {
    pub(crate) async fn recv(
        &mut self,
    ) -> std::result::Result<Vec<MissingGlyph>, async_channel::RecvError> {
        loop {
            let queued = self.receiver.recv().await?;
            let mut missing_glyphs = Vec::new();
            for queued in std::iter::once(queued)
                .chain(std::iter::from_fn(|| self.receiver.try_recv().ok()))
                .take(MAX_REPORTED_MISSING_GLYPHS)
            {
                let generation = self.generation.load(Ordering::Acquire);
                if self.state.generation != generation {
                    self.state.reset(generation);
                    missing_glyphs.clear();
                }
                if queued.generation != generation
                    || !self.state.reported.insert(queued.missing_glyph.clone())
                {
                    continue;
                }
                self.state
                    .reported_order
                    .push_back(queued.missing_glyph.clone());
                missing_glyphs.push(queued.missing_glyph);
                if self.state.reported.len() > MAX_REPORTED_MISSING_GLYPHS
                    && let Some(expired) = self.state.reported_order.pop_front()
                {
                    self.state.reported.remove(&expired);
                }
            }
            if !missing_glyphs.is_empty() {
                return Ok(missing_glyphs);
            }
            let mut yielded = false;
            std::future::poll_fn(|cx| {
                if std::mem::replace(&mut yielded, true) {
                    std::task::Poll::Ready(())
                } else {
                    cx.waker().wake_by_ref();
                    std::task::Poll::Pending
                }
            })
            .await;
        }
    }
}

impl Drop for MissingGlyphReceiver {
    fn drop(&mut self) {
        self.receiver.close();
        while self.receiver.try_recv().is_ok() {}
    }
}

pub struct TextSystem {
    platform_text_system: Arc<dyn PlatformTextSystem>,
    font_ids_by_font: RwLock<FxHashMap<Font, Result<FontId>>>,
    resolved_fonts: RwLock<FxHashMap<Font, FontId>>,
    platform_font_generation: AtomicU64,
    font_metrics: RwLock<FxHashMap<FontId, FontMetrics>>,
    raster_bounds: RwLock<FxHashMap<RenderGlyphParams, Bounds<DevicePixels>>>,
    wrapper_pool: Mutex<FxHashMap<FontIdWithSize, Vec<LineWrapper>>>,
    font_runs_pool: Mutex<Vec<Vec<FontRun>>>,
    fallback_font_stack: SmallVec<[Font; 2]>,
    font_generation: Arc<AtomicUsize>,
    missing_glyph_reporter: Arc<MissingGlyphReporter>,
    missing_glyph_receiver: Mutex<Option<MissingGlyphReceiver>>,
}

impl TextSystem {
    pub fn new(platform_text_system: Arc<dyn PlatformTextSystem>) -> Self {
        let platform_text_system = crate::frame_stats::text_system(platform_text_system);
        let (sender, receiver) = async_channel::bounded(MAX_REPORTED_MISSING_GLYPHS);
        let missing_glyph_generation = Arc::<AtomicUsize>::default();
        TextSystem {
            platform_text_system,
            font_metrics: RwLock::default(),
            raster_bounds: RwLock::default(),
            font_ids_by_font: RwLock::default(),
            resolved_fonts: RwLock::default(),
            platform_font_generation: AtomicU64::new(0),
            wrapper_pool: Mutex::default(),
            font_runs_pool: Mutex::default(),
            fallback_font_stack: smallvec![
                font(".ZedMono"),
                font(".ZedSans"),
                font("Helvetica"),
                font("Segoe UI"),
                font("Ubuntu"),
                font("Adwaita Sans"),
                font("Cantarell"),
                font("Noto Sans"),
                font("DejaVu Sans"),
                font("Arial"),
            ],
            font_generation: Arc::default(),
            missing_glyph_reporter: Arc::new(MissingGlyphReporter {
                generation: missing_glyph_generation.clone(),
                sender,
            }),
            missing_glyph_receiver: Mutex::new(Some(MissingGlyphReceiver {
                state: MissingGlyphState::default(),
                generation: missing_glyph_generation,
                receiver,
            })),
        }
    }

    pub fn all_font_names(&self) -> Vec<String> {
        let mut names = self.platform_text_system.all_font_names();
        names.sort_unstable();
        names.dedup();
        names
    }

    pub fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.platform_text_system.add_fonts(fonts)?;
        self.font_ids_by_font.write().clear();
        self.resolved_fonts.write().clear();
        self.missing_glyph_reporter.reset();
        self.font_generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    pub(crate) fn take_missing_glyph_receiver(&self) -> Option<MissingGlyphReceiver> {
        self.missing_glyph_receiver
            .try_lock()
            .and_then(|mut receiver| receiver.take())
    }

    pub(crate) fn enable_missing_glyph_reporting(&self) {
        self.platform_text_system
            .set_missing_glyph_sink(Some(self.missing_glyph_reporter.clone()));
    }

    pub(crate) fn disable_missing_glyph_reporting(&self) {
        self.platform_text_system.set_missing_glyph_sink(None);
        self.missing_glyph_reporter.reset();
    }

    #[cfg(test)]
    pub(crate) fn report_missing_glyphs_in_test(&self, missing_glyphs: Vec<MissingGlyph>) {
        self.missing_glyph_reporter.report(missing_glyphs);
    }

    fn font_id(&self, font: &Font) -> Result<FontId> {
        self.sync_font_generation();
        fn clone_font_id_result(font_id: &Result<FontId>) -> Result<FontId> {
            match font_id {
                Ok(font_id) => Ok(*font_id),
                Err(err) => Err(anyhow!("{err}")),
            }
        }

        let font_id = self
            .font_ids_by_font
            .read()
            .get(font)
            .map(clone_font_id_result);
        if let Some(font_id) = font_id {
            font_id
        } else {
            let font_id = self.platform_text_system.font_id(font);
            self.font_ids_by_font
                .write()
                .insert(font.clone(), clone_font_id_result(&font_id));
            font_id
        }
    }

    pub fn has_font(&self, family: &str) -> bool {
        self.font_id(&font(family.to_owned())).is_ok()
    }

    pub fn get_font_for_id(&self, id: FontId) -> Option<Font> {
        let lock = self.font_ids_by_font.read();
        lock.iter()
            .filter_map(|(font, result)| match result {
                Ok(font_id) if *font_id == id => Some(font.clone()),
                _ => None,
            })
            .next()
    }

    pub fn resolve_font(&self, font: &Font) -> FontId {
        self.sync_font_generation();
        if let Some(font_id) = self.resolved_fonts.read().get(font) {
            return *font_id;
        }
        let font_id = self.resolve_font_uncached(font);
        self.resolved_fonts.write().insert(font.clone(), font_id);
        font_id
    }

    fn sync_font_generation(&self) {
        let generation = self.platform_text_system.font_generation();
        if self.platform_font_generation.load(Ordering::Relaxed) != generation
            && self
                .platform_font_generation
                .swap(generation, Ordering::Relaxed)
                != generation
        {
            self.font_ids_by_font.write().clear();
            self.resolved_fonts.write().clear();
            self.font_generation.fetch_add(1, Ordering::Release);
        }
    }

    pub(crate) fn font_cache_generation(&self) -> usize {
        self.sync_font_generation();
        self.font_generation.load(Ordering::Acquire)
    }

    fn resolve_font_uncached(&self, font: &Font) -> FontId {
        if let Ok(font_id) = self.font_id(font) {
            return font_id;
        }
        for fallback in &self.fallback_font_stack {
            let as_asked = Font {
                family: fallback.family.clone(),
                features: font.features.clone(),
                fallbacks: font.fallbacks.clone(),
                weight: font.weight,
                style: font.style,
            };
            if let Ok(font_id) = self.font_id(&as_asked) {
                return font_id;
            }
            if let Ok(font_id) = self.font_id(fallback) {
                return font_id;
            }
        }

        panic!(
            "failed to resolve font '{}' or any of the fallbacks: {}",
            font.family,
            self.fallback_font_stack
                .iter()
                .map(|fallback| &fallback.family)
                .join(", ")
        );
    }

    pub fn prewarm_fonts(&self, fonts: &[Font]) {
        let mut font_ids = SmallVec::<[FontId; 8]>::new();
        for font in fonts {
            let font_id = self.resolve_font(font);
            if !font_ids.contains(&font_id) {
                font_ids.push(font_id);
            }
        }
        self.platform_text_system.prewarm_fonts(&font_ids);
    }

    pub fn bounding_box(&self, font_id: FontId, font_size: Pixels) -> Bounds<Pixels> {
        self.read_metrics(font_id, |metrics| metrics.bounding_box(font_size))
    }

    pub fn typographic_bounds(
        &self,
        font_id: FontId,
        font_size: Pixels,
        character: char,
    ) -> Result<Bounds<Pixels>> {
        let glyph_id = self
            .platform_text_system
            .glyph_for_char(font_id, character)
            .with_context(|| format!("glyph not found for character '{character}'"))?;
        let bounds = self
            .platform_text_system
            .typographic_bounds(font_id, glyph_id)?;
        Ok(self.read_metrics(font_id, |metrics| {
            (bounds / metrics.units_per_em as f32 * font_size.0).map(px)
        }))
    }

    pub fn advance(&self, font_id: FontId, font_size: Pixels, ch: char) -> Result<Size<Pixels>> {
        let glyph_id = self
            .platform_text_system
            .glyph_for_char(font_id, ch)
            .with_context(|| format!("glyph not found for character '{ch}'"))?;
        let result = self.platform_text_system.advance(font_id, glyph_id)?
            / self.units_per_em(font_id) as f32;

        Ok(result * font_size)
    }

    pub fn layout_width(&self, font_id: FontId, font_size: Pixels, ch: char) -> Pixels {
        let mut buffer = [0; 4];
        let buffer = ch.encode_utf8(&mut buffer);
        self.platform_text_system
            .layout_line(
                buffer,
                font_size,
                &[FontRun {
                    len: buffer.len(),
                    font_id,
                }],
            )
            .width
    }

    pub fn em_width(&self, font_id: FontId, font_size: Pixels) -> Result<Pixels> {
        Ok(self.typographic_bounds(font_id, font_size, 'm')?.size.width)
    }

    pub fn em_advance(&self, font_id: FontId, font_size: Pixels) -> Result<Pixels> {
        Ok(self.advance(font_id, font_size, 'm')?.width)
    }

    pub fn ch_width(&self, font_id: FontId, font_size: Pixels) -> Result<Pixels> {
        Ok(self.typographic_bounds(font_id, font_size, '0')?.size.width)
    }

    pub fn ch_advance(&self, font_id: FontId, font_size: Pixels) -> Result<Pixels> {
        Ok(self.advance(font_id, font_size, '0')?.width)
    }

    pub fn units_per_em(&self, font_id: FontId) -> u32 {
        self.read_metrics(font_id, |metrics| metrics.units_per_em)
    }

    pub fn cap_height(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.cap_height(font_size))
    }

    pub fn x_height(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.x_height(font_size))
    }

    pub fn ascent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.ascent(font_size))
    }

    pub fn descent(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.descent(font_size))
    }

    pub fn line_gap(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.line_gap(font_size))
    }

    pub fn underline_thickness(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.read_metrics(font_id, |metrics| metrics.underline_thickness(font_size))
    }

    pub fn baseline_offset(
        &self,
        font_id: FontId,
        font_size: Pixels,
        line_height: Pixels,
    ) -> Pixels {
        let ascent = self.ascent(font_id, font_size);
        let descent = self.descent(font_id, font_size);
        let padding_top = (line_height - ascent - descent) / 2.;
        padding_top + ascent
    }

    fn read_metrics<T>(&self, font_id: FontId, read: impl FnOnce(&FontMetrics) -> T) -> T {
        let lock = self.font_metrics.upgradable_read();

        if let Some(metrics) = lock.get(&font_id) {
            read(metrics)
        } else {
            let mut lock = RwLockUpgradableReadGuard::upgrade(lock);
            let metrics = lock
                .entry(font_id)
                .or_insert_with(|| self.platform_text_system.font_metrics(font_id));
            read(metrics)
        }
    }

    pub fn line_wrapper(self: &Arc<Self>, font: Font, font_size: Pixels) -> LineWrapperHandle {
        let lock = &mut self.wrapper_pool.lock();
        let font_id = self.resolve_font(&font);
        let wrappers = lock
            .entry(FontIdWithSize { font_id, font_size })
            .or_default();
        let wrapper = wrappers
            .pop()
            .unwrap_or_else(|| LineWrapper::new(font_id, font_size, self.clone()));

        LineWrapperHandle {
            wrapper: Some(wrapper),
            text_system: self.clone(),
        }
    }

    pub(crate) fn raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        let raster_bounds = self.raster_bounds.upgradable_read();
        if let Some(bounds) = raster_bounds.get(params) {
            Ok(*bounds)
        } else {
            let mut raster_bounds = RwLockUpgradableReadGuard::upgrade(raster_bounds);
            let bounds = self.platform_text_system.glyph_raster_bounds(params)?;
            raster_bounds.insert(params.clone(), bounds);
            Ok(bounds)
        }
    }

    pub(crate) fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        let raster_bounds = self.raster_bounds(params)?;
        self.platform_text_system
            .rasterize_glyph(params, raster_bounds)
    }

    pub(crate) fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        self.platform_text_system.glyph_dilation_for_color(color)
    }

    pub(crate) fn recommended_rendering_mode(
        &self,
        font_id: FontId,
        font_size: Pixels,
    ) -> TextRenderingMode {
        self.platform_text_system
            .recommended_rendering_mode(font_id, font_size)
    }
}

#[derive(Deref)]
pub struct WindowTextSystem {
    line_layout_cache: LineLayoutCache,
    #[deref]
    text_system: Arc<TextSystem>,
}

impl WindowTextSystem {
    pub fn new(text_system: Arc<TextSystem>) -> Self {
        Self {
            line_layout_cache: LineLayoutCache::new(
                text_system.platform_text_system.clone(),
                text_system.font_generation.clone(),
            ),
            text_system,
        }
    }

    pub(crate) fn layout_index(&self) -> LineLayoutIndex {
        self.line_layout_cache.layout_index()
    }

    pub(crate) fn reuse_layouts(&self, index: Range<LineLayoutIndex>) {
        self.line_layout_cache.reuse_layouts(index)
    }

    pub(crate) fn truncate_layouts(&self, index: LineLayoutIndex) {
        self.line_layout_cache.truncate_layouts(index)
    }

    pub fn shape_line(
        &self,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
        force_width: Option<Pixels>,
    ) -> ShapedLine {
        debug_assert!(
            text.find('\n').is_none(),
            "text argument should not contain newlines"
        );

        let mut decoration_runs = SmallVec::<[DecorationRun; 32]>::new();
        for run in runs {
            if let Some(last_run) = decoration_runs.last_mut()
                && last_run.color == run.color
                && last_run.underline == run.underline
                && last_run.strikethrough == run.strikethrough
                && last_run.background_color == run.background_color
            {
                last_run.len += run.len as u32;
                continue;
            }
            decoration_runs.push(DecorationRun {
                len: run.len as u32,
                color: run.color,
                background_color: run.background_color,
                underline: run.underline,
                strikethrough: run.strikethrough,
            });
        }

        let layout = self.layout_line(&text, font_size, runs, force_width);

        ShapedLine {
            layout,
            text,
            decoration_runs,
        }
    }

    pub fn shape_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[TextRun],
        force_width: Option<Pixels>,
        materialize_text: impl FnOnce() -> SharedString,
    ) -> ShapedLine {
        let mut decoration_runs = SmallVec::<[DecorationRun; 32]>::new();
        for run in runs {
            if let Some(last_run) = decoration_runs.last_mut()
                && last_run.color == run.color
                && last_run.underline == run.underline
                && last_run.strikethrough == run.strikethrough
                && last_run.background_color == run.background_color
            {
                last_run.len += run.len as u32;
                continue;
            }
            decoration_runs.push(DecorationRun {
                len: run.len as u32,
                color: run.color,
                background_color: run.background_color,
                underline: run.underline,
                strikethrough: run.strikethrough,
            });
        }

        let mut used_force_width = force_width;
        let layout = self.layout_line_by_hash(
            text_hash,
            text_len,
            font_size,
            runs,
            used_force_width,
            || {
                let text = materialize_text();
                debug_assert!(
                    text.find('\n').is_none(),
                    "text argument should not contain newlines"
                );
                text
            },
        );

        let text: SharedString = SharedString::new_static("");

        ShapedLine {
            layout,
            text,
            decoration_runs,
        }
    }

    pub fn shape_text(
        &self,
        text: SharedString,
        font_size: Pixels,
        runs: &[TextRun],
        wrap_width: Option<Pixels>,
        line_clamp: Option<usize>,
    ) -> Result<SmallVec<[WrappedLine; 1]>> {
        let decoration_run_capacity = runs.len().min(32);
        let mut runs = runs.iter().filter(|run| run.len > 0).cloned().peekable();
        let mut font_runs = self.font_runs_pool.lock().pop().unwrap_or_default();

        let mut lines = SmallVec::new();
        let mut max_wrap_lines = line_clamp;
        let mut wrapped_lines = 0;

        let mut process_line = |line_text: SharedString, line_start, line_end| {
            font_runs.clear();

            let mut decoration_runs = <Vec<DecorationRun>>::with_capacity(decoration_run_capacity);
            let mut run_start = line_start;
            while run_start < line_end {
                let Some(run) = runs.peek_mut() else {
                    log::warn!("`TextRun`s do not cover the entire to be shaped text");
                    break;
                };

                let run_len_within_line = cmp::min(line_end - run_start, run.len);

                let decoration_changed = if let Some(last_run) = decoration_runs.last_mut()
                    && last_run.color == run.color
                    && last_run.underline == run.underline
                    && last_run.strikethrough == run.strikethrough
                    && last_run.background_color == run.background_color
                {
                    last_run.len += run_len_within_line as u32;
                    false
                } else {
                    decoration_runs.push(DecorationRun {
                        len: run_len_within_line as u32,
                        color: run.color,
                        background_color: run.background_color,
                        underline: run.underline,
                        strikethrough: run.strikethrough,
                    });
                    true
                };

                let font_id = self.resolve_font(&run.font);
                if let Some(font_run) = font_runs.last_mut()
                    && font_id == font_run.font_id
                    && !decoration_changed
                {
                    font_run.len += run_len_within_line;
                } else {
                    font_runs.push(FontRun {
                        len: run_len_within_line,
                        font_id,
                    });
                }

                run.len -= run_len_within_line;
                if run.len == 0 {
                    runs.next();
                }
                run_start += run_len_within_line;
            }

            let layout = self.line_layout_cache.layout_wrapped_line(
                &line_text,
                font_size,
                &font_runs,
                wrap_width,
                max_wrap_lines.map(|max| max.saturating_sub(wrapped_lines)),
            );
            wrapped_lines += layout.wrap_boundaries.len();

            lines.push(WrappedLine {
                layout,
                decoration_runs,
                text: line_text,
            });

            if let Some(run) = runs.peek_mut() {
                run.len -= 1;
                if run.len == 0 {
                    runs.next();
                }
            }
        };

        let mut split_lines = text.split('\n');

        if let Some(first_line) = split_lines.next()
            && let Some(second_line) = split_lines.next()
        {
            let mut line_start = 0;
            process_line(
                SharedString::new(first_line),
                line_start,
                line_start + first_line.len(),
            );
            line_start += first_line.len() + '\n'.len_utf8();
            process_line(
                SharedString::new(second_line),
                line_start,
                line_start + second_line.len(),
            );
            for line_text in split_lines {
                line_start += line_text.len() + '\n'.len_utf8();
                process_line(
                    SharedString::new(line_text),
                    line_start,
                    line_start + line_text.len(),
                );
            }
        } else {
            let end = text.len();
            process_line(text, 0, end);
        }

        self.font_runs_pool.lock().push(font_runs);

        Ok(lines)
    }

    pub(crate) fn finish_frame(&self) {
        self.line_layout_cache.finish_frame()
    }

    pub fn layout_line(
        &self,
        text: &str,
        font_size: Pixels,
        runs: &[TextRun],
        force_width: Option<Pixels>,
    ) -> Arc<LineLayout> {
        let mut last_run = None::<&TextRun>;
        let mut font_runs = self.font_runs_pool.lock().pop().unwrap_or_default();
        font_runs.clear();

        for run in runs.iter() {
            let decoration_changed = if let Some(last_run) = last_run
                && last_run.color == run.color
                && last_run.underline == run.underline
                && last_run.strikethrough == run.strikethrough
            {
                false
            } else {
                last_run = Some(run);
                true
            };

            let font_id = self.resolve_font(&run.font);
            if let Some(font_run) = font_runs.last_mut()
                && font_id == font_run.font_id
                && !decoration_changed
            {
                font_run.len += run.len;
            } else {
                font_runs.push(FontRun {
                    len: run.len,
                    font_id,
                });
            }
        }

        let layout = self.line_layout_cache.layout_line(
            &SharedString::new(text),
            font_size,
            &font_runs,
            force_width,
        );

        self.font_runs_pool.lock().push(font_runs);

        layout
    }

    pub fn layout_width(&self, font_id: FontId, font_size: Pixels, ch: char) -> Pixels {
        let mut buffer = [0; 4];
        let buffer: &_ = ch.encode_utf8(&mut buffer);
        self.line_layout_cache
            .layout_line(
                buffer,
                font_size,
                &[FontRun {
                    len: buffer.len(),
                    font_id,
                }],
                None,
            )
            .width
    }

    pub fn em_layout_width(&self, font_id: FontId, font_size: Pixels) -> Pixels {
        self.layout_width(font_id, font_size, 'm')
    }

    pub fn try_layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[TextRun],
        force_width: Option<Pixels>,
    ) -> Option<Arc<LineLayout>> {
        let mut last_run = None::<&TextRun>;
        let mut font_runs = self.font_runs_pool.lock().pop().unwrap_or_default();
        font_runs.clear();

        for run in runs.iter() {
            let decoration_changed = if let Some(last_run) = last_run
                && last_run.color == run.color
                && last_run.underline == run.underline
                && last_run.strikethrough == run.strikethrough
            {
                false
            } else {
                last_run = Some(run);
                true
            };

            let font_id = self.resolve_font(&run.font);
            if let Some(font_run) = font_runs.last_mut()
                && font_id == font_run.font_id
                && !decoration_changed
            {
                font_run.len += run.len;
            } else {
                font_runs.push(FontRun {
                    len: run.len,
                    font_id,
                });
            }
        }

        let layout = self.line_layout_cache.try_layout_line_by_hash(
            text_hash,
            text_len,
            font_size,
            &font_runs,
            force_width,
        );

        self.font_runs_pool.lock().push(font_runs);

        layout
    }

    pub fn layout_line_by_hash(
        &self,
        text_hash: u64,
        text_len: usize,
        font_size: Pixels,
        runs: &[TextRun],
        force_width: Option<Pixels>,
        materialize_text: impl FnOnce() -> SharedString,
    ) -> Arc<LineLayout> {
        let mut last_run = None::<&TextRun>;
        let mut font_runs = self.font_runs_pool.lock().pop().unwrap_or_default();
        font_runs.clear();

        for run in runs.iter() {
            let decoration_changed = if let Some(last_run) = last_run
                && last_run.color == run.color
                && last_run.underline == run.underline
                && last_run.strikethrough == run.strikethrough
            {
                false
            } else {
                last_run = Some(run);
                true
            };

            let font_id = self.resolve_font(&run.font);
            if let Some(font_run) = font_runs.last_mut()
                && font_id == font_run.font_id
                && !decoration_changed
            {
                font_run.len += run.len;
            } else {
                font_runs.push(FontRun {
                    len: run.len,
                    font_id,
                });
            }
        }

        let layout = self.line_layout_cache.layout_line_by_hash(
            text_hash,
            text_len,
            font_size,
            &font_runs,
            force_width,
            materialize_text,
        );

        self.font_runs_pool.lock().push(font_runs);

        layout
    }
}

#[derive(Hash, Eq, PartialEq)]
struct FontIdWithSize {
    font_id: FontId,
    font_size: Pixels,
}

pub struct LineWrapperHandle {
    wrapper: Option<LineWrapper>,
    text_system: Arc<TextSystem>,
}

impl Drop for LineWrapperHandle {
    fn drop(&mut self) {
        let mut state = self.text_system.wrapper_pool.lock();
        let wrapper = self.wrapper.take().unwrap();
        state
            .get_mut(&FontIdWithSize {
                font_id: wrapper.font_id,
                font_size: wrapper.font_size,
            })
            .unwrap()
            .push(wrapper);
    }
}

impl Deref for LineWrapperHandle {
    type Target = LineWrapper;

    fn deref(&self) -> &Self::Target {
        self.wrapper.as_ref().unwrap()
    }
}

impl DerefMut for LineWrapperHandle {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.wrapper.as_mut().unwrap()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize, Add, Sub, FromStr)]
#[serde(transparent)]
pub struct FontWeight(pub f32);

impl Display for FontWeight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<f32> for FontWeight {
    fn from(weight: f32) -> Self {
        FontWeight(weight)
    }
}

impl Default for FontWeight {
    #[inline]
    fn default() -> FontWeight {
        FontWeight::NORMAL
    }
}

impl Hash for FontWeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(u32::from_be_bytes(self.0.to_be_bytes()));
    }
}

impl Eq for FontWeight {}

impl FontWeight {
    pub const THIN: FontWeight = FontWeight(100.0);
    pub const EXTRA_LIGHT: FontWeight = FontWeight(200.0);
    pub const LIGHT: FontWeight = FontWeight(300.0);
    pub const NORMAL: FontWeight = FontWeight(400.0);
    pub const MEDIUM: FontWeight = FontWeight(500.0);
    pub const SEMIBOLD: FontWeight = FontWeight(600.0);
    pub const BOLD: FontWeight = FontWeight(700.0);
    pub const EXTRA_BOLD: FontWeight = FontWeight(800.0);
    pub const BLACK: FontWeight = FontWeight(900.0);

    pub const ALL: [FontWeight; 9] = [
        Self::THIN,
        Self::EXTRA_LIGHT,
        Self::LIGHT,
        Self::NORMAL,
        Self::MEDIUM,
        Self::SEMIBOLD,
        Self::BOLD,
        Self::EXTRA_BOLD,
        Self::BLACK,
    ];
}

impl schemars::JsonSchema for FontWeight {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "FontWeight".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        use schemars::json_schema;
        json_schema!({
            "type": "number",
            "minimum": Self::THIN,
            "maximum": Self::BLACK,
            "default": Self::default(),
            "description": "Font weight value between 100 (thin) and 900 (black)"
        })
    }
}

#[derive(Clone, Copy, Eq, PartialEq, Debug, Hash, Default, Serialize, Deserialize, JsonSchema)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

impl Display for FontStyle {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Debug::fmt(self, f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TextRun {
    pub len: usize,
    pub font: Font,
    pub color: Hsla,
    pub background_color: Option<Hsla>,
    pub underline: Option<UnderlineStyle>,
    pub strikethrough: Option<StrikethroughStyle>,
}

#[cfg(all(target_os = "macos", test))]
impl TextRun {
    fn with_len(&self, len: usize) -> Self {
        let mut this = self.clone();
        this.len = len;
        this
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[repr(C)]
pub struct GlyphId(pub u32);

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct GlyphRenderOptions {
    pub font_smoothing: FontSmoothing,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum FontSmoothing {
    #[default]
    PlatformDefault,
    Disabled,
    Enabled(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderGlyphParams {
    pub font_id: FontId,
    pub glyph_id: GlyphId,
    pub font_size: Pixels,
    pub subpixel_variant: Point<u8>,
    pub scale_factor: f32,
    pub is_emoji: bool,
    pub subpixel_rendering: bool,
    pub font_smoothing: bool,
    pub font_smoothing_strength: u8,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
}

impl Eq for RenderGlyphParams {}

impl Hash for RenderGlyphParams {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.font_id.0.hash(state);
        self.glyph_id.0.hash(state);
        self.font_size.0.to_bits().hash(state);
        self.subpixel_variant.hash(state);
        self.scale_factor.to_bits().hash(state);
        self.is_emoji.hash(state);
        self.subpixel_rendering.hash(state);
        self.font_smoothing.hash(state);
        self.font_smoothing_strength.hash(state);
        self.synthetic_bold.hash(state);
        self.synthetic_italic.hash(state);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Font {
    pub family: SharedString,

    pub features: FontFeatures,

    pub fallbacks: Option<FontFallbacks>,

    pub weight: FontWeight,

    pub style: FontStyle,
}

impl Default for Font {
    fn default() -> Self {
        font(".SystemUIFont")
    }
}

pub fn font(family: impl Into<SharedString>) -> Font {
    Font {
        family: family.into(),
        features: FontFeatures::default(),
        weight: FontWeight::default(),
        style: FontStyle::default(),
        fallbacks: None,
    }
}

impl Font {
    pub fn bold(mut self) -> Self {
        self.weight = FontWeight::BOLD;
        self
    }

    pub fn italic(mut self) -> Self {
        self.style = FontStyle::Italic;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FontMetrics {
    pub units_per_em: u32,

    pub ascent: f32,

    pub descent: f32,

    pub line_gap: f32,

    pub underline_position: f32,

    pub underline_thickness: f32,

    pub cap_height: f32,

    pub x_height: f32,

    pub bounding_box: Bounds<f32>,
}

impl FontMetrics {
    pub fn ascent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.ascent / self.units_per_em as f32) * font_size.0)
    }

    pub fn descent(&self, font_size: Pixels) -> Pixels {
        Pixels((self.descent / self.units_per_em as f32) * font_size.0)
    }

    pub fn line_gap(&self, font_size: Pixels) -> Pixels {
        Pixels((self.line_gap / self.units_per_em as f32) * font_size.0)
    }

    pub fn underline_position(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_position / self.units_per_em as f32) * font_size.0)
    }

    pub fn underline_thickness(&self, font_size: Pixels) -> Pixels {
        Pixels((self.underline_thickness / self.units_per_em as f32) * font_size.0)
    }

    pub fn cap_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.cap_height / self.units_per_em as f32) * font_size.0)
    }

    pub fn x_height(&self, font_size: Pixels) -> Pixels {
        Pixels((self.x_height / self.units_per_em as f32) * font_size.0)
    }

    pub fn bounding_box(&self, font_size: Pixels) -> Bounds<Pixels> {
        (self.bounding_box / self.units_per_em as f32 * font_size.0).map(px)
    }
}

#[allow(unused)]
pub fn font_name_with_fallbacks<'a>(name: &'a str, system: &'a str) -> &'a str {
    match name {
        ".SystemUIFont" => system,
        ".ZedSans" | "Zed Plex Sans" => "IBM Plex Sans",
        ".ZedMono" | "Zed Plex Mono" => "Lilex",
        _ => name,
    }
}

#[allow(unused)]
pub fn font_name_with_fallbacks_shared<'a>(
    name: &'a SharedString,
    system: &'a SharedString,
) -> &'a SharedString {
    match name.as_str() {
        ".SystemUIFont" => system,
        ".ZedSans" | "Zed Plex Sans" => const { &SharedString::new_static("IBM Plex Sans") },
        ".ZedMono" | "Zed Plex Mono" => const { &SharedString::new_static("Lilex") },
        _ => name,
    }
}

#[cfg(test)]
mod font_resolution_tests {
    use super::*;
    use crate::NoopTextSystem;

    #[test]
    fn missing_primary_font_preserves_requested_fallback_attributes() {
        let featured = Font {
            features: FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
            fallbacks: Some(FontFallbacks::from_fonts(vec!["Emoji Test".into()])),
            ..font("Missing Primary").bold().italic()
        };
        for requested in [
            font("Missing Primary").bold(),
            font("Missing Primary").italic(),
            featured,
        ] {
            let text_system = TextSystem::new(Arc::new(NoopTextSystem));
            let fallback = text_system.fallback_font_stack[0].clone();
            let styled_fallback = Font {
                family: fallback.family.clone(),
                ..requested.clone()
            };
            {
                let mut ids = text_system.font_ids_by_font.write();
                ids.insert(requested.clone(), Err(anyhow!("missing primary font")));
                ids.insert(fallback, Ok(FontId(10)));
                ids.insert(styled_fallback, Ok(FontId(20)));
            }

            assert_eq!(text_system.resolve_font(&requested), FontId(20));
            assert_eq!(text_system.resolve_font(&requested), FontId(20));
        }
    }

    #[test]
    fn has_font_reports_whether_the_family_loads() {
        let text_system = TextSystem::new(Arc::new(NoopTextSystem));
        text_system
            .font_ids_by_font
            .write()
            .insert(font("Missing Sans"), Err(anyhow!("missing family")));

        assert!(!text_system.has_font("Missing Sans"));
        assert!(text_system.has_font("Installed Sans"));
    }

    #[test]
    fn missing_styled_fallback_still_resolves_the_plain_fallback() {
        let requested = font("Missing Primary").bold().italic();
        let text_system = TextSystem::new(Arc::new(NoopTextSystem));
        let fallback = text_system.fallback_font_stack[0].clone();
        let styled_fallback = Font {
            family: fallback.family.clone(),
            ..requested.clone()
        };
        {
            let mut ids = text_system.font_ids_by_font.write();
            ids.insert(requested.clone(), Err(anyhow!("missing primary font")));
            ids.insert(styled_fallback, Err(anyhow!("missing fallback style")));
            ids.insert(fallback, Ok(FontId(10)));
        }

        assert_eq!(text_system.resolve_font(&requested), FontId(10));
    }
}

#[cfg(test)]
mod missing_glyph_tests {
    use super::*;
    use futures::FutureExt as _;

    #[test]
    fn bounds_retained_missing_glyphs() {
        let (reporter, mut receiver) = missing_glyph_channel();
        reporter.report(
            (0..MAX_REPORTED_MISSING_GLYPHS)
                .map(|index| {
                    MissingGlyph::new(index.to_string().into(), FallbackFontClass::Proportional)
                })
                .collect(),
        );
        assert!(receiver.recv().now_or_never().unwrap().is_ok());

        let newest = MissingGlyph::new("newest".into(), FallbackFontClass::Monospace);
        reporter.report(vec![newest.clone()]);
        assert!(receiver.recv().now_or_never().unwrap().is_ok());

        let state = &receiver.state;
        assert_eq!(state.reported.len(), MAX_REPORTED_MISSING_GLYPHS);
        assert_eq!(state.reported_order.len(), MAX_REPORTED_MISSING_GLYPHS);
        assert!(state.reported.contains(&newest));
    }

    #[test]
    fn dropping_receiver_closes_and_clears_reports() {
        let (reporter, receiver) = missing_glyph_channel();
        reporter.report(vec![missing_glyph("missing")]);

        drop(receiver);

        assert!(reporter.sender.is_closed());
        assert!(reporter.sender.is_empty());
    }

    fn missing_glyph_channel() -> (MissingGlyphReporter, MissingGlyphReceiver) {
        let (sender, receiver) = async_channel::bounded(MAX_REPORTED_MISSING_GLYPHS);
        let generation = Arc::<AtomicUsize>::default();
        let reporter = MissingGlyphReporter {
            generation: generation.clone(),
            sender,
        };
        let receiver = MissingGlyphReceiver {
            state: MissingGlyphState::default(),
            generation,
            receiver,
        };
        (reporter, receiver)
    }

    fn missing_glyph(grapheme: &'static str) -> MissingGlyph {
        MissingGlyph::new(grapheme.into(), FallbackFontClass::Proportional)
    }
}
