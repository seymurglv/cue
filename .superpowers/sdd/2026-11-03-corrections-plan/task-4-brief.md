### Task 4: settings — learned words and the switch

**Files:**
- Modify: `app/crates/core/src/settings.rs`

**Interfaces:**
- Produces: `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct LearnedWord { pub word: String, pub heard: String }`;
  `DictionarySettings.learned: Vec<LearnedWord>` (newest first, not written while empty);
  `pub struct LearnSettings { enabled: Option<bool> }` with `pub fn enabled(&self) -> bool` (default true);
  `Settings.learn: LearnSettings`; `SettingChange::DictLearn(LearnedWord)` (JSON `{"kind":"dictLearn","value":{"word":…,"heard":…}}`)
  and `SettingChange::Learn(bool)` (JSON `{"kind":"learn","value":false}`); `DictRemove` also removes from `learned`.

- [ ] **Step 1: write the failing tests** next to `dictionary_words_are_added_first_and_removed`:

```rust
    fn learned(word: &str, heard: &str) -> LearnedWord {
        LearnedWord { word: word.into(), heard: heard.into() }
    }

    /// A word from a suggestion goes into both lists, first; it is refused like a typed one, and when what was heard
    /// cannot go into the request; removing the word takes it out of both.
    #[test]
    fn a_learned_word_joins_the_dictionary_with_how_it_was_heard() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::DictAdd("Soniox".into())));
        assert!(s.apply(SettingChange::DictLearn(learned(" Marisol ", " Марисоль "))));
        assert_eq!(s.dictionary.terms, ["Marisol", "Soniox"]);
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        assert!(!s.apply(SettingChange::DictLearn(learned("marisol", "марисоль"))), "already there");
        assert!(!s.apply(SettingChange::DictLearn(learned("GitHub", "a\nb"))), "heard on two lines");
        assert!(!s.apply(SettingChange::DictLearn(learned("", "пусто"))));
        assert!(s.apply(SettingChange::DictRemove("Marisol".into())));
        assert!(s.dictionary.learned.is_empty() && s.dictionary.terms == ["Soniox"]);
        let change: SettingChange =
            serde_json::from_value(serde_json::json!({ "kind": "dictLearn", "value": { "word": "GitHub", "heard": "гитхаб" } })).unwrap();
        assert_eq!(change, SettingChange::DictLearn(learned("GitHub", "гитхаб")));
    }

    #[test]
    fn learning_is_on_by_default_and_only_off_is_written() {
        let mut s = Settings::default();
        assert!(s.learn.enabled());
        assert!(s.apply(SettingChange::Learn(false)));
        assert!(!s.learn.enabled());
        assert_eq!(serde_json::to_value(&s).unwrap()["learn"], serde_json::json!({ "enabled": false }));
        assert!(!s.apply(SettingChange::Learn(false)), "no change");
        assert!(s.apply(SettingChange::Learn(true)));
        assert!(serde_json::to_value(&s).unwrap().get("learn").is_none(), "the default is not written");
        for odd in [r#"{"learn":5}"#, r#"{"learn":{"enabled":"no"}}"#, r#"{"learn":{"enabled":null}}"#] {
            let s: Settings = serde_json::from_str(odd).unwrap();
            assert!(s.learn.enabled(), "{odd}");
        }
    }

    #[test]
    fn learned_words_read_leniently_and_an_empty_list_is_not_written() {
        let s: Settings = serde_json::from_str(
            r#"{"dictionary":{"terms":["Marisol"],"learned":[{"word":"Marisol","heard":"Марисоль"},{"word":1},"x"]}}"#,
        )
        .unwrap();
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        let s: Settings = serde_json::from_str(r#"{"dictionary":{"terms":["A"],"learned":7}}"#).unwrap();
        assert!(s.dictionary.learned.is_empty());
        let mut s = Settings::default();
        s.dictionary.terms = vec!["A".into()];
        assert!(serde_json::to_value(&s).unwrap()["dictionary"].get("learned").is_none());
    }
```

