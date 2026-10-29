//! A benchmark-only decorator: real ALA text/layout code, with optional MHR-style shaping.
//! Cache hits compare complete keys, including text, size, runs and native font generation.

#[path = "segments.rs"]
mod segments;

use ala::*;
#[cfg(target_os = "linux")]
use ala_platform::wgpu::CosmicTextSystem;
use std::{
    borrow::Cow,
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Cache {
    generation: u64,
    entries: HashMap<u64, Vec<Entry>>,
    glyphs: usize,
    clock: u64,
}

struct Entry {
    text: String,
    size: Pixels,
    runs: Vec<FontRun>,
    layout: Arc<LineLayout>,
    glyphs: usize,
    used: u64,
}

pub struct ExperimentalTextSystem {
    inner: Arc<dyn PlatformTextSystem>,
    segmented: bool,
    cache: Mutex<Cache>,
    pub calls: AtomicUsize,
    pub bytes_shaped: AtomicUsize,
}

impl ExperimentalTextSystem {
    pub fn new(segmented: bool) -> Arc<Self> {
        #[cfg(target_os = "linux")]
        let inner: Arc<dyn PlatformTextSystem> =
            Arc::new(CosmicTextSystem::new_without_system_fonts("IBM Plex Sans"));
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        let inner = ala_platform::current_platform(true).text_system();
        inner
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!(
                    "../../../ala/assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf"
                )),
                Cow::Borrowed(include_bytes!(
                    "../../../ala/assets/fonts/lilex/Lilex-Regular.ttf"
                )),
            ])
            .unwrap();
        Arc::new(Self {
            inner,
            segmented,
            cache: Mutex::default(),
            calls: AtomicUsize::new(0),
            bytes_shaped: AtomicUsize::new(0),
        })
    }

    fn raw(&self, text: &str, size: Pixels, runs: &[FontRun]) -> LineLayout {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.bytes_shaped.fetch_add(text.len(), Ordering::Relaxed);
        self.inner.layout_line(text, size, runs)
    }

    pub fn reset_counts(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.bytes_shaped.store(0, Ordering::Relaxed);
    }

    pub fn retained_glyphs(&self) -> usize {
        self.cache.lock().unwrap().glyphs
    }
}

