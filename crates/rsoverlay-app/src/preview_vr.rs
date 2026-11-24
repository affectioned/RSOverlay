//! VR Preview mode — SteamVR Display-VR-View aesthetic.
//!
//! What this is for: ironing out how the overlay reads from HMD POV
//! without putting the headset on for every change. The overlay quad is
//! rendered in a perspective-correct 3D scene, positioned where a
//! wrist-mounted overlay would be (default: left-wrist, ~25cm from the
//! head, tilted up for a glance). Mouse drag looks around, scroll
//! dollies the head forward/back, a chaperone-style floor grid gives
//! spatial grounding. The overlay content is rendered through the
//! exact same `EguiPanel` → `OverlayTarget` pipeline the desktop
//! preview and VR mode use, so visual parity is maintained.

use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::NotifyChannel;
use rsoverlay_render::{GpuCtx, OverlayTarget};
use rsoverlay_ui::{
    views::{Anchor, DevView, Placement},
    EguiPanel,
};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use wgpu::util::DeviceExt;
use winit::{
    event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

// Dashboard aspect is landscape — mirror on the right, card column on
// the left. Pick an overlay texture that matches so the egui UI fills
// the quad instead of leaving a big black band.
const OVERLAY_TEX_W: u32 = 2048;
const OVERLAY_TEX_H: u32 = 1280;
const INITIAL_WIN_W: u32 = 1280;
const INITIAL_WIN_H: u32 = 800;

pub fn run(
    _notify: Option<NotifyChannel>,
    test: crate::preview::TestHooks,
    app_config: rsoverlay_core::Config,
    config_path: std::path::PathBuf,
) -> Result<()> {
    let event_loop = EventLoop::new().context("event loop")?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RSOverlay — VR Preview")
            .with_inner_size(winit::dpi::LogicalSize::new(INITIAL_WIN_W, INITIAL_WIN_H))
            .with_min_inner_size(winit::dpi::LogicalSize::new(800u32, 600u32))
            .build(&event_loop)
            .context("creating VR preview window")?,
    );

    let gpu = pollster::block_on(GpuCtx::new(RuntimeMode::Preview))?;

    let surface = gpu
        .instance
        .create_surface(window.clone())
        .context("creating wgpu surface")?;
    let size = window.inner_size();
    let caps = surface.get_capabilities(&gpu.adapter);
    let surface_format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);

    // Overlay texture lives at VR resolution regardless of window — it's
    // what would be submitted to OpenVR for real.
    let overlay_target = OverlayTarget::new(&gpu, OVERLAY_TEX_W, OVERLAY_TEX_H);
    let mut egui_panel = EguiPanel::new(
        &gpu.device,
        overlay_target.format,
        overlay_target.width,
        overlay_target.height,
    );
    let mut egui_winit = egui_winit::State::new(
        egui_panel.ctx.clone(),
        egui::ViewportId::ROOT,
        &*window,
        None,
        None,
    );
    let mut dev_view = DevView::default();
    crate::preview::seed_dev_view(&mut dev_view, &app_config, &test);
    let mut persister =
        crate::preview::ConfigPersister::new(config_path.clone(), app_config.clone());
    let config_base = app_config.clone();

    let mut scene = SceneRenderer::new(&gpu, surface_format, &overlay_target)?;
    let mut camera = OrbitCamera::default();
    let mut input = InputState::default();
    let mut last_tick = Instant::now();
    let mut frame_counter: u64 = 0;
    let screenshot_path = test.screenshot.clone();
    let exit_after = test.exit_after;

    info!("VR preview window up — drag to look, scroll to dolly, Esc to exit");

    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run(move |event, elwt| match event {
        Event::WindowEvent { event, window_id } if window_id == window.id() => {
            // Egui never gets mouse drags while the user is looking
            // around, but typing etc. still passes through.
            let egui_response = egui_winit.on_window_event(&window, &event);
            match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed
                        && event.logical_key == Key::Named(NamedKey::Escape)
                    {
                        elwt.exit();
                    }
                }
                WindowEvent::Resized(new_size) => {
                    config.width = new_size.width.max(1);
                    config.height = new_size.height.max(1);
                    surface.configure(&gpu.device, &config);
                }
                WindowEvent::MouseInput { state, button, .. } => {
                    if button == MouseButton::Left {
                        input.dragging = state == ElementState::Pressed;
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let new = (position.x as f32, position.y as f32);
                    if input.dragging && !egui_response.consumed {
                        let (dx, dy) = (new.0 - input.last.0, new.1 - input.last.1);
                        camera.orbit(dx, dy);
                    }
                    input.last = new;
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 50.0,
                    };
                    camera.dolly(dy);
                }
                WindowEvent::RedrawRequested => {
                    // 1. Render egui UI into overlay_target (unchanged
                    // from the desktop preview path).
                    let raw_input = egui_winit.take_egui_input(&window);
                    egui_panel.render(
                        &gpu.device,
                        &gpu.queue,
                        &overlay_target.view,
                        raw_input,
                        |ctx| dev_view.ui(ctx),
                    );

                    // 2. Render the 3D VR preview scene into the window.
                    let frame = match surface.get_current_texture() {
                        Ok(f) => f,
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            surface.configure(&gpu.device, &config);
                            return;
                        }
                        Err(e) => {
                            warn!("surface error: {e:?}");
                            return;
                        }
                    };
                    let view = frame
                        .texture
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    let aspect = config.width as f32 / config.height.max(1) as f32;
                    scene.render(
                        &gpu,
                        &camera,
                        &dev_view.placement,
                        dev_view.accent,
                        dev_view.overlay_tint(),
                        aspect,
                        &view,
                    );

                    frame_counter = frame_counter.wrapping_add(1);
                    if let Some(trigger) = exit_after {
                        if frame_counter == trigger {
                            if let Some(path) = &screenshot_path {
                                match crate::preview::save_surface_as_png(
                                    &gpu,
                                    &frame.texture,
                                    config.width,
                                    config.height,
                                    path,
                                ) {
                                    Ok(n) => info!(
                                        "wrote screenshot {} ({} bytes)",
                                        path.display(),
                                        n
                                    ),
                                    Err(e) => warn!("screenshot failed: {e:#}"),
                                }
                            }
                            info!("--exit-after {trigger} reached; clean exit");
                            elwt.exit();
                        }
                    }

                    frame.present();

                    if dev_view.reset_requested {
                        let base = rsoverlay_core::Config::default();
                        crate::preview::seed_dev_view(
                            &mut dev_view,
                            &base,
                            &crate::preview::TestHooks::default(),
                        );
                        dev_view.reset_requested = false;
                    }
                    if let Some(cfg) = persister.poll_external_change() {
                        crate::preview::seed_dev_view(
                            &mut dev_view,
                            &cfg,
                            &crate::preview::TestHooks::default(),
                        );
                    }
                    let current = crate::preview::config_from_dev_view(&dev_view, &config_base);
                    persister.tick(&current);

                    let now = Instant::now();
                    dev_view.frame = dev_view.frame.wrapping_add(1);
                    dev_view.last_fps = 1.0 / now.duration_since(last_tick).as_secs_f32().max(1e-5);
                    last_tick = now;
                }
                _ => {}
            }
        }
        Event::AboutToWait => window.request_redraw(),
        _ => {}
    })?;
    Ok(())
}

