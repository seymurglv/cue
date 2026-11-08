# Task 6 report: the runtime (counting, the rule, the card at rest, Undo, widening, the applied habits)

Status: DONE. No commits, no git add/stash/checkout.

## What was done

- `app/crates/core/src/ui.rs`: `TrayKind::NewHabit` (JSON `"newHabit"`), added to `every_variant_has_its_json_name`.
- `app/crates/core/src/corrections.rs`: `Offers::waiting()` and `Offers::open()`, plus the test
  `waiting_and_open_say_where_the_suggestion_is`.
- `app/src-tauri/src/ipc.rs`: `Ui::habit_rule(&str)` sends `cue://habit-rule` to the Drop.
- `app/src-tauri/src/runtime.rs`:
  - imports: `VecDeque`, `cue_core::habits`, `RefineOutcome`, `cue_engines::habit`;
  - `CoreEvent::Corrected` gains `app: Option<String>`; new `CoreEvent::HabitRule { draft, rule }`;
  - `watch_next` carries the habit app: `Some` only for a dictation with a known app, `None` for a translation;
  - new state: `habits_sent`, `habits_applied`, `drafts`, `habit_cards`, `habit_card`, `notices`, `offer_found`;
  - `on_corrected` counts habit forms (`count_habits`) after the dictionary candidate, and always ends with
    `show_notice()`;
  - `count_habits` calls `Habits::count` and saves. It sends `cue://settings` when a habit grew or widened, and asks
    for the rule when a draft came out;
  - `ask_rule` spawns `habit::rule` inside `tokio::time::timeout(RULE_WAIT_MS)` and also passes the same wait as the
    function's own timeout (ruling 3);
  - `on_habit_rule` adopts the draft (the cleanup's rule or the template), saves, logs "habit learned" with kinds and
    numbers only, sends `cue://settings` and queues the card. It is dropped when the draft is gone or learning is off;
  - `send_settings()` and `cleanup_key()` helpers; `refine` uses `cleanup_key()`; `learn_word` uses `send_settings()`;
  - `show_offer` → `show_notice` (all callers renamed). It opens either the waiting suggestion or the next habit card,
    the one found first (`first_notice`), and only at rest. `show_habit_card` sends the rule, then the
    `NewHabit` tray, and skips habits forgotten meanwhile. `send_habit_rule` resends the rule on `UiReady`;
  - free `Notice` and `first_notice` next to `at_rest`, with the test `the_notice_found_first_opens_first`. The
    `at_rest` doc now covers both notices;
  - `on_tray_action`: `NewHabit` → `learn_word(SettingChange::HabitForget(id))` (declines its forms, sends settings),
    log "habit undone";
  - `TrayClose` and `Effect::Tray` clear `habit_card`;
  - `change_setting`, "Learn from me" off: clears `drafts` and `habit_cards`. Saved habits whose card was never shown
    are taken out of `habits.list` (not declined) and the file is written again (ruling 2);
  - Suggest arms: `show_notice()` after the capsule closes in `SuggestDismiss`, `SuggestDone`, `SuggestUndo` and the
    refused `SuggestAdd` branch (ruling 1);
  - `refine`: a dictation's Clean/Rewrite job, with learning on, sends the habits of `stored_app(attempt.app)` (or of
    everywhere) through `ctx.with_habits(...)` and remembers them in `habits_sent`;
  - `on_refined`: when the outcome is `Refined`, `habits::marks(sent, text)` goes into `habits_applied`, otherwise
    nothing;
  - `on_finished` Commit: the entry takes the marks of its own dictation id;
  - a module comment bullet about habits.

## Test evidence

All commands were run from `C:\Users\seymurglv\Desktop\cue-dev`.

- Step 1, red: `cargo test -p cue-core` failed with `no method named waiting/open found for struct Offers` and `no
  variant ... NewHabit`. `cargo test -p cue --no-run` failed with `cannot find function first_notice` and `cannot find
  type Notice`.
