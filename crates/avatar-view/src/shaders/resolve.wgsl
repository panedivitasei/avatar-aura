// Box-filter MSAA resolve, replacing ResolveSubresource, which reads back blank above about 32k pixels on the Radeon RX Vega M D3D12 driver.

@group(0) @binding(0) var samples: texture_multisampled_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((index << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(index & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(pos.xy);
    let count = i32(textureNumSamples(samples));
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < count; i++) {
        sum += textureLoad(samples, texel, i);
    }
    return sum / f32(count);
}
