# Final fix wave: report

Status: all ten items fixed. No commits, no git add/stash/checkout/reset. Every check is green and prints no warnings.

## Per item

1. **Sanity check with a replacement habit (Critical).**
   - `app/crates/core/src/refine.rs:148`: `RefineContext.said` holds the new words of the sent habits' replacements
     (`Form::Replacement.to`). `Debug` prints only their count. `with_habits` (:176) now takes `&[Habit]` and fills
     `habits` (the rules) and `said`.
   - :320 `sanity_ok_with(raw, refined, terms, said, s)`. The `said` words that pass `is_term` count as said, just as
     dictionary terms do (:347), and they count for that one check only. `sanity_ok` delegates with `&[]`, so its
     signature and every caller stay as they were.
   - `app/crates/engines/src/claude.rs:220` adds `decide_with` (`decide` delegates with `&[]`, so `refine_compare`
     needs no change). `settle` (:364) now takes `said`. Clean and Rewrite use `decide_with`, and Translate still uses
     `decide_translation` without `said`. The callers pass `&ctx.said`: claude.rs:358, openrouter.rs:355, chat.rs:168.
   - `app/src-tauri/src/runtime.rs` `refine()`: `ctx.with_habits(&sent)`.
   - The words are never written to the dictionary: they ride on the context, and the `dict` passed in is unchanged.
   - Tests:
     - refine.rs:1429 `sanity_counts_the_words_of_the_habits_sent_as_said`: the greeting is accepted with `said` and
       rejected without it; it also covers "with a dictionary too", "an answer is still caught" and "an unusable entry
       adds nothing".
     - refine.rs:1414 `the_context_carries_the_habits_and_prints_no_text`: `said` comes only from a replacement, and
       `Debug` shows no text.
     - claude.rs:609 `settle_keeps_the_words_of_a_habit_sent_with_the_cleanup`: Rewrite and Clean are Refined with
       `said`, and Rewrite falls back to RawFallback without it. Translate gives the same result with or without `said`
       for two answers. The dictionary stays the default.

2. **OpenRouter routing for the rule request.**
   - `app/crates/engines/src/openrouter.rs:186`: the `provider(routing)` helper was extracted from
     `request_body_styled`, which now uses it (:241). The body is unchanged byte for byte, and the existing body test
     still passes.
   - `app/crates/engines/src/habit.rs:68`: OpenRouter's `chat_body` sets
     `provider = openrouter::provider(Routing { require_parameters: p.structured, ..Routing::STRICT })`.
     A "no provider" refusal is an `Err` from `openrouter::stream`, and the template then writes the rule. There is no
     relaxed retry. The doc comment says so.
   - Test: habit.rs:193 `openrouter_gets_the_cleanups_routing_and_no_provider_gives_none`. The fake OpenRouter returns
     a rule, and the sent body carries `{sort: latency, data_collection: deny, require_parameters: true}`, the schema
     name and only the ask. A 404 "no endpoints" gives `None` after exactly one call.

3. **Privacy text and the leak scan.**
   - (a) README, Privacy section: the rule request carries the example once, and from then on the rule goes with every
     cleanup request of a dictation in that app, or in every app once the habit holds everywhere.
   - (b) README, Privacy in detail, the list of what the cleanup's connection receives: it now names the rules of the
     habits that are on for the dictation's app. The Habits paragraph also says that over OpenRouter the rule request
     goes only to providers that do not store requests for training, and that the template writes the rule when none
     can take it. It also says that the rules go with every cleanup request.
   - (c) leak.rs: the old comment was replaced by the doc of the new `Private::add_habits` (:163). It says that a short
     name the cleanup wrote and a replacement's words are the user's, and that template words and most rules are
     literals.
   - (d) leak.rs: new `Kind::HabitWords(usize)` (:69), displayed as "the words or short name of habit N" and counted
     in `summary`. `add_habits` adds each example side, each replacement's `from` and `to`, and the short name, all with
     `MIN_NAME_CHARS`. The short name is skipped when it equals the template's short name for the habit's forms in
     English or Russian. Rules are not looked for. `gather` calls it (:213). The module doc was updated.
   - Test: leak.rs:912 `a_habits_example_and_words_are_found_but_not_the_templates`. The example of an end-mark habit
     is flagged as `HabitExample(0)`. "без точки в конце", "без точки" and "no full stop" are not flagged. The
     replacement's `from` and `to` are flagged as `HabitWords(1)`, and so is a short name the cleanup wrote (UTF-16).

