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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCause {
    Net,
    SonioxKey,
    ClaudeKey,
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
        let s = UiState { phase: UiPhase::Error, error: Some(ErrorCause::SonioxKey), retry: true, paused: false };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"phase":"error","error":"sonioxKey","retry":true,"paused":false}"#
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
        let causes = [(ErrorCause::Net, "net"), (ErrorCause::SonioxKey, "sonioxKey"), (ErrorCause::ClaudeKey, "claudeKey")];
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
        let s = UiState { phase: UiPhase::Listen, error: None, retry: false, paused: true };
        assert_eq!(json(s), r#"{"phase":"listen","error":null,"retry":false,"paused":true}"#);
    }
}
