### Task 13: opposite habits ("Habit changed")

Decided 06.11.2026 (spec, "Opposite forms" and "Changed").

**Files:**
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/ui.rs` (`TrayKind`, line 61; its test, line 155)
- Modify: `app/src-tauri/src/runtime.rs`
- Modify: `app/ui/src/strings.ts`
- Test: `app/ui/test/controller.test.ts`
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Tasks 11 and 12.
- Produces in `habits`: `Form::opposite(&self) -> Option<Form>` (end mark removed ↔ added, lower case ↔ capital,
  `A → B` ↔ `B → A`; digits and dash none); `fn opposes(a: &[Form], b: &[Form]) -> bool`;
  `#[derive(Debug, PartialEq)] pub struct Learned { pub habit: Habit, pub replaced: Vec<Habit>, pub merged: bool }`;
  `Habits::learn(&mut self, Draft, Option<RuleText>, lang) -> Learned` (`adopt` becomes `learn(..).habit`, its callers
  unchanged); `fn Habits::opposed(&self, forms, id) -> bool`; `Habits::revert(&mut self, id: &str, replaced: Vec<Habit>,
  decline: bool) -> bool`. `count` lets no habit with an opposite habit in some app widen.
- Produces in `ui`: `TrayKind::HabitChanged` (JSON `"habitChanged"`), drawn like `newHabit` with the rule just before
  it on `cue://habit-rule`; its "Undo" is `UiAction::TrayAction` → `Habits::revert(id, replaced, true)`.
- Produces in the runtime: `struct HabitCard { id: String, replaced: Vec<habits::Habit> }` with `kind()`;
  `habit_cards: VecDeque<(u64, HabitCard)>`, `habit_card: Option<HabitCard>`; `undo_habit(&mut self, HabitCard)`.
- Produces in `strings.ts`: `TrayKind` gains `'habitChanged'`; `TRAYS.habitChanged = { icon: 'habit', tone: 'ok',
  act: 'undo', ttl: 7000 }`; `en.tray.habitChanged` / `ru.tray.habitChanged` (same keys as `newHabit`).

- [ ] **Step 1: the failing tests.** In `habits.rs`'s tests, before
  `a_replacement_outside_the_stored_example_is_not_drafted_but_still_counted`:

