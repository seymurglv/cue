//! The welcome window (spec 4a §1): hello, the keys, the microphone, a first dictation, done. It opens at a start when
//! the Soniox or the Anthropic key is missing — the first start, or the keys removed since — and with `--welcome`, also
//! when a second launch brings it to the CUE already running. Like the main window (`main_window`) there is one at a
//! time, created when opened and destroyed when closed, never built on the main thread. Windows draws its frame, in the
//! Windows theme, with the close button alone; it takes the focus, so its first dictation is pasted into its own field.
//!
//! While it is open the hotkey starts nothing except on its first dictation's step (`Input::WelcomeGate`): it is held
//! from before the window is created and let go when the window is destroyed or could not be created (`Hold`); in
//! between its page moves it with its steps (`commands::welcome_step`). A chord it holds brings it forward
//! (`bring_forward`). Its microphone preview goes with it; the main window's stays.
use std::ffi::OsStr;
use std::sync::{Mutex, PoisonError};

use cue_core::flow::Input;
use cue_engines::keys::{self, KeyName};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tokio::sync::mpsc::UnboundedSender;

use crate::ipc::{CoreTx, WELCOME_LABEL};
use crate::main_window::{Creating, Open, decide};
use crate::runtime::CoreEvent;

/// Its size as the design draws it (logical px); it never changes.
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 430.0;
/// The argument that opens it whatever the keys.
const WELCOME_ARG: &str = "--welcome";

/// The window is being created: a second open meanwhile creates no second one.
static CREATING: Mutex<bool> = Mutex::new(false);

/// Whether a key is in Windows Credential Manager: read there, with no network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyPresence {
    Saved,
    Missing,
    /// The store could not be read.
    Unreadable,
}

/// The window opens when it is asked for (`forced`) or a key is missing. A store that could not be read is not a
/// missing key: no window for that (the main window's keys show the failure).
pub fn should_welcome(soniox: KeyPresence, anthropic: KeyPresence, forced: bool) -> bool {
    forced || soniox == KeyPresence::Missing || anthropic == KeyPresence::Missing
}

/// `--welcome` among a launch's arguments; CUE takes no others.
pub fn asks_welcome<S: AsRef<OsStr>>(args: impl IntoIterator<Item = S>) -> bool {
    args.into_iter().any(|a| a.as_ref() == WELCOME_ARG)
}

/// At startup: the window opens if a key is missing, or when `forced` (`--welcome`). The log gets whether each key is
/// there, never a key.
pub fn open_at_start(app: &AppHandle, forced: bool) {
    let (soniox, anthropic) = (presence(KeyName::Soniox), presence(KeyName::Anthropic));
    let opens = should_welcome(soniox, anthropic, forced);
    tracing::info!(?soniox, ?anthropic, forced, opens, "welcome window at startup");
    if opens {
        open(app);
    }
}

/// Whether `name`'s key is saved; the key itself is dropped at once.
fn presence(name: KeyName) -> KeyPresence {
    match keys::lookup(name) {
        Ok(Some(_)) => KeyPresence::Saved,
        Ok(None) => KeyPresence::Missing,
        Err(_) => {
            tracing::warn!(key = name.as_str(), "welcome window: the key store was not read");
            KeyPresence::Unreadable
        }
    }
}

/// Opens the welcome window, or brings it forward. From any thread; returns at once.
pub fn open(app: &AppHandle) {
    let (step, win) = {
        let mut creating = CREATING.lock().unwrap_or_else(PoisonError::into_inner);
        // Looked up under the lock: a creation that has ended has its window registered by then.
        let win = app.get_webview_window(WELCOME_LABEL);
        (decide(&mut creating, win.is_some()), win)
    };
    match (step, win) {
        (Open::Bring, Some(win)) => bring(&win),
        (Open::Create, _) => spawn_create(app.clone()),
        _ => {}
    }
}

/// A chord the window holds: the window comes forward if it is there. It is never created for that: a hold without a
/// window is one being created, or one whose end is on its way.
pub fn bring_forward(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(WELCOME_LABEL) {
        tracing::info!("welcome window brought forward: it holds the hotkey");
        bring(&win);
    }
}

fn spawn_create(app: AppHandle) {
    let creating = Creating(&CREATING);
    let Some(core) = app.try_state::<CoreTx>().map(|core| core.0.clone()) else {
        tracing::error!("welcome window not opened: no core");
        return;
    };
    // Held from here on: the window's end lets it go, and so does a creation that fails or a thread that does not start
    // (the closure drops it).
    let hold = Hold::new(core);
    let spawned = std::thread::Builder::new().name("cue-welcome-window".into()).spawn(move || {
        let _creating = creating;
        match create(&app, hold) {
            Ok(()) => tracing::info!("welcome window opened"),
            Err(e) => tracing::error!("welcome window not created: {e}"),
        }
    });
    if let Err(e) = spawned {
        tracing::error!("welcome window not opened: {e}");
    }
}

