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
//! - the log gets events, per-stage latencies and error codes; no text, no audio, no keys.
mod attempt;
mod files;
mod insert;
mod state;
mod tasks;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cue_core::audio::{Silence, SilenceDetector};
use cue_core::flow::{DictationId, Effect, Flow, InsertOutcome, Input, Outcome, SttError, TimerKind};
use cue_core::refine::{RefineContext, join_for_insert};
use cue_core::settings::{self, DropPosition, Settings};
use cue_core::translate;
use cue_core::ui::{Mode, TrayKind, UiAction, UiPhase, UiState};
use cue_core::usage::{self, SttPath};
use cue_engines::EngineError;
use cue_engines::claude::{self, Job, RefineResult};
use cue_engines::keys::{self, KeyName};
use cue_engines::soniox::SonioxConfig;
use cue_engines::stt::{self, Engine, Fallback, SttOutput, SttSession};
use cue_platform_win::chord::HotkeyEvent;
use cue_platform_win::focus::{Editability, FocusInfo, FocusWorker};
use cue_platform_win::hook;
use cue_platform_win::mic::{self, MicEvent, MicHandle};
use cue_platform_win::wav::WavSink;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use crate::ipc::Ui;
use crate::lang;
use crate::paths::Paths;
use crate::trayicon::TrayLook;
use attempt::{Attempt, UsageLog, log_finished, needs_row};
use files::LastFiles;
use insert::{InsertJob, first_known_app, insert_app, is_terminal, known_app, one_line, trim_edges};
use state::{InsertWatch, LevelGate, MicGate, TaskSlot, Timers, WarmUp};
use tasks::{guarded, refine_panicked, stt_panicked};

/// Timeout for each UI Automation query.
const FOCUS_TIMEOUT: Duration = Duration::from_millis(150);
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
    Mic { id: DictationId, ev: MicEvent },
    /// `task` is the task generation; `path` is how the audio was actually transcribed (for the price in usage.csv).
    SttDone { id: DictationId, task: u64, result: Result<SttOutput, EngineError>, path: SttPath },
    RefineDone { id: DictationId, task: u64, result: RefineResult },
    /// `at` is the moment of Ctrl+V (or of the "no field" decision): the latency is counted up to it.
    InsertDone { id: DictationId, outcome: InsertOutcome, at: Instant },
    InsertTimeout { id: DictationId, generation: u64 },
    TimerFired { kind: TimerKind, generation: u64 },
    /// Wake from sleep or unlock: the pooled connections are most likely dead.
    Wake,
    /// The Drop was dragged to a new place: saved in settings.json.
    DropMoved(DropPosition),
}

