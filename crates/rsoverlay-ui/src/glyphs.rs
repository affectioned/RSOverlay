//! Symbols egui's bundled fonts don't cover (⌫ ✕ ← ↓ ↑ → ⏎ render as
//! empty boxes otherwise), drawn with the bundled Tabler icons.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Glyph {
    Close,
    Left,
    Right,
    Up,
    Down,
    Backspace,
    Enter,
    Minus,
    Plus,
    Star,
    StarOutline,
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
            "★" => Self::Star,
            "☆" => Self::StarOutline,
            _ => return None,
        })
    }
}

/// Classic arrow mouse pointer with its hotspot at `tip`: white fill,
/// black outline — readable over any content.
pub fn draw_pointer(p: &egui::Painter, tip: egui::Pos2, size: f32) {
    let s = size / 16.0;
    let pts: Vec<egui::Pos2> = [(0.0, 0.0), (0.0, 13.0), (3.4, 10.0), (5.6, 15.0), (7.8, 14.0), (5.6, 9.2), (10.0, 9.2)]
        .iter()
        .map(|(x, y)| tip + egui::vec2(x * s, y * s))
        .collect();
    // Fill the non-convex silhouette as convex pieces, then outline.
    for tri in [[0, 1, 2], [0, 2, 6], [2, 3, 5], [3, 4, 5]] {
        p.add(egui::Shape::convex_polygon(tri.iter().map(|&i| pts[i]).collect(), egui::Color32::WHITE, egui::Stroke::NONE));
    }
    p.add(egui::Shape::closed_line(pts, egui::Stroke::new(1.2_f32, egui::Color32::BLACK)));
}

/// Draw `g` centred at `c`, roughly `size` points across.
pub fn draw(p: &egui::Painter, g: Glyph, c: egui::Pos2, size: f32, color: egui::Color32) {
    use crate::icons;
    let src = match g {
        Glyph::Close => icons::CLOSE,
        Glyph::Left => icons::ARROW_LEFT,
        Glyph::Right => icons::ARROW_RIGHT,
        Glyph::Up => icons::ARROW_UP,
        Glyph::Down => icons::ARROW_DOWN,
        Glyph::Backspace => icons::BACKSPACE,
        Glyph::Enter => icons::ENTER,
        Glyph::Minus => icons::MINUS,
        Glyph::Plus => icons::PLUS,
        Glyph::Star => icons::STAR,
        Glyph::StarOutline => icons::STAR_OUTLINE,
    };
    // Tabler draws inside a 24-unit box with ~2 units of padding: a touch
    // larger keeps the glyph's ink the size the old vector glyphs had.
    let s = size * 1.25;
    icons::paint(p, src, egui::Rect::from_center_size(c, egui::vec2(s, s)), color);
}
