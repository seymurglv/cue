//! The Drop window: creation, its place on the screen (one of four, chosen by dragging the Drop), click-through outside
//! the capsule and the tray card, and the z-order guard.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cue_core::settings::DropPosition;
use cue_platform_win::dropwin::{self, GuardStep, Seen, ZGuard};
use cue_platform_win::placement::{self, DropMetrics, Monitor, PxRect};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;

use crate::ipc::Ui;
use crate::lang::Lang;

pub const DROP_LABEL: &str = "drop";
/// The Drop's height in every state and the diameter of its circle, logical px. The same value as `--h` in
/// `app/ui/src/design/drop.css` (a test keeps them equal).
pub const DROP_SIZE: f64 = 36.0;
/// From the work area's edge to the Drop's outer edge, logical px.
pub const GAP_PX: f64 = 12.0;
pub const METRICS: DropMetrics = DropMetrics { size: DROP_SIZE, gap: GAP_PX };
/// Logical px. The window is centered on the Drop's center in every place (the page puts the Drop at its center), so a
/// new place never moves the Drop inside the window. It reaches far enough every way for the widest Drop, the tray
/// cards and the "Ask" answer tray (420 × up to 410, 2 % of spring overshoot, its shadow: 28 px to the sides, 12 up,
/// 44 down) growing toward the screen center: tall at the top and bottom, wide at the sides. A square big enough for
/// both would lie mostly on the old monitor after a move to one of another scale (see the test of rest moves). The
/// part beyond the edge of the work area is transparent and lets the mouse through, like the rest of the window.
pub fn window_size(place: DropPosition) -> (f64, f64) {
    match place {
        DropPosition::Bottom | DropPosition::Top => (640.0, 860.0),
        DropPosition::Left | DropPosition::Right => (900.0, 540.0),
    }
}
/// How often the window's place, the z-order and full screen are checked while the Drop is at rest.
const TICK: Duration = Duration::from_millis(500);
/// How often the Drop follows the cursor while dragged, and the step of the snap.
const FRAME: Duration = Duration::from_millis(8);
/// The snap to the chosen place after a drag: a short spring.
const SNAP: Duration = Duration::from_millis(250);
/// How far the snap may go past the place before it settles, logical px.
const SNAP_OVERSHOOT: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize)]
pub struct HitRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl HitRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    /// Covers the whole window of logical size `window`, give or take a pixel of rounding: what the page reports while
    /// the Drop is dragged. No capsule or tray card comes near that size.
    fn covers(&self, window: (f64, f64)) -> bool {
        self.x <= 1.0 && self.y <= 1.0 && self.x + self.w >= window.0 - 1.0 && self.y + self.h >= window.1 - 1.0
    }
}

/// What the cursor hit test needs: the rectangles of the capsule and the tray card in logical px relative to the window
/// (the page writes them), and whether the placement thread is moving the window.
#[derive(Clone, Default)]
pub struct HitState {
    rects: Arc<Mutex<Vec<HitRect>>>,
    /// A drag or the snap after it is going on. While dragging, the page reports the whole window as its rectangle;
    /// outside a move such a rectangle is ignored, so a page that missed the end of a drag (busy or hung) cannot leave
    /// the whole window catching clicks.
    moving: Arc<AtomicBool>,
}

impl HitState {
    /// The page's rectangles (the `*_hit_rects` commands of the Drop and the card windows).
    pub fn set(&self, rects: Vec<HitRect>) {
        *self.rects.lock().unwrap() = rects;
    }

    fn set_moving(&self, on: bool) {
        self.moving.store(on, Ordering::Relaxed);
    }
}

/// The cursor at (`x`, `y`) (logical px, relative to the window) is over one of `rects`. A rectangle covering the whole
/// window (of logical size `window`) counts only while the window is `moving`.
pub fn hit(rects: &[HitRect], x: f64, y: f64, window: (f64, f64), moving: bool) -> bool {
    rects.iter().filter(|r| moving || !r.covers(window)).any(|r| r.contains(x, y))
}

/// The window's drag messages: the pointer went down on the capsule and moved past the threshold, or went up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragPhase {
    Start,
    End,
}

/// Passes drag messages from the window to the placement thread, which moves the window.
pub struct Placer(Sender<DragPhase>);

impl Placer {
    pub fn new(tx: Sender<DragPhase>) -> Self {
        Self(tx)
    }
}

#[tauri::command]
pub fn drop_drag(window: WebviewWindow, phase: DragPhase, placer: tauri::State<'_, Placer>) {
    if crate::ipc::guard(&window, "drop_drag") {
        let _ = placer.0.send(phase);
    }
}

