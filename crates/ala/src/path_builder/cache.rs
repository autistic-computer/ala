//! Opt-in tessellation retention, inspired by longbridge/gpui-fast's
//! Apache-2.0 `fast/path_cache.rs` at 07b42ebf1f43088b8b74a7d4fca6b95e44e04265.
//! Coordinates remain absolute: a hit returns the exact original mesh.

use super::*;
use crate::collections::FxHashMap;
use lyon::path::PathEvent;

/// Caller-owned cache for repeated paths at identical coordinates.
///
/// Use [`PathBuilder::build_with_cache`] only where shapes repeat. Unique or
/// moving paths pay key construction and insertion costs without saving work.
/// The payload budget counts keys, stored paths and vertex capacities; allocator
/// and hash-table overhead are additional. An oversized path is not retained.
/// On exhaustion the cache clears, bounding retained payload without a global cache.
pub struct PathCache {
    paths: FxHashMap<Box<[u32]>, Path<Pixels>>,
    budget: usize,
    retained: usize,
    hits: u64,
    misses: u64,
}

impl Default for PathCache {
    fn default() -> Self {
        Self::new(8 * 1024 * 1024)
    }
}

impl PathCache {
    pub fn new(payload_budget_bytes: usize) -> Self {
        Self {
            paths: FxHashMap::default(),
            budget: payload_budget_bytes,
            retained: 0,
            hits: 0,
            misses: 0,
        }
    }

    /// Release entries and the hash table. Hit/miss counters remain cumulative.
    pub fn clear(&mut self) {
        self.paths = FxHashMap::default();
        self.retained = 0;
    }

    pub fn retained_payload_bytes(&self) -> usize {
        self.retained
    }

