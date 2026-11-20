//! Built-in egui views used by the default panel set.

use crate::media::{self, MediaInfo};
use chrono::{Local, Timelike};
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

struct Toast {
    n: Notification,
    expires: Instant,
}

pub struct DevView {
    pub frame: u64,
    pub last_fps: f32,
    pub mirror: Option<MirrorHandle>,
    toasts: VecDeque<Toast>,
    notifications_received: u64,
    media: Option<MediaInfo>,
    last_media_poll: Instant,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
            mirror: None,
            toasts: VecDeque::new(),
            notifications_received: 0,
            media: None,
            last_media_poll: Instant::now() - Duration::from_secs(10),
        }
    }
}

impl DevView {
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

    fn maybe_refresh_media(&mut self) {
        // Poll SMTC at most twice a second. The `get()` call blocks on the
        // async op which isn't free on every frame.
        if self.last_media_poll.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_media_poll = Instant::now();
        self.media = media::poll();
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        while matches!(self.toasts.front(), Some(t) if t.expires < now) {
            self.toasts.pop_front();
        }
        self.maybe_refresh_media();

        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::from_rgb(14, 14, 18)))
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    clock_bar(ui);
                    ui.add_space(6.0);
                    ui.horizontal_top(|ui| {
                        // Left: media + stats stack
                        ui.vertical(|ui| {
                            ui.set_width(300.0);
                            media_card(ui, self.media.as_ref());
                            ui.add_space(8.0);
                            stats_card(ui, self.frame, self.last_fps, self.notifications_received);
                        });
                        ui.add_space(12.0);
                        // Right: mirror
                        ui.vertical(|ui| {
                            if let Some(m) = self.mirror {
                                let avail = ui.available_size();
                                let aspect = m.width as f32 / m.height as f32;
                                let max_w = avail.x;
                                let max_h = (avail.y - 20.0).max(160.0);
                                let (w, h) = fit(aspect, max_w, max_h);
                                let frame = egui::Frame::group(ui.style())
                                    .fill(egui::Color32::from_rgb(20, 20, 24))
                                    .rounding(6.0)
                                    .inner_margin(egui::Margin::same(6.0));
                                frame.show(ui, |ui| {
                                    ui.image((m.id, egui::vec2(w, h)));
                                });
                            } else {
                                ui.weak("no desktop mirror");
                            }
                        });
                    });
                });
            });

        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .show(ctx, |ui| {
                for t in self.toasts.iter() {
                    toast_card(ui, &t.n);
                    ui.add_space(8.0);
                }
            });

        // The clock and toast timers need to repaint even without input.
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn clock_bar(ui: &mut egui::Ui) {
    let now = Local::now();
    let time = format!("{:02}:{:02}", now.hour(), now.minute());
    let date = now.format("%A · %b %-d").to_string();
    let frame = egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(20, 20, 24))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(16.0, 10.0));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(time)
                    .size(42.0)
                    .color(egui::Color32::WHITE)
                    .strong(),
            );
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("RSOverlay").strong().color(egui::Color32::from_gray(220)));
                ui.label(egui::RichText::new(date).color(egui::Color32::from_gray(160)));
            });
        });
    });
}

fn media_card(ui: &mut egui::Ui, info: Option<&MediaInfo>) {
    let frame = egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(20, 20, 24))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("NOW PLAYING").small().color(egui::Color32::from_gray(150)));
            ui.add_space(4.0);
            match info {
                Some(m) if !m.title.is_empty() => {
                    let state = if m.playing { "▶ playing" } else { "⏸ paused" };
                    ui.label(
                        egui::RichText::new(&m.title)
                            .strong()
                            .color(egui::Color32::WHITE)
                            .size(16.0),
                    );
                    if !m.artist.is_empty() {
                        ui.label(egui::RichText::new(&m.artist).color(egui::Color32::from_gray(200)));
                    }
                    ui.label(egui::RichText::new(state).small().color(egui::Color32::from_gray(140)));
                }
                _ => {
                    ui.label(egui::RichText::new("nothing playing").weak());
                }
            }
        });
    });
}

fn stats_card(ui: &mut egui::Ui, frame_count: u64, fps: f32, notifs: u64) {
    let frame = egui::Frame::group(ui.style())
        .fill(egui::Color32::from_rgb(20, 20, 24))
        .rounding(6.0)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.vertical(|ui| {
            ui.label(egui::RichText::new("STATS").small().color(egui::Color32::from_gray(150)));
            ui.add_space(4.0);
            egui::Grid::new("stats-grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.label("frame");
                    ui.monospace(frame_count.to_string());
                    ui.end_row();
                    ui.label("fps");
                    ui.monospace(format!("{:.1}", fps));
                    ui.end_row();
                    ui.label("notifications");
                    ui.monospace(notifs.to_string());
                    ui.end_row();
                    ui.label("ws bind");
                    ui.monospace("127.0.0.1:42069");
                    ui.end_row();
                });
        });
    });
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
