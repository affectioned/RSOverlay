//! Keyboard shortcut strings → modifier set + main key.
//!
//! Grammar: `mod+mod+key`, case-insensitive, whitespace ignored.
//! - modifiers: `ctrl`/`control`, `shift`, `alt`, `win`/`super`/`meta`
//! - key: `a`-`z`, `0`-`9`, `f1`-`f24`, named keys (`enter`, `tab`,
//!   `esc`, `space`, `up`, `pgdn`, …) or media keys (`media_play_pause`,
//!   `volume_mute`, …).
//!
//! Parsing is platform-neutral; [`Key::vk`] yields the Win32 virtual-key
//! code (the only injector backend today).

use std::fmt;

/// Modifier bitset. Bit order is also the press order on injection
/// (Ctrl → Alt → Shift → Win), matching how humans chord.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Mods(u8);

impl Mods {
    pub const CTRL: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const SHIFT: Self = Self(1 << 2);
    pub const WIN: Self = Self(1 << 3);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Win32 VK codes for the set modifiers, in press order.
    pub fn vks(self) -> impl Iterator<Item = u16> {
        // VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN
        const TABLE: [u16; 4] = [0xA2, 0xA4, 0xA0, 0x5B];
        (0..4)
            .filter(move |i| self.0 & (1 << i) != 0)
            .map(|i| TABLE[i])
    }
}

impl std::ops::BitOr for Mods {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Main (non-modifier) key, stored as its Win32 VK code.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Key(u16);

impl Key {
    pub fn vk(self) -> u16 {
        self.0
    }

    /// Navigation-cluster keys that need `KEYEVENTF_EXTENDEDKEY`;
    /// without it, Shift+Arrow is read as the numpad variant and
    /// Shift drops out of the chord.
    pub fn is_extended(self) -> bool {
        matches!(
            self.0,
            0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0x5C | 0x6F | 0x90
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub mods: Mods,
    pub key: Key,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    UnknownToken(String),
    /// Only modifiers, e.g. `ctrl+shift`.
    NoKey,
    /// Two non-modifier keys, e.g. `a+b`.
    MultipleKeys,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty shortcut"),
            Self::UnknownToken(t) => write!(f, "unknown key `{t}`"),
            Self::NoKey => write!(f, "needs a key besides modifiers"),
            Self::MultipleKeys => write!(f, "only one non-modifier key allowed"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Named keys → Win32 VK. Sorted only for readability; lookup is linear
/// over ~40 entries, which is cheaper than hashing at this size.
const NAMED: &[(&str, u16)] = &[
    ("enter", 0x0D),
    ("return", 0x0D),
    ("tab", 0x09),
    ("esc", 0x1B),
    ("escape", 0x1B),
    ("space", 0x20),
    ("backspace", 0x08),
    ("delete", 0x2E),
    ("del", 0x2E),
    ("insert", 0x2D),
    ("ins", 0x2D),
    ("home", 0x24),
    ("end", 0x23),
    ("pageup", 0x21),
    ("pgup", 0x21),
    ("pagedown", 0x22),
    ("pgdn", 0x22),
    ("up", 0x26),
    ("down", 0x28),
    ("left", 0x25),
    ("right", 0x27),
    ("printscreen", 0x2C),
    ("prtsc", 0x2C),
    ("pause", 0x13),
    ("volume_mute", 0xAD),
    ("volume_down", 0xAE),
    ("volume_up", 0xAF),
    ("media_next", 0xB0),
    ("media_prev", 0xB1),
    ("media_stop", 0xB2),
    ("media_play_pause", 0xB3),
];

fn parse_key(tok: &str) -> Option<Key> {
    let b = tok.as_bytes();
    if b.len() == 1 {
        return match b[0] {
            c @ b'a'..=b'z' => Some(Key(u16::from(c - b'a') + 0x41)),
            c @ b'0'..=b'9' => Some(Key(u16::from(c - b'0') + 0x30)),
            _ => None,
        };
    }
    if let Some(n) = tok.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()) {
        // VK_F1 = 0x70 … VK_F24 = 0x87
        return (1..=24).contains(&n).then(|| Key(0x6F + n));
    }
    NAMED
        .iter()
        .find(|(name, _)| *name == tok)
        .map(|&(_, vk)| Key(vk))
}

fn parse_mod(tok: &str) -> Option<Mods> {
    Some(match tok {
        "ctrl" | "control" => Mods::CTRL,
        "alt" => Mods::ALT,
        "shift" => Mods::SHIFT,
        "win" | "super" | "meta" => Mods::WIN,
        _ => return None,
    })
}

impl std::str::FromStr for Shortcut {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, ParseError> {
        let norm: String = s
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        if norm.is_empty() {
            return Err(ParseError::Empty);
        }
        let mut mods = Mods::default();
        let mut key = None;
        for tok in norm.split('+') {
            if let Some(m) = parse_mod(tok) {
                mods = mods | m;
            } else if let Some(k) = parse_key(tok) {
                if key.replace(k).is_some() {
                    return Err(ParseError::MultipleKeys);
                }
            } else {
                return Err(ParseError::UnknownToken(tok.to_string()));
            }
        }
        key.map(|key| Shortcut { mods, key })
            .ok_or(ParseError::NoKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Shortcut, ParseError> {
        s.parse()
    }

    #[test]
    fn letters_digits_fkeys() {
        assert_eq!(p("m").unwrap().key.vk(), 0x4D);
        assert_eq!(p("7").unwrap().key.vk(), 0x37);
        assert_eq!(p("f1").unwrap().key.vk(), 0x70);
        assert_eq!(p("F13").unwrap().key.vk(), 0x7C);
        assert_eq!(p("f24").unwrap().key.vk(), 0x87);
        assert!(p("f25").is_err());
        assert!(p("f0").is_err());
    }

    #[test]
    fn modifiers_any_order_and_case() {
        let s = p(" Shift + CTRL+m ").unwrap();
        assert_eq!(s.mods, Mods::CTRL | Mods::SHIFT);
        assert_eq!(s.mods.vks().collect::<Vec<_>>(), vec![0xA2, 0xA0]);
        assert_eq!(
            p("win+shift+s").unwrap().mods.vks().collect::<Vec<_>>(),
            vec![0xA0, 0x5B]
        );
    }

    #[test]
    fn named_and_media() {
        assert_eq!(p("alt+tab").unwrap().key.vk(), 0x09);
        assert_eq!(p("media_play_pause").unwrap().key.vk(), 0xB3);
        assert!(p("shift+left").unwrap().key.is_extended());
        assert!(!p("a").unwrap().key.is_extended());
    }

    #[test]
    fn errors() {
        assert_eq!(p(""), Err(ParseError::Empty));
        assert_eq!(p("ctrl+shift"), Err(ParseError::NoKey));
        assert_eq!(p("a+b"), Err(ParseError::MultipleKeys));
        assert_eq!(
            p("ctrl+banana"),
            Err(ParseError::UnknownToken("banana".into()))
        );
        assert_eq!(p("ctrl++a"), Err(ParseError::UnknownToken(String::new())));
    }
}
