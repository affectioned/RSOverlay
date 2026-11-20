//! Desktop preview mode.
//!
//! Opens a winit window, builds a wgpu surface on it, and runs the same panel
//! render pipeline the VR path uses. The overlay texture is rendered
//! offscreen into an [`OverlayTarget`] via egui-wgpu and then blitted onto the
//! window via a fullscreen-triangle pass. This matches the VR flow one-to-one;
//! only the final presentation step differs.
//!
//! Window events are forwarded to `egui-winit` so the UI inside the overlay
//! responds to the cursor exactly the way VR laser-pointer input will
//! eventually drive it.

use anyhow::{Context, Result};
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::NotifyChannel;
use rsoverlay_render::{GpuCtx, OverlayTarget};
use rsoverlay_ui::{views::DevView, EguiPanel};
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

const OVERLAY_TEX_SIZE: u32 = 1024;
const INITIAL_WIN_W: u32 = 1024;
const INITIAL_WIN_H: u32 = 1024;

pub fn run(mut notify: Option<NotifyChannel>) -> Result<()> {
    let event_loop = EventLoop::new().context("creating winit event loop")?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RSOverlay — Preview")
            .with_inner_size(winit::dpi::LogicalSize::new(INITIAL_WIN_W, INITIAL_WIN_H))
            .build(&event_loop)
            .context("creating preview window")?,
    );

    let gpu = pollster::block_on(GpuCtx::new(RuntimeMode::Preview))?;

    // Surface on the winit window.
    let surface = gpu
        .instance
        .create_surface(window.clone())
        .context("creating wgpu surface for preview window")?;
    let size = window.inner_size();
    let caps = surface.get_capabilities(&gpu.adapter);
    let surface_format = caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(caps.formats[0]);
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&gpu.device, &config);

    // The overlay texture — same type/role as the one we would hand to OpenVR.
    let overlay_target = OverlayTarget::new(&gpu, OVERLAY_TEX_SIZE, OVERLAY_TEX_SIZE);

    // Blit pipeline: fullscreen triangle samples the overlay texture.
    let blit = BlitPipeline::new(&gpu.device, &overlay_target, surface_format);

    // egui panel drawing into the overlay target.
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
        Some(window.scale_factor() as f32),
        None,
    );
    let mut dev_view = DevView::default();
    let mut fps_counter = FpsCounter::new();

    info!("preview window up — close window or press Esc to exit");

    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { event, window_id } if window_id == window.id() => {
                // Forward to egui first so the UI gets input.
                let _ = egui_winit.on_window_event(&window, &event);
                match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::KeyboardInput { event, .. } => {
                        if event.state == winit::event::ElementState::Pressed
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
                    WindowEvent::RedrawRequested => {
                        // 0. Drain any newly-arrived notifications.
                        if let Some(n) = notify.as_mut() {
                            while let Ok(notif) = n.rx.try_recv() {
                                dev_view.push_notification(notif);
                            }
                        }

                        // 1. Run egui into the overlay target.
                        dev_view.frame = dev_view.frame.wrapping_add(1);
                        dev_view.last_fps = fps_counter.tick();
                        let raw_input = egui_winit.take_egui_input(&window);
                        egui_panel.render(
                            &gpu.device,
                            &gpu.queue,
                            &overlay_target.view,
                            raw_input,
                            |ctx| dev_view.ui(ctx),
                        );

                        // 2. Blit the overlay target into the window.
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
                        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut enc = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("preview-blit-encoder"),
                        });
                        blit.record(&mut enc, &view);
                        gpu.queue.submit(Some(enc.finish()));
                        frame.present();
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;
    Ok(())
}

/// Rolling FPS counter — one sample per second.
struct FpsCounter {
    last_sample: Instant,
    frames: u32,
    cached: f32,
}

impl FpsCounter {
    fn new() -> Self {
        Self { last_sample: Instant::now(), frames: 0, cached: 0.0 }
    }
    fn tick(&mut self) -> f32 {
        self.frames += 1;
        let elapsed = self.last_sample.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.cached = self.frames as f32 / elapsed;
            self.frames = 0;
            self.last_sample = Instant::now();
        }
        self.cached
    }
}

/// Fullscreen-triangle blit of a sampled texture onto the swapchain view.
struct BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl BlitPipeline {
    fn new(device: &wgpu::Device, source: &OverlayTarget, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("blit-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("blit-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit-pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });
        Self { pipeline, bind_group }
    }

    fn record(&self, enc: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("blit-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
