//! Built-in egui views used by the default panel set.
//!
//! Visual language: Terminal Brutalist chassis + Spatial Glass-Lite cards.
//! Tokens live in `theme::*`; no hardcoded colors in this module.

use crate::icons::{self, icon};
use crate::locale;
use crate::media::{self, MediaInfo};
use crate::theme;
use chrono::Local;
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

/// Mouse interaction reported by the mirror this frame. Coordinates are in
/// the monitor's pixel space (origin top-left of the monitor).
#[derive(Copy, Clone, Debug)]
pub struct MirrorHit {
    pub x: i32,
    pub y: i32,
    pub clicked: bool,
}

/// Where the overlay is anchored in space. XSOverlay-style spatial
/// states, so users migrating from XSO have the same mental model.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Follows the non-dominant hand controller.
    Wrist,
    /// Follows the HMD — always in view.
    Hmd,
    /// Pinned to a fixed world-space position. Doesn't move with
    /// the body or head.
    World,
}

impl Anchor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wrist => "Wrist",
            Self::Hmd => "HMD-locked",
            Self::World => "World-pinned",
        }
    }
}

/// Placement state the VR preview + eventual real-VR submission pipe
/// both read from. Offset coordinates are in metres, local to whatever
/// the current anchor is. This mirrors XSO's "Edit → Position" workflow:
/// pick an anchor, tweak offset, grab-to-undock (future milestone).
#[derive(Copy, Clone, Debug)]
pub struct Placement {
    pub anchor: Anchor,
    pub offset_x: f32,
    pub offset_y: f32,
    pub offset_z: f32,
    pub size_w: f32,
    pub size_h: f32,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            anchor: Anchor::Wrist,
            // Natural wrist-glance offset, same numbers the VR preview
            // used to hardcode. Tweakable by the user now.
            offset_x: -0.22,
            offset_y: 1.30,
            offset_z: -0.40,
            size_w: 0.36,
            size_h: 0.225,
        }
    }
}

pub struct DevView {
    pub frame: u64,
    pub last_fps: f32,
    pub mirror: Option<MirrorHandle>,
    pub forward_input: bool,
    pub accent: egui::Color32,
    pub show_mirror: bool,
    pub show_media: bool,
    pub show_chat: bool,
    pub show_keyboard: bool,
    /// Spatial placement of the overlay quad. The VR preview reads
    /// this every frame to position the quad; VR mode will too.
    pub placement: Placement,
    pub keyboard: crate::keyboard::Keyboard,
    pending_hit: Option<MirrorHit>,
    toasts: VecDeque<Toast>,
    notifications_received: u64,
    media: Option<MediaInfo>,
    last_media_poll: Instant,
    chat_draft: String,
    pending_chat: Option<rsoverlay_osc::ChatMessage>,
}

impl Default for DevView {
    fn default() -> Self {
        Self {
            frame: 0,
            last_fps: 0.0,
            mirror: None,
            forward_input: false,
            accent: theme::ACCENT,
            show_mirror: true,
            show_media: true,
            show_chat: true,
            show_keyboard: false,
            placement: Placement::default(),
            keyboard: crate::keyboard::Keyboard::default(),
            pending_hit: None,
            toasts: VecDeque::new(),
            notifications_received: 0,
            media: None,
            last_media_poll: Instant::now() - Duration::from_secs(10),
            chat_draft: String::new(),
            pending_chat: None,
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

    /// Take the pending mirror hit, if any. The caller is expected to drain
    /// this once per frame and forward valid hits to the input injector.
    pub fn take_mirror_hit(&mut self) -> Option<MirrorHit> {
        self.pending_hit.take()
    }

    /// Take the pending VRChat chatbox message, if any.
    pub fn take_pending_chat(&mut self) -> Option<rsoverlay_osc::ChatMessage> {
        self.pending_chat.take()
    }

    fn maybe_refresh_media(&mut self) {
        if self.last_media_poll.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.last_media_poll = Instant::now();
        self.media = media::poll();
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        self.ui_with(ctx, None);
    }

    /// Full UI with optional input injector. When `Some`, keyboard key
    /// presses inside the dashboard fire SendInput events against the
    /// OS. When `None`, the keyboard renders but keys are inert —
    /// useful for VR mode before laser input is wired, or headless
    /// rendering.
    pub fn ui_with(
        &mut self,
        ctx: &egui::Context,
        injector: Option<&rsoverlay_input::InputInjector>,
    ) {
        let now = Instant::now();
        while matches!(self.toasts.front(), Some(t) if t.expires < now) {
            self.toasts.pop_front();
        }
        self.maybe_refresh_media();

        let accent = self.accent;
        egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(&ctx.style()).fill(theme::BG_ROOT))
            .show(ctx, |ui| {
                ui.add_space(theme::GUTTER);
                clock_card(ui, accent);
                ui.add_space(theme::GUTTER);
                // Content row: left column packs all cards vertically,
                // right column holds the mirror. Default window height
                // is sized to fit every left-column card without
                // scrolling — if the user shrinks the window, bottom
                // cards clip but that's a conscious resize choice.
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(theme::COL_LEFT_WIDTH);
                        if self.show_media {
                            media_card(ui, self.media.as_ref(), accent);
                            ui.add_space(theme::GUTTER);
                        }
                        stats_card(
                            ui,
                            self.frame,
                            self.last_fps,
                            self.notifications_received,
                            accent,
                        );
                        ui.add_space(theme::GUTTER);
                        forward_toggle(ui, &mut self.forward_input, accent);
                        ui.add_space(theme::GUTTER);
                        if self.show_chat {
                            if let Some(msg) =
                                chatbox_card(ui, &mut self.chat_draft, accent)
                            {
                                self.pending_chat = Some(msg);
                            }
                            ui.add_space(theme::GUTTER);
                        }
                        appearance_card(
                            ui,
                            &mut self.accent,
                            &mut self.show_mirror,
                            &mut self.show_media,
                            &mut self.show_chat,
                            &mut self.show_keyboard,
                        );
                        ui.add_space(theme::GUTTER);
                        placement_card(ui, &mut self.placement, accent);
                    });
                    ui.add_space(theme::GUTTER);
                    if self.show_mirror {
                        ui.vertical(|ui| {
                            if let Some(hit) =
                                mirror_card(ui, self.mirror, self.forward_input, accent)
                            {
                                self.pending_hit = Some(hit);
                            }
                        });
                    }
                });
            });

        if self.show_keyboard {
            if let Some(inj) = injector {
                egui::Area::new(egui::Id::new("virtual-keyboard"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -theme::GUTTER))
                    .show(ctx, |ui| {
                        theme::card().show(ui, |ui| {
                            card_header(ui, icons::CHAT, "Keyboard", accent);
                            ui.add_space(6.0);
                            crate::keyboard::show(ui, &mut self.keyboard, inj, accent);
                        });
                    });
            }
        }

        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-theme::GUTTER, theme::GUTTER))
            .show(ctx, |ui| {
                for t in self.toasts.iter() {
                    toast_card(ui, &t.n, accent);
                    ui.add_space(8.0);
                }
            });

        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

