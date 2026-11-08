# Final fix wave — report

Status: all 10 items fixed. No commits, nothing staged; the working tree still holds the whole feature, uncommitted.

## Per item

### 1. Rings never ran out under reduced motion (Important)
- `app/ui/src/design/drop.js`
  - `SG_MS={sugg:8000,added:6000}` (line 36).
  - The suggestion timer's state, `sgLeft/sgFrom/sgTimer`, is set up in the constructor (around line 63).
  - `enter()` starts 8 s for `sugg` when it is not a peek, and 6 s for `added`. Every other state stops the timer (line 90).
  - New methods `sgStart/sgRun/sgPause/sgStop/sgEnd` (lines 119–128), modelled on the tray TTL in controller.ts (`runTtl/pauseTtl/stopTtl`, hover-aware like `trayHovered`). `sgRun` does nothing while `this.hov` is set, or in the static, non-live Drop.
  - The Drop's own mouseenter/mouseleave now pause and resume the timer (line 81).
  - Removed the `animationend` listener on `sbt`. The CSS ring is decoration only: with reduced motion it stays still and the times still run out.
- controller.ts is unchanged: `onSuggest` already wakes the frame loop, so after the timer ends the Drop is idle and `calm` lets the loop sleep.
- Tests (`app/ui/test/controller.test.ts`, "Dictionary suggestion"): the describe now runs on fake timers.
  - Changed: "undo takes the word out…" now advances 6 s and checks for `suggestDone`. "a ring run out tells the core…" now advances 8 s and checks for `suggestDismiss`. The old hand-fired `animationend` test is now "a hover-opened missed capsule never runs out": 60 s hovered, then 60 s after leaving, and nothing is sent.
  - New: "hover pauses the 8 s, leaving resumes it from what is left".
  - New: "the 6 s after "+" waits while the mouse is on the capsule".
  - New: "the time is kept without the ring: its end alone does nothing" (`animationend` changes nothing).
  - New: "a dictation stops the time of an open capsule".

### 2. Capsule shown over a full-screen app or while paused but not clickable (Important)
- Core: `app/src-tauri/src/runtime.rs`
  - New pure helper `fn at_rest(phase, tray, answer_open, paused, hidden) -> bool` (line 2342), with a unit test `a_suggestion_opens_only_on_the_shown_idle_drop` (line 3025).
  - `show_offer` (line 812) now uses it, with `ui_state().paused` and a new `Runtime.idle_hidden` field (line 284).
  - New `CoreEvent::IdleHidden(bool)` (line 170). Its handler (line 642) stores the flag and calls `show_offer`.
  - The end of a pause needs nothing new: `TogglePause` and the `PauseEnd` timer both go through `feed()`, which already ends with `show_offer()`.
- Platform: the runtime cannot see idle-hidden changes on its own (`Ui::idle_hidden` only emits to the page), so the placement thread now tells the core.
  - `app/src-tauri/src/drop_window.rs`: `spawn_placement` takes a new `hidden: impl Fn(bool)` callback (line 224). `Keeper` keeps it as `told` (lines 502–507) and calls it next to `ui.idle_hidden(busy)` (line 537).
  - `app/src-tauri/src/main.rs:154-167`: the callback sends `CoreEvent::IdleHidden`.
- Page: in `app/ui/src/app.css:11-13`, the idle-hidden rule now covers `.drop:is([data-s=idle],[data-s=sugg],[data-s=added])`, with a short comment.
  - Test: "the capsule hides with the idle-hidden Drop: nothing is drawn that cannot be clicked". It checks opacity 0 and pointer-events none for both sugg and added, and that a recording Drop is not hidden.

### 3. CUE's own windows not excluded (WebView2) (Important)
- In `app/crates/platform-win/src/fieldwatch.rs`, `open()` (line 110) now checks `is_own(front_process(), element, std::process::id())`. The element-PID check stays.
  - New `front_process()` (line 124): `GetForegroundWindow` plus `GetWindowThreadProcessId`, the same pattern as focus.rs `window_process`.
  - New pure helper `is_own(front, element, me)` (line 38), with test `cues_own_windows_are_not_read` (line 213).

### 4. Typed-on words joined the offered word (Important)
- In `app/crates/core/src/corrections.rs` `changes()` (lines 66–86): when a run's new side has more words than its old side, only the leading new words that each pass `looks_like_name(clean(w), starts(j), corrected)` are kept. `starts` is the sentence-start rule, now a closure that the run's `sentence_start` uses too. The doc comment of `changes()` says so.
- Test `words_typed_on_after_the_corrected_one_stay_out_of_it` (line 394) covers the ruling's three cases, plus "спроси клод" → "спроси Claude Haiku", which gives "Claude Haiku".

