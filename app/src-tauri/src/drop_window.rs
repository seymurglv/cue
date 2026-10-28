//! The Drop window: creation, its place on the screen (one of four, chosen by dragging the Drop), click-through outside
//! the capsule and the tray card, and the z-order guard.
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cue_core::settings::DropPosition;
use cue_platform_win::dropwin::{self, Seen};
use cue_platform_win::placement::{self, DropMetrics, Monitor, PxRect};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::ipc::Ui;

pub const DROP_LABEL: &str = "drop";
/// The Drop's height in every state and the diameter of its circle, logical px. The same value as `--h` in
/// `app/ui/src/design/drop.css` (a test keeps them equal).
pub const DROP_SIZE: f64 = 36.0;
/// From the work area's edge to the Drop's outer edge, logical px.
pub const GAP_PX: f64 = 12.0;
pub const METRICS: DropMetrics = DropMetrics { size: DROP_SIZE, gap: GAP_PX };
/// Logical px. The window is centered on the Drop's center in every place (the page puts the Drop at its center), so a
/// new place never moves the Drop inside the window: only the window moves. It reaches far enough every way for the
/// widest Drop and the largest tray card growing toward the screen center, with their shadows. The part beyond the
/// edge of the work area is transparent and lets the mouse through, like the rest of the window.
pub const WINDOW_W: f64 = 640.0;
pub const WINDOW_H: f64 = 400.0;
/// How often the window's place, the z-order and full screen are checked while the Drop is at rest.
const TICK: Duration = Duration::from_millis(500);
/// How often the Drop follows the cursor while dragged, and the step of the snap.
const FRAME: Duration = Duration::from_millis(8);
/// The snap to the chosen place after a drag.
const SNAP: Duration = Duration::from_millis(200);

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

/// The window's drag messages: the pointer went down on the capsule and moved past the threshold, or went up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragPhase {
    Start,
    End,
}

/// Passes drag messages from the window to the placement thread, which alone moves the window.
pub struct Placer(Sender<DragPhase>);

impl Placer {
    pub fn new(tx: Sender<DragPhase>) -> Self {
        Self(tx)
    }
}

#[tauri::command]
pub fn drop_drag(phase: DragPhase, placer: tauri::State<'_, Placer>) {
    let _ = placer.0.send(phase);
}

pub fn create(app: &AppHandle, lang: crate::lang::Lang, place: DropPosition) -> anyhow::Result<WebviewWindow> {
    let win = WebviewWindowBuilder::new(app, DROP_LABEL, WebviewUrl::App("drop.html".into()))
        .initialization_script(lang.init_script())
        .initialization_script(place_init_script(place))
        .title("CUE")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .inner_size(WINDOW_W, WINDOW_H)
        .visible(false)
        .build()?;
    let hwnd = win.hwnd()?;
    // Before the first show: from then on the subclass keeps the style and the no-activate show through any tao action.
    dropwin::make_non_activating(hwnd)?;
    // If the window is created at another scale than the target monitor's, Windows resizes it on the way there and the
    // placement thread corrects the position on its first tick.
    if let Some(mon) = placement::foreground_monitor() {
        let (x, y) = placement::drop_origin(mon, place, METRICS, WINDOW_W, WINDOW_H);
        dropwin::move_to(hwnd, x, y)?;
    }
    win.set_ignore_cursor_events(true)?;
    win.show()?;
    Ok(win)
}

