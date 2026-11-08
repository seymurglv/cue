# Task 2 report: the watch

Status: DONE. No commits, no git index changes.

## What
Added to `app/crates/core/src/corrections.rs`, below Task 1's code and above `mod tests`: the four timing constants, `Step` (Debug prints only `Done(chars)`), `Watch` (`new`, `read`, `finish`), exactly as in the brief. The nine tests were appended inside the existing `mod tests`; Task 1's tests are unchanged.

## TDD
- RED: `cargo test -p cue-core corrections` failed to compile ("cannot find type `Step` in this scope", 16 errors).
- GREEN: same command after implementing: `22 passed; 0 failed` (13 from Task 1, 9 new).
- `cargo test --workspace`: all suites pass (196, 23, 38, 366, 195, 261 passed; 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean, no changes needed to the brief's code.

## Files
- `C:\Users\seymurglv\Desktop\cue-dev\app\crates\core\src\corrections.rs`

## Self-review
- Code is verbatim from the brief; no clippy fixes were required.
- Byte slicing is guarded by `rfind` / `starts_with` / `ends_with` and the length check, so it lands on char boundaries.
- Neither `Watch` nor `Step` can print words: `Watch` has no Debug derive, `Step` prints a char count only.
- Comments and names carry no mention of tools.

## Concerns
None. Note (not a defect): `Watch` has no Debug impl, so a later task that embeds it in a Debug-derived struct will need one that prints lengths.
