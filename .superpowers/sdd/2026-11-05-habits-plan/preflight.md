# Pre-flight: habits plan (2026-11-05)

Plan `docs/specs/2026-11-05-habits-plan.md`, spec `docs/specs/2026-11-04-habits.md` (binding).

**How it was checked.** The plan's Rust for Tasks 1–5 was pasted verbatim into a scratch copy of `cue-core` and
`cue-engines` (outside the repo, with its own target dir) and built:
- Tasks 1–5 together: `cargo test` passes (core 411, engines 201). All 33 new tests are green, and so are the existing
  `corrections` tests after the `runs` refactor. `clippy --all-targets -D warnings` is clean.
- Task 1 on its own: the tests pass, but **clippy fails** (B-3 below).
- FNV-1a hashes and the habit id were recomputed by hand and match the plan (`620301a9632d813f`, `85657e6730abe435`,
  `94128fbe83ca55c4`, `3e6b38c8e27e485e`, `84a975dc80118b92`, `h9be169a0a7be8fdd`).
- Tasks 6–9 (runtime, TS) were checked by reading against the real files. They were not compiled.

Severity: **B** = blocking (a spec or constraint violation, or a broken release step the plan's tests won't catch).
**S** = should fix before dispatch (a check the task mandates will fail as written). **N** = note.

## 1. Pairs of tasks that share a file or an interface

| Tasks | Producer → consumer (names) | Agree? |
|---|---|---|
| 1→2 | `Correction{old,new,found}`, `Found{form,old,new}`, `Correction::example(&[&Found])`, `Form::rank`, `cut`, `fnv1a` → used by `Habits::count`, `adopt`, `part` | Yes (compiled) |
| 1→6 | `corrections.rs`: Task 1 replaces `changes` with `runs`; Task 6 adds `Offers::waiting/open` after `view` | Yes, the changes don't overlap |
| 2→3 | `Habits` (`is_default`, `switch`, `forget`, `forget_all`, `adopt`), `Draft`, `Form::hash` → `Settings.habits`, `SettingChange::Habit*` arms | Yes (compiled) |
| 2→4 | `habits::one_line` (made `pub(crate)` in T4), `MAX_RULE_CHARS` → `refine::habits_prompt` | Yes (compiled) |
| 2→5 | `RuleAsk`, `RuleText`, `rule_messages(&RuleAsk,&str)`, `read_rule(&str)`, `Draft::ask` → `cue_engines::habit` | Yes (compiled) |
| 2→6 | `Habits::count(app,&Correction,today,lang)->Counted{new,widened,grown}`, `adopt(Draft,Option<RuleText>,lang)->Habit`, `for_app`, `marks(&[Habit],&str)`, `Form::kind`, `RULE_WAIT_MS` → runtime | Yes, names and signatures match |
| 2→9 | `history::HabitMark{id,short}`, `Entry.habits` → `EntryView.habits: Vec<HabitMark>` | Yes |
| 2 / 6 / 8 / 9 | `Entry` literals at `commands.rs:849`, `ipc.rs:618,649,703`, `runtime.rs:2143,2159,2516`, `leak.rs:1124`, `history.rs:450` | Yes, all exist as listed |
| 3→6 | `SettingChange::HabitForget(String)` → card Undo via `learn_word` | Yes |
| 3→8 | JSON `habitOn{id,on}` / `habitForget:"h…"` / `{"kind":"habitsForget"}` (adjacently tagged) → TS `SettingChange` union, fixture `settingsSet` | Yes. The T3 test pins the JSON. TS narrowing is fine because the fixture returns before its generic `c.value` line |
| 3→10 | `leak.rs` scans each habit's `before/after/rule/short` → T10 Step 3 "the leak scan still passes on a settings.json with a habit" | **No**, see B-1 |
| 4→5 | Both edit `claude.rs`, `chat.rs` and `openrouter.rs` (`system_blocks` signature; `thinking_for`, `effort_for`, `post`, `TITLE` made `pub(crate)`) | Yes (compiled in sequence) |
| 4→6 | `RefineContext::with_habits(Vec<String>)`, `Job::Clean \| Job::Rewrite` gate → `refine()` | Yes |
| 5→6 | `habit::rule(&Client, Option<&str>, &RefineSettings, &RuleAsk, lang:&str, Duration)` → `ask_rule` spawns with `self.lang.code()` (`&'static str`, `lang.rs:43`) | Yes |
| 6→7 | `TrayKind::NewHabit` (serde camelCase `"newHabit"`), `cue://habit-rule` (string), `trayAction`/`trayClose` → TS `'newHabit'`, `listen<string>`, `ctl.habitRule` | Yes |
| 6→8 | `send_settings()` / `cue://settings` → `page.pushed()` → `habits.render()` | Yes. The new `pushed()` drops `tray.learnChanged()` and T8 Step 8 deletes it: consistent |
| 6→9 | `habits_applied` → `on_finished` puts `e.habits = marks` → `EntryView::of` clones it | Yes |
| 7→8/9 | `drop.js` `IC.habit` → `icons.ts` `icon('habit')` through `{...CUE().IC, ...OWN}`, once `IconName` gains `'habit'` (T8 Step 5) | Yes |
| 8→9 | T8 adds `habitsApplied` to `text.ts`, `habits: []` to the fixture's `entry()`, `'habit'` to `IconName` → T9's `habitChips` and test | Yes (T9 depends on T8, and the order holds) |
| 6 / 8 | `ipc.rs`: T6 adds `Ui::habit_rule` next to `model_name` (l.376); T8 adds `SettingsView.habits`, `HabitView` | Yes, the changes don't overlap |

