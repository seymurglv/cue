### Task 11: the hash's secret (`habits::Secret`, Credential Manager, a new one resets the counts)

Decided 06.11.2026 (spec, "The hash's secret"). Line numbers are those of the working tree after Tasks 1–10.

**Files:**
- Modify: `Cargo.toml` (workspace dependencies, after `sha2 = "0.10"`, line 42), `app/crates/core/Cargo.toml`,
  `app/crates/engines/Cargo.toml`
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/settings.rs` (test, line 2201)
- Modify: `app/crates/engines/src/keys.rs`, `app/src-tauri/src/runtime.rs` (`run`, line 472), `app/crates/cli/src/leak.rs`
  (`gather`, line 186)
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Tasks 1–10 as built.
- Produces in `habits`: `pub const SECRET_BYTES: usize = 32`; `pub struct Secret([u8; 32])` (Clone, PartialEq, Eq,
  Default = all zero = no secret; hand-written `Debug`: `Secret(..)` or `Secret(none)`, nothing of the bytes) with
  `from_bytes([u8; 32])`, `is_set()`, `to_hex() -> String` (64 hex digits, for the key store only),
  `from_hex(&str) -> Option<Secret>`; `Form::hash(&self, secret: &Secret) -> String` (pure: HMAC-SHA-256 of the kind and
  detail under the secret, the first 8 bytes as 16 hex digits); `Habit::hashes(&self, secret: &Secret)`;
  `Habits.secret: Secret` (`#[serde(skip)]`: never in settings.json); `Habits::rekey(&mut self, Secret) -> (usize, usize)`
  (the counts and declined forms dropped); `id_of(forms: &[Form], secret: &Secret) -> String`. `Habits::is_default`
  ignores the secret, `forget_all` keeps it, and `count` counts nothing without one.
- Produces in `keys`: `pub enum SecretRead { Kept(Secret), Made { secret: Secret, saved: Result<(), EngineError> } }`,
  `pub fn habit_secret() -> Result<SecretRead, EngineError>` (reads, or makes and saves a new one; `Err` only without
  random bytes), `pub fn kept_habit_secret() -> Option<Secret>` (reads, never makes); the entry `habits-secret` under
  `SERVICE` ("CUE").

- [ ] **Step 1: write the failing tests.** In `habits.rs`'s `mod tests`, after `const STOP`:

```rust
    fn key(byte: u8) -> Secret {
        Secret::from_bytes([byte; SECRET_BYTES])
    }
    /// The habits with a secret, as the runtime holds them.
    fn habits() -> Habits {
        Habits { secret: key(7), ..Habits::default() }
    }
```

  Replace `the_hash_is_stable` (line 839) with:

```rust
    /// HMAC-SHA-256 under the secret: the same form and secret give the same hash in every version, so a count
    /// survives an update; another secret gives another hash.
    #[test]
    fn the_hash_is_stable_under_one_secret_and_another_under_another() {
        let k = key(1);
        assert_eq!(Form::Digits.hash(&k), "0cda9c2120dc4085");
        assert_eq!(Form::Dash.hash(&k), "1e280b5650f82678");
        assert_eq!(STOP.hash(&k), "d61bb7341f7ef877");
        assert_eq!(Form::FirstLetter { lower: true }.hash(&k), "1465766befda9e4e");
        assert_eq!(Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }.hash(&k), "06f5620c5fa0f9db");
        assert_eq!(Form::Digits.hash(&k), Form::Digits.hash(&key(1)), "the same form and secret");
        assert_ne!(Form::EndMark { mark: '.', removed: false }.hash(&k), STOP.hash(&k));
        for form in [Form::Digits, Form::Dash, STOP] {
            assert_ne!(form.hash(&k), form.hash(&key(2)), "{form:?} under another secret");
        }
        assert_eq!(Form::Digits.hash(&key(2)), "d74d04473faae824");
    }

    /// RFC 4231, test case 2.
    #[test]
    fn hmac_sha256_is_the_standard_one() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn the_secret_reads_back_from_its_hex_digits_and_shows_nothing() {
        let k = key(0x5c);
        assert_eq!(k.to_hex(), "5c".repeat(SECRET_BYTES));
        assert_eq!(Secret::from_hex(&k.to_hex()), Some(k.clone()));
        let (short, long, letters, signs, zero) =
            ("5c".to_owned(), "5c".repeat(SECRET_BYTES + 1), "zz".repeat(SECRET_BYTES), "+5".repeat(SECRET_BYTES), "00".repeat(SECRET_BYTES));
        for bad in ["", &short, &long, &letters, &signs, &zero] {
            assert_eq!(Secret::from_hex(bad), None, "{bad}");
        }
        assert_eq!(format!("{k:?}"), "Secret(..)");
        assert_eq!(format!("{:?}", Secret::default()), "Secret(none)");
        let h = Habits { secret: k, ..Habits::default() };
        let shown = format!("{h:?}");
        assert!(!shown.contains("5c5c") && !shown.contains("92, 92"), "{shown}");
        assert_eq!(serde_json::to_value(&h).unwrap(), serde_json::json!({}), "never written");
    }
```

  After `three_times_in_one_app_make_one_habit_of_the_forms_together`:

