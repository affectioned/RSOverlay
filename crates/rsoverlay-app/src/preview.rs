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

/// Automation affordances for running the preview non-interactively —
/// an LLM or CI driver can launch with `--screenshot path.png
/// --exit-after 120` and come back to a single PNG without needing to
/// drive the window.
#[derive(Clone, Debug, Default)]
pub struct TestHooks {
    pub screenshot: Option<std::path::PathBuf>,
    pub exit_after: Option<u64>,
    pub overlay_pos: Option<(f32, f32, f32)>,
    pub overlay_size: Option<(f32, f32)>,
    pub overlay_anchor: Option<String>,
    pub show_keyboard: bool,
    pub show_history: bool,
}

/// Apply CLI overrides to a default Placement. Any field not overridden
/// keeps its default value. Kept public so external callers (tests,
/// future tooling) can reuse the CLI-override logic independently of
/// the broader `seed_dev_view` flow.
#[allow(dead_code)]
pub fn placement_with_overrides(test: &TestHooks) -> rsoverlay_ui::views::Placement {
    let mut p = rsoverlay_ui::views::Placement::default();
    if let Some((x, y, z)) = test.overlay_pos {
        p.offset_x = x;
        p.offset_y = y;
        p.offset_z = z;
    }
    if let Some((w, h)) = test.overlay_size {
        p.size_w = w;
        p.size_h = h;
    }
    if let Some(a) = test.overlay_anchor.as_deref() {
        p.anchor = match a.to_ascii_lowercase().as_str() {
            "wrist" => rsoverlay_ui::views::Anchor::Wrist,
            "hmd" | "head" => rsoverlay_ui::views::Anchor::Hmd,
            "world" => rsoverlay_ui::views::Anchor::World,
            _ => p.anchor,
        };
    }
    p
}

/// Seed a fresh [`DevView`] with values from the persisted config,
/// then layer any CLI test-hook overrides on top. Keeps the "load →
/// override → use" ordering in one place so every entry mode
/// (preview, preview_vr, vr) picks the same initial state.
pub fn seed_dev_view(
    dev: &mut rsoverlay_ui::views::DevView,
    cfg: &rsoverlay_core::Config,
    test: &TestHooks,
) {
    use rsoverlay_core::PlacementAnchor;
    use rsoverlay_ui::views::Anchor as UiAnchor;

    // Appearance.
    if let Some(c) = parse_hex_color(&cfg.appearance.accent_hex) {
        dev.accent = c;
    }
    dev.show_mirror = cfg.appearance.show_mirror;
    dev.show_media = cfg.appearance.show_media;
    dev.show_chat = cfg.appearance.show_chat;
    dev.show_keyboard = cfg.appearance.show_keyboard;
    dev.show_history = cfg.appearance.show_history;
    dev.forward_input = cfg.appearance.forward_input;
    if test.show_keyboard {
        dev.show_keyboard = true;
    }
    if test.show_history {
        dev.show_history = true;
    }

    // Placement.
    dev.placement.anchor = match cfg.placement.anchor {
        PlacementAnchor::Wrist => UiAnchor::Wrist,
        PlacementAnchor::Hmd => UiAnchor::Hmd,
        PlacementAnchor::World => UiAnchor::World,
    };
    dev.placement.offset_x = cfg.placement.offset_x;
    dev.placement.offset_y = cfg.placement.offset_y;
    dev.placement.offset_z = cfg.placement.offset_z;
    dev.placement.size_w = cfg.placement.size_w;
    dev.placement.size_h = cfg.placement.size_h;
    if let Some((x, y, z)) = test.overlay_pos {
        dev.placement.offset_x = x;
        dev.placement.offset_y = y;
        dev.placement.offset_z = z;
    }
    if let Some((w, h)) = test.overlay_size {
        dev.placement.size_w = w;
        dev.placement.size_h = h;
    }
    if let Some(a) = test.overlay_anchor.as_deref() {
        dev.placement.anchor = match a.to_ascii_lowercase().as_str() {
            "wrist" => UiAnchor::Wrist,
            "hmd" | "head" => UiAnchor::Hmd,
            "world" => UiAnchor::World,
            _ => dev.placement.anchor,
        };
    }
}

