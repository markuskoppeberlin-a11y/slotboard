//! Platform-independent keyboard logic.
//!
//! Every platform hook translates its native key events into [`KeyEvent`]s
//! and asks the [`KeyFilter`] two things: should this event be swallowed
//! (hidden from the focused app), and does it trigger an [`Action`]?
//!
//! "primary" is ⌘ on macOS and Ctrl on Windows/Linux.
//!
//! * primary+C → normal copy, never swallowed. While primary stays held,
//!   tapping a digit stores the clipboard in that slot (the digit is
//!   swallowed).
//! * primary+V → swallowed. As long as primary stays held there is no time
//!   limit: a digit pastes that slot. Releasing primary without a digit (or
//!   pressing another key) makes the engine send a normal primary+V.
//!
//! The filter must stay fast and must never panic: on macOS and Windows it
//! runs inside the operating system's keyboard callback.

use std::time::{Duration, Instant};

/// Safety net: how long after primary+C (with primary still held) a digit
/// still counts as "store in slot". Releasing primary ends it right away.
pub const COPY_WINDOW: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A digit key (top row or numpad). The value is the printed digit 0..=9.
    Digit(u8),
    C,
    V,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    /// ⌘ on macOS, Ctrl on Windows/Linux.
    pub primary: bool,
    /// ⌥ on macOS, Alt on Windows/Linux.
    pub alt: bool,
    /// Control on macOS, Win/Super and AltGr on Windows/Linux.
    pub other: bool,
    pub shift: bool,
}

impl Mods {
    pub fn only_primary(&self) -> bool {
        self.primary && !self.alt && !self.other && !self.shift
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Down { repeat: bool },
    Up,
    /// A modifier key was pressed or released; `mods` holds the new state.
    ModifiersChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub kind: KeyKind,
    pub key: Key,
    /// Modifier state for this event (after the change for `ModifiersChanged`).
    pub mods: Mods,
}

impl KeyEvent {
    pub fn down(key: Key, mods: Mods) -> Self {
        Self { kind: KeyKind::Down { repeat: false }, key, mods }
    }
    pub fn repeat(key: Key, mods: Mods) -> Self {
        Self { kind: KeyKind::Down { repeat: true }, key, mods }
    }
    pub fn up(key: Key, mods: Mods) -> Self {
        Self { kind: KeyKind::Up, key, mods }
    }
    pub fn mods(mods: Mods) -> Self {
        Self { kind: KeyKind::ModifiersChanged, key: Key::Other, mods }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// primary+C was pressed; the app is about to copy.
    CopyArmed,
    /// Store the clipboard in slot `0..=9` (slot 0 = key "1", slot 9 = key "0").
    SaveSlot(u8),
    /// primary+V was swallowed. The engine waits for `PasteSlot` (digit) or
    /// `PasteFlush` (normal paste).
    PasteArmed,
    /// A digit followed primary+V: paste slot `0..=9` instead.
    PasteSlot(u8),
    /// Send the pending normal paste right now (primary released or another
    /// key pressed before the window ended).
    PasteFlush,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub swallow: bool,
    pub action: Option<Action>,
}

impl Decision {
    pub const PASS: Decision = Decision { swallow: false, action: None };
    fn pass_with(action: Action) -> Self {
        Decision { swallow: false, action: Some(action) }
    }
    fn swallow() -> Self {
        Decision { swallow: true, action: None }
    }
    fn swallow_with(action: Action) -> Self {
        Decision { swallow: true, action: Some(action) }
    }
}

/// Maps a printed digit to a slot index: "1" → 0 … "9" → 8, "0" → 9.
pub fn slot_for_digit(digit: u8) -> u8 {
    if digit == 0 {
        9
    } else {
        digit.min(9) - 1
    }
}

/// Maps a slot index back to its key label.
pub fn label_for_slot(slot: u8) -> String {
    if slot >= 9 {
        "0".to_string()
    } else {
        (slot + 1).to_string()
    }
}

#[derive(Debug, Default)]
pub struct KeyFilter {
    copy_armed_at: Option<Instant>,
    paste_armed_at: Option<Instant>,
    /// Digits whose key-down we swallowed, so the matching key-up is swallowed too.
    swallowed_digits: [bool; 10],
    /// The V key-down was swallowed, so its key-up (and auto-repeat) is too.
    swallowed_v: bool,
}

impl KeyFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_copy_armed(&self) -> bool {
        self.copy_armed_at.is_some()
    }