/// Initialization script for the Drop window: the place before the page's own scripts run, so the first frame is
/// already laid out for it. The code goes in through Rust's `{:?}` (a quoted Rust string literal); that is a valid
/// JavaScript string only because the code is always one of four fixed ASCII words.
fn place_init_script(place: DropPosition) -> String {
    format!("window.__CUE_PLACE__ = {:?};", place.code())
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

/// The one thread that moves the Drop window. At rest, every 0.5 s: the Drop goes to its place on the monitor of the
/// foreground window (again after a scale change), the z-order guard runs, and over a full-screen app the idle strip
/// hides. While dragged, the Drop follows the cursor; when let go, it snaps to the nearest place, which is saved
/// through `moved` (to settings.json) and sent to the page.
pub fn spawn_placement(
    win: WebviewWindow,
    ui: Ui,
    place: DropPosition,
    drags: Receiver<DragPhase>,
    moved: impl Fn(DropPosition) + Send + 'static,
) {
    let spawned = std::thread::Builder::new().name("cue-placement".into()).spawn(move || {
        // HWND is not Send, so we get it inside the thread: one request to the window thread.
        let hwnd = match win.hwnd() {
            Ok(hwnd) => hwnd,
            Err(e) => {
                tracing::error!("Drop: no HWND for placement: {e}");
                return;
            }
        };
        let mut mover = Mover::new(hwnd, ui, place, Box::new(moved));
        let mut next_tick = Instant::now();
        loop {
            let wait = match mover.mode {
                Mode::Rest => next_tick.saturating_duration_since(Instant::now()),
                Mode::Drag { .. } | Mode::Snap { .. } => FRAME,
            };
            match drags.recv_timeout(wait) {
                Ok(DragPhase::Start) => mover.start_drag(),
                Ok(DragPhase::End) => mover.end_drag(),
                Err(RecvTimeoutError::Timeout) => {}
                // The sender lives in the app state; without it no drag can come, but the Drop still needs placing.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
            }
            let Seen::Shown { rect, scale } = (match dropwin::observe(hwnd) {
                Seen::Gone => break,
                seen => seen,
            }) else {
                continue;
            };
            match mover.mode {
                Mode::Drag { grab } => mover.follow(grab, rect),
                Mode::Snap { from, to, start } => mover.snap_step(from, to, start, rect),
                Mode::Rest if Instant::now() >= next_tick => {
                    mover.tick(rect, scale);
                    next_tick = Instant::now() + TICK;
                }
                Mode::Rest => {}
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!("Drop placement thread not started: {e}");
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Rest,
    /// Dragged: the Drop's center stays `grab` away from the cursor.
    Drag { grab: (i32, i32) },
    /// Snapping to its place: the Drop's center goes from `from` to `to` (physical px).
    Snap { from: (i32, i32), to: (i32, i32), start: Instant },
}

struct Mover {
    hwnd: windows::Win32::Foundation::HWND,
    ui: Ui,
    place: DropPosition,
    moved: Box<dyn Fn(DropPosition) + Send>,
    mode: Mode,
    /// The last origin requested while dragging or snapping: the same one is not sent again.
    last_origin: Option<(i32, i32)>,
    /// The last move at rest failed: logged once, not twice a second.
    move_failed: bool,
    /// An ordinary window was over the Drop on the last tick: logged once per occurrence.
    covered: bool,
    /// The window gets `idle-hidden` only on a change; at the start it assumes "don't hide".
    busy: bool,
}

impl Mover {
    fn new(
        hwnd: windows::Win32::Foundation::HWND,
        ui: Ui,
        place: DropPosition,
        moved: Box<dyn Fn(DropPosition) + Send>,
    ) -> Self {
        Self { hwnd, ui, place, moved, mode: Mode::Rest, last_origin: None, move_failed: false, covered: false, busy: false }
    }

    /// Moves the window so its center is at `center`, keeping its current size.
    fn center_at(&mut self, center: (i32, i32), rect: PxRect) {
        let origin = placement::origin_for_center(center, rect.width(), rect.height());
        // Already there, or already asked for (the move is posted to the window's thread and may still be on its way).
        if (rect.left, rect.top) == origin || self.last_origin == Some(origin) {
            return;
        }
        if dropwin::move_to(self.hwnd, origin.0, origin.1).is_ok() {
            self.last_origin = Some(origin);
        }
    }

    /// Only while the primary button is down: a page message that arrives late, after the button went up, starts
    /// nothing. A refused start still answers with the place, so the page leaves its dragging state.
    fn start_drag(&mut self) {
        let rect = match dropwin::observe(self.hwnd) {
            Seen::Shown { rect, .. } if dropwin::primary_button_down() => rect,
            _ => {
                self.ui.place(self.place);
                return;
            }
        };
        let (cx, cy) = placement::cursor_pos();
        let (wx, wy) = rect.center();
        self.mode = Mode::Drag { grab: (wx - cx, wy - cy) };
        self.last_origin = None;
        tracing::debug!("Drop: drag started");
    }

    /// The center follows the cursor; it is computed from the current window size, so a scale change on the way to
    /// another monitor keeps the Drop under the cursor. The drag ends when the button is up, even if the page never
    /// said so.
    fn follow(&mut self, grab: (i32, i32), rect: PxRect) {
        if !dropwin::primary_button_down() {
            self.end_drag();
            return;
        }
        let (cx, cy) = placement::cursor_pos();
        self.center_at((cx + grab.0, cy + grab.1), rect);
    }

    /// The nearest place to where the Drop was let go, on the monitor under it; saved if it changed and sent to the
    /// page in any case (that also ends the page's dragging state). Then the snap to that place on the monitor of the
    /// foreground window, as always: at once when Windows animations are off.
    fn end_drag(&mut self) {
        if !matches!(self.mode, Mode::Drag { .. }) {
            return;
        }
        self.mode = Mode::Rest;
        let Seen::Shown { rect, .. } = dropwin::observe(self.hwnd) else {
            self.ui.place(self.place);
            return;
        };
        let center = rect.center();
        if let Some(under) = placement::monitor_at(center) {
            let place = placement::nearest_place(under, METRICS, center);
            if place != self.place {
                tracing::info!(place = place.code(), "Drop moved");
                self.place = place;
                (self.moved)(place);
            }
        }
        self.ui.place(self.place);
        let Some(mon) = placement::foreground_monitor().or_else(|| placement::monitor_at(center)) else { return };
        let to = placement::anchor(mon.work, mon.scale, self.place, METRICS);
        if dropwin::animations_enabled() {
            self.mode = Mode::Snap { from: center, to, start: Instant::now() };
        } else {
            self.center_at(to, rect);
        }
    }

    fn snap_step(&mut self, from: (i32, i32), to: (i32, i32), start: Instant, rect: PxRect) {
        let t = start.elapsed().as_secs_f64() / SNAP.as_secs_f64();
        self.center_at(placement::lerp_point(from, to, t), rect);
        if t >= 1.0 {
            self.mode = Mode::Rest;
        }
    }

    /// At rest: the place on the foreground window's monitor, the z-order guard, full screen.
    fn tick(&mut self, rect: PxRect, scale: f64) {
        if let Some(mon) = placement::foreground_monitor() {
            let origin = rest_origin(mon, self.place, rect, scale);
            if (rect.left, rect.top) != origin {
                match dropwin::move_to(self.hwnd, origin.0, origin.1) {
                    Ok(()) => self.move_failed = false,
                    // Retry on the next tick; log it once, not twice a second.
                    Err(e) if !self.move_failed => {
                        self.move_failed = true;
                        tracing::warn!("Drop not repositioned: {e}");
                    }
                    Err(_) => {}
                }
            }
        }
        self.guard_z_order(rect);
        let busy = placement::busy_fullscreen();
        if busy != self.busy {
            self.busy = busy;
            self.ui.idle_hidden(busy);
        }
    }

    /// An ordinary window above the Drop that overlaps it means the Drop lost its place on top: it is put back,
    /// without activation. Logged once per occurrence; window titles are never logged.
    fn guard_z_order(&mut self, rect: PxRect) {
        let covered = dropwin::covered_by_ordinary_window(rect, &dropwin::windows_above(self.hwnd));
        if covered {
            // Again on every tick while it lasts (a raise that has not helped yet costs nothing), logged only once.
            let raised = dropwin::raise_topmost(self.hwnd);
            if !self.covered {
                match raised {
                    Ok(()) => tracing::warn!("Drop was below an ordinary window: put back on top"),
                    Err(e) => tracing::warn!("Drop was below an ordinary window and was not put back on top: {e}"),
                }
            }
        }
        self.covered = covered;
    }
}

/// Where the window belongs at rest: centered on the place's anchor. Its current size is used when it already has the
/// monitor's scale; otherwise the size it will have there (Windows resizes it on arrival), so it lands right in one go
/// when Windows keeps the top-left corner, and the next tick corrects it when it does not.
fn rest_origin(mon: Monitor, place: DropPosition, rect: PxRect, scale: f64) -> (i32, i32) {
    if (scale - mon.scale).abs() < 1e-6 {
        placement::origin_for_center(placement::anchor(mon.work, mon.scale, place, METRICS), rect.width(), rect.height())
    } else {
        placement::drop_origin(mon, place, METRICS, WINDOW_W, WINDOW_H)
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

    /// The Drop's size lives in two places: `--h` in drop.css draws it, DROP_SIZE places it.
    #[test]
    fn drop_size_matches_the_css() {
        let css = include_str!("../../ui/src/design/drop.css");
        let at = css.find("--h:").expect("drop.css defines --h");
        let value: String = css[at + 4..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        assert_eq!(css[at + 4 + value.len()..].get(..2), Some("px"));
        assert_eq!(value.parse::<f64>().unwrap(), DROP_SIZE);
    }

    #[test]
    fn window_center_is_the_drop_center_at_rest() {
        let mon = Monitor { work: PxRect { left: 0, top: 0, right: 1920, bottom: 1032 }, scale: 1.0 };
        for place in DropPosition::ALL {
            // Already on this monitor: the current size is used.
            let rect = PxRect { left: 5, top: 5, right: 645, bottom: 405 };
            let (x, y) = rest_origin(mon, place, rect, 1.0);
            let center = PxRect { left: x, top: y, right: x + 640, bottom: y + 400 }.center();
            assert_eq!(center, placement::anchor(mon.work, 1.0, place, METRICS), "{place:?}");
        }
    }

    #[test]
    fn rest_origin_on_another_scale_uses_the_size_the_window_will_have() {
        let mon = Monitor { work: PxRect { left: 1920, top: 0, right: 5760, bottom: 2088 }, scale: 1.5 };
        // Still at 100 % (640×400): placed as the 960×600 it becomes at 150 %.
        let rect = PxRect { left: 0, top: 0, right: 640, bottom: 400 };
        assert_eq!(rest_origin(mon, DropPosition::Left, rect, 1.0), (1485, 744));
        assert_eq!(rest_origin(mon, DropPosition::Left, rect, 1.0), placement::drop_origin(mon, DropPosition::Left, METRICS, WINDOW_W, WINDOW_H));
    }

    #[test]
    fn place_script_sets_the_code() {
        assert_eq!(place_init_script(DropPosition::Right), r#"window.__CUE_PLACE__ = "right";"#);
        assert_eq!(place_init_script(DropPosition::Bottom), r#"window.__CUE_PLACE__ = "bottom";"#);
    }

    #[test]
    fn drag_phases_from_the_page() {
        assert_eq!(serde_json::from_str::<DragPhase>(r#""start""#).unwrap(), DragPhase::Start);
        assert_eq!(serde_json::from_str::<DragPhase>(r#""end""#).unwrap(), DragPhase::End);
        assert!(serde_json::from_str::<DragPhase>(r#""move""#).is_err());
    }
}
