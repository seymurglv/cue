//! Core events → the Drop and card windows, and window commands → the core (names as in `app/ui/src/main.ts` and
//! `app/ui/src/card.ts`).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cue_core::settings::DropPosition;
use cue_core::ui::{TrayKind, UiAction, UiState};
use tauri::{AppHandle, Emitter, State};

use crate::runtime::CoreEvent;

/// Channel to the core for window commands.
pub struct CoreTx(pub tokio::sync::mpsc::UnboundedSender<CoreEvent>);

/// Where the card's arrow points and how tall the card may grow (spec 2 §7). Positions only, no text.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardPlaceView {
    pub below: bool,
    pub arrow_x: f64,
    pub max_height: f64,
}

/// Sends events to the windows: the state, the level, tray card kinds, the Drop's place and the card's. No dictation text
/// and no audio; an "Ask" answer goes to the windows that draw it (`answer`).
#[derive(Clone)]
pub struct Ui {
    app: AppHandle,
    /// The last value sent in `cue://idle-hidden`: a window loaded later (or reloaded) gets it via `drop_ready`.
    idle_hidden: Arc<AtomicBool>,
    /// The last place sent in `cue://place`, as an index into `DropPosition::ALL`; resent on `drop_ready` too.
    place: Arc<AtomicUsize>,
    /// The last card place, resent on `card_ready`.
    card: Arc<std::sync::Mutex<Option<CardPlaceView>>>,
}

impl Ui {
    pub fn new(app: AppHandle, place: DropPosition) -> Self {
        Self {
            app,
            idle_hidden: Arc::new(AtomicBool::new(false)),
            place: Arc::new(AtomicUsize::new(place_index(place))),
            card: Default::default(),
        }
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

    /// The "Ask" answer (spec 2 §7), `None` once it closed. The windows get the question, the answer and the sources'
    /// sites and titles: they draw them. Never the URLs: only the core opens a source.
    pub fn answer(&self, view: Option<&cue_core::ask::AnswerView>) {
        self.emit("cue://answer", view);
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

    /// The Drop's place: the layout follows it. Sent after every drag, also when the place stayed the same: it ends the
    /// window's dragging state.
    pub fn place(&self, place: DropPosition) {
        self.place.store(place_index(place), Ordering::Relaxed);
        self.emit("cue://place", place.code());
    }

    /// Resend the last place (the window has just subscribed to events).
    pub fn resend_place(&self) {
        let place = DropPosition::ALL.get(self.place.load(Ordering::Relaxed)).copied().unwrap_or_default();
        self.emit("cue://place", place.code());
    }

    /// The card's side, arrow and height: sent before the card window shows.
    pub fn card_place(&self, place: CardPlaceView) {
        *self.card.lock().unwrap() = Some(place);
        self.emit("cue://card-place", place);
    }

    /// Resend the last card place (the card window has just subscribed to events).
    pub fn resend_card_place(&self) {
        let place = *self.card.lock().unwrap();
        if let Some(place) = place {
            self.emit("cue://card-place", place);
        }
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

/// A source chip of the answer was clicked: the core opens its page.
#[tauri::command]
pub fn answer_source(n: u32, core: State<'_, CoreTx>) {
    let _ = core.0.send(CoreEvent::AnswerSource(n));
}

fn place_index(place: DropPosition) -> usize {
    DropPosition::ALL.iter().position(|&p| p == place).unwrap_or(0)
}

/// The window has subscribed to events: the core sends the current state, the tray card, `idle-hidden` and the place.
#[tauri::command]
pub fn drop_ready(core: State<'_, CoreTx>) {
    let _ = core.0.send(CoreEvent::UiReady);
}

/// The card window has subscribed to events: the core sends the current state, the card's place and the open answer.
#[tauri::command]
pub fn card_ready(core: State<'_, CoreTx>) {
    let _ = core.0.send(CoreEvent::CardReady);
}
