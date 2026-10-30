//! Core events → the Drop, card and main windows, and window commands → the core (names as in `app/ui/src/main.ts` and
//! `app/ui/src/card.ts`).
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use cue_core::history;
use cue_core::settings::{DropPosition, Keep, Settings, UiLanguage};
use cue_core::translate;
use cue_core::ui::{TrayKind, UiAction, UiState};
use tauri::{AppHandle, Emitter, State};

use crate::lang::Lang;
use crate::runtime::CoreEvent;

/// The main window: the only one that gets history text and settings.
pub const MAIN_LABEL: &str = "main";

/// Channel to the core for window commands.
pub struct CoreTx(pub tokio::sync::mpsc::UnboundedSender<CoreEvent>);

/// The settings the main window shows (spec 3 §3), as the core holds them: the history's period, the pinned microphone
/// (`None` — the one Windows uses), the translation language, the interface language as set and as it is now (`lang`),
/// the languages a translation can go to (code and English name), and whether changes reach settings.json (`saved`:
/// it was read at startup). `Debug` shows the microphone's name only by its length.
#[derive(Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub history_keep: Keep,
    pub mic_device: Option<String>,
    pub translate_target: &'static str,
    pub ui_language: UiLanguage,
    pub lang: &'static str,
    pub languages: &'static [(&'static str, &'static str)],
    pub saved: bool,
}

impl SettingsView {
    pub fn of(s: &Settings, lang: Lang, saved: bool) -> Self {
        Self {
            history_keep: s.history.keep,
            mic_device: s.audio.device.clone(),
            translate_target: s.translate.target(),
            ui_language: s.ui.language,
            lang: lang.code(),
            languages: translate::LANGUAGES,
            saved,
        }
    }
}

impl std::fmt::Debug for SettingsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsView")
            .field("history_keep", &self.history_keep)
            .field("mic_device_chars", &self.mic_device.as_ref().map(|d| d.chars().count()))
            .field("translate_target", &self.translate_target)
            .field("ui_language", &self.ui_language)
            .field("lang", &self.lang)
            .field("languages", &self.languages.len())
            .field("saved", &self.saved)
            .finish()
    }
}

/// One history entry as the main window draws it (spec 3 §2): its time of day `HH:MM`, the mode, the app (empty when
/// not known), the text and an "Ask" question. Only the main window gets it; `Debug` shows the lengths of the text and
/// the question, never them.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: String,
    pub time: String,
    pub mode: history::Mode,
    pub app: String,
    pub text: String,
    pub question: Option<String>,
}

impl From<&history::Entry> for EntryView {
    fn from(e: &history::Entry) -> Self {
        Self {
            id: e.id.clone(),
            // `YYYY-MM-DDTHH:MM:SS.mmm`: hours and minutes.
            time: e.ts.get(11..16).unwrap_or_default().to_owned(),
            mode: e.mode,
            app: e.app.clone(),
            text: e.text.clone(),
            question: e.question.clone(),
        }
    }
}

impl std::fmt::Debug for EntryView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntryView")
            .field("id", &self.id)
            .field("time", &self.time)
            .field("mode", &self.mode)
            .field("app", &self.app)
            .field("text_chars", &self.text.chars().count())
            .field("question_chars", &self.question.as_ref().map(|q| q.chars().count()))
            .finish()
    }
}

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
        self.emit("cue://place", self.current_place().code());
    }

    /// The Drop's place as last sent.
    fn current_place(&self) -> DropPosition {
        DropPosition::ALL.get(self.place.load(Ordering::Relaxed)).copied().unwrap_or_default()
    }

    /// A new history entry (spec 3 §2), to the main window alone: no other window ever gets history text. With no main
    /// window open nobody gets it.
    pub fn history_added(&self, entry: &EntryView) {
        if let Err(e) = self.app.emit_to(MAIN_LABEL, "cue://history", entry) {
            tracing::debug!("main window: event cue://history not sent: {e}");
        }
    }

    /// The interface language changed while CUE runs (decision 5): the tray menu takes the new labels, and the Drop and
    /// the card load their pages again in it — once loaded, each says it is ready and gets its state back. On the main
    /// thread, like every menu and window change; the Drop's place is read there, so a drag just before still counts.
    pub fn relang(&self, lang: Lang) {
        let ui = self.clone();
        let posted = self.app.run_on_main_thread(move || {
            crate::tray::relabel(&ui.app, lang);
            crate::drop_window::reload(&ui.app, lang, ui.current_place());
            crate::card_window::reload(&ui.app, lang);
        });
        if let Err(e) = posted {
            // The app is closing.
            tracing::debug!("interface language: change not posted: {e}");
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::settings::SettingChange;
    use serde_json::{Value, json};

    /// The names the main window reads.
    #[test]
    fn settings_view_json_names() {
        let mut s = Settings::default();
        let view = serde_json::to_value(SettingsView::of(&s, Lang::En, true)).unwrap();
        let mut names: Vec<&str> = view.as_object().unwrap().keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["historyKeep", "lang", "languages", "micDevice", "saved", "translateTarget", "uiLanguage"]);
        let defaults = [
            ("historyKeep", json!("month")),
            ("micDevice", Value::Null),
            ("translateTarget", json!("en")),
            ("uiLanguage", json!("auto")),
            ("lang", json!("en")),
            ("saved", json!(true)),
        ];
        for (name, value) in defaults {
            assert_eq!(view[name], value, "{name}");
        }
        let languages = view["languages"].as_array().unwrap();
        assert_eq!(languages.len(), 60);
        assert_eq!(languages[0], json!(["af", "Afrikaans"]));
        assert!(languages.contains(&json!(["ru", "Russian"])));

        for change in [
            SettingChange::HistoryKeep(Keep::Off),
            SettingChange::MicDevice(Some("USB Microphone".into())),
            SettingChange::TranslateTarget("de".into()),
            SettingChange::UiLanguage(UiLanguage::Ru),
        ] {
            assert!(s.apply(change));
        }
        let view = serde_json::to_value(SettingsView::of(&s, Lang::Ru, false)).unwrap();
        let set = [
            ("historyKeep", json!("off")),
            ("micDevice", json!("USB Microphone")),
            ("translateTarget", json!("de")),
            ("uiLanguage", json!("ru")),
            ("lang", json!("ru")),
            ("saved", json!(false)),
        ];
        for (name, value) in set {
            assert_eq!(view[name], value, "{name}");
        }
        assert!(!format!("{:?}", SettingsView::of(&s, Lang::Ru, false)).contains("USB"), "no device name in a log");
    }

    #[test]
    fn entry_view_has_the_time_of_day() {
        let e = history::Entry {
            id: "2026-10-31T07:05:09.123-4".into(),
            ts: "2026-10-31T07:05:09.123".into(),
            mode: history::Mode::Ask,
            app: "Notepad".into(),
            text: "private answer".into(),
            question: Some("secret question".into()),
        };
        let view = EntryView::from(&e);
        let expected = json!({
            "id": "2026-10-31T07:05:09.123-4",
            "time": "07:05",
            "mode": "ask",
            "app": "Notepad",
            "text": "private answer",
            "question": "secret question",
        });
        assert_eq!(serde_json::to_value(&view).unwrap(), expected);
        let shown = format!("{view:?}");
        assert!(!shown.contains("private") && !shown.contains("secret"), "{shown}");
        let dictation = history::Entry { mode: history::Mode::Dictation, question: None, ..e };
        assert_eq!(serde_json::to_value(EntryView::from(&dictation)).unwrap()["question"], Value::Null);
    }
}