```rust
    const ADDED: Form = Form::EndMark { mark: '.', removed: false };

    #[test]
    fn each_form_has_its_opposite_but_digits_and_a_dash() {
        assert_eq!((STOP.opposite(), ADDED.opposite()), (Some(ADDED), Some(STOP)));
        assert_eq!(Form::FirstLetter { lower: true }.opposite(), Some(Form::FirstLetter { lower: false }));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(hello.opposite(), Some(Form::Replacement { from: "добрый день".into(), to: "здравствуйте".into() }));
        assert_eq!((Form::Digits.opposite(), Form::Dash.opposite()), (None, None));
    }

    /// The user goes back on a habit: the opposite counts like any form, and at three the habit changes. The old one
    /// goes without being declined; "Undo" brings it back as it was and declines the new forms.
    #[test]
    fn an_opposite_three_times_changes_the_habit_and_undo_brings_it_back() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        assert_eq!(kinds("Скинь файл", "Скинь файл."), [ADDED]);
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count(TG, &back, "2026-11-04", "en");
            assert!(!counted.grown, "the habit's own count does not grow");
        }
        let changed = h.learn(counted.new.expect("the opposite at three"), None, "en");
        assert_eq!((changed.replaced.as_slice(), changed.merged), ([old.clone()].as_slice(), false));
        assert_eq!((changed.habit.rule.as_str(), changed.habit.app.as_deref()), ("In Telegram Desktop: a full stop at the end", Some(TG)));
        assert_eq!(h.list, std::slice::from_ref(&changed.habit), "the two never live together");
        assert!(h.declined.is_empty(), "the old habit may be learned again");
        assert!(h.revert(&changed.habit.id, changed.replaced, true));
        assert_eq!(h.list, [old]);
        assert_eq!(h.declined, [ADDED.hash(&h.secret)]);
        assert_eq!(h.count(TG, &back, "2026-11-04", "en"), Counted::default(), "the new form is declined");
        assert!(!h.revert("nope", Vec::new(), true));
    }

    /// An opposite in another app is that app's own habit: the first one is not changed.
    #[test]
    fn an_opposite_in_another_app_leaves_the_habit_alone() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let word = h.learn(counted.new.expect("Word's own"), None, "en");
        assert!(word.replaced.is_empty());
        assert_eq!(h.list, [word.habit, old]);
    }

    /// A habit of everywhere is changed by an opposite reaching three in one app, and the change holds everywhere: the
    /// app habits it goes back on go too. Without `decline`, reverting leaves the new forms free.
    #[test]
    fn a_change_of_a_habit_of_everywhere_holds_everywhere() {
        let mut h = habits();
        let draft = |forms: Vec<Form>, app: &str| Draft { forms, app: app.into(), before: "a".into(), after: "b".into(), n: 3 };
        h.adopt(draft(vec![Form::FirstLetter { lower: true }], TG), None, "en");
        h.list[0].app = None; // as if it had widened
        h.adopt(draft(vec![STOP], "Notepad"), None, "en");
        let dash = h.adopt(draft(vec![Form::Dash], "Word"), None, "en");
        let before = h.list.clone();
        let back = correction("скинь файл", "Скинь файл.");
        assert_eq!(back.found.iter().map(|f| f.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: false }, ADDED]);
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let changed = h.learn(counted.new.expect("in Word"), None, "en");
        assert_eq!(changed.replaced, before[1..], "the full stop of Notepad and the lower case of everywhere");
        assert_eq!((changed.habit.app.as_deref(), changed.habit.rule.as_str()), (None, "A capital at the start and a full stop at the end"));
        assert_eq!(h.list, [changed.habit.clone(), dash]);
        assert!(h.revert(&changed.habit.id, changed.replaced, false));
        assert!(h.declined.is_empty());
        let mut back_again: Vec<String> = h.list.iter().map(|x| x.id.clone()).collect();
        let mut was: Vec<String> = before.iter().map(|x| x.id.clone()).collect();
        back_again.sort();
        was.sort();
        assert_eq!(back_again, was, "the three as they were");
    }

    /// A habit with an opposite habit in another app never holds everywhere, where the two would live together:
    /// its forms count for a third app on their own, and a second draft of them only adds to its count.
    #[test]
    fn a_habit_with_an_opposite_in_another_app_never_widens() {
        let mut h = habits();
        let tg = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let word = Draft { forms: vec![ADDED], app: "Word".into(), before: "a".into(), after: "a.".into(), n: 3 };
        let word = h.adopt(word, None, "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Notepad", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty());
        let merged = h.learn(counted.new.expect("its form counted for Notepad"), None, "en");
        assert!(merged.merged && merged.replaced.is_empty());
        assert_eq!((merged.habit.id.as_str(), merged.habit.app.as_deref(), merged.habit.n), (tg.id.as_str(), Some(TG), 6));
        assert_eq!(h.list.len(), 2);
        assert_eq!(h.list[0], word);
    }
```

  In `ui.rs`'s `every_variant_has_its_json_name`, add `(TrayKind::HabitChanged, "habitChanged")` to `trays`. In
  `runtime.rs`'s tests, after `the_notice_found_first_opens_first`:

```rust
    /// A habit's card says "New habit", or "Habit changed" when the habit took the place of others.
    #[test]
    fn a_card_that_replaced_a_habit_says_it_changed() {
        let fresh = HabitCard { id: "h1".into(), replaced: Vec::new() };
        assert_eq!(fresh.kind(), TrayKind::NewHabit);
        let d = habits::Draft { forms: vec![habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let old = habits::Habits::default().adopt(d, None, "en");
        assert_eq!(HabitCard { id: "h2".into(), replaced: vec![old] }.kind(), TrayKind::HabitChanged);
    }
```

  In `controller.test.ts`, add `'habitChanged'` to `KINDS` (line 24), and after the `'New habit card'` block (it ends at
  line 1291):

```ts
describe('Habit changed card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — a full stop at the end';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the new rule under "Habit changed", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    expect(box().querySelector('.nhd b')!.textContent).toBe('Habit changed');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Restored", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    act().click();
    expect(sent).toEqual(['trayAction']);
    expect([...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('')).toBe('Restored');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });
});
```

  And in the "Russian interface" block, after the habit card's test:

