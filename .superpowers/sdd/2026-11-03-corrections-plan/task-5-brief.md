### Task 5: refine hears how learned words sound

**Files:**
- Modify: `app/crates/core/src/refine.rs`, `app/crates/engines/src/claude.rs`, `chat.rs`, `openrouter.rs`,
  `app/src-tauri/src/runtime.rs` (lines ~1049 and ~1142), `app/crates/engines/examples/*.rs` that call these.

**Interfaces:**
- Consumes: `settings::{DictionarySettings, LearnedWord}` (Task 4).
- Produces: `pub fn terms_prompt(terms: &[String], learned: &[LearnedWord]) -> Option<String>`; on the refine path every
  `terms: &[String]` parameter becomes `dict: &DictionarySettings`.

- [ ] **Step 1: write the failing test** in refine.rs's dictionary tests:

```rust
    #[test]
    fn learned_words_say_how_they_sound() {
        let learned = [LearnedWord { word: "Marisol".into(), heard: "Марисоль".into() }];
        let block = terms_prompt(&terms(&["Soniox", "Marisol"]), &learned).unwrap();
        assert_eq!(block, format!("{TERMS_INTRO}\n\nSoniox\nMarisol — sounds like «Марисоль»"));
        let orphan = [LearnedWord { word: "Gone".into(), heard: "гон".into() }];
        assert_eq!(terms_prompt(&terms(&["Soniox"]), &orphan).unwrap(), format!("{TERMS_INTRO}\n\nSoniox"), "only words in the list");
    }
```

and change the existing calls `terms_prompt(&…)` in refine.rs and openrouter.rs tests to `terms_prompt(&…, &[])`.

- [ ] **Step 2: run and see it fail** — `cargo test -p cue-core refine`.

- [ ] **Step 3: implement in refine.rs**

```rust
/// The dictionary block: a second system block after the unchanged `SYSTEM_PROMPT`, one term per line; a word added
/// from a suggestion also says how recognition heard it. `None` when no usable term is left: the request is then
/// exactly the one without a dictionary.
pub fn terms_prompt(terms: &[String], learned: &[LearnedWord]) -> Option<String> {
    let usable = usable_terms(terms);
    let line = |t: &str| match learned.iter().find(|l| l.word.trim() == t) {
        Some(l) => format!("{t} — sounds like «{}»", l.heard.trim()),
        None => t.to_owned(),
    };
    (!usable.is_empty()).then(|| format!("{TERMS_INTRO}\n\n{}", usable.iter().map(|t| line(t)).collect::<Vec<_>>().join("\n")))
}
```

with `use crate::settings::LearnedWord;` (refine.rs and settings.rs already refer to each other; that is fine in one
crate).

- [ ] **Step 4: carry the dictionary down the refine path.** Change each parameter `terms: &[String]` in these
  functions to `dict: &DictionarySettings` (import `cue_core::settings::DictionarySettings`), and inside each body use
  `&dict.terms` where `terms` was used, except `system_blocks`, which calls `terms_prompt(&dict.terms, &dict.learned)`:
  - `engines/src/claude.rs`: `request_body` (57), `request_body_for` (64), `request_body_styled` (69),
    `system_blocks` (103), `warm_body` (136), `decide` (209), `decide_translation` (224), the functions at 254, 270,
    309, `settle` (352), `warm` (396), `warm_at` (401);
  - `engines/src/chat.rs`: 38, `refine` (112), 122, 135;
  - `engines/src/openrouter.rs`: `request_body` (188), `request_body_for` (194), 206, `refine` (279), 289, 321.

  `refine::sanity_ok` and `translate::translation_ok` keep `terms: &[String]`; their callers pass `&dict.terms`. In the
  engines' tests build the argument with `&DictionarySettings { terms: …, ..Default::default() }` (a small helper
  `fn dict(t: &[&str]) -> DictionarySettings` in each test module keeps the calls short). In `runtime.rs` replace
  `self.settings.dictionary.terms.clone()` with `self.settings.dictionary.clone()` at both places and pass `&dict` on.
  The compiler lists any call left; follow it until `cargo build --workspace --all-targets` is clean.

- [ ] **Step 5: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`: green.
  The request with no learned words must stay byte for byte the same (the existing request-body tests prove it).

---

