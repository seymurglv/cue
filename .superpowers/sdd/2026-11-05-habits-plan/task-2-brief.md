### Task 2: counting, learning, widening, declining; templates; the applied check

**Files:**
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/history.rs`
- Modify (an `Entry` literal each gets `habits: Vec::new()`): `app/crates/core/src/history.rs:450`,
  `app/src-tauri/src/commands.rs:849`, `app/src-tauri/src/ipc.rs:618,649,703`, `app/src-tauri/src/runtime.rs:2143,2159,2516`,
  `app/crates/cli/src/leak.rs:1124` (the compiler lists any other)

**Interfaces:**
- Consumes: Task 1.
- Produces in `history`: `#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct HabitMark { pub id: String, pub short: String }`
  (hand-written `Debug`: id and `short_chars`); `Entry.habits: Vec<HabitMark>` (JSON `habits`, not written while empty,
  read as empty when missing).
- Produces in `habits`: `pub const LEARN_AT: u32 = 3`, `SEEN_DAYS: u32 = 30`, `MAX_SEEN: usize = 300`,
  `MAX_HABITS: usize = 50`, `MAX_RULE_CHARS: usize = 80`, `MAX_SHORT_CHARS: usize = 32`, `RULE_WAIT_MS: u64 = 5_000`;
  `pub struct Seen { pub app, pub form, pub n: u32, pub last }`;
  `pub struct Habit { pub id, pub forms: Vec<Form>, pub rule, pub short, pub before, pub after, pub app: Option<String>, pub n: u32, pub on: bool }`
  with `hashes()`, `applied(&str)`;
  `#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)] pub struct Habits { pub seen: Vec<Seen>, pub list: Vec<Habit>, pub declined: Vec<String> }`
  with `is_default`, `for_app(&str) -> impl Iterator<Item = &Habit>`, `count(&mut self, app, &Correction, today, lang) -> Counted`,
  `adopt(&mut self, Draft, Option<RuleText>, lang) -> Habit`, `forget(&mut self, id) -> bool`, `forget_all(&mut self) -> bool`,
  `switch(&mut self, id, on) -> bool`;
  `pub struct Draft { pub forms, pub app, pub before, pub after, pub n }` with `ask(&self) -> RuleAsk`;
  `pub struct Counted { pub new: Option<Draft>, pub widened: Vec<String>, pub grown: bool }`;
  `#[derive(Serialize)] pub struct RuleAsk { pub kind: Vec<&'static str>, pub before, pub after, pub app }`;
  `#[derive(Deserialize)] pub struct RuleText { pub rule: String, pub short: String }`;
  `pub fn template(&[Form], Option<&str>, lang: &str) -> RuleText`, `pub fn unscoped(rule, app, &[Form], lang) -> String`,
  `pub fn rule_messages(&RuleAsk, lang) -> (String, String)`, `pub fn read_rule(&str) -> Option<RuleText>`,
  `pub fn marks(&[Habit], text: &str) -> Vec<HabitMark>`. `lang` is the interface language's code, `"ru"` or anything
  else for English.

