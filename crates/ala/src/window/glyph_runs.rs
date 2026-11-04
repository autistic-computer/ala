use crate::collections::{FxHashMap, FxHasher};
use crate::{
    AtlasTile, Bounds, DevicePixels, FontId, GlyphId, GlyphRenderOptions, Hsla, LineLayout, Pixels,
    Point, TextAlign, WrapBoundary,
};
use std::hash::{Hash, Hasher};
use std::mem;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GlyphRunKey {
    layout: usize,
    font_size: u32,
    line_height: u32,
    align: u8,
    align_width: Option<u32>,
    scale_factor: u32,
    options: GlyphRenderOptions,
    decorations: u64,
    wraps: u64,
}

impl GlyphRunKey {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        layout: &LineLayout,
        line_height: Pixels,
        align: TextAlign,
        align_width: Option<Pixels>,
        colors: impl IntoIterator<Item = (u32, Hsla)>,
        wraps: &[WrapBoundary],
        options: GlyphRenderOptions,
        scale_factor: f32,
    ) -> Self {
        let mut decorations = FxHasher::default();
        for (len, color) in colors {
            len.hash(&mut decorations);
            for channel in [color.h, color.s, color.l, color.a] {
                channel.to_bits().hash(&mut decorations);
            }
        }
        let mut wrap_hash = FxHasher::default();
        for wrap in wraps {
            wrap.run_ix.hash(&mut wrap_hash);
            wrap.glyph_ix.hash(&mut wrap_hash);
        }
        Self {
            layout: std::ptr::from_ref(layout) as usize,
            font_size: layout.font_size.0.to_bits(),
            line_height: line_height.0.to_bits(),
            align: align as u8,
            align_width: align_width.map(|width| width.0.to_bits()),
            scale_factor: scale_factor.to_bits(),
            options,
            decorations: decorations.finish(),
            wraps: wrap_hash.finish(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CachedTile {
    Unknown,
    Empty {
        variant: Point<u8>,
    },
    Monochrome {
        variant: Point<u8>,
        raster_origin: Point<DevicePixels>,
        tile: AtlasTile,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedGlyph {
    pub(crate) id: GlyphId,
    pub(crate) font_id: FontId,
    pub(crate) position: Point<u32>,
    pub(crate) offset: Point<Pixels>,
    pub(crate) cull: Bounds<Pixels>,
    pub(crate) color: Hsla,
    pub(crate) tile: CachedTile,
}

#[derive(Debug)]
pub(crate) struct GlyphRun {
    pub(crate) font_generation: usize,
    pub(crate) atlas_generation: u64,
    pub(crate) glyphs: Vec<CachedGlyph>,
}

impl GlyphRun {
    pub(crate) fn matches(&self, layout: &LineLayout, generations: (usize, u64)) -> bool {
        if (self.font_generation, self.atlas_generation) != generations {
            return false;
        }
        let shaped = layout
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.font_id, glyph)));
        let mut cached = self.glyphs.iter();
        for (font_id, glyph) in shaped {
            let Some(entry) = cached.next() else {
                return false;
            };
            if entry.id != glyph.id
                || entry.font_id != font_id
                || entry.position.x != glyph.position.x.0.to_bits()
                || entry.position.y != glyph.position.y.0.to_bits()
            {
                return false;
            }
        }
        cached.next().is_none()
    }
}

#[derive(Default)]
pub(crate) struct GlyphRunCache {
    current: FxHashMap<GlyphRunKey, GlyphRun>,
    previous: FxHashMap<GlyphRunKey, GlyphRun>,
}

impl GlyphRunCache {
    pub(crate) fn begin_frame(&mut self) {
        mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
    }

    pub(crate) fn take(&mut self, key: &GlyphRunKey) -> Option<GlyphRun> {
        self.current
            .remove(key)
            .or_else(|| self.previous.remove(key))
    }

    pub(crate) fn put(&mut self, key: GlyphRunKey, run: GlyphRun) {
        self.current.insert(key, run);
    }
}
