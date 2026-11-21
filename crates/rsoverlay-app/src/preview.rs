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
use rsoverlay_capture::{enumerate_monitors, DxgiDuplication, Frame, SharedCapture};
use rsoverlay_input::{InputInjector, MonitorGeom, MouseButton};
use rsoverlay_core::RuntimeMode;
use rsoverlay_notify::NotifyChannel;
use rsoverlay_render::{
    mirror::MirrorTexture,
    shared::{wgpu_adapter_luid, SharedMirror},
    GpuCtx, OverlayTarget,
};
use rsoverlay_ui::{
    views::{DevView, MirrorHandle},
    EguiPanel,
};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info, warn};

#[derive(Copy, Clone, Debug)]
pub struct CaptureCfg {
    pub enabled: bool,
    pub monitor_index: u32,
    /// When true, skip the GPU-to-GPU shared-handle path entirely and
    /// use the CPU readback path. Useful for A/B testing and for working
    /// around driver bugs we haven't shaken out yet.
    pub force_cpu: bool,
}
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{Key, NamedKey},
    window::WindowBuilder,
};

/// Overlay texture resolution.
///
/// Sized to stay crisp at ~1.5-2× texels-per-display-pixel on current retail
/// HMDs for a 1 m-wide overlay at 2 m. The worst case we size for today is
/// Pimax Crystal / Varjo Aero (~1200 display pixels across the quad). 2048
/// gives a ~1.7× sampling ratio and ~16 MB of GPU memory — safe for every
/// shipping headset through late-2026. Power users on Crystal Super / XR-4
/// can override via `[render] overlay_texture_size` in config.toml.
const OVERLAY_TEX_SIZE: u32 = 2048;
// Tall enough that every left-column card (Media, Stats, Forward,
// Chatbox, Appearance) plus the Clock bar fits without scrolling at
// default ppp. If users want smaller they can resize down.
const INITIAL_WIN_W: u32 = 1280;
const INITIAL_WIN_H: u32 = 1360;