### 5. LearnedWord Debug
- In `app/crates/core/src/settings.rs:245`, `LearnedWord` no longer derives Debug. A hand-written Debug prints `word_chars` and `heard_chars`.
- The test is in `a_learned_word_joins_the_dictionary_with_how_it_was_heard`. It checks that neither word appears in the Debug of a `LearnedWord` or of `DictionarySettings`, and that the lengths do.

### 6. A `cue://settings` push redrew the whole tray
- In `app/ui/src/window/page.ts`, the push now goes to a new `pushed(view)` (line 402). It stores the view, redraws the Dictionary, and calls `tray.learnChanged()`. `took()` is still used for the window's own commands.
- New `SettingsTray.learnChanged()` in `app/ui/src/window/settings.ts:299`. When the tray is open, it sets `aria-checked` on the Learn switch, if the page shown has one, and changes nothing else.
- Test in `app/ui/test/window-settings.test.ts:800`. An unsaved custom-instructions draft keeps its textarea element and its value across a push, and the dictionary chip appears. On the General page, the same switch element follows `learn:false`.

### 7. Hint spacing
- The hint now has one space before the book (`app/ui/src/window/dictionary.ts:73-76`). The text after the book has no leading space; instead the book has `margin-right:.28em` (`app/ui/src/design/window.css:259`). On screen there is one space on each side of the icon, and the hint's text has a single space.
- The test's expected text in `window-dictionary.test.ts` is now single-spaced.

### 8. ARCHITECTURE and the runtime comment
- `docs/ARCHITECTURE.md`:
  - `cue://settings` now goes to "the main window" (Corrections step 6, which adds that only the Dictionary is drawn again).
  - The platform-win crate list is rewrapped, and so is the `drop_ready` sentence, which now also lists the suggestion among what is resent.
  - Step 4 says `refine::is_term` applies to the heard text too, and describes the typed-on rule.
  - The `cue-field` line says that turning "Learn from me" off also ends the watch.
  - Step 5 is updated for `at_rest`/`CoreEvent::IdleHidden` and the page hiding the capsule. Step 6 is updated for the JS timers.
  - The `cue-placement` paragraph says that idle-hidden is told to the page and to the core; that paragraph is rewrapped.
  - No added line is longer than 120 characters.
- The `learn_word` comment in `runtime.rs` now says "the main window gets the settings".

### 9. README Privacy
- Privacy, the "Your corrections" bullet: once you press +, the cleanup's connection gets the word with the dictionary, along with how it was heard.
- Privacy in detail, the cleanup's connection: "`dictionary.terms` (for a word added from a correction, also how it was heard, `dictionary.learned`)". That bullet is rewrapped.
- The warm-up line now says "your dictionary (as the cleanup gets it)".

### 10. apply() doc
- A two-line addition to the `Settings::apply` doc comment (settings.rs, around line 752): "`DictLearn` puts a word in the same way, with how it was heard (refused too when that is no term); `Learn` sets "Learn from me", whose default (on) is written as no value." The ruling asked for one line; at 120 columns it takes two.

## Checks (all green, nothing else printed)
- `cargo test --workspace`: cue 198 passed (1 ignored), cue-bench 23, cue-cli 38, cue-core 379, cue-engines 196 (4 ignored), cue-platform-win 265 (1 ignored). 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: Finished, no warnings.
- `npx vitest run` (from app): 14 files, 372 tests passed. No stderr and no warnings from the tests.
- `npx tsc --noEmit -p tsconfig.json`: exit 0.
- `git diff -U0`: none of my edits added a string with `\n`, and the build and tests confirm no string was broken. Line endings were kept as each file had them (CRLF files edited through a newline-preserving script, or with Edit).

## Departures and judgement calls
- Item 1: the timer lives in drop.js, beside the Drop's own states (`s`, `peek`, `hov`), not in controller.ts. It is modelled on the tray TTL as the ruling asked. Putting it in the controller would have meant mirroring drop.js's state changes, such as entering `added` from its own click, or a peek.
- Item 2: the runtime had no signal for idle-hidden changes. I added a callback from the placement thread (`CoreEvent::IdleHidden`) rather than reading the `Ui`'s shared flag, so `show_offer` runs when the Drop is shown again.
- Item 4: typed-on words are pruned before the three-word limit is checked, so "Марго и скажи что я опоздаю" still offers "Марго" instead of being dropped as a run too long. Old-side runs of more than three words are still not candidates.
- Item 7: one space each side of the icon on screen, and one space in the hint's text, by moving the second space into a margin on the icon.

## Concerns
- When the Drop becomes hidden (paused, or over a full-screen app) while a capsule is already open, the 8 s keeps running unseen and ends as a missed dot. That is consistent with the rules, but the user never sees the capsule. Pausing the timer while hidden was not asked for.
- Item 3's foreground check is tested only through the pure helper. The Windows call itself is not exercised by the tests; the ignored manual test covers `open()` by hand.
