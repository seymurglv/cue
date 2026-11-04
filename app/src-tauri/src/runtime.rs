//! The app core, an actor. Events (keys, the Drop window, microphone, engines, timers, power) → the `Flow`
//! state machine → effects → execution. Only `Flow` makes decisions; here live the microphone, transcription,
//! refine, insert, timers, the window and the tray icon, the last-dictation files and usage accounting.
//!
//! Executor rules:
//! - timers: each kind has its own generation (`state::Timers`), a stale firing never reaches `Flow`;
//! - microphone frames are tagged with the dictation id and accepted only until its `StopMic` (`state::MicGate`);
//!   stopping the microphone itself (waiting for its thread) happens off the core thread;
//! - a pinned microphone (spec 3 §3) is looked for on the microphone's own thread, the core never waits; when it is not
//!   there the recording goes on with the Windows one, and the user hears it once per loss (`state::MicTold`): its
//!   names go to the Drop alone, never to the log;
//! - the transcription result (`SttFinal` / `SttFailed`) goes to `Flow` only after `FinishStt` / `TranscribeSaved`;
//!   each transcription and refine task has its own generation (`state::TaskSlot`): the result of a cancelled or
//!   replaced task is not accepted; a panic inside a task still yields a result (`tasks::guarded`);
//! - an insert always ends: the insert thread is time-bounded, and on top of that there is a 10 s watchdog
//!   (`state::InsertWatch`);
//! - "retry" is a new attempt of the same dictation: `Finished` arrives twice, each attempt gets its own usage.csv
//!   row with its own outcome; a retry resets the tokens, latencies and transcription path, the audio and the app
//!   stay (`attempt`). After a failed translation "retry" refines the saved text again, and its row has no audio;
//! - the last-dictation files (`last\`) change only on success (`files`); the last text is also kept in memory;
//! - a prompt cache warm-up (refine's `warm_cache`, "Ask"'s `warm_ask_cache`) is paid for and gets a usage.csv row of
//!   its own (`warmup`);
//! - "Ask" (spec 2 §6–7): the context is taken at the chord, on the focus thread; the request streams in a task of its
//!   own (`ask_task`, with a generation like the others); the answer view goes to the windows at once for a new turn,
//!   an end or an opening, and at most every 50 ms while words stream (`state::ViewGate`); the flow hears once per
//!   request that its answer shows (`state::ShownOnce`). The attempt of an answer still printing when a follow-up
//!   starts waits aside (`answering`) and gets its own row when it ends;
//! - every result goes to the history (spec 3 §2, `HistoryLog`): a dictation or a translation when its text is
//!   committed, an "Ask" turn when it ends; never in the way of the result: a disk error is one log line, and nothing
//!   is written while the history is off. While its period is unknown (settings.json not read, or a value CUE does not
//!   know) the history is neither written nor cleaned up, until the main window sets it. The new entry goes to the main
//!   window only. A dictation's or a translation's entry keeps the text as it was recognized (`raw_text`), and each
//!   entry its recording, for the period the user chose (`recordings`): the sound is taken on the core thread — a
//!   dictation's only once `last\audio.wav` really is its own and the entry is written, an "Ask" question's as the
//!   question goes out, while `current.wav` is its own recording (a follow-up may start recording before its answer
//!   ends), kept with the turn and given to its entry when it ends — and encoded on a worker of its own. Only an
//!   attempt's own recording ever becomes its sound (`wav_of`);
//! - the main window reads and changes the settings through the core (`CoreEvent::Settings`): the core's copy takes the
//!   change, the whole file is written, and what depends on it follows at once (the translation pair, the interface
//!   language, the history's period); its "Paste again" and "Copy" of a history entry go through the insert thread like
//!   any paste (`CoreEvent::HistoryText`);
//! - a dictation's or a translation's paste is watched while "Learn from me" is on (docs/specs/2026-11-02-corrections.md):
//!   the field focused once Ctrl+V went in is read on a thread of its own (`fieldwatch`) until the correction is over,
//!   a new dictation opens the microphone or the switch goes off; a word the user wrote instead becomes the one
//!   suggestion (`Offers`), shown on the Drop at rest. The field's text stays in memory: the log gets numbers only;
//! - a dictation's correction also counts for habits (docs/specs/2026-11-04-habits.md): a form corrected three times in
//!   one app becomes a habit, its rule asked of the cleanup's connection (5 s, else a template), its card shown at rest
//!   after the suggestion found before it; the habits of the dictation's app go with its cleanup, and those whose effect
//!   shows are kept on its history entry; a correction that goes back on a habit three times changes it, and its card
//!   says so;
//! - the log gets events, per-stage latencies and error codes; no text, no audio, no keys. Never `Effect` or `Input`
//!   with `{:?}`: their `Debug` prints text.
mod ask;
mod attempt;
mod files;
mod insert;
pub(crate) mod llm;
mod model;
pub(crate) mod recordings;
mod state;
mod tasks;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cue_core::ask::{AnswerPlace, AskContext, MAX_SELECTION_CHARS, SelectionKind, TurnState};
use cue_core::audio::{Silence, SilenceDetector};
use cue_core::corrections::{self, Offers};
use cue_core::flow::{DictationId, Effect, Flow, InsertOutcome, Input, Outcome, PasteHow, RefineOutcome, SttError, TimerKind};
use cue_core::habits;
use cue_core::history;
use cue_core::model::{ModelState, ModelsView};
use cue_core::refine::RefineContext;
use cue_core::stats;
use cue_core::settings::{self, AudioKeep, DropPosition, HistorySettings, Keep, LearnedWord, Period, SettingChange, Settings};
use cue_core::speech;
use cue_core::translate;
use cue_core::ui::{Mode, TrayKind, UiAction, UiPhase, UiState};
use cue_core::usage::{self, SttPath, UsageRecord};
use cue_engines::EngineError;
use cue_engines::ask as engine_ask;
use cue_engines::claude::{self, Job, RefineResult};
use cue_engines::habit;
use cue_engines::hub;
use cue_engines::local;
use cue_engines::local_llm::Generate;
use cue_engines::{chat, openrouter};
use cue_core::connections::Kind;
use cue_engines::keys::{self, KeyName};
use cue_engines::stt::{self, Engine, SttOutput, SttSession};
use cue_platform_win::cardplace::{self, CARD};
use cue_platform_win::chord::HotkeyEvent;
use cue_platform_win::clipboard::{WinKeys, cloud_clipboard_uploads, may_paste};
use cue_platform_win::fieldwatch;
use cue_platform_win::focus::{Editability, FocusInfo, FocusWorker, SelectionRead, WindowRef};
use cue_platform_win::hook;
use cue_platform_win::mic::{self, MicEvent, MicHandle};
use cue_platform_win::selection::{copies_line, needs_copy_fallback};
use cue_platform_win::terminal::terminal_app;
use cue_platform_win::wav::WavSink;
use cue_platform_win::{placement, power, selection};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::card_window::Card;
use crate::ipc::{CardPlaceView, EntryView, SettingsView, Ui};
use crate::lang::{self, Lang};
use crate::paths::Paths;
use crate::trayicon::TrayLook;
use attempt::{Attempt, UsageLog, log_finished, needs_row, warmup_record};
use files::{LastFiles, Sound};
use insert::{InsertJob, first_known_app, insert_app, known_app, one_line, pasted_text, trim_edges};
use recordings::{AudioLimit, Recordings, Snapshot};
use state::{
    CacheWarm, InsertWatch, LevelGate, MicGate, MicTold, ShownOnce, TaskSlot, Timers, ViewGate, ViewSend, WarmUp,
};
pub(crate) use model::Choice;
use model::{ModelKeeper, ModelNews, Told};
use tasks::{ask_panicked, guarded, refine_panicked, stt_panicked};

