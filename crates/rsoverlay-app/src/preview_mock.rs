//! Stand-in window contents for the VR preview scenes. Real captures
//! would put whatever is on the user's desktop into screenshots, so the
//! preview paints a few generic, unbranded "apps" instead: an editor,
//! notes, a terminal and a game guide.

use egui::{pos2, vec2, Color32, FontId, Rect};
use rsoverlay_ui::theme;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MockApp {
    Editor,
    Notes,
    Terminal,
    Guide,
}

const KW: Color32 = Color32::from_rgb(0xC6, 0x9C, 0xE6);
const TY: Color32 = Color32::from_rgb(0x7D, 0xD3, 0xC0);
const FN: Color32 = Color32::from_rgb(0x82, 0xB1, 0xFF);
const ST: Color32 = Color32::from_rgb(0xB2, 0xD9, 0x7C);
const CM: Color32 = Color32::from_rgb(0x6B, 0x6B, 0x75);
const TX: Color32 = Color32::from_rgb(0xD8, 0xD8, 0xDE);
const NU: Color32 = Color32::from_rgb(0xE6, 0xB3, 0x5C);

impl MockApp {
    pub fn title(self) -> &'static str {
        match self {
            Self::Editor => "Editor: panel.rs",
            Self::Notes => "Notes: sprint plan",
            Self::Terminal => "Terminal",
            Self::Guide => "Guide: Act III bosses",
        }
    }

    pub fn paint(self, ui: &egui::Ui, r: Rect) {
        let p = ui.painter_at(r);
        p.rect_filled(r, 10.0, Color32::from_rgb(0x0E, 0x0E, 0x11));
        p.rect_stroke(r, 10.0, egui::Stroke::new(1.0_f32, theme::STROKE_CARD));
        match self {
            Self::Editor => editor(&p, r),
            Self::Notes => notes(&p, r),
            Self::Terminal => terminal(&p, r),
            Self::Guide => guide(&p, r),
        }
    }
}

fn spans(p: &egui::Painter, at: egui::Pos2, parts: &[(&str, Color32)], font: &FontId) {
    let mut x = at.x;
    for (t, c) in parts {
        let g = p.layout_no_wrap(t.to_string(), font.clone(), *c);
        let w = g.size().x;
        p.galley(pos2(x, at.y), g, *c);
        x += w;
    }
}

fn editor(p: &egui::Painter, r: Rect) {
    // Tab strip.
    let tabs = Rect::from_min_size(r.min, vec2(r.width(), 60.0));
    p.rect_filled(tabs, egui::Rounding { nw: 10.0, ne: 10.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(0x15, 0x15, 0x19));
    let mut x = r.left() + 12.0;
    for (i, t) in ["panel.rs", "layout.rs", "Cargo.toml"].iter().enumerate() {
        let g = p.layout_no_wrap(t.to_string(), FontId::proportional(24.0), if i == 0 { TX } else { CM });
        let w = g.size().x + 28.0;
        let tab = Rect::from_min_size(pos2(x, tabs.top() + 6.0), vec2(w, 52.0));
        if i == 0 {
            p.rect_filled(tab, egui::Rounding { nw: 8.0, ne: 8.0, sw: 0.0, se: 0.0 }, Color32::from_rgb(0x0E, 0x0E, 0x11));
        }
        p.galley(tab.center() - g.size() / 2.0, g, TX);
        x += w + 4.0;
    }
    let f = FontId::monospace(25.6);
    let lines: &[&[(&str, Color32)]] = &[
        &[("/// Fade panels you're not looking at.", CM)],
        &[("pub fn ", KW), ("in_view", FN), ("(head: &", TX), ("Affine3A", TY), (", panel: &", TX), ("Affine3A", TY), (", clip: ", TX), ("f32", TY), (") -> ", TX), ("bool", TY), (" {", TX)],
        &[("    let ", KW), ("to_panel = (panel.translation - head.translation).", TX), ("normalize_or_zero", FN), ("();", TX)],
        &[("    let ", KW), ("normal = panel.matrix3.z_axis;", TX)],
        &[("    let ", KW), ("cos = -to_panel.", TX), ("dot", FN), ("(normal);", TX)],
        &[("    cos >= clip.", TX), ("to_radians", FN), ("().", TX), ("cos", FN), ("()", TX)],
        &[("}", TX)],
        &[("", TX)],
        &[("#[test]", NU)],
        &[("fn ", KW), ("edge_on_panels_hide", FN), ("() {", TX)],
        &[("    let ", KW), ("head = ", TX), ("Affine3A", TY), ("::", TX), ("IDENTITY", NU), (";", TX)],
        &[("    let ", KW), ("side = ", TX), ("Affine3A", TY), ("::", TX), ("from_rotation_y", FN), ("(", TX), ("80f32", NU), (".", TX), ("to_radians", FN), ("());", TX)],
        &[("    assert!", FN), ("(!", TX), ("in_view", FN), ("(&head, &side, ", TX), ("60.0", NU), ("));", TX)],
        &[("}", TX)],
        &[("", TX)],
        &[("// TODO: ", CM), ("hysteresis so panels don't flicker at the edge", ST)],
    ];
    let mut y = tabs.bottom() + 16.0;
    for (n, line) in lines.iter().enumerate() {
        if y > r.bottom() - 24.0 {
            break;
        }
        let num = p.layout_no_wrap(format!("{:>3}", n + 41), f.clone(), CM);
        p.galley(pos2(r.left() + 14.0, y), num, CM);
        if n == 4 {
            p.rect_filled(Rect::from_min_size(pos2(r.left() + 60.0, y - 3.0), vec2(r.width() - 90.0, 38.0)), 4.0, Color32::from_rgb(0x1A, 0x1A, 0x20));
        }
        spans(p, pos2(r.left() + 92.0, y), line, &f);
        y += 41.6;
    }
}

fn notes(p: &egui::Painter, r: Rect) {
    let x = r.left() + 26.0;
    let mut y = r.top() + 22.0;
    p.text(pos2(x, y), egui::Align2::LEFT_TOP, "Sprint plan", FontId::proportional(48.0), TX);
    y += 80.0;
    p.text(pos2(x, y), egui::Align2::LEFT_TOP, "Week 48 · VR overlay", FontId::proportional(25.6), CM);
    y += 64.0;
    let items: &[(bool, &str)] = &[
        (true, "Wrist toolbar: compact + More"),
        (true, "Dock + close on every panel"),
        (false, "Focus mode: dim everything else"),
        (false, "Work layout preset"),
        (false, "Resize windows from VR"),
        (false, "Multi-monitor wall"),
    ];
    for (done, t) in items {
        let b = Rect::from_min_size(pos2(x, y + 2.0), vec2(28.0, 28.0));
        p.rect_stroke(b, 4.0, egui::Stroke::new(1.5_f32, if *done { TY } else { CM }));
        if *done {
            p.line_segment([b.left_center() + vec2(4.0, 0.0), b.center_bottom() - vec2(1.0, 5.0)], egui::Stroke::new(2.0_f32, TY));
            p.line_segment([b.center_bottom() - vec2(1.0, 5.0), b.right_top() + vec2(-3.0, 4.0)], egui::Stroke::new(2.0_f32, TY));
        }
        p.text(pos2(x + 48.0, y), egui::Align2::LEFT_TOP, *t, FontId::proportional(30.4), if *done { CM } else { TX });
        y += 60.8;
    }
    y += 22.4;
    p.text(pos2(x, y), egui::Align2::LEFT_TOP, "Notes", FontId::proportional(35.2), TX);
    y += 60.8;
    for t in ["Panels idle at 5 fps when you look away.", "Keyboard: real key hold, no auto-repeat by default."] {
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, format!("•  {t}"), FontId::proportional(27.2), TX);
        y += 48.0;
    }
}

