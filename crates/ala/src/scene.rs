#![cfg_attr(windows, allow(dead_code))]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    AtlasTextureId, AtlasTile, Background, Bounds, ContentMask, Corners, Edges, Glass, Hsla,
    Pixels, Point, Radians, ScaledPixels, SharedString, Size, bounds_tree::BoundsTree, point,
};
use std::{
    fmt::Debug,
    hash::{Hash, Hasher},
    iter::Peekable,
    ops::{Add, Range, Sub},
    rc::Rc,
    slice,
    sync::Arc,
};

#[allow(non_camel_case_types, unused)]
pub type PathVertex_ScaledPixels = PathVertex<ScaledPixels>;

pub type DrawOrder = u32;

#[cfg(feature = "perf-experiments")]
impl Scene {
    pub(crate) fn push_experimental_opacity(&mut self, opacity: f32) {
        let opacity = self.experimental_opacity.last().copied().unwrap_or(1.0) * opacity;
        self.experimental_opacity.push(opacity);
    }

    pub(crate) fn pop_experimental_opacity(&mut self) {
        self.experimental_opacity
            .pop()
            .expect("balanced opacity scopes");
    }
}

#[cfg(feature = "perf-experiments")]
fn apply_experimental_opacity(primitive: &mut Primitive, opacity: f32) {
    match primitive {
        Primitive::Shadow(value) => value.color = value.color.opacity(opacity),
        Primitive::Quad(value) => {
            value.background = value.background.opacity(opacity);
            value.border_color = value.border_color.opacity(opacity);
        }
        Primitive::Path(value) => value.color = value.color.opacity(opacity),
        Primitive::Underline(value) => value.color = value.color.opacity(opacity),
        Primitive::MonochromeSprite(value) => value.color = value.color.opacity(opacity),
        Primitive::SubpixelSprite(value) => value.color = value.color.opacity(opacity),
        Primitive::PolychromeSprite(value) => value.opacity *= opacity,
        Primitive::Surface(_) | Primitive::ShaderLayer(_) | Primitive::Glass(_) => {
            panic!("this experimental opacity prototype does not support surfaces or effect layers")
        }
    }
}

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct PaddedBool32(u32);

