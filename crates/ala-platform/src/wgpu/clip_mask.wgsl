fn clip_corner_distance(outset: vec2<f32>, radius: f32, smoothing: f32) -> f32 {
    if (radius <= 0.0 || outset.x <= 0.0 || outset.y <= 0.0) {
        return -1.0;
    }
    var distance: f32;
    if (smoothing <= 2.001) {
        distance = length(outset);
    } else {
        distance = pow(pow(outset.x, smoothing) + pow(outset.y, smoothing), 1.0 / smoothing);
    }
    return distance - radius;
}

fn rounded_clip_alpha(position: vec2<f32>, rounded: vec4<f32>, r: vec4<f32>, smoothing: f32) -> f32 {
    if (all(r <= vec4<f32>(0.0))) {
        return 1.0;
    }
    let near = position - rounded.xy;
    let far = rounded.xy + rounded.zw - position;
    let pill = min(rounded.z, rounded.w) * 0.5 - 0.01;
    let exponent = select(vec4<f32>(smoothing), vec4<f32>(2.0), r >= vec4<f32>(pill));
    let distance = max(
        max(clip_corner_distance(r.x - near, r.x, exponent.x),
            clip_corner_distance(vec2<f32>(r.y - far.x, r.y - near.y), r.y, exponent.y)),
        max(clip_corner_distance(r.z - far, r.z, exponent.z),
            clip_corner_distance(vec2<f32>(r.w - near.x, r.w - far.y), r.w, exponent.w)));
    return saturate(0.5 - distance);
}
