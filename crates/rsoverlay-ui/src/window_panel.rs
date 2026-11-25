//! Floating window overlay (VR): live capture + a dock toolbar.
//!
//! ```text
//! ╭────────────────────────────────────────────╮
//! │                                            │
//! │            captured window                 │
//! │      (laser = mouse on the real window)    │
//! │                                            │
//! ├────────────────────────────────────────────┤
//! │ Discord      [World][Head][L hand][R hand] ✕│
//! ╰────────────────────────────────────────────╯
//! ```

use crate::theme;
use crate::views::{CaptureSource, MirrorHandle, MirrorHit, SourcePicker};
use crate::motion;

/// Where a window overlay is docked.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Attach {
    World,
    Head,
    LeftHand,
    RightHand,
}

impl Attach {
    pub const ALL: [Attach; 4] = [Attach::World, Attach::Head, Attach::LeftHand, Attach::RightHand];

    pub fn label(self) -> &'static str {
        match self {
            Self::World => "World",
            Self::Head => "Head",
            Self::LeftHand => "L hand",
            Self::RightHand => "R hand",
        }
    }
}

#[derive(Debug, Default)]
pub struct WindowPanelAction {
    /// Laser over the content: move (and maybe click) the real window.
    pub hit: Option<MirrorHit>,
    /// Where the capture was drawn this frame (panel points) — the app
    /// maps raw laser events through it to drive the real window.
    pub image_rect: Option<egui::Rect>,
    pub attach: Option<Attach>,
    pub close: bool,
    /// Leave the edge-docked group this panel is linked into.
    pub unlink: bool,
    /// Content opacity changed on the toolbar slider.
    pub opacity: Option<f32>,
}

/// Lowest per-window content opacity: still visible enough to aim at.
pub const MIN_CONTENT_OPACITY: f32 = 0.1;

pub const TOOLBAR_H: f32 = 52.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    title: &str,
    mirror: Option<MirrorHandle>,
    attach: Attach,
    input_blocked: bool,
    icon: Option<egui::TextureId>,
    linked: bool,
    opacity: f32,
    accent: egui::Color32,
) -> WindowPanelAction {
    let mut out = WindowPanelAction::default();
    egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| {
        let full = ui.max_rect();
        let bar = egui::Rect::from_min_max(egui::pos2(full.left(), full.bottom() - TOOLBAR_H), full.max);
        let content = egui::Rect::from_min_max(full.min, egui::pos2(full.right(), bar.top() - 6.0));

        // Content: the capture, letterboxed to its aspect, centred.
        match mirror {
            Some(m) => {
                let aspect = m.width as f32 / m.height.max(1) as f32;
                let (w, h) = fit(aspect, content.width(), content.height());
                let rect = egui::Rect::from_center_size(content.center(), egui::vec2(w, h));
                // Per-window opacity fades only the content (the toolbar
                // stays solid so it can always be found): OpenVR honours
                // the texture's alpha.
                let a = opacity.clamp(MIN_CONTENT_OPACITY, 1.0);
                ui.painter().rect_filled(rect.expand(2.0), 10.0, theme::BG_ROOT.gamma_multiply(a));
                let resp = ui.put(
                    rect,
                    egui::Image::new((m.id, rect.size()))
                        .uv(m.uv)
                        .rounding(8.0)
                        .tint(egui::Color32::WHITE.gamma_multiply(a))
                        .sense(egui::Sense::click()),
                );
                crate::views::paint_mirror_cursor(ui, rect, m.cursor);
                out.image_rect = Some(rect);
                if input_blocked {
                    uac_banner(ui, rect);
                } else if let Some(pos) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) {
                    let u = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                    let v = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
                    out.hit = Some(MirrorHit {
                        x: (u * m.width as f32) as i32,
                        y: (v * m.height as f32) as i32,
                        clicked: resp.clicked(),
                    });
                }
            }
            None => {
                ui.painter().rect_filled(content, 12.0, theme::BG_CARD);
                ui.painter().text(content.center(), egui::Align2::CENTER_CENTER, "capture unavailable",
                    egui::FontId::proportional(theme::SIZE_BODY), theme::FG_MUTED);
            }
        }

        let mut op = opacity;
        (out.attach, out.close, out.unlink) = toolbar(ui, bar, title, icon, attach, linked, Some(&mut op), accent);
        if (op - opacity).abs() > f32::EPSILON {
            out.opacity = Some(op);
        }
    });
    out
}

