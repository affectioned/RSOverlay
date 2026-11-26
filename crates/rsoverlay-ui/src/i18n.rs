//! Lightweight i18n: string tables per locale, keyed lookup with a
//! guaranteed English fallback so the UI never renders a raw `missing_key`
//! placeholder. New languages drop in as another `[Lang::X, &[...]]`
//! entry — no codegen, no files, everything embedded at compile time.
//!
//! String keys use dotted scopes (`appearance.title`, `keyboard.shift`)
//! rather than full English sentences, so a translator sees the
//! context before translating and misses fail loud (`"MISSING:" + key`).

use serde::{Deserialize, Serialize};

/// The UI's text isn't routed through these tables yet (it's English in
/// code), so the language picker stays hidden until it is: offering
/// languages that change nothing would only confuse.
pub const UI_TRANSLATED: bool = false;

/// Locales we ship UI translations for. More get added here as the
/// research on top VR regions lands.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Lang {
    #[default]
    #[serde(rename = "en-US")]
    EnUs,
    #[serde(rename = "de-DE")]
    DeDe,
    #[serde(rename = "fr-FR")]
    FrFr,
    #[serde(rename = "es-ES")]
    EsEs,
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "ja-JP")]
    JaJp,
    #[serde(rename = "ru-RU")]
    RuRu,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ko-KR")]
    KoKr,
}

impl Lang {
    /// Short display name in that language's own script — so a Spanish
    /// user sees "Español" not "Spanish". Shown in the UI language
    /// picker.
    pub fn native_name(self) -> &'static str {
        match self {
            Self::EnUs => "English",
            Self::DeDe => "Deutsch",
            Self::FrFr => "Français",
            Self::EsEs => "Español",
            Self::PtBr => "Português",
            Self::JaJp => "日本語",
            Self::RuRu => "Русский",
            Self::ZhCn => "简体中文",
            Self::KoKr => "한국어",
        }
    }

    pub fn all() -> &'static [Lang] {
        &[
            Self::EnUs,
            Self::DeDe,
            Self::FrFr,
            Self::EsEs,
            Self::PtBr,
            Self::JaJp,
            Self::RuRu,
            Self::ZhCn,
            Self::KoKr,
        ]
    }

    /// Parse a BCP-47 tag back into a Lang. Case-insensitive on the
    /// region part so `en-us` and `en-US` both work.
    pub fn from_bcp47(tag: &str) -> Option<Self> {
        for l in Self::all() {
            if l.bcp47().eq_ignore_ascii_case(tag) {
                return Some(*l);
            }
        }
        None
    }

    /// BCP-47 tag (en-US, de-DE, …). Matches the `rename` directive on
    /// the serde enum so this tag is what lands in config.toml.
    pub fn bcp47(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::DeDe => "de-DE",
            Self::FrFr => "fr-FR",
            Self::EsEs => "es-ES",
            Self::PtBr => "pt-BR",
            Self::JaJp => "ja-JP",
            Self::RuRu => "ru-RU",
            Self::ZhCn => "zh-CN",
            Self::KoKr => "ko-KR",
        }
    }
}

