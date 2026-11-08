## Global constraints

- Files are in English and say nothing about the tools the code was written with.
- No commits unless the owner asks; each task ends with the checks green, not with a commit.
- The field's text never reaches disk or the log: the log gets "correction found / none", "habit forms counted",
  "habit learned" with the kinds and numbers only. Every new `Debug` that could hold text prints its length (see
  `SettingChange`'s hand-written `Debug`, `LearnedWord`'s): `Form`, `Found`, `Correction`, `Seen`, `Habit`, `Draft`,
  `RuleAsk`, `RuleText`, `HabitMark`, `RefineContext`.
- The habit's example (`before`/`after`, at most 40 characters each) is the only text of a correction ever written to
  disk (decided 04.11.2026). A replacement's words, kept in its form, are always words of that example (Task 1 makes
  sure of it). The rule and its short name are written from it.
- `learn.enabled` off: no field is read, nothing is counted, learned or applied; no habit goes into a request; habits
  waiting for their rule or their card are dropped.
- Texts in English and Russian; the card and the section's buttons have accessible names.
- Motion follows the family: a new row grows in (`height .45s var(--e-sm)`) with `rowIn .6s var(--e-spring)` and the
  green highlight of a new dictionary chip; a removed one folds away the same way; switching a habit off mutes its row in
  place (color, never opacity: `--muted` keeps contrast ≥ 4.5:1); labels change with `CUE.setLabel` (shared letters
  stay); the card grows out of the Drop like the other tray cards; reduced motion is respected (rows appear and go at
  once).
- Checks, from `C:\Users\seymurglv\Desktop\cue-dev`: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`; from `app`: `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`.

- NO git commits, no git add/stash/checkout/reset on the real index, no push. Leave changes uncommitted.
- Never rebuild a file from git HEAD; if an edit damages a file, stop and report.
- Escapes: an edit tool may turn a written backslash-n into a real line break inside a string literal; check `git diff -U0` and fix with a small `python -I` script outside the repo if needed.
- The plan's code was written without compiling: names, anchors and signatures may be off — follow the real code and report each deviation.
- Spec (binding): docs/specs/2026-11-04-habits.md.