/// A window panel's bottom toolbar: app icon + title, dock + close, and
/// Unlink while edge-docked to other windows, and the content-opacity
/// slider when `opacity` is given. Returns (dock, close, unlink).
#[allow(clippy::too_many_arguments)]
pub fn toolbar(
    ui: &mut egui::Ui,
    bar: egui::Rect,
    title: &str,
    icon: Option<egui::TextureId>,
    attach: Attach,
    linked: bool,
    opacity: Option<&mut f32>,
    accent: egui::Color32,
) -> (Option<Attach>, bool, bool) {
    ui.painter().rect_filled(bar, 14.0, theme::BG_ROOT);
    ui.painter().rect_stroke(bar, 14.0, egui::Stroke::new(1.0_f32, theme::STROKE_CARD));
    let mut bar_ui = ui.child_ui(bar.shrink2(egui::vec2(16.0, 8.0)), egui::Layout::left_to_right(egui::Align::Center));
    if let Some(id) = icon {
        bar_ui.add(egui::Image::new((id, egui::vec2(24.0, 24.0))));
    }
    bar_ui.add(egui::Label::new(egui::RichText::new(title).color(theme::FG_PRIMARY).size(theme::SIZE_BODY)).truncate(true));
    let mut out = (None, false, false);
    bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        (out.0, out.1) = dock_controls(ui, attach, accent);
        if linked {
            ui.add_space(8.0);
            out.2 = unlink_pill(ui, accent);
        }
        if let Some(op) = opacity {
            ui.add_space(12.0);
            ui.spacing_mut().slider_width = 120.0;
            let mut pct = (*op * 100.0).round();
            let lo = (MIN_CONTENT_OPACITY * 100.0).round();
            if ui
                .add(egui::Slider::new(&mut pct, lo..=100.0).fixed_decimals(0).suffix("%").show_value(true))
                .on_hover_text("Window opacity: see your game world through this window (the toolbar stays solid)")
                .changed()
            {
                *op = pct / 100.0;
            }
            ui.label(theme::caption("Opacity"));
        }
    });
    out
}

/// Shown over an elevated window's capture when RSOverlay isn't elevated:
/// Windows (UIPI) drops our input, so say so instead of failing silently.
fn uac_banner(ui: &egui::Ui, content: egui::Rect) {
    let w = (content.width() - 40.0).min(720.0);
    let r = egui::Rect::from_min_size(egui::pos2(content.center().x - w / 2.0, content.top() + 20.0), egui::vec2(w, 92.0));
    let p = ui.painter();
    p.rect_filled(r, 14.0, egui::Color32::from_black_alpha(230));
    p.rect_stroke(r, 14.0, egui::Stroke::new(1.5_f32, theme::DANGER));
    p.text(r.center_top() + egui::vec2(0.0, 26.0), egui::Align2::CENTER_CENTER,
        "This window is running as administrator",
        egui::FontId::proportional(theme::SIZE_CARD_TITLE), theme::DANGER);
    p.text(r.center_top() + egui::vec2(0.0, 58.0), egui::Align2::CENTER_CENTER,
        "Windows (UAC) blocks VR input to it — use your PC's mouse/keyboard,",
        egui::FontId::proportional(theme::SIZE_BODY), theme::FG_PRIMARY);
    p.text(r.center_top() + egui::vec2(0.0, 78.0), egui::Align2::CENTER_CENTER,
        "or enable Settings → Administrator → Run as administrator.",
        egui::FontId::proportional(theme::SIZE_BODY), theme::FG_MUTED);
}

/// The controls every floating panel carries (windows, widgets, the
/// dashboard; not the keyboard): dock target + close. Call inside a
/// right-to-left layout. Returns (dock request, close).
pub fn dock_controls(ui: &mut egui::Ui, attach: Attach, accent: egui::Color32) -> (Option<Attach>, bool) {
    let mut out = (None, false);
    if pill(ui, "✕", false, theme::DANGER).on_hover_text("Close").clicked() {
        out.1 = true;
    }
    ui.add_space(8.0);
    for a in Attach::ALL.into_iter().rev() {
        if pill(ui, a.label(), attach == a, accent).on_hover_text(format!("Dock to {}", a.label())).clicked() && attach != a {
            out.0 = Some(a);
        }
    }
    ui.label(theme::caption("Dock"));
    out
}

/// Shown on a panel that's edge-docked to others. Returns clicked.
pub fn unlink_pill(ui: &mut egui::Ui, accent: egui::Color32) -> bool {
    pill(ui, "Unlink", true, accent)
        .on_hover_text("Docked to the panels beside it — they move together. Unlink to move this one alone.")
        .clicked()
}

