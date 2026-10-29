//! CUE chords from keyboard events. Windows key codes (VK), but no Windows API calls.
use cue_core::ui::Mode;

pub const VK_CONTROL: u16 = 0x11;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_MENU: u16 = 0x12;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;
pub const VK_V: u16 = 0x56;
pub const VK_ESCAPE: u16 = 0x1B;
/// An unused key: pressing it between Win and the Win release keeps the Start menu from opening.
pub const VK_MASK: u16 = 0xE8;

/// Bit of the left modifier key (and of the sideless key: `VK_CONTROL`, `VK_SHIFT`, `VK_MENU`).
const LEFT: u8 = 1;
/// Bit of the right modifier key.
const RIGHT: u8 = 2;
/// Modifiers that `Chord::resync` checks (the hook always gives sided codes).
const SIDED: [u16; 8] = [VK_LCONTROL, VK_RCONTROL, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU];

/// Event for the core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyEvent {
    /// The chord formed: Ctrl + Win and at most one of Shift / Alt, which picks the mode.
    ChordDown(Mode),
    /// Alt pressed while the chord is held, with "Ask" on: the dictation asks (spec 2 §4).
    ModeChange(Mode),
    /// A fresh Shift press (not its auto-repeat) while the chord is held, or on its own while CUE records
    /// (`Chord::set_recording`): dictation and translation swap. The core keeps the mode; a release changes nothing.
    /// After a tap, a key or a mouse button pressed while that Shift is still held takes the toggle back with one more
    /// `ModeToggle` (`Decision::undo_toggle`, `Chord::on_mouse_down`).
    ModeToggle,
    /// The pair broke up after `ChordDown`; also comes after `ChordAborted`.
    ChordUp,
    /// A third key was pressed inside the chord: this is a system shortcut, not a dictation.
    ChordAborted,
    /// `Esc` while `esc_active`.
    Esc,
    /// `V` inside the chord: paste the last text.
    PasteLast,
}

/// What the hook should do with a keyboard event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Decision {
    /// Do not pass the event on to the system.
    pub swallow: bool,
    /// Event for the core, if any.
    pub event: Option<HotkeyEvent>,
    /// Before passing this event on, send `VK_MASK` (press and release):
    /// then the Win release will not open the Start menu.
    pub inject_mask_first: bool,
    /// Before `event`, give the core one more `HotkeyEvent::ModeToggle`: this key went down while a Shift that toggled
    /// the mode after a tap is still held, so that Shift was part of a shortcut, a capital letter or the next chord,
    /// and its toggle is taken back.
    pub undo_toggle: bool,
}

/// Chord state. Modifiers as `LEFT` / `RIGHT` bits.
#[derive(Debug)]
pub struct Chord {
    ctrl: u8,
    win: u8,
    shift: u8,
    alt: u8,
    /// Between `ChordDown` and `ChordUp`.
    active: bool,
    /// The chord is cancelled (`ChordAborted` sent, or the pair was formed as a system shortcut: Shift and Alt
    /// together, or Alt while "Ask" is off).
    aborted: bool,
    /// Ctrl + Win + Alt is a chord: "Ask" is on.
    ask: bool,
    /// CUE is starting or recording: a fresh Shift press on its own toggles the mode after the chord was released too.
    recording: bool,
    /// Shift keys (`LEFT` / `RIGHT` bits) that toggled the mode on their own after a tap and are still held: the next
    /// other key or mouse button takes their toggle back.
    undo_shift: u8,
    /// This Win press had a `ChordDown`: the last Win release is masked.
    win_needs_mask: bool,
    /// Alt took part in an "Ask" chord: its last release is masked.
    alt_needs_mask: bool,
    /// Swallowed keys: their auto-repeat and release are swallowed too.
    swallowed: [bool; 256],
    /// Swallowed keys that `resync` considered released: only a release is expected from them;
    /// a new press means the release was lost.
    release_only: [bool; 256],
}

impl Default for Chord {
    fn default() -> Self {
        Self {
            ctrl: 0,
            win: 0,
            shift: 0,
            alt: 0,
            active: false,
            aborted: false,
            ask: false,
            recording: false,
            undo_shift: 0,
            win_needs_mask: false,
            alt_needs_mask: false,
            swallowed: [false; 256],
            release_only: [false; 256],
        }
    }
}

impl Chord {
    /// Whether Ctrl + Win + Alt is a CUE chord ("Ask", spec 2 §4). Off: Alt makes it a system shortcut, as before "Ask" existed.
    pub fn set_ask(&mut self, on: bool) {
        self.ask = on;
    }

    /// Whether CUE is starting or recording (set by the core through the hook). The user often taps the chord and
    /// presses Shift only while speaking: by then the chord is released, and only this flag says the press matters.
    pub fn set_recording(&mut self, on: bool) {
        self.recording = on;
    }

