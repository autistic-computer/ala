use anyhow::{Result, ensure};
use image::RgbaImage;

#[cfg(test)]
use crate::plan_glass;
use crate::{
    Bounds, ContentMask, Corners, GLASS_MAX_SHAPES, Glass, GlassMaterial, GlassShadow, Quad,
    ScaledPixels, Scene, hsla, point, px, size,
};

struct ShadowCase {
    name: &'static str,
    glass: Glass,
    shadow: GlassShadow,
    inside: &'static [(u32, u32)],
    outside: &'static [(u32, u32)],
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(y)),
        size: size(ScaledPixels(width), ScaledPixels(height)),
    }
}

fn body(bounds: &[Bounds<ScaledPixels>], radius: f32, smoothing: f32, merge: f32) -> Glass {
    let mut shapes = [(Bounds::default(), Corners::default()); GLASS_MAX_SHAPES];
    for (shape, bounds) in shapes.iter_mut().zip(bounds) {
        *shape = (*bounds, Corners::all(ScaledPixels(radius)));
    }
    let union = bounds.iter().copied().reduce(|a, b| a.union(&b)).unwrap();
    let material = GlassMaterial::regular()
        .vanished()
        .blur(px(3.))
        .bezel(px(5.))
        .refraction(px(6.))
        .tint(hsla(0.58, 0.2, 0.5, 0.2))
        .merge(px(merge))
        .opacity(0.55);
    Glass {
        order: 0,
        bounds: union.dilate(ScaledPixels(2.)),
        backdrop_bounds: union.dilate(ScaledPixels(material.backdrop_reach().as_f32())),
        content_mask: ContentMask {
            bounds: rect(0., 0., 64., 64.),
        },
        shapes,
        shape_count: bounds.len() as u32,
        corner_smoothing: smoothing,
        material,
        shadow: None,
    }
}

fn caster(bounds: Bounds<ScaledPixels>, radius: f32, smoothing: f32, blur: f32) -> GlassShadow {
    GlassShadow {
        bounds,
        corner_radii: Corners::all(ScaledPixels(radius)),
        blur_radius: ScaledPixels(blur),
        color: hsla(0., 0., 0., 0.8),
        corner_smoothing: smoothing,
    }
}

fn cases() -> Vec<ShadowCase> {
    let circle = body(&[rect(16., 16., 32., 32.)], 16., 2., 0.);
    vec![
        ShadowCase {
            name: "circle",
            glass: circle.clone(),
            shadow: caster(rect(16., 16., 32., 32.), 16., 2., 4.),
            inside: &[(32, 32), (18, 32), (32, 18), (45, 32), (32, 45)],
            outside: &[(18, 18), (32, 50)],
        },
        ShadowCase {
            name: "squircle",
            glass: body(&[rect(16., 16., 32., 32.)], 9., 4., 0.),
            shadow: caster(rect(16., 16., 32., 32.), 9., 4., 4.),
            inside: &[(32, 32), (20, 20), (20, 24), (24, 20), (43, 32)],
            outside: &[(16, 16), (32, 50)],
        },
        ShadowCase {
            name: "negative spread",
            glass: circle.clone(),
            shadow: caster(rect(20., 20., 24., 24.), 12., 2., 6.),
            inside: &[(32, 32), (18, 32), (32, 18), (45, 32), (32, 45)],
            outside: &[(32, 49)],
        },
        ShadowCase {
            name: "positive spread",
            glass: circle.clone(),
            shadow: caster(rect(12., 12., 40., 40.), 20., 2., 4.),
            inside: &[(32, 32), (18, 32), (32, 18), (45, 32), (32, 45)],
            outside: &[(18, 18), (32, 53)],
        },
        ShadowCase {
            name: "offset",
            glass: circle,
            shadow: caster(rect(24., 22., 32., 32.), 16., 2., 4.),
            inside: &[(32, 32), (18, 32), (32, 18), (45, 32), (32, 45)],
            outside: &[(55, 38), (40, 55)],
        },
        ShadowCase {
            name: "merged bridge",
            glass: body(
                &[rect(8., 20., 22., 24.), rect(34., 20., 22., 24.)],
                11.,
                2.,
                16.,
            ),
            shadow: caster(rect(8., 20., 48., 24.), 11., 2., 4.),
            inside: &[(19, 32), (45, 32), (31, 32), (32, 32), (33, 32)],
            outside: &[(32, 17), (19, 46)],
        },
    ]
}

fn scene(case: &ShadowCase, shadow_alpha: Option<f32>, face: bool, layer: bool) -> Scene {
    let mut scene = Scene::default();
    for y in 0..8 {
        for x in 0..8 {
            let bounds = rect(x as f32 * 8., y as f32 * 8., 8., 8.);
            let color = if (x + y) % 2 == 0 {
                hsla(0.58, 0.35, 0.45, 1.)
            } else {
                hsla(0.1, 0.4, 0.65, 1.)
            };
            scene.insert_primitive(Quad {
                bounds,
                content_mask: ContentMask { bounds },
                background: color.into(),
                ..Default::default()
            });
        }
    }
    if layer {
        scene.push_layer(rect(0., 0., 64., 64.));
    }
    if face {
        scene.insert_primitive(case.glass.clone());
    }
    if let Some(alpha) = shadow_alpha {
        let mut shadow = case.shadow;
        shadow.color.a = alpha;
        scene.insert_primitive(case.glass.with_shadow(shadow));
    }
    if layer {
        scene.pop_layer();
    }
    scene.finish();
    scene
}