pub fn create(app: &AppHandle, lang: Lang, place: DropPosition) -> anyhow::Result<WebviewWindow> {
    let (w, h) = window_size(place);
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
        .inner_size(w, h)
        .visible(false)
        .build()?;
    let hwnd = win.hwnd()?;
    // Before the first show: from then on the subclass keeps the style and the no-activate show through any tao action.
    dropwin::make_non_activating(hwnd)?;
    // Centered on its place at its current size. If the window was created at another scale than the target monitor's,
    // most of it is on that monitor all the same (see `rest_origin`): Windows gives it that monitor's scale and size,
    // and the placement thread centers it again on its first tick.
    if let Some(mon) = placement::foreground_monitor() {
        let (x, y) = rest_origin(mon, place, dropwin::window_rect(hwnd)?);
        dropwin::move_to(hwnd, x, y)?;
    }
    win.set_ignore_cursor_events(true)?;
    win.show()?;
    Ok(win)
}

/// Loads the Drop's page again in another interface language (decision 5). Its address gets the language and the place
/// the Drop has now, which the page reads before the initialization scripts' values of startup: the first frame is laid
/// out for the place it is in. Once loaded, the page says it is ready (`drop_ready`) and gets the core's state back.
/// Main thread.
pub fn reload(app: &AppHandle, lang: Lang, place: DropPosition) {
    let Some(win) = app.get_webview_window(DROP_LABEL) else { return };
    let url = match win.url() {
        Ok(url) => crate::lang::with_param(crate::lang::lang_url(url, lang.code()), "place", place.code()),
        Err(e) => {
            tracing::warn!("Drop window: page address not read: {e}");
            return;
        }
    };
    if let Err(e) = win.navigate(url) {
        tracing::warn!("Drop window: page not loaded again: {e}");
    }
}

/// Initialization script for the Drop window: the place before the page's own scripts run, so the first frame is
/// already laid out for it. The code goes in through Rust's `{:?}` (a quoted Rust string literal); that is a valid
/// JavaScript string only because the code is always one of four fixed ASCII words.
fn place_init_script(place: DropPosition) -> String {
    format!("window.__CUE_PLACE__ = {:?};", place.code())
}

/// ~60 times a second while the window is visible: it takes the mouse only over its page's rectangles (the Drop's capsule
/// and tray cards; the "Ask" card in the card window). The window position and scale are read straight from the HWND; the
/// thread goes to the window thread only to switch. Ends when the window is gone.
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
                let window = (f64::from(rect.width()) / scale, f64::from(rect.height()) / scale);
                let moving = state.moving.load(Ordering::Relaxed);
                let over = hit(&state.rects.lock().unwrap(), x, y, window, moving);
                // If it failed, retry on the next tick; a closed window is caught by observe.
                if over == ignoring && win.set_ignore_cursor_events(!over).is_ok() {
                    ignoring = !over;
                }
            }
        })
        .expect("cursor hit-test thread");
}

/// The thread that moves the Drop window after its creation (Windows itself also resizes it on a scale change). At
/// rest, every 0.5 s: the Drop goes to its place on the monitor of the foreground window (again after a scale change),
/// the z-order guard runs, and over a full-screen app the idle strip hides. While dragged, the Drop follows the cursor;
/// when let go, it snaps to the nearest place, which is saved through `moved` (to settings.json) and sent to the page.
pub fn spawn_placement(
    win: WebviewWindow,
    ui: Ui,
    place: DropPosition,
    drags: Receiver<DragPhase>,
    hits: HitState,
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
        let mut desk = WinDesk { hwnd, ui: ui.clone(), moved: Box::new(moved) };
        let mut keeper = Keeper::new(hwnd, ui);
        let mut motion = Motion::new(place);
        let mut next_tick = Instant::now();
        loop {
            let wait = if motion.moving() { FRAME } else { next_tick.saturating_duration_since(Instant::now()) };
            match drags.recv_timeout(wait) {
                Ok(DragPhase::Start) => motion.start(&mut desk),
                Ok(DragPhase::End) => motion.end(&mut desk, Instant::now()),
                Err(RecvTimeoutError::Timeout) => {}
                // The sender lives in the app state; without it no drag can come, but the Drop still needs placing.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(wait),
            }
            match dropwin::observe(hwnd) {
                Seen::Gone => break,
                // Nothing to place while hidden, and no drag goes on. Looked at again in a tick: `next_tick` moves on,
                // so the loop waits instead of spinning.
                Seen::Hidden => {
                    motion.hidden(&mut desk);
                    next_tick = Instant::now() + TICK;
                }
                Seen::Shown { rect, scale } => {
                    if motion.moving() {
                        motion.frame(&mut desk, Instant::now());
                    } else if Instant::now() >= next_tick {
                        keeper.tick(motion.place, rect, scale);
                        next_tick = Instant::now() + TICK;
                    }
                }
            }
            hits.set_moving(motion.moving());
        }
        hits.set_moving(false);
    });
    if let Err(e) = spawned {
        tracing::error!("Drop placement thread not started: {e}");
    }
}

