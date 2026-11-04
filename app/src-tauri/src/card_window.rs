//! The "Ask" card at the selection (spec 2 §7): its own window, created hidden at startup, never activated (the Drop's
//! window subclass), always on top, click-through outside the card. Its thread puts the window at the selection, waits for
//! its size on a monitor of another scale, and shows or hides it; the page draws the answer it gets as `cue://answer`.
//!
//! Shown and hidden through Tauri, never with a bare `ShowWindow`: on any change of its flags (the hit test switches
//! click-through ~60 times a second) tao hides a window it believes hidden.
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use cue_platform_win::cardplace::{self, CARD};
use cue_platform_win::dropwin;
use cue_platform_win::placement::PxRect;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;

use crate::drop_window::{HitRect, HitState};
use crate::lang::Lang;

pub const CARD_LABEL: &str = "card";
/// How long a card moved to a monitor of another scale waits for Windows to give it its size there.
const SETTLE_FOR: Duration = Duration::from_millis(300);
const SETTLE_POLL: Duration = Duration::from_millis(15);
/// The card folds away (`.ansC.out` in answer.css, 0.25 s) before its window hides.
const HIDE_AFTER: Duration = Duration::from_millis(260);

/// The card window's hit test state: a type of its own, so Tauri keeps it apart from the Drop's.
#[derive(Clone, Default)]
pub struct CardHits(pub HitState);

#[tauri::command]
pub fn card_hit_rects(window: WebviewWindow, rects: Vec<HitRect>, state: tauri::State<'_, CardHits>) {
    if crate::ipc::guard(&window, "card_hit_rects") {
        state.0.set(rects);
    }
}

pub fn create(app: &AppHandle, lang: Lang) -> anyhow::Result<WebviewWindow> {
    let (w, h) = cardplace::window_size(CARD);
    let win = WebviewWindowBuilder::new(app, CARD_LABEL, WebviewUrl::App("card.html".into()))
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
        .inner_size(w, h)
        .visible(false)
        .build()?;
    crate::webview_health::watch(&win);
    // Before the first show, as for the Drop: no activation on any show, move or z-order change.
    dropwin::make_non_activating(win.hwnd()?)?;
    win.set_ignore_cursor_events(true)?;
    Ok(win)
}

/// Loads the card's page again in another interface language (decision 5): its address gets the language, which the
/// page reads before the initialization script's value of startup. Shown or hidden, the window stays as it is; once
/// loaded, the page says it is ready (`card_ready`) and gets the state, its place and an open answer back. Main thread.
pub fn reload(app: &AppHandle, lang: Lang) {
    let Some(win) = app.get_webview_window(CARD_LABEL) else { return };
    let url = match win.url() {
        Ok(url) => crate::lang::lang_url(url, lang.code()),
        Err(e) => {
            tracing::warn!("card window: page address not read: {e}");
            return;
        }
    };
    if let Err(e) = win.navigate(url) {
        tracing::warn!("card window: page not loaded again: {e}");
    }
}

enum Cmd {
    Show(PxRect),
    Hide,
}

/// The core's handle on the card window. Without a window (it was not created) every answer goes to the tray.
#[derive(Clone)]
pub struct Card(Option<Sender<Cmd>>);

impl Card {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn available(&self) -> bool {
        self.0.is_some()
    }

    /// The card window at `rect` (physical px), shown.
    pub fn show(&self, rect: PxRect) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(Cmd::Show(rect));
        }
    }

    /// Hidden once the card has folded away.
    pub fn hide(&self) {
        if let Some(tx) = &self.0 {
            let _ = tx.send(Cmd::Hide);
        }
    }
}

/// The thread that moves, shows and hides the card window. Ends when the core drops its handle.
pub fn spawn(win: WebviewWindow) -> Card {
    let (tx, rx) = mpsc::channel();
    match std::thread::Builder::new().name("cue-card".into()).spawn(move || run(&win, &rx)) {
        Ok(_) => Card(Some(tx)),
        Err(e) => {
            tracing::error!("card thread not started: {e}");
            Card::none()
        }
    }
}

