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
    /// Pin pill clicked (stays visible when Windows hides the rest).
    pub toggle_pin: bool,
    /// Resize the real window to (w, h) pixels (Size menu).
    pub resize: Option<(u32, u32)>,
    /// "Keep smooth when not looking" toggled.
    pub toggle_smooth: bool,
    /// "Readable size": size the panel so its text is legible.
    pub readable: bool,
    /// Tilt pill clicked (panels rotate with the controller while held).
    pub toggle_tilt: bool,
    /// Snap pill clicked (this window joins others edge to edge).
    pub toggle_snap: bool,
}

/// Window sizes on the toolbar's Size menu (outer size, pixels).
pub const SIZE_PRESETS: [(u32, u32); 4] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1440)];

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
    pinned: Option<bool>,
    show_toolbar: bool,
    smooth: bool,
    tilt: Option<bool>,
    snap: Option<bool>,
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
                let rect = snap_to_texels(egui::Rect::from_center_size(content.center(), egui::vec2(w, h)), ui.ctx().pixels_per_point());
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
                theme::mark_hit_area(ui.ctx(), rect.expand(2.0));
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
        // Window menu: smooth refresh for any capture; sizes only for
        // real windows Windows lets us touch.
        let menu = mirror.map(|m| WindowMenu { resizable: m.resizable && !input_blocked, smooth, tilt, snap });
        // Auto-hidden toolbars leave their strip empty (no layout jump).
        if show_toolbar {
            let t = toolbar(ui, bar, title, icon, attach, linked, pinned, menu, Some(&mut op), accent);
            (out.attach, out.close, out.unlink, out.toggle_pin, out.resize, out.toggle_smooth, out.readable, out.toggle_tilt, out.toggle_snap) =
                (t.attach, t.close, t.unlink, t.toggle_pin, t.resize, t.toggle_smooth, t.readable, t.toggle_tilt, t.toggle_snap);
        }
        if (op - opacity).abs() > f32::EPSILON {
            out.opacity = Some(op);
        }
    });
    out
}

/// Put a rect's corners on whole texels. At native resolution the
/// capture is then drawn one source pixel per texel, every sample at a
/// texel centre: an exact copy, not a blend of neighbours.
pub fn snap_to_texels(r: egui::Rect, ppp: f32) -> egui::Rect {
    let snap = |v: f32| (v * ppp).round() / ppp;
    egui::Rect::from_min_max(egui::pos2(snap(r.min.x), snap(r.min.y)), egui::pos2(snap(r.max.x), snap(r.max.y)))
}

#[cfg(test)]
mod snap_tests {
    #[test]
    fn capture_rect_lands_on_whole_texels() {
        let ppp = 2.0;
        let r = super::snap_to_texels(egui::Rect::from_min_size(egui::pos2(0.3, 10.77), egui::vec2(1280.0, 720.1)), ppp);
        for v in [r.min.x, r.min.y, r.max.x, r.max.y] {
            assert_eq!((v * ppp).fract(), 0.0, "{v}");
        }
        assert_eq!(r.width() * ppp, 2560.0);
    }
}

/// The toolbar's Window menu: what it offers for this panel.
#[derive(Copy, Clone, Debug)]
pub struct WindowMenu {
    /// A real window we may resize (Size presets).
    pub resizable: bool,
    /// "Keep smooth when not looking" is on.
    pub smooth: bool,
    /// Tilt pill: panels rotate with the controller while held (Some =
    /// shown, with its state).
    pub tilt: Option<bool>,
    /// Snap pill: this window edge-docks to others (Some = shown).
    pub snap: Option<bool>,
}

/// What the toolbar's controls asked for this frame.
#[derive(Debug, Default)]
pub struct ToolbarOut {
    pub attach: Option<Attach>,
    pub close: bool,
    pub unlink: bool,
    pub toggle_pin: bool,
    pub resize: Option<(u32, u32)>,
    /// "Keep smooth when not looking" toggled.
    pub toggle_smooth: bool,
    /// "Readable size" picked.
    pub readable: bool,
    /// Tilt pill clicked.
    pub toggle_tilt: bool,
    /// Snap pill clicked.
    pub toggle_snap: bool,
}