fn clock_card(ui: &mut egui::Ui, accent: egui::Color32) {
    let now = Local::now();
    // Follow the host's clock convention: 12h with AM/PM on US-English and
    // friends, 24h everywhere else. Detected from the system locale, not
    // from chrono; chrono formats, the OS decides.
    let time = if locale::uses_24h_clock() {
        now.format("%H:%M").to_string()
    } else {
        now.format("%-I:%M %p").to_string()
    };
    let date = now.format("%A · %b %-d").to_string();
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.add(icon(icons::BRAND, 56.0, accent));
            ui.add_space(14.0);
            ui.label(theme::headline(&time));
            ui.add_space(16.0);
            ui.separator();
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add(icon(icons::CLOCK, 18.0, accent));
                    ui.add_space(6.0);
                    ui.label(theme::caption("RSOverlay"));
                });
                ui.label(theme::body_muted(&date));
            });
        });
    });
}

fn media_card(ui: &mut egui::Ui, info: Option<&MediaInfo>, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::PULSE, "Now playing", accent);
        ui.add_space(6.0);
        match info {
            Some(m) if !m.title.is_empty() => {
                ui.label(theme::card_title(&m.title));
                if !m.artist.is_empty() {
                    ui.label(theme::body_muted(&m.artist));
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(icon(
                        if m.playing { icons::PLAY } else { icons::PAUSE },
                        18.0,
                        accent,
                    ));
                    ui.add_space(6.0);
                    ui.label(theme::body_muted(if m.playing { "playing" } else { "paused" }));
                });
            }
            _ => {
                ui.label(theme::body_muted("nothing playing"));
            }
        }
    });
}

fn card_header(
    ui: &mut egui::Ui,
    source: egui::ImageSource<'static>,
    label: &str,
    accent: egui::Color32,
) {
    ui.horizontal(|ui| {
        ui.add(icon(source, 18.0, accent));
        ui.add_space(6.0);
        ui.label(theme::caption(label));
    });
}

fn stats_card(ui: &mut egui::Ui, frame_count: u64, fps: f32, notifs: u64, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Stats", accent);
        ui.add_space(6.0);
        kv(ui, "frame", &frame_count.to_string());
        kv(ui, "fps", &format!("{:.1}", fps));
        kv(ui, "notifications", &notifs.to_string());
        kv(ui, "ws bind", "127.0.0.1:42069");
    });
}

fn placement_card(ui: &mut egui::Ui, p: &mut Placement, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "Placement", accent);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            for a in [Anchor::Wrist, Anchor::Hmd, Anchor::World] {
                let selected = p.anchor == a;
                let label = egui::RichText::new(a.label())
                    .color(if selected { accent } else { theme::FG_MUTED });
                if ui.selectable_label(selected, label).clicked() {
                    p.anchor = a;
                }
            }
        });
        ui.add_space(6.0);
        placement_slider(ui, "X", &mut p.offset_x, -0.6, 0.6);
        placement_slider(ui, "Y", &mut p.offset_y, 0.8, 2.0);
        placement_slider(ui, "Z", &mut p.offset_z, -1.0, 0.0);
        ui.add_space(4.0);
        placement_slider(ui, "W", &mut p.size_w, 0.1, 0.8);
        placement_slider(ui, "H", &mut p.size_h, 0.08, 0.6);
    });
}