```ts
  it('the changed habit card in Russian: «Отменить» becomes «Возвращено»', () => {
    ctl.habitRule('В Telegram — с точкой в конце');
    ctl.tray('habitChanged');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Привычка изменилась');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с точкой в конце. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Возвращено');
  });
```

  Run `cargo test -p cue-core`, `cargo test -p cue` and `npx vitest run test/controller.test.ts` (from `app`) and see
  them fail: `opposite`, `learn`, `revert`, `HabitChanged`, `HabitCard` and `'habitChanged'` are not defined.

- [ ] **Step 2: opposites in the core** (`habits.rs`). In `impl Form`, before `shows_in`:

```rust
    /// The form that goes back on this one (docs/specs/2026-11-04-habits.md, "Opposite forms"): the end mark added for
    /// one removed and back, the other case, the replacement the other way. Digits and a dash have none.
    pub fn opposite(&self) -> Option<Form> {
        match self {
            Self::EndMark { mark, removed } => Some(Self::EndMark { mark: *mark, removed: !removed }),
            Self::FirstLetter { lower } => Some(Self::FirstLetter { lower: !lower }),
            Self::Replacement { from, to } => Some(Self::Replacement { from: to.clone(), to: from.clone() }),
            Self::Digits | Self::Dash => None,
        }
    }
```

  Above `/// What counting one correction changed.`:

```rust
/// Forms that go back on each other: one of `a` has its opposite in `b`.
fn opposes(a: &[Form], b: &[Form]) -> bool {
    a.iter().any(|f| f.opposite().is_some_and(|o| b.contains(&o)))
}

/// What learning a draft did ([`Habits::learn`]).
#[derive(Debug, PartialEq)]
pub struct Learned {
    /// The habit as it is now.
    pub habit: Habit,
    /// The habits it changed, as they were: they held an opposite of its forms and are gone, not declined.
    pub replaced: Vec<Habit>,
    /// A habit of its forms was listed already ([`Habits::listed`]): the draft added to it.
    pub merged: bool,
}
```

  In `count`'s `sets` (Task 12), the first filter becomes
  `.filter(|h| h.app.as_deref().is_some_and(|a| a != app) && !self.opposed(&h.forms, &h.id))`. Replace `adopt` (its doc
  comment and body) with:

```rust
    /// The draft's habit, as [`Habits::learn`] makes it.
    pub fn adopt(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Habit {
        self.learn(draft, rule, lang).habit
    }

    /// The draft's habit goes first, with the rule the cleanup wrote or else the template's; past `MAX_HABITS` the
    /// oldest go. Its id is the hash of its forms. The habits holding an opposite of its forms in its app or everywhere
    /// are changed by it (docs/specs/2026-11-04-habits.md, "Changed"): they go, not declined; when one of them held
    /// everywhere, the new habit does too, and those of every app go as well. When a habit of its forms is listed
    /// already ([`Habits::listed`]), the draft adds to it instead, as a widening would: its count adds up, and a draft
    /// of another app makes it hold everywhere (its rule loses its app, the draft's rule is not used) unless an opposite
    /// habit lives in some app ([`Habits::opposed`]).
    pub fn learn(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Learned {
        let opposite = |h: &Habit| opposes(&h.forms, &draft.forms);
        let everywhere = self.list.iter().any(|h| h.app.is_none() && opposite(h));
        let (replaced, kept): (Vec<Habit>, Vec<Habit>) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|h| opposite(h) && (everywhere || h.app.as_deref() == Some(draft.app.as_str())));
        self.list = kept;
        let id = id_of(&draft.forms, &self.secret);
        let opposed = self.opposed(&draft.forms, &id);
        if let Some(h) = self.list.iter_mut().find(|h| h.id == id) {
            if !opposed
                && (everywhere || h.app.as_deref().is_some_and(|a| a != draft.app))
                && let Some(was) = h.app.take()
            {
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
            }
            h.n = h.n.saturating_add(draft.n);
            return Learned { habit: h.clone(), replaced, merged: true };
        }
        let RuleText { rule, short } = rule.unwrap_or_else(|| template(&draft.forms, Some(&draft.app), lang));
        let (rule, app) =
            if everywhere { (unscoped(&rule, &draft.app, &draft.forms, lang), None) } else { (rule, Some(draft.app)) };
        let habit = Habit { id, forms: draft.forms, rule, short, before: draft.before, after: draft.after, app, n: draft.n, on: true };
        self.list.insert(0, habit.clone());
        self.list.truncate(MAX_HABITS);
        Learned { habit, replaced, merged: false }
    }

    /// A habit other than `id` holds an opposite of one of `forms`: a habit of them never holds everywhere, where the
    /// two would live together.
    fn opposed(&self, forms: &[Form], id: &str) -> bool {
        self.list.iter().any(|h| h.id != id && opposes(&h.forms, forms))
    }

    /// A change undone: the habit `id` goes and the habits it replaced come back as they were, first. With `decline`
    /// (the card's "Undo") its forms are never learned again; without (a change dropped before its card, "Learn from
    /// me" off) they may be. False when there is no such habit.
    pub fn revert(&mut self, id: &str, replaced: Vec<Habit>, decline: bool) -> bool {
        let gone = if decline {
            self.forget(id)
        } else {
            let before = self.list.len();
            self.list.retain(|h| h.id != id);
            self.list.len() != before
        };
        if !gone {
            return false;
        }
        for h in replaced.into_iter().rev() {
            self.list.retain(|x| x.id != h.id);
            self.list.insert(0, h);
        }
        self.list.truncate(MAX_HABITS);
        true
    }
```

  Run `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

- [ ] **Step 3: the kind.** In `ui.rs` after `NewHabit`:

```rust
    /// A habit took the place of its opposite (docs/specs/2026-11-04-habits.md, "Changed"): the new rule comes to the
    /// Drop just before the card, and the card's button brings the old habit back.
    HabitChanged,
