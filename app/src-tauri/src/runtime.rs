//! The app core, an actor. Events (keys, the Drop window, microphone, engines, timers, power) → the `Flow`
//! state machine → effects → execution. Only `Flow` makes decisions; here live the microphone, transcription,
//! refine, insert, timers, the window and the tray icon, the last-dictation files and usage accounting.
//!
//! Executor rules:
//! - timers: each kind has its own generation (`state::Timers`), a stale firing never reaches `Flow`;
//! - microphone frames are tagged with the dictation id and accepted only until its `StopMic` (`state::MicGate`);
//!   stopping the microphone itself (waiting for its thread) happens off the core thread;
//! - the transcription result (`SttFinal` / `SttFailed`) goes to `Flow` only after `FinishStt` / `TranscribeSaved`;
//!   each transcription and refine task has its own generation (`state::TaskSlot`): the result of a cancelled or
//!   replaced task is not accepted; a panic inside a task still yields a result (`tasks::guarded`);
//! - an insert always ends: the insert thread is time-bounded, and on top of that there is a 10 s watchdog
//!   (`state::InsertWatch`);
//! - "retry" is a new attempt of the same dictation: `Finished` arrives twice, each attempt gets its own usage.csv
//!   row with its own outcome; a retry resets the tokens, latencies and transcription path, the audio and the app
//!   stay (`attempt`). After a failed translation "retry" refines the saved text again, and its row has no audio.
//!   An attempt without a stream (no Soniox key, the stream did not connect) is billed at the stream price: an
//!   overestimate in a rare failure, accepted;
//! - the last-dictation files (`last\`) change only on success (`files`); the last text is also kept in memory;
//! - a prompt cache warm-up (refine's `warm_cache`, "Ask"'s `warm_ask_cache`) is paid for and gets a usage.csv row of
//!   its own (`warmup`);
//! - a dictation over a short selection sends it with its cleanup and pastes over it with its edges (spec 2 §12.13); it
//!   reads the selection only through the focus query — never a Ctrl+C;
//! - "Ask" (spec 2 §6–7): the context is taken at the chord, on the focus thread; the request streams in a task of its
//!   own (`ask_task`, with a generation like the others); the answer view goes to the windows at once for a new turn,
//!   an end or an opening, and at most every 50 ms while words stream (`state::ViewGate`); the flow hears once per
//!   request that its answer shows (`state::ShownOnce`). The attempt of an answer still printing when a follow-up
//!   starts waits aside (`answering`) and gets its own row when it ends;
//! - the log gets events, per-stage latencies and error codes; no text, no audio, no keys. Never `Effect` or `Input`
//!   with `{:?}`: their `Debug` prints text.
mod ask;
mod attempt;
mod files;
mod insert;
mod state;
mod tasks;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cue_core::ask::{AnswerPlace, AskContext, MAX_SELECTION_CHARS, SelectionKind};
use cue_core::audio::{Silence, SilenceDetector};
use cue_core::flow::{DictationId, Effect, Flow, InsertOutcome, Input, Outcome, PasteHow, SttError, TimerKind};
use cue_core::refine::{self, RefineContext};
use cue_core::settings::{self, DropPosition, Settings};
use cue_core::translate;
use cue_core::ui::{Mode, TrayKind, UiAction, UiPhase, UiState};
use cue_core::usage::{self, SttPath, UsageRecord};
use cue_engines::EngineError;
use cue_engines::ask as engine_ask;
use cue_engines::claude::{self, Job, RefineResult};
use cue_engines::keys::{self, KeyName};
use cue_engines::soniox::SonioxConfig;
use cue_engines::stt::{self, Engine, Fallback, SttOutput, SttSession};
use cue_platform_win::cardplace::{self, CARD};
use cue_platform_win::chord::HotkeyEvent;
use cue_platform_win::clipboard::{WinKeys, cloud_clipboard_uploads, may_paste};
use cue_platform_win::focus::{Editability, FocusInfo, FocusWorker, SelectionRead, WindowRef};
use cue_platform_win::hook;
use cue_platform_win::mic::{self, MicEvent, MicHandle};
use cue_platform_win::selection::{copies_line, needs_copy_fallback};
use cue_platform_win::terminal::terminal_app;
use cue_platform_win::wav::WavSink;
use cue_platform_win::{placement, selection};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::card_window::Card;
use crate::ipc::{CardPlaceView, Ui};
use crate::lang;
use crate::paths::Paths;
use crate::trayicon::TrayLook;
use attempt::{Attempt, UsageLog, log_finished, needs_row, warmup_record};
use files::LastFiles;
use insert::{InsertJob, first_known_app, insert_app, known_app, one_line, pasted_text, trim_edges};
use state::{CacheWarm, InsertWatch, LevelGate, MicGate, ShownOnce, TaskSlot, Timers, ViewGate, ViewSend, WarmUp};
use tasks::{ask_panicked, guarded, refine_panicked, stt_panicked};

