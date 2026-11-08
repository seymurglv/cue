### Task 6: the runtime — counting, the rule, the card at rest, Undo, widening, the applied habits

**Files:**
- Modify: `app/crates/core/src/ui.rs` (`TrayKind`, line 44), `app/crates/core/src/corrections.rs` (`Offers`, line 257),
  `app/src-tauri/src/ipc.rs` (`Ui`, next to `model_name`, line 370), `app/src-tauri/src/runtime.rs`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces: `TrayKind::NewHabit` (JSON `"newHabit"`); `Offers::waiting(&self) -> bool`, `Offers::open(&self) -> bool`;
  `Ui::habit_rule(&self, rule: &str)` emitting `cue://habit-rule` (a string) to the Drop just before the card;
  `CoreEvent::Corrected { pasted, span, stop, app: Option<String> }` (`app`: the dictation's app for habits, `None` for a
  translation); `CoreEvent::HabitRule { draft: u64, rule: Option<habits::RuleText> }`; the card's "Undo" is
  `UiAction::TrayAction` on `TrayKind::NewHabit` → `SettingChange::HabitForget`; `cue://settings` after a habit is
  learned, widened, grown or forgotten from the card.

- [ ] **Step 1: the failing tests.** In `ui.rs`'s `every_variant_has_its_json_name`, add `(TrayKind::NewHabit, "newHabit")`
  to `trays`. In `corrections.rs`'s tests:

```rust
    #[test]
    fn waiting_and_open_say_where_the_suggestion_is() {
        let mut o = Offers::default();
        assert!(!o.waiting() && !o.open());
        o.offer(marisol());
        assert!(o.waiting() && !o.open());
        o.at_rest();
        assert!(!o.waiting() && o.open());
        o.added();
        assert!(o.open(), "showing undo");
        o.done();
        o.offer(marisol());
        o.at_rest();
        o.dismissed();
        assert!(!o.waiting() && !o.open(), "a missed one is a dot");
    }
```

  In `runtime.rs`'s tests:

```rust
    /// The suggestion and a habit's card at rest: the one found first; a card never while the capsule is open.
    #[test]
    fn the_notice_found_first_opens_first() {
        assert_eq!(first_notice(Some(1), Some(2), false), Some(Notice::Offer));
        assert_eq!(first_notice(Some(3), Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(None, Some(2), true), None, "the capsule is open");
        assert_eq!(first_notice(None, Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(Some(1), None, false), Some(Notice::Offer));
        assert_eq!(first_notice(None, None, false), None);
    }
```

  Run `cargo test -p cue-core` and `cargo test -p cue` and see them fail.

- [ ] **Step 2: the kind, the offer's state, the push.** In `ui.rs` after `ModelSwitched`:

```rust
    /// A habit was just learned (docs/specs/2026-11-04-habits.md): its rule comes to the Drop just before the card, and
    /// the card's button undoes it.
    NewHabit,
```

  In `corrections.rs`, in `impl Offers` after `view`:

```rust
    /// Found while the Drop was busy: it opens at the next rest.
    pub fn waiting(&self) -> bool {
        matches!(self.cur, Some((_, Stage::Waiting)))
    }

    /// The capsule is open: with its ring, or showing "undo".
    pub fn open(&self) -> bool {
        matches!(self.cur, Some((_, Stage::Open | Stage::Added)))
    }
```

  In `ipc.rs` after `model_name`:

```rust
    /// The rule of the habit just learned, just before its card (docs/specs/2026-11-04-habits.md): to the Drop, which
    /// draws it.
    pub fn habit_rule(&self, rule: &str) {
        if let Err(e) = self.app.emit_to(DROP_LABEL, "cue://habit-rule", rule) {
            tracing::debug!("Drop window: event cue://habit-rule not sent: {e}");
        }
    }
```