/// English — the authoritative set of keys. Every other language's
/// table is looked up against this one; missing keys fall back here.
const EN_US: &[(&str, &str)] = &[
    ("brand", "RSOverlay"),
    ("card.now_playing", "Now playing"),
    ("card.stats", "Stats"),
    ("card.desktop", "Desktop"),
    ("card.input_forwarding", "Input forwarding"),
    ("card.chatbox", "VRChat chatbox"),
    ("card.appearance", "Appearance"),
    ("card.placement", "Placement"),
    ("card.keyboard", "Keyboard"),
    ("card.notifications", "Notifications"),
    ("appearance.reset", "Reset defaults"),
    ("appearance.mirror", "Mirror"),
    ("appearance.media", "Media"),
    ("appearance.chat", "Chat"),
    ("appearance.keyboard", "Keyboard"),
    ("appearance.history", "History"),
    ("placement.anchor.wrist", "Wrist"),
    ("placement.anchor.hmd", "HMD-locked"),
    ("placement.anchor.world", "World-pinned"),
    ("placement.curve", "Curve"),
    ("chat.hint", "message VRChat chatbox…"),
    ("chat.send", "send"),
    ("notifications.clear_all", "Clear all"),
    ("notifications.empty", "no notifications yet"),
    ("media.playing", "playing"),
    ("media.paused", "paused"),
    ("media.nothing", "nothing playing"),
    ("media.prev_tt", "Previous track"),
    ("media.next_tt", "Next track"),
    ("media.play_tt", "Play"),
    ("media.pause_tt", "Pause"),
    ("input.inert", "clicks on the mirror are inert"),
    ("input.live", "clicks on the mirror reach the real desktop"),
    ("mirror.none", "no monitor captured"),
    ("stats.frame", "frame"),
    ("stats.fps", "fps"),
    ("stats.notifications", "notifications"),
    ("stats.ws_bind", "ws bind"),
    ("kb.shift", "Shift"),
    ("kb.caps", "Caps"),
    ("kb.ctrl", "Ctrl"),
    ("kb.alt", "Alt"),
    ("kb.win", "Win"),
    ("kb.space", "space"),
];

const DE_DE: &[(&str, &str)] = &[
    ("card.now_playing", "Jetzt läuft"),
    ("card.stats", "Statistik"),
    ("card.desktop", "Desktop"),
    ("card.input_forwarding", "Eingabe weiterleiten"),
    ("card.chatbox", "VRChat-Chatbox"),
    ("card.appearance", "Darstellung"),
    ("card.placement", "Platzierung"),
    ("card.keyboard", "Tastatur"),
    ("card.notifications", "Benachrichtigungen"),
    ("appearance.reset", "Zurücksetzen"),
    ("appearance.mirror", "Spiegel"),
    ("appearance.media", "Medien"),
    ("appearance.chat", "Chat"),
    ("appearance.keyboard", "Tastatur"),
    ("appearance.history", "Verlauf"),
    ("placement.anchor.wrist", "Handgelenk"),
    ("placement.anchor.hmd", "Am HMD"),
    ("placement.anchor.world", "In der Welt"),
    ("placement.curve", "Krümmung"),
    ("chat.hint", "VRChat-Nachricht…"),
    ("chat.send", "senden"),
    ("notifications.clear_all", "Alle löschen"),
    ("notifications.empty", "keine Benachrichtigungen"),
    ("media.playing", "läuft"),
    ("media.paused", "pausiert"),
    ("media.nothing", "nichts läuft"),
    ("input.inert", "Klicks auf dem Spiegel sind inaktiv"),
    ("input.live", "Klicks werden an den Desktop weitergegeben"),
    ("mirror.none", "kein Monitor erfasst"),
    ("stats.frame", "Frame"),
    ("stats.notifications", "Nachrichten"),
    ("kb.space", "Leer"),
];

const FR_FR: &[(&str, &str)] = &[
    ("card.now_playing", "En cours de lecture"),
    ("card.stats", "Statistiques"),
    ("card.desktop", "Bureau"),
    ("card.input_forwarding", "Transfert de saisie"),
    ("card.chatbox", "Chatbox VRChat"),
    ("card.appearance", "Apparence"),
    ("card.placement", "Placement"),
    ("card.keyboard", "Clavier"),
    ("card.notifications", "Notifications"),
    ("appearance.reset", "Réinitialiser"),
    ("appearance.mirror", "Miroir"),
    ("appearance.media", "Média"),
    ("appearance.history", "Historique"),
    ("placement.anchor.wrist", "Poignet"),
    ("placement.anchor.hmd", "Casque"),
    ("placement.anchor.world", "Fixé au monde"),
    ("placement.curve", "Courbure"),
    ("chat.hint", "message VRChat…"),
    ("chat.send", "envoyer"),
    ("notifications.clear_all", "Tout effacer"),
    ("notifications.empty", "aucune notification"),
    ("media.playing", "en lecture"),
    ("media.paused", "en pause"),
    ("media.nothing", "rien ne joue"),
    ("mirror.none", "aucun écran capturé"),
    ("stats.notifications", "notifications"),
    ("kb.space", "espace"),
];