#[derive(Default)]
struct InputState {
    dragging: bool,
    last: (f32, f32),
}

/// First-person look + dolly camera. Simulates a stationary standing VR
/// user turning their head. Head height is 1.6m above the floor plane.
struct OrbitCamera {
    yaw: f32,
    pitch: f32,
    head_height: f32,
    // How far back from the overlay the "head" starts. Dolly shifts the
    // head forward/back along its facing vector.
    dolly: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        // Natural wrist-glance: yaw slightly toward the left wrist,
        // pitch tilted ~25° down. With the overlay auto-facing the
        // viewer, these need less lean than a fixed-tilt panel would.
        Self {
            yaw: -0.18,
            pitch: -0.45,
            head_height: 1.6,
            dolly: 0.05,
        }
    }
}

impl OrbitCamera {
    fn orbit(&mut self, dx: f32, dy: f32) {
        // Typical SteamVR mouse-look sensitivity: ~0.3° per screen pixel.
        self.yaw -= dx.to_radians() * 0.3;
        self.pitch -= dy.to_radians() * 0.3;
        self.pitch = self.pitch.clamp(-1.4, 1.4);
    }

    fn dolly(&mut self, amount: f32) {
        self.dolly = (self.dolly - amount * 0.1).clamp(-0.6, 1.2);
    }

    fn eye(&self) -> Vec3 {
        let forward = self.forward();
        Vec3::new(0.0, self.head_height, 0.0) + forward * self.dolly
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            -self.pitch.cos() * self.yaw.cos(),
        )
    }

    fn view(&self) -> Mat4 {
        let eye = self.eye();
        Mat4::look_at_rh(eye, eye + self.forward(), Vec3::Y)
    }

    fn proj(&self, aspect: f32) -> Mat4 {
        // ~100° horizontal is typical of modern HMDs. Convert to the
        // equivalent vertical fov for glam's perspective_rh.
        let hfov_rad = 100f32.to_radians();
        let vfov = 2.0 * ((hfov_rad * 0.5).tan() / aspect).atan();
        Mat4::perspective_rh(vfov, aspect, 0.05, 100.0)
    }
}

/// Where the overlay quad lives relative to the viewer. Default: left
/// wrist area, raised and tilted toward the face. Returns a model
/// matrix that scales a unit quad into the overlay's physical size.
const HEAD_DEFAULT: Vec3 = Vec3::new(0.0, 1.6, 0.0);