/// Timeout for each UI Automation query.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(150);
/// The focus query of a dictation at the second press, with the start of the selection (spec 2 §12.13): a few more UI
/// Automation calls than the text before the caret alone. It runs while the speech is transcribed.
const DICTATION_FOCUS_TIMEOUT: Duration = Duration::from_millis(250);
/// The focus query of an "Ask" question (spec 2 §6.1): the selection of a long document and its rectangles take longer
/// than the text before the caret.
const ASK_FOCUS_TIMEOUT: Duration = Duration::from_millis(500);
/// How long the request task waits for the Ctrl+C fallback (keys up ≤ 1.5 s, the app's copy ≤ 0.4 s): the core never
/// waits.
const COPY_WAIT: Duration = Duration::from_secs(2);
/// Text before the caret on insert: the join (`join_for_insert`) needs only its end. A short read is a
/// quick query that is rarely late (a late one answers "unknown").
const INSERT_CONTEXT_CHARS: usize = 16;
/// How long we wait for the Soniox stream result after the second press; then batch transcription of the recording.
const STT_FINISH_TIMEOUT: Duration = Duration::from_secs(5);
/// Insert watchdog: the insert thread finishes within ~7 s (see `insert::spawn_inserter`).
const INSERT_WATCHDOG: Duration = Duration::from_secs(10);
/// Warm-up: a request with no key and no text, just for a TLS connection in the shared client's pool. The response
/// (a refusal) is not needed. The API address, not the site root: the root could redirect to another host.
const WARM_URL: &str = claude::API_URL;
const WARM_TIMEOUT: Duration = Duration::from_secs(5);
const MIC_SETTINGS: &str = "ms-settings:privacy-microphone";

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
    TimerFired { kind: TimerKind, generation: u64 },
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

/// The Soniox stream of the recording in progress. The key is needed again for the batch fallback.
/// No `Debug`: it holds the key.
struct Stream {
    id: DictationId,
    session: SttSession,
    key: String,
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
    /// The other language of the translation pair (`translate::resolve_home`), decided once at startup like the
    /// interface language. `None`: no language other than the target was found, and target speech is only cleaned up.
    translate_home: Option<&'static str>,
    paths: Paths,
    /// One client for the whole app (refine, batch transcription, warm-up): a shared connection pool.
    http: reqwest::Client,
    focus: Arc<FocusWorker>,
    insert: Option<std::sync::mpsc::Sender<InsertJob>>,
    timers: Timers<JoinHandle<()>>,
    mic_gate: MicGate,
    mic: Option<MicHandle>,
    wav: Option<WavSink>,
    silence: SilenceDetector,
    /// The silence detector said "there is sound" at least once: if the microphone fails, what was said is processed.
    heard: bool,
    stream: Option<Stream>,
    /// The transcription result in flight (stream + fallback, or "retry"): `AbortStt` cancels it too.
    /// Without a task (there was no stream) it is `None`: the result was sent at once.
    stt_task: TaskSlot<Option<JoinHandle<()>>>,
    refine_task: TaskSlot<JoinHandle<()>>,
    /// The focused field, queried right after the second press, for the refine context.
    refine_focus: Option<(DictationId, std::sync::mpsc::Receiver<FocusInfo>)>,
    /// The selection the cleanup of dictation `id` carried, as the field reported it: the paste goes over it with its edges.
    woven: Option<(DictationId, String)>,
    insert_watch: InsertWatch,
    /// The bare final text of the dictation (no join, line breaks not replaced), for `last\result.txt`.
    final_text: Option<(DictationId, String)>,
    /// The last text given to the user: "Copy" and Ctrl+Win+V take it from here, after a restart from disk.
    last_text: Option<String>,
    /// The last real app name from the field queries (for the terminal safeguard when the query on insert
    /// was late).
    last_app: String,
    /// The last tray card sent to the window: its button acts on it.
    tray: Option<TrayKind>,
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
}