/// Timeout for each UI Automation query.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(150);
/// The focus query of an "Ask" question (spec 2 §6.1): the selection of a long document and its rectangles take longer
/// than the text before the caret.
const ASK_FOCUS_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the request task waits for the Ctrl+C fallback (keys up ≤ 1.5 s, the app's copy ≤ 0.4 s): the core never
/// waits.
const COPY_WAIT: Duration = Duration::from_secs(2);
/// Text before the caret on insert: the join (`join_for_insert`) needs only its end. A short read is a
/// quick query that is rarely late (a late one answers "unknown").
const INSERT_CONTEXT_CHARS: usize = 16;
/// Insert watchdog: the insert thread finishes within ~7 s (see `insert::spawn_inserter`).
const INSERT_WATCHDOG: Duration = Duration::from_secs(10);
/// A tray card whose button shows a done label ("Copied", "Removed") keeps it for 1100 ms, then folds in 430 ms
/// (controller.ts `DONE_HOLD_MS`, the tray's fold): the Drop is not at rest until then.
const TRAY_DONE_MS: u64 = 1100 + 430;
/// Warm-up: a request with no key and no text, just for a TLS connection in the shared client's pool. The response
/// (a refusal) is not needed. The API address, not the site root: the root could redirect to another host.
const WARM_URL: &str = claude::API_URL;
const WARM_TIMEOUT: Duration = Duration::from_secs(5);
/// The Windows microphone privacy settings: the "microphone is silent" card opens them, and the welcome window.
pub(crate) const MIC_SETTINGS: &str = "ms-settings:privacy-microphone";

pub enum CoreEvent {
    Input(Input),
    UiAction(UiAction),
    /// The Drop window has subscribed to events.
    UiReady,
    /// The card window has subscribed to events.
    CardReady,
    Mic { id: DictationId, ev: MicEvent },
    /// `task` is the task generation; `path` is how the audio was actually transcribed (for the price in usage.csv).
    SttDone { id: DictationId, task: u64, result: Result<SttOutput, EngineError>, path: SttPath },
    RefineDone { id: DictationId, task: u64, result: RefineResult },
    /// `at` is the moment of Ctrl+V (or of the "no field" decision): the latency is counted up to it.
    InsertDone { id: DictationId, outcome: InsertOutcome, at: Instant },
    InsertTimeout { id: DictationId, generation: u64 },
    /// The field watch after a paste is over: `span` is the corrected text between the paste's anchors; `stop` is that
    /// watch's own flag, which tells it from a later one; `app` is the dictation's app, when its correction counts for
    /// habits (`None` for a translation). The text is never logged.
    Corrected { pasted: String, span: Option<String>, stop: Arc<AtomicBool>, app: Option<String> },
    /// The rule of the habit being made under `draft`, from the cleanup's connection: `None` (no connection, no answer
    /// in 5 s, none usable) — the template writes it. Never logged.
    HabitRule { draft: u64, rule: Option<habits::RuleText> },
    TimerFired { kind: TimerKind, generation: u64 },
    /// The hold after a tray card's action (`hold_tray`) is over: the card has folded. `generation` tells it from a
    /// later hold.
    TrayDone { generation: u64 },
    /// A step of the "Ask" request `task` of dictation `id`.
    AskStep { id: DictationId, task: u64, step: engine_ask::Step },
    /// The Ctrl+C copy of the first question of dictation `id` came: it is its selection. `task` is the request that
    /// waited for it, which may have been dropped since (Esc): "restore" asks with the copy all the same.
    AskCopied { id: DictationId, task: u64, text: String },
    /// The "Ask" request `task` ended.
    AskDone { id: DictationId, task: u64, result: engine_ask::AskResult },
    /// The answer view changed sooner than `VIEW_EVERY_MS` after it last went out: send it now.
    AnswerFlush,
    /// A source chip was clicked: open its page.
    AnswerSource(u32),
    /// A cache warm-up got an answer: its tokens go to usage.csv, priced with the model it warmed.
    Warmed { usage: claude::Usage, model: String },
    /// Wake from sleep or unlock: the pooled connections are most likely dead.
    Wake,
    /// The Drop was dragged to a new place: saved in settings.json.
    DropMoved(DropPosition),
    /// The idle Drop hid over a full-screen app (`true`) or shows again (`false`).
    IdleHidden(bool),
    /// The main window reads the settings or changes one (spec 3 §3). The answer is always the settings as the core
    /// holds them after the command.
    Settings { cmd: SettingsCmd, reply: tokio::sync::oneshot::Sender<SettingsView> },
    /// "Clear history" in the main window: every day of the history goes, with every recording.
    HistoryClear,
    /// A step of a speech model's download or load, or of pinning a found one.
    Model(ModelNews),
    /// A window asks for the speech model's state.
    ModelGet(tokio::sync::oneshot::Sender<ModelState>),
    /// "Retry" on the speech model in the main or the welcome window.
    ModelRetry,
    /// The Model page asks for its list (`cue://models` carries every change after).
    ModelsGet(tokio::sync::oneshot::Sender<ModelsView>),
    /// The Model page searches Hugging Face; the answer comes from a task of its own.
    ModelsSearch { query: String, reply: tokio::sync::oneshot::Sender<Result<Vec<hub::Found>, EngineError>> },
    /// The Model page chose a model: it is switched to.
    ModelChoose(Choice),
    /// The Model page deletes a model's files; the answer — whether they went.
    ModelDelete { id: String, reply: tokio::sync::oneshot::Sender<bool> },
    /// A step of the cleanup's language model: its download, its load, or pinning a found one.
    Llm(llm::LlmNews),
    /// The Cleanup page asks for the local list (`cue://llms` carries every change after).
    LlmsGet(tokio::sync::oneshot::Sender<ModelsView>),
    /// The Cleanup page searches Hugging Face for GGUF models; the answer comes from a task of its own.
    LlmsSearch { query: String, reply: tokio::sync::oneshot::Sender<Result<Vec<hub::LlmFound>, EngineError>> },
    /// The Cleanup page chose a local model: it is switched to.
    LlmChoose(Choice),
    /// The Cleanup page deletes a local model's file; the answer — whether it went.
    LlmDelete { id: String, reply: tokio::sync::oneshot::Sender<bool> },
    /// A history entry's text from the main window (decision 3): "Paste again" into `target`, the window that was under
    /// the main one and is now in front, or "Copy" (`None`) — the clipboard. Neither is a new result: the last result
    /// (Ctrl+Win+V) and the history stay as they are.
    HistoryText { text: String, target: Option<WindowRef> },
}

/// What the main window wants of the settings: to read them, or to change one.
pub enum SettingsCmd {
    Get,
    Set(SettingChange),
}

pub fn input_of(ev: HotkeyEvent) -> Input {
    match ev {
        HotkeyEvent::ChordDown(mode) => Input::ChordDown(mode),
        HotkeyEvent::ModeChange(mode) => Input::ModeChange(mode),
        HotkeyEvent::ModeToggle => Input::ModeToggle,
        HotkeyEvent::ChordUp => Input::ChordUp,
        HotkeyEvent::ChordAborted => Input::ChordAborted,
        HotkeyEvent::Esc => Input::Esc,
        HotkeyEvent::PasteLast => Input::PasteLast,
    }
}

/// The recognition session of the recording in progress: it keeps the frames and recognizes them at the second press.
struct Stream {
    id: DictationId,
    session: SttSession,
}

pub struct Runtime {
    flow: Flow,
    t0: Instant,
    tx: UnboundedSender<CoreEvent>,
    ui: Ui,
    /// The "Ask" card window (none — every answer goes to the tray).
    card: Card,
    settings: Settings,
    /// The settings were read without error. Otherwise we run with defaults and do not overwrite the file for the
    /// whole session: the user's corrupt or unreadable settings.json must not be replaced with defaults.
    can_save_settings: bool,
    /// The other language of the translation pair (`translate_pair`), decided at startup and again when the main window
    /// changes the target. `None`: no language other than the target was found, and target speech is only cleaned up.
    translate_home: Option<&'static str>,
    /// The interface language the tray menu, the Drop and the card speak now (decided at startup, changed from the main
    /// window).
    lang: Lang,
    paths: Paths,
    history: HistoryLog,
    /// One client for the whole app (refine, batch transcription, warm-up): a shared connection pool.
    http: reqwest::Client,
    focus: Arc<FocusWorker>,
    insert: Option<std::sync::mpsc::Sender<InsertJob>>,
    timers: Timers<JoinHandle<()>>,
    mic_gate: MicGate,
    mic: Option<MicHandle>,
    /// The pinned microphone the open recording asked for (`None` — the Windows one), whether the user was told it is
    /// missing, and the names of the last fallback told: the pinned one and the one recording instead (its card shows
    /// them, and a Drop loaded again gets them back).
    mic_wanted: Option<String>,
    mic_told: MicTold,
    mic_missing: Option<(String, String)>,
    wav: Option<WavSink>,
    /// The dictation whose own recording `current.wav` is: none when its attempt could not make the file — what is
    /// there then was left by an earlier attempt — and none once the file has gone.
    wav_of: Option<DictationId>,
    silence: SilenceDetector,
    /// The silence detector said "there is sound" at least once: if the microphone fails, what was said is processed.
    heard: bool,
    stream: Option<Stream>,
    /// The speech model: its state, its download and load, the slot recognition waits on.
    model: ModelKeeper,
    /// The cleanup's language model on this computer, and the slot the cleanup takes it from.
    llm: llm::LlmKeeper,
    /// The recognition in flight (at the second press, or "retry"): `AbortStt` cancels it too. Without a task (there
    /// was no session) it is `None`: the result was sent at once.
    stt_task: TaskSlot<Option<JoinHandle<()>>>,
    refine_task: TaskSlot<JoinHandle<()>>,
    /// The focused field, queried right after the second press, for the refine context.
    refine_focus: Option<(DictationId, std::sync::mpsc::Receiver<FocusInfo>)>,
    insert_watch: InsertWatch,
    /// The bare final text of the dictation (no join, line breaks not replaced), for `last\result.txt`.
    final_text: Option<(DictationId, String)>,
    /// The text of the dictation as it was recognized, before refine cleaned it up or translated it: its history entry
    /// keeps it. The user's text: only lengths ever reach the log.
    raw_text: Option<(DictationId, String)>,
    /// The last text given to the user: "Copy" and Ctrl+Win+V take it from here, after a restart from disk.
    last_text: Option<String>,
    /// The last real app name from the field queries (for the terminal safeguard when the query on insert
    /// was late).
    last_app: String,
    /// The last tray card sent to the window: its button acts on it.
    tray: Option<TrayKind>,
    /// A card whose action shows its done label is still on the Drop until it folds (`hold_tray`): that hold's
    /// generation. The Drop is not at rest meanwhile.
    tray_closing: Option<u64>,
    tray_gen: u64,
    /// The dictionary suggestion from a correction (docs/specs/2026-11-02-corrections.md).
    offers: Offers,
    /// The idle Drop is hidden over a full-screen app (the placement thread tells): no suggestion opens then.
    idle_hidden: bool,
    /// Windows was asked not to slow CUE down: a dictation records or is worked on (`power::full_speed`).
    full_speed: bool,
    /// The dictation whose paste is awaited, with its text, and its app when its correction counts for habits: the field
    /// is watched once it went in.
    watch_next: Option<(DictationId, String, Option<String>)>,
    /// The flag of the field watch running: set to end it early (a new dictation opened the microphone, "Learn from
    /// me" went off).
    field_stop: Option<Arc<AtomicBool>>,
    /// Habits (docs/specs/2026-11-04-habits.md), for each dictation: the ones sent with its cleanup with its text as
    /// heard, and those whose effect the cleanup brought (its history entry lists them).
    habits_sent: ByDictation<(Vec<habits::Habit>, String)>,
    habits_applied: ByDictation<Vec<history::HabitMark>>,
    /// Habits waiting for their rule, by the number they were found with.
    drafts: Vec<(u64, habits::Draft)>,
    /// Habit cards waiting for the Drop's rest, in the order found, and the one open.
    habit_cards: VecDeque<(u64, HabitCard)>,
    habit_card: Option<HabitCard>,
    /// Numbers the notices as they are found (`next_notice`), the suggestion's (`offer_found`) and each habit's: at rest
    /// the one found first opens first.
    notices: u64,
    offer_found: u64,
    level: LevelGate,
    /// The connection warm-up; the refine prompt cache has its own (`cache_warm`).
    warm: WarmUp,
    cache_warm: CacheWarm,
    attempt: Attempt,
    usage: UsageLog,
    /// "Ask": the focus query with the selection taken at the chord (spec 2 §6.1) and its answer once read; the Ctrl+C
    /// copy started at the end of the recording; what the first question was asked with (for "restore"); the open
    /// conversation; its request in flight; its own prompt cache warm-up; how often the answer view goes out; whether
    /// the flow has heard that the request's answer shows.
    ask_query: Option<(DictationId, std::sync::mpsc::Receiver<FocusInfo>)>,
    /// Windows uploads copied text to the cloud clipboard by itself, as read at the chord: no Ctrl+C fallback then.
    ask_cloud: bool,
    ask_info: Option<(DictationId, FocusInfo)>,
    ask_copy: Option<(DictationId, std::sync::mpsc::Receiver<Option<String>>)>,
    ask_taken: ask::KeptTaken,
    conversation: Option<ask::Conversation>,
    ask_task: TaskSlot<JoinHandle<()>>,
    ask_warm: CacheWarm,
    view_gate: ViewGate,
    ask_shown: ShownOnce,
    /// A request was dropped while its effects ran (`AbortAsk`): the view goes out once they all have, unless the
    /// answer closed with them — never just before it folds.
    view_changed: bool,
    /// The attempt of an answer still printing when a follow-up started (decision 12): its row is written when it ends.
    answering: Option<Attempt>,
    /// The sound of the "Ask" question asked last, taken as it went out, under a name of its own until its turn ends
    /// and its entry has an id; kept for Retry while its request failed.
    ask_sound: Option<(DictationId, Snapshot)>,
}

impl Runtime {
    pub fn new(
        ui: Ui,
        card: Card,
        settings: Settings,
        can_save_settings: bool,
        lang: Lang,
        paths: Paths,
        tx: UnboundedSender<CoreEvent>,
    ) -> Self {
        let settings = kept_settings(settings, can_save_settings);
        let has_last = paths.last_text().exists();
        let insert = match insert::spawn_inserter(tx.clone()) {
            Ok(tx) => Some(tx),
            Err(e) => {
                tracing::error!("insert thread not started: {e}");
                None
            }
        };
        let translate_home = pair_now(&settings);
        let model = ModelKeeper::new(paths.models.clone(), &settings.stt.model);
        let llm = llm::LlmKeeper::new(paths.models.clone(), &settings.refine.local, settings.refine.runs == settings::Runs::Local);
        let history = HistoryLog::new(paths.history.clone(), Recordings::new(paths.history_audio.clone()));
        // Alt picks "Ask" only when it is on (spec 2 §9); the hook reads the switch on every key.
        hook::ASK_ENABLED.store(settings.ask.enabled(), Ordering::Relaxed);
        tracing::info!(ask = settings.ask.enabled(), web_search = settings.ask.web_search(), "ask");
        Self {
            flow: Flow::new(settings.hold_hints_left, has_last),
            t0: Instant::now(),
            tx,
            ui,
            card,
            settings,
            can_save_settings,
            translate_home,
            lang,
            paths,
            history,
            http: reqwest::Client::new(),
            focus: Arc::new(FocusWorker::spawn()),
            insert,
            timers: Timers::default(),
            mic_gate: MicGate::default(),
            mic: None,
            mic_wanted: None,
            mic_told: MicTold::default(),
            mic_missing: None,
            wav: None,
            wav_of: None,
            silence: SilenceDetector::new(),
            heard: false,
            stream: None,
            model,
            llm,
            stt_task: TaskSlot::default(),
            refine_task: TaskSlot::default(),
            refine_focus: None,
            insert_watch: InsertWatch::default(),
            final_text: None,
            raw_text: None,
            last_text: None,
            last_app: String::new(),
            tray: None,
            tray_closing: None,
            tray_gen: 0,
            offers: Offers::default(),
            idle_hidden: false,
            full_speed: false,
            watch_next: None,
            field_stop: None,
            habits_sent: ByDictation::default(),
            habits_applied: ByDictation::default(),
            drafts: Vec::new(),
            habit_cards: VecDeque::new(),
            habit_card: None,
            notices: 0,
            offer_found: 0,
            level: LevelGate::default(),
            warm: WarmUp::default(),
            cache_warm: CacheWarm::default(),
            attempt: Attempt::new(DictationId::default()),
            usage: UsageLog::default(),
            ask_query: None,
            ask_cloud: false,
            ask_info: None,
            ask_copy: None,
            ask_taken: ask::KeptTaken::default(),
            conversation: None,
            ask_task: TaskSlot::default(),
            ask_warm: CacheWarm::default(),
            view_gate: ViewGate::default(),
            ask_shown: ShownOnce::default(),
            view_changed: false,
            answering: None,
            ask_sound: None,
        }
    }

    /// The longest a recording may stay, as the core holds it: the main window's commands play none past it.
    pub fn audio_limit(&self) -> AudioLimit {
        self.history.recordings.limit()
    }

    /// The core loop. Blocking calls (UI Automation up to 150 ms, the key store) run right here,
    /// so `run` spins on its own thread (`block_on`), not in the shared tokio pool.
    pub async fn run(mut self, mut rx: UnboundedReceiver<CoreEvent>) {
        self.sync_hook_flags();
        // The days and the recordings the history no longer keeps go at startup, before any new entry; none while its
        // period is unknown.
        let h = &self.settings.history;
        if h.keep.known().is_none() {
            tracing::warn!("history: period unknown, nothing is written to it or deleted from it until it is set");
        } else if h.audio.known().is_none() {
            tracing::warn!("history: recordings period unknown, none is recorded until it is set");
        }
        self.history.start(&self.settings.history, &local_today());
        self.model.start(&self.tx, &self.http);
        self.ui.model(self.model.state());
        self.ui.models(&self.model.view());
        self.llm.start(&self.tx, &self.http);
        self.ui.llms(&self.llm.view());
        forget_soniox_key();
        self.key_habits();
        while let Some(ev) = rx.recv().await {
            // A crash in one event does not stop the core; the panic hook logs where it happened.
            if catch_unwind(AssertUnwindSafe(|| self.on_event(ev))).is_err() {
                tracing::error!("core: event not handled because of a crash, carrying on");
            }
            self.sync_hook_flags();
        }
    }

    /// The keyboard hook reads these flags itself: whether to swallow Esc, whether Shift toggles the mode and whether
    /// CUE is paused.
    fn sync_hook_flags(&self) {
        hook::ESC_ACTIVE.store(self.flow.esc_active(), Ordering::Relaxed);
        hook::RECORDING.store(self.flow.recording_now(), Ordering::Relaxed);
        hook::PAUSED.store(self.flow.ui_state().paused, Ordering::Relaxed);
    }

    fn now(&self) -> u64 {
        self.t0.elapsed().as_millis() as u64
    }

    fn ms(&self, at: Instant) -> u64 {
        at.saturating_duration_since(self.t0).as_millis() as u64
    }

    /// `StartStt` runs last in its batch: the UI Automation query (the app name for the refine context)
    /// does not delay showing the Drop or the timers. Meanwhile microphone frames wait in the core queue.
    fn feed(&mut self, input: Input) {
        let now = self.now();
        let mut later = Vec::new();
        let fx = self.flow.handle(input, now);
        // The hook sees the new phase before the effects run: opening the microphone and the field query take a while,
        // and a Shift pressed right after a tap must already toggle the recording's mode.
        self.sync_hook_flags();
        for e in fx {
            if matches!(e, Effect::StartStt { .. }) {
                later.push(e);
            } else {
                self.exec(e);
            }
        }
        for e in later {
            self.exec(e);
        }
        self.sync_answer();
        self.show_notice();
    }

    /// Once the flow's effects have run: the answer's rows of a follow-up (spec 2 §7 "Listening to the follow-up…";
    /// decision 7: one that could not be transcribed), and the view a dropped request changed. Sent only if the answer
    /// is still there, so it never goes out just before the answer closes.
    fn sync_answer(&mut self) {
        let (listening, missed) =
            follow_up_rows(self.flow.answer_open(), self.flow.recording_now(), self.flow.ui_state().phase);
        let dropped = std::mem::take(&mut self.view_changed);
        let Some(c) = self.conversation.as_mut() else { return };
        let rows = (c.view.listening, c.view.missed) != (listening, missed);
        c.view.listening = listening;
        c.view.missed = missed;
        if rows || dropped {
            self.send_view(true);
        }
    }

    fn on_event(&mut self, ev: CoreEvent) {
        match ev {
            CoreEvent::Input(i @ (Input::ModeChange(_) | Input::ModeToggle)) => {
                self.feed(i);
                // Only the flow decides the mode, and only while recording: the attempt follows, so one that ends
                // before refine (Esc, a failed transcription) writes the mode it was recording in.
                if self.flow.recording_now() {
                    let id = self.flow.current_id();
                    self.attempt.follow_mode(id, self.flow.mode());
                    // Alt picked "Ask" while the chord is held: the context is taken now, the focus is still in place.
                    let taken = self.ask_query.as_ref().is_some_and(|(q, _)| *q == id);
                    if self.flow.mode() == Mode::Ask && !self.flow.answer_open() && !taken {
                        self.take_ask_context(id);
                    }
                }
            }
            CoreEvent::Input(Input::SessionLocked) => {
                self.feed(Input::SessionLocked);
                // Locked or asleep, nobody dictates for a while: the speech model is let go, once the dictation the
                // lock ended is recognized.
                self.model.rest_soon();
            }
            CoreEvent::Input(i) => self.feed(i),
            CoreEvent::UiAction(UiAction::TrayAction) => self.on_tray_action(),
            CoreEvent::UiAction(UiAction::TrayClose) => {
                self.tray = None;
                self.tray_closing = None;
                self.habit_card = None;
                self.show_notice();
            }
            // The flow does not change: the runtime copies the answer.
            CoreEvent::UiAction(UiAction::AnswerCopy) => self.copy_answer(),
            // The Drop's answers to the suggestion; one out of turn (the suggestion it answers is gone) changes nothing.
            // Once the capsule closes, a habit's card waiting behind it may open.
            CoreEvent::UiAction(UiAction::SuggestAdd) => {
                if let Some(r) = self.offers.added() {
                    let took = self.learn_word(SettingChange::DictLearn(LearnedWord { word: r.word, heard: r.heard }));
                    if !took {
                        self.offers.undone();
                        self.ui.suggest(None);
                        self.show_notice();
                    }
                }
            }
            CoreEvent::UiAction(UiAction::SuggestUndo) => {
                if let Some(r) = self.offers.undone() {
                    self.learn_word(SettingChange::DictRemove(r.word));
                    self.ui.suggest(None);
                    self.show_notice();
                }
            }
            CoreEvent::UiAction(UiAction::SuggestDismiss) => {
                if self.offers.dismissed() {
                    self.ui.suggest(self.offers.view().as_ref());
                    self.show_notice();
                }
            }
            CoreEvent::UiAction(UiAction::SuggestDone) => {
                if self.offers.done() {
                    self.ui.suggest(None);
                    self.show_notice();
                }
            }
            CoreEvent::UiAction(a) => {
                // "Restore" (the flow accepts it only in "Cancelled"): the dictation latency is counted from it.
                let now = self.now();
                if a == UiAction::Undo
                    && self.flow.ui_state().phase == UiPhase::Cancel
                    && self.attempt.id == self.flow.current_id()
                {
                    self.attempt.undo(now);
                }
                self.feed(Input::Ui(a));
            }
            CoreEvent::UiReady => {
                tracing::info!("Drop window ready");
                let s = self.flow.ui_state();
                self.emit_state(&s);
                self.send_mic_names(self.tray);
                self.send_habit_rule(self.tray);
                self.ui.tray(self.tray);
                self.ui.suggest(self.offers.view().as_ref());
                self.ui.model(self.model.state());
                self.ui.resend_idle_hidden();
                self.ui.resend_place();
                if let Some(c) = self.conversation.as_ref().filter(|c| c.opened) {
                    self.ui.answer(Some(&c.view));
                }
            }
            CoreEvent::CardReady => {
                tracing::info!("card window ready");
                // The answer's buttons and cross follow the phase in the card as in the tray.
                self.ui.state(&self.flow.ui_state());
                self.ui.resend_card_place();
                if let Some(c) = self.conversation.as_ref().filter(|c| c.opened) {
                    self.ui.answer(Some(&c.view));
                }
            }
            CoreEvent::Mic { id, ev } => self.on_mic(id, ev),
            CoreEvent::SttDone { id, task, result, path } => self.on_stt_done(id, task, result, path),
            CoreEvent::RefineDone { id, task, result } => self.on_refined(id, task, result),
            CoreEvent::InsertDone { id, outcome, at } => {
                if self.insert_watch.done(id) {
                    let at = self.ms(at);
                    if let Some(a) = self.attempt_for(id) {
                        a.inserted(at);
                    }
                    // The text awaited goes whatever the outcome; only a paste of this dictation is watched.
                    if let Some((wanted, pasted, app)) = self.watch_next.take()
                        && wanted == id
                        && outcome == InsertOutcome::Pasted
                    {
                        self.watch_field(pasted, app);
                    }
                    self.feed(Input::Inserted { id, outcome });
                }
            }
            CoreEvent::Corrected { pasted, span, stop, app } => self.on_corrected(&pasted, span.as_deref(), &stop, app.as_deref()),
            CoreEvent::HabitRule { draft, rule } => self.on_habit_rule(draft, rule),
            CoreEvent::InsertTimeout { id, generation } => {
                if self.insert_watch.expired(id, generation) {
                    tracing::warn!("insert did not finish within {} s: \"no field\"", INSERT_WATCHDOG.as_secs());
                    self.watch_next = None;
                    let now = self.now();
                    if let Some(a) = self.attempt_for(id) {
                        a.inserted(now);
                    }
                    self.feed(Input::Inserted { id, outcome: InsertOutcome::NoField });
                }
            }
            CoreEvent::TimerFired { kind, generation } => {
                if self.timers.fire(kind, generation) {
                    self.feed(Input::Timer(kind));
                }
            }
            CoreEvent::TrayDone { generation } => {
                if self.tray_closing == Some(generation) {
                    self.tray_closing = None;
                    self.show_notice();
                }
            }
            CoreEvent::AskStep { id, task, step } => self.on_ask_step(id, task, step),
            CoreEvent::AskCopied { id, task, text } => match self.conversation.as_mut() {
                // In flight, the request that waited for it was sent with the copy; "restore" asks with it too.
                Some(c) if self.ask_task.running(task) => {
                    c.copied(&text);
                    self.ask_taken.keep(id, &c.taken);
                }
                // Esc dropped that request first: "restore" asks with the copy all the same.
                _ => self.ask_taken.copied(id, &text),
            },
            CoreEvent::AskDone { id, task, result } => self.on_ask_done(id, task, result),
            CoreEvent::AnswerFlush => {
                let now = self.now();
                self.view_gate.flushed(now);
                if let Some(c) = self.conversation.as_ref().filter(|c| c.opened) {
                    self.ui.answer(Some(&c.view));
                }
            }
            CoreEvent::AnswerSource(n) => self.open_source(n),
            CoreEvent::Warmed { usage, model } => self.on_warmed(usage, &model),
            CoreEvent::Wake => {
                self.warm.reset();
                self.cache_warm.reset();
                self.ask_warm.reset();
            }
            CoreEvent::DropMoved(place) => {
                self.settings.drop.position = place;
                self.save_settings();
            }
            CoreEvent::IdleHidden(hidden) => {
                self.idle_hidden = hidden;
                self.show_notice();
            }
            CoreEvent::Model(news) => {
                let told = self.model.on_event(news, &self.tx, &self.http);
                self.ui.model(self.model.state());
                self.ui.models(&self.model.view());
                let tray = match told {
                    Told::Nothing => None,
                    Told::Ready => Some(TrayKind::ModelReady),
                    // The new model is written only now that it has loaded.
                    Told::Switched { id, name } => {
                        self.settings.stt.model = id;
                        self.save_settings();
                        self.ui.model_name(&name);
                        Some(TrayKind::ModelSwitched)
                    }
                };
                // "Model is ready" after a download or a switch, only on a Drop at rest: a dictation that waited shows
                // its text instead.
                if tray.is_some() && self.tray.is_none() && self.tray_closing.is_none() && self.flow.ui_state().phase == UiPhase::Idle {
                    self.tray = tray;
                    self.ui.tray(self.tray);
                }
            }
            CoreEvent::ModelGet(reply) => {
                let _ = reply.send(self.model.state());
            }
            CoreEvent::ModelRetry => {
                self.model.retry(&self.tx, &self.http);
                self.ui.model(self.model.state());
                self.ui.models(&self.model.view());
            }
            CoreEvent::ModelsGet(reply) => {
                let _ = reply.send(self.model.view());
            }
            CoreEvent::ModelsSearch { query, reply } => {
                let http = self.http.clone();
                tokio::spawn(async move {
                    let _ = reply.send(hub::search(&http, &query).await);
                });
            }
            CoreEvent::ModelChoose(choice) => {
                self.model.choose(choice, &self.tx, &self.http);
                self.ui.models(&self.model.view());
            }
            CoreEvent::ModelDelete { id, reply } => {
                let _ = reply.send(self.model.delete(&id));
                self.ui.models(&self.model.view());
            }
            CoreEvent::Llm(news) => {
                // The new local model is written only now that it has loaded.
                if let Told::Switched { id, .. } = self.llm.on_event(news, &self.tx, &self.http) {
                    self.settings.refine.local = id;
                    self.save_settings();
                    self.send_settings();
                    // The first dictation after a load would otherwise evaluate CUE's whole system part.
                    if let Some(g) = self.llm.slot().get() {
                        local::warm(g, &self.settings.dictionary, &[], &self.settings.refine);
                    }
                }
                self.ui.llms(&self.llm.view());
            }
            CoreEvent::LlmsGet(reply) => {
                let _ = reply.send(self.llm.view());
            }
            CoreEvent::LlmsSearch { query, reply } => {
                let http = self.http.clone();
                tokio::spawn(async move {
                    let _ = reply.send(hub::search_llms(&http, &query).await);
                });
            }
            CoreEvent::LlmChoose(choice) => {
                self.llm.choose(choice, &self.tx, &self.http);
                self.ui.llms(&self.llm.view());
            }
            CoreEvent::LlmDelete { id, reply } => {
                let _ = reply.send(self.llm.delete(&id, &self.tx));
                self.ui.llms(&self.llm.view());
            }
            CoreEvent::Settings { cmd, reply } => {
                if let SettingsCmd::Set(change) = cmd {
                    self.change_setting(change);
                }
                // The window may have closed meanwhile.
                let _ = reply.send(SettingsView::of(&self.settings, self.lang, self.can_save_settings));
            }
            CoreEvent::HistoryClear => self.history.clear(),
            CoreEvent::HistoryText { text, target } => match history_job(text, target) {
                HistoryJob::Paste { text, target } => self.dispatch_insert(text, None, Some(target), None),
                HistoryJob::Leave(job) => self.send_job(job, None),
            },
        }
    }

    /// A setting changed in the main window (spec 3 §3): the core's copy takes it and the file is written
    /// (`apply_setting`), then what depends on it follows at once — the translation pair, the interface language
    /// (decision 5), the days and the recordings the history keeps. The microphone is read when the next recording
    /// opens it. The log gets codes and states, never a device name.
    fn change_setting(&mut self, change: SettingChange) {
        let s = &self.settings;
        let (target, language, periods) = (s.translate.target(), s.ui.language, s.history.clone());
        let device = s.audio.device.clone();
        let runs = s.refine.runs;
        // "Forget all": habits still waiting for their rule or their card do not come back after it.
        if change == SettingChange::HabitsForget {
            self.drafts.clear();
            self.habit_cards.clear();
        }
        if !apply_setting(&mut self.settings, change, &self.paths.settings, self.can_save_settings) {
            return;
        }
        let s = &self.settings;
        if s.translate.target() != target {
            self.translate_home = pair_now(s);
        }
        if s.ui.language != language {
            let lang = Lang::resolve(s.ui.language, Lang::detect);
            tracing::info!(lang = lang.code(), setting = ?s.ui.language, "interface language");
            if lang != self.lang {
                self.lang = lang;
                self.ui.relang(lang);
            }
        }
        if s.history != periods {
            let (keep, audio) = (s.history.keep.known(), s.history.audio.known());
            let (keep, audio) = (keep.map_or("unknown", Keep::code), audio.map_or("unknown", AudioKeep::code));
            tracing::info!(keep, audio, "history: period changed");
            self.history.prune(&s.history, &local_today());
        }
        if s.refine.runs != runs {
            tracing::info!(runs = ?s.refine.runs, "cleanup: where it runs changed");
            // On this computer: the chosen model loads when its file is here. Back to the cloud: the next cleanup goes
            // there at once, the local model is let go (its memory freed), and a local download goes on.
            self.llm.set_active(s.refine.runs == settings::Runs::Local, &self.tx, &self.http);
            self.ui.llms(&self.llm.view());
        }
        let s = &self.settings;
        if s.audio.device != device {
            tracing::info!(pinned = s.audio.device.is_some(), "microphone setting changed");
            // Another choice: a microphone missing from now on is told about again.
            self.mic_told.forget();
        }
        // "Learn from me" off: no field is read from now on, and the suggestion goes; so do the habits waiting for their
        // rule or their card, the saved ones among them taken out of the list (not declined: they may be learned again),
        // and a change waiting for its card is taken back, the habits it replaced returning. The newest goes first, so a
        // change of a habit itself waiting is taken back before it. An open card stays until it closes; its "Undo" still
        // forgets.
        if !self.settings.learn.enabled() {
            self.watch_next = None;
            if let Some(stop) = self.field_stop.take() {
                stop.store(true, Ordering::Relaxed);
            }
            if self.offers.dictation_started() {
                self.ui.suggest(None);
            }
            self.offers = Offers::default();
            self.drafts.clear();
            let mut dropped = 0;
            for (_, card) in self.habit_cards.drain(..).rev() {
                if card.take_back(&mut self.settings.habits).is_some() {
                    dropped += 1;
                }
            }
            if dropped > 0 {
                tracing::info!(dropped, "habits not yet announced dropped");
                self.save_settings();
            }
        }
        // The habit whose card is open was forgotten here: its card goes.
        if let Some(card) = &self.habit_card
            && !self.settings.habits.list.iter().any(|h| h.id == card.id)
        {
            self.habit_card = None;
            self.tray = None;
            self.ui.tray(None);
            self.show_notice();
        }
    }

    /// A change from the Drop's suggestion: applied and written like the window's own, and the main window gets the
    /// settings (its Dictionary draws the word). False when the change was refused.
    fn learn_word(&mut self, change: SettingChange) -> bool {
        let took = apply_setting(&mut self.settings, change, &self.paths.settings, self.can_save_settings);
        if took {
            self.send_settings();
        }
        took
    }

    /// The field just pasted into is read until the user's correction is over (docs/specs/2026-11-02-corrections.md).
    /// Only the paste `insert` awaited, and only while "Learn from me" is on.
    fn watch_field(&mut self, pasted: String, app: Option<String>) {
        if !self.settings.learn.enabled() {
            return;
        }
        if let Some(old) = self.field_stop.take() {
            old.store(true, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, text, own) = (self.tx.clone(), pasted.clone(), stop.clone());
        if fieldwatch::spawn(text, stop.clone(), move |span| {
            let _ = tx.send(CoreEvent::Corrected { pasted, span, stop: own, app });
        }) {
            self.field_stop = Some(stop);
        }
    }

    /// A watch is over: a word the user wrote instead of the one heard becomes the suggestion (the first, when there
    /// are several), and a dictation's correction counts for habits (`count_habits`). The log gets numbers, never the
    /// text.
    fn on_corrected(&mut self, pasted: &str, span: Option<&str>, stop: &Arc<AtomicBool>, app: Option<&str>) {
        // A watch replaced by a later one leaves the later one's flag in place.
        self.field_stop.take_if(|running| Arc::ptr_eq(running, stop));
        if !self.settings.learn.enabled() {
            return;
        }
        let Some(span) = span else {
            tracing::info!("correction: none");
            return;
        };
        let found = corrections::candidates(pasted, span, &self.settings.dictionary.terms);
        tracing::info!(chars = span.chars().count(), candidates = found.len(), "correction found");
        if let Some(first) = found.into_iter().next() {
            let before = self.offers.view();
            self.offers.offer(first);
            if self.offers.waiting() {
                self.offer_found = self.next_notice();
            }
            // A missed one it replaced (the dot) is gone from the page until the new one opens at rest.
            let after = self.offers.view();
            if after != before {
                self.ui.suggest(after.as_ref());
            }
        }
        if let Some(app) = app {
            self.count_habits(app, pasted, span);
        }
        self.show_notice();
    }

    /// The forms of a dictation's correction in `app` are counted (`Habits::count`): a habit of another app whose form
    /// reached three here holds everywhere now (its row changes, no card), and forms that reached three here together
    /// become a habit once its rule comes (`ask_rule`). The log gets kinds and numbers.
    fn count_habits(&mut self, app: &str, pasted: &str, span: &str) {
        let c = habits::correction(pasted, span);
        if c.found.is_empty() {
            return;
        }
        let counted = self.settings.habits.count(app, &c, &local_today(), self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = c.found.iter().map(|f| f.form.kind()).collect();
        tracing::info!(?kinds, widened = counted.widened.len(), new = counted.new.is_some(), "habit forms counted");
        if counted.grown || !counted.widened.is_empty() {
            self.send_settings();
        }
        // A habit that widened has the id of everywhere now: its card, open or waiting, follows it.
        follow(&counted.widened, self.habit_card.as_mut(), &mut self.habit_cards);
        if let Some(draft) = counted.new {
            self.ask_rule(draft);
        }
    }

    /// The rule of a habit just found, asked of the cleanup's connection: within `RULE_WAIT_MS`, or at once without a
    /// connection, `on_habit_rule` gets it or nothing. The habit is numbered now, as found.
    fn ask_rule(&mut self, draft: habits::Draft) {
        let n = self.next_notice();
        let ask = draft.ask();
        self.drafts.push((n, draft));
        let (http, s, tx, lang, key) = (self.http.clone(), self.settings.refine.clone(), self.tx.clone(), self.lang.code(), self.cleanup_key());
        let to = route(s.runs, &self.llm.slot());
        tokio::spawn(async move {
            let wait = Duration::from_millis(habits::RULE_WAIT_MS);
            // On this computer the rule never asks the cloud: the local model, or the template.
            let asked = async {
                match to {
                    Route::Local(g) => local::rule(g, &s, &ask, lang, wait).await,
                    Route::NotReady => None,
                    Route::Cloud => habit::rule(&http, key.as_deref(), &s, &ask, lang, wait).await,
                }
            };
            let rule = tokio::time::timeout(wait, asked).await.ok().flatten();
            let _ = tx.send(CoreEvent::HabitRule { draft: n, rule });
        });
    }

    /// The rule came (or the template writes it): the habit is saved, the main window draws it, and its card waits for
    /// the Drop's rest. Dropped when "Learn from me" went off meanwhile.
    fn on_habit_rule(&mut self, n: u64, rule: Option<habits::RuleText>) {
        let Some(at) = self.drafts.iter().position(|(d, _)| *d == n) else { return };
        let (_, draft) = self.drafts.remove(at);
        if !self.settings.learn.enabled() {
            return;
        }
        let from_cleanup = rule.is_some();
        // A draft of the same forms in this app came first, or the habit widened meanwhile (`merged`): this one adds
        // to that habit, and its row changes with no second card. Habits that held an opposite of its forms went
        // (`replaced`): its card says "Habit changed". One of them whose card still waits takes that card's chain
        // along (`carry`), and that card goes.
        let habits::Learned { habit, replaced, merged, was } = self.settings.habits.learn(draft, rule, self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = habit.forms.iter().map(habits::Form::kind).collect();
        tracing::info!(?kinds, n = habit.n, from_cleanup, merged, replaced = replaced.len(), "habit learned");
        self.send_settings();
        let replaced = carry(&mut self.habit_cards, replaced);
        if !merged || !replaced.is_empty() {
            self.habit_cards.push_back((n, HabitCard { id: habit.id, replaced, was }));
            self.show_notice();
        }
    }

    fn next_notice(&mut self) -> u64 {
        self.notices += 1;
        self.notices
    }

    /// The settings to the main window after a change it did not ask for.
    fn send_settings(&self) {
        self.ui.settings(&SettingsView::of(&self.settings, self.lang, self.can_save_settings));
    }

    /// The cleanup connection's key from the store (a local server has none; `None` too when it cannot be read).
    fn cleanup_key(&self) -> Option<String> {
        KeyName::of_connection(self.settings.refine.connection().id).and_then(|name| {
            keys::lookup(name).unwrap_or_else(|e| {
                tracing::warn!("{} key: {e}", name.as_str());
                None
            })
        })
    }

    /// The habits' hash secret from the key store (docs/specs/2026-11-04-habits.md, "The hash's secret"), before the
    /// first correction is counted. A new one (none was kept, or what was kept is malformed) leaves the counts and the
    /// declined forms meaningless: they go, the habits' ids are made again, and the file is written. A store that cannot
    /// be read is left alone: no secret this session, so nothing is counted or declined, and nothing is dropped or
    /// written; the next start reads it again. So it is without random bytes. The log never gets the secret. Under the
    /// secret kept, a habit's id made otherwise (a settings.json restored or brought over) is made again and written
    /// (`Habits::reid`), so an id is always the one its forms and app give.
    fn key_habits(&mut self) {
        match keys::habit_secret() {
            Ok(keys::SecretRead::Kept(secret)) => {
                self.settings.habits.secret = secret;
                if self.settings.habits.reid() {
                    tracing::info!(habits = self.settings.habits.list.len(), "habits: ids made again under the kept secret");
                    self.save_settings();
                }
            }
            Ok(keys::SecretRead::Made { secret, saved }) => {
                if let Err(e) = saved {
                    tracing::warn!("{e}");
                }
                let (seen, declined) = self.settings.habits.rekey(secret);
                let habits = self.settings.habits.list.len();
                tracing::info!(seen, declined, habits, "habits: a new hash secret, counts and declined forms dropped");
                if seen + declined + habits > 0 {
                    self.save_settings();
                }
            }
            Err(e) => tracing::warn!("{e}: no habit is counted this session"),
        }
    }

    /// A notice waiting opens once the Drop is at rest and can be clicked (`at_rest`): the dictionary suggestion or a
    /// habit's card, one at a time, the one found first (`first_notice`). Called again after anything that may end a
    /// busy Drop: every input (the end of a pause among them), a tray card going, the suggestion's capsule closing, the
    /// idle Drop showing again once no full-screen app is in front, a habit learned.
    fn show_notice(&mut self) {
        let s = self.flow.ui_state();
        let tray = self.tray.is_some() || self.tray_closing.is_some();
        if !at_rest(s.phase, tray, self.flow.answer_open(), s.paused, self.idle_hidden) {
            return;
        }
        // A habit forgotten while its card waited has no card: it neither goes first nor holds the suggestion back.
        let list = &self.settings.habits.list;
        self.habit_cards.retain(|(_, card)| list.iter().any(|h| h.id == card.id));
        let offer = self.offers.waiting().then_some(self.offer_found);
        let card = self.habit_cards.front().map(|(n, _)| *n);
        match first_notice(offer, card, self.offers.open()) {
            Some(Notice::Offer) => {
                if self.offers.at_rest() {
                    self.ui.suggest(self.offers.view().as_ref());
                }
            }
            Some(Notice::Habit) => self.show_habit_card(),
            None => {}
        }
    }

    /// The next habit's card: its rule to the Drop, then the card ("New habit" or "Habit changed"). One forgotten from
    /// the main window meanwhile has none.
    fn show_habit_card(&mut self) {
        while let Some((_, card)) = self.habit_cards.pop_front() {
            let Some(rule) = self.settings.habits.list.iter().find(|h| h.id == card.id).map(|h| h.rule.clone()) else { continue };
            self.tray = Some(card.kind());
            self.habit_card = Some(card);
            self.ui.habit_rule(&rule);
            self.ui.tray(self.tray);
            return;
        }
    }

    /// The rule of the habit whose card is open, before the card goes to a Drop loaded again.
    fn send_habit_rule(&self, kind: Option<TrayKind>) {
        if matches!(kind, Some(TrayKind::NewHabit | TrayKind::HabitChanged))
            && let Some(h) = self.habit_card.as_ref().and_then(|c| self.settings.habits.list.iter().find(|h| h.id == c.id))
        {
            self.ui.habit_rule(&h.rule);
        }
    }

    fn on_mic(&mut self, id: DictationId, ev: MicEvent) {
        if !self.mic_gate.accepts(id) {
            return;
        }
        match ev {
            MicEvent::Frame { pcm, peak_dbfs, level } => {
                self.mic_told.frame();
                self.attempt.add_audio(pcm.len());
                if let Some(w) = self.wav.as_mut() {
                    w.write(&pcm);
                }
                let verdict = self.silence.push(peak_dbfs, pcm.len() as u64 / stt::SAMPLES_PER_MS);
                self.heard |= verdict == Silence::Alive;
                if self.level.allow(self.now()) {
                    self.ui.level(level);
                }
                if let Some(s) = self.stream.as_ref().filter(|s| s.id == id) {
                    s.session.push(pcm);
                }
                if verdict == Silence::Dead {
                    self.feed(Input::MicDead);
                }
            }
            // Spec 3 §3: the recording goes on, and the user hears it once per loss. No name in the log.
            MicEvent::Fallback { used } => {
                tracing::info!("microphone: the pinned one is not there, recording from the Windows one");
                if let Some(wanted) = self.mic_wanted.clone()
                    && self.mic_told.fell_back(&wanted)
                {
                    self.mic_missing = Some((wanted, used));
                    self.feed(Input::MicFallback);
                }
            }
            MicEvent::Failed(e) => {
                tracing::warn!(heard = self.heard, "microphone: {e}");
                self.feed(Input::MicFailed { heard: self.heard });
            }
        }
    }

    /// The missing microphone's names go to the Drop just before its card.
    fn send_mic_names(&self, kind: Option<TrayKind>) {
        if kind == Some(TrayKind::MicMissing)
            && let Some((wanted, used)) = &self.mic_missing
        {
            self.ui.mic_names(wanted, used);
        }
    }

    fn on_stt_done(&mut self, id: DictationId, task: u64, result: Result<SttOutput, EngineError>, path: SttPath) {
        if !self.stt_task.finished(task) {
            // The result of a cancelled task: its dictation is already over or is being transcribed again.
            return;
        }
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.stt_done(path, now);
        }
        let input = match result {
            Ok(out) => {
                if self.attempt.id == id {
                    // The model tells no language: the speech's language is read from the text's alphabet.
                    let pair = (self.settings.translate.target(), self.translate_home);
                    self.attempt.speech_language = out
                        .language
                        .or_else(|| speech::speech_language(&out.text, pair.0, pair.1).map(str::to_owned));
                }
                Input::SttFinal { id, text: out.text }
            }
            Err(e) => {
                tracing::warn!(stt_path = path.as_str(), "recognition failed: {e}");
                Input::SttFailed { id, error: stt_error(&e) }
            }
        };
        self.feed(input);
    }

    fn on_refined(&mut self, id: DictationId, task: u64, result: RefineResult) {
        if !self.refine_task.finished(task) {
            // The result of a cancelled refine (Esc in "Thinking"): after "restore" we wait for the new request.
            return;
        }
        // Of the habits sent, those whose effect shows in the cleaned text go on its history entry; a raw fallback shows
        // none.
        if let Some((sent, raw)) = self.habits_sent.take(id) {
            let marks = if result.outcome == RefineOutcome::Refined { habits::marks(&sent, &raw, &result.text) } else { Vec::new() };
            self.habits_applied.put(id, marks);
        }
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.refine_done(result.usage, result.outcome, now);
        }
        // A key problem (`RawKeyProblem`) is already in the log: the connection's refine writes one line for each failed
        // refine, whatever the insert outcome. The Drop names the connection in its error.
        if result.outcome == RefineOutcome::RawKeyProblem {
            self.ui.cleanup_name(self.settings.refine.connection().name);
        }
        self.feed(Input::Refined { id, text: result.text, outcome: result.outcome });
    }

    fn exec(&mut self, e: Effect) {
        match e {
            Effect::OpenMic { id } => {
                if let Some(stop) = &self.field_stop {
                    stop.store(true, Ordering::Relaxed);
                }
                if self.offers.dictation_started() {
                    self.ui.suggest(None);
                }
                self.open_mic(id)
            }
            Effect::StopMic { id, keep } => self.stop_mic(id, keep),
            Effect::StartStt { id } => self.start_stt(id),
            Effect::FinishStt { id } => self.finish_stt(id),
            Effect::AbortStt { id } => self.abort_stt(id),
            Effect::TranscribeSaved { id } => self.transcribe_saved(id),
            Effect::Refine { id, raw, mode } => self.refine(id, raw, mode),
            Effect::RefineAgain { id, raw, mode } => {
                if self.attempt.id == id {
                    let now = self.now();
                    self.attempt.retry_text(now);
                }
                self.refine(id, raw, mode)
            }
            Effect::AbortRefine { id } => {
                if let Some(task) = self.refine_task.take_for(id) {
                    task.abort();
                }
            }
            Effect::Ask { id, question, follow_up } => self.ask(id, &question, follow_up),
            Effect::AskAgain { id } => {
                let now = self.now();
                self.attempt.ask_again(id, now);
                if let Some(c) = self.conversation.as_mut() {
                    c.again();
                }
                self.send_view(true);
                self.spawn_ask(id);
            }
            Effect::AbortAsk { id } => {
                if let Some(task) = self.ask_task.take_for(id) {
                    task.abort();
                }
                // Its turn goes: "restore" asks again and takes the sound anew.
                if self.ask_sound.as_ref().is_some_and(|(i, _)| *i == id) {
                    self.drop_ask_sound();
                }
                // A follow-up's turn goes; for a first question the conversation closes right after.
                if let Some(c) = self.conversation.as_mut() {
                    c.abort();
                    self.view_changed = true;
                }
            }
            Effect::AskPaste { id, how } => self.ask_paste(id, how),
            Effect::CloseAnswer => self.close_answer(),
            Effect::Insert { id, text } => self.insert(id, text),
            Effect::PasteLast => self.paste_last(),
            Effect::Ui(s) => self.emit_state(&s),
            Effect::Shake => self.ui.shake(),
            // The tray card only: `None` — a new recording, a follow-up's too — never closes an open answer.
            Effect::Tray(kind) => {
                if kind == Some(TrayKind::MicMissing)
                    && let Some((wanted, _)) = &self.mic_missing
                {
                    self.mic_told.told(wanted);
                }
                self.send_mic_names(kind);
                self.habit_card = None; // the flow's card, or none, takes a habit's place
                self.tray_closing = None;
                self.tray = kind;
                self.ui.tray(kind);
            }
            Effect::SetTimer { kind, after } => self.set_timer(kind, after),
            Effect::CancelTimer(kind) => {
                if let Some(task) = self.timers.cancel(kind) {
                    task.abort();
                }
            }
            Effect::HoldHintShown => self.hold_hint_shown(),
            Effect::Finished { id, outcome } => self.on_finished(id, outcome),
            Effect::BringWelcome => crate::welcome_window::bring_forward(self.ui.app()),
        }
    }

    /// The only place the state goes out to the window and the tray icon; the voice-level rule lives here too.
    fn emit_state(&mut self, s: &UiState) {
        let busy = works(s.phase);
        if busy != self.full_speed {
            self.full_speed = busy;
            if !power::full_speed(busy) {
                tracing::debug!(busy, "Windows did not take the processor speed asked for");
            }
            // A speech model let go at rest loads as the dictation starts; one ending starts the wait before it rests.
            self.model.busy(busy, &self.tx);
        }
        self.level.phase(s.phase);
        self.ui.state(s);
        crate::tray::sync(self.ui.app(), TrayLook::of(s), s.paused);
    }

    /// Remembers the real app name from a field query.
    fn note_app(&mut self, app: &str) {
        if let Some(app) = known_app(app)
            && app != self.last_app
        {
            self.last_app = app.to_owned();
        }
    }

    fn open_mic(&mut self, id: DictationId) {
        if let Some(old) = self.mic.take() {
            stop_mic_soon(old);
        }
        let old = std::mem::replace(&mut self.attempt, Attempt::new(id));
        // A follow-up starts while the answer it follows still prints (decision 12): that answer's attempt waits
        // aside until it ends.
        if self.ask_task.is_for(old.id) {
            self.answering = Some(old);
        }
        // A new recording ends any "restore" or retry of an earlier question: what it was asked with is not kept.
        self.clear_ask_context();
        self.silence = SilenceDetector::new();
        self.heard = false;
        // The pinned microphone, as the settings say now; the `cue-mic` thread looks for it, the core never waits.
        self.mic_wanted = self.settings.audio.device.clone();
        self.mic_told.opened();
        // The attempt's audio goes to current.wav. A file left from the previous attempt is deleted first rather than
        // written over: a question's recording may still be taking its sound from it (a hard link, `recordings`).
        if let Err(e) = files::discard(&self.paths) {
            tracing::warn!("recording audio of the previous attempt not deleted: {e}");
        }
        self.wav = match WavSink::create(&self.paths.current_wav()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("recording audio is not written to disk: {e}");
                None
            }
        };
        // Without a file of its own, whatever stayed in current.wav is no sound of this attempt's.
        self.wav_of = self.wav.as_ref().map(|_| id);
        self.mic_gate.open(id);
        let (events, rx) = std::sync::mpsc::channel();
        let core = self.tx.clone();
        let forward = std::thread::Builder::new().name("cue-mic-events".into()).spawn(move || {
            for ev in rx {
                if core.send(CoreEvent::Mic { id, ev }).is_err() {
                    break;
                }
            }
        });
        match forward {
            Ok(_) => self.mic = Some(mic::open(self.mic_wanted.clone(), events)),
            Err(e) => {
                tracing::error!("microphone event thread not started: {e}");
                let _ = self.tx.send(CoreEvent::Mic { id, ev: MicEvent::Failed("no event thread".into()) });
            }
        }
    }

    /// The gate closes first: frames arriving after that reach neither the WAV nor the stream. The microphone itself
    /// stops in the background: its thread (and the driver) do not hold up the core.
    fn stop_mic(&mut self, id: DictationId, keep: bool) {
        self.mic_gate.close(id);
        if let Some(m) = self.mic.take() {
            stop_mic_soon(m);
        }
        if let Some(w) = self.wav.take()
            && let Err(e) = w.finish()
        {
            tracing::warn!("recording audio not fully saved: {e}");
        }
        if !keep {
            self.discard_sound();
        }
    }

    /// The attempt's audio, if any, is deleted: `current.wav` is nobody's recording any more.
    fn discard_sound(&mut self) {
        self.wav_of = None;
        if let Err(e) = files::discard(&self.paths) {
            tracing::warn!("recording audio not deleted: {e}");
        }
    }

    fn start_stt(&mut self, id: DictationId) {
        let info = self.focus.query(0, FOCUS_TIMEOUT);
        self.note_app(&info.app);
        self.attempt.app = info.app;
        self.attempt.mode = self.flow.mode();
        if self.flow.mode() == Mode::Ask {
            if self.flow.answer_open() {
                // A follow-up after a pause reads a warm cache too.
                self.warm_ask_cache();
            } else {
                self.take_ask_context(id);
            }
        }
        self.warm_up();
        self.warm_cache();
        self.warm_local();
        if let Some(old) = self.stream.take() {
            old.session.abort();
        }
        // The session keeps the frames whether the model is ready or not: a dictation made while it downloads waits for
        // it at the second press.
        let session = stt::start(Engine::Local(self.model.slot()));
        self.stream = Some(Stream { id, session });
    }

    fn finish_stt(&mut self, id: DictationId) {
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.finish_stt(now);
        }
        // Not for a question cancelled by Esc while recording (the flow is "cancelled" then): "restore" reads it if
        // asked.
        if self.flow.mode() == Mode::Ask && !self.flow.answer_open() && self.flow.ui_state().phase == UiPhase::Think {
            self.read_ask_context(id);
        }
        self.warm_up();
        self.prefetch_focus(id);
        let tx = self.tx.clone();
        let Some(Stream { session, .. }) = self.stream.take_if(|s| s.id == id) else {
            // No session was started for this dictation: a flow bug, reported as a failed recognition with Retry.
            let old = self.stt_task.start(id, |task| {
                let result = Err(EngineError::Model("no recognition session".into()));
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path: SttPath::Local });
                None
            });
            abort_stt_task(old);
            return;
        };
        let timeout = self.model.timeout();
        let old = self.stt_task.start(id, |task| {
            Some(tokio::spawn(async move {
                // If the task crashed: an error with Retry; the attempt's audio stays in current.wav.
                let result = guarded("recognition", session.finish(timeout), || Err(stt_panicked())).await;
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path: SttPath::Local });
            }))
        });
        abort_stt_task(old);
    }

    fn abort_stt(&mut self, id: DictationId) {
        if let Some(s) = self.stream.take_if(|s| s.id == id) {
            s.session.abort();
        }
        // A cancelled task drops the recognition too: its result is not needed.
        abort_stt_task(self.stt_task.take_for(id));
    }

    fn transcribe_saved(&mut self, id: DictationId) {
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.retry(now);
        }
        self.warm_up();
        self.prefetch_focus(id);
        // Retry after the model failed: its download goes on first, and recognition waits for it.
        self.model.retry(&self.tx, &self.http);
        self.ui.model(self.model.state());
        let (slot, timeout) = (self.model.slot(), self.model.timeout());
        let wav = self.paths.current_wav();
        let tx = self.tx.clone();
        let old = self.stt_task.start(id, |task| {
            Some(tokio::spawn(async move {
                let body = async {
                    tokio::time::timeout(timeout, stt::recognize_wav(slot, &wav)).await.unwrap_or(Err(EngineError::Timeout))
                };
                let result = guarded("retry recognition", body, || Err(stt_panicked())).await;
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path: SttPath::Local });
            }))
        });
        abort_stt_task(old);
    }


    /// Connect to Anthropic ahead of time: when recording starts and on the second press (in parallel with
    /// transcription), at most once every 20 s (right away after sleep) and only if there is a key: refine does not pay
    /// for the TLS handshake inside its 1.5 s budget.
    fn warm_up(&mut self) {
        let anthropic = self.settings.refine.runs == settings::Runs::Cloud && self.settings.refine.connection().kind == Kind::Anthropic;
        if !anthropic || !self.warm.due(self.now()) || !matches!(keys::lookup(KeyName::Anthropic), Ok(Some(_))) {
            return;
        }
        let http = self.http.clone();
        tokio::spawn(async move {
            let _ = http.head(WARM_URL).timeout(WARM_TIMEOUT).send().await;
        });
    }

    /// Warms the refine prompt cache when a dictation starts and the cache may have expired (spec 2 §6.3): the first refine
    /// after a pause reads the prompt from the cache. Dictation and translation share the prefix; "Ask" has its own
    /// (`warm_ask_cache`).
    fn warm_cache(&mut self) {
        // Over another connection there is no warm-up: its first cleanup writes the cache, where it has one.
        let model = self.settings.refine.model().id.to_owned();
        let anthropic = self.settings.refine.runs == settings::Runs::Cloud && self.settings.refine.connection().kind == Kind::Anthropic;
        if !anthropic || self.flow.mode() == Mode::Ask || !claude::can_warm(&model) || !self.cache_warm.due(self.now()) {
            return;
        }
        let Ok(Some(key)) = keys::lookup(KeyName::Anthropic) else { return };
        let (http, tx, dict) = (self.http.clone(), self.tx.clone(), self.settings.dictionary.clone());
        tokio::spawn(async move {
            if let Some(usage) = claude::warm(&http, &key, &model, &dict).await {
                let _ = tx.send(CoreEvent::Warmed { usage, model });
            }
        });
    }

    /// Warms the local model for a dictation just started on this computer: the cleanup's system part, with this app's
    /// habits, is evaluated while the person speaks. A translation and "Ask" have other prompts.
    fn warm_local(&self) {
        if self.settings.refine.runs != settings::Runs::Local || self.flow.mode() != Mode::Dictate {
            return;
        }
        let Some(g) = self.llm.slot().get() else { return };
        let habits: Vec<String> = if self.settings.learn.enabled() {
            self.settings.habits.for_app(&stored_app(&self.attempt.app)).map(|h| h.rule.clone()).collect()
        } else {
            Vec::new()
        };
        local::warm(g, &self.settings.dictionary, &habits, &self.settings.refine);
    }

    /// Warms the "Ask" prompt cache at its chord when it may have expired (spec 2 §6.3): its own prefix — the tools and
    /// the "Ask" instruction — with the thinking and effort of the request. Four minutes, like the refine cache.
    fn warm_ask_cache(&mut self) {
        let now = self.now();
        if !self.settings.ask.enabled() || !self.ask_warm.due(now) {
            return;
        }
        let Ok(Some(key)) = keys::lookup(KeyName::Anthropic) else { return };
        let model = self.settings.ask.model().to_owned();
        let search = self.ask_search();
        let (http, tx) = (self.http.clone(), self.tx.clone());
        tokio::spawn(async move {
            if let Some(usage) = engine_ask::warm(&http, &key, &model, search).await {
                let _ = tx.send(CoreEvent::Warmed { usage, model });
            }
        });
    }

    /// The warm-up's tokens are paid for: a usage.csv row of its own, priced with the model it warmed. The log gets the
    /// token counts only.
    fn on_warmed(&mut self, usage: claude::Usage, model: &str) {
        tracing::info!(
            model,
            cache_write = usage.cache_creation_input_tokens,
            cache_read = usage.cache_read_input_tokens,
            "prompt cache warmed"
        );
        let rec = warmup_record(usage, utc_now(), &self.settings, model);
        self.append_usage(rec);
    }

    /// The focused field (with the text before the caret), right after the second press, in parallel with
    /// transcription: by the time the text arrives the refine context is ready.
    fn prefetch_focus(&mut self, id: DictationId) {
        let (tx, rx) = std::sync::mpsc::channel();
        let focus = Arc::clone(&self.focus);
        let chars = self.settings.refine.context_chars;
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(focus.query(chars, FOCUS_TIMEOUT));
        });
        self.refine_focus = Some((id, rx));
    }

    fn focus_for_refine(&mut self, id: DictationId) -> FocusInfo {
        match self.refine_focus.take() {
            // The query is limited to 150 ms; if it did not come, "unknown" (refine without context).
            Some((f, rx)) if f == id => rx.recv_timeout(FOCUS_TIMEOUT * 2).unwrap_or_default(),
            _ => self.focus.query(self.settings.refine.context_chars, FOCUS_TIMEOUT),
        }
    }

    fn refine(&mut self, id: DictationId, raw: String, mode: Mode) {
        // The text as it was heard, for the history: a retry refines the same text again.
        self.raw_text = Some((id, raw.clone()));
        let info = self.focus_for_refine(id);
        self.note_app(&info.app);
        // The query was late ("unknown"): use the app at the start of recording.
        let app = first_known_app(&[&info.app, &self.attempt.app]).to_owned();
        let ctx = RefineContext::new(app, info.before_cursor.as_deref(), self.settings.refine.context_chars);
        // The connection's key (a local server has none). No key: refine writes the log line itself and returns the raw
        // text.
        let kind = self.settings.refine.connection().kind;
        let key = self.cleanup_key();
        let to = route(self.settings.refine.runs, &self.llm.slot());
        let now = self.now();
        // "Ask" has a prefix of its own (`ask_warm`).
        if mode != Mode::Ask {
            self.cache_warm.touched(now);
        }
        if self.attempt.id == id {
            self.attempt.refine_started(now);
            // The mode is final by the time refine starts.
            self.attempt.mode = mode;
        }
        let t = &self.settings.translate;
        let job = job_for(mode, t.speech(), self.attempt.speech_language.as_deref(), t.target(), self.translate_home);
        if let Job::Translate(to) = job {
            // The language code only, no text.
            tracing::info!(to, "translating");
        }
        // The habits of this app (or of everywhere) go with a dictation's cleanup, never with a translation or "Ask".
        let sent: Vec<habits::Habit> = if habits_for(mode, self.settings.learn.enabled(), job) {
            self.settings.habits.for_app(&stored_app(&self.attempt.app)).cloned().collect()
        } else {
            Vec::new()
        };
        let ctx = ctx.with_habits(&sent);
        self.habits_sent.put(id, (sent, raw.clone()));
        let (http, s, tx) = (self.http.clone(), self.settings.refine.clone(), self.tx.clone());
        let dict = self.settings.dictionary.clone();
        let old = self.refine_task.start(id, |task| {
            tokio::spawn(async move {
                let fallback_raw = raw.clone();
                // One future for both jobs, so `guarded` covers a translation too.
                let body = async {
                    let key = key.as_deref();
                    // On this computer the local model; in the cloud the connection picks the client; the settings give
                    // it the model and the style.
                    match to {
                        Route::Local(g) => {
                            return match job {
                                Job::Clean | Job::Rewrite => local::refine(g, &raw, &ctx, &dict, &s).await,
                                Job::Translate(to) => local::translate(g, &raw, &ctx, &dict, &s, to).await,
                            };
                        }
                        Route::NotReady => return local::not_ready(&raw),
                        Route::Cloud => {}
                    }
                    match (kind, job) {
                        (Kind::Anthropic, Job::Clean | Job::Rewrite) => claude::refine(&http, key, s.model().id, &raw, &ctx, &dict, &s).await,
                        (Kind::Anthropic, Job::Translate(to)) => {
                            claude::translate(&http, key, s.model().id, &raw, &ctx, &dict, &s, to).await
                        }
                        (Kind::OpenRouter, Job::Clean | Job::Rewrite) => openrouter::refine(&http, key, &raw, &ctx, &dict, &s).await,
                        (Kind::OpenRouter, Job::Translate(to)) => openrouter::translate(&http, key, &raw, &ctx, &dict, &s, to).await,
                        (Kind::Chat, Job::Clean | Job::Rewrite) => chat::refine(&http, key, &raw, &ctx, &dict, &s).await,
                        (Kind::Chat, Job::Translate(to)) => chat::translate(&http, key, &raw, &ctx, &dict, &s, to).await,
                    }
                };
                let result = guarded("refine", body, move || refine_panicked(fallback_raw)).await;
                let _ = tx.send(CoreEvent::RefineDone { id, task, result });
            })
        });
        if let Some(old) = old {
            old.abort();
        }
    }

    fn insert(&mut self, id: DictationId, text: String) {
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.insert_started(now);
        }
        self.final_text = Some((id, text.clone()));
        self.watch_insert(id);
        // A dictation's correction counts for habits in its app as the history names it; a translation's only offers
        // words.
        let habit_app = (self.attempt.mode == Mode::Dictate).then(|| stored_app(&self.attempt.app)).filter(|a| !a.is_empty());
        self.watch_next = (self.settings.learn.enabled() && self.attempt.mode != Mode::Ask).then(|| (id, text.clone(), habit_app));
        self.dispatch_insert(text, Some(id), None, None);
    }

    /// The flow waits for the insert of `id`: its result is awaited, and a watchdog stands in for one that never comes.
    fn watch_insert(&mut self, id: DictationId) {
        let generation = self.insert_watch.start(id);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INSERT_WATCHDOG).await;
            let _ = tx.send(CoreEvent::InsertTimeout { id, generation });
        });
    }

    fn paste_last(&mut self) {
        if let Some(text) = self.last_result() {
            self.dispatch_insert(text, None, None, None);
        }
    }

    /// The last result: from memory, and after a restart from disk.
    fn last_result(&self) -> Option<String> {
        if let Some(text) = &self.last_text {
            return Some(text.clone());
        }
        match std::fs::read_to_string(self.paths.last_text()) {
            Ok(text) if !text.is_empty() => Some(text),
            Ok(_) => None,
            Err(e) => {
                tracing::warn!("last result not read: {e}");
                None
            }
        }
    }

    /// One path for dictation, Ctrl+Win+V and an "Ask" paste at the caret or over the selection: the focused field (one
    /// short query) → the text as it is pasted (`pasted_text`: edges and the join space, or the replaced selection's own
    /// edges; one line in a terminal) → the insert thread. No field: the text goes to the clipboard. If the query was
    /// late ("unknown"), the terminal safeguard looks at the last known app, then at the app at the start of recording
    /// (`insert_app`). Within one dictation the last known app is never older than the app at the start of recording
    /// (`note_app` runs both at the start of recording and on every later query). `target` — an "Ask" paste: only into
    /// that window (`InsertJob::Paste`); `replaced` — the selection a Replace goes over.
    fn dispatch_insert(&mut self, text: String, id: Option<DictationId>, target: Option<WindowRef>, replaced: Option<&str>) {
        let info = self.focus.query(INSERT_CONTEXT_CHARS, FOCUS_TIMEOUT);
        let started = if id.is_some_and(|i| i == self.attempt.id) { self.attempt.app.as_str() } else { "" };
        let terminal = terminal_app(insert_app(&info.app, &self.last_app, started));
        self.note_app(&info.app);
        let text = pasted_text(&text, info.before_cursor.as_deref(), terminal, replaced);
        let job = if info.editable == Editability::No {
            InsertJob::Leave { id, text }
        } else {
            InsertJob::Paste { id, text, collapse: false, target }
        };
        self.send_job(job, id);
    }

    fn send_job(&mut self, job: InsertJob, id: Option<DictationId>) {
        if self.insert.as_ref().is_some_and(|tx| tx.send(job).is_ok()) {
            return;
        }
        tracing::error!("insert thread is not running");
        if let Some(id) = id {
            let _ = self.tx.send(CoreEvent::InsertDone { id, outcome: InsertOutcome::NoField, at: Instant::now() });
        }
    }

    /// The context of an "Ask" question, at the chord (or when Alt picked "Ask"), while the focus and the selection are
    /// where the user left them (spec 2 §6.1): on the focus thread, not to delay the Drop. Its prompt cache warms up
    /// here too (§6.3). A new question forgets what the last one was asked with.
    fn take_ask_context(&mut self, id: DictationId) {
        let (tx, rx) = std::sync::mpsc::channel();
        let focus = Arc::clone(&self.focus);
        let chars = self.settings.refine.context_chars;
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(focus.query_with_selection(chars, MAX_SELECTION_CHARS, ASK_FOCUS_TIMEOUT));
        });
        self.ask_query = Some((id, rx));
        // Once per chord, two short registry reads: whether a Ctrl+C copy would go to the user's other devices.
        self.ask_cloud = cloud_clipboard_uploads();
        self.ask_info = None;
        self.ask_copy = None;
        self.ask_taken.clear();
        self.warm_ask_cache();
    }

    /// At the end of the recording: the selection read at the chord, and the Ctrl+C copy when the field could not say
    /// (spec 2 §6.1). The copy needs the chord's keys up — they are only now — and runs on the insert thread, which
    /// owns the clipboard, alongside the transcription; the request task waits for it. Never in a terminal, nor in an
    /// editor that copies its current line when nothing is selected, nor while Windows uploads copies to the cloud
    /// clipboard by itself (the selection then reads as none).
    fn read_ask_context(&mut self, id: DictationId) {
        let info = match self.ask_query.take() {
            Some((q, rx)) if q == id => rx.recv_timeout(ASK_FOCUS_TIMEOUT).unwrap_or_default(),
            _ => FocusInfo::default(),
        };
        let app = first_known_app(&[&info.app, &self.attempt.app]).to_owned();
        let skip = terminal_app(&app) || copies_line(&app);
        let copy = needs_copy_fallback(&info.selection, info.editable, skip, self.ask_cloud);
        // Only into the window the field was read in: the insert thread looks again right before Ctrl+C.
        let target = info.window.filter(|_| copy);
        if let Some(target) = target {
            let (reply, rx) = std::sync::mpsc::channel();
            if self.insert.as_ref().is_some_and(|tx| tx.send(InsertJob::CopySelection { reply, target }).is_ok()) {
                self.ask_copy = Some((id, rx));
            }
        }
        // The kind and the length only (`SelectionRead`'s `Debug`), and states.
        tracing::info!(selection = ?info.selection, copy = target.is_some(), cloud_upload = self.ask_cloud, "ask: context");
        self.ask_info = Some((id, info));
    }

    /// The first question of dictation `id` has ended for good: its selection and the rest of what it was asked with
    /// are not kept for a "restore" or a retry any more.
    fn forget_ask_context(&mut self, id: DictationId) {
        self.ask_query.take_if(|(i, _)| *i == id);
        self.ask_info.take_if(|(i, _)| *i == id);
        self.ask_copy.take_if(|(i, _)| *i == id);
        self.ask_taken.forget(id);
    }

    /// No question's context is kept: a new recording has begun.
    fn clear_ask_context(&mut self) {
        self.ask_query = None;
        self.ask_info = None;
        self.ask_copy = None;
        self.ask_taken.clear();
    }

    /// What the question is asked with (spec 2 §6.1): the selection read at the chord (a Ctrl+C copy joins later, in
    /// the request task), the text before the caret, the app, today's local date.
    fn taken(&mut self, id: DictationId) -> ask::Taken {
        let info = self.ask_info.take().filter(|(i, _)| *i == id).map(|(_, info)| info).unwrap_or_default();
        let app = first_known_app(&[&info.app, &self.attempt.app]).to_owned();
        let (selection, sel, rects, clipped) = match info.selection {
            SelectionRead::Text { text, rects, read_only, clipped } => {
                (Some(text), if read_only { SelectionKind::ReadOnly } else { SelectionKind::Writable }, rects, clipped)
            }
            _ => (None, SelectionKind::None, Vec::new(), false),
        };
        let chars = self.settings.refine.context_chars;
        let before_cursor = RefineContext::new(String::new(), info.before_cursor.as_deref(), chars).before_cursor;
        let terminal = terminal_app(&app);
        let ctx = AskContext { app, before_cursor, selection, today: local_today() };
        ask::Taken { ctx, sel, clipped, rects, terminal, window: info.window }
    }

    /// An "Ask" question goes out (spec 2 §6.2): a first one starts a new conversation with its context — the one kept
    /// for its dictation when "restore" asks again; a follow-up joins the open one, and the answer shows the new
    /// question at once.
    fn ask(&mut self, id: DictationId, question: &str, follow_up: bool) {
        if !follow_up || self.conversation.is_none() {
            let taken = match self.ask_taken.get(id) {
                Some(kept) => kept,
                None => {
                    // Esc while recording skipped the read at the end of the recording (the flow was not thinking):
                    // "restore" reads the selection taken at the chord now, with the keys up for a Ctrl+C fallback.
                    if !self.ask_info.as_ref().is_some_and(|(i, _)| *i == id) {
                        self.read_ask_context(id);
                    }
                    let fresh = self.taken(id);
                    self.ask_taken.keep(id, &fresh);
                    fresh
                }
            };
            self.conversation = Some(ask::Conversation::new(taken));
        }
        if let Some(c) = self.conversation.as_mut() {
            c.ask(question);
        }
        if follow_up {
            self.send_view(true);
        }
        self.take_ask_sound(id);
        self.spawn_ask(id);
    }

    /// The sound of the question `id`, now that it goes out: its recording is final, and a follow-up may start
    /// recording before its answer ends. Only while `current.wav` is this question's own recording; it replaces the
    /// sound of a question asked before.
    fn take_ask_sound(&mut self, id: DictationId) {
        self.drop_ask_sound();
        if self.attempt.id != id || self.wav_of != Some(id) {
            return;
        }
        let sound = self.paths.current_wav();
        if let Some(snapshot) = self.history.ask_snapshot(&self.settings.history, &sound, local_time()) {
            self.ask_sound = Some((id, snapshot));
        }
    }

    /// The sound kept for a question goes: its turn will have no entry.
    fn drop_ask_sound(&mut self) {
        if let Some((_, snapshot)) = self.ask_sound.take() {
            self.history.recordings.discard(snapshot);
        }
    }

    /// The web search of "Ask", if on and if the model takes one.
    fn ask_search(&self) -> Option<engine_ask::SearchTool> {
        let a = &self.settings.ask;
        if a.web_search() { engine_ask::search_tool(a.max_searches()) } else { None }
    }

    /// The request of the conversation's last question, in a task of its own. The steps and the end come back as events
    /// of this task's generation: a dropped or replaced request's never reach the conversation.
    fn spawn_ask(&mut self, id: DictationId) {
        let Some(c) = self.conversation.as_ref() else { return };
        let messages = c.request();
        // The Ctrl+C copy of a first question, still on its way: the request task waits for it, never the core.
        let copy = match self.ask_copy.take() {
            Some((i, rx)) if i == id => c.question().map(|q| (rx, q.to_owned(), c.taken.ctx.clone())),
            _ => None,
        };
        let core = self.tx.clone();
        let model = self.settings.ask.model().to_owned();
        let search = self.ask_search();
        let key = keys::lookup(KeyName::Anthropic).unwrap_or_else(|e| {
            tracing::warn!("Anthropic key: {e}");
            None
        });
        let now = self.now();
        self.ask_warm.touched(now);
        if self.attempt.id == id {
            self.attempt.ask_started(now);
        }
        // Counts only: no text.
        tracing::info!(turns = messages.len(), search = search.is_some(), copy = copy.is_some(), "ask: asking");
        let (http, tx) = (self.http.clone(), self.tx.clone());
        let old = self.ask_task.start(id, |task| {
            // The wait for the copy is not this request's: should Esc drop the request, the copy still reaches the core.
            let copy = copy.map(|(rx, question, ctx)| (wait_for_copy(rx, id, task, core), question, ctx));
            tokio::spawn(async move {
                let mut messages = messages;
                if let Some((copied, question, ctx)) = copy
                    && let Ok(Some(text)) = copied.await
                {
                    messages = vec![ask::first_turn(&question, &ask::with_copied(&ctx, &text))];
                }
                let steps = tx.clone();
                let body = async {
                    let thinking = engine_ask::Thinking::DEFAULT;
                    engine_ask::run(&http, key.as_deref(), &model, search, thinking, &messages, move |step| {
                        let _ = steps.send(CoreEvent::AskStep { id, task, step });
                    })
                    .await
                };
                let result = guarded("ask", body, ask_panicked).await;
                let _ = tx.send(CoreEvent::AskDone { id, task, result });
            })
        });
        if let Some(old) = old {
            old.abort();
        }
    }

    /// The attempt of dictation `id`: the current one, or the one whose answer still prints while a follow-up records.
    fn attempt_for(&mut self, id: DictationId) -> Option<&mut Attempt> {
        if self.attempt.id == id { Some(&mut self.attempt) } else { self.answering.as_mut().filter(|a| a.id == id) }
    }

    fn on_ask_step(&mut self, id: DictationId, task: u64, step: engine_ask::Step) {
        if !self.ask_task.running(task) {
            return;
        }
        let now = self.now();
        if let engine_ask::Step::Spent(usage) = &step {
            if let Some(a) = self.attempt_for(id) {
                a.ask_spent(*usage);
            }
            return;
        }
        // Text that is not blank, or the start of a write (decision 6).
        if step.is_first_words()
            && let Some(a) = self.attempt_for(id)
        {
            a.first_words(now);
        }
        let Some(c) = self.conversation.as_mut() else { return };
        let (shows, opened) = (c.step(step), c.opened);
        if shows && !opened {
            self.open_answer();
            self.send_view(true);
        } else {
            self.send_view(false);
        }
        // Once per request, on its first visible step — a follow-up's and a retry's too. The flow takes it only while
        // it waits for the answer (thinking).
        if shows && self.ask_shown.first(task) {
            self.feed(Input::AskOpened { id });
        }
    }

    fn on_ask_done(&mut self, id: DictationId, task: u64, result: engine_ask::AskResult) {
        if !self.ask_task.finished(task) {
            return;
        }
        let now = self.now();
        if let Some(a) = self.attempt_for(id) {
            a.ask_done(result.usage, now);
        }
        // A write is shown, not pasted, while a follow-up is under way (decision 12: not under the user's hands) or CUE
        // is paused (no input is injected then), and when the window the question was asked in is not in front any more
        // (ruling 6b; the insert thread checks again right before the paste).
        let show_write = self.flow.shows_write(id);
        let Some(c) = self.conversation.as_mut() else { return };
        let (window, terminal) = (c.taken.window, c.taken.terminal);
        let heard = c.end(&result, show_write, |how, text| write_may_paste(window, terminal, how, text));
        let opened = c.opened;
        // The turn that ended goes to the history once the flow has heard of it (decision 2).
        let entry = ask_entry(heard, c, self.history.stamp(local_time()));
        // Its question's sound, taken as it was asked (`take_ask_sound`). A failed request may be asked again (Retry):
        // its sound waits — a question asked next, a close of the answer, drops it.
        let mut snapshot = self.ask_sound.take_if(|(i, _)| *i == id).map(|(_, s)| s);
        if matches!(heard, ask::Heard::Failed(_)) {
            self.ask_sound = snapshot.take().map(|s| (id, s));
        }
        // Kinds and counts only.
        tracing::info!(heard = ?heard, searches = result.usage.web_searches, "ask: done");
        if heard.shows() {
            if !opened {
                self.open_answer();
            }
            self.send_view(true);
        }
        let input = match heard {
            ask::Heard::Answered => Input::AskAnswered { id },
            ask::Heard::Write(how) => Input::AskWrite { id, how },
            ask::Heard::Failed(error) => Input::AskFailed { id, error },
        };
        self.feed(input);
        let written = self.history.add_ask(&self.settings.history, entry, snapshot);
        self.show_in_history(written);
    }

    /// Where the answer shows (spec 2 §7): the card at the selection when the selection is on screen and there is room for a
    /// card; otherwise the tray from the Drop.
    fn open_answer(&mut self) {
        let Some(c) = self.conversation.as_mut() else { return };
        let card = self.card.available().then(|| selection::bounds(&c.taken.rects)).flatten().and_then(|(first, all)| {
            let mon = placement::monitor_at(first.center())?;
            cardplace::card_place(first, all, mon, CARD)
        });
        match card {
            Some(p) => {
                c.open(AnswerPlace::Card);
                self.ui.card_place(CardPlaceView { below: p.below, arrow_x: p.arrow_x, max_height: p.max_height });
                self.card.show(p.rect);
            }
            None => c.open(AnswerPlace::Tray),
        }
        // The place only: where the selection is stays out of the log.
        tracing::info!(place = ?c.view.place, "ask: answer opened");
    }

    /// The answer view to the windows (spec 2 §7): at once for a new turn, an end, an opening or a follow-up row; while
    /// words stream, at most every `VIEW_EVERY_MS`, the rest with one flush.
    fn send_view(&mut self, urgent: bool) {
        let now = self.now();
        if !self.conversation.as_ref().is_some_and(|c| c.opened) {
            return;
        }
        let send = if urgent {
            self.view_gate.sent(now);
            ViewSend::Now
        } else {
            self.view_gate.change(now)
        };
        match send {
            ViewSend::Now => {
                if let Some(c) = self.conversation.as_ref() {
                    self.ui.answer(Some(&c.view));
                }
            }
            ViewSend::Later(after) => {
                let tx = self.tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(after)).await;
                    let _ = tx.send(CoreEvent::AnswerFlush);
                });
            }
            ViewSend::Wait => {}
        }
    }

    /// The answer closes (spec 2 §6.2: its conversation is forgotten).
    fn close_answer(&mut self) {
        if self.conversation.as_ref().is_some_and(|c| c.view.place == AnswerPlace::Card) {
            self.card.hide();
        }
        self.conversation = None;
        self.ui.answer(None);
        // No Retry of its last question any more.
        self.drop_ask_sound();
    }

    /// Insert, Insert below, Replace, or the model's write (spec 2 §6.2, §7). A write is its attempt's result
    /// (`Finished` commits it); an answer pasted with its button was counted when it was complete, so its text becomes
    /// the last result here, for Ctrl+Win+V (§4). Every paste goes only into the window the question was asked in
    /// (ruling 6b): the insert thread checks it right before the paste, and when it is not known nothing is pasted —
    /// the text stays in the clipboard, with the "Nowhere to paste" card.
    fn ask_paste(&mut self, id: DictationId, how: PasteHow) {
        let found = self.conversation.as_ref().and_then(|c| {
            Some((c.paste_text()?, c.has_write(), c.taken.window, c.taken.ctx.selection.clone()))
        });
        let Some((text, write, window, selection)) = found else {
            // Nothing to paste (a guard: a refused turn has no buttons). The flow waits in "inserting" for this answer,
            // and `InsertDone` reaches it only for a watched paste: unwatched, Esc would stay swallowed everywhere.
            tracing::warn!("ask: nothing to paste");
            self.watch_insert(id);
            let _ = self.tx.send(CoreEvent::InsertDone { id, outcome: InsertOutcome::NoField, at: Instant::now() });
            return;
        };
        let now = self.now();
        if let Some(a) = self.attempt_for(id) {
            a.insert_started(now);
        }
        if write {
            self.final_text = Some((id, text.clone()));
        } else {
            self.commit_text(&text);
        }
        self.watch_insert(id);
        let Some(window) = window else {
            tracing::info!("ask: the window the question was asked in is not known: text left in the clipboard");
            self.send_job(InsertJob::Leave { id: Some(id), text }, Some(id));
            return;
        };
        match how {
            PasteHow::Below => self.dispatch_below(text, id, window),
            PasteHow::Replace => self.dispatch_insert(text, Some(id), Some(window), selection.as_deref()),
            PasteHow::AtCaret => self.dispatch_insert(text, Some(id), Some(window), None),
        }
    }

    /// "Insert below" (spec 2 §7): the caret goes to the end of the selection (Right), then a line break and the text
    /// in one paste — an Enter would send a chat message. The insert thread adds the line break, only in `target`, and
    /// never in a terminal (`InsertJob::Paste`).
    fn dispatch_below(&mut self, text: String, id: DictationId, target: WindowRef) {
        let info = self.focus.query(0, FOCUS_TIMEOUT);
        self.note_app(&info.app);
        let text = trim_edges(&text).to_owned();
        let job = if info.editable == Editability::No {
            InsertJob::Leave { id: Some(id), text }
        } else {
            InsertJob::Paste { id: Some(id), text, collapse: true, target: Some(target) }
        };
        self.send_job(job, Some(id));
    }

    /// An answer pasted with its button becomes the last result (Ctrl+Win+V); it has no audio of its own.
    fn commit_text(&mut self, text: &str) {
        if let Err(e) = files::commit(&self.paths, text, Sound::Other) {
            tracing::warn!("last result not saved: {e}");
        }
        self.last_text = Some(text.to_owned());
    }

    /// Copy (spec 2 §7): the answer's plain text, into the clipboard the normal way.
    fn copy_answer(&mut self) {
        if let Some(text) = self.conversation.as_ref().and_then(|c| c.paste_text()) {
            self.send_job(InsertJob::Leave { id: None, text }, None);
        }
    }

    /// A click on a source chip (spec 2 §7): its page in the default browser. Only a web page (`openable`); on a short
    /// thread of its own, so the shell never holds the core.
    fn open_source(&self, n: u32) {
        let Some(url) = self.conversation.as_ref().and_then(|c| c.source_url(n)).map(str::to_owned) else { return };
        let spawned = std::thread::Builder::new().name("cue-open-source".into()).spawn(move || {
            if let Err(e) = open_url(&url) {
                tracing::warn!("source page did not open: {e}");
            }
        });
        if let Err(e) = spawned {
            tracing::warn!("source page not opened: {e}");
        }
    }

    fn set_timer(&mut self, kind: TimerKind, after: u64) {
        let tx = self.tx.clone();
        let old = self.timers.arm(kind, |generation| {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(after)).await;
                let _ = tx.send(CoreEvent::TimerFired { kind, generation });
            })
        });
        if let Some(task) = old {
            task.abort();
        }
    }

    fn hold_hint_shown(&mut self) {
        self.settings.hold_hints_left = self.settings.hold_hints_left.saturating_sub(1);
        self.save_settings();
    }

    fn save_settings(&self) {
        write_settings(&self.settings, &self.paths.settings, self.can_save_settings);
    }

    /// The tray card button arrives without a kind: it acts on the last tray card sent to the window (a click that
    /// coincides with a card change acts on the previous one, accepted). After the button the card closes, and the
    /// window does not report that itself.
    fn on_tray_action(&mut self) {
        let kind = self.tray.take();
        match kind {
            Some(TrayKind::NoField) => {
                if let Some(text) = self.last_result() {
                    self.send_job(InsertJob::Leave { id: None, text }, None);
                }
            }
            Some(TrayKind::MicSilent) => {
                if let Err(e) = std::process::Command::new("explorer.exe").arg(MIC_SETTINGS).spawn() {
                    tracing::warn!("microphone settings did not open: {e}");
                }
            }
            // "Undo" on a habit's card: the habit goes, and its forms are never learned again; a changed habit's card
            // brings back the habits it replaced, as they were; a merged card, the habit its draft added to.
            Some(TrayKind::NewHabit | TrayKind::HabitChanged) => {
                if let Some(card) = self.habit_card.take() {
                    self.undo_habit(card);
                }
            }
            Some(TrayKind::HoldHint | TrayKind::LastMinute | TrayKind::MicMissing | TrayKind::ModelReady | TrayKind::ModelSwitched) | None => {}
        }
        // A card with a done label ("Copied", "Removed") stays on the Drop while it shows and folds: nothing opens over
        // it until then.
        if matches!(kind, Some(TrayKind::NoField | TrayKind::NewHabit | TrayKind::HabitChanged)) {
            self.hold_tray();
        } else {
            self.show_notice();
        }
    }

    /// A habit's card undone (`HabitCard::undo`): a new habit is forgotten, its forms declined; a changed one goes the
    /// same way and the habits it replaced come back (`Habits::revert`), taking out a habit learned or widened since
    /// that goes back on one of them (its card, if one waits, goes too). A merged card brings back the habit its draft
    /// added to as it was, declining nothing. The main window redraws the section.
    fn undo_habit(&mut self, card: HabitCard) {
        let (merged, changed) = (card.was.is_some(), !card.replaced.is_empty());
        if let Some(removed) = card.undo(&mut self.settings.habits) {
            self.habit_cards.retain(|(_, c)| !removed.iter().any(|h| h.id == c.id));
            self.save_settings();
            self.send_settings();
            tracing::info!(merged, changed, removed = removed.len(), "habit undone");
        }
    }

    /// The card just acted on stays for `TRAY_DONE_MS`; then `TrayDone` of this hold opens what waits. A later hold, a
    /// card closed or another card sent makes this one stale.
    fn hold_tray(&mut self) {
        self.tray_gen += 1;
        let generation = self.tray_gen;
        self.tray_closing = Some(generation);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(TRAY_DONE_MS)).await;
            let _ = tx.send(CoreEvent::TrayDone { generation });
        });
    }

    fn on_finished(&mut self, id: DictationId, outcome: Outcome) {
        if let Some(a) = self.answering.take_if(|a| a.id == id) {
            // The follow-up's recording has replaced its audio: only its row (spec 2 §10) — and the text of a write it
            // pasted once that follow-up was dropped, which becomes the last result like any other.
            if files::last_files(outcome) == LastFiles::Commit {
                self.commit_last(id);
            }
            log_finished(outcome, &a);
            if needs_row(outcome, a.audio_ms()) {
                let rec = a.record(outcome, utc_now(), &self.settings);
                self.append_usage(rec);
            }
            return;
        }
        // A failed question may still be retried — its transcription from the saved audio needs what it was asked with.
        if outcome != Outcome::Failed {
            self.forget_ask_context(id);
        }
        // The attempt files (current.wav) and accounting belong to `self.attempt`: a result of another attempt is
        // a flow bug, and then we touch nothing.
        debug_assert_eq!(id, self.attempt.id, "Finished of the wrong attempt");
        if id != self.attempt.id {
            tracing::warn!("result of the wrong attempt: files and usage left untouched");
            return;
        }
        // A retry refines or transcribes again, and keeps the text as it was heard anew.
        let raw = self.raw_text.take_if(|(t, _)| *t == id);
        // Today counts the words of what a dictation or a translation wrote, before `commit_last` takes the text.
        let words = match self.attempt.mode {
            Mode::Dictate | Mode::Translate => {
                self.final_text.as_ref().filter(|(t, _)| *t == id).map_or(0, |(_, text)| usage::count_words(text))
            }
            Mode::Ask => 0,
        };
        let committed = match files::last_files(outcome) {
            LastFiles::Commit => {
                // The history takes the text before `commit_last` moves it out (decision 2).
                let stamp = self.history.stamp(local_time());
                let mut entry = dictation_entry(&self.attempt, self.final_text.as_ref(), raw.as_ref(), stamp);
                if let Some(e) = entry.as_mut()
                    && let Some(marks) = self.habits_applied.take(id)
                {
                    e.habits = marks;
                }
                let committed = self.commit_last(id);
                // Its recording only once `last\audio.wav` is really this dictation's sound (`committed`).
                let sound = self.paths.last_wav();
                let written = self.history.add_dictation(&self.settings.history, entry, committed, &sound);
                self.show_in_history(written);
                committed
            }
            LastFiles::KeepForRetry => false,
            LastFiles::Discard => {
                self.discard_sound();
                false
            }
        };
        log_finished(outcome, &self.attempt);
        if !needs_row(outcome, self.attempt.audio_ms()) {
            return;
        }
        let ts = utc_now();
        self.write_usage(outcome, &ts, words);
        // Only if this dictation's audio really became `last\audio.wav`; otherwise the previous one is there.
        if committed && self.settings.bench.save_recordings {
            self.save_for_bench(id, &ts);
        }
    }

    /// The attempt's text and audio become the last dictation — its own recording only (`wav_of`): an attempt that
    /// could not make its file has none, and the answer of a follow-up's question leaves the follow-up's alone. The text
    /// stays in memory whatever happens with the disk write: "Copy" and Ctrl+Win+V work even if the disk failed. `true`
    /// if the files were written.
    fn commit_last(&mut self, id: DictationId) -> bool {
        let Some((_, text)) = self.final_text.take_if(|(t, _)| *t == id) else {
            tracing::warn!("last dictation not saved: no final text");
            return false;
        };
        let sound = sound_of(self.wav_of, id);
        if sound != Sound::Other {
            self.wav_of = None;
        }
        let written = match files::commit(&self.paths, &text, sound) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("last dictation not saved: {e}");
                false
            }
        };
        self.last_text = Some(text);
        written
    }

    /// A result written to the history (decision 2) goes to the main window if it is open. Its recording is still
    /// being encoded: the window asks for it when the entry opens.
    fn show_in_history(&self, written: Option<history::Entry>) {
        if let Some(entry) = written {
            self.ui.history_added(&EntryView::of(&entry, false));
        }
    }

    fn write_usage(&mut self, outcome: Outcome, ts: &str, words: u32) {
        let mut rec = self.attempt.record(outcome, ts.to_owned(), &self.settings);
        rec.words = words;
        self.append_usage(rec);
    }

    /// One usage.csv row, after the rows that could not be written before it.
    fn append_usage(&mut self, mut rec: UsageRecord) {
        rec.day = local_today();
        let (path, settings) = (&self.paths.usage, &self.settings);
        // Refine's model, or the one "Ask" answered with.
        if self.usage.warn_no_price(&settings.pricing, &rec.refine_model) {
            tracing::warn!("usage: no price for model {} in the settings, its tokens counted as 0", rec.refine_model);
        }
        if let Err(e) = self.usage.write(rec, |r| usage::append(path, r, &settings.pricing)) {
            tracing::warn!(
                pending = self.usage.pending(),
                dropped = self.usage.dropped(),
                "usage: row not written ({e}), will append it with the next one"
            );
        }
    }

    /// Bench: the dictation audio and the app name go to `bench\recordings\`.
    fn save_for_bench(&self, id: DictationId, ts: &str) {
        let stem = format!("{}-{}", ts.replace(':', "-"), id.0);
        let dir = &self.paths.bench_recordings;
        let app = bench_json(&self.attempt.app);
        let saved = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::copy(self.paths.last_wav(), dir.join(format!("{stem}.wav"))))
            .and_then(|_| std::fs::write(dir.join(format!("{stem}.json")), app));
        if let Err(e) = saved {
            tracing::warn!("bench recording not saved: {e}");
        }
    }
}

