// Fullscreen-triangle blit of a sampled 2D texture onto the swapchain.
// No vertex buffer: the three vertices are generated from vertex_index.

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
// rgb = night-light colour multiplier. (a = opacity; a desktop window
// can't be translucent, so the blit ignores it.)
@group(0) @binding(2) var<uniform> tint: vec4<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    // Covers the screen with a single oversized triangle.
    var out: VsOut;
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    out.uv = vec2<f32>(x, 1.0 - y);
    out.pos = vec4<f32>(x * 2.0 - 1.0, y * 2.0 - 1.0, 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src_tex, src_samp, in.uv);
    return vec4<f32>(c.rgb * tint.rgb, c.a);
}