```

- [ ] **Step 4: the runtime.** Next to `Notice` (line 2629):

```rust
/// A habit's card: the habit learned, and the habits it changed (docs/specs/2026-11-04-habits.md, "Changed"), none for a
/// new one. Its "Undo" brings those back as they were.
#[derive(Clone, Debug, PartialEq)]
struct HabitCard {
    id: String,
    replaced: Vec<habits::Habit>,
}

impl HabitCard {
    fn kind(&self) -> TrayKind {
        if self.replaced.is_empty() { TrayKind::NewHabit } else { TrayKind::HabitChanged }
    }
}
```

  `struct Runtime` (line 318): `habit_cards: VecDeque<(u64, HabitCard)>,` and `habit_card: Option<HabitCard>,` (the
  comment: "…in the order found, and the one open."). `on_habit_rule` (line 936), from `let from_cleanup` to its end:

```rust
        let from_cleanup = rule.is_some();
        // Another app's draft of the same forms came first (`merged`): this one adds to that habit, which holds
        // everywhere now, and its row changes with no second card. Habits that held an opposite of its forms went
        // (`replaced`): its card says "Habit changed".
        let habits::Learned { habit, replaced, merged } = self.settings.habits.learn(draft, rule, self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = habit.forms.iter().map(habits::Form::kind).collect();
        tracing::info!(?kinds, n = habit.n, from_cleanup, merged, replaced = replaced.len(), "habit learned");
        self.send_settings();
        if !merged || !replaced.is_empty() {
            self.habit_cards.push_back((n, HabitCard { id: habit.id, replaced }));
            self.show_notice();
        }
    }
```

  In `show_notice` (line 989): `self.habit_cards.retain(|(_, card)| list.iter().any(|h| h.id == card.id));`.
  `show_habit_card` (line 1004) and `send_habit_rule` (line 1016):

```rust
    /// The next habit's card: its rule to the Drop, then the card ("New habit" or "Habit changed"). One forgotten from
    /// the main window meanwhile has none.
    fn show_habit_card(&mut self) {
        while let Some((_, card)) = self.habit_cards.pop_front() {
            let Some(rule) = self.settings.habits.list.iter().find(|h| h.id == card.id).map(|h| h.rule.clone()) else { continue };
            self.tray = Some(card.kind());
            self.habit_card = Some(card);
            self.ui.habit_rule(&rule);
            self.ui.tray(self.tray);
            return;
        }
    }

    /// The rule of the habit whose card is open, before the card goes to a Drop loaded again.
    fn send_habit_rule(&self, kind: Option<TrayKind>) {
        if matches!(kind, Some(TrayKind::NewHabit | TrayKind::HabitChanged))
            && let Some(h) = self.habit_card.as_ref().and_then(|c| self.settings.habits.list.iter().find(|h| h.id == c.id))
        {
            self.ui.habit_rule(&h.rule);
        }
    }
