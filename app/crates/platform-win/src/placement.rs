//! Where the Drop sits: the work area of the monitor with the active window, the cursor position.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Top-left corner of the Drop window: centered on the bottom edge of the work area, `gap` above it.
pub fn drop_origin(work: PxRect, w: i32, h: i32, gap: i32) -> (i32, i32) {
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.bottom - gap - h;
    (x, y)
}

/// The same in physical px of a monitor with scale `scale` (1.25 = 125 %): the size `w`×`h` and the gap `gap` are logical.
pub fn drop_origin_scaled(work: PxRect, scale: f64, w: f64, h: f64, gap: f64) -> (i32, i32) {
    let px = |v: f64| (v * scale).round() as i32;
    drop_origin(work, px(w), px(h), px(gap))
}

#[cfg(windows)]
pub fn foreground_work_area() -> Option<PxRect> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe {
        let fg = GetForegroundWindow();
        let mon = if fg.0.is_null() {
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
        } else {
            MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
        };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            let r = mi.rcWork;
            Some(PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
        } else {
            None
        }
    }
}

#[cfg(windows)]
pub fn cursor_pos() -> (i32, i32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Full-screen video, game or presentation — the idle strip is hidden.
/// The shell did not answer — "not busy": the strip is visible.
#[cfg(windows)]
pub fn busy_fullscreen() -> bool {
    use windows::Win32::UI::Shell::SHQueryUserNotificationState;
    // SAFETY: no arguments, only reads the shell state.
    unsafe { SHQueryUserNotificationState() }.is_ok_and(is_busy)
}

/// Busy full screen: a full-screen app, a Direct3D game, presentation mode.
#[cfg(windows)]
fn is_busy(state: windows::Win32::UI::Shell::QUERY_USER_NOTIFICATION_STATE) -> bool {
    use windows::Win32::UI::Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
    state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centered_above_taskbar() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        assert_eq!(drop_origin(work, 360, 240, 12), (780, 780));
    }

    #[test]
    fn second_monitor_with_offset() {
        let work = PxRect { left: 1920, top: 0, right: 4480, bottom: 1400 };
        assert_eq!(drop_origin(work, 540, 360, 18), (2930, 1022));
    }

    #[test]
    fn scaled_100_percent() {
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        assert_eq!(drop_origin_scaled(work, 1.0, 360.0, 240.0, 12.0), (780, 780));
    }

    #[test]
    fn scaled_125_percent() {
        // 2560×1440, taskbar 48 logical = 60 physical px.
        let work = PxRect { left: 0, top: 0, right: 2560, bottom: 1380 };
        // 450×300, gap 15.
        assert_eq!(drop_origin_scaled(work, 1.25, 360.0, 240.0, 12.0), (1055, 1065));
    }

    #[test]
    fn scaled_150_percent_second_monitor() {
        // To the right of a 1920 px monitor: 3840×2160, taskbar 72 physical px.
        let work = PxRect { left: 1920, top: 0, right: 5760, bottom: 2088 };
        // 540×360, gap 18.
        assert_eq!(drop_origin_scaled(work, 1.5, 360.0, 240.0, 12.0), (3570, 1710));
    }

    #[test]
    fn scaled_rounds_to_nearest_pixel() {
        // 110 %: 396×264, gap 13.2 → 13.
        let work = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };
        assert_eq!(drop_origin_scaled(work, 1.1, 360.0, 240.0, 12.0), (762, 755));
    }

    #[cfg(windows)]
    #[test]
    fn full_screen_states_are_busy() {
        use windows::Win32::UI::Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN};
        for state in [QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN, QUNS_PRESENTATION_MODE] {
            assert!(is_busy(state), "{state:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn other_states_are_not_busy() {
        use windows::Win32::UI::Shell::{QUNS_ACCEPTS_NOTIFICATIONS, QUNS_APP, QUNS_NOT_PRESENT, QUNS_QUIET_TIME};
        for state in [QUNS_ACCEPTS_NOTIFICATIONS, QUNS_QUIET_TIME, QUNS_NOT_PRESENT, QUNS_APP] {
            assert!(!is_busy(state), "{state:?}");
        }
    }
}
