//! The engine thread: reacts to shortcut actions, talks to the clipboard,
//! sends the paste shortcut and keeps an eye on the permission.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::filter::Action;
use crate::platform::{self, clipboard, Permission};
use crate::state::{Core, Slot, SLOT_COUNT};

pub enum Msg {
    /// From the keyboard hook.
    Key(Action),
    /// From the menu / setup window.
    CopySlotToClipboard(u8),
    ClearAll,
    SetPaused(bool),
    CheckPermission,
}

/// After pasting a slot, the normal clipboard is put back after this delay.
const RESTORE_DELAY: Duration = Duration::from_millis(600);
/// How long we wait for the app to finish copying after ⌘C.
const COPY_SETTLE_MAX: Duration = Duration::from_millis(700);
/// Safety net: if the key-release event got lost, drop a waiting paste.
const PASTE_GIVE_UP: Duration = Duration::from_secs(30);
const PERMISSION_POLL: Duration = Duration::from_millis(1000);
const HOOK_RETRY: Duration = Duration::from_millis(2500);

struct PendingRestore {
    original: Option<clipboard::Snapshot>,
    expected_count: u64,
    at: Instant,
}

pub fn spawn(app: AppHandle, core: Arc<Core>, rx: Receiver<Msg>, tx: Sender<Msg>) {
    let _ = std::thread::Builder::new().name("slotboard-engine".into()).spawn(move || {
        let mut engine = Engine {
            app,
            core,
            tx,
            copy_baseline: None,
            pending_paste: None,
            pending_restore: None,
            last_permission_check: Instant::now() - PERMISSION_POLL,
            last_hook_attempt: None,
        };
        engine.run(rx);
    });
}

struct Engine {
    app: AppHandle,
    core: Arc<Core>,
    tx: Sender<Msg>,
    copy_baseline: Option<u64>,
    pending_paste: Option<Instant>,
    pending_restore: Option<PendingRestore>,
    last_permission_check: Instant,
    last_hook_attempt: Option<Instant>,
}

impl Engine {
    fn run(&mut self, rx: Receiver<Msg>) {
        self.check_permission();
        loop {
            let now = Instant::now();
            let mut deadline = self.last_permission_check + PERMISSION_POLL;
            if let Some(at) = self.pending_paste {
                deadline = deadline.min(at + PASTE_GIVE_UP);
            }
            if let Some(r) = &self.pending_restore {
                deadline = deadline.min(r.at);
            }
            match rx.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.run_timers();
        }
    }

