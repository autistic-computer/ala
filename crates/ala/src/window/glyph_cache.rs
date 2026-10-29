use crate::{AtlasTile, Bounds, DevicePixels, RenderGlyphParams};

const GLYPH_SLOT_BITS: u32 = 12;

pub(super) struct GlyphLookupCache {
    slots: Box<[Option<GlyphSlot>]>,
    atlas_generation: u64,
    font_generation: Option<usize>,
    #[cfg(test)]
    pub(super) lookups: std::cell::Cell<usize>,
}

#[derive(Clone)]
struct GlyphSlot {
    params: RenderGlyphParams,
    bounds: Bounds<DevicePixels>,
    tile: Option<AtlasTile>,
}

impl Default for GlyphLookupCache {
    fn default() -> Self {
        Self {
            slots: vec![None; 1 << GLYPH_SLOT_BITS].into_boxed_slice(),
            atlas_generation: 0,
            font_generation: None,
            #[cfg(test)]
            lookups: std::cell::Cell::new(0),
        }
    }
}

impl GlyphLookupCache {
    fn slot(params: &RenderGlyphParams) -> usize {
        const K: u64 = 0x9e37_79b9_7f4a_7c15;
        let words = [
            params.glyph_id.0 as u64 | (params.font_id.0 as u64) << 32,
            params.font_size.0.to_bits() as u64
                | (params.subpixel_variant.x as u64) << 32
                | (params.subpixel_variant.y as u64) << 40
                | (params.font_smoothing_strength as u64) << 48
                | (params.subpixel_rendering as u64) << 56
                | (params.is_emoji as u64) << 57
                | (params.font_smoothing as u64) << 58
                | (params.synthetic_bold as u64) << 59
                | (params.synthetic_italic as u64) << 60,
            params.scale_factor.to_bits() as u64,
        ];
        let hash = words.iter().fold(0u64, |hash, word| {
            (hash.rotate_left(26) ^ word).wrapping_mul(K)
        });
        (hash >> (64 - GLYPH_SLOT_BITS)) as usize
    }

    pub(super) fn sync_font_generation(&mut self, generation: usize) {
        if self.font_generation != Some(generation) {
            if self.font_generation.is_some() {
                self.slots.fill(None);
            }
            self.font_generation = Some(generation);
        }
    }

    pub(super) fn sync_atlas_generation(&mut self, generation: u64) {
        if self.atlas_generation != generation {
            for slot in self.slots.iter_mut().flatten() {
                slot.tile = None;
            }
            self.atlas_generation = generation;
        }
    }

    pub(super) fn atlas_generation(&self) -> u64 {
        self.atlas_generation
    }

    pub(super) fn lookup(
        &self,
        params: &RenderGlyphParams,
    ) -> Option<(Bounds<DevicePixels>, Option<AtlasTile>)> {
        #[cfg(test)]
        self.lookups.set(self.lookups.get() + 1);
        match &self.slots[Self::slot(params)] {
            Some(slot) if slot.params == *params => Some((slot.bounds, slot.tile)),
            _ => None,
        }
    }

    pub(super) fn insert_bounds(
        &mut self,
        params: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) {
        self.slots[Self::slot(params)] = Some(GlyphSlot {
            params: params.clone(),
            bounds,
            tile: None,
        });
    }