    /// Checks the remembered Ctrl, Win, Shift and Alt against the physical keys: `held(vk)` — is the key held now.
    /// Drops keys whose release never reached the hook (secure desktop after
    /// Ctrl+Alt+Del, UAC prompt, lock screen, elevated window); adds no new presses.
    /// If the Ctrl + Win pair is gone after `ChordDown`, returns `ChordUp` (once).
    /// If no Win is held, the Start menu mask is no longer needed; if no Alt is held, the app menu mask neither.
    /// A swallowed V or Esc that `held` reports as released is switched to "expect only a release":
    /// `GetAsyncKeyState` does not see the swallowed press (it never reached the system), so the key may
    /// still be held. `on_key` will swallow its release; if a new press comes first, the
    /// release was lost, and the press is handled as a fresh one.
    ///
    /// Call ONLY before a non-injected Ctrl or Win press, before `on_key`, with
    /// `held = |vk| GetAsyncKeyState(vk) < 0` (in the hook procedure this state does not include the current event yet).
    /// After such a press auto-repeat moves to Ctrl or Win, and the next event of a swallowed V or Esc
    /// will be its release or a new press. Do not call before releases or before V and Esc events:
    /// for example, before Win↑, while a swallowed V is repeating, its repeats would reach the app.
    /// Exception: a `held` that returns `true` for `VK_V` and `VK_ESCAPE` leaves the V and Esc marks alone — such a call
    /// is safe at any moment (the resync before every press, the chord timer and `after_gap` in `hook.rs` rely on this).
    pub fn resync(&mut self, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
        for vk in SIDED {
            if !held(vk) {
                self.set(vk, false);
            }
        }
        // A Shift whose release never came was released: its toggle stands.
        self.undo_shift &= self.shift;
        for vk in [VK_V, VK_ESCAPE] {
            let idx = usize::from(vk);
            if self.swallowed[idx] && !held(vk) {
                self.release_only[idx] = true;
            }
        }
        if self.win == 0 {
            self.win_needs_mask = false;
        }
        if self.alt == 0 {
            self.alt_needs_mask = false;
        }
        if self.active && !self.paired() {
            self.active = false;
            return Some(HotkeyEvent::ChordUp);
        }
        None
    }

    /// Records a modifier press or release; leaves other keys alone. Returns whether the state changed: a press of a key
    /// already held is its auto-repeat.
    fn set(&mut self, vk: u16, down: bool) -> bool {
        let (slot, bit) = match vk {
            VK_LCONTROL | VK_CONTROL => (&mut self.ctrl, LEFT),
            VK_RCONTROL => (&mut self.ctrl, RIGHT),
            VK_LWIN => (&mut self.win, LEFT),
            VK_RWIN => (&mut self.win, RIGHT),
            VK_LSHIFT | VK_SHIFT => (&mut self.shift, LEFT),
            VK_RSHIFT => (&mut self.shift, RIGHT),
            VK_LMENU | VK_MENU => (&mut self.alt, LEFT),
            VK_RMENU => (&mut self.alt, RIGHT),
            _ => return false,
        };
        let before = *slot;
        if down { *slot |= bit } else { *slot &= !bit }
        *slot != before
    }

    fn paired(&self) -> bool {
        self.ctrl != 0 && self.win != 0
    }

    /// The mode the held Shift and Alt pick, or `None` for a system shortcut: both together (the Office key), or Alt while
    /// "Ask" is off.
    fn mode_now(&self) -> Option<Mode> {
        match (self.shift != 0, self.alt != 0) {
            (false, false) => Some(Mode::Dictate),
            (true, false) => Some(Mode::Translate),
            (false, true) if self.ask => Some(Mode::Ask),
            _ => None,
        }
    }

    /// Whether the chord is held: between `ChordDown` and `ChordUp`.
    pub fn active(&self) -> bool {
        self.active
    }

    /// A mouse button went down (spec 2 §4): while a Shift that toggled after a tap is still held, this is Shift + click —
    /// extending a selection — and the toggle is taken back, as another key takes it back. A Shift that `held` reports
    /// released lost its release on the way: its toggle stands. Returns whether the core gets one more `ModeToggle`.
    pub fn on_mouse_down(&mut self, held: impl Fn(u16) -> bool) -> bool {
        for vk in [VK_LSHIFT, VK_RSHIFT] {
            if !held(vk) {
                self.set(vk, false);
            }
        }
        self.undo_shift &= self.shift;
        let undo = self.undo_shift.count_ones() % 2 == 1;
        self.undo_shift = 0;
        undo
    }

    /// A mouse button press would take a Shift toggle back now: the mouse hook is needed only while this holds.
    pub fn watches_mouse(&self) -> bool {
        self.undo_shift != 0
    }

