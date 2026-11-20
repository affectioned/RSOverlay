//! Built-in egui views used by the default panel set.
//!
//! Visual language: Terminal Brutalist chassis + Spatial Glass-Lite cards.
//! Tokens live in `theme::*`; no hardcoded colors in this module.

use crate::media::{self, MediaInfo};
use crate::theme;
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
            .frame(egui::Frame::central_panel(&ctx.style()).fill(theme::BG_ROOT))
            .show(ctx, |ui| {
                ui.add_space(theme::GUTTER);
                clock_card(ui);
                ui.add_space(theme::GUTTER);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(theme::COL_LEFT_WIDTH);
                        media_card(ui, self.media.as_ref());
                        ui.add_space(theme::GUTTER);
                        stats_card(ui, self.frame, self.last_fps, self.notifications_received);
                    });
                    ui.add_space(theme::GUTTER);
                    ui.vertical(|ui| {
                        mirror_card(ui, self.mirror);
                    });
                });
            });

        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-theme::GUTTER, theme::GUTTER))
            .show(ctx, |ui| {
                for t in self.toasts.iter() {
                    toast_card(ui, &t.n);
                    ui.add_space(8.0);
                }
            });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn clock_card(ui: &mut egui::Ui) {
    let now = Local::now();
    let time = format!("{:02}:{:02}", now.hour(), now.minute());
    let date = now.format("%A · %b %-d").to_string();
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(theme::headline(&time));
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(theme::accent_dot());
                    ui.label(theme::caption("RSOverlay"));
                });
                ui.label(theme::body_muted(&date));
            });
        });
    });
}

fn media_card(ui: &mut egui::Ui, info: Option<&MediaInfo>) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::caption("Now playing"));
        ui.add_space(6.0);
        match info {
            Some(m) if !m.title.is_empty() => {
                ui.label(theme::card_title(&m.title));
                if !m.artist.is_empty() {
                    ui.label(theme::body_muted(&m.artist));
                }
                ui.add_space(6.0);
                let state = if m.playing { "▶  playing" } else { "⏸  paused" };
                ui.horizontal(|ui| {
                    ui.label(theme::accent_dot());
                    ui.label(theme::body_muted(state));
                });
            }
            _ => {
                ui.label(theme::body_muted("nothing playing"));
            }
        }
    });
}

fn stats_card(ui: &mut egui::Ui, frame_count: u64, fps: f32, notifs: u64) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::caption("Stats"));
        ui.add_space(6.0);
        kv(ui, "frame", &frame_count.to_string());
        kv(ui, "fps", &format!("{:.1}", fps));
        kv(ui, "notifications", &notifs.to_string());
        kv(ui, "ws bind", "127.0.0.1:42069");
    });
}

fn kv(ui: &mut egui::Ui, k: &str, v: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::body_muted(k));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(egui::RichText::new(v).color(theme::FG_PRIMARY).size(theme::SIZE_BODY));
        });
    });
}

fn mirror_card(ui: &mut egui::Ui, mirror: Option<MirrorHandle>) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::caption("Desktop"));
        ui.add_space(6.0);
        match mirror {
            Some(m) => {
                let avail = ui.available_size();
                let aspect = m.width as f32 / m.height as f32;
                let max_w = avail.x;
                let max_h = (avail.y - 4.0).max(160.0);
                let (w, h) = fit(aspect, max_w, max_h);
                ui.image((m.id, egui::vec2(w, h)));
            }
            None => {
                ui.label(theme::body_muted("no monitor captured"));
            }
        }
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
    theme::pill().show(ui, |ui| {
        ui.set_width(260.0);
        ui.vertical(|ui| {
            if !n.title.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(theme::accent_dot());
                    ui.label(theme::card_title(&n.title));
                });
            }
            if !n.content.is_empty() {
                ui.label(theme::body(&n.content));
            }
            if !n.source_app.is_empty() {
                ui.label(theme::caption(&n.source_app));
            }
        });
    });
}