- `cargo build --workspace --all-targets`: `Finished dev profile`, no warnings.
- `cargo test --workspace`: every suite ok. The results were 199, 23, 38, 418, 201 and 265 passed, plus three empty
  suites, with 0 failed. The new tests passed: `runtime::tests::the_notice_found_first_opens_first`,
  `corrections::tests::waiting_and_open_say_where_the_suggestion_is` and
  `ui::tests::every_variant_has_its_json_name`.
- `cargo clippy --workspace --all-targets -- -D warnings`: `Finished`, clean.

## Deviations, and why

1. `use std::collections::VecDeque;` is a new import line. There was no `std::collections` import to extend.
2. The brief counts "five callers" of `show_offer`. There were four call sites (`feed`, `TrayClose`, `on_tray_action`,
   `IdleHidden`) plus `on_corrected`, which was rewritten. All of them now call `show_notice`.
3. Ruling 1: `show_notice()` sits inside each Suggest arm's "it changed" branch, not unconditionally at the end of the
   arm. A Drop answer that comes out of turn then changes nothing.
4. Ruling 2: unannounced saved habits are removed in `change_setting`'s Learn-off block, followed by
   `save_settings()` and the log line "habits not yet announced dropped" (`dropped = N`). `send_settings()` is not
   called: `change_setting` is only reached from `CoreEvent::Settings`, whose reply already carries the new view.
5. The `show_notice` doc comment also names "the suggestion's capsule closing" among its triggers (ruling 1).
6. The brief's Learn-off comment was extended to describe the dropped saved habits.
7. `CoreEvent::Corrected`'s doc gained the `app` sentence inside the existing comment.

## Files

- C:\Users\seymurglv\Desktop\cue-dev\app\crates\core\src\ui.rs
- C:\Users\seymurglv\Desktop\cue-dev\app\crates\core\src\corrections.rs
- C:\Users\seymurglv\Desktop\cue-dev\app\src-tauri\src\ipc.rs
- C:\Users\seymurglv\Desktop\cue-dev\app\src-tauri\src\runtime.rs

All four keep CRLF line endings. A `sed -i` stripped them once in runtime.rs; they were restored with a byte-level
replace, and the file was checked at 3191 CRLF out of 3191 lines.

## Self-review

- **No text in logs.** The new lines are "habit forms counted" (static kind names and counts), "habit learned" (kinds,
  `n`, `from_cleanup`), "habit undone", "habits not yet announced dropped" (a count) and the habit-rule emit failure
  (an error only). Rules, examples and field text never reach a log or a `Debug`.
- **"Learn from me" off gates everything.** `on_corrected` returns before any counting, and `count_habits` is only
  reached from there. `watch_next` is not set when learning is off. `on_habit_rule` drops a late rule. `refine` sends
  no habits, so the entry gets empty marks. Turning learning off clears drafts and queued cards and removes the
  unannounced saved habits. An open card stays, and its Undo still forgets (per the brief).
- **One card at a time.** A habit card sets `self.tray`, and `at_rest` requires no tray. `first_notice` never opens a
  card while the capsule is open (ring or undo). A waiting suggestion waits behind an open card until `TrayClose`. A
  missed suggestion's dot does not hold a card back (as designed).
- **Answers out of turn change nothing.** `TrayAction` forgets a habit only while the last tray sent is `NewHabit` and
  `habit_card` is set (`take`). The Suggest arms act only when `Offers` changed. A habit forgotten from the main
  window while its card waits is skipped. If its card is already open, Undo finds nothing to forget (`apply` returns
  false, no settings sent).
- **Invariant.** `habit_card` is `Some` only while `tray == Some(NewHabit)`. It is cleared on `TrayClose`, on every
  flow `Effect::Tray`, and by the Undo action. The model-ready path only sets the tray when none is open.

## Concerns (for Task 7 and the coordinator)

1. `on_tray_action` closes the tray in the core at once (`tray.take()`), then calls `show_notice()`. The spec has the
   card show "Removed" for a moment before it closes, so a queued notice (another habit card, or a waiting
   suggestion) can be sent while the Drop is still showing "Removed". The same holds for the existing cards. If the
   Drop then sends `TrayClose` for the old card after "Removed", it would clear the newly opened card's
   `tray`/`habit_card`. Task 7 should either not send `TrayClose` after an action (the current contract:
   "the window does not report that itself"), or the core should delay `show_notice` after Undo.