    fn run_timers(&mut self) {
        let now = Instant::now();
        if let Some(at) = self.pending_paste {
            if now >= at + PASTE_GIVE_UP {
                self.pending_paste = None;
            }
        }
        if self.pending_restore.as_ref().is_some_and(|r| now >= r.at) {
            self.restore_now();
        }
        if now >= self.last_permission_check + PERMISSION_POLL {
            self.check_permission();
        }
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Key(Action::CopyArmed) => {
                self.copy_baseline = Some(clipboard::change_count());
            }
            Msg::Key(Action::SaveSlot(slot)) => self.save_slot(slot),
            Msg::Key(Action::PasteArmed) => {
                if self.pending_paste.take().is_some() {
                    self.normal_paste();
                }
                self.pending_paste = Some(Instant::now());
            }
            Msg::Key(Action::PasteSlot(slot)) => {
                self.pending_paste = None;
                self.paste_slot(slot);
            }
            Msg::Key(Action::PasteFlush) => {
                if self.pending_paste.take().is_some() {
                    self.normal_paste();
                }
            }
            Msg::CopySlotToClipboard(slot) => {
                self.pending_restore = None;
                if let Some(data) = self.slot_data(slot) {
                    clipboard::write(&data, false);
                }
            }
            Msg::ClearAll => {
                if let Ok(mut slots) = self.core.slots.lock() {
                    slots.iter_mut().for_each(|s| *s = None);
                }
                self.slots_changed();
            }
            Msg::SetPaused(paused) => {
                platform::set_paused(paused);
                self.pending_paste = None;
                if let Ok(mut status) = self.core.status.lock() {
                    status.paused = paused;
                }
                self.status_changed();
            }
            Msg::CheckPermission => self.check_permission(),
        }
    }

    fn slot_data(&self, slot: u8) -> Option<clipboard::Snapshot> {
        let slots = self.core.slots.lock().ok()?;
        slots.get(slot as usize)?.as_ref().map(|s| s.data.clone())
    }

    /// ⌘C + digit: wait until the app has finished copying, then keep a copy.
    fn save_slot(&mut self, slot: u8) {
        if slot as usize >= SLOT_COUNT {
            return;
        }
        if let Some(baseline) = self.copy_baseline {
            let start = Instant::now();
            while clipboard::change_count() == baseline && start.elapsed() < COPY_SETTLE_MAX {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        // A second digit for the same copy must not wait again.
        self.copy_baseline = None;

        let Some(snapshot) = clipboard::read() else { return };
        if snapshot.is_concealed() {
            // Passwords from password managers are never stored.
            return;
        }
        let kind = snapshot.kind();
        if let Ok(mut slots) = self.core.slots.lock() {
            slots[slot as usize] = Some(Slot { data: snapshot, kind });
        }
        self.slots_changed();
    }

    /// ⌘V + digit: put the slot on the clipboard, paste, then put the normal
    /// clipboard back.
    fn paste_slot(&mut self, slot: u8) {
        let Some(data) = self.slot_data(slot) else { return };
        let original = match self.pending_restore.take() {
            Some(pending) => pending.original,
            None => clipboard::read(),
        };
        if !clipboard::write(&data, true) {
            return;
        }
        let expected_count = clipboard::change_count();
        platform::send_paste();
        self.pending_restore = Some(PendingRestore { original, expected_count, at: Instant::now() + RESTORE_DELAY });
    }

    fn normal_paste(&mut self) {
        // Make sure the normal clipboard is back before a normal paste.
        if self.pending_restore.is_some() {
            self.restore_now();
        }
        platform::send_paste();
    }

    fn restore_now(&mut self) {
        let Some(pending) = self.pending_restore.take() else { return };
        // Only if nobody copied something new in the meantime.
        if clipboard::change_count() != pending.expected_count {
            return;
        }
        match &pending.original {
            Some(original) => {
                clipboard::write(original, true);
            }
            None => clipboard::clear(),
        }
    }

    fn check_permission(&mut self) {
        self.last_permission_check = Instant::now();
        let mut permission = platform::permission();
        if permission == Permission::Granted && !platform::hook_running() {
            let may_try = self.last_hook_attempt.is_none_or(|t| t.elapsed() >= HOOK_RETRY);
            if may_try {
                self.last_hook_attempt = Some(Instant::now());
                if let Err(err) = platform::start_hook(self.tx.clone()) {
                    eprintln!("Slotboard: keyboard hook failed: {err}");
                    platform::note_hook_failure();
                    permission = platform::permission();
                }
            }
        }
        if platform::hook_running() {
            platform::ensure_hook_enabled();
        }
        let active = permission == Permission::Granted && platform::hook_running();
        let changed = match self.core.status.lock() {
            Ok(mut status) => {
                let became_active = active && !status.active;
                let changed = status.permission != permission || status.active != active;
                status.permission = permission;
                status.active = active;
                status.notice = platform::notice().map(str::to_string);
                if became_active {
                    crate::ui::focus_setup(&self.app);
                }
                changed
            }
            Err(_) => false,
        };
        if changed {
            self.status_changed();
        }
    }

    fn slots_changed(&self) {
        crate::tray::refresh(&self.app);
        crate::ui::emit_state(&self.app);
    }

    fn status_changed(&self) {
        crate::tray::refresh(&self.app);
        crate::ui::emit_state(&self.app);
    }
}