impl Runtime {
    pub fn new(
        ui: Ui,
        card: Card,
        settings: Settings,
        can_save_settings: bool,
        paths: Paths,
        tx: UnboundedSender<CoreEvent>,
    ) -> Self {
        let has_last = paths.last_text().exists();
        let insert = match insert::spawn_inserter(tx.clone()) {
            Ok(tx) => Some(tx),
            Err(e) => {
                tracing::error!("insert thread not started: {e}");
                None
            }
        };
        let target = settings.translate.target();
        let translate_home = translate::resolve_home(
            settings.translate.home.as_deref(),
            lang::windows_language_code().as_deref(),
            &settings.stt.language_hints,
            target,
        );
        // Language codes only, no user text.
        tracing::info!(to = target, home = translate_home.unwrap_or("none"), "translation pair");
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
            paths,
            http: reqwest::Client::new(),
            focus: Arc::new(FocusWorker::spawn()),
            insert,
            timers: Timers::default(),
            mic_gate: MicGate::default(),
            mic: None,
            wav: None,
            silence: SilenceDetector::new(),
            heard: false,
            stream: None,
            stt_task: TaskSlot::default(),
            refine_task: TaskSlot::default(),
            refine_focus: None,
            woven: None,
            insert_watch: InsertWatch::default(),
            final_text: None,
            last_text: None,
            last_app: String::new(),
            tray: None,
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
        }
    }

    /// The core loop. Blocking calls (UI Automation up to 150 ms, the key store) run right here,
    /// so `run` spins on its own thread (`block_on`), not in the shared tokio pool.
    pub async fn run(mut self, mut rx: UnboundedReceiver<CoreEvent>) {
        self.sync_hook_flags();
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

    /// `StartStt` runs last in its batch: the UI Automation query (the app name for Soniox)
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
            CoreEvent::Input(i) => self.feed(i),
            CoreEvent::UiAction(UiAction::TrayAction) => self.on_tray_action(),
            CoreEvent::UiAction(UiAction::TrayClose) => self.tray = None,
            // The flow does not change: the runtime copies the answer.
            CoreEvent::UiAction(UiAction::AnswerCopy) => self.copy_answer(),
            CoreEvent::UiAction(a) => {
                let now = self.now();
                if is_restore(&a, self.flow.ui_state().phase) {
                    // The read queued at Esc saw the field as it was then: the cleanup after "restore" reads it anew.
                    self.refine_focus = None;
                    if self.attempt.id == self.flow.current_id() {
                        self.attempt.undo(now);
                    }
                }
                self.feed(Input::Ui(a));
            }
            CoreEvent::UiReady => {
                tracing::info!("Drop window ready");
                let s = self.flow.ui_state();
                self.emit_state(&s);
                self.ui.tray(self.tray);
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
                    self.feed(Input::Inserted { id, outcome });
                }
            }
            CoreEvent::InsertTimeout { id, generation } => {
                if self.insert_watch.expired(id, generation) {
                    tracing::warn!("insert did not finish within {} s: \"no field\"", INSERT_WATCHDOG.as_secs());
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
        }
    }

    fn on_mic(&mut self, id: DictationId, ev: MicEvent) {
        if !self.mic_gate.accepts(id) {
            return;
        }
        match ev {
            MicEvent::Frame { pcm, peak_dbfs, level } => {
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
            MicEvent::Failed(e) => {
                tracing::warn!(heard = self.heard, "microphone: {e}");
                self.feed(Input::MicFailed { heard: self.heard });
            }
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
                    self.attempt.speech_language = out.language;
                }
                Input::SttFinal { id, text: out.text }
            }
            Err(e) => {
                tracing::warn!(stt_path = path.as_str(), "transcription failed: {e}");
                let error = if e.is_key_problem() { SttError::KeyProblem } else { SttError::Network };
                Input::SttFailed { id, error }
            }
        };
        self.feed(input);
    }

    fn on_refined(&mut self, id: DictationId, task: u64, result: RefineResult) {
        if !self.refine_task.finished(task) {
            // The result of a cancelled refine (Esc in "Thinking"): after "restore" we wait for the new request.
            return;
        }
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.refine_done(result.usage, result.outcome, now);
        }
        // A Claude key problem (`RawKeyProblem`) is already in the log: `claude::refine` itself writes one line
        // for each failed refine, whatever the insert outcome.
        self.feed(Input::Refined { id, text: result.text, outcome: result.outcome });
    }

    fn exec(&mut self, e: Effect) {
        match e {
            Effect::OpenMic { id } => self.open_mic(id),
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
        }
    }

    /// The only place the state goes out to the window and the tray icon; the voice-level rule lives here too.
    fn emit_state(&mut self, s: &UiState) {
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
        // A new recording forgets the selection the last one's cleanup carried.
        self.woven = None;
        // A new recording ends any "restore" or retry of an earlier question: what it was asked with is not kept.
        self.clear_ask_context();
        self.silence = SilenceDetector::new();
        self.heard = false;
        // The attempt's audio goes to current.wav; a file left from the previous attempt is replaced.
        self.wav = match WavSink::create(&self.paths.current_wav()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("recording audio is not written to disk: {e}");
                None
            }
        };
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
            Ok(_) => self.mic = Some(mic::open(events)),
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
        if !keep && let Err(e) = files::discard(&self.paths) {
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
        if let Some(old) = self.stream.take() {
            old.session.abort();
        }
        let Ok(key) = soniox_key() else { return };
        let cfg = SonioxConfig {
            model: self.settings.stt.soniox_model.clone(),
            // The starting mode: Shift may still toggle it while recording, which language identification covers.
            language_hints: soniox_hints(self.flow.mode(), &self.settings),
            // "Unknown" is not an app name: no context.
            app: known_app(&self.attempt.app).map(str::to_owned),
            terms: vec![],
        };
        let session = stt::start(Engine::Soniox { key: key.clone(), cfg });
        self.stream = Some(Stream { id, session, key });
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
        let Some(Stream { session, key, .. }) = self.stream.take_if(|s| s.id == id) else {
            // The stream was not started (no Soniox key, or the key store failed; the reason is already in the log).
            let old = self.stt_task.start(id, |task| {
                let result = Err(EngineError::KeyProblem("transcription stream not started: no Soniox key".into()));
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path: SttPath::Stream });
                None
            });
            abort_stt_task(old);
            return;
        };
        let fallback = self.fallback(key);
        let wav = self.paths.current_wav();
        let old = self.stt_task.start(id, |task| {
            Some(tokio::spawn(async move {
                let body = async {
                    match session.finish(STT_FINISH_TIMEOUT).await {
                        Err(e) if stt::should_fallback(&e) => {
                            tracing::warn!("transcription stream: {e}; transcribing the whole recording");
                            (fallback.transcribe(&wav).await, SttPath::StreamThenBatch)
                        }
                        other => (other, SttPath::Stream),
                    }
                };
                // If the task crashed: "No connection" and "retry"; the attempt's audio stays in current.wav.
                let (result, path) = guarded("transcription", body, || (Err(stt_panicked()), SttPath::Stream)).await;
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path });
            }))
        });
        abort_stt_task(old);
    }

    fn abort_stt(&mut self, id: DictationId) {
        if let Some(s) = self.stream.take_if(|s| s.id == id) {
            s.session.abort();
        }
        // A cancelled task drops both the wait for the stream and the fallback (which cleans up its own Soniox upload).
        abort_stt_task(self.stt_task.take_for(id));
    }

    fn transcribe_saved(&mut self, id: DictationId) {
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.retry(now);
        }
        self.warm_up();
        self.prefetch_focus(id);
        let fallback = soniox_key().map(|key| self.fallback(key));
        let wav = self.paths.current_wav();
        let tx = self.tx.clone();
        let old = self.stt_task.start(id, |task| {
            Some(tokio::spawn(async move {
                let body = async {
                    match fallback {
                        Ok(fb) => fb.transcribe(&wav).await,
                        Err(e) => Err(e),
                    }
                };
                let result = guarded("retry transcription", body, || Err(stt_panicked())).await;
                let _ = tx.send(CoreEvent::SttDone { id, task, result, path: SttPath::Batch });
            }))
        });
        abort_stt_task(old);
    }

    fn fallback(&self, soniox_key: String) -> Fallback {
        Fallback {
            http: self.http.clone(),
            soniox_key,
            model: self.settings.stt.soniox_async_model.clone(),
            // The mode at the second press or the retry: the one the dictation ends with.
            hints: soniox_hints(self.flow.mode(), &self.settings),
        }
    }

    /// Connect to Anthropic ahead of time: when recording starts and on the second press (in parallel with
    /// transcription), at most once every 20 s (right away after sleep) and only if there is a key: refine does not pay
    /// for the TLS handshake inside its 1.5 s budget.
    fn warm_up(&mut self) {
        if !self.warm.due(self.now()) || !matches!(keys::lookup(KeyName::Anthropic), Ok(Some(_))) {
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
        let model = self.settings.refine.model.clone();
        if self.flow.mode() == Mode::Ask || !claude::can_warm(&model) || !self.cache_warm.due(self.now()) {
            return;
        }
        let Ok(Some(key)) = keys::lookup(KeyName::Anthropic) else { return };
        let (http, tx, terms) = (self.http.clone(), self.tx.clone(), self.settings.dictionary.terms.clone());
        tokio::spawn(async move {
            if let Some(usage) = claude::warm(&http, &key, &model, &terms).await {
                let _ = tx.send(CoreEvent::Warmed { usage, model });
            }
        });
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

    /// The focused field (with the text before the caret and the start of the selection, spec 2 §12.13), right after the
    /// second press, in parallel with transcription: by the time the text arrives the refine context is ready.
    fn prefetch_focus(&mut self, id: DictationId) {
        let (tx, rx) = std::sync::mpsc::channel();
        let focus = Arc::clone(&self.focus);
        let chars = self.settings.refine.context_chars;
        let mode = self.flow.mode();
        let timeout = if mode == Mode::Ask { FOCUS_TIMEOUT } else { DICTATION_FOCUS_TIMEOUT };
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(dictation_focus(&focus, chars, mode, timeout));
        });
        self.refine_focus = Some((id, rx));
    }

    fn focus_for_refine(&mut self, id: DictationId, mode: Mode) -> FocusInfo {
        match self.refine_focus.take() {
            // The query is limited to 250 ms (150 ms in "Ask"); if it did not come, "unknown" (refine without context).
            Some((f, rx)) if f == id => rx.recv_timeout(FOCUS_TIMEOUT * 2).unwrap_or_default(),
            // A retried translation and "restore" read the field, and so the selection, anew.
            _ => dictation_focus(&self.focus, self.settings.refine.context_chars, mode, FOCUS_TIMEOUT),
        }
    }

    fn refine(&mut self, id: DictationId, raw: String, mode: Mode) {
        let info = self.focus_for_refine(id, mode);
        self.note_app(&info.app);
        // The query was late ("unknown"): use the app at the start of recording.
        let app = first_known_app(&[&info.app, &self.attempt.app]).to_owned();
        // A short selection goes with the cleanup (spec 2 §12.13). The context's one value decides what goes and so whether
        // the paste goes over it: the tag, the block, the check and the paste never disagree.
        let chars = self.settings.refine.context_chars;
        let offered = offered_selection(&info.selection, chars);
        let ctx = RefineContext::new(app, info.before_cursor.as_deref(), chars).with_selection(offered);
        if let Some(sent) = ctx.sent_selection() {
            // Its length only.
            tracing::info!(selection_chars = sent.chars().count(), "refine: the selection goes with it");
        }
        self.woven = ctx.sent_selection().and(offered).map(|read| (id, read.to_owned()));
        // No key: `claude::refine` writes the log line itself and returns the raw text.
        let key = keys::lookup(KeyName::Anthropic).unwrap_or_else(|e| {
            tracing::warn!("Anthropic key: {e}");
            None
        });
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
        let (http, s, tx) = (self.http.clone(), self.settings.refine.clone(), self.tx.clone());
        let terms = self.settings.dictionary.terms.clone();
        let old = self.refine_task.start(id, |task| {
            tokio::spawn(async move {
                let fallback_raw = raw.clone();
                // One future for both jobs, so `guarded` covers a translation too.
                let body = async {
                    match job {
                        Job::Clean => claude::refine(&http, key.as_deref(), &s.model, &raw, &ctx, &terms, &s).await,
                        Job::Translate(to) => {
                            claude::translate(&http, key.as_deref(), &s.model, &raw, &ctx, &terms, &s, to).await
                        }
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
        let replaced = self.woven.take_if(|(i, _)| *i == id).map(|(_, s)| s);
        self.dispatch_insert(text, Some(id), None, replaced.as_deref());
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
    /// that window (`InsertJob::Paste`); `replaced` — the selection the text goes over: "Ask"'s Replace, or a dictation
    /// that sent it.
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
        self.spawn_ask(id);
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
        let (messages, container) = (c.request(), c.container.clone());
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
                    engine_ask::run(&http, key.as_deref(), &model, search, thinking, &messages, container, move |step| {
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
        if let Err(e) = files::commit(&self.paths, text) {
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

    /// The whole settings file at once, so the hint counter and the Drop's place never overwrite each other. Not when
    /// the file could not be read at startup: the user's file stays as it is.
    fn save_settings(&self) {
        if !self.can_save_settings {
            return;
        }
        if let Err(e) = settings::save(&self.settings, &self.paths.settings) {
            tracing::warn!("settings not saved: {e}");
        }
    }

    /// The tray card button arrives without a kind: it acts on the last tray card sent to the window (a click that
    /// coincides with a card change acts on the previous one, accepted). After the button the card closes, and the
    /// window does not report that itself.
    fn on_tray_action(&mut self) {
        match self.tray.take() {
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
            Some(TrayKind::HoldHint | TrayKind::LastMinute) | None => {}
        }
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
        let committed = match files::last_files(outcome) {
            LastFiles::Commit => self.commit_last(id),
            LastFiles::KeepForRetry => false,
            LastFiles::Discard => {
                if let Err(e) = files::discard(&self.paths) {
                    tracing::warn!("recording audio not deleted: {e}");
                }
                false
            }
        };
        log_finished(outcome, &self.attempt);
        if !needs_row(outcome, self.attempt.audio_ms()) {
            return;
        }
        let ts = utc_now();
        self.write_usage(outcome, &ts);
        // Only if this dictation's audio really became `last\audio.wav`; otherwise the previous one is there.
        if committed && self.settings.bench.save_recordings {
            self.save_for_bench(id, &ts);
        }
    }

    /// The attempt's text and audio become the last dictation. The text stays in memory whatever happens with the
    /// disk write: "Copy" and Ctrl+Win+V work even if the disk failed. `true` if the files were written.
    fn commit_last(&mut self, id: DictationId) -> bool {
        let Some((_, text)) = self.final_text.take_if(|(t, _)| *t == id) else {
            tracing::warn!("last dictation not saved: no final text");
            return false;
        };
        let written = match files::commit(&self.paths, &text) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!("last dictation not saved: {e}");
                false
            }
        };
        self.last_text = Some(text);
        written
    }

    fn write_usage(&mut self, outcome: Outcome, ts: &str) {
        let rec = self.attempt.record(outcome, ts.to_owned(), &self.settings);
        self.append_usage(rec);
    }

    /// One usage.csv row, after the rows that could not be written before it.
    fn append_usage(&mut self, rec: UsageRecord) {
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

/// What refine does with the dictation. A translation goes the way the speech language says (spec 2 §5,
/// `translate::output_language`): the language fixed in `translate.speech`, else the one Soniox heard most (`heard`; a
/// code CUE does not know counts as unknown speech). Dictation and "Ask" are cleaned up.
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

/// The field for a dictation's cleanup: the text before the caret and, outside "Ask", the start of the selection it may
/// weave in (spec 2 §12.13). With field context off (`refine.context_chars = 0`) nothing of the field goes to Anthropic,
/// so the selection is not even read.
fn dictation_focus(focus: &FocusWorker, chars: usize, mode: Mode, timeout: Duration) -> FocusInfo {
    if mode == Mode::Ask || chars == 0 {
        focus.query(chars, timeout)
    } else {
        focus.query_for_dictation(chars, refine::WOVEN_READ_CHARS, timeout)
    }
}

/// The selection the field offers a dictation's cleanup (spec 2 §12.13), as the field reported it — the paste goes over it
/// with its edges. None while field context is off (`refine.context_chars = 0`). Whether it goes is the context's call
/// (`RefineContext::sent_selection`), and so is the paste over it.
fn offered_selection(read: &SelectionRead, context_chars: usize) -> Option<&str> {
    if context_chars == 0 {
        return None;
    }
    selection::dictation_selection(read)
}

/// "Restore" of a cancelled dictation (the flow accepts it only in "Cancelled"): its latency is counted from here, and the
/// field is read anew — the read queued at Esc saw the field as it was then.
fn is_restore(a: &UiAction, phase: UiPhase) -> bool {
    *a == UiAction::Undo && phase == UiPhase::Cancel
}

/// Soniox language hints: a fixed speech language for translation goes to Soniox as the only hint (spec 2 §5);
/// otherwise the hints from the settings. Language identification stays on either way (`soniox::start_message`).
fn soniox_hints(mode: Mode, s: &Settings) -> Vec<String> {
    match (mode, s.translate.speech()) {
        (Mode::Translate, Some(code)) => vec![code.to_owned()],
        _ => s.stt.language_hints.clone(),
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
fn local_today() -> String {
    // SAFETY: no arguments; returns a SYSTEMTIME by value.
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

/// The rows an open answer shows for a follow-up: (listening, missed). Listening while it records (spec 2 §7); missed
/// while the error of one that could not be transcribed is up, with its Retry (decision 7). None once the answer
/// closed.
fn follow_up_rows(answer_open: bool, recording: bool, phase: UiPhase) -> (bool, bool) {
    (answer_open && recording, answer_open && phase == UiPhase::Error)
}

/// A web page in the default browser (spec 2 §7). `ShellExecuteW` with the "open" verb takes the URL as one string: no
/// command line of another program parses it.
fn open_url(url: &str) -> Result<(), String> {
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

/// The Soniox key. No key, or the key store failed: for the flow this is a "Soniox key problem"
/// (closer to the truth than "no connection"); the reason goes to the log (the key name and the kind of failure,
/// not the value).
fn soniox_key() -> Result<String, EngineError> {
    keys::get(KeyName::Soniox).map_err(|e| {
        tracing::warn!("Soniox key: {e}");
        match e {
            EngineError::KeyProblem(_) => e,
            other => EngineError::KeyProblem(other.to_string()),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_speech_language_is_the_only_hint_in_translation() {
        let mut s = Settings::default();
        assert_eq!(soniox_hints(Mode::Translate, &s), ["ru", "en"], "auto: the hints from the settings");
        s.translate.speech = Some("de".into());
        assert_eq!(soniox_hints(Mode::Translate, &s), ["de"]);
        assert_eq!(soniox_hints(Mode::Dictate, &s), ["ru", "en"], "a dictation keeps the hints from the settings");
        assert_eq!(soniox_hints(Mode::Ask, &s), ["ru", "en"]);
    }

    #[test]
    fn a_translation_goes_the_way_the_speech_language_says() {
        let tr = |fixed, heard, home| job_for(Mode::Translate, fixed, heard, "en", home);
        assert_eq!(tr(None, Some("ru"), Some("ru")), Job::Translate("en"));
        assert_eq!(tr(None, Some("en"), Some("ru")), Job::Translate("ru"), "target speech goes home");
        assert_eq!(tr(None, Some("en-US"), Some("ru")), Job::Translate("ru"), "Soniox's code goes through `known`");
        assert_eq!(tr(Some("ru"), Some("en"), Some("ru")), Job::Translate("en"), "the fixed language beats Soniox's");
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

    /// Spec 2 §12.13: the field offers its selection only while field context is on (`refine.context_chars = 0` sends nothing
    /// of the field); what goes, and so what the paste goes over, is the context's one value.
    #[test]
    fn a_dictation_offers_the_selection_only_with_field_context_on() {
        let read = |text: &str| SelectionRead::Text { text: text.into(), rects: vec![], read_only: false, clipped: false };
        let link = " https://github.com/seymurglv/cue\n";
        assert_eq!(offered_selection(&read(link), 500), Some(link));
        assert_eq!(offered_selection(&read(link), 0), None, "context off: nothing of the field goes");
        assert_eq!(offered_selection(&SelectionRead::Unknown, 500), None, "no Ctrl+C in a dictation");
        let sent = RefineContext::new("Slack", None, 500).with_selection(offered_selection(&read(link), 500));
        assert_eq!(sent.sent_selection(), Some("https://github.com/seymurglv/cue"));
        let not_sent = RefineContext::new("Slack", None, 500).with_selection(offered_selection(&read("две\nстроки"), 500));
        assert_eq!(not_sent.sent_selection(), None, "offered but not one that goes: no paste over it either");
    }

    /// "Restore" of a cancelled dictation reads the field anew: the read queued at Esc saw the field as it was then.
    #[test]
    fn only_a_restore_of_a_cancelled_dictation_reads_the_field_anew() {
        assert!(is_restore(&UiAction::Undo, UiPhase::Cancel));
        assert!(!is_restore(&UiAction::Undo, UiPhase::Think), "the flow takes a restore only when cancelled");
        assert!(!is_restore(&UiAction::Retry, UiPhase::Cancel));
    }

    #[test]
    fn bench_json_has_null_for_unknown_app() {
        use cue_platform_win::focus::UNKNOWN_APP;
        assert_eq!(bench_json(UNKNOWN_APP), r#"{"app":null}"#);
        assert_eq!(bench_json(""), r#"{"app":null}"#);
        assert_eq!(bench_json("Блокнот"), r#"{"app":"Блокнот"}"#);
    }
}