/// What the drag and the snap read from the system and do to it; a fake in the tests.
trait Desk {
    /// The Drop window's rect in physical px; None while it is hidden or gone.
    fn window(&self) -> Option<PxRect>;
    fn cursor(&self) -> (i32, i32);
    /// The primary mouse button is physically down.
    fn button_down(&self) -> bool;
    fn monitor_at(&self, point: (i32, i32)) -> Option<Monitor>;
    fn foreground_monitor(&self) -> Option<Monitor>;
    /// Windows animation effects are on.
    fn animations(&self) -> bool;
    /// Asks for the window's top-left corner at `origin` (physical px); false when the request failed.
    fn move_to(&mut self, origin: (i32, i32)) -> bool;
    /// Gives the window the size `size` (logical px) for the place, around the center of `rect`; the rect it asked for
    /// (or `rect` if the size is already that, or the request failed).
    fn resize(&mut self, size: (f64, f64), rect: PxRect) -> PxRect;
    /// Sends the place to the page.
    fn send_place(&mut self, place: DropPosition);
    /// Saves a new place (settings.json, through the core).
    fn save_place(&mut self, place: DropPosition);
}

struct WinDesk {
    hwnd: HWND,
    ui: Ui,
    moved: Box<dyn Fn(DropPosition) + Send>,
}

impl Desk for WinDesk {
    fn window(&self) -> Option<PxRect> {
        match dropwin::observe(self.hwnd) {
            Seen::Shown { rect, .. } => Some(rect),
            Seen::Hidden | Seen::Gone => None,
        }
    }

    fn cursor(&self) -> (i32, i32) {
        placement::cursor_pos()
    }

    fn button_down(&self) -> bool {
        dropwin::primary_button_down()
    }

    fn monitor_at(&self, point: (i32, i32)) -> Option<Monitor> {
        placement::monitor_at(point)
    }

    fn foreground_monitor(&self) -> Option<Monitor> {
        placement::foreground_monitor()
    }

    fn animations(&self) -> bool {
        dropwin::animations_enabled()
    }

    fn move_to(&mut self, origin: (i32, i32)) -> bool {
        dropwin::move_to(self.hwnd, origin.0, origin.1).is_ok()
    }

    fn resize(&mut self, size: (f64, f64), rect: PxRect) -> PxRect {
        let Seen::Shown { scale, .. } = dropwin::observe(self.hwnd) else { return rect };
        let (w, h) = ((size.0 * scale).round() as i32, (size.1 * scale).round() as i32);
        if (w, h) == (rect.width(), rect.height()) {
            return rect;
        }
        let (x, y) = placement::origin_for_center(rect.center(), w, h);
        let to = PxRect { left: x, top: y, right: x + w, bottom: y + h };
        match dropwin::set_rect(self.hwnd, to) {
            Ok(()) => to,
            Err(e) => {
                tracing::warn!("Drop window not resized: {e}");
                rect
            }
        }
    }

    fn send_place(&mut self, place: DropPosition) {
        self.ui.place(place);
    }

    fn save_place(&mut self, place: DropPosition) {
        (self.moved)(place);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Rest,
    /// Dragged: the Drop's center stays `grab` away from the cursor.
    Drag { grab: (i32, i32) },
    /// Snapping to its place: the Drop's center goes from `from` to `to` (physical px), at most `over` px past it.
    Snap { from: (i32, i32), to: (i32, i32), start: Instant, over: f64 },
}

/// The drag and the snap after it.
struct Motion {
    place: DropPosition,
    mode: Mode,
    /// The last origin requested while dragging or snapping: the same one is not sent again.
    last_origin: Option<(i32, i32)>,
}

impl Motion {
    fn new(place: DropPosition) -> Self {
        Self { place, mode: Mode::Rest, last_origin: None }
    }

    /// Dragged or snapping: frames every 8 ms, and the page's whole-window hit rectangle counts.
    fn moving(&self) -> bool {
        !matches!(self.mode, Mode::Rest)
    }

    /// Moves the window so its center is at `center`, keeping its current size (`rect`).
    fn center_at(&mut self, desk: &mut impl Desk, center: (i32, i32), rect: PxRect) {
        let origin = placement::origin_for_center(center, rect.width(), rect.height());
        // Already there, or already asked for (the move is posted to the window's thread and may still be on its way).
        if (rect.left, rect.top) == origin || self.last_origin == Some(origin) {
            return;
        }
        if desk.move_to(origin) {
            self.last_origin = Some(origin);
        }
    }

