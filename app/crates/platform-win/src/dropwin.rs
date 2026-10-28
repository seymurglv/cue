//! The Drop window: always on top, never activated, no button on the taskbar or in Alt+Tab.
//!
//! On every change of its flags (click-through, show, hide) tao rebuilds `GWL_EXSTYLE`
//! from them: it always has `WS_EX_APPWINDOW` and never `WS_EX_TOOLWINDOW`. So setting the style once
//! is not enough — a window subclass fixes every new style before Windows applies it.
//!
//! Always on top is set once, but Windows does not keep it in every case: an ordinary window of another app has been
//! seen above the Drop (and above other always-on-top windows). The z-order guard (`windows_above`,
//! `covering_windows`, `ZGuard`, `raise_topmost`) puts the Drop back on top when that happens, and checks that it
//! helped.
use std::time::{Duration, Instant};

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
        let dpi = GetDpiForWindow(hwnd);
        match window_rect(hwnd) {
            Ok(rect) if dpi != 0 => Seen::Shown { rect, scale: f64::from(dpi) / f64::from(USER_DEFAULT_SCREEN_DPI) },
            _ => Seen::Gone,
        }
    }
}

/// The window's rect in physical px, shown or not. From any thread.
pub fn window_rect(hwnd: HWND) -> windows::core::Result<PxRect> {
    let mut r = RECT::default();
    // SAFETY: only reads the window's rect into our variable; a destroyed HWND gives an error.
    unsafe { GetWindowRect(hwnd, &mut r) }?;
    Ok(PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
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
    pub hwnd: HWND,
    pub rect: PxRect,
    /// Always on top (`WS_EX_TOPMOST`). Such a window is allowed above the Drop: the guard never competes with it.
    pub topmost: bool,
    /// Visible to the window manager but not drawn (DWM cloaking): closed shell panels such as Start and Search,
    /// windows on another virtual desktop. It covers nothing.
    pub cloaked: bool,
}

/// The windows that cover the Drop, as ids (their handle values). Every ordinary (not always-on-top) window belongs
/// below the Drop, so one that is above it, drawn, and overlapping the Drop window means the Drop has lost its place on
/// top. Always-on-top windows above the Drop are left alone, and so are windows that do not overlap it.
pub fn covering_windows(drop: PxRect, above: &[Above]) -> Vec<usize> {
    above.iter().filter(|w| !w.topmost && !w.cloaked && w.rect.intersects(&drop)).map(|w| w.hwnd.0 as usize).collect()
}

/// What the z-order guard does after one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardStep {
    /// Nothing to do.
    Idle,
    /// The Drop is covered: put it back on top. The next check tells whether that helped.
    Raise,
    /// The Drop was raised and is no longer covered. Reported once per occurrence.
    Fixed,
    /// The Drop was raised and the same windows still cover it: most likely it cannot be raised above them (a window
    /// in a band above always-on-top windows), or one of them came back on top within the check. It is raised again
    /// when the covering windows change, and otherwise every `GUARD_RETRY`. Reported once per occurrence.
    NotFixed,
    /// The Drop no longer covered after the guard held off or a raise failed.
    Uncovered,
}

/// How long the guard waits before raising the Drop again over the same windows, once raising them did not help or
/// two raises in a row failed. A raise that worked can be undone within one check by the same window coming back on
/// top, so the guard keeps trying, but slowly: never twice a second against a window that stays above it.
pub const GUARD_RETRY: Duration = Duration::from_secs(30);

