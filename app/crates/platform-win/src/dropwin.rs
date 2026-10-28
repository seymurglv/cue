//! The Drop window: always on top, never activated, no button on the taskbar or in Alt+Tab.
//!
//! On every change of its flags (click-through, show, hide) tao rebuilds `GWL_EXSTYLE`
//! from them: it always has `WS_EX_APPWINDOW` and never `WS_EX_TOOLWINDOW`. So setting the style once
//! is not enough — a window subclass fixes every new style before Windows applies it.
//!
//! Always on top is set once, but Windows does not keep it in every case: an ordinary window of another app has been
//! seen above the Drop (and above other always-on-top windows). The z-order guard (`windows_above`,
//! `covered_by_ordinary_window`, `raise_topmost`) puts the Drop back on top when that happens.
use windows::Win32::Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, GetWindow, GetWindowLongPtrW, GetWindowRect, IsWindow, IsWindowVisible, SetWindowLongPtrW,
    SetWindowPos, SystemParametersInfoW, GWL_EXSTYLE, GW_HWNDPREV, HWND_TOPMOST, SM_SWAPBUTTON,
    SPI_GETCLIENTAREAANIMATION, STYLESTRUCT, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, USER_DEFAULT_SCREEN_DPI, WINDOWPOS, WM_NCDESTROY,
    WM_STYLECHANGING, WM_WINDOWPOSCHANGING, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
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

/// Moves the Drop so its top-left corner is at `(x, y)` (physical px): no resize, no z-order change, no activation.
/// From any thread: the request is posted to the window's thread, the caller does not wait for it.
pub fn move_to(hwnd: HWND, x: i32, y: i32) -> windows::core::Result<()> {
    // SAFETY: only the position of our own window; a destroyed HWND gives an error.
    unsafe { SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS) }
}

/// A visible top-level window above the Drop in z-order, as the z-order guard sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Above {
    pub rect: PxRect,
    /// Always on top (`WS_EX_TOPMOST`). Such a window is allowed above the Drop: the guard never competes with it.
    pub topmost: bool,
    /// Visible to the window manager but not drawn (DWM cloaking): closed shell panels such as Start and Search,
    /// windows on another virtual desktop. It covers nothing.
    pub cloaked: bool,
}

/// The z-order guard's decision. Every ordinary (not always-on-top) window belongs below the Drop, so one that is
/// above it, drawn, and overlapping the Drop window means the Drop has lost its place on top. Always-on-top windows
/// above the Drop are left alone, and so are windows that do not overlap it.
pub fn covered_by_ordinary_window(drop: PxRect, above: &[Above]) -> bool {
    above.iter().any(|w| !w.topmost && !w.cloaked && w.rect.intersects(&drop))
}

/// Stops a walk that would never end if the z-order kept changing under it.
const MAX_WALK: usize = 4096;

/// Visible windows above `hwnd` in z-order, the nearest first. Hidden windows are skipped. From any thread.
pub fn windows_above(hwnd: HWND) -> Vec<Above> {
    let mut out = Vec::new();
    let mut cur = hwnd;
    for _ in 0..MAX_WALK {
        // SAFETY: GetWindow only reads the window list; at the top it returns an error (a null window).
        cur = match unsafe { GetWindow(cur, GW_HWNDPREV) } {
            Ok(h) if !h.0.is_null() => h,
            _ => break,
        };
        // SAFETY: the next calls only read another window's state; a window destroyed meanwhile gives
        // "not visible", a zero style or an error, never memory corruption.
        if !unsafe { IsWindowVisible(cur) }.as_bool() {
            continue;
        }
        let mut r = RECT::default();
        if unsafe { GetWindowRect(cur, &mut r) }.is_err() {
            continue;
        }
        let ex = unsafe { GetWindowLongPtrW(cur, GWL_EXSTYLE) } as u32;
        let mut cloaked = 0u32;
        let cloaked = unsafe {
            DwmGetWindowAttribute(cur, DWMWA_CLOAKED, (&raw mut cloaked).cast(), size_of::<u32>() as u32)
        }
        .is_ok_and(|()| cloaked != 0);
        out.push(Above {
            rect: PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
            topmost: ex & WS_EX_TOPMOST.0 != 0,
            cloaked,
        });
    }
    out
}

