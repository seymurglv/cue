### Task 4: the habits block in the cleanup request (Clean and Rewrite only)

**Files:**
- Modify: `app/crates/core/src/refine.rs` (`RefineContext`, line 126), `app/crates/engines/src/claude.rs`
  (`system_blocks`, line 103; `warm_body`, line 137), `chat.rs` (line 46), `openrouter.rs` (line 214)
- Modify (a `RefineContext` literal each gets `..Default::default()`): `refine.rs:605,607,714,959`, `chat.rs:196`,
  `claude.rs:430`, `openrouter.rs:773`

**Interfaces:**
- Consumes: `habits::MAX_RULE_CHARS` (Task 2).
- Produces: `RefineContext.habits: Vec<String>` (the rules), `#[derive(Clone, PartialEq, Default)]` with a hand-written
  `Debug`; `pub fn with_habits(self, rules: Vec<String>) -> Self`; `pub fn habits_prompt(rules: &[String]) -> Option<String>`;
  `pub(crate) fn system_blocks(dict: &DictionarySettings, habits: &[String], job: Job, custom: Option<&str>) -> Value`.

- [ ] **Step 1: write the failing tests.** In `refine.rs`'s tests, after `no_usable_terms_means_no_block`:

```rust
    #[test]
    fn habits_prompt_lists_the_rules_one_per_line() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned(), " ".to_owned(), "two\nlines".to_owned()];
        assert_eq!(habits_prompt(&rules).unwrap(), format!("{HABITS_INTRO}\n\nВ Telegram — с маленькой буквы и без точки"));
        assert_eq!(habits_prompt(&[]), None);
        assert_eq!(habits_prompt(&["x".repeat(crate::habits::MAX_RULE_CHARS + 1)]), None);
    }

    #[test]
    fn the_context_carries_the_habits_and_prints_no_text() {
        let ctx = RefineContext::new("Telegram Desktop", Some("секрет"), 500).with_habits(vec!["без точки".into()]);
        assert_eq!(ctx.habits, ["без точки"]);
        let shown = format!("{ctx:?}");
        assert!(!shown.contains("секрет") && !shown.contains("точки") && !shown.contains("Telegram"), "{shown}");
    }
```

  In `claude.rs`'s tests (`dict` helper at line 425; add `use cue_core::refine::habits_prompt;`):

```rust
    /// Habits go after the dictionary, behind the cache mark, for a cleanup only; with none the body is the same.
    #[test]
    fn habits_go_after_the_dictionary_for_a_cleanup_only() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned()];
        let with = RefineContext { habits: rules.clone(), ..ctx() };
        let block = habits_prompt(&rules).unwrap();
        let v = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&["Soniox"]), Job::Rewrite);
        let texts: Vec<&str> = v["system"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!((texts.len(), texts[0], texts[2], texts[3]), (4, SYSTEM_PROMPT, block.as_str(), BOLD_PROMPT));
        assert!(v["system"][1]["cache_control"].is_object() && v["system"][2].get("cache_control").is_none(), "the cached prefix is the one without habits");
        assert_eq!(request_body("claude-haiku-5-5", "привет", &with, &dict(&[]))["system"][1]["text"], block);
        let to_en = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&[]), Job::Translate("en"));
        assert!(!to_en["system"].to_string().contains("без точки"), "a translation takes none");
        let none = RefineContext { habits: Vec::new(), ..ctx() };
        assert_eq!(request_body("claude-haiku-5-5", "привет", &none, &dict(&[])), request_body("claude-haiku-5-5", "привет", &ctx(), &dict(&[])));
    }
```

  In `chat.rs`'s tests, after `the_style_and_the_translation_blocks_are_claude_s`:

```rust
    #[test]
    fn habits_ride_in_the_system_message() {
        let c = find("openai").unwrap();
        let with = RefineContext { habits: vec!["No full stop".into()], ..ctx() };
        let v = request_body(c, "gpt-6-luna", RAW, &with, &Default::default(), Job::Rewrite, None);
        let block = cue_core::refine::habits_prompt(&with.habits).unwrap();
        assert_eq!(v["messages"][0]["content"], format!("{SYSTEM_PROMPT}\n\n{block}\n\n{BOLD_PROMPT}"));
    }
```

