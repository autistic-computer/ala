use ala::{
    AtlasBackend, AtlasKey, AtlasState, AtlasTextureId, AtlasTextureKind, AtlasTextureList,
    AtlasTile, Bounds, DevicePixels, PlatformAtlas, Point, Size,
};
use anyhow::{Context as _, Result};
use etagere::{BucketedAtlasAllocator, size2};
use parking_lot::Mutex;
use std::{borrow::Cow, mem, ops, sync::Arc};

use crate::wgpu::WgpuContext;

fn device_size_to_etagere(size: Size<DevicePixels>) -> etagere::Size {
    size2(size.width.0, size.height.0)
}

fn etagere_point_to_device(point: etagere::Point) -> Point<DevicePixels> {
    Point {
        x: DevicePixels(point.x),
        y: DevicePixels(point.y),
    }
}

pub struct WgpuAtlas(Mutex<AtlasState<WgpuAtlasTextures>>);

struct PendingUpload {
    id: AtlasTextureId,
    bounds: Bounds<DevicePixels>,
    data: ops::Range<usize>,
}

struct WgpuAtlasTextures {
    device: Arc<wgpu::Device>,
    max_texture_size: u32,
    color_texture_format: wgpu::TextureFormat,
    storage: WgpuAtlasStorage,
    pending_uploads: Vec<PendingUpload>,
    pending_bytes: Vec<u8>,
    next_texture_generation: u64,
}

pub struct WgpuTextureInfo {
    pub view: wgpu::TextureView,
    pub generation: u64,
}

impl WgpuAtlas {
    pub fn new(device: Arc<wgpu::Device>, color_texture_format: wgpu::TextureFormat) -> Self {
        let max_texture_size = device.limits().max_texture_dimension_2d;
        WgpuAtlas(Mutex::new(AtlasState::new(WgpuAtlasTextures {
            device,
            max_texture_size,
            color_texture_format,
            storage: WgpuAtlasStorage::default(),
            pending_uploads: Vec::new(),
            pending_bytes: Vec::new(),
            next_texture_generation: 0,
        })))
    }

    pub fn from_context(context: &WgpuContext) -> Self {
        Self::new(context.device.clone(), context.color_texture_format())
    }

    pub fn before_frame(&self, encoder: &mut Option<wgpu::CommandEncoder>) {
        let mut lock = self.0.lock();
        lock.backend.flush_uploads(encoder);
    }

    pub fn get_texture_info(&self, id: AtlasTextureId) -> Option<WgpuTextureInfo> {
        let lock = self.0.lock();
        let texture = lock.backend.storage.get(id)?;
        Some(WgpuTextureInfo {
            view: texture.view.clone(),
            generation: texture.generation,
        })
    }

    pub fn clear(&self) {
        self.0.lock().clear(|textures| {
            textures.storage = WgpuAtlasStorage::default();
            textures.pending_uploads.clear();
            textures.pending_bytes.clear();
        });
    }

    pub fn handle_device_lost(&self, context: &WgpuContext) {
        self.0.lock().clear(|textures| {
            textures.device = context.device.clone();
            textures.color_texture_format = context.color_texture_format();
            textures.storage = WgpuAtlasStorage::default();
            textures.pending_uploads.clear();
            textures.pending_bytes.clear();
        });
    }
}

impl PlatformAtlas for WgpuAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: AtlasKey,
        build: &mut dyn FnMut() -> Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> Result<Option<AtlasTile>> {
        self.0.lock().get_or_insert_with(key, build)
    }

    fn remove(&self, key: &AtlasKey) {
        self.0.lock().remove(key);
    }

    fn generation(&self) -> u64 {
        self.0.lock().generation()
    }
}