/// Compute the overlay's anchor matrix: rotation + translation only,
/// no scale. The shader applies width/height in metres so curvature
/// math can bend the quad in physical units before this transform.
fn overlay_model(placement: &Placement, camera: &OrbitCamera) -> Mat4 {
    match placement.anchor {
        Anchor::Hmd => {
            let eye = camera.eye();
            let forward = camera.forward();
            let right = forward.cross(Vec3::Y).normalize();
            let up = right.cross(forward).normalize();
            let pos = eye
                + forward * (-placement.offset_z)
                + right * placement.offset_x
                + up * (placement.offset_y - camera.head_height);
            let look_dir = (eye - pos).normalize();
            let quat = Quat::from_rotation_arc(Vec3::Z, look_dir);
            Mat4::from_translation(pos) * Mat4::from_quat(quat)
        }
        Anchor::Wrist | Anchor::World => {
            let pos = Vec3::new(placement.offset_x, placement.offset_y, placement.offset_z);
            let forward = (HEAD_DEFAULT - pos).normalize();
            let quat = Quat::from_rotation_arc(Vec3::Z, forward);
            Mat4::from_translation(pos) * Mat4::from_quat(quat)
        }
    }
}

struct SceneRenderer {
    camera_buf: wgpu::Buffer,
    #[allow(dead_code)]
    camera_bgl: wgpu::BindGroupLayout,
    camera_bg: wgpu::BindGroup,

    quad_buf: wgpu::Buffer,
    quad_bg: wgpu::BindGroup,
    quad_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
}

impl SceneRenderer {
    fn new(
        gpu: &GpuCtx,
        surface_format: wgpu::TextureFormat,
        overlay: &OverlayTarget,
    ) -> Result<Self> {
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("vr-scene-shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
            });

        let camera_buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vr-camera"),
            size: std::mem::size_of::<CameraGpu>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let camera_bgl = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vr-camera-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vr-camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let quad_buf = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("vr-quad-model"),
                contents: bytemuck::bytes_of(&QuadGpu {
                    model: Mat4::IDENTITY.to_cols_array_2d(),
                    params: [0.0, 0.36, 0.225, 0.0],
                    tint: [1.0; 4],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vr-overlay-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let quad_bgl = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("vr-quad-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let quad_bg = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vr-quad-bg"),
            layout: &quad_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: quad_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&overlay.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let quad_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("vr-quad-pl"),
                    bind_group_layouts: &[&camera_bgl, &quad_bgl],
                    push_constant_ranges: &[],
                });
        let grid_pipeline_layout =
            gpu.device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("vr-grid-pl"),
                    bind_group_layouts: &[&camera_bgl],
                    push_constant_ranges: &[],
                });

        let quad_pipeline =
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("vr-quad-pipeline"),
                    layout: Some(&quad_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: "vs_quad",
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs_quad",
                        targets: &[Some(wgpu::ColorTargetState {
                            format: surface_format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                });
        let grid_pipeline =
            gpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("vr-grid-pipeline"),
                    layout: Some(&grid_pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: "vs_grid",
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: "fs_grid",
                        targets: &[Some(wgpu::ColorTargetState {
                            format: surface_format,
                            blend: Some(wgpu::BlendState::REPLACE),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState {
                        cull_mode: None,
                        ..Default::default()
                    },
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview: None,
                });

        Ok(Self {
            camera_buf,
            camera_bgl,
            camera_bg,
            quad_buf,
            quad_bg,
            quad_pipeline,
            grid_pipeline,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        &mut self,
        gpu: &GpuCtx,
        camera: &OrbitCamera,
        placement: &Placement,
        accent: egui::Color32,
        tint: [f32; 4],
        aspect: f32,
        view: &wgpu::TextureView,
    ) {
        let vp = camera.proj(aspect) * camera.view();
        let eye = camera.eye();
        let accent_v = [
            accent.r() as f32 / 255.0,
            accent.g() as f32 / 255.0,
            accent.b() as f32 / 255.0,
            1.0,
        ];
        let cam_gpu = CameraGpu {
            view_proj: vp.to_cols_array_2d(),
            eye: [eye.x, eye.y, eye.z, 0.0],
            accent: accent_v,
        };
        gpu.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&cam_gpu));

        // Push the up-to-date overlay model matrix + size + curvature
        // so sliders take effect next frame.
        let model = overlay_model(placement, camera).to_cols_array_2d();
        let params = [
            placement.curvature,
            placement.size_w,
            placement.size_h,
            0.0,
        ];
        gpu.queue
            .write_buffer(&self.quad_buf, 0, bytemuck::bytes_of(&QuadGpu { model, params, tint }));

        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("vr-scene-enc"),
            });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vr-scene-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Deep void black — SteamVR chaperone backdrop.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            // Floor grid first.
            pass.set_pipeline(&self.grid_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.draw(0..6, 0..1);
            // Then the overlay quad on top.
            pass.set_pipeline(&self.quad_pipeline);
            pass.set_bind_group(0, &self.camera_bg, &[]);
            pass.set_bind_group(1, &self.quad_bg, &[]);
            pass.draw(0..6, 0..1);
        }
        gpu.queue.submit(Some(enc.finish()));
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraGpu {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    accent: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct QuadGpu {
    model: [[f32; 4]; 4],
    // x = curvature in [0,1], y = width in metres, z = height in
    // metres, w reserved.
    params: [f32; 4],
    // rgb = night-light multiplier, a = opacity.
    tint: [f32; 4],
}