fn placement_slider(ui: &mut egui::Ui, label: &str, v: &mut f32, lo: f32, hi: f32) {
    ui.horizontal(|ui| {
        ui.label(theme::body_muted(label));
        ui.add(
            egui::Slider::new(v, lo..=hi)
                .fixed_decimals(2)
                .show_value(true),
        );
    });
}

fn appearance_card(
    ui: &mut egui::Ui,
    accent: &mut egui::Color32,
    show_mirror: &mut bool,
    show_media: &mut bool,
    show_chat: &mut bool,
    show_keyboard: &mut bool,
) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::SETTINGS, "Appearance", *accent);
        ui.add_space(6.0);
        // One row: all accent swatches + custom color picker, inline.
        ui.horizontal_wrapped(|ui| {
            for (name, preset) in theme::ACCENT_PRESETS {
                let selected = *accent == *preset;
                let size = egui::vec2(20.0, 20.0);
                let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                let painter = ui.painter();
                let center = rect.center();
                painter.circle_filled(center, 8.0, *preset);
                if selected {
                    painter.circle_stroke(
                        center,
                        10.0,
                        egui::Stroke::new(1.5_f32, theme::FG_PRIMARY),
                    );
                }
                resp.on_hover_text(*name).clicked().then(|| *accent = *preset);
            }
            ui.color_edit_button_srgba(accent);
        });
        ui.add_space(6.0);
        // One row: all card show/hide checkboxes side by side.
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(show_mirror, "Mirror");
            ui.checkbox(show_media, "Media");
            ui.checkbox(show_chat, "Chat");
            ui.checkbox(show_keyboard, "Keyboard");
        });
    });
}

fn chatbox_card(
    ui: &mut egui::Ui,
    draft: &mut String,
    accent: egui::Color32,
) -> Option<rsoverlay_osc::ChatMessage> {
    let mut result = None;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::CHAT, "VRChat chatbox", accent);
        ui.add_space(6.0);
        let edit = ui.add(
            egui::TextEdit::singleline(draft)
                .desired_width(f32::INFINITY)
                .hint_text("message VRChat chatbox…"),
        );
        let submit =
            (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                || ui.button(egui::RichText::new("send").color(accent)).clicked();
        if submit && !draft.trim().is_empty() {
            result = Some(rsoverlay_osc::ChatMessage {
                text: draft.clone(),
                send_immediately: true,
                play_notification: true,
            });
            draft.clear();
        }
    });
    result
}

fn forward_toggle(ui: &mut egui::Ui, on: &mut bool, accent: egui::Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::POINTER, "Input forwarding", accent);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(on, "");
            ui.label(theme::body(if *on {
                "clicks on the mirror reach the real desktop"
            } else {
                "clicks on the mirror are inert"
            }));
        });
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

fn mirror_card(
    ui: &mut egui::Ui,
    mirror: Option<MirrorHandle>,
    forward_input: bool,
    accent: egui::Color32,
) -> Option<MirrorHit> {
    let mut hit = None;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        card_header(ui, icons::MONITOR, "Desktop", accent);
        ui.add_space(6.0);
        match mirror {
            Some(m) => {
                let avail = ui.available_size();
                let aspect = m.width as f32 / m.height as f32;
                let max_w = avail.x;
                let max_h = (avail.y - 4.0).max(160.0);
                let (w, h) = fit(aspect, max_w, max_h);
                let sense = if forward_input {
                    egui::Sense::click_and_drag()
                } else {
                    egui::Sense::hover()
                };
                let response = ui.add(
                    egui::Image::new((m.id, egui::vec2(w, h))).sense(sense),
                );
                if forward_input {
                    if let Some(pos) = response.hover_pos().or_else(|| response.interact_pointer_pos()) {
                        let rect = response.rect;
                        let u = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                        let v = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                        let px = (u * m.width as f32) as i32;
                        let py = (v * m.height as f32) as i32;
                        let clicked = response.clicked();
                        hit = Some(MirrorHit { x: px, y: py, clicked });
                    }
                }
            }
            None => {
                ui.label(theme::body_muted("no monitor captured"));
            }
        }
    });
    hit
}

fn fit(aspect: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    let w_by_h = max_h * aspect;
    if w_by_h <= max_w {
        (w_by_h, max_h)
    } else {
        (max_w, max_w / aspect)
    }
}

fn toast_card(ui: &mut egui::Ui, n: &Notification, accent: egui::Color32) {
    theme::pill().show(ui, |ui| {
        ui.set_width(260.0);
        ui.horizontal(|ui| {
            ui.add(icon(icons::BELL, 26.0, accent));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                if !n.title.is_empty() {
                    ui.label(theme::card_title(&n.title));
                }
                if !n.content.is_empty() {
                    ui.label(theme::body(&n.content));
                }
                if !n.source_app.is_empty() {
                    ui.label(theme::caption(&n.source_app));
                }
            });
        });
    });
}