## 2. Per task: does its own text agree with itself?

| Task | Tests vs code | Files created vs touched | Verdict |
|---|---|---|---|
| 1 | 12 tests pass against the code (run). Hashes are correct | `habits.rs` is created here; T2 appends to it and widens its imports, as T1 says | **Step 6 fails**: B-3 |
| 2 | 16 habits tests + the history test pass (run). The template strings, `unscoped`, the 300 cap, the 30-day prune and `n = 6/7` on widening are all correct | OK | Green |
| 3 | 2 settings tests pass (run, core part). `leak.rs` was not compiled; its names were read against `leak.rs:67/87/136/176-183` | OK | Green, but B-1 (behavior) |
| 4 | 4 tests pass (run). With no habits the body is unchanged (existing tests green) | OK | Green |
| 5 | 3 tests pass (run). The bodies equal `anthropic_body` / `chat_body`. The 200 ms hang returns `None` | `habit.rs` new, `lib.rs` | Green |
| 6 | `first_notice` table checked by hand (6/6). The `Offers` test checked by hand against `Stage` | `ui.rs`, `corrections.rs`, `ipc.rs`, `runtime.rs` | Logic gap: B-2. Constraint: S-7 |
| 7 | The new card tests match `trayHtml` and the controller's `done` path (same pattern as the "Copied" tests at l.936) | 5 files | **Breaks an existing test**: S-2 |
| 8 | New `window-habits` tests match `habits.ts` by reading | `habits.ts` new | **Collisions and broken existing tests**: S-3 to S-6 |
| 9 | The test matches the code. `icon` is already imported in `history.ts:19` | OK | Green by reading |
| 10 | Docs only | README, ARCHITECTURE | N-8 |

## 3. Conflicts with the Global constraints or the spec

**B-1 (T3, T10). The leak scan will report false leaks on any template habit.**
- `gather` adds `rule` and `short` with `MIN_NAME_CHARS = 4`.
- The matcher is `ascii_case_insensitive` exact bytes (`leak.rs:338`).
- Template short names and parts are string literals in `cue.exe`: "no full stop", "без точки", "digits", "dash", "тире", "lower case" and others.
- An everywhere template rule ("Numbers in digits") matches its literal case-insensitively.
- So `-ScanBuild` fails on the owner's machine once a habit came from the template, and T10 Step 3 fails.
- The spec says only the example is correction text.
- *Ruling:* scan only `habit.before` and `habit.after`. Drop `rule` and `short` from the loop and from the module comment.

**B-2 (T6). A waiting habit card never opens after the suggestion's capsule closes.**
- `first_notice` holds the card while the capsule is open (`Open | Added`).
- The arms that close the capsule don't call `show_notice()`: `SuggestDismiss` (Open→Missed, `runtime.rs:534`), `SuggestDone` (Added→None, l.539), `SuggestUndo` and a refused `SuggestAdd`.
- The card then waits for an unrelated `Input`, in practice the next dictation.
- This contradicts the spec ("the one found first goes first") and live check 7 ("the card after it folds").
- *Ruling:* in T6 Step 6, add `self.show_notice();` at the end of those four arms.

**S-7 (T6, Global constraints). Turning "Learn from me" off leaves unannounced habits behind.**
- The constraint reads "`learn.enabled` off: … habits waiting for their rule or their card are dropped".
- `on_habit_rule` adopts and saves the habit before its card waits for rest.
- The learn-off block only clears `habit_cards`, so those habits stay learned and are never announced. The spec says the habit "says so once".
- *Ruling (smallest):* in the learn-off block, drain `habit_cards` and `list.retain` out those ids (not declined), then `save_settings()` and `send_settings()`. The other option is a ruling that amends the constraint to "cards are dropped, the habit stays".

