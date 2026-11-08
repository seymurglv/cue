### Task 12: stricter widening (a habit's whole set, three times in another app)

Decided 06.11.2026 (spec, "Wider", changed).

**Files:**
- Modify: `app/crates/core/src/habits.rs` (`Counted`, `Seen`, `Habits::count`, `Habits::forget`, tests)
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Task 11 (`hash(&self.secret)`, `habits()` in the tests).
- Produces: no new names. `Habits::count` widens a habit of another app only when all its forms came together in one
  correction `LEARN_AT` times there; each such correction bumps one `seen` entry `{ app, form: <the habit's id>, n, last }`.
  A form of a habit of this app or of everywhere grows that habit and is in no set; the other forms count for this app
  as any form does, and can make its own habit. `forget` drops the habit's set counts too.

- [ ] **Step 1: the tests.** Rename `a_habit_widens_when_its_form_reaches_three_in_another_app` to
  `a_habit_of_one_form_widens_when_it_comes_three_times_in_another_app` with the doc comment "/// A habit of one form:
  the form alone is its whole set." (its body stays: the dash alone is the habit's set).
  `a_habit_of_two_forms_grows_by_one_per_correction_and_widening_adds_only_its_count` stays as it is (Word gets both forms
  together each time). Replace `widening_removes_the_counts_of_all_its_forms_in_every_app` with:

```rust
    #[test]
    fn widening_removes_the_counts_of_its_set_and_its_forms_in_every_app() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Notepad", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        h.count("Notepad", &correction("Скинь файл.", "Скинь файл"), "2026-11-04", "en");
        let seen: Vec<(&str, &str, u32)> = h.seen.iter().map(|s| (s.app.as_str(), s.form.as_str(), s.n)).collect();
        let stop = STOP.hash(&h.secret);
        assert_eq!(seen, [("Notepad", stop.as_str(), 1), ("Notepad", id.as_str(), 1)], "the set under the habit's id");
        let c = correction("Скинь файл.", "скинь файл");
        h.count("Word", &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        assert!(h.seen.is_empty(), "{:?}", h.seen);
    }
```

  And add, before `a_replacement_outside_the_stored_example_is_not_drafted_but_still_counted`:

```rust
    /// Widening asks for the habit's whole set in one dictation: its forms seen apart in the other app count for that
    /// app, as any form does.
    #[test]
    fn all_its_forms_together_three_times_in_another_app_widen_a_habit() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        let both = correction("Скинь файл.", "скинь файл");
        let stop = correction("Скинь файл.", "Скинь файл");
        h.count("Word", &both, "2026-11-04", "en");
        h.count("Word", &stop, "2026-11-04", "en");
        h.count("Word", &both, "2026-11-04", "en");
        let seen: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(seen, [(id.as_str(), 2), (STOP.hash(&h.secret).as_str(), 1)], "the set twice, the full stop alone once");
        assert_eq!(h.list[0].app.as_deref(), Some(TG));
        let counted = h.count("Word", &both, "2026-11-04", "en");
        assert_eq!((counted.widened, counted.new), (vec![id], None));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Lower case at the start and no full stop at the end"));
        assert!(h.seen.is_empty());
    }

    /// One form of a habit, three times alone in another app: no widening, but that app's own habit.
    #[test]
    fn one_form_of_a_habit_alone_in_another_app_makes_that_apps_own_habit() {
        let mut h = habits();
        let first = learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty() && !counted.grown);
        let draft = counted.new.expect("Word's own");
        assert_eq!((draft.forms.as_slice(), draft.app.as_str()), ([STOP].as_slice(), "Word"));
        let word = h.adopt(draft, None, "en");
        assert_eq!(h.list, [word.clone(), first.clone()], "the first stays in its app");
        assert_eq!(word.rule, "In Word: no full stop at the end");
        assert_eq!(h.for_app("Word").map(|x| x.id.as_str()).collect::<Vec<_>>(), [word.id.as_str()]);
        assert_eq!(h.for_app(TG).map(|x| x.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()]);
        // In Word now, the full stop counts on Word's habit and the first letter alone; the set is not whole.
        let counted = h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert!(counted.grown && counted.widened.is_empty());
        assert_eq!(h.list[0].n, 4);
        assert_eq!(h.seen.iter().map(|s| s.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: true }.hash(&h.secret)]);
    }

    #[test]
    fn a_forgotten_habit_takes_its_sets_counts_along() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), 1);
        assert!(h.forget(&id));
        assert!(h.seen.is_empty());
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core habits`. Expected: the four new or rewritten
  tests fail (the old `count` widens on the full stop alone and keeps per-form counts in Word); the others pass.

- [ ] **Step 3: `count` counts a habit's set.** Replace `count`, from its doc comment down to the line before
  `// Only replacements whose words stand in the stored example are drafted; …` (that part and the draft after it stay
  as they are), with:

