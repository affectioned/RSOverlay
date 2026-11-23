//! Physical keyboard layout data. Each row is `(unshifted, shifted)`
//! pairs so Shift handling is uniform across every layout.
//!
//! IME-driven scripts (ja-JP, zh-CN, ko-KR): ship a US QWERTY and let
//! the host OS IME compose. The research deliberately picked these as
//! "QWERTY + system IME" so no in-overlay IME is needed.
//!
//! Dead-key behavior (German umlauts via ´+a, French accents) is
//! handled by the OS when we emit the raw Unicode through
//! `type_char`. That means our tables here can be flat ASCII-ish — we
//! just need the right positions.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LayoutId {
    #[default]
    Qwerty,
    Qwertz,
    Azerty,
    JcukenRu,
    Abnt2,
}

impl LayoutId {
    pub fn all() -> &'static [LayoutId] {
        &[Self::Qwerty, Self::Qwertz, Self::Azerty, Self::JcukenRu, Self::Abnt2]
    }

    /// Display label for the layout picker, short form so it fits.
    pub fn label(self) -> &'static str {
        match self {
            Self::Qwerty => "QWERTY",
            Self::Qwertz => "QWERTZ",
            Self::Azerty => "AZERTY",
            Self::JcukenRu => "ЙЦУКЕН",
            Self::Abnt2 => "ABNT2",
        }
    }

    /// Stable config.toml slug — matches the serde kebab-case rename.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Qwerty => "qwerty",
            Self::Qwertz => "qwertz",
            Self::Azerty => "azerty",
            Self::JcukenRu => "jcuken-ru",
            Self::Abnt2 => "abnt2",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        for l in Self::all() {
            if l.slug().eq_ignore_ascii_case(s) {
                return Some(*l);
            }
        }
        None
    }
}

/// A row of the keyboard: list of (lower, upper) glyph pairs.
pub type Row = &'static [(char, char)];

/// Full layout = four rows (numbers / top / home / bottom).
pub struct Layout {
    pub id: LayoutId,
    pub numbers: Row,
    pub top: Row,
    pub home: Row,
    pub bottom: Row,
}

const QWERTY: Layout = Layout {
    id: LayoutId::Qwerty,
    numbers: &[
        ('1','!'), ('2','@'), ('3','#'), ('4','$'), ('5','%'),
        ('6','^'), ('7','&'), ('8','*'), ('9','('), ('0',')'),
    ],
    top: &[
        ('q','Q'), ('w','W'), ('e','E'), ('r','R'), ('t','T'),
        ('y','Y'), ('u','U'), ('i','I'), ('o','O'), ('p','P'),
    ],
    home: &[
        ('a','A'), ('s','S'), ('d','D'), ('f','F'), ('g','G'),
        ('h','H'), ('j','J'), ('k','K'), ('l','L'),
    ],
    bottom: &[
        ('z','Z'), ('x','X'), ('c','C'), ('v','V'), ('b','B'),
        ('n','N'), ('m','M'), (',','<'), ('.','>'), ('/','?'),
    ],
};

// QWERTZ: swap Y↔Z vs QWERTY, add ß on home row (as the extra 11th key
// position isn't drawable in our grid, we keep 10/10/9/10 shape).
const QWERTZ: Layout = Layout {
    id: LayoutId::Qwertz,
    numbers: &[
        ('1','!'), ('2','"'), ('3','§'), ('4','$'), ('5','%'),
        ('6','&'), ('7','/'), ('8','('), ('9',')'), ('0','='),
    ],
    top: &[
        ('q','Q'), ('w','W'), ('e','E'), ('r','R'), ('t','T'),
        ('z','Z'), ('u','U'), ('i','I'), ('o','O'), ('p','P'),
    ],
    home: &[
        ('a','A'), ('s','S'), ('d','D'), ('f','F'), ('g','G'),
        ('h','H'), ('j','J'), ('k','K'), ('l','L'),
    ],
    bottom: &[
        ('y','Y'), ('x','X'), ('c','C'), ('v','V'), ('b','B'),
        ('n','N'), ('m','M'), (',',';'), ('.',':'), ('-','_'),
    ],
};

