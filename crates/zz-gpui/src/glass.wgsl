struct GlassShape {
    rect: vec4<f32>,
    radii: vec4<f32>,
}

struct Glass {
    shapes: array<GlassShape, 8>,
    quad: vec4<f32>,
    backdrop: vec4<f32>,
    tint: vec4<f32>,
    optics: vec4<f32>,
    light: vec4<f32>,
    tone: vec4<f32>,
    rim: vec4<f32>,
    shape: vec4<f32>,
    glow: vec4<f32>,
    viewport: vec4<f32>,
    mask_rect: vec4<f32>,
    mask_radii: vec4<f32>,
    mask: vec4<f32>,
    light_color: vec4<f32>,
    edge_color: vec4<f32>,
    shadow_bounds: vec4<f32>,
    shadow_radii: vec4<f32>,
    shadow_color: vec4<f32>,
    shadow: vec4<f32>,
}

struct Blur {
    source: vec4<f32>,
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> glass: Glass;
@group(0) @binding(1) var glass_sampler: sampler;
@group(0) @binding(2) var sharp_backdrop: texture_2d<f32>;
@group(0) @binding(3) var blurred_backdrop: texture_2d<f32>;

@group(0) @binding(4) var<uniform> blur: Blur;
@group(0) @binding(5) var blur_sampler: sampler;
@group(0) @binding(6) var blur_source: texture_2d<f32>;

@vertex
fn vs_glass_blur(@builtin(vertex_index) vertex_id: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((vertex_id << 1u) & 2u), f32(vertex_id & 2u));
    return vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
}

fn blur_tap(position: vec2<f32>) -> vec4<f32> {
    let clamped = clamp(position, blur.source.xy + 0.5, blur.source.zw - 0.5);
    let size = vec2<f32>(textureDimensions(blur_source));
    return textureSampleLevel(blur_source, blur_sampler, clamped / size, 0.0);
}

@fragment
fn fs_glass_down(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let center = position.xy * 2.0;
    let o = blur.params.x;
    var sum = blur_tap(center) * 4.0;
    sum += blur_tap(center + vec2<f32>(-o, -o));
    sum += blur_tap(center + vec2<f32>(o, o));
    sum += blur_tap(center + vec2<f32>(o, -o));
    sum += blur_tap(center + vec2<f32>(-o, o));
    return sum / 8.0;
}

@fragment
fn fs_glass_up(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let center = position.xy * 0.5;
    let o = blur.params.x;
    var sum = blur_tap(center + vec2<f32>(-2.0 * o, 0.0));
    sum += blur_tap(center + vec2<f32>(2.0 * o, 0.0));
    sum += blur_tap(center + vec2<f32>(0.0, -2.0 * o));
    sum += blur_tap(center + vec2<f32>(0.0, 2.0 * o));
    sum += blur_tap(center + vec2<f32>(-o, -o)) * 2.0;
    sum += blur_tap(center + vec2<f32>(o, -o)) * 2.0;
    sum += blur_tap(center + vec2<f32>(-o, o)) * 2.0;
    sum += blur_tap(center + vec2<f32>(o, o)) * 2.0;
    return sum / 12.0;
}

@fragment
fn fs_glass_copy(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(blur_source, vec2<i32>(position.xy) + vec2<i32>(blur.params.yz), 0);
}

@vertex
fn vs_glass(@builtin(vertex_index) vertex_id: u32) -> @builtin(position) vec4<f32> {
    var position: vec2<f32>;
    if (glass.shadow.z > 0.5) {
        position = shadow_ring_vertex(vertex_id);
    } else {
        let unit = vec2<f32>(f32(vertex_id & 1u), 0.5 * f32(vertex_id & 2u));
        position = glass.quad.xy + unit * glass.quad.zw;
    }
    let device = position / glass.viewport.xy * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0);
    return vec4<f32>(device, 0.0, 1.0);
}

