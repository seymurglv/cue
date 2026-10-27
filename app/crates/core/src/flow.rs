//! The dictation flow: one state machine. Pure logic with no OS, network or clock.
use crate::ui::{ErrorCause, TrayKind, UiAction, UiPhase, UiState};

pub type Ms = u64;

pub const STARTING_MS: Ms = 380;
pub const HOLD_MS: Ms = 600;
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
    Network,
    KeyProblem,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Inserted,
    NoField,
    Cancelled,
    Empty,
    Failed,
    Discarded,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    ChordDown,
    ChordUp,
    ChordAborted,
    Esc,
    PasteLast,
    TogglePause,
    Ui(UiAction),
    MicFailed { heard: bool },
    MicDead,
    SessionLocked,
    SttFinal { id: DictationId, text: String },
    SttFailed { id: DictationId, error: SttError },
    Refined { id: DictationId, text: String, outcome: RefineOutcome },
    Inserted { id: DictationId, outcome: InsertOutcome },
    Timer(TimerKind),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    OpenMic { id: DictationId },
    StopMic { id: DictationId, keep: bool },
    StartStt { id: DictationId },
    FinishStt { id: DictationId },
    AbortStt { id: DictationId },
    TranscribeSaved { id: DictationId },
    Refine { id: DictationId, raw: String },
    AbortRefine { id: DictationId },
    Insert { id: DictationId, text: String },
    PasteLast,
    Ui(UiState),
    Shake,
    Tray(Option<TrayKind>),
    SetTimer { kind: TimerKind, after: Ms },
    CancelTimer(TimerKind),
    HoldHintShown,
    Finished { id: DictationId, outcome: Outcome },
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
    paused: bool,
    hold_hints_left: u8,
    has_last: bool,
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
            paused: false,
            hold_hints_left,
            has_last,
        }
    }

    pub fn current_id(&self) -> DictationId {
        self.id
    }

    /// Swallow Esc: while CUE is starting, recording, thinking (including pasting, when the Drop shows
    /// "thinking"; Esc does nothing there) or showing an error.
    pub fn esc_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Starting | Phase::Recording | Phase::Thinking | Phase::Inserting | Phase::Error { .. }
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
        };
        UiState { phase, error, retry, paused: self.paused }
    }

    pub fn handle(&mut self, input: Input, now: Ms) -> Vec<Effect> {
        let mut fx = Vec::new();
        match input {
            Input::ChordDown => self.on_chord_down(now, &mut fx),
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
                if self.phase == Phase::Starting {
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
                    self.enter_idle(&mut fx);
                }
            }
            // The tray card carries out its own buttons; they do not change the flow.
            Input::Ui(UiAction::TrayAction | UiAction::TrayClose) => {}
            Input::PasteLast => self.on_paste_last(&mut fx),
            Input::TogglePause => self.on_toggle_pause(&mut fx),
            Input::MicFailed { heard } => self.on_mic_lost(heard, &mut fx),
            Input::MicDead => self.on_mic_lost(false, &mut fx),
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
            Input::Inserted { id, outcome } if id == self.id && self.phase == Phase::Inserting => {
                self.has_last = true;
                self.on_inserted(outcome, &mut fx);
            }
            Input::Timer(kind) => self.on_timer(kind, &mut fx),
            // Results of another (earlier) dictation, and repeats of ones already received.
            Input::SttFinal { .. } | Input::SttFailed { .. } | Input::Refined { .. } | Input::Inserted { .. } => {}
        }
        fx
    }

    fn recording_now(&self) -> bool {
        matches!(self.phase, Phase::Starting | Phase::Recording)
    }

    /// The transcription result belongs to the current dictation and is still awaited.
    fn stt_for(&self, id: DictationId) -> bool {
        id == self.id && self.stt_pending
    }

    fn on_chord_down(&mut self, now: Ms, fx: &mut Vec<Effect>) {
        if self.paused {
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
                self.start(fx);
            }
            Phase::Idle | Phase::Done | Phase::Empty | Phase::Error { .. } => self.start(fx),
        }
    }

    fn start(&mut self, fx: &mut Vec<Effect>) {
        self.id = DictationId(self.id.0.wrapping_add(1));
        self.phase = Phase::Starting;
        self.chord_held = true;
        self.hold_mode = false;
        self.raw = None;
        self.stt_error = None;
        self.refine_outcome = None;
        self.stt_pending = true;
        self.refine_pending = false;
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

    /// The start-up or recording is thrown away: the audio is not kept, transcription is aborted, no "restore".
    /// The Drop and `Finished` are up to the caller.
    fn drop_recording(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::StopMic { id: self.id, keep: false });
        self.abort_stt(fx);
        self.cancel_record_timers(fx);
        self.phase = Phase::Idle;
        self.chord_held = false;
        self.hold_mode = false;
    }

    /// Quietly, with no error and no "restore": a system chord or Ctrl+Win+V in the first 380 ms,
    /// a pause or a session lock during recording.
    fn discard(&mut self, fx: &mut Vec<Effect>) {
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
            fx.push(Effect::Finished { id: self.id, outcome: Outcome::Failed });
            self.phase = Phase::Error { cause: cause_of(error), retry: true };
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
        if !self.refine_pending {
            fx.push(Effect::Refine { id: self.id, raw: raw.to_owned() });
            self.refine_pending = true;
        }
        true
    }

    fn on_cancel(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Starting | Phase::Recording => {
                // The audio is kept and the text is still fetched, for "restore".
                fx.push(Effect::StopMic { id: self.id, keep: true });
                if self.stt_pending {
                    fx.push(Effect::FinishStt { id: self.id });
                }
                self.cancel_record_timers(fx);
                self.enter_cancelled(fx);
            }
            Phase::Thinking => {
                if self.refine_pending {
                    fx.push(Effect::AbortRefine { id: self.id });
                    self.refine_pending = false;
                }
                self.enter_cancelled(fx);
            }
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
        if self.has_last {
            fx.push(Effect::PasteLast);
        }
    }

    fn on_toggle_pause(&mut self, fx: &mut Vec<Effect>) {
        if self.paused {
            self.paused = false;
            fx.push(Effect::CancelTimer(TimerKind::PauseEnd));
        } else {
            // Keys do nothing during a pause: an ongoing recording is thrown away, with no "restore".
            if self.recording_now() {
                self.drop_recording(fx);
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Discarded });
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
                self.push_ui(fx);
                fx.push(Effect::Tray(Some(TrayKind::MicSilent)));
            }
            _ => {}
        }
    }

    fn on_inserted(&mut self, outcome: InsertOutcome, fx: &mut Vec<Effect>) {
        match outcome {
            InsertOutcome::Pasted => {
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::Inserted });
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
                fx.push(Effect::Finished { id: self.id, outcome: Outcome::NoField });
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
            (TimerKind::WarnLastMinute, Phase::Recording) => fx.push(Effect::Tray(Some(TrayKind::LastMinute))),
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
        self.phase = Phase::Idle;
        self.push_ui(fx);
    }

    fn enter_empty(&mut self, fx: &mut Vec<Effect>) {
        fx.push(Effect::Finished { id: self.id, outcome: Outcome::Empty });
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
        SttError::Network => ErrorCause::Net,
        SttError::KeyProblem => ErrorCause::SonioxKey,
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
        f.handle(I::ChordDown, t);
        f.handle(I::ChordUp, t + 100);
        f.handle(I::Timer(TimerKind::StartingDone), t + STARTING_MS);
        f.handle(I::Timer(TimerKind::HoldCheck), t + HOLD_MS);
        f.current_id()
    }

    /// The recording was finished by a second press, and the transcribed text has arrived.
    fn thinking_with_raw(f: &mut Flow, t: Ms, raw: &str) -> DictationId {
        let id = recording(f, t);
        f.handle(I::ChordDown, t + 5_000);
        f.handle(I::ChordUp, t + 5_100);
        f.handle(I::SttFinal { id, text: raw.into() }, t + 5_300);
        id
    }

    #[test]
    fn tap_starts_and_second_press_finishes() {
        let mut f = Flow::new(2, false);
        let fx = f.handle(I::ChordDown, 0);
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
        let fx = f.handle(I::ChordDown, 5_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }));
        assert!(has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
    }

    #[test]
    fn happy_path_inserts_then_hides() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown, 5_000);
        let fx = f.handle(I::SttFinal { id, text: "ну привет".into() }, 5_200);
        assert!(has(&fx, &E::Refine { id, raw: "ну привет".into() }));
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
            f.handle(I::ChordDown, t);
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
        f.handle(I::ChordDown, 0);
        f.handle(I::ChordUp, 50);
        assert!(f.handle(I::ChordDown, 200).is_empty());
    }

    #[test]
    fn system_shortcut_in_first_380ms_discards_silently() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
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
        assert!(has(&fx, &E::Refine { id, raw: "сказанное".into() }));
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
        assert!(has(&fx, &E::Refine { id, raw: "позже".into() }));
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
        assert!(has(&fx, &E::Refine { id, raw: "черновик".into() }));
    }

    #[test]
    fn press_while_thinking_shakes() {
        let mut f = Flow::new(2, false);
        thinking_with_raw(&mut f, 0, "x");
        assert_eq!(f.handle(I::ChordDown, 5_600), vec![E::Shake]);
    }

    #[test]
    fn stt_failure_shows_error_and_retry_uses_saved_audio() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown, 5_000);
        let fx = f.handle(I::SttFailed { id, error: SttError::Network }, 6_000);
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::Net), true));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let fx = f.handle(I::Ui(UiAction::Retry), 9_000);
        assert!(has(&fx, &E::TranscribeSaved { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
        let fx = f.handle(I::SttFailed { id, error: SttError::KeyProblem }, 9_500);
        assert_eq!(last_ui(&fx).unwrap().error, Some(ErrorCause::SonioxKey));
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
        f.handle(I::ChordDown, 5_000);
        f.handle(I::SttFailed { id, error: SttError::Network }, 6_000);
        assert_eq!(phase(&f.handle(I::Ui(UiAction::Dismiss), 7_000)), Some(UiPhase::Idle));
        f.handle(I::ChordDown, 10_000);
        let id2 = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 10_380);
        f.handle(I::ChordDown, 15_000);
        f.handle(I::SttFailed { id: id2, error: SttError::Network }, 16_000);
        let fx = f.handle(I::ChordDown, 20_000);
        assert!(has(&fx, &E::OpenMic { id: DictationId(id2.0 + 1) }));
    }

    #[test]
    fn empty_transcript_collapses_quietly() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::ChordDown, 5_000);
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
        f.handle(I::ChordDown, 0);
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
        assert!(f.handle(I::ChordDown, 1_000).is_empty());
        assert!(f.handle(I::PasteLast, 2_000).is_empty());
        let fx = f.handle(I::Timer(TimerKind::PauseEnd), PAUSE_MS);
        assert!(!last_ui(&fx).unwrap().paused);
        assert!(!f.handle(I::ChordDown, PAUSE_MS + 1_000).is_empty());
    }

    #[test]
    fn new_press_during_undo_window_finishes_previous_as_cancelled() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        f.handle(I::Esc, 3_000);
        let fx = f.handle(I::ChordDown, 4_000);
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Cancelled }));
        assert!(has(&fx, &E::OpenMic { id: DictationId(id.0 + 1) }));
    }

    #[test]
    fn stale_results_are_ignored() {
        let mut f = Flow::new(2, false);
        let old = thinking_with_raw(&mut f, 0, "a");
        f.handle(I::Esc, 5_400);
        f.handle(I::ChordDown, 6_000);
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
        assert!(has(&fx, &E::Refine { id, raw: "успел сказать".into() }));
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
        f.handle(I::ChordDown, 0);
        let id = f.current_id();
        let fx = f.handle(I::MicFailed { heard: true }, 100);
        assert!(has(&fx, &E::StopMic { id, keep: false }));
        assert!(has(&fx, &E::AbortStt { id }));
        assert!(has(&fx, &E::Tray(Some(TrayKind::MicSilent))));
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        assert_eq!(phase(&fx), Some(UiPhase::Idle));
        assert!(f.handle(I::Timer(TimerKind::StartingDone), 380).is_empty(), "the start-up is already dropped");
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
        f.handle(I::ChordDown, 0);
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
        f.handle(I::ChordDown, 4_000);
        let id2 = f.current_id();
        f.handle(I::ChordUp, 4_100);
        f.handle(I::Timer(TimerKind::StartingDone), 4_380);
        f.handle(I::Timer(TimerKind::HoldCheck), 4_600);
        assert!(f.handle(I::ChordUp, 5_000).is_empty());
        assert!(has(&f.handle(I::ChordDown, 6_000), &E::FinishStt { id: id2 }));
    }

    #[test]
    fn pause_while_starting_discards() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
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
        assert!(f.handle(I::ChordDown, 5_800).is_empty(), "no shake during a pause either");
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
        assert!(has(&f.handle(I::ChordDown, 4_000), &E::FinishStt { id }));
    }

    #[test]
    fn events_after_aborted_chord_are_safe() {
        let mut f = Flow::new(2, true);
        f.handle(I::ChordDown, 0);
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
        f.handle(I::ChordDown, 3_000);
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
        assert_eq!((WARN_MS, AUTO_FINISH_MS, PAUSE_MS), (480_000, 540_000, 3_600_000));
        let mut f = Flow::new(2, false);
        let fx = f.handle(I::ChordDown, 0);
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
        f.handle(I::ChordDown, 0);
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert!(f.handle(I::ChordUp, 599).is_empty());
        assert!(f.handle(I::Timer(TimerKind::HoldCheck), 600).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Listen);
        // Held for 600 ms: "hold".
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        f.handle(I::Timer(TimerKind::HoldCheck), 600);
        assert!(has(&f.handle(I::ChordUp, 600), &E::FinishStt { id }));
        // Debounce: a press after 299 ms does not count, after 300 ms it does.
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
        f.handle(I::ChordAborted, 50);
        f.handle(I::ChordUp, 60);
        assert!(f.handle(I::ChordDown, 299).is_empty());
        assert!(has(&f.handle(I::ChordDown, 300), &E::OpenMic { id: DictationId(2) }));
    }

    #[test]
    fn release_during_start_is_tap_mode_even_if_timers_reorder() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
        let id = f.current_id();
        f.handle(I::Timer(TimerKind::HoldCheck), 360);
        assert!(f.handle(I::ChordUp, 370).is_empty());
        f.handle(I::Timer(TimerKind::StartingDone), 380);
        assert!(has(&f.handle(I::ChordDown, 5_000), &E::FinishStt { id }));
        f.handle(I::SttFinal { id, text: "т".into() }, 5_100);
        f.handle(I::Refined { id, text: "Т.".into(), outcome: RefineOutcome::Refined }, 5_200);
        let fx = f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_300);
        assert!(!has(&fx, &E::Tray(Some(TrayKind::HoldHint))), "this was \"press and release\"");
    }

    /// "Press again and the recording ends"; the only exception is a press within 0.3 s.
    #[test]
    fn second_press_during_start_finishes() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
        let id = f.current_id();
        f.handle(I::ChordUp, 100);
        let fx = f.handle(I::ChordDown, 350);
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
        f.handle(I::ChordDown, 0);
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
        f.handle(I::ChordDown, 0);
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
        assert_eq!(f.ui_state(), UiState { phase: UiPhase::Idle, error: None, retry: false, paused: false });
        f.handle(I::ChordDown, 1_000);
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
        assert_eq!(f.handle(I::ChordDown, 5_470), vec![E::Shake]);
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        assert!(!f.esc_active());
        assert!(f.handle(I::Esc, 5_600).is_empty());
    }

    #[test]
    fn paste_last_without_result_still_cancels_start() {
        let mut f = Flow::new(2, false);
        f.handle(I::ChordDown, 0);
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
        assert!(f.handle(I::SttFailed { id, error: SttError::KeyProblem }, 2_000).is_empty(), "recording goes on");
        let fx = f.handle(I::ChordDown, 5_000);
        assert!(has(&fx, &E::StopMic { id, keep: true }), "the recording is kept for \"retry\"");
        assert!(!has(&fx, &E::FinishStt { id }), "transcription has already ended");
        assert!(has(&fx, &E::Finished { id, outcome: Outcome::Failed }));
        let ui = last_ui(&fx).unwrap();
        assert_eq!((ui.phase, ui.error, ui.retry), (UiPhase::Error, Some(ErrorCause::SonioxKey), true));
        assert!(has(&f.handle(I::Ui(UiAction::Retry), 6_000), &E::TranscribeSaved { id }));
    }

    #[test]
    fn early_stt_final_is_used_when_recording_ends() {
        let mut f = Flow::new(2, false);
        let id = recording(&mut f, 0);
        assert!(f.handle(I::SttFinal { id, text: "рано".into() }, 2_000).is_empty());
        let fx = f.handle(I::ChordDown, 5_000);
        assert!(has(&fx, &E::Refine { id, raw: "рано".into() }));
        assert!(!has(&fx, &E::FinishStt { id }));
        assert_eq!(phase(&fx), Some(UiPhase::Think));
    }

    #[test]
    fn duplicate_results_are_ignored() {
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "раз");
        assert!(f.handle(I::SttFinal { id, text: "два".into() }, 5_400).is_empty(), "refine is not requested a second time");
        assert!(f.handle(I::SttFailed { id, error: SttError::Network }, 5_450).is_empty());
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
        let fx = f.handle(I::ChordDown, 4_000);
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
        f.handle(I::ChordDown, 5_000);
        for kind in all {
            assert!(f.handle(I::Timer(kind), 5_100).is_empty(), "{kind:?} in \"thinking\"");
        }
        // "Done" → a new recording: a late DoneHide does not touch it.
        let mut f = Flow::new(2, false);
        let id = thinking_with_raw(&mut f, 0, "текст");
        f.handle(I::Refined { id, text: "Текст.".into(), outcome: RefineOutcome::Refined }, 5_400);
        f.handle(I::Inserted { id, outcome: InsertOutcome::Pasted }, 5_500);
        f.handle(I::ChordDown, 6_000);
        assert!(f.handle(I::Timer(DoneHide), 7_000).is_empty());
        assert_eq!(f.ui_state().phase, UiPhase::Start);
    }

    /// Random input sequences: no panic; the microphone is open exactly while recording;
    /// every transcription, refine and paste is closed by a result or an abort; a dictation has one `Finished`
    /// (plus one per "retry"), and it arrives before the next dictation starts; a timer is armed again
    /// only after it fired or was cancelled; Esc is swallowed only when there is something to cancel or close.
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
                    0 | 1 => I::ChordDown,
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
                    13 => I::SttFailed { id, error: if next(2) == 0 { SttError::Network } else { SttError::KeyProblem } },
                    14 => I::Refined {
                        id,
                        text,
                        outcome: [RefineOutcome::Refined, RefineOutcome::RawFallback, RefineOutcome::RawKeyProblem]
                            [next(3) as usize],
                    },
                    15 => I::Inserted { id, outcome: if next(2) == 0 { InsertOutcome::Pasted } else { InsertOutcome::NoField } },
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
                for e in f.handle(input.clone(), now) {
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
                        E::Refine { id, .. } => {
                            assert_eq!(refine, None, "a second refine ({input:?})");
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
                    assert!(matches!(ui.phase, UiPhase::Start | UiPhase::Listen | UiPhase::Think | UiPhase::Error));
                }
            }
        }
        assert!(steps == 120_000 && seen_finished > 1_000, "sequences reach outcomes: {seen_finished}");
    }
}
