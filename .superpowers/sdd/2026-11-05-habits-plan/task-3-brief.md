### Task 3: settings — the habits section and its three changes

**Files:**
- Modify: `app/crates/core/src/settings.rs`, `app/crates/cli/src/leak.rs`

**Interfaces:**
- Consumes: `habits::Habits` (Task 2).
- Produces: `Settings.habits: Habits` (JSON `habits`, not written while empty, a section that is not an object reads
  as empty); `SettingChange::HabitOn { id: String, on: bool }` (JSON `{"kind":"habitOn","value":{"id":…,"on":…}}`),
  `SettingChange::HabitForget(String)` (`{"kind":"habitForget","value":"h…"}`), `SettingChange::HabitsForget`
  (`{"kind":"habitsForget"}`).

- [ ] **Step 1: write the failing tests** next to `learning_is_on_by_default_and_only_off_is_written`:

```rust
    /// The habits keep their own section, written only when there is something in it; the window changes one at a time.
    #[test]
    fn habits_are_kept_in_their_section_and_changed_one_at_a_time() {
        let mut s = Settings::default();
        assert!(serde_json::to_value(&s).unwrap().get("habits").is_none(), "nothing written while there are none");
        let d = crate::habits::Draft { forms: vec![crate::habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let id = s.habits.adopt(d, None, "en").id;
        assert_eq!(serde_json::to_value(&s).unwrap()["habits"]["list"][0]["after"], "a — b");
        assert!(s.apply(SettingChange::HabitOn { id: id.clone(), on: false }));
        assert!(!s.apply(SettingChange::HabitOn { id: id.clone(), on: false }), "no change");
        assert!(!s.habits.list[0].on);
        assert!(s.apply(SettingChange::HabitForget(id.clone())));
        assert_eq!(s.habits.declined, [crate::habits::Form::Dash.hash()]);
        assert!(!s.apply(SettingChange::HabitForget(id)), "already gone");
        assert!(s.apply(SettingChange::HabitsForget));
        assert!(!s.apply(SettingChange::HabitsForget), "nothing left");
        let read = |json: serde_json::Value| serde_json::from_value::<SettingChange>(json).unwrap();
        assert_eq!(read(serde_json::json!({ "kind": "habitOn", "value": { "id": "h1", "on": true } })), SettingChange::HabitOn { id: "h1".into(), on: true });
        assert_eq!(read(serde_json::json!({ "kind": "habitForget", "value": "h1" })), SettingChange::HabitForget("h1".into()));
        assert_eq!(read(serde_json::json!({ "kind": "habitsForget" })), SettingChange::HabitsForget);
        let s: Settings = serde_json::from_str(r#"{"habits":"nonsense"}"#).unwrap();
        assert!(s.habits.is_default());
    }

    /// A habit's example and rule are the user's words: neither the settings nor a change show them in a log.
    #[test]
    fn debug_shows_no_habit_text() {
        let mut s = Settings::default();
        let d = crate::habits::Draft {
            forms: vec![crate::habits::Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }],
            app: "Почта".into(),
            before: "Здравствуйте, Анна!".into(),
            after: "Добрый день, Анна!".into(),
            n: 3,
        };
        let id = s.habits.adopt(d, None, "ru").id;
        let shown = format!("{s:?} {:?} {:?}", SettingChange::HabitOn { id: id.clone(), on: false }, SettingChange::HabitForget(id));
        for word in ["Анна", "добрый", "здравствуйте", "Почта"] {
            assert!(!shown.contains(word), "{word} in a log");
        }
        assert_eq!(format!("{:?}", SettingChange::HabitsForget), "HabitsForget");
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core settings`.

- [ ] **Step 3: implement.** `use crate::habits::Habits;` at the top (line 5 block). In `Settings` (line 859), after
  `learn`:

```rust
    /// Habits (docs/specs/2026-11-04-habits.md): counts, habits and declined forms. Not written while empty.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "Habits::is_default")]
    pub habits: Habits,
```

  and `habits: Habits::default(),` in `impl Default for Settings` (line 940). In `SettingChange` (line 675), after
  `Learn`:

```rust
    /// A habit on or off (docs/specs/2026-11-04-habits.md).
    HabitOn { id: String, on: bool },
    /// A habit forgotten from the main window or its card's "Undo": its forms are never learned again.
    HabitForget(String),
    /// "Forget all": every habit, the counts and the declined forms.
    HabitsForget,
```

  `Debug` arms (line 715; ids are hashes, never text):

```rust
            Self::HabitOn { id, on } => f.debug_struct("HabitOn").field("id", id).field("on", on).finish(),
            Self::HabitForget(id) => f.debug_tuple("HabitForget").field(id).finish(),
            Self::HabitsForget => f.write_str("HabitsForget"),
```

  `apply` arms (line 754), and the sentence "`HabitOn` switches a habit, `HabitForget` forgets one and declines its
  forms, `HabitsForget` forgets them all; each is refused when it changes nothing." at the end of its doc comment:

```rust
            SettingChange::HabitOn { id, on } => self.habits.switch(&id, on),
            SettingChange::HabitForget(id) => self.habits.forget(&id),
            SettingChange::HabitsForget => self.habits.forget_all(),
```

  Add ", the habits" to the module's first sentence after "the user's dictionary".

- [ ] **Step 4: the leak scan looks for the examples.** In `app/crates/cli/src/leak.rs`: a kind `HabitExample(usize)`
  after `DictionaryTerm(usize)` (line 67), its `Display` arm `Kind::HabitExample(i) => write!(f, "habit example {}", i + 1),`,
  in `summary` (line 136) "habit examples {}" after "dictionary terms {}" with
  `self.count(|k| matches!(k, Kind::HabitExample(_))),`, and in `gather` after the learned words (line 183):

```rust
        // A habit's example is the user's text; its rule and short name are written from it.
        for (i, habit) in settings.habits.list.iter().enumerate() {
            for text in [&habit.before, &habit.after, &habit.rule, &habit.short] {
                private.add(Kind::HabitExample(i), text, MIN_NAME_CHARS);
            }
        }
```

  Update the module comment's list ("the dictionary terms, the habits' examples and rules and the microphone names (4
  or longer)") and any test that pins `summary()`'s text.

- [ ] **Step 5: run** `cargo test --workspace` and clippy: green.

---


## Coordinator rulings for this task (binding, override the text above)
- The leak scan checks only a habit's `before` and `after` (the example), never `rule` or `short`: template words are literals in cue.exe and would always be flagged.
