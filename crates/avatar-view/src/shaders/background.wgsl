// CSS `radial-gradient(ellipse at 50% 35%, #fff, #e0e8d9)` behind the transparent three.js canvas.

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let size = max(frame.params.yz, vec2<f32>(1.0));
    // Farthest-corner ellipse keeps the closest-side aspect: radii (0.5w, 0.35h) scaled by sqrt(1 + (0.65 / 0.35)^2).
    let d = (pos.xy - vec2<f32>(0.5, 0.35) * size) / (vec2<f32>(0.5, 0.35) * size);
    let t = clamp(length(d) / sqrt(1.0 + (0.65 / 0.35) * (0.65 / 0.35)), 0.0, 1.0);
    let srgb = mix(vec3<f32>(1.0), vec3<f32>(224.0, 232.0, 217.0) / 255.0, t);
    if frame.params.x > 0.5 {
        return vec4<f32>(srgb, 1.0);
    }
    return vec4<f32>(srgb_to_linear(srgb), 1.0);
}