/// Puts the Drop back on top of the always-on-top windows, without moving or activating it. From any thread: the request
/// is posted to the window's thread.
pub fn raise_topmost(hwnd: HWND) -> windows::core::Result<()> {
    // SAFETY: only the z-order of our own window; the subclass adds SWP_NOACTIVATE anyway.
    unsafe {
        SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)
    }
}

/// The primary mouse button (the left one, or the right one when the buttons are swapped) is physically down.
/// On the secure desktop (lock screen, UAC) this reads as "up".
pub fn primary_button_down() -> bool {
    // SAFETY: both calls only read input state.
    unsafe {
        let vk = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        GetAsyncKeyState(i32::from(vk.0)) < 0
    }
}

/// Windows animation effects are on (Settings → Accessibility → Visual effects). WebView2 reports the same setting
/// to the page as `prefers-reduced-motion`. If it cannot be read: on.
pub fn animations_enabled() -> bool {
    let mut on = windows::core::BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL to the pointer, which is our own variable.
    let read = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    read.is_err() || on.as_bool()
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

    /// The Drop window at the bottom of a 1920×1080 screen.
    const DROP: PxRect = PxRect { left: 640, top: 802, right: 1280, bottom: 1202 };
    const MAXIMIZED: PxRect = PxRect { left: -8, top: -8, right: 1928, bottom: 1040 };

    fn win(rect: PxRect, topmost: bool, cloaked: bool) -> Above {
        Above { rect, topmost, cloaked }
    }

    #[test]
    fn nothing_above_is_fine() {
        assert!(!covered_by_ordinary_window(DROP, &[]));
    }

    /// The case seen live: a maximized ordinary window above the Drop and above two always-on-top windows.
    #[test]
    fn an_ordinary_window_over_the_drop_is_caught() {
        let explorer = PxRect { left: 300, top: 200, right: 1500, bottom: 900 };
        let above = [win(MAXIMIZED, false, false), win(explorer, true, false), win(explorer, true, false)];
        assert!(covered_by_ordinary_window(DROP, &above));
        // Nearest or farthest, it does not matter where in the list it is.
        assert!(covered_by_ordinary_window(DROP, &[win(explorer, true, false), win(MAXIMIZED, false, false)]));
    }

    #[test]
    fn always_on_top_windows_above_are_left_alone() {
        let above = [win(MAXIMIZED, true, false), win(DROP, true, false)];
        assert!(!covered_by_ordinary_window(DROP, &above));
    }

    #[test]
    fn an_ordinary_window_elsewhere_does_not_count() {
        let top_left = PxRect { left: 0, top: 0, right: 600, bottom: 400 };
        let touching = PxRect { left: 0, top: 402, right: 640, bottom: 802 };
        let minimized = PxRect { left: -32000, top: -32000, right: -31840, bottom: -31972 };
        let above = [win(top_left, false, false), win(touching, false, false), win(minimized, false, false)];
        assert!(!covered_by_ordinary_window(DROP, &above));
    }

    /// Closed Start and Search panels stay "visible" but cloaked; so do windows on another virtual desktop.
    #[test]
    fn cloaked_windows_do_not_count() {
        assert!(!covered_by_ordinary_window(DROP, &[win(MAXIMIZED, false, true)]));
        assert!(covered_by_ordinary_window(DROP, &[win(MAXIMIZED, false, true), win(DROP, false, false)]));
    }

    /// A one-pixel overlap with the Drop window (not only with the capsule) is enough.
    #[test]
    fn any_overlap_with_the_drop_window_counts() {
        let corner = PxRect { left: 1279, top: 1201, right: 1500, bottom: 1300 };
        assert!(covered_by_ordinary_window(DROP, &[win(corner, false, false)]));
    }
}