pub fn run(
    mut notify: Option<NotifyChannel>,
    capture_cfg: CaptureCfg,
    osc: Option<rsoverlay_osc::OscClient>,
) -> Result<()> {
    let event_loop = EventLoop::new().context("creating winit event loop")?;
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("RSOverlay — Preview")
            .with_inner_size(winit::dpi::LogicalSize::new(INITIAL_WIN_W, INITIAL_WIN_H))
            .with_min_inner_size(winit::dpi::LogicalSize::new(900u32, 760u32))
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

    // In preview mode we size the overlay texture to match the window
    // surface so blit is 1:1 — this means mouse clicks and egui's
    // rendered widgets agree in coordinates (no non-uniform stretch).
    // In VR mode a separate code path pins the overlay to the
    // HMD-tuned OVERLAY_TEX_SIZE; the preview just needs
    // input/render consistency.
    let mut overlay_target =
        OverlayTarget::new(&gpu, size.width.max(1), size.height.max(1));
    let _ = OVERLAY_TEX_SIZE;

    // Blit pipeline: fullscreen triangle samples the overlay texture.
    let mut blit = BlitPipeline::new(&gpu.device, &overlay_target, surface_format);

    // egui panel drawing into the overlay target.
    let mut egui_panel = EguiPanel::new(
        &gpu.device,
        overlay_target.format,
        overlay_target.width,
        overlay_target.height,
    );
    // native_pixels_per_point = None so egui-winit reads from
    // ctx.pixels_per_point() each frame. Combined with overlay_target
    // tracking window size 1:1 this keeps mouse hit-testing aligned
    // with egui's rendered widget positions.
    let mut egui_winit = egui_winit::State::new(
        egui_panel.ctx.clone(),
        egui::ViewportId::ROOT,
        &*window,
        None,
        None,
    );
    let mut dev_view = DevView::default();
    let mut fps_counter = FpsCounter::new();

    // Capture pipeline. We try the GPU-to-GPU shared-handle path first
    // (see `rsoverlay-capture::shared` + `rsoverlay-render::shared`). If
    // anything along that path fails — or the user forced CPU mode —
    // we fall back to the original CPU readback through an mpsc channel.
    let (capture_tx, capture_rx) = mpsc::sync_channel::<Frame>(2);
    let mut mirror_texture: Option<MirrorTexture> = None;
    let mut mirror_handle: Option<MirrorHandle> = None;
    let mut mirror_geom: Option<MonitorGeom> = None;
    let mut shared_mirror: Option<SharedMirror> = None;
    let mut using_gpu_path = false;

    if capture_cfg.enabled {
        if let Ok(mons) = enumerate_monitors() {
            if let Some(m) = mons.into_iter().find(|m| m.index == capture_cfg.monitor_index) {
                mirror_geom = Some(MonitorGeom {
                    origin_x: m.origin_x,
                    origin_y: m.origin_y,
                    width: m.width,
                    height: m.height,
                });
            }
        }

        if !capture_cfg.force_cpu {
            match try_setup_shared(&gpu, capture_cfg.monitor_index) {
                Ok((mut capture, mirror)) => {
                    let w = mirror.width;
                    let h = mirror.height;
                    // Capture thread: drives the shared texture GPU-side; no
                    // Vec, no mpsc data, just keyed-mutex ping-pong.
                    std::thread::Builder::new()
                        .name("rsoverlay-shared-capture".into())
                        .spawn(move || loop {
                            if let Err(e) = capture.capture_next_frame(16) {
                                warn!("shared capture lost ({e:#}); thread exit");
                                break;
                            }
                        })
                        .context("spawning shared-capture thread")?;
                    let id = egui_panel.renderer.register_native_texture(
                        &gpu.device,
                        &mirror.view,
                        wgpu::FilterMode::Linear,
                    );
                    mirror_handle = Some(MirrorHandle { id, width: w, height: h });
                    shared_mirror = Some(mirror);
                    using_gpu_path = true;
                    info!(
                        "desktop capture: GPU-to-GPU shared handle ({}x{}) — \
                         zero CPU readback",
                        w, h
                    );
                }
                Err(e) => {
                    warn!("GPU-to-GPU capture unavailable, falling back to CPU: {e:#}");
                }
            }
        }

        if !using_gpu_path {
            let tx = capture_tx;
            let idx = capture_cfg.monitor_index;
            std::thread::Builder::new()
                .name("rsoverlay-capture".into())
                .spawn(move || capture_loop(idx, tx))
                .context("spawning CPU capture thread")?;
        }
    }
    let injector = InputInjector::new().context("initializing input injector")?;

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
                        // Rebuild the overlay target + blit pipeline so
                        // the overlay texture stays 1:1 with the window
                        // surface — mouse coordinates flow straight
                        // through without a non-uniform stretch.
                        overlay_target =
                            OverlayTarget::new(&gpu, config.width, config.height);
                        blit = BlitPipeline::new(
                            &gpu.device,
                            &overlay_target,
                            surface_format,
                        );
                        egui_panel.resize(config.width, config.height);
                    }
                    WindowEvent::RedrawRequested => {
                        // 0a. Drain any newly-arrived notifications.
                        if let Some(n) = notify.as_mut() {
                            while let Ok(notif) = n.rx.try_recv() {
                                dev_view.push_notification(notif);
                            }
                        }

                        // 0b. CPU path only: drain the latest captured frame
                        // (coalesce — only the newest matters) and upload it.
                        // On the GPU path the shared texture is already
                        // kept current by the capture thread; nothing to do
                        // here except acquire a read lease below.
                        if !using_gpu_path {
                            let mut latest: Option<Frame> = None;
                            while let Ok(f) = capture_rx.try_recv() {
                                latest = Some(f);
                            }
                            if let Some(frame) = latest {
                                let (mt, needs_register) = match mirror_texture.as_ref() {
                                    Some(mt) if mt.width == frame.width && mt.height == frame.height => {
                                        (mt, false)
                                    }
                                    _ => {
                                        let mt = MirrorTexture::new(&gpu, frame.width, frame.height);
                                        mirror_texture = Some(mt);
                                        (mirror_texture.as_ref().unwrap(), true)
                                    }
                                };
                                if let Err(e) = mt.upload(&gpu, &frame) {
                                    warn!("mirror upload: {e:#}");
                                }
                                if needs_register {
                                    let id = egui_panel.renderer.register_native_texture(
                                        &gpu.device,
                                        &mt.view,
                                        wgpu::FilterMode::Linear,
                                    );
                                    mirror_handle = Some(MirrorHandle {
                                        id,
                                        width: frame.width,
                                        height: frame.height,
                                    });
                                }
                            }
                        }
                        dev_view.mirror = mirror_handle;

                        // Acquire the keyed-mutex read lease on the GPU
                        // path for the duration of this frame. Dropped at
                        // the end of RedrawRequested, handing write
                        // ownership back to the capture thread.
                        let _read_lease = shared_mirror
                            .as_ref()
                            .and_then(|m| m.acquire_read(8).ok());

                        // 0c. Forward any pending mirror hit to the real
                        // desktop. Move always; click only if a click was
                        // actually recorded this frame.
                        if let (Some(hit), Some(geom)) =
                            (dev_view.take_mirror_hit(), mirror_geom)
                        {
                            if hit.clicked {
                                if let Err(e) = injector.click_at(&geom, hit.x, hit.y, MouseButton::Left) {
                                    warn!("input inject click: {e:#}");
                                }
                            } else if let Err(e) = injector.move_to(&geom, hit.x, hit.y) {
                                warn!("input inject move: {e:#}");
                            }
                        }

                        // 0d. Forward any pending chatbox send.
                        if let Some(msg) = dev_view.take_pending_chat() {
                            match &osc {
                                Some(client) => {
                                    if let Err(e) = client.tx.send(msg) {
                                        warn!("osc channel closed: {e:#}");
                                    }
                                }
                                None => warn!("chatbox send ignored: OSC disabled via --no-osc"),
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

/// Build both ends of the GPU-to-GPU mirror. Either returns the pair
/// ready to drive, or an error describing which step failed so the
/// caller can decide whether to fall back.
fn try_setup_shared(
    gpu: &GpuCtx,
    monitor_index: u32,
) -> Result<(SharedCapture, SharedMirror)> {
    let luid = wgpu_adapter_luid(gpu);
    let capture = SharedCapture::new(monitor_index, luid)
        .context("SharedCapture::new")?;
    let mirror = unsafe {
        SharedMirror::open(gpu, capture.shared_handle_raw(), capture.width, capture.height)
    }
    .context("SharedMirror::open on wgpu DX12 device")?;
    Ok((capture, mirror))
}

/// DXGI duplication pump. Runs on its own thread, publishes the newest frame
/// every ~16 ms. On disconnection of the channel it exits cleanly. On any
/// recoverable duplication error (e.g. output switch) it retries after a
/// brief sleep.
fn capture_loop(monitor_index: u32, tx: mpsc::SyncSender<Frame>) {
    loop {
        let mut dup = match DxgiDuplication::new(monitor_index) {
            Ok(d) => d,
            Err(e) => {
                error!("desktop capture setup failed for monitor {monitor_index}: {e:#}");
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };
        info!("desktop capture started on monitor {monitor_index}");
        loop {
            match dup.next_frame(16) {
                Ok(Some(frame)) => {
                    // try_send: drop the frame if UI is behind, keep latency low.
                    let _ = tx.try_send(frame);
                }
                Ok(None) => {} // timeout; no new frame this tick
                Err(e) => {
                    warn!("capture lost ({e:#}); retrying");
                    break;
                }
            }
        }
    }
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