- [ ] **Step 3: the runtime's state.** Imports: `use std::collections::VecDeque;`, `use cue_core::habits;`,
  `use cue_core::flow::RefineOutcome` (into the `cue_core::flow` line 66), `use cue_engines::habit;`. `CoreEvent::Corrected`
  (line 150) gains `app: Option<String>` ("the dictation's app, when its correction counts for habits: `None` for a
  translation"), and after it:

```rust
    /// The rule of the habit being made under `draft`, from the cleanup's connection: `None` (no connection, no answer
    /// in 5 s, none usable) — the template writes it. Never logged.
    HabitRule { draft: u64, rule: Option<habits::RuleText> },
```

  `watch_next` (line 285) becomes `Option<(DictationId, String, Option<String>)>` ("…with its text, and its app when
  its correction counts for habits"). In `struct Runtime`, after `field_stop`:

```rust
    /// Habits (docs/specs/2026-11-04-habits.md): the ones sent with a dictation's cleanup, and those whose effect showed
    /// in its text (its history entry lists them).
    habits_sent: Option<(DictationId, Vec<habits::Habit>)>,
    habits_applied: Option<(DictationId, Vec<history::HabitMark>)>,
    /// Habits waiting for their rule, by the number they were found with.
    drafts: Vec<(u64, habits::Draft)>,
    /// Habit cards waiting for the Drop's rest, in the order found, and the habit whose card is open.
    habit_cards: VecDeque<(u64, String)>,
    habit_card: Option<String>,
    /// Numbers the notices as they are found (`next_notice`), the suggestion's (`offer_found`) and each habit's: at rest
    /// the one found first opens first.
    notices: u64,
    offer_found: u64,
```

  initialized `habits_sent: None, habits_applied: None, drafts: Vec::new(), habit_cards: VecDeque::new(), habit_card: None, notices: 0, offer_found: 0,`
  in `Runtime::new` (after `field_stop: None,`, line 383).

- [ ] **Step 4: a dictation's paste is watched for habits too.** In `insert` (line 1322), the `watch_next` line
  becomes:

```rust
        // A dictation's correction counts for habits in its app as the history names it; a translation's only offers
        // words.
        let habit_app = (self.attempt.mode == Mode::Dictate).then(|| stored_app(&self.attempt.app)).filter(|a| !a.is_empty());
        self.watch_next = (self.settings.learn.enabled() && self.attempt.mode != Mode::Ask).then(|| (id, text.clone(), habit_app));
```

  In `InsertDone` (line 588): `if let Some((wanted, pasted, app)) = self.watch_next.take()` … `self.watch_field(pasted, app);`.
  `watch_field` (line 767) takes `app: Option<String>` and sends `CoreEvent::Corrected { pasted, span, stop: own, app }`.
  The `Corrected` arm (line 596): `CoreEvent::Corrected { pasted, span, stop, app } => self.on_corrected(&pasted, span.as_deref(), &stop, app.as_deref()),`.
  And the new arm next to it: `CoreEvent::HabitRule { draft, rule } => self.on_habit_rule(draft, rule),`.

- [ ] **Step 5: counting, the rule, the habit.** `on_corrected` (line 785) becomes:

```rust
    /// A watch is over: a word the user wrote instead of the one heard becomes the suggestion (the first, when there
    /// are several), and a dictation's correction counts for habits (`count_habits`). The log gets numbers, never the
    /// text.
    fn on_corrected(&mut self, pasted: &str, span: Option<&str>, stop: &Arc<AtomicBool>, app: Option<&str>) {
        // A watch replaced by a later one leaves the later one's flag in place.
        self.field_stop.take_if(|running| Arc::ptr_eq(running, stop));
        if !self.settings.learn.enabled() {
            return;
        }
        let Some(span) = span else {
            tracing::info!("correction: none");
            return;
        };
        let found = corrections::candidates(pasted, span, &self.settings.dictionary.terms);
        tracing::info!(chars = span.chars().count(), candidates = found.len(), "correction found");
        if let Some(first) = found.into_iter().next() {
            let before = self.offers.view();
            self.offers.offer(first);
            if self.offers.waiting() {
                self.offer_found = self.next_notice();
            }
            // A missed one it replaced (the dot) is gone from the page until the new one opens at rest.
            let after = self.offers.view();
            if after != before {
                self.ui.suggest(after.as_ref());
            }
        }
        if let Some(app) = app {
            self.count_habits(app, pasted, span);
        }
        self.show_notice();
    }

    /// The forms of a dictation's correction in `app` are counted (`Habits::count`): a habit of another app whose form
    /// reached three here holds everywhere now (its row changes, no card), and forms that reached three here together
    /// become a habit once its rule comes (`ask_rule`). The log gets kinds and numbers.
    fn count_habits(&mut self, app: &str, pasted: &str, span: &str) {
        let c = habits::correction(pasted, span);
        if c.found.is_empty() {
            return;
        }
        let counted = self.settings.habits.count(app, &c, &local_today(), self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = c.found.iter().map(|f| f.form.kind()).collect();
        tracing::info!(?kinds, widened = counted.widened.len(), new = counted.new.is_some(), "habit forms counted");
        if counted.grown || !counted.widened.is_empty() {
            self.send_settings();
        }
        if let Some(draft) = counted.new {
            self.ask_rule(draft);
        }
    }

    /// The rule of a habit just found, asked of the cleanup's connection: within `RULE_WAIT_MS`, or at once without a
    /// connection, `on_habit_rule` gets it or nothing. The habit is numbered now, as found.
    fn ask_rule(&mut self, draft: habits::Draft) {
        let n = self.next_notice();
        let ask = draft.ask();
        self.drafts.push((n, draft));
        let (http, s, tx, lang, key) = (self.http.clone(), self.settings.refine.clone(), self.tx.clone(), self.lang.code(), self.cleanup_key());
        tokio::spawn(async move {
            let wait = Duration::from_millis(habits::RULE_WAIT_MS);
            let rule = tokio::time::timeout(wait, habit::rule(&http, key.as_deref(), &s, &ask, lang, wait)).await.ok().flatten();
            let _ = tx.send(CoreEvent::HabitRule { draft: n, rule });
        });
    }

    /// The rule came (or the template writes it): the habit is saved, the main window draws it, and its card waits for
    /// the Drop's rest. Dropped when "Learn from me" went off meanwhile.
    fn on_habit_rule(&mut self, n: u64, rule: Option<habits::RuleText>) {
        let Some(at) = self.drafts.iter().position(|(d, _)| *d == n) else { return };
        let (_, draft) = self.drafts.remove(at);
        if !self.settings.learn.enabled() {
            return;
        }
        let from_cleanup = rule.is_some();
        let habit = self.settings.habits.adopt(draft, rule, self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = habit.forms.iter().map(habits::Form::kind).collect();
        tracing::info!(?kinds, n = habit.n, from_cleanup, "habit learned");
        self.send_settings();
        self.habit_cards.push_back((n, habit.id));
        self.show_notice();
    }

    fn next_notice(&mut self) -> u64 {
        self.notices += 1;
        self.notices
    }

    /// The settings to the main window after a change it did not ask for.
    fn send_settings(&self) {
        self.ui.settings(&SettingsView::of(&self.settings, self.lang, self.can_save_settings));
    }

    /// The cleanup connection's key from the store (a local server has none; `None` too when it cannot be read).
    fn cleanup_key(&self) -> Option<String> {
        KeyName::of_connection(self.settings.refine.connection().id).and_then(|name| {
            keys::lookup(name).unwrap_or_else(|e| {
                tracing::warn!("{} key: {e}", name.as_str());
                None
            })
        })
    }
```

  In `refine` (line 1260) replace the key lookup with `let key = self.cleanup_key();`, and in `learn_word` (line 757)
  replace its `self.ui.settings(…)` with `self.send_settings()`.

- [ ] **Step 6: one notice at a time, the one found first.** `show_offer` (line 812) becomes `show_notice`; rename
  its five callers (`feed`, the `TrayClose` arm, `on_corrected`, `on_tray_action`, the `IdleHidden` arm):

```rust
    /// A notice waiting opens once the Drop is at rest and can be clicked (`at_rest`): the dictionary suggestion or a
    /// habit's card, one at a time, the one found first (`first_notice`). Called again after anything that may end a
    /// busy Drop: every input (the end of a pause among them), a tray card going, the idle Drop showing again once no
    /// full-screen app is in front, a habit learned.
    fn show_notice(&mut self) {
        let s = self.flow.ui_state();
        if !at_rest(s.phase, self.tray.is_some(), self.flow.answer_open(), s.paused, self.idle_hidden) {
            return;
        }
        let offer = self.offers.waiting().then_some(self.offer_found);
        let card = self.habit_cards.front().map(|(n, _)| *n);
        match first_notice(offer, card, self.offers.open()) {
            Some(Notice::Offer) => {
                if self.offers.at_rest() {
                    self.ui.suggest(self.offers.view().as_ref());
                }
            }
            Some(Notice::Habit) => self.show_habit_card(),
            None => {}
        }
    }

    /// The next habit's card: its rule to the Drop, then the card. One forgotten from the main window meanwhile has none.
    fn show_habit_card(&mut self) {
        while let Some((_, id)) = self.habit_cards.pop_front() {
            let Some(rule) = self.settings.habits.list.iter().find(|h| h.id == id).map(|h| h.rule.clone()) else { continue };
            self.habit_card = Some(id);
            self.ui.habit_rule(&rule);
            self.tray = Some(TrayKind::NewHabit);
            self.ui.tray(self.tray);
            return;
        }
    }

    /// The rule of the habit whose card is open, before the card goes to a Drop loaded again.
    fn send_habit_rule(&self, kind: Option<TrayKind>) {
        if kind == Some(TrayKind::NewHabit)
            && let Some(h) = self.habit_card.as_ref().and_then(|id| self.settings.habits.list.iter().find(|h| &h.id == id))
        {
            self.ui.habit_rule(&h.rule);
        }
    }
```

  Free functions next to `at_rest` (line 2342):

```rust
/// Which notice opens at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Offer,
    Habit,
}

/// The one found first of a waiting suggestion and a habit's card, each by the number it was found with; a card never
/// opens while the suggestion's capsule is open (`open`: with its ring or "undo"). A missed suggestion's dot does not
/// hold a card back.
fn first_notice(offer: Option<u64>, card: Option<u64>, open: bool) -> Option<Notice> {
    match (offer, card) {
        (Some(o), Some(c)) if c < o => (!open).then_some(Notice::Habit),
        (Some(_), _) => Some(Notice::Offer),
        (None, Some(_)) => (!open).then_some(Notice::Habit),
        (None, None) => None,
    }
}
```

  Update `at_rest`'s doc comment: "The Drop is at rest for a notice (the suggestion, a habit's card): …".

- [ ] **Step 7: the card's Undo, and cards that go.** In `on_tray_action` (line 1832), a new arm before the catch-all:

```rust
            // "Undo" on a habit's card: the habit goes, and its forms are never learned again.
            Some(TrayKind::NewHabit) => {
                if let Some(id) = self.habit_card.take() {
                    self.learn_word(SettingChange::HabitForget(id));
                    tracing::info!("habit undone");
                }
            }
```

  The `TrayClose` arm (line 511) sets `self.habit_card = None;` before `show_notice()`. In `exec`'s `Effect::Tray(kind)`
  arm (line 971), before `self.tray = kind;`: `self.habit_card = None; // the flow's card, or none, takes a habit's place`.
  In `UiReady` (line 554), before `self.ui.tray(self.tray);`: `self.send_habit_rule(self.tray);`. In `change_setting`'s
  "Learn from me" off block (line 744), add `self.drafts.clear();` and `self.habit_cards.clear();` (an open card stays
  until it closes; its Undo still forgets).

- [ ] **Step 8: the habits go with a dictation's cleanup, and the applied ones on its entry.** In `refine` (line 1260),
  after `let job = job_for(…)` and before the task is spawned:

```rust
        // The habits of this app (or of everywhere) go with a dictation's cleanup, never with a translation or "Ask".
        let sent: Vec<habits::Habit> = if mode == Mode::Dictate && self.settings.learn.enabled() && matches!(job, Job::Clean | Job::Rewrite) {
            self.settings.habits.for_app(&stored_app(&self.attempt.app)).cloned().collect()
        } else {
            Vec::new()
        };
        let ctx = ctx.with_habits(sent.iter().map(|h| h.rule.clone()).collect());
        self.habits_sent = Some((id, sent));
```

  In `on_refined` (line 897), after the `finished` check:

```rust
        // Of the habits sent, those whose effect shows in the cleaned text go on its history entry; a raw fallback shows
        // none.
        if let Some((_, sent)) = self.habits_sent.take_if(|(i, _)| *i == id) {
            let marks = if result.outcome == RefineOutcome::Refined { habits::marks(&sent, &result.text) } else { Vec::new() };
            self.habits_applied = Some((id, marks));
        }
```

  In `on_finished`'s `LastFiles::Commit` branch (line 1887):

```rust
                let mut entry = dictation_entry(&self.attempt, self.final_text.as_ref(), raw.as_ref(), stamp);
                if let Some(e) = entry.as_mut()
                    && let Some((_, marks)) = self.habits_applied.take_if(|(i, _)| *i == id)
                {
                    e.habits = marks;
                }
```

  Add a bullet to the module comment after the corrections one: "a dictation's correction also counts for habits
  (docs/specs/2026-11-04-habits.md): a form corrected three times in one app becomes a habit, its rule asked of the
  cleanup's connection (5 s, else a template), its card shown at rest after the suggestion found before it; the
  habits of the dictation's app go with its cleanup, and those whose effect shows are kept on its history entry;".

- [ ] **Step 9: run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; `cargo build -p cue`
  builds.

---


## Coordinator rulings for this task (binding, override the text above)
- Call `show_notice()` at the end of the `SuggestDismiss`, `SuggestDone`, `SuggestUndo` arms and of the refused `SuggestAdd` branch, so a habit card queued behind the suggestion capsule opens when the capsule closes.
- Turning "Learn from me" off also drops habits saved but not yet announced (no card shown yet): remove them from `habits.list` and their forms from nothing else (not declined).

- Task 5's `cue_engines::habit::rule` does not apply the wait itself: pass `Duration::from_millis(cue_core::habits::RULE_WAIT_MS)` as its timeout.
