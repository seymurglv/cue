# Task 11 report: the hash's secret

Status: DONE. No commits, nothing staged.

## What was done

- **TDD:** I added the brief's tests first: `habits.rs` (stable hash under one secret and different under another,
  RFC 4231 HMAC, hex round-trip and Debug, nothing counted without a secret, `rekey`), the twelve
  `Habits::default()` replaced by `habits()`, the edits to `counts_older…` and `a_forgotten_habit…`, the new id
  `hca6cdd4168133ac4`, `settings.rs:2201`, and the two tests in `keys.rs`. `cargo test -p cue-core habits` failed to
  compile as expected (`Secret`, `SECRET_BYTES`, `hmac_sha256`, `rekey` and `secret` missing; `hash` taking no argument).
- **Dependencies:** in the workspace `Cargo.toml`, `getrandom = "0.3"` follows `sha2`. `cue-core` gets
  `sha2.workspace = true` and `cue-engines` gets `getrandom.workspace = true`. Cargo.lock gains only `sha2 0.10.9` under
  cue-core and `getrandom 0.3.4` under cue-engines. Nothing new was downloaded.
- **HMAC:** written by hand over `sha2` per RFC 2104 (ruling 19), with no `hmac` crate. It is checked against RFC 4231
  test case 2. Every hash value the plan expects matched on the first run.
- **`habits.rs`:** `SECRET_BYTES`, `Secret` (its Debug prints `Secret(..)` or `Secret(none)`; `from_hex` rejects all
  zeros), `hmac_sha256`, `Form::hash(&Secret)`, `Habit::hashes(&Secret)`, `Habits.secret` with `#[serde(skip)]`,
  `is_default` that ignores the secret, `count` that does nothing without a secret, `forget_all` that keeps the secret,
  `rekey`, `id_of(forms, &Secret)`, `correction` deduplicating by form, and updated module and doc comments.
- **`keys.rs`:** `HABIT_SECRET = "habits-secret"`, `SecretRead`, `kept_secret`, `new_secret`, `kept_habit_secret`,
  `habit_secret`, and the module comment line. Error texts carry only keyring's Display text, never the secret or the
  stored bytes.
- **`runtime.rs`:** `run` calls `self.key_habits()` right after `forget_soniox_key()`, before the event loop.
  `key_habits` follows the brief: a kept secret is set; a new one rekeys, logs counts only and saves when there was
  anything; with no random bytes it logs a warning and nothing is counted. The window's settings reply is served inside
  the loop, so it always sees the new ids.
- **`leak.rs`:** `gather` adds the kept secret's hex as `Kind::SavedKey("habits' secret")` and never makes one. The
  module comment is updated and rewrapped to 120 columns.
- **README:** I added the brief's three passages (Privacy, `habits.seen`, and the Habits bullet in privacy in detail).
  The Habits bullet was rewrapped as a whole; it is new in this feature, not in HEAD. The `seen` bullet was rewrapped to
  120 columns.
- **ARCHITECTURE:** step 2's FNV-1a sentence is replaced with the brief's text.

## Checks (from the repo root and from `app`)

