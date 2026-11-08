# Task 1 report: word diff and candidate rules

Status: DONE. No commits (per constraints).

## Implemented
- New `app/crates/core/src/corrections.rs`: brief's code verbatim (Replacement with length-only Debug, MAX_TOKENS, changes() LCS diff, looks_like_name / other_alphabet, candidates()) plus the 13 tests.
- `app/crates/core/src/lib.rs`: `pub mod corrections;` after `connections`.

## TDD
- RED: `cargo test -p cue-core corrections` -> E0425 `MAX_TOKENS`, E0422 `Replacement`, E0425 `candidates` not found (4 errors).
- GREEN: same command -> `13 passed; 0 failed`.
- `cargo test --workspace`: all suites pass (core 357 passed, others 196/23/38/195/261, 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, no changes needed to the brief's code.

## Files changed
- app/crates/core/src/corrections.rs (new)
- app/crates/core/src/lib.rs

## Self-review
Matches the brief; no tool mentions in code/comments. Pre-existing uncommitted spec/plan files in docs/ were not touched.

## Concerns
None.

## Fix round 1
- Change: `very_long_texts_are_not_diffed` now uses pasted = "слово " x (MAX_TOKENS+1) + "марисоль" vs corrected = same prefix + "Marisol" (expects no candidates), plus a positive twin with a 10-word prefix expecting exactly [("марисоль","Marisol")]. No production change.
- Mutation check: with the guard temporarily disabled the test FAILED (12 passed; 1 failed); guard restored.
- `cargo test -p cue-core corrections`: 13 passed; 0 failed. `cargo clippy -p cue-core --all-targets -- -D warnings`: clean.
