//! Core events → the Drop window, and window commands → the core (names as in `app/ui/src/main.ts`).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cue_core::ui::{TrayKind, UiAction, UiState};
use tauri::{AppHandle, Emitter, State};

use crate::runtime::CoreEvent;

/// Channel to the core for window commands.
pub struct CoreTx(pub tokio::sync::mpsc::UnboundedSender<CoreEvent>);

/// Sends events to the Drop window. No dictation text and no audio: only the state, the level and tray card kinds.
#[derive(Clone)]
pub struct Ui {
    app: AppHandle,
    /// The last value sent in `cue://idle-hidden`: a window loaded later (or reloaded) gets it via `drop_ready`.
    idle_hidden: Arc<AtomicBool>,
}

impl Ui {
    pub fn new(app: AppHandle) -> Self {
        Self { app, idle_hidden: Arc::new(AtomicBool::new(false)) }
    }

    /// For the tray icon (`tray::sync`).
    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    pub fn state(&self, s: &UiState) {
        self.emit("cue://state", s);
    }

    pub fn level(&self, v: f32) {
        self.emit("cue://level", v);
    }

    pub fn tray(&self, k: Option<TrayKind>) {
        self.emit("cue://tray", k);
    }

    pub fn shake(&self) {
        self.emit("cue://shake", ());
    }

    /// The idle bar hides over a full-screen app.
    pub fn idle_hidden(&self, hidden: bool) {
        self.idle_hidden.store(hidden, Ordering::Relaxed);
        self.emit("cue://idle-hidden", hidden);
    }

    /// Resend the last `idle_hidden` (the window has just subscribed to events).
    pub fn resend_idle_hidden(&self) {
        self.emit("cue://idle-hidden", self.idle_hidden.load(Ordering::Relaxed));
    }

    fn emit<S: serde::Serialize + Clone>(&self, event: &str, payload: S) {
        if let Err(e) = self.app.emit(event, payload) {
            tracing::debug!("Drop window: event {event} not sent: {e}");
        }
    }
}

#[tauri::command]
pub fn drop_action(action: UiAction, core: State<'_, CoreTx>) {
    let _ = core.0.send(CoreEvent::UiAction(action));
}

/// The window has subscribed to events: the core sends the current state, the tray card and `idle-hidden`.
#[tauri::command]
pub fn drop_ready(core: State<'_, CoreTx>) {
    let _ = core.0.send(CoreEvent::UiReady);
}