fn terminal(p: &egui::Painter, r: Rect) {
    let f = FontId::monospace(24.0);
    let lines: &[&[(&str, Color32)]] = &[
        &[("~/rsoverlay ", FN), ("$ ", CM), ("cargo test --workspace", TX)],
        &[("   Compiling ", ST), ("rsoverlay-ui v0.1.0", TX)],
        &[("   Compiling ", ST), ("rsoverlay-app v0.1.0", TX)],
        &[("    Finished ", ST), ("test profile in 9.84s", TX)],
        &[("running 51 tests", TX)],
        &[("test shell::tests::slugs_roundtrip ... ", TX), ("ok", ST)],
        &[("test wrist::tests::compact_order ... ", TX), ("ok", ST)],
        &[("test laser::tests::click_stabilises ... ", TX), ("ok", ST)],
        &[("test result: ", TX), ("ok", ST), (". 51 passed; 0 failed", TX)],
        &[("", TX)],
        &[("~/rsoverlay ", FN), ("$ ", CM), ("git push", TX)],
        &[("   147ad2f..fb7560f  main -> main", CM)],
        &[("~/rsoverlay ", FN), ("$ ", CM), ("▌", TX)],
    ];
    let mut y = r.top() + 18.0;
    for line in lines {
        if y > r.bottom() - 22.0 {
            break;
        }
        spans(p, pos2(r.left() + 18.0, y), line, &f);
        y += 38.4;
    }
}

fn guide(p: &egui::Painter, r: Rect) {
    let x = r.left() + 26.0;
    let mut y = r.top() + 22.0;
    p.text(pos2(x, y), egui::Align2::LEFT_TOP, "The Hollow Warden", FontId::proportional(48.0), TX);
    y += 76.8;
    p.text(pos2(x, y), egui::Align2::LEFT_TOP, "Act III · optional boss · recommended level 42", FontId::proportional(25.6), CM);
    y += 64.0;
    for (h, body) in [
        ("Phase 1", "Stay at mid range. The sweep has a long wind-up: dodge through it, not away."),
        ("Phase 2 (below 60%)", "Adds spawn from the pillars. Clear them first; the boss heals while they live."),
        ("Phase 3 (below 25%)", "Arena floods. Stand on the raised stones; frost damage is doubled in water."),
        ("Drops", "Warden's Lantern (12%), Hollow Plate set, 3,400 gold."),
    ] {
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, h, FontId::proportional(32.0), NU);
        y += 48.0;
        let g = p.layout(body.to_string(), FontId::proportional(27.2), TX, r.width() - 60.0);
        let h = g.size().y;
        p.galley(pos2(x, y), g, TX);
        y += h + 34.0;
    }
}