/// The z-order guard over time: raises the Drop when it is covered, checks on the next check that the raise helped,
/// and when it did not, holds off while the same windows cover the Drop, trying again every `GUARD_RETRY` (at once
/// when the covering windows change). The same windows back on top within `GUARD_RETRY` of a raise that helped are the
/// same occurrence going on (one of them keeps coming back on top): held off the same way, not raised again at once.
/// So it never raises twice a second for nothing, and does not keep putting the Drop above other always-on-top windows
/// when that cannot help. A raise that failed outright (`raise_failed`) says nothing about the windows: it is not taken
/// as "cannot be raised above". An occurrence ends when the Drop is seen uncovered after the guard held off, or stays
/// uncovered for `GUARD_RETRY` after a raise that helped, or other windows cover it.
#[derive(Debug, Default)]
pub struct ZGuard {
    state: Cover,
    /// The windows of the last raise that helped (sorted ids), and when the guard saw that it did.
    fixed: Option<(Vec<usize>, Instant)>,
    /// `NotFixed` was already reported in this occurrence.
    reported: bool,
    /// `Fixed` was already reported in this occurrence.
    fixed_reported: bool,
    /// Raises that failed in a row: after the first the next check tries again, after two the guard holds off.
    failures: u32,
    /// A failed raise was already reported in this occurrence.
    failure_reported: bool,
}

#[derive(Debug, Default, PartialEq)]
enum Cover {
    #[default]
    Clear,
    /// Raised because of these windows (sorted ids); the next check tells whether it helped.
    Raised(Vec<usize>),
    /// Raising did not help against these windows (sorted ids), as of `since`.
    Held { by: Vec<usize>, since: Instant },
    /// The raise against these windows (sorted ids) failed at `at`: the Drop was not raised.
    Unraised { by: Vec<usize>, at: Instant },
}

impl ZGuard {
    /// One check at `now` with the windows that cover the Drop (`covering_windows`).
    pub fn check(&mut self, mut covering: Vec<usize>, now: Instant) -> GuardStep {
        covering.sort_unstable();
        covering.dedup();
        if matches!(self.state, Cover::Raised(_)) {
            // The last raise was not reported as failed: it went through.
            self.failures = 0;
        }
        let fresh = |at: Instant| now.saturating_duration_since(at) < GUARD_RETRY;
        if covering.is_empty() {
            return match std::mem::take(&mut self.state) {
                Cover::Raised(by) => {
                    // It helped. The same windows back on top soon would be this occurrence going on.
                    self.fixed = Some((by, now));
                    if std::mem::replace(&mut self.fixed_reported, true) { GuardStep::Idle } else { GuardStep::Fixed }
                }
                Cover::Held { .. } | Cover::Unraised { .. } => {
                    self.end_occurrence();
                    GuardStep::Uncovered
                }
                Cover::Clear => {
                    if self.fixed.as_ref().is_some_and(|(_, at)| !fresh(*at)) {
                        self.end_occurrence();
                    }
                    GuardStep::Idle
                }
            };
        }
        if matches!(self.state, Cover::Clear)
            && let Some((by, at)) = self.fixed.take()
        {
            if by == covering && fresh(at) {
                // One of them keeps coming back on top: hold off, and raise again `GUARD_RETRY` after the raise that
                // helped.
                self.state = Cover::Held { by: covering, since: at };
                return if std::mem::replace(&mut self.reported, true) { GuardStep::Idle } else { GuardStep::NotFixed };
            }
            // Other windows, or long after: a new occurrence.
            self.end_occurrence();
        }
        let raise = match &self.state {
            Cover::Raised(by) if *by == covering => {
                self.state = Cover::Held { by: covering, since: now };
                return if std::mem::replace(&mut self.reported, true) { GuardStep::Idle } else { GuardStep::NotFixed };
            }
            Cover::Held { by, since } if *by == covering => now.saturating_duration_since(*since) >= GUARD_RETRY,
            Cover::Unraised { by, at } if *by == covering => {
                self.failures < 2 || now.saturating_duration_since(*at) >= GUARD_RETRY
            }
            _ => true,
        };
        if !raise {
            return GuardStep::Idle;
        }
        self.state = Cover::Raised(covering);
        GuardStep::Raise
    }

    /// The raise asked for by the last `Raise` failed (the request to put the Drop on top was refused): the Drop was
    /// not raised, so the next check does not take the same windows over it as "cannot be raised above"; it raises
    /// again, and after two failures in a row only every `GUARD_RETRY`. true: the first failure in this occurrence,
    /// worth a log line.
    pub fn raise_failed(&mut self, now: Instant) -> bool {
        let Cover::Raised(by) = &self.state else { return false };
        self.state = Cover::Unraised { by: by.clone(), at: now };
        self.failures = self.failures.saturating_add(1);
        !std::mem::replace(&mut self.failure_reported, true)
    }

