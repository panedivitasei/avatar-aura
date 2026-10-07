// Frame-wide uniforms shared by every pipeline.
struct Frame {
    view_proj: mat4x4<f32>,
    // xyz camera position.
    eye: vec4<f32>,
    // x: 1 when the shader must sRGB-encode, yz: viewport size in pixels.
    params: vec4<f32>,
    // Contact shadow: centre x, floor y, centre z, avatar height.
    contact: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = x * 12.92;
    let hi = 1.055 * pow(x, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, x <= vec3<f32>(0.0031308));
}

// Linear colour to the target's encoding.
fn output(c: vec3<f32>) -> vec3<f32> {
    if frame.params.x > 0.5 {
        return linear_to_srgb(c);
    }
    return c;
}
