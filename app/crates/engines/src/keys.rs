//! API keys in Windows Credential Manager (service "CUE"). For personal use only:
//! in the product the keys are held by the CUE server.
//!
//! A key's value never reaches the output, the log or an error text — only [`mask`].
use crate::error::EngineError;

pub const SERVICE: &str = "CUE";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    Soniox,
    Anthropic,
    /// Comparing refine models through OpenRouter; the app does not read this key yet.
    OpenRouter,
}

impl KeyName {
    pub const ALL: [KeyName; 3] = [KeyName::Soniox, KeyName::Anthropic, KeyName::OpenRouter];

    pub fn as_str(self) -> &'static str {
        match self {
            KeyName::Soniox => "soniox",
            KeyName::Anthropic => "anthropic",
            KeyName::OpenRouter => "openrouter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        KeyName::ALL.into_iter().find(|k| k.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

/// Below this length the mask shows neither the start nor the end: "start…end" would reveal most of it.
const MASK_MIN_CHARS: usize = 20;

/// "sk-ant…f3a9" — shows which key is saved without revealing it.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() < MASK_MIN_CHARS {
        return "•".repeat(chars.len());
    }
    let head: String = chars[..6].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

/// A pasted key without spaces and line breaks at the edges; an empty one is rejected.
fn prepare(value: &str) -> Result<&str, EngineError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(EngineError::KeyProblem("an empty key is not saved".into()));
    }
    Ok(value)
}

/// Outcome of a store read: the key is there / the key is missing (`None`) / failure.
/// keyring errors go only through Display: Debug of `BadEncoding` holds the secret's bytes.
fn read_outcome(
    name: KeyName,
    read: Result<String, keyring::Error>,
) -> Result<Option<String>, EngineError> {
    let name = name.as_str();
    match read {
        Ok(value) => Ok(prepare(&value).ok().map(str::to_owned)),
        Err(keyring::Error::NoEntry) => Ok(None),
        // The saved value is corrupted: the key has to be entered again.
        Err(keyring::Error::BadEncoding(_)) => Err(EngineError::KeyProblem(format!(
            "the {name} key is saved in a wrong encoding, enter it again"
        ))),
        Err(e) => Err(EngineError::Service(format!("could not read the {name} key: {e}"))),
    }
}

/// A store write failure is a refusal of an external service, not a key problem.
fn write_error(name: KeyName, e: keyring::Error) -> EngineError {
    EngineError::Service(format!("could not save the {} key: {e}", name.as_str()))
}

/// Key from Credential Manager: `Ok(None)` if it is missing; `Err` if the store failed.
/// The error text holds only the key name and the reason, never the value.
pub fn lookup(name: KeyName) -> Result<Option<String>, EngineError> {
    read_outcome(
        name,
        keyring::Entry::new(SERVICE, name.as_str()).and_then(|e| e.get_password()),
    )
}

/// A key the engine cannot work without: a missing one is `KeyProblem`.
pub fn get(name: KeyName) -> Result<String, EngineError> {
    lookup(name)?.ok_or_else(|| EngineError::KeyProblem(format!("no {} key", name.as_str())))
}

/// Saves the key (without spaces at the edges). An empty key is not saved.
pub fn set(name: KeyName, value: &str) -> Result<(), EngineError> {
    let value = prepare(value)?;
    keyring::Entry::new(SERVICE, name.as_str())
        .and_then(|e| e.set_password(value))
        .map_err(|e| write_error(name, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_names() {
        assert_eq!(KeyName::parse("soniox"), Some(KeyName::Soniox));
        assert_eq!(KeyName::parse("Anthropic"), Some(KeyName::Anthropic));
        assert_eq!(KeyName::parse("openrouter"), Some(KeyName::OpenRouter));
        assert_eq!(KeyName::parse(" OpenRouter "), Some(KeyName::OpenRouter));
        assert_eq!(KeyName::parse("openai"), None);
        // The key of a dropped engine is gone: its name is like any unknown one.
        assert_eq!(KeyName::parse("elevenlabs"), None);
        // `keys show` prints a line for every key in `ALL`.
        assert_eq!(KeyName::ALL.map(KeyName::as_str).as_slice(), ["soniox", "anthropic", "openrouter"]);
    }

    #[test]
    fn mask_hides_middle() {
        assert_eq!(mask("sk-ant-api03-abcdefghijklmnop-f3a9"), "sk-ant…f3a9");
        assert_eq!(mask("short"), "•••••");
    }

    #[test]
    fn mask_hides_short_values_entirely() {
        // For a short value "start…end" would reveal almost the whole key.
        assert_eq!(mask("abcdefghijkl"), "•".repeat(12));
        assert_eq!(mask("abcdefghijklmnopqrs"), "•".repeat(19));
        assert_eq!(mask("abcdefghijklmnopqrst"), "abcdef…qrst");
        assert_eq!(mask(""), "");
    }

    #[test]
    fn prepare_trims_and_rejects_empty() {
        assert_eq!(prepare("  sk-key-123\r\n"), Ok("sk-key-123"));
        assert_eq!(prepare("sk-key-123"), Ok("sk-key-123"));
        let empty = prepare(" \t\r\n").unwrap_err();
        assert!(empty.is_key_problem());
        assert!(prepare("").is_err());
    }

    fn platform_failure(what: &str) -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(std::io::Error::other(what.to_owned())))
    }

    #[test]
    fn read_outcome_present_missing_blank() {
        let k = KeyName::Soniox;
        assert_eq!(read_outcome(k, Ok("  sk-key-123\r\n".into())), Ok(Some("sk-key-123".into())));
        assert_eq!(read_outcome(k, Err(keyring::Error::NoEntry)), Ok(None));
        assert_eq!(read_outcome(k, Ok(" \t".into())), Ok(None));
    }

    #[test]
    fn read_outcome_store_failure_is_visible_service_error() {
        let err = read_outcome(KeyName::Anthropic, Err(platform_failure("boom"))).unwrap_err();
        let EngineError::Service(msg) = &err else { panic!("expected Service, not {err:?}") };
        assert!(msg.contains("anthropic") && msg.contains("boom"), "{msg}");
    }

    #[test]
    fn read_outcome_bad_encoding_never_shows_secret() {
        let secret = b"sk-ant-SECRET-BYTES".to_vec();
        let err = read_outcome(KeyName::Soniox, Err(keyring::Error::BadEncoding(secret))).unwrap_err();
        assert!(err.is_key_problem());
        for shown in [err.to_string(), format!("{err:?}")] {
            assert!(!shown.contains("SECRET") && !shown.contains("83, 69, 67"), "{shown}");
            assert!(shown.contains("soniox"), "{shown}");
        }
    }

    #[test]
    fn write_failure_is_service_error() {
        let err = write_error(KeyName::Anthropic, platform_failure("denied"));
        let EngineError::Service(msg) = &err else { panic!("expected Service, not {err:?}") };
        assert!(msg.contains("anthropic") && msg.contains("denied"), "{msg}");
        let too_long = keyring::Error::TooLong("password encoded as UTF-16".into(), 2560);
        assert!(matches!(write_error(KeyName::Soniox, too_long), EngineError::Service(_)));
    }
}
