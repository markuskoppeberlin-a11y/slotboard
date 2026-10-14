//! Shared application state.

use std::sync::mpsc::Sender;
use std::sync::Mutex;

use serde::Serialize;

use crate::engine::Msg;
use crate::filter::label_for_slot;
use crate::i18n::Lang;
use crate::platform::{self, clipboard::Snapshot, ContentKind, Permission};
use crate::settings::Settings;

pub const SLOT_COUNT: usize = 10;

pub struct Slot {
    pub data: Snapshot,
    pub kind: ContentKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Status {
    pub permission: Permission,
    /// The keyboard shortcuts are live.
    pub active: bool,
    pub paused: bool,
    pub notice: Option<String>,
}

pub struct Core {
    pub slots: Mutex<Vec<Option<Slot>>>,
    pub status: Mutex<Status>,
    pub settings: Mutex<Settings>,
    pub lang: Mutex<Lang>,
    pub tx: Mutex<Option<Sender<Msg>>>,
}

impl Core {
    pub fn new() -> Self {
        Self {
            slots: Mutex::new((0..SLOT_COUNT).map(|_| None).collect()),
            status: Mutex::new(Status::default()),
            settings: Mutex::new(Settings::default()),
            lang: Mutex::new(Lang::from_code(&platform::system_language())),
            tx: Mutex::new(None),
        }
    }

    pub fn send(&self, msg: Msg) {
        if let Ok(guard) = self.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(msg);
            }
        }
    }

    pub fn lang(&self) -> Lang {
        self.lang.lock().map(|l| *l).unwrap_or_default()
    }

    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn slot_infos(&self) -> Vec<SlotInfo> {
        let slots = match self.slots.lock() {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        slots
            .iter()
            .enumerate()
            .map(|(i, slot)| SlotInfo::new(i as u8, slot.as_ref().map(|s| &s.kind)))
            .collect()
    }
}

/// What the setup window shows about a slot.
#[derive(Clone, Debug, Serialize)]
pub struct SlotInfo {
    pub index: u8,
    pub key: String,
    /// "empty", "text", "image", "files" or "other"
    pub kind: &'static str,
    pub text: String,
}

impl SlotInfo {
    pub fn new(index: u8, kind: Option<&ContentKind>) -> Self {
        let (kind, text) = match kind {
            None => ("empty", String::new()),
            Some(ContentKind::Text(t)) => ("text", one_line(t, 120)),
            Some(ContentKind::Image) => ("image", String::new()),
            Some(ContentKind::Files(f)) => ("files", f.join(", ")),
            Some(ContentKind::Other) => ("other", String::new()),
        };
        Self { index, key: label_for_slot(index), kind, text }
    }
}

/// Collapses whitespace and shortens text for menus.
pub fn one_line(text: &str, max_chars: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > max_chars {
        let cut: String = collapsed.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    } else {
        collapsed
    }
}
