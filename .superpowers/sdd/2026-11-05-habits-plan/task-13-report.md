# Task 13 report: opposite habits ("Habit changed")

Status: DONE. Nothing committed, staged or stashed. Live check (Step 8) not done; it is the owner's.

## What was built

- `app/crates/core/src/habits.rs`
  - `Form::opposite`: an end mark flips between removed and added, the first letter flips between lower case and a
    capital, a replacement reverses; digits and a dash return `None`.
  - Private `fn opposes(a, b)`; `pub struct Learned { habit, replaced, merged }`.
  - `Habits::learn` (the plan's body). It takes out every habit holding an opposite of the draft's forms in the
    draft's app or everywhere, switched-off ones too, and does not decline them. When one of them held everywhere, the
    new habit holds everywhere too and the opposites of every app go. A merged draft widens only when no opposite habit
    lives in some app. `adopt` is now `learn(..).habit`; its callers are unchanged.
  - Private `Habits::opposed(forms, id)`; `pub Habits::revert(id, replaced, decline) -> bool`.
  - In `count`'s sets: a habit with an opposite habit in some app is not a set and never widens (ruling 25). A habit
    with empty `forms` is skipped (`!hashes.is_empty()`), the Task 12 review item. `count`'s doc comment says both.
  - Tests: the plan's five, plus `a_habit_of_no_forms_is_no_set` for the empty-forms fix.
- `app/crates/core/src/ui.rs`: `TrayKind::HabitChanged` (JSON `"habitChanged"`), added to the JSON-name test.
- `app/src-tauri/src/runtime.rs`
  - New `HabitCard { id, replaced }` with `kind()`; `habit_cards: VecDeque<(u64, HabitCard)>` and
    `habit_card: Option<HabitCard>`.
  - `on_habit_rule` uses `learn`. It queues a card when the draft is not merged or it replaced habits (ruling 28).
    The log line adds `replaced = <count>`.
  - `show_notice`'s retain, `show_habit_card` (the tray is the card's kind) and `send_habit_rule` (`NewHabit` or
    `HabitChanged`) all take the new card type.
  - "Learn from me" off reverts waiting cards with `revert(.., false)`. The one open-card check is adapted.
  - `on_tray_action`: the arm covers both kinds and calls the new `undo_habit`. Both kinds hold the tray.
  - Module comment and the `a_card_that_replaced_a_habit_says_it_changed` test added.
- `app/ui/src/strings.ts`: `'habitChanged'` added to `TrayKind` and `TRAYS`. EN text: "Habit changed", done label
  "Restored"; RU text: «Привычка изменилась», done label «Возвращено». The EN and RU keys match.
- `app/ui/test/controller.test.ts`: `'habitChanged'` added to `KINDS`, the "Habit changed card" block and the
  Russian test. `trays.ts` and `controller.ts` needed nothing.
- `README.md` and `docs/ARCHITECTURE.md`: the plan's texts are added. The long lines are rewrapped to 120 columns:
  ARCHITECTURE Habits step 3's last line and the README Habits bullet.

## Checks (all green)

- `cargo build --workspace --all-targets`: passed.
- `cargo test --workspace`: cue-core 436 passed; cue 204 passed and 1 ignored; the other crates passed.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `npx vitest run` (from `app`): 403 of 403 passed in 15 files.
- `npx tsc --noEmit -p tsconfig.json`: exit 0.
- TDD: the core and UI tests failed first (`opposite`, `learn`, `revert` undefined; 6 vitest failures) and pass now.

## Deviations from the plan

1. "Learn from me" off reverts waiting cards **newest first** (`drain(..).rev()`). Suppose a later waiting change
   replaced an earlier waiting habit. Reverting oldest first would fail on the earlier one, because it is already gone.
   The later revert would then bring it back, and a habit would be left learned while learning is off. Newest first
   puts the list back as it was.
2. ARCHITECTURE step 6: the plan's "(done label "Restored")" is merged into the existing `hold_tray` sentence, which
   now reads "the done label ("Removed", or "Restored" on `habitChanged`)", so the label is not named twice. Step 5
   also gets one sentence for ruling 28: "A merged draft that also replaced habits still gets a card."
3. `adopt`'s old `self.list.retain(|h| h.id != habit.id)` before inserting is gone, as in the plan. It could never
   match, because the branch is reached only when no habit has that id.
4. Files: `runtime.rs`, `strings.ts`, `controller.test.ts` and `ARCHITECTURE.md` use CRLF, and it is kept.
   `habits.rs`, `ui.rs` and `README.md` use LF, and it is kept. No backslash-n escapes went into string literals.

## Self-review

- No text in logs. The new lines are "habit learned" (adds `replaced = <count>`), "habit undone", "habit change
  undone" and "habits not yet announced dropped" (`dropped = <count>`). `Learned` and `HabitCard` derive `Debug` over
  `Habit`, whose hand-written `Debug` prints lengths only.
