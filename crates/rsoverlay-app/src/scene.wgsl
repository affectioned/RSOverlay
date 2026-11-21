// VR Preview scene — SteamVR Display-VR-View aesthetic.
//
// Rendered in two passes using the same vertex_index trick:
//   1. Floor grid: large horizontal quad at y=0, teal chaperone lines.
//   2. Overlay quad: rendered where the wrist-mounted overlay lives,
//      textured from the same OverlayTarget egui draws into.
//
// Group 0 holds the camera; group 1 holds per-quad model matrix + the
// overlay texture + sampler. Grid uses only group 0.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    accent: vec4<f32>,
};

struct QuadUniforms {
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> cam: CameraUniforms;

@group(1) @binding(0) var<uniform> q: QuadUniforms;
@group(1) @binding(1) var overlay_tex: texture_2d<f32>;
@group(1) @binding(2) var overlay_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
};

// ─── Overlay quad ─────────────────────────────────────────────────────

@vertex
fn vs_quad(@builtin(vertex_index) vid: u32) -> VsOut {
    // Unit quad in local XY plane. The model matrix sizes and places it.
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>(-0.5,  0.5),
        vec2<f32>( 0.5, -0.5),
        vec2<f32>( 0.5,  0.5),
    );
    // UVs flipped so the overlay's top-left is the top of the rendered
    // quad.
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );
    let p = positions[vid];
    let world4 = q.model * vec4<f32>(p.x, p.y, 0.0, 1.0);
    var out: VsOut;
    out.pos = cam.view_proj * world4;
    out.uv = uvs[vid];
    out.world = world4.xyz;
    return out;
}

@fragment
fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    let overlay = textureSample(overlay_tex, overlay_samp, in.uv);
    // A soft inner glow in the chosen accent along the quad edges — the
    // same cue SteamVR uses to make overlays feel "present" in-space.
    let dist = min(min(in.uv.x, 1.0 - in.uv.x), min(in.uv.y, 1.0 - in.uv.y));
    let edge = pow(1.0 - clamp(dist * 14.0, 0.0, 1.0), 2.0);
    let glow = cam.accent.rgb * edge * 0.35;
    return vec4<f32>(overlay.rgb + glow, overlay.a);
}

// ─── Chaperone floor grid ────────────────────────────────────────────

@vertex
fn vs_grid(@builtin(vertex_index) vid: u32) -> VsOut {
    // 20m x 20m quad on y=0, centered on the viewer's local origin.
    let s = 10.0;
    var positions = array<vec3<f32>, 6>(
        vec3<f32>(-s, 0.0, -s),
        vec3<f32>( s, 0.0, -s),
        vec3<f32>(-s, 0.0,  s),
        vec3<f32>(-s, 0.0,  s),
        vec3<f32>( s, 0.0, -s),
        vec3<f32>( s, 0.0,  s),
    );
    let p = positions[vid];
    var out: VsOut;
    out.pos = cam.view_proj * vec4<f32>(p, 1.0);
    out.uv = p.xz;
    out.world = p;
    return out;
}

@fragment
fn fs_grid(in: VsOut) -> @location(0) vec4<f32> {
    // Fine lines every 0.25m, bold lines every 1m. Fade with distance
    // from viewer so the grid dissolves into the void, matching
    // SteamVR's chaperone falloff.
    let world_xz = in.uv;
    let d = length(world_xz - cam.eye.xz);
    let fade = 1.0 - clamp(d / 8.0, 0.0, 1.0);

    let cell_fine = fract(world_xz / 0.25) - vec2<f32>(0.5, 0.5);
    let cell_bold = fract(world_xz / 1.0) - vec2<f32>(0.5, 0.5);
    let fine = step(0.48, abs(cell_fine.x)) + step(0.48, abs(cell_fine.y));
    let bold = step(0.47, abs(cell_bold.x)) + step(0.47, abs(cell_bold.y));
    let intensity = clamp(0.25 * fine + 0.75 * bold, 0.0, 1.0);

    let col = cam.accent.rgb * intensity * fade * 0.55;
    return vec4<f32>(col, 1.0);
}