/// A window panel's bottom toolbar: app icon + title, dock + close, and
/// Unlink while edge-docked to other windows, Pin (`pinned` given), the
/// Size menu (`resizable`) and the content-opacity slider (`opacity`
/// given).
#[allow(clippy::too_many_arguments)]
pub fn toolbar(
    ui: &mut egui::Ui,
    bar: egui::Rect,
    title: &str,
    icon: Option<egui::TextureId>,
    attach: Attach,
    linked: bool,
    pinned: Option<bool>,
    menu: Option<WindowMenu>,
    opacity: Option<&mut f32>,
    accent: egui::Color32,
) -> ToolbarOut {
    // The toolbar and the strip above it (no laser gap between the
    // window and its bar).
    theme::mark_hit_area(ui.ctx(), bar.expand2(egui::vec2(0.0, 4.0)).with_min_y(bar.top() - 8.0));
    ui.painter().rect_filled(bar, 14.0, theme::BG_ROOT);
    ui.painter().rect_stroke(bar, 14.0, egui::Stroke::new(1.0_f32, theme::STROKE_CARD));
    let mut bar_ui = ui.child_ui(bar.shrink2(egui::vec2(16.0, 8.0)), egui::Layout::left_to_right(egui::Align::Center));
    if let Some(id) = icon {
        bar_ui.add(egui::Image::new((id, egui::vec2(24.0, 24.0))));
    }
    bar_ui.add(egui::Label::new(egui::RichText::new(title).color(theme::FG_PRIMARY).size(theme::SIZE_BODY)).truncate(true));
    let mut out = ToolbarOut::default();
    bar_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        (out.attach, out.close, out.toggle_pin) = dock_controls(ui, attach, pinned, accent);
        if linked {
            ui.add_space(8.0);
            out.unlink = unlink_pill(ui, accent);
        }
        if let Some(on) = menu.and_then(|m| m.tilt) {
            ui.add_space(8.0);
            let tip = if on {
                "Tilt on: a held window rotates with your controller (twist or tilt your wrist). Click to have windows face you instead."
            } else {
                "Tilt off: a held window always turns to face you. Click to rotate windows with your controller."
            };
            if pill(ui, "Tilt", on, accent).on_hover_text(tip).clicked() {
                out.toggle_tilt = true;
            }
        }
        if let Some(on) = menu.and_then(|m| m.snap) {
            ui.add_space(8.0);
            let tip = if on {
                "Snap on: this window joins other windows edge to edge when you drop it beside them. Click to keep it apart."
            } else {
                "Snap off: this window never joins other windows. Click to let it snap edge to edge again."
            };
            if pill(ui, "Snap", on, accent).on_hover_text(tip).clicked() {
                out.toggle_snap = true;
            }
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
        if let Some(m) = menu {
            ui.add_space(12.0);
            (out.resize, out.toggle_smooth, out.readable) = window_menu(ui, m);
        }
    });
    out
}