impl AtlasBackend for WgpuAtlasTextures {
    fn insert(
        &mut self,
        kind: AtlasTextureKind,
        size: Size<DevicePixels>,
        bytes: &[u8],
    ) -> Result<AtlasTile> {
        anyhow::ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "{kind:?} atlas upload requires positive dimensions, got {size:?}"
        );
        anyhow::ensure!(
            size.width.0 as u32 <= self.max_texture_size
                && size.height.0 as u32 <= self.max_texture_size,
            "atlas tile {size:?} exceeds the device texture limit {}",
            self.max_texture_size
        );
        let channels = match kind {
            AtlasTextureKind::Monochrome => 1,
            AtlasTextureKind::Polychrome | AtlasTextureKind::Subpixel => 4,
        };
        let expected = (size.width.0 as usize)
            .checked_mul(size.height.0 as usize)
            .and_then(|pixels| pixels.checked_mul(channels))
            .ok_or_else(|| anyhow::anyhow!("atlas upload byte count overflow for {size:?}"))?;
        anyhow::ensure!(
            bytes.len() == expected,
            "{kind:?} atlas upload for {size:?} requires {expected} bytes, got {}",
            bytes.len()
        );
        let tile = self.allocate(size, kind).context("failed to allocate")?;
        self.upload_texture(tile.texture_id, tile.bounds, bytes);
        Ok(tile)
    }

    fn remove(&mut self, tile: AtlasTile) {
        let id = tile.texture_id;
        let Some(texture_slot) = self.storage[id.kind].textures.get_mut(id.index as usize) else {
            return;
        };

        if let Some(mut texture) = texture_slot.take() {
            texture.allocator.deallocate(tile.tile_id.into());
            texture.decrement_ref_count();
            if texture.is_unreferenced() {
                self.pending_uploads
                    .retain(|upload| upload.id != texture.id);
                self.storage[id.kind]
                    .free_list
                    .push(texture.id.index as usize);
            } else {
                *texture_slot = Some(texture);
            }
        }
    }
}

impl WgpuAtlasTextures {
    fn allocate(
        &mut self,
        size: Size<DevicePixels>,
        texture_kind: AtlasTextureKind,
    ) -> Option<AtlasTile> {
        {
            let textures = &mut self.storage[texture_kind];

            if let Some(tile) = textures
                .iter_mut()
                .rev()
                .find_map(|texture| texture.allocate(size))
            {
                return Some(tile);
            }
        }

        let texture = self.push_texture(size, texture_kind);
        texture.allocate(size)
    }