- [ ] **Step 1: write the failing tests** (add to `habits.rs`'s `mod tests`):

```rust
    const TG: &str = "Telegram Desktop";

    fn learned(h: &mut Habits, pasted: &str, corrected: &str, lang: &str) -> Habit {
        let c = correction(pasted, corrected);
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", lang).new;
        }
        h.adopt(draft.expect("the third time"), None, lang)
    }

    #[test]
    fn three_times_in_one_app_make_one_habit_of_the_forms_together() {
        let mut h = Habits::default();
        let c = correction("Скинь файл.", "скинь файл");
        assert_eq!(h.count(TG, &c, "2026-11-02", "ru"), Counted::default());
        assert_eq!(h.seen.len(), 2);
        assert!(h.seen.iter().all(|s| s.n == 1 && s.app == TG && s.last == "2026-11-02"));
        assert_eq!(h.count(TG, &c, "2026-11-03", "ru").new, None);
        let draft = h.count(TG, &c, "2026-11-04", "ru").new.expect("the third time");
        assert_eq!(draft.forms, [Form::FirstLetter { lower: true }, STOP]);
        assert_eq!((draft.app.as_str(), draft.before.as_str(), draft.after.as_str(), draft.n), (TG, "Скинь файл.", "скинь файл", 3));
        assert!(h.seen.is_empty(), "the counts went into the habit");
        let habit = h.adopt(draft, None, "ru");
        assert_eq!(habit.rule, "В Telegram Desktop: с маленькой буквы и без точки в конце");
        assert_eq!(habit.short, "без точки");
        assert_eq!(habit.id, "h9be169a0a7be8fdd");
        assert_eq!((habit.app.as_deref(), habit.n, habit.on), (Some(TG), 3, true));
        assert_eq!(h.list, [habit]);
    }

    #[test]
    fn other_apps_count_apart_and_an_unknown_app_not_at_all() {
        let mut h = Habits::default();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        h.count(TG, &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        h.count(TG, &c, "2026-11-04", "en");
        assert_eq!(h.seen.iter().map(|s| (s.app.as_str(), s.n)).collect::<Vec<_>>(), [(TG, 2), ("Word", 1)]);
        assert_eq!(h.count("", &c, "2026-11-04", "en"), Counted::default());
        assert_eq!(h.seen.len(), 2);
    }

    #[test]
    fn counts_older_than_thirty_days_go_and_at_most_three_hundred_stay() {
        let (dash, digits) = (Form::Dash.hash(), Form::Digits.hash());
        let mut h = Habits::default();
        h.seen.push(Seen { app: TG.into(), form: dash.clone(), n: 2, last: "2026-10-04".into() });
        h.seen.push(Seen { app: TG.into(), form: digits.clone(), n: 2, last: "2026-10-05".into() });
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        let left: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(left, [(dash.as_str(), 1), (digits.as_str(), 2)], "the dash's old count went; the digits' stays");
        for i in 0..400 {
            h.seen.push(Seen { app: format!("app{i}"), form: dash.clone(), n: 1, last: "2026-11-04".into() });
        }
        h.count("Word", &correction("рост двенадцать процентов", "рост 12 %"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), MAX_SEEN);
        assert_eq!((h.seen[0].app.as_str(), h.seen[0].form.as_str()), ("Word", digits.as_str()), "the newest first");
    }

    #[test]
    fn a_habit_widens_when_its_form_reaches_three_in_another_app() {
        let mut h = Habits::default();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", "ru").new;
        }
        let rule = RuleText { rule: "В Telegram Desktop — тире вместо дефиса".into(), short: "тире".into() };
        let id = h.adopt(draft.unwrap(), Some(rule), "ru").id;
        h.count("Word", &c, "2026-11-04", "ru");
        h.count("Word", &c, "2026-11-04", "ru");
        let counted = h.count("Word", &c, "2026-11-04", "ru");
        assert_eq!((counted.new, counted.widened), (None, vec![id]));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Тире вместо дефиса"));
        assert!(h.seen.is_empty());
        assert!(h.count("Notepad", &c, "2026-11-04", "ru").grown, "everywhere now: a further correction counts on it");
        assert_eq!(h.list[0].n, 7);
    }

    #[test]
    fn a_rule_that_does_not_name_the_app_is_written_anew_from_the_template() {
        assert_eq!(unscoped("In Telegram Desktop: no full stop at the end", TG, &[STOP], "en"), "No full stop at the end");
        assert_eq!(unscoped("В Telegram Desktop — с маленькой буквы", TG, &[STOP], "ru"), "С маленькой буквы");
        assert_eq!(unscoped("Без точки, как вы пишете", TG, &[STOP], "ru"), "Без точки в конце");
    }

    #[test]
    fn a_forgotten_habit_is_never_learned_again_until_all_is_forgotten() {
        let mut h = Habits::default();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert!(h.forget(&id));
        assert!(!h.forget(&id), "already gone");
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(), STOP.hash()]);
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..5 {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty(), "a declined form is not even counted");
        assert!(h.forget_all());
        assert_eq!(h, Habits::default());
        assert!(!h.forget_all(), "nothing left");
    }

    #[test]
    fn a_habit_switched_off_stays_but_goes_into_no_request() {
        let mut h = Habits::default();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert_eq!(h.for_app(TG).count(), 1);
        assert_eq!(h.for_app("Word").count(), 0, "another app's");
        assert_eq!(h.for_app("").count(), 0, "an unknown app gets only the habits of everywhere");
        assert!(h.switch(&id, false));
        assert!(!h.switch(&id, false), "no change");
        assert_eq!(h.for_app(TG).count(), 0);
        assert!(!h.switch("nope", true));
    }

    #[test]
    fn at_most_fifty_habits_the_newest_first() {
        let mut h = Habits::default();
        for i in 0..MAX_HABITS + 2 {
            let forms = vec![Form::Replacement { from: format!("a{i}"), to: format!("b{i}") }];
            h.adopt(Draft { forms, app: TG.into(), before: format!("a{i}"), after: format!("b{i}"), n: 3 }, None, "en");
        }
        assert_eq!(h.list.len(), MAX_HABITS);
        assert_eq!(h.list[0].after, format!("b{}", MAX_HABITS + 1));
    }

    #[test]
    fn templates_say_the_rule_and_its_short_name() {
        let t = |forms: &[Form], app: Option<&str>, lang: &str| {
            let r = template(forms, app, lang);
            (r.rule, r.short)
        };
        assert_eq!(t(&[STOP], Some("Telegram"), "en"), ("In Telegram: no full stop at the end".into(), "no full stop".into()));
        assert_eq!(t(&[STOP], Some("Telegram"), "ru"), ("В Telegram: без точки в конце".into(), "без точки".into()));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(t(&[hello], None, "ru"), ("Писать «добрый день» вместо «здравствуйте»".into(), "добрый день".into()));
        assert_eq!(t(&[Form::Digits], None, "en").0, "Numbers in digits");
        assert_eq!(t(&[Form::Dash], Some("Docs"), "ru").0, "В Docs: тире вместо дефиса");
        assert_eq!(
            t(&[Form::FirstLetter { lower: true }, STOP], Some("Telegram"), "en").0,
            "In Telegram: lower case at the start and no full stop at the end",
        );
        let long = Form::Replacement { from: "я".repeat(40), to: "ю".repeat(40) };
        let r = template(&[long], Some("Telegram"), "ru");
        assert!(r.rule.chars().count() <= MAX_RULE_CHARS && r.short.chars().count() <= MAX_SHORT_CHARS);
    }

    #[test]
    fn the_rule_request_holds_the_kind_the_example_and_the_app_only() {
        let forms = vec![Form::FirstLetter { lower: true }, STOP];
        let d = Draft { forms, app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 };
        let (system, user) = rule_messages(&d.ask(), "ru");
        let v: serde_json::Value = serde_json::from_str(&user).unwrap();
        let expected = serde_json::json!({ "kind": ["firstLetter", "endMark"], "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram" });
        assert_eq!(v, expected);
        assert!(system.contains("Write both in Russian.") && !system.contains("Скинь") && !system.contains("скинь"));
        assert!(rule_messages(&d.ask(), "en").0.contains("Write both in English."));
    }

    #[test]
    fn only_a_usable_answer_is_taken() {
        let ok = read_rule(r#"{"rule":"В Telegram — с маленькой буквы и без точки.","short":"без точки"}"#).unwrap();
        assert_eq!((ok.rule.as_str(), ok.short.as_str()), ("В Telegram — с маленькой буквы и без точки", "без точки"));
        assert!(read_rule("```json\n{\"rule\":\"No full stop\",\"short\":\"no stop\"}\n```").is_some(), "in a code fence");
        let long = format!(r#"{{"rule":"{}","short":"x"}}"#, "a".repeat(MAX_RULE_CHARS + 1));
        for bad in [
            r#"{"rule":"","short":"x"}"#,
            long.as_str(),
            r#"{"rule":"two\nlines","short":"x"}"#,
            r#"{"rule":"fine","short":"one two three four"}"#,
            "not json",
            r#"{"text":"fine"}"#,
        ] {
            assert!(read_rule(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_applied_check_per_kind() {
        assert!(STOP.shows_in("скинь файл") && !STOP.shows_in("Скинь файл."));
        assert!(Form::EndMark { mark: '.', removed: false }.shows_in("Готово."));
        let lower = Form::FirstLetter { lower: true };
        assert!(lower.shows_in("«скинь» файл") && !lower.shows_in("Скинь файл"));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert!(hello.shows_in("Добрый день, Анна!"));
        assert!(!hello.shows_in("Здравствуйте, Анна!") && !hello.shows_in("Добрый день! Здравствуйте, Анна"));
        assert!(Form::Digits.shows_in("рост 12 %") && !Form::Digits.shows_in("рост двенадцать процентов"));
        assert!(Form::Dash.shows_in("клиенты — в основном") && !Form::Dash.shows_in("клиенты - в основном"));
    }

    #[test]
    fn only_habits_whose_every_form_shows_are_marked() {
        let habit = |id: &str, forms: Vec<Form>| Habit {
            id: id.into(),
            forms,
            rule: "r".into(),
            short: format!("s{id}"),
            before: String::new(),
            after: String::new(),
            app: None,
            n: 3,
            on: true,
        };
        let sent = [habit("1", vec![Form::FirstLetter { lower: true }, STOP]), habit("2", vec![Form::Digits])];
        assert_eq!(marks(&sent, "скинь файл"), [HabitMark { id: "1".into(), short: "s1".into() }]);
        assert!(marks(&sent, "Скинь файл").is_empty());
    }

    #[test]
    fn the_lists_read_leniently_and_print_no_text() {
        let json = serde_json::json!({
            "seen": [{ "app": "Telegram Desktop", "form": "94128fbe83ca55c4", "n": 2, "last": "2026-11-04" }, { "app": 1 }],
            "list": [
                { "id": "h1", "forms": [{ "kind": "endMark", "mark": ".", "removed": true }], "rule": "В Telegram: без точки в конце",
                  "short": "без точки", "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram Desktop", "n": 3 },
                { "id": "h2", "forms": [{ "kind": "later" }] }
            ],
            "declined": ["85657e6730abe435", 7]
        });
        let h: Habits = serde_json::from_value(json).unwrap();
        assert_eq!((h.seen.len(), h.list.len(), h.declined.len()), (1, 1, 1));
        assert!(h.list[0].on, "on unless written off");
        let shown = format!("{h:?}");
        for word in ["Скинь", "скинь", "Telegram", "точки"] {
            assert!(!shown.contains(word), "{word} in {shown}");
        }
        let h: Habits = serde_json::from_value(serde_json::json!({ "seen": 5, "list": "x" })).unwrap();
        assert_eq!(h, Habits::default());
        assert_eq!(serde_json::to_value(Habits::default()).unwrap(), serde_json::json!({}));
    }
```

  In `history.rs`'s tests, next to `entry_debug_has_no_text` (line 1018):

```rust
    /// The habits that shaped an entry: written only when there are some, read as none from an earlier line, and
    /// shown in a log only by their count.
    #[test]
    fn an_entry_keeps_the_habits_that_shaped_it() {
        let mut e = entry("2026-11-04T10:00:00.000", Mode::Dictation, "скинь файл");
        let line = serde_json::to_value(&e).unwrap();
        assert!(line.get("habits").is_none());
        e.habits = vec![HabitMark { id: "h1".into(), short: "без точки".into() }];
        let line = serde_json::to_string(&e).unwrap();
        assert!(line.contains(r#""habits":[{"id":"h1","short":"без точки"}]"#), "{line}");
        assert_eq!(serde_json::from_str::<Entry>(&line).unwrap(), e);
        assert!(!format!("{e:?}").contains("точки"));
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core`; expected: `Habits`, `HabitMark` and the rest
  are not defined.

- [ ] **Step 3: `history.rs`.** Above `pub struct Entry` (line 38):

```rust
/// A habit whose effect showed in an entry's text (docs/specs/2026-11-04-habits.md): its id, and its short name as it
/// was then, so the entry's chip stays when the habit is forgotten.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitMark {
    pub id: String,
    pub short: String,
}

/// A short name may hold the words of a replacement: only its length.
impl std::fmt::Debug for HabitMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HabitMark").field("id", &self.id).field("short_chars", &self.short.chars().count()).finish()
    }
}
```

  In `Entry`, after `raw`, and the sentence "`habits` are the habits whose effect showed in a dictation's text."
  at the end of its doc comment:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub habits: Vec<HabitMark>,
```

  In its `Debug`, after `raw_chars`: `.field("habits", &self.habits.len())`. Add `habits: Vec::new()` to every `Entry`
  literal the compiler names (listed under Files).

- [ ] **Step 4: the state, the templates and the check** (`habits.rs`, after `impl Correction`; the imports gain
  `Deserializer` and `crate::history::{self, HabitMark}`):

```rust
/// A form becomes a habit when it was corrected this many times in one app.
pub const LEARN_AT: u32 = 3;
/// A count not seen for this many days is dropped.
pub const SEEN_DAYS: u32 = 30;
/// The most counts kept; the least recently seen go first.
pub const MAX_SEEN: usize = 300;
/// The most habits kept; the oldest go first.
pub const MAX_HABITS: usize = 50;
/// The longest rule, and short name, a habit takes.
pub const MAX_RULE_CHARS: usize = 80;
pub const MAX_SHORT_CHARS: usize = 32;
/// How long the rule request may take before the template writes the rule.
pub const RULE_WAIT_MS: u64 = 5_000;

/// How many times a form was corrected in an app, and the day it last was: no text (`form` is its hash).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub app: String,
    pub form: String,
    pub n: u32,
    pub last: String,
}

/// The app's name is the user's: only its length.
impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seen")
            .field("app_chars", &self.app.chars().count())
            .field("form", &self.form)
            .field("n", &self.n)
            .field("last", &self.last)
            .finish()
    }
}

fn yes() -> bool {
    true
}

/// A habit: its forms (each kind with its detail), the rule the cleanup follows and its short name, the example, the
/// app it holds in (`None`: everywhere), how many times it was corrected, and whether it is on.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Habit {
    pub id: String,
    pub forms: Vec<Form>,
    pub rule: String,
    pub short: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub app: Option<String>,
    pub n: u32,
    #[serde(default = "yes")]
    pub on: bool,
}

impl std::fmt::Debug for Habit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Habit")
            .field("id", &self.id)
            .field("forms", &self.forms)
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("everywhere", &self.app.is_none())
            .field("n", &self.n)
            .field("on", &self.on)
            .finish()
    }
}

impl Habit {
    pub fn hashes(&self) -> Vec<String> {
        self.forms.iter().map(Form::hash).collect()
    }

    /// Its effect shows in `text`: every one of its forms does.
    pub fn applied(&self, text: &str) -> bool {
        !self.forms.is_empty() && self.forms.iter().all(|f| f.shows_in(text))
    }
}

/// A list of items as settings.json holds them: anything but a list reads as none, and an item that does not read (a
/// later version's, a typo) is skipped.
fn items_or_nothing<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}

/// `habits` in settings.json: the counts (`seen`, hashes only), the habits (`list`, the newest first) and the forms
/// never to learn again (`declined`, hashes only). Each list is written only when it is not empty.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Habits {
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<Seen>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Habit>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub declined: Vec<String>,
}

/// A habit being made: its forms, the app and the example, and how many times they were corrected. Its rule comes
/// from the cleanup's connection or the template ([`Habits::adopt`]).
#[derive(Clone, PartialEq)]
pub struct Draft {
    pub forms: Vec<Form>,
    pub app: String,
    pub before: String,
    pub after: String,
    pub n: u32,
}

impl std::fmt::Debug for Draft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draft")
            .field("forms", &self.forms)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("n", &self.n)
            .finish()
    }
}

/// What the rule request carries, and nothing else: the kinds, the example and the app's name.
#[derive(Clone, PartialEq, Serialize)]
pub struct RuleAsk {
    pub kind: Vec<&'static str>,
    pub before: String,
    pub after: String,
    pub app: String,
}

impl std::fmt::Debug for RuleAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleAsk")
            .field("kind", &self.kind)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .finish()
    }
}

impl Draft {
    pub fn ask(&self) -> RuleAsk {
        RuleAsk { kind: self.forms.iter().map(Form::kind).collect(), before: self.before.clone(), after: self.after.clone(), app: self.app.clone() }
    }
}

/// A habit's rule, a sentence of at most `MAX_RULE_CHARS`, and its short name, one to three words.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct RuleText {
    pub rule: String,
    pub short: String,
}

impl std::fmt::Debug for RuleText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleText")
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .finish()
    }
}

/// What counting one correction changed.
#[derive(Debug, Default, PartialEq)]
pub struct Counted {
    /// Forms that reached `LEARN_AT` in this app together: the habit to make.
    pub new: Option<Draft>,
    /// Habits of another app whose form reached `LEARN_AT` here: they hold everywhere now.
    pub widened: Vec<String>,
    /// A habit's form was corrected again: its count grew.
    pub grown: bool,
}

impl Habits {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The habits that go into a dictation's cleanup request in `app`: on, and for that app or everywhere (an unknown
    /// app, `""`, gets the ones of everywhere).
    pub fn for_app<'a>(&'a self, app: &'a str) -> impl Iterator<Item = &'a Habit> + 'a {
        self.list.iter().filter(move |h| h.on && h.app.as_deref().is_none_or(|a| !app.is_empty() && a == app))
    }

    /// One correction of a dictation in `app` (the history's name for it, `today` its local date): each form counts
    /// once. A declined form is not counted. A form already a habit here or everywhere adds to that habit's count; one
    /// that reaches `LEARN_AT` and is a habit of another app widens it (its rule loses its app, its counts add up); the
    /// others that reach it together become one draft. The counts that went into a habit go; counts older than
    /// `SEEN_DAYS` go, and past `MAX_SEEN` the least recently seen.
    pub fn count(&mut self, app: &str, c: &Correction, today: &str, lang: &str) -> Counted {
        let mut out = Counted::default();
        if app.is_empty() {
            return out;
        }
        self.prune(today);
        let mut reached: Vec<&Found> = Vec::new();
        for f in &c.found {
            let hash = f.form.hash();
            if self.declined.contains(&hash) {
                continue;
            }
            if let Some(h) = self.list.iter_mut().find(|h| h.hashes().contains(&hash) && h.app.as_deref().is_none_or(|a| a == app)) {
                h.n = h.n.saturating_add(1);
                out.grown = true;
                continue;
            }
            let n = self.bump(app, &hash, today);
            if n < LEARN_AT {
                continue;
            }
            if let Some(h) = self.list.iter_mut().find(|h| h.hashes().contains(&hash)) {
                let was = h.app.take().unwrap_or_default();
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
                h.n = h.n.saturating_add(n);
                self.seen.retain(|s| s.form != hash);
                out.widened.push(h.id.clone());
                continue;
            }
            reached.push(f);
        }
        if !reached.is_empty() {
            let gone: Vec<String> = reached.iter().map(|f| f.form.hash()).collect();
            self.seen.retain(|s| s.app != app || !gone.contains(&s.form));
            let mut forms: Vec<Form> = reached.iter().map(|f| f.form.clone()).collect();
            forms.sort_by_key(Form::rank);
            let (before, after) = c.example(&reached);
            out.new = Some(Draft { forms, app: app.to_owned(), before, after, n: LEARN_AT });
        }
        out
    }

    /// The form's count in `app` grows by one and goes first (the most recently seen); the new count.
    fn bump(&mut self, app: &str, form: &str, today: &str) -> u32 {
        let n = match self.seen.iter().position(|s| s.app == app && s.form == form) {
            Some(i) => self.seen.remove(i).n.saturating_add(1),
            None => 1,
        };
        self.seen.insert(0, Seen { app: app.to_owned(), form: form.to_owned(), n, last: today.to_owned() });
        self.seen.truncate(MAX_SEEN);
        n
    }

    /// Counts last seen before the `SEEN_DAYS` before `today` go (none when `today` is not a date).
    fn prune(&mut self, today: &str) {
        if let Some(first_kept) = history::date_minus(today, SEEN_DAYS) {
            self.seen.retain(|s| s.last.as_str() >= first_kept.as_str());
        }
    }

    /// The draft's habit goes first, with the rule the cleanup wrote or else the template's; past `MAX_HABITS` the
    /// oldest go. Its id is the hash of its forms (a form is in one habit at most: a forgotten one is declined).
    pub fn adopt(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Habit {
        let RuleText { rule, short } = rule.unwrap_or_else(|| template(&draft.forms, Some(&draft.app), lang));
        let hashes: Vec<String> = draft.forms.iter().map(Form::hash).collect();
        let id = format!("h{:016x}", fnv1a(hashes.join(",").as_bytes()));
        let habit = Habit { id, forms: draft.forms, rule, short, before: draft.before, after: draft.after, app: Some(draft.app), n: draft.n, on: true };
        self.list.retain(|h| h.id != habit.id);
        self.list.insert(0, habit.clone());
        self.list.truncate(MAX_HABITS);
        habit
    }

    /// The habit goes, and its forms are declined: never learned again. False when there is no such habit.
    pub fn forget(&mut self, id: &str) -> bool {
        let Some(i) = self.list.iter().position(|h| h.id == id) else { return false };
        for hash in self.list.remove(i).hashes() {
            if !self.declined.contains(&hash) {
                self.declined.push(hash);
            }
        }
        true
    }

    /// Every habit goes, with the counts and the declined forms. False when there was nothing.
    pub fn forget_all(&mut self) -> bool {
        let changed = !self.is_default();
        *self = Self::default();
        changed
    }

    /// A habit on or off; false when nothing changed.
    pub fn switch(&mut self, id: &str, on: bool) -> bool {
        self.list.iter_mut().find(|h| h.id == id).is_some_and(|h| std::mem::replace(&mut h.on, on) != on)
    }
}

/// `s` with its first letter a capital.
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|first| first.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// One form's part of a template rule, and its short name.
fn part(form: &Form, ru: bool) -> (String, String) {
    let own = |rule: &str, short: &str| (rule.to_owned(), short.to_owned());
    match (form, ru) {
        (Form::EndMark { mark: '.', removed: true }, false) => own("no full stop at the end", "no full stop"),
        (Form::EndMark { mark: '.', removed: true }, true) => own("без точки в конце", "без точки"),
        (Form::EndMark { mark, removed: true }, false) => (format!("no “{mark}” at the end"), format!("no “{mark}”")),
        (Form::EndMark { mark, removed: true }, true) => (format!("без «{mark}» в конце"), format!("без «{mark}»")),
        (Form::EndMark { mark: '.', removed: false }, false) => own("a full stop at the end", "full stop"),
        (Form::EndMark { mark: '.', removed: false }, true) => own("с точкой в конце", "с точкой"),
        (Form::EndMark { mark, removed: false }, false) => (format!("“{mark}” at the end"), format!("with “{mark}”")),
        (Form::EndMark { mark, removed: false }, true) => (format!("«{mark}» в конце"), format!("с «{mark}»")),
        (Form::FirstLetter { lower: true }, false) => own("lower case at the start", "lower case"),
        (Form::FirstLetter { lower: true }, true) => own("с маленькой буквы", "с маленькой"),
        (Form::FirstLetter { lower: false }, false) => own("a capital at the start", "capital"),
        (Form::FirstLetter { lower: false }, true) => own("с большой буквы", "с большой"),
        (Form::Replacement { from, to }, false) => (format!("write “{to}” instead of “{from}”"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Replacement { from, to }, true) => (format!("писать «{to}» вместо «{from}»"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Digits, false) => own("numbers in digits", "digits"),
        (Form::Digits, true) => own("числа цифрами", "цифрами"),
        (Form::Dash, false) => own("a dash between words, not a hyphen", "dash"),
        (Form::Dash, true) => own("тире вместо дефиса", "тире"),
    }
}

/// The rule when the cleanup's connection gave none: "In <app>: …" / «В <app>: …» with each form's part (the last
/// joined by "and" / «и»), or without the app for everywhere; the short name is the last form's. At most
/// `MAX_RULE_CHARS`.
pub fn template(forms: &[Form], app: Option<&str>, lang: &str) -> RuleText {
    let ru = lang == "ru";
    let parts: Vec<(String, String)> = forms.iter().map(|f| part(f, ru)).collect();
    let and = if ru { " и " } else { " and " };
    let what = match parts.as_slice() {
        [] => String::new(),
        [one] => one.0.clone(),
        [init @ .., last] => format!("{}{and}{}", init.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", "), last.0),
    };
    let rule = match app {
        Some(app) => format!("{} {app}: {what}", if ru { "В" } else { "In" }),
        None => capitalized(&what),
    };
    let short = parts.last().map(|p| p.1.clone()).unwrap_or_default();
    RuleText { rule: cut(&rule, MAX_RULE_CHARS, false), short }
}

/// A rule written for one app, for every app: its "In <app>" / «В <app>» start cut off (with a dash, a colon or a
/// comma after it), the rest from a capital; a rule that does not start so is written anew from the template.
pub fn unscoped(rule: &str, app: &str, forms: &[Form], lang: &str) -> String {
    for lead in ["In ", "in ", "В ", "в "] {
        if let Some(rest) = rule.strip_prefix(lead).and_then(|r| r.strip_prefix(app)) {
            let rest = rest.trim_start_matches([' ', ':', '—', '–', '-', ',']);
            if !rest.is_empty() {
                return capitalized(rest);
            }
        }
    }
    template(forms, None, lang).rule
}

/// The rule request's instructions; `{LANGUAGE}` becomes the interface language.
const RULE_PROMPT: &str = "\
A dictation app pastes text that the user then corrects by hand, and the user has made the same correction three \
times in one app. From the kind of correction (endMark: the last sentence mark removed or added; firstLetter: the first \
letter's case changed; replacement: words written another way; digits: numbers written as digits; dash: a dash between \
words instead of a hyphen), one example (the text before and after the correction) and the app's name, write the rule \
the app should follow there from now on. \"rule\": one sentence of at most 80 characters that starts with the app, like \
\"In Telegram — lower case and no full stop\". \"short\": one to three words that name it, like \"no full stop\". No full \
stop at the end of either. Write both in {LANGUAGE}. Answer with a JSON object with exactly the fields \"rule\" and \
\"short\".";

/// The rule request's two messages: the instructions, and the user message — the kinds, the example and the app as a
/// JSON object, nothing else.
pub fn rule_messages(ask: &RuleAsk, lang: &str) -> (String, String) {
    let language = if lang == "ru" { "Russian" } else { "English" };
    (RULE_PROMPT.replace("{LANGUAGE}", language), serde_json::to_string(ask).unwrap_or_default())
}

/// One line: no control character, no line or paragraph separator.
fn one_line(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// The cleanup's answer to the rule request: a JSON object with `rule` and `short`, in a code fence or not. Usable when
/// each is one line, the rule not empty and at most `MAX_RULE_CHARS`, the short name one to three words and at most
/// `MAX_SHORT_CHARS`; a full stop at their end goes.
pub fn read_rule(content: &str) -> Option<RuleText> {
    let t = content.trim();
    let t = match t.strip_prefix("```json").or_else(|| t.strip_prefix("```")) {
        Some(inner) => inner.trim_end().strip_suffix("```").unwrap_or(inner),
        None => t,
    };
    let r: RuleText = serde_json::from_str(t.trim()).ok()?;
    let rule = r.rule.trim().trim_end_matches('.').trim_end().to_owned();
    let short = r.short.trim().trim_end_matches('.').trim_end().to_owned();
    let words = short.split_whitespace().count();
    let ok = !rule.is_empty()
        && rule.chars().count() <= MAX_RULE_CHARS
        && one_line(&rule)
        && (1..=3).contains(&words)
        && short.chars().count() <= MAX_SHORT_CHARS
        && one_line(&short);
    ok.then_some(RuleText { rule, short })
}

/// Of the habits sent with a dictation's cleanup, the ones whose effect shows in the cleaned text: its history entry
/// lists them.
pub fn marks(sent: &[Habit], text: &str) -> Vec<HabitMark> {
    sent.iter().filter(|h| h.applied(text)).map(|h| HabitMark { id: h.id.clone(), short: h.short.clone() }).collect()
}
```

  Task 4's `habits_prompt` checks a rule with `one_line` too, and makes it `pub(crate)`.

- [ ] **Step 5: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green. Then
  `cargo build --workspace --all-targets`: the `Entry` literals outside the crate need `habits: Vec::new()`; follow the
  compiler.

---


- Task 1 changed the Digits rule: a run counts as Digits only when `to` holds no uppercase letter and no letter of the other alphabet (`corrections::other_alphabet` is pub(crate)). If your applied-check or template code relies on Digits, keep that in mind.
- Run clippy for the workspace at the end of this task (deferred from Task 1); the `rank` dead-code warning must be gone once used.
- Cover what Task 1's review found untested if this task uses it: `Form::shows_in`, `has_run`, `example` with several forms (the replacement fallback and the final cut).