impl From<bool> for PaddedBool32 {
    fn from(value: bool) -> Self {
        PaddedBool32(value as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowCornerMask {
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub corner_smoothing: f32,
}

#[derive(Default)]
pub struct Scene {
    #[cfg(feature = "perf-experiments")]
    experimental_opacity: Vec<f32>,
    pub(crate) paint_operations: Vec<PaintOperation>,
    primitive_bounds: BoundsTree<ScaledPixels>,
    layer_stack: Vec<DrawOrder>,
    pub window_corner_mask: Option<WindowCornerMask>,
    pub shadows: Vec<Shadow>,
    pub quads: Vec<Quad>,
    pub paths: Vec<Path<ScaledPixels>>,
    pub underlines: Vec<Underline>,
    pub monochrome_sprites: Vec<MonochromeSprite>,
    pub subpixel_sprites: Vec<SubpixelSprite>,
    pub polychrome_sprites: Vec<PolychromeSprite>,
    pub surfaces: Vec<PaintSurface>,
    pub shader_layers: Vec<ShaderLayer>,
    pub glasses: Vec<Glass>,
    open_shader_layers: Vec<OpenShaderLayer>,
    transforms: Vec<SceneTransform>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SceneTransform {
    pub(crate) scale: f32,
    pub(crate) offset: Point<ScaledPixels>,
    pub(crate) clip: Bounds<ScaledPixels>,
}

impl SceneTransform {
    fn within(self, outer: SceneTransform) -> SceneTransform {
        SceneTransform {
            scale: self.scale * outer.scale,
            offset: outer.point(self.offset),
            clip: outer.bounds(self.clip).intersect(&outer.clip),
        }
    }

    fn length(&self, length: ScaledPixels) -> ScaledPixels {
        ScaledPixels(length.0 * self.scale)
    }

    fn point(&self, point: Point<ScaledPixels>) -> Point<ScaledPixels> {
        Point {
            x: ScaledPixels(point.x.0 * self.scale + self.offset.x.0),
            y: ScaledPixels(point.y.0 * self.scale + self.offset.y.0),
        }
    }

    fn bounds(&self, bounds: Bounds<ScaledPixels>) -> Bounds<ScaledPixels> {
        Bounds {
            origin: self.point(bounds.origin),
            size: Size {
                width: self.length(bounds.size.width),
                height: self.length(bounds.size.height),
            },
        }
    }

    fn mask(&self, mask: ContentMask<ScaledPixels>) -> ContentMask<ScaledPixels> {
        ContentMask {
            bounds: self.bounds(mask.bounds).intersect(&self.clip),
        }
    }

    fn corners(&self, corners: Corners<ScaledPixels>) -> Corners<ScaledPixels> {
        Corners {
            top_left: self.length(corners.top_left),
            top_right: self.length(corners.top_right),
            bottom_right: self.length(corners.bottom_right),
            bottom_left: self.length(corners.bottom_left),
        }
    }

    fn edges(&self, edges: Edges<ScaledPixels>) -> Edges<ScaledPixels> {
        Edges {
            top: self.length(edges.top),
            right: self.length(edges.right),
            bottom: self.length(edges.bottom),
            left: self.length(edges.left),
        }
    }

    fn matrix(&self, matrix: TransformationMatrix) -> TransformationMatrix {
        if matrix == TransformationMatrix::unit() {
            return matrix;
        }
        let [[a, b], [c, d]] = matrix.rotation_scale;
        let (x, y) = (self.offset.x.0, self.offset.y.0);
        TransformationMatrix {
            rotation_scale: matrix.rotation_scale,
            translation: [
                matrix.translation[0] * self.scale + x - (a * x + b * y),
                matrix.translation[1] * self.scale + y - (c * x + d * y),
            ],
        }
    }

    fn apply(&self, primitive: &mut Primitive) {
        match primitive {
            Primitive::Shadow(shadow) => {
                shadow.bounds = self.bounds(shadow.bounds);
                shadow.content_mask = self.mask(shadow.content_mask);
                shadow.corner_radii = self.corners(shadow.corner_radii);
                shadow.blur_radius = self.length(shadow.blur_radius);
                shadow.element_bounds = self.bounds(shadow.element_bounds);
                shadow.element_corner_radii = self.corners(shadow.element_corner_radii);
            }
            Primitive::Quad(quad) => {
                quad.bounds = self.bounds(quad.bounds);
                quad.content_mask = self.mask(quad.content_mask);
                quad.corner_radii = self.corners(quad.corner_radii);
                quad.border_widths = self.edges(quad.border_widths);
            }
            Primitive::Path(path) => {
                path.bounds = self.bounds(path.bounds);
                path.content_mask = self.mask(path.content_mask);
                for vertex in &mut path.vertices {
                    vertex.xy_position = self.point(vertex.xy_position);
                    vertex.content_mask = self.mask(vertex.content_mask);
                }
            }
            Primitive::Underline(underline) => {
                underline.bounds = self.bounds(underline.bounds);
                underline.content_mask = self.mask(underline.content_mask);
                underline.thickness = self.length(underline.thickness);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.bounds = self.bounds(sprite.bounds);
                sprite.content_mask = self.mask(sprite.content_mask);
                sprite.transformation = self.matrix(sprite.transformation);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.bounds = self.bounds(sprite.bounds);
                sprite.content_mask = self.mask(sprite.content_mask);
                sprite.transformation = self.matrix(sprite.transformation);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.bounds = self.bounds(sprite.bounds);
                sprite.content_mask = self.mask(sprite.content_mask);
                sprite.corner_radii = self.corners(sprite.corner_radii);
            }
            Primitive::Surface(surface) => {
                surface.bounds = self.bounds(surface.bounds);
                surface.content_mask = self.mask(surface.content_mask);
                surface.corner_radii = self.corners(surface.corner_radii);
            }
            Primitive::ShaderLayer(layer) => {
                layer.bounds = self.bounds(layer.bounds);
                layer.content_mask = self.mask(layer.content_mask);
            }
            Primitive::Glass(glass) => {
                glass.bounds = self.bounds(glass.bounds);
                glass.backdrop_bounds = self.bounds(glass.backdrop_bounds);
                glass.content_mask = self.mask(glass.content_mask);
                let count = (glass.shape_count as usize).min(glass.shapes.len());
                for (bounds, corners) in &mut glass.shapes[..count] {
                    *bounds = self.bounds(*bounds);
                    *corners = self.corners(*corners);
                }
                if let Some(shadow) = &mut glass.shadow {
                    shadow.bounds = self.bounds(shadow.bounds);
                    shadow.corner_radii = self.corners(shadow.corner_radii);
                    shadow.blur_radius = self.length(shadow.blur_radius);
                }
            }
        }
    }
}

struct OpenShaderLayer {
    descriptor: ShaderLayerDescriptor,
    scene: Scene,
}

impl Scene {
    pub fn clear(&mut self) {
        #[cfg(feature = "perf-experiments")]
        self.experimental_opacity.clear();
        self.paint_operations.clear();
        self.primitive_bounds.clear();
        self.layer_stack.clear();
        self.window_corner_mask = None;
        self.paths.clear();
        self.shadows.clear();
        self.quads.clear();
        self.underlines.clear();
        self.monochrome_sprites.clear();
        self.subpixel_sprites.clear();
        self.polychrome_sprites.clear();
        self.surfaces.clear();
        self.shader_layers.clear();
        self.glasses.clear();
        self.open_shader_layers.clear();
        self.transforms.clear();
    }

    pub fn len(&self) -> usize {
        self.paint_operations.len()
    }

    pub fn push_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let placed = match self.transforms.last() {
            Some(transform) => transform.bounds(bounds),
            None => bounds,
        };
        self.target().open_layer(placed);
        self.paint_operations
            .push(PaintOperation::StartLayer(bounds));
    }

    pub(crate) fn push_transform(&mut self, transform: SceneTransform) {
        self.paint_operations
            .push(PaintOperation::StartTransform(transform));
        let composed = match self.transforms.last() {
            Some(outer) => transform.within(*outer),
            None => transform,
        };
        self.transforms.push(composed);
    }

    pub(crate) fn pop_transform(&mut self) {
        self.paint_operations.push(PaintOperation::EndTransform);
        self.transforms.pop();
    }

    pub fn pop_layer(&mut self) {
        self.target().layer_stack.pop();
        self.paint_operations.push(PaintOperation::EndLayer);
    }

    pub(crate) fn push_shader_layer(&mut self, descriptor: ShaderLayerDescriptor) {
        self.paint_operations
            .push(PaintOperation::StartShaderLayer(descriptor.clone()));
        let mut descriptor = descriptor;
        if let Some(transform) = self.transforms.last() {
            descriptor.bounds = transform.bounds(descriptor.bounds);
            descriptor.content_mask = transform.mask(descriptor.content_mask);
        }
        self.open_shader_layers.push(OpenShaderLayer {
            descriptor,
            scene: Scene::default(),
        });
    }

    pub(crate) fn pop_shader_layer(&mut self) {
        self.paint_operations.push(PaintOperation::EndShaderLayer);
        let Some(OpenShaderLayer {
            descriptor,
            mut scene,
        }) = self.open_shader_layers.pop()
        else {
            return;
        };
        scene.finish();
        let layer = ShaderLayer {
            order: 0,
            bounds: descriptor.bounds,
            content_mask: descriptor.content_mask,
            shader: descriptor.shader,
            uniforms: descriptor.uniforms,
            scene: Rc::new(scene),
        };
        self.target()
            .place_primitive(&mut Primitive::ShaderLayer(layer));
    }

    fn target(&mut self) -> &mut Scene {
        match self.open_shader_layers.len() {
            0 => self,
            open => &mut self.open_shader_layers[open - 1].scene,
        }
    }

    fn open_layer(&mut self, bounds: Bounds<ScaledPixels>) {
        let order = self.primitive_bounds.insert(bounds);
        self.layer_stack.push(order);
    }

    pub fn insert_primitive(&mut self, primitive: impl Into<Primitive>) {
        let mut primitive = primitive.into();
        #[cfg(feature = "perf-experiments")]
        if let Some(opacity) = self.experimental_opacity.last().copied() {
            let mut placed = primitive.clone();
            if let Some(transform) = self.transforms.last() {
                transform.apply(&mut placed);
            }
            apply_experimental_opacity(&mut placed, opacity);
            if self.target().place_primitive(&mut placed) {
                self.paint_operations
                    .push(PaintOperation::Primitive(primitive));
            }
            return;
        }
        let Some(transform) = self.transforms.last().copied() else {
            if self.target().place_primitive(&mut primitive) {
                self.paint_operations
                    .push(PaintOperation::Primitive(primitive));
            }
            return;
        };
        let mut placed = primitive.clone();
        transform.apply(&mut placed);
        if self.target().place_primitive(&mut placed) {
            self.paint_operations
                .push(PaintOperation::Primitive(primitive));
        }
    }

    fn place_primitive(&mut self, primitive: &mut Primitive) -> bool {
        let clipped_bounds = primitive
            .bounds()
            .intersect(&primitive.content_mask().bounds);

        if clipped_bounds.is_empty() {
            return false;
        }

        let ordered_bounds = match &*primitive {
            Primitive::Glass(glass) => glass.backdrop_bounds,
            _ => clipped_bounds,
        };
        let order = self
            .layer_stack
            .last()
            .copied()
            .unwrap_or_else(|| self.primitive_bounds.insert(ordered_bounds));
        match primitive {
            Primitive::Shadow(shadow) => {
                shadow.order = order;
                self.shadows.push(*shadow);
            }
            Primitive::Quad(quad) => {
                quad.order = order;
                self.quads.push(*quad);
            }
            Primitive::Path(path) => {
                path.order = order;
                path.id = PathId(self.paths.len());
                self.paths.push(path.clone());
            }
            Primitive::Underline(underline) => {
                underline.order = order;
                self.underlines.push(*underline);
            }
            Primitive::MonochromeSprite(sprite) => {
                sprite.order = order;
                self.monochrome_sprites.push(*sprite);
            }
            Primitive::SubpixelSprite(sprite) => {
                sprite.order = order;
                self.subpixel_sprites.push(*sprite);
            }
            Primitive::PolychromeSprite(sprite) => {
                sprite.order = order;
                self.polychrome_sprites.push(*sprite);
            }
            Primitive::Surface(surface) => {
                surface.order = order;
                self.surfaces.push(surface.clone());
            }
            Primitive::ShaderLayer(layer) => {
                layer.order = order;
                self.shader_layers.push(layer.clone());
            }
            Primitive::Glass(glass) => {
                glass.order = order;
                self.glasses.push((**glass).clone());
            }
        }
        true
    }

    pub fn replay(&mut self, range: Range<usize>, prev_scene: &Scene) {
        for operation in &prev_scene.paint_operations[range] {
            match operation {
                PaintOperation::Primitive(primitive) => self.insert_primitive(primitive.clone()),
                PaintOperation::StartLayer(bounds) => self.push_layer(*bounds),
                PaintOperation::EndLayer => self.pop_layer(),
                PaintOperation::StartShaderLayer(descriptor) => {
                    self.push_shader_layer(descriptor.clone())
                }
                PaintOperation::EndShaderLayer => self.pop_shader_layer(),
                PaintOperation::StartTransform(transform) => self.push_transform(*transform),
                PaintOperation::EndTransform => self.pop_transform(),
            }
        }
    }

    pub fn finish(&mut self) {
        self.shadows.sort_by_key(|shadow| shadow.order);
        self.quads.sort_by_key(|quad| quad.order);
        self.paths.sort_by_key(|path| path.order);
        self.underlines.sort_by_key(|underline| underline.order);
        self.monochrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.texture_id.index));
        self.subpixel_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.texture_id.index));
        self.polychrome_sprites
            .sort_by_key(|sprite| (sprite.order, sprite.tile.texture_id.index));
        self.surfaces.sort_by_key(|surface| surface.order);
        self.shader_layers.sort_by_key(|layer| layer.order);
        self.glasses.sort_by_key(|glass| glass.order);
        #[cfg(target_os = "macos")]
        for surface in &mut self.surfaces {
            let shown = surface.bounds.intersect(&surface.content_mask.bounds);
            if surface.underlay
                && !self.glasses.iter().any(|glass| {
                    glass.order > surface.order
                        && glass.reads_backdrop()
                        && glass.bounds.intersects(&shown)
                })
            {
                surface.image_buffer = None;
            }
        }
    }

    #[cfg_attr(
        all(
            any(target_os = "linux", target_os = "freebsd"),
            not(any(feature = "x11", feature = "wayland"))
        ),
        allow(dead_code)
    )]
    pub fn batches(&self) -> impl Iterator<Item = PrimitiveBatch> + '_ {
        BatchIterator {
            shadows_start: 0,
            shadows_iter: self.shadows.iter().peekable(),
            quads_start: 0,
            quads_iter: self.quads.iter().peekable(),
            paths_start: 0,
            paths_iter: self.paths.iter().peekable(),
            underlines_start: 0,
            underlines_iter: self.underlines.iter().peekable(),
            monochrome_sprites_start: 0,
            monochrome_sprites_iter: self.monochrome_sprites.iter().peekable(),
            subpixel_sprites_start: 0,
            subpixel_sprites_iter: self.subpixel_sprites.iter().peekable(),
            polychrome_sprites_start: 0,
            polychrome_sprites_iter: self.polychrome_sprites.iter().peekable(),
            surfaces_start: 0,
            surfaces_iter: self.surfaces.iter().peekable(),
            shader_layers_start: 0,
            shader_layers_iter: self.shader_layers.iter().peekable(),
            glasses_start: 0,
            glasses_iter: self.glasses.iter().peekable(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Default)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub(crate) enum PrimitiveKind {
    Shadow,
    #[default]
    Quad,
    Path,
    Underline,
    MonochromeSprite,
    SubpixelSprite,
    PolychromeSprite,
    Surface,
    ShaderLayer,
    Glass,
}

pub(crate) enum PaintOperation {
    Primitive(Primitive),
    StartLayer(Bounds<ScaledPixels>),
    EndLayer,
    StartShaderLayer(ShaderLayerDescriptor),
    EndShaderLayer,
    StartTransform(SceneTransform),
    EndTransform,
}

#[derive(Clone)]
pub enum Primitive {
    Shadow(Shadow),
    Quad(Quad),
    Path(Path<ScaledPixels>),
    Underline(Underline),
    MonochromeSprite(MonochromeSprite),
    SubpixelSprite(SubpixelSprite),
    PolychromeSprite(PolychromeSprite),
    Surface(PaintSurface),
    ShaderLayer(ShaderLayer),
    Glass(Box<Glass>),
}

const _: () = assert!(std::mem::size_of::<Primitive>() <= std::mem::size_of::<Quad>() + 8);

impl Primitive {
    pub fn bounds(&self) -> &Bounds<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.bounds,
            Primitive::Quad(quad) => &quad.bounds,
            Primitive::Path(path) => &path.bounds,
            Primitive::Underline(underline) => &underline.bounds,
            Primitive::MonochromeSprite(sprite) => &sprite.bounds,
            Primitive::SubpixelSprite(sprite) => &sprite.bounds,
            Primitive::PolychromeSprite(sprite) => &sprite.bounds,
            Primitive::Surface(surface) => &surface.bounds,
            Primitive::ShaderLayer(layer) => &layer.bounds,
            Primitive::Glass(glass) => &glass.backdrop_bounds,
        }
    }

    pub fn content_mask(&self) -> &ContentMask<ScaledPixels> {
        match self {
            Primitive::Shadow(shadow) => &shadow.content_mask,
            Primitive::Quad(quad) => &quad.content_mask,
            Primitive::Path(path) => &path.content_mask,
            Primitive::Underline(underline) => &underline.content_mask,
            Primitive::MonochromeSprite(sprite) => &sprite.content_mask,
            Primitive::SubpixelSprite(sprite) => &sprite.content_mask,
            Primitive::PolychromeSprite(sprite) => &sprite.content_mask,
            Primitive::Surface(surface) => &surface.content_mask,
            Primitive::ShaderLayer(layer) => &layer.content_mask,
            Primitive::Glass(glass) => &glass.content_mask,
        }
    }
}