    fn push_texture(
        &mut self,
        min_size: Size<DevicePixels>,
        kind: AtlasTextureKind,
    ) -> &mut WgpuAtlasTexture {
        const DEFAULT_ATLAS_SIZE: Size<DevicePixels> = Size {
            width: DevicePixels(1024),
            height: DevicePixels(1024),
        };
        let max_texture_size = self.max_texture_size as i32;
        let max_atlas_size = Size {
            width: DevicePixels(max_texture_size),
            height: DevicePixels(max_texture_size),
        };

        let size = min_size.min(&max_atlas_size).max(&DEFAULT_ATLAS_SIZE);
        let format = match kind {
            AtlasTextureKind::Monochrome => wgpu::TextureFormat::R8Unorm,
            AtlasTextureKind::Subpixel | AtlasTextureKind::Polychrome => self.color_texture_format,
        };

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: size.width.0 as u32,
                height: size.height.0 as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | if cfg!(test) {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_list = &mut self.storage[kind];
        let index = texture_list.free_list.pop();
        let generation = self.next_texture_generation;
        self.next_texture_generation = self.next_texture_generation.wrapping_add(1);

        let atlas_texture = WgpuAtlasTexture {
            id: AtlasTextureId {
                index: index.unwrap_or(texture_list.textures.len()) as u32,
                kind,
            },
            generation,
            allocator: BucketedAtlasAllocator::new(device_size_to_etagere(size)),
            format,
            texture,
            view,
            live_atlas_keys: 0,
        };

        if let Some(ix) = index {
            texture_list.textures[ix] = Some(atlas_texture);
            texture_list
                .textures
                .get_mut(ix)
                .and_then(|t| t.as_mut())
                .expect("texture must exist")
        } else {
            texture_list.textures.push(Some(atlas_texture));
            texture_list
                .textures
                .last_mut()
                .and_then(|t| t.as_mut())
                .expect("texture must exist")
        }
    }

    fn upload_texture(&mut self, id: AtlasTextureId, bounds: Bounds<DevicePixels>, bytes: &[u8]) {
        let start = self.pending_bytes.len();
        self.pending_bytes.extend_from_slice(bytes);
        if let Some(texture) = self.storage.get(id) {
            swizzle_upload_data(&mut self.pending_bytes[start..], texture.format);
        }
        self.pending_uploads.push(PendingUpload {
            id,
            bounds,
            data: start..self.pending_bytes.len(),
        });
    }

    fn flush_uploads(&mut self, encoder: &mut Option<wgpu::CommandEncoder>) {
        let uploads = mem::take(&mut self.pending_uploads);
        let bytes = mem::take(&mut self.pending_bytes);
        let row_bytes = |upload: &PendingUpload, texture: &WgpuAtlasTexture| {
            upload.bounds.size.width.0 as u64 * u64::from(texture.bytes_per_pixel())
        };
        let pitch = uploads
            .iter()
            .filter_map(|upload| Some(row_bytes(upload, self.storage.get(upload.id)?)))
            .max()
            .unwrap_or(0)
            .next_multiple_of(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT));
        let mut placements = Vec::with_capacity(uploads.len());
        let (mut shelf, mut shelf_height, mut column) = (0, 0, 0);
        for upload in &uploads {
            let Some(texture) = self.storage.get(upload.id) else {
                continue;
            };
            let row = row_bytes(upload, texture);
            if column + row > pitch {
                shelf += shelf_height * pitch;
                shelf_height = 0;
                column = 0;
            }
            placements.push((upload, texture, row, shelf + column));
            column = (column + row).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
            shelf_height = shelf_height.max(upload.bounds.size.height.0 as u64);
        }
        if placements.is_empty() {
            return;
        }

        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas_uploads"),
            size: shelf + shelf_height * pitch,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
        });
        {
            let mut view = match staging.get_mapped_range_mut(..) {
                Ok(view) => view,
                Err(error) => {
                    log::error!("failed to map the atlas upload buffer: {error}");
                    return;
                }
            };
            for &(upload, _, row, offset) in &placements {
                for (line, source) in bytes[upload.data.clone()]
                    .chunks_exact(row as usize)
                    .enumerate()
                {
                    let start = (offset + line as u64 * pitch) as usize;
                    view.slice(start..start + row as usize)
                        .copy_from_slice(source);
                }
            }
        }
        staging.unmap();

        let encoder = encoder.get_or_insert_with(|| {
            self.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("atlas_upload_encoder"),
                })
        });
        for (upload, texture, _, offset) in placements {
            encoder.copy_buffer_to_texture(
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(pitch as u32),
                        rows_per_image: None,
                    },
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: upload.bounds.origin.x.0 as u32,
                        y: upload.bounds.origin.y.0 as u32,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: upload.bounds.size.width.0 as u32,
                    height: upload.bounds.size.height.0 as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

#[derive(Default)]
struct WgpuAtlasStorage {
    monochrome_textures: AtlasTextureList<WgpuAtlasTexture>,
    subpixel_textures: AtlasTextureList<WgpuAtlasTexture>,
    polychrome_textures: AtlasTextureList<WgpuAtlasTexture>,
}

impl ops::Index<AtlasTextureKind> for WgpuAtlasStorage {
    type Output = AtlasTextureList<WgpuAtlasTexture>;
    fn index(&self, kind: AtlasTextureKind) -> &Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &self.monochrome_textures,
            AtlasTextureKind::Subpixel => &self.subpixel_textures,
            AtlasTextureKind::Polychrome => &self.polychrome_textures,
        }
    }
}

impl ops::IndexMut<AtlasTextureKind> for WgpuAtlasStorage {
    fn index_mut(&mut self, kind: AtlasTextureKind) -> &mut Self::Output {
        match kind {
            AtlasTextureKind::Monochrome => &mut self.monochrome_textures,
            AtlasTextureKind::Subpixel => &mut self.subpixel_textures,
            AtlasTextureKind::Polychrome => &mut self.polychrome_textures,
        }
    }
}

impl WgpuAtlasStorage {
    fn get(&self, id: AtlasTextureId) -> Option<&WgpuAtlasTexture> {
        self[id.kind]
            .textures
            .get(id.index as usize)
            .and_then(|t| t.as_ref())
    }
}

struct WgpuAtlasTexture {
    id: AtlasTextureId,
    generation: u64,
    allocator: BucketedAtlasAllocator,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
    live_atlas_keys: u32,
}

impl WgpuAtlasTexture {
    fn allocate(&mut self, size: Size<DevicePixels>) -> Option<AtlasTile> {
        let allocation = self.allocator.allocate(device_size_to_etagere(size))?;
        let tile = AtlasTile {
            texture_id: self.id,
            tile_id: allocation.id.into(),
            padding: 0,
            bounds: Bounds {
                origin: etagere_point_to_device(allocation.rectangle.min),
                size,
            },
        };
        self.live_atlas_keys += 1;
        Some(tile)
    }

    fn bytes_per_pixel(&self) -> u8 {
        match self.format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm => 4,
            _ => 4,
        }
    }

    fn decrement_ref_count(&mut self) {
        self.live_atlas_keys -= 1;
    }

    fn is_unreferenced(&self) -> bool {
        self.live_atlas_keys == 0
    }
}

