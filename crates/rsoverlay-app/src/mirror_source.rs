//! One live GPU-to-GPU mirror pipeline — monitor (DXGI duplication) or
//! single window (Windows.Graphics.Capture) — that can be torn down and
//! replaced at runtime when the user picks a different source.
//!
//! Both sources share the same shape: a capture thread writes into an
//! NT-shared D3D11 texture, the render side opens it once on the wgpu
//! DX12 device, egui samples it as a native texture. Only the window
//! path has a live content sub-rect (see `rsoverlay_capture::window`).

use anyhow::{Context, Result};
use rsoverlay_capture::window::{self, ContentSize};
use rsoverlay_capture::{enumerate_monitors, enumerate_windows, SharedCapture, WindowCapture};
use rsoverlay_input::MonitorGeom;
use rsoverlay_render::mips::MipMirror;
use rsoverlay_render::shared::{wgpu_adapter_luid, ReadLease, SharedMirror};
use rsoverlay_render::GpuCtx;
use rsoverlay_ui::views::{CaptureSource, MirrorHandle};
use rsoverlay_ui::EguiPanel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use tracing::{info, warn};

pub struct GpuMirror {
    pub source: CaptureSource,
    mirror: SharedMirror,
    /// Mipmapped copy egui draws from (see `rsoverlay_render::mips`).
    mips: MipMirror,
    tex_id: egui::TextureId,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// `Some` for window capture — the live sub-rect of the texture.
    content: Option<ContentSize>,
    monitor_geom: Option<MonitorGeom>,
}

impl GpuMirror {
    pub fn start(gpu: &GpuCtx, panel: &mut EguiPanel, source: CaptureSource) -> Result<Self> {
        let luid = wgpu_adapter_luid(gpu);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (mirror, thread, content, monitor_geom) = match &source {
            CaptureSource::Monitor(idx) => {
                let mut cap = SharedCapture::new(*idx, luid).context("SharedCapture::new")?;
                let mirror = unsafe { SharedMirror::open(gpu, cap.shared_handle_raw(), cap.width, cap.height) }
                    .context("SharedMirror::open (monitor)")?;
                let thread = spawn("rsoverlay-shared-capture", move || {
                    while !flag.load(Ordering::Relaxed) {
                        if let Err(e) = cap.capture_next_frame(16) {
                            warn!("monitor capture lost ({e:#}); thread exit");
                            break;
                        }
                    }
                })?;
                let geom = enumerate_monitors().ok().and_then(|ms| {
                    ms.into_iter().find(|m| m.index == *idx).map(|m| MonitorGeom {
                        origin_x: m.origin_x,
                        origin_y: m.origin_y,
                        width: m.width,
                        height: m.height,
                    })
                });
                (mirror, thread, None, geom)
            }
            CaptureSource::Window { hwnd, .. } => {
                let mut cap = WindowCapture::new(*hwnd, luid).context("WindowCapture::new")?;
                let mirror = unsafe { SharedMirror::open(gpu, cap.shared_handle_raw(), cap.width, cap.height) }
                    .context("SharedMirror::open (window)")?;
                let content = cap.content.clone();
                let thread = spawn("rsoverlay-window-capture", move || {
                    while !flag.load(Ordering::Relaxed) {
                        if let Err(e) = cap.capture_next_frame(4) {
                            warn!("window capture ended ({e:#}); thread exit");
                            break;
                        }
                    }
                })?;
                (mirror, thread, Some(content), None)
            }
        };
        let mips = MipMirror::new(gpu, mirror.width, mirror.height);
        mips.refresh(gpu, &mirror.texture);
        let tex_id = panel.renderer.register_native_texture_with_sampler_options(&gpu.device, &mips.view, MipMirror::sampler());
        info!(
            "mirror source: {} ({}x{} shared texture, GPU-to-GPU)",
            source.label(),
            mirror.width,
            mirror.height
        );
        Ok(Self { source, mirror, mips, tex_id, stop, thread: Some(thread), content, monitor_geom })
    }