    /// Only while the primary button is down: a page message that arrives late, after the button went up, starts
    /// nothing. A refused start still answers with the place, so the page leaves its dragging state.
    fn start(&mut self, desk: &mut impl Desk) {
        let rect = match desk.window() {
            Some(rect) if desk.button_down() => rect,
            _ => {
                desk.send_place(self.place);
                return;
            }
        };
        let (cx, cy) = desk.cursor();
        let (wx, wy) = rect.center();
        self.mode = Mode::Drag { grab: (wx - cx, wy - cy) };
        self.last_origin = None;
        tracing::debug!("Drop: drag started");
    }

    /// The end of a drag (and nothing else): the nearest place to where the Drop was let go, on the monitor under it;
    /// saved if it changed and sent to the page in any case (that also ends the page's dragging state). Then the snap to
    /// that place on the monitor of the foreground window, as always: at once when Windows animations are off.
    fn end(&mut self, desk: &mut impl Desk, now: Instant) {
        if !matches!(self.mode, Mode::Drag { .. }) {
            return;
        }
        self.mode = Mode::Rest;
        let Some(rect) = desk.window() else {
            desk.send_place(self.place);
            return;
        };
        let center = rect.center();
        if let Some(under) = desk.monitor_at(center) {
            let place = placement::nearest_place(under, METRICS, center);
            if place != self.place {
                tracing::info!(place = place.code(), "Drop moved");
                self.place = place;
                desk.save_place(place);
            }
        }
        desk.send_place(self.place);
        // The window turns with the place (decision 14), around the Drop's center: the Drop stays where it was let go.
        // No drag happens while the answer tray is open (the page refuses it), so no tray is cut by the change.
        let rect = desk.resize(window_size(self.place), rect);
        let Some(mon) = desk.foreground_monitor().or_else(|| desk.monitor_at(center)) else { return };
        let to = placement::anchor(mon.work, mon.scale, self.place, METRICS);
        if desk.animations() {
            self.mode = Mode::Snap { from: center, to, start: now, over: SNAP_OVERSHOOT * mon.scale };
        } else {
            self.center_at(desk, to, rect);
        }
    }

    /// One frame of a drag or a snap. Dragged, the center follows the cursor; it is computed from the current window
    /// size, so a scale change on the way to another monitor keeps the Drop under the cursor, and the drag ends when the
    /// button is up, even if the page never said so. Snapping, the center goes along the snap curve.
    fn frame(&mut self, desk: &mut impl Desk, now: Instant) {
        let Some(rect) = desk.window() else {
            self.hidden(desk);
            return;
        };
        match self.mode {
            Mode::Rest => {}
            Mode::Drag { grab } => {
                if !desk.button_down() {
                    self.end(desk, now);
                    return;
                }
                let (cx, cy) = desk.cursor();
                self.center_at(desk, (cx + grab.0, cy + grab.1), rect);
            }
            Mode::Snap { from, to, start, over } => {
                let t = now.saturating_duration_since(start).as_secs_f64() / SNAP.as_secs_f64();
                self.center_at(desk, placement::snap_point(from, to, t, over), rect);
                if t >= 1.0 {
                    self.mode = Mode::Rest;
                }
            }
        }
    }

    /// The window is hidden: a drag ends where it is (the page gets the place, so it leaves its dragging state) and a
    /// snap stops; the next tick at rest puts the window in its place once it is shown again.
    fn hidden(&mut self, desk: &mut impl Desk) {
        match self.mode {
            Mode::Drag { .. } => {
                self.mode = Mode::Rest;
                desk.send_place(self.place);
            }
            Mode::Snap { .. } => self.mode = Mode::Rest,
            Mode::Rest => {}
        }
    }
}

/// The checks at rest, every 0.5 s: the window's place and size, the z-order guard, full screen.
struct Keeper {
    hwnd: HWND,
    ui: Ui,
    guard: ZGuard,
    /// The last move at rest failed: logged once, not twice a second.
    move_failed: bool,
    /// The window gets `idle-hidden` only on a change; at the start it assumes "don't hide".
    busy: bool,
}

impl Keeper {
    fn new(hwnd: HWND, ui: Ui) -> Self {
        Self { hwnd, ui, guard: ZGuard::default(), move_failed: false, busy: false }
    }