/// Extract the current DevView state into a fresh Config body, ready
/// to persist. Preserves fields we don't touch (notify, render, etc.)
/// from the given template.
pub fn config_from_dev_view(
    dev: &rsoverlay_ui::views::DevView,
    base: &rsoverlay_core::Config,
) -> rsoverlay_core::Config {
    use rsoverlay_core::PlacementAnchor;
    use rsoverlay_ui::views::Anchor as UiAnchor;
    let mut out = base.clone();
    out.appearance.accent_hex = format!(
        "#{:02X}{:02X}{:02X}",
        dev.accent.r(),
        dev.accent.g(),
        dev.accent.b()
    );
    out.appearance.show_mirror = dev.show_mirror;
    out.appearance.show_media = dev.show_media;
    out.appearance.show_chat = dev.show_chat;
    out.appearance.show_keyboard = dev.show_keyboard;
    out.appearance.show_history = dev.show_history;
    out.appearance.forward_input = dev.forward_input;
    out.placement.anchor = match dev.placement.anchor {
        UiAnchor::Wrist => PlacementAnchor::Wrist,
        UiAnchor::Hmd => PlacementAnchor::Hmd,
        UiAnchor::World => PlacementAnchor::World,
    };
    out.placement.offset_x = dev.placement.offset_x;
    out.placement.offset_y = dev.placement.offset_y;
    out.placement.offset_z = dev.placement.offset_z;
    out.placement.size_w = dev.placement.size_w;
    out.placement.size_h = dev.placement.size_h;
    out
}

fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Debounced "save when something changed" helper. Owns the last
/// saved Config and compares what's live each tick. Writes at most
/// once per `SAVE_DEBOUNCE_MS` even if the user is dragging a slider.
///
/// Also watches the backing file for external edits — the user can
/// hand-edit `config.toml` in a text editor and the running overlay
/// picks up the changes within ~1 second without restart.
pub struct ConfigPersister {
    path: std::path::PathBuf,
    last_saved: rsoverlay_core::Config,
    pending_since: Option<std::time::Instant>,
    last_known_mtime: Option<std::time::SystemTime>,
    last_watch_check: std::time::Instant,
}

impl ConfigPersister {
    const SAVE_DEBOUNCE_MS: u64 = 400;
    const WATCH_INTERVAL_MS: u64 = 1000;

    pub fn new(path: std::path::PathBuf, initial: rsoverlay_core::Config) -> Self {
        let last_known_mtime = Self::read_mtime(&path);
        Self {
            path,
            last_saved: initial,
            pending_since: None,
            last_known_mtime,
            last_watch_check: std::time::Instant::now(),
        }
    }

    /// Call once per frame. If `now` differs from the last saved
    /// snapshot, start / restart the debounce. Once the debounce
    /// window elapses without further changes, write to disk.
    pub fn tick(&mut self, now: &rsoverlay_core::Config) {
        use std::time::Instant;
        let changed = !configs_equivalent(&self.last_saved, now);
        if changed {
            self.pending_since = Some(Instant::now());
            self.last_saved = now.clone();
        }
        if let Some(since) = self.pending_since {
            if since.elapsed().as_millis() as u64 >= Self::SAVE_DEBOUNCE_MS {
                if let Err(e) = self.last_saved.save(&self.path) {
                    tracing::warn!("config save failed: {e:#}");
                }
                // Our own write bumps mtime; track it so the hot-reload
                // check below doesn't fire on our own writes.
                self.last_known_mtime = Self::read_mtime(&self.path);
                self.pending_since = None;
            }
        }
    }

    /// Check whether the backing file was modified externally since
    /// our last tick. Returns the reloaded config if so.
    ///
    /// Call once per frame alongside `tick`. The poll itself runs at
    /// most once per [`WATCH_INTERVAL_MS`] to keep the stat() rate
    /// trivial.
    pub fn poll_external_change(&mut self) -> Option<rsoverlay_core::Config> {
        use std::time::Instant;
        if self.last_watch_check.elapsed().as_millis() as u64 <= Self::WATCH_INTERVAL_MS {
            return None;
        }
        self.last_watch_check = Instant::now();
        let current_mtime = Self::read_mtime(&self.path);
        if current_mtime == self.last_known_mtime {
            return None;
        }
        // File changed externally. Reload and adopt as our new
        // "last_saved" so tick() doesn't immediately re-save.
        match rsoverlay_core::Config::load(&self.path) {
            Ok(cfg) => {
                tracing::info!("config.toml changed externally — reloading");
                self.last_saved = cfg.clone();
                self.last_known_mtime = current_mtime;
                self.pending_since = None;
                Some(cfg)
            }
            Err(e) => {
                tracing::warn!("external config.toml reload failed: {e:#}");
                None
            }
        }
    }

    fn read_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
    }

    /// Final flush — write unconditionally. Call on clean shutdown.
    #[allow(dead_code)]
    pub fn flush(&mut self) {
        if let Err(e) = self.last_saved.save(&self.path) {
            tracing::warn!("config final flush failed: {e:#}");
        }
        self.pending_since = None;
    }
}

