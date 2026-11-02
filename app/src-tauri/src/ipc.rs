//! Core events → the Drop and card windows, new history entries → the main window's channel, and window commands → the
//! core (names as in `app/ui/src/main.ts` and `app/ui/src/card.ts`). The Drop's and the card's commands answer those
//! windows alone (`guard`).
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cue_core::history;
use cue_core::settings::{AudioKeep, DropPosition, Keep, RefineStyle, Settings, UiLanguage};
use cue_core::translate;
use cue_core::ui::{TrayKind, UiAction, UiState};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State, WebviewWindow};

use crate::card_window::CARD_LABEL;
use crate::drop_window::DROP_LABEL;
use crate::lang::Lang;
use crate::runtime::CoreEvent;

/// The main window: the only one that gets history text — its commands answer it, and the welcome window only those
/// its steps need (`commands::guard`) — and new entries come over its own channel (`HistoryFeed`).
pub const MAIN_LABEL: &str = "main";
/// The welcome window (`welcome_window`).
pub const WELCOME_LABEL: &str = "welcome";

/// Channel to the core for window commands.
pub struct CoreTx(pub tokio::sync::mpsc::UnboundedSender<CoreEvent>);

/// The settings the main window shows (spec 3 §3), as the core holds them: the history's period (`None` — unknown, so
/// nothing is written to the history or deleted from it until the user picks one: settings.json was not read, or its
/// value is not one CUE knows), how long recordings are kept as the user chose it (`None` — unknown: none is recorded
/// until the user picks one), the pinned microphone (`None` — the one Windows uses), the translation language, the
/// interface language as set and as it is now (`lang`), the languages a translation can go to (code and English name),
/// and whether changes reach settings.json (`saved`: it was read at startup). `Debug` shows the microphone's name only
/// by its length.
#[derive(Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub history_keep: Option<Keep>,
    pub history_audio: Option<AudioKeep>,
    pub mic_device: Option<String>,
    pub translate_target: &'static str,
    pub ui_language: UiLanguage,
    pub lang: &'static str,
    pub languages: &'static [(&'static str, &'static str)],
    pub saved: bool,
    /// The dictionary's words as settings.json keeps them, the newest first.
    pub dictionary: Vec<String>,
    /// How freely refine edits.
    pub refine_style: RefineStyle,
}

impl SettingsView {
    pub fn of(s: &Settings, lang: Lang, saved: bool) -> Self {
        Self {
            history_keep: s.history.keep.known(),
            history_audio: s.history.audio.known(),
            mic_device: s.audio.device.clone(),
            translate_target: s.translate.target(),
            ui_language: s.ui.language,
            lang: lang.code(),
            languages: translate::LANGUAGES,
            saved,
            dictionary: s.dictionary.terms.clone(),
            refine_style: s.refine.style,
        }
    }
}

impl std::fmt::Debug for SettingsView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsView")
            .field("history_keep", &self.history_keep)
            .field("history_audio", &self.history_audio)
            .field("mic_device_chars", &self.mic_device.as_ref().map(|d| d.chars().count()))
            .field("translate_target", &self.translate_target)
            .field("ui_language", &self.ui_language)
            .field("lang", &self.lang)
            .field("languages", &self.languages.len())
            .field("saved", &self.saved)
            .field("dictionary", &self.dictionary.len())
            .field("refine_style", &self.refine_style)
            .finish()
    }
}

/// One history entry as the main window draws it (spec 3 §2): its time of day `HH:MM`, the mode, the app (empty when
/// not known), the text, an "Ask" question, the text as it was heard before CUE cleaned it up (`raw`: a dictation's, a
/// translation's speech; `None` for "Ask" and for entries of earlier versions), and whether its recording is there to
/// play (`has_audio`: false for a new entry, whose recording is still being encoded — the window asks when the entry
/// opens). Only the main window gets it; `Debug` shows the lengths of the texts, never them.
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryView {
    pub id: String,
    pub time: String,
    pub mode: history::Mode,
    pub app: String,
    pub text: String,
    pub question: Option<String>,
    pub raw: Option<String>,
    pub has_audio: bool,
}