    fn end_occurrence(&mut self) {
        self.fixed = None;
        self.reported = false;
        self.fixed_reported = false;
        self.failures = 0;
        self.failure_reported = false;
    }
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
            hwnd: cur,
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
        win_id(1, rect, topmost, cloaked)
    }

    fn win_id(id: usize, rect: PxRect, topmost: bool, cloaked: bool) -> Above {
        Above { hwnd: HWND(id as *mut core::ffi::c_void), rect, topmost, cloaked }
    }

    fn covered_by_ordinary_window(drop: PxRect, above: &[Above]) -> bool {
        !covering_windows(drop, above).is_empty()
    }

    #[test]
    fn nothing_above_is_fine() {
        assert!(!covered_by_ordinary_window(DROP, &[]));
    }

    #[test]
    fn covering_windows_are_the_ordinary_drawn_overlapping_ones() {
        let above = [
            win_id(10, MAXIMIZED, false, false),
            win_id(11, MAXIMIZED, true, false),
            win_id(12, MAXIMIZED, false, true),
            win_id(13, PxRect { left: 0, top: 0, right: 100, bottom: 100 }, false, false),
            win_id(14, DROP, false, false),
        ];
        assert_eq!(covering_windows(DROP, &above), [10, 14]);
    }

    use GuardStep::{Fixed, Idle, NotFixed, Raise, Uncovered};

    /// A guard driven by checks every 0.5 s, the way the placement thread runs it.
    struct Clocked {
        g: ZGuard,
        t0: Instant,
        ticks: u32,
    }

    impl Clocked {
        fn new() -> Self {
            Self { g: ZGuard::default(), t0: Instant::now(), ticks: 0 }
        }

        fn now(&self) -> Instant {
            self.t0 + Duration::from_millis(500) * self.ticks
        }

        /// The next check.
        fn check(&mut self, covering: &[usize]) -> GuardStep {
            self.ticks += 1;
            self.g.check(covering.to_vec(), self.now())
        }

        /// Checks while the same windows cover the Drop for `secs`; the steps other than Idle, with their time.
        fn hold(&mut self, covering: &[usize], secs: u32) -> Vec<(u32, GuardStep)> {
            (0..secs * 2).map(|_| (self.ticks + 1, self.check(covering))).filter(|&(_, s)| s != Idle).collect()
        }
    }

    #[test]
    fn guard_stays_idle_while_nothing_covers_the_drop() {
        let mut g = Clocked::new();
        for _ in 0..5 {
            assert_eq!(g.check(&[]), Idle);
        }
    }

    #[test]
    fn guard_raises_then_checks_that_it_helped() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[]), Fixed);
        assert_eq!(g.check(&[]), Idle);
        // A new occurrence is answered the same way.
        assert_eq!(g.check(&[9]), Raise);
        assert_eq!(g.check(&[]), Fixed);
    }

    #[test]
    fn guard_holds_off_when_raising_does_not_help_and_tries_again_every_30_s() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        // Tick 2: still covered by the same window. Reported once.
        assert_eq!(g.check(&[7]), NotFixed);
        // Nothing for 30 s (60 checks) from that check, then one raise; still covered on the next check: held again
        // from there, without a second report. And so on.
        let steps = g.hold(&[7], 95);
        assert_eq!(steps, [(62, Raise), (123, Raise), (184, Raise)]);
        assert_eq!(g.check(&[]), Uncovered);
        assert_eq!(g.check(&[]), Idle);
    }

    #[test]
    fn a_retry_that_works_is_reported_as_on_top_again() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[7]), NotFixed);
        assert_eq!(g.hold(&[7], 30), [(62, Raise)]);
        assert_eq!(g.check(&[]), Fixed);
        // Uncovered for 30 s: that occurrence is over, and a new one is answered and reported again.
        assert_eq!(g.hold(&[], 30), []);
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[7]), NotFixed);
    }

    /// The case seen live: a maximized ordinary window that comes back above the Drop a check after each raise.
    #[test]
    fn a_window_that_keeps_coming_back_on_top_is_held_off_too() {
        let mut g = Clocked::new();
        let mut steps = vec![];
        // Five minutes: each raise helps, and on the check after it the window is back on top.
        let mut uncovered_next = false;
        for _ in 0..600 {
            let covering: &[usize] = if uncovered_next { &[] } else { &[7] };
            let step = g.check(covering);
            uncovered_next = step == Raise;
            if step != Idle {
                steps.push((g.ticks, step));
            }
        }
        // One raise, "on top again" and "not fixed" once each, then a raise every 30 s from the one that helped.
        assert_eq!(steps[..3], [(1, Raise), (2, Fixed), (3, NotFixed)]);
        let raises: Vec<u32> = steps[3..].iter().map(|&(t, s)| if s == Raise { t } else { 0 }).collect();
        assert_eq!(raises, [62, 123, 184, 245, 306, 367, 428, 489, 550]);
    }

    #[test]
    fn other_windows_after_a_fix_are_a_new_occurrence() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[]), Fixed);
        assert_eq!(g.check(&[9]), Raise);
        assert_eq!(g.check(&[]), Fixed);
        // The same window back after more than 30 s: answered at once, a new occurrence.
        assert_eq!(g.hold(&[], 31), []);
        assert_eq!(g.check(&[9]), Raise);
        assert_eq!(g.check(&[]), Fixed);
    }

    #[test]
    fn guard_tries_again_at_once_when_the_covering_windows_change() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[7]), NotFixed);
        // Another window joins: one raise for it.
        assert_eq!(g.check(&[7, 8]), Raise);
        // It helped against that one; the first still covers the Drop: one more try, then quiet, and "not fixed" is
        // not reported twice in one occurrence.
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[7]), Idle);
        assert_eq!(g.check(&[7]), Idle);
        assert_eq!(g.check(&[]), Uncovered);
        // After the Drop was uncovered, a new occurrence is reported again.
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[7]), NotFixed);
    }

    #[test]
    fn guard_ignores_the_order_and_repeats_of_the_windows() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[8, 7]), Raise);
        assert_eq!(g.check(&[7, 8, 8]), NotFixed);
        assert_eq!(g.check(&[8, 7]), Idle);
    }

    #[test]
    fn a_failed_raise_is_not_taken_as_cannot_be_raised_above() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        // The request failed: reported (the first failure), and the next check raises again instead of "not fixed".
        assert!(g.g.raise_failed(g.now()));
        assert_eq!(g.check(&[7]), Raise);
        // This one went through but did not help: now that is the verdict.
        assert_eq!(g.check(&[7]), NotFixed);
        // A failure and then the Drop uncovered: no verdict on the windows either.
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert!(g.g.raise_failed(g.now()));
        assert_eq!(g.check(&[]), Uncovered);
        assert_eq!(g.check(&[7]), Raise);
        assert_eq!(g.check(&[]), Fixed);
    }

    #[test]
    fn failing_raises_back_off_too() {
        let mut g = Clocked::new();
        assert_eq!(g.check(&[7]), Raise);
        assert!(g.g.raise_failed(g.now()));
        assert_eq!(g.check(&[7]), Raise);
        // A second failure in a row: reported no more, and the next try comes 30 s later.
        assert!(!g.g.raise_failed(g.now()));
        assert_eq!(g.hold(&[7], 30), [(62, Raise)]);
        // A change of the covering windows is tried at once, as always.
        assert!(!g.g.raise_failed(g.now()));
        assert_eq!(g.check(&[7, 8]), Raise);
        // Only a raise is ever marked failed.
        assert_eq!(g.check(&[7, 8]), NotFixed);
        assert!(!g.g.raise_failed(g.now()));
        assert_eq!(g.check(&[7, 8]), Idle);
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