```

  In `change_setting`, the "Learn from me" off block (line 819): a change waiting for its card is taken back, its
  replaced habits return (nothing is learned while off); the comment above the block adds "a change waiting for its
  card is taken back":

```rust
            let mut dropped = 0;
            for (_, card) in self.habit_cards.drain(..) {
                if self.settings.habits.revert(&card.id, card.replaced, false) {
                    dropped += 1;
                }
            }
            if dropped > 0 {
                tracing::info!(dropped, "habits not yet announced dropped");
                self.save_settings();
            }
```

  and the check after it (line 829): `if let Some(card) = &self.habit_card && !self.settings.habits.list.iter().any(|h| h.id == card.id)`.
  In `on_tray_action` (line 2063), the habit's arm and the hold:

```rust
            // "Undo" on a habit's card: the habit goes, and its forms are never learned again; a changed habit's card
            // brings back the habits it replaced, as they were.
            Some(TrayKind::NewHabit | TrayKind::HabitChanged) => {
                if let Some(card) = self.habit_card.take() {
                    self.undo_habit(card);
                }
            }
```

  `if matches!(kind, Some(TrayKind::NoField | TrayKind::NewHabit | TrayKind::HabitChanged)) {`, and after
  `on_tray_action`:

```rust
    /// A habit's card undone: a new habit is forgotten (`HabitForget`: its forms declined); a changed one goes the same
    /// way and the habits it replaced come back (`Habits::revert`). The main window redraws the section.
    fn undo_habit(&mut self, card: HabitCard) {
        if card.replaced.is_empty() {
            self.learn_word(SettingChange::HabitForget(card.id));
            tracing::info!("habit undone");
        } else if self.settings.habits.revert(&card.id, card.replaced, true) {
            self.save_settings();
            self.send_settings();
            tracing::info!("habit change undone");
        }
    }
```

  The module comment's habits bullet adds "; a correction that goes back on a habit three times changes it, and its
  card says so".

- [ ] **Step 5: `strings.ts`.** `TrayKind` (line 4) gains `| 'habitChanged'`; `TRAYS` gains
  `habitChanged: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },`; `en.tray`, after `newHabit`:

```ts
    habitChanged: {
      title: 'Habit changed',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Restored',
      bare: 'CUE writes it this way from now on.',
    },
```

  and `ru.tray`:

```ts
    habitChanged: {
      title: 'Привычка изменилась',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Возвращено',
      bare: 'Дальше CUE пишет так сам.',
    },
```

  `trays.ts` and `controller.ts` need nothing: the card takes `{RULE}` and the done label by its kind, as `newHabit`
  does, and it grows out of the Drop, pauses at the mouse and morphs "Undo" → "Restored" by shared letters like the
  other cards (reduced motion as they do).

- [ ] **Step 6: README and ARCHITECTURE.**
  - README, the **Habits** bullet, after "…can become that app's own habit." (Task 12): "When you go back on a habit in
    its app three times (the full stop it takes off added again, the capital back, the old words back), the habit
    changes: the old one goes, the new one is learned, and a "Habit changed" card says the new rule; its **Undo** brings
    the old habit back as it was and never learns the new one. A habit of every app changes the same way, from three
    times in any one app, and the change holds everywhere. Two opposite habits never live together, so a habit with an
    opposite in some app does not spread to every app."
  - ARCHITECTURE, Habits, step 5, after "…with no second card (`merged` in the log line).": "`adopt` is
    `Habits::learn`, which also takes out the habits holding an opposite of the draft's forms (`Form::opposite`: an end
    mark removed and added, lower case and a capital, a replacement and its reverse) in its app or in every app, not
    declined; when one of them held everywhere, the new habit does too and the opposites of every app go with it. They
    come back in `Learned.replaced`, kept on the card (`HabitCard`). A habit with an opposite habit in some app never
    widens (`Habits::opposed`), by its set or by a second draft." Step 6: "…then the plain tray kind `newHabit` on
    `cue://tray`" becomes "…then the plain tray kind `newHabit`, or `habitChanged` for a card with replaced habits, on
    `cue://tray`", and after "(the habit goes and its hashes join `declined`)": "; on `habitChanged`, `undo_habit` calls
    `Habits::revert(id, replaced, true)`, which also brings the replaced habits back (done label "Restored")". "Learn
    from me" turned off reverts a change waiting for its card without declining.

