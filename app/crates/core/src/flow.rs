//! The dictation flow: one state machine. Pure logic with no OS, network or clock.
use crate::ui::{ErrorCause, Mode, TrayKind, UiAction, UiPhase, UiState};

pub type Ms = u64;

pub const STARTING_MS: Ms = 380;
pub const HOLD_MS: Ms = 600;
/// A third key while the starting chord is still held, sooner than this after the press: a system shortcut.
pub const SYSTEM_CHORD_MS: Ms = 1_500;
pub const DEBOUNCE_MS: Ms = 300;
pub const DONE_MS: Ms = 1_500;
pub const EMPTY_MS: Ms = 1_000;
pub const UNDO_MS: Ms = 3_600;
pub const WARN_MS: Ms = 8 * 60_000;
pub const AUTO_FINISH_MS: Ms = 9 * 60_000;
pub const PAUSE_MS: Ms = 60 * 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct DictationId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimerKind {
    StartingDone,
    HoldCheck,
    DoneHide,
    EmptyHide,
    UndoExpire,
    WarnLastMinute,
    AutoFinish,
    PauseEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttError {
    /// The speech model is not on this computer, or its files did not load: Retry downloads or checks it again.
    Model,
    /// ONNX Runtime cannot run on this processor (no AVX2): nothing to retry.
    Unsupported,
    /// Recognition itself failed: Retry recognizes the saved recording again.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefineOutcome {
    Refined,
    RawFallback,
    RawKeyProblem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InsertOutcome {
    Pasted,
    NoField,
}

/// Where "Ask" puts text (spec 2 §6.2, §7): at the caret (the tray's Insert; a write with nothing selected), over the
/// selection (Replace), or on a new line after it (the card's Insert below; a write that adds to the selection).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteHow {
    AtCaret,
    Replace,
    Below,
}

/// How an "Ask" request failed: a key, balance or limit problem the user has to fix, or anything else (the network, a
/// timeout, the service), which Retry may help (spec 2 §8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskError {
    Key,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Inserted,
    NoField,
    Cancelled,
    Empty,
    Failed,
    Discarded,
    /// An "Ask" answer was shown in full (a refusal too): paid for, nothing pasted.
    Answered,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// The chord formed; the mode is what the held Shift / Alt picked.
    ChordDown(Mode),
    /// Alt was pressed while the chord is held: the mode it picks ("Ask", when `ask.enabled` is on).
    ModeChange(Mode),
    /// A fresh Shift press, with the chord held or on its own after a tap: dictation and translation swap while
    /// recording, and a translation goes back to the mode it was switched from. The hook sends one more to take it back
    /// when that Shift turns out to be part of other keys or of a click.
    ModeToggle,
    ChordUp,
    ChordAborted,
    Esc,
    PasteLast,
    TogglePause,
    /// The welcome window holds the hotkey (`true`, spec 4a §1): from its creation until its first dictation's step,
    /// and again on any other step. A chord then starts no recording but brings the window forward
    /// (`BringWelcome`), and Ctrl+Win+V pastes nothing, but a recording already running ends as usual (a chord, a
    /// release, Esc). Unlike a pause the keyboard hook still swallows the chord, so the Start menu does not open.
    WelcomeGate(bool),
    Ui(UiAction),
    MicFailed { heard: bool },
    MicDead,
    /// The pinned microphone is not there and the recording goes on with the Windows one (spec 3 §3). The runtime sends
    /// it once per loss: the flow only shows it.
    MicFallback,
    SessionLocked,
    SttFinal { id: DictationId, text: String },
    SttFailed { id: DictationId, error: SttError },
    Refined { id: DictationId, text: String, outcome: RefineOutcome },
    Inserted { id: DictationId, outcome: InsertOutcome },
    Timer(TimerKind),
    /// The "Ask" answer of `id` has something to show (a search began, its sources came, the first words): it opens.
    AskOpened { id: DictationId },
    /// The "Ask" answer of `id` is complete (a refusal, or a write shown instead of written, too).
    AskAnswered { id: DictationId },
    /// The model wrote text for the field; the runtime holds it, `how` says where it goes.
    AskWrite { id: DictationId, how: PasteHow },
    AskFailed { id: DictationId, error: AskError },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenMic { id: DictationId },
    StopMic { id: DictationId, keep: bool },
    StartStt { id: DictationId },
    FinishStt { id: DictationId },
    AbortStt { id: DictationId },
    TranscribeSaved { id: DictationId },
    Refine { id: DictationId, raw: String, mode: Mode },
    /// "Retry" after a failed translation: the same text is refined again; a new usage row without audio (spec 2 §5).
    RefineAgain { id: DictationId, raw: String, mode: Mode },
    AbortRefine { id: DictationId },
    /// Ask the model (spec 2 §6.2): the first question of a conversation, or a follow-up while the answer is open.
    Ask { id: DictationId, question: String, follow_up: bool },
    /// Retry after a failed "Ask" request: the question of `id` again; a new usage row without audio. It can be earlier
    /// than the last question asked (a follow-up aborted before its answer showed), so the runtime keeps every question
    /// of the conversation under its id until `CloseAnswer`.
    AskAgain { id: DictationId },
    /// Abort the request of `id`, the one in flight. An aborted follow-up's question also leaves the conversation: a
    /// later `AskAgain` or follow-up builds on the conversation without it.
    AbortAsk { id: DictationId },
    /// Paste the answer of request `id`, or the text it wrote, as `how` says; the runtime holds the text, and answers
    /// with `Inserted` under the same `id`. It can be earlier than the last question asked, like `AskAgain`'s.
    AskPaste { id: DictationId, how: PasteHow },
    /// The answer closes and its conversation is forgotten (spec 2 §6.2: it lives while the answer is open).
    CloseAnswer,
    Insert { id: DictationId, text: String },
    PasteLast,
    Ui(UiState),
    Shake,
    Tray(Option<TrayKind>),
    SetTimer { kind: TimerKind, after: Ms },
    CancelTimer(TimerKind),
    HoldHintShown,
    Finished { id: DictationId, outcome: Outcome },
    /// A chord the welcome window's hold ignored: the window comes forward, so one opened behind others shows why the
    /// keys did nothing.
    BringWelcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Starting,
    Recording,
    Thinking,
    Inserting,
    Done,
    Empty,
    Cancelled,
    Error { cause: ErrorCause, retry: bool },
    /// An "Ask" answer is open and nothing records: Esc closes it, a chord records a follow-up.
    Answer,
}

pub struct Flow {
    phase: Phase,
    id: DictationId,
    /// An accepted `ChordDown` still without its `ChordUp`.
    chord_held: bool,
    /// The last counted press (for the 0.3 s debounce).
    last_chord_down: Option<Ms>,
    /// Hold mode: the recording ends on release.
    hold_mode: bool,
    /// The transcribed text of `id` (for refine and for "restore").
    raw: Option<String>,
    /// Transcription of `id` failed; this is shown once the recording is over.
    stt_error: Option<SttError>,
    refine_outcome: Option<RefineOutcome>,
    /// Transcription of `id` has started (`StartStt` / `TranscribeSaved`), but there is no result yet.
    stt_pending: bool,
    /// Refine for `id` was requested, but there is no result yet.
    refine_pending: bool,
    /// The error on screen is a failed translation: "retry" refines the saved text instead of transcribing the audio.
    retry_text: bool,
    paused: bool,
    /// The welcome window holds the hotkey (`Input::WelcomeGate`).
    welcome: bool,
    /// The mode of the dictation `id`: what the chord picked, changed only while starting or recording (Shift toggles,
    /// Alt asks while the starting chord is held), and back to `Dictate` on every way into idle (`set_idle`): the idle
    /// Drop shows no mode.
    mode: Mode,
    /// The mode a Shift toggle turned into the translation (`on_mode_toggle`): the next toggle goes back to it. `None`
    /// when the dictation started as a translation, or Alt picked the mode since.
    toggled_from: Option<Mode>,
    hold_hints_left: u8,
    has_last: bool,
    /// The dictation whose "Ask" request is in flight (its answer may already be on screen). It can be an earlier one than
    /// `id`: a follow-up is recorded while the answer it follows still prints (decision 12).
    asking: Option<DictationId>,
    /// The request whose answer the open answer shows (it opened, completed or failed): Retry and the paste buttons name
    /// it. A follow-up takes it over once its own answer shows; a follow-up dropped or cancelled before that leaves it,
    /// so it can be earlier than `id`.
    asked: Option<DictationId>,
    /// A follow-up's question, transcribed while the answer it follows still prints: it goes once that answer has ended.
    held: Option<String>,
    /// The runtime holds an "Ask" conversation: from the first `Ask` effect until `CloseAnswer`.
    conversation: bool,
    /// The answer (tray or card) is on screen: Esc closes it, a chord records a follow-up.
    answer_open: bool,
    /// The request of `asked` failed: the answer shows the error with Retry.
    ask_failed: bool,
    /// The "Ask" request whose text the paste under way puts in (it can be earlier than `id`); `None` for a dictation's
    /// own paste.
    pasting: Option<DictationId>,
    /// The paste under way is of an answer already counted (`Finished` went out when it was complete).
    paste_counted: bool,
}

impl Flow {
    pub fn new(hold_hints_left: u8, has_last: bool) -> Self {
        Self {
            phase: Phase::Idle,
            id: DictationId(0),
            chord_held: false,
            last_chord_down: None,
            hold_mode: false,
            raw: None,
            stt_error: None,
            refine_outcome: None,
            stt_pending: false,
            refine_pending: false,
            retry_text: false,
            paused: false,
            welcome: false,
            mode: Mode::Dictate,
            toggled_from: None,
            hold_hints_left,
            has_last,
            asking: None,
            asked: None,
            held: None,
            conversation: false,
            answer_open: false,
            ask_failed: false,
            pasting: None,
            paste_counted: false,
        }
    }

