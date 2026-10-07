// Radial contact blob under the feet: 128px canvas gradient #39432c at alpha 0x55 / 0x20 / 0 for stops 0 / 0.45 / 1.

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[index];
    let h = frame.contact.w;
    let world = vec3<f32>(frame.contact.x + c.x * 0.4 * h, frame.contact.y, frame.contact.z - c.y * 0.3 * h);
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.local = c;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let r = length(in.local);
    var a = 0.0;
    if r < 0.45 {
        a = mix(85.0, 32.0, r / 0.45) / 255.0;
    } else if r < 1.0 {
        a = mix(32.0, 0.0, (r - 0.45) / 0.55) / 255.0;
    }
    // The canvas texture carries no colour space, so three.js treats its bytes as linear.
    let rgb = vec3<f32>(57.0, 67.0, 44.0) / 255.0;
    return vec4<f32>(output(rgb), a);
}
