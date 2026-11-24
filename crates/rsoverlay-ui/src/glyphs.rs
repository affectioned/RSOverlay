//! Vector-drawn symbols for glyphs egui's bundled fonts don't cover
//! (⌫ ✕ ← ↓ ↑ → ⏎ ⤢ render as empty boxes otherwise). Resolution-
//! independent and crisp at any overlay size.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Glyph {
    Close,
    Left,
    Right,
    Up,
    Down,
    Backspace,
    Enter,
}

impl Glyph {
    /// Map a label that would otherwise be drawn as text.
    pub fn from_label(s: &str) -> Option<Self> {
        Some(match s {
            "✕" => Self::Close,
            "←" => Self::Left,
            "→" => Self::Right,
            "↑" => Self::Up,
            "↓" => Self::Down,
            "⌫" => Self::Backspace,
            "⏎" => Self::Enter,
            _ => return None,
        })
    }
}

/// Draw `g` centred at `c`, roughly `size` points across.
pub fn draw(p: &egui::Painter, g: Glyph, c: egui::Pos2, size: f32, color: egui::Color32) {
    let s = egui::Stroke::new((size * 0.12).max(1.6), color);
    let h = size * 0.5;
    let v = egui::vec2;
    let arrow = |dir: egui::Vec2| {
        // Shaft + two barbs, pointing along `dir`.
        let tip = c + dir * h;
        let tail = c - dir * h;
        let n = egui::vec2(-dir.y, dir.x);
        p.line_segment([tail, tip], s);
        p.line_segment([tip, tip - dir * h * 0.6 + n * h * 0.55], s);
        p.line_segment([tip, tip - dir * h * 0.6 - n * h * 0.55], s);
    };
    match g {
        Glyph::Close => {
            let d = h * 0.75;
            p.line_segment([c + v(-d, -d), c + v(d, d)], s);
            p.line_segment([c + v(-d, d), c + v(d, -d)], s);
        }
        Glyph::Left => arrow(v(-1.0, 0.0)),
        Glyph::Right => arrow(v(1.0, 0.0)),
        Glyph::Up => arrow(v(0.0, -1.0)),
        Glyph::Down => arrow(v(0.0, 1.0)),
        Glyph::Backspace => {
            // Key-shaped outline pointing left, with an × inside.
            let (l, r, t, b) = (c.x - h * 1.2, c.x + h, c.y - h * 0.7, c.y + h * 0.7);
            let pts = vec![egui::pos2(l, c.y), egui::pos2(l + h * 0.6, t), egui::pos2(r, t), egui::pos2(r, b), egui::pos2(l + h * 0.6, b)];
            p.add(egui::Shape::closed_line(pts, s));
            let xc = egui::pos2(c.x + h * 0.25, c.y);
            let d = h * 0.32;
            p.line_segment([xc + v(-d, -d), xc + v(d, d)], s);
            p.line_segment([xc + v(-d, d), xc + v(d, -d)], s);
        }
        Glyph::Enter => {
            // ⏎: down from the top right, then left with an arrowhead.
            let corner = egui::pos2(c.x + h * 0.8, c.y + h * 0.35);
            p.line_segment([egui::pos2(corner.x, c.y - h * 0.7), corner], s);
            let tip = egui::pos2(c.x - h * 0.9, corner.y);
            p.line_segment([corner, tip], s);
            p.line_segment([tip, tip + v(h * 0.45, -h * 0.4)], s);
            p.line_segment([tip, tip + v(h * 0.45, h * 0.4)], s);
        }
    }
}
