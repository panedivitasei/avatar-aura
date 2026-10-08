// Free-pose overlay: flat-coloured gizmo handles and the bones.js marker sprite.
// The marker is the 128px canvas plus shape: arms 36px wide spanning 108px, 8px corner arcs, 4px #f5ffe9 stroke.

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) kind: f32,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) kind: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.kind = in.kind;
    return out;
}

fn round_box(p: vec2<f32>, half: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if in.kind < 0.5 {
        return vec4<f32>(output(in.color.rgb), in.color.a);
    }
    // Canvas units over 64: arm half-width 18, half-length 54, corner radius 8, stroke half-width 2.
    let long = 54.0 / 64.0;
    let short = 18.0 / 64.0;
    let radius = 8.0 / 64.0;
    let stroke = 2.0 / 64.0;
    let d = min(round_box(in.uv, vec2<f32>(long, short), radius), round_box(in.uv, vec2<f32>(short, long), radius));
    let aa = max(fwidth(d), 1e-4);
    let coverage = 1.0 - smoothstep(stroke - aa, stroke + aa, d);
    let edge = 1.0 - smoothstep(stroke - aa, stroke + aa, abs(d));
    let rim = vec3<f32>(245.0, 255.0, 233.0) / 255.0;
    let rgb = mix(in.color.rgb, rim, edge);
    return vec4<f32>(output(rgb), coverage * in.color.a);
}