    pub(super) fn insert_tile(&mut self, params: &RenderGlyphParams, tile: AtlasTile) {
        if let Some(slot) = &mut self.slots[Self::slot(params)]
            && slot.params == *params
        {
            slot.tile = Some(tile);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GlyphLookupCache;
    use crate::{
        AtlasKey, AtlasState, AtlasTextureKind, Bounds, DevicePixels, FontId, GlyphId,
        HeadlessAtlasBackend, NoopTextSystem, RenderGlyphParams, TextSystem, point, px, size,
    };
    use std::{borrow::Cow, sync::Arc};

    fn glyph(glyph_id: u32) -> RenderGlyphParams {
        RenderGlyphParams {
            font_id: FontId(1),
            glyph_id: GlyphId(glyph_id),
            font_size: px(14.),
            subpixel_variant: point(0, 0),
            scale_factor: 2.,
            is_emoji: false,
            subpixel_rendering: false,
            font_smoothing: false,
            font_smoothing_strength: 0,
            synthetic_bold: false,
            synthetic_italic: false,
        }
    }

    fn bounds(value: i32) -> Bounds<DevicePixels> {
        Bounds::new(
            point(DevicePixels(value), DevicePixels(-1)),
            size(DevicePixels(10), DevicePixels(14)),
        )
    }

    #[test]
    fn glyph_lookup_requires_the_complete_carried_render_key() {
        let mut cache = GlyphLookupCache::default();
        let params = glyph(7);
        assert_eq!(cache.lookup(&params), None);
        cache.insert_bounds(&params, bounds(5));
        assert_eq!(cache.lookup(&params), Some((bounds(5), None)));
        for changed in [
            RenderGlyphParams {
                font_id: FontId(2),
                ..params.clone()
            },
            RenderGlyphParams {
                glyph_id: GlyphId(8),
                ..params.clone()
            },
            RenderGlyphParams {
                font_size: px(15.),
                ..params.clone()
            },
            RenderGlyphParams {
                subpixel_variant: point(1, 0),
                ..params.clone()
            },
            RenderGlyphParams {
                subpixel_variant: point(0, 1),
                ..params.clone()
            },
            RenderGlyphParams {
                scale_factor: 1.,
                ..params.clone()
            },
            RenderGlyphParams {
                is_emoji: true,
                ..params.clone()
            },
            RenderGlyphParams {
                subpixel_rendering: true,
                ..params.clone()
            },
            RenderGlyphParams {
                font_smoothing: true,
                ..params.clone()
            },
            RenderGlyphParams {
                font_smoothing_strength: 64,
                ..params.clone()
            },
            RenderGlyphParams {
                synthetic_bold: true,
                ..params.clone()
            },
            RenderGlyphParams {
                synthetic_italic: true,
                ..params.clone()
            },
        ] {
            assert_eq!(cache.lookup(&changed), None);
        }
        for glyph_id in 0..10_000 {
            cache.insert_bounds(&glyph(glyph_id), bounds(glyph_id as i32));
        }
        for glyph_id in 0..10_000 {
            let found = cache.lookup(&glyph(glyph_id));
            assert!(found.is_none() || found == Some((bounds(glyph_id as i32), None)));
        }
        assert_eq!(cache.lookup(&glyph(9_999)), Some((bounds(9_999), None)));
    }

    #[test]
    fn atlas_generation_change_requires_a_fresh_tile() {
        let params = glyph(7);
        let mut atlas = AtlasState::new(HeadlessAtlasBackend::default());
        let mut build = || Ok(Some((bounds(5).size, Cow::Owned(vec![0; 140]))));
        let tile = atlas
            .get_or_insert_with(AtlasKey::Glyph(params.clone()), &mut build)
            .unwrap()
            .unwrap();
        assert_eq!(tile.texture_id.kind, AtlasTextureKind::Monochrome);
        let mut cache = GlyphLookupCache::default();
        cache.insert_tile(&params, tile);
        assert_eq!(cache.lookup(&params), None);
        cache.insert_bounds(&params, bounds(5));
        cache.insert_tile(&params, tile);
        assert_eq!(cache.lookup(&params), Some((bounds(5), Some(tile))));
        cache.insert_tile(&glyph(8), tile);
        assert_eq!(cache.lookup(&glyph(8)), None);
        cache.sync_atlas_generation(atlas.generation());
        assert_eq!(cache.lookup(&params), Some((bounds(5), Some(tile))));
        atlas.clear(|_| {});
        cache.sync_atlas_generation(atlas.generation());
        assert_eq!(cache.lookup(&params), Some((bounds(5), None)));
        let new_tile = atlas
            .get_or_insert_with(AtlasKey::Glyph(params.clone()), &mut build)
            .unwrap()
            .unwrap();
        assert_ne!(new_tile, tile);
        cache.insert_tile(&params, new_tile);
        assert_eq!(cache.lookup(&params), Some((bounds(5), Some(new_tile))));
    }

    #[test]
    fn font_installation_expires_local_bounds_and_tiles() {
        let text_system = TextSystem::new(Arc::new(NoopTextSystem));
        let generation = text_system.font_cache_generation();
        let mut cache = GlyphLookupCache::default();
        cache.sync_font_generation(generation);
        cache.insert_bounds(&glyph(7), bounds(5));
        cache.sync_font_generation(generation);
        assert_eq!(cache.lookup(&glyph(7)), Some((bounds(5), None)));
        text_system.add_fonts(Vec::new()).unwrap();
        let next_generation = text_system.font_cache_generation();
        assert_ne!(generation, next_generation);
        cache.sync_font_generation(next_generation);
        assert_eq!(cache.lookup(&glyph(7)), None);
    }

    #[test]
    fn zero_raster_bounds_stay_cached_without_an_atlas_tile() {
        let mut cache = GlyphLookupCache::default();
        cache.sync_font_generation(0);
        cache.insert_bounds(&glyph(7), Bounds::default());
        cache.sync_atlas_generation(1);
        assert_eq!(cache.lookup(&glyph(7)), Some((Bounds::default(), None)));
        cache.sync_font_generation(1);
        assert_eq!(cache.lookup(&glyph(7)), None);
    }
}