    /// Decision for one keyboard hook event.
    ///
    /// - `vk` — virtual key code; `down` — press (including auto-repeat), otherwise release.
    /// - `injected` — the event was sent by a program (injected flag): state does not change, the event passes.
    /// - `esc_active` — CUE is in "Recording" or "Thinking": an Esc press is swallowed and gives `HotkeyEvent::Esc`.
    ///
    /// A swallowed key (V, Esc) stays swallowed — auto-repeat and release — until it is released,
    /// regardless of `esc_active` and of whether the chord is still held.
    /// The chord is Ctrl + Win and at most one of Shift / Alt (spec 2 §4): Shift translates, Alt asks (only while "Ask"
    /// is on). If Shift and Alt together (the Office key), or Alt while "Ask" is off, are held when the pair forms, this
    /// is a system shortcut — no events, no swallowed V, no mask: Windows sees everything as is. Every fresh Shift press
    /// while the chord is held toggles the mode, its auto-repeat and its release do not. While CUE records with the chord
    /// released (`set_recording`), only Shift on its own toggles (Ctrl, Win and Alt up), and the next other key pressed
    /// while that Shift is still held takes the toggle back (`Decision::undo_toggle`; a mouse button too,
    /// `on_mouse_down`). An Alt press inside a held chord picks "Ask". Shift and Alt together inside a held chord cancel
    /// it and drop the masks. Any other third key (Alt while "Ask" is off included) cancels the chord, and the mask
    /// stays: this Win press had a `ChordDown`.
    pub fn on_key(&mut self, vk: u16, down: bool, injected: bool, esc_active: bool) -> Decision {
        let mut d = Decision::default();
        if injected {
            return d;
        }
        let idx = (vk & 0xFF) as usize;
        if self.swallowed[idx] {
            if down && self.release_only[idx] {
                // The release was lost: this is a new press, handle it as usual.
                self.swallowed[idx] = false;
                self.release_only[idx] = false;
            } else {
                if !down {
                    self.swallowed[idx] = false;
                    self.release_only[idx] = false;
                }
                d.swallow = true;
                return d;
            }
        }
        let was_paired = self.paired();
        let fresh = self.set(vk, down) && down;
        if down && !matches!(vk, VK_LSHIFT | VK_RSHIFT | VK_SHIFT) && self.undo_shift != 0 {
            // A key went down while a Shift that toggled after a tap is still held: that Shift was part of a shortcut, a
            // capital letter or the next chord (Shift + Ctrl + Win), not a switch. Each such toggle is taken back once,
            // before this key's own event (a finishing chord then ends in the mode it started with), and two of them
            // cancel out.
            d.undo_toggle = self.undo_shift.count_ones() % 2 == 1;
            self.undo_shift = 0;
        }
        match vk {
            VK_LCONTROL | VK_RCONTROL | VK_CONTROL | VK_LWIN | VK_RWIN => {
                let paired = self.paired();
                if paired && !was_paired {
                    // Exactly one of Shift / Alt picks the mode; both, or Alt while "Ask" is off, is a system shortcut: the
                    // chord is cancelled at once and transparent (no mask).
                    let mode = self.mode_now();
                    self.active = mode.is_some();
                    self.aborted = !self.active;
                    if let Some(mode) = mode {
                        self.win_needs_mask = true;
                        self.alt_needs_mask = mode == Mode::Ask;
                        d.event = Some(HotkeyEvent::ChordDown(mode));
                    }
                } else if !paired && self.active {
                    self.active = false;
                    d.event = Some(HotkeyEvent::ChordUp);
                }
                let is_win = vk == VK_LWIN || vk == VK_RWIN;
                if !down && is_win && self.win == 0 && self.win_needs_mask {
                    self.win_needs_mask = false;
                    d.inject_mask_first = true;
                }
            }
            VK_LSHIFT | VK_RSHIFT | VK_SHIFT | VK_LMENU | VK_RMENU | VK_MENU => {
                let is_alt = matches!(vk, VK_LMENU | VK_RMENU | VK_MENU);
                if down && self.active && self.shift != 0 && self.alt != 0 {
                    // The Office key: Windows must see it as is, the Start menu and Office included. Also when Alt has
                    // already cancelled the chord ("Ask" off): the result must not depend on the press order.
                    self.win_needs_mask = false;
                    self.alt_needs_mask = false;
                    if !self.aborted {
                        self.aborted = true;
                        d.event = Some(HotkeyEvent::ChordAborted);
                    }
                } else if is_alt {
                    if down && self.active && !self.aborted {
                        if !self.ask {
                            // Alt while "Ask" is off: a system shortcut, like any other third key.
                            self.aborted = true;
                            d.event = Some(HotkeyEvent::ChordAborted);
                        } else if fresh {
                            self.alt_needs_mask = true;
                            d.event = Some(HotkeyEvent::ModeChange(Mode::Ask));
                        }
                    }
                    if !down && self.alt == 0 && self.alt_needs_mask {
                        self.alt_needs_mask = false;
                        d.inject_mask_first = true;
                    }
                } else {
                    // Shift toggles: the core keeps the mode, so the chord need not know it. A release never toggles: the
                    // order of releases must not change the result.
                    if fresh && self.active {
                        // Inside a held chord the event goes out even before the core has said it records: the flow
                        // drops it when nothing records (the press finished the recording, or a system shortcut already
                        // dropped it).
                        d.event = Some(HotkeyEvent::ModeToggle);
                    } else if fresh && self.recording && self.ctrl == 0 && self.win == 0 && self.alt == 0 {
                        // After a tap only Shift on its own counts. With Ctrl, Win or Alt down it belongs to another
                        // shortcut or to the finishing chord (Ctrl + Shift + Win), which must not turn a translation into
                        // a dictation. A Shift pressed before the other keys is caught by the next key (`undo_shift`).
                        self.undo_shift |= if vk == VK_RSHIFT { RIGHT } else { LEFT };
                        d.event = Some(HotkeyEvent::ModeToggle);
                    }
                    // A released Shift's toggle stands: no key can take it back any more.
                    self.undo_shift &= self.shift;
                }
            }
            VK_V if down && self.active => {
                self.swallowed[idx] = true;
                d.swallow = true;
                d.event = Some(HotkeyEvent::PasteLast);
            }
            VK_ESCAPE if down && esc_active => {
                self.swallowed[idx] = true;
                d.swallow = true;
                d.event = Some(HotkeyEvent::Esc);
            }
            _ => {
                if down && self.active && !self.aborted {
                    self.aborted = true;
                    d.event = Some(HotkeyEvent::ChordAborted);
                }
            }
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HotkeyEvent::*;

    const DOWN: HotkeyEvent = HotkeyEvent::ChordDown(Mode::Dictate);
    const VK_D: u16 = 0x44;
    const VK_B: u16 = 0x42;
    const VK_L: u16 = 0x4C;
    const VK_DELETE: u16 = 0x2E;

    fn press(c: &mut Chord, vk: u16) -> Decision { c.on_key(vk, true, false, false) }
    fn release(c: &mut Chord, vk: u16) -> Decision { c.on_key(vk, false, false, false) }

    #[test]
    fn ctrl_then_win_fires_once() {
        let mut c = Chord::default();
        assert_eq!(press(&mut c, VK_LCONTROL).event, None);
        let d = press(&mut c, VK_LWIN);
        assert_eq!(d.event, Some(DOWN));
        assert!(!d.swallow, "Ctrl and Win are not swallowed");
        assert_eq!(press(&mut c, VK_LWIN).event, None, "auto-repeat does not repeat the event");
    }

    #[test]
    fn win_then_ctrl_and_right_keys() {
        let mut c = Chord::default();
        press(&mut c, VK_RWIN);
        assert_eq!(press(&mut c, VK_RCONTROL).event, Some(DOWN));
    }

    #[test]
    fn release_sends_up_and_masks_start_menu() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        let d = release(&mut c, VK_LWIN);
        assert_eq!(d.event, Some(ChordUp));
        assert!(d.inject_mask_first);
        let d = release(&mut c, VK_LCONTROL);
        assert_eq!(d.event, None);
        assert!(!d.inject_mask_first);
    }

    #[test]
    fn ctrl_released_first_masks_later_win_up() {
        let mut c = Chord::default();
        press(&mut c, VK_LWIN);
        press(&mut c, VK_LCONTROL);
        assert_eq!(release(&mut c, VK_LCONTROL).event, Some(ChordUp));
        assert!(release(&mut c, VK_LWIN).inject_mask_first);
    }

    #[test]
    fn win_alone_is_not_masked() {
        let mut c = Chord::default();
        press(&mut c, VK_LWIN);
        press(&mut c, VK_D);
        let d = release(&mut c, VK_LWIN);
        assert_eq!(d, Decision::default());
    }

    #[test]
    fn third_key_aborts_and_passes() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        let d = press(&mut c, VK_D);
        assert_eq!(d.event, Some(ChordAborted));
        assert!(!d.swallow);
        assert_eq!(press(&mut c, VK_L).event, None, "the abort is reported once");
    }

