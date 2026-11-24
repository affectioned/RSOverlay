// VR Preview scene — SteamVR Display-VR-View aesthetic.
//
// Rendered in two passes using the same vertex_index trick:
//   1. Floor grid: large horizontal quad at y=0, teal chaperone lines.
//   2. Panel quads (one draw per panel, far to near) + laser beams,
//      each textured from the OverlayTarget its egui UI draws into.
//
// Group 0 holds the camera; group 1 holds per-quad model matrix + the
// overlay texture + sampler. Grid uses only group 0.

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    accent: vec4<f32>,
};

struct QuadUniforms {
    /// Rotation + translation of the overlay anchor point. Scale is
    /// applied in the shader so we can insert the curvature bend
    /// cleanly in metres space.
    model: mat4x4<f32>,
    /// x = curvature in [0,1], y = width in metres, z = height in
    /// metres, w reserved.
    params: vec4<f32>,
    /// rgb = colour multiplier (beams take the accent), a = opacity.
    tint: vec4<f32>,
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
    // Curvature bend: wrap the quad around a vertical cylinder of
    // radius `width / (curvature * PI)`. c=0 → flat; c=1 → the full
    // width spans 180° around a tight radius so edges tuck behind the
    // center.
    let c = clamp(q.params.x, 0.0, 1.0);
    let width = q.params.y;
    let height = q.params.z;
    let x_m = p.x * width;
    let y_m = p.y * height;
    var local_x = x_m;
    var local_z = 0.0;
    if (c > 1e-4) {
        let radius = width / max(c * 3.14159265, 1e-3);
        let angle = x_m / radius;
        local_x = radius * sin(angle);
        local_z = radius * (1.0 - cos(angle));
    }
    // `model` has no scale baked in, so we pass metres through.
    let world4 = q.model * vec4<f32>(local_x, y_m, local_z, 1.0);
    var out: VsOut;
    out.pos = cam.view_proj * world4;
    out.uv = uvs[vid];
    out.world = world4.xyz;
    return out;
}

@fragment
fn fs_quad(in: VsOut) -> @location(0) vec4<f32> {
    // Panel textures are premultiplied with transparent corners, exactly
    // what the VR compositor receives; tint multiplies colour (beams take
    // the accent) and `a` is overall opacity.
    let c = textureSample(overlay_tex, overlay_samp, in.uv);
    return vec4<f32>(c.rgb * q.tint.rgb, c.a) * q.tint.a;
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
    // Quiet chaperone floor: thin neutral lines (a hint of accent),
    // fine every 0.5 m, bolder every 2 m, dissolving into the void a
    // few metres out so the panels stay the subject.
    let world_xz = in.uv;
    let d = length(world_xz - cam.eye.xz);
    let fade = 1.0 - smoothstep(1.0, 6.0, d);
    let fw = fwidth(world_xz);
    let fine_g = abs(fract(world_xz / 0.5 - 0.5) - 0.5) / (fw / 0.5);
    let bold_g = abs(fract(world_xz / 2.0 - 0.5) - 0.5) / (fw / 2.0);
    let fine = 1.0 - min(min(fine_g.x, fine_g.y), 1.0);
    let bold = 1.0 - min(min(bold_g.x, bold_g.y), 1.0);
    let intensity = max(0.35 * fine, bold);
    let tone = mix(vec3<f32>(0.55, 0.55, 0.6), cam.accent.rgb, 0.25);
    return vec4<f32>(tone * intensity * fade * 0.28, 1.0);
}