fn swizzle_upload_data(bytes: &mut [u8], format: wgpu::TextureFormat) {
    if format == wgpu::TextureFormat::Rgba8Unorm {
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;
    use ala::block_on;
    use ala::{ImageId, RenderImageParams};
    use std::sync::Arc;

    fn test_device_and_queue() -> anyhow::Result<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
        block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                })
                .await
                .map_err(|error| anyhow::anyhow!("failed to request adapter: {error}"))?;
            eprintln!("atlas test adapter: {:?}", adapter.get_info());
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("wgpu_atlas_test_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits())
                        .using_alignment(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::disabled(),
                })
                .await
                .map_err(|error| anyhow::anyhow!("failed to request device: {error}"))?;
            Ok((Arc::new(device), Arc::new(queue)))
        })
    }

    #[test]
    fn invalid_uploads_do_not_allocate_or_cache_tiles() -> anyhow::Result<()> {
        let (device, _) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, wgpu::TextureFormat::Bgra8Unorm);

        for (index, (width, height, bytes)) in [
            (1, 1, vec![0; 3]),
            (1, 1, vec![0; 5]),
            (0, 1, Vec::new()),
            (-1, 1, Vec::new()),
        ]
        .into_iter()
        .enumerate()
        {
            let key = AtlasKey::Image(RenderImageParams {
                image_id: ImageId(index),
                frame_index: 0,
            });
            let size = Size {
                width: DevicePixels(width),
                height: DevicePixels(height),
            };
            let result = atlas
                .get_or_insert_with(key.clone(), &mut || Ok(Some((size, Cow::Borrowed(&bytes)))));
            assert!(
                result.is_err(),
                "{size:?} with {} bytes was accepted",
                bytes.len()
            );
            let state = atlas.0.lock();
            assert!(!state.contains(&key));
            assert!(
                state
                    .backend
                    .storage
                    .polychrome_textures
                    .textures
                    .is_empty()
            );
            assert!(state.backend.pending_uploads.is_empty());
        }
        Ok(())
    }

    #[test]
    fn oversized_upload_does_not_allocate_a_backing_texture() -> anyhow::Result<()> {
        let (device, _) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, wgpu::TextureFormat::Bgra8Unorm);
        let width = atlas.0.lock().backend.max_texture_size as i32 + 1;
        let size = Size {
            width: DevicePixels(width),
            height: DevicePixels(1),
        };
        let bytes = vec![0; width as usize * 4];
        let key = AtlasKey::Image(RenderImageParams {
            image_id: ImageId(0),
            frame_index: 0,
        });
        assert!(
            atlas
                .get_or_insert_with(key.clone(), &mut || {
                    Ok(Some((size, Cow::Borrowed(&bytes))))
                })
                .is_err()
        );
        let state = atlas.0.lock();
        assert!(!state.contains(&key));
        assert!(
            state
                .backend
                .storage
                .polychrome_textures
                .textures
                .is_empty()
        );
        assert!(state.backend.pending_uploads.is_empty());
        Ok(())
    }

    #[test]
    fn before_frame_skips_uploads_for_removed_texture() -> anyhow::Result<()> {
        let (device, queue) = test_device_and_queue()?;

        let atlas = WgpuAtlas::new(device, wgpu::TextureFormat::Bgra8Unorm);
        let key = AtlasKey::Image(RenderImageParams {
            image_id: ImageId(1),
            frame_index: 0,
        });
        let size = Size {
            width: DevicePixels(1),
            height: DevicePixels(1),
        };
        let mut build = || Ok(Some((size, Cow::Owned(vec![0, 0, 0, 255]))));

        atlas
            .get_or_insert_with(key.clone(), &mut build)?
            .expect("tile should be created");
        atlas.remove(&key);
        flush(&atlas, &queue);
        Ok(())
    }

    #[test]
    fn remove_deallocates_tile_space_for_reuse() -> anyhow::Result<()> {
        let (device, _) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, wgpu::TextureFormat::Bgra8Unorm);

        let small = Size {
            width: DevicePixels(64),
            height: DevicePixels(64),
        };
        let big = Size {
            width: DevicePixels(700),
            height: DevicePixels(700),
        };

        let make_key = |image_id: usize| {
            AtlasKey::Image(RenderImageParams {
                image_id: ImageId(image_id),
                frame_index: 0,
            })
        };
        let insert = |key: AtlasKey, size: Size<DevicePixels>| {
            let byte_count = (size.width.0 as usize) * (size.height.0 as usize) * 4;
            atlas
                .get_or_insert_with(key, &mut || {
                    Ok(Some((size, Cow::Owned(vec![0u8; byte_count]))))
                })
                .expect("allocation should succeed")
                .expect("callback returns Some")
        };

        let keeper_key = make_key(1);
        let big_key_a = make_key(2);
        let big_key_b = make_key(3);

        let keeper_tile = insert(keeper_key, small);
        let tile_a = insert(big_key_a.clone(), big);
        assert_eq!(keeper_tile.texture_id, tile_a.texture_id);

        atlas.remove(&big_key_a);
        let tile_b = insert(big_key_b, big);
        assert_eq!(tile_b.texture_id, keeper_tile.texture_id);
        Ok(())
    }

    #[test]
    fn reused_texture_id_has_new_generation() -> anyhow::Result<()> {
        let (device, _) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device, wgpu::TextureFormat::Bgra8Unorm);
        let size = Size {
            width: DevicePixels(700),
            height: DevicePixels(700),
        };
        let make_key = |image_id| {
            AtlasKey::Image(RenderImageParams {
                image_id: ImageId(image_id),
                frame_index: 0,
            })
        };
        let insert = |key: AtlasKey| {
            atlas
                .get_or_insert_with(key, &mut || {
                    Ok(Some((
                        size,
                        Cow::Owned(vec![0; size.width.0 as usize * size.height.0 as usize * 4]),
                    )))
                })
                .expect("allocation should succeed")
                .expect("callback returns Some")
        };

        let first_key = make_key(1);
        let first_tile = insert(first_key.clone());
        let first_generation = atlas
            .get_texture_info(first_tile.texture_id)
            .context("first texture should exist")?
            .generation;
        atlas.remove(&first_key);
        assert!(atlas.get_texture_info(first_tile.texture_id).is_none());

        let second_tile = insert(make_key(2));
        let second_generation = atlas
            .get_texture_info(second_tile.texture_id)
            .context("second texture should exist")?
            .generation;

        assert_eq!(second_tile.texture_id, first_tile.texture_id);
        assert_ne!(second_generation, first_generation);
        Ok(())
    }

    fn flush(atlas: &WgpuAtlas, queue: &wgpu::Queue) {
        let mut encoder = None;
        atlas.before_frame(&mut encoder);
        queue.submit(encoder.map(|encoder| encoder.finish()));
    }

    fn read_tile(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        bounds: Bounds<DevicePixels>,
        bytes_per_pixel: u32,
    ) -> Vec<u8> {
        let width = bounds.size.width.0 as u32;
        let height = bounds.size.height.0 as u32;
        let row = width * bytes_per_pixel;
        let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas_readback"),
            size: u64::from(padded * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: bounds.origin.x.0 as u32,
                    y: bounds.origin.y.0 as u32,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("readback");
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        mapped
            .chunks_exact(padded as usize)
            .flat_map(|line| &line[..row as usize])
            .copied()
            .collect()
    }

    #[test]
    fn uploads_land_at_their_tile_bounds() -> anyhow::Result<()> {
        let (device, queue) = test_device_and_queue()?;
        let atlas = WgpuAtlas::new(device.clone(), wgpu::TextureFormat::Bgra8Unorm);
        let shapes = [
            (AtlasTextureKind::Monochrome, 1, 1),
            (AtlasTextureKind::Monochrome, 3, 7),
            (AtlasTextureKind::Monochrome, 17, 5),
            (AtlasTextureKind::Monochrome, 300, 2),
            (AtlasTextureKind::Monochrome, 11, 19),
            (AtlasTextureKind::Polychrome, 5, 3),
            (AtlasTextureKind::Polychrome, 70, 9),
            (AtlasTextureKind::Polychrome, 1, 1),
            (AtlasTextureKind::Monochrome, 64, 64),
        ];
        let mut expected = Vec::new();
        for round in 0..2 {
            for (index, &(kind, width, height)) in shapes.iter().enumerate() {
                let size = Size {
                    width: DevicePixels(width),
                    height: DevicePixels(height),
                };
                let channels = if kind == AtlasTextureKind::Monochrome {
                    1
                } else {
                    4
                };
                let bytes: Vec<u8> = (0..width * height * channels)
                    .map(|byte| (byte * 7 + index as i32 * 13 + round * 101) as u8)
                    .collect();
                let key = match kind {
                    AtlasTextureKind::Monochrome => AtlasKey::Svg(ala::RenderSvgParams {
                        path: format!("tile-{round}-{index}").into(),
                        size,
                    }),
                    _ => AtlasKey::Image(RenderImageParams {
                        image_id: ImageId(round as usize * 100 + index),
                        frame_index: 0,
                    }),
                };
                let tile = atlas
                    .get_or_insert_with(key, &mut || Ok(Some((size, Cow::Owned(bytes.clone())))))?
                    .context("tile")?;
                expected.push((tile, channels as u32, bytes));
            }
            flush(&atlas, &queue);
        }
        for (tile, channels, bytes) in expected {
            let texture = atlas
                .0
                .lock()
                .backend
                .storage
                .get(tile.texture_id)
                .context("texture")?
                .texture
                .clone();
            assert_eq!(
                read_tile(&device, &queue, &texture, tile.bounds, channels),
                bytes,
                "tile {tile:?}"
            );
        }
        Ok(())
    }

    fn texture_bytes(atlas: &WgpuAtlas) -> (usize, u64) {
        let lock = atlas.0.lock();
        let storage = &lock.backend.storage;
        [
            &storage.monochrome_textures,
            &storage.subpixel_textures,
            &storage.polychrome_textures,
        ]
        .into_iter()
        .flat_map(|list| list.textures.iter().flatten())
        .fold((0, 0), |(pages, bytes), texture| {
            (
                pages + 1,
                bytes
                    + u64::from(texture.texture.width())
                        * u64::from(texture.texture.height())
                        * u64::from(texture.bytes_per_pixel()),
            )
        })
    }

    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "benchmark: cargo test -p ala-platform --all-features --release bench_glyph_atlas_zoom_bytes -- --ignored --nocapture"]
    fn bench_glyph_atlas_zoom_bytes() -> anyhow::Result<()> {
        use ala::{
            IsZero, PlatformTextSystem, RenderGlyphParams, SUBPIXEL_VARIANTS_X, font, point, px,
        };
        let text_system = crate::wgpu::CosmicTextSystem::new("DejaVu Sans");
        let font_id = text_system.font_id(&font("DejaVu Sans Mono"))?;
        let (device, queue) = test_device_and_queue()?;
        for subpixel_rendering in [false, true] {
            let atlas = WgpuAtlas::new(device.clone(), wgpu::TextureFormat::Bgra8Unorm);
            let mut steps = Vec::new();
            for font_size in 13..=23 {
                for ch in (b' '..=b'~').map(char::from) {
                    let Some(glyph_id) = text_system.glyph_for_char(font_id, ch) else {
                        continue;
                    };
                    for variant in 0..SUBPIXEL_VARIANTS_X {
                        let params = RenderGlyphParams {
                            font_id,
                            glyph_id,
                            font_size: px(font_size as f32),
                            subpixel_variant: point(variant, 0),
                            scale_factor: 1.25,
                            is_emoji: false,
                            subpixel_rendering,
                            font_smoothing: false,
                            font_smoothing_strength: 0,
                            synthetic_bold: false,
                            synthetic_italic: false,
                        };
                        let bounds = text_system.glyph_raster_bounds(&params)?;
                        if bounds.is_zero() {
                            continue;
                        }
                        atlas.get_or_insert_with(AtlasKey::Glyph(params.clone()), &mut || {
                            let (size, bytes) = text_system.rasterize_glyph(&params, bounds)?;
                            Ok(Some((size, Cow::Owned(bytes))))
                        })?;
                    }
                }
                flush(&atlas, &queue);
                let (pages, bytes) = texture_bytes(&atlas);
                steps.push(format!("{font_size}px={pages}/{}KiB", bytes / 1024));
            }
            println!(
                "bench_glyph_atlas_zoom_bytes subpixel={subpixel_rendering} pages/bytes after each size: {}",
                steps.join(" ")
            );
        }
        Ok(())
    }

    #[test]
    fn swizzle_upload_data_preserves_bgra_uploads() {
        let mut data = vec![0x10, 0x20, 0x30, 0x40];
        swizzle_upload_data(&mut data, wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!(data, [0x10, 0x20, 0x30, 0x40]);
    }

    #[test]
    fn swizzle_upload_data_converts_bgra_to_rgba() {
        let mut data = vec![0x10, 0x20, 0x30, 0x40, 0xAA, 0xBB, 0xCC, 0xDD];
        swizzle_upload_data(&mut data, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(data, [0x30, 0x20, 0x10, 0x40, 0xCC, 0xBB, 0xAA, 0xDD]);
    }
}