```rust
    /// One correction of a dictation in `app` (the history's name for it, `today` its local date): each form counts
    /// once. A declined form is not counted. A form of a habit of this app or of everywhere adds to that habit's count.
    /// A habit of another app whose forms all came here together counts as one entry of this app, under its id; at
    /// `LEARN_AT` it widens (its rule loses its app, its counts add up). The other forms count for this app, and those
    /// that reach `LEARN_AT` together become one draft. The counts that went into a habit go; counts older than
    /// `SEEN_DAYS` go, and past `MAX_SEEN` the least recently seen. Without the secret nothing is counted.
    pub fn count(&mut self, app: &str, c: &Correction, today: &str, lang: &str) -> Counted {
        let mut out = Counted::default();
        if app.is_empty() || !self.secret.is_set() {
            return out;
        }
        self.prune(today);
        let live: Vec<(&Found, String)> =
            c.found.iter().map(|f| (f, f.form.hash(&self.secret))).filter(|(_, hash)| !self.declined.contains(hash)).collect();
        // The forms counted on a habit already, or in a habit's set.
        let mut handled: Vec<String> = Vec::new();
        let mut grown: Vec<String> = Vec::new();
        for (_, hash) in &live {
            let here = |h: &&Habit| h.app.as_deref().is_none_or(|a| a == app) && h.hashes(&self.secret).contains(hash);
            if let Some(h) = self.list.iter().find(here) {
                if !grown.contains(&h.id) {
                    grown.push(h.id.clone());
                }
                handled.push(hash.clone());
            }
        }
        let free: Vec<&String> = live.iter().map(|(_, hash)| hash).filter(|hash| !handled.contains(hash)).collect();
        let sets: Vec<(String, Vec<String>)> = self
            .list
            .iter()
            .filter(|h| h.app.as_deref().is_some_and(|a| a != app))
            .map(|h| (h.id.clone(), h.hashes(&self.secret)))
            .filter(|(_, hashes)| hashes.iter().all(|hash| free.contains(&hash)))
            .collect();
        let mut widened: Vec<(String, u32)> = Vec::new();
        for (id, hashes) in sets {
            handled.extend(hashes);
            let n = self.bump(app, &id, today);
            if n >= LEARN_AT {
                widened.push((id, n));
            }
        }
        for (id, n) in &widened {
            if let Some(h) = self.list.iter_mut().find(|h| &h.id == id) {
                let was = h.app.take().unwrap_or_default();
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
                h.n = h.n.saturating_add(*n);
                // Its set's counts and its forms' counts, in every app.
                let mut gone = h.hashes(&self.secret);
                gone.push(id.clone());
                self.seen.retain(|s| !gone.contains(&s.form));
            }
        }
        // A habit's count grows once per correction.
        for h in &mut self.list {
            if grown.contains(&h.id) {
                h.n = h.n.saturating_add(1);
            }
        }
        out.grown = !grown.is_empty();
        out.widened = widened.into_iter().map(|(id, _)| id).collect();
        let mut reached: Vec<&Found> = Vec::new();
        for (f, hash) in &live {
            if !handled.contains(hash) && self.bump(app, hash, today) >= LEARN_AT {
                reached.push(f);
            }
        }
```

  `Counted.widened`'s comment: "Habits of another app whose forms came together here `LEARN_AT` times: they hold
  everywhere now." `Seen`'s comment: "How many times a form was corrected in an app, or a habit of another app was
  corrected there whole, and the day it last was: no text (`form` is the form's hash, or the habit's id)." `forget`'s
  comment ends "…never learned again; its set's counts go too. False when there is no such habit.", and its first line
  after the `let Some(i)` is `self.seen.retain(|s| s.form != id);`.

- [ ] **Step 4: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

- [ ] **Step 5: README and ARCHITECTURE.**
  - README, the **Habits** bullet (line 278): "When the same correction reaches three in another app, the habit holds in
    every app: its row says so, and no card opens." becomes "When all of a habit's corrections come together in one
    dictation three times in another app, the habit holds in every app: its row says so, and no card opens. One of
    them alone there counts for that app, and can become that app's own habit."
  - README, `habits.seen` (line 534): add "; a habit's whole set corrected together in another app is counted as one
    entry, under the habit's id."
  - ARCHITECTURE, Habits, step 3: "A form that reaches `LEARN_AT` (3) and is a habit of another app widens it: its app
    is cleared, the rule loses its "In <app>" start (`habits::unscoped`), the counts add up, and `send_settings` redraws
    the section without a card." becomes "A habit of another app whose forms all came together in the correction counts
    once for this app, under its id (a form already counted on a habit of this app or of every app is in no set); when
    that set reaches `LEARN_AT` (3) the habit widens: its app is cleared, the rule loses its "In <app>" start
    (`habits::unscoped`), the counts add up, its set's and its forms' counts go in every app, and `send_settings`
    redraws the section without a card. A habit's form alone in another app counts for that app as any form does."

- [ ] **Step 6: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`: green.

---


- Task 11 is done: `Form::hash` and friends now take the secret (`&Secret`); `Habits` holds it (`#[serde(skip)]`); with no secret set nothing is counted and `forget` declines nothing. Use the real signatures.
