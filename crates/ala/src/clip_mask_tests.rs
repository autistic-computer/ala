use std::{borrow::Cow, rc::Rc, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use image::RgbaImage;

use crate::{
    AtlasKey, AtlasTile, Bounds, ClipMask, ContentMask, Corners, CustomShader, DevicePixels,
    GLASS_MAX_SHAPES, Glass, GlassMaterial, Hsla, ImageId, MonochromeSprite, Path, PlatformAtlas,
    PolychromeSprite, Primitive, Quad, RenderImageParams, RenderSvgParams, ScaledPixels, Scene,
    ShaderLayer, Shadow, Size, TransformationMatrix, Underline, hsla, point, px, size,
};

const SIDE: f32 = 64.;
const RADIUS: f32 = 24.;

fn full() -> Bounds<ScaledPixels> {
    Bounds::new(point(px(0.), px(0.)), size(px(SIDE), px(SIDE))).scale(1.)
}

fn rounded() -> ClipMask<ScaledPixels> {
    ClipMask::rounded(full(), Corners::all(ScaledPixels(RADIUS)), 2.)
}

fn red() -> Hsla {
    hsla(0., 1., 0.5, 1.)
}

fn tiles(atlas: &dyn PlatformAtlas) -> Result<(AtlasTile, AtlasTile)> {
    let extent = Size {
        width: DevicePixels(SIDE as i32),
        height: DevicePixels(SIDE as i32),
    };
    let pixels = (SIDE * SIDE) as usize;
    let mono = atlas
        .get_or_insert_with(
            AtlasKey::Svg(RenderSvgParams {
                path: "rounded clip".into(),
                size: extent,
            }),
            &mut || Ok(Some((extent, Cow::Owned(vec![255; pixels])))),
        )?
        .context("no monochrome tile")?;
    let poly = atlas
        .get_or_insert_with(
            AtlasKey::Image(RenderImageParams {
                image_id: ImageId(usize::MAX),
                frame_index: 0,
            }),
            &mut || Ok(Some((extent, Cow::Owned([0, 0, 255, 255].repeat(pixels))))),
        )?
        .context("no polychrome tile")?;
    Ok((mono, poly))
}

const RED_SHADER: &str = "@fragment
fn main(@location(0) position: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}";

fn glass() -> Glass {
    let mut shapes: [(Bounds<ScaledPixels>, Corners<ScaledPixels>); GLASS_MAX_SHAPES] =
        Default::default();
    shapes[0] = (full(), Corners::default());
    Glass {
        order: 0,
        bounds: full(),
        backdrop_bounds: full(),
        content_mask: rounded(),
        shapes,
        shape_count: 1,
        corner_smoothing: 2.,
        material: GlassMaterial::regular().tint(red()),
        shadow: None,
    }
}

fn primitives(mono: AtlasTile, poly: AtlasTile) -> Vec<(&'static str, Primitive)> {
    let content_mask = rounded();
    let mut path = Path::new(point(px(0.), px(0.)));
    path.line_to(point(px(SIDE), px(0.)));
    path.line_to(point(px(SIDE), px(SIDE)));
    path.line_to(point(px(0.), px(SIDE)));
    path.line_to(point(px(0.), px(0.)));
    let mut path = path.scale(1.);
    path.content_mask = content_mask;
    path.color = red().into();
    vec![
        (
            "quad",
            Quad {
                bounds: full(),
                content_mask,
                background: red().into(),
                ..Default::default()
            }
            .into(),
        ),
        (
            "shadow",
            Shadow {
                order: 0,
                blur_radius: ScaledPixels(0.),
                bounds: full(),
                corner_radii: Corners::default(),
                content_mask,
                color: red(),
                element_bounds: full(),
                element_corner_radii: Corners::default(),
                inset: 0,
                corner_smoothing: 2.,
            }
            .into(),
        ),
        (
            "underline",
            Underline {
                order: 0,
                pad: 0,
                bounds: full(),
                content_mask,
                color: red(),
                thickness: ScaledPixels(SIDE),
                wavy: false.into(),
            }
            .into(),
        ),
        ("path", path.into()),
        (
            "monochrome sprite",
            MonochromeSprite {
                order: 0,
                pad: 0,
                bounds: full(),
                content_mask,
                color: red(),
                tile: mono,
                transformation: TransformationMatrix::unit(),
            }
            .into(),
        ),
        (
            "polychrome sprite",
            PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: false.into(),
                opacity: 1.,
                bounds: full(),
                content_mask,
                corner_radii: Corners::default(),
                corner_smoothing: 2.,
                pad2: 0,
                tile: poly,
            }
            .into(),
        ),
        (
            "shader layer",
            ShaderLayer {
                order: 0,
                bounds: full(),
                content_mask,
                shader: CustomShader::new(RED_SHADER),
                uniforms: Arc::from([]),
                scene: Rc::new(Scene::default()),
            }
            .into(),
        ),
        ("glass", glass().into()),
    ]
}

pub fn check_rounded_clip_rendering(
    atlas: &dyn PlatformAtlas,
    mut render: impl FnMut(&Scene) -> Result<RgbaImage>,
) -> Result<()> {
    let (mono, poly) = tiles(atlas)?;
    for (name, primitive) in primitives(mono, poly) {
        let mut scene = Scene::default();
        scene.insert_primitive(Quad {
            bounds: full(),
            content_mask: ContentMask { bounds: full() }.into(),
            background: hsla(0., 0., 0., 1.).into(),
            ..Default::default()
        });
        scene.insert_primitive(primitive);
        scene.finish();
        let image = render(&scene)?;
        for (x, y) in [(1, 1), (62, 1), (62, 62), (1, 62), (4, 6)] {
            ensure!(
                image.get_pixel(x, y).0[..3] == [0, 0, 0],
                "{name} drew past the rounded corner at ({x}, {y}): {:?}",
                image.get_pixel(x, y).0
            );
        }
        for (x, y) in [(32, 32), (1, 32), (32, 1), (62, 32), (10, 10)] {
            ensure!(
                image.get_pixel(x, y).0[..3] != [0, 0, 0],
                "{name} was clipped inside the rounded mask at ({x}, {y})"
            );
        }
        if name == "glass" {
            continue;
        }
        let edge = image.get_pixel(7, 6).0;
        ensure!(
            edge.iter().take(3).any(|channel| *channel > 0)
                && edge.iter().take(3).all(|channel| *channel < 250),
            "{name} has no antialiased edge at (7, 6): {edge:?}"
        );
    }
    Ok(())
}
