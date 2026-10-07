// Skinned avatar mesh with the three.js MeshPhongMaterial lighting of viewport.js (r180 physical light units).

@group(0) @binding(1) var<storage, read> palette: array<mat4x4<f32>>;

struct Material {
    // x: alpha test at 0.5, y: double sided, z: alpha blended.
    flags: vec4<f32>,
};

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var diffuse_map: texture_2d<f32>;
@group(1) @binding(2) var diffuse_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let skin = palette[v.joints.x] * v.weights.x
        + palette[v.joints.y] * v.weights.y
        + palette[v.joints.z] * v.weights.z
        + palette[v.joints.w] * v.weights.w;
    let world = skin * vec4<f32>(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.world = world.xyz;
    out.normal = (skin * vec4<f32>(v.normal, 0.0)).xyz;
    out.uv = v.uv;
    out.color = v.color;
    return out;
}

const PI: f32 = 3.141592653589793;
const SHININESS: f32 = 28.0;

// Specular 0x202020 in linear space.
const SPECULAR: vec3<f32> = vec3<f32>(0.014443844);

fn blinn_phong(l: vec3<f32>, v: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let h = normalize(l + v);
    let dot_nh = clamp(dot(n, h), 0.0, 1.0);
    let dot_vh = clamp(dot(v, h), 0.0, 1.0);
    let fresnel = SPECULAR + (vec3<f32>(1.0) - SPECULAR) * pow(1.0 - dot_vh, 5.0);
    let d = (SHININESS * 0.5 + 1.0) * pow(dot_nh, SHININESS) / PI;
    return fresnel * 0.25 * d;
}

struct Lit {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
};

fn directional(lit: Lit, dir: vec3<f32>, color: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> Lit {
    let l = normalize(dir);
    let irradiance = clamp(dot(n, l), 0.0, 1.0) * color;
    var out = lit;
    out.diffuse += irradiance;
    out.specular += irradiance * blinn_phong(l, v, n);
    return out;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let texel = textureSample(diffuse_map, diffuse_sampler, in.uv);
    if material.flags.x > 0.5 && texel.a < 0.5 {
        discard;
    }
    var n = normalize(in.normal);
    if !front {
        n = -n;
    }
    let v = normalize(frame.eye.xyz - in.world);
    let sky = vec3<f32>(1.0);
    let ground = srgb_to_linear(vec3<f32>(179.0, 175.0, 163.0) / 255.0);
    var lit: Lit;
    lit.diffuse = mix(ground, sky, 0.5 * n.y + 0.5) * 1.35;
    lit.specular = vec3<f32>(0.0);
    lit = directional(lit, vec3<f32>(3.0, 8.0, 5.0), srgb_to_linear(vec3<f32>(1.0, 250.0 / 255.0, 243.0 / 255.0)) * 1.5, n, v);
    lit = directional(lit, vec3<f32>(-3.0, 2.0, 4.0), srgb_to_linear(vec3<f32>(241.0, 246.0, 255.0) / 255.0) * 0.7, n, v);
    lit = directional(lit, vec3<f32>(-1.0, 4.0, -3.0), vec3<f32>(0.6), n, v);
    let rgb = texel.rgb * lit.diffuse / PI + lit.specular;
    var alpha = 1.0;
    if material.flags.z > 0.5 {
        alpha = texel.a;
    }
    return vec4<f32>(output(rgb), alpha);
}