4. **README and ARCHITECTURE wording and width.**
   - The README `learn.enabled` entry now reads "The switch in the Habits section's heading sets it too."
   - The README leak-scan paragraph was rewrapped, and it now names the replacements' words and the short names the
     cleanup wrote.
   - The ARCHITECTURE `app/ui` bullet (with `habits.ts`) and the General line were rewrapped to 120 columns.
   - The README `habits.list` entry says that a replacement's form holds the replaced words and the words written
     instead, which are words of its example, with a JSON sample. ARCHITECTURE's "Text on disk" paragraph says the same.

5. **The two-app race.** I chose the smaller change, the runtime and `adopt` route.
   - `app/crates/core/src/habits.rs:639` adds `pub fn id_of(forms)`; `adopt` already computed the same id inline.
   - :586 `Habits::listed(&draft)` reports whether the habit with that id is already listed.
   - :596 `adopt`: when the id is listed, the draft adds to that habit instead of replacing it. The counts add up. If
     the habit belongs to another app, its app is cleared and its rule is unscoped, as a widening does, and the draft's
     rule is not used.
   - runtime `on_habit_rule` computes `merged = listed(&draft)` before `adopt`, logs `merged` in "habit learned", and
     queues a card and calls `show_notice` only when nothing was merged. `send_settings` still runs, so the row
     changes.
   - Test: habits.rs:1113 `a_second_draft_of_the_same_forms_adds_to_the_habit_listed`. Two interleaved apps produce two
     drafts. After the first adopt the second draft is `listed`. The second adopt leaves one habit with the same id,
     app `None`, `n` 6 and the rule "Тире вместо дефиса". A repeat in the same app adds 3 and keeps the app.

6. **`habits_sent` per dictation.**
   - runtime.rs:314: `habits_sent: ByDictation<(Vec<Habit>, String)>`. The text as heard is kept with the habits for
     item 8. `habits_applied` became `ByDictation<Vec<HabitMark>>`, because it had the same single slot.
   - :2603 `ByDictation<T>`: a deque of the last `KEPT` (4) values by id. `put` replaces the dictation's own value, so a
     retry overwrites the first try. `take` removes the value. The cap means a result that never comes cannot leak.
   - It is used in `refine()` (`put`), `on_refined` (`take`, then `put` the marks) and `on_finished` (`take`).
   - Test: runtime.rs `what_a_dictation_sent_is_kept_for_it_alone`: a later id does not displace an earlier one, a
     value is taken once, a retry replaces the earlier value, and the cap holds.

7. **The missed-suggestion dot under a tray card.**
   - `app/ui/src/design/drop.js:137`: `away(b)` now records `this.aw` (initialised at :63).
   - :81: the mouseenter on the missed dot does nothing while `this.aw` is set, that is, while a tray card has the Drop
     away.
   - Test: `app/ui/test/controller.test.ts:1692` "hovering a missed one while a habit card is open leaves the capsule
     closed". With a missed suggestion and an open `newHabit` card, a hover leaves the Drop idle without `sgwait`. After
     the card closes, a hover opens the capsule. I confirmed that the test fails without the fix.

