//! Key presses on behalf of CUE (SendInput) and modifier state.
use std::time::{Duration, Instant};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
};

use crate::chord::{
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MASK, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_V,
};

pub fn key_input(vk: u16, up: bool) -> INPUT {
    // SAFETY: only maps the key code to a scan code of the current layout.
    let scan = unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Sided modifiers: paste waits for them to be released, the hook resyncs them.
pub const MODIFIERS: [u16; 8] = [VK_LCONTROL, VK_RCONTROL, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT, VK_LMENU, VK_RMENU];

/// `true` if `SendInput` accepted all events. Windows does not report a UIPI block (elevated window) this way.
/// No logging: also called from the hook procedure, the caller logs.
fn send(inputs: &[INPUT]) -> bool {
    // SAFETY: a slice of live INPUTs, the struct size is its own.
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

/// An empty press of an unused key: Windows treats Win as part of a chord and does not open the Start menu.
pub fn send_mask_key() -> bool {
    send(&[key_input(VK_MASK, false), key_input(VK_MASK, true)])
}

/// Whether the key is held right now (`GetAsyncKeyState`, high bit).
/// In the hook procedure this state does not include the current event yet, and it does not see swallowed presses.
pub fn key_held(vk: u16) -> bool {
    // SAFETY: only reads keyboard state.
    unsafe { GetAsyncKeyState(i32::from(vk)) < 0 }
}

pub fn modifiers_held() -> bool {
    MODIFIERS.into_iter().any(key_held)
}

/// Wait until the user releases the modifiers, otherwise Ctrl+V would turn into a different chord.
pub fn wait_modifiers_released(timeout: Duration) -> bool {
    let until = Instant::now() + timeout;
    while modifiers_held() {
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    true
}

/// `false` — `SendInput` did not accept all presses: the paste failed, the text stays in the clipboard.
pub fn press_ctrl_v() -> bool {
    send(&[
        key_input(VK_CONTROL, false),
        key_input(VK_V, false),
        key_input(VK_V, true),
        key_input(VK_CONTROL, true),
    ])
}

const VK_C: u16 = 0x43;
const VK_RIGHT: u16 = 0x27;

/// Ctrl+C for the copy fallback of "Ask". `false` — not all presses were accepted.
pub fn press_ctrl_c() -> bool {
    send(&[key_input(VK_CONTROL, false), key_input(VK_C, false), key_input(VK_C, true), key_input(VK_CONTROL, true)])
}

/// Right arrow: collapses a selection to its end in text fields ("Insert below", spec 2 §7). An extended key: without the
/// flag its scan code is the keypad's 6.
pub fn press_right() -> bool {
    send(&[extended(key_input(VK_RIGHT, false)), extended(key_input(VK_RIGHT, true))])
}

fn extended(mut input: INPUT) -> INPUT {
    // SAFETY: key_input fills the ki variant; only its flags change.
    unsafe { input.Anonymous.ki.dwFlags = KEYBD_EVENT_FLAGS(input.Anonymous.ki.dwFlags.0 | KEYEVENTF_EXTENDEDKEY.0) };
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_input_carries_virtual_and_scan_code() {
        let down = key_input(VK_CONTROL, false);
        assert_eq!(down.r#type, INPUT_KEYBOARD);
        // SAFETY: key_input fills the ki variant.
        let ki = unsafe { down.Anonymous.ki };
        assert_eq!(ki.wVk, VIRTUAL_KEY(VK_CONTROL));
        assert_eq!(ki.wScan, 0x1D, "the Ctrl scan code is the same in every layout");
        assert_eq!(ki.dwFlags, KEYBD_EVENT_FLAGS(0));
        assert_eq!((ki.time, ki.dwExtraInfo), (0, 0));

        let up = key_input(VK_CONTROL, true);
        // SAFETY: key_input fills the ki variant.
        let ki = unsafe { up.Anonymous.ki };
        assert_eq!(ki.dwFlags, KEYEVENTF_KEYUP);
        assert_eq!(ki.wScan, 0x1D);
    }

    #[test]
    fn the_right_arrow_is_an_extended_key() {
        let down = extended(key_input(VK_RIGHT, false));
        // SAFETY: key_input fills the ki variant.
        let ki = unsafe { down.Anonymous.ki };
        assert_eq!(ki.wVk, VIRTUAL_KEY(VK_RIGHT));
        assert_ne!(ki.dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
        assert_eq!(ki.dwFlags.0 & KEYEVENTF_KEYUP.0, 0);
        // SAFETY: as above.
        let ki = unsafe { extended(key_input(VK_RIGHT, true)).Anonymous.ki };
        assert_ne!(ki.dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
        assert_ne!(ki.dwFlags.0 & KEYEVENTF_KEYUP.0, 0, "a key-up stays a key-up");
    }
}