impl EntryView {
    pub fn of(e: &history::Entry, has_audio: bool) -> Self {
        Self {
            id: e.id.clone(),
            // `YYYY-MM-DDTHH:MM:SS.mmm`: hours and minutes.
            time: e.ts.get(11..16).unwrap_or_default().to_owned(),
            mode: e.mode,
            app: e.app.clone(),
            text: e.text.clone(),
            question: e.question.clone(),
            raw: e.raw.clone(),
            has_audio,
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
            .field("raw_chars", &self.raw.as_ref().map(|r| r.chars().count()))
            .field("has_audio", &self.has_audio)
            .finish()
    }
}

/// One day of the history as the main window draws it: its date `YYYY-MM-DD` and its entries, newest first. `Debug` is
/// the entries' own: lengths, never text.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct DayView {
    pub date: String,
    pub entries: Vec<EntryView>,
}

impl DayView {
    /// `has_audio`: whether the entry of this id has a recording to play.
    pub fn of(d: &history::Day, has_audio: &impl Fn(&str) -> bool) -> Self {
        let entries = d.entries.iter().map(|e| EntryView::of(e, has_audio(&e.id))).collect();
        Self { date: d.date.clone(), entries }
    }
}

/// Days of the history, newest first, and whether an earlier day has entries too ("Show earlier").
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PageView {
    pub days: Vec<DayView>,
    pub more: bool,
}

impl PageView {
    /// `has_audio`: whether the entry of this id has a recording to play.
    pub fn of(p: &history::Page, has_audio: &impl Fn(&str) -> bool) -> Self {
        Self { days: p.days.iter().map(|d| DayView::of(d, has_audio)).collect(), more: p.more }
    }
}

/// New history entries for the main window (spec 3 §2): a channel its page opened (`commands::history_subscribe`). A
/// channel reaches only the webview that made it, while an event reaches any page that listens to its name, whatever
/// window it was sent to: so no other window ever gets history text. One at a time: a new subscription (the page loaded
/// again) replaces the old one; it goes when the window is destroyed or a send fails.
#[derive(Clone, Default)]
pub struct HistoryFeed(Arc<Mutex<Option<Channel<EntryView>>>>);

impl HistoryFeed {
    pub fn follow(&self, channel: Channel<EntryView>) {
        *self.slot() = Some(channel);
    }

    pub fn stop(&self) {
        *self.slot() = None;
    }

    /// The entry to the main window, if it follows. Sent outside the lock; a failed send ends that subscription only.
    fn send(&self, entry: &EntryView) {
        let Some(channel) = self.slot().clone() else { return };
        if let Err(e) = channel.send(entry.clone()) {
            tracing::debug!("main window: history entry not sent: {e}");
            let mut slot = self.slot();
            if slot.as_ref().is_some_and(|now| now.id() == channel.id()) {
                *slot = None;
            }
        }
    }

    fn slot(&self) -> MutexGuard<'_, Option<Channel<EntryView>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
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
/// and no audio; an "Ask" answer goes to the windows that draw it (`answer`), a missing microphone's names to the Drop
/// (`mic_names`).
#[derive(Clone)]
pub struct Ui {
    app: AppHandle,
    /// The last value sent in `cue://idle-hidden`: a window loaded later (or reloaded) gets it via `drop_ready`.
    idle_hidden: Arc<AtomicBool>,
    /// The last place sent in `cue://place`, as an index into `DropPosition::ALL`; resent on `drop_ready` too.
    place: Arc<AtomicUsize>,
    /// The last card place, resent on `card_ready`.
    card: Arc<std::sync::Mutex<Option<CardPlaceView>>>,
    /// The main window's channel for new history entries.
    feed: HistoryFeed,
}

impl Ui {
    pub fn new(app: AppHandle, place: DropPosition) -> Self {
        Self {
            app,
            idle_hidden: Arc::new(AtomicBool::new(false)),
            place: Arc::new(AtomicUsize::new(place_index(place))),
            card: Default::default(),
            feed: HistoryFeed::default(),
        }
    }