fn ensure_same_image(expected: &RgbaImage, actual: &RgbaImage, context: &str) -> Result<()> {
    for (x, y, pixel) in expected.enumerate_pixels() {
        let actual = actual.get_pixel(x, y).0;
        ensure!(
            pixel.0.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 1),
            "{context} changed ({x}, {y}): {:?} became {actual:?}",
            pixel.0,
        );
    }
    Ok(())
}

pub fn check_glass_shadow_rendering(
    mut render: impl FnMut(&Scene) -> Result<RgbaImage>,
) -> Result<()> {
    let mut case = cases().remove(0);
    let backdrop = render(&scene(&case, None, false, false))?;
    let expected = render(&scene(&case, Some(0.4), false, false))?;
    ensure!(
        expected.get_pixel(32, 50)[0] + 2 < backdrop.get_pixel(32, 50)[0],
        "a shadow-only scene should draw before any backdrop-reading glass",
    );
    for opacity in [0., 0.25, 1.] {
        case.glass.material.opacity = opacity;
        let actual = render(&scene(&case, Some(0.4), false, false))?;
        ensure_same_image(
            &expected,
            &actual,
            "final shadow alpha must not follow material opacity",
        )?;
    }

    for case in cases() {
        let plain = render(&scene(&case, None, true, false))?;
        let faint = render(&scene(&case, Some(0.2), true, false))?;
        let strong = render(&scene(&case, Some(0.8), true, false))?;
        let transparent = render(&scene(&case, Some(0.), true, false))?;
        ensure_same_image(&plain, &transparent, case.name)?;
        for &(x, y) in case.inside {
            let expected = plain.get_pixel(x, y).0;
            for image in [&faint, &strong] {
                let actual = image.get_pixel(x, y).0;
                ensure!(
                    expected.iter().zip(actual).all(|(a, b)| a.abs_diff(b) <= 1),
                    "{} own shadow changed its interior at ({x}, {y}): {expected:?} became {actual:?}",
                    case.name,
                );
            }
        }
        for &(x, y) in case.outside {
            let brightness = |image: &RgbaImage| {
                image.get_pixel(x, y).0[..3]
                    .iter()
                    .map(|value| u32::from(*value))
                    .sum::<u32>()
            };
            let (plain, faint, strong) =
                (brightness(&plain), brightness(&faint), brightness(&strong));
            ensure!(
                faint + 1 < plain && strong + 6 < faint,
                "{} shadow should darken outside at ({x}, {y}), RGB sums plain={plain}, faint={faint}, strong={strong}",
                case.name,
            );
        }
        let layered = render(&scene(&case, Some(0.8), true, true))?;
        ensure_same_image(
            &strong,
            &layered,
            &format!("{} in a forced layer", case.name),
        )?;
    }

    Ok(())
}

#[test]
fn glass_shadow_planning_does_not_copy_or_blur_the_backdrop() {
    for case in cases() {
        let shadow = case.glass.with_shadow(case.shadow);
        assert!(!shadow.reads_backdrop(), "{}", case.name);
        assert_eq!(shadow.blur_plan(6).levels, 0, "{}", case.name);
        assert_eq!(shadow.shapes, case.glass.shapes, "{}", case.name);
        assert_eq!(shadow.shape_count, case.glass.shape_count, "{}", case.name);
        assert_eq!(
            shadow.corner_smoothing, case.glass.corner_smoothing,
            "{}",
            case.name
        );
        assert_eq!(
            shadow.bounds,
            case.shadow.bounds.dilate(case.shadow.blur_radius * 3.),
            "{}",
            case.name,
        );
        let runs = plan_glass(&[shadow], size(64, 64), None);
        assert_eq!(runs.len(), 1, "{}", case.name);
        let run = &runs[0];
        assert!(run.copies.is_empty(), "{}", case.name);
        assert!(run.passes.is_empty(), "{}", case.name);
        assert_eq!(run.depth, 0, "{}", case.name);
        assert_eq!(run.draws.len(), 1, "{}", case.name);
        assert!(!run.draws[0].0.reads_backdrop(), "{}", case.name);
    }
}

#[test]
fn forced_layer_keeps_glass_faces_before_their_own_shadows() {
    for case in cases() {
        let scene = scene(&case, Some(0.8), true, true);
        assert_eq!(scene.glasses.len(), 2, "{}", case.name);
        assert_eq!(
            scene.glasses[0].order, scene.glasses[1].order,
            "{}",
            case.name
        );
        assert!(scene.glasses[0].shadow.is_none(), "{}", case.name);
        assert!(scene.glasses[1].shadow.is_some(), "{}", case.name);
    }
}