/// When an entry came and its id (`history::Entry`): `ts` is the local time, `YYYY-MM-DDTHH:MM:SS.mmm`, and the id is
/// `ts-n`, `n` counting this run's entries, so two entries of one millisecond still have ids of their own.
struct Stamp {
    ts: String,
    id: String,
}

/// The history's writer (spec 3 §2): only the core writes. It counts this run's entries for their ids and remembers
/// the day it last deleted old days for, so the first entry of a new day deletes them first. The entries' recordings
/// are kept with them, never longer (`recordings`).
struct HistoryLog {
    dir: PathBuf,
    recordings: Recordings,
    count: u64,
    pruned: Option<String>,
}

impl HistoryLog {
    fn new(dir: PathBuf, recordings: Recordings) -> Self {
        Self { dir, recordings, count: 0, pruned: None }
    }

    /// The time and the id of the next entry, at the local time `ts`.
    fn stamp(&mut self, ts: String) -> Stamp {
        self.count += 1;
        Stamp { id: format!("{ts}-{}", self.count), ts }
    }

    /// At startup, before any entry: what a crash left half written among the recordings goes, then what the periods
    /// `h` no longer keep (`prune`).
    fn start(&mut self, h: &HistorySettings, today: &str) {
        self.recordings.sweep();
        self.prune(h, today);
    }