Extend `debug_shows_no_device_name` (or the test that checks `DictAdd`'s `Debug`) with
`SettingChange::DictLearn(learned("Marisol", "Марисоль"))`: its `Debug` must contain neither word. Extend
`setting_change_reads_the_window_json` with the two new kinds in its existing shape.

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core settings`.

- [ ] **Step 3: implement**

Next to `DictionarySettings`:

```rust
/// A word added from a suggestion after a correction, with what recognition heard instead; refine is told both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearnedWord {
    pub word: String,
    pub heard: String,
}

fn learned_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<LearnedWord>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}
```

Add to `DictionarySettings` (after `terms`), and update its doc comment's last sentence to "…recognition does not take
them. `learned` holds the words added from a suggestion, newest first; each is in `terms` too.":

```rust
    #[serde(deserialize_with = "learned_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub learned: Vec<LearnedWord>,
```

Next to `AskSettings`:

```rust
/// "Learn from me": after a paste CUE reads the field to notice a correction (docs/specs/2026-11-02-corrections.md).
/// On by default; only `false` is written.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSettings {
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl LearnSettings {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
```

In `Settings`, after `dictionary`:

```rust
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "LearnSettings::is_default")]
    pub learn: LearnSettings,
```

and `learn: LearnSettings::default(),` in `impl Default for Settings`.

In `SettingChange`, after `DictRemove`:

```rust
    /// A word from a suggestion: first in the dictionary, with how it was heard; refused like `DictAdd`, and when
    /// `heard` could not go into the request.
    DictLearn(LearnedWord),
    /// "Learn from me" on or off.
    Learn(bool),
```

`Debug` arms:

```rust
            Self::DictLearn(l) => f
                .debug_struct("DictLearn")
                .field("chars", &l.word.chars().count())
                .field("heard_chars", &l.heard.chars().count())
                .finish(),
            Self::Learn(on) => f.debug_tuple("Learn").field(on).finish(),
```

In `apply`, move `DictAdd`'s refusal into a helper both arms use, and widen `DictRemove`:

```rust
            SettingChange::DictAdd(word) => self.dictionary.put_first(word.trim()),
            SettingChange::DictLearn(l) => {
                let (word, heard) = (l.word.trim(), l.heard.trim());
                if !refine::is_term(heard) || !self.dictionary.put_first(word) {
                    return false;
                }
                self.dictionary.learned.retain(|x| x.word.to_lowercase() != word.to_lowercase());
                self.dictionary.learned.insert(0, LearnedWord { word: word.to_owned(), heard: heard.to_owned() });
                true
            }
            SettingChange::DictRemove(word) => {
                let d = &mut self.dictionary;
                let (terms, learned) = (d.terms.len(), d.learned.len());
                d.terms.retain(|t| *t != word);
                d.learned.retain(|l| l.word != word);
                d.terms.len() != terms || d.learned.len() != learned
            }
            SettingChange::Learn(on) => set_if_changed(&mut self.learn.enabled, (!on).then_some(false)),
```

```rust
impl DictionarySettings {
    /// A word first in `terms`: refused when empty, not one line, too long, there in any case, or the list is full.
    fn put_first(&mut self, word: &str) -> bool {
        let taken = self.terms.iter().any(|t| t.trim().to_lowercase() == word.to_lowercase());
        if !refine::is_term(word) || taken || self.terms.len() >= refine::MAX_TERMS {
            return false;
        }
        self.terms.insert(0, word.to_owned());
        true
    }
}
```

(`impl DictionarySettings` already exists for `is_default`; add `put_first` there.)

- [ ] **Step 4: run** `cargo test -p cue-core` and clippy: green. `cargo test --workspace` will fail to compile in
  `app/crates/cli/src/leak.rs` or `ipc.rs` only if they build `DictionarySettings` with a struct literal; fix any such
  literal with `..Default::default()`.

- [ ] **Step 5: leak-scan sees learned words.** In `app/crates/cli/src/leak.rs` next to the loop over
  `settings.dictionary.terms` (line ~175), add the same loop over `settings.dictionary.learned` checking both `word`
  and `heard` with the same `Kind::DictionaryTerm(i)` (index continuing after the terms). Run its tests:
  `cargo test -p cue-cli` (check the crate's name in `app/crates/cli/Cargo.toml`).

---

