//! CUE chords from keyboard events. Windows key codes (VK), but no Windows API calls.

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
    /// Exactly the Ctrl + Win pair is held (no Shift or Alt).
    ChordDown,
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
    /// The chord is cancelled (`ChordAborted` sent, or the pair was formed together with Shift/Alt).
    aborted: bool,
    /// This Win press had a `ChordDown`: the last Win release is masked.
    win_needs_mask: bool,
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
            win_needs_mask: false,
            swallowed: [false; 256],
            release_only: [false; 256],
        }
    }
}

impl Chord {
    /// Checks the remembered Ctrl, Win, Shift and Alt against the physical keys: `held(vk)` — is the key held now.
    /// Drops keys whose release never reached the hook (secure desktop after
    /// Ctrl+Alt+Del, UAC prompt, lock screen, elevated window); adds no new presses.
    /// If the Ctrl + Win pair is gone after `ChordDown`, returns `ChordUp` (once).
    /// If no Win is held, the Start menu mask is no longer needed.
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
        for vk in [VK_V, VK_ESCAPE] {
            let idx = usize::from(vk);
            if self.swallowed[idx] && !held(vk) {
                self.release_only[idx] = true;
            }
        }
        if self.win == 0 {
            self.win_needs_mask = false;
        }
        if self.active && !self.paired() {
            self.active = false;
            return Some(HotkeyEvent::ChordUp);
        }
        None
    }

    /// Records a modifier press or release; leaves other keys alone.
    fn set(&mut self, vk: u16, down: bool) {
        let (slot, bit) = match vk {
            VK_LCONTROL | VK_CONTROL => (&mut self.ctrl, LEFT),
            VK_RCONTROL => (&mut self.ctrl, RIGHT),
            VK_LWIN => (&mut self.win, LEFT),
            VK_RWIN => (&mut self.win, RIGHT),
            VK_LSHIFT | VK_SHIFT => (&mut self.shift, LEFT),
            VK_RSHIFT => (&mut self.shift, RIGHT),
            VK_LMENU | VK_MENU => (&mut self.alt, LEFT),
            VK_RMENU => (&mut self.alt, RIGHT),
            _ => return,
        };
        if down { *slot |= bit } else { *slot &= !bit }
    }

    fn paired(&self) -> bool {
        self.ctrl != 0 && self.win != 0
    }

    /// Whether the chord is held: between `ChordDown` and `ChordUp`.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Decision for one keyboard hook event.
    ///
    /// - `vk` — virtual key code; `down` — press (including auto-repeat), otherwise release.
    /// - `injected` — the event was sent by a program (injected flag): state does not change, the event passes.
    /// - `esc_active` — CUE is in "Recording" or "Thinking": an Esc press is swallowed and gives `HotkeyEvent::Esc`.
    ///
    /// A swallowed key (V, Esc) stays swallowed — auto-repeat and release — until it is released,
    /// regardless of `esc_active` and of whether the chord is still held.
    /// The chord is exactly Ctrl + Win: if Shift or Alt is held when the pair forms, this is a system shortcut
    /// (Office key, Ctrl+Shift+Win+B) — no events, no swallowed V, no mask: Windows sees everything as is.
    /// The mask stays if this Win press already had a `ChordDown` (including one cancelled by a third key).
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
        self.set(vk, down);
        match vk {
            VK_LCONTROL | VK_RCONTROL | VK_CONTROL | VK_LWIN | VK_RWIN => {
                let paired = self.paired();
                if paired && !was_paired {
                    // Exactly Ctrl + Win: with Shift or Alt the chord is cancelled at once and transparent (no mask).
                    self.active = self.shift == 0 && self.alt == 0;
                    self.aborted = !self.active;
                    if self.active {
                        self.win_needs_mask = true;
                        d.event = Some(HotkeyEvent::ChordDown);
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
        assert_eq!(d.event, Some(ChordDown));
        assert!(!d.swallow, "Ctrl and Win are not swallowed");
        assert_eq!(press(&mut c, VK_LWIN).event, None, "auto-repeat does not repeat the event");
    }

    #[test]
    fn win_then_ctrl_and_right_keys() {
        let mut c = Chord::default();
        press(&mut c, VK_RWIN);
        assert_eq!(press(&mut c, VK_RCONTROL).event, Some(ChordDown));
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
        assert_eq!(press(&mut c, VK_LSHIFT).event, None, "the abort is reported once");
    }

    #[test]
    fn v_inside_chord_is_paste_last_and_swallowed() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        let d = press(&mut c, VK_V);
        assert_eq!(d, Decision { swallow: true, event: Some(PasteLast), inject_mask_first: false });
        assert_eq!(press(&mut c, VK_V), Decision { swallow: true, event: None, inject_mask_first: false });
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
        assert_eq!(d, Decision { swallow: true, event: Some(Esc), inject_mask_first: false });
        assert!(c.on_key(VK_ESCAPE, false, false, true).swallow);
    }

    #[test]
    fn injected_events_are_ignored() {
        let mut c = Chord::default();
        c.on_key(VK_LCONTROL, true, true, false);
        assert_eq!(c.on_key(VK_LWIN, true, true, false), Decision::default());
    }

    const SWALLOWED: Decision = Decision { swallow: true, event: None, inject_mask_first: false };
    const MASK_ONLY: Decision = Decision { swallow: false, event: None, inject_mask_first: true };

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
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown));
    }

    #[test]
    fn resync_of_passed_system_chord_is_silent() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LSHIFT);
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
    fn shift_or_alt_inside_chord_aborts() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_LWIN);
        assert_eq!(press(&mut c, VK_RSHIFT).event, Some(ChordAborted));
        let mut c = Chord::default();
        press(&mut c, VK_LWIN);
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LMENU).event, Some(ChordAborted));
    }

    #[test]
    fn shift_held_lets_system_chord_pass() {
        // Ctrl+Shift+Win+B — restarts the graphics driver.
        let mut c = Chord::default();
        for vk in [VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_B] {
            assert_eq!(press(&mut c, vk), Decision::default(), "press {vk:#04x}");
        }
        for vk in [VK_B, VK_LSHIFT, VK_LCONTROL] {
            assert_eq!(release(&mut c, vk), Decision::default(), "release {vk:#04x}");
        }
        assert_eq!(release(&mut c, VK_LWIN), Decision::default(), "no ChordUp, no mask — Windows sees everything as is");
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
        assert_eq!(press(&mut c, VK_RWIN).event, Some(ChordDown), "without Alt — a normal chord again");
    }

    #[test]
    fn shift_after_fired_chord_keeps_mask() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown));
        assert_eq!(release(&mut c, VK_LCONTROL).event, Some(ChordUp));
        press(&mut c, VK_LSHIFT);
        assert_eq!(press(&mut c, VK_LCONTROL), Decision::default(), "with Shift the chord does not start");
        release(&mut c, VK_LCONTROL);
        release(&mut c, VK_LSHIFT);
        assert_eq!(release(&mut c, VK_LWIN), MASK_ONLY, "this Win press already had a ChordDown — the mask is needed");
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
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown));
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
        let fresh = Decision { swallow: true, event: Some(PasteLast), inject_mask_first: false };
        assert_eq!(press(&mut c, VK_V), fresh, "a new V press is \"paste last\" again");
        assert_eq!(release(&mut c, VK_V), SWALLOWED);
    }

    #[test]
    fn both_keys_of_a_pair() {
        let mut c = Chord::default();
        press(&mut c, VK_LCONTROL);
        press(&mut c, VK_RCONTROL);
        assert_eq!(press(&mut c, VK_LWIN).event, Some(ChordDown));
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
        assert_eq!(d, Decision { swallow: true, event: Some(Esc), inject_mask_first: false }, "CUE is recording — Esc cancels");
        c.on_key(VK_ESCAPE, false, false, true);
        let d = press(&mut c, VK_ESCAPE);
        assert_eq!(d, Decision { swallow: false, event: Some(ChordAborted), inject_mask_first: false }, "otherwise Esc is a third key");
    }
}