#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
struct BatchIterator<'a> {
    shadows_start: usize,
    shadows_iter: Peekable<slice::Iter<'a, Shadow>>,
    quads_start: usize,
    quads_iter: Peekable<slice::Iter<'a, Quad>>,
    paths_start: usize,
    paths_iter: Peekable<slice::Iter<'a, Path<ScaledPixels>>>,
    underlines_start: usize,
    underlines_iter: Peekable<slice::Iter<'a, Underline>>,
    monochrome_sprites_start: usize,
    monochrome_sprites_iter: Peekable<slice::Iter<'a, MonochromeSprite>>,
    subpixel_sprites_start: usize,
    subpixel_sprites_iter: Peekable<slice::Iter<'a, SubpixelSprite>>,
    polychrome_sprites_start: usize,
    polychrome_sprites_iter: Peekable<slice::Iter<'a, PolychromeSprite>>,
    surfaces_start: usize,
    surfaces_iter: Peekable<slice::Iter<'a, PaintSurface>>,
    shader_layers_start: usize,
    shader_layers_iter: Peekable<slice::Iter<'a, ShaderLayer>>,
    glasses_start: usize,
    glasses_iter: Peekable<slice::Iter<'a, Glass>>,
}

impl<'a> Iterator for BatchIterator<'a> {
    type Item = PrimitiveBatch;

