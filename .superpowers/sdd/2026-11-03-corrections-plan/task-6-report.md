# Task 6 report: the field watcher

## What
Added `app/crates/platform-win/src/fieldwatch.rs` (pure `FieldReader`/`Pace`/`run`; on Windows `win` module with the UI Automation reader `UiaField`, `open()`, `Clock`, `spawn` on thread `cue-field`, and an `#[ignore]` live test). Added `pub mod fieldwatch;` to lib.rs (alphabetical, not behind cfg). No commits.

## TDD
- RED: with `run` stubbed (`todo!()`), `cargo test -p cue-platform-win fieldwatch` -> 0 passed; 3 failed (not yet implemented).
- GREEN: `3 passed; 0 failed; 1 ignored` (the live test, not run).
- `cargo test --workspace`: all result lines ok, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.

## API deviations
None. The brief's call shapes matched focus.rs (GetFocusedElement, CurrentIsPassword, CurrentProcessId, GetCurrentPatternAs, DocumentRange/GetText, CurrentValue). Only formatting changes (rustfmt-style import order, a multi-line `value` expression).

## Self-review
No `\n` string literals; field text is never logged (only the thread-start error is). Drop order of UiaField: patterns, UIA, COM.

## Concerns
- A temp file `body.txt` I created in the crate dir was moved (not deleted, a hook blocks rm) to the session scratchpad; the repo has no stray file.
- The live UIA reader is untested by hand.