- "Learn from me" gates everything. `on_habit_rule` drops a draft while learning is off. Off reverts the waiting
  changes without declining (ruling 27). An open card's Undo still works (ruling 16).
- Answers out of turn change nothing:
  - `on_tray_action` acts only when the last tray kind is a habit's and a `habit_card` is held.
  - `revert` returns false for an id no longer listed.
  - A card whose habit was forgotten, or was cleared by "Forget all", is closed by the existing check.
  - A flow card that takes the tray clears `habit_card`, as for `NewHabit`.
- Two opposites never live together. `learn` removes the opposites in the draft's app and everywhere. An opposite in
  another app stays, as that app's own habit. A habit with such an opposite neither widens through `count` nor
  through a merged draft.

## Concerns, for the owner

- An empty file `t13_dummy` was left in the session scratchpad, because the delete hook blocked removing it. It is
  harmless.
- `Habits::listed` has no caller left outside tests and doc comments, since the runtime now reads `Learned.merged`. It
  is kept because it is public and named in the docs.

## Fix round 1

1. **An Undo wins over a habit learned or widened since then (Important).**
   - `habits.rs` gets a new private `fn clash(a, b)`. It is true when two habits oppose each other in the same app,
     or when either holds everywhere.
   - `Habits::revert` now returns `Option<Vec<Habit>>`: `None` when there is no habit `id`, otherwise the habits it
     removed.
   - Every replaced habit is always restored. Before each one goes back, every listed habit that clashes with it is
     removed. Removed habits are not declined.
   - Runtime `undo_habit` drops any waiting cards of the removed habits, saves, sends settings, and logs
     `removed = <count>`.
   - Tests:
     - `an_undo_takes_out_a_habit_learned_since_that_opposes_the_one_it_brings_back` follows your trace. R (Telegram,
       lower case + no stop) is replaced by C (stop added); then D (capital) is learned; Undo on C returns `[D]`, the
       list is `[R]`, and only C's form is declined.
     - `an_undo_takes_out_a_habit_widened_since_that_opposes_the_one_it_brings_back` covers the Word route. A Word
       habit {stop added} is held back by R. C {capital} replaces R. Word's habit then widens from Notepad. Undo
       returns the widened habit and the list is `[R]`.
2. **Learn off with a merged card that also replaced habits.**
   - `Learned` gains `was: Option<Habit>`, the merged habit before the draft added to it. `HabitCard` gains `was` too.
   - New `HabitCard::take_back(self, &mut Habits)` calls `revert(id, was + replaced, false)`. A merged habit comes back
     as it was (app, n, rule); a new one is simply removed. The learn-off loop uses it, newest first.
   - The card's Undo is unchanged (ruling 28: it removes the merged habit and declines its forms).
   - Tests:
     - Core: `a_merged_change_taken_back_leaves_the_habit_it_added_to_as_it_was` (also checks `learned.was`).
     - Runtime: `a_merged_change_taken_back_restores_the_habit_it_added_to`.
3. **Newest-first order is pinned.** `changes_are_taken_back_newest_first` sets up R, then C1 replacing R, then C2
   replacing C1. Newest first leaves `[R]`. Oldest first fails the first revert and leaves C1 behind.
4. **Docs and `listed`.**
   - `count`'s doc comment is reflowed to 120 columns.
   - `Habits::listed` is removed. Its one test use is now an inline closure, and the doc mentions in `Learned` and
     `learn` are reworded.
   - ARCHITECTURE step 5 now says:
     - the runtime calls `Habits::learn` and reads `Learned.merged`;
     - `Learned.was` keeps the merged habit as it was;
     - `adopt` keeps only `.habit`.
   - ARCHITECTURE step 6 now describes:
     - the Undo-wins removal;
     - `HabitCard::take_back` with `was`.
   - Steps 5 and 6 are reflowed to 120 columns.
5. **Whole habits compared.** `a_change_of_a_habit_of_everywhere_holds_everywhere` now compares whole habits, sorted
   by id, against `before`. The plan's test with `[old.clone()].as_slice()` is kept. The new tests use
   `std::slice::from_ref`, which clippy's `cloned_ref_to_slice_refs` asks for.

### Commands (all green)

- `cargo test --workspace`: cue-core 440 passed (4 new); cue 205 passed, 1 ignored (1 new); the other crates pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean. The first run flagged 5 `cloned_ref_to_slice_refs`
  in the new tests; they are fixed.
- `npx vitest run` (from `app`): 403 of 403 passed in 15 files.
- `npx tsc --noEmit -p tsconfig.json`: exit 0.
- Line endings are kept: CRLF for `runtime.rs` and `ARCHITECTURE.md`, LF for `habits.rs`. Nothing committed.