    /// Deletes the days the history's period no longer keeps, counted back from `today` (`YYYY-MM-DD`), and the
    /// recordings past their limit (`audio_limit`) — every one while the history or its recordings are off: at startup,
    /// on the first entry of a new day and when a period changes. Nothing while the history's period is unknown. An
    /// error is one log line.
    fn prune(&mut self, h: &HistorySettings, today: &str) {
        self.recordings.prune(h.audio_limit(), today);
        let Some(keep) = h.keep.known() else { return };
        self.pruned = Some(today.to_owned());
        match history::prune(&self.dir, keep, today) {
            Ok(0) => {}
            Ok(days) => tracing::info!(days, keep = keep.code(), "history: old days deleted"),
            Err(e) => tracing::warn!("history: old days not deleted: {e}"),
        }
    }

    /// Writes `entry` while the history is on (`h.keep`); `true` if it was written. Nothing at all while it is off or
    /// its period unknown. A disk error is one log line, without text.
    fn write(&mut self, h: &HistorySettings, entry: &history::Entry) -> bool {
        if h.keep.known().is_none_or(|keep| keep == Keep::Off) {
            return false;
        }
        let day = entry.ts.get(..10).unwrap_or_default();
        if self.pruned.as_deref() != Some(day) {
            self.prune(h, day);
        }
        match history::append(&self.dir, entry) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("history: entry not written: {e}");
                false
            }
        }
    }

    /// A dictation's or a translation's entry, and once it is written, its recording from `sound` (`last\audio.wav`)
    /// — only when `committed`: otherwise that file is still an earlier dictation's. The entry written, if it was.
    fn add_dictation(
        &mut self,
        h: &HistorySettings,
        entry: Option<history::Entry>,
        committed: bool,
        sound: &Path,
    ) -> Option<history::Entry> {
        let entry = entry.filter(|e| self.write(h, e))?;
        if committed && let Some(snapshot) = self.snapshot(h, sound, &entry.id) {
            self.recordings.encode(snapshot);
        }
        Some(entry)
    }

    /// The sound of an "Ask" question, from `sound` (`current.wav`), taken at the local time `ts` as the question goes
    /// out, before its turn has an entry: under an id of its own (`stamp`) that no entry gets, so a crash leaves it to the
    /// startup sweep; `add_ask` gives it to the turn's entry.
    fn ask_snapshot(&mut self, h: &HistorySettings, sound: &Path, ts: String) -> Option<Snapshot> {
        if !records(h) {
            return None;
        }
        let asked = self.stamp(ts);
        self.recordings.snapshot(sound, &asked.id)
    }

    /// An "Ask" turn's entry; the `snapshot` of its question's sound becomes the entry's recording once the entry is
    /// written, and is deleted otherwise. The entry written, if it was.
    fn add_ask(
        &mut self,
        h: &HistorySettings,
        entry: Option<history::Entry>,
        snapshot: Option<Snapshot>,
    ) -> Option<history::Entry> {
        let written = entry.filter(|e| self.write(h, e));
        match (snapshot, &written) {
            (Some(snapshot), Some(entry)) => self.recordings.encode_as(snapshot, &entry.id),
            (Some(snapshot), None) => self.recordings.discard(snapshot),
            (None, _) => {}
        }
        written
    }

    /// A snapshot of `sound` for the entry `id` while recordings are kept (`records`).
    fn snapshot(&self, h: &HistorySettings, sound: &Path, id: &str) -> Option<Snapshot> {
        records(h).then(|| self.recordings.snapshot(sound, id)).flatten()
    }

    /// "Clear history": every day goes, and every recording with them, whatever the periods say.
    fn clear(&self) {
        match history::clear(&self.dir) {
            Ok(()) => tracing::info!("history cleared"),
            Err(e) => tracing::warn!("history: not cleared: {e}"),
        }
        self.recordings.clear();
    }
}