**N-1. Spec deviations already ruled (Rulings 1–3, 6, 12).** Nothing to fix, as long as the owner accepts them:
- `app` is the history name, not the exe.
- A habit has `forms[]` instead of `kind/form`.
- An entry has `habits:[{id,short}]` instead of `[id]`.
- A scoped replacement template starts "В Почта: …".
- A missed suggestion's dot does not hold a card back. The spec says the two "never show together", and a dot plus a card do show together.

**N-2 (T2). Widening a merged habit counts its `n` twice.**
- Say the first form reaches 3 in app B: the habit widens and adds 3.
- The second form in the same correction then matches the now-everywhere habit and adds 1 more.
- Its own `seen` row in B (n=2) stays until the 30-day prune.
- The spec only says "its count adds up". This is acceptable. If a fix is wanted: after a widen, `continue` past the forms already in that habit.

## 4. What a reviewer would call a defect

**S-3 (T8). `.hempty` is already the History's empty line** (`window.css:341`, `history.ts:190`).
- The History section comes first in the DOM, so `$('.hempty')` finds the History's line.
- The tests "an empty list says…" (text) and "Forget all… shows the empty line" (hidden) read the wrong element.
- The plan's CSS rule `.hempty{margin:0;…animation:msIn}` also restyles the History's line.
- *Ruling:* rename the Habits class to `habempty`, in `habits.ts`, the CSS and the 3 test selectors.

**S-2 (T7). Adding `'newHabit'` to `KINDS` breaks "every card in every language"** (`controller.test.ts:1037-1054`).
- That test builds the expected body without replacing `{RULE}`.
- `trayHtml` replaces `{RULE}` with `''`.
- *Ruling:* add `.replace('{RULE}', '')` to the test's expected chain in T7 Step 1.

**S-4 (T8). An existing assertion pins the old General page** (`window-settings.test.ts:336`).
- It expects `['Start at sign-in', 'Learn from me']`, and T8 doesn't touch it.
- *Ruling:* change it to `['Start at sign-in']`. Then drop the plan's new near-duplicate test "the General page has 'Start at sign-in' only", or keep just its `[data-sw="learn"]`-is-null line in the l.329 test.

**S-5 (T8). The header-links test pins three links** (`window-today.test.ts:128`).
- *Ruling:* expect `['Today', 'History', 'Dictionary', 'Habits']`.

**S-6 (T8). The History test counts `.bsec .bmore` as 1** (`window-history.test.ts:93-94`).
- The Habits section adds its own `.bmore`, so the counts become 2.
- *Ruling:* scope both lines to `.bsec[data-scr="history"] .bmore`. The other option is to give the Habits footer its own class (`hmore`), which also keeps it out of the `.bmore button` focus rule at `window.css:429`, if that matters.

**B-3 (T1). Step 6 clippy fails on Task 1 alone.**
- `rank`, `example`, `cut` and `CONTEXT_WORDS` have no non-test user until T2.
- Confirmed: 4 `dead_code` errors in the lib, 1 in lib-test.
- *Ruling:* move `rank` into T2's code block, and let T1 Step 6 run clippy without `--all-targets`… Simplest: in T1 Step 6, say "clippy is run at the end of Task 2: until then `rank`, `example` and `cut` are used only by the tests". Alternatively, put `#[cfg_attr(not(test), allow(dead_code))]` on the T1 helpers and remove it in T2. (Rated B only because a literal executor stalls at this step. It is trivial.)

**N-3. Duplicated logic.**
- `habits::items_or_nothing` is `settings::learned_or_nothing` (l.254) made generic. Make one generic `pub(crate)` helper in `settings.rs` and use it in both places.
- `habits::one_line` repeats the one-line check inside `refine::is_term` (l.109). Extract `refine::one_line` and call it from both.
- `habit::chat_body` repeats the budget and `response_format` logic of `chat::request_body` and `openrouter::request_body_styled`. Acceptable, since the schema differs; a comment saying so would help.
- `HabitsView.forgetAllPressed` copies the "Clear history" double-press logic. Acceptable: it already shares `CLEAR_ARMED_MS`.

**N-4 (T5). The log test could pass vacuously.**
- The loop only checks lines with target `cue_engines::habit`.
- *Ruling:* add `assert!(lines.iter().any(|l| l.contains("cue_engines::habit")))` before the loop.

