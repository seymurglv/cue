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
    Net,
    SonioxKey,
    ClaudeKey,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_match_typescript() {
        let s = UiState { phase: UiPhase::Error, error: Some(ErrorCause::SonioxKey), retry: true, paused: false, mode: Mode::Dictate };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"phase":"error","error":"sonioxKey","retry":true,"paused":false,"mode":"dictate"}"#
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
        ];
        for (v, name) in phases {
            assert_eq!(json(v), format!("\"{name}\""));
        }
        let causes = [
            (ErrorCause::Net, "net"),
            (ErrorCause::SonioxKey, "sonioxKey"),
            (ErrorCause::ClaudeKey, "claudeKey"),
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