    fn next(&mut self) -> Option<Self::Item> {
        let mut orders_and_kinds = [
            (
                self.shadows_iter.peek().map(|s| s.order),
                PrimitiveKind::Shadow,
            ),
            (self.quads_iter.peek().map(|q| q.order), PrimitiveKind::Quad),
            (self.paths_iter.peek().map(|q| q.order), PrimitiveKind::Path),
            (
                self.underlines_iter.peek().map(|u| u.order),
                PrimitiveKind::Underline,
            ),
            (
                self.monochrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::MonochromeSprite,
            ),
            (
                self.subpixel_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::SubpixelSprite,
            ),
            (
                self.polychrome_sprites_iter.peek().map(|s| s.order),
                PrimitiveKind::PolychromeSprite,
            ),
            (
                self.surfaces_iter.peek().map(|s| s.order),
                PrimitiveKind::Surface,
            ),
            (
                self.shader_layers_iter.peek().map(|layer| layer.order),
                PrimitiveKind::ShaderLayer,
            ),
            (
                self.glasses_iter.peek().map(|glass| glass.order),
                PrimitiveKind::Glass,
            ),
        ];
        orders_and_kinds.sort_by_key(|(order, kind)| (order.unwrap_or(u32::MAX), *kind));

        let first = orders_and_kinds[0];
        let second = orders_and_kinds[1];
        let (batch_kind, max_order_and_kind) = if first.0.is_some() {
            (first.1, (second.0.unwrap_or(u32::MAX), second.1))
        } else {
            return None;
        };

        match batch_kind {
            PrimitiveKind::Shadow => {
                let shadows_start = self.shadows_start;
                let mut shadows_end = shadows_start + 1;
                self.shadows_iter.next();
                while self
                    .shadows_iter
                    .next_if(|shadow| (shadow.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    shadows_end += 1;
                }
                self.shadows_start = shadows_end;
                Some(PrimitiveBatch::Shadows(shadows_start..shadows_end))
            }
            PrimitiveKind::Quad => {
                let quads_start = self.quads_start;
                let mut quads_end = quads_start + 1;
                self.quads_iter.next();
                while self
                    .quads_iter
                    .next_if(|quad| (quad.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    quads_end += 1;
                }
                self.quads_start = quads_end;
                Some(PrimitiveBatch::Quads(quads_start..quads_end))
            }
            PrimitiveKind::Path => {
                let paths_start = self.paths_start;
                let mut paths_end = paths_start + 1;
                self.paths_iter.next();
                while self
                    .paths_iter
                    .next_if(|path| (path.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    paths_end += 1;
                }
                self.paths_start = paths_end;
                Some(PrimitiveBatch::Paths(paths_start..paths_end))
            }
            PrimitiveKind::Underline => {
                let underlines_start = self.underlines_start;
                let mut underlines_end = underlines_start + 1;
                self.underlines_iter.next();
                while self
                    .underlines_iter
                    .next_if(|underline| (underline.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    underlines_end += 1;
                }
                self.underlines_start = underlines_end;
                Some(PrimitiveBatch::Underlines(underlines_start..underlines_end))
            }
            PrimitiveKind::MonochromeSprite => {
                let texture_id = self.monochrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.monochrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.monochrome_sprites_iter.next();
                while self
                    .monochrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.monochrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::MonochromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::SubpixelSprite => {
                let texture_id = self.subpixel_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.subpixel_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.subpixel_sprites_iter.next();
                while self
                    .subpixel_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.subpixel_sprites_start = sprites_end;
                Some(PrimitiveBatch::SubpixelSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::PolychromeSprite => {
                let texture_id = self.polychrome_sprites_iter.peek().unwrap().tile.texture_id;
                let sprites_start = self.polychrome_sprites_start;
                let mut sprites_end = sprites_start + 1;
                self.polychrome_sprites_iter.next();
                while self
                    .polychrome_sprites_iter
                    .next_if(|sprite| {
                        (sprite.order, batch_kind) < max_order_and_kind
                            && sprite.tile.texture_id == texture_id
                    })
                    .is_some()
                {
                    sprites_end += 1;
                }
                self.polychrome_sprites_start = sprites_end;
                Some(PrimitiveBatch::PolychromeSprites {
                    texture_id,
                    range: sprites_start..sprites_end,
                })
            }
            PrimitiveKind::Surface => {
                let surfaces_start = self.surfaces_start;
                let mut surfaces_end = surfaces_start + 1;
                self.surfaces_iter.next();
                while self
                    .surfaces_iter
                    .next_if(|surface| (surface.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    surfaces_end += 1;
                }
                self.surfaces_start = surfaces_end;
                Some(PrimitiveBatch::Surfaces(surfaces_start..surfaces_end))
            }
            PrimitiveKind::ShaderLayer => {
                let layers_start = self.shader_layers_start;
                let mut layers_end = layers_start + 1;
                self.shader_layers_iter.next();
                while self
                    .shader_layers_iter
                    .next_if(|layer| (layer.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    layers_end += 1;
                }
                self.shader_layers_start = layers_end;
                Some(PrimitiveBatch::ShaderLayers(layers_start..layers_end))
            }
            PrimitiveKind::Glass => {
                let glasses_start = self.glasses_start;
                let mut glasses_end = glasses_start + 1;
                self.glasses_iter.next();
                while self
                    .glasses_iter
                    .next_if(|glass| (glass.order, batch_kind) < max_order_and_kind)
                    .is_some()
                {
                    glasses_end += 1;
                }
                self.glasses_start = glasses_end;
                Some(PrimitiveBatch::Glass(glasses_start..glasses_end))
            }
        }
    }
}

#[derive(Debug)]
#[cfg_attr(
    all(
        any(target_os = "linux", target_os = "freebsd"),
        not(any(feature = "x11", feature = "wayland"))
    ),
    allow(dead_code)
)]
pub enum PrimitiveBatch {
    Shadows(Range<usize>),
    Quads(Range<usize>),
    Paths(Range<usize>),
    Underlines(Range<usize>),
    MonochromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    SubpixelSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    PolychromeSprites {
        texture_id: AtlasTextureId,
        range: Range<usize>,
    },
    Surfaces(Range<usize>),
    ShaderLayers(Range<usize>),
    Glass(Range<usize>),
}

impl PrimitiveBatch {
    pub fn label(&self) -> String {
        match self {
            Self::Shadows(range) => format!("shadows ({})", range.len()),
            Self::Quads(range) => format!("quads ({})", range.len()),
            Self::Paths(range) => format!("paths ({})", range.len()),
            Self::Underlines(range) => format!("underlines ({})", range.len()),
            Self::MonochromeSprites { texture_id, range } => {
                format!(
                    "monochrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::SubpixelSprites { texture_id, range } => {
                format!(
                    "subpixel sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::PolychromeSprites { texture_id, range } => {
                format!(
                    "polychrome sprites ({}) on atlas {}",
                    range.len(),
                    texture_id.index
                )
            }
            Self::Surfaces(range) => format!("surfaces ({})", range.len()),
            Self::ShaderLayers(range) => format!("shader layers ({})", range.len()),
            Self::Glass(range) => format!("glass ({})", range.len()),
        }
    }
}

#[derive(Default, Debug, Copy, Clone)]
#[repr(C)]
pub struct Quad {
    pub order: DrawOrder,
    pub border_style: BorderStyle,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub background: Background,
    pub border_color: Hsla,
    pub corner_radii: Corners<ScaledPixels>,
    pub border_widths: Edges<ScaledPixels>,
    pub corner_smoothing: f32,
    pub pad: [u32; 3],
}

const _: () = assert!(size_of::<Quad>().is_multiple_of(8));
const _: () = assert!(size_of::<Quad>().is_multiple_of(16));
const _: () = assert!(size_of::<Shadow>().is_multiple_of(8));

impl From<Quad> for Primitive {
    fn from(quad: Quad) -> Self {
        Primitive::Quad(quad)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct Underline {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub thickness: ScaledPixels,
    pub wavy: PaddedBool32,
}

impl From<Underline> for Primitive {
    fn from(underline: Underline) -> Self {
        Primitive::Underline(underline)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct Shadow {
    pub order: DrawOrder,
    pub blur_radius: ScaledPixels,
    pub bounds: Bounds<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub element_bounds: Bounds<ScaledPixels>,
    pub element_corner_radii: Corners<ScaledPixels>,
    pub inset: u32,
    pub corner_smoothing: f32,
}

impl From<Shadow> for Primitive {
    fn from(shadow: Shadow) -> Self {
        Primitive::Shadow(shadow)
    }
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[repr(C)]
pub enum BorderStyle {
    #[default]
    Solid = 0,
    Dashed = 1,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct TransformationMatrix {
    pub rotation_scale: [[f32; 2]; 2],
    pub translation: [f32; 2],
}

impl Eq for TransformationMatrix {}

impl TransformationMatrix {
    pub fn unit() -> Self {
        Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [0.0, 0.0],
        }
    }

    pub fn translate(mut self, point: Point<ScaledPixels>) -> Self {
        self.compose(Self {
            rotation_scale: [[1.0, 0.0], [0.0, 1.0]],
            translation: [point.x.0, point.y.0],
        })
    }

    pub fn rotate(self, angle: Radians) -> Self {
        self.compose(Self {
            rotation_scale: [
                [angle.0.cos(), -angle.0.sin()],
                [angle.0.sin(), angle.0.cos()],
            ],
            translation: [0.0, 0.0],
        })
    }

    pub fn scale(self, size: Size<f32>) -> Self {
        self.compose(Self {
            rotation_scale: [[size.width, 0.0], [0.0, size.height]],
            translation: [0.0, 0.0],
        })
    }

    #[inline]
    pub fn compose(self, other: TransformationMatrix) -> TransformationMatrix {
        if other == Self::unit() {
            return self;
        }
        TransformationMatrix {
            rotation_scale: [
                [
                    self.rotation_scale[0][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][0],
                    self.rotation_scale[0][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[0][1] * other.rotation_scale[1][1],
                ],
                [
                    self.rotation_scale[1][0] * other.rotation_scale[0][0]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][0],
                    self.rotation_scale[1][0] * other.rotation_scale[0][1]
                        + self.rotation_scale[1][1] * other.rotation_scale[1][1],
                ],
            ],
            translation: [
                self.translation[0]
                    + self.rotation_scale[0][0] * other.translation[0]
                    + self.rotation_scale[0][1] * other.translation[1],
                self.translation[1]
                    + self.rotation_scale[1][0] * other.translation[0]
                    + self.rotation_scale[1][1] * other.translation[1],
            ],
        }
    }

    pub fn apply(&self, point: Point<Pixels>) -> Point<Pixels> {
        let input = [point.x.0, point.y.0];
        let mut output = self.translation;
        for (i, output_cell) in output.iter_mut().enumerate() {
            for (k, input_cell) in input.iter().enumerate() {
                *output_cell += self.rotation_scale[i][k] * *input_cell;
            }
        }
        Point::new(output[0].into(), output[1].into())
    }
}

impl Default for TransformationMatrix {
    fn default() -> Self {
        Self::unit()
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct MonochromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<MonochromeSprite> for Primitive {
    fn from(sprite: MonochromeSprite) -> Self {
        Primitive::MonochromeSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct SubpixelSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub color: Hsla,
    pub tile: AtlasTile,
    pub transformation: TransformationMatrix,
}

impl From<SubpixelSprite> for Primitive {
    fn from(sprite: SubpixelSprite) -> Self {
        Primitive::SubpixelSprite(sprite)
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct PolychromeSprite {
    pub order: DrawOrder,
    pub pad: u32,
    pub grayscale: PaddedBool32,
    pub opacity: f32,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub corner_smoothing: f32,
    pub pad2: u32,
    pub tile: AtlasTile,
}

const _: () = assert!(size_of::<PolychromeSprite>().is_multiple_of(8));

impl From<PolychromeSprite> for Primitive {
    fn from(sprite: PolychromeSprite) -> Self {
        Primitive::PolychromeSprite(sprite)
    }
}

#[derive(Clone, Debug)]
pub struct PaintSurface {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub corner_radii: Corners<ScaledPixels>,
    pub corner_smoothing: f32,
    #[cfg(target_os = "macos")]
    pub image_buffer: Option<core_video::pixel_buffer::CVPixelBuffer>,
    #[cfg(target_os = "macos")]
    pub underlay: bool,
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub texture_view: wgpu::TextureView,
    #[cfg(target_os = "windows")]
    pub texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
}

impl From<PaintSurface> for Primitive {
    fn from(surface: PaintSurface) -> Self {
        Primitive::Surface(surface)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PathId(pub usize);

#[derive(Clone, Debug)]
pub struct Path<P: Clone + Debug + Default + PartialEq> {
    pub id: PathId,
    pub order: DrawOrder,
    pub bounds: Bounds<P>,
    pub content_mask: ContentMask<P>,
    pub vertices: Vec<PathVertex<P>>,
    pub color: Background,
    start: Point<P>,
    current: Point<P>,
    contour_count: usize,
}

impl Path<Pixels> {
    pub fn new(start: Point<Pixels>) -> Self {
        Self {
            id: PathId(0),
            order: DrawOrder::default(),
            vertices: Vec::new(),
            start,
            current: start,
            bounds: Bounds {
                origin: start,
                size: Default::default(),
            },
            content_mask: Default::default(),
            color: Default::default(),
            contour_count: 0,
        }
    }

    pub fn scale(&self, factor: f32) -> Path<ScaledPixels> {
        Path {
            id: self.id,
            order: self.order,
            bounds: self.bounds.scale(factor),
            content_mask: self.content_mask.scale(factor),
            vertices: self
                .vertices
                .iter()
                .map(|vertex| vertex.scale(factor))
                .collect(),
            start: self.start.map(|start| start.scale(factor)),
            current: self.current.scale(factor),
            contour_count: self.contour_count,
            color: self.color,
        }
    }

    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        self.start = to;
        self.current = to;
    }

    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }
        self.current = to;
    }

    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.contour_count += 1;
        if self.contour_count > 1 {
            self.push_triangle(
                (self.start, self.current, to),
                (point(0., 1.), point(0., 1.), point(0., 1.)),
            );
        }

        self.push_triangle(
            (self.current, ctrl, to),
            (point(0., 0.), point(0.5, 0.), point(1., 1.)),
        );
        self.current = to;
    }

    pub fn push_triangle(
        &mut self,
        xy: (Point<Pixels>, Point<Pixels>, Point<Pixels>),
        st: (Point<f32>, Point<f32>, Point<f32>),
    ) {
        self.bounds = self
            .bounds
            .union(&Bounds {
                origin: xy.0,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.1,
                size: Default::default(),
            })
            .union(&Bounds {
                origin: xy.2,
                size: Default::default(),
            });

        self.vertices.push(PathVertex {
            xy_position: xy.0,
            st_position: st.0,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.1,
            st_position: st.1,
            content_mask: Default::default(),
        });
        self.vertices.push(PathVertex {
            xy_position: xy.2,
            st_position: st.2,
            content_mask: Default::default(),
        });
    }
}

impl<T> Path<T>
where
    T: Clone + Debug + Default + PartialEq + PartialOrd + Add<T, Output = T> + Sub<Output = T>,
{
    #[allow(unused)]
    pub fn clipped_bounds(&self) -> Bounds<T> {
        self.bounds.intersect(&self.content_mask.bounds)
    }
}

impl From<Path<ScaledPixels>> for Primitive {
    fn from(path: Path<ScaledPixels>) -> Self {
        Primitive::Path(path)
    }
}

#[derive(Clone, Debug)]
#[repr(C)]
pub struct PathVertex<P: Clone + Debug + Default + PartialEq> {
    pub xy_position: Point<P>,
    pub st_position: Point<f32>,
    pub content_mask: ContentMask<P>,
}

impl PathVertex<Pixels> {
    pub fn scale(&self, factor: f32) -> PathVertex<ScaledPixels> {
        PathVertex {
            xy_position: self.xy_position.scale(factor),
            st_position: self.st_position,
            content_mask: self.content_mask.scale(factor),
        }
    }
}

#[derive(Clone)]
pub struct CustomShader(Arc<CustomShaderSource>);

struct CustomShaderSource {
    id: u64,
    wgsl: SharedString,
}

impl CustomShader {
    pub fn new(wgsl: impl Into<SharedString>) -> Self {
        let wgsl = wgsl.into();
        let mut hasher = std::hash::DefaultHasher::new();
        wgsl.hash(&mut hasher);
        Self(Arc::new(CustomShaderSource {
            id: hasher.finish(),
            wgsl,
        }))
    }

    pub fn id(&self) -> u64 {
        self.0.id
    }

    pub fn wgsl(&self) -> &str {
        &self.0.wgsl
    }
}

impl Debug for CustomShader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomShader")
            .field("id", &format_args!("{:016x}", self.0.id))
            .finish()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ShaderLayerDescriptor {
    pub(crate) bounds: Bounds<ScaledPixels>,
    pub(crate) content_mask: ContentMask<ScaledPixels>,
    pub(crate) shader: CustomShader,
    pub(crate) uniforms: Arc<[u8]>,
}

#[derive(Clone)]
pub struct ShaderLayer {
    pub order: DrawOrder,
    pub bounds: Bounds<ScaledPixels>,
    pub content_mask: ContentMask<ScaledPixels>,
    pub shader: CustomShader,
    pub uniforms: Arc<[u8]>,
    pub scene: Rc<Scene>,
}

impl Debug for ShaderLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShaderLayer")
            .field("order", &self.order)
            .field("bounds", &self.bounds)
            .field("shader", &self.shader)
            .finish_non_exhaustive()
    }
}

impl From<Glass> for Primitive {
    fn from(glass: Glass) -> Self {
        Primitive::Glass(Box::new(glass))
    }
}

impl From<ShaderLayer> for Primitive {
    fn from(layer: ShaderLayer) -> Self {
        Primitive::ShaderLayer(layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{px, size};

    fn quad(x: f32) -> Quad {
        let bounds = Bounds::new(point(px(x), px(0.)), size(px(4.), px(4.))).scale(1.0);
        Quad {
            bounds,
            content_mask: ContentMask { bounds },
            ..Default::default()
        }
    }

    fn layer_descriptor() -> ShaderLayerDescriptor {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(32.), px(32.))).scale(1.0);
        ShaderLayerDescriptor {
            bounds,
            content_mask: ContentMask { bounds },
            shader: CustomShader::new("@fragment fn main() {}"),
            uniforms: Arc::from([1u8, 2, 3]),
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_underlay_is_a_hole_unless_glass_painted_after_it_reads_it() {
        use core_video::pixel_buffer::{CVPixelBuffer, kCVPixelFormatType_32BGRA};
        let buffer = CVPixelBuffer::new(kCVPixelFormatType_32BGRA, 4, 4, None).unwrap();
        let rect = |x: f32| Bounds::new(point(px(x), px(0.)), size(px(32.), px(32.))).scale(1.0);
        let underlay = |x: f32| PaintSurface {
            order: 0,
            bounds: rect(x),
            content_mask: ContentMask { bounds: rect(x) },
            corner_radii: Corners::default(),
            corner_smoothing: 2.,
            image_buffer: Some(buffer.clone()),
            underlay: true,
        };
        let glass = |x: f32| crate::Glass {
            order: 0,
            bounds: rect(x),
            backdrop_bounds: rect(x).dilate(ScaledPixels(40.)),
            content_mask: ContentMask { bounds: rect(x) },
            shapes: Default::default(),
            shape_count: 1,
            corner_smoothing: 2.,
            material: crate::GlassMaterial::default(),
            shadow: None,
        };
        let mut scene = Scene::default();
        scene.insert_primitive(glass(64.));
        scene.insert_primitive(underlay(0.));
        scene.insert_primitive(underlay(64.));
        scene.insert_primitive(underlay(128.));
        scene.insert_primitive(glass(0.));
        scene.insert_primitive(glass(192.));
        scene.finish();
        let mut drawn: Vec<_> = scene
            .surfaces
            .iter()
            .map(|surface| (surface.bounds.origin.x.0, surface.image_buffer.is_some()))
            .collect();
        drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(drawn, [(0., true), (64., false), (128., false)]);
    }

    fn paint(scene: &mut Scene) {
        scene.insert_primitive(quad(0.));
        scene.push_shader_layer(layer_descriptor());
        scene.insert_primitive(quad(4.));
        scene.push_shader_layer(layer_descriptor());
        scene.insert_primitive(quad(8.));
        scene.pop_shader_layer();
        scene.pop_shader_layer();
        scene.insert_primitive(quad(12.));
        scene.finish();
    }

    fn assert_nested(scene: &Scene) {
        assert_eq!(scene.quads.len(), 2);
        assert_eq!(scene.shader_layers.len(), 1);
        let outer = &scene.shader_layers[0].scene;
        assert_eq!(outer.quads.len(), 1);
        assert_eq!(outer.shader_layers.len(), 1);
        assert_eq!(outer.shader_layers[0].scene.quads.len(), 1);
        assert_eq!(&*scene.shader_layers[0].uniforms, &[1, 2, 3]);
    }

    #[test]
    fn shader_layers_capture_what_they_enclose() {
        let mut scene = Scene::default();
        paint(&mut scene);
        assert_nested(&scene);
    }

    #[test]
    fn replayed_shader_layers_capture_the_same_primitives() {
        let mut painted = Scene::default();
        paint(&mut painted);
        let mut replayed = Scene::default();
        replayed.replay(0..painted.len(), &painted);
        replayed.finish();
        assert_nested(&replayed);
    }
}
