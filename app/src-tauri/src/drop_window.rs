//! The Drop window: creation, placement at the bottom edge, click-through outside the capsule and the tray card.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cue_platform_win::dropwin::{self, Seen};
use cue_platform_win::placement;
use tauri::{AppHandle, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const DROP_LABEL: &str = "drop";
/// Logical px. With room for the core's tray card (300 px) and its height.
pub const DROP_W: f64 = 360.0;
pub const DROP_H: f64 = 240.0;
pub const GAP_PX: f64 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct HitRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Rectangles of the capsule and the tray card in logical px relative to the window. The window writes them,
/// the hit-test thread reads them.
#[derive(Clone, Default)]
pub struct HitState(pub Arc<Mutex<Vec<HitRect>>>);

pub fn hit(rects: &[HitRect], x: f64, y: f64) -> bool {
    rects.iter().any(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h)
}

pub fn create(app: &AppHandle, lang: crate::lang::Lang) -> anyhow::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, DROP_LABEL, WebviewUrl::App("drop.html".into()))
        .initialization_script(lang.init_script())
        .title("CUE")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .inner_size(DROP_W, DROP_H)
        .visible(false)
        .build()?;
    // Before the first show: from then on the subclass keeps the style and the no-activate show through any tao action.
    dropwin::make_non_activating(win.hwnd()?)?;
    place(&win)?;
    win.set_ignore_cursor_events(true)?;
    win.show()?;
    Ok(win)
}

/// Centered at the bottom edge of the work area of the monitor with the foreground window.
pub fn place(win: &WebviewWindow) -> anyhow::Result<()> {
    let Some(work) = placement::foreground_work_area() else { return Ok(()) };
    // Take the scale of the monitor the Drop will move to, not the one the window is on now.
    let (cx, cy) = ((work.left + work.right) / 2, (work.top + work.bottom) / 2);
    let scale = match win.monitor_from_point(f64::from(cx), f64::from(cy)).ok().flatten() {
        Some(monitor) => monitor.scale_factor(),
        None => win.scale_factor()?,
    };
    let (x, y) = placement::drop_origin_scaled(work, scale, DROP_W, DROP_H, GAP_PX);
    win.set_position(PhysicalPosition::new(x, y))?;
    Ok(())
}

/// ~60 times a second while the Drop is visible: the window takes the mouse only over the capsule and the tray card.
/// The window position and scale are read straight from the HWND; the thread goes to the window thread only to switch.
/// Ends when the window is gone.
pub fn spawn_hit_test(win: WebviewWindow, state: HitState) {
    std::thread::Builder::new()
        .name("cue-hit-test".into())
        .spawn(move || {
            // HWND is not Send, so we get it inside the thread: one request to the window thread.
            let hwnd = match win.hwnd() {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    tracing::error!("Drop: no HWND for the cursor hit test: {e}");
                    return;
                }
            };
            let mut ignoring = true;
            loop {
                std::thread::sleep(Duration::from_millis(16));
                let (rect, scale) = match dropwin::observe(hwnd) {
                    Seen::Gone => break,
                    Seen::Hidden => continue,
                    Seen::Shown { rect, scale } => (rect, scale),
                };
                let (cx, cy) = placement::cursor_pos();
                let x = f64::from(cx - rect.left) / scale;
                let y = f64::from(cy - rect.top) / scale;
                let over = hit(&state.0.lock().unwrap(), x, y);
                // If it failed, retry on the next tick; a closed window is caught by observe.
                if over == ignoring && win.set_ignore_cursor_events(!over).is_ok() {
                    ignoring = !over;
                }
            }
        })
        .expect("cursor hit-test thread");
}

/// Every 0.5 s: the Drop moves to the monitor of the foreground window (and is placed again if that monitor's scale
/// changed); over a full-screen app the idle bar hides.
pub fn spawn_placement(win: WebviewWindow, ui: crate::ipc::Ui) {
    let spawned = std::thread::Builder::new().name("cue-placement".into()).spawn(move || {
        let mut placed: Option<(placement::PxRect, u32)> = None;
        let mut failed = false;
        // The window gets `idle-hidden` only on a change; at the start it assumes "don't hide".
        let mut busy = false;
        loop {
            if let Some(work) = placement::foreground_work_area() {
                let target = (work, monitor_dpi(work));
                if placed != Some(target) {
                    match place(&win) {
                        Ok(()) => {
                            placed = Some(target);
                            failed = false;
                        }
                        // Retry on the next tick; log it once, not twice a second.
                        Err(e) if !failed => {
                            failed = true;
                            tracing::warn!("Drop not repositioned: {e}");
                        }
                        Err(_) => {}
                    }
                }
            }
            let now_busy = placement::busy_fullscreen();
            if now_busy != busy {
                busy = now_busy;
                ui.idle_hidden(busy);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
    if let Err(e) = spawned {
        tracing::error!("Drop placement thread not started: {e}");
    }
}

/// DPI of the monitor at the center of the work area (scale × 96); 0 if unknown.
fn monitor_dpi(work: placement::PxRect) -> u32 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    let center = POINT { x: (work.left + work.right) / 2, y: (work.top + work.bottom) / 2 };
    let (mut x, mut y) = (0u32, 0u32);
    // SAFETY: a point and constants; the monitor comes from MonitorFromPoint (with NEAREST there always is one);
    // the pointers are to our own variables.
    match unsafe { GetDpiForMonitor(MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST), MDT_EFFECTIVE_DPI, &mut x, &mut y) } {
        Ok(()) => x,
        Err(_) => 0,
    }
}

#[tauri::command]
pub fn drop_hit_rects(rects: Vec<HitRect>, state: tauri::State<'_, HitState>) {
    *state.0.lock().unwrap() = rects;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_and_outside() {
        let r = [HitRect { x: 100.0, y: 200.0, w: 160.0, h: 40.0 }];
        assert!(hit(&r, 100.0, 200.0));
        assert!(hit(&r, 259.9, 239.9));
        assert!(!hit(&r, 260.0, 220.0));
        assert!(!hit(&r, 150.0, 199.0));
        assert!(!hit(&[], 1.0, 1.0));
    }
}
