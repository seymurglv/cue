//! The welcome window (spec 4a §1): hello, who cleans the text, the speech model, the microphone, a first dictation,
//! done. It opens at CUE's first start here — no settings.json yet, or one that never set the cleanup
//! (`first_start`) — and with `--welcome`, at a start and when a second launch brings it to the CUE already running.
//! CUE holds no key, so a missing one opens nothing. Like the main window (`main_window`) there is one at a time, created when opened and
//! destroyed when closed, never built on the main thread. Windows draws its frame, in the Windows theme, with the close
//! button alone; it takes the focus, so its first dictation is pasted into its own field.
//!
//! While it is open the hotkey starts nothing except on its first dictation's step (`Input::WelcomeGate`): it is held
//! from before the window is created and let go when the window is destroyed or could not be created (`Hold`); in
//! between its page moves it with its steps (`commands::welcome_step`). A chord it holds brings it forward
//! (`bring_forward`). Its microphone preview goes with it; the main window's stays.
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use cue_core::flow::Input;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tokio::sync::mpsc::UnboundedSender;

use crate::ipc::{CoreTx, WELCOME_LABEL};
use crate::main_window::{Creating, Open, decide};
use crate::runtime::CoreEvent;

/// Its size as the design draws it (logical px); it never changes.
const WIDTH: f64 = 560.0;
const HEIGHT: f64 = 430.0;
/// The argument that opens it.
const WELCOME_ARG: &str = "--welcome";

/// The window is being created: a second open meanwhile creates no second one.
static CREATING: Mutex<bool> = Mutex::new(false);

/// `--welcome` among a launch's arguments; CUE takes no others.
pub fn asks_welcome<S: AsRef<OsStr>>(args: impl IntoIterator<Item = S>) -> bool {
    args.into_iter().any(|a| a.as_ref() == WELCOME_ARG)
}

/// Whether this start is CUE's first here, read before anything is written: no settings.json (a fresh install), or one
/// that never set the cleanup (`settings::cleanup_set`) — never a missing key. A file that cannot be read is the
/// user's: no first start.
pub fn first_start(settings: &Path) -> bool {
    match std::fs::read(settings) {
        Ok(bytes) => !cue_core::settings::cleanup_set(&bytes),
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// The window opens at a start when asked for (`forced`, `--welcome`) or on the first start (`first`).
pub fn opens_at_start(first: bool, forced: bool) -> bool {
    forced || first
}

/// At startup: the window opens when asked for or on the first start.
pub fn open_at_start(app: &AppHandle, first: bool, forced: bool) {
    let opens = opens_at_start(first, forced);
    tracing::info!(first, forced, opens, "welcome window at startup");
    if opens {
        open(app);
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
    crate::webview_health::watch(&win);
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
    use std::ffi::OsString;
    use tokio::sync::mpsc::UnboundedReceiver;

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

    /// A settings.json of its own in the temporary folder (never the user's), written with `json` unless `None`.
    fn settings_file(name: &str, json: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-welcome-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        if let Some(json) = json {
            std::fs::write(&path, json).unwrap();
        }
        path
    }

    /// A fresh install has no settings.json: the window opens at its first start.
    #[test]
    fn welcome_opens_on_a_fresh_install() {
        let path = settings_file("fresh", None);
        assert!(first_start(&path));
        assert!(opens_at_start(first_start(&path), false));
        // Settings written by an earlier start of this version say where the cleanup runs: no window then.
        let path = settings_file("written", Some(r#"{"refine":{"where":"local","fallback":"raw"}}"#));
        assert!(!first_start(&path));
        assert!(!opens_at_start(first_start(&path), false));
        // Settings that never set the cleanup (only the microphone, say) are a first start still.
        let path = settings_file("unset", Some(r#"{"refine":{"context_chars":500},"audio":{"device":"USB Microphone"}}"#));
        assert!(first_start(&path));
    }

    /// The owner's case: settings of an earlier version, whose cleanup went to the cloud with a key CUE no longer holds
    /// — the window does not open for the missing key; the move to the user's agent is the core's.
    #[test]
    fn welcome_does_not_open_for_missing_key() {
        for json in [
            r#"{"refine":{"runs":"cloud","connection":"anthropic","style":"bold","context_chars":500}}"#,
            r#"{"refine":{"runs":"cloud","connection":"openrouter"},"ui":{"language":"ru"}}"#,
            r#"{"refine":{"model":"claude-haiku-4-5","context_chars":500}}"#,
            r#"{"refine":{"where":"agent","agent":"codex"}}"#,
        ] {
            let path = settings_file("upgrade", Some(json));
            assert!(!first_start(&path), "{json}");
            assert!(!opens_at_start(first_start(&path), false), "{json}");
        }
        // A file CUE cannot read is the user's: no first start, and it is not touched.
        let path = settings_file("corrupt", Some("{not json"));
        assert!(!first_start(&path));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{not json");
    }

    /// `--welcome` opens it whatever the settings say.
    #[test]
    fn welcome_opens_when_asked() {
        assert!(opens_at_start(false, true));
        assert!(opens_at_start(true, true));
        assert!(!opens_at_start(false, false));
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