// AZERTY: Q→A, W→Z, M moves to home-row end. Numbers row is
// shift-flipped (digits ARE the shift state on real AZERTY) but the
// on-screen keyboard uses their shifted digits as the unshifted face.
const AZERTY: Layout = Layout {
    id: LayoutId::Azerty,
    numbers: &[
        ('1','1'), ('2','2'), ('3','3'), ('4','4'), ('5','5'),
        ('6','6'), ('7','7'), ('8','8'), ('9','9'), ('0','0'),
    ],
    top: &[
        ('a','A'), ('z','Z'), ('e','E'), ('r','R'), ('t','T'),
        ('y','Y'), ('u','U'), ('i','I'), ('o','O'), ('p','P'),
    ],
    home: &[
        ('q','Q'), ('s','S'), ('d','D'), ('f','F'), ('g','G'),
        ('h','H'), ('j','J'), ('k','K'), ('l','L'),
    ],
    bottom: &[
        ('w','W'), ('x','X'), ('c','C'), ('v','V'), ('b','B'),
        ('n','N'), ('m','M'), (',','?'), (';','.'), (':','/'),
    ],
};

// ЙЦУКЕН — Russian JCUKEN layout.
const JCUKEN_RU: Layout = Layout {
    id: LayoutId::JcukenRu,
    numbers: &[
        ('1','!'), ('2','"'), ('3','№'), ('4',';'), ('5','%'),
        ('6',':'), ('7','?'), ('8','*'), ('9','('), ('0',')'),
    ],
    top: &[
        ('й','Й'), ('ц','Ц'), ('у','У'), ('к','К'), ('е','Е'),
        ('н','Н'), ('г','Г'), ('ш','Ш'), ('щ','Щ'), ('з','З'),
    ],
    home: &[
        ('ф','Ф'), ('ы','Ы'), ('в','В'), ('а','А'), ('п','П'),
        ('р','Р'), ('о','О'), ('л','Л'), ('д','Д'),
    ],
    bottom: &[
        ('я','Я'), ('ч','Ч'), ('с','С'), ('м','М'), ('и','И'),
        ('т','Т'), ('ь','Ь'), ('б','Б'), ('ю','Ю'), ('.',','),
    ],
};

// Brazilian ABNT2 — like QWERTY + Ç + shift gives upper and accented
// forms where applicable.
const ABNT2: Layout = Layout {
    id: LayoutId::Abnt2,
    numbers: &[
        ('1','!'), ('2','@'), ('3','#'), ('4','$'), ('5','%'),
        ('6','¨'), ('7','&'), ('8','*'), ('9','('), ('0',')'),
    ],
    top: &[
        ('q','Q'), ('w','W'), ('e','E'), ('r','R'), ('t','T'),
        ('y','Y'), ('u','U'), ('i','I'), ('o','O'), ('p','P'),
    ],
    home: &[
        ('a','A'), ('s','S'), ('d','D'), ('f','F'), ('g','G'),
        ('h','H'), ('j','J'), ('k','K'), ('l','L'),
    ],
    bottom: &[
        ('z','Z'), ('x','X'), ('c','C'), ('v','V'), ('b','B'),
        ('n','N'), ('m','M'), ('ç','Ç'), (',','<'), ('.','>'),
    ],
};

pub fn layout(id: LayoutId) -> &'static Layout {
    match id {
        LayoutId::Qwerty => &QWERTY,
        LayoutId::Qwertz => &QWERTZ,
        LayoutId::Azerty => &AZERTY,
        LayoutId::JcukenRu => &JCUKEN_RU,
        LayoutId::Abnt2 => &ABNT2,
    }
}