    pub fn is_paste_armed(&self) -> bool {
        self.paste_armed_at.is_some()
    }

    /// Forget all state (used when pausing or after the hook restarts).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn handle(&mut self, ev: &KeyEvent, now: Instant) -> Decision {
        if let Some(at) = self.copy_armed_at {
            if now.saturating_duration_since(at) > COPY_WINDOW || !ev.mods.primary {
                self.copy_armed_at = None;
            }
        }

        match ev.kind {
            KeyKind::ModifiersChanged => {
                if self.paste_armed_at.is_some() && !ev.mods.primary {
                    self.paste_armed_at = None;
                    return Decision::pass_with(Action::PasteFlush);
                }
                Decision::PASS
            }
            KeyKind::Down { repeat } => self.key_down(ev.key, ev.mods, repeat, now),
            KeyKind::Up => match ev.key {
                Key::Digit(d) if (d as usize) < 10 && self.swallowed_digits[d as usize] => {
                    self.swallowed_digits[d as usize] = false;
                    Decision::swallow()
                }
                Key::V if self.swallowed_v => {
                    self.swallowed_v = false;
                    Decision::swallow()
                }
                _ => Decision::PASS,
            },
        }
    }

    fn key_down(&mut self, key: Key, mods: Mods, repeat: bool, now: Instant) -> Decision {
        match key {
            Key::Digit(d) if d < 10 => {
                let slot = slot_for_digit(d);
                if self.paste_armed_at.is_some() && mods.only_primary() && !repeat {
                    self.paste_armed_at = None;
                    self.swallowed_digits[d as usize] = true;
                    return Decision::swallow_with(Action::PasteSlot(slot));
                }
                if self.copy_armed_at.is_some() && mods.only_primary() {
                    self.swallowed_digits[d as usize] = true;
                    if repeat {
                        return Decision::swallow();
                    }
                    return Decision::swallow_with(Action::SaveSlot(slot));
                }
                self.flush_or_pass()
            }
            Key::V => {
                if mods.only_primary() {
                    if repeat {
                        // Holding ⌘V down: nothing to do until a digit or release.
                        return Decision::swallow();
                    }
                    // A second ⌘V while waiting: the engine sends the first
                    // paste and starts waiting again.
                    self.paste_armed_at = Some(now);
                    self.swallowed_v = true;
                    return Decision::swallow_with(Action::PasteArmed);
                }
                self.flush_or_pass()
            }
            Key::C => {
                let decision = self.flush_or_pass();
                if mods.only_primary() && !repeat {
                    self.copy_armed_at = Some(now);
                    if decision.action.is_none() {
                        return Decision::pass_with(Action::CopyArmed);
                    }
                }
                decision
            }
            Key::Digit(_) | Key::Other => self.flush_or_pass(),
        }
    }