    #[test]
    fn v_inside_chord_is_paste_last_and_swallowed() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        let d = press(&mut c, VK_V);
        assert_eq!(d, Decision { swallow: true, event: Some(PasteLast), inject_mask_first: false, undo_toggle: false });
        assert_eq!(press(&mut c, VK_V), Decision { swallow: true, event: None, inject_mask_first: false, undo_toggle: false });
        release(&mut c, VK_LWIN);
        assert!(release(&mut c, VK_V).swallow, "the release of a swallowed V is swallowed too");
        assert!(!press(&mut c, VK_V).swallow, "a normal V passes");
    }

    #[test]
    fn esc_only_when_active() {
        let mut c = Chord::default();
        assert_eq!(c.on_key(VK_ESCAPE, true, false, false), Decision::default());
        c.on_key(VK_ESCAPE, false, false, false);
        let d = c.on_key(VK_ESCAPE, true, false, true);
        assert_eq!(d, Decision { swallow: true, event: Some(Esc), inject_mask_first: false, undo_toggle: false });
        assert!(c.on_key(VK_ESCAPE, false, false, true).swallow);
    }

    #[test]
    fn injected_events_are_ignored() {
        let mut c = Chord::default();
        c.on_key(VK_LCONTROL, true, true, false);
        assert_eq!(c.on_key(VK_LWIN, true, true, false), Decision::default());
    }

    const SWALLOWED: Decision = Decision { swallow: true, event: None, inject_mask_first: false, undo_toggle: false };
    const MASK_ONLY: Decision = Decision { swallow: false, event: None, inject_mask_first: true, undo_toggle: false };

    #[test]
    fn swallowed_key_stays_swallowed_until_release() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_V).event, Some(PasteLast));
        assert_eq!(release(&mut c, VK_LWIN).event, Some(ChordUp));
        release(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_V), SWALLOWED, "V auto-repeat after the chord ends is swallowed");
        assert_eq!(release(&mut c, VK_V), SWALLOWED);
        assert_eq!(press(&mut c, VK_V), Decision::default(), "a new V press passes");

        assert_eq!(c.on_key(VK_ESCAPE, true, false, true).event, Some(Esc));
        // The core cancelled the dictation: esc_active is already false, but Esc is still held.
        assert_eq!(c.on_key(VK_ESCAPE, true, false, false), SWALLOWED, "Esc auto-repeat after the cancel is swallowed");
        assert_eq!(c.on_key(VK_ESCAPE, false, false, false), SWALLOWED);
        assert_eq!(c.on_key(VK_ESCAPE, true, false, false), Decision::default(), "a new Esc press passes");
    }

    #[test]
    fn resync_drops_ctrl_lost_on_secure_desktop() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LMENU);
        press(&mut c, VK_DELETE);
        // The releases went to the secure desktop (Ctrl+Alt+Del → "Cancel").
        assert_eq!(c.resync(|_| false), None);
        assert_eq!(press(&mut c, VK_LWIN).event, None, "a lone Win does not start a dictation");
        assert_eq!(release(&mut c, VK_LWIN), Decision::default(), "and the Start menu is not masked");
    }

    #[test]
    fn resync_drops_win_lost_on_lock_screen() {
        let mut c = Chord::default();
        press(&mut c, VK_LWIN);
        press(&mut c, VK_L);
        // Win+L: the Win release happened on the lock screen.
        assert_eq!(c.resync(|_| false), None);
        assert_eq!(press(&mut c, VK_LCONTROL).event, None, "Ctrl+C after unlocking is not a dictation");
    }

    #[test]
    fn resync_ends_chord_when_pair_is_gone() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(c.resync(|vk| vk == VK_LWIN), Some(ChordUp), "Ctrl was released unnoticed");
        assert_eq!(c.resync(|vk| vk == VK_LWIN), None, "ChordUp is reported once");
        assert_eq!(release(&mut c, VK_LWIN), MASK_ONLY, "Win was in the chord — the mask stays");
    }

    #[test]
    fn resync_keeps_complete_pair() {
        let mut c = Chord::default();
        press(&mut c, VK_RCONTROL);
        press(&mut c, VK_RWIN);
        assert_eq!(c.resync(|vk| vk == VK_RCONTROL || vk == VK_RWIN), None);
        let d = release(&mut c, VK_RWIN);
        assert_eq!(d.event, Some(ChordUp));
        assert!(d.inject_mask_first);
    }

    #[test]
    fn resync_without_win_drops_mask() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        // Both releases were lost.
        assert_eq!(c.resync(|_| false), Some(ChordUp));
        press(&mut c, VK_LWIN);
        assert_eq!(release(&mut c, VK_LWIN), Decision::default(), "a lone Win opens the Start menu again");
    }

    #[test]
    fn resync_drops_shift_and_alt() {
        let mut c = Chord::default();
        press(&mut c, VK_LSHIFT);
        press(&mut c, VK_RMENU);
        assert_eq!(c.resync(|_| false), None);
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
    }

    #[test]
    fn resync_of_passed_system_chord_is_silent() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
        press(&mut c, VK_LMENU);
        press(&mut c, VK_LWIN);
        assert_eq!(c.resync(|_| false), None, "there was no ChordDown — no ChordUp needed");
        press(&mut c, VK_LWIN);
        assert_eq!(release(&mut c, VK_LWIN), Decision::default());
    }

    #[test]
    fn chord_up_and_mask_follow_abort() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_D).event, Some(ChordAborted));
        release(&mut c, VK_D);
        let d = release(&mut c, VK_LWIN);
        assert_eq!(d.event, Some(ChordUp), "the core ends the hold on ChordUp after an abort too");
        assert!(d.inject_mask_first);
        assert_eq!(release(&mut c, VK_LCONTROL), Decision::default());
    }

    #[test]
    fn alt_inside_chord_aborts_while_ask_is_off() {
        let mut c = Chord::default();
        press(&mut c, VK_LWIN);
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LMENU).event, Some(ChordAborted));
    }

    #[test]
    fn ctrl_shift_win_b_starts_translate_then_aborts() {
        // Ctrl+Shift+Win+B restarts the graphics driver: the recording starts, the third key drops it (flow: SYSTEM_CHORD_MS).
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown(Mode::Translate)));
        let d = press(&mut c, VK_B);
        assert_eq!(d.event, Some(ChordAborted));
        assert!(!d.swallow, "Windows still gets B");
    }

    #[test]
    fn office_key_does_not_start_dictation() {
        // The Office key (Hyper) = Ctrl+Shift+Alt+Win.
        let mut c = Chord::default();
        for vk in [VK_LCONTROL, VK_LSHIFT, VK_LMENU, VK_LWIN] {
            assert_eq!(press(&mut c, vk), Decision::default(), "press {vk:#04x}");
        }
        assert_eq!(press(&mut c, VK_V), Decision::default(), "V is not swallowed");
        assert_eq!(release(&mut c, VK_V), Decision::default());
        for vk in [VK_LMENU, VK_LSHIFT, VK_LCONTROL] {
            assert_eq!(release(&mut c, vk), Decision::default(), "release {vk:#04x}");
        }
        assert_eq!(release(&mut c, VK_LWIN), Decision::default(), "no mask — the Office key works as usual");
    }

    #[test]
    fn alt_held_lets_chord_pass() {
        let mut c = Chord::default();
        press(&mut c, VK_RMENU);
        press(&mut c, VK_RWIN);
        assert_eq!(press(&mut c, VK_RCONTROL), Decision::default());
        assert_eq!(press(&mut c, VK_D), Decision::default(), "the chord never started — nothing to abort");
        release(&mut c, VK_D);
        assert_eq!(release(&mut c, VK_RMENU), Decision::default(), "the pair existed before Alt went away — it does not start");
        assert_eq!(release(&mut c, VK_RWIN), Decision::default(), "CUE did not step in — no mask");
        assert_eq!(press(&mut c, VK_RWIN).event, Some(DOWN), "without Alt — a normal chord again");
    }

    #[test]
    fn shift_after_fired_chord_starts_translate() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
        assert_eq!(release(&mut c, VK_LCONTROL).event, Some(ChordUp));
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LCONTROL).event, Some(ChordDown(Mode::Translate)));
        assert_eq!(release(&mut c, VK_LCONTROL).event, Some(ChordUp));
        release(&mut c, VK_LSHIFT);
        assert_eq!(release(&mut c, VK_LWIN), MASK_ONLY, "this Win press had a ChordDown — the mask is needed");
    }

    #[test]
    fn resync_clears_stale_swallow() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_V).event, Some(PasteLast));
        assert_eq!(c.resync(|vk| matches!(vk, VK_LCONTROL | VK_LWIN | VK_V)), None);
        assert_eq!(press(&mut c, VK_V), SWALLOWED, "V is held — swallowing stays");
        // The V release was lost.
        release(&mut c, VK_LWIN);
        release(&mut c, VK_LCONTROL);
        assert_eq!(c.resync(|_| false), None);
        assert_eq!(press(&mut c, VK_V), Decision::default(), "the next V press passes");

        assert_eq!(c.on_key(VK_ESCAPE, true, false, true).event, Some(Esc));
        // The Esc release was lost.
        assert_eq!(c.resync(|_| false), None);
        assert_eq!(c.on_key(VK_ESCAPE, true, false, false), Decision::default(), "the next Esc press passes");
    }

    #[test]
    fn resync_keeps_release_of_held_swallowed_key() {
        // A swallowed press never reached the system: GetAsyncKeyState reports V and Esc as released although they are held.
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_V).event, Some(PasteLast));
        assert_eq!(release(&mut c, VK_LWIN).event, Some(ChordUp));
        assert_eq!(c.resync(|vk| vk == VK_LCONTROL), None, "before a new Win press");
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
        assert_eq!(release(&mut c, VK_V), SWALLOWED, "the release of a held V does not reach the app");
        release(&mut c, VK_LWIN);
        release(&mut c, VK_LCONTROL);

        assert_eq!(c.on_key(VK_ESCAPE, true, false, true).event, Some(Esc));
        assert_eq!(c.resync(|_| false), None, "before a Ctrl press, Esc is still held");
        press(&mut c, VK_LCONTROL);
        assert_eq!(c.on_key(VK_ESCAPE, false, false, false), SWALLOWED, "and the Esc release too");
        assert_eq!(c.on_key(VK_ESCAPE, true, false, false), Decision::default(), "the next Esc press passes");
    }

    #[test]
    fn press_after_lost_release_is_fresh() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_V).event, Some(PasteLast));
        // The V release was lost.
        release(&mut c, VK_LWIN);
        assert_eq!(c.resync(|vk| vk == VK_LCONTROL), None);
        press(&mut c, VK_LWIN);
        let fresh = Decision { swallow: true, event: Some(PasteLast), inject_mask_first: false, undo_toggle: false };
        assert_eq!(press(&mut c, VK_V), fresh, "a new V press is \"paste last\" again");
        assert_eq!(release(&mut c, VK_V), SWALLOWED);
    }

    #[test]
    fn both_keys_of_a_pair() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_RCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
        assert_eq!(press(&mut c, VK_RWIN).event, None);
        assert_eq!(release(&mut c, VK_LCONTROL), Decision::default(), "the right Ctrl is still held");
        assert_eq!(release(&mut c, VK_LWIN), Decision::default(), "the right Win is still held — no ChordUp, no mask");
        let d = release(&mut c, VK_RWIN);
        assert_eq!(d.event, Some(ChordUp));
        assert!(d.inject_mask_first, "the mask is on the last Win release");
        assert_eq!(release(&mut c, VK_RCONTROL), Decision::default());
    }

    #[test]
    fn esc_inside_chord() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        let d = c.on_key(VK_ESCAPE, true, false, true);
        assert_eq!(d, Decision { swallow: true, event: Some(Esc), inject_mask_first: false, undo_toggle: false }, "CUE is recording — Esc cancels");
        c.on_key(VK_ESCAPE, false, false, true);
        let d = press(&mut c, VK_ESCAPE);
        assert_eq!(d, Decision { swallow: false, event: Some(ChordAborted), inject_mask_first: false, undo_toggle: false }, "otherwise Esc is a third key");
    }

    // ---- Modes: Shift translates, Alt asks (spec 2 §4) ----

    fn ask_chord() -> Chord {
        let mut c = Chord::default();
        c.set_ask(true);
        c
    }

    #[test]
    fn shift_held_when_the_pair_forms_translates() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown(Mode::Translate)));
    }

    const TOGGLE: Decision = Decision { swallow: false, event: Some(ModeToggle), inject_mask_first: false, undo_toggle: false };

    #[test]
    fn every_fresh_shift_press_inside_the_chord_toggles() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_RSHIFT), TOGGLE, "Shift is not swallowed");
        assert_eq!(press(&mut c, VK_RSHIFT), Decision::default(), "auto-repeat does not toggle again");
        assert_eq!(release(&mut c, VK_RSHIFT), Decision::default(), "a release changes nothing");
        assert_eq!(press(&mut c, VK_RSHIFT), TOGGLE, "a second press toggles back");
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE, "so does the other Shift");
        release(&mut c, VK_LSHIFT);
        release(&mut c, VK_RSHIFT);
        let d = release(&mut c, VK_LWIN);
        assert_eq!(d.event, Some(ChordUp));
        assert!(d.inject_mask_first);
    }

    #[test]
    fn shift_after_a_tap_toggles_while_recording() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
        assert_eq!(release(&mut c, VK_LWIN), Decision { swallow: false, event: Some(ChordUp), inject_mask_first: true, undo_toggle: false });
        release(&mut c, VK_LCONTROL);
        // The core has started the recording.
        c.set_recording(true);
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert_eq!(press(&mut c, VK_LSHIFT), Decision::default(), "auto-repeat of a held Shift toggles once");
        assert_eq!(press(&mut c, VK_LSHIFT), Decision::default());
        assert_eq!(release(&mut c, VK_LSHIFT), Decision::default(), "a release changes nothing");
        assert_eq!(press(&mut c, VK_RSHIFT), TOGGLE);
        assert_eq!(release(&mut c, VK_RSHIFT), Decision::default());
        assert_eq!(press(&mut c, VK_LCONTROL), Decision::default());
        assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN), "the finishing tap is an ordinary chord");
        release(&mut c, VK_LWIN);
        release(&mut c, VK_LCONTROL);
        // The recording is over: thinking, done, idle.
        c.set_recording(false);
        assert_eq!(press(&mut c, VK_LSHIFT), Decision::default(), "Shift outside a recording does nothing");
        assert_eq!(release(&mut c, VK_LSHIFT), Decision::default());
    }

    #[test]
    fn translate_chord_toggles_back_with_a_new_shift_press() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown(Mode::Translate)));
        assert_eq!(release(&mut c, VK_LSHIFT), Decision::default(), "a release changes nothing");
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE, "inside the held chord");
        release(&mut c, VK_LSHIFT);
        release(&mut c, VK_LWIN);
        release(&mut c, VK_LCONTROL);
        c.set_recording(true);
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE, "after the release");
    }

    const NOTHING: [HotkeyEvent; 0] = [];

    /// The events a decision gives the core, in order: the undo of a Shift toggle comes first.
    fn sent(d: Decision) -> Vec<HotkeyEvent> {
        d.undo_toggle.then_some(ModeToggle).into_iter().chain(d.event).collect()
    }

    /// A chord whose tap started a recording: the pair is up again, and the core says it records.
    fn tapped() -> Chord {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        release(&mut c, VK_LWIN);
        release(&mut c, VK_LCONTROL);
        c.set_recording(true);
        c
    }

    /// Presses the keys in order, then releases them in the same order: the events the core gets.
    fn keys(c: &mut Chord, keys: &[u16]) -> Vec<HotkeyEvent> {
        let mut out: Vec<HotkeyEvent> = keys.iter().flat_map(|&vk| sent(press(c, vk))).collect();
        out.extend(keys.iter().flat_map(|&vk| sent(release(c, vk))));
        out
    }

    #[test]
    fn shift_in_the_finishing_chord_after_a_tap_leaves_the_mode_alone() {
        let finish = [ChordDown(Mode::Translate), ChordUp];
        let undone = [ModeToggle, ModeToggle, ChordDown(Mode::Translate), ChordUp];
        assert_eq!(keys(&mut tapped(), &[VK_LSHIFT, VK_LCONTROL, VK_LWIN]), undone, "Shift first: toggled, then undone");
        assert_eq!(keys(&mut tapped(), &[VK_RSHIFT, VK_RWIN, VK_RCONTROL]), undone);
        assert_eq!(keys(&mut tapped(), &[VK_LCONTROL, VK_LSHIFT, VK_LWIN]), finish, "Shift after Ctrl does not toggle");
        assert_eq!(keys(&mut tapped(), &[VK_LWIN, VK_RSHIFT, VK_LCONTROL]), finish, "Shift after Win does not toggle");
    }

    #[test]
    fn shift_with_other_keys_after_a_tap_leaves_the_mode_alone() {
        assert_eq!(keys(&mut tapped(), &[VK_LSHIFT, VK_D]), [ModeToggle, ModeToggle], "a capital letter");
        assert_eq!(keys(&mut tapped(), &[VK_LSHIFT, VK_LMENU]), [ModeToggle, ModeToggle], "Shift + Alt");
        assert_eq!(keys(&mut tapped(), &[VK_LMENU, VK_LSHIFT]), NOTHING, "Alt + Shift");
        assert_eq!(keys(&mut tapped(), &[VK_LCONTROL, VK_LSHIFT, VK_D]), NOTHING, "Ctrl + Shift + a key");
        assert_eq!(keys(&mut tapped(), &[VK_LWIN, VK_LSHIFT, VK_L]), NOTHING, "Win + Shift + a key");
        let mut c = tapped();
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert_eq!(sent(press(&mut c, VK_D)), [ModeToggle], "the first key takes the toggle back");
        release(&mut c, VK_D);
        assert_eq!(sent(press(&mut c, VK_B)), NOTHING, "the next ones do not: once per Shift press");
        release(&mut c, VK_B);
        assert_eq!(release(&mut c, VK_LSHIFT), Decision::default());
    }

    #[test]
    fn a_lone_shift_after_a_tap_toggles_for_good() {
        let mut c = tapped();
        assert_eq!(keys(&mut c, &[VK_LSHIFT]), [ModeToggle]);
        assert_eq!(keys(&mut c, &[VK_D]), NOTHING, "a key after the Shift release takes nothing back");
        assert_eq!(keys(&mut c, &[VK_RSHIFT]), [ModeToggle]);
        assert_eq!(keys(&mut c, &[VK_LCONTROL, VK_LWIN]), [ChordDown(Mode::Dictate), ChordUp], "the finishing tap");
    }

    #[test]
    fn each_toggling_shift_is_taken_back_once() {
        // Both Shifts held: two toggles, so the key takes back two, which is no toggle at all.
        let mut c = tapped();
        assert_eq!(keys(&mut c, &[VK_LSHIFT, VK_RSHIFT, VK_D]), [ModeToggle, ModeToggle]);
        // The right Shift was released first: its toggle stands, the left one's is taken back.
        let mut c = tapped();
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert_eq!(press(&mut c, VK_RSHIFT), TOGGLE);
        release(&mut c, VK_RSHIFT);
        assert_eq!(sent(press(&mut c, VK_D)), [ModeToggle]);
    }

    #[test]
    fn esc_with_the_toggling_shift_held_takes_the_toggle_back_first() {
        let mut c = tapped();
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        let d = c.on_key(VK_ESCAPE, true, false, true);
        assert_eq!(d, Decision { swallow: true, event: Some(Esc), inject_mask_first: false, undo_toggle: true });
        assert_eq!(sent(d), [ModeToggle, Esc], "the mode is back before the cancel, so Restore refines in it");
    }

    /// Spec 2 §4: Shift + click extends a selection while CUE records; the click takes the Shift's toggle back.
    #[test]
    fn a_click_while_the_toggling_shift_is_held_takes_its_toggle_back() {
        let mut c = tapped();
        assert!(!c.watches_mouse());
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert!(c.watches_mouse(), "the mouse matters while that Shift is held");
        assert!(c.on_mouse_down(|vk| vk == VK_LSHIFT));
        assert!(!c.watches_mouse());
        assert!(!c.on_mouse_down(|vk| vk == VK_LSHIFT), "a second click takes nothing back: once per Shift press");
        assert_eq!(sent(press(&mut c, VK_D)), NOTHING, "nor does a key after it");
        release(&mut c, VK_D);
        release(&mut c, VK_LSHIFT);
        // The right Shift was released first: its toggle stands, the left one's is taken back.
        let mut c = tapped();
        press(&mut c, VK_LSHIFT);
        press(&mut c, VK_RSHIFT);
        release(&mut c, VK_RSHIFT);
        assert!(c.on_mouse_down(|vk| vk == VK_LSHIFT));
        // Both Shifts toggled and held: the click takes back two, which is no toggle at all, as with a key.
        let mut c = tapped();
        press(&mut c, VK_LSHIFT);
        press(&mut c, VK_RSHIFT);
        assert!(!c.on_mouse_down(|vk| vk == VK_LSHIFT || vk == VK_RSHIFT), "both Shifts");
        assert!(!c.watches_mouse());
    }

    #[test]
    fn a_click_takes_nothing_back_without_a_held_toggling_shift() {
        assert!(!tapped().on_mouse_down(|_| false), "no Shift");
        let mut c = tapped();
        assert_eq!(keys(&mut c, &[VK_LSHIFT]), [ModeToggle]);
        assert!(!c.watches_mouse());
        assert!(!c.on_mouse_down(|_| false), "Shift pressed and released before the click");
        let mut c = Chord::default();
        c.set_recording(true);
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert!(!c.watches_mouse());
        assert!(!c.on_mouse_down(|vk| [VK_LCONTROL, VK_LWIN, VK_LSHIFT].contains(&vk)), "Shift inside a held chord");
        let mut c = tapped();
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LSHIFT), Decision::default());
        assert!(!c.on_mouse_down(|vk| [VK_LCONTROL, VK_LSHIFT].contains(&vk)), "Shift with Ctrl down after a tap");
        // The Shift release went to the secure desktop: the toggle stands, and nothing is left to take back.
        let mut c = tapped();
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert!(!c.on_mouse_down(|_| false), "a lost Shift release");
        assert!(!c.watches_mouse());
        assert_eq!(sent(press(&mut c, VK_D)), NOTHING);
    }

    #[test]
    fn inside_a_held_chord_other_keys_do_not_take_the_toggle_back() {
        let mut c = Chord::default();
        c.set_recording(true);
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_LSHIFT), TOGGLE);
        assert_eq!(sent(press(&mut c, VK_D)), [ChordAborted], "a third key aborts, as before");
    }

    #[test]
    fn injected_shift_does_nothing() {
        let mut c = Chord::default();
        c.set_recording(true);
        assert_eq!(c.on_key(VK_LSHIFT, true, true, true), Decision::default());
        assert_eq!(c.on_key(VK_LSHIFT, false, true, true), Decision::default());
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(c.on_key(VK_RSHIFT, true, true, true), Decision::default(), "inside the chord too");
        assert_eq!(press(&mut c, VK_RSHIFT), TOGGLE, "the injected press left no Shift behind");
    }

    #[test]
    fn shift_while_idle_does_nothing() {
        let mut c = Chord::default();
        assert_eq!(press(&mut c, VK_LSHIFT), Decision::default());
        assert_eq!(release(&mut c, VK_LSHIFT), Decision::default());
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_RSHIFT), Decision::default(), "Ctrl alone is no chord");
    }

    #[test]
    fn paste_last_works_in_translate_mode() {
        let mut c = Chord::default();
        for vk in [VK_LCONTROL, VK_LSHIFT, VK_LWIN] {
            press(&mut c, vk);
        }
        assert_eq!(press(&mut c, VK_V).event, Some(PasteLast));
    }

    #[test]
    fn alt_held_when_the_pair_forms_asks_only_when_enabled() {
        let mut off = Chord::default();
        press(&mut off, VK_LMENU);
        press(&mut off, VK_LCONTROL);
        assert_eq!(press(&mut off, VK_LWIN), Decision::default(), "\"Ask\" off: a system shortcut");
        let mut c = ask_chord();
        press(&mut c, VK_LMENU);
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown(Mode::Ask)));
        assert_eq!(release(&mut c, VK_LMENU), MASK_ONLY, "the Alt the chord formed with is masked too");
        assert_eq!(release(&mut c, VK_LWIN).event, Some(ChordUp));
    }

    #[test]
    fn alt_release_after_ask_is_masked() {
        let mut c = ask_chord();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_LMENU).event, Some(ModeChange(Mode::Ask)));
        let d = release(&mut c, VK_LMENU);
        assert_eq!(d, MASK_ONLY, "the app's menu must not open on the Alt release");
        assert_eq!(release(&mut c, VK_LWIN).event, Some(ChordUp));
    }

    #[test]
    fn translate_can_switch_to_ask_while_held() {
        let mut c = ask_chord();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
        press(&mut c, VK_LWIN);
        release(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_RMENU).event, Some(ModeChange(Mode::Ask)));
        assert_eq!(press(&mut c, VK_RMENU).event, None, "auto-repeat does not repeat the change");
    }

    #[test]
    fn ask_can_be_picked_again_after_a_shift_toggle() {
        let mut c = ask_chord();
        press(&mut c, VK_LMENU);
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown(Mode::Ask)));
        release(&mut c, VK_LMENU);
        assert_eq!(press(&mut c, VK_LSHIFT).event, Some(ModeToggle), "the flow turns \"Ask\" into a translation");
        release(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LMENU).event, Some(ModeChange(Mode::Ask)), "a fresh Alt asks again");
    }

    #[test]
    fn shift_and_alt_together_are_the_office_key_and_drop_the_mask() {
        let mut c = ask_chord();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LMENU).event, Some(ChordAborted));
        assert_eq!(release(&mut c, VK_LMENU), Decision::default(), "no Alt mask");
        release(&mut c, VK_LSHIFT);
        let d = release(&mut c, VK_LWIN);
        assert_eq!(d.event, Some(ChordUp));
        assert!(!d.inject_mask_first, "Windows sees the Office key as is");
    }

    #[test]
    fn resync_drops_the_alt_mask_when_alt_is_gone() {
        let mut c = ask_chord();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        press(&mut c, VK_LMENU);
        // All releases were lost (lock screen).
        assert_eq!(c.resync(|_| false), Some(ChordUp));
        press(&mut c, VK_LMENU);
        assert_eq!(release(&mut c, VK_LMENU), Decision::default());
    }

    #[test]
    fn office_key_inside_the_chord_does_not_depend_on_press_order() {
        for ask in [false, true] {
            for order in [[VK_LSHIFT, VK_LMENU], [VK_LMENU, VK_LSHIFT]] {
                let case = format!("ask={ask} order={order:x?}");
                let mut c = Chord::default();
                c.set_ask(ask);
                press(&mut c, VK_LCONTROL);
                assert_eq!(press(&mut c, VK_LWIN).event, Some(DOWN));
                let events: Vec<HotkeyEvent> = order.iter().filter_map(|&vk| press(&mut c, vk).event).collect();
                assert_eq!(events.iter().filter(|&&e| e == ChordAborted).count(), 1, "{case}: {events:?}");
                assert_eq!(events.last(), Some(&ChordAborted), "{case}: {events:?}");
                assert_eq!(release(&mut c, VK_LMENU), Decision::default(), "{case}: no Alt mask");
                release(&mut c, VK_LSHIFT);
                let d = release(&mut c, VK_LWIN);
                assert_eq!(d.event, Some(ChordUp), "{case}");
                assert!(!d.inject_mask_first, "{case}: Windows sees the Office key as is");
            }
        }
    }
}
