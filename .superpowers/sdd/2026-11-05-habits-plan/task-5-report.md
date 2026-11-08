# Task 5 report: habit rule request (cue-engines::habit)

Status: DONE, no commits.

## What
- New `app/crates/engines/src/habit.rs`: `rule`, `anthropic_body`, `chat_body`, as in the brief, plus its 3 tests.
- `lib.rs`: `pub mod habit;` after `pub mod error;`. `claude.rs`: `thinking_for`, `effort_for` now `pub(crate)`. `chat.rs`: `post` `pub(crate)`. `openrouter.rs`: `TITLE` `pub(crate)`.
- The rule request writes nothing to usage.csv.

## TDD
- RED: tests only in habit.rs, `cargo test -p cue-engines habit`: compile error "cannot find function `rule`" (28 errors).
- GREEN: implementation added, same command: 5 passed (3 new habit tests).

## Checks
- `cargo test -p cue-engines`: 201 passed, 0 failed, 4 ignored.
- `cargo test --workspace`: all ok (core 417, engines 201, app 198, others ok).
- `cargo build --workspace --all-targets`: ok. `cargo clippy --workspace --all-targets -- -D warnings`: clean.

## Deviations
None: every name in the plan (RuleAsk, RuleText, rule_messages, read_rule, MessageResp, Block, stream, reason, Connection fields) matched the real code; the code compiled unchanged.

## Self-review
habit.rs has no strings broken by raw line breaks. The log lines carry only the reason (HTTP status, timeout, network, unusable answer), never text or the key; a test asserts it.

## Concerns
- `RULE_WAIT_MS` is not used here (the caller passes `timeout`); Task 6 should pass `Duration::from_millis(habits::RULE_WAIT_MS)`.
- Anthropic path sends a plain non-streaming request with the timeout on the request; a slow body read is bounded by the same reqwest timeout.