/// Whose sound `current.wav` is for the attempt `id`, when `wav_of` made it (`Runtime::wav_of`).
fn sound_of(wav_of: Option<DictationId>, id: DictationId) -> Sound {
    match wav_of {
        Some(of) if of == id => Sound::Own,
        Some(_) => Sound::Other,
        None => Sound::Stale,
    }
}

/// Whether recordings are kept (`audio_period`): not while they or the history are off, nor while a period is unknown.
fn records(h: &HistorySettings) -> bool {
    h.audio_period().is_some_and(|keep| keep != AudioKeep::Off)
}

/// The history entry of a dictation or a translation that gave its text (decision 2): the text `commit_last` takes for
/// the attempt `a` (`final_text`) — the one pasted or left in the clipboard — its mode, the app it was recorded in
/// (empty when not known), and the text as it was heard (`raw`: for a translation, the speech before it was
/// translated). An "Ask" attempt writes none here: its turn was written when it ended (`ask_entry`), and an answer
/// pasted with its button is not a new result.
fn dictation_entry(
    a: &Attempt,
    final_text: Option<&(DictationId, String)>,
    raw: Option<&(DictationId, String)>,
    at: Stamp,
) -> Option<history::Entry> {
    let mode = match a.mode {
        Mode::Dictate => history::Mode::Dictation,
        Mode::Translate => history::Mode::Translation,
        Mode::Ask => return None,
    };
    let (_, text) = final_text.filter(|(id, _)| *id == a.id)?;
    let raw = raw.filter(|(id, _)| *id == a.id).map(|(_, raw)| raw.clone());
    let (Stamp { ts, id }, app) = (at, stored_app(&a.app));
    Some(history::Entry { id, ts, mode, app, text: text.clone(), question: None, raw, habits: Vec::new() })
}

