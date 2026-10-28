//! Where the Drop sits: the four places on the work area of the monitor with the active window, the nearest place to
//! a point, monitors and their scale, the cursor position.
use cue_core::settings::DropPosition;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Rounded down, like every other center here.
    pub fn center(&self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }

    /// The two rects share at least one pixel (touching edges do not count).
    pub fn intersects(&self, other: &PxRect) -> bool {
        self.left < other.right && other.left < self.right && self.top < other.bottom && other.top < self.bottom
    }
}

/// A monitor: its work area (without the taskbar) in physical px and its scale (1.25 = 125 %).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub work: PxRect,
    pub scale: f64,
}

/// The Drop's size and distance from the edge, in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropMetrics {
    /// The Drop's height in every state, which is also the diameter of its circle.
    pub size: f64,
    /// From the work area's edge to the Drop's outer edge.
    pub gap: f64,
}

/// The center of the Drop's circle at `place`, in physical px: `gap` from the edge of the work area to the Drop's outer
/// edge, then half the Drop. Bottom and top are centered horizontally, left and right vertically.
pub fn anchor(work: PxRect, scale: f64, place: DropPosition, m: DropMetrics) -> (i32, i32) {
    let inset = ((m.gap + m.size / 2.0) * scale).round() as i32;
    let (cx, cy) = work.center();
    match place {
        DropPosition::Bottom => (cx, work.bottom - inset),
        DropPosition::Top => (cx, work.top + inset),
        DropPosition::Left => (work.left + inset, cy),
        DropPosition::Right => (work.right - inset, cy),
    }
}

/// Top-left corner of a `w`×`h` window (physical px) whose center is `center`.
pub fn origin_for_center(center: (i32, i32), w: i32, h: i32) -> (i32, i32) {
    (center.0 - w / 2, center.1 - h / 2)
}

/// Top-left corner of the Drop window (logical size `w`×`h`) at `place` on a monitor: the window is centered on the
/// Drop's center, so the Drop never moves inside the window when the place changes.
pub fn drop_origin(mon: Monitor, place: DropPosition, m: DropMetrics, w: f64, h: f64) -> (i32, i32) {
    let px = |v: f64| (v * mon.scale).round() as i32;
    origin_for_center(anchor(mon.work, mon.scale, place, m), px(w), px(h))
}

/// The place whose center is nearest to `point` (physical px) on this monitor. A tie goes to the earlier place in
/// `DropPosition::ALL` (bottom, top, left, right).
pub fn nearest_place(mon: Monitor, m: DropMetrics, point: (i32, i32)) -> DropPosition {
    let dist = |place| {
        let (x, y) = anchor(mon.work, mon.scale, place, m);
        let (dx, dy) = (i64::from(x - point.0), i64::from(y - point.1));
        dx * dx + dy * dy
    };
    // min_by_key keeps the first of equal keys.
    DropPosition::ALL.into_iter().min_by_key(|&p| dist(p)).unwrap_or_default()
}

/// Smooth start and a soft landing: `t` from 0 to 1 (clamped) gives 0 to 1.
pub fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// The point `t` (0…1, eased) of the way from `from` to `to`, rounded to whole pixels.
pub fn lerp_point(from: (i32, i32), to: (i32, i32), t: f64) -> (i32, i32) {
    let e = ease_out(t);
    let mix = |a: i32, b: i32| (f64::from(a) + (f64::from(b) - f64::from(a)) * e).round() as i32;
    (mix(from.0, to.0), mix(from.1, to.1))
}

/// The monitor of the foreground window (the primary one when there is none).
#[cfg(windows)]
pub fn foreground_monitor() -> Option<Monitor> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint, MonitorFromWindow};
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: GetForegroundWindow has no arguments; MonitorFromWindow/MonitorFromPoint take a window (or none) and
    // flags, and with DEFAULTTONEAREST/DEFAULTTOPRIMARY always return a monitor.
    let mon = unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)
        } else {
            MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)
        }
    };
    monitor_info(mon)
}

/// The monitor that contains `point` (physical px), or the nearest one.
#[cfg(windows)]
pub fn monitor_at(point: (i32, i32)) -> Option<Monitor> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    // SAFETY: a point and a flag; with DEFAULTTONEAREST there always is a monitor.
    monitor_info(unsafe { MonitorFromPoint(POINT { x: point.0, y: point.1 }, MONITOR_DEFAULTTONEAREST) })
}

