# Task 4 report: the habits block in the cleanup request

Status: done, nothing committed.

## What
- `refine.rs`: `RefineContext.habits: Vec<String>` (derive Clone, PartialEq, Default; hand-written Debug printing lengths only), `with_habits`, `HABITS_INTRO`, `pub fn habits_prompt`.
- `habits.rs`: `one_line` is now `pub(crate)`.
- `claude.rs`: `system_blocks(dict, habits, job, custom)` adds the habits block (no cache mark) after the dictionary block for Clean and Rewrite only; `request_body_styled` passes `&ctx.habits`; `warm_body` uses `RefineContext::default()`; `habits_prompt` imported.
- `chat.rs`, `openrouter.rs`: callers pass `&ctx.habits`.
- Test `RefineContext` literals got `..Default::default()` (refine.rs x4, chat.rs, claude.rs, openrouter.rs).

## TDD
- RED: `cargo test -p cue-core refine` failed to compile (habits_prompt, HABITS_INTRO, with_habits, `habits` field missing).
- GREEN: `cargo test -p cue-core refine` 59 passed; `cargo test -p cue-engines` 198 passed (new: habits_prompt_lists..., the_context_carries..., habits_go_after_the_dictionary..., habits_ride_in_the_system_message).
- `cargo test --workspace` all ok; `cargo build --workspace --all-targets` ok; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Existing request-body tests unchanged and passing (no habits = same bytes).

## Deviations
- `refine.rs` line numbers differed (struct at 126 matched); the `RefineContext` literal at the old line 714 was in an assert_eq, patched the same way. No name deviations.
- `claude.rs` test module also needed `use cue_core::refine::habits_prompt;` (as briefed).

## Self-review
My editing script wrote real line breaks inside the `habits_prompt` format string and two tests; found via `git diff -U0`, fixed to `\n` escapes, tests re-run green. Files keep their CRLF endings.

## Concerns
None. Callers that build the real context (src-tauri) do not yet call `with_habits`; that is a later task.