    pub fn handle(&self) -> MirrorHandle {
        match &self.content {
            None => {
                let mut h = MirrorHandle::full(self.tex_id, self.mirror.width, self.mirror.height);
                // Desktop duplication omits the cursor: report where it is.
                h.cursor = self.monitor_geom.and_then(|g| {
                    let (x, y) = rsoverlay_capture::window::cursor_pos()?;
                    let (u, v) = ((x - g.origin_x) as f32 / g.width as f32, (y - g.origin_y) as f32 / g.height as f32);
                    ((0.0..1.0).contains(&u) && (0.0..1.0).contains(&v)).then_some(egui::vec2(u, v))
                });
                h
            }
            Some(c) => {
                let (w, h) = c.get();
                let (w, h) = (w.max(1), h.max(1));
                let uv = egui::Rect::from_min_max(
                    egui::Pos2::ZERO,
                    egui::pos2(w as f32 / self.mirror.width as f32, h as f32 / self.mirror.height as f32),
                );
                // WGC captures the cursor itself (SetIsCursorCaptureEnabled).
                MirrorHandle { id: self.tex_id, width: w, height: h, uv, cursor: None, resizable: true }
            }
        }
    }

    /// Screen geometry for input forwarding. Windows move, so the window
    /// rect is re-queried on every call.
    pub fn geom(&self) -> Option<MonitorGeom> {
        match &self.source {
            CaptureSource::Monitor(_) => self.monitor_geom,
            CaptureSource::Window { hwnd, .. } => {
                window::window_frame_rect(*hwnd).map(|(x, y, w, h)| MonitorGeom {
                    origin_x: x,
                    origin_y: y,
                    width: w,
                    height: h,
                })
            }
        }
    }

    pub fn window_hwnd(&self) -> Option<isize> {
        match &self.source {
            CaptureSource::Window { hwnd, .. } => Some(*hwnd),
            CaptureSource::Monitor(_) => None,
        }
    }

    /// Pull the latest captured frame into the mipmapped copy. Call
    /// before each render that draws [`Self::handle`].
    pub fn refresh(&self, gpu: &GpuCtx) {
        self.mips.refresh(gpu, &self.mirror.texture);
    }

    pub fn acquire_read(&self, timeout_ms: u32) -> Option<ReadLease<'_>> {
        self.mirror.acquire_read(timeout_ms).ok()
    }

    /// Stop the capture thread and release the egui texture slot.
    pub fn shutdown(mut self, panel: &mut EguiPanel) {
        self.stop_thread();
        panel.renderer.free_texture(&self.tex_id);
    }

    fn stop_thread(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for GpuMirror {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>> {
    std::thread::Builder::new().name(name.into()).spawn(f).context("spawning capture thread")
}

/// App icons for windows not already in `have`, as egui images.
/// Which program each listed window belongs to (for hiding apps).
pub fn fetch_apps(sources: &[CaptureSource], have: &mut std::collections::HashMap<isize, String>) {
    for s in sources {
        if let CaptureSource::Window { hwnd, .. } = s {
            if !have.contains_key(hwnd) {
                if let Some(app) = rsoverlay_capture::window::window_app(*hwnd) {
                    have.insert(*hwnd, app);
                }
            }
        }
    }
}

pub fn fetch_icons(sources: &[CaptureSource], have: &mut std::collections::HashMap<isize, egui::ColorImage>) {
    for s in sources {
        if let CaptureSource::Window { hwnd, .. } = s {
            if have.contains_key(hwnd) {
                continue;
            }
            if let Some((w, h, px)) = rsoverlay_capture::window::window_icon(*hwnd) {
                have.insert(*hwnd, egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px));
            }
        }
    }
}

/// Thumbnail width for the Desktop picker, px.
const THUMB_W: u32 = 320;

/// Grabs Desktop-picker thumbnails on its own thread: `PrintWindow` can
/// block on a hung app and costs a full-size CPU copy per window, neither
/// of which belongs on the render loop. Requests are coalesced (a request
/// arriving while one is in flight is dropped — the picker asks again in
/// 2 s). The thread exits when the worker is dropped.
pub struct ThumbWorker {
    tx: std::sync::mpsc::SyncSender<Vec<isize>>,
    out: Arc<std::sync::Mutex<(u64, std::collections::HashMap<isize, egui::ColorImage>)>>,
    seen: u64,
}

impl ThumbWorker {
    pub fn spawn() -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<isize>>(1);
        let out = Arc::new(std::sync::Mutex::new((0u64, std::collections::HashMap::new())));
        let shared = out.clone();
        let _ = std::thread::Builder::new().name("rso-thumbs".into()).spawn(move || {
            while let Ok(hwnds) = rx.recv() {
                let fresh: std::collections::HashMap<isize, egui::ColorImage> = hwnds
                    .into_iter()
                    .filter_map(|h| {
                        rsoverlay_capture::thumbnail::window_thumbnail(h, THUMB_W)
                            .map(|(w, hh, px)| (h, egui::ColorImage::from_rgba_unmultiplied([w as usize, hh as usize], &px)))
                    })
                    .collect();
                if let Ok(mut o) = shared.lock() {
                    o.0 += 1;
                    o.1 = fresh;
                }
            }
        });
        Self { tx, out, seen: 0 }
    }

    /// Ask for thumbnails of these windows (dropped if one is in flight).
    pub fn request(&self, sources: &[CaptureSource]) {
        let hwnds = sources
            .iter()
            .filter_map(|s| match s {
                CaptureSource::Window { hwnd, .. } => Some(*hwnd),
                CaptureSource::Monitor(_) => None,
            })
            .collect();
        let _ = self.tx.try_send(hwnds);
    }

    /// Copy a finished batch into the picker; true if anything changed.
    pub fn collect(&mut self, picker: &mut rsoverlay_ui::views::SourcePicker) -> bool {
        let Ok(o) = self.out.lock() else { return false };
        if o.0 == self.seen {
            return false;
        }
        self.seen = o.0;
        picker.set_thumbnails(o.0, &o.1);
        true
    }
}