- [ ] **Step 2: run and see them fail** — `cargo test -p cue-core refine` and `cargo test -p cue-engines`.

- [ ] **Step 3: implement in `refine.rs`.** Next to `TERMS_INTRO`:

```rust
/// The start of the habits block ([`habits_prompt`]); the rules follow, one per line.
const HABITS_INTRO: &str = "The user's habits; follow them unless the speech clearly asks otherwise:";

/// The habits block (docs/specs/2026-11-04-habits.md): a system block after the dictionary's, one rule per line. A rule
/// that is empty, not one line or longer than `habits::MAX_RULE_CHARS` is skipped; `None` when none is left, and the
/// request is then exactly the one without habits.
pub fn habits_prompt(rules: &[String]) -> Option<String> {
    let usable: Vec<&str> = rules
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty() && r.chars().count() <= crate::habits::MAX_RULE_CHARS && crate::habits::one_line(r))
        .collect();
    (!usable.is_empty()).then(|| format!("{HABITS_INTRO}\n\n{}", usable.join("\n")))
}
```

  (make `habits::one_line` `pub(crate)`). `RefineContext` becomes:

```rust
/// The request context: the app, the text before the caret, and the rules of the user's habits for this dictation
/// (docs/specs/2026-11-04-habits.md; empty for a translation and "Ask").
#[derive(Clone, PartialEq, Default)]
pub struct RefineContext {
    pub app: String,
    pub before_cursor: Option<String>,
    pub habits: Vec<String>,
}

/// The app, the text and the rules are the user's: only their lengths.
impl std::fmt::Debug for RefineContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefineContext")
            .field("app_chars", &self.app.chars().count())
            .field("before_cursor_chars", &self.before_cursor.as_ref().map(|b| b.chars().count()))
            .field("habits", &self.habits.len())
            .finish()
    }
}
```

  In `RefineContext::new`: `Self { app: app.into(), before_cursor, habits: Vec::new() }`, and add:

```rust
    /// The context with the rules of the habits sent with this dictation.
    pub fn with_habits(mut self, rules: Vec<String>) -> Self {
        self.habits = rules;
        self
    }
```

- [ ] **Step 4: the engines.** In `claude.rs` import `habits_prompt`, and `system_blocks` (line 103) becomes:

```rust
/// The system blocks of a refine request: the prompt, the dictionary block ([`terms_prompt`]) with the cache mark on the
/// last of them; then, for a cleanup, the habits block ([`habits_prompt`]); then the job's own block: CUE's cleanup
/// ([`BOLD_PROMPT`]) or the user's instructions (`custom`), or the translation's. Everything after the mark changes
/// with the app or the job, so the cached prefix is always the same.
pub(crate) fn system_blocks(dict: &DictionarySettings, habits: &[String], job: Job, custom: Option<&str>) -> Value {
    let mut system = match terms_prompt(&dict.terms, &dict.learned) {
        None => vec![json!({ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } })],
        Some(block) => vec![
            json!({ "type": "text", "text": SYSTEM_PROMPT }),
            json!({ "type": "text", "text": block, "cache_control": { "type": "ephemeral" } }),
        ],
    };
    if matches!(job, Job::Clean | Job::Rewrite)
        && let Some(block) = habits_prompt(habits)
    {
        system.push(json!({ "type": "text", "text": block }));
    }
    match job {
        Job::Clean => {}
        Job::Rewrite => {
            let block = custom.map_or_else(|| BOLD_PROMPT.to_owned(), |text| format!("{CUSTOM_PREAMBLE}{text}"));
            system.push(json!({ "type": "text", "text": block }))
        }
        Job::Translate(to) => system.push(json!({ "type": "text", "text": translate::instruction(to) })),
    }
    Value::Array(system)
}
```

  Its three callers pass the context's rules: `claude.rs:75` `system_blocks(dict, &ctx.habits, job, custom)`,
  `chat.rs:46` and `openrouter.rs:214` `claude::system_blocks(dict, &ctx.habits, job, custom)`. `warm_body` (line
  137): `let ctx = RefineContext::default();`. Every test literal `RefineContext { app: …, before_cursor: … }` gets
  `..Default::default()`.

- [ ] **Step 5: run** `cargo test --workspace` and clippy: green. The request with no habits stays byte for byte the same
  (the existing request-body tests prove it).

---