/// The history entry of an "Ask" turn that ended (decision 2): the question, and the text the model wrote for the field
/// or the answer as a paste puts it in, without its Markdown (`Conversation::paste_text`), with the app the question
/// was asked in. A refusal, a failure and an answer without words write none. No text as it was heard: the question is
/// already that.
fn ask_entry(heard: ask::Heard, c: &ask::Conversation, at: Stamp) -> Option<history::Entry> {
    if !matches!(heard, ask::Heard::Answered | ask::Heard::Write(_)) {
        return None;
    }
    // A refusal is heard as an answer too: only a turn that is done has something to keep.
    let turn = c.view.turns.last().filter(|t| t.state == TurnState::Done)?;
    let text = c.paste_text()?;
    let (Stamp { ts, id }, app) = (at, stored_app(&c.taken.ctx.app));
    let question = Some(turn.question.clone());
    Some(history::Entry { id, ts, mode: history::Mode::Ask, app, text, question, raw: None, habits: Vec::new() })
}

/// Where a history entry's text goes (decision 3).
enum HistoryJob {
    /// Into `target` alone, the way Ctrl+Win+V pastes (`dispatch_insert`): the insert thread checks right before
    /// Ctrl+V that it is still in front, and otherwise leaves the text in the clipboard.
    Paste { text: String, target: WindowRef },
    /// Into the clipboard, like "Copy" on the Drop's tray card.
    Leave(InsertJob),
}

/// "Paste again" into the window that was under the main one, or the clipboard when there is no such window (and for
/// "Copy"). The flow waits for neither.
fn history_job(text: String, target: Option<WindowRef>) -> HistoryJob {
    match target {
        Some(target) => HistoryJob::Paste { text, target },
        None => HistoryJob::Leave(InsertJob::Leave { id: None, text }),
    }
}

/// The settings the core keeps: as read, except that with settings.json unreadable the history's period is unknown —
/// the defaults' month is not the user's choice — so the history is neither written nor cleaned up until the main
/// window sets it. Nothing of this reaches the file: an unreadable one is never written.
fn kept_settings(mut settings: Settings, can_save: bool) -> Settings {
    if !can_save {
        settings.history.keep = Period::Unknown(serde_json::Value::Null);
    }
    settings
}

/// One change from the main window (spec 3 §3) on the core's copy of the settings; `true` if a value changed. Only then
/// is the whole file written, and never when settings.json could not be read at startup (`can_save`): the change still
/// holds until CUE quits. The same value again, or one the settings refuse, changes and writes nothing.
fn apply_setting(settings: &mut Settings, change: SettingChange, path: &Path, can_save: bool) -> bool {
    let changed = settings.apply(change);
    if changed {
        write_settings(settings, path, can_save);
    }
    changed
}

/// The whole settings file at once, so the hint counter, the Drop's place and the main window's changes never
/// overwrite each other. Not when the file could not be read at startup: the user's file stays as it is.
fn write_settings(settings: &Settings, path: &Path, can_save: bool) {
    if !can_save {
        return;
    }
    if let Err(e) = settings::save(settings, path) {
        tracing::warn!("settings not saved: {e}");
    }
}

/// The other language of the translation pair (spec 2 §9): `translate.home`, else the Windows display language
/// (`windows`, a locale name), else Russian or English — never the target itself. `None`: no other language was found.
fn translate_pair(s: &Settings, windows: Option<&str>) -> Option<&'static str> {
    // The languages CUE is made for stand in when Windows names none.
    let hints = ["ru".to_owned(), "en".to_owned()];
    translate::resolve_home(s.translate.home.as_deref(), windows, &hints, s.translate.target())
}

/// The translation pair of the settings as they are now, with this Windows' display language. The log gets the codes.
fn pair_now(s: &Settings) -> Option<&'static str> {
    let home = translate_pair(s, lang::windows_language_code().as_deref());
    // Language codes only, no user text.
    tracing::info!(to = s.translate.target(), home = home.unwrap_or("none"), "translation pair");
    home
}

/// The app as the history keeps it: empty when not known.
fn stored_app(app: &str) -> String {
    known_app(app).unwrap_or_default().to_owned()
}

/// What refine does with the dictation. A translation goes the way the speech language says (spec 2 §5,
/// `translate::output_language`): the language fixed in `translate.speech`, else the one the text was heard in
/// (`heard`, from its alphabet; a code CUE does not know counts as unknown speech). Dictation and "Ask" are cleaned up.
fn job_for(
    mode: Mode,
    fixed: Option<&'static str>,
    heard: Option<&str>,
    target: &'static str,
    home: Option<&'static str>,
) -> Job {
    match mode {
        Mode::Translate => {
            let speech = fixed.or_else(|| heard.and_then(translate::known));
            Job::Translate(translate::output_language(speech, target, home))
        }
        Mode::Dictate | Mode::Ask => Job::Clean,
    }
}

/// Where a dictation's cleanup, a translation and a habit's rule go.
enum Route {
    /// The local model in the slot.
    Local(Arc<dyn Generate>),
    /// On this computer, no model ready yet: the raw text at once.
    NotReady,
    /// The cloud connection.
    Cloud,
}

fn route(runs: settings::Runs, slot: &llm::LlmSlot) -> Route {
    match (runs, slot.get()) {
        (settings::Runs::Cloud, _) => Route::Cloud,
        (settings::Runs::Local, Some(g)) => Route::Local(g),
        (settings::Runs::Local, None) => Route::NotReady,
    }
}

/// A recognition failure as the flow tells it: the model missing or damaged (Retry downloads it), a processor it cannot
/// run on (no Retry), anything else (Retry recognizes the saved recording again).
fn stt_error(e: &EngineError) -> SttError {
    match e {
        EngineError::Model(_) => SttError::Model,
        EngineError::Unsupported(_) => SttError::Unsupported,
        _ => SttError::Failed,
    }
}

/// The usage.csv time: UTC, like the providers' invoices.
fn utc_now() -> String {
    usage::utc_timestamp(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// Waits off the core for the Ctrl+C copy of the first question of dictation `id` (`COPY_WAIT`), tells the core
/// (`AskCopied`, before the request `task` can send a step), then hands it to that request if it still waits.
fn wait_for_copy(
    rx: std::sync::mpsc::Receiver<Option<String>>,
    id: DictationId,
    task: u64,
    core: UnboundedSender<CoreEvent>,
) -> tokio::sync::oneshot::Receiver<Option<String>> {
    let (to_request, copied) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let wait = tokio::task::spawn_blocking(move || rx.recv_timeout(COPY_WAIT).ok().flatten());
        let text = wait.await.ok().flatten();
        if let Some(text) = &text {
            let _ = core.send(CoreEvent::AskCopied { id, task, text: text.clone() });
        }
        let _ = to_request.send(text);
    });
    copied
}

/// Whether a write of the model may be pasted now (ruling 6b): the window the question was asked in (`window`; not known —
/// no) is in front and takes the text as `how` would paste it — after a line break below the selection, on one line in a
/// terminal the chord named (`terminal`) — by `clipboard::may_paste`. A short look at the window in front, on the core
/// thread; the insert thread looks again right before Ctrl+V.
fn write_may_paste(window: Option<WindowRef>, terminal: bool, how: PasteHow, text: &str) -> bool {
    let Some(target) = window else { return false };
    let pasted = match how {
        PasteHow::Below => format!("\n{text}"),
        _ if terminal => one_line(text),
        _ => text.to_owned(),
    };
    may_paste(&WinKeys, target, &pasted)
}

/// Today's local date, `YYYY-MM-DD` (spec 2 §6.1): it grounds searches, so a user east of UTC must not get yesterday.
/// The history's days are local days too (the main window's first page is today and yesterday).
pub(crate) fn local_today() -> String {
    let mut now = local_time();
    now.truncate(10);
    now
}

/// Local time minus UTC now, in minutes: Today puts the rows written before usage.csv kept their local day on theirs.
pub(crate) fn utc_offset_minutes() -> i32 {
    // SAFETY: no arguments; each returns a SYSTEMTIME by value.
    let (local, utc) = unsafe {
        (windows::Win32::System::SystemInformation::GetLocalTime(), windows::Win32::System::SystemInformation::GetSystemTime())
    };
    let minutes = |t: &windows::Win32::Foundation::SYSTEMTIME| {
        stats::days_from_civil(i64::from(t.wYear), u32::from(t.wMonth), u32::from(t.wDay)) * 1_440
            + i64::from(t.wHour) * 60
            + i64::from(t.wMinute)
    };
    i32::try_from(minutes(&local) - minutes(&utc)).unwrap_or(0)
}

/// The local time now, `YYYY-MM-DDTHH:MM:SS.mmm`: the time of a history entry, whose date names its day file.
fn local_time() -> String {
    // SAFETY: no arguments; returns a SYSTEMTIME by value.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

/// The rows an open answer shows for a follow-up: (listening, missed). Listening while it records (spec 2 §7); missed
/// while the error of one that could not be transcribed is up, with its Retry (decision 7). None once the answer
/// closed.
fn follow_up_rows(answer_open: bool, recording: bool, phase: UiPhase) -> (bool, bool) {
    (answer_open && recording, answer_open && phase == UiPhase::Error)
}

/// The Drop is at rest for a notice (the suggestion, a habit's card): idle, no tray card, no answer, and shown, so the
/// capsule can be clicked (paused or over a full-screen app the idle Drop is hidden and lets the mouse through).
fn at_rest(phase: UiPhase, tray: bool, answer_open: bool, paused: bool, hidden: bool) -> bool {
    phase == UiPhase::Idle && !tray && !answer_open && !paused && !hidden
}

/// A dictation records or is worked on (recognized, cleaned up, answered): CUE asks for full speed then.
fn works(phase: UiPhase) -> bool {
    matches!(phase, UiPhase::Start | UiPhase::Listen | UiPhase::Think)
}

/// Habits go with a dictation's cleanup while "Learn from me" is on: never with a translation or "Ask".
fn habits_for(mode: Mode, learn_on: bool, job: Job) -> bool {
    mode == Mode::Dictate && learn_on && matches!(job, Job::Clean | Job::Rewrite)
}

/// A value for each of the last few dictations, by id: a later dictation's never takes the place of an earlier one's
/// still waiting for its result. One whose result never came (cancelled, failed) goes once `KEPT` later ones came.
struct ByDictation<T>(VecDeque<(DictationId, T)>);

impl<T> Default for ByDictation<T> {
    fn default() -> Self {
        Self(VecDeque::new())
    }
}

impl<T> ByDictation<T> {
    const KEPT: usize = 4;

    /// The dictation's value, in place of one it had (a retry refines again).
    fn put(&mut self, id: DictationId, value: T) {
        self.0.retain(|(i, _)| *i != id);
        self.0.push_back((id, value));
        while self.0.len() > Self::KEPT {
            self.0.pop_front();
        }
    }

    fn take(&mut self, id: DictationId) -> Option<T> {
        let at = self.0.iter().position(|(i, _)| *i == id)?;
        self.0.remove(at).map(|(_, value)| value)
    }
}

/// A habit's card: the habit learned, and the habits it changed (docs/specs/2026-11-04-habits.md, "Changed"), none for a
/// new one. Its "Undo" brings those back as they were. `was`: the habit a merged draft added to, as it was before.
#[derive(Clone, Debug, PartialEq)]
struct HabitCard {
    id: String,
    replaced: Vec<habits::Habit>,
    was: Option<habits::Habit>,
}

impl HabitCard {
    fn kind(&self) -> TrayKind {
        if self.replaced.is_empty() { TrayKind::NewHabit } else { TrayKind::HabitChanged }
    }

    /// The change taken back without a trace ("Learn from me" off before its card): the habit goes undeclined, the one
    /// a merged draft added to comes back as it was, and so do the habits it replaced (`Habits::revert`). The habits
    /// that went for them; `None` when the habit is gone already.
    fn take_back(self, habits: &mut habits::Habits) -> Option<Vec<habits::Habit>> {
        let back: Vec<habits::Habit> = self.was.into_iter().chain(self.replaced).collect();
        habits.revert(&self.id, back, false)
    }

    /// The card's "Undo": a habit it learned goes and its forms are declined, the habits it replaced coming back
    /// (`Habits::revert`). A merged card learned no habit: the one its draft added to comes back as it was, with the
    /// habits it replaced, and nothing is declined (`take_back`). The habits that went for them; `None` when the habit
    /// is gone already.
    fn undo(self, habits: &mut habits::Habits) -> Option<Vec<habits::Habit>> {
        if self.was.is_some() {
            return self.take_back(habits);
        }
        habits.revert(&self.id, self.replaced, true)
    }
}

/// The habits a change replaced, for its card: one whose own card still waits comes back as that card would bring
/// back (the habit its draft added to as it was, then the habits it replaced), and that card goes. So the new card's
/// "Undo", or "Learn from me" off, takes back both changes.
fn carry(waiting: &mut VecDeque<(u64, HabitCard)>, replaced: Vec<habits::Habit>) -> Vec<habits::Habit> {
    let mut out = Vec::new();
    for h in replaced {
        match waiting.iter().position(|(_, c)| c.id == h.id).and_then(|at| waiting.remove(at)) {
            Some((_, card)) => out.extend(card.was.into_iter().chain(card.replaced)),
            None => out.push(h),
        }
    }
    out
}

/// The cards of habits that widened (`Counted::widened`: the id each had, the id of everywhere it has now) follow
/// them: the one open and those waiting.
fn follow(widened: &[(String, String)], open: Option<&mut HabitCard>, waiting: &mut VecDeque<(u64, HabitCard)>) {
    for card in waiting.iter_mut().map(|(_, c)| c).chain(open) {
        if let Some((_, all)) = widened.iter().find(|(had, _)| *had == card.id) {
            card.id.clone_from(all);
        }
    }
}

/// Which notice opens at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Offer,
    Habit,
}

/// The one found first of a waiting suggestion and a habit's card, each by the number it was found with; a card never
/// opens while the suggestion's capsule is open (`open`: with its ring or "undo"). A missed suggestion's dot does not
/// hold a card back.
fn first_notice(offer: Option<u64>, card: Option<u64>, open: bool) -> Option<Notice> {
    match (offer, card) {
        (Some(o), Some(c)) if c < o => (!open).then_some(Notice::Habit),
        (Some(_), _) => Some(Notice::Offer),
        (None, Some(_)) => (!open).then_some(Notice::Habit),
        (None, None) => None,
    }
}

/// A web page in the default browser (spec 2 §7), or a page of Windows Settings (`ms-settings:`, the welcome window's).
/// `ShellExecuteW` with the "open" verb takes the URL as one string: no command line of another program parses it.
/// Called on a thread of the caller's own: it sets up COM there.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    // SAFETY: ShellExecute asks for a COM apartment on its thread (this one is ours alone); the strings outlive the
    // call.
    let r = unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL)
    };
    // Above 32 — done; 32 or less — an error code.
    let code = r.0 as isize;
    if code > 32 { Ok(()) } else { Err(format!("ShellExecute {code}")) }
}

/// JSON of a bench recording. If the app is unknown, `"app": null`, not the "unknown" placeholder.
fn bench_json(app: &str) -> String {
    serde_json::json!({ "app": known_app(app) }).to_string()
}

/// `MicHandle::stop` waits for the microphone thread (the driver may hang), so not on the core thread. Late frames are
/// cut off by the gate (`MicGate`). If the thread did not start, the dropped `MicHandle` closes the microphone itself.
fn stop_mic_soon(m: MicHandle) {
    if let Err(e) = std::thread::Builder::new().name("cue-mic-stop".into()).spawn(move || m.stop()) {
        tracing::warn!("microphone: stop thread not started ({e}), the microphone will close by itself");
    }
}

fn abort_stt_task(task: Option<Option<JoinHandle<()>>>) {
    if let Some(Some(task)) = task {
        task.abort();
    }
}