/// Every monitor plus every capturable window, for the source picker.
pub fn list_sources() -> Vec<CaptureSource> {
    let mut out: Vec<CaptureSource> = enumerate_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|m| CaptureSource::Monitor(m.index))
        .collect();
    out.extend(
        enumerate_windows()
            .unwrap_or_default()
            .into_iter()
            .map(|w| CaptureSource::Window { hwnd: w.hwnd, title: w.title }),
    );
    out
}

/// On the real GPU, through the real window-panel draw: at native
/// resolution a capture of 1-pixel black / white columns (the finest
/// detail text has) must come out as exactly that, not grey. This is
/// what makes monitor text as sharp as XSOverlay's.
#[cfg(test)]
mod native_gpu_tests {
    use rsoverlay_render::mips::MipMirror;
    use rsoverlay_render::GpuCtx;
    use rsoverlay_ui::views::MirrorHandle;
    use rsoverlay_ui::window_panel::{self, Attach};
    use rsoverlay_ui::EguiPanel;

    fn readback(gpu: &GpuCtx, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<u8> {
        let row = (4 * w).div_ceil(256) * 256;
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::ImageCopyBuffer { buffer: &buf, layout: wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        gpu.queue.submit(Some(enc.finish()));
        buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::Maintain::Wait);
        let data = buf.slice(..).get_mapped_range();
        (0..h as usize).flat_map(|y| data[y * row as usize..][..4 * w as usize].to_vec()).collect()
    }

    /// Draw a 2560 x 1440 capture of 1-px columns into a window panel at
    /// render multiplier `k`; returns (fraction of exact texels, panel).
    fn draw(gpu: &GpuCtx, k: f32) -> f32 {
        let (sw, sh) = (2560u32, 1440u32);
        let src = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8UnormSrgb,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let px: Vec<u8> = (0..sw * sh).flat_map(|i| if (i % sw) % 2 == 0 { [0, 0, 0, 255] } else { [255; 4] }).collect();
        gpu.queue.write_texture(
            src.as_image_copy(),
            &px,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * sw), rows_per_image: Some(sh) },
            wgpu::Extent3d { width: sw, height: sh, depth_or_array_layers: 1 },
        );
        // The window panel as the VR shell builds it: 1920 x 1200 base
        // at 1.5 px/pt, times k.
        let (tw, th) = ((1920.0 * k).round() as u32, (1200.0 * k).round() as u32);
        let ppp = 1.5 * k;
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: tw, height: th, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut panel = EguiPanel::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm, tw, th);
        panel.set_pixels_per_point(ppp);
        let mips = MipMirror::new(gpu, sw, sh);
        mips.refresh(gpu, &src);
        let id = panel.renderer.register_native_texture_with_sampler_options(&gpu.device, &mips.view, MipMirror::sampler());
        let mut rect = None;
        // Two frames, like the live panel: settings apply from the next.
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(tw as f32 / ppp, th as f32 / ppp))), ..Default::default() };
            panel.render(&gpu.device, &gpu.queue, &view, raw, |ctx| {
                let a = window_panel::show(ctx, "test", Some(MirrorHandle::full(id, sw, sh)), Attach::World, false, None, false, 1.0, None, false, false, egui::Color32::WHITE);
                rect = a.image_rect;
            });
        }
        assert_eq!(panel.ctx.pixels_per_point(), ppp);
        let out = readback(gpu, &target, tw, th);
        let r = rect.expect("capture drawn");
        if std::env::var("RSO_DEBUG_NATIVE").is_ok() {
            eprintln!("k {k} tex {tw}x{th} rect {r:?} px {:?}", (r.min.x * ppp, r.min.y * ppp, r.max.x * ppp, r.max.y * ppp));
        }
        // Interior of the capture (clear of the rounded corners).
        let (x0, y0) = ((r.min.x * ppp) as usize + 16, (r.min.y * ppp) as usize + 16);
        let (x1, y1) = ((r.max.x * ppp) as usize - 16, (r.max.y * ppp) as usize - 16);
        let (mut exact, mut n) = (0usize, 0usize);
        for y in (y0..y1).step_by(37) {
            for x in x0..x1 {
                let p = out[(y * tw as usize + x) * 4];
                // Texel x shows source column (x - left edge).
                let want = if (x - (r.min.x * ppp).round() as usize).is_multiple_of(2) { 0 } else { 255 };
                // Float rounding in the sampler can leak ~0.4% of a neighbour
                // (13/255 in sRGB on black): invisible. A blur is ~50%.
                let ok = p.abs_diff(want) <= 20;
                if !ok && n % 97 == 0 && std::env::var("RSO_DEBUG_NATIVE").is_ok() {
                    eprintln!("miss at ({x},{y}): {p} want {want}");
                }
                exact += usize::from(ok);
                n += 1;
            }
        }
        exact as f32 / n as f32
    }

    #[test]
    fn native_resolution_copies_the_capture_exactly() {
        let Ok(gpu) = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Preview)) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let k = crate::vr_logic::native_render_k(1.0, 2560, 1920.0);
        let native = draw(&gpu, k);
        assert!(native > 0.99, "native: only {:.1}% of texels exact", native * 100.0);
        // The old fixed 1920-texel panel: the columns blur together.
        let old = draw(&gpu, 1.0);
        assert!(old < 0.5, "base size should be resampled ({:.1}% exact)", old * 100.0);
        println!("exact texels: native {:.1}%, old 1920 {:.1}%", native * 100.0, old * 100.0);
    }

    /// Draw a once-drawn VR texture the way the VR loop does
    /// (render_settled, transparent clear) and read it back.
    fn draw_static(gpu: &GpuCtx, w: u32, h: u32, paint: fn(&egui::Context)) -> Vec<u8> {
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut panel = EguiPanel::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm, w, h);
        panel.set_pixels_per_point(1.0);
        panel.set_clear(wgpu::Color::TRANSPARENT);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))), ..Default::default() };
        panel.render_settled(&gpu.device, &gpu.queue, &view, raw, paint);
        readback(gpu, &target, w, h)
    }

    /// Tester: the laser beam never showed although SteamVR found it in
    /// the right place: its once-drawn texture was egui's unsettled first
    /// frame (max alpha 15). Beam: bright core, soft glow across, fading
    /// in from the controller.
    #[test]
    fn beam_texture_has_a_bright_core_and_fades_in() {
        let Ok(gpu) = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Preview)) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let (w, h) = (256u32, 32u32);
        let px = draw_static(&gpu, w, h, rsoverlay_ui::shell::paint_beam);
        let a = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
        println!("core along x: start {} 20% {} middle {} tip {}; across at middle: edge {} glow {} core {}", a(2, 16), a(51, 16), a(128, 16), a(254, 16), a(128, 0), a(128, 8), a(128, 16));
        assert!(a(128, 16) > 230, "bright core");
        assert!(a(128, 8) < a(128, 16) && a(128, 8) > 20, "soft glow beside it");
        assert!(a(128, 0) < a(128, 8), "fading to the edges");
        assert!(a(2, 16) < 40, "fades in from the controller");
        assert!(a(230, 16) > 230, "solid nearly to the tip");
        assert!(a(255, 16) < 140 && a(255, 10) < 10, "narrows to a point");
        if let Ok(d) = std::env::var("RSO_TEX_DUMP") {
            std::fs::write(format!("{d}/beam_{w}x{h}.raw"), &px).unwrap();
        }
    }

    #[test]
    fn cursor_texture_is_drawn() {
        let Ok(gpu) = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Preview)) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let px = draw_static(&gpu, 128, 128, rsoverlay_ui::shell::paint_cursor_dot);
        let a = |x: usize, y: usize| px[(y * 128 + x) * 4 + 3];
        assert!(a(64, 64) > 200, "centre dot");
        assert!(a(2, 2) < 10, "transparent corners");
        if let Ok(d) = std::env::var("RSO_TEX_DUMP") {
            std::fs::write(format!("{d}/cursor_128x128.raw"), &px).unwrap();
        }
    }

    /// The laser texture (beam + ring in one): the ring sits at the hit,
    /// the beam stops at the ring, nothing after it. Writes the texture
    /// to RSO_LASER_DUMP (raw BGRA) when set, to look at by eye.
    #[test]
    fn laser_texture_ends_in_the_ring() {
        let Ok(gpu) = pollster::block_on(GpuCtx::new(rsoverlay_core::RuntimeMode::Preview)) else {
            eprintln!("SKIPPED: no GPU adapter");
            return;
        };
        let (w, h) = (1024u32, 64u32);
        // A 0.5 m hit, 6.5 mm ring, seen at 40 degrees.
        let (len, ring) = (0.5f32, 0.0065f32);
        let fs = 40f32.to_radians().sin();
        let total = len + ring * 1.3 / fs;
        let height = 2.4 * ring;
        let look = rsoverlay_ui::shell::LaserLook {
            hit_u: len / total,
            pt_per_m_x: w as f32 / total,
            pt_per_m_y: h as f32 / height,
            core_m: 0.0025,
            ring_m: ring,
            foreshorten: fs,
        };
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut panel = EguiPanel::new(&gpu.device, wgpu::TextureFormat::Bgra8Unorm, w, h);
        panel.set_pixels_per_point(1.0);
        panel.set_clear(wgpu::Color::TRANSPARENT);
        let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))), ..Default::default() };
        panel.render_settled(&gpu.device, &gpu.queue, &view, raw, move |ctx| rsoverlay_ui::shell::paint_laser(ctx, &look));
        let px = readback(&gpu, &target, w, h);
        if let Ok(path) = std::env::var("RSO_LASER_DUMP") {
            std::fs::write(path, &px).unwrap();
        }
        let a = |x: usize, y: usize| px[(y * w as usize + x) * 4 + 3];
        let tip = (look.hit_u * w as f32) as usize;
        assert!(a(tip, 32) > 200, "centre dot at the hit");
        assert!(a(w as usize / 2, 32) > 200, "beam core midway");
        assert!(a(2, 32) < 40, "beam fades in from the controller");
        assert!(a(w as usize - 1, 32) < 20, "nothing past the ring");
    }
}