pub fn pill(ui: &mut egui::Ui, text: &str, on: bool, accent: egui::Color32) -> egui::Response {
    let glyph = crate::glyphs::Glyph::from_label(text);
    let font = egui::FontId::proportional(theme::SIZE_CAPTION + 2.0);
    let galley = ui.painter().layout_no_wrap(if glyph.is_some() { String::new() } else { text.to_string() }, font, theme::FG_PRIMARY);
    let size = egui::vec2(if glyph.is_some() { 34.0 } else { galley.size().x + 24.0 }, 34.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
    let sel = motion::toggle(ui.ctx(), resp.id.with("on"), on, motion::MEDIUM);
    let base = motion::mix(crate::theme::BG_RAISED, crate::theme::BG_HOVER, hover);
    ui.painter().rect_filled(rect, 17.0, motion::mix(base, accent.gamma_multiply(0.25), sel));
    let color = motion::mix(theme::FG_MUTED, if on { accent } else { theme::FG_PRIMARY }, sel.max(hover));
    match glyph {
        Some(g) => crate::glyphs::draw(ui.painter(), g, rect.center(), 14.0, color),
        None => ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color),
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn fit(aspect: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    if max_h * aspect <= max_w {
        (max_h * aspect, max_h)
    } else {
        (max_w, max_w / aspect)
    }
}

/// VR Desktop page: every capturable window + monitor as a big tile;
/// clicking one requests it as a floating window overlay.
pub fn source_grid(ui: &mut egui::Ui, sources: &mut SourcePicker, open: &[CaptureSource], accent: egui::Color32) {
    ui.label(theme::body_muted("Pick a window or monitor — it opens floating in front of you. Grip to carry it, dock it from its toolbar."));
    ui.add_space(theme::GUTTER);
    if sources.choices.is_empty() {
        ui.label(theme::body_muted("looking for windows…"));
        return;
    }
    let cols = 3usize;
    let gap = theme::GUTTER;
    let w = (ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32;
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut sources.filter).hint_text("Filter windows…").desired_width(320.0));
        if !sources.filter.is_empty() && pill(ui, "✕", false, accent).on_hover_text("Clear").clicked() {
            sources.filter.clear();
        }
    });
    ui.add_space(theme::GUTTER);
    let needle = sources.filter.trim().to_lowercase();
    let choices: Vec<CaptureSource> =
        sources.choices.iter().filter(|s| needle.is_empty() || s.label().to_lowercase().contains(&needle)).cloned().collect();
    if !needle.is_empty() {
        ui.label(egui::RichText::new(format!("{} of {}", choices.len(), sources.choices.len())).size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
        ui.add_space(6.0);
    }
    for row in choices.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for src in row {
                let is_open = open.contains(src);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 64.0), egui::Sense::click());
                let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
                let fill = motion::mix(theme::BG_CARD, crate::theme::BG_HOVER, hover);
                ui.painter().rect_filled(rect, 14.0, fill);
                if is_open {
                    ui.painter().rect_stroke(rect, 14.0, egui::Stroke::new(1.5_f32, accent));
                }
                let icon_r = egui::Rect::from_center_size(rect.left_center() + egui::vec2(30.0, 0.0), egui::vec2(32.0, 32.0));
                let label = match src {
                    CaptureSource::Monitor(i) => {
                        crate::icons::icon(crate::icons::MONITOR, 28.0, if is_open { accent } else { theme::FG_MUTED })
                            .paint_at(ui, icon_r.shrink(2.0));
                        format!("Monitor {i}")
                    }
                    CaptureSource::Window { hwnd, title } => {
                        match sources.icon_texture(ui.ctx(), *hwnd) {
                            Some(id) => egui::Image::new((id, icon_r.size())).paint_at(ui, icon_r),
                            None => crate::icons::icon(crate::icons::MONITOR, 28.0, theme::FG_MUTED).paint_at(ui, icon_r.shrink(2.0)),
                        }
                        title.clone()
                    }
                };
                let painter = ui.painter_at(rect.shrink(12.0));
                // Titles carry symbols the font lacks (terminal spinners
                // like ◑ ✳) — drop those rather than draw empty boxes.
                let font = egui::FontId::proportional(theme::SIZE_BODY);
                let label: String = ui.fonts(|f| label.chars().filter(|c| f.has_glyph(&font, *c)).collect());
                painter.text(rect.left_center() + egui::vec2(56.0, 0.0), egui::Align2::LEFT_CENTER,
                    shell_truncate(label.trim(), 28), font, theme::FG_PRIMARY);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !is_open {
                    sources.request = Some(src.clone());
                }
            }
        });
        ui.add_space(gap);
    }
}

fn shell_truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}