```rust
    #[test]
    fn without_a_secret_nothing_is_counted() {
        let mut h = Habits { secret: Secret::default(), ..Habits::default() };
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..LEARN_AT {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty());
    }

    /// The secret was missing or unreadable, and a new one was made: the counts and the declined forms go (their hashes
    /// no longer match), the habits keep working with their ids made again.
    #[test]
    fn a_new_secret_drops_the_counts_and_the_declined_forms_and_keeps_the_habits() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        h.declined.push(Form::Digits.hash(&h.secret));
        assert_eq!(h.rekey(key(9)), (1, 1));
        assert!(h.seen.is_empty() && h.declined.is_empty());
        assert_eq!(h.secret, key(9));
        let id = id_of(&[Form::FirstLetter { lower: true }, STOP], &key(9));
        assert_ne!(id, old);
        assert_eq!(h.list[0].id, id, "its id is made with the new secret");
        assert_eq!(h.for_app(TG).count(), 1, "still applied");
        assert!(h.count(TG, &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en").grown, "and counted on");
        assert!(h.forget(&id));
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(&key(9)), STOP.hash(&key(9))]);
    }
```

  The existing tests take the secret (the counting ones need it; nothing else in them changes):
  - every `let mut h = Habits::default();` (twelve of them) becomes `let mut h = habits();`;
  - `counts_older_than_thirty_days_go_and_at_most_three_hundred_stay`: `let mut h = habits();` moves up, and the first
    line becomes `let (dash, digits) = (Form::Dash.hash(&h.secret), Form::Digits.hash(&h.secret));`;
  - `a_forgotten_habit_is_never_learned_again_until_all_is_forgotten`: `[Form::FirstLetter { lower: true }.hash(&h.secret), STOP.hash(&h.secret)]`,
    and `assert_eq!(h, Habits::default());` becomes `assert_eq!(h, habits(), "the secret stays");`;
  - `three_times_in_one_app_make_one_habit_of_the_forms_together`: `assert_eq!(habit.id, "hca6cdd4168133ac4");`
    (the FNV-1a of its forms' hashes under `key(7)`).

  In `settings.rs`, line 2201: `assert_eq!(s.habits.declined, [crate::habits::Form::Dash.hash(&s.habits.secret)]);`.

  In `keys.rs`'s tests, after `write_failure_is_service_error`:

```rust
    /// The habits' secret as the store gives it back; missing, unreadable or not a secret, it is none and a new one is
    /// made. Nothing of a store error goes anywhere.
    #[test]
    fn a_kept_secret_reads_back_and_anything_else_is_none() {
        let secret = Secret::from_bytes([0x5c; 32]);
        assert_eq!(kept_secret(Ok(secret.to_hex())), Some(secret.clone()));
        assert_eq!(kept_secret(Ok(format!(" {}\r\n", secret.to_hex()))), Some(secret));
        assert_eq!(kept_secret(Err(keyring::Error::NoEntry)), None);
        assert_eq!(kept_secret(Err(platform_failure("boom"))), None);
        assert_eq!(kept_secret(Err(keyring::Error::BadEncoding(b"5c5c".to_vec()))), None);
        assert_eq!(kept_secret(Ok("not a secret".into())), None);
    }

    #[test]
    fn a_new_secret_is_random_and_shows_nothing() {
        let (a, b) = (new_secret().unwrap(), new_secret().unwrap());
        assert!(a.is_set() && a != b);
        assert_eq!(format!("{a:?}"), "Secret(..)");
        let made = SecretRead::Made { secret: a, saved: Ok(()) };
        assert!(!format!("{made:?}").contains(&b.to_hex()[..8]));
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core habits` and `cargo test -p cue-engines keys`
  (from `C:\Users\seymurglv\Desktop\cue-dev`). Expected: compile errors, `Secret`, `hmac_sha256`, `rekey`, `kept_secret`
  and `new_secret` are not defined and `hash` takes no secret.

- [ ] **Step 3: the dependencies.** In the workspace `Cargo.toml`, after `sha2 = "0.10"`:

```toml
# The habits' hash secret (cue-engines::keys): 32 random bytes from the system. Already in Cargo.lock through tauri.
getrandom = "0.3"
```

  In `app/crates/core/Cargo.toml`'s `[dependencies]`: `sha2.workspace = true` (the habits' keyed hash). In
  `app/crates/engines/Cargo.toml`'s `[dependencies]`, after `sha2.workspace = true`: `getrandom.workspace = true`.