#[cfg(windows)]
fn monitor_info(mon: windows::Win32::Graphics::Gdi::HMONITOR) -> Option<Monitor> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::USER_DEFAULT_SCREEN_DPI;
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: the monitor handle comes from MonitorFrom*; the pointers are to our own variables.
    if !unsafe { GetMonitorInfoW(mon, &mut mi) }.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
    // SAFETY: as above.
    let dpi = match unsafe { GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
        Ok(()) if dpi_x > 0 => dpi_x,
        _ => USER_DEFAULT_SCREEN_DPI,
    };
    let r = mi.rcWork;
    Some(Monitor {
        work: PxRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
        scale: f64::from(dpi) / f64::from(USER_DEFAULT_SCREEN_DPI),
    })
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

    const M: DropMetrics = DropMetrics { size: 36.0, gap: 12.0 };
    const FHD: PxRect = PxRect { left: 0, top: 0, right: 1920, bottom: 1032 };

    fn mon(work: PxRect, scale: f64) -> Monitor {
        Monitor { work, scale }
    }

    #[test]
    fn anchors_at_100_percent() {
        // 12 px gap plus half of the 36 px Drop from each edge; centered along the edge.
        assert_eq!(anchor(FHD, 1.0, DropPosition::Bottom, M), (960, 1002));
        assert_eq!(anchor(FHD, 1.0, DropPosition::Top, M), (960, 30));
        assert_eq!(anchor(FHD, 1.0, DropPosition::Left, M), (30, 516));
        assert_eq!(anchor(FHD, 1.0, DropPosition::Right, M), (1890, 516));
    }

    #[test]
    fn window_is_centered_on_the_drop_in_every_place() {
        let m = mon(FHD, 1.0);
        assert_eq!(drop_origin(m, DropPosition::Bottom, M, 640.0, 400.0), (640, 802));
        assert_eq!(drop_origin(m, DropPosition::Top, M, 640.0, 400.0), (640, -170));
        assert_eq!(drop_origin(m, DropPosition::Left, M, 640.0, 400.0), (-290, 316));
        assert_eq!(drop_origin(m, DropPosition::Right, M, 640.0, 400.0), (1570, 316));
    }

    #[test]
    fn scaled_125_percent() {
        // 2560×1440, taskbar 48 logical = 60 physical px. Inset (12 + 18) × 1.25 = 37.5 → 38; window 800×500.
        let m = mon(PxRect { left: 0, top: 0, right: 2560, bottom: 1380 }, 1.25);
        assert_eq!(drop_origin(m, DropPosition::Bottom, M, 640.0, 400.0), (880, 1092));
        assert_eq!(drop_origin(m, DropPosition::Top, M, 640.0, 400.0), (880, -212));
        assert_eq!(drop_origin(m, DropPosition::Left, M, 640.0, 400.0), (-362, 440));
        assert_eq!(drop_origin(m, DropPosition::Right, M, 640.0, 400.0), (2122, 440));
    }

    #[test]
    fn scaled_150_percent_second_monitor() {
        // To the right of a 1920 px monitor: 3840×2160, taskbar 72 physical px. Inset 45; window 960×600.
        let m = mon(PxRect { left: 1920, top: 0, right: 5760, bottom: 2088 }, 1.5);
        assert_eq!(drop_origin(m, DropPosition::Bottom, M, 640.0, 400.0), (3360, 1743));
        assert_eq!(drop_origin(m, DropPosition::Top, M, 640.0, 400.0), (3360, -255));
        assert_eq!(drop_origin(m, DropPosition::Left, M, 640.0, 400.0), (1485, 744));
        assert_eq!(drop_origin(m, DropPosition::Right, M, 640.0, 400.0), (5235, 744));
    }

    #[test]
    fn scaled_rounds_to_nearest_pixel() {
        // 110 %: inset 33, window 704×440.
        let m = mon(FHD, 1.1);
        assert_eq!(drop_origin(m, DropPosition::Bottom, M, 640.0, 400.0), (608, 779));
        assert_eq!(drop_origin(m, DropPosition::Right, M, 640.0, 400.0), (1535, 296));
    }

    /// Whatever the scale, the outer edge of the Drop is `gap` (scaled, ±1 px of rounding) from the work area edge.
    #[test]
    fn outer_edge_is_one_gap_from_the_work_area_edge() {
        let work = PxRect { left: -1280, top: -200, right: 1280, bottom: 1240 };
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 2.25] {
            let (half, gap) = (M.size / 2.0 * scale, M.gap * scale);
            let edge = |place| {
                let (x, y) = anchor(work, scale, place, M);
                let (x, y) = (f64::from(x), f64::from(y));
                match place {
                    DropPosition::Bottom => f64::from(work.bottom) - (y + half),
                    DropPosition::Top => y - half - f64::from(work.top),
                    DropPosition::Left => x - half - f64::from(work.left),
                    DropPosition::Right => f64::from(work.right) - (x + half),
                }
            };
            for place in DropPosition::ALL {
                assert!((edge(place) - gap).abs() <= 1.0, "{place:?} at {scale}: {}", edge(place));
            }
        }
    }

    #[test]
    fn nearest_place_by_distance_to_each_center() {
        let m = mon(FHD, 1.0);
        assert_eq!(nearest_place(m, M, (960, 1002)), DropPosition::Bottom);
        assert_eq!(nearest_place(m, M, (700, 900)), DropPosition::Bottom);
        assert_eq!(nearest_place(m, M, (1200, 120)), DropPosition::Top);
        assert_eq!(nearest_place(m, M, (150, 700)), DropPosition::Left);
        assert_eq!(nearest_place(m, M, (1700, 300)), DropPosition::Right);
        // A corner goes to the nearer center: the bottom-left corner is nearer the left center (30, 516) than the
        // bottom center (960, 1002).
        assert_eq!(nearest_place(m, M, (40, 1000)), DropPosition::Left);
        // Outside the work area (dragged past the edge) the nearest center still wins.
        assert_eq!(nearest_place(m, M, (2500, 516)), DropPosition::Right);
        assert_eq!(nearest_place(m, M, (960, -300)), DropPosition::Top);
    }

    #[test]
    fn nearest_place_tie_goes_to_bottom_then_top() {
        // The exact center of the work area is as far from the bottom center as from the top center.
        assert_eq!(nearest_place(mon(FHD, 1.0), M, (960, 516)), DropPosition::Bottom);
        // On a square work area the center is as far from all four.
        let square = mon(PxRect { left: 0, top: 0, right: 1000, bottom: 1000 }, 1.0);
        assert_eq!(nearest_place(square, M, (500, 500)), DropPosition::Bottom);
    }

    #[test]
    fn nearest_place_uses_the_monitor_scale() {
        // At 200 % the centers sit 60 px in from the edges: (60, 540) is the left center itself.
        let m = mon(PxRect { left: 0, top: 0, right: 3840, bottom: 1080 }, 2.0);
        assert_eq!(anchor(m.work, m.scale, DropPosition::Left, M), (60, 540));
        assert_eq!(nearest_place(m, M, (60, 540)), DropPosition::Left);
        assert_eq!(nearest_place(m, M, (1920, 1000)), DropPosition::Bottom);
    }

    #[test]
    fn rect_intersection_needs_a_shared_pixel() {
        let a = PxRect { left: 0, top: 0, right: 100, bottom: 100 };
        assert!(a.intersects(&PxRect { left: 99, top: 99, right: 200, bottom: 200 }));
        assert!(a.intersects(&PxRect { left: -50, top: 20, right: 150, bottom: 30 }));
        assert!(!a.intersects(&PxRect { left: 100, top: 0, right: 200, bottom: 100 }));
        assert!(!a.intersects(&PxRect { left: 0, top: 100, right: 100, bottom: 200 }));
        assert!(!a.intersects(&PxRect { left: -32000, top: -32000, right: -31840, bottom: -31972 }));
        assert_eq!(a.center(), (50, 50));
    }

    #[test]
    fn snap_curve_starts_and_ends_exactly_and_never_goes_back() {
        assert_eq!(lerp_point((10, 20), (110, -80), 0.0), (10, 20));
        assert_eq!(lerp_point((10, 20), (110, -80), 1.0), (110, -80));
        assert_eq!(lerp_point((10, 20), (110, -80), 7.0), (110, -80));
        assert_eq!(lerp_point((10, 20), (110, -80), -1.0), (10, 20));
        let mut last = 0.0;
        for i in 1..=20 {
            let e = ease_out(f64::from(i) / 20.0);
            assert!(e >= last && e <= 1.0);
            last = e;
        }
        // Eased out: more than half of the way in the first third of the time.
        assert!(ease_out(1.0 / 3.0) > 0.5);
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