/// Builds the window (off the main thread, which it waits for). Its page is `welcome.html`. `hold` goes with the
/// window, or with this call when it fails.
fn create(app: &AppHandle, hold: Hold) -> tauri::Result<()> {
    let win = WebviewWindowBuilder::new(app, WELCOME_LABEL, WebviewUrl::App("welcome.html".into()))
        .title("CUE")
        .inner_size(WIDTH, HEIGHT)
        .center()
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .decorations(true)
        .theme(None)
        .focused(true)
        .build()?;
    let hold = Mutex::new(Some(hold));
    win.on_window_event(move |ev| on_event(&hold, ev));
    // Opened at startup or from a second launch, a new window does not always come in front by itself.
    if let Err(e) = win.set_focus() {
        tracing::warn!("welcome window not focused: {e}");
    }
    Ok(())
}

/// The window is destroyed: the hotkey is let go and its microphone preview ends (`Hold`), once.
fn on_event(hold: &Mutex<Option<Hold>>, ev: &WindowEvent) {
    if let WindowEvent::Destroyed = ev {
        let held = hold.lock().unwrap_or_else(PoisonError::into_inner).take();
        if held.is_some() {
            drop(held);
            tracing::info!("welcome window closed");
        }
    }
}

/// The window as it is: restored if minimized, shown, in front.
fn bring(win: &WebviewWindow) {
    if let Err(e) = win.unminimize().and_then(|()| win.show()).and_then(|()| win.set_focus()) {
        tracing::warn!("welcome window not brought forward: {e}");
    }
}

/// The hotkey held for the welcome window (`Input::WelcomeGate`): held when made, let go when dropped — when the window
/// is destroyed, or with its creation however that failed. The window's microphone preview ends then too.
struct Hold(UnboundedSender<CoreEvent>);

impl Hold {
    fn new(core: UnboundedSender<CoreEvent>) -> Self {
        gate(&core, true);
        Self(core)
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        gate(&self.0, false);
        crate::micpreview::stop(WELCOME_LABEL);
    }
}

fn gate(core: &UnboundedSender<CoreEvent>, held: bool) {
    // A core that has ended holds nothing.
    let _ = core.send(CoreEvent::Input(Input::WelcomeGate(held)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use KeyPresence::{Missing, Saved, Unreadable};
    use std::ffi::OsString;
    use tokio::sync::mpsc::UnboundedReceiver;

    #[test]
    fn welcome_when_a_key_is_missing() {
        assert!(should_welcome(Missing, Missing, false), "the first start");
        assert!(should_welcome(Missing, Saved, false));
        assert!(should_welcome(Saved, Missing, false));
        assert!(should_welcome(Missing, Unreadable, false), "the other key not read: this one is still missing");
    }

    #[test]
    fn no_welcome_when_both_keys_are_there() {
        assert!(!should_welcome(Saved, Saved, false));
    }

    /// A store that failed says nothing about the keys: no window for that.
    #[test]
    fn store_error_is_not_a_missing_key() {
        assert!(!should_welcome(Unreadable, Saved, false));
        assert!(!should_welcome(Saved, Unreadable, false));
        assert!(!should_welcome(Unreadable, Unreadable, false));
    }

    #[test]
    fn forced_welcome_opens_with_keys() {
        for (soniox, anthropic) in [(Saved, Saved), (Unreadable, Unreadable), (Missing, Saved)] {
            assert!(should_welcome(soniox, anthropic, true), "{soniox:?} {anthropic:?}");
        }
    }

    /// At startup from `std::env::args_os`, in a second launch from the argv the running CUE gets: the program first.
    #[test]
    fn welcome_arg_is_found_in_argv() {
        assert!(asks_welcome(["cue.exe", "--welcome"]));
        assert!(asks_welcome(vec!["cue.exe".to_owned(), "--welcome".to_owned()]), "a second launch's argv");
        assert!(asks_welcome([OsString::from("cue.exe"), OsString::from("--welcome")]), "the process's own");
        assert!(!asks_welcome(["cue.exe"]));
        assert!(!asks_welcome(Vec::<String>::new()));
        for other in ["--Welcome", "welcome", "-welcome", "--welcome=1", " --welcome", "/welcome"] {
            assert!(!asks_welcome(["cue.exe", other]), "{other:?}");
        }
    }

    /// What the core was told of the gate, in order.
    fn gates(rx: &mut UnboundedReceiver<CoreEvent>) -> Vec<bool> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .map(|ev| match ev {
                CoreEvent::Input(Input::WelcomeGate(held)) => held,
                _ => panic!("not the welcome gate"),
            })
            .collect()
    }

    /// The hotkey is held from before the window is created; its end lets it go, once, and so does a creation that
    /// failed (the window's own microphone preview ends with it).
    #[test]
    fn gate_opens_when_the_window_is_destroyed_or_fails() {
        let (core, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let hold = Mutex::new(Some(Hold::new(core.clone())));
        assert_eq!(gates(&mut rx), [true], "held before the window is built");
        on_event(&hold, &WindowEvent::Focused(false));
        assert!(gates(&mut rx).is_empty(), "only its end lets it go");
        on_event(&hold, &WindowEvent::Destroyed);
        assert_eq!(gates(&mut rx), [false]);
        on_event(&hold, &WindowEvent::Destroyed);
        assert!(gates(&mut rx).is_empty(), "once");

        // Not created: the hold goes with the failed creation.
        let hold = Hold::new(core);
        assert_eq!(gates(&mut rx), [true]);
        drop(hold);
        assert_eq!(gates(&mut rx), [false]);
    }
}
