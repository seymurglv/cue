# Cleanup model picks Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A measured short list of local cleanup models, S1-mini run as it was trained, a one-line caption under each
model with measured numbers, and "Recommended for you" chosen from the languages the user dictates in.

**Architecture:** The decisions stay in `cue-core` (catalog, the language rule, the word error rate); `cue-engines`
gets S1-mini's input format and the English-only gate in the local path; the runtime reads the history's said texts
for the language rule and keeps the decided value in `settings.json`; the main window's Cleanup rows show the caption.
Measuring runs through an example binary over a fixed dictation set kept in the repo.

**Tech Stack:** Rust (cue-core, cue-engines, src-tauri), llama-cpp-2 =0.1.161, TypeScript + vitest (app/ui).

**Spec:** `docs/specs/2026-11-05-cleanup-model-picks.md`

## Global Constraints

- English only in code, comments and docs; no mention of development tools.
- Commits happen only when the owner says so: each task's "Commit" step stages and waits; the executor commits on
  the owner's yes.
- Before any build that compiles cue-engines: `. scripts\dev-env.ps1` in PowerShell.
- The model is named "S1-mini" by "Superwhisper", capitalized exactly so, wherever CUE shows it; NOTICE credits it.
- S1-mini's system prompt is sent word for word: `You are a text normalizer for speech-to-text transcripts. The input
  begins with a control line specifying the styling, structure, and context settings; clean the transcript to match
  those settings and output only the cleaned text.`
- S1-mini's control line: `[Styling: semi-formal] [Structure: prose] [Context: general]`, then a newline, then the text.
- The recommendation never switches the model in use and never downloads one.
- The owner does not measure local models; the measuring is ours.
- The UI follows the family (merlin-skin): 11/12/13 scale, contrast ≥ 4.5:1 in both themes, amber for warnings only.

## Review Focus

- A Russian dictation with Latin brand names in it ("отправь в Slack до пятницы") with S1-mini in use: CUE reads its
  alphabet as Cyrillic and pastes it as said, never through S1-mini (Task 5 test
  `a_russian_dictation_with_latin_names_is_not_given_to_an_english_only_model`).
- S1-mini answering with an empty think block or echoing the control line: stripped before the checks, never pasted
  (Task 1 test `s1_answer_loses_a_think_block_and_an_echoed_control_line`).
- History empty, cleared or off: the stored decision stands; with none stored, the interface language decides (Task 3
  test `no_history_keeps_what_was_decided`).
- A user still running a model that leaves the list (Gemma 4 E2B, Qwen3 1.7B): it stays listed as downloaded and
  usable, with no numbers and no "Recommended" (Task 4 test `a_model_off_the_list_stays_usable_without_numbers`).
- A long English dictation with S1-mini: cleaned in pieces like any local model, with no text before the caret sent
  (Task 1 test `s1_cleans_a_long_dictation_in_pieces_without_context`).

---

### Task 1: S1-mini's own input format

**Files:**
- Modify: `app/crates/core/src/llm.rs` (an `is_s1_mini` helper)
- Modify: `app/crates/engines/src/local_llm.rs` (a raw-prompt method on `Generate`, its `LocalLlm` body)
- Modify: `app/crates/engines/src/local.rs` (the S1-mini path in `run`/`one`, `translate`, `rule`, `warm`)

**Interfaces:**
- Produces: `cue_core::llm::is_s1_mini(id: &str) -> bool` (true for ids starting `llm--superwhisper--s1-mini`);
  `Generate::complete(&self, prompt: &str, deadline: Instant, stop: &AtomicBool) -> Result<String, EngineError>` (a
  prompt already templated, no grammar; default body returns `Err(EngineError::Llm("no raw prompt".into()))`);
  `cue_engines::local::s1_prompt(text: &str) -> String`; `cue_engines::local::s1_answer(out: &str) -> String`.

- [ ] **Step 1: Write the failing tests** in `local.rs`'s tests (the `Echo` fake gains a recorded `complete`):
  - `s1_prompt_is_the_trained_chat_turn`: `s1_prompt("so um send it friday")` equals
    `"<|im_start|>system\n{S1_SYSTEM}<|im_end|>\n<|im_start|>user\n[Styling: semi-formal] [Structure: prose] [Context: general]\nso um send it friday<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"`.
  - `s1_answer_loses_a_think_block_and_an_echoed_control_line`: `s1_answer("<think>\n\n</think>\n\n[Styling: semi-formal] [Structure: prose] [Context: general]\nSend it Friday.")` == `"Send it Friday."`.
  - `s1_cleans_through_complete_not_generate`: with an S1 id, `refine` calls `complete` once, never `generate`; the
    result is `Refined` with the fake's text.
  - `s1_sends_no_dictionary_habits_or_context`: the prompt `complete` got contains neither a dictionary term nor
    `<before_cursor>`.
  - `s1_cleans_a_long_dictation_in_pieces_without_context`: a 140-word text makes two `complete` calls.
  - `s1_does_not_translate`: `translate` with an S1 id returns `RawFallback` without calling the model.
  - `s1_words_no_habit_rule`: `rule` with an S1 id returns `None`.
- [ ] **Step 2: Run them and see them fail.** `cargo test -p cue-engines -q local::` → compile errors on `s1_prompt`,
  `complete`.
- [ ] **Step 3: Implement.** `is_s1_mini` in llm.rs. `complete` on the trait; `LocalLlm::complete` tokenizes the prompt
  as is (no chat template, no grammar), reuses the evaluated common prefix as `generate` does, greedy, stops at
  `<|im_end|>` or EOS, same deadline and `stop`. In `local.rs`, when `is_s1_mini(llm.id())`: each piece goes through
  `s1_prompt` → `complete` → `s1_answer` → `claude::settle`; translate returns `claude::raw_fallback(raw,
  RawFallback, …, "S1-mini does not translate")`; `rule` returns `None`; `warm` warms nothing.
- [ ] **Step 4: Run.** `cargo test -p cue-engines -q local::` → all pass. Then the real model: with
  `CUE_TEST_GGUF=%LOCALAPPDATA%\CUE\models\llm--superwhisper--s1-mini-gguf--q4_k_m\s1-mini-q4_k_m.gguf`, the ignored
  `local_jobs_on_a_real_model` prints a cleaned English text, no think block.
- [ ] **Step 5: Stage** the three files; commit on the owner's yes.

### Task 2: The dictation set and the bench

**Files:**
- Create: `bench/cleanup/en.json`, `bench/cleanup/ru.json` (12 dictations each)
- Modify: `app/crates/core/src/refine.rs` (`word_error_rate`)
- Create: `app/crates/engines/examples/local_compare.rs`

**Interfaces:**
- Produces: `cue_core::refine::word_error_rate(hyp: &str, reference: &str) -> f64` (word-level Levenshtein over
  lowercased words with punctuation and dashes removed, divided by the reference's word count; 0.0 for two empties);
  the example's JSON line per call `{model, lang, id, words, ms, outcome, wer}` and a Markdown summary table.

- [ ] **Step 1: Write the failing tests** in refine.rs: `wer_ignores_case_and_punctuation` (`"Send it, Friday."` vs
  `"send it friday"` → 0.0); `wer_counts_a_dropped_word` (`"send friday"` vs `"send it friday"` → 1/3);
  `wer_of_empty_hypothesis_is_one` (`""` vs `"a b"` → 1.0).
- [ ] **Step 2: Run and see them fail.** `cargo test -p cue-core -q wer_` → not found.
- [ ] **Step 3: Implement `word_error_rate`.** Then write the set: per language, 12 entries `{id, kind, raw,
  reference}`, kinds `filler`, `self_correction`, `numbers`, `date_money`, `spoken_list`, `dictionary_name`,
  `message`, `question`, `short`, `two_sentences`, `long` ×2 (about 200 words). `raw` looks like Parakeet Ultra's
  output (its casing and punctuation, no invented recognition errors); `reference` is the hand cleanup. Nothing from
  the owner's history. Then the example: `cargo run -p cue-engines --release --example local_compare -- <gguf>… --set
  bench/cleanup --out target/local_compare.jsonl`; for each model it loads `LocalLlm`, warms it, runs
  `local::refine` (default settings, the dictionary term of `dictionary_name` set) over both files three times, and
  prints the table: model, lang, median ms at ~30 words, median ms for `long`, passed checks (Refined / total), mean
  WER.
- [ ] **Step 4: Run.** `cargo test -p cue-core -q wer_` passes; the example runs on S1-mini over `en.json` and prints
  its table.
- [ ] **Step 5: Stage**; commit on the owner's yes.

### Task 3: The language a user dictates in

**Files:**
- Create: `app/crates/core/src/speaks.rs` (add `pub mod speaks;` to `lib.rs`)
- Modify: `app/crates/core/src/settings.rs` (`refine.speaks`)
- Modify: `app/crates/core/src/history.rs` (`last_said`)

**Interfaces:**
- Produces: `pub enum Speaks { English, RussianAndEnglish }` (serde `"en"`, `"ru-en"`);
  `pub fn decide(stored: Option<Speaks>, recent: &[Alphabet], interface_ru: bool) -> (Speaks, Option<Speaks>)` —
  the mark to show and the value to store (`None`: store nothing new); `RefineSettings.speaks: Option<Speaks>`
  (skipped when `None`); `SettingChange::Speaks(Speaks)`; `history::last_said(dir: &Path, n: usize) -> Vec<String>` —
  the newest `n` dictations' and translations' `raw` (else `text`), newest first.
- Rules (from the spec): `recent` newest first, at most 20. Nothing stored and fewer than 5 → from the interface
  (ru → RussianAndEnglish), store nothing. Nothing stored and 5 or more → any Cyrillic in them → RussianAndEnglish,
  else English, and store it. Stored English and Cyrillic ≥ 5 of the last 20 (a quarter) → RussianAndEnglish, store;
  stored RussianAndEnglish and Cyrillic 0 of the last 20 with 20 present → English, store (the other language, Latin
  only, is then all twenty). Otherwise the stored value.

- [ ] **Step 1: Write the failing tests** in speaks.rs: `before_five_the_interface_decides`,
  `the_fifth_dictation_decides_once`, `one_russian_dictation_among_five_is_enough`,
  `a_quarter_of_the_last_twenty_changes_it`, `three_of_twenty_do_not`, `no_history_keeps_what_was_decided`; in
  settings.rs `speaks_reads_and_writes` (`"speaks":"ru-en"` round-trips, absent → `None`); in history.rs
  `last_said_takes_the_raw_text_newest_first_across_days`.
- [ ] **Step 2: Run and see them fail.** `cargo test -p cue-core -q speaks` → not found.
- [ ] **Step 3: Implement** `decide`, the setting and its `SettingChange`, `last_said` (reads day files newest first,
  skips "Ask" entries).
- [ ] **Step 4: Run.** `cargo test -p cue-core -q` → all pass.
- [ ] **Step 5: Stage**; commit on the owner's yes.

### Task 4 (checkpoint with the owner): measure, then the short list

**Files:**
- Create: `docs/specs/2026-11-05-cleanup-model-picks-results.md`
- Modify: `app/crates/core/src/model.rs` (`Manifest.seconds`, `ModelEntry.seconds`)
- Modify: `app/crates/core/src/llm.rs` (`catalog`, `pinned`, `english_only`, `recommended_for`)
- Modify: `app/src-tauri/src/runtime/llm.rs`, `app/src-tauri/src/runtime.rs` (the upgrade write and the first
  "This computer")

**Interfaces:**
- Consumes: Task 2's example, Task 3's `Speaks`.
- Produces: `Manifest.seconds: Option<f32>` and `ModelEntry.seconds: Option<f32>` (median seconds at ~30 words, serde
  default, skipped when `None`); `pinned(name, repo, commit, quant, file, size, sha256, license, languages: &[&str],
  seconds: f32)`; `llm::english_only(id: &str) -> bool` (a catalog model whose measured languages are only `en`);
  `llm::recommended_for(s: Speaks) -> Option<String>` (English → the first catalog model whose languages are only
  `en`, else the first; RussianAndEnglish → the first that has `ru`); `llm::LEGACY_DEFAULT: &str` = the Qwen3 4B
  Instruct 2507 id of 0.1.3.

- [ ] **Step 1: Measure.** Download (SHA-256 checked) Gemma 4 E4B Q4_K_M (unsloth, 4.98 GB), Qwen3 4B Instruct 2507
  Q4_K_M, QVikhr-3 4B Instruction Q4_K_M; S1-mini is present. Run Task 2's example on all four, both files (S1-mini
  on `en.json`; its Russian row is "not run: English only"). Read every output.
- [ ] **Step 2: Write the results file:** the table (model, language, seconds ~30 words, seconds long, passed, mean
  WER, size, verdict), the computer (i7-12700KF, 8 performance cores), and the verdict rule as applied: out if more
  than 2 of a language's 12 fail the checks or change meaning; among the rest the faster wins unless the slower's mean
  WER is lower by 0.05 or more.
- [ ] **Step 3: Report to the owner** the table and the proposed list (in the owner's language); wait for the yes.
- [ ] **Step 4: Write the failing tests** in llm.rs: `the_short_list_is_the_measured_one` (names and order from the
  results), `english_only_is_what_was_measured` (S1-mini true, the bilingual pick false, an id off the list false),
  `the_recommendation_follows_the_language`; in runtime/llm.rs
  `a_model_off_the_list_stays_usable_without_numbers`, `an_empty_choice_with_the_old_default_on_disk_is_written_down`,
  `this_computer_with_nothing_chosen_takes_the_recommended_and_keeps_it`.
- [ ] **Step 5: Run and see them fail;** then implement: the catalog from the results; `ModelsView.recommended` from
  `recommended_for(settings speaks)`; at start, an empty `refine.local` with `LEGACY_DEFAULT`'s folder present is
  written as `refine.local`; switching to "This computer" with `refine.local` empty saves `recommended_for` as the
  choice (it downloads as a choice does today).
- [ ] **Step 6: Run.** `cargo test --workspace -q` → all pass.
- [ ] **Step 7: Stage**; commit on the owner's yes.

### Task 5: The runtime keeps the language and the English-only gate

**Files:**
- Modify: `app/src-tauri/src/runtime.rs` (`speaks` at start and after each finished dictation)
- Modify: `app/crates/engines/src/local.rs` (the gate)

**Interfaces:**
- Consumes: `speaks::decide`, `history::last_said`, `llm::english_only`, `speech::alphabet_of`.

- [ ] **Step 1: Write the failing tests:** in local.rs
  `a_russian_dictation_with_latin_names_is_not_given_to_an_english_only_model` (S1 id, "отправь в Slack до пятницы
  отчёт" → `RawFallback`, the fake not called), `an_english_dictation_reaches_it`; in runtime.rs
  `the_mark_follows_the_fifth_dictation` (five finished dictations, one Cyrillic → `settings.refine.speaks ==
  Some(RussianAndEnglish)`, the view's `recommended` is the bilingual id).
- [ ] **Step 2: Run and see them fail.**
- [ ] **Step 3: Implement:** in `local::run`, when `english_only(llm.id())` and `alphabet_of(raw)` is not Latin, return
  `raw_fallback(raw, RawFallback, …, "the model is English only")`; in the runtime, after `Finished` of a dictation or
  translation and at start, `decide(settings.refine.speaks, alphabets of last_said(20), ui is ru)`; a value to store
  goes through `SettingChange::Speaks`; the Cleanup view is sent again when the mark moves.
- [ ] **Step 4: Run.** `cargo test --workspace -q` → all pass.
- [ ] **Step 5: Stage**; commit on the owner's yes.

### Task 6 (checkpoint with the owner): the caption, by the family's process

**Files:**
- Create: a variants artifact (CUE's real Cleanup page on a mock core, `merlin-skin/tools/tauri-mock.md`)
- Modify: `app/ui/src/window/models.ts` (`llmSource.about`), `app/ui/src/window/text.ts`, `app/ui/src/window/api.ts`
  (`seconds` on the entry), `app/ui/src/window/settings.ts` (the S1-mini note)
- Test: `app/ui/test/window-cleanup.test.ts`

**Interfaces:**
- Consumes: `ModelEntry.seconds`, `ModelEntry.languages`, `ModelsView.recommended`.

- [ ] **Step 1: Variants.** Two or three captions on the real Cleanup page, light and dark, Windows: (A) words
  "Recommended for you · Russian and English · about 3 s · 2.5 GB"; (B) the same with a language glyph instead of the
  words; (C) the speed as a short bar beside the size. Measure each with `audit.js`. Show the owner; wait for the pick.
- [ ] **Step 2: Write the failing tests** for the pick, e.g. `a_measured_model_says_its_languages_speed_and_size`,
  `the_recommended_one_says_for_you`, `a_found_model_shows_only_what_hugging_face_says`,
  `s1_mini_is_named_as_its_license_asks`, `the_s1_note_says_it_cleans_english_and_does_not_translate` (English and
  Russian copy).
- [ ] **Step 3: Run and see them fail;** implement; `npx vitest run` and `npx tsc --noEmit` in `app` pass;
  `audit.js` on the built page in both themes finds no text under 4.5:1.
- [ ] **Step 4: Stage**; commit on the owner's yes.

### Task 7: Words, the release check

**Files:**
- Modify: `README.md` (the local list with its measured lines, how the recommendation is decided, what S1-mini does
  not do), `NOTICE` (S1-mini by Superwhisper, Apache-2.0 with its naming term; Gemma 4; QVikhr if listed)

- [ ] **Step 1: Write** the README and NOTICE sections from the results file.
- [ ] **Step 2: Verify.** `cargo test --workspace -q`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `npx vitest run`, `npx tsc --noEmit`: all green. `git grep -n "S1 mini\|s1-mini" -- README.md NOTICE app/ui/src`
  shows no other spelling of the displayed name.
- [ ] **Step 3: Stage**; commit on the owner's yes.