- [ ] **Step 4: the secret and the keyed hash** (`habits.rs`). Imports: `use sha2::{Digest, Sha256};` after the `serde`
  line. Above `/// A kind of correction with its detail; counted by its [`Form::hash`].`:

```rust
/// The length of the hashes' secret, in bytes.
pub const SECRET_BYTES: usize = 32;

/// This installation's secret the forms' hashes are made with (docs/specs/2026-11-04-habits.md, "The hash's secret"):
/// 32 random bytes kept in Windows Credential Manager, never in settings.json, so that hashing common words does not
/// tell which ones `habits.seen` and `habits.declined` count. All zero is no secret, and nothing is counted with it.
/// `Debug` shows nothing of it; only the key store gets its hex digits (`to_hex`).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret([u8; SECRET_BYTES]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_set() { "Secret(..)" } else { "Secret(none)" })
    }
}

impl Secret {
    pub fn from_bytes(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// Made: not all zero.
    pub fn is_set(&self) -> bool {
        self.0.iter().any(|b| *b != 0)
    }

    /// As the key store keeps it: 64 lower-case hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// From the key store's 64 hex digits; anything else, or all zero, is none.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 2 * SECRET_BYTES || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut bytes = [0u8; SECRET_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(bytes)).filter(Secret::is_set)
    }
}

/// HMAC-SHA-256 (RFC 2104) of `message` under `key`.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| k.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}
```

  `Form::hash` (line 92) takes the secret; its `canon` stays as it is, and the last line hashes it under the secret:

```rust
    /// What the form is counted by: its kind and detail hashed under this installation's secret (HMAC-SHA-256, its
    /// first 8 bytes as 16 hex digits). Never its text, and without the secret no guess at its words.
    pub fn hash(&self, secret: &Secret) -> String {
        let canon = match self {
            // … the five arms as they are …
        };
        hmac_sha256(&secret.0, canon.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
```

  `correction` (line 170) keeps each form once by the form itself, not its hash (it has no secret):

```rust
    let mut forms = Vec::new();
    found.retain(|f| {
        let first = !forms.contains(&f.form);
        forms.push(f.form.clone());
        first
    });
```

  `Habit::hashes` (line 369):

```rust
    pub fn hashes(&self, secret: &Secret) -> Vec<String> {
        self.forms.iter().map(|f| f.hash(secret)).collect()
    }
```

  `Habits` (line 395): its doc comment ends "… Each list is written only when it is not empty. The secret the hashes are
  made with is never written: the runtime sets it from the key store."; after `declined`:

