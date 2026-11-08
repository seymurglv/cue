# Task 2 report: counting, learning, widening, declining; templates; the applied check

Status: DONE. No commits, no git add/stash/checkout.

## What
- `app/crates/core/src/habits.rs`: constants, `Seen`, `Habit`, `Habits` (lenient lists), `Draft`, `RuleAsk`, `RuleText`,
  `Counted`, `count` / `adopt` / `forget` / `forget_all` / `switch` / `for_app`, `template`, `unscoped`, `rule_messages`,
  `read_rule`, `marks`. Code is the brief's verbatim; the imports gained `Deserializer` and `crate::history::{self, HabitMark}`.
  All 14 new tests from the brief added to `mod tests`.
- `app/crates/core/src/history.rs`: `HabitMark` (hand-written `Debug`), `Entry.habits` (not written while empty, read as
  empty when missing), `Debug` prints the count, the doc sentence, the new test `an_entry_keeps_the_habits_that_shaped_it`.
- `Entry` literals given `habits: Vec::new()`: history.rs tests helper, `app/src-tauri/src/commands.rs`, `ipc.rs` (3),
  `runtime.rs` (3), `app/crates/cli/src/leak.rs`.

## TDD
- RED: tests added first, then `cargo test -p cue-core habits` -> `error[E0433]: cannot find type Habits in this scope` ...
  `could not compile cue-core (lib test) due to 46 previous errors`.
- GREEN: after the implementation `cargo test -p cue-core` -> `test result: ok. 408 passed; 0 failed`
  (the hard-coded id `h9be169a0a7be8fdd` matched first time).
- `cargo build --workspace --all-targets` listed the 8 outside `Entry` literals (as the brief predicted); fixed.
- `cargo test --workspace`: all suites ok (198, 23, 38, 408, 196, 265 passed; 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (the `rank` dead-code warning is gone).

## Deviations
- None in logic. Line numbers of the literals differed slightly from the brief; I followed the compiler.
- `runtime.rs` and `leak.rs` use CRLF line endings; inserted lines were given CRLF to keep each file consistent.

## Self-review
- `git diff -U0` shows no string broken by a raw line break; the only backslash-n sequences are in raw strings / escapes
  inside the tests and compile and pass. `habits.rs` is untracked (Task 1), checked by compile + tests.
- No `Debug` here prints text: `Seen`, `Habit`, `Draft`, `RuleAsk`, `RuleText`, `HabitMark` print lengths; the
  `the_lists_read_leniently_and_print_no_text` test checks it.

## Concerns
- Not directly covered (the brief only said "if used"): `Correction::example`'s replacement fallback and final cut with
  several forms (Task 1's gap); `count` only exercises it with first letter + end mark. `shows_in` and `has_run` are now
  covered by `the_applied_check_per_kind`.
- `git diff` warns that CRLF will be normalized to LF for `leak.rs` and `runtime.rs` (pre-existing git config behavior).

## Fix round 1
Changes (all in `app/crates/core/src/habits.rs`):
1. `count` collects the habits that grew and the ones that widened, then raises each `n` once after the loop: by the
   widening's count when it widened, else by 1. `Counted.grown` is true only for a habit that grew without widening.
2. After the forms are reached, the example is computed and every replacement whose `from`/`to` is not a word run of
   the stored example is dropped from the draft (repeated until stable; the example is recomputed for what is left).
   Dropped forms keep their `seen` counts (only the drafted forms' counts are removed), so they draft next time.
3. `unscoped` strips the app name only when a separating mark (colon, dash, comma) follows it, so a longer name such as
   "Telegram Desktop app" is not cut; otherwise the template is used. (Interpretation: a space alone does not count as a
   word boundary here, since "In Telegram Desktop app — …" with app "Telegram" must not be cut.)
4. Widening removes the `seen` counts of all the habit's hashes in every app.

Tests added: a two-form habit goes 3 -> 4 on a further correction, and a widening adds only its count (3 -> 4 -> 7,
`grown` false); widening clears the counts in every app; a far replacement stays out of the draft and keeps its count;
`example` with several forms (the replacement fallback, and the final cut keeping the end); `unscoped` at a word boundary.
Commands: `cargo test -p cue-core` -> 413 passed, 0 failed; `cargo test --workspace` -> all ok;
`cargo clippy --workspace --all-targets -- -D warnings` -> clean. No commits.