2. If the open card's habit is forgotten from the main window and the Drop then reloads (`UiReady`), the
   `NewHabit` tray is resent without a rule. This is a rare edge case and has no effect on data.
3. The marks are computed against the refined text. The history entry stores `final_text` (the text actually
   inserted), which is normally the same text.

---

# Fix round 1

Status: DONE. No commits. All changes are in `app/src-tauri/src/runtime.rs`, and its CRLF line endings are kept (3333 of 3333 lines).

## Changes

1. **The Drop stays busy while "Removed" or "Copied" shows and folds** (Important).
   - `TRAY_DONE_MS = 1100 + 430` (`:136`) is the page's `DONE_HOLD_MS` plus the tray card's fold.
   - New event `CoreEvent::TrayDone { generation }` (`:168`).
   - New fields `tray_closing: Option<u64>` and `tray_gen: u64` (`:300`).
   - `on_tray_action` (`:2046-2073`) keeps the kind it took. For `NoField` and `NewHabit` it calls `hold_tray()`
     (`:2078`) instead of `show_notice()`. `hold_tray` bumps the generation, sets `tray_closing`, and spawns a sleep
     that sends `TrayDone`.
   - The `TrayDone` arm (`:666`) clears the hold and calls `show_notice()` only when its generation is current, so a
     stale timer does nothing.
   - `show_notice` (`:978`) treats a hold like an open tray. The model-ready card (`:722`) also waits for the hold.
   - The hold ends early when the card's cross sends `TrayClose` (`:557`) or the flow sends any `Effect::Tray`
     (`:1184`).
   - `habit_card` is still cleared at the action (`take`). The invariant "Some only while tray == NewHabit" holds. The
     page contract is unchanged.
2. **Forgotten queued habits never block or jump the queue.** `show_notice` drops ids from `habit_cards` that are no
   longer in `habits.list` before it computes the order (`:984`). This covers HabitForget, HabitsForget, and the
   `MAX_HABITS` cut.
3. **"Forget all" also drops pending habits.** `change_setting` clears `drafts` and `habit_cards` on
   `SettingChange::HabitsForget` (`:776`). It does this before the change is applied, so a habit whose rule is still
   pending is dropped even when `forget_all` finds nothing saved.
4. **Forgetting the open card's habit closes the card.** At the end of `change_setting` (`:828`): if the habit of the
   open card is gone, `habit_card` and `tray` become `None`, `Tray(None)` is sent to the Drop, and `show_notice()`
   runs.
5. **The habit gate is now a pure function.** `fn habits_for(mode: Mode, learn_on: bool, job: Job) -> bool`
   (`:2592`) is used by `refine` (`:1496`). The test `habits_go_only_with_a_dictations_cleanup` (`:3306`) checks
   Dictate with learning on and Clean/Rewrite (true), and returns false for learning off, `Job::Translate`, Translate
   mode and Ask.

## Commands and output (from `C:\Users\seymurglv\Desktop\cue-dev`)

- `cargo build --workspace --all-targets`: `Finished dev profile`, no warnings.
- `cargo test --workspace`: 200, 23, 38, 418, 201 and 265 passed, plus three empty suites, with 0 failed.
  `runtime::tests::habits_go_only_with_a_dictations_cleanup ... ok` and
  `runtime::tests::the_notice_found_first_opens_first ... ok`.
- `cargo clippy --workspace --all-targets -- -D warnings`: `Finished`, clean.

## Notes

- After a card is closed from the main window (item 4), the next notice may open during the card's 430 ms fold. That
  is the same as the cross's `TrayClose` today; the ruling asked for a hold only after an action.
- A Drop reload during a hold has no card to show. The hold simply runs out within 1.53 s, and then what waits opens.
- The scratchpad edit script is `C:\Users\SEYMUR~1\AppData\Local\Temp\claude\C--Users-seymurglv-Desktop-cue-dev\a5708eff-3f42-4a62-ae43-4196090a754a\scratchpad\task6_round1.py`. It was needed because a heredoc broke on the quotes.
