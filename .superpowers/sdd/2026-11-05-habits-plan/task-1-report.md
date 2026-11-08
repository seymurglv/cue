# Task 1 report (relayed from the implementer's reply; it could not write this file)
Status: DONE, no commits. cargo test -p cue-core: 392 passed (13 new habits tests). cargo test --workspace and cargo build green. Existing corrections tests unchanged and passing. One warning: method `rank` is never used (expected until Task 2). Clippy deferred per ruling. No deviations from the plan's code.
Files: app/crates/core/src/habits.rs (new: Step 4 code + Step 1 tests verbatim; Debug on Form, Found, Correction prints lengths/counts only); app/crates/core/src/lib.rs (`pub mod habits;`); app/crates/core/src/corrections.rs (`use std::ops::Range;`; MAX_RUN, clean, key, looks_like_name now pub(crate); the alignment loop of `changes` replaced by `words`, `Run`, `sentence_start`, `runs`, which `changes` calls).
TDD: RED `cargo test -p cue-core habits` → "could not compile cue-core (lib test) due to 22 previous errors"; GREEN → "13 passed; 0 failed; 379 filtered out"; full core 392 passed.
Self-review: no raw line breaks in strings; nothing staged.

## Fix round 1
The Digits branch of `replaced` in habits.rs now also requires that `to` holds no uppercase letter and no letter of the other alphabet than the text around it (`corrections::other_alphabet`, made `pub(crate)`). A term edit such as "гпт четыре" to "GPT-4" gives no form. New test `digits_for_number_words_but_not_for_a_term` covers it and "пять минут" to "5 минут"; the existing "12 %" test still passes. `cargo test -p cue-core`: 393 passed, 0 failed. No commits.