/// "Window" menu: keep this panel at full frame rate when you look
/// away, and (real windows) resize the window so its text reads well at
/// the panel's size. Returns (size picked, smooth toggled, readable).
fn window_menu(ui: &mut egui::Ui, m: WindowMenu) -> (Option<(u32, u32)>, bool, bool) {
    let (mut pick, mut toggle, mut readable) = (None, false, false);
    ui.menu_button("Window", |ui| {
        if ui
            .button("Readable size")
            .on_hover_text("Make this panel big enough to read its text from where you are (up to 90° of your view)")
            .clicked()
        {
            readable = true;
            ui.close_menu();
        }
        let mut on = m.smooth;
        if ui
            .checkbox(&mut on, "Keep smooth when not looking")
            .on_hover_text("Full frame rate even out of the corner of your eye (videos, a game you watch). Uses a little more GPU.")
            .changed()
        {
            toggle = true;
            ui.close_menu();
        }
        if m.resizable {
            ui.separator();
            ui.label(theme::caption("Resize the real window"));
            for (w, h) in SIZE_PRESETS {
                if ui.button(format!("{w} × {h}")).clicked() {
                    pick = Some((w, h));
                    ui.close_menu();
                }
            }
        }
    })
    .response
    .on_hover_text("Refresh and size options for this window");
    (pick, toggle, readable)
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
/// right-to-left layout. `pinned`: show the Pin pill (stays up when the Windows button hides the rest;
/// `None` = not applicable). Returns (dock request, close, pin toggled).
pub fn dock_controls(ui: &mut egui::Ui, attach: Attach, pinned: Option<bool>, accent: egui::Color32) -> (Option<Attach>, bool, bool) {
    dock_controls_with(ui, attach, pinned, PIN_TIPS_WINDOW, accent)
}

/// Pin tooltips (pinned, not pinned) for a window / widget.
pub const PIN_TIPS_WINDOW: (&str, &str) = (
    "Pinned: stays visible when the wrist Windows button hides your windows. Click to unpin.",
    "Pin to keep this one visible when the wrist Windows button hides your windows.",
);
/// Pin tooltips for the dashboard.
pub const PIN_TIPS_DASHBOARD: (&str, &str) = (
    "Pinned: the dashboard reopens right here instead of in front of you. Click to unpin.",
    "Pin to have the dashboard reopen right here instead of in front of you.",
);

/// [`dock_controls`] with the Pin pill's own tooltips (pinned, unpinned).
pub fn dock_controls_with(ui: &mut egui::Ui, attach: Attach, pinned: Option<bool>, tips: (&str, &str), accent: egui::Color32) -> (Option<Attach>, bool, bool) {
    let mut out = (None, false, false);
    if pill(ui, "✕", false, theme::DANGER).on_hover_text("Close").clicked() {
        out.1 = true;
    }
    ui.add_space(8.0);
    if let Some(on) = pinned {
        let tip = if on { tips.0 } else { tips.1 };
        if pill(ui, if on { "Pinned" } else { "Pin" }, on, accent).on_hover_text(tip).clicked() {
            out.2 = true;
        }
        ui.add_space(8.0);
    }
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

/// Desktop-picker tile height: a preview area plus a caption row.
const TILE_H: f32 = 190.0;

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
        let monitors = sources.choices.iter().filter(|s| matches!(s, CaptureSource::Monitor(_))).count();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !sources.hidden_apps.is_empty() {
                let label = if sources.show_hidden { "Hide hidden".to_string() } else { format!("Show hidden ({})", sources.hidden_apps.len()) };
                if pill(ui, &label, sources.show_hidden, accent)
                    .on_hover_text("Apps you hid from this list: show them again to open or unhide one")
                    .clicked()
                {
                    sources.show_hidden = !sources.show_hidden;
                }
            }
            if monitors > 1
                && pill(ui, "All monitors", false, accent)
                    .on_hover_text("Open every monitor at once, arranged in an arc around you (left to right as on your desk)")
                    .clicked()
            {
                sources.wall_request = true;
            }
            if monitors > 0
                && pill(ui, "Big screen", false, accent)
                    .on_hover_text("Your main monitor as one large curved screen in front of you, pinned")
                    .clicked()
            {
                sources.big_screen_request = true;
            }
        });
    });
    ui.add_space(theme::GUTTER);
    let needle = sources.filter.trim().to_lowercase();
    // Snapshots: the grid below needs `sources` mutably.
    let (apps, hidden_apps, show_hidden) = (sources.apps.clone(), sources.hidden_apps.clone(), sources.show_hidden);
    let app_of = |s: &CaptureSource| match s {
        CaptureSource::Window { hwnd, .. } => apps.get(hwnd).cloned(),
        _ => None,
    };
    let is_hidden = |s: &CaptureSource| app_of(s).is_some_and(|a| hidden_apps.contains(&a));
    let choices: Vec<CaptureSource> = sources
        .choices
        .iter()
        .filter(|s| needle.is_empty() || s.label().to_lowercase().contains(&needle))
        .filter(|s| show_hidden || !is_hidden(s))
        .cloned()
        .collect();
    // (tile, hide / unhide) asked for this frame.
    let mut toggle_app: Option<String> = None;
    if !needle.is_empty() {
        ui.label(egui::RichText::new(format!("{} of {}", choices.len(), sources.choices.len())).size(theme::SIZE_CAPTION + 1.0).color(theme::FG_MUTED));
        ui.add_space(6.0);
    }
    for row in choices.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for src in row {
                let is_open = open.contains(src);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, TILE_H), egui::Sense::click());
                let hover = motion::toggle(ui.ctx(), resp.id, resp.hovered(), motion::FAST);
                let fill = motion::mix(theme::BG_CARD, crate::theme::BG_HOVER, hover);
                ui.painter().rect_filled(rect, 14.0, fill);
                if is_open {
                    ui.painter().rect_stroke(rect, 14.0, egui::Stroke::new(1.5_f32, accent));
                }
                // Preview on top, icon + title along the bottom.
                let preview = egui::Rect::from_min_max(rect.min + egui::vec2(10.0, 10.0), egui::pos2(rect.right() - 10.0, rect.bottom() - 52.0));
                let caption = rect.left_bottom() + egui::vec2(0.0, -26.0);
                let icon_r = egui::Rect::from_center_size(caption + egui::vec2(28.0, 0.0), egui::vec2(26.0, 26.0));
                ui.painter().rect_filled(preview, 10.0, theme::BG_ROOT);
                let label = match src {
                    CaptureSource::Monitor(i) => {
                        let tint = if is_open { accent } else { theme::FG_MUTED };
                        crate::icons::icon(crate::icons::MONITOR, 48.0, tint)
                            .paint_at(ui, egui::Rect::from_center_size(preview.center(), egui::vec2(48.0, 48.0)));
                        crate::icons::icon(crate::icons::MONITOR, 22.0, tint).paint_at(ui, icon_r.shrink(2.0));
                        crate::views::monitor_label(*i)
                    }
                    CaptureSource::Window { hwnd, title } => {
                        let icon = sources.icon_texture(ui.ctx(), *hwnd);
                        match sources.thumb_texture(ui.ctx(), *hwnd) {
                            Some((id, size)) => {
                                let (tw, th) = fit(size.x / size.y.max(1.0), preview.width(), preview.height());
                                let r = egui::Rect::from_center_size(preview.center(), egui::vec2(tw, th));
                                egui::Image::new((id, r.size())).rounding(8.0).paint_at(ui, r);
                            }
                            // Minimised / not captured yet: the app icon, large.
                            None => match icon {
                                Some(id) => egui::Image::new((id, egui::vec2(48.0, 48.0)))
                                    .paint_at(ui, egui::Rect::from_center_size(preview.center(), egui::vec2(48.0, 48.0))),
                                None => crate::icons::icon(crate::icons::MONITOR, 48.0, theme::FG_FAINT)
                                    .paint_at(ui, egui::Rect::from_center_size(preview.center(), egui::vec2(48.0, 48.0))),
                            },
                        }
                        match icon {
                            Some(id) => egui::Image::new((id, icon_r.size())).paint_at(ui, icon_r),
                            None => crate::icons::icon(crate::icons::MONITOR, 22.0, theme::FG_MUTED).paint_at(ui, icon_r.shrink(2.0)),
                        }
                        title.clone()
                    }
                };
                let painter = ui.painter_at(rect.shrink(10.0));
                // Titles carry symbols the font lacks (terminal spinners
                // like ◑ ✳) — drop those rather than draw empty boxes.
                let font = egui::FontId::proportional(theme::SIZE_BODY);
                let label: String = ui.fonts(|f| label.chars().filter(|c| f.has_glyph(&font, *c)).collect());
                painter.text(caption + egui::vec2(50.0, 0.0), egui::Align2::LEFT_CENTER,
                    shell_truncate(label.trim(), 28), font, theme::FG_PRIMARY);
                // Hide / unhide this app from the list (top-right of the tile).
                let mut hide_clicked = false;
                if let Some(app) = app_of(src) {
                    let hidden = hidden_apps.contains(&app);
                    if hidden || resp.hovered() {
                        let r = egui::Rect::from_min_size(egui::pos2(rect.right() - 86.0, rect.top() + 14.0), egui::vec2(72.0, 30.0));
                        let mut sub = ui.child_ui(r, egui::Layout::right_to_left(egui::Align::Center));
                        let tip = if hidden { format!("Show {app} in this list again") } else { format!("Hide every {app} window from this list") };
                        if pill(&mut sub, if hidden { "Unhide" } else { "Hide" }, hidden, accent).on_hover_text(tip).clicked() {
                            hide_clicked = true;
                            toggle_app = Some(app);
                        }
                    }
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !is_open && !hide_clicked {
                    sources.request = Some(src.clone());
                }
            }
        });
        ui.add_space(gap);
    }
    if let Some(app) = toggle_app {
        match sources.hidden_apps.iter().position(|a| *a == app) {
            Some(i) => {
                sources.hidden_apps.remove(i);
            }
            None if sources.hidden_apps.len() < 100 => sources.hidden_apps.push(app),
            None => {}
        }
        if sources.hidden_apps.is_empty() {
            sources.show_hidden = false;
        }
    }
}

fn shell_truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}
