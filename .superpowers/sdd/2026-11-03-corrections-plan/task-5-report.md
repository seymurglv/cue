# Task 5 report: refine hears how learned words sound

Status: DONE. No commits, no git index changes.

## What
- `refine::terms_prompt(terms, learned)`: a term that matches a `LearnedWord.word` (trimmed) is written as
  `Word — sounds like «heard»`; learned words not in the list are ignored. With no learned words the block is
  byte-identical to before.
- The refine path now carries `&DictionarySettings` instead of `&[String]`.

## TDD
The new test `learned_words_say_how_they_sound` was written together with the implementation in the same edit, so no
separate RED run was captured (honest note). GREEN: `cargo test -p cue-core refine` -> 57 passed, 0 failed.

## Signatures changed (`terms: &[String]` -> `dict: &DictionarySettings`)
- core/refine.rs: `terms_prompt(terms: &[String], learned: &[LearnedWord])` (`sanity_ok`, `usable_terms` unchanged).
- engines/claude.rs: `request_body`, `request_body_for`, `request_body_styled`, `system_blocks`, `warm_body`,
  `decide`, `decide_translation`, `refine`, `translate`, `refine_at`, `settle`, `warm`, `warm_at`.
  `decide*` pass `&dict.terms` to `sanity_ok` / `translation_ok`.
- engines/chat.rs: `request_body`, `refine`, `translate`, `refine_job`.
- engines/openrouter.rs: `request_body`, `request_body_for`, `request_body_styled`, `refine`, `translate`, `refine_at`.
- src-tauri/runtime.rs: both places clone `self.settings.dictionary` (`dict`) and pass `&dict`.
- Callers outside the brief's list: examples `refine_compare.rs` (builds a `DictionarySettings` from `--terms`),
  `refine_probe.rs`, `warm_probe.rs`; `bench/src/main.rs` (`&s.dictionary`).
- Doc comments mentioning `terms` as the parameter updated (claude.rs, openrouter.rs).

## Tests
engines test modules got `fn dict(&[&str]) -> DictionarySettings`; `&[]` arguments became `&Default::default()`.
Expected values of existing request-body tests untouched (they pass, so the no-learned request is byte-identical).

## Verification (from the repo root)
- `cargo build --workspace --all-targets`: clean.
- `cargo test --workspace`: all green (core 378, engines 195, app 261, others pass; 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.

## Self-review / concerns
- Learned lookup is linear per term (at most 200 terms), fine.
- Files are CRLF in the working tree; edits preserved that.
- No new Debug or logging was added.

## Fix round 1
- refine.rs: `terms_prompt` and the test restored to `\n\n` / `\n` escapes in one-line literals.
- `terms_prompt` adds the « » suffix only when `is_term(heard.trim())`; otherwise the line is the term alone. Test extended
  with heard "a\nb", "" and "  " (all give the bare term).
- claude.rs: new test `a_learned_word_reaches_the_request_with_how_it_sounds` (second system block contains
  "Marisol — sounds like «Марисоль»").
- openrouter.rs: test helper `dict` moved below the `use` lines; doc wording now "the user's dictionary (`dict`)".
- Commands: `cargo test -p cue-core refine` -> 57 passed; `cargo test -p cue-engines` -> 196 passed, 0 failed;
  `cargo clippy --workspace --all-targets -- -D warnings` -> clean. No commits.

## Fix round 2
The string literals in refine.rs (`terms_prompt` and the tests) still had raw line breaks. A script outside the repo
(Temp\fixnl.py, run with `python -I`) turned every line break inside an open string literal in those two regions into
the two characters backslash and n; CRLF endings kept.
Proof, `grep -nF 'TERMS_INTRO}\n\n' app/crates/core/src/refine.rs`:
- 122: `(!usable.is_empty()).then(|| format!("{TERMS_INTRO}\n\n{}", usable.iter().map(|t| line(t)).collect::<Vec<_>>().join("\n")))`
- 1284, 1326, 1328, 1332: the test lines (`...Soniox\nMarisol — sounds like «Марисоль»`, orphan `...\n\nSoniox`, loop `...\n\nMarisol`)
- 1330: `for heard in ["a\nb", "", "  "] {`
Other files: added diff lines with an odd number of quotes in engines, src-tauri and bench: none; the claude.rs
`"Claude\nHaiku"` literal is correct.
`cargo test -p cue-core refine` -> 57 passed; clippy for the workspace -> clean. No commits.