    /// Any other key while waiting for a digit: do the normal paste now.
    fn flush_or_pass(&mut self) -> Decision {
        if self.paste_armed_at.take().is_some() {
            return Decision::pass_with(Action::PasteFlush);
        }
        Decision::PASS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { primary: false, alt: false, other: false, shift: false };
    const P: Mods = Mods { primary: true, alt: false, other: false, shift: false };
    const PA: Mods = Mods { primary: true, alt: true, other: false, shift: false };
    const A: Mods = Mods { primary: false, alt: true, other: false, shift: false };

    fn run(f: &mut KeyFilter, evs: &[KeyEvent]) -> Vec<Decision> {
        let t = Instant::now();
        evs.iter().map(|e| f.handle(e, t)).collect()
    }

    fn act(a: Action) -> Option<Action> {
        Some(a)
    }

    #[test]
    fn slot_mapping() {
        assert_eq!(slot_for_digit(1), 0);
        assert_eq!(slot_for_digit(9), 8);
        assert_eq!(slot_for_digit(0), 9);
        assert_eq!(label_for_slot(0), "1");
        assert_eq!(label_for_slot(9), "0");
    }

    // ---- storing ----

    #[test]
    fn copy_then_digit_saves_and_swallows() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[
                KeyEvent::mods(P),
                KeyEvent::down(Key::C, P),
                KeyEvent::up(Key::C, P),
                KeyEvent::down(Key::Digit(3), P),
                KeyEvent::up(Key::Digit(3), P),
                KeyEvent::mods(NONE),
            ],
        );
        assert_eq!(d[1], Decision { swallow: false, action: act(Action::CopyArmed) });
        assert_eq!(d[2], Decision::PASS);
        assert_eq!(d[3], Decision { swallow: true, action: act(Action::SaveSlot(2)) });
        assert!(d[4].swallow, "matching key-up is swallowed");
        assert_eq!(d[5], Decision::PASS);
    }

    #[test]
    fn plain_cmd_digit_passes_through() {
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::Digit(1), P), KeyEvent::up(Key::Digit(1), P)]);
        assert_eq!(d, vec![Decision::PASS, Decision::PASS]);
    }

    #[test]
    fn releasing_primary_disarms_copy() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[
                KeyEvent::down(Key::C, P),
                KeyEvent::mods(NONE),
                KeyEvent::mods(P),
                KeyEvent::down(Key::Digit(1), P),
            ],
        );
        assert_eq!(d[3], Decision::PASS, "Cmd+1 after re-pressing Cmd is a normal shortcut");
    }

    #[test]
    fn copy_window_times_out() {
        let mut f = KeyFilter::new();
        let t0 = Instant::now();
        f.handle(&KeyEvent::down(Key::C, P), t0);
        let later = t0 + COPY_WINDOW + Duration::from_millis(10);
        assert_eq!(f.handle(&KeyEvent::down(Key::Digit(1), P), later), Decision::PASS);
    }

    #[test]
    fn copy_with_extra_modifiers_does_not_arm() {
        let mut f = KeyFilter::new();
        let ps = Mods { shift: true, ..P };
        let d = run(&mut f, &[KeyEvent::down(Key::C, ps), KeyEvent::down(Key::Digit(1), P)]);
        assert_eq!(d, vec![Decision::PASS, Decision::PASS]);
        let d = run(&mut f, &[KeyEvent::down(Key::C, PA), KeyEvent::down(Key::Digit(1), PA)]);
        assert_eq!(d, vec![Decision::PASS, Decision::PASS]);
    }

    #[test]
    fn one_copy_into_several_slots() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[
                KeyEvent::down(Key::C, P),
                KeyEvent::down(Key::Digit(1), P),
                KeyEvent::up(Key::Digit(1), P),
                KeyEvent::down(Key::Digit(0), P),
            ],
        );
        assert_eq!(d[1].action, act(Action::SaveSlot(0)));
        assert_eq!(d[3].action, act(Action::SaveSlot(9)));
    }

    #[test]
    fn copy_digit_autorepeat_is_swallowed_silently() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[KeyEvent::down(Key::C, P), KeyEvent::down(Key::Digit(2), P), KeyEvent::repeat(Key::Digit(2), P)],
        );
        assert_eq!(d[2], Decision { swallow: true, action: None });
    }

    // ---- pasting ----

    #[test]
    fn paste_then_digit_pastes_slot() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[
                KeyEvent::mods(P),
                KeyEvent::down(Key::V, P),
                KeyEvent::down(Key::Digit(4), P),
                KeyEvent::up(Key::V, P),
                KeyEvent::up(Key::Digit(4), P),
                KeyEvent::mods(NONE),
            ],
        );
        assert_eq!(d[1], Decision { swallow: true, action: act(Action::PasteArmed) });
        assert_eq!(d[2], Decision { swallow: true, action: act(Action::PasteSlot(3)) });
        assert!(d[3].swallow, "V key-up swallowed");
        assert!(d[4].swallow, "digit key-up swallowed");
        assert_eq!(d[5], Decision::PASS, "no flush after slot paste");
    }

    #[test]
    fn digit_after_v_released_still_counts() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[KeyEvent::down(Key::V, P), KeyEvent::up(Key::V, P), KeyEvent::down(Key::Digit(0), P)],
        );
        assert_eq!(d[2].action, act(Action::PasteSlot(9)));
    }

    #[test]
    fn paste_flushes_on_primary_release() {
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::V, P), KeyEvent::up(Key::V, P), KeyEvent::mods(NONE)]);
        assert_eq!(d[2], Decision { swallow: false, action: act(Action::PasteFlush) });
    }

    #[test]
    fn paste_flushes_on_other_key() {
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::V, P), KeyEvent::down(Key::Other, P)]);
        assert_eq!(d[1], Decision { swallow: false, action: act(Action::PasteFlush) });
    }

    #[test]
    fn paste_waits_as_long_as_primary_is_held() {
        let mut f = KeyFilter::new();
        let t0 = Instant::now();
        f.handle(&KeyEvent::down(Key::V, P), t0);
        f.handle(&KeyEvent::up(Key::V, P), t0);
        let much_later = t0 + Duration::from_secs(20);
        let d = f.handle(&KeyEvent::down(Key::Digit(5), P), much_later);
        assert_eq!(d, Decision { swallow: true, action: act(Action::PasteSlot(4)) });
    }

    #[test]
    fn double_paste_rearms() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[
                KeyEvent::down(Key::V, P),
                KeyEvent::up(Key::V, P),
                KeyEvent::down(Key::V, P),
                KeyEvent::up(Key::V, P),
                KeyEvent::mods(NONE),
            ],
        );
        assert_eq!(d[2], Decision { swallow: true, action: act(Action::PasteArmed) });
        assert_eq!(d[4].action, act(Action::PasteFlush));
    }

    #[test]
    fn holding_v_is_swallowed_until_digit() {
        let mut f = KeyFilter::new();
        let d = run(
            &mut f,
            &[KeyEvent::down(Key::V, P), KeyEvent::repeat(Key::V, P), KeyEvent::repeat(Key::V, P), KeyEvent::down(Key::Digit(2), P)],
        );
        assert_eq!(d[1], Decision { swallow: true, action: None });
        assert_eq!(d[2], Decision { swallow: true, action: None });
        assert_eq!(d[3].action, act(Action::PasteSlot(1)));
    }

    #[test]
    fn paste_with_extra_modifiers_is_left_alone() {
        // ⌘⌥⇧V ("paste and match style") and friends keep working.
        let mut f = KeyFilter::new();
        let pas = Mods { alt: true, shift: true, ..P };
        let ps = Mods { shift: true, ..P };
        let d = run(&mut f, &[KeyEvent::down(Key::V, pas), KeyEvent::down(Key::V, ps), KeyEvent::down(Key::V, PA)]);
        assert_eq!(d, vec![Decision::PASS; 3]);
    }

    #[test]
    fn alt_digit_without_primary_passes() {
        // e.g. ⌥5 types "[" on a German Mac keyboard
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::Digit(5), A), KeyEvent::up(Key::Digit(5), A)]);
        assert_eq!(d, vec![Decision::PASS, Decision::PASS]);
    }

    #[test]
    fn copy_while_paste_pending_flushes_then_arms_copy() {
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::V, P), KeyEvent::down(Key::C, P), KeyEvent::down(Key::Digit(2), P)]);
        assert_eq!(d[1].action, act(Action::PasteFlush));
        assert_eq!(d[2].action, act(Action::SaveSlot(1)));
    }

    #[test]
    fn paste_digit_wins_over_copy_digit() {
        let mut f = KeyFilter::new();
        let d = run(&mut f, &[KeyEvent::down(Key::C, P), KeyEvent::down(Key::V, P), KeyEvent::down(Key::Digit(1), P)]);
        assert_eq!(d[2].action, act(Action::PasteSlot(0)));
    }

    #[test]
    fn reset_clears_state() {
        let mut f = KeyFilter::new();
        run(&mut f, &[KeyEvent::down(Key::C, P), KeyEvent::down(Key::V, P)]);
        f.reset();
        assert!(!f.is_copy_armed());
        assert!(!f.is_paste_armed());
        assert_eq!(f.handle(&KeyEvent::up(Key::V, P), Instant::now()), Decision::PASS);
    }
}
