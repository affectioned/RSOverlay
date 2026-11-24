//! Global search: pages, settings, cards, windows and widgets from one
//! box in the header. Pure index + matcher (tested); the header draws
//! the field and results, DevView performs the jump.

use crate::shell::Page;
use crate::views::CaptureSource;
use crate::widgets::WidgetKind;

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Page(Page),
    /// Jump to `page`, scroll to and flash the card with this caption.
    Card(Page, &'static str),
    Window(CaptureSource),
    Widget(WidgetKind),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub title: String,
    /// Where it lives, shown dimmed beside the title.
    pub hint: &'static str,
    pub keywords: &'static str,
    pub target: Target,
}

/// Static part of the index: every page and setting.
fn static_entries() -> Vec<Entry> {
    use Page::*;
    let card = |title: &str, hint, keywords, page, caption| Entry {
        title: title.to_string(),
        hint,
        keywords,
        target: Target::Card(page, caption),
    };
    let mut v: Vec<Entry> = Page::ALL
        .iter()
        .map(|p| Entry { title: p.label().to_string(), hint: "Page", keywords: "", target: Target::Page(*p) })
        .collect();
    v.extend([
        card("Accent color", "Settings › Appearance", "colour theme pink rose teal", Settings, "Appearance"),
        card("Show / hide cards", "Settings › Appearance", "visibility mirror media chat keyboard history battery macros volume system", Settings, "Appearance"),
        card("Language", "Settings › Appearance", "translation locale english deutsch", Settings, "Appearance"),
        card("Keyboard layout", "Settings › Appearance", "qwerty qwertz azerty", Settings, "Appearance"),
        card("Night light", "Settings", "warm blue light schedule evening", Settings, "Night light"),
        card("Laser hand", "Settings › VR", "left handed right handed dominant hand wrist", Settings, "VR"),
        card("Hide panels beyond angle", "Settings › VR", "clip angle fade view", Settings, "VR"),
        card("Wrist opacity", "Settings › VR", "watch transparency", Settings, "VR"),
        card("VR text size", "Settings › Accessibility", "font scale zoom bigger larger", Settings, "Accessibility"),
        card("Laser beams", "Settings › Accessibility", "pointer ray", Settings, "Accessibility"),
        card("Reduce motion", "Settings › Accessibility", "animation transitions", Settings, "Accessibility"),
        card("Run as administrator", "Settings › Administrator", "uac admin elevated permissions", Settings, "Administrator"),
        card("Reset all settings", "Settings › Reset", "defaults factory restore", Settings, "Reset"),
        card("Wrist toolbar buttons", "Settings › Wrist toolbar", "customize customise wrist buttons", Settings, "Wrist toolbar"),
        card("Media theme", "Settings › Wrist toolbar", "album art colour color accent", Settings, "Wrist toolbar"),
        card("Cursor smoothing", "Settings › Pointer", "jitter steady laser", Settings, "Pointer"),
        card("Scroll speed", "Settings › Pointer", "wheel scrolling", Settings, "Pointer"),
        card("Placement", "Settings", "position offset size curve curvature opacity anchor", Settings, "Placement"),
        card("Performance stats", "Settings › Stats", "fps frame time cpu ram vram memory footprint light fast", Settings, "Stats"),
        card("Volume mixer", "Audio", "sound mute master app volume", Audio, "Volume"),
        card("Now playing", "Audio", "music media spotify song track play pause", Audio, "Now playing"),
        card("VRChat chatbox", "Chat", "osc message type send", Chat, "VRChat chatbox"),
        card("Share now playing", "Chat", "song to chatbox osc", Chat, "VRChat chatbox"),
        card("Macros", "Macros", "shortcut hotkey streamdeck button obs discord", Macros, "Macros"),
        card("Notification history", "Notifications", "alerts toasts clear", Notifications, "Notifications"),
        card("Battery", "Home", "controller headset charge warn osc avatar", Home, "Battery"),
        card("System stats", "Home", "cpu gpu ram vram performance", Home, "System"),
    ]);
    v
}

/// Full index: static entries + live windows (+ widgets in VR).
pub fn index(windows: &[CaptureSource], widgets: bool) -> Vec<Entry> {
    let mut v = static_entries();
    v.extend(windows.iter().map(|w| Entry {
        title: w.label(),
        hint: "Window",
        keywords: "open capture mirror desktop",
        target: Target::Window(w.clone()),
    }));
    if widgets {
        use WidgetKind::*;
        v.extend([NowPlaying, System, Volume, Battery, Macros, Chat, Notifications].map(|k| Entry {
            title: format!("{} widget", k.label()),
            hint: "Pop out",
            keywords: "widget float dock",
            target: Target::Widget(k),
        }));
    }
    v
}

/// Every query word must appear in title or keywords (case-insensitive).
/// Ranked: title starts with the query > a title word starts with it >
/// substring anywhere; then shorter titles first.
pub fn search<'a>(entries: &'a [Entry], query: &str, limit: usize) -> Vec<&'a Entry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(u8, usize, &Entry)> = entries
        .iter()
        .filter_map(|e| {
            let title = e.title.to_lowercase();
            let hay = format!("{title} {}", e.keywords);
            if !words.iter().all(|w| hay.contains(w)) {
                return None;
            }
            let rank = if title.starts_with(&q) {
                0
            } else if title.split_whitespace().any(|t| t.starts_with(words[0])) {
                1
            } else {
                2
            };
            Some((rank, e.title.len(), e))
        })
        .collect();
    hits.sort_by_key(|(r, len, _)| (*r, *len));
    hits.into_iter().take(limit).map(|(_, _, e)| e).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(q: &str) -> Vec<String> {
        let idx = index(&[CaptureSource::Window { hwnd: 1, title: "Discord".into() }], true);
        search(&idx, q, 5).into_iter().map(|e| e.title.clone()).collect()
    }

    #[test]
    fn finds_settings_by_keyword() {
        assert_eq!(titles("night")[0], "Night light");
        assert_eq!(titles("uac")[0], "Run as administrator");
        assert_eq!(titles("bigger text")[0], "VR text size");
        assert_eq!(titles("left handed")[0], "Laser hand");
    }

    #[test]
    fn prefix_ranks_first_and_windows_are_found() {
        assert_eq!(titles("disc")[0], "Discord");
        assert_eq!(titles("vol")[0], "Volume mixer");
    }

    #[test]
    fn empty_or_unknown_query() {
        assert!(titles("   ").is_empty());
        assert!(titles("zzzz nothing").is_empty());
    }

    #[test]
    fn card_targets_point_at_real_captions() {
        // Captions must match the card headers, or jump-and-flash breaks.
        for e in static_entries() {
            if let Target::Card(_, cap) = e.target {
                assert!(
                    ["Appearance", "Night light", "VR", "Pointer", "Wrist toolbar", "Accessibility", "Administrator", "Reset", "Placement", "Stats",
                        "Volume", "Now playing", "VRChat chatbox", "Macros", "Notifications", "Battery", "System"]
                        .contains(&cap),
                    "{cap}"
                );
            }
        }
    }
}