```rust
    #[serde(skip)]
    pub secret: Secret,
```

  `is_default` (line 479) no longer compares the whole struct (a set secret would write `"habits": {}`):

```rust
    /// Nothing counted, learned or declined (the secret aside).
    pub fn is_default(&self) -> bool {
        self.seen.is_empty() && self.list.is_empty() && self.declined.is_empty()
    }
```

  In `count` (line 494): the doc comment ends "… the least recently seen. Without the secret nothing is counted.", the
  first check becomes `if app.is_empty() || !self.secret.is_set() {`, and every `f.form.hash()` becomes
  `f.form.hash(&self.secret)`, every `h.hashes()` `h.hashes(&self.secret)` (the closures borrow `self.secret` beside
  `self.list`: disjoint fields). In `listed` and `adopt`: `let id = id_of(&draft.forms, &self.secret);`. In `forget`:
  `for hash in self.list.remove(i).hashes(&self.secret) {`. `forget_all` keeps the secret, and `rekey` follows it:

```rust
    /// Every habit goes, with the counts and the declined forms; the secret stays. False when there was nothing.
    pub fn forget_all(&mut self) -> bool {
        let changed = !self.is_default();
        self.seen.clear();
        self.list.clear();
        self.declined.clear();
        changed
    }

    /// A new secret, the one kept being missing or unreadable: the counts and the declined forms were hashed with the
    /// old one and go; the habits keep their forms, and their ids are made again. How many counts and declined forms
    /// went.
    pub fn rekey(&mut self, secret: Secret) -> (usize, usize) {
        let dropped = (self.seen.len(), self.declined.len());
        self.seen.clear();
        self.declined.clear();
        for h in &mut self.list {
            h.id = id_of(&h.forms, &secret);
        }
        self.secret = secret;
        dropped
    }
```

  `id_of` (line 639):

```rust
/// A habit's id: the hash of its forms' hashes (made with the secret).
pub fn id_of(forms: &[Form], secret: &Secret) -> String {
    let hashes: Vec<String> = forms.iter().map(|f| f.hash(secret)).collect();
    format!("h{:016x}", fnv1a(hashes.join(",").as_bytes()))
}
```

  The module comment's second paragraph: "A form is counted by its hash under this installation's secret, never by its
  text. …".

- [ ] **Step 5: the store** (`keys.rs`). Imports: `use cue_core::habits::Secret;` after `use crate::error::EngineError;`.
  After `set`:

```rust
/// The habits' hash secret's entry in the store, beside the connections' keys (docs/specs/2026-11-04-habits.md).
const HABIT_SECRET: &str = "habits-secret";

/// The habits' hash secret as `habit_secret` found it.
#[derive(Debug, PartialEq)]
pub enum SecretRead {
    /// The one the store keeps.
    Kept(Secret),
    /// None was kept, or it could not be read: a new one, and whether the store took it.
    Made { secret: Secret, saved: Result<(), EngineError> },
}

/// The secret the store gave back: 64 hex digits. Missing, unreadable or anything else is none. A keyring error goes
/// nowhere: `BadEncoding`'s `Debug` holds the stored bytes.
fn kept_secret(read: Result<String, keyring::Error>) -> Option<Secret> {
    read.ok().and_then(|hex| Secret::from_hex(hex.trim()))
}

/// 32 random bytes from the system.
fn new_secret() -> Result<Secret, EngineError> {
    let mut bytes = [0u8; cue_core::habits::SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| EngineError::Service(format!("no random bytes for the habits' secret: {e}")))?;
    Ok(Secret::from_bytes(bytes))
}

/// The habits' hash secret kept in the store, without making one (the leak scan).
pub fn kept_habit_secret() -> Option<Secret> {
    kept_secret(keyring::Entry::new(SERVICE, HABIT_SECRET).and_then(|e| e.get_password()))
}

/// The habits' hash secret: the one kept, else a new one, saved. A save that fails leaves the new one for this session
/// (the next start makes another). `Err` only when the system gives no random bytes. No text holds the secret.
pub fn habit_secret() -> Result<SecretRead, EngineError> {
    if let Some(secret) = kept_habit_secret() {
        return Ok(SecretRead::Kept(secret));
    }
    let secret = new_secret()?;
    let saved = keyring::Entry::new(SERVICE, HABIT_SECRET)
        .and_then(|e| e.set_password(&secret.to_hex()))
        .map_err(|e| EngineError::Service(format!("could not save the habits' secret: {e}")));
    Ok(SecretRead::Made { secret, saved })
}
```

  The module comment gains a line: "and the habits' hash secret (`habit_secret`), under its own name; the same rule
  holds for it."