- `cargo build --workspace --all-targets`: OK.
- `cargo test --workspace`: all green (core 425, src-tauri 203 plus 1 ignored, engines 205 plus 4 ignored, cli 265
  plus 1 ignored, and the others; 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `npx vitest run`: 15 files, 399 tests passed.
- `npx tsc --noEmit -p tsconfig.json`: clean.

## Deviations from the plan

1. **`keys.rs` `a_new_secret_is_random_and_shows_nothing`:** the plan checked that `made`'s Debug lacks `b`'s hex, but
   `b` is not in `made`, so that check proved nothing. It now wraps `a.clone()` and checks that `a`'s hex is absent.
2. **`count`:** the two closures borrow `let secret = &self.secret;` (a local next to `self.list`) instead of reading
   `&self.secret` inline. Same behaviour.
3. **`core/Cargo.toml`:** `sha2` has a one-line comment above it ("The habits' keyed hash (HMAC-SHA-256).").
4. **Line endings:** my Python edit briefly turned `docs/ARCHITECTURE.md` from CRLF to LF. I restored CRLF on every line
   right away; the diff shows only my own lines. `runtime.rs` and `settings.rs` (CRLF) were edited with the edit tool
   and kept CRLF. `leak.rs`, `README.md` and the `Cargo.toml` files are LF, like their unmodified neighbours
   (`cli/Cargo.toml`, `cli/src/main.rs`, `engines/src/lib.rs`).
5. **Escapes:** I checked `git diff -U0`. `\u{1f}` in `Form::hash`'s canon is intact, and no literal line breaks were
   introduced.

## Concerns

- **A failed read means a new secret, by the spec.** Any read failure, a transient `PlatformFailure` included, makes a
  new secret, overwrites the stored one, and drops `seen` and `declined` ("missing or cannot be read"). A flaky
  Credential Manager would quietly reset the counts and un-decline forgotten forms (they could be learned again).
  The owner may want only `NoEntry` and bad data to count as missing, and a store failure to skip counting for the
  session instead.
- **Ruling 22:** history entries keep the old habit ids after a rekey, as accepted.
- **Leak-scan wording:** the finding reads "the saved habits' secret key" (from `SavedKey`'s Display). A little
  awkward, but it is what the plan asked for.
- **Not run against the real store:** the runtime and leak-scan parts are compiled and linted only. No test exercises
  the real Credential Manager (there were none before either).

---

# Fix round 1

Status: DONE. No commits, nothing staged.

## Changes

1. **Three-way read of the secret** (`app/crates/engines/src/keys.rs`):
   - `kept_secret` now returns `Result<Option<Secret>, EngineError>`. `NoEntry`, `BadEncoding`, or stored text that
     `from_hex` rejects give `Ok(None)`, and a new secret is made. Any other keyring error gives
     `Err(Service("could not read the habits' secret: {keyring Display}"))`, which never holds store text (no
     `BadEncoding` bytes).
   - New pure helper `secret_from(read, make, save)` makes the decision:
     - a kept secret comes back as `Kept` and nothing is written;
     - missing or malformed: `make()` once, then `save(hex)` once, giving `Made { saved }`;
     - an unreadable store returns `Err` before `make` or `save` is called.
   - `habit_secret()` calls `secret_from(read_habit_secret(), new_secret, set_password)`. `kept_habit_secret()` now
     returns `Result<Option<Secret>, EngineError>`.
   - Doc comments are updated: `SecretRead::Made` says "what was kept is malformed", and `kept_secret`, `secret_from`
     and `habit_secret` say `Err` means an unreadable store or no random bytes.
2. **Runtime** (`app/src-tauri/src/runtime.rs`): when `key_habits` gets `Err` (an unreadable store, or no random
   bytes), it logs a warning ("…: no habit is counted this session") and does nothing else: no rekey, no save, no
   `set_password`. Its doc comment now says missing or malformed means a new secret, and an unreadable store is left
   alone for the session.
3. **`Habits::forget` without a secret** (`app/crates/core/src/habits.rs`): it removes the habit and pushes nothing to
   `declined`. The doc comment says why. The `rekey` doc says "missing or malformed".
4. **Leak scan** (`app/crates/cli/src/leak.rs`):
   - `gather` matches on `kept_habit_secret()`: `Ok(Some)` is added, `Ok(None)` (missing or malformed) is skipped, and
     `Err` bails with "could not read the saved keys: {e}", the same as the saved keys.
   - A new `Kind::HabitSecret` displays "the habits' secret", replacing `SavedKey("habits' secret")`.
   - `summary` now has "habits' secret yes/no" after "saved keys".
5. **Minor fixes:**
   - The module comment on habits.rs line 6 is rewrapped to 120 columns.
   - A new test checks RFC 4231 case 6 (131 × 0xaa key); it passed with no code change.
   - Two new lines in keys.rs were wrapped to stay under 120 columns.
6. **Docs:**
   - README (privacy in detail, Habits bullet): "When it is missing or malformed, CUE makes a new one … When Credential
     Manager cannot be read, the secret kept there is left alone and nothing is counted until the next start." The
     bullet was rewrapped.
   - ARCHITECTURE step 2: a missing or malformed secret goes through `rekey`; an unreadable store is left alone
     (`keys::secret_from` neither makes nor writes); without a secret nothing is counted or declined. The file is still
     CRLF on every line.
7. **`settings.rs` test** `habits_are_kept_in_their_section_and_changed_one_at_a_time`: it now sets the secret, as the
   runtime does, before forgetting. Without one, the new `forget` declines nothing and the `declined` assertion failed.
   It also checks that a set secret writes no `"habits"`.

## Tests (written first; they failed before the fix, as expected)

- **keys.rs**
  - `a_kept_secret_reads_back_a_missing_or_malformed_one_is_none_and_an_unreadable_one_an_error` replaces the old
    `a_kept_secret_reads_back_and_anything_else_is_none`. `PlatformFailure` and `NoStorageAccess` are now `Err(Service)`.
  - `only_a_missing_or_malformed_secret_is_made_and_written` (new). An unreadable read means 0 makes and 0 writes, and
    the result is `Err`. `NoEntry`, `BadEncoding` and `"zz"` each mean 1 make and 1 write of the made secret's hex. A
    kept secret means 0 makes and 0 writes. A failed save still returns `Made` with `saved: Err(Service)`.
- **habits.rs**
  - `without_a_secret_a_forgotten_habit_goes_and_nothing_is_declined` (new; failed before the fix).
  - `hmac_sha256_hashes_a_key_longer_than_the_block_first` (new, RFC 4231 case 6).
- **leak.rs:** `what_is_printed_holds_no_private_text` checks "saved keys 1, habits' secret no", then "habits'
  secret yes" with no hex shown, and that `Kind::HabitSecret` displays "the habits' secret".

## Commands and output (from `C:\Users\seymurglv\Desktop\cue-dev`)

- `cargo test --workspace`: every suite ok, 1163 passed and 0 failed in total (core 427, engines 206 plus 4 ignored,
  cli 265 plus 1 ignored, src-tauri 203 plus 1 ignored, 23, 39).
- `cargo clippy --workspace --all-targets -- -D warnings`: `Finished`, no warnings.

## Notes

- The spec (docs/specs/2026-11-04-habits.md, "The hash's secret") still says "missing or cannot be read". I left it
  unchanged; the coordinator's ruling governs the code and the README and ARCHITECTURE text. The owner may want the
  spec brought in line.
- `BadDataFormat` from keyring counts as "any other error" (unreadable), per the ruling. Its Display shows the platform
  error, not the data.