    fn tick(&mut self, place: DropPosition, rect: PxRect, scale: f64) {
        if let Some(mon) = placement::foreground_monitor() {
            let to = rest_rect(mon, place, rect, scale);
            let done = if (to.width(), to.height()) != (rect.width(), rect.height()) {
                // The size of another place: the resize after a drag failed or never landed.
                Some(dropwin::set_rect(self.hwnd, to))
            } else if (to.left, to.top) != (rect.left, rect.top) {
                Some(dropwin::move_to(self.hwnd, to.left, to.top))
            } else {
                None
            };
            match done {
                None => {}
                Some(Ok(())) => self.move_failed = false,
                // Retry on the next tick; log it once, not twice a second.
                Some(Err(e)) if !self.move_failed => {
                    self.move_failed = true;
                    tracing::warn!("Drop not repositioned: {e}");
                }
                Some(Err(_)) => {}
            }
        }
        self.guard_z_order(rect);
        let busy = placement::busy_fullscreen();
        if busy != self.busy {
            self.busy = busy;
            self.ui.idle_hidden(busy);
        }
    }

    /// An ordinary window above the Drop that overlaps it means the Drop lost its place on top: it is put back, without
    /// activation, and the next check tells whether that helped (`ZGuard`). Window titles are never logged.
    fn guard_z_order(&mut self, rect: PxRect) {
        let now = Instant::now();
        match self.guard.check(dropwin::covering_windows(rect, &dropwin::windows_above(self.hwnd)), now) {
            GuardStep::Idle => {}
            GuardStep::Raise => {
                tracing::debug!("Drop below an ordinary window: raising it");
                if let Err(e) = dropwin::raise_topmost(self.hwnd) {
                    // The request itself failed: the Drop was not raised, which says nothing about the windows over it.
                    if self.guard.raise_failed(now) {
                        tracing::warn!("Drop below an ordinary window, and the request to put it on top failed: {e}");
                    } else {
                        tracing::debug!("Drop: the request to put it on top failed again: {e}");
                    }
                }
            }
            GuardStep::Fixed => tracing::warn!("Drop was below an ordinary window: on top again"),
            GuardStep::NotFixed => {
                let every = dropwin::GUARD_RETRY.as_secs();
                tracing::warn!("Drop is still below an ordinary window after being raised: trying again every {every} s");
            }
            GuardStep::Uncovered => tracing::info!("Drop no longer covered"),
        }
    }
}

/// Where the window belongs at rest: centered on its place's anchor, at its current size. On a monitor of another scale
/// the size changes once the window is there. Most of the window is on that monitor even before (the anchor is a few
/// dozen px inside it and the window is centered on it), so Windows does give it that monitor's scale, and the next
/// tick centers it again at its new size. Placed at the size it would have there instead, most of it can stay on the
/// old monitor: then its scale never changes and it never arrives.
fn rest_origin(mon: Monitor, place: DropPosition, rect: PxRect) -> (i32, i32) {
    placement::origin_for_center(placement::anchor(mon.work, mon.scale, place, METRICS), rect.width(), rect.height())
}

/// Where the window belongs at rest, with its size: the place's size at the window's own `scale` (as `rest_origin`,
/// never the target monitor's), so a resize after a drag that failed or never landed is made here; the current size
/// when it is that, give or take a pixel of rounding.
fn rest_rect(mon: Monitor, place: DropPosition, rect: PxRect, scale: f64) -> PxRect {
    let (lw, lh) = window_size(place);
    let (w, h) = ((lw * scale).round() as i32, (lh * scale).round() as i32);
    let off = (rect.width() - w).abs() > 1 || (rect.height() - h).abs() > 1;
    let sized = if off { PxRect { right: rect.left + w, bottom: rect.top + h, ..rect } } else { rect };
    let (x, y) = rest_origin(mon, place, sized);
    PxRect { left: x, top: y, right: x + sized.width(), bottom: y + sized.height() }
}

#[tauri::command]
pub fn drop_hit_rects(window: WebviewWindow, rects: Vec<HitRect>, state: tauri::State<'_, HitState>) {
    if crate::ipc::guard(&window, "drop_hit_rects") {
        state.set(rects);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_and_outside() {
        let r = [HitRect { x: 100.0, y: 200.0, w: 160.0, h: 40.0 }];
        let window = (640.0, 400.0);
        assert!(hit(&r, 100.0, 200.0, window, false));
        assert!(hit(&r, 259.9, 239.9, window, false));
        assert!(!hit(&r, 260.0, 220.0, window, false));
        assert!(!hit(&r, 150.0, 199.0, window, false));
        assert!(!hit(&[], 1.0, 1.0, window, false));
    }

    #[test]
    fn the_whole_window_rect_counts_only_while_the_window_is_moved() {
        let window = (640.0, 400.0);
        let whole = [HitRect { x: 0.0, y: 0.0, w: 640.0, h: 400.0 }];
        assert!(hit(&whole, 10.0, 10.0, window, true));
        assert!(!hit(&whole, 10.0, 10.0, window, false));
        assert!(!hit(&whole, 320.0, 200.0, window, false));
        // A fraction of a pixel off is still the whole window.
        assert!(!hit(&[HitRect { x: 0.0, y: 0.0, w: 639.4, h: 399.6 }], 10.0, 10.0, window, false));
        // The capsule and the largest tray card count either way.
        let parts = [HitRect { x: 302.0, y: 182.0, w: 183.0, h: 36.0 }, HitRect { x: 170.0, y: 34.0, w: 300.0, h: 148.0 }];
        for moving in [false, true] {
            assert!(hit(&parts, 310.0, 190.0, window, moving));
            assert!(hit(&parts, 200.0, 100.0, window, moving));
            assert!(!hit(&parts, 10.0, 10.0, window, moving));
        }
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

    const FHD: Monitor = Monitor { work: PxRect { left: 0, top: 0, right: 1920, bottom: 1032 }, scale: 1.0 };

    fn window_at(center: (i32, i32), w: i32, h: i32) -> PxRect {
        let (x, y) = placement::origin_for_center(center, w, h);
        PxRect { left: x, top: y, right: x + w, bottom: y + h }
    }

    #[test]
    fn window_center_is_the_drop_center_at_rest() {
        for place in DropPosition::ALL {
            let rect = PxRect { left: 5, top: 5, right: 645, bottom: 405 };
            let (x, y) = rest_origin(FHD, place, rect);
            let center = PxRect { left: x, top: y, right: x + 640, bottom: y + 400 }.center();
            assert_eq!(center, placement::anchor(FHD.work, 1.0, place, METRICS), "{place:?}");
        }
    }

    /// A monitor: its full bounds, and its work area with a 48 px (logical) taskbar at the bottom.
    fn screen(left: i32, top: i32, w: i32, h: i32, scale: f64) -> (PxRect, Monitor) {
        let bounds = PxRect { left, top, right: left + w, bottom: top + h };
        let taskbar = (48.0 * scale).round() as i32;
        (bounds, Monitor { work: PxRect { bottom: bounds.bottom - taskbar, ..bounds }, scale })
    }

    /// Windows gives a window the scale of the monitor it shares the most area with. Moved to a monitor of another
    /// scale at its current size, the window lies mostly on that monitor, so it gets that scale and the next tick
    /// centers it again at its new size.
    #[test]
    fn a_rest_move_to_a_monitor_of_another_scale_lands_mostly_on_it() {
        // (the monitor the window is on, the target): side by side and one above the other, either scale larger.
        let pairs = [
            (screen(0, 0, 1920, 1080, 1.0), screen(1920, 0, 3840, 2160, 1.5)),
            (screen(0, 0, 3840, 2160, 1.5), screen(3840, 0, 1920, 1080, 1.0)),
            (screen(0, 0, 1920, 1080, 1.0), screen(-2560, 0, 2560, 1440, 1.25)),
            (screen(0, 1080, 3840, 2160, 2.0), screen(0, 0, 1920, 1080, 1.0)),
            (screen(0, 0, 1920, 1080, 1.0), screen(0, 1080, 3840, 2160, 2.0)),
            (screen(0, -1440, 2560, 1440, 1.75), screen(0, 0, 1920, 1080, 1.0)),
        ];
        for ((old_bounds, old), (new_bounds, new)) in pairs {
            for place in DropPosition::ALL {
                // The window as it is on the old monitor at this place: its size there.
                let (lw, lh) = window_size(place);
                let (w, h) = ((lw * old.scale).round() as i32, (lh * old.scale).round() as i32);
                let rect = window_at(placement::anchor(old.work, old.scale, place, METRICS), w, h);
                let (x, y) = rest_origin(new, place, rect);
                let moved = PxRect { left: x, top: y, right: x + w, bottom: y + h };
                assert_eq!(moved.center(), placement::anchor(new.work, new.scale, place, METRICS));
                assert_eq!(
                    placement::largest_overlap(moved, &[old_bounds, new_bounds]),
                    Some(1),
                    "{place:?} from {old:?} to {new:?}"
                );
            }
        }
        // Placed at the size it would have there instead (960×600 at 150 %), a 640×400 window from the 100 % monitor
        // lies mostly on the 100 % monitor: its scale never changes.
        let ((old_bounds, _), (new_bounds, new)) = pairs[0];
        let anchor = placement::anchor(new.work, new.scale, DropPosition::Left, METRICS);
        let (x, y) = placement::origin_for_center(anchor, 960, 600);
        let stuck = PxRect { left: x, top: y, right: x + 640, bottom: y + 400 };
        assert_eq!(placement::largest_overlap(stuck, &[old_bounds, new_bounds]), Some(0));
    }

    /// A resize after a drag that failed or never landed: the next check at rest gives the window its place's size at
    /// its own scale, centered on the place; a size a pixel off (rounding) is left as it is.
    #[test]
    fn at_rest_the_window_gets_its_places_size_back() {
        let bottom = placement::anchor(FHD.work, 1.0, DropPosition::Bottom, METRICS);
        let to = rest_rect(FHD, DropPosition::Bottom, window_at(bottom, 900, 540), 1.0);
        assert_eq!((to.width(), to.height(), to.center()), (640, 860, bottom));
        let close = window_at(bottom, 641, 859);
        assert_eq!(rest_rect(FHD, DropPosition::Bottom, close, 1.0), close, "rounding: left as it is");
        let left = placement::anchor(FHD.work, FHD.scale, DropPosition::Left, METRICS);
        let to = rest_rect(FHD, DropPosition::Left, window_at((5, 5), 960, 1290), 1.5);
        assert_eq!((to.width(), to.height(), to.center()), (1350, 810, left), "the window's own scale");
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

    /// One 1920×1080 monitor at 100 %; the window moves at once when asked.
    struct Fake {
        window: Option<PxRect>,
        cursor: (i32, i32),
        button: bool,
        animations: bool,
        moves: Vec<(i32, i32)>,
        sent: Vec<DropPosition>,
        saved: Vec<DropPosition>,
    }

    impl Fake {
        /// The window at the bottom place, the cursor on the Drop, the button down.
        fn new() -> Self {
            let bottom = placement::anchor(FHD.work, 1.0, DropPosition::Bottom, METRICS);
            Self {
                window: Some(window_at(bottom, 640, 860)),
                cursor: (bottom.0, bottom.1 - 2),
                button: true,
                animations: true,
                moves: vec![],
                sent: vec![],
                saved: vec![],
            }
        }

        fn center(&self) -> (i32, i32) {
            self.window.unwrap().center()
        }
    }

    impl Desk for Fake {
        fn window(&self) -> Option<PxRect> {
            self.window
        }

        fn cursor(&self) -> (i32, i32) {
            self.cursor
        }

        fn button_down(&self) -> bool {
            self.button
        }

        fn monitor_at(&self, _: (i32, i32)) -> Option<Monitor> {
            Some(FHD)
        }

        fn foreground_monitor(&self) -> Option<Monitor> {
            Some(FHD)
        }

        fn animations(&self) -> bool {
            self.animations
        }

        fn move_to(&mut self, origin: (i32, i32)) -> bool {
            self.moves.push(origin);
            if let Some(r) = &mut self.window {
                *r = PxRect { left: origin.0, top: origin.1, right: origin.0 + r.width(), bottom: origin.1 + r.height() };
            }
            true
        }

        /// At 100 %: the logical size is the physical one.
        fn resize(&mut self, size: (f64, f64), rect: PxRect) -> PxRect {
            let to = window_at(rect.center(), size.0 as i32, size.1 as i32);
            self.window = Some(to);
            to
        }

        fn send_place(&mut self, place: DropPosition) {
            self.sent.push(place);
        }

        fn save_place(&mut self, place: DropPosition) {
            self.saved.push(place);
        }
    }

    const MS: Duration = Duration::from_millis(1);

    /// Picks the Drop up and carries it so its center is at `to`.
    fn drag_to(m: &mut Motion, d: &mut Fake, to: (i32, i32), now: Instant) {
        m.start(d);
        assert!(m.moving());
        let (cx, cy) = d.cursor;
        let (wx, wy) = d.center();
        d.cursor = (to.0 - (wx - cx), to.1 - (wy - cy));
        m.frame(d, now);
        assert_eq!(d.center(), to);
    }

    #[test]
    fn start_is_refused_when_the_button_is_up_or_the_window_hidden() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        d.button = false;
        m.start(&mut d);
        assert!(!m.moving());
        // The page still gets the place, so it leaves its dragging state.
        assert_eq!(d.sent, [DropPosition::Bottom]);
        d.button = true;
        d.window = None;
        m.start(&mut d);
        assert!(!m.moving());
        assert_eq!(d.sent, [DropPosition::Bottom, DropPosition::Bottom]);
        assert!(d.moves.is_empty() && d.saved.is_empty());
    }

    #[test]
    fn end_counts_only_during_a_drag() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        let now = Instant::now();
        m.end(&mut d, now);
        assert!(!m.moving());
        assert!(d.sent.is_empty() && d.saved.is_empty() && d.moves.is_empty());
        drag_to(&mut m, &mut d, (300, 602), now);
        m.end(&mut d, now);
        assert!(matches!(m.mode, Mode::Snap { .. }));
        let (sent, saved) = (d.sent.clone(), d.saved.clone());
        // A second end (the page's, after the core already ended the drag) changes nothing.
        m.end(&mut d, now + 10 * MS);
        assert!(matches!(m.mode, Mode::Snap { .. }));
        assert_eq!((d.sent.clone(), d.saved.clone()), (sent, saved));
    }

    #[test]
    fn the_drop_follows_the_cursor_and_the_drag_ends_when_the_button_goes_up() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        let now = Instant::now();
        m.start(&mut d);
        // The center keeps its distance from the cursor (2 px below it here).
        d.cursor = (300, 600);
        m.frame(&mut d, now);
        assert_eq!(d.center(), (300, 602));
        // The same cursor: no new move request.
        m.frame(&mut d, now + FRAME);
        assert_eq!(d.moves.len(), 1);
        // The button goes up without a word from the page: the drag ends there. The left center (30, 516) is the nearest.
        d.button = false;
        m.frame(&mut d, now + 2 * FRAME);
        assert_eq!(d.saved, [DropPosition::Left]);
        assert_eq!(d.sent, [DropPosition::Left]);
        assert_eq!(m.place, DropPosition::Left);
        assert!(matches!(m.mode, Mode::Snap { to: (30, 516), .. }));
    }

    #[test]
    fn the_snap_settles_exactly_on_the_place() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        let t0 = Instant::now();
        drag_to(&mut m, &mut d, (300, 602), t0);
        m.end(&mut d, t0);
        let mut centers = vec![];
        let mut t = t0;
        while m.moving() {
            t += FRAME;
            m.frame(&mut d, t);
            centers.push(d.center());
            assert!(t - t0 <= SNAP + FRAME, "the snap does not end");
        }
        assert_eq!(d.center(), (30, 516));
        assert_eq!(*d.moves.last().unwrap(), (30 - 450, 516 - 270));
        // A short spring: it goes past the place by a few px at most (moving left here), then comes back.
        let leftmost = centers.iter().map(|c| c.0).min().unwrap();
        assert!((30 - 5..30).contains(&leftmost), "{leftmost}");
        assert!(centers.iter().all(|c| c.1 >= 514), "{centers:?}");
    }