const ES_ES: &[(&str, &str)] = &[
    ("card.now_playing", "Reproduciendo"),
    ("card.stats", "Estadísticas"),
    ("card.desktop", "Escritorio"),
    ("card.input_forwarding", "Reenvío de entrada"),
    ("card.chatbox", "Chatbox de VRChat"),
    ("card.appearance", "Apariencia"),
    ("card.placement", "Colocación"),
    ("card.keyboard", "Teclado"),
    ("card.notifications", "Notificaciones"),
    ("appearance.reset", "Restablecer"),
    ("appearance.history", "Historial"),
    ("placement.anchor.wrist", "Muñeca"),
    ("placement.anchor.hmd", "En HMD"),
    ("placement.anchor.world", "Fijado"),
    ("placement.curve", "Curvatura"),
    ("chat.hint", "mensaje de VRChat…"),
    ("chat.send", "enviar"),
    ("notifications.clear_all", "Borrar todo"),
    ("notifications.empty", "sin notificaciones"),
    ("media.playing", "reproduciendo"),
    ("media.paused", "pausado"),
    ("media.nothing", "nada reproduciéndose"),
    ("mirror.none", "ningún monitor capturado"),
    ("kb.space", "espacio"),
];

const PT_BR: &[(&str, &str)] = &[
    ("card.now_playing", "Tocando agora"),
    ("card.stats", "Estatísticas"),
    ("card.desktop", "Área de trabalho"),
    ("card.input_forwarding", "Encaminhamento"),
    ("card.chatbox", "Chat do VRChat"),
    ("card.appearance", "Aparência"),
    ("card.placement", "Posição"),
    ("card.keyboard", "Teclado"),
    ("card.notifications", "Notificações"),
    ("appearance.reset", "Restaurar padrões"),
    ("appearance.history", "Histórico"),
    ("placement.anchor.wrist", "Pulso"),
    ("placement.anchor.hmd", "No HMD"),
    ("placement.anchor.world", "Fixado"),
    ("placement.curve", "Curvatura"),
    ("chat.hint", "mensagem do VRChat…"),
    ("chat.send", "enviar"),
    ("notifications.clear_all", "Limpar tudo"),
    ("notifications.empty", "nenhuma notificação"),
    ("media.playing", "tocando"),
    ("media.paused", "pausado"),
    ("media.nothing", "nada tocando"),
    ("mirror.none", "nenhum monitor capturado"),
    ("kb.space", "espaço"),
];

const JA_JP: &[(&str, &str)] = &[
    ("card.now_playing", "再生中"),
    ("card.stats", "統計"),
    ("card.desktop", "デスクトップ"),
    ("card.input_forwarding", "入力転送"),
    ("card.chatbox", "VRChat チャットボックス"),
    ("card.appearance", "外観"),
    ("card.placement", "配置"),
    ("card.keyboard", "キーボード"),
    ("card.notifications", "通知"),
    ("appearance.reset", "既定に戻す"),
    ("appearance.history", "履歴"),
    ("placement.anchor.wrist", "手首"),
    ("placement.anchor.hmd", "HMD 固定"),
    ("placement.anchor.world", "ワールド固定"),
    ("placement.curve", "湾曲"),
    ("chat.hint", "VRChat にメッセージ…"),
    ("chat.send", "送信"),
    ("notifications.clear_all", "すべて消去"),
    ("notifications.empty", "通知はありません"),
    ("media.playing", "再生中"),
    ("media.paused", "一時停止"),
    ("media.nothing", "何も再生していません"),
    ("mirror.none", "モニター未キャプチャ"),
    ("kb.space", "スペース"),
];