fn shadow_ring_vertex(vertex_id: u32) -> vec2<f32> {
    let corner = (vertex_id / 2u) % 4u;
    let unit = vec2<f32>(f32(corner == 1u || corner == 2u), f32(corner >= 2u));
    if ((vertex_id & 1u) == 0u) {
        return glass.quad.xy + unit * glass.quad.zw;
    }
    let shape = glass.shapes[0];
    let inset = max(max(shape.radii.x, shape.radii.y), max(shape.radii.z, shape.radii.w)) + 0.5;
    let low = max(shape.rect.xy + inset, glass.quad.xy);
    let high = min(shape.rect.xy + shape.rect.zw - inset, glass.quad.xy + glass.quad.zw);
    if (any(high <= low)) {
        return glass.quad.xy + 0.5 * glass.quad.zw;
    }
    return mix(low, high, unit);
}

fn rounded_rect(point: vec2<f32>, shape: GlassShape, smoothing: f32, bezel: f32) -> vec3<f32> {
    let half_size = shape.rect.zw * 0.5;
    let center_to_point = point - (shape.rect.xy + half_size);
    var radius: f32;
    if (center_to_point.x < 0.0) {
        radius = select(shape.radii.w, shape.radii.x, center_to_point.y < 0.0);
    } else {
        radius = select(shape.radii.z, shape.radii.y, center_to_point.y < 0.0);
    }
    let half_minor = min(half_size.x, half_size.y);
    radius = min(radius, half_minor);
    let exponent = select(smoothing, 2.0, radius >= half_minor - 0.01);
    let side = select(vec2<f32>(1.0), vec2<f32>(-1.0), center_to_point < vec2<f32>(0.0));
    let corner = abs(center_to_point) - half_size;

    let q = corner + radius;
    let outset = max(q, vec2<f32>(0.0));
    var distance: f32;
    if (outset.x > 0.0 && outset.y > 0.0) {
        if (exponent <= 2.001) {
            distance = length(outset);
        } else {
            distance = pow(pow(outset.x, exponent) + pow(outset.y, exponent), 1.0 / exponent);
        }
    } else {
        distance = max(q.x, q.y);
    }

    let soft = corner + min(max(radius, bezel), half_minor);
    let soft_outset = max(soft, vec2<f32>(0.0));
    var normal: vec2<f32>;
    if (soft_outset.x > 0.0 && soft_outset.y > 0.0) {
        if (exponent <= 2.001) {
            normal = soft_outset;
        } else {
            normal = pow(soft_outset, vec2<f32>(exponent - 1.0));
        }
    } else if (soft.x > soft.y) {
        normal = vec2<f32>(1.0, 0.0);
    } else {
        normal = vec2<f32>(0.0, 1.0);
    }
    return vec3<f32>(distance - radius, normalize(normal) * side);
}

fn glass_field(point: vec2<f32>) -> vec4<f32> {
    let count = u32(glass.shape.x);
    let smoothing = glass.shape.y;
    let merge = glass.shape.z;
    let bezel = glass.optics.x;
    var field = vec4<f32>(
        rounded_rect(point, glass.shapes[0], smoothing, bezel),
        0.5 * min(glass.shapes[0].rect.z, glass.shapes[0].rect.w),
    );
    for (var i = 1u; i < count; i += 1u) {
        let shape = glass.shapes[i];
        let next = rounded_rect(point, shape, smoothing, bezel);
        let k = max(merge * min(0.5 * length(next.yz - field.yz), 1.0), 1e-4);
        let h = clamp(0.5 + 0.5 * (next.x - field.x) / k, 0.0, 1.0);
        let distance = mix(next.x, field.x, h) - k * h * (1.0 - h);
        let gradient = mix(next.yz, field.yz, h);
        let half_minor = mix(0.5 * min(shape.rect.z, shape.rect.w), field.w, h);
        field = vec4<f32>(distance, gradient, half_minor);
    }
    return field;
}