    /// Decision 14: the window turns with the place, around the Drop's center.
    #[test]
    fn a_new_place_turns_the_window_around_the_drop() {
        let now = Instant::now();
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        drag_to(&mut m, &mut d, (300, 602), now);
        m.end(&mut d, now);
        assert_eq!(d.window.map(|r| (r.width(), r.height(), r.center())), Some((900, 540, (300, 602))));
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        drag_to(&mut m, &mut d, (800, 900), now);
        m.end(&mut d, now);
        assert_eq!(d.window.map(|r| (r.width(), r.height())), Some((640, 860)), "the same place: the same window");
    }

    /// Tall where the answer grows up or down, wide where it grows sideways; the same for opposite places.
    #[test]
    fn the_window_size_follows_the_place() {
        assert_eq!(window_size(DropPosition::Bottom), (640.0, 860.0));
        assert_eq!(window_size(DropPosition::Top), window_size(DropPosition::Bottom));
        assert_eq!(window_size(DropPosition::Left), (900.0, 540.0));
        assert_eq!(window_size(DropPosition::Right), window_size(DropPosition::Left));
    }

    #[test]
    fn without_animations_the_drop_lands_at_once() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        d.animations = false;
        let now = Instant::now();
        drag_to(&mut m, &mut d, (1700, 300), now);
        d.button = false;
        m.frame(&mut d, now + FRAME);
        assert!(!m.moving());
        assert_eq!(d.saved, [DropPosition::Right]);
        assert_eq!(d.center(), (1890, 516));
    }

    #[test]
    fn let_go_near_its_own_place_saves_nothing_but_still_answers_the_page() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        let now = Instant::now();
        drag_to(&mut m, &mut d, (800, 900), now);
        // The page's end comes first this time.
        m.end(&mut d, now + FRAME);
        assert!(d.saved.is_empty());
        assert_eq!(d.sent, [DropPosition::Bottom]);
        assert!(matches!(m.mode, Mode::Snap { to: (960, 1002), .. }));
    }

    #[test]
    fn a_hidden_window_ends_the_drag_where_it_is_and_stops_a_snap() {
        let (mut m, mut d) = (Motion::new(DropPosition::Bottom), Fake::new());
        let now = Instant::now();
        drag_to(&mut m, &mut d, (300, 602), now);
        d.window = None;
        m.frame(&mut d, now + FRAME);
        assert!(!m.moving());
        assert_eq!(d.sent, [DropPosition::Bottom]);
        assert!(d.saved.is_empty());
        // Hidden while snapping: the snap stops, nothing more is sent.
        d.window = Some(window_at((300, 602), 640, 400));
        drag_to(&mut m, &mut d, (300, 602), now);
        m.end(&mut d, now);
        assert!(matches!(m.mode, Mode::Snap { .. }));
        let sent = d.sent.len();
        m.hidden(&mut d);
        assert!(!m.moving());
        assert_eq!(d.sent.len(), sent);
    }
}
