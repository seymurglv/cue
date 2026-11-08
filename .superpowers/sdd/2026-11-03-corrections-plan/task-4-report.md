# Task 4 report: settings, learned words and the "Learn from me" switch

Status: DONE. No commits.

## What
In app/crates/core/src/settings.rs: `LearnedWord`, `learned_or_nothing`, `DictionarySettings.learned`, `LearnSettings`
(+ `enabled()`), `Settings.learn`, `SettingChange::{DictLearn, Learn}` with length-only Debug arms, `apply` arms
(DictAdd now uses `DictionarySettings::put_first`; DictRemove also clears `learned`). `set_if_changed` needed no
adaptation. In app/crates/cli/src/leak.rs: a loop over `settings.dictionary.learned` adds `word` and `heard` as
`Kind::DictionaryTerm(terms.len() + i)`. leak.rs has no test that builds settings from a file for this loop (the loop
is in the I/O path of the scan), so no test added there. No `DictionarySettings {` struct literals exist elsewhere.

## TDD
RED: tests added first (three new tests, plus extensions to `debug_shows_no_device_name` and
`setting_change_reads_the_window_json`); `cargo test -p cue-core settings` -> "could not compile `cue-core` (lib test)
due to 23 previous errors" (no field `learned`, unknown `LearnedWord`, `DictLearn`, etc.).
GREEN: same command -> `test result: ok. 64 passed; 0 failed`.

## Workspace
`cargo test --workspace`: all suites ok (195, 196, 23, 261, 377, 38 passed; 0 failed).
`cargo clippy --workspace --all-targets -- -D warnings`: clean.

## Files
- app/crates/core/src/settings.rs
- app/crates/cli/src/leak.rs

## Self-review
- Debug of DictLearn prints only character counts (tested for both words, Latin and Cyrillic).
- DictLearn refuses before mutating (heard checked first, then put_first), so a refusal leaves terms untouched.
- Re-learning an existing word is refused by put_first (already in terms), so the `retain` dedupe in `learned` only
  matters if `learned` and `terms` are out of sync (hand-edited file); it keeps `learned` free of duplicates then.
- `learned` entries are read leniently; a learned word missing from `terms` (hand edit) is not repaired.

## Concerns
- leak.rs: the summary "dictionary terms N" now counts learned words and heard texts as extra terms (word is in `terms`
  too, so it is scanned twice). Harmless for the scan; the count is slightly inflated.
- Nothing prunes `learned` when `terms` is edited by hand to drop a word; DictRemove handles the window path.