- [ ] **Step 6: the runtime sets it before the first event.** In `run` (line 472), after `forget_soniox_key();`:
  `self.key_habits();`. After `cleanup_key`:

```rust
    /// The habits' hash secret from the key store (docs/specs/2026-11-04-habits.md, "The hash's secret"), before the
    /// first correction is counted. A new one (none was kept, or it could not be read) leaves the counts and the
    /// declined forms meaningless: they go, the habits' ids are made again, and the file is written. Without random
    /// bytes nothing is counted this session. The log never gets the secret.
    fn key_habits(&mut self) {
        match keys::habit_secret() {
            Ok(keys::SecretRead::Kept(secret)) => self.settings.habits.secret = secret,
            Ok(keys::SecretRead::Made { secret, saved }) => {
                if let Err(e) = saved {
                    tracing::warn!("{e}");
                }
                let (seen, declined) = self.settings.habits.rekey(secret);
                let habits = self.settings.habits.list.len();
                tracing::info!(seen, declined, habits, "habits: a new hash secret, counts and declined forms dropped");
                if seen + declined + habits > 0 {
                    self.save_settings();
                }
            }
            Err(e) => tracing::warn!("{e}: no habit is counted"),
        }
    }
```

- [ ] **Step 7: the leak scan looks for it** like a saved key (`leak.rs`, `gather`, after the keys' loop, line 192):

```rust
        // The habits' hash secret, as the store keeps it (never made here).
        if let Some(secret) = keys::kept_habit_secret() {
            private.add(Kind::SavedKey("habits' secret"), &secret.to_hex(), 1);
        }
```

  The module comment's list says "the keys saved in Credential Manager (the habits' hash secret among them)".

- [ ] **Step 8: README and ARCHITECTURE.**
  - README, Privacy (line 89), after "…are the only text of a correction it writes to disk.": "What it counts on the
    way is kept by a hash made with a random secret of this computer, which Windows Credential Manager keeps beside the
    keys (never in `settings.json`), so a copy of the file does not tell which words were counted."
  - README, `habits.seen` (line 534): "the form by its hash (made with this computer's secret) and no text".
  - README, privacy in detail, the **Habits** bullet (line 750): after "…a number and a day, no text.": "The hash is
    made with a random secret of this installation (32 bytes, an HMAC), kept in Windows Credential Manager under
    `habits-secret` beside the keys, never in `settings.json` or the log. When it is missing or cannot be read, CUE
    makes a new one at start and drops the counts and the declined corrections (their hashes no longer match); the
    habits keep working."
  - ARCHITECTURE, Habits, step 2: the sentence "A form is counted by its hash, FNV-1a written out (`Form::hash`),
    because std's hasher is not stable between versions." becomes "A form is counted by its hash under this
    installation's secret (`Form::hash(&Secret)`: HMAC-SHA-256 of its kind and detail, written out over `sha2`, the
    first 8 bytes; a habit's id is FNV-1a over its forms' hashes). The secret (`habits::Secret`, 32 random bytes) is
    kept in Credential Manager (`keys::habit_secret`, the entry `habits-secret`); `Runtime::run` sets it before the
    first event (`key_habits`), and a new one goes through `Habits::rekey`: `seen` and `declined` go, the ids are made
    again. It never reaches settings.json (`#[serde(skip)]`) or a log (`Secret(..)`), and without one nothing is
    counted."

- [ ] **Step 9: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` (from
  `C:\Users\seymurglv\Desktop\cue-dev`): green. `Cargo.lock` gains `sha2` under `cue-core` and `getrandom 0.3.4` under
  `cue-engines`, nothing new downloaded.

---