pub fn input_of(ev: HotkeyEvent) -> Input {
    match ev {
        HotkeyEvent::ChordDown(mode) => Input::ChordDown(mode),
        HotkeyEvent::ModeChange(mode) => Input::ModeChange(mode),
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
    warm: WarmUp,
    attempt: Attempt,
    usage: UsageLog,
}

impl Runtime {
    pub fn new(
        ui: Ui,
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
        tracing::info!(target, home = translate_home.unwrap_or("none"), "translation pair");
        Self {
            flow: Flow::new(settings.hold_hints_left, has_last),
            t0: Instant::now(),
            tx,
            ui,
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
            insert_watch: InsertWatch::default(),
            final_text: None,
            last_text: None,
            last_app: String::new(),
            tray: None,
            level: LevelGate::default(),
            warm: WarmUp::default(),
            attempt: Attempt::new(DictationId::default()),
            usage: UsageLog::default(),
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

    /// The keyboard hook reads these flags itself: whether to swallow Esc and whether CUE is paused.
    fn sync_hook_flags(&self) {
        hook::ESC_ACTIVE.store(self.flow.esc_active(), Ordering::Relaxed);
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
        for e in self.flow.handle(input, now) {
            if matches!(e, Effect::StartStt { .. }) {
                later.push(e);
            } else {
                self.exec(e);
            }
        }
        for e in later {
            self.exec(e);
        }
    }

    fn on_event(&mut self, ev: CoreEvent) {
        match ev {
            CoreEvent::Input(i) => self.feed(i),
            CoreEvent::UiAction(UiAction::TrayAction) => self.on_tray_action(),
            CoreEvent::UiAction(UiAction::TrayClose) => self.tray = None,
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
                self.ui.tray(self.tray);
                self.ui.resend_idle_hidden();
                self.ui.resend_place();
            }
            CoreEvent::Mic { id, ev } => self.on_mic(id, ev),
            CoreEvent::SttDone { id, task, result, path } => self.on_stt_done(id, task, result, path),
            CoreEvent::RefineDone { id, task, result } => self.on_refined(id, task, result),
            CoreEvent::InsertDone { id, outcome, at } => {
                if self.insert_watch.done(id) {
                    let at = self.ms(at);
                    if self.attempt.id == id {
                        self.attempt.inserted(at);
                    }
                    self.feed(Input::Inserted { id, outcome });
                }
            }
            CoreEvent::InsertTimeout { id, generation } => {
                if self.insert_watch.expired(id, generation) {
                    tracing::warn!("insert did not finish within {} s: \"no field\"", INSERT_WATCHDOG.as_secs());
                    let now = self.now();
                    if self.attempt.id == id {
                        self.attempt.inserted(now);
                    }
                    self.feed(Input::Inserted { id, outcome: InsertOutcome::NoField });
                }
            }
            CoreEvent::TimerFired { kind, generation } => {
                if self.timers.fire(kind, generation) {
                    self.feed(Input::Timer(kind));
                }
            }
            CoreEvent::Wake => self.warm.reset(),
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
            Effect::Insert { id, text } => self.insert(id, text),
            Effect::PasteLast => self.paste_last(),
            Effect::Ui(s) => self.emit_state(&s),
            Effect::Shake => self.ui.shake(),
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
        self.attempt = Attempt::new(id);
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
        self.warm_up();
        if let Some(old) = self.stream.take() {
            old.session.abort();
        }
        let Ok(key) = soniox_key() else { return };
        let cfg = SonioxConfig {
            model: self.settings.stt.soniox_model.clone(),
            // The starting mode: it may still change while the chord is held, which language identification covers.
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
        let info = self.focus_for_refine(id);
        self.note_app(&info.app);
        // The query was late ("unknown"): use the app at the start of recording.
        let app = first_known_app(&[&info.app, &self.attempt.app]).to_owned();
        let ctx = RefineContext::new(app, info.before_cursor.as_deref(), self.settings.refine.context_chars);
        // No key: `claude::refine` writes the log line itself and returns the raw text.
        let key = keys::lookup(KeyName::Anthropic).unwrap_or_else(|e| {
            tracing::warn!("Anthropic key: {e}");
            None
        });
        if self.attempt.id == id {
            let now = self.now();
            self.attempt.refine_started(now);
        }
        // Which way to translate (spec 2 §5): the speech language fixed in the settings, else the one Soniox heard most.
        let job = match mode {
            Mode::Translate => {
                let speech = self
                    .settings
                    .translate
                    .speech()
                    .or_else(|| self.attempt.speech_language.as_deref().and_then(translate::known));
                Job::Translate(translate::output_language(speech, self.settings.translate.target(), self.translate_home))
            }
            Mode::Dictate | Mode::Ask => Job::Clean,
        };
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
        let generation = self.insert_watch.start(id);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(INSERT_WATCHDOG).await;
            let _ = tx.send(CoreEvent::InsertTimeout { id, generation });
        });
        self.dispatch_insert(text, Some(id));
    }

    fn paste_last(&mut self) {
        if let Some(text) = self.last_result() {
            self.dispatch_insert(text, None);
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

    /// One path for dictation and Ctrl+Win+V: the focused field (one short query) → text edges without spaces →
    /// join space based on the text before the caret → one line in a terminal → the insert thread. No field:
    /// the text goes to the clipboard. If the query was late ("unknown"), the terminal safeguard looks at the last
    /// known app, then at the app at the start of recording (`insert_app`). Within one dictation the last known app
    /// is never older than the app at the start of recording (`note_app` runs both at the start of recording and on
    /// every later query).
    fn dispatch_insert(&mut self, text: String, id: Option<DictationId>) {
        let info = self.focus.query(INSERT_CONTEXT_CHARS, FOCUS_TIMEOUT);
        let started = if id.is_some_and(|i| i == self.attempt.id) { self.attempt.app.as_str() } else { "" };
        let terminal = is_terminal(insert_app(&info.app, &self.last_app, started));
        self.note_app(&info.app);
        let mut text = join_for_insert(info.before_cursor.as_deref(), trim_edges(&text));
        if terminal {
            text = one_line(&text);
        }
        let job = if info.editable == Editability::No {
            InsertJob::Leave { id, text }
        } else {
            InsertJob::Paste { id, text }
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
        let ts = usage::utc_timestamp(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()));
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
        let (path, settings) = (&self.paths.usage, &self.settings);
        if self.usage.warn_no_price(&settings.pricing, &settings.refine.model) {
            tracing::warn!("usage: no price for model {} in the settings, refine counted as 0", settings.refine.model);
        }
        let rec = self.attempt.record(outcome, ts.to_owned(), settings);
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

/// Soniox language hints: a fixed speech language for translation goes to Soniox as the only hint (spec 2 §5);
/// otherwise the hints from the settings. Language identification stays on either way (`soniox::start_message`).
fn soniox_hints(mode: Mode, s: &Settings) -> Vec<String> {
    match (mode, s.translate.speech()) {
        (Mode::Translate, Some(code)) => vec![code.to_owned()],
        _ => s.stt.language_hints.clone(),
    }
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
    fn hotkeys_map_to_flow_inputs() {
        assert_eq!(input_of(HotkeyEvent::ChordDown(Mode::Translate)), Input::ChordDown(Mode::Translate));
        assert_eq!(input_of(HotkeyEvent::ModeChange(Mode::Translate)), Input::ModeChange(Mode::Translate));
        assert_eq!(input_of(HotkeyEvent::ChordUp), Input::ChordUp);
        assert_eq!(input_of(HotkeyEvent::ChordAborted), Input::ChordAborted);
        assert_eq!(input_of(HotkeyEvent::Esc), Input::Esc);
        assert_eq!(input_of(HotkeyEvent::PasteLast), Input::PasteLast);
    }

    #[test]
    fn bench_json_has_null_for_unknown_app() {
        use cue_platform_win::focus::UNKNOWN_APP;
        assert_eq!(bench_json(UNKNOWN_APP), r#"{"app":null}"#);
        assert_eq!(bench_json(""), r#"{"app":null}"#);
        assert_eq!(bench_json("Блокнот"), r#"{"app":"Блокнот"}"#);
    }
}
