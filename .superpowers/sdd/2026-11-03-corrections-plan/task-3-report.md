# Task 3 report: offer state and Drop actions

Status: DONE. No commits made.

## What
- `app/crates/core/src/corrections.rs`: added `SuggestView` (Serialize, Debug prints length only), private `Stage`, `Offers` with offer/at_rest/view/dismissed/added/undone/done/dictation_started, exactly per brief; the 8 brief tests added.
- `app/crates/core/src/ui.rs`: `UiAction::{SuggestAdd, SuggestUndo, SuggestDismiss, SuggestDone}` after `TrayClose`; four `(name, variant)` entries in `every_variant_has_its_json_name`.
- `app/crates/core/src/flow.rs` (not in brief): `Flow::handle` matches `Input::Ui(UiAction)` exhaustively (no catch-all), so one arm was added: the four suggest actions are a no-op for the flow (E0004 otherwise).

## TDD
- RED: `cargo test -p cue-core` with tests only: E0433/E0425 `Offers` not found, E0422 `SuggestView` not found, E0599 no variant `SuggestAdd/Undo/Dismiss/Done`.
- After implementing the first time: E0004 non-exhaustive match in flow.rs; fixed with the no-op arm.
- GREEN: `cargo test -p cue-core` 374 passed; `cargo test --workspace` all ok (196+23+38+374+195+261 passed, 0 failed); `cargo clippy --workspace --all-targets -- -D warnings` clean.

## Self-review
Diff matches brief verbatim; no tool mentions; Debug of SuggestView leaks no word. app/src-tauri compiled (clippy checked it).

## Concerns
- The brief said the runtime match has a catch-all; the one in core/flow.rs does not, hence the extra arm (Task 7 may route these actions elsewhere before the flow).
- TypeScript side not touched/checked.