impl PlatformTextSystem for ExperimentalTextSystem {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> anyhow::Result<()> {
        self.inner.add_fonts(fonts)?;
        *self.cache.lock().unwrap() = Cache::default();
        Ok(())
    }
    fn set_missing_glyph_sink(&self, sink: Option<Arc<dyn MissingGlyphSink>>) {
        self.inner.set_missing_glyph_sink(sink);
    }
    fn all_font_names(&self) -> Vec<String> {
        self.inner.all_font_names()
    }
    fn font_id(&self, font: &Font) -> anyhow::Result<FontId> {
        self.inner.font_id(font)
    }
    fn prewarm_fonts(&self, fonts: &[FontId]) {
        self.inner.prewarm_fonts(fonts);
    }
    fn font_generation(&self) -> u64 {
        self.inner.font_generation()
    }
    fn font_metrics(&self, font: FontId) -> FontMetrics {
        self.inner.font_metrics(font)
    }
    fn typographic_bounds(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Bounds<f32>> {
        self.inner.typographic_bounds(font, glyph)
    }
    fn advance(&self, font: FontId, glyph: GlyphId) -> anyhow::Result<Size<f32>> {
        self.inner.advance(font, glyph)
    }
    fn glyph_for_char(&self, font: FontId, ch: char) -> Option<GlyphId> {
        self.inner.glyph_for_char(font, ch)
    }
    fn glyph_raster_bounds(
        &self,
        params: &RenderGlyphParams,
    ) -> anyhow::Result<Bounds<DevicePixels>> {
        self.inner.glyph_raster_bounds(params)
    }
    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) -> anyhow::Result<(Size<DevicePixels>, Vec<u8>)> {
        self.inner.rasterize_glyph(params, bounds)
    }
    fn recommended_rendering_mode(&self, font: FontId, size: Pixels) -> TextRenderingMode {
        self.inner.recommended_rendering_mode(font, size)
    }
    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        self.inner.glyph_dilation_for_color(color)
    }

    fn layout_line(&self, text: &str, size: Pixels, runs: &[FontRun]) -> LineLayout {
        let cuts = self.segmented.then(|| segments::cut_points(text)).flatten();
        let Some(cuts) = cuts else {
            return self.raw(text, size, runs);
        };
        let mut cache = self.cache.lock().unwrap();
        if cache.generation != self.inner.font_generation() {
            *cache = Cache {
                generation: self.inner.font_generation(),
                ..Cache::default()
            };
        }
        let mut clipper = segments::RunClipper::new(runs);
        let mut start = 0;
        let mut parts = Vec::with_capacity(cuts.len() + 1);
        for end in cuts.into_iter().chain([text.len()]) {
            let part = &text[start..end];
            let fonts = clipper.clip(start..end);
            let mut hash = ala::collections::FxHasher::default();
            part.hash(&mut hash);
            f32::from(size).to_bits().hash(&mut hash);
            fonts.hash(&mut hash);
            let hash = hash.finish();
            cache.clock += 1;
            let clock = cache.clock;
            let found = cache.entries.get_mut(&hash).and_then(|bucket| {
                bucket
                    .iter_mut()
                    .find(|entry| {
                        entry.text == part
                            && entry.size == size
                            && entry.runs.as_slice() == fonts.as_slice()
                    })
                    .map(|entry| {
                        entry.used = clock;
                        entry.layout.clone()
                    })
            });
            let layout = found.unwrap_or_else(|| {
                let layout = Arc::new(self.raw(part, size, &fonts));
                let glyphs = layout
                    .runs
                    .iter()
                    .map(|run| run.glyphs.len())
                    .sum::<usize>();
                cache.glyphs += glyphs;
                cache.entries.entry(hash).or_default().push(Entry {
                    text: part.to_owned(),
                    size,
                    runs: fonts.to_vec(),
                    layout: layout.clone(),
                    glyphs,
                    used: clock,
                });
                layout
            });
            parts.push((start, layout));
            start = end;
        }
        // Match MHR's bounded glyph cache, including a steady-state eviction workload.
        const LIMIT: usize = 1 << 18;
        if cache.glyphs > LIMIT {
            let mut ages: Vec<_> = cache
                .entries
                .iter()
                .flat_map(|(&hash, entries)| {
                    entries
                        .iter()
                        .map(move |entry| (entry.used, hash, entry.glyphs))
                })
                .collect();
            ages.sort_unstable();
            for (used, hash, glyphs) in ages {
                if cache.glyphs <= LIMIT * 3 / 4 {
                    break;
                }
                cache
                    .entries
                    .get_mut(&hash)
                    .unwrap()
                    .retain(|entry| entry.used != used);
                cache.glyphs -= glyphs;
            }
            cache.entries.retain(|_, entries| !entries.is_empty());
        }
        segments::join(&parts, text.len(), size)
    }
}

pub fn paragraph(bytes: usize, seed: usize) -> String {
    const WORDS: &[&str] = &[
        "the",
        "quick",
        "brown",
        "fox",
        "jumps",
        "over",
        "a",
        "lazy",
        "dog",
        "while",
        "streaming",
        "tokens",
        "arrive",
        "one",
        "after",
        "another",
        "and",
        "the",
        "renderer",
        "keeps",
        "up",
        "with",
        "layout,",
        "which",
        "is",
        "why",
        "caching",
        "matters.",
        "Hello,",
        "world!",
        "It's",
        "fine",
        "(really)",
        "—",
        "naïve",
        "café",
        "résumé",
        "123,456",
        "x=1;",
        "https://example.com/a/b",
    ];
    let mut text = String::with_capacity(bytes + 32);
    let mut state = seed as u64;
    while text.len() < bytes {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        text.push_str(WORDS[(state >> 33) as usize % WORDS.len()]);
        text.push(' ');
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
}