fn run(win: &WebviewWindow, rx: &Receiver<Cmd>) {
    // HWND is not Send, so we get it inside the thread: one request to the window thread.
    let hwnd = match win.hwnd() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            tracing::error!("card: no HWND: {e}");
            return;
        }
    };
    let mut next = None;
    loop {
        let cmd = match next.take() {
            Some(cmd) => cmd,
            None => match rx.recv() {
                Ok(cmd) => cmd,
                Err(_) => return,
            },
        };
        match cmd {
            Cmd::Show(rect) => {
                settle(hwnd, rect);
                let _ = dropwin::raise_topmost(hwnd);
                if let Err(e) = win.show() {
                    tracing::warn!("card not shown: {e}");
                }
            }
            // A new answer that comes while the card folds away keeps the window.
            Cmd::Hide => match rx.recv_timeout(HIDE_AFTER) {
                Ok(cmd) => next = Some(cmd),
                Err(RecvTimeoutError::Timeout) => {
                    let _ = win.hide();
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let _ = win.hide();
                    return;
                }
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Settle {
    Done,
    /// At its size there, but Windows moved it while rescaling: move it again.
    MoveAgain,
    /// Not yet at the size it has on that monitor.
    Wait,
}

/// The window `now` after a move toward `target`. Sizes count as equal give or take a pixel of rounding.
fn settled(now: PxRect, target: PxRect) -> Settle {
    let near = |a: i32, b: i32| (a - b).abs() <= 1;
    if !near(now.width(), target.width()) || !near(now.height(), target.height()) {
        return Settle::Wait;
    }
    if now.left == target.left && now.top == target.top { Settle::Done } else { Settle::MoveAgain }
}

/// Moves the hidden window to `rect`. On a monitor of another scale Windows gives it that scale and its size there a moment
/// later (most of the window is on that monitor already: the card is inside the work area); then it moves again, so the
/// arrow lands exactly. At most `SETTLE_FOR`: after that it shows where it is.
fn settle(hwnd: HWND, rect: PxRect) {
    if dropwin::move_to(hwnd, rect.left, rect.top).is_err() {
        return;
    }
    let until = Instant::now() + SETTLE_FOR;
    while Instant::now() < until {
        std::thread::sleep(SETTLE_POLL);
        match dropwin::window_rect(hwnd).map(|now| settled(now, rect)) {
            Ok(Settle::Done) => return,
            Ok(Settle::MoveAgain) => {
                let _ = dropwin::move_to(hwnd, rect.left, rect.top);
            }
            Ok(Settle::Wait) | Err(_) => {}
        }
    }
    tracing::debug!("card: not settled in time, shown as it is");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> PxRect {
        PxRect { left, top, right, bottom }
    }

    /// The card's sizes live in two places: answer.css draws the card, CARD places its window.
    #[test]
    fn card_metrics_match_the_css() {
        let css = include_str!("../../ui/src/design/answer.css");
        let (pad, top, width) = (CARD.pad, CARD.pad + CARD.arrow, CARD.width);
        assert!(css.contains(&format!(".ansC{{position:absolute;left:{pad}px;top:{top}px;width:{width}px;")), "left, top, width");
        assert!(css.contains(&format!(".ansC.above{{top:auto;bottom:{top}px;")), "the card above the selection");
        assert!(css.contains(&format!("max-height:var(--max-h,{}px)", CARD.max_height)), "the tallest card");
    }

    #[test]
    fn the_window_is_settled_at_its_size_there_give_or_take_a_pixel() {
        let target = r(556, 300, 994, 654);
        assert_eq!(settled(target, target), Settle::Done);
        assert_eq!(settled(r(556, 300, 995, 653), target), Settle::Done, "rounding at 125 %");
        assert_eq!(settled(r(540, 280, 978, 634), target), Settle::MoveAgain, "rescaled and moved by Windows");
        assert_eq!(settled(r(556, 300, 848, 536), target), Settle::Wait, "still at the old scale's size");
    }
}