    pub fn current_id(&self) -> DictationId {
        self.id
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn answer_open(&self) -> bool {
        self.answer_open
    }

    /// Swallow Esc: while CUE is starting, recording, thinking (including pasting, when the Drop shows
    /// "thinking"; Esc does nothing there), showing an error or an "Ask" answer.
    pub fn esc_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Starting | Phase::Recording | Phase::Thinking | Phase::Inserting | Phase::Error { .. } | Phase::Answer
        )
    }

    pub fn ui_state(&self) -> UiState {
        let (phase, error, retry) = match self.phase {
            Phase::Idle => (UiPhase::Idle, None, false),
            Phase::Starting => (UiPhase::Start, None, false),
            Phase::Recording => (UiPhase::Listen, None, false),
            Phase::Thinking | Phase::Inserting => (UiPhase::Think, None, false),
            Phase::Done => (UiPhase::Done, None, false),
            Phase::Empty => (UiPhase::Empty, None, false),
            Phase::Cancelled => (UiPhase::Cancel, None, false),
            Phase::Error { cause, retry } => (UiPhase::Error, Some(cause), retry),
            Phase::Answer => (UiPhase::Answer, None, false),
        };
        UiState { phase, error, retry, paused: self.paused, mode: self.mode }
    }

    pub fn handle(&mut self, input: Input, now: Ms) -> Vec<Effect> {
        let mut fx = Vec::new();
        match input {
            Input::ChordDown(mode) => self.on_chord_down(mode, now, &mut fx),
            Input::ModeChange(mode) => self.on_mode_change(mode, &mut fx),
            Input::ModeToggle => self.on_mode_toggle(&mut fx),
            Input::ChordUp => {
                // A ChordUp without a pair (chord pressed during a pause, after a cancel, after the key check)
                // only clears the hold.
                // "Hold" vs "press and release" is decided by how long the chord was held, not by timer order.
                let held = std::mem::replace(&mut self.chord_held, false);
                if held && self.recording_now() {
                    let long = self.last_chord_down.is_some_and(|t| now.saturating_sub(t) >= HOLD_MS);
                    self.hold_mode = long;
                    if long {
                        self.finish(&mut fx);
                    }
                }
            }
            Input::ChordAborted => {
                // Ctrl+Win+D or Ctrl+Win+arrows: the third key often comes after the start-up is over.
                // Later than SYSTEM_CHORD_MS it is taken as a slip during a held dictation, and the recording goes on.
                let fresh = self.chord_held
                    && self.last_chord_down.is_some_and(|t| now.saturating_sub(t) < SYSTEM_CHORD_MS);
                if self.phase == Phase::Starting || (self.phase == Phase::Recording && fresh) {
                    self.discard(&mut fx);
                }
            }
            Input::Esc | Input::Ui(UiAction::Cancel) => self.on_cancel(&mut fx),
            Input::Ui(UiAction::Finish) => {
                if self.phase == Phase::Recording {
                    self.finish(&mut fx);
                }
            }
            Input::Ui(UiAction::Undo) => self.on_undo(&mut fx),
            Input::Ui(UiAction::Retry) => self.on_retry(&mut fx),
            Input::Ui(UiAction::Dismiss) => {
                if matches!(self.phase, Phase::Error { .. }) {
                    if self.answer_open {
                        // The error of a follow-up that could not be transcribed: back to the answer (decision 7).
                        self.phase = Phase::Answer;
                        self.push_ui(&mut fx);
                    } else {
                        self.enter_idle(&mut fx);
                    }
                }
            }
            // The tray card carries out its own buttons; they do not change the flow.
            Input::Ui(UiAction::TrayAction | UiAction::TrayClose) => {}
            // The suggestion belongs to the runtime's offers; the flow does not change.
            Input::Ui(UiAction::SuggestAdd | UiAction::SuggestUndo | UiAction::SuggestDismiss | UiAction::SuggestDone) => {}
            Input::PasteLast => self.on_paste_last(&mut fx),
            Input::TogglePause => self.on_toggle_pause(&mut fx),
            Input::WelcomeGate(held) => self.welcome = held,
            Input::MicFailed { heard } => self.on_mic_lost(heard, &mut fx),
            Input::MicDead => self.on_mic_lost(false, &mut fx),
            // One card at a time: none over an open answer, as for "one minute left".
            Input::MicFallback => {
                if self.recording_now() && !self.answer_open {
                    fx.push(Effect::Tray(Some(TrayKind::MicMissing)));
                }
            }
            Input::SessionLocked => {
                if self.recording_now() {
                    self.discard(&mut fx);
                }
            }
            Input::SttFinal { id, text } if self.stt_for(id) => {
                self.stt_pending = false;
                self.raw = Some(text);
                if self.phase == Phase::Thinking {
                    self.advance(&mut fx);
                }
            }
            Input::SttFailed { id, error } if self.stt_for(id) => {
                self.stt_pending = false;
                self.stt_error = Some(error);
                if self.phase == Phase::Thinking {
                    self.advance(&mut fx);
                }
            }
            Input::Refined { id, text, outcome }
                if id == self.id && self.refine_pending && self.phase == Phase::Thinking =>
            {
                self.refine_pending = false;
                // A failed translation is never replaced with the untranslated text (spec 2 §5): an error with "retry",
                // which refines the saved text again.
                if self.mode == Mode::Translate && (outcome != RefineOutcome::Refined || text.trim().is_empty()) {
                    let cause = if outcome == RefineOutcome::RawKeyProblem { ErrorCause::ClaudeKey } else { ErrorCause::Translate };
                    fx.push(Effect::Finished { id, outcome: Outcome::Failed });
                    self.retry_text = true;
                    self.phase = Phase::Error { cause, retry: true };
                    self.push_ui(&mut fx);
                    return fx;
                }
                // An empty result would not pass the sanity check: the raw text is pasted.
                let (text, outcome) = match self.raw.clone() {
                    Some(raw) if text.trim().is_empty() => {
                        let outcome = match outcome {
                            RefineOutcome::RawKeyProblem => RefineOutcome::RawKeyProblem,
                            RefineOutcome::Refined | RefineOutcome::RawFallback => RefineOutcome::RawFallback,
                        };
                        (raw, outcome)
                    }
                    _ => (text, outcome),
                };
                if text.trim().is_empty() {
                    self.enter_empty(&mut fx);
                } else {
                    self.refine_outcome = Some(outcome);
                    self.phase = Phase::Inserting;
                    fx.push(Effect::Insert { id, text });
                }
            }
            Input::Inserted { id, outcome } if id == self.pasting.unwrap_or(self.id) && self.phase == Phase::Inserting => {
                self.has_last = true;
                self.on_inserted(outcome, &mut fx);
            }
            Input::Ui(UiAction::AnswerClose) => {
                if self.phase == Phase::Answer {
                    self.close_answer(&mut fx);
                }
            }
            Input::Ui(UiAction::AnswerInsert) => self.on_answer_paste(PasteHow::AtCaret, &mut fx),
            Input::Ui(UiAction::AnswerReplace) => self.on_answer_paste(PasteHow::Replace, &mut fx),
            Input::Ui(UiAction::AnswerBelow) => self.on_answer_paste(PasteHow::Below, &mut fx),
            Input::Ui(UiAction::AnswerRetry) => self.on_answer_retry(&mut fx),
            // The runtime copies the answer: the flow does not change.
            Input::Ui(UiAction::AnswerCopy) => {}
            // The first visible step of a request of the current dictation — a follow-up's too: its answer shows.
            Input::AskOpened { id } if self.ask_for(id) && id == self.id && self.phase == Phase::Thinking => {
                self.answer_shows(id, false);
                self.open_answer(&mut fx);
            }
            Input::AskAnswered { id } if self.ask_for(id) => self.answered(id, &mut fx),
            Input::AskWrite { id, how } if self.ask_for(id) && self.asking_now() && !self.shows_write(id) => {
                self.asking = None;
                self.pasting = Some(id);
                self.paste_counted = false;
                self.phase = Phase::Inserting;
                // The text lives in the conversation: the paste is asked for before the conversation goes.
                fx.push(Effect::AskPaste { id, how });
                self.end_conversation(&mut fx);
                self.push_ui(&mut fx);
            }
            // A write that is shown instead (`shows_write`): the runtime says `AskAnswered` then; if a write comes all
            // the same, it counts as answered.
            Input::AskWrite { id, .. } if self.ask_for(id) && self.shows_write(id) => self.answered(id, &mut fx),
            Input::AskFailed { id, error } if self.ask_for(id) => {
                self.asking = None;
                fx.push(Effect::Finished { id, outcome: Outcome::Failed });
                match error {
                    // Retry would fail the same way: the Drop's "Claude key problem"; the answer closes, and a follow-up
                    // under way goes with it (spec 2 §8).
                    AskError::Key => {
                        if self.follow_up_under_way(id) {
                            self.drop_pending_follow_up(&mut fx);
                        }
                        self.end_conversation(&mut fx);
                        self.phase = Phase::Error { cause: ErrorCause::ClaudeKey, retry: false };
                        self.push_ui(&mut fx);
                    }
                    AskError::Failed => {
                        self.answer_shows(id, true);
                        self.answer_on_screen(&mut fx);
                        if !self.follow_up_under_way(id) {
                            // The answer shows the error with Retry; the question is kept (spec 2 §8).
                            self.phase = Phase::Answer;
                            self.push_ui(&mut fx);
                        } else {
                            // A follow-up waiting for this answer goes now, in place of the failed question.
                            self.send_held(&mut fx);
                        }
                    }
                }
            }
            Input::Timer(kind) => self.on_timer(kind, &mut fx),
            // Results of another (earlier) dictation or "Ask" request (a dropped follow-up never had one), and repeats of
            // ones already received.
            Input::SttFinal { .. }
            | Input::SttFailed { .. }
            | Input::Refined { .. }
            | Input::Inserted { .. }
            | Input::AskOpened { .. }
            | Input::AskAnswered { .. }
            | Input::AskWrite { .. }
            | Input::AskFailed { .. } => {}
        }
        fx
    }

    /// Starting or recording: the microphone is open, and Shift toggles the mode. The runtime publishes it to the
    /// keyboard hook as `hook::RECORDING`.
    pub fn recording_now(&self) -> bool {
        matches!(self.phase, Phase::Starting | Phase::Recording)
    }

    /// The transcription result belongs to the current dictation and is still awaited.
    fn stt_for(&self, id: DictationId) -> bool {
        id == self.id && self.stt_pending
    }

    /// The "Ask" request of dictation `id` is awaited.
    fn ask_for(&self, id: DictationId) -> bool {
        self.asking == Some(id)
    }

    /// Waiting for the request or showing its answer: where a write is pasted.
    fn asking_now(&self) -> bool {
        matches!(self.phase, Phase::Thinking | Phase::Answer)
    }

    /// A follow-up recorded after the request `id` is still under way: recording, being transcribed, waiting for the
    /// answer it follows, or showing that it could not be transcribed. One dropped since has left the flow at the answer.
    fn follow_up_under_way(&self, id: DictationId) -> bool {
        id != self.id && self.phase != Phase::Answer
    }

    /// A write that ends the request `id` now is shown in the answer, with Copy, not pasted: a follow-up is under way
    /// (not under the user's hands, decision 12), or CUE is paused (it never injects input then). The runtime asks this
    /// when the request ends.
    pub fn shows_write(&self, id: DictationId) -> bool {
        self.follow_up_under_way(id) || self.paused
    }

    /// The request `id` ended with an answer to show (a refusal, or a write shown instead, too): counted, and the
    /// answer opens if it is the current question's and nothing showed yet; a follow-up waiting for it goes now.
    fn answered(&mut self, id: DictationId, fx: &mut Vec<Effect>) {
        self.asking = None;
        fx.push(Effect::Finished { id, outcome: Outcome::Answered });
        self.answer_shows(id, false);
        if id == self.id && self.phase == Phase::Thinking {
            self.open_answer(fx);
        }
        self.send_held(fx);
    }

    /// The answer of request `id` is what the open answer shows now, complete or failed: Retry and the paste buttons
    /// act on it.
    fn answer_shows(&mut self, id: DictationId, failed: bool) {
        self.asked = Some(id);
        self.ask_failed = failed;
    }

    /// A follow-up transcribed while the answer it follows printed goes now that the answer has ended (decision 12).
    /// The buttons stay with that answer until the follow-up's own shows.
    fn send_held(&mut self, fx: &mut Vec<Effect>) {
        if let Some(question) = self.held.take() {
            self.asking = Some(self.id);
            fx.push(Effect::Ask { id: self.id, question, follow_up: true });
        }
    }

    fn open_answer(&mut self, fx: &mut Vec<Effect>) {
        self.answer_on_screen(fx);
        self.phase = Phase::Answer;
        self.push_ui(fx);
    }

    /// The answer is on screen from now on. One card at a time: a tray card still up (a missing microphone told at the
    /// start of the question) closes as the answer opens.
    fn answer_on_screen(&mut self, fx: &mut Vec<Effect>) {
        if !self.answer_open {
            fx.push(Effect::Tray(None));
        }
        self.answer_open = true;
    }

    /// Aborts the request in flight, if any. `counted` — its attempt ends "cancelled" now (an open answer closing), not
    /// later (a first question cancelled with "restore", finished when "restore" expires).
    fn abort_asking(&mut self, counted: bool, fx: &mut Vec<Effect>) {
        if let Some(id) = self.asking.take() {
            fx.push(Effect::AbortAsk { id });
            if counted {
                fx.push(Effect::Finished { id, outcome: Outcome::Cancelled });
            }
        }
    }

    /// Esc or the cross while the answer is open: a request still under way is aborted (its attempt ends "cancelled"), the
    /// conversation is forgotten (spec 2 §6.2) and the Drop rests.
    fn close_answer(&mut self, fx: &mut Vec<Effect>) {
        self.abort_asking(true, fx);
        self.end_conversation(fx);
        self.enter_idle(fx);
    }

    /// The runtime forgets the conversation, if there is one, and the answer leaves the screen.
    fn end_conversation(&mut self, fx: &mut Vec<Effect>) {
        if self.conversation {
            fx.push(Effect::CloseAnswer);
        }
        self.conversation = false;
        self.answer_open = false;
        self.asked = None;
        self.ask_failed = false;
        self.held = None;
    }

    /// The follow-up under way goes without a trace — the conversation it belongs to ended (a key problem of the answer it
    /// follows): its recording, its transcription or its waiting question. The caller shows what comes next.
    fn drop_pending_follow_up(&mut self, fx: &mut Vec<Effect>) {
        self.held = None;
        match self.phase {
            Phase::Starting | Phase::Recording => self.stop_follow_up(Outcome::Discarded, fx),
            Phase::Thinking => {
                self.abort_stt(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Discarded });
            }
            _ => {}
        }
    }

    /// A follow-up recording is dropped (Esc, a system chord, Ctrl+Win+V, a lock): the open answer stays, with no
    /// "restore" (spec 2 §7).
    fn drop_follow_up(&mut self, outcome: Outcome, fx: &mut Vec<Effect>) {
        self.stop_follow_up(outcome, fx);
        self.phase = Phase::Answer;
        self.push_ui(fx);
    }

    /// The follow-up's recording stops and is thrown away, its transcription is aborted, its attempt ends with `outcome`;
    /// the phase and the Drop are up to the caller.
    fn stop_follow_up(&mut self, outcome: Outcome, fx: &mut Vec<Effect>) {
        fx.push(Effect::StopMic { id: self.id, keep: false });
        self.abort_stt(fx);
        self.cancel_record_timers(fx);
        self.chord_held = false;
        self.hold_mode = false;
        fx.push(Effect::Finished { id: self.id, outcome });
    }

    /// Insert, Insert below or Replace: only for an answer complete on screen. The answer closes; its attempt was counted
    /// when it was complete, so the paste brings no second `Finished`.
    fn on_answer_paste(&mut self, how: PasteHow, fx: &mut Vec<Effect>) {
        if self.phase != Phase::Answer || self.asking.is_some() || self.ask_failed {
            return;
        }
        let Some(id) = self.asked else { return };
        self.pasting = Some(id);
        self.paste_counted = true;
        // The hold hint follows the paste that ends a held recording; a button pressed later does not end one.
        self.hold_mode = false;
        self.phase = Phase::Inserting;
        fx.push(Effect::AskPaste { id, how });
        self.end_conversation(fx);
        self.push_ui(fx);
    }

    /// Retry in an answer whose request failed: the same question again (a new attempt without audio).
    fn on_answer_retry(&mut self, fx: &mut Vec<Effect>) {
        if self.phase == Phase::Answer
            && self.ask_failed
            && self.asking.is_none()
            && let Some(id) = self.asked
        {
            self.ask_failed = false;
            self.asking = Some(id);
            fx.push(Effect::AskAgain { id });
        }
    }

    fn on_chord_down(&mut self, mode: Mode, now: Ms, fx: &mut Vec<Effect>) {
        if self.paused {
            return;
        }
        // The welcome window holds the key: only the press that ends a running recording counts; any other brings the
        // window forward.
        if self.welcome && !self.recording_now() {
            fx.push(Effect::BringWelcome);
            return;
        }
        if let Some(t) = self.last_chord_down
            && now.saturating_sub(t) < DEBOUNCE_MS
        {
            return;
        }
        self.last_chord_down = Some(now);
        self.chord_held = true;
        match self.phase {
            // "Press again and the recording ends", during start-up too.
            Phase::Starting | Phase::Recording => self.finish(fx),
            Phase::Thinking | Phase::Inserting => fx.push(Effect::Shake),
            Phase::Cancelled => {
                self.close_cancelled(fx);
                self.start(mode, fx);
            }
            // A chord while the answer is open records a follow-up at once, also while the answer still prints (decision 12,
            // the approved design): the follow-up goes once both have ended. A dictation needs the answer closed first (§7).
            Phase::Answer => self.start(Mode::Ask, fx),
            // A follow-up that could not be transcribed: a new chord records another.
            Phase::Error { .. } if self.answer_open => self.start(Mode::Ask, fx),
            Phase::Idle | Phase::Done | Phase::Empty | Phase::Error { .. } => self.start(mode, fx),
        }
    }

    fn start(&mut self, mode: Mode, fx: &mut Vec<Effect>) {
        if !self.answer_open {
            self.end_conversation(fx);
        }
        self.id = DictationId(self.id.0.wrapping_add(1));
        self.mode = mode;
        self.toggled_from = None;
        self.phase = Phase::Starting;
        self.chord_held = true;
        self.hold_mode = false;
        self.raw = None;
        self.stt_error = None;
        self.refine_outcome = None;
        self.stt_pending = true;
        self.refine_pending = false;
        self.retry_text = false;
        self.held = None;
        self.pasting = None;
        self.paste_counted = false;
        for kind in [TimerKind::DoneHide, TimerKind::EmptyHide, TimerKind::UndoExpire] {
            fx.push(Effect::CancelTimer(kind));
        }
        fx.push(Effect::Tray(None));
        fx.push(Effect::OpenMic { id: self.id });
        fx.push(Effect::StartStt { id: self.id });
        fx.push(Effect::SetTimer { kind: TimerKind::StartingDone, after: STARTING_MS });
        fx.push(Effect::SetTimer { kind: TimerKind::HoldCheck, after: HOLD_MS });
        fx.push(Effect::SetTimer { kind: TimerKind::WarnLastMinute, after: WARN_MS });
        fx.push(Effect::SetTimer { kind: TimerKind::AutoFinish, after: AUTO_FINISH_MS });
        self.push_ui(fx);
    }

    /// Alt pressed while the starting chord is still held (hold mode included): the dictation switches to the mode Alt
    /// picks. After the release Alt no longer counts; the finishing press has already moved the flow to "thinking".
    fn on_mode_change(&mut self, mode: Mode, fx: &mut Vec<Effect>) {
        if self.chord_held && self.recording_now() && self.mode != mode && !self.answer_open {
            self.mode = mode;
            self.toggled_from = None;
            self.push_ui(fx);
        }
    }

    /// A fresh Shift press while starting or recording, with the chord still held or after a tap: dictation and
    /// translation swap ("Ask" becomes a translation), and the Drop changes its icon at once. A translation goes back
    /// to the mode it was switched from ("Ask" included; a dictation that started as a translation becomes a
    /// dictation), so every toggle is undone by the next one. After a tap the hook sends it only for a Shift pressed on
    /// its own, and sends one more to take it back when another key or a mouse button goes down while that Shift is
    /// held (`chord::Decision::undo_toggle`, `chord::Chord::on_mouse_down`): a capital letter, Shift + click or the
    /// finishing chord never switches the mode. The user often taps the chord and decides on the translation only while
    /// speaking, and a second Shift takes a slip back. The finishing press has already moved the flow to "thinking", so
    /// a later Shift changes nothing. A follow-up stays "Ask".
    fn on_mode_toggle(&mut self, fx: &mut Vec<Effect>) {
        if self.recording_now() && !self.answer_open {
            self.mode = match self.mode {
                Mode::Translate => self.toggled_from.take().unwrap_or(Mode::Dictate),
                from @ (Mode::Dictate | Mode::Ask) => {
                    self.toggled_from = Some(from);
                    Mode::Translate
                }
            };
            self.push_ui(fx);
        }
    }

    /// The start-up or recording is thrown away: the audio is not kept, transcription is aborted, no "restore".
    /// The Drop and `Finished` are up to the caller.
    fn drop_recording(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::StopMic { id: self.id, keep: false });
        self.abort_stt(fx);
        self.cancel_record_timers(fx);
        self.set_idle();
        self.chord_held = false;
        self.hold_mode = false;
    }

    /// Quietly, with no error and no "restore": a system chord (see `SYSTEM_CHORD_MS`), Ctrl+Win+V in the first 380 ms,
    /// a pause or a session lock during recording.
    fn discard(&mut self, fx: &mut Vec<Effect>) {
        if self.answer_open {
            self.drop_follow_up(Outcome::Discarded, fx);
            return;
        }
        self.drop_recording(fx);
        fx.push(Effect::Finished { id: self.id, outcome: Outcome::Discarded });
        self.push_ui(fx);
    }

    /// The recording ends (second press, release in hold mode, "done", 9:00, microphone failure after speech).
    fn finish(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::StopMic { id: self.id, keep: true });
        if self.stt_pending {
            fx.push(Effect::FinishStt { id: self.id });
        }
        self.cancel_record_timers(fx);
        self.phase = Phase::Thinking;
        if self.advance(fx) {
            self.push_ui(fx);
        }
    }

    /// In "thinking": if the transcription result is already here, take the next step.
    /// Returns `true` if we stayed in "thinking" (the caller then updates the Drop if needed).
    fn advance(&mut self, fx: &mut Vec<Effect>) -> bool {
        if let Some(error) = self.stt_error.take() {
            // The conversation stays: an open answer keeps a follow-up's error with Retry (decision 7), and a first
            // question has none yet.
            fx.push(Effect::Finished { id: self.id, outcome: Outcome::Failed });
            self.retry_text = false;
            self.phase = Phase::Error { cause: cause_of(error), retry: error != SttError::Unsupported };
            self.push_ui(fx);
            return false;
        }
        let Some(raw) = self.raw.as_deref() else {
            return true;
        };
        if raw.trim().is_empty() {
            self.enter_empty(fx);
            return false;
        }
        if self.mode == Mode::Ask {
            match self.asking {
                // Already asked (`advance` runs again).
                Some(id) if id == self.id => {}
                // The answer this follow-up follows still prints: the question waits for it (decision 12).
                Some(_) => self.held = Some(raw.to_owned()),
                // A follow-up leaves the buttons with the answer on screen until its own answer shows.
                None => {
                    fx.push(Effect::Ask { id: self.id, question: raw.to_owned(), follow_up: self.answer_open });
                    self.asking = Some(self.id);
                    self.conversation = true;
                }
            }
            return true;
        }
        if !self.refine_pending {
            fx.push(Effect::Refine { id: self.id, raw: raw.to_owned(), mode: self.mode });
            self.refine_pending = true;
        }
        true
    }

    fn on_cancel(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Starting | Phase::Recording if self.answer_open => self.drop_follow_up(Outcome::Cancelled, fx),
            Phase::Starting | Phase::Recording => {
                // The audio is kept and the text is still fetched, for "restore".
                fx.push(Effect::StopMic { id: self.id, keep: true });
                if self.stt_pending {
                    fx.push(Effect::FinishStt { id: self.id });
                }
                self.cancel_record_timers(fx);
                self.enter_cancelled(fx);
            }
            Phase::Thinking if self.answer_open => {
                // A follow-up being transcribed, waiting or asked: it goes, the answer stays — and keeps printing if it
                // still does (no "restore": spec 2 §7).
                self.held = None;
                if self.asking == Some(self.id) {
                    self.abort_asking(false, fx);
                }
                self.abort_stt(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Cancelled });
                self.phase = Phase::Answer;
                self.push_ui(fx);
            }
            Phase::Thinking => {
                if self.refine_pending {
                    fx.push(Effect::AbortRefine { id: self.id });
                    self.refine_pending = false;
                }
                // A first question before its answer showed: cancelled like a dictation; "restore" asks again, in a new
                // conversation with the same context (decision 28).
                self.abort_asking(false, fx);
                self.end_conversation(fx);
                self.enter_cancelled(fx);
            }
            // The error of a follow-up that could not be transcribed: back to the answer, still open (decision 7).
            Phase::Error { .. } if self.answer_open => {
                self.phase = Phase::Answer;
                self.push_ui(fx);
            }
            Phase::Answer => self.close_answer(fx),
            Phase::Error { .. } => self.enter_idle(fx),
            // The paste is already under way and cannot be cancelled; Esc is only swallowed.
            Phase::Inserting => {}
            Phase::Idle | Phase::Done | Phase::Empty | Phase::Cancelled => {}
        }
    }

    fn on_undo(&mut self, fx: &mut Vec<Effect>) {
        if self.phase != Phase::Cancelled {
            return;
        }
        fx.push(Effect::CancelTimer(TimerKind::UndoExpire));
        self.phase = Phase::Thinking;
        if self.advance(fx) {
            self.push_ui(fx);
        }
    }

    fn on_retry(&mut self, fx: &mut Vec<Effect>) {
        if let Phase::Error { retry: true, .. } = self.phase {
            if self.retry_text
                && let Some(raw) = self.raw.clone()
            {
                self.retry_text = false;
                self.phase = Phase::Thinking;
                self.refine_pending = true;
                fx.push(Effect::RefineAgain { id: self.id, raw, mode: self.mode });
                self.push_ui(fx);
                return;
            }
            self.raw = None;
            self.stt_error = None;
            self.refine_outcome = None;
            self.stt_pending = true;
            self.phase = Phase::Thinking;
            fx.push(Effect::TranscribeSaved { id: self.id });
            self.push_ui(fx);
        }
    }

    /// The "restore" window is over (it expired or a new recording started): the cancel is final.
    fn close_cancelled(&mut self, fx: &mut Vec<Effect>) {
        self.abort_stt(fx);
        self.end_conversation(fx);
        fx.push(Effect::Finished { id: self.id, outcome: Outcome::Cancelled });
    }

    fn abort_stt(&mut self, fx: &mut Vec<Effect>) {
        if self.stt_pending {
            fx.push(Effect::AbortStt { id: self.id });
            self.stt_pending = false;
        }
    }

    fn on_paste_last(&mut self, fx: &mut Vec<Effect>) {
        if self.paused {
            return;
        }
        match self.phase {
            Phase::Recording | Phase::Thinking | Phase::Inserting => return,
            // Ctrl+Win+V is not dictation: the dictation's start-up is dropped even if there is nothing to paste.
            Phase::Starting => self.discard(fx),
            _ => {}
        }
        // The welcome window holds the key: nothing is pasted, though a start-up begun before it is still dropped.
        if self.welcome {
            return;
        }
        if self.has_last {
            fx.push(Effect::PasteLast);
        }
    }

    fn on_toggle_pause(&mut self, fx: &mut Vec<Effect>) {
        if self.paused {
            self.paused = false;
            fx.push(Effect::CancelTimer(TimerKind::PauseEnd));
        } else {
            // Keys do nothing during a pause: an ongoing recording is thrown away, with no "restore", and an open answer
            // closes (Esc would not reach it) — with a follow-up being transcribed or waiting, and the request in flight.
            if self.recording_now() {
                self.drop_recording(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Discarded });
            } else if self.answer_open && self.phase == Phase::Thinking && self.asking != Some(self.id) {
                self.held = None;
                self.abort_stt(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Discarded });
            }
            if self.answer_open {
                self.abort_asking(true, fx);
                self.end_conversation(fx);
                self.set_idle();
            }
            self.paused = true;
            self.chord_held = false;
            fx.push(Effect::SetTimer { kind: TimerKind::PauseEnd, after: PAUSE_MS });
        }
        self.push_ui(fx);
    }

    /// The microphone failed (`heard`: speech was already heard) or has been silent since the recording started.
    fn on_mic_lost(&mut self, heard: bool, fx: &mut Vec<Effect>) {
        match self.phase {
            // Process what was said so far.
            Phase::Recording if heard => self.finish(fx),
            Phase::Starting | Phase::Recording => {
                self.drop_recording(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Failed });
                // One card at a time: the microphone card replaces an open answer, and the answer still printing stops.
                self.abort_asking(true, fx);
                self.end_conversation(fx);
                self.push_ui(fx);
                fx.push(Effect::Tray(Some(TrayKind::MicSilent)));
            }
            _ => {}
        }
    }

    fn on_inserted(&mut self, outcome: InsertOutcome, fx: &mut Vec<Effect>) {
        // An "Ask" paste is of its request, which can be earlier than the current dictation.
        let id = self.pasting.take().unwrap_or(self.id);
        let counted = std::mem::take(&mut self.paste_counted);
        match outcome {
            InsertOutcome::Pasted => {
                if !counted {
                    fx.push(Effect::Finished { id, outcome: Outcome::Inserted });
                }
                if self.refine_outcome == Some(RefineOutcome::RawKeyProblem) {
                    self.phase = Phase::Error { cause: ErrorCause::ClaudeKey, retry: false };
                    self.push_ui(fx);
                    return;
                }
                self.phase = Phase::Done;
                self.push_ui(fx);
                fx.push(Effect::SetTimer { kind: TimerKind::DoneHide, after: DONE_MS });
                if self.hold_mode && self.hold_hints_left > 0 {
                    self.hold_hints_left -= 1;
                    fx.push(Effect::Tray(Some(TrayKind::HoldHint)));
                    fx.push(Effect::HoldHintShown);
                }
            }
            InsertOutcome::NoField => {
                if !counted {
                    fx.push(Effect::Finished { id, outcome: Outcome::NoField });
                }
                self.enter_idle(fx);
                fx.push(Effect::Tray(Some(TrayKind::NoField)));
            }
        }
    }

    /// Timers carry no dictation number: a late timer is recognized because the current phase is not its own.
    fn on_timer(&mut self, kind: TimerKind, fx: &mut Vec<Effect>) {
        match (kind, self.phase) {
            (TimerKind::StartingDone, Phase::Starting) => {
                self.phase = Phase::Recording;
                self.push_ui(fx);
            }
            (TimerKind::HoldCheck, Phase::Starting | Phase::Recording) => {
                if self.chord_held {
                    self.hold_mode = true;
                }
            }
            (TimerKind::WarnLastMinute, Phase::Recording) if !self.answer_open => fx.push(Effect::Tray(Some(TrayKind::LastMinute))),
            (TimerKind::AutoFinish, Phase::Recording) => self.finish(fx),
            (TimerKind::DoneHide, Phase::Done) | (TimerKind::EmptyHide, Phase::Empty) => self.enter_idle(fx),
            (TimerKind::UndoExpire, Phase::Cancelled) => {
                self.close_cancelled(fx);
                self.enter_idle(fx);
            }
            (TimerKind::PauseEnd, _) if self.paused => {
                self.paused = false;
                self.push_ui(fx);
            }
            _ => {}
        }
    }

    fn cancel_record_timers(&self, fx: &mut Vec<Effect>) {
        for kind in [TimerKind::StartingDone, TimerKind::HoldCheck, TimerKind::WarnLastMinute, TimerKind::AutoFinish] {
            fx.push(Effect::CancelTimer(kind));
        }
    }

    fn enter_idle(&mut self, fx: &mut Vec<Effect>) {
        self.set_idle();
        self.push_ui(fx);
    }

    /// The only way into `Phase::Idle`.
    fn set_idle(&mut self) {
        self.phase = Phase::Idle;
        // The idle Drop shows no mode; "restore" and "retry" do not live in idle, so nothing still needs it.
        self.mode = Mode::Dictate;
    }

    fn enter_empty(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Finished { id: self.id, outcome: Outcome::Empty });
        if self.answer_open {
            // A follow-up nobody said anything in: back to the answer, still open.
            self.phase = Phase::Answer;
            self.push_ui(fx);
            return;
        }
        self.phase = Phase::Empty;
        self.push_ui(fx);
        fx.push(Effect::SetTimer { kind: TimerKind::EmptyHide, after: EMPTY_MS });
    }

    fn enter_cancelled(&mut self, fx: &mut Vec<Effect>) {
        self.phase = Phase::Cancelled;
        self.push_ui(fx);
        fx.push(Effect::SetTimer { kind: TimerKind::UndoExpire, after: UNDO_MS });
    }

    fn push_ui(&self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Ui(self.ui_state()));
    }
}

