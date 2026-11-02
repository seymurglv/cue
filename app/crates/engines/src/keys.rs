//! API keys in Windows Credential Manager (service "CUE"): the keys of the cleanup's cloud connections.
//!
//! A key's value never reaches the output, the log or an error text — only [`mask`].
use crate::error::EngineError;

pub const SERVICE: &str = "CUE";

/// The keys of the cleanup's cloud connections (`cue_core::connections`), each stored under its connection's id.
/// Anthropic's is "Ask"'s too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyName {
    Anthropic,
    OpenRouter,
    OpenAi,
    Gemini,
    DeepSeek,
    Mistral,
    Groq,
    Xai,
}

impl KeyName {
    pub const ALL: [KeyName; 8] = [
        KeyName::Anthropic,
        KeyName::OpenRouter,
        KeyName::OpenAi,
        KeyName::Gemini,
        KeyName::DeepSeek,
        KeyName::Mistral,
        KeyName::Groq,
        KeyName::Xai,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            KeyName::Anthropic => "anthropic",
            KeyName::OpenRouter => "openrouter",
            KeyName::OpenAi => "openai",
            KeyName::Gemini => "gemini",
            KeyName::DeepSeek => "deepseek",
            KeyName::Mistral => "mistral",
            KeyName::Groq => "groq",
            KeyName::Xai => "xai",
        }
    }

    /// The key of the connection `id`; `None` for a local server, which needs none, and an unknown id.
    pub fn of_connection(id: &str) -> Option<Self> {
        KeyName::ALL.into_iter().find(|k| k.as_str() == id)
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

/// The Soniox key of versions before local recognition, deleted once: CUE does not use it any more. `true` if there
/// was one.
pub fn forget_soniox() -> Result<bool, EngineError> {
    match keyring::Entry::new(SERVICE, "soniox").and_then(|e| e.delete_credential()) {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(EngineError::Service(format!("could not delete the soniox key: {e}"))),
    }
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
        assert_eq!(KeyName::parse("Anthropic"), Some(KeyName::Anthropic));
        assert_eq!(KeyName::parse("openrouter"), Some(KeyName::OpenRouter));
        assert_eq!(KeyName::parse(" OpenRouter "), Some(KeyName::OpenRouter));
        assert_eq!(KeyName::parse("OpenAI"), Some(KeyName::OpenAi));
        assert_eq!(KeyName::parse("bard"), None);
        // The keys of dropped engines are gone: their names are like any unknown one.
        assert_eq!(KeyName::parse("elevenlabs"), None);
        assert_eq!(KeyName::parse("soniox"), None);
        // `keys show` prints a line for every key in `ALL`.
        let all = ["anthropic", "openrouter", "openai", "gemini", "deepseek", "mistral", "groq", "xai"];
        assert_eq!(KeyName::ALL.map(KeyName::as_str).as_slice(), all);
    }

    /// Every cloud connection has its key under its id; a local server has none.
    #[test]
    fn every_cloud_connection_has_a_key() {
        for c in &cue_core::connections::CONNECTIONS {
            assert_eq!(KeyName::of_connection(c.id).is_some(), !c.local, "{}", c.id);
            if let Some(k) = KeyName::of_connection(c.id) {
                assert_eq!(k.as_str(), c.id);
            }
        }
        assert_eq!(KeyName::of_connection("bard"), None);
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
        let k = KeyName::OpenRouter;
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
        let err = read_outcome(KeyName::OpenRouter, Err(keyring::Error::BadEncoding(secret))).unwrap_err();
        assert!(err.is_key_problem());
        for shown in [err.to_string(), format!("{err:?}")] {
            assert!(!shown.contains("SECRET") && !shown.contains("83, 69, 67"), "{shown}");
            assert!(shown.contains("openrouter"), "{shown}");
        }
    }

    #[test]
    fn write_failure_is_service_error() {
        let err = write_error(KeyName::Anthropic, platform_failure("denied"));
        let EngineError::Service(msg) = &err else { panic!("expected Service, not {err:?}") };
        assert!(msg.contains("anthropic") && msg.contains("denied"), "{msg}");
        let too_long = keyring::Error::TooLong("password encoded as UTF-16".into(), 2560);
        assert!(matches!(write_error(KeyName::OpenRouter, too_long), EngineError::Service(_)));
    }
}