- [ ] **Step 7: run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; from `app`:
  `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`: green.

- [ ] **Step 8: live, with the owner** (as Task 10, Step 4):
  1. The first start after Tasks 11–13: the log has "habits: a new hash secret" with the numbers; `habits.seen` and
     `habits.declined` are gone from `settings.json`, the habits stay and still shape the dictations; Credential
     Manager lists the entry beside the keys. The next start logs nothing of it.
  2. Telegram has «с маленькой буквы и без точки». In Notepad, take off the full stop alone three times: Notepad's own
     habit «без точки» (its card), and the Telegram row still says «в Telegram». Then in Word, lower case and no full
     stop together three times: the Telegram row says «везде», no card (Notepad's row too: its one form came each time,
     its whole set).
  3. Telegram has «без точки»; add the full stop back three times: the card «Привычка изменилась» with the new rule;
     «Отменить» → «Возвращено», the old row is back and the full stop is never learned there again.
  4. Both themes; reduced motion.
- [ ] **Step 9: report to the owner** what passed, what did not, and ask whether to commit.

---

## Rulings

1. **The app is the history's name for it** ("Telegram Desktop", from the file description), not the exe the spec's
   example shows: the spec says "the dictation's app as the history records it", and the history keeps
   `stored_app(attempt.app)`. It is also the name the section and the rule show. A dictation whose app is not known is
   not counted, and gets only the habits of everywhere.
2. **A habit keeps `forms` (each kind with its detail) instead of one `kind` and one `form` hash.** A merged habit (lower
   case and no full stop) has several forms, and the applied check and widening need their details, not only hashes;
   the hashes are computed from the forms. A replacement's words are always words of the example (a replacement longer
   than 40 characters is no form, and the example falls back to the replacement's own words), so no correction text
   beyond the example reaches disk.
3. **The history entry keeps `habits: [{ id, short }]`, not `[id]`:** "a forgotten habit's chip shows its short name as
   it was" needs the short name on the entry.
4. **`newHabit` is a plain tray kind; the rule goes just before the card on `cue://habit-rule`.** `TrayKind` is `Copy`
   and goes to the page as a bare string; the missing microphone's names and the switched model's name already come
   the same way.
5. **The rule request names the interface language in its instructions;** the user message holds exactly the kind(s),
   the example and the app (the test checks the keys). The model is asked for a rule that starts with the app; on
   widening the "In <app>" / «В <app>» start is cut off, and a rule that does not start so is written anew from the
   template's everywhere form.