/// Field-by-field compare of the two Config variants that matter for
/// debouncing. Ignores Render / Notify / Preview defaults the user
/// can't change from inside the UI yet.
fn configs_equivalent(a: &rsoverlay_core::Config, b: &rsoverlay_core::Config) -> bool {
    a.appearance.accent_hex == b.appearance.accent_hex
        && a.appearance.show_mirror == b.appearance.show_mirror
        && a.appearance.show_media == b.appearance.show_media
        && a.appearance.show_chat == b.appearance.show_chat
        && a.appearance.show_keyboard == b.appearance.show_keyboard
        && a.appearance.show_history == b.appearance.show_history
        && a.appearance.forward_input == b.appearance.forward_input
        && a.placement.anchor == b.placement.anchor
        && (a.placement.offset_x - b.placement.offset_x).abs() < 0.001
        && (a.placement.offset_y - b.placement.offset_y).abs() < 0.001
        && (a.placement.offset_z - b.placement.offset_z).abs() < 0.001
        && (a.placement.size_w - b.placement.size_w).abs() < 0.001
        && (a.placement.size_h - b.placement.size_h).abs() < 0.001
}

/// Save the given surface frame to a PNG. BGRA → RGBA swizzle, then
/// `png` crate for a minimal encoder footprint. Returns the number of
/// bytes written so the caller can log it. Called from the frame-tick
/// handlers of both preview modes once they've reached their capture
/// frame.
pub fn save_surface_as_png(
    gpu: &rsoverlay_render::GpuCtx,
    surface_texture: &wgpu::Texture,
    width: u32,
    height: u32,
    path: &std::path::Path,
) -> anyhow::Result<usize> {
    use anyhow::Context;

    // wgpu requires row-pitch to be a multiple of 256 bytes for texture
    // → buffer copies. BGRA8 at 4 bytes/px → row pitch = width * 4
    // rounded up.
    let bpp = 4u32;
    let unpadded = width * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align) * align;
    let buffer_size = (padded as u64) * height as u64;

    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("screenshot-staging"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut enc = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("screenshot-encoder"),
        });
    enc.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: surface_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(enc.finish()));

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::Maintain::Wait);
    rx.recv()
        .context("map_async channel")?
        .context("map_async")?;

    let data = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        let row_slice = &data[start..start + unpadded as usize];
        for px in row_slice.as_chunks::<4>().0 {
            // wgpu swapchain is BGRA on Windows → emit as RGBA.
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    drop(data);
    buffer.unmap();

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).context("creating output dir")?;
        }
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("creating PNG at {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("PNG header")?;
    writer.write_image_data(&rgba).context("PNG data")?;
    Ok(rgba.len())
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
const INITIAL_WIN_H: u32 = 1600;

pub fn run(
    mut notify: Option<NotifyChannel>,
    capture_cfg: CaptureCfg,
    osc: Option<rsoverlay_osc::OscClient>,
    test: TestHooks,
    app_config: rsoverlay_core::Config,
    config_path: std::path::PathBuf,
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
    seed_dev_view(&mut dev_view, &app_config, &test);
    let mut persister = ConfigPersister::new(config_path.clone(), app_config.clone());
    let config_base = app_config.clone();
    let mut fps_counter = FpsCounter::new();
    let mut frame_counter: u64 = 0;
    let screenshot_path = test.screenshot.clone();
    let exit_after = test.exit_after;

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
                            |ctx| dev_view.ui_with(ctx, Some(&injector)),
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

                        // Snap the OverlayTarget (what VR would receive)
                        // before presenting, so the PNG matches what a
                        // headset would actually render.
                        frame_counter = frame_counter.wrapping_add(1);
                        if let Some(trigger) = exit_after {
                            if frame_counter == trigger {
                                if let Some(path) = &screenshot_path {
                                    match save_surface_as_png(
                                        &gpu,
                                        &overlay_target.texture,
                                        overlay_target.width,
                                        overlay_target.height,
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

                        // "Reset defaults" clicked: snap DevView back to
                        // pristine state. Persistence tick on the next
                        // line notices the drift and writes that out.
                        if dev_view.reset_requested {
                            let base = rsoverlay_core::Config::default();
                            seed_dev_view(&mut dev_view, &base, &TestHooks::default());
                            dev_view.reset_requested = false;
                        }

                        // Pick up external edits (user opened
                        // config.toml in a text editor and saved).
                        if let Some(cfg) = persister.poll_external_change() {
                            seed_dev_view(&mut dev_view, &cfg, &TestHooks::default());
                        }

                        // Persist state changes (debounced inside the
                        // persister). Slider/toggle writes land on disk
                        // ~400ms after the last change.
                        let current = config_from_dev_view(&dev_view, &config_base);
                        persister.tick(&current);
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