const RU_RU: &[(&str, &str)] = &[
    ("card.now_playing", "Сейчас играет"),
    ("card.stats", "Статистика"),
    ("card.desktop", "Рабочий стол"),
    ("card.input_forwarding", "Передача ввода"),
    ("card.chatbox", "Чат VRChat"),
    ("card.appearance", "Оформление"),
    ("card.placement", "Размещение"),
    ("card.keyboard", "Клавиатура"),
    ("card.notifications", "Уведомления"),
    ("appearance.reset", "Сброс"),
    ("appearance.history", "История"),
    ("placement.anchor.wrist", "Запястье"),
    ("placement.anchor.hmd", "К шлему"),
    ("placement.anchor.world", "В мире"),
    ("placement.curve", "Изгиб"),
    ("chat.hint", "сообщение в VRChat…"),
    ("chat.send", "отправить"),
    ("notifications.clear_all", "Очистить всё"),
    ("notifications.empty", "нет уведомлений"),
    ("media.playing", "играет"),
    ("media.paused", "пауза"),
    ("media.nothing", "ничего не играет"),
    ("mirror.none", "монитор не захвачен"),
    ("kb.space", "пробел"),
];

const ZH_CN: &[(&str, &str)] = &[
    ("card.now_playing", "正在播放"),
    ("card.stats", "统计"),
    ("card.desktop", "桌面"),
    ("card.input_forwarding", "输入转发"),
    ("card.chatbox", "VRChat 聊天框"),
    ("card.appearance", "外观"),
    ("card.placement", "位置"),
    ("card.keyboard", "键盘"),
    ("card.notifications", "通知"),
    ("appearance.reset", "恢复默认"),
    ("appearance.history", "历史"),
    ("placement.anchor.wrist", "手腕"),
    ("placement.anchor.hmd", "头显锁定"),
    ("placement.anchor.world", "世界锁定"),
    ("placement.curve", "曲率"),
    ("chat.hint", "发送至 VRChat…"),
    ("chat.send", "发送"),
    ("notifications.clear_all", "全部清除"),
    ("notifications.empty", "暂无通知"),
    ("media.playing", "播放中"),
    ("media.paused", "已暂停"),
    ("media.nothing", "未在播放"),
    ("mirror.none", "未捕获显示器"),
    ("kb.space", "空格"),
];

const KO_KR: &[(&str, &str)] = &[
    ("card.now_playing", "재생 중"),
    ("card.stats", "통계"),
    ("card.desktop", "바탕화면"),
    ("card.input_forwarding", "입력 전달"),
    ("card.chatbox", "VRChat 채팅"),
    ("card.appearance", "모양"),
    ("card.placement", "배치"),
    ("card.keyboard", "키보드"),
    ("card.notifications", "알림"),
    ("appearance.reset", "기본값 복원"),
    ("appearance.history", "기록"),
    ("placement.anchor.wrist", "손목"),
    ("placement.anchor.hmd", "HMD 고정"),
    ("placement.anchor.world", "월드 고정"),
    ("placement.curve", "곡률"),
    ("chat.hint", "VRChat 메시지…"),
    ("chat.send", "전송"),
    ("notifications.clear_all", "모두 지우기"),
    ("notifications.empty", "알림 없음"),
    ("media.playing", "재생 중"),
    ("media.paused", "일시 정지"),
    ("media.nothing", "재생 중이 아님"),
    ("mirror.none", "캡처된 모니터 없음"),
    ("kb.space", "공백"),
];

fn table(lang: Lang) -> &'static [(&'static str, &'static str)] {
    match lang {
        Lang::EnUs => EN_US,
        Lang::DeDe => DE_DE,
        Lang::FrFr => FR_FR,
        Lang::EsEs => ES_ES,
        Lang::PtBr => PT_BR,
        Lang::JaJp => JA_JP,
        Lang::RuRu => RU_RU,
        Lang::ZhCn => ZH_CN,
        Lang::KoKr => KO_KR,
    }
}

/// Look up `key` in the given language. Missing entry → fall back to
/// English. Missing in English → return the key wrapped so the gap is
/// visible during development.
pub fn t(lang: Lang, key: &str) -> &'static str {
    for (k, v) in table(lang) {
        if *k == key {
            return v;
        }
    }
    if lang != Lang::EnUs {
        for (k, v) in EN_US {
            if *k == key {
                return v;
            }
        }
    }
    tracing::debug!("i18n: missing key {key:?} for {lang:?}");
    "???"
}
