//! The main window (spec 3 §1): an ordinary window with the history and the settings, opened from the tray icon ("Open
//! CUE" in its menu, or a double click). There is one at a time: opening it again brings it forward. It is created when
//! opened and destroyed when closed (decision 6): no WebView stays in memory, and its page reads the history and the
//! settings again each time; its channel of new history entries and its microphone preview go with it. Closing it never
//! closes CUE (`RunEvent::ExitRequested` in `main`). Windows draws its frame, in the Windows theme; unlike the Drop, it
//! takes the focus.
//!
//! Never created on the main thread: on Windows, building a WebView inside a menu or tray icon handler there deadlocks
//! (wry#583). A thread of its own builds it while the main thread runs on.
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::ipc::{HistoryFeed, MAIN_LABEL};

/// Its size as the design draws it (logical px); narrower than its column it never gets.
const WIDTH: f64 = 700.0;
const HEIGHT: f64 = 760.0;
const MIN_HEIGHT: f64 = 480.0;

/// The window is being created: a second open meanwhile creates no second one.
static CREATING: Mutex<bool> = Mutex::new(false);

/// What an open does (the welcome window's too).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Open {
    /// The window is there: it comes forward.
    Bring,
    /// There is none: it is created, and `creating` is set until that ends.
    Create,
    /// It is being created: it comes forward by itself.
    Wait,
}

/// The step of an open, from whether the window is being created (`creating`, set here for a `Create`) and whether it
/// exists.
pub(crate) fn decide(creating: &mut bool, exists: bool) -> Open {
    if *creating {
        Open::Wait
    } else if exists {
        Open::Bring
    } else {
        *creating = true;
        Open::Create
    }
}

/// Opens the main window, or brings it forward. From any thread; returns at once.
pub fn open(app: &AppHandle) {
    let (step, win) = {
        let mut creating = CREATING.lock().unwrap_or_else(PoisonError::into_inner);
        // Looked up under the lock: a creation that has ended has its window registered by then.
        let win = app.get_webview_window(MAIN_LABEL);
        (decide(&mut creating, win.is_some()), win)
    };
    match (step, win) {
        (Open::Bring, Some(win)) => bring(&win),
        (Open::Create, _) => spawn_create(app.clone()),
        _ => {}
    }
}

/// The creation under way: its window's `CREATING` is cleared when it is dropped — when the creating thread ends
/// however it ends, or with that thread's closure when the thread did not start.
pub(crate) struct Creating(pub(crate) &'static Mutex<bool>);

impl Drop for Creating {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = false;
    }
}

fn spawn_create(app: AppHandle) {
    let creating = Creating(&CREATING);
    let spawned = std::thread::Builder::new().name("cue-main-window".into()).spawn(move || {
        let _creating = creating;
        match create(&app) {
            Ok(()) => tracing::info!("main window opened"),
            Err(e) => tracing::error!("main window not created: {e}"),
        }
    });
    if let Err(e) = spawned {
        tracing::error!("main window not opened: {e}");
    }
}

/// Builds the window (off the main thread, which it waits for). Its page is `window.html`.
fn create(app: &AppHandle) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("window.html".into()))
        .title("CUE")
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(WIDTH, MIN_HEIGHT)
        .center()
        .decorations(true)
        .theme(None)
        .focused(true)
        .build()?;
    let handle = app.clone();
    win.on_window_event(move |ev| {
        if let WindowEvent::Destroyed = ev {
            // Its page is gone: new history entries go nowhere, and a microphone level it showed closes (not the
            // welcome window's).
            if let Some(feed) = handle.try_state::<HistoryFeed>() {
                feed.stop();
            }
            crate::micpreview::stop(MAIN_LABEL);
            tracing::info!("main window closed");
        }
    });
    // Opened from the tray icon, a new window does not always come in front by itself.
    if let Err(e) = win.set_focus() {
        tracing::warn!("main window not focused: {e}");
    }
    Ok(())
}

/// The window as it is: restored if minimized, shown, in front.
fn bring(win: &WebviewWindow) {
    if let Err(e) = win.unminimize().and_then(|()| win.show()).and_then(|()| win.set_focus()) {
        tracing::warn!("main window not brought forward: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One window at a time: an open while it is being created (a double click right after "Open CUE") creates no
    /// other; once it is there, an open brings it forward.
    #[test]
    fn a_second_open_while_the_window_is_created_creates_no_other() {
        let mut creating = false;
        assert_eq!(decide(&mut creating, false), Open::Create);
        assert!(creating);
        assert_eq!(decide(&mut creating, false), Open::Wait);
        assert_eq!(decide(&mut creating, true), Open::Wait);
        creating = false;
        assert_eq!(decide(&mut creating, true), Open::Bring);
        assert!(!creating, "bringing it forward creates nothing");
        assert_eq!(decide(&mut creating, false), Open::Create, "closed since: created again");
    }
}