fn cause_of(error: SttError) -> ErrorCause {
    match error {
        SttError::Model => ErrorCause::Model,
        SttError::Unsupported => ErrorCause::Cpu,
        SttError::Failed => ErrorCause::Stt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Effect as E;
    use Input as I;

    fn last_ui(fx: &[Effect]) -> Option<UiState> {
        fx.iter().rev().find_map(|e| if let E::Ui(s) = e { Some(s.clone()) } else { None })
    }
    fn phase(fx: &[Effect]) -> Option<UiPhase> {
        last_ui(fx).map(|s| s.phase)
    }
    fn has(fx: &[Effect], e: &Effect) -> bool {
        fx.contains(e)
    }

    /// Pressed and released, start-up done: recording in "press and release" mode.
    fn recording(f: &mut Flow, t: Ms) -> DictationId {
        recording_in(f, t, Mode::Dictate)
    }

    /// As `recording`, with the chord that picked `mode`.
    fn recording_in(f: &mut Flow, t: Ms, mode: Mode) -> DictationId {
        f.handle(I::ChordDown(mode), t);
        f.handle(I::ChordUp, t + 100);
        f.handle(I::Timer(TimerKind::StartingDone), t + STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), t + HOLD_MS);
        f.current_id()
    }

    /// The recording was finished by a second press, and the transcribed text has arrived.
    fn thinking_with_raw(f: &mut Flow, t: Ms, raw: &str) -> DictationId {
        let id = recording(f, t);
        f.handle(I::ChordDown(Mode::Dictate), t + 5_000);
        f.handle(I::ChordUp, t + 5_100);
        f.handle(I::SttFinal { id, text: raw.into() }, t + 5_300);
        id
    }

    /// An "Ask" question recorded by tap and transcribed: its request is in flight.
    fn asking(f: &mut Flow, t: Ms, question: &str) -> DictationId {
        let id = recording_in(f, t, Mode::Ask);
        f.handle(I::ChordDown(Mode::Dictate), t + 5_000);
        f.handle(I::ChordUp, t + 5_100);
        f.handle(I::SttFinal { id, text: question.into() }, t + 5_300);
        id
    }

    /// As `asking`, and the answer has opened (its first words came).
    fn answer_open(f: &mut Flow, t: Ms) -> DictationId {
        let id = asking(f, t, "что такое HTTP 418");
        f.handle(I::AskOpened { id }, t + 6_000);
        id
    }

    /// While the answer is open: a follow-up recorded by tap (its start-up done), not yet finished.
    fn follow_up(f: &mut Flow, t: Ms) -> DictationId {
        f.handle(I::ChordDown(Mode::Dictate), t);
        f.handle(I::ChordUp, t + 100);
        f.handle(I::Timer(TimerKind::StartingDone), t + STARTING_MS);
        f.current_id()
    }

    /// Every state the Drop is told about, in order.
    fn uis(fx: &[Effect]) -> Vec<UiState> {
        fx.iter().filter_map(|e| if let E::Ui(s) = e { Some(s.clone()) } else { None }).collect()
    }

    /// The Drop's "Claude key problem" after an "Ask" request, without Retry.
    fn claude_key_error() -> UiState {
        UiState { phase: UiPhase::Error, error: Some(ErrorCause::ClaudeKey), retry: false, paused: false, mode: Mode::Ask }
    }

    #[test]
    fn tap_starts_and_second_press_finishes() {
        let mut f = Flow::new(2, false);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        assert_eq!(id, DictationId(1));
        assert!(has(&fx, &E::OpenMic { id }));
        assert!(has(&fx, &E::StartStt { id }));
        assert!(has(&fx, &E::Tray(None)), "a new recording closes the tray card");
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::StartingDone, after: STARTING_MS }));
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::AutoFinish, after: AUTO_FINISH_MS }));
        assert_eq!(phase(&fx), Some(UiPhase::Start));
        assert!(f.handle(I::ChordUp, 100).is_empty());
        let fx = f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert_eq!(phase(&fx), Some(UiPhase::Listen));
        assert!(f.handle(I::Timer(TimerKind::HoldCheck), 600).is_empty());
        let fx = f.handle(I::ChordDown(Mode::Dictate), 5_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
    }

    #[test]
    fn happy_path_inserts_then_hides() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFinal { id, text: "ну привет".into() }, 5_200);
        assert!(has(&fx, &E::Refine { id, raw: "ну привет".into(), mode: Mode::Dictate }));
        let fx = f.handle(I::Refined { id, text: "Привет.".into(), outcome: RefineOutcome::Refined }, 5_900);
        assert!(has(&fx, &E::Insert { id, text: "Привет.".into() }));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_000);
        assert_eq!(phase(&fx), Some(UiPhase::Done));
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::DoneHide, after: DONE_MS }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Inserted }));
        assert!(!fx.iter().any(|e| matches!(e, E::Tray(Some(_)))), "no hint without a hold");
        assert_eq!(phase(&f.handle(I::Timer(TimerKind::DoneHide), 7_500)), Some(UiPhase::Idle));
    }

    #[test]
    fn hold_finishes_on_release_and_hint_shows_twice() {
        let mut f = Flow::new(2, false);
        for round in 0..3u64 {
            let t = round * 100_000;
            f.handle(I::ChordDown(Mode::Dictate), t);
            let id = f.current_id();
            f.handle(I::Timer(TimerKind::StartingDone), t + 380);
            f.handle(I::Timer(TimerKind::HoldCheck), t + 600);
            let fx = f.handle(I::ChordUp, t + 3_000);
            assert!(has(&fx, &E::FinishStt { id }), "release ends the recording in hold mode");
            f.handle(I::SttFinal { id, text: "текст".into() }, t + 3_200);
            f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, t + 3_500);
            let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, t + 3_600);
            let hinted = has(&fx, &E::Tray(Some(TrayKind::HoldHint)));
            assert_eq!(hinted, round < 2, "the hint shows only the first 2 times");
            assert_eq!(has(&fx, &E::HoldHintShown), round < 2);
        }
    }

    #[test]
    fn second_press_within_debounce_is_ignored() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        f.handle(I::ChordUp, 50);
        assert!(f.handle(I::ChordDown(Mode::Dictate), 200).is_empty());
    }

    #[test]
    fn system_shortcut_in_first_380ms_discards_silently() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        let fx = f.handle(I::ChordAborted, 120);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(f.handle(I::ChordAborted, 500).is_empty());
    }

    #[test]
    fn aborted_after_start_is_ignored() {
        let mut f = Flow::new(2, false);
        recording(&mut f, 0);
        assert!(f.handle(I::ChordAborted, 2_000).is_empty());
    }

    /// Ctrl+Win held a moment before the arrow: the start-up is over, the chord is still held.
    #[test]
    fn system_shortcut_while_chord_held_discards_after_start() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS);
        let fx = f.handle(I::ChordAborted, SYSTEM_CHORD_MS - 1);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(f.handle(I::ChordUp, SYSTEM_CHORD_MS + 100).is_empty(), "the release does not finish anything");
        assert_eq!(f.ui_state().phase, UiPhase::Idle);
    }

    /// The arrow after the start-up and a short release: no tap-mode recording is left running.
    #[test]
    fn system_shortcut_before_hold_check_leaves_nothing_running() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        let fx = f.handle(I::ChordAborted, 450);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(f.handle(I::ChordUp, 550).is_empty());
        assert!(f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Idle);
    }

    /// A third key late in a held dictation is a slip: the recording goes on and the release finishes it.
    #[test]
    fn late_third_key_in_held_dictation_keeps_recording() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS);
        assert!(f.handle(I::ChordAborted, SYSTEM_CHORD_MS).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Listen);
        assert!(has(&f.handle(I::ChordUp, 4_000), &E::FinishStt { id }));
    }

    #[test]
    fn esc_while_recording_cancels_and_undo_continues() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert!(f.esc_active());
        let fx = f.handle(I::Esc, 3_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }), "the text is still fetched, for \"restore\"");
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::UndoExpire, after: UNDO_MS }));
        assert_eq!(phase(&fx), Some(UiPhase::Cancel));
        assert!(!f.esc_active());
        assert!(f.handle(I::SttFinal { id, text: "сказанное".into() }, 3_300).is_empty(), "nothing is pasted while cancelled");
        let fx = f.handle(I::Ui(UiAction::Undo), 4_000);
        assert!(has(&fx, &E::CancelTimer(TimerKind::UndoExpire)));
        assert!(has(&fx, &E::Refine { id, raw: "сказанное".into(), mode: Mode::Dictate }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
    }

    #[test]
    fn undo_before_transcript_waits_for_it() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::Ui(UiAction::Undo), 3_200);
        assert!(!fx.iter().any(|e| matches!(e, E::Refine { .. })));
        let fx = f.handle(I::SttFinal { id, text: "позже".into() }, 3_600);
        assert!(has(&fx, &E::Refine { id, raw: "позже".into(), mode: Mode::Dictate }));
    }

    #[test]
    fn undo_window_expires() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::Timer(TimerKind::UndoExpire), 6_600);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn esc_while_thinking_aborts_refine_and_undo_refines_again() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "черновик");
        let fx = f.handle(I::Esc, 5_400);
        assert!(has(&fx, &E::AbortRefine { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Cancel));
        assert!(f.handle(I::Refined { id, text: "Поздно.".into(), outcome: RefineOutcome::Refined }, 5_500).is_empty());
        let fx = f.handle(I::Ui(UiAction::Undo), 6_000);
        assert!(has(&fx, &E::Refine { id, raw: "черновик".into(), mode: Mode::Dictate }));
    }

    #[test]
    fn press_while_thinking_shakes() {
        let mut f = Flow::new(2, false);
        thinking_with_raw(&mut f, 0, "x");
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 5_600), vec![E::Shake]);
    }

    #[test]
    fn stt_failure_shows_error_and_retry_uses_saved_audio() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFailed { id, error: SttError::Failed }, 6_000);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Stt), true));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let fx = f.handle(I::Ui(UiAction::Retry), 9_000);
        assert!(has(&fx, &E::TranscribeSaved { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let fx = f.handle(I::SttFailed { id, error: SttError::Model }, 9_500);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.error, ui.retry), (Some(ErrorCause::Model), true), "the model: Retry downloads it again");
    }

    /// A processor ONNX Runtime cannot run on: nothing to retry, the recording stays for the user to keep elsewhere.
    #[test]
    fn an_unsupported_processor_has_no_retry() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFailed { id, error: SttError::Unsupported }, 6_000);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Cpu), false));
        assert!(!has(&f.handle(I::Ui(UiAction::Retry), 7_000), &E::TranscribeSaved { id }));
    }

    #[test]
    fn claude_key_problem_inserts_raw_then_shows_error() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "сырой текст");
        let fx = f.handle(I::Refined { id, text: "сырой текст".into(), outcome: RefineOutcome::RawKeyProblem }, 5_600);
        assert!(has(&fx, &E::Insert { id, text: "сырой текст".into() }));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_700);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::ClaudeKey), false));
        assert!(f.esc_active(), "Esc closes the error");
        assert_eq!(phase(&f.handle(I::Esc, 8_000)), Some(UiPhase::Idle));
    }

    #[test]
    fn error_closes_by_dismiss_or_new_dictation() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 6_000);
        assert_eq!(phase(&f.handle(I::Ui(UiAction::Dismiss), 7_000)), Some(UiPhase::Idle));
        f.handle(I::ChordDown(Mode::Dictate), 10_000);
        let id2 = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 10_380);
        f.handle(I::ChordDown(Mode::Dictate), 15_000);
        f.handle(I::SttFailed { id: id2, error: SttError::Failed }, 16_000);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 20_000);
        assert!(has(&fx, &E::OpenMic { id: DictationId(id2.0 + 1) }));
    }

    #[test]
    fn empty_transcript_collapses_quietly() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFinal { id, text: "  ".into() }, 5_200);
        assert!(!fx.iter().any(|e| matches!(e, E::Refine { .. })));
        assert_eq!(phase(&fx), Some(UiPhase::Empty));
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::EmptyHide, after: EMPTY_MS }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Empty }));
        assert_eq!(phase(&f.handle(I::Timer(TimerKind::EmptyHide), 6_200)), Some(UiPhase::Idle));
    }

    #[test]
    fn no_field_opens_tray() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_600);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::NoField }, 5_700);
        assert!(has(&fx, &E::Tray(Some(TrayKind::NoField))));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::NoField }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn dead_mic_stops_and_opens_tray() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::MicDead, 4_000);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Tray(Some(TrayKind::MicSilent))));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn last_minute_warning_and_auto_finish() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert_eq!(f.handle(I::Timer(TimerKind::WarnLastMinute), WARN_MS), vec![E::Tray(Some(TrayKind::LastMinute))]);
        let fx = f.handle(I::Timer(TimerKind::AutoFinish), AUTO_FINISH_MS);
        assert!(has(&fx, &E::FinishStt { id }));
    }

    #[test]
    fn paste_last_rules() {
        let mut f = Flow::new(2, false);
        assert!(f.handle(I::PasteLast, 0).is_empty(), "nothing to paste");
        let mut f = Flow::new(2, true);
        assert_eq!(f.handle(I::PasteLast, 0), vec![E::PasteLast]);
        recording(&mut f, 1_000);
        assert!(f.handle(I::PasteLast, 3_000).is_empty(), "does nothing during recording");
        let mut f = Flow::new(2, true);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        let fx = f.handle(I::PasteLast, 50);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }), "Ctrl+Win+V does not start a dictation");
        assert_eq!(fx.last(), Some(&E::PasteLast));
    }

    #[test]
    fn pause_blocks_hotkeys_for_an_hour() {
        let mut f = Flow::new(2, true);
        let fx = f.handle(I::TogglePause, 0);
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::PauseEnd, after: PAUSE_MS }));
        assert!(last_ui(&fx).unwrap().paused);
        assert!(f.handle(I::ChordDown(Mode::Dictate), 1_000).is_empty());
        assert!(f.handle(I::PasteLast, 2_000).is_empty());
        let fx = f.handle(I::Timer(TimerKind::PauseEnd), PAUSE_MS);
        assert!(!last_ui(&fx).unwrap().paused);
        assert!(!f.handle(I::ChordDown(Mode::Dictate), PAUSE_MS + 1_000).is_empty());
    }

    /// While the welcome window holds the key (spec 4a §1) a chord starts nothing — no dictation, no follow-up, no
    /// recording in place of a "restore" — but brings the window forward, and Ctrl+Win+V pastes nothing. It is no
    /// pause: the hook keeps swallowing the chord (`hook::PAUSED` follows `paused` alone), so the Start menu does not
    /// open.
    #[test]
    fn gate_ignores_a_new_chord_and_the_hook_still_swallows_it() {
        let mut f = Flow::new(2, true);
        assert!(f.handle(I::WelcomeGate(true), 0).is_empty());
        let idle = f.ui_state();
        for (i, mode) in [Mode::Dictate, Mode::Translate, Mode::Ask].into_iter().enumerate() {
            let t = 1_000 * (i as Ms + 1);
            assert_eq!(f.handle(I::ChordDown(mode), t), [E::BringWelcome], "{mode:?}");
            assert!(f.handle(I::ChordUp, t + 100).is_empty());
        }
        assert!(f.handle(I::PasteLast, 4_000).is_empty());
        assert_eq!(f.ui_state(), idle);
        assert!(!f.ui_state().paused, "the hook still swallows Ctrl+Win");
        assert!(!f.esc_active() && !f.recording_now());
        // The dictation's step lets it go; the press it ignored counted for nothing, not even for the debounce.
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 5_000), [E::BringWelcome]);
        f.handle(I::WelcomeGate(false), 5_050);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 5_100);
        assert!(has(&fx, &E::OpenMic { id: f.current_id() }));

        // An open answer: no follow-up.
        let mut f = Flow::new(2, true);
        answer_open(&mut f, 0);
        f.handle(I::WelcomeGate(true), 7_000);
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 8_000), [E::BringWelcome]);
        assert_eq!(f.ui_state().phase, UiPhase::Answer);
        // Esc still closes it.
        assert_eq!(phase(&f.handle(I::Esc, 9_000)), Some(UiPhase::Idle));
    }

    /// A recording already running when the window takes the key ends as usual — the second press, the release of a
    /// held chord, Esc — and its text is still pasted.
    #[test]
    fn gate_lets_a_running_recording_finish() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::WelcomeGate(true), 1_000);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 5_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }) && has(&fx, &E::FinishStt { id }));
        f.handle(I::ChordUp, 5_100);
        f.handle(I::SttFinal { id, text: "ну привет".into() }, 5_200);
        let fx = f.handle(I::Refined { id, text: "Привет.".into(), outcome: RefineOutcome::Refined }, 5_900);
        assert!(has(&fx, &E::Insert { id, text: "Привет.".into() }));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Inserted }));
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 7_000), [E::BringWelcome], "done: a new chord starts nothing");

        // Held: the release ends it.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::WelcomeGate(true), 100);
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS);
        let fx = f.handle(I::ChordUp, 3_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));

        // Esc cancels it, with "restore", and a chord does not trade that for a new recording.
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::WelcomeGate(true), 1_000);
        assert!(f.esc_active());
        let fx = f.handle(I::Esc, 2_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert_eq!(phase(&fx), Some(UiPhase::Cancel));
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 3_000), [E::BringWelcome]);
        assert_eq!(f.ui_state().phase, UiPhase::Cancel);

        // Ctrl+Win+V still throws away a start-up begun before the window took the key, and pastes nothing.
        let mut f = Flow::new(2, true);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::WelcomeGate(true), 20);
        let fx = f.handle(I::PasteLast, 50);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(!has(&fx, &E::PasteLast));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn new_press_during_undo_window_finishes_previous_as_cancelled() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 4_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert!(has(&fx, &E::OpenMic { id: DictationId(id.0 + 1) }));
    }

    #[test]
    fn stale_results_are_ignored() {
        let mut f = Flow::new(2, false);
        let old = thinking_with_raw(&mut f, 0, "a");
        f.handle(I::Esc, 5_400);
        f.handle(I::ChordDown(Mode::Dictate), 6_000);
        assert!(f.handle(I::SttFinal { id: old, text: "старое".into() }, 6_100).is_empty());
        assert!(f.handle(I::Inserted { id: old, outcome: InsertOutcome::Pasted }, 6_200).is_empty());
    }

    // ---- Edge cases ----

    /// The microphone failed, but speech was already heard: finish the dictation as after a second press.
    #[test]
    fn mic_failed_after_speech_finishes_like_second_press() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::MicFailed { heard: true }, 3_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert!(!has(&fx, &E::AbortStt { id }));
        assert!(!fx.iter().any(|e| matches!(e, E::Tray(Some(_)) | E::Finished { .. })));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let fx = f.handle(I::SttFinal { id, text: "успел сказать".into() }, 3_300);
        assert!(has(&fx, &E::Refine { id, raw: "успел сказать".into(), mode: Mode::Dictate }));
        f.handle(I::Refined { id, text: "Успел сказать.".into(), outcome: RefineOutcome::Refined }, 3_800);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 3_900);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Inserted }));
        assert_eq!(phase(&fx), Some(UiPhase::Done));
    }

    #[test]
    fn mic_failed_without_speech_discards_and_opens_tray() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::MicFailed { heard: false }, 3_000);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Tray(Some(TrayKind::MicSilent))));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(!f.esc_active());
    }

    #[test]
    fn mic_failed_while_starting_discards_even_if_heard() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        let fx = f.handle(I::MicFailed { heard: true }, 100);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Tray(Some(TrayKind::MicSilent))));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(f.handle(I::Timer(TimerKind::StartingDone), 380).is_empty(), "the start-up is already dropped");
    }

    /// The pinned microphone is not there: the recording goes on with the Windows one, and the card says so.
    #[test]
    fn mic_fallback_shows_the_card_while_recording() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        assert_eq!(f.handle(I::MicFallback, 40), vec![E::Tray(Some(TrayKind::MicMissing))], "during start-up");
        assert_eq!(f.ui_state().phase, UiPhase::Start, "the recording goes on");
        let mut f = Flow::new(2, false);
        recording_in(&mut f, 0, Mode::Translate);
        assert_eq!(f.handle(I::MicFallback, 1_000), vec![E::Tray(Some(TrayKind::MicMissing))]);
        assert!(f.recording_now() && f.mode() == Mode::Translate);
    }

    /// Too late, or while an "Ask" answer is open (one card at a time, as "one minute left"): nothing.
    #[test]
    fn mic_fallback_after_recording_is_ignored() {
        let mut f = Flow::new(2, false);
        assert!(f.handle(I::MicFallback, 0).is_empty(), "idle");
        thinking_with_raw(&mut f, 1_000, "x");
        assert!(f.handle(I::MicFallback, 6_500).is_empty(), "thinking");
        let mut f = Flow::new(2, false);
        recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        assert!(f.handle(I::MicFallback, 3_100).is_empty(), "cancelled");
        let mut f = Flow::new(2, false);
        answer_open(&mut f, 0);
        follow_up(&mut f, 10_000);
        assert!(f.handle(I::MicFallback, 10_500).is_empty(), "a follow-up under an open answer");
    }

    /// One card at a time: a tray card still up when an "Ask" answer opens (a missing microphone told at the start of
    /// the question) closes, also when the answer opens with an error or as a refusal.
    #[test]
    fn an_answer_opening_closes_a_tray_card() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Ask), 0);
        let id = f.current_id();
        assert_eq!(f.handle(I::MicFallback, 40), vec![E::Tray(Some(TrayKind::MicMissing))]);
        f.handle(I::ChordUp, 100);
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::ChordDown(Mode::Dictate), 2_000);
        f.handle(I::ChordUp, 2_100);
        let fx = f.handle(I::SttFinal { id, text: "что такое HTTP 418".into() }, 2_300);
        assert!(!has(&fx, &E::Tray(None)), "the card stays while the question is asked");
        let fx = f.handle(I::AskOpened { id }, 3_000);
        let closed = fx.iter().position(|e| *e == E::Tray(None)).expect("the card goes as the answer opens");
        let shown = fx.iter().position(|e| matches!(e, E::Ui(s) if s.phase == UiPhase::Answer)).expect("the answer");
        assert!(closed < shown);
        assert!(f.handle(I::AskAnswered { id }, 4_000).iter().all(|e| *e != E::Tray(None)), "already open");

        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "что такое HTTP 418");
        let fx = f.handle(I::AskFailed { id, error: AskError::Failed }, 6_000);
        assert!(has(&fx, &E::Tray(None)) && phase(&fx) == Some(UiPhase::Answer), "the error, with Retry");

        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "что такое HTTP 418");
        let fx = f.handle(I::AskAnswered { id }, 6_000);
        assert!(has(&fx, &E::Tray(None)) && phase(&fx) == Some(UiPhase::Answer), "a refusal shown at once");
    }

    #[test]
    fn mic_events_outside_recording_are_ignored() {
        let mut f = Flow::new(2, false);
        assert!(f.handle(I::MicDead, 0).is_empty());
        assert!(f.handle(I::MicFailed { heard: true }, 0).is_empty());
        thinking_with_raw(&mut f, 1_000, "x");
        assert!(f.handle(I::MicDead, 6_500).is_empty());
        assert!(f.handle(I::MicFailed { heard: false }, 6_600).is_empty());
    }

    /// A pause during recording throws the dictation away, with no "restore", and clears the hold.
    #[test]
    fn pause_during_recording_discards_dictation() {
        let mut f = Flow::new(2, false);
        // Hold mode: the chord is still held down.
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        f.handle(I::Timer(TimerKind::HoldCheck), 600);
        let fx = f.handle(I::TogglePause, 2_000);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(has(&fx, &E::SetTimer { kind: TimerKind::PauseEnd, after: PAUSE_MS }));
        assert!(!fx.iter().any(|e| matches!(e, E::FinishStt { .. } | E::SetTimer { kind: TimerKind::UndoExpire, .. })));
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.paused), (UiPhase::Idle, true));
        assert!(!f.esc_active());
        assert!(f.handle(I::ChordUp, 2_500).is_empty(), "the hold is cleared: release ends nothing");
        f.handle(I::TogglePause, 3_000);
        assert!(f.handle(I::ChordUp, 3_100).is_empty());
        // The next dictation starts in "press and release" mode.
        f.handle(I::ChordDown(Mode::Dictate), 4_000);
        let id2 = f.current_id();
        f.handle(I::ChordUp, 4_100);
        f.handle(I::Timer(TimerKind::StartingDone), 4_380);
        f.handle(I::Timer(TimerKind::HoldCheck), 4_600);
        assert!(f.handle(I::ChordUp, 5_000).is_empty());
        assert!(has(&f.handle(I::ChordDown(Mode::Dictate), 6_000), &E::FinishStt { id: id2 }));
    }

    #[test]
    fn pause_while_starting_discards() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        let fx = f.handle(I::TogglePause, 100);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(f.handle(I::Timer(TimerKind::StartingDone), 380).is_empty());
    }

    #[test]
    fn pause_while_thinking_lets_dictation_finish() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        let fx = f.handle(I::TogglePause, 5_400);
        assert!(!fx.iter().any(|e| matches!(
            e,
            E::StopMic { .. } | E::AbortStt { .. } | E::AbortRefine { .. } | E::Finished { .. }
        )));
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.paused)), Some((UiPhase::Think, true)));
        assert!(f.handle(I::ChordDown(Mode::Dictate), 5_800).is_empty(), "no shake during a pause either");
        let fx = f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_900);
        assert!(has(&fx, &E::Insert { id, text: "Текст.".into() }));
    }

    /// An unpaired event neither starts nor ends anything.
    #[test]
    fn unpaired_chord_up_is_a_no_op() {
        let mut f = Flow::new(2, true);
        assert!(f.handle(I::ChordUp, 0).is_empty(), "when idle");
        let id = recording(&mut f, 1_000);
        assert!(f.handle(I::ChordUp, 3_000).is_empty(), "does not end a \"press and release\" recording");
        assert_eq!(f.ui_state().phase, UiPhase::Listen);
        assert!(has(&f.handle(I::ChordDown(Mode::Dictate), 4_000), &E::FinishStt { id }));
    }

    #[test]
    fn events_after_aborted_chord_are_safe() {
        let mut f = Flow::new(2, true);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        f.handle(I::ChordAborted, 100);
        assert!(f.handle(I::ChordUp, 200).is_empty(), "ChordUp after an abort");
        assert_eq!(f.handle(I::PasteLast, 250), vec![E::PasteLast], "V in an aborted chord pastes the last result");
        assert!(f.handle(I::ChordAborted, 260).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Idle);
    }

    /// A session lock throws away the ongoing recording.
    #[test]
    fn session_lock_discards_recording() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::SessionLocked, 2_000);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(!fx.iter().any(|e| matches!(e, E::SetTimer { kind: TimerKind::UndoExpire, .. } | E::Tray(Some(_)))));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        f.handle(I::ChordDown(Mode::Dictate), 3_000);
        let id2 = f.current_id();
        let fx = f.handle(I::SessionLocked, 3_100);
        assert!(has(&fx, &E::Finished { id: id2, outcome: Outcome::Discarded }), "during start-up too");
    }

    #[test]
    fn session_lock_outside_recording_changes_nothing() {
        let mut f = Flow::new(2, false);
        assert!(f.handle(I::SessionLocked, 0).is_empty());
        let id = thinking_with_raw(&mut f, 1_000, "текст");
        assert!(f.handle(I::SessionLocked, 6_400).is_empty(), "in \"thinking\"");
        let fx = f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 6_500);
        assert!(has(&fx, &E::Insert { id, text: "Текст.".into() }));
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_600);
        assert!(f.handle(I::SessionLocked, 6_700).is_empty(), "in \"done\"");
        recording(&mut f, 20_000);
        f.handle(I::Esc, 22_000);
        assert!(f.handle(I::SessionLocked, 22_100).is_empty(), "in \"cancelled\"");
        assert_eq!(phase(&f.handle(I::Ui(UiAction::Undo), 22_200)), Some(UiPhase::Think));
    }

    // ---- More flow cases: timings, boundaries, ordering ----

    #[test]
    fn flow_timings_are_armed() {
        assert_eq!((STARTING_MS, HOLD_MS, DEBOUNCE_MS, DONE_MS, UNDO_MS), (380, 600, 300, 1_500, 3_600));
        assert_eq!(SYSTEM_CHORD_MS, 1_500);
        assert_eq!((WARN_MS, AUTO_FINISH_MS, PAUSE_MS), (480_000, 540_000, 3_600_000));
        let mut f = Flow::new(2, false);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 0);
        for (kind, after) in [
            (TimerKind::StartingDone, STARTING_MS),
            (TimerKind::HoldCheck, HOLD_MS),
            (TimerKind::WarnLastMinute, WARN_MS),
            (TimerKind::AutoFinish, AUTO_FINISH_MS),
        ] {
            assert!(has(&fx, &E::SetTimer { kind, after }), "{kind:?}");
        }
    }

    #[test]
    fn hold_threshold_and_debounce_boundaries() {
        // Released after 599 ms: "press and release".
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert!(f.handle(I::ChordUp, 599).is_empty());
        assert!(f.handle(I::Timer(TimerKind::HoldCheck), 600).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Listen);
        // Held for 600 ms: "hold".
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        f.handle(I::Timer(TimerKind::HoldCheck), 600);
        assert!(has(&f.handle(I::ChordUp, 600), &E::FinishStt { id }));
        // Debounce: a press after 299 ms does not count, after 300 ms it does.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        f.handle(I::ChordAborted, 50);
        f.handle(I::ChordUp, 60);
        assert!(f.handle(I::ChordDown(Mode::Dictate), 299).is_empty());
        assert!(has(&f.handle(I::ChordDown(Mode::Dictate), 300), &E::OpenMic { id: DictationId(2) }));
    }

    #[test]
    fn release_during_start_is_tap_mode_even_if_timers_reorder() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::HoldCheck), 360);
        assert!(f.handle(I::ChordUp, 370).is_empty());
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert!(has(&f.handle(I::ChordDown(Mode::Dictate), 5_000), &E::FinishStt { id }));
        f.handle(I::SttFinal { id, text: "т".into() }, 5_100);
        f.handle(I::Refined { id, text: "Т.".into(), outcome: RefineOutcome::Refined }, 5_200);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_300);
        assert!(!has(&fx, &E::Tray(Some(TrayKind::HoldHint))), "this was \"press and release\"");
    }

    /// "Press again and the recording ends"; the only exception is a press within 0.3 s.
    #[test]
    fn second_press_during_start_finishes() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::ChordUp, 100);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 350);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert!(has(&fx, &E::CancelTimer(TimerKind::StartingDone)));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        assert!(f.handle(I::Timer(TimerKind::StartingDone), 380).is_empty());
        assert!(f.handle(I::ChordAborted, 390).is_empty(), "the start-up is gone, nothing to abort");
    }

    /// "Held for 0.6 s or longer" is decided by the release time, not by timer order.
    #[test]
    fn hold_is_decided_by_time_not_timer_order() {
        // HoldCheck on time, StartingDone late: held for 650 ms, so release ends the recording.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::HoldCheck), 600);
        let fx = f.handle(I::ChordUp, 650);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        assert!(f.handle(I::Timer(TimerKind::StartingDone), 700).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        // HoldCheck late: held for 700 ms, still "hold", and the hint is shown.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert!(has(&f.handle(I::ChordUp, 700), &E::FinishStt { id }));
        assert!(f.handle(I::Timer(TimerKind::HoldCheck), 710).is_empty());
        f.handle(I::SttFinal { id, text: "т".into() }, 800);
        f.handle(I::Refined { id, text: "Т.".into(), outcome: RefineOutcome::Refined }, 900);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 1_000);
        assert!(has(&fx, &E::Tray(Some(TrayKind::HoldHint))));
    }

    /// An empty refine result would fail the sanity check: the raw text is pasted.
    #[test]
    fn empty_refined_text_inserts_raw() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "сырой текст");
        let fx = f.handle(I::Refined { id, text: "  ".into(), outcome: RefineOutcome::Refined }, 5_400);
        assert!(has(&fx, &E::Insert { id, text: "сырой текст".into() }));
        assert!(!fx.iter().any(|e| matches!(e, E::Finished { .. })));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Inserted }));
        assert_eq!(phase(&fx), Some(UiPhase::Done), "as with RawFallback: a normal \"done\"");
        // A Claude key problem is not lost.
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "сырой текст");
        let fx = f.handle(I::Refined { id, text: String::new(), outcome: RefineOutcome::RawKeyProblem }, 5_400);
        assert!(has(&fx, &E::Insert { id, text: "сырой текст".into() }));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        assert_eq!(last_ui(&fx).unwrap().error, Some(ErrorCause::ClaudeKey));
    }

    #[test]
    fn ui_state_reports_phase_and_pause() {
        let mut f = Flow::new(2, false);
        assert_eq!(f.ui_state(), UiState { phase: UiPhase::Idle, error: None, retry: false, paused: false, mode: Mode::Dictate });
        f.handle(I::ChordDown(Mode::Dictate), 1_000);
        assert_eq!(f.ui_state().phase, UiPhase::Start);
        f.handle(I::Timer(TimerKind::StartingDone), 1_380);
        assert_eq!(f.ui_state().phase, UiPhase::Listen);
        f.handle(I::Ui(UiAction::Finish), 5_000);
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        f.handle(I::TogglePause, 5_200);
        assert!(f.ui_state().paused);
        let fx = f.handle(I::TogglePause, 5_400);
        assert!(has(&fx, &E::CancelTimer(TimerKind::PauseEnd)));
        assert!(!f.ui_state().paused);
    }

    #[test]
    fn drop_buttons_finish_and_cancel() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::Ui(UiAction::Finish), 2_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        let fx = f.handle(I::Ui(UiAction::Cancel), 2_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert_eq!(phase(&fx), Some(UiPhase::Cancel));
        for action in [UiAction::Cancel, UiAction::Finish, UiAction::Retry, UiAction::Dismiss, UiAction::TrayAction, UiAction::TrayClose] {
            assert!(f.handle(I::Ui(action), 2_100).is_empty(), "{action:?} in \"cancelled\"");
        }
    }

    /// Pasting looks like "thinking": Esc does not reach the app, but it does not cancel either, as the paste is under way.
    #[test]
    fn esc_is_swallowed_but_ignored_while_inserting() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_400);
        assert!(f.esc_active(), "Esc is swallowed while the Drop shows \"thinking\"");
        assert!(f.handle(I::Esc, 5_450).is_empty());
        assert!(f.handle(I::Ui(UiAction::Cancel), 5_455).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        assert!(f.handle(I::PasteLast, 5_460).is_empty());
        assert_eq!(f.handle(I::ChordDown(Mode::Dictate), 5_470), vec![E::Shake]);
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        assert!(!f.esc_active());
        assert!(f.handle(I::Esc, 5_600).is_empty());
    }

    #[test]
    fn paste_last_without_result_still_cancels_start() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        let fx = f.handle(I::PasteLast, 50);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }), "Ctrl+Win+V is not dictation, even with nothing to paste");
        assert!(!has(&fx, &E::PasteLast));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn paste_last_becomes_available_after_insert() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_400);
        f.handle(I::Inserted { id, outcome: InsertOutcome::NoField }, 5_500);
        assert_eq!(f.handle(I::PasteLast, 6_000), vec![E::PasteLast]);
    }

    #[test]
    fn early_stt_failure_is_shown_when_recording_ends() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert!(f.handle(I::SttFailed { id, error: SttError::Model }, 2_000).is_empty(), "recording goes on");
        let fx = f.handle(I::ChordDown(Mode::Dictate), 5_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }), "the recording is kept for \"retry\"");
        assert!(!has(&fx, &E::FinishStt { id }), "transcription has already ended");
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Model), true));
        assert!(has(&f.handle(I::Ui(UiAction::Retry), 6_000), &E::TranscribeSaved { id }));
    }

    #[test]
    fn early_stt_final_is_used_when_recording_ends() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert!(f.handle(I::SttFinal { id, text: "рано".into() }, 2_000).is_empty());
        let fx = f.handle(I::ChordDown(Mode::Dictate), 5_000);
        assert!(has(&fx, &E::Refine { id, raw: "рано".into(), mode: Mode::Dictate }));
        assert!(!has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
    }

    #[test]
    fn duplicate_results_are_ignored() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "раз");
        assert!(f.handle(I::SttFinal { id, text: "два".into() }, 5_400).is_empty(), "refine is not requested a second time");
        assert!(f.handle(I::SttFailed { id, error: SttError::Failed }, 5_450).is_empty());
        f.handle(I::Refined { id, text: "Раз.".into(), outcome: RefineOutcome::Refined }, 5_500);
        assert!(f.handle(I::Refined { id, text: "Раз!".into(), outcome: RefineOutcome::Refined }, 5_550).is_empty());
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_600);
        assert!(f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_650).is_empty());
    }

    #[test]
    fn final_cancel_aborts_pending_transcript() {
        // The "restore" window expired and no text came: transcription is aborted.
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::Timer(TimerKind::UndoExpire), 6_600);
        assert!(has(&fx, &E::AbortStt { id }));
        // A new recording during the "restore" window: the old transcription is aborted before the new recording.
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 4_000);
        let abort = fx.iter().position(|e| *e == E::AbortStt { id });
        let open = fx.iter().position(|e| matches!(e, E::OpenMic { .. }));
        assert!(abort.is_some() && abort < open);
        // The text has already arrived: nothing to abort.
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        f.handle(I::SttFinal { id, text: "есть".into() }, 3_200);
        assert!(!has(&f.handle(I::Timer(TimerKind::UndoExpire), 6_600), &E::AbortStt { id }));
    }

    #[test]
    fn stale_timers_are_ignored() {
        use TimerKind::*;
        let all = [StartingDone, HoldCheck, DoneHide, EmptyHide, UndoExpire, WarnLastMinute, AutoFinish, PauseEnd];
        let mut f = Flow::new(2, false);
        for kind in all {
            assert!(f.handle(I::Timer(kind), 0).is_empty(), "{kind:?} when idle");
        }
        recording(&mut f, 1_000);
        for kind in [StartingDone, HoldCheck, DoneHide, EmptyHide, UndoExpire, PauseEnd] {
            assert!(f.handle(I::Timer(kind), 2_000).is_empty(), "{kind:?} during recording");
        }
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        for kind in all {
            assert!(f.handle(I::Timer(kind), 5_100).is_empty(), "{kind:?} in \"thinking\"");
        }
        // "Done" → a new recording: a late DoneHide does not touch it.
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_400);
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        f.handle(I::ChordDown(Mode::Dictate), 6_000);
        assert!(f.handle(I::Timer(DoneHide), 7_000).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Start);
    }

    // ---- Modes: Shift translates, Alt asks (spec 2 §4) ----

    #[test]
    fn translate_chord_carries_its_mode_to_refine() {
        let mut f = Flow::new(2, false);
        let fx = f.handle(I::ChordDown(Mode::Translate), 0);
        assert_eq!(last_ui(&fx).map(|s| s.mode), Some(Mode::Translate));
        let id = f.current_id();
        f.handle(I::ChordUp, 100);
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        assert!(has(&fx, &E::Refine { id, raw: "привет".into(), mode: Mode::Translate }), "the finishing press does not change the mode");
    }

    /// Toggles the mode and returns the mode the Drop was told about.
    fn toggle(f: &mut Flow, t: Ms) -> Option<Mode> {
        last_ui(&f.handle(I::ModeToggle, t)).map(|s| s.mode)
    }

    #[test]
    fn shift_toggles_the_mode_after_a_tap() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert_eq!(toggle(&mut f, 1_000), Some(Mode::Translate), "the Drop switches its icon at once");
        assert_eq!(f.mode(), Mode::Translate);
        assert_eq!(toggle(&mut f, 2_000), Some(Mode::Dictate), "a second Shift goes back to dictation");
        assert_eq!(toggle(&mut f, 3_000), Some(Mode::Translate));
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        assert!(f.handle(I::ModeToggle, 5_050).is_empty(), "the finishing press has moved the flow to thinking");
        f.handle(I::ChordUp, 5_100);
        let fx = f.handle(I::SttFinal { id, text: "текст".into() }, 5_300);
        assert!(has(&fx, &E::Refine { id, raw: "текст".into(), mode: Mode::Translate }));
    }

    #[test]
    fn shift_toggles_during_start_up() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        f.handle(I::ChordUp, 100);
        assert_eq!(toggle(&mut f, 200), Some(Mode::Translate), "still starting: the toggle counts");
        let fx = f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.mode)), Some((UiPhase::Listen, Mode::Translate)));
    }

    #[test]
    fn shift_toggles_while_the_chord_is_held_and_hold_refines_in_the_mode_at_release() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS);
        assert_eq!(toggle(&mut f, 1_000), Some(Mode::Translate));
        assert_eq!(toggle(&mut f, 2_000), Some(Mode::Dictate), "Shift released and pressed again");
        f.handle(I::ChordUp, 3_000);
        let fx = f.handle(I::SttFinal { id, text: "текст".into() }, 3_200);
        assert!(has(&fx, &E::Refine { id, raw: "текст".into(), mode: Mode::Dictate }));
    }

    #[test]
    fn translate_chord_toggles_back_to_dictation() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        assert_eq!(toggle(&mut f, 1_000), Some(Mode::Dictate));
        f.handle(I::ChordDown(Mode::Translate), 5_000);
        let fx = f.handle(I::SttFinal { id, text: "текст".into() }, 5_300);
        assert!(has(&fx, &E::Refine { id, raw: "текст".into(), mode: Mode::Dictate }), "the finishing chord's mode does not count");
    }

    #[test]
    fn shift_outside_a_recording_changes_nothing() {
        let mut f = Flow::new(2, false);
        assert!(f.handle(I::ModeToggle, 0).is_empty(), "idle");
        let id = thinking_with_raw(&mut f, 1_000, "текст");
        assert!(f.handle(I::ModeToggle, 6_400).is_empty(), "thinking");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 6_500);
        assert!(f.handle(I::ModeToggle, 6_550).is_empty(), "pasting");
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_600);
        assert_eq!(f.ui_state().phase, UiPhase::Done);
        assert!(f.handle(I::ModeToggle, 6_700).is_empty(), "done");
        recording(&mut f, 10_000);
        f.handle(I::Esc, 11_000);
        assert!(f.handle(I::ModeToggle, 11_100).is_empty(), "cancelled");
        let id = recording(&mut f, 20_000);
        f.handle(I::ChordDown(Mode::Dictate), 25_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 25_200);
        assert!(f.handle(I::ModeToggle, 25_300).is_empty(), "error");
        f.handle(I::Ui(UiAction::Dismiss), 25_400);
        f.handle(I::TogglePause, 26_000);
        assert!(f.handle(I::ModeToggle, 26_100).is_empty(), "paused");
        assert_eq!(f.mode(), Mode::Dictate);
    }

    #[test]
    fn ask_needs_the_starting_chord_held() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        let fx = f.handle(I::ModeChange(Mode::Ask), 200);
        assert_eq!(last_ui(&fx).map(|s| s.mode), Some(Mode::Ask), "Alt inside the held chord");
        assert!(f.handle(I::ModeChange(Mode::Ask), 250).is_empty(), "the same mode again changes nothing");
        assert_eq!(toggle(&mut f, 300), Some(Mode::Translate), "Shift turns \"Ask\" into a translation");
        f.handle(I::ChordUp, 400);
        assert!(f.handle(I::ModeChange(Mode::Ask), 1_000).is_empty(), "the chord was released: Alt changes nothing");
        assert_eq!(f.mode(), Mode::Translate);
    }

    /// A toggle is undone by the next one: a translation goes back to the mode it was switched from, "Ask" included, so
    /// a second Shift, or the hook taking a toggle back, never turns a question into a dictation.
    #[test]
    fn a_translation_toggles_back_to_the_mode_it_came_from() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Ask);
        assert_eq!(toggle(&mut f, 1_000), Some(Mode::Translate));
        assert_eq!(toggle(&mut f, 1_100), Some(Mode::Ask), "back to \"Ask\", not to dictation");
        assert_eq!(toggle(&mut f, 1_200), Some(Mode::Translate));
        assert_eq!(toggle(&mut f, 1_300), Some(Mode::Ask));
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::ChordUp, 5_100);
        let fx = f.handle(I::SttFinal { id, text: "какая погода".into() }, 5_300);
        assert!(fx.iter().any(|e| matches!(e, E::Ask { .. })), "the question is asked");
        // A new recording forgets where the last translation came from.
        let mut f = Flow::new(2, false);
        recording_in(&mut f, 0, Mode::Ask);
        toggle(&mut f, 1_000);
        f.handle(I::Esc, 2_000);
        recording_in(&mut f, 10_000, Mode::Translate);
        assert_eq!(toggle(&mut f, 11_000), Some(Mode::Dictate), "a translation chord toggles to dictation");
        // Alt inside the held chord picks the mode anew: a translation toggled before it does not go back to dictation.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Dictate), 0);
        assert_eq!(toggle(&mut f, 100), Some(Mode::Translate));
        f.handle(I::ModeChange(Mode::Ask), 200);
        assert_eq!(toggle(&mut f, 300), Some(Mode::Translate));
        assert_eq!(toggle(&mut f, 400), Some(Mode::Ask));
    }

    #[test]
    fn idle_drop_shows_no_mode() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "текст".into() }, 5_200);
        f.handle(I::Refined { id, text: "Text.".into(), outcome: RefineOutcome::Refined }, 5_900);
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_000);
        let fx = f.handle(I::Timer(TimerKind::DoneHide), 7_500);
        assert_eq!(last_ui(&fx).map(|s| s.mode), Some(Mode::Dictate));
    }

    #[test]
    fn quietly_discarded_translation_leaves_no_mode() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown(Mode::Translate), 0);
        // Ctrl+Shift+Win+B: the third key comes within SYSTEM_CHORD_MS.
        let fx = f.handle(I::ChordAborted, 120);
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.mode)), Some((UiPhase::Idle, Mode::Dictate)));
        assert_eq!(f.mode(), Mode::Dictate);
    }

    /// Every other way a recording is thrown away into idle: Ctrl+Win+V during start-up, a pause, a lock, a dead microphone.
    #[test]
    fn dropped_translation_leaves_no_mode() {
        for input in [I::PasteLast, I::TogglePause, I::SessionLocked, I::MicDead] {
            let mut f = Flow::new(2, true);
            f.handle(I::ChordDown(Mode::Translate), 0);
            let fx = f.handle(input.clone(), 100);
            assert_eq!(last_ui(&fx).map(|s| (s.phase, s.mode)), Some((UiPhase::Idle, Mode::Dictate)), "{input:?}");
            assert_eq!(f.mode(), Mode::Dictate, "{input:?}");
        }
    }

    // ---- Translation errors (spec 2 §5, §8) ----

    #[test]
    fn failed_translation_is_an_error_and_pastes_nothing() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        let fx = f.handle(I::Refined { id, text: "привет".into(), outcome: RefineOutcome::RawFallback }, 6_000);
        assert!(!fx.iter().any(|e| matches!(e, E::Insert { .. })), "the untranslated text is never pasted");
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Translate), true));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let fx = f.handle(I::Ui(UiAction::Retry), 7_000);
        assert!(has(&fx, &E::RefineAgain { id, raw: "привет".into(), mode: Mode::Translate }), "retry refines the saved text");
        assert!(!fx.iter().any(|e| matches!(e, E::TranscribeSaved { .. })));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let fx = f.handle(I::Refined { id, text: "Hi.".into(), outcome: RefineOutcome::Refined }, 8_000);
        assert!(has(&fx, &E::Insert { id, text: "Hi.".into() }));
    }

    #[test]
    fn translation_key_problem_shows_the_key_error_with_retry() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        let fx = f.handle(I::Refined { id, text: "привет".into(), outcome: RefineOutcome::RawKeyProblem }, 6_000);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.error, ui.retry), (Some(ErrorCause::ClaudeKey), true));
        assert!(!fx.iter().any(|e| matches!(e, E::Insert { .. })));
    }

    /// An empty "translation" is a failure too, not a reason to paste what was said.
    #[test]
    fn empty_translation_is_an_error() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        let fx = f.handle(I::Refined { id, text: "  ".into(), outcome: RefineOutcome::Refined }, 6_000);
        assert!(!fx.iter().any(|e| matches!(e, E::Insert { .. })));
        assert_eq!(last_ui(&fx).unwrap().error, Some(ErrorCause::Translate));
    }

    #[test]
    fn dictation_still_pastes_the_raw_text_on_fallback() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "привет");
        let fx = f.handle(I::Refined { id, text: "привет".into(), outcome: RefineOutcome::RawFallback }, 6_000);
        assert!(has(&fx, &E::Insert { id, text: "привет".into() }));
    }

    #[test]
    fn stt_error_retry_still_transcribes_again() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 5_200);
        let fx = f.handle(I::Ui(UiAction::Retry), 6_000);
        assert!(has(&fx, &E::TranscribeSaved { id }));
    }

    /// Esc during the retried translation, then "restore": the saved text is refined again in the same mode.
    #[test]
    fn cancelled_retry_restores_into_a_translation() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        f.handle(I::Refined { id, text: "привет".into(), outcome: RefineOutcome::RawFallback }, 6_000);
        f.handle(I::Ui(UiAction::Retry), 7_000);
        assert!(has(&f.handle(I::Esc, 7_100), &E::AbortRefine { id }));
        let fx = f.handle(I::Ui(UiAction::Undo), 7_500);
        assert!(has(&fx, &E::Refine { id, raw: "привет".into(), mode: Mode::Translate }));
    }

    /// A new dictation after a failed translation starts clean: its own transcription error retries the audio.
    #[test]
    fn a_new_dictation_forgets_the_text_retry() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        f.handle(I::SttFinal { id, text: "привет".into() }, 5_200);
        f.handle(I::Refined { id, text: "привет".into(), outcome: RefineOutcome::RawFallback }, 6_000);
        let id2 = recording_in(&mut f, 10_000, Mode::Translate);
        f.handle(I::ChordDown(Mode::Dictate), 15_000);
        f.handle(I::SttFailed { id: id2, error: SttError::Failed }, 15_200);
        let fx = f.handle(I::Ui(UiAction::Retry), 16_000);
        assert!(has(&fx, &E::TranscribeSaved { id: id2 }));
        assert!(!fx.iter().any(|e| matches!(e, E::RefineAgain { .. })));
    }

    // ---- "Ask": the answer, follow-ups, pastes (spec 2 §6.2, §7, §8) ----

    #[test]
    fn ask_chord_asks_the_model_instead_of_refine() {
        let mut f = Flow::new(2, false);
        let id = recording_in(&mut f, 0, Mode::Ask);
        f.handle(I::ChordDown(Mode::Dictate), 5_000);
        let fx = f.handle(I::SttFinal { id, text: "что такое HTTP 418".into() }, 5_300);
        assert!(has(&fx, &E::Ask { id, question: "что такое HTTP 418".into(), follow_up: false }));
        assert!(!fx.iter().any(|e| matches!(e, E::Refine { .. })), "an \"Ask\" question is not cleaned up");
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        assert!(!f.answer_open());
    }

    #[test]
    fn the_answer_opens_on_its_first_words_and_is_counted_once_complete() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "что такое HTTP 418");
        let fx = f.handle(I::AskOpened { id }, 6_000);
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.mode)), Some((UiPhase::Answer, Mode::Ask)));
        assert!(f.answer_open() && f.esc_active(), "Esc closes an open answer");
        assert!(!f.recording_now());
        assert!(f.handle(I::AskOpened { id }, 6_100).is_empty(), "it opens once");
        assert_eq!(f.handle(I::AskAnswered { id }, 7_000), vec![E::Finished { id, outcome: Outcome::Answered }], "it stays open");
        assert!(f.handle(I::AskAnswered { id }, 7_100).is_empty(), "a repeat is ignored");
    }

    #[test]
    fn an_answer_that_ends_before_it_showed_opens_complete() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "спасибо");
        let fx = f.handle(I::AskAnswered { id }, 6_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Answered }));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
    }

    #[test]
    fn esc_closes_the_answer_and_forgets_the_conversation() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id }, 7_000);
        let idle = UiState { phase: UiPhase::Idle, error: None, retry: false, paused: false, mode: Mode::Dictate };
        assert_eq!(f.handle(I::Esc, 8_000), vec![E::CloseAnswer, E::Ui(idle)]);
        assert!(!f.answer_open() && !f.esc_active());
        assert!(f.handle(I::Ui(UiAction::AnswerClose), 8_100).is_empty(), "already closed");
    }

    #[test]
    fn closing_an_answer_that_still_streams_aborts_it() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        let fx = f.handle(I::Ui(UiAction::AnswerClose), 6_500);
        assert_eq!(&fx[..3], &[E::AbortAsk { id }, E::Finished { id, outcome: Outcome::Cancelled }, E::CloseAnswer]);
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(f.handle(I::AskAnswered { id }, 7_000).is_empty(), "the late end of the aborted request is ignored");
    }

    #[test]
    fn a_chord_while_the_answer_is_open_records_a_follow_up() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 10_000);
        let id = f.current_id();
        assert_ne!(id, first, "a follow-up is a recording of its own");
        assert!(has(&fx, &E::OpenMic { id }) && has(&fx, &E::StartStt { id }));
        assert!(!fx.contains(&E::CloseAnswer), "the answer stays open");
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.mode)), Some((UiPhase::Start, Mode::Ask)));
        f.handle(I::ChordUp, 10_100);
        f.handle(I::Timer(TimerKind::StartingDone), 10_380);
        assert!(f.handle(I::ModeToggle, 11_000).is_empty(), "Shift does not turn a follow-up into a translation");
        assert!(f.handle(I::ModeChange(Mode::Ask), 11_100).is_empty());
        f.handle(I::ChordDown(Mode::Dictate), 14_000);
        let fx = f.handle(I::SttFinal { id, text: "а для пяти серверов".into() }, 14_300);
        assert!(has(&fx, &E::Ask { id, question: "а для пяти серверов".into(), follow_up: true }));
        assert_eq!(phase(&f.handle(I::AskOpened { id }, 15_000)), Some(UiPhase::Answer));
        assert_eq!(f.handle(I::AskAnswered { id }, 16_000), vec![E::Finished { id, outcome: Outcome::Answered }]);
    }

    /// Decision 12 (the approved design): a chord while the answer still prints records a follow-up at once; the answer
    /// keeps printing, and the follow-up goes when both have ended — no abort, a clean history.
    #[test]
    fn a_chord_while_the_answer_still_comes_records_a_follow_up_that_waits_for_it() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let fx = f.handle(I::ChordDown(Mode::Dictate), 7_000);
        let id = f.current_id();
        assert_ne!(id, first);
        assert!(has(&fx, &E::OpenMic { id }) && !has(&fx, &E::AbortAsk { id: first }), "the answer keeps printing");
        f.handle(I::ChordUp, 7_100);
        f.handle(I::Timer(TimerKind::StartingDone), 7_000 + STARTING_MS);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        let fx = f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        assert!(!fx.iter().any(|e| matches!(e, E::Ask { .. })), "it waits for the answer it follows");
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        let fx = f.handle(I::AskAnswered { id: first }, 10_000);
        assert_eq!(
            fx,
            vec![E::Finished { id: first, outcome: Outcome::Answered }, E::Ask { id, question: "а подробнее".into(), follow_up: true }]
        );
        assert_eq!(phase(&f.handle(I::AskOpened { id }, 11_000)), Some(UiPhase::Answer), "a follow-up's first step");
    }

    /// A write that ends an answer is shown rather than pasted under the user's hands (decision 12): from the
    /// follow-up's recording through its transcription, its wait and its miss, to its drop.
    #[test]
    fn a_follow_up_is_under_way_until_it_is_dropped() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        assert!(!f.follow_up_under_way(first), "nothing records: the answer's own request");
        let id = follow_up(&mut f, 7_000);
        assert!(f.follow_up_under_way(first), "recording");
        assert!(!f.follow_up_under_way(id), "the follow-up's own request is not one");
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        assert!(f.follow_up_under_way(first), "being transcribed");
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        assert!(f.follow_up_under_way(first), "waiting for the answer it follows");
        f.handle(I::Esc, 9_500);
        assert!(!f.follow_up_under_way(first), "dropped: back at the answer");

        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 9_300);
        assert_eq!(f.ui_state().phase, UiPhase::Error);
        assert!(f.follow_up_under_way(first), "missed, with its Retry");
    }

    #[test]
    fn esc_drops_a_waiting_follow_up_and_the_answer_goes_on() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        let fx = f.handle(I::Esc, 9_500);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }) && !has(&fx, &E::AbortAsk { id: first }));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
        assert_eq!(
            f.handle(I::AskAnswered { id: first }, 10_000),
            vec![E::Finished { id: first, outcome: Outcome::Answered }],
            "nothing waits any more"
        );
    }

    #[test]
    fn a_key_problem_while_a_follow_up_records_drops_it_too() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        let fx = f.handle(I::AskFailed { id: first, error: AskError::Key }, 8_000);
        assert!(has(&fx, &E::Finished { id: first, outcome: Outcome::Failed }));
        assert!(has(&fx, &E::StopMic { id, keep: false }) && has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(has(&fx, &E::CloseAnswer));
        assert_eq!(uis(&fx), vec![claude_key_error()], "straight to the error, no answer in between");
        assert!(!f.answer_open());
    }

    /// A key problem while the follow-up is transcribed or waits for the answer it follows: it goes too.
    #[test]
    fn a_key_problem_while_a_follow_up_is_transcribed_or_waits_drops_it_too() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        let fx = f.handle(I::AskFailed { id: first, error: AskError::Key }, 9_100);
        assert_eq!(
            fx,
            vec![
                E::Finished { id: first, outcome: Outcome::Failed },
                E::AbortStt { id },
                E::Finished { id, outcome: Outcome::Discarded },
                E::CloseAnswer,
                E::Ui(claude_key_error()),
            ]
        );
        assert!(f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300).is_empty(), "its transcription was aborted");

        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        let fx = f.handle(I::AskFailed { id: first, error: AskError::Key }, 9_500);
        assert_eq!(
            fx,
            vec![
                E::Finished { id: first, outcome: Outcome::Failed },
                E::Finished { id, outcome: Outcome::Discarded },
                E::CloseAnswer,
                E::Ui(claude_key_error()),
            ],
            "the waiting question never goes"
        );
    }

    #[test]
    fn esc_drops_a_follow_up_and_keeps_the_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        let fx = f.handle(I::Esc, 11_000);
        assert!(has(&fx, &E::StopMic { id, keep: false }) && has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert!(!fx.contains(&E::CloseAnswer));
        assert!(!fx.iter().any(|e| matches!(e, E::SetTimer { kind: TimerKind::UndoExpire, .. })), "no restore for a follow-up");
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
        assert!(f.answer_open());
        // A follow-up being asked: the request goes, the answer stays.
        let id = follow_up(&mut f, 20_000);
        f.handle(I::ChordDown(Mode::Dictate), 23_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 23_300);
        let fx = f.handle(I::Esc, 23_500);
        assert!(has(&fx, &E::AbortAsk { id }) && has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert!(!fx.contains(&E::CloseAnswer));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
    }

    #[test]
    fn an_empty_follow_up_goes_back_to_the_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        let fx = f.handle(I::SttFinal { id, text: " ".into() }, 12_300);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Empty }));
        assert!(!fx.iter().any(|e| matches!(e, E::SetTimer { kind: TimerKind::EmptyHide, .. })));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
    }

    #[test]
    fn a_write_pastes_like_a_dictation_and_ends_the_conversation() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "напиши письмо Ане");
        let fx = f.handle(I::AskWrite { id, how: PasteHow::AtCaret }, 6_000);
        assert_eq!(&fx[..2], &[E::AskPaste { id, how: PasteHow::AtCaret }, E::CloseAnswer], "the text is taken before the conversation goes");
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 6_300);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Inserted }));
        assert_eq!(phase(&fx), Some(UiPhase::Done));
    }

    /// CUE never injects input while paused: a write that ends during a pause — a first question still in flight when
    /// the pause began — is shown in the answer, with Copy, and counted as answered.
    #[test]
    fn a_write_that_ends_during_a_pause_is_shown_not_pasted() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "напиши письмо Ане");
        assert!(!f.shows_write(id), "not paused: pasted");
        f.handle(I::TogglePause, 5_500);
        assert!(f.shows_write(id), "paused: the runtime shows it");
        let fx = f.handle(I::AskWrite { id, how: PasteHow::AtCaret }, 6_000);
        let pasted = fx.iter().any(|e| matches!(e, E::AskPaste { .. } | E::CloseAnswer));
        assert!(!pasted, "nothing pasted, the answer stays");
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Answered }));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
        assert!(f.answer_open());
        // Still paused, the answer's Copy works and its cross closes it.
        let fx = f.handle(I::Ui(UiAction::AnswerClose), 7_000);
        assert!(has(&fx, &E::CloseAnswer));
    }

    #[test]
    fn a_write_after_words_already_shown_closes_the_answer() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        let fx = f.handle(I::AskWrite { id, how: PasteHow::Below }, 6_500);
        assert_eq!(&fx[..2], &[E::AskPaste { id, how: PasteHow::Below }, E::CloseAnswer]);
        assert!(!f.answer_open());
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::NoField }, 7_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::NoField }) && has(&fx, &E::Tray(Some(TrayKind::NoField))));
    }

    #[test]
    fn answer_buttons_paste_once_the_answer_is_complete() {
        for (action, how) in [
            (UiAction::AnswerInsert, PasteHow::AtCaret),
            (UiAction::AnswerReplace, PasteHow::Replace),
            (UiAction::AnswerBelow, PasteHow::Below),
        ] {
            let mut f = Flow::new(2, false);
            let id = answer_open(&mut f, 0);
            assert!(f.handle(I::Ui(action), 6_100).is_empty(), "not while it streams: {action:?}");
            f.handle(I::AskAnswered { id }, 7_000);
            assert!(f.handle(I::Ui(UiAction::AnswerCopy), 7_500).is_empty(), "the runtime copies; the flow stays");
            let fx = f.handle(I::Ui(action), 8_000);
            assert_eq!(&fx[..2], &[E::AskPaste { id, how }, E::CloseAnswer], "{action:?}");
            let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 8_300);
            assert!(!fx.iter().any(|e| matches!(e, E::Finished { .. })), "the answer was counted when it was complete");
            assert_eq!(phase(&fx), Some(UiPhase::Done));
        }
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id }, 7_000);
        f.handle(I::Ui(UiAction::AnswerInsert), 8_000);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::NoField }, 8_300);
        assert!(has(&fx, &E::Tray(Some(TrayKind::NoField))));
        assert!(!fx.iter().any(|e| matches!(e, E::Finished { .. })));
    }

    #[test]
    fn a_failed_request_shows_retry_inside_the_answer() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "курс евро");
        let fx = f.handle(I::AskFailed { id, error: AskError::Failed }, 8_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
        assert!(f.handle(I::Ui(UiAction::AnswerInsert), 8_100).is_empty(), "nothing to paste");
        assert_eq!(f.handle(I::Ui(UiAction::AnswerRetry), 9_000), vec![E::AskAgain { id }]);
        assert!(f.handle(I::Ui(UiAction::AnswerRetry), 9_100).is_empty(), "one request at a time");
        assert_eq!(f.handle(I::AskAnswered { id }, 10_000), vec![E::Finished { id, outcome: Outcome::Answered }]);
    }

    #[test]
    fn a_key_problem_closes_the_answer_and_shows_the_key_error() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        let fx = f.handle(I::AskFailed { id, error: AskError::Key }, 6_500);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }) && has(&fx, &E::CloseAnswer));
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::ClaudeKey), false));
        assert!(!f.answer_open());
    }

    #[test]
    fn esc_before_the_answer_shows_cancels_with_restore_like_a_dictation() {
        let mut f = Flow::new(2, false);
        let id = asking(&mut f, 0, "что такое HTTP 418");
        let fx = f.handle(I::Esc, 5_600);
        assert!(has(&fx, &E::AbortAsk { id }) && has(&fx, &E::CloseAnswer));
        assert_eq!(phase(&fx), Some(UiPhase::Cancel));
        let fx = f.handle(I::Ui(UiAction::Undo), 6_000);
        assert!(has(&fx, &E::Ask { id, question: "что такое HTTP 418".into(), follow_up: false }), "restore asks again");
        f.handle(I::Esc, 6_200);
        let fx = f.handle(I::Timer(TimerKind::UndoExpire), 9_800);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
    }

    #[test]
    fn pause_closes_an_open_answer_and_drops_a_follow_up() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        let fx = f.handle(I::TogglePause, 6_500);
        assert!(has(&fx, &E::AbortAsk { id }) && has(&fx, &E::CloseAnswer));
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.paused)), Some((UiPhase::Idle, true)));

        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        let fx = f.handle(I::TogglePause, 10_500);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }) && has(&fx, &E::CloseAnswer));
        assert_eq!(f.ui_state().phase, UiPhase::Idle);

        // A follow-up being transcribed: it goes, and the answer closes.
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        let fx = f.handle(I::TogglePause, 12_100);
        assert!(has(&fx, &E::AbortStt { id }) && has(&fx, &E::Finished { id, outcome: Outcome::Discarded }));
        assert!(has(&fx, &E::CloseAnswer));
        assert!(!f.answer_open() && f.ui_state().phase == UiPhase::Idle);
    }

    /// Decision 7: a follow-up that could not be transcribed keeps the answer; the error shows as in dictation, Retry
    /// transcribes it again and asks it in the same conversation, Esc goes back to the answer.
    #[test]
    fn a_follow_up_that_could_not_be_transcribed_keeps_the_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        let fx = f.handle(I::SttFailed { id, error: SttError::Failed }, 12_300);
        assert!(!fx.contains(&E::CloseAnswer) && has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Stt), true));
        assert!(f.answer_open() && f.esc_active());
        assert!(has(&f.handle(I::Ui(UiAction::Retry), 13_000), &E::TranscribeSaved { id }));
        let fx = f.handle(I::SttFinal { id, text: "а подробнее".into() }, 14_000);
        assert!(has(&fx, &E::Ask { id, question: "а подробнее".into(), follow_up: true }), "the same conversation");

        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 12_300);
        assert_eq!(phase(&f.handle(I::Esc, 13_000)), Some(UiPhase::Answer), "back to the answer, still open");
        assert!(f.answer_open());
        assert!(has(&f.handle(I::ChordDown(Mode::Dictate), 14_000), &E::OpenMic { id: DictationId(id.0 + 1) }), "another follow-up");
    }

    #[test]
    fn a_silent_microphone_in_a_follow_up_closes_the_answer_for_its_card() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        let fx = f.handle(I::MicDead, 14_000);
        let close = fx.iter().position(|e| *e == E::CloseAnswer).expect("the answer closes");
        let card = fx.iter().position(|e| *e == E::Tray(Some(TrayKind::MicSilent))).expect("the microphone card");
        assert!(close < card, "one card at a time");
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
    }

    #[test]
    fn paste_last_while_the_answer_is_open_keeps_it() {
        let mut f = Flow::new(2, true);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 10_000);
        let id = f.current_id();
        let fx = f.handle(I::PasteLast, 10_050);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Discarded }) && has(&fx, &E::PasteLast));
        assert!(!fx.contains(&E::CloseAnswer));
        assert_eq!(f.ui_state().phase, UiPhase::Answer);
    }

    #[test]
    fn no_last_minute_card_over_an_open_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        follow_up(&mut f, 10_000);
        assert!(f.handle(I::Timer(TimerKind::WarnLastMinute), 10_000 + WARN_MS).is_empty());
    }

    #[test]
    fn esc_on_an_answer_still_printing_aborts_it() {
        let mut f = Flow::new(2, false);
        let id = answer_open(&mut f, 0);
        let idle = UiState { phase: UiPhase::Idle, error: None, retry: false, paused: false, mode: Mode::Dictate };
        assert_eq!(
            f.handle(I::Esc, 6_500),
            vec![E::AbortAsk { id }, E::Finished { id, outcome: Outcome::Cancelled }, E::CloseAnswer, E::Ui(idle)]
        );
        assert!(f.handle(I::AskAnswered { id }, 7_000).is_empty(), "the late end of the aborted request is ignored");
    }

    #[test]
    fn dismiss_on_a_follow_ups_error_goes_back_to_the_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        f.handle(I::SttFailed { id, error: SttError::Failed }, 12_300);
        let answer = UiState { phase: UiPhase::Answer, error: None, retry: false, paused: false, mode: Mode::Ask };
        assert_eq!(f.handle(I::Ui(UiAction::Dismiss), 13_000), vec![E::Ui(answer)]);
        assert!(f.answer_open());
        let fx = f.handle(I::Ui(UiAction::AnswerInsert), 13_500);
        assert_eq!(fx.first(), Some(&E::AskPaste { id: first, how: PasteHow::AtCaret }), "the answer is still there");
    }

    /// The answer a follow-up waits for fails: the follow-up goes in place of the failed question, and until its own
    /// answer shows, the answer on screen is the failed one.
    #[test]
    fn a_failure_of_the_answer_a_follow_up_waits_for_sends_the_follow_up() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        let fx = f.handle(I::AskFailed { id: first, error: AskError::Failed }, 10_000);
        assert_eq!(
            fx,
            vec![E::Finished { id: first, outcome: Outcome::Failed }, E::Ask { id, question: "а подробнее".into(), follow_up: true }]
        );
        assert_eq!(f.ui_state().phase, UiPhase::Think);
        // Cancelled before its answer showed: the failed answer is back, with Retry.
        let fx = f.handle(I::Esc, 10_500);
        assert!(has(&fx, &E::AbortAsk { id }) && has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert_eq!(phase(&fx), Some(UiPhase::Answer));
        assert!(f.handle(I::Ui(UiAction::AnswerInsert), 11_000).is_empty(), "nothing to paste: it failed");
        assert_eq!(f.handle(I::Ui(UiAction::AnswerRetry), 11_500), vec![E::AskAgain { id: first }]);

        // Its answer shows: the buttons act on it.
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        f.handle(I::AskFailed { id: first, error: AskError::Failed }, 10_000);
        assert_eq!(phase(&f.handle(I::AskOpened { id }, 10_500)), Some(UiPhase::Answer));
        f.handle(I::AskAnswered { id }, 11_000);
        assert!(f.handle(I::Ui(UiAction::AnswerRetry), 11_200).is_empty(), "the answer on screen did not fail");
        let fx = f.handle(I::Ui(UiAction::AnswerInsert), 11_500);
        assert_eq!(fx.first(), Some(&E::AskPaste { id, how: PasteHow::AtCaret }));
    }

    /// The answer a follow-up follows ends in a write: it is shown instead (decision 12) and counted as answered; a
    /// waiting follow-up goes in the same step, one still recording goes on.
    #[test]
    fn a_write_of_the_answer_a_follow_up_follows_counts_as_answered() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::ChordDown(Mode::Dictate), 9_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 9_300);
        let fx = f.handle(I::AskWrite { id: first, how: PasteHow::AtCaret }, 10_000);
        assert_eq!(
            fx,
            vec![E::Finished { id: first, outcome: Outcome::Answered }, E::Ask { id, question: "а подробнее".into(), follow_up: true }]
        );
        assert!(f.answer_open() && f.ui_state().phase == UiPhase::Think);

        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        follow_up(&mut f, 7_000);
        let fx = f.handle(I::AskWrite { id: first, how: PasteHow::Below }, 8_000);
        assert_eq!(fx, vec![E::Finished { id: first, outcome: Outcome::Answered }]);
        assert!(f.recording_now(), "the follow-up goes on recording");
    }

    #[test]
    fn pause_while_a_follow_ups_request_is_in_flight_aborts_it_and_closes_the_answer() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 12_300);
        let fx = f.handle(I::TogglePause, 12_500);
        assert_eq!(&fx[..3], &[E::AbortAsk { id }, E::Finished { id, outcome: Outcome::Cancelled }, E::CloseAnswer]);
        assert_eq!(fx.iter().filter(|e| matches!(e, E::Finished { .. })).count(), 1, "one end for its attempt");
        assert!(!fx.iter().any(|e| matches!(e, E::AbortStt { .. })), "its transcription had ended");
        assert_eq!(last_ui(&fx).map(|s| (s.phase, s.paused)), Some((UiPhase::Idle, true)));
        assert!(!f.answer_open());
    }

    /// The hold hint follows the paste that ends a held recording: a write right after a held question, as in
    /// dictation; not a button pressed later.
    #[test]
    fn the_hold_hint_follows_a_write_but_not_a_button_paste() {
        let held_question = |f: &mut Flow| {
            f.handle(I::ChordDown(Mode::Ask), 0);
            let id = f.current_id();
            f.handle(I::Timer(TimerKind::StartingDone), STARTING_MS);
            f.handle(I::Timer(TimerKind::HoldCheck), HOLD_MS);
            f.handle(I::ChordUp, 3_000);
            f.handle(I::SttFinal { id, text: "что такое HTTP 418".into() }, 3_300);
            id
        };
        let mut f = Flow::new(2, false);
        let id = held_question(&mut f);
        f.handle(I::AskAnswered { id }, 4_000);
        f.handle(I::Ui(UiAction::AnswerInsert), 9_000);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 9_300);
        assert!(!has(&fx, &E::Tray(Some(TrayKind::HoldHint))) && !has(&fx, &E::HoldHintShown));

        let mut f = Flow::new(2, false);
        let id = held_question(&mut f);
        f.handle(I::AskWrite { id, how: PasteHow::AtCaret }, 4_000);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 4_300);
        assert!(has(&fx, &E::Tray(Some(TrayKind::HoldHint))) && has(&fx, &E::HoldHintShown));
    }

    // ---- The answer's buttons name the request whose answer shows, never a dropped follow-up ----

    #[test]
    fn retry_after_a_dropped_follow_up_asks_the_failed_request_again() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        let id = follow_up(&mut f, 7_000);
        f.handle(I::Esc, 7_500);
        f.handle(I::AskFailed { id: first, error: AskError::Failed }, 8_000);
        assert_eq!(f.handle(I::Ui(UiAction::AnswerRetry), 9_000), vec![E::AskAgain { id: first }]);
        assert!(f.handle(I::AskAnswered { id }, 9_500).is_empty(), "the dropped follow-up has no request");
        assert_eq!(f.handle(I::AskAnswered { id: first }, 10_000), vec![E::Finished { id: first, outcome: Outcome::Answered }]);
    }

    #[test]
    fn a_paste_after_a_dropped_follow_up_names_the_answers_request() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        follow_up(&mut f, 7_000);
        f.handle(I::Esc, 7_500);
        f.handle(I::AskAnswered { id: first }, 8_000);
        let fx = f.handle(I::Ui(UiAction::AnswerInsert), 9_000);
        assert_eq!(&fx[..2], &[E::AskPaste { id: first, how: PasteHow::AtCaret }, E::CloseAnswer]);
        let fx = f.handle(I::Inserted { id: first, outcome: InsertOutcome::Pasted }, 9_300);
        assert!(!fx.iter().any(|e| matches!(e, E::Finished { .. })), "the answer was counted when it was complete");
        assert_eq!(phase(&fx), Some(UiPhase::Done));

        // The model writes instead: the write of the answer's request is pasted and counted under its id.
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        follow_up(&mut f, 7_000);
        f.handle(I::Esc, 7_500);
        let fx = f.handle(I::AskWrite { id: first, how: PasteHow::Replace }, 8_000);
        assert_eq!(&fx[..2], &[E::AskPaste { id: first, how: PasteHow::Replace }, E::CloseAnswer]);
        let fx = f.handle(I::Inserted { id: first, outcome: InsertOutcome::NoField }, 8_300);
        assert!(has(&fx, &E::Finished { id: first, outcome: Outcome::NoField }) && has(&fx, &E::Tray(Some(TrayKind::NoField))));
    }

    /// Esc, a system chord, Ctrl+Win+V, a lock or an empty transcript: none of them changes what the answer's buttons name.
    #[test]
    fn every_way_of_dropping_a_follow_up_keeps_the_answers_request() {
        for drop in ["Esc", "a system chord", "Ctrl+Win+V", "a lock", "an empty transcript"] {
            let mut f = Flow::new(2, true);
            let first = answer_open(&mut f, 0);
            f.handle(I::ChordDown(Mode::Dictate), 7_000);
            let id = f.current_id();
            match drop {
                "Esc" => f.handle(I::Esc, 7_050),
                "a system chord" => f.handle(I::ChordAborted, 7_050),
                "Ctrl+Win+V" => f.handle(I::PasteLast, 7_050),
                "a lock" => f.handle(I::SessionLocked, 7_050),
                _ => {
                    f.handle(I::ChordUp, 7_100);
                    f.handle(I::ChordDown(Mode::Dictate), 9_000);
                    f.handle(I::SttFinal { id, text: " ".into() }, 9_300)
                }
            };
            assert_eq!(f.ui_state().phase, UiPhase::Answer, "{drop}");
            f.handle(I::AskFailed { id: first, error: AskError::Failed }, 10_000);
            assert_eq!(f.handle(I::Ui(UiAction::AnswerRetry), 11_000), vec![E::AskAgain { id: first }], "{drop}");
            f.handle(I::AskAnswered { id: first }, 12_000);
            let fx = f.handle(I::Ui(UiAction::AnswerBelow), 13_000);
            assert_eq!(fx.first(), Some(&E::AskPaste { id: first, how: PasteHow::Below }), "{drop}");
        }
    }

    /// A follow-up asked and cancelled before its answer showed: the answer on screen is still the one before, and the
    /// buttons act on it — its paste, or its Retry when it had failed.
    #[test]
    fn a_follow_up_cancelled_before_its_answer_showed_leaves_the_answer_before() {
        let mut f = Flow::new(2, false);
        let first = answer_open(&mut f, 0);
        f.handle(I::AskAnswered { id: first }, 7_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        f.handle(I::SttFinal { id, text: "а подробнее".into() }, 12_300);
        assert!(has(&f.handle(I::Esc, 12_500), &E::AbortAsk { id }));
        let fx = f.handle(I::Ui(UiAction::AnswerInsert), 13_000);
        assert_eq!(fx.first(), Some(&E::AskPaste { id: first, how: PasteHow::AtCaret }));

        let mut f = Flow::new(2, false);
        let first = asking(&mut f, 0, "курс евро");
        f.handle(I::AskFailed { id: first, error: AskError::Failed }, 6_000);
        let id = follow_up(&mut f, 10_000);
        f.handle(I::ChordDown(Mode::Dictate), 12_000);
        f.handle(I::SttFinal { id, text: "а в долларах".into() }, 12_300);
        f.handle(I::Esc, 12_500);
        assert!(f.handle(I::Ui(UiAction::AnswerInsert), 13_000).is_empty(), "the answer on screen failed: nothing to paste");
        assert_eq!(f.handle(I::Ui(UiAction::AnswerRetry), 13_500), vec![E::AskAgain { id: first }]);
    }

    /// Random input sequences, dictations and translations: no panic; the microphone is open exactly while recording;
    /// every transcription, refine and paste is closed by a result or an abort; a dictation has one `Finished`
    /// (plus one per "retry"), and it arrives before the next dictation starts; a timer is armed again
    /// only after it fired or was cancelled; Esc is swallowed only when there is something to cancel or close; the mode
    /// changes only while recording (every Shift toggle then switches it and shows it on the Drop), at a new dictation
    /// or on the way into idle; refine runs in the dictation's mode, and the idle Drop shows no mode.
    /// Timers mostly arrive as the state machine armed them (as the runtime does), sometimes at random.
    #[test]
    fn random_inputs_never_panic_and_keep_invariants() {
        use std::collections::HashMap;
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let timers = [
            TimerKind::StartingDone,
            TimerKind::HoldCheck,
            TimerKind::DoneHide,
            TimerKind::EmptyHide,
            TimerKind::UndoExpire,
            TimerKind::WarnLastMinute,
            TimerKind::AutoFinish,
            TimerKind::PauseEnd,
        ];
        let slot = |kind: TimerKind| timers.iter().position(|&k| k == kind).unwrap();
        let actions = [
            UiAction::Cancel,
            UiAction::Finish,
            UiAction::Undo,
            UiAction::Retry,
            UiAction::Dismiss,
            UiAction::TrayAction,
            UiAction::TrayClose,
        ];
        let mut steps = 0u64;
        let mut seen_finished = 0u64;
        let mut seen_translate_errors = 0u64;
        let mut mode_changes = 0u64;
        let mut toggles = 0u64;
        for round in 0..300u64 {
            let mut f = Flow::new((round % 3) as u8, round % 2 == 0);
            let mut now: Ms = 0;
            let mut mic: Option<DictationId> = None;
            let mut stt: Option<DictationId> = None;
            let mut refine: Option<DictationId> = None;
            let mut insert: Option<DictationId> = None;
            let mut armed = [false; 8];
            let mut finished: HashMap<u64, u32> = HashMap::new();
            let mut retries: HashMap<u64, u32> = HashMap::new();
            for _ in 0..400 {
                steps += 1;
                now += next(1_500);
                let id = DictationId(f.current_id().0.saturating_sub(next(2)));
                let text = if next(4) == 0 { " ".to_string() } else { "текст".to_string() };
                let input = match next(20) {
                    0 | 1 => I::ChordDown(if next(3) == 0 { Mode::Translate } else { Mode::Dictate }),
                    2 | 3 => I::ChordUp,
                    4 => I::ChordAborted,
                    5 => I::Esc,
                    6 => I::PasteLast,
                    7 => I::TogglePause,
                    8 => I::Ui(actions[next(7) as usize]),
                    9 => I::MicFailed { heard: next(2) == 0 },
                    10 => I::MicDead,
                    11 => I::SessionLocked,
                    12 => I::SttFinal { id, text },
                    13 => I::SttFailed { id, error: if next(2) == 0 { SttError::Failed } else { SttError::Model } },
                    14 => I::Refined {
                        id,
                        text,
                        outcome: [RefineOutcome::Refined, RefineOutcome::RawFallback, RefineOutcome::RawKeyProblem]
                            [next(3) as usize],
                    },
                    15 => I::Inserted { id, outcome: if next(2) == 0 { InsertOutcome::Pasted } else { InsertOutcome::NoField } },
                    16 if next(3) == 0 => I::ModeChange([Mode::Ask, Mode::Translate, Mode::Dictate][next(3) as usize]),
                    16 => I::ModeToggle,
                    _ => {
                        let on: Vec<usize> = (0..timers.len()).filter(|&i| armed[i]).collect();
                        let k = if !on.is_empty() && next(5) != 0 {
                            on[next(on.len() as u64) as usize]
                        } else {
                            next(timers.len() as u64) as usize
                        };
                        I::Timer(timers[k])
                    }
                };
                // What the state machine will accept: the result of the work in progress and a fired armed timer.
                match &input {
                    I::SttFinal { id, .. } | I::SttFailed { id, .. } if stt == Some(*id) => stt = None,
                    I::Refined { id, .. } if refine == Some(*id) => refine = None,
                    I::Inserted { id, .. } if insert == Some(*id) => insert = None,
                    I::Timer(kind) => armed[slot(*kind)] = false,
                    _ => {}
                }
                let (phase_before, mode_before) = (f.ui_state().phase, f.mode());
                let fx = f.handle(input.clone(), now);
                let recording_before = matches!(phase_before, UiPhase::Start | UiPhase::Listen);
                if f.mode() != mode_before {
                    let by_keys = matches!(input, I::ModeChange(_) | I::ModeToggle) && recording_before;
                    let new_dictation = fx.iter().any(|e| matches!(e, E::OpenMic { .. }));
                    let into_idle = f.ui_state().phase == UiPhase::Idle;
                    assert!(by_keys || new_dictation || into_idle, "mode changed outside recording ({input:?})");
                    if by_keys {
                        assert_eq!(last_ui(&fx).map(|s| s.mode), Some(f.mode()), "the Drop shows the new mode");
                        mode_changes += 1;
                    }
                }
                if input == I::ModeToggle {
                    if recording_before {
                        assert_ne!(f.mode(), mode_before, "a Shift toggle while recording always switches the mode");
                        toggles += 1;
                    } else {
                        assert!(fx.is_empty(), "a Shift toggle outside recording does nothing ({phase_before:?})");
                    }
                }
                for e in fx {
                    match e {
                        E::OpenMic { id } => {
                            assert_eq!(mic, None, "microphone opened twice ({input:?})");
                            if id.0 > 1 {
                                assert!(finished.contains_key(&(id.0 - 1)), "previous dictation without Finished ({input:?})");
                            }
                            mic = Some(id);
                        }
                        E::StopMic { id, .. } => {
                            assert_eq!(mic, Some(id), "the wrong microphone was closed ({input:?})");
                            mic = None;
                        }
                        E::StartStt { id } | E::TranscribeSaved { id } => {
                            assert_eq!(stt, None, "two transcriptions at once ({input:?})");
                            assert_eq!(id, f.current_id());
                            if matches!(e, E::TranscribeSaved { .. }) {
                                *retries.entry(id.0).or_default() += 1;
                            }
                            stt = Some(id);
                        }
                        E::FinishStt { id } => assert_eq!(stt, Some(id), "FinishStt without a transcription ({input:?})"),
                        E::AbortStt { id } => {
                            assert_eq!(stt, Some(id), "AbortStt without a transcription ({input:?})");
                            stt = None;
                        }
                        E::Refine { id, mode, .. } | E::RefineAgain { id, mode, .. } => {
                            assert_eq!(refine, None, "a second refine ({input:?})");
                            assert_eq!(mode, f.mode(), "refine in another mode than the dictation's ({input:?})");
                            if matches!(e, E::RefineAgain { .. }) {
                                assert_eq!(id, f.current_id());
                                *retries.entry(id.0).or_default() += 1;
                            }
                            refine = Some(id);
                        }
                        E::AbortRefine { id } => {
                            assert_eq!(refine, Some(id), "AbortRefine without a refine ({input:?})");
                            refine = None;
                        }
                        E::Insert { id, .. } => {
                            assert_eq!(insert, None, "a second paste ({input:?})");
                            insert = Some(id);
                        }
                        E::SetTimer { kind, .. } => {
                            assert!(!armed[slot(kind)], "{kind:?} armed again before the previous one fired or was cancelled ({input:?})");
                            armed[slot(kind)] = true;
                        }
                        E::CancelTimer(kind) => armed[slot(kind)] = false,
                        E::Ui(UiState { error: Some(ErrorCause::Translate), .. }) => seen_translate_errors += 1,
                        E::Finished { id, .. } => {
                            seen_finished += 1;
                            let n = finished.entry(id.0).or_default();
                            *n += 1;
                            let allowed = 1 + retries.get(&id.0).copied().unwrap_or(0);
                            assert!(*n <= allowed, "extra Finished for {id:?} ({input:?})");
                        }
                        _ => {}
                    }
                }
                let ui = f.ui_state();
                assert_eq!(mic.is_some(), matches!(ui.phase, UiPhase::Start | UiPhase::Listen), "{input:?}");
                if stt.is_some() {
                    assert!(matches!(ui.phase, UiPhase::Start | UiPhase::Listen | UiPhase::Think | UiPhase::Cancel), "{input:?}");
                }
                if refine.is_some() || insert.is_some() {
                    assert_eq!(ui.phase, UiPhase::Think, "{input:?}");
                }
                if f.esc_active() {
                    assert!(matches!(
                        ui.phase,
                        UiPhase::Start | UiPhase::Listen | UiPhase::Think | UiPhase::Error | UiPhase::Answer
                    ));
                }
                if ui.phase == UiPhase::Idle {
                    assert_eq!(ui.mode, Mode::Dictate, "the idle Drop shows a mode ({input:?})");
                }
            }
        }
        assert!(
            steps == 120_000 && seen_finished > 1_000 && seen_translate_errors > 10 && mode_changes > 100 && toggles > 100,
            "sequences reach outcomes: {seen_finished}, failed translations: {seen_translate_errors}, \
             mode changes: {mode_changes}, Shift toggles while recording: {toggles}"
        );
    }

    /// "Ask" under random input, with the invariants of an open answer: one request at a time; a conversation starts with
    /// a first question and ends with one `CloseAnswer`; a paste of the answer comes in the same step as its close; an open
    /// answer always has its conversation and catches Esc; idle has neither a conversation nor a request; `Finished` once
    /// per attempt (plus one per retry). And those of every recording, follow-ups included: the microphone is open
    /// exactly while starting or recording; one transcription at a time, closed by its result or an abort; a timer is
    /// armed again only after it fired or was cancelled. Timers mostly arrive as the state machine armed them.
    #[test]
    fn random_ask_inputs_keep_the_answer_invariants() {
        use std::collections::HashMap;
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % n
        };
        let hows = [PasteHow::AtCaret, PasteHow::Replace, PasteHow::Below];
        let actions = [
            UiAction::AnswerInsert,
            UiAction::AnswerReplace,
            UiAction::AnswerBelow,
            UiAction::AnswerCopy,
            UiAction::AnswerRetry,
            UiAction::AnswerClose,
            UiAction::Cancel,
            UiAction::Finish,
            UiAction::Undo,
            UiAction::Retry,
            UiAction::Dismiss,
        ];
        let timers = [
            TimerKind::StartingDone,
            TimerKind::HoldCheck,
            TimerKind::DoneHide,
            TimerKind::EmptyHide,
            TimerKind::UndoExpire,
            TimerKind::WarnLastMinute,
            TimerKind::AutoFinish,
            TimerKind::PauseEnd,
        ];
        let slot = |kind: TimerKind| timers.iter().position(|&k| k == kind).unwrap();
        let (mut opened, mut follow_ups, mut pastes, mut closes) = (0u64, 0u64, 0u64, 0u64);
        let mut follow_up_recordings = 0u64;
        for round in 0..300u64 {
            let mut f = Flow::new(2, round % 2 == 0);
            let mut now: Ms = 0;
            let mut conversation = false;
            let mut ask: Option<DictationId> = None;
            // The requests of the living conversation: Retry and pastes name one of them, never a dropped follow-up.
            let mut questions: Vec<DictationId> = Vec::new();
            let mut mic: Option<DictationId> = None;
            let mut stt: Option<DictationId> = None;
            let mut armed = [false; 8];
            let mut finished: HashMap<u64, u32> = HashMap::new();
            let mut retries: HashMap<u64, u32> = HashMap::new();
            for _ in 0..400 {
                now += next(1_500);
                let id = DictationId(f.current_id().0.saturating_sub(next(2)));
                let text = if next(5) == 0 { " ".to_string() } else { "вопрос".to_string() };
                let input = match next(18) {
                    0 | 1 => I::ChordDown([Mode::Ask, Mode::Ask, Mode::Dictate, Mode::Translate][next(4) as usize]),
                    2 => I::ChordUp,
                    3 => I::Esc,
                    4 => I::Ui(actions[next(actions.len() as u64) as usize]),
                    5 => I::SttFinal { id, text },
                    6 => I::SttFailed { id, error: SttError::Failed },
                    7 => I::AskOpened { id },
                    8 => I::AskAnswered { id },
                    9 => I::AskWrite { id, how: hows[next(3) as usize] },
                    10 => I::AskFailed { id, error: if next(3) == 0 { AskError::Key } else { AskError::Failed } },
                    11 => I::Inserted { id, outcome: if next(2) == 0 { InsertOutcome::Pasted } else { InsertOutcome::NoField } },
                    12 => [I::ModeToggle, I::PasteLast, I::TogglePause, I::MicDead][next(4) as usize].clone(),
                    13 => [I::ChordAborted, I::SessionLocked, I::MicFailed { heard: true }, I::MicFailed { heard: false }]
                        [next(4) as usize]
                        .clone(),
                    _ => {
                        let on: Vec<usize> = (0..timers.len()).filter(|&i| armed[i]).collect();
                        let k = if !on.is_empty() && next(5) != 0 {
                            on[next(on.len() as u64) as usize]
                        } else {
                            next(timers.len() as u64) as usize
                        };
                        I::Timer(timers[k])
                    }
                };
                // What the state machine will accept: the transcription in progress and a fired armed timer.
                match &input {
                    I::SttFinal { id, .. } | I::SttFailed { id, .. } if stt == Some(*id) => stt = None,
                    I::Timer(kind) => armed[slot(*kind)] = false,
                    _ => {}
                }
                let open_before = f.answer_open();
                let fx = f.handle(input.clone(), now);
                let mut pasted = false;
                for e in &fx {
                    match e {
                        E::OpenMic { id } => {
                            assert_eq!(mic, None, "microphone opened twice ({input:?})");
                            follow_up_recordings += u64::from(open_before);
                            mic = Some(*id);
                        }
                        E::StopMic { id, .. } => {
                            assert_eq!(mic, Some(*id), "the wrong microphone was closed ({input:?})");
                            mic = None;
                        }
                        E::StartStt { id } | E::TranscribeSaved { id } => {
                            assert_eq!(stt, None, "two transcriptions at once ({input:?})");
                            assert_eq!(*id, f.current_id());
                            if matches!(e, E::TranscribeSaved { .. }) {
                                *retries.entry(id.0).or_default() += 1;
                            }
                            stt = Some(*id);
                        }
                        E::FinishStt { id } => assert_eq!(stt, Some(*id), "FinishStt without a transcription ({input:?})"),
                        E::AbortStt { id } => {
                            assert_eq!(stt, Some(*id), "AbortStt without a transcription ({input:?})");
                            stt = None;
                        }
                        E::SetTimer { kind, .. } => {
                            assert!(!armed[slot(*kind)], "{kind:?} armed again before it fired or was cancelled ({input:?})");
                            armed[slot(*kind)] = true;
                        }
                        E::CancelTimer(kind) => armed[slot(*kind)] = false,
                        E::Ask { id, follow_up, .. } => {
                            assert_eq!(ask, None, "two requests at once ({input:?})");
                            assert_eq!(*follow_up, open_before, "a follow-up exactly while the answer is open ({input:?})");
                            assert!(*follow_up || !conversation, "a first question while a conversation lives ({input:?})");
                            assert_eq!(f.mode(), Mode::Ask);
                            follow_ups += u64::from(*follow_up);
                            conversation = true;
                            questions.push(*id);
                            ask = Some(*id);
                        }
                        E::AskAgain { id } => {
                            assert_eq!(ask, None, "a retry while a request is in flight ({input:?})");
                            assert!(conversation);
                            assert!(questions.contains(id), "a retry of {id:?}, never asked ({input:?})");
                            *retries.entry(id.0).or_default() += 1;
                            ask = Some(*id);
                        }
                        E::AbortAsk { id } => {
                            assert_eq!(ask, Some(*id), "abort of nothing ({input:?})");
                            ask = None;
                        }
                        E::AskPaste { id, .. } => {
                            assert!(conversation, "a paste needs the conversation's text ({input:?})");
                            assert!(questions.contains(id), "a paste of {id:?}, never asked ({input:?})");
                            // A write ends its request here.
                            if ask == Some(*id) {
                                ask = None;
                            }
                            pasted = true;
                            pastes += 1;
                        }
                        E::CloseAnswer => {
                            assert!(conversation, "a close without a conversation ({input:?})");
                            conversation = false;
                            questions.clear();
                            closes += 1;
                        }
                        E::Finished { id, .. } => {
                            // A request ends with its answer, its write or its failure, when the flow accepted it — before a
                            // follow-up that waited for it goes out in the same step.
                            if ask == Some(*id) && matches!(input, I::AskAnswered { .. } | I::AskWrite { .. } | I::AskFailed { .. }) {
                                ask = None;
                            }
                            let n = finished.entry(id.0).or_default();
                            *n += 1;
                            assert!(*n <= 1 + retries.get(&id.0).copied().unwrap_or(0), "extra Finished for {id:?} ({input:?})");
                        }
                        _ => {}
                    }
                }
                if pasted {
                    assert!(!conversation, "the paste closes the answer in the same step ({input:?})");
                }
                let phase = f.ui_state().phase;
                assert_eq!(mic.is_some(), matches!(phase, UiPhase::Start | UiPhase::Listen), "microphone ({input:?})");
                if stt.is_some() {
                    assert!(
                        matches!(phase, UiPhase::Start | UiPhase::Listen | UiPhase::Think | UiPhase::Cancel),
                        "a transcription in {phase:?} ({input:?})"
                    );
                }
                if f.answer_open() {
                    opened += 1;
                    assert!(conversation, "an open answer without its conversation ({input:?})");
                    assert!(f.esc_active());
                }
                if f.ui_state().phase == UiPhase::Idle {
                    assert!(!conversation && ask.is_none(), "idle with a conversation or a request ({input:?})");
                }
            }
        }
        assert!(
            opened > 100 && follow_up_recordings > 50 && follow_ups > 5 && pastes > 5 && closes > 20,
            "sequences reach the answer: open steps {opened}, follow-up recordings {follow_up_recordings}, \
             follow-ups asked {follow_ups}, pastes {pastes}, closes {closes}"
        );
    }
}