**N-5 (T6). An Undo can be cut short by the next queued card.**
- `on_tray_action` → `show_notice()` with `tray` already `None`.
- A second habit card waiting in the queue opens at once and replaces "Removed" before `DONE_HOLD_MS`.
- Rare: it needs two habits learned back to back. *Ruling:* accept, or skip `show_notice` in the NewHabit arm and let `TrayClose` trigger it.

**N-6 (T6). No runtime test** covers counting → rule → card → Undo → applied marks. Only `first_notice` is tested. This is acceptable because the logic lives in core, and the live check covers the rest.

**N-7 (T8).** When the core refuses a forget, `render(false)` draws the row back with the "new" highlight, because `drawn` is non-empty. This is cosmetic.

## 5. Spot checks against the real code (anchor → found)

| Task | Anchor | Real code |
|---|---|---|
| 1 | `changes` l.51, `MAX_RUN` / `clean` / `key` / `looks_like_name` private, `MAX_TOKENS` pub | Matches (l.10, 40, 44, 51, 104; `MAX_TOKENS` pub l.8) |
| 1 | `lib.rs` "after `pub mod flow;`" | Matches |
| 2 | `history::date_minus` l.200, `Entry` l.38, `entry(ts,mode,text)` helper l.448, `entry_debug_has_no_text` l.1018 | Matches |
| 3 | `section_or_default` l.409, `SettingChange` l.675 (adjacently tagged), `Debug` l.715, `apply` l.754, `Settings` l.859 (derived `Debug`), `Default` l.940 | Matches. `Settings`'s derived `Debug` prints `Habits` through the hand-written `Habit`/`Seen` impls: no text |
| 3 | `leak.rs` `DictionaryTerm` l.67, `summary` l.135, `gather` learned l.180, the test pins only `"saved keys 1, history texts 1"` | Matches |
| 4 | `system_blocks` l.103 (called at l.75 in `request_body_styled`), `warm_body` l.137, `chat.rs:46`, `openrouter.rs:214` | Matches |
| 5 | `thinking_for` l.20, `effort_for` l.31, `THINKING_HEADROOM` `pub(crate)`, `messages_post` `pub(crate)` l.378, `MessageResp`/`Block` pub, `chat::post` l.100, `openrouter::TITLE` l.24, `stream`/`reason` `pub(crate)`, `model_profile` pub | Matches. `s.address()` + `/messages` equals `API_URL`, since `base_url` is `https://api.anthropic.com/v1` |
| 6 | `CoreEvent::Corrected` l.150, `watch_next` l.286, `field_stop: None` l.385, `InsertDone` l.587, `watch_field` l.767, `on_corrected` l.785, `show_offer` l.812 (5 callers: 474/513/644/805/1846), `at_rest` l.2342, `stored_app` l.2229, `local_today`, `on_tray_action` l.1832 (an exhaustive list, not `_`), `Effect::Tray` l.971 | Matches. The new arm must go into the explicit list, as the plan does |
| 6 | `RefineOutcome::Refined` (flow.rs:45, `PartialEq`), `dictation_entry -> Option<Entry>`, the key lookup in `refine` l.1270 | Matches |
| 7 | `IC` with `undo`/`book` in `drop.js`, `TrayLook.act` union, `trayHtml` signature, `private model` l.76, `modelName` l.278, `trayHtml(...)` l.285, `DONE_HOLD_MS` l.39, `cue://model-name` `main.ts:65`, `KINDS` l.24 | Matches. S-2 |
| 8 | `section()`, links, `col.append` in `page.ts:432-467`, `took` l.382, `pushed` l.402, focus listener l.289, `generalRows` l.803 (return l.818), `learnChanged` l.299, `onClick` learn l.465, `CLEAR_ARMED_MS` exported l.56, `fill`/`plural`/`Forms` in `text.ts`, `globe` in `OWN`, `settings_view_json_names` (sorted: `habits` goes after `dictionary`) | Matches. S-3 to S-6 |
| 9 | `EntryView` `ipc.rs:171` (`Eq`; `HabitMark` is `Eq` too), `details` `history.ts:532`, `inner.append(this.player…)` l.549, `.hraw` CSS l.181 | Matches |
| 10 | README l.85, 255, 257, 506, 706; ARCHITECTURE "## Corrections" l.492 | Matches. N-8: README l.257 also says "(**Learn from me** in the General settings…)". Update it to "in the Habits heading" too |

## Totals

Blocking: 3 (B-1 leak scan, B-2 card after capsule, B-3 T1 clippy). Should fix: 6 (S-2 to S-7). Notes: 8 (N-1 to N-8).