    pub fn len(&self) -> usize {
        self.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn hits(&self) -> u64 {
        self.hits
    }

    pub fn misses(&self) -> u64 {
        self.misses
    }
}

impl PathBuilder {
    /// Tessellate with a caller-owned, bounded cache of exact path inputs.
    ///
    /// Keys include transformed coordinates, stroke/fill options and dashes.
    /// Full keys are compared after hashing. Returned paths own their vertices;
    /// changing paint, clipping or vertices cannot mutate a retained mesh.
    /// [`Self::build`] remains available for cold or continuously changing paths.
    pub fn build_with_cache(self, cache: &mut PathCache) -> Result<Path<Pixels>, Error> {
        let path = match self.transform {
            Some(transform) => self.raw.build().transformed(&transform),
            None => self.raw.build(),
        };
        let key = shape_key(&path, &self.style, self.dash_array.as_deref());
        if let Some(path) = cache.paths.get(key.as_slice()) {
            cache.hits += 1;
            return Ok(path.clone());
        }
        cache.misses += 1;
        let result = match self.style {
            PathStyle::Stroke(options) => Self::tessellate_stroke(self.dash_array, &path, &options),
            PathStyle::Fill(options) => Self::tessellate_fill(&path, &options),
        }?;
        let minimum_bytes = key.len() * std::mem::size_of::<u32>()
            + std::mem::size_of::<Path<Pixels>>()
            + result.vertices.len() * std::mem::size_of::<crate::PathVertex<Pixels>>();
        if minimum_bytes > cache.budget {
            return Ok(result);
        }
        // Charge the stored clone's actual allocation, rather than the source's
        // spare capacity. Boxed keys have no unaccounted spare capacity.
        let stored = result.clone();
        let key = key.into_boxed_slice();
        let bytes = std::mem::size_of_val(key.as_ref())
            + std::mem::size_of::<Path<Pixels>>()
            + stored.vertices.capacity() * std::mem::size_of::<crate::PathVertex<Pixels>>();
        if bytes <= cache.budget {
            if bytes > cache.budget - cache.retained {
                cache.clear();
            }
            cache.paths.insert(key, stored);
            cache.retained += bytes;
        }
        Ok(result)
    }
}

fn shape_key(path: &lyon::path::Path, style: &PathStyle, dashes: Option<&[Pixels]>) -> Vec<u32> {
    let mut key = Vec::with_capacity(64);
    let point = |key: &mut Vec<u32>, point: lyon::math::Point| {
        key.extend([point.x.to_bits(), point.y.to_bits()]);
    };
    for event in path.iter() {
        match event {
            PathEvent::Begin { at } => {
                key.push(0);
                point(&mut key, at);
            }
            PathEvent::Line { to, .. } => {
                key.push(1);
                point(&mut key, to);
            }
            PathEvent::Quadratic { ctrl, to, .. } => {
                key.push(2);
                point(&mut key, ctrl);
                point(&mut key, to);
            }
            PathEvent::Cubic {
                ctrl1, ctrl2, to, ..
            } => {
                key.push(3);
                point(&mut key, ctrl1);
                point(&mut key, ctrl2);
                point(&mut key, to);
            }
            PathEvent::End { close, .. } => key.push(4 + u32::from(close)),
        }
    }
    // Both Lyon option structs are non_exhaustive. Their debug representation
    // covers every current and future field, including the style discriminant.
    let options = match style {
        PathStyle::Stroke(options) => format!("stroke:{options:?}"),
        PathStyle::Fill(options) => format!("fill:{options:?}"),
    };
    key.push(u32::MAX);
    key.push(options.len() as u32);
    key.extend(options.bytes().map(u32::from));
    match dashes {
        Some(dashes) => {
            key.push(1);
            key.push(dashes.len() as u32);
            key.extend(dashes.iter().map(|dash| dash.0.to_bits()));
        }
        None => key.push(0),
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(seed: usize) -> PathBuilder {
        let mut builder = if seed.is_multiple_of(2) {
            PathBuilder::fill().with_style(PathStyle::Fill(
                FillOptions::default().with_tolerance(0.01 + (seed % 7) as f32 * 0.05),
            ))
        } else {
            PathBuilder::stroke(px(0.5 + (seed % 13) as f32 * 0.2))
        };
        if seed % 5 == 1 {
            builder = builder.dash_array(&[px(4.), px(2.), px(3.)]);
        }
        builder.move_to(point(px(-8.25), px(2.75)));
        builder.line_to(point(px(35.125), px(-3.25)));
        builder.curve_to(point(px(70.5), px(50.25)), point(px(100.5), px(10.1)));
        builder.cubic_bezier_to(
            point(px(10.25), px(80.5)),
            point(px(120.), px(90.)),
            point(px(-15.2), px(105.3)),
        );
        builder.close();
        builder.rotate(seed as f32 * 1.7);
        builder.scale(0.7 + (seed % 8) as f32 * 0.13);
        builder.translate(point(px(seed as f32 * 0.31), px(-12.7)));
        builder
    }

    #[test]
    fn exact_meshes_and_independent_return_values() {
        let mut cache = PathCache::default();
        for seed in 0..160 {
            let expected = format!("{:?}", shape(seed).build().unwrap());
            let mut first = shape(seed).build_with_cache(&mut cache).unwrap();
            assert_eq!(expected, format!("{first:?}"));
            first.vertices.clear();
            let again = shape(seed).build_with_cache(&mut cache).unwrap();
            assert_eq!(expected, format!("{again:?}"));
        }
        assert_eq!(cache.hits(), 160);
        assert_eq!(cache.misses(), 160);
    }

    #[test]
    fn bounded_payload_and_explicit_release() {
        let mut cache = PathCache::new(24_000);
        for seed in 0..160 {
            shape(seed).build_with_cache(&mut cache).unwrap();
            assert!(cache.retained_payload_bytes() <= 24_000);
        }
        assert!(cache.len() < 160);
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.retained_payload_bytes(), 0);
        let mut disabled = PathCache::new(0);
        shape(0).build_with_cache(&mut disabled).unwrap();
        assert!(disabled.is_empty());
        let mut too_small = PathCache::new(1);
        shape(0).build_with_cache(&mut too_small).unwrap();
        assert!(too_small.is_empty());
    }

    #[test]
    fn style_and_dashes_distinguish_identical_geometry() {
        let mut cache = PathCache::default();
        for variant in 0..12 {
            let make = || {
                let mut builder = shape(0).with_style(if variant < 8 {
                    let mut options = StrokeOptions::default()
                        .with_line_width(1. + (variant % 2) as f32)
                        .with_tolerance(0.01 + (variant / 2 % 2) as f32 * 0.2);
                    if variant >= 4 {
                        options.start_cap = lyon::tessellation::LineCap::Round;
                    }
                    PathStyle::Stroke(options)
                } else {
                    PathStyle::Fill(
                        FillOptions::default().with_tolerance(0.01 + (variant % 2) as f32 * 0.2),
                    )
                });
                if variant >= 10 {
                    // A dash key is conservative even when fill ignores dashes.
                    builder = builder.dash_array(&[px(3.), px(1.)]);
                }
                builder
            };
            let expected = format!("{:?}", make().build().unwrap());
            assert_eq!(
                expected,
                format!("{:?}", make().build_with_cache(&mut cache).unwrap())
            );
        }
        assert_eq!(cache.misses(), 12);
        assert_eq!(cache.len(), 12);
    }
}