8. **History chips claimed too much.**
   - habits.rs:759: `marks(sent, raw, text)` keeps only `applied(text) && !applied(raw)`.
   - runtime `on_refined` passes the text as heard, which is stored with `habits_sent`.
   - Tests: the existing `only_habits_whose_every_form_shows_are_marked` now passes `raw`. New: habits.rs:1090
     `a_habit_whose_effect_the_text_as_heard_had_already_is_not_marked`. A Digits habit is marked for
     "двенадцать" → "12" and not when the text as heard already had "12". An EndMark habit is marked when the text as
     heard had the full stop and not when it did not.

9. **ARCHITECTURE notes, Habits section.**
   - Step 5 describes the merge of two drafts (`listed`, `adopt`, no second card, `merged` in the log).
   - After step 6:
     - A habit widened while its card is open: the card keeps the rule it opened with.
     - Drafts and waiting cards live in memory only. Quitting loses a draft, and its counts are not given back. It also
       loses a card not yet shown, and that habit stays listed without being announced.
   - Also documented:
     - the request side: `said`, and the sanity check through `settle` → `decide_with` → `sanity_ok_with`;
     - `ByDictation`;
     - marks that need the text as heard;
     - step 4's OpenRouter routing;
     - the new leak-scan scope.

10. **Spec** (`docs/specs/2026-11-04-habits.md`).
    - "The habit" now shows the built shape `{ "id", "forms": [{ "kind", … }], … }`, with each form's JSON and how the
      id is made. It notes that a replacement's words are words of the example, and that two drafts of the same forms
      become one habit.
    - A new "The sanity check" paragraph was added.
    - "What the History shows" now uses the effect the cleanup brought and `habits: [{ "id", "short" }]`.

## Commands (repo root, then `app`)

- `cargo test --workspace`: all passed (cue 203 + 1 ignored, bench 23, cli 39, core 421, engines 203 + 4 ignored,
  platform-win 265 + 1 ignored). No warnings.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0, no warnings.
- `npx vitest run`: 15 files and 399 tests passed, with no stderr or warnings.
- `npx tsc --noEmit -p tsconfig.json`: exit 0.
- Self-review:
  - `git diff -U0` shows no raw line break inside a string literal; the `\n` escapes in the new tests are intact.
  - Line endings match the state before this pass. Git Bash `sed -i` had turned `drop.js` and `ARCHITECTURE.md` from
    CRLF into LF; both were converted back to CRLF.

## Departures from the rulings

- Item 1: the ruling allowed either "RefineContext carries them" or "the term list is extended". I used the context
  (`said`) together with a separate `said` parameter (`sanity_ok_with`, `decide_with`, `settle`). Appending the words to
  `dict.terms` would have changed the request's dictionary block and its cache prefix, and `MAX_TERMS` could have cut
  them.
- Item 2: `require_parameters` follows `p.structured`, as the ruling says. Every OpenRouter model CUE offers is
  structured, so in practice the field is always true, the same as the cleanup's STRICT.
- Item 5: I took the runtime and `adopt` route, not "count treats pending drafts as listed". It merges by habit id only.
- Item 6: `habits_applied` got the same per-dictation keeping. The ruling named only `habits_sent`, but the two had the
  same single-slot shape.
- The ARCHITECTURE `app/ui` bullet was reflowed as a whole paragraph, because its lines were joined. The wording is
  unchanged apart from the earlier `habits.ts` insertion.

## Concerns

- Item 5 merges only drafts with exactly the same forms, because it merges by id. Two drafts that share some forms
  (one with {first letter, end mark} in app A, one with {end mark} in app B) still become two habits that share a form.
  Handling that would need `count` to see the pending drafts (the ruling's other option).
- If the first habit is forgotten (so its forms are declined) before the second draft's rule arrives, `listed` is false
  and the second draft is adopted with declined forms. This was already so before this pass.
- The leak scan skips a short name only when it equals the template's short name for the habit's forms. A cleanup that
  wrote a template-like short name ("без точки") is skipped too, which is correct, since those are CUE's own words.
- Many long lines that predate this pass remain in README, ARCHITECTURE and the code; only the ones the ruling named
  were rewrapped.