fn shadow_gaussian(x: f32, sigma: f32) -> f32 {
    return exp(-(x * x) / (2.0 * sigma * sigma)) / (sqrt(6.283185307179586) * sigma);
}

fn shadow_erf(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn shadow_erf4(v: vec4<f32>) -> vec4<f32> {
    let s = sign(v);
    let a = abs(v);
    let r1 = 1.0 + (0.278393 + (0.230389 + (0.000972 + 0.078108 * a) * a) * a) * a;
    let r2 = r1 * r1;
    return s - s / (r2 * r2);
}

fn shadow_blur_along_x(x: f32, y: f32, sigma: f32, corner: f32, half_size: vec2<f32>, smoothing: f32) -> f32 {
    let delta = min(half_size.y - corner - abs(y), 0.0);
    var reach = sqrt(max(0.0, corner * corner - delta * delta));
    if (corner > 0.0 && smoothing > 2.001 && delta < 0.0) {
        reach = corner * pow(max(0.0, 1.0 - pow(abs(delta) / corner, smoothing)), 1.0 / smoothing);
    }
    let curved = half_size.x - corner + reach;
    let integral = 0.5 + 0.5 * shadow_erf((x + vec2<f32>(-curved, curved)) * (sqrt(0.5) / sigma));
    return integral.y - integral.x;
}

fn glass_shadow(position: vec2<f32>) -> vec4<f32> {
    if (any(glass.shadow_bounds.zw <= vec2<f32>(0.0)) || covered_by_body(position)) {
        return vec4<f32>(0.0);
    }
    let caster = GlassShape(glass.shadow_bounds, glass.shadow_radii);
    let sigma = glass.shadow.x;
    let smoothing = glass.shadow.y;
    let half_size = caster.rect.zw * 0.5;
    let point = position - (caster.rect.xy + half_size);
    var radius: f32;
    if (point.x < 0.0) {
        radius = select(caster.radii.w, caster.radii.x, point.y < 0.0);
    } else {
        radius = select(caster.radii.z, caster.radii.y, point.y < 0.0);
    }
    let largest_corner = max(max(caster.radii.x, caster.radii.y), max(caster.radii.z, caster.radii.w));
    var alpha: f32;
    if (sigma == 0.0) {
        alpha = saturate(0.5 - rounded_rect(position, caster, smoothing, 0.0).x);
    } else if (largest_corner <= 0.25 * sigma) {
        let edges = (vec4<f32>(point, point) + vec4<f32>(half_size, -half_size)) * (sqrt(0.5) / sigma);
        let integral = shadow_erf4(edges);
        alpha = 0.25 * (integral.x - integral.z) * (integral.y - integral.w);
    } else {
        let exponent = select(smoothing, 2.0, radius >= min(half_size.x, half_size.y) - 0.01);
        let low = point.y - half_size.y;
        let high = point.y + half_size.y;
        let start = clamp(-3.0 * sigma, low, high);
        let end = clamp(3.0 * sigma, low, high);
        let step = (end - start) / 4.0;
        var y = start + step * 0.5;
        alpha = 0.0;
        for (var i = 0; i < 4; i += 1) {
            let blur = shadow_blur_along_x(point.x, point.y - y, sigma, radius, half_size, exponent);
            alpha += blur * shadow_gaussian(y, sigma) * step;
            y += step;
        }
    }
    alpha = saturate(alpha) * saturate(0.5 + glass_field(position).x) * saturate(glass.shadow_color.a);
    if (glass.mask.y > 0.5) {
        let window = GlassShape(glass.mask_rect, glass.mask_radii);
        alpha *= saturate(0.5 - rounded_rect(position, window, glass.mask.x, 0.0).x);
    }
    return vec4<f32>(glass.shadow_color.rgb * alpha, alpha);
}

fn under(frame_position: vec2<f32>) -> vec4<f32> {
    let position = frame_position - glass.viewport.zw;
    let clamped = clamp(position, glass.backdrop.xy + 0.5, glass.backdrop.zw - 0.5);
    let size = vec2<f32>(textureDimensions(sharp_backdrop));
    return textureSampleLevel(sharp_backdrop, glass_sampler, clamped / size, 0.0);
}

fn backdrop(frame_position: vec2<f32>) -> vec4<f32> {
    let level = glass.optics.w;
    if (level < 0.5) {
        return under(frame_position);
    }
    let position = frame_position - glass.viewport.zw;
    let scale = exp2(-level);
    let low = floor(glass.backdrop.xy * scale) + 0.5;
    let high = ceil(glass.backdrop.zw * scale) - 0.5;
    let clamped = clamp(position * scale, low, high);
    let size = vec2<f32>(textureDimensions(blurred_backdrop));
    return textureSampleLevel(blurred_backdrop, glass_sampler, clamped / size, 0.0);
}

fn grain(position: vec2<f32>) -> f32 {
    var state = u32(position.x) * 747796405u + u32(position.y) * 2891336453u;
    state = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    state = (state >> 22u) ^ state;
    return f32(state) / 4294967295.0 - 0.5;
}

fn deep_inside(point: vec2<f32>, shape: GlassShape, depth: f32) -> bool {
    let half_size = shape.rect.zw * 0.5;
    let inset = half_size - abs(point - (shape.rect.xy + half_size));
    let radius = max(max(shape.radii.x, shape.radii.y), max(shape.radii.z, shape.radii.w));
    return min(inset.x, inset.y) >= depth && max(inset.x, inset.y) >= max(radius, depth);
}

fn covered_by_body(point: vec2<f32>) -> bool {
    let count = u32(glass.shape.x);
    for (var i = 0u; i < count; i += 1u) {
        if (deep_inside(point, glass.shapes[i], 0.5)) {
            return true;
        }
    }
    return false;
}

fn on_face(point: vec2<f32>) -> bool {
    let depth = glass.shape.w;
    if (depth <= 0.0) {
        return false;
    }
    if (glass.mask.y > 0.5 && !deep_inside(point, GlassShape(glass.mask_rect, glass.mask_radii), 1.5)) {
        return false;
    }
    if (deep_inside(point, glass.shapes[0], depth)) {
        return true;
    }
    let count = u32(glass.shape.x);
    for (var i = 1u; i < count; i += 1u) {
        if (deep_inside(point, glass.shapes[i], depth)) {
            return true;
        }
    }
    return false;
}

fn toned(color: vec4<f32>) -> vec3<f32> {
    var rgb = color.rgb / max(color.a, 1e-4);
    let luminance = dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    rgb = mix(vec3<f32>(luminance), rgb, glass.tone.x);
    rgb = (rgb - 0.5) * glass.tone.z + 0.5 + glass.tone.y;
    return mix(rgb, glass.tint.rgb, glass.tint.a);
}

fn finish(position: vec2<f32>, color: vec4<f32>, lit: vec3<f32>, coverage: f32, contour: f32) -> vec4<f32> {
    var rgb = lit;
    if (glass.glow.w > 0.0) {
        let reach = 1.0 - saturate(length(position - glass.glow.xy) / max(glass.glow.z, 1.0));
        rgb = rgb + glass.glow.w * reach * reach;
    }

    if (glass.tone.w > 0.0) {
        rgb = rgb + grain(position) * glass.tone.w;
    }
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));

    let strength = coverage * glass.rim.w;
    let face = color.a + (1.0 - color.a) * glass.tint.a;
    let edge = contour * glass.rim.w;
    if (glass.tint.a < 1.0) {
        let held = under(position);
        let outlined = vec4<f32>(glass.edge_color.rgb * edge, edge) + held * (1.0 - edge);
        return mix(outlined, vec4<f32>(rgb * face, face), strength);
    }
    let body = vec4<f32>(rgb * face, face) * strength;
    let alpha = body.a + (1.0 - body.a) * edge;
    return vec4<f32>(body.rgb + glass.edge_color.rgb * (alpha - body.a), alpha);
}