    /// The main window's channel, which its commands set and its end clears.
    pub fn history_feed(&self) -> HistoryFeed {
        self.feed.clone()
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

    /// The pinned microphone that is not there and the one recording instead (spec 3 §3), just before their card: to
    /// the Drop, which draws it.
    pub fn mic_names(&self, wanted: &str, used: &str) {
        #[derive(Clone, serde::Serialize)]
        struct MicNames<'a> {
            wanted: &'a str,
            used: &'a str,
        }
        if let Err(e) = self.app.emit_to(DROP_LABEL, "cue://mic-names", MicNames { wanted, used }) {
            tracing::debug!("Drop window: event cue://mic-names not sent: {e}");
        }
    }

    /// The speech model's state, to every window: the Drop's error shows its bytes, the main and welcome windows its
    /// progress.
    pub fn model(&self, state: cue_core::model::ModelState) {
        self.emit("cue://model", state);
    }

    /// The Model page's list, to every window (only the main one draws it).
    pub fn models(&self, view: &cue_core::model::ModelsView) {
        self.emit("cue://models", view);
    }

    /// The name of the model just switched to, just before its card: to the Drop, which draws it.
    pub fn model_name(&self, name: &str) {
        if let Err(e) = self.app.emit_to(DROP_LABEL, "cue://model-name", name) {
            tracing::debug!("Drop window: event cue://model-name not sent: {e}");
        }
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

    /// A new history entry (spec 3 §2), to the main window alone, over its channel (`HistoryFeed`): never an event,
    /// which the Drop's or the card's page could listen to. With no main window open nobody gets it.
    pub fn history_added(&self, entry: &EntryView) {
        self.feed.send(entry);
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

/// Every Drop and card command starts here, with its name: `drop_action` and `answer_source` answer the Drop's page
/// and the card's, the others their own window's alone, and none the main or the welcome window's (they have their
/// own commands, `commands`). A refused command does nothing.
pub fn guard(window: &WebviewWindow, command: &str) -> bool {
    let answered = guard_label(window.label(), command);
    if !answered {
        tracing::warn!(window = window.label(), command, "window command refused");
    }
    answered
}

fn guard_label(label: &str, command: &str) -> bool {
    match command {
        "drop_action" | "answer_source" => label == DROP_LABEL || label == CARD_LABEL,
        "drop_ready" | "drop_drag" | "drop_hit_rects" => label == DROP_LABEL,
        "card_ready" | "card_hit_rects" => label == CARD_LABEL,
        _ => false,
    }
}

#[tauri::command]
pub fn drop_action(window: WebviewWindow, action: UiAction, core: State<'_, CoreTx>) {
    if guard(&window, "drop_action") {
        let _ = core.0.send(CoreEvent::UiAction(action));
    }
}

/// A source chip of the answer was clicked: the core opens its page.
#[tauri::command]
pub fn answer_source(window: WebviewWindow, n: u32, core: State<'_, CoreTx>) {
    if guard(&window, "answer_source") {
        let _ = core.0.send(CoreEvent::AnswerSource(n));
    }
}

fn place_index(place: DropPosition) -> usize {
    DropPosition::ALL.iter().position(|&p| p == place).unwrap_or(0)
}

/// The window has subscribed to events: the core sends the current state, the tray card, `idle-hidden` and the place.
#[tauri::command]
pub fn drop_ready(window: WebviewWindow, core: State<'_, CoreTx>) {
    if guard(&window, "drop_ready") {
        let _ = core.0.send(CoreEvent::UiReady);
    }
}

/// The card window has subscribed to events: the core sends the current state, the card's place and the open answer.
#[tauri::command]
pub fn card_ready(window: WebviewWindow, core: State<'_, CoreTx>) {
    if guard(&window, "card_ready") {
        let _ = core.0.send(CoreEvent::CardReady);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::settings::{Period, SettingChange};
    use serde_json::{Value, json};

    /// The names the main window reads.
    #[test]
    fn settings_view_json_names() {
        let mut s = Settings::default();
        let view = serde_json::to_value(SettingsView::of(&s, Lang::En, true)).unwrap();
        let mut names: Vec<&str> = view.as_object().unwrap().keys().map(String::as_str).collect();
        names.sort_unstable();
        let expected =
            ["dictionary", "historyAudio", "historyKeep", "lang", "languages", "micDevice", "refineStyle", "saved", "translateTarget", "uiLanguage"];
        assert_eq!(names, expected);
        let defaults = [
            ("historyKeep", json!("month")),
            ("historyAudio", json!("week")),
            ("micDevice", Value::Null),
            ("translateTarget", json!("en")),
            ("uiLanguage", json!("auto")),
            ("lang", json!("en")),
            ("saved", json!(true)),
            ("dictionary", json!([])),
            ("refineStyle", json!("careful")),
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
            SettingChange::HistoryAudio(AudioKeep::Day),
            SettingChange::MicDevice(Some("USB Microphone".into())),
            SettingChange::TranslateTarget("de".into()),
            SettingChange::UiLanguage(UiLanguage::Ru),
            SettingChange::RefineStyle(RefineStyle::Bold),
        ] {
            assert!(s.apply(change));
        }
        let view = serde_json::to_value(SettingsView::of(&s, Lang::Ru, false)).unwrap();
        let set = [
            ("historyKeep", json!("off")),
            ("historyAudio", json!("day")),
            ("micDevice", json!("USB Microphone")),
            ("translateTarget", json!("de")),
            ("uiLanguage", json!("ru")),
            ("lang", json!("ru")),
            ("saved", json!(false)),
            ("refineStyle", json!("bold")),
        ];
        for (name, value) in set {
            assert_eq!(view[name], value, "{name}");
        }
        assert!(!format!("{:?}", SettingsView::of(&s, Lang::Ru, false)).contains("USB"), "no device name in a log");

        // A period CUE does not know shows unset.
        s.history.keep = Period::Unknown(json!("forver"));
        s.history.audio = Period::Unknown(json!("fortnight"));
        let view = serde_json::to_value(SettingsView::of(&s, Lang::Ru, false)).unwrap();
        assert_eq!((&view["historyKeep"], &view["historyAudio"]), (&Value::Null, &Value::Null));
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
            raw: None,
        };
        let view = EntryView::of(&e, false);
        let expected = json!({
            "id": "2026-10-31T07:05:09.123-4",
            "time": "07:05",
            "mode": "ask",
            "app": "Notepad",
            "text": "private answer",
            "question": "secret question",
            "raw": null,
            "hasAudio": false,
        });
        assert_eq!(serde_json::to_value(&view).unwrap(), expected);
        let shown = format!("{view:?}");
        assert!(!shown.contains("private") && !shown.contains("secret"), "{shown}");
        let dictation = history::Entry { mode: history::Mode::Dictation, question: None, ..e };
        assert_eq!(serde_json::to_value(EntryView::of(&dictation, false)).unwrap()["question"], Value::Null);
    }

    /// A dictation's text as it was heard goes to the main window with it, and whether its recording plays; a log
    /// shows only its length.
    #[test]
    fn entry_view_has_the_text_as_heard_and_the_recording() {
        let e = history::Entry {
            id: "2026-10-31T07:05:09.123-5".into(),
            ts: "2026-10-31T07:05:09.123".into(),
            mode: history::Mode::Dictation,
            app: String::new(),
            text: "Clean words.".into(),
            question: None,
            raw: Some("uh secret words".into()),
        };
        let view = serde_json::to_value(EntryView::of(&e, true)).unwrap();
        assert_eq!((&view["raw"], &view["hasAudio"]), (&json!("uh secret words"), &json!(true)));
        let shown = format!("{:?}", EntryView::of(&e, true));
        assert!(!shown.contains("secret") && !shown.contains("Clean"), "{shown}");
        assert!(shown.contains("raw_chars: Some(15)") && shown.contains("has_audio: true"), "{shown}");

        // A page asks per entry whether its recording plays.
        let entries = vec![e.clone(), history::Entry { id: "x".into(), ..e }];
        let day = history::Day { date: "2026-10-31".into(), entries };
        let page = PageView::of(&history::Page { days: vec![day], more: true }, &|id: &str| id != "x");
        let played: Vec<bool> = page.days[0].entries.iter().map(|e| e.has_audio).collect();
        assert_eq!((played, page.more), (vec![true, false], true));
    }

    /// The Drop's and the card's commands answer their own pages: the main and welcome windows', which show text of
    /// their own, never drive the Drop or the card.
    #[test]
    fn drop_and_card_commands_answer_their_windows_alone() {
        let drop = ["drop_action", "answer_source", "drop_ready", "drop_drag", "drop_hit_rects"];
        let card = ["drop_action", "answer_source", "card_ready", "card_hit_rects"];
        for &command in drop.iter().chain(&card) {
            assert_eq!(guard_label(DROP_LABEL, command), drop.contains(&command), "drop {command}");
            assert_eq!(guard_label(CARD_LABEL, command), card.contains(&command), "card {command}");
            for label in [MAIN_LABEL, WELCOME_LABEL, "Drop", "drop ", "card ", ""] {
                assert!(!guard_label(label, command), "{label:?} {command}");
            }
        }
        assert!(!guard_label(DROP_LABEL, "settings_get") && !guard_label(CARD_LABEL, ""));
    }

    /// A channel that keeps the ids of the entries it gets, or refuses them all (a page that is gone).
    fn channel(works: bool) -> (Channel<EntryView>, Arc<Mutex<Vec<String>>>) {
        let got = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&got);
        let channel = Channel::new(move |body| {
            let tauri::ipc::InvokeResponseBody::Json(json) = body else { panic!("not JSON") };
            let id = serde_json::from_str::<Value>(&json).unwrap()["id"].as_str().unwrap().to_owned();
            seen.lock().unwrap().push(id);
            if works { Ok(()) } else { Err(tauri::Error::WebviewNotFound) }
        });
        (channel, got)
    }

    fn view(id: &str) -> EntryView {
        let ts = "2026-10-31T07:05:09.123";
        let e = history::Entry {
            id: id.into(),
            ts: ts.into(),
            mode: history::Mode::Dictation,
            app: String::new(),
            text: "Hello.".into(),
            question: None,
            raw: None,
        };
        EntryView::of(&e, false)
    }

    /// New entries go only to the channel the main window opened last; none goes once it has ended.
    #[test]
    fn history_entries_go_to_the_main_windows_channel_alone() {
        let feed = HistoryFeed::default();
        feed.send(&view("a"));

        let (first, to_first) = channel(true);
        feed.follow(first);
        feed.send(&view("b"));
        let (second, to_second) = channel(true);
        feed.follow(second);
        feed.send(&view("c"));
        assert_eq!(*to_first.lock().unwrap(), ["b"], "a page loaded again replaces its channel");
        assert_eq!(*to_second.lock().unwrap(), ["c"]);

        feed.stop();
        feed.send(&view("d"));
        assert_eq!(*to_second.lock().unwrap(), ["c"], "the window was destroyed");

        let (gone, to_gone) = channel(false);
        feed.follow(gone);
        feed.send(&view("e"));
        feed.send(&view("f"));
        assert_eq!(*to_gone.lock().unwrap(), ["e"], "a failed send ends the subscription");
    }
}
