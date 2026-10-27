//! The Drop window: always on top, never activated, no button on the taskbar or in Alt+Tab.
//!
//! On every change of its flags (click-through, show, hide) tao rebuilds `GWL_EXSTYLE`
//! from them: it always has `WS_EX_APPWINDOW` and never `WS_EX_TOOLWINDOW`. So setting the style once
//! is not enough — a window subclass fixes every new style before Windows applies it.
use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    HWND_TOPMOST, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, USER_DEFAULT_SCREEN_DPI,
    WINDOWPOS, WM_NCDESTROY, WM_STYLECHANGING, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use crate::placement::PxRect;

/// Our subclass id. A subclass is identified by the (procedure, id) pair, so it would not collide with wry
/// (`WM_USER + 0x64`, `WM_USER + 0x66`) or tauri (`WM_USER + 1`) anyway; the id is our own all the same.
const SUBCLASS_ID: usize = 0x4355_4531; // "CUE1"

/// The Drop's extended style from the proposed `ex`: `WS_EX_TOOLWINDOW` (not on the taskbar or in Alt+Tab)
/// and `WS_EX_NOACTIVATE` (a click does not take focus) are always set, `WS_EX_APPWINDOW` (taskbar button) never.
/// The other bits, including click-through `WS_EX_LAYERED | WS_EX_TRANSPARENT`, stay as proposed.
pub fn drop_ex_style(ex: u32) -> u32 {
    (ex | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) & !WS_EX_APPWINDOW.0
}

/// Installs the subclass, sets the Drop style right away and raises the window to the top (`WS_EX_TOPMOST` cannot be set
/// through `SetWindowLongPtr`, only with `SetWindowPos(HWND_TOPMOST)`). Call from the window's thread before showing it.
pub fn make_non_activating(hwnd: HWND) -> windows::core::Result<()> {
    // SAFETY: hwnd is a live window of this thread; the `extern "system"` procedure is static, the subclass has no data.
    unsafe { SetWindowSubclass(hwnd, Some(drop_subclass_proc), SUBCLASS_ID, 0) }.ok()?;
    // SAFETY: reading and writing GWL_EXSTYLE of our own window.
    let ex = window_long(|| unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) })? as u32;
    window_long(|| unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, drop_ex_style(ex) as isize) })?;
    // SAFETY: only the z-order and a frame recalculation, without moving or activating.
    unsafe {
        SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED)
    }
}

/// `Get/SetWindowLongPtrW` return 0 both on error and for a legitimate zero; only `GetLastError` tells them apart.
fn window_long(call: impl FnOnce() -> isize) -> windows::core::Result<isize> {
    // SAFETY: SetLastError/GetLastError work with a field of the current thread.
    unsafe { SetLastError(ERROR_SUCCESS) };
    let value = call();
    if value == 0 && unsafe { GetLastError() } != ERROR_SUCCESS {
        return Err(windows::core::Error::from_thread());
    }
    Ok(value)
}

unsafe extern "system" fn drop_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match msg {
        // tao writes to GWL_EXSTYLE a style built from its flags (APPWINDOW set, TOOLWINDOW not) on every
        // set_ignore_cursor_events, show and hide. Windows lets us fix the proposed style before it is applied.
        WM_STYLECHANGING if wparam.0 as i32 == GWL_EXSTYLE.0 => {
            // SAFETY: for WM_STYLECHANGING lParam points to a STYLESTRUCT that is alive for the duration of the call.
            if let Some(style) = unsafe { (lparam.0 as *mut STYLESTRUCT).as_mut() } {
                style.styleNew = drop_ex_style(style.styleNew);
            }
        }
        // tao shows the window with ShowWindow(SW_SHOW), not SW_SHOWNOACTIVATE: it drops the "do not focus" flag
        // right after creation. Any move, show or z-order change of the Drop goes without activation.
        WM_WINDOWPOSCHANGING => {
            // SAFETY: for WM_WINDOWPOSCHANGING lParam points to a WINDOWPOS that is alive for the duration of the call.
            if let Some(pos) = unsafe { (lparam.0 as *mut WINDOWPOS).as_mut() } {
                pos.flags |= SWP_NOACTIVATE;
            }
        }
        // comctl32 requires removing the subclass before the window is destroyed.
        WM_NCDESTROY => {
            // SAFETY: removing our own subclass on the window's thread.
            let _ = unsafe { RemoveWindowSubclass(hwnd, Some(drop_subclass_proc), SUBCLASS_ID) };
        }
        _ => {}
    }
    // On down the chain: other subclasses (wry) and tao's window procedure see the message already fixed.
    // SAFETY: the same arguments that came into the subclass procedure.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// The Drop window as seen by the cursor-check thread. Read straight from the HWND, without going through the window's thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seen {
    /// The window is gone.
    Gone,
    /// The window is hidden.
    Hidden,
    /// The window is visible: its rect in physical px and the monitor scale (1.25 = 125 %).
    Shown { rect: PxRect, scale: f64 },
}

pub fn observe(hwnd: HWND) -> Seen {
    // SAFETY: IsWindow, IsWindowVisible, GetWindowRect and GetDpiForWindow only read and are allowed from any
    // thread; for a foreign or already destroyed HWND they return an error rather than corrupt memory.
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return Seen::Gone;
        }
        if !IsWindowVisible(hwnd).as_bool() {
            return Seen::Hidden;
        }
        let mut r = RECT::default();
        let dpi = GetDpiForWindow(hwnd);
        if GetWindowRect(hwnd, &mut r).is_err() || dpi == 0 {
            return Seen::Gone;
        }
        let rect = PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        Seen::Shown { rect, scale: f64::from(dpi) / f64::from(USER_DEFAULT_SCREEN_DPI) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        WS_EX_ACCEPTFILES, WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_EX_WINDOWEDGE,
    };

    #[test]
    fn fixes_style_tao_rebuilds() {
        // The live style after set_ignore_cursor_events(true): APPWINDOW set, TOOLWINDOW not.
        assert_eq!(drop_ex_style(0x080C_0138), 0x0808_01B8);
    }

    #[test]
    fn adds_toolwindow_and_noactivate_removes_appwindow() {
        let tao = WS_EX_APPWINDOW.0 | WS_EX_WINDOWEDGE.0 | WS_EX_ACCEPTFILES.0;
        let want = WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_WINDOWEDGE.0 | WS_EX_ACCEPTFILES.0;
        assert_eq!(drop_ex_style(tao), want);
        assert_eq!(drop_ex_style(0), WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0);
    }

    #[test]
    fn keeps_click_through_bits_as_proposed() {
        let through = WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0;
        assert_eq!(drop_ex_style(through) & through, through);
        assert_eq!(drop_ex_style(WS_EX_APPWINDOW.0) & through, 0);
    }

    #[test]
    fn idempotent() {
        for ex in [0, 0x080C_0138, 0x0804_0118, u32::MAX] {
            assert_eq!(drop_ex_style(drop_ex_style(ex)), drop_ex_style(ex));
        }
    }
}
