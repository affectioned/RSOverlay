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
        let tex_id = panel.renderer.register_native_texture(&gpu.device, &mirror.view, wgpu::FilterMode::Linear);
        info!(
            "mirror source: {} ({}x{} shared texture, GPU-to-GPU)",
            source.label(),
            mirror.width,
            mirror.height
        );
        Ok(Self { source, mirror, tex_id, stop, thread: Some(thread), content, monitor_geom })
    }

    pub fn handle(&self) -> MirrorHandle {
        match &self.content {
            None => MirrorHandle::full(self.tex_id, self.mirror.width, self.mirror.height),
            Some(c) => {
                let (w, h) = c.get();
                let (w, h) = (w.max(1), h.max(1));
                let uv = egui::Rect::from_min_max(
                    egui::Pos2::ZERO,
                    egui::pos2(w as f32 / self.mirror.width as f32, h as f32 / self.mirror.height as f32),
                );
                MirrorHandle { id: self.tex_id, width: w, height: h, uv }
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
