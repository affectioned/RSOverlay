//! Built-in egui views used by the default panel set.

use rsoverlay_notify::Notification;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Handle to a wgpu-backed egui texture plus its intrinsic size.
#[derive(Copy, Clone, Debug)]
pub struct MirrorHandle {
    pub id: egui::TextureId,
    pub width: u32,
    pub height: u32,
}

/// A notification that is currently on screen.
struct Toast {
    n: Notification,
    expires: Instant,
}

/// State for the dev/diagnostics view shown in the P0 preview overlay.
/// Also owns the live toast stack so the UI stays one place in this
/// milestone.
pub struct DevView {
    pub frame: u64,
    pub last_fps: f32,
    pub message: String,
    pub mirror: Option<MirrorHandle>,
    toasts: VecDeque<Toast>,
    notifications_received: u64,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
            message: "RSOverlay preview".to_string(),
            mirror: None,
            toasts: VecDeque::new(),
            notifications_received: 0,
        }
    }
}

impl DevView {
    /// Enqueue a notification from the websocket server.
    pub fn push_notification(&mut self, n: Notification) {
        self.notifications_received += 1;
        let timeout = n.timeout.clamp(0.5, 30.0);
        let expires = Instant::now() + Duration::from_secs_f32(timeout);
        self.toasts.push_back(Toast { n, expires });
        while self.toasts.len() > 8 {
            self.toasts.pop_front();
        }
    }

    pub fn notifications_received(&self) -> u64 {
        self.notifications_received
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        // Cull expired toasts first.
        let now = Instant::now();
        while matches!(self.toasts.front(), Some(t) if t.expires < now) {
            self.toasts.pop_front();
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                ui.heading("RSOverlay");
                ui.add_space(4.0);
                ui.label(egui::RichText::new(&self.message).weak());
                ui.add_space(8.0);
                if let Some(m) = self.mirror {
                    let avail = ui.available_size();
                    let mirror_aspect = m.width as f32 / m.height as f32;
                    let max_w = avail.x * 0.9;
                    let max_h = (avail.y - 160.0).max(120.0);
                    let (w, h) = fit(mirror_aspect, max_w, max_h);
                    ui.image((m.id, egui::vec2(w, h)));
                    ui.add_space(8.0);
                }
                ui.separator();
                ui.add_space(6.0);
                egui::Grid::new("dev-grid")
                    .num_columns(2)
                    .spacing([24.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("frame");
                        ui.monospace(self.frame.to_string());
                        ui.end_row();
                        ui.label("fps");
                        ui.monospace(format!("{:.1}", self.last_fps));
                        ui.end_row();
                        ui.label("mode");
                        ui.monospace("preview");
                        ui.end_row();
                        ui.label("notifications");
                        ui.monospace(self.notifications_received.to_string());
                        ui.end_row();
                        ui.label("ws bind");
                        ui.monospace("127.0.0.1:42069");
                        ui.end_row();
                        if let Some(m) = self.mirror {
                            ui.label("mirror");
                            ui.monospace(format!("{}x{}", m.width, m.height));
                            ui.end_row();
                        }
                    });
            });
        });

        // Toast stack in the top-right.
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .show(ctx, |ui| {
                for t in self.toasts.iter() {
                    toast_card(ui, &t.n);
                    ui.add_space(8.0);
                }
            });

        if !self.toasts.is_empty() {
            // Keep repainting while a toast is live so it auto-dismisses on
            // time instead of waiting for the next input event.
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

fn fit(aspect: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    let w_by_h = max_h * aspect;
    if w_by_h <= max_w {
        (w_by_h, max_h)
    } else {
        (max_w, max_w / aspect)
    }
}

fn toast_card(ui: &mut egui::Ui, n: &Notification) {
    egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(28, 28, 32))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(12.0, 8.0))
        .show(ui, |ui| {
            ui.set_width(260.0);
            ui.vertical(|ui| {
                if !n.title.is_empty() {
                    ui.label(
                        egui::RichText::new(&n.title)
                            .strong()
                            .color(egui::Color32::WHITE),
                    );
                }
                if !n.content.is_empty() {
                    ui.label(
                        egui::RichText::new(&n.content)
                            .color(egui::Color32::from_gray(200)),
                    );
                }
                if !n.source_app.is_empty() {
                    ui.label(
                        egui::RichText::new(&n.source_app)
                            .small()
                            .color(egui::Color32::from_gray(140)),
                    );
                }
            });
        });
}