/// The Soniox key of earlier versions is deleted from Windows Credential Manager: CUE does not use it any more. Every
/// start asks; after the first there is none. The log gets whether one was deleted.
fn forget_soniox_key() {
    match keys::forget_soniox() {
        Ok(true) => tracing::info!("the Soniox key of an earlier version was deleted"),
        Ok(false) => {}
        Err(e) => tracing::warn!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    /// Where a dictation's cleanup, a translation and a habit's rule go (docs/specs/2026-11-04-local-cleanup.md).
    mod route_of_the_cleanup {
        use super::super::*;
        use std::time::Instant;

        struct Here;
        impl cue_engines::local_llm::Generate for Here {
            fn generate(&self, _: &str, _: &str, _: &str, _: Instant, _: &std::sync::atomic::AtomicBool) -> Result<String, EngineError> {
                Ok(String::new())
            }
            fn id(&self) -> &str {
                "llm--here--model--q4_k_m"
            }
        }

        #[test]
        fn local_with_nothing_ready_pastes_raw_at_once() {
            let slot = llm::LlmSlot::default();
            assert!(matches!(route(settings::Runs::Local, &slot), Route::NotReady));
        }

        #[test]
        fn local_refine_uses_the_local_engine() {
            let slot = llm::LlmSlot::default();
            slot.ready(std::sync::Arc::new(Here));
            assert!(matches!(route(settings::Runs::Local, &slot), Route::Local(g) if g.id() == "llm--here--model--q4_k_m"));
        }

        #[test]
        fn back_to_cloud_goes_to_the_cloud_at_once() {
            let slot = llm::LlmSlot::default();
            slot.ready(std::sync::Arc::new(Here));
            assert!(matches!(route(settings::Runs::Cloud, &slot), Route::Cloud), "a loaded local model is not used");
        }
    }

    use super::*;
    use cue_core::settings::Cutoff;
    use cue_engines::ask::{AskEnd, AskFailure, AskResult, AskUsage, Step};

    #[test]
    fn recognition_failures_tell_the_flow_what_retry_does() {
        assert_eq!(stt_error(&EngineError::Model("x".into())), SttError::Model);
        assert_eq!(stt_error(&EngineError::Unsupported("x".into())), SttError::Unsupported);
        assert_eq!(stt_error(&EngineError::Timeout), SttError::Failed);
        assert_eq!(stt_error(&EngineError::Protocol("x".into())), SttError::Failed);
    }

    #[test]
    fn a_translation_goes_the_way_the_speech_language_says() {
        let tr = |fixed, heard, home| job_for(Mode::Translate, fixed, heard, "en", home);
        assert_eq!(tr(None, Some("ru"), Some("ru")), Job::Translate("en"));
        assert_eq!(tr(None, Some("en"), Some("ru")), Job::Translate("ru"), "target speech goes home");
        assert_eq!(tr(None, Some("en-US"), Some("ru")), Job::Translate("ru"), "a code goes through `known`");
        assert_eq!(tr(Some("ru"), Some("en"), Some("ru")), Job::Translate("en"), "the fixed language beats the heard one");
        assert_eq!(tr(Some("en"), Some("ru"), Some("ru")), Job::Translate("ru"));
        assert_eq!(tr(None, Some("xx"), Some("ru")), Job::Translate("en"), "an unknown code is unknown speech");
        assert_eq!(tr(None, None, Some("ru")), Job::Translate("en"));
        assert_eq!(tr(None, Some("en"), None), Job::Translate("en"), "no home language: target speech stays");
    }

    #[test]
    fn dictation_and_ask_are_cleaned_up() {
        for mode in [Mode::Dictate, Mode::Ask] {
            assert_eq!(job_for(mode, Some("ru"), Some("ru"), "en", Some("ru")), Job::Clean, "{mode:?}");
            assert_eq!(job_for(mode, None, None, "en", None), Job::Clean, "{mode:?}");
        }
    }

    #[test]
    fn hotkeys_map_to_flow_inputs() {
        assert_eq!(input_of(HotkeyEvent::ChordDown(Mode::Translate)), Input::ChordDown(Mode::Translate));
        assert_eq!(input_of(HotkeyEvent::ModeChange(Mode::Ask)), Input::ModeChange(Mode::Ask));
        assert_eq!(input_of(HotkeyEvent::ModeToggle), Input::ModeToggle);
        assert_eq!(input_of(HotkeyEvent::ChordUp), Input::ChordUp);
        assert_eq!(input_of(HotkeyEvent::ChordAborted), Input::ChordAborted);
        assert_eq!(input_of(HotkeyEvent::Esc), Input::Esc);
        assert_eq!(input_of(HotkeyEvent::PasteLast), Input::PasteLast);
    }

    #[test]
    fn today_is_a_local_date() {
        let d = local_today();
        let shape = d.char_indices().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
        assert!(d.len() == 10 && shape, "{d}");
    }

    /// Time zones run from UTC−12 to UTC+14, in steps of 15 minutes.
    #[test]
    fn the_utc_offset_is_a_time_zone() {
        let o = utc_offset_minutes();
        assert!((-720..=840).contains(&o) && o % 15 == 0, "{o}");
    }

    /// The time of a history entry: the local date, then the time to the millisecond.
    #[test]
    fn an_entry_time_is_local_to_the_millisecond() {
        let t = local_time();
        let separator = |i: usize| match i {
            4 | 7 => Some('-'),
            10 => Some('T'),
            13 | 16 => Some(':'),
            19 => Some('.'),
            _ => None,
        };
        let shape = t.char_indices().all(|(i, c)| separator(i).map_or(c.is_ascii_digit(), |s| c == s));
        assert!(t.len() == 23 && shape, "{t}");
        assert!(history::date_minus(&t[..10], 0).is_some(), "a real date: {t}");
    }

    /// A fresh directory for one test; a leftover of an earlier run is removed first.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-runtime-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A history writer in `dir`, its recordings in `dir\audio` waiting for the test to run their worker's jobs.
    fn log_in(dir: &Path) -> (HistoryLog, recordings::Queue) {
        let (recordings, queue) = Recordings::queued(dir.join("audio"));
        (HistoryLog::new(dir.to_owned(), recordings), queue)
    }

    fn at(ts: &str) -> Stamp {
        log_in(Path::new("")).0.stamp(ts.into())
    }

    /// The history kept for `keep`, its recordings for a week.
    fn keeping(keep: Keep) -> HistorySettings {
        HistorySettings { keep: Period::Set(keep), ..HistorySettings::default() }
    }

    fn attempt(mode: Mode, app: &str) -> Attempt {
        let mut a = Attempt::new(DictationId(7));
        a.mode = mode;
        a.app = app.into();
        a
    }

    #[test]
    fn stamps_count_this_runs_entries() {
        let (mut log, _) = log_in(Path::new(""));
        let first = log.stamp("2026-10-31T07:30:00.123".into());
        let second = log.stamp("2026-10-31T07:30:00.123".into());
        assert_eq!((first.ts.as_str(), first.id.as_str()), ("2026-10-31T07:30:00.123", "2026-10-31T07:30:00.123-1"));
        assert_eq!(second.id, "2026-10-31T07:30:00.123-2", "the same millisecond, an id of its own");
    }

    /// The text `commit_last` takes — the one given to the user, line breaks and edges as they are — with the attempt's
    /// mode and app, and the text as it was heard.
    #[test]
    fn dictation_entry_takes_the_committed_text_mode_and_app() {
        let a = attempt(Mode::Dictate, "Notepad");
        let committed = (DictationId(7), "Hello, world.\nSecond line ".to_owned());
        let heard = (DictationId(7), "uh hello world\nsecond line".to_owned());
        let e = dictation_entry(&a, Some(&committed), Some(&heard), at("2026-10-31T07:30:00.123")).unwrap();
        let expected = history::Entry {
            id: "2026-10-31T07:30:00.123-1".into(),
            ts: "2026-10-31T07:30:00.123".into(),
            mode: history::Mode::Dictation,
            app: "Notepad".into(),
            text: "Hello, world.\nSecond line ".into(),
            question: None,
            raw: Some("uh hello world\nsecond line".into()),
            habits: Vec::new(),
        };
        assert_eq!(e, expected);
        // Another attempt's text, or none, is not this attempt's result.
        let other = (DictationId(6), "earlier".to_owned());
        assert_eq!(dictation_entry(&a, Some(&other), Some(&heard), at("2026-10-31T07:30:00.123")), None);
        assert_eq!(dictation_entry(&a, None, Some(&heard), at("2026-10-31T07:30:00.123")), None);
        // Another attempt's text as heard is not this one's: the entry has none.
        let e = dictation_entry(&a, Some(&committed), Some(&other), at("2026-10-31T07:30:00.123")).unwrap();
        assert_eq!((e.text.as_str(), e.raw), ("Hello, world.\nSecond line ", None));
        assert_eq!(dictation_entry(&a, Some(&committed), None, at("2026-10-31T07:30:00.123")).unwrap().raw, None);
        // A write of "Ask" pasted at its end was written when its turn ended.
        let asked = attempt(Mode::Ask, "Notepad");
        assert_eq!(dictation_entry(&asked, Some(&committed), Some(&heard), at("2026-10-31T07:30:00.123")), None);
    }

    /// A translation keeps the speech before it was translated.
    #[test]
    fn translation_entry_is_marked_translation() {
        let a = attempt(Mode::Translate, "Telegram Desktop");
        let committed = (DictationId(7), "See you on Friday.".to_owned());
        let heard = (DictationId(7), "Увидимся в пятницу".to_owned());
        let e = dictation_entry(&a, Some(&committed), Some(&heard), at("2026-10-31T07:30:00.123")).unwrap();
        assert_eq!(e.mode, history::Mode::Translation);
        assert_eq!((e.app.as_str(), e.text.as_str(), e.question), ("Telegram Desktop", "See you on Friday.", None));
        assert_eq!(e.raw.as_deref(), Some("Увидимся в пятницу"));
    }

    fn asked_in(app: &str) -> ask::Conversation {
        let ctx = AskContext { app: app.into(), before_cursor: None, selection: None, today: "2026-10-31".into() };
        ask::Conversation::new(ask::Taken { ctx, ..Default::default() })
    }

    fn ended(end: AskEnd) -> AskResult {
        let content = vec![serde_json::json!({ "type": "text", "text": "x" })];
        AskResult { end, content, usage: AskUsage::default() }
    }

    /// A question and its answer as streamed, then ended with `end`; `may_paste` — the window asked in is in front.
    fn turn(c: &mut ask::Conversation, question: &str, words: &str, end: AskEnd, may_paste: bool) -> ask::Heard {
        c.ask(question);
        c.step(Step::TextStart);
        c.step(Step::Text(words.into()));
        c.end(&ended(end), false, |_, _| may_paste)
    }

    #[test]
    fn ask_turn_entry_has_the_question_and_the_plain_answer() {
        let mut c = asked_in("Telegram Desktop");
        let heard = turn(&mut c, "What is CUE?", "**CUE** is a dictation app.", AskEnd::Answer, true);
        assert_eq!(heard, ask::Heard::Answered);
        let e = ask_entry(heard, &c, at("2026-10-31T07:30:00.123")).unwrap();
        assert_eq!(e.mode, history::Mode::Ask);
        assert_eq!(e.question.as_deref(), Some("What is CUE?"));
        assert_eq!(e.text, "CUE is a dictation app.", "as a paste of the answer puts it in");
        assert_eq!(e.app, "Telegram Desktop");
        assert_eq!((e.id.as_str(), e.ts.as_str()), ("2026-10-31T07:30:00.123-1", "2026-10-31T07:30:00.123"));
        assert_eq!(e.raw, None, "the question is the text as it was heard");
        // A follow-up is an entry of its own, with its own question.
        let heard = turn(&mut c, "And who makes it?", "One person.", AskEnd::Answer, true);
        let e = ask_entry(heard, &c, at("2026-10-31T07:31:00.000")).unwrap();
        assert_eq!((e.question.as_deref(), e.text.as_str()), (Some("And who makes it?"), "One person."));
    }

    #[test]
    fn ask_write_entry_has_the_written_text() {
        let write = || AskEnd::Write { text: "I will send it on Thursday.".into(), replace: false };
        let mut c = asked_in("Notepad");
        let heard = turn(&mut c, "write that I will send it on Thursday", "", write(), true);
        assert!(matches!(heard, ask::Heard::Write(_)), "{heard:?}");
        let e = ask_entry(heard, &c, at("2026-10-31T07:30:00.123")).unwrap();
        assert_eq!(e.text, "I will send it on Thursday.");
        assert_eq!(e.question.as_deref(), Some("write that I will send it on Thursday"));
        // A write shown instead (the window asked in is not in front): the same text, as it was written.
        let mut c = asked_in("Notepad");
        let heard = turn(&mut c, "write that I will send it on Thursday", "", write(), false);
        assert_eq!(heard, ask::Heard::Answered);
        assert_eq!(ask_entry(heard, &c, at("2026-10-31T07:30:00.123")).unwrap().text, "I will send it on Thursday.");
    }

    #[test]
    fn refused_or_failed_turn_writes_no_entry() {
        let mut c = asked_in("Notepad");
        let heard = turn(&mut c, "q", "Partial words", AskEnd::Refused, true);
        assert_eq!(heard, ask::Heard::Answered, "the flow hears a refusal as an answer");
        assert_eq!(ask_entry(heard, &c, at("2026-10-31T07:30:00.123")), None);

        let mut c = asked_in("Notepad");
        let failed = AskEnd::Failed(AskFailure::Network);
        let heard = turn(&mut c, "q", "Words already shown", failed, true);
        assert!(matches!(heard, ask::Heard::Failed(_)), "{heard:?}");
        assert_eq!(ask_entry(heard, &c, at("2026-10-31T07:30:00.123")), None);

        // An answer without words has nothing to keep.
        let mut c = asked_in("Notepad");
        let heard = turn(&mut c, "q", "  ", AskEnd::Answer, true);
        assert_eq!(ask_entry(heard, &c, at("2026-10-31T07:30:00.123")), None);
    }

    #[test]
    fn unknown_app_is_stored_empty() {
        use cue_platform_win::focus::UNKNOWN_APP;
        let committed = (DictationId(7), "Hello.".to_owned());
        for app in [UNKNOWN_APP, "", "  "] {
            let a = attempt(Mode::Dictate, app);
            let dictation = dictation_entry(&a, Some(&committed), None, at("2026-10-31T07:30:00.123"));
            assert_eq!(dictation.unwrap().app, "", "{app:?}");
            let mut c = asked_in(app);
            let heard = turn(&mut c, "q", "An answer.", AskEnd::Answer, true);
            assert_eq!(ask_entry(heard, &c, at("2026-10-31T07:30:00.123")).unwrap().app, "", "{app:?}");
        }
    }

    fn dictated(ts: &str, text: &str) -> history::Entry {
        let a = attempt(Mode::Dictate, "Notepad");
        dictation_entry(&a, Some(&(DictationId(7), text.to_owned())), None, at(ts)).unwrap()
    }

    /// "Paste again" with no window under the main one, and "Copy": the text is left in the clipboard, and the flow
    /// waits for nothing. With a window, it is pasted there alone.
    #[test]
    fn history_text_without_target_is_left_in_the_clipboard() {
        match history_job("Hello.".into(), None) {
            HistoryJob::Leave(InsertJob::Leave { id: None, text }) => assert_eq!(text, "Hello."),
            _ => panic!("not left in the clipboard"),
        }
        let notepad = WindowRef { window: 0x10, process: 100, app_process: None };
        match history_job("Hello.".into(), Some(notepad)) {
            HistoryJob::Paste { text, target } => assert_eq!((text.as_str(), target), ("Hello.", notepad)),
            HistoryJob::Leave(_) => panic!("not pasted"),
        }
    }

    #[test]
    fn keep_off_writes_nothing() {
        let dir = tmp("keep-off");
        let (mut log, _) = log_in(&dir);
        let e = dictated("2026-10-31T07:30:00.123", "Hello.");
        assert!(!log.write(&keeping(Keep::Off), &e));
        assert!(!dir.exists(), "not even the directory");
        assert!(log.write(&keeping(Keep::Month), &e));
        assert_eq!(history::find(&dir, &e.id), Some(e));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// "Clear history" takes the recordings with the days, the whole folder, whatever is in it.
    #[test]
    fn clearing_the_history_clears_its_recordings() {
        let dir = tmp("clear");
        let audio = dir.join("audio");
        let (mut log, _) = log_in(&dir);
        let e = dictated("2026-10-31T07:30:00.123", "Hello.");
        assert!(log.write(&keeping(Keep::Month), &e));
        std::fs::create_dir_all(&audio).unwrap();
        for name in [history::audio_file(&e.id).unwrap(), "notes.txt".into()] {
            std::fs::write(audio.join(name), b"OggS").unwrap();
        }
        log.clear();
        assert_eq!(history::find(&dir, &e.id), None);
        assert!(!audio.exists(), "the recordings folder is gone");
        log.clear();
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Old days go at startup and with the first entry of each new day, not with every entry.
    #[test]
    fn the_first_entry_of_a_new_day_deletes_old_days() {
        let dir = tmp("new-day");
        for ts in ["2026-10-22T09:00:00.000", "2026-10-23T09:00:00.000"] {
            history::append(&dir, &dictated(ts, "old")).unwrap();
        }
        let (mut log, _) = log_in(&dir);
        let week = keeping(Keep::Week);
        log.prune(&week, "2026-10-30");
        let days = |dir: &Path| -> Vec<String> {
            history::older(dir, "2026-11-01", 10).days.into_iter().map(|d| d.date).collect()
        };
        assert_eq!(days(&dir), ["2026-10-23"], "a week back from the 30th keeps the 23rd");
        assert!(log.write(&week, &dictated("2026-10-30T10:00:00.000", "same day")));
        assert_eq!(days(&dir), ["2026-10-30", "2026-10-23"]);
        assert!(log.write(&week, &dictated("2026-10-31T00:00:01.000", "a new day")));
        assert_eq!(days(&dir), ["2026-10-31", "2026-10-30"], "a week back from the 31st no longer keeps the 23rd");
        // Within the day nothing more is deleted.
        history::append(&dir, &dictated("2026-10-01T09:00:00.000", "put back")).unwrap();
        assert!(log.write(&week, &dictated("2026-10-31T08:00:00.000", "later that day")));
        assert_eq!(days(&dir), ["2026-10-31", "2026-10-30", "2026-10-01"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// While the period is unknown — settings.json unreadable, or a value CUE does not know — the history is neither
    /// written nor cleaned up, at startup or on a new day, and no recording is made or deleted; once the user sets it
    /// in the main window, all resume.
    #[test]
    fn an_unknown_period_neither_writes_nor_prunes_until_it_is_set() {
        let dir = tmp("unknown-period");
        let old = dictated("2026-01-05T09:00:00.000", "Kept.");
        history::append(&dir, &old).unwrap();
        let old_recording = dir.join("audio").join(history::audio_file(&old.id).unwrap());
        std::fs::create_dir_all(dir.join("audio")).unwrap();
        std::fs::write(&old_recording, b"OggS").unwrap();
        let sound = dir.join("audio.wav");
        recordings::tone(&sound, 100);
        let unreadable = kept_settings(Settings::default(), false);
        let typo = kept_settings(serde_json::from_str(r#"{"history":{"keep":"forver"}}"#).unwrap(), true);
        for s in [unreadable, typo] {
            let h = &s.history;
            assert_eq!(h.keep.known(), None);
            let (mut log, queue) = log_in(&dir);
            log.start(h, "2026-10-30");
            let e = Some(dictated("2026-10-30T10:00:00.000", "Not kept."));
            assert_eq!(log.add_dictation(h, e, true, &sound), None);
            assert!(!log.write(h, &dictated("2026-10-31T00:00:01.000", "A new day.")));
            assert_eq!(queue.run(), 0, "nothing to encode or prune");
            assert_eq!(history::older(&dir, "2026-11-01", 10).days.len(), 1, "nothing written");
            assert_eq!(history::find(&dir, &old.id), Some(old.clone()), "nothing deleted");
            assert!(old_recording.exists(), "no recording deleted");
            assert_eq!(log.recordings.limit().get(), None, "the main window plays what is there");
        }
        let readable = kept_settings(Settings::default(), true);
        assert_eq!(readable.history.keep.known(), Some(Keep::Month), "no period in the file is a month");

        let mut s = kept_settings(Settings::default(), false);
        assert!(apply_setting(&mut s, SettingChange::HistoryKeep(Keep::Week), &dir.join("settings.json"), false));
        assert_eq!(s.history.keep.known(), Some(Keep::Week));
        let (mut log, queue) = log_in(&dir);
        log.prune(&s.history, "2026-10-31");
        assert_eq!(queue.run(), 1);
        assert_eq!(history::find(&dir, &old.id), None, "cleaned up with the period the user chose");
        assert!(!old_recording.exists(), "and its recording");
        let new = dictated("2026-10-31T10:00:00.000", "Kept now.");
        assert!(log.write(&s.history, &new));
        assert_eq!(history::find(&dir, &new.id), Some(new));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The recordings folder's file names, sorted.
    fn recorded(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir.join("audio"))
            .map(|l| l.map(|e| e.unwrap().file_name().into_string().unwrap()).collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// A dictation's sound is recorded only when `commit_last` made `last\audio.wav` its own and its entry was written:
    /// otherwise that file is an earlier dictation's, or there is no entry to keep it for.
    #[test]
    fn a_dictation_is_recorded_only_when_committed_and_written() {
        let dir = tmp("record-dictation");
        let sound = dir.join("audio.wav");
        recordings::tone(&sound, 500);
        let (mut log, queue) = log_in(&dir);
        let month = keeping(Keep::Month);
        log.prune(&month, "2026-10-31");
        assert_eq!(queue.run(), 1, "the day's prune, before its first entry");

        let e = dictated("2026-10-31T07:30:00.000", "Not committed.");
        assert_eq!(log.add_dictation(&month, Some(e.clone()), false, &sound), Some(e), "written all the same");
        assert_eq!((recorded(&dir), queue.run()), (Vec::<String>::new(), 0));
        let off = keeping(Keep::Off);
        assert_eq!(log.add_dictation(&off, Some(dictated("2026-10-31T07:31:00.000", "Off.")), true, &sound), None);
        assert_eq!(log.add_dictation(&month, None, true, &sound), None);
        assert_eq!((recorded(&dir), queue.run()), (Vec::<String>::new(), 0));

        // The history's folder cannot be made: the entry is not written, and its sound is not taken.
        std::fs::write(dir.join("blocked"), b"").unwrap();
        let (others, blocked_queue) = Recordings::queued(dir.join("audio"));
        let mut blocked = HistoryLog::new(dir.join("blocked"), others);
        let e = dictated("2026-10-31T07:32:00.000", "Not written.");
        assert_eq!(blocked.add_dictation(&month, Some(e), true, &sound), None);
        blocked_queue.run();
        assert_eq!(recorded(&dir), Vec::<String>::new());

        let e = dictated("2026-10-31T07:33:00.000", "Kept.");
        assert_eq!(log.add_dictation(&month, Some(e.clone()), true, &sound), Some(e.clone()));
        assert_eq!(recorded(&dir), [history::audio_snapshot(&e.id).unwrap()], "the snapshot, on the core thread");
        assert!(sound.exists(), "the last dictation's sound stays where it is");
        assert_eq!(queue.run(), 1);
        assert_eq!(recorded(&dir), [history::audio_file(&e.id).unwrap()]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// How long a recording plays, in ms: the last page's position less the pre-skip, at 48 kHz.
    fn recorded_ms(file: &Path) -> u64 {
        let bytes = std::fs::read(file).unwrap();
        let last = bytes.windows(4).rposition(|w| w == b"OggS").unwrap();
        (u64::from_le_bytes(bytes[last + 6..last + 14].try_into().unwrap()) - 312) / 48
    }

    /// An "Ask" question's sound is taken as the question goes out, under an id of its own the startup sweep knows: a
    /// follow-up that starts recording before the answer ends leaves it whole. It becomes the recording of the turn's
    /// entry once that is written; it goes when the entry is not, and a clear meanwhile drops it. Nothing is taken
    /// while recordings or the history are off.
    #[test]
    fn an_ask_question_keeps_the_sound_it_was_asked_with() {
        let dir = tmp("record-ask");
        let paths = Paths::new(&dir.join("Roaming"), &dir.join("Local"));
        std::fs::create_dir_all(&paths.last_dir).unwrap();
        let (mut log, queue) = log_in(&dir);
        let month = keeping(Keep::Month);
        log.prune(&month, "2026-10-31");
        assert_eq!(queue.run(), 1, "the day's prune, before its first entry");
        let mut c = asked_in("Notepad");

        recordings::tone(&paths.current_wav(), 400);
        let snapshot = log.ask_snapshot(&month, &paths.current_wav(), "2026-10-31T07:30:00.000".into());
        let taken = recorded(&dir);
        assert_eq!(history::audio_temps(taken.iter().map(String::as_str)).len(), 1, "{taken:?}");
        // A follow-up starts recording while the answer prints (`open_mic`).
        files::discard(&paths).unwrap();
        recordings::tone(&paths.current_wav(), 100);
        let heard = turn(&mut c, "What is CUE?", "A dictation app.", AskEnd::Answer, true);
        let entry = ask_entry(heard, &c, log.stamp("2026-10-31T07:30:05.000".into()));
        assert_eq!(entry.as_ref().unwrap().id, "2026-10-31T07:30:05.000-2", "no entry has the question's own id");
        assert_eq!(log.add_ask(&month, entry.clone(), snapshot), entry);
        assert_eq!(queue.run(), 1);
        let kept = history::audio_file(&entry.unwrap().id).unwrap();
        assert_eq!(recorded(&dir), [kept.as_str()]);
        assert_eq!(recorded_ms(&dir.join("audio").join(&kept)), 400, "the question's sound, not the follow-up's");

        // The history's folder cannot be made: the entry is not written, and the snapshot taken for it goes.
        std::fs::write(dir.join("blocked"), b"").unwrap();
        let (others, blocked_queue) = Recordings::queued(dir.join("audio"));
        let mut blocked = HistoryLog::new(dir.join("blocked"), others);
        let snapshot = blocked.ask_snapshot(&month, &paths.current_wav(), "2026-10-31T07:31:00.000".into());
        assert!(snapshot.is_some());
        let entry = ask_entry(heard, &c, blocked.stamp("2026-10-31T07:31:05.000".into()));
        assert_eq!(blocked.add_ask(&month, entry, snapshot), None);
        blocked_queue.run();
        assert_eq!(recorded(&dir), [kept.as_str()], "no snapshot left behind");

        // Cleared while the answer printed: never encoded, and the folder is not made again.
        let snapshot = log.ask_snapshot(&month, &paths.current_wav(), "2026-10-31T07:32:00.000".into());
        assert!(snapshot.is_some());
        log.clear();
        let entry = ask_entry(heard, &c, log.stamp("2026-10-31T07:32:05.000".into()));
        assert!(log.add_ask(&keeping(Keep::Month), entry, snapshot).is_some());
        queue.run();
        assert!(!dir.join("audio").exists());

        // Off, or the history off: no sound is taken, and no id is used up.
        let off = HistorySettings { keep: Period::Set(Keep::Month), audio: Period::Set(AudioKeep::Off) };
        for h in [off, keeping(Keep::Off)] {
            assert!(log.ask_snapshot(&h, &paths.current_wav(), "2026-10-31T07:33:00.000".into()).is_none());
        }
        assert_eq!(log.stamp("2026-10-31T07:34:00.000".into()).id, "2026-10-31T07:34:00.000-5");
        assert!(!dir.join("audio").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Only an attempt's own recording becomes the last dictation's sound.
    #[test]
    fn the_last_dictation_takes_only_its_own_sound() {
        let (one, two) = (DictationId(1), DictationId(2));
        assert_eq!(sound_of(Some(one), one), Sound::Own);
        assert_eq!(sound_of(Some(two), one), Sound::Other, "a follow-up's, still in use");
        assert_eq!(sound_of(None, one), Sound::Stale, "its file could not be made");
    }

    /// No sound is taken while recordings or the history are off, or a period is unknown.
    #[test]
    fn nothing_is_recorded_while_off_or_unknown() {
        let dir = tmp("record-off");
        let sound = dir.join("audio.wav");
        recordings::tone(&sound, 100);
        let (log, queue) = log_in(&dir);
        let unknown = serde_json::json!("fortnite");
        let with_audio = |audio| HistorySettings { keep: Period::Set(Keep::Month), audio };
        for h in [
            with_audio(Period::Set(AudioKeep::Off)),
            with_audio(Period::Unknown(unknown.clone())),
            HistorySettings { keep: Period::Set(Keep::Off), audio: Period::Set(AudioKeep::Month) },
            HistorySettings { keep: Period::Unknown(unknown.clone()), audio: Period::Set(AudioKeep::Month) },
        ] {
            assert!(log.snapshot(&h, &sound, "2026-10-31T07:30:00.000-1").is_none(), "{h:?}");
        }
        assert!(!dir.join("audio").exists() && queue.run() == 0);
        for audio in [AudioKeep::Day, AudioKeep::Week, AudioKeep::Month] {
            assert!(log.snapshot(&with_audio(Period::Set(audio)), &sound, "2026-10-31T07:30:00.000-1").is_some());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// At startup what a crash left half written goes before anything else, then what the periods no longer keep; a
    /// period changed to off deletes every recording at once, and drops an encoding still waiting.
    #[test]
    fn startup_sweeps_then_prunes_and_off_clears_at_once() {
        let dir = tmp("record-start");
        let audio = dir.join("audio");
        std::fs::create_dir_all(&audio).unwrap();
        let old = history::audio_file("2026-10-20T09:00:00.000-1").unwrap();
        let kept = history::audio_file("2026-10-30T09:00:00.000-2").unwrap();
        let left = history::audio_snapshot("2026-10-30T09:00:00.000-3").unwrap();
        for name in [old.as_str(), &kept, &left, "notes.tmp"] {
            std::fs::write(audio.join(name), b"x").unwrap();
        }
        let (mut log, queue) = log_in(&dir);
        let h = HistorySettings { keep: Period::Set(Keep::Year), audio: Period::Set(AudioKeep::Week) };
        log.start(&h, "2026-10-31");
        assert_eq!(recorded(&dir), [old.as_str(), &kept, "notes.tmp"], "swept at once");
        assert_eq!(queue.run(), 1);
        assert_eq!(recorded(&dir), [kept.as_str(), "notes.tmp"], "pruned on the worker");
        assert_eq!(log.recordings.limit().get(), Some(AudioKeep::Week.cutoff()));

        let sound = dir.join("audio.wav");
        recordings::tone(&sound, 100);
        let e = dictated("2026-10-31T07:30:00.000", "Hello.");
        assert!(log.add_dictation(&h, Some(e.clone()), true, &sound).is_some());
        let off = HistorySettings { audio: Period::Set(AudioKeep::Off), ..h };
        log.prune(&off, "2026-10-31");
        assert!(!audio.exists(), "off: the folder is gone at once");
        assert_eq!(queue.run(), 2, "the encoding waiting, then the folder set aside");
        assert!(!audio.exists(), "the encoding waiting was dropped");
        let names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with("audio.cleared")), "nothing set aside is left: {names:?}");
        assert_eq!(log.recordings.limit().get(), Some(Cutoff::All));
        assert_eq!(history::find(&dir, &e.id), Some(e), "the entry stays");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn target_change_recomputes_the_translation_pair() {
        let path = tmp("pair").join("settings.json");
        let windows = Some("ru-RU");
        let mut s = Settings::default();
        assert_eq!((s.translate.target(), translate_pair(&s, windows)), ("en", Some("ru")));
        assert!(apply_setting(&mut s, SettingChange::TranslateTarget("de".into()), &path, true));
        assert_eq!((s.translate.target(), translate_pair(&s, windows)), ("de", Some("ru")));
        // The target is the Windows language now: the pair takes the first other language hint.
        assert!(apply_setting(&mut s, SettingChange::TranslateTarget("ru".into()), &path, true));
        assert_eq!((s.translate.target(), translate_pair(&s, windows)), ("ru", Some("en")));
        let home = translate_pair(&s, windows);
        assert_eq!(job_for(Mode::Translate, None, Some("en"), s.translate.target(), home), Job::Translate("ru"));
        assert_eq!(job_for(Mode::Translate, None, Some("ru"), s.translate.target(), home), Job::Translate("en"));
        // A home language in the settings comes first; no Windows language: the hints.
        s.translate.home = Some("uk".into());
        assert_eq!(translate_pair(&s, windows), Some("uk"));
        s.translate.home = None;
        assert_eq!(translate_pair(&s, None), Some("en"));
        assert_eq!(settings::load(&path).unwrap().translate.target(), "ru", "written at once");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unchanged_setting_is_not_saved() {
        let path = tmp("unchanged").join("settings.json");
        let mut s = Settings::default();
        let same_or_refused = [
            SettingChange::HistoryKeep(Keep::Month),
            SettingChange::TranslateTarget("en".into()),
            SettingChange::TranslateTarget(" EN ".into()),
            SettingChange::UiLanguage(settings::UiLanguage::Auto),
            SettingChange::MicDevice(None),
            SettingChange::MicDevice(Some("   ".into())),
            // Refused: a language CUE does not know, a name no microphone has.
            SettingChange::TranslateTarget("xx".into()),
            SettingChange::MicDevice(Some("m".repeat(201))),
        ];
        for change in same_or_refused {
            let shown = format!("{change:?}");
            assert!(!apply_setting(&mut s, change, &path, true), "{shown}");
        }
        assert_eq!(s, Settings::default());
        assert!(!path.exists(), "nothing written");
        assert!(apply_setting(&mut s, SettingChange::HistoryKeep(Keep::Week), &path, true));
        assert_eq!(settings::load(&path).unwrap().history.keep.known(), Some(Keep::Week));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    /// settings.json that could not be read at startup is never replaced: the changes hold until CUE quits.
    #[test]
    fn unreadable_settings_apply_but_never_save() {
        let dir = tmp("unreadable");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        let mut s = Settings::default();
        assert!(apply_setting(&mut s, SettingChange::HistoryKeep(Keep::Week), &path, false));
        assert!(apply_setting(&mut s, SettingChange::TranslateTarget("de".into()), &path, false));
        assert_eq!((s.history.keep.known(), s.translate.target()), (Some(Keep::Week), "de"), "the changes hold");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json", "the user's file stays as it is");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "no other file either");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The listening row while a follow-up records, the missed row while it could not be transcribed (decision 7); none
    /// once the answer has closed.
    #[test]
    fn the_answer_shows_a_follow_ups_rows_only_while_it_is_open() {
        assert_eq!(follow_up_rows(true, true, UiPhase::Start), (true, false));
        assert_eq!(follow_up_rows(true, true, UiPhase::Listen), (true, false));
        assert_eq!(follow_up_rows(true, false, UiPhase::Think), (false, false));
        assert_eq!(follow_up_rows(true, false, UiPhase::Error), (false, true));
        assert_eq!(follow_up_rows(true, false, UiPhase::Answer), (false, false));
        assert_eq!(follow_up_rows(false, true, UiPhase::Listen), (false, false), "a dictation records");
        assert_eq!(follow_up_rows(false, false, UiPhase::Error), (false, false), "the Drop's own error");
    }

    /// A suggestion opens only on a Drop that can be seen and clicked: idle, no tray card, no answer, not paused, not
    /// hidden over a full-screen app.
    #[test]
    fn a_suggestion_opens_only_on_the_shown_idle_drop() {
        assert!(at_rest(UiPhase::Idle, false, false, false, false));
        assert!(!at_rest(UiPhase::Done, false, false, false, false));
        assert!(!at_rest(UiPhase::Idle, true, false, false, false), "a tray card");
        assert!(!at_rest(UiPhase::Idle, false, true, false, false), "an answer");
        assert!(!at_rest(UiPhase::Idle, false, false, true, false), "paused");
        assert!(!at_rest(UiPhase::Idle, false, false, false, true), "over a full-screen app");
        for p in [UiPhase::Start, UiPhase::Listen, UiPhase::Think] {
            assert!(works(p), "{p:?}");
        }
        for p in [UiPhase::Idle, UiPhase::Done, UiPhase::Empty, UiPhase::Cancel, UiPhase::Error, UiPhase::Answer] {
            assert!(!works(p), "{p:?}");
        }
    }

    /// The habits a dictation's cleanup took, and their marks, stay that dictation's.
    #[test]
    fn what_a_dictation_sent_is_kept_for_it_alone() {
        let mut by = ByDictation::default();
        by.put(DictationId(1), "first");
        by.put(DictationId(2), "second");
        assert_eq!(by.take(DictationId(1)), Some("first"), "a later dictation does not take an earlier one's place");
        assert_eq!(by.take(DictationId(1)), None, "taken once");
        by.put(DictationId(2), "again");
        assert_eq!(by.take(DictationId(2)), Some("again"), "a retry's in place of the first try's");
        for i in 3..10 {
            by.put(DictationId(i), "later");
        }
        assert_eq!(by.0.len(), ByDictation::<&str>::KEPT, "one whose result never came does not stay");
        assert_eq!(by.take(DictationId(9)), Some("later"));
    }

    /// Habits go into a dictation's cleanup with "Learn from me" on, and into nothing else.
    #[test]
    fn habits_go_only_with_a_dictations_cleanup() {
        assert!(habits_for(Mode::Dictate, true, Job::Clean));
        assert!(habits_for(Mode::Dictate, true, Job::Rewrite));
        assert!(!habits_for(Mode::Dictate, false, Job::Clean), "learn off");
        assert!(!habits_for(Mode::Dictate, true, Job::Translate("en")), "a translation's job");
        assert!(!habits_for(Mode::Translate, true, Job::Clean), "translate mode");
        assert!(!habits_for(Mode::Ask, true, Job::Clean), "Ask");
    }

    /// The suggestion and a habit's card at rest: the one found first; a card never while the capsule is open.
    #[test]
    fn the_notice_found_first_opens_first() {
        assert_eq!(first_notice(Some(1), Some(2), false), Some(Notice::Offer));
        assert_eq!(first_notice(Some(3), Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(None, Some(2), true), None, "the capsule is open");
        assert_eq!(first_notice(None, Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(Some(1), None, false), Some(Notice::Offer));
        assert_eq!(first_notice(None, None, false), None);
    }

    /// A habit's card says "New habit", or "Habit changed" when the habit took the place of others.
    #[test]
    fn a_card_that_replaced_a_habit_says_it_changed() {
        let fresh = HabitCard { id: "h1".into(), replaced: Vec::new(), was: None };
        assert_eq!(fresh.kind(), TrayKind::NewHabit);
        let old = habits::Habits::default().adopt(draft(vec![habits::Form::Dash], "Word"), None, "en");
        assert_eq!(HabitCard { id: "h2".into(), replaced: vec![old], was: None }.kind(), TrayKind::HabitChanged);
    }

    fn secret_habits() -> habits::Habits {
        habits::Habits { secret: habits::Secret::from_bytes([7; habits::SECRET_BYTES]), ..Default::default() }
    }
    fn draft(forms: Vec<habits::Form>, app: &str) -> habits::Draft {
        habits::Draft { forms, app: app.into(), before: "a".into(), after: "b".into(), n: 3 }
    }
    fn card_of(l: habits::Learned) -> HabitCard {
        HabitCard { id: l.habit.id, replaced: l.replaced, was: l.was }
    }
    const STOP: habits::Form = habits::Form::EndMark { mark: '.', removed: true };
    const ADDED: habits::Form = habits::Form::EndMark { mark: '.', removed: false };

    /// "Undo" on a card that learned a habit: the habit goes and its forms are declined; a changed habit's card brings
    /// back the habits it replaced, as they were.
    #[test]
    fn undo_on_a_card_that_learned_a_habit_forgets_it_and_brings_back_what_it_replaced() {
        let mut h = secret_habits();
        let r = h.adopt(draft(vec![STOP], "Telegram"), None, "en");
        let card = card_of(h.learn(draft(vec![ADDED], "Telegram"), None, "en"));
        assert_eq!((card.kind(), card.replaced.as_slice()), (TrayKind::HabitChanged, std::slice::from_ref(&r)));
        assert_eq!(card.undo(&mut h), Some(Vec::new()));
        assert_eq!(h.list, std::slice::from_ref(&r));
        assert_eq!(h.declined, [ADDED.hash(&h.secret)]);
        let card = card_of(h.learn(draft(vec![habits::Form::Dash], "Word"), None, "en"));
        assert_eq!(card.kind(), TrayKind::NewHabit);
        assert_eq!(card.undo(&mut h), Some(Vec::new()));
        assert_eq!(h.list, [r]);
        assert_eq!(h.declined, [ADDED.hash(&h.secret), habits::Form::Dash.hash(&h.secret)]);
    }

    /// "Undo" on a merged card: the habit the draft added to comes back as it was (its count, app and rule), with the
    /// habits the card replaced, and nothing is declined: that habit was never undone. "Learn from me" off before the
    /// card does the same.
    #[test]
    fn undo_on_a_merged_card_restores_the_habit_it_added_to_and_declines_nothing() {
        let mut h = secret_habits();
        let x = h.adopt(draft(vec![STOP], "Telegram"), None, "en");
        let y = h.adopt(draft(vec![ADDED], "Word"), None, "en");
        let merged = h.learn(draft(vec![STOP], "Telegram"), None, "en");
        assert!(merged.merged && merged.habit.n == 6);
        // A merged card that changed a habit too, as a hand-written settings.json can make it: y went for it.
        h.list.retain(|h| h.id != y.id);
        let card = HabitCard { replaced: vec![y.clone()], ..card_of(merged) };
        assert_eq!(card.kind(), TrayKind::HabitChanged);
        let mut off = h.clone();
        assert_eq!(card.clone().undo(&mut h), Some(Vec::new()));
        assert_eq!(h.list, [x, y]);
        assert!(h.declined.is_empty());
        assert_eq!(card.take_back(&mut off), Some(Vec::new()));
        assert_eq!(off, h);
    }

    /// A change of a habit whose card still waits: that card goes, and the new card carries its chain, so its "Undo",
    /// or "Learn from me" off, brings back what was there before both.
    #[test]
    fn a_change_of_a_habit_whose_card_waits_carries_that_cards_chain() {
        let mut h = secret_habits();
        let lower = habits::Form::FirstLetter { lower: true };
        let r = h.adopt(draft(vec![lower], "Telegram"), None, "en");
        let a = h.learn(draft(vec![habits::Form::FirstLetter { lower: false }, STOP], "Telegram"), None, "en");
        assert_eq!(a.replaced, std::slice::from_ref(&r));
        let mut cards = VecDeque::from([(1, card_of(a))]);
        let b = h.learn(draft(vec![ADDED], "Telegram"), None, "en");
        assert_eq!(b.replaced.len(), 1, "it changed the habit whose card waits");
        let card = HabitCard { replaced: carry(&mut cards, b.replaced.clone()), ..card_of(b) };
        assert!(cards.is_empty(), "that card goes");
        assert_eq!((card.kind(), card.replaced.as_slice()), (TrayKind::HabitChanged, std::slice::from_ref(&r)));
        let mut off = h.clone();
        assert_eq!(card.clone().take_back(&mut off), Some(Vec::new()));
        assert_eq!((off.list.as_slice(), off.declined.len()), (std::slice::from_ref(&r), 0));
        assert_eq!(card.undo(&mut h), Some(Vec::new()));
        assert_eq!(h.list, std::slice::from_ref(&r));
        assert_eq!(h.declined, [ADDED.hash(&h.secret)]);
        // A merged card's habit as it was comes first, then what it replaced; a habit with no card waiting stays.
        let w = h.adopt(draft(vec![habits::Form::Dash], "Word"), None, "en");
        let mut cards = VecDeque::from([(2, HabitCard { id: "h2".into(), replaced: vec![r.clone()], was: Some(w.clone()) })]);
        let mut two = r.clone();
        two.id = "h2".into();
        assert_eq!(carry(&mut cards, vec![two, w.clone()]), [w.clone(), r, w]);
    }

    /// The cards of a habit that widened follow it to the id of everywhere.
    #[test]
    fn the_cards_of_a_widened_habit_follow_its_new_id() {
        let card = |id: &str| HabitCard { id: id.into(), replaced: Vec::new(), was: None };
        let mut open = card("h1");
        let mut waiting = VecDeque::from([(1, card("h2")), (2, card("h3"))]);
        let widened = [("h1".to_owned(), "h9".to_owned()), ("h3".to_owned(), "h9".to_owned())];
        follow(&widened, Some(&mut open), &mut waiting);
        assert_eq!(open.id, "h9");
        assert_eq!(waiting.iter().map(|(_, c)| c.id.as_str()).collect::<Vec<_>>(), ["h2", "h9"]);
    }

    #[test]
    fn bench_json_has_null_for_unknown_app() {
        use cue_platform_win::focus::UNKNOWN_APP;
        assert_eq!(bench_json(UNKNOWN_APP), r#"{"app":null}"#);
        assert_eq!(bench_json(""), r#"{"app":null}"#);
        assert_eq!(bench_json("Блокнот"), r#"{"app":"Блокнот"}"#);
    }
}
