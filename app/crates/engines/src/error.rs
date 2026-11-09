//! Engine errors. What matters for the flow is one thing: a refusal the user has to act on (access or a limit), or
//! anything else.

/// How many characters of an HTTP refusal body we keep in the error: the gist is visible, the whole page is not.
const MAX_BODY_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("network: {0}")]
    Network(String),
    #[error("key, balance or limit: {0}")]
    KeyProblem(String),
    #[error("service unavailable: {0}")]
    Service(String),
    #[error("protocol: {0}")]
    Protocol(String),
    #[error("timeout")]
    Timeout,
    #[error("aborted")]
    Aborted,
    /// The speech model is missing, damaged or failed to load or run.
    #[error("speech model: {0}")]
    Model(String),
    /// ONNX Runtime cannot run on this processor.
    #[error("processor not supported: {0}")]
    Unsupported(String),
    /// The cleanup's language model is missing, damaged, or cannot load or run.
    #[error("language model: {0}")]
    Llm(String),
}

impl EngineError {
    /// HTTP status of a service response → error kind. 401 / 402 / 403 / 429 mean key, money or limit.
    pub fn from_status(code: u16, msg: impl Into<String>) -> Self {
        let msg = format!("{code}: {}", msg.into());
        match code {
            401 | 402 | 403 | 429 => EngineError::KeyProblem(msg),
            408 | 500..=599 => EngineError::Service(msg),
            _ => EngineError::Protocol(msg),
        }
    }

    pub fn is_key_problem(&self) -> bool {
        matches!(self, EngineError::KeyProblem(_))
    }

    /// HTTP refusal from a service (a REST response) → error by status `code`.
    /// The text holds the start of the body without `secret` (cut out **before** truncation to [`MAX_BODY_CHARS`]);
    /// an empty body gives the standard status reason `reason`. The status is a number: tungstenite and reqwest
    /// have their own `http` dependencies, and their versions need not match.
    pub(crate) fn refusal(code: u16, reason: Option<&str>, body: &str, secret: &str) -> Self {
        let body: String = hide(body.trim(), secret).chars().take(MAX_BODY_CHARS).collect();
        let what = if body.is_empty() { reason.unwrap_or("").to_owned() } else { body };
        EngineError::from_status(code, what)
    }
}

/// HTTP request or response-read failure → network. The reqwest error text holds only the failure kind and the URL:
/// no body and no headers (no key).
pub(crate) fn net(e: impl std::fmt::Display) -> EngineError {
    EngineError::Network(e.to_string())
}

/// REST response: a success is returned as is, a refusal becomes an error by status ([`EngineError::from_status`])
/// with the start of the body; `secret` (the request key) is cut out of the body before truncation.
/// Success bodies never get here: they may hold dictation text.
pub(crate) async fn checked(resp: reqwest::Response, secret: &str) -> Result<reqwest::Response, EngineError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(EngineError::refusal(status.as_u16(), status.canonical_reason(), &body, secret))
}

/// Text without the secret; an empty secret changes nothing.
fn hide(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_owned()
    } else {
        text.replace(secret, "•••")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert!(EngineError::from_status(401, "x").is_key_problem());
        assert!(EngineError::from_status(402, "x").is_key_problem());
        assert!(EngineError::from_status(403, "x").is_key_problem());
        assert!(EngineError::from_status(429, "x").is_key_problem());
        assert_eq!(EngineError::from_status(503, "x"), EngineError::Service("503: x".into()));
        assert_eq!(EngineError::from_status(408, "x"), EngineError::Service("408: x".into()));
        assert_eq!(EngineError::from_status(400, "x"), EngineError::Protocol("400: x".into()));
    }

    #[test]
    fn rest_refusal_is_redacted_before_truncation() {
        let key = "sk-STRADDLE-0123456789";
        let body = format!("{}{key} tail", "x".repeat(MAX_BODY_CHARS - 8));
        let e = EngineError::refusal(402, Some("Payment Required"), &body, key);
        assert!(e.is_key_problem());
        assert!(!e.to_string().contains("sk-STRA"), "a piece of the key at the truncation boundary: {e}");
        let e = EngineError::refusal(502, Some("Bad Gateway"), " \n", key);
        assert_eq!(e, EngineError::Service("502: Bad Gateway".into()), "an empty body gives the status reason");
        assert_eq!(EngineError::refusal(599, None, "", key), EngineError::Service("599: ".into()));
    }

    #[test]
    fn net_failure_is_network() {
        assert_eq!(net("error sending request"), EngineError::Network("error sending request".into()));
    }
}
