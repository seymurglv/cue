//! Windows Credential Manager (service "CUE"): the habits' hash secret (`habit_secret`), the one entry CUE keeps. CUE
//! holds no API key: the keys an earlier version saved for its cloud connections are deleted at startup
//! ([`forget_api_keys`]).
//!
//! No value read from the store reaches the output, the log or an error text.
use crate::error::EngineError;
use cue_core::habits::Secret;

pub const SERVICE: &str = "CUE";

/// The entries earlier versions saved the cloud connections' keys under: one per connection, by its id.
pub const API_KEY_NAMES: [&str; 8] = ["anthropic", "openrouter", "openai", "gemini", "deepseek", "mistral", "groq", "xai"];

/// What [`forget_api_keys`] did: the entries it deleted, and those the store failed on with keyring's reason (its
/// `Display` only, which holds no stored value).
#[derive(Debug, Default, PartialEq)]
pub struct Forgotten {
    pub deleted: Vec<&'static str>,
    pub failed: Vec<(&'static str, String)>,
}

/// Deletes the API keys an earlier version saved ([`API_KEY_NAMES`]); `habits-secret` and anything else stay. An entry
/// that is not there is no failure, and one the store fails on does not stop the others. Blocking: run it off the core
/// thread.
pub fn forget_api_keys() -> Forgotten {
    forget_with(|name| keyring::Entry::new(SERVICE, name).and_then(|e| e.delete_credential()))
}

/// [`forget_api_keys`] over the store's `delete` (tests give their own).
fn forget_with(mut delete: impl FnMut(&str) -> Result<(), keyring::Error>) -> Forgotten {
    let mut gone = Forgotten::default();
    for name in API_KEY_NAMES {
        match delete(name) {
            Ok(()) => gone.deleted.push(name),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => gone.failed.push((name, e.to_string())),
        }
    }
    gone
}

/// The habits' hash secret's entry in the store (docs/specs/2026-11-04-habits.md).
const HABIT_SECRET: &str = "habits-secret";

/// The habits' hash secret as `habit_secret` found it.
#[derive(Debug, PartialEq)]
pub enum SecretRead {
    /// The one the store keeps.
    Kept(Secret),
    /// None was kept, or what was kept is malformed: a new one, and whether the store took it.
    Made { secret: Secret, saved: Result<(), EngineError> },
}

/// The secret the store gave back: 64 hex digits. Missing (`NoEntry`) or malformed (`BadEncoding`, or text that is not
/// a secret) is none, and a new one is made. Any other error is the store that cannot be read: an error, and the secret
/// kept there is left alone. The error text has keyring's reason only: `BadEncoding`'s `Debug` holds the stored bytes.
fn kept_secret(read: Result<String, keyring::Error>) -> Result<Option<Secret>, EngineError> {
    match read {
        Ok(hex) => Ok(Secret::from_hex(hex.trim())),
        Err(keyring::Error::NoEntry | keyring::Error::BadEncoding(_)) => Ok(None),
        Err(e) => Err(EngineError::Service(format!("could not read the habits' secret: {e}"))),
    }
}

/// 32 random bytes from the system.
fn new_secret() -> Result<Secret, EngineError> {
    let mut bytes = [0u8; cue_core::habits::SECRET_BYTES];
    getrandom::fill(&mut bytes)
        .map_err(|e| EngineError::Service(format!("no random bytes for the habits' secret: {e}")))?;
    Ok(Secret::from_bytes(bytes))
}

/// The store's entry of the habits' hash secret, read.
fn read_habit_secret() -> Result<String, keyring::Error> {
    keyring::Entry::new(SERVICE, HABIT_SECRET).and_then(|e| e.get_password())
}

/// The habits' hash secret kept in the store, without making one (the leak scan): `Ok(None)` when it is missing or
/// malformed, `Err` when the store cannot be read.
pub fn kept_habit_secret() -> Result<Option<Secret>, EngineError> {
    kept_secret(read_habit_secret())
}

/// What `habit_secret` does with the store's read: the secret kept; else, missing or malformed, one `make` gives,
/// handed to `save`; an unreadable store is an error, and nothing is made or saved over what it keeps.
fn secret_from(
    read: Result<String, keyring::Error>,
    make: impl FnOnce() -> Result<Secret, EngineError>,
    save: impl FnOnce(&str) -> Result<(), keyring::Error>,
) -> Result<SecretRead, EngineError> {
    if let Some(secret) = kept_secret(read)? {
        return Ok(SecretRead::Kept(secret));
    }
    let secret = make()?;
    let saved =
        save(&secret.to_hex()).map_err(|e| EngineError::Service(format!("could not save the habits' secret: {e}")));
    Ok(SecretRead::Made { secret, saved })
}

/// The habits' hash secret: the one kept, else (missing or malformed) a new one, saved. A save that fails leaves the
/// new one for this session (the next start makes another). `Err` when the store cannot be read (the secret is left
/// unset for the session, nothing is written) or the system gives no random bytes. No text holds the secret.
pub fn habit_secret() -> Result<SecretRead, EngineError> {
    secret_from(read_habit_secret(), new_secret, |hex| {
        keyring::Entry::new(SERVICE, HABIT_SECRET).and_then(|e| e.set_password(hex))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn platform_failure(what: &str) -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(std::io::Error::other(what.to_owned())))
    }

    /// The habits' secret as the store gives it back. Missing or malformed, it is none and a new one is made; a store
    /// that cannot be read is an error, which holds nothing the store keeps.
    #[test]
    fn a_kept_secret_reads_back_a_missing_or_malformed_one_is_none_and_an_unreadable_one_an_error() {
        let secret = Secret::from_bytes([0x5c; 32]);
        assert_eq!(kept_secret(Ok(secret.to_hex())), Ok(Some(secret.clone())));
        assert_eq!(kept_secret(Ok(format!(" {}\r\n", secret.to_hex()))), Ok(Some(secret)));
        assert_eq!(kept_secret(Err(keyring::Error::NoEntry)), Ok(None));
        assert_eq!(kept_secret(Err(keyring::Error::BadEncoding(b"5c5c".to_vec()))), Ok(None));
        assert_eq!(kept_secret(Ok("not a secret".into())), Ok(None));
        let err = kept_secret(Err(platform_failure("boom"))).unwrap_err();
        let EngineError::Service(msg) = &err else { panic!("expected Service, not {err:?}") };
        assert!(msg.contains("habits' secret") && msg.contains("boom"), "{msg}");
        let no_access = keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")));
        assert!(matches!(kept_secret(Err(no_access)), Err(EngineError::Service(_))));
    }

    /// Only a missing or malformed secret is made anew and written; one that cannot be read is left alone: nothing is
    /// made, nothing is written over it, and the session counts nothing.
    #[test]
    fn only_a_missing_or_malformed_secret_is_made_and_written() {
        let made = Secret::from_bytes([0x11; 32]);
        let run = |read: Result<String, keyring::Error>, save: Result<(), keyring::Error>| {
            let (mut makes, mut writes) = (0, Vec::new());
            let out = secret_from(
                read,
                || {
                    makes += 1;
                    Ok(made.clone())
                },
                |hex| {
                    writes.push(hex.to_owned());
                    save
                },
            );
            (out, makes, writes)
        };
        let (out, makes, writes) = run(Err(platform_failure("boom")), Ok(()));
        assert!(matches!(out, Err(EngineError::Service(_))), "{out:?}");
        assert_eq!((makes, writes.len()), (0, 0), "an unreadable secret is never written over");
        for read in [Err(keyring::Error::NoEntry), Err(keyring::Error::BadEncoding(b"x".to_vec())), Ok("zz".into())] {
            let (out, makes, writes) = run(read, Ok(()));
            assert_eq!(out, Ok(SecretRead::Made { secret: made.clone(), saved: Ok(()) }));
            assert_eq!((makes, writes), (1, vec![made.to_hex()]));
        }
        let kept = Secret::from_bytes([0x5c; 32]);
        let (out, makes, writes) = run(Ok(kept.to_hex()), Ok(()));
        assert_eq!((out, makes, writes.len()), (Ok(SecretRead::Kept(kept)), 0, 0));
        let (out, _, _) = run(Err(keyring::Error::NoEntry), Err(platform_failure("denied")));
        let Ok(SecretRead::Made { saved: Err(EngineError::Service(msg)), .. }) = &out else { panic!("{out:?}") };
        assert!(msg.contains("habits' secret") && msg.contains("denied"), "{msg}");
    }

    /// A store of entries by name, as Credential Manager keeps service `CUE`'s: the eight keys an earlier version saved
    /// go, `habits-secret` stays, and one the store fails on does not stop the others.
    #[test]
    fn forget_api_keys_deletes_the_eight_and_keeps_habits_secret() {
        let mut store: BTreeMap<String, String> = API_KEY_NAMES
            .iter()
            .map(|n| (n.to_string(), format!("sk-SECRET-{n}")))
            .chain([(HABIT_SECRET.to_owned(), "5c".repeat(32)), ("soniox".to_owned(), "x".into())])
            .collect();
        let gone = forget_with(|name| store.remove(name).map(|_| ()).ok_or(keyring::Error::NoEntry));
        assert_eq!(gone.deleted, API_KEY_NAMES.to_vec());
        assert!(gone.failed.is_empty());
        assert_eq!(store.keys().map(String::as_str).collect::<Vec<_>>(), [HABIT_SECRET, "soniox"], "only the eight go");
        assert_eq!(API_KEY_NAMES, ["anthropic", "openrouter", "openai", "gemini", "deepseek", "mistral", "groq", "xai"]);

        // Run again: nothing is left to delete, and that is no failure.
        let again = forget_with(|_| Err(keyring::Error::NoEntry));
        assert_eq!(again, Forgotten::default());

        // A store that fails on one name: the others still go; the failure names the entry and the reason only.
        let mut asked = Vec::new();
        let partly = forget_with(|name| {
            asked.push(name.to_owned());
            if name == "openai" { Err(platform_failure("denied")) } else { Ok(()) }
        });
        assert_eq!(asked.len(), 8, "every name is tried");
        assert_eq!(partly.deleted.len(), 7);
        assert_eq!(partly.failed.len(), 1);
        let (name, why) = &partly.failed[0];
        assert_eq!(*name, "openai");
        assert!(why.contains("denied") && !why.contains("SECRET"), "{why}");
        assert!(!format!("{partly:?}").contains("SECRET"));
    }

    #[test]
    fn a_new_secret_is_random_and_shows_nothing() {
        let (a, b) = (new_secret().unwrap(), new_secret().unwrap());
        assert!(a.is_set() && a != b);
        assert_eq!(format!("{a:?}"), "Secret(..)");
        let made = SecretRead::Made { secret: a.clone(), saved: Ok(()) };
        assert!(!format!("{made:?}").contains(&a.to_hex()[..8]), "the secret made is not shown");
    }
}
