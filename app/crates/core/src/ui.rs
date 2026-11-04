//! The Drop's state and the user's actions, shared with the Drop window (TypeScript) via JSON.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UiPhase {
    Idle,
    Start,
    Listen,
    Think,
    Done,
    Empty,
    Cancel,
    Error,
    /// An "Ask" answer is open (in the tray or the card) and nothing records: the Drop rests.
    Answer,
}

/// What the dictation does with the speech, picked by the chord (spec 2 §4): `Ctrl + Win` dictates, `+ Shift` translates,
/// `+ Alt` asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Dictate,
    Translate,
    Ask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCause {
    /// The speech model is not downloaded or did not load.
    Model,
    /// This processor cannot run the speech model.
    Cpu,
    /// Recognition failed.
    Stt,
    /// "Ask"'s Anthropic key is missing or refused: "Ask" always goes to Claude.
    ClaudeKey,
    /// The cleanup's cloud connection refused its key, a limit or the balance (or there is no key): the Drop names the
    /// connection (`cue://cleanup-name`).
    CleanupKey,
    /// The translation failed (network, timeout, the check): nothing was pasted.
    Translate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrayKind {
    NoField,
    MicSilent,
    HoldHint,
    LastMinute,
    /// The pinned microphone is not there, and the recording goes on with the Windows one (spec 3 §3): its names come
    /// to the Drop just before the card.
    MicMissing,
    /// The speech model finished downloading in the background: recognition works now.
    ModelReady,
    /// Another speech model, chosen on the Model page, has loaded and recognizes now: its name comes to the Drop just
    /// before the card.
    ModelSwitched,
    /// A habit was just learned (docs/specs/2026-11-04-habits.md): its rule comes to the Drop just before the card, and
    /// the card's button undoes it.
    NewHabit,
    /// A habit took the place of its opposite (docs/specs/2026-11-04-habits.md, "Changed"): the new rule comes to the
    /// Drop just before the card, and the card's button brings the old habit back.
    HabitChanged,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    pub phase: UiPhase,
    pub error: Option<ErrorCause>,
    pub retry: bool,
    pub paused: bool,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UiAction {
    Cancel,
    Finish,
    Undo,
    Retry,
    Dismiss,
    TrayAction,
    TrayClose,
    /// The suggestion's "+": the word goes into the dictionary.
    SuggestAdd,
    /// "Undo" after "+": the word comes out again.
    SuggestUndo,
    /// The suggestion's 8 s ring ran out: it waits as a dot.
    SuggestDismiss,
    /// The 6 s ring after "+" ran out.
    SuggestDone,
    /// Buttons of an "Ask" answer (spec 2 §7): paste it at the caret, below the selection, over the selection; copy it;
    /// ask again after a failure; close it.
    AnswerInsert,
    AnswerReplace,
    AnswerBelow,
    AnswerCopy,
    AnswerRetry,
    AnswerClose,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_match_typescript() {
        let s = UiState { phase: UiPhase::Error, error: Some(ErrorCause::Model), retry: true, paused: false, mode: Mode::Dictate };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"phase":"error","error":"model","retry":true,"paused":false,"mode":"dictate"}"#
        );
        assert_eq!(serde_json::to_string(&TrayKind::NoField).unwrap(), r#""noField""#);
        assert_eq!(serde_json::from_str::<UiAction>(r#""trayAction""#).unwrap(), UiAction::TrayAction);
    }

    /// The Drop window (TypeScript) relies on every one of these names.
    #[test]
    fn every_variant_has_its_json_name() {
        fn json<T: Serialize>(v: T) -> String {
            serde_json::to_string(&v).unwrap()
        }
        let phases = [
            (UiPhase::Idle, "idle"),
            (UiPhase::Start, "start"),
            (UiPhase::Listen, "listen"),
            (UiPhase::Think, "think"),
            (UiPhase::Done, "done"),
            (UiPhase::Empty, "empty"),
            (UiPhase::Cancel, "cancel"),
            (UiPhase::Error, "error"),
            (UiPhase::Answer, "answer"),
        ];
        for (v, name) in phases {
            assert_eq!(json(v), format!("\"{name}\""));
        }
        let causes = [
            (ErrorCause::Model, "model"),
            (ErrorCause::Cpu, "cpu"),
            (ErrorCause::Stt, "stt"),
            (ErrorCause::ClaudeKey, "claudeKey"),
            (ErrorCause::CleanupKey, "cleanupKey"),
            (ErrorCause::Translate, "translate"),
        ];
        for (v, name) in causes {
            assert_eq!(json(v), format!("\"{name}\""));
        }
        let trays = [
            (TrayKind::NoField, "noField"),
            (TrayKind::MicSilent, "micSilent"),
            (TrayKind::HoldHint, "holdHint"),
            (TrayKind::LastMinute, "lastMinute"),
            (TrayKind::MicMissing, "micMissing"),
            (TrayKind::ModelReady, "modelReady"),
            (TrayKind::ModelSwitched, "modelSwitched"),
            (TrayKind::NewHabit, "newHabit"),
            (TrayKind::HabitChanged, "habitChanged"),
        ];
        for (v, name) in trays {
            assert_eq!(json(v), format!("\"{name}\""));
        }
        let actions = [
            ("cancel", UiAction::Cancel),
            ("finish", UiAction::Finish),
            ("undo", UiAction::Undo),
            ("retry", UiAction::Retry),
            ("dismiss", UiAction::Dismiss),
            ("trayAction", UiAction::TrayAction),
            ("trayClose", UiAction::TrayClose),
            ("answerInsert", UiAction::AnswerInsert),
            ("answerReplace", UiAction::AnswerReplace),
            ("answerBelow", UiAction::AnswerBelow),
            ("answerCopy", UiAction::AnswerCopy),
            ("answerRetry", UiAction::AnswerRetry),
            ("answerClose", UiAction::AnswerClose),
            ("suggestAdd", UiAction::SuggestAdd),
            ("suggestUndo", UiAction::SuggestUndo),
            ("suggestDismiss", UiAction::SuggestDismiss),
            ("suggestDone", UiAction::SuggestDone),
        ];
        for (name, v) in actions {
            assert_eq!(serde_json::from_str::<UiAction>(&format!("\"{name}\"")).unwrap(), v);
        }
        for (v, name) in [(Mode::Dictate, "dictate"), (Mode::Translate, "translate"), (Mode::Ask, "ask")] {
            assert_eq!(json(v), format!("\"{name}\""));
        }
        let s = UiState { phase: UiPhase::Listen, error: None, retry: false, paused: true, mode: Mode::Translate };
        assert_eq!(json(s), r#"{"phase":"listen","error":null,"retry":false,"paused":true,"mode":"translate"}"#);
    }
}
