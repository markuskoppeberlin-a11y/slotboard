//! Menu texts in German and English (the setup window has its own texts).

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    De,
    #[default]
    En,
}

impl Lang {
    pub fn from_code(code: &str) -> Self {
        if code.to_ascii_lowercase().starts_with("de") {
            Lang::De
        } else {
            Lang::En
        }
    }
}

pub struct MenuTexts {
    pub empty: &'static str,
    pub image: &'static str,
    pub content: &'static str,
    pub files: &'static str,
    pub clear_all: &'static str,
    pub paused: &'static str,
    pub autostart: &'static str,
    pub setup: &'static str,
    pub finish_setup: &'static str,
    pub unsupported: &'static str,
    pub quit: &'static str,
}

pub fn menu(lang: Lang) -> MenuTexts {
    match lang {
        Lang::De => MenuTexts {
            empty: "–",
            image: "Bild",
            content: "Inhalt",
            files: "Dateien",
            clear_all: "Alle Fächer leeren",
            paused: "Pausieren",
            autostart: "Beim Anmelden starten",
            setup: "Hilfe & Einrichtung…",
            finish_setup: "⚠︎ Einrichtung abschließen…",
            unsupported: "⚠︎ Auf diesem System nicht verfügbar",
            quit: "Slotboard beenden",
        },
        Lang::En => MenuTexts {
            empty: "–",
            image: "Image",
            content: "Content",
            files: "files",
            clear_all: "Clear all slots",
            paused: "Pause",
            autostart: "Open at login",
            setup: "Help & setup…",
            finish_setup: "⚠︎ Finish setup…",
            unsupported: "⚠︎ Not available on this system",
            quit: "Quit Slotboard",
        },
    }
}