@fragment
fn fs_glass(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let position = frag.xy;
    if (glass.shadow.z > 0.5) {
        return glass_shadow(position);
    }
    let rim_only = glass.edge_color.w > 0.5;
    if (on_face(position)) {
        if (rim_only) {
            return vec4<f32>(0.0);
        }
        var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        if (glass.tint.a < 1.0) {
            color = backdrop(position);
        }
        return finish(position, color, toned(color), 1.0, 0.0);
    }
    let field = glass_field(position);
    let distance = field.x;
    let normal = field.yz / max(length(field.yz), 1e-5);
    let light = glass.light.xy;
    let incidence = dot(normal, light);
    let facing = abs(incidence);
    let toward = max(incidence, 0.0);
    let away = max(-incidence, 0.0);
    let directionality = glass.mask.z;

    let contour_width = glass.rim.z;
    var contour = 0.0;
    if (contour_width > 0.0) {
        let lit = mix(mix(0.35, 1.0, facing), mix(0.2, 1.0, away), directionality);
        contour = glass.rim.y * (1.0 - saturate(distance / contour_width))
            * saturate(distance + 0.5)
            * mix(lit, 1.0, glass.mask.w);
    }
    var clip = 1.0;
    if (glass.mask.y > 0.5) {
        let window = GlassShape(glass.mask_rect, glass.mask_radii);
        clip = saturate(0.5 - rounded_rect(position, window, glass.mask.x, 0.0).x);
    }
    contour = contour * clip;
    let coverage = saturate(0.5 - distance) * clip;
    if (coverage <= 0.0) {
        return finish(position, vec4<f32>(0.0, 0.0, 0.0, 1.0), vec3<f32>(0.0), 0.0, contour);
    }

    let depth = max(-distance, 0.0);
    let bezel = clamp(glass.optics.x, 1e-3, max(field.w * 0.5, 1e-3));
    let edge = 1.0 - saturate(depth / bezel);
    let refraction = clamp(glass.optics.y, -field.w, field.w);
    let pull = refraction * (1.0 - sqrt(max(1.0 - edge * edge, 0.0)));
    let offset = -normal * pull;

    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    if (glass.tint.a < 1.0) {
        let dispersion = glass.optics.z * pull;
        if (dispersion > 0.25) {
            let red = backdrop(position + offset - normal * dispersion * 0.5);
            let green = backdrop(position + offset);
            let blue = backdrop(position + offset + normal * dispersion * 0.5);
            color = vec4<f32>(red.r, green.g, blue.b, green.a);
        } else {
            color = backdrop(position + offset);
        }
    }

    var glint = 0.0;
    var glow = 0.0;
    if (edge > 0.0) {
        let glint_width = max(glass.light.w, 1e-3);
        let line = saturate(1.0 - depth / glint_width) + 0.2 * saturate(1.0 - depth / (glint_width * 4.0));
        let lobe = facing * facing;
        let directional_lobe = 0.06 + 0.94 * toward * toward + 0.12 * pow(away, 4.0);
        glint = glass.light.z * line * mix(mix(0.15, 1.0, lobe), directional_lobe, directionality);
        glow = glass.rim.x * edge * edge * edge * mix(1.0, 0.25 + 0.75 * toward, directionality);
    }

    if (rim_only) {
        let veil = glow * glass.light_color.a;
        let light = glass.light_color.rgb * (glint * (1.0 - veil) + glow);
        return vec4<f32>(light, veil) * (coverage * glass.rim.w);
    }

    var rgb = toned(color) + glass.light_color.rgb * glint;
    rgb = rgb * (1.0 - glow * glass.light_color.a) + glass.light_color.rgb * glow;

    return finish(position, color, rgb, coverage, contour);
}