6. **Template wording** follows the spec's examples; a scoped habit's template always starts with the app ("В Почта:
   писать «добрый день» вместо «здравствуйте»"), and the spec's replacement example is read as the everywhere form. App
   names are not declined in Russian.
7. **The habits block goes after the cache mark** (after the dictionary block, before the style's block): it changes with
   the app, and the cached prefix stays the one every dictation reads.
8. **"Forget all" also clears the counts (`seen`)**, besides the habits and the declined forms: a form at two would
   otherwise become a habit at the next correction right after the user cleared everything.
9. **A habit's `n` keeps growing** with further corrections of its form in its app (or anywhere, once everywhere), and
   widening adds the other app's count — the row's "how many times".
10. **Only dictations count and take habits.** The watch also runs after a translation (for dictionary words), but a
    translation's correction is not counted, and Translate and "Ask" requests take no habits.
11. **The applied check runs only on a refined text:** a raw fallback marks no habit, though its text may happen to
    show one.
12. **Notices in order:** a habit is "found" when its forms reach three (its number is taken then), though its card can
    open only once its rule came (≤ 5 s). A card waits while the suggestion's capsule is open or shows "undo"; a missed
    suggestion's dot does not hold it back.
13. **"No cleanup connected"** means the chosen connection is a cloud one with no saved key (a local server counts as
    connected). The line is checked when the section is drawn, when the connection changes and when the window gets
    the focus back — not the moment a key is typed on the Keys page (the window has no event for that).
14. **The rule request is not written to usage.csv** (at most one per habit learned, a sentence long).
15. **End mark and first letter need the same last / first word:** one mark swapped for another is neither; a
    correction that shares no word with the paste holds no forms.
16. **"Learn from me" off** drops the habits waiting for their rule and the cards waiting for rest; a card already
    open stays until it closes, and its Undo still forgets.
17. **`RefineContext` gets a hand-written `Debug`** (lengths and a count): it now carries the rules, and its derived
    `Debug` printed the app and the text before the caret.
18. **Hard in the code:** `corrections::changes` cuts a run with more words written than heard to its leading names, so
    "здравствуйте → добрый день" never reached it; the alignment is split out (`corrections::runs`) and habits read the
    raw runs, with the dictionary's behavior unchanged. std's hasher is not stable between versions, so the form hash is
    FNV-1a written out.

### 06.11.2026 addendum

19. **The keyed hash is HMAC-SHA-256, not FNV-1a with the secret mixed in.** Each FNV-1a step can be undone, so the
    hash of a form everyone knows (`digits`, `dash`) gives back the state after the secret, and with it every other
    hash: the secret would protect nothing. HMAC-SHA-256 is written out over `sha2` (already in the workspace; it joins
    `cue-core`), checked against RFC 4231, and cut to its first 8 bytes so `seen` keeps its 16 hex digits. A habit's id
    stays FNV-1a over its forms' hashes, which are keyed already.
20. **The secret lives in `Habits` (`#[serde(skip)]`), and `Form::hash` takes it as a parameter.**
    `SettingChange::HabitForget` declines by hash through `Settings::apply`, which has no other way to reach it. All zero is no secret:
    without one (the system gave no random bytes) nothing is counted, and existing habits still apply. `is_default`
    ignores it, so settings.json never gets an empty `"habits": {}`; `forget_all` keeps it.
21. **The secret is kept as 64 hex digits under `habits-secret` (service "CUE") and read in `Runtime::run`** before the
    first event, beside the Soniox clean-up. A save that fails keeps the new secret for the session (logged); the next
    start then makes another and drops that session's counts again. The leak scan looks for it like a saved key, and
    reads it without ever making one (`kept_habit_secret`).
22. **A new secret makes the habits' ids again** (an id is the hash of keyed hashes). History entries keep the old ids:
    their chips show the short name they carry and never look a habit up, so nothing breaks.
23. **A habit's set is counted under the habit's id**, the spec's "hash of its forms together", in `seen.form` beside
    the forms' hashes; `forget` drops it with the habit. A form counted on a habit of this app or of everywhere belongs
    to no set. Sets may overlap: every habit of another app whose forms all came in the correction counts, and each
    widens at three (a one-form habit's set is its form).
24. **An opposite changes a habit when the new one is learned (`Habits::learn`), not when it is counted:** the spec has
    the opposite count "like any form", which `count` already does, so it only drafts at three as usual. `learn` takes
    out every habit holding an opposite of the draft's forms in the draft's app or everywhere, switched-off ones too
    (two opposites never live together; the spec's "on" changes nothing else, since every correction counts). A habit
    of several forms goes whole when the opposite of one of them is learned (the spec: "the old habit goes"); being
    removed and not declined, its other forms can be learned again.
25. **A habit with an opposite habit in some app never widens**, neither by its set nor by a second draft of its forms
    (that draft only adds to its count): "везде" would put the two together.
26. **"Habit changed" is its own tray kind, `habitChanged`,** a plain string like `newHabit`, with the same look. The
    spec names no done label: it is "Restored" / «Возвращено», since "Undo" brings the old habit back. The habits to
    bring back live on the runtime's card (`HabitCard.replaced`), so its Undo calls `Habits::revert` and writes the
    settings itself instead of going through a `SettingChange`; they come back first in the list, as they were (id, `n`,
    switch).
27. **"Learn from me" turned off with a change waiting for its card** takes the change back (`revert` without declining):
    nothing is learned while it is off, and the replaced habits return.
28. **A merged draft that also replaced habits still gets a card**, "Habit changed"; its Undo removes the merged habit
    and declines its forms (a declined form is in no habit), and brings the replaced ones back.

- Ruling 25 (a habit with an opposite habit in some app never widens) was not built in Task 12: add it in this task, with a test.

- Also, from Task 12's review: in `Habits::count` (habits.rs ~585-587) skip a habit whose `forms` is empty (`!hashes.is_empty() &&`), with a test; rewrap the long last line of ARCHITECTURE's Habits step 3 and README's Habits bullet line to 120 columns.
