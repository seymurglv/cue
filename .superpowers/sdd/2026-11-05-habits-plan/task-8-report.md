# Task 8 report: the main window's Habits section (variant A)

Status: DONE. No commits, nothing staged.

## What was done
- Rust `app/src-tauri/src/ipc.rs`: `SettingsView.habits: Vec<HabitView>` (after `learn`), new `HabitView`
  `{ id, rule, before, after, app, n, on }` (`app` null for everywhere), filled in `of` from `s.habits.list`. `Debug`
  shows only the count. Tests: `settings_view_json_names` now expects `"habits"` and the default `[]`; new
  `settings_view_has_the_habits`.
- `ui/src/window/api.ts`: `HabitView`, `SettingsView.habits`, and `SettingChange` gains `habitOn`, `habitForget`,
  `habitsForget`.
- `ui/src/window/text.ts`: the Habits texts in en/ru (brief Step 4), `learnHint` removed, a doc comment on `learn`.
- `ui/src/window/icons.ts`: `trash` (stage 5 grid), `IconName` gains `'habit'` (the shared set's).
- `ui/src/window/habits.ts` (new): `HabitsView` with `mount(sec)`, `render(animate?)`, `recheck()`. Heading with
  the count (rolls with `CUE.setLabel`) and the "Learn from me" switch (`[data-sw="learn"]`) at its right; the hint;
  the no-cleanup line; rows (rule; struck before → green after · globe + everywhere / "in Telegram" · n times; trash on
  hover; switch); muted rows when a habit or "Learn from me" is off; the empty line; "Forget all" on a second press
  within 3 s. Rows are kept and changed in place; new ones grow in with the highlight, gone ones fold away; reduced
  motion makes both immediate.
- `ui/src/window/page.ts`: the section after the Dictionary, the header link, `took()` and `pushed()` redraw the
  Habits, the window's focus asks `recheck()`; `tray.learnChanged()` call gone.
- `ui/src/window/settings.ts`: the General page has only "Start at sign-in"; the `learn` switch branch of `onClick` and
  `learnChanged()` are gone.
- `ui/src/design/window.css`: the Habits rules from variant A, placed after the Dictionary's whole block (after
  `@keyframes shake`, its last rule).

## TDD
RED (Rust), from `C:\Users\seymurglv\Desktop\cue-dev`: `cargo test -p cue ipc`
```
failures:
    ipc::tests::settings_view_has_the_habits      (left: Null, right: [ {..."rule":"In Word: a dash between words, not a hyphen"...} ])
    ipc::tests::settings_view_json_names          (no "habits" in the names)
test result: FAILED. 5 passed; 2 failed
```
GREEN: `test result: ok. 7 passed; 0 failed`.

RED (UI), from `app`: `npx vitest run ui/test/window-habits.test.ts ui/test/window-settings.test.ts ui/test/window-today.test.ts ui/test/window-history.test.ts`
```
Test Files  3 failed | 1 passed (4)
Tests  16 failed | 78 passed (94)
```
(all 12 Habits tests, the 3 settings tests, the header-links test; the History test passed as it only got scoped).
GREEN: `Test Files 4 passed (4)  Tests 94 passed (94)`.

The extra test "a key saved in the tray: the line goes" was seen failing with the `keysChanged` call commented out
(1 failed | 12 passed), then passing with it back.

Final checks:
- `cargo test --workspace`: all ok (1146 passed, 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `npx vitest run`: `Test Files 15 passed (15)  Tests 393 passed (393)`.
- `npx tsc --noEmit -p tsconfig.json`: no output, exit 0.

## Deviations (each from the brief, and why)
1. Coordinator ruling: the empty line's class is `.hbempty` (not `.hempty`, the History's); the new test file uses
   `.hbempty` in its three checks.
2. Rulings on existing tests: window-settings.test.ts "General holds only start at sign-in…" now expects
   `['Start at sign-in']`; the "Learn from me" test replaced and the "settings pushed…" test changed as the brief says;
   window-today.test.ts expects four links; window-history.test.ts scopes the two `.bmore` counts to
   `.bsec[data-scr="history"]`.
3. `entry()` in windowFixture.ts did NOT get `habits: []`: `EntryView` (TS and Rust) has no `habits` field yet, so
   tsc would reject the excess property. Task 9 should add it together with the field.
4. Brief's `drawn` set dropped: a row is highlighted when it comes in an animated render (`animate` true) — the same
   result for every case the brief had, except that a refused "Forget" now draws the row back without the highlight.
5. `forget(id)` finds its row by comparing `dataset.w` instead of interpolating the id into a selector.
6. Added: `Ctx.keysChanged?()` (optional), implemented by `WindowPage` as `habits.recheck()`, called from the tray's
   `KeyRows` change callback. Without it, saving a key in the tray left "Habits work when the cleanup is on" showing
   until the window lost and regained focus. Covered by the new test "a key saved in the tray: the line goes".
7. The row wrapper `.hw` has `role="none"`, so the list item is directly in the list for a screen reader.
8. CSS: `.hdel` keeps `.ibt`'s background/color transitions with its opacity one (the brief's `transition:opacity .2s`
   would have dropped them); `.bsec[data-scr=habits] .bmore{margin-top:10px}` from the variant (scoped by the
   section's attribute instead of the variant page's `.hsec`). `.hdel` not added to the focus-visible list: `.ibt` has
   its own `:focus-visible` rule.
9. The page.ts header comment rewrapped to the 120-column width after adding "the Dictionary and Habits sections".

## Files
- C:\Users\seymurglv\Desktop\cue-dev\app\src-tauri\src\ipc.rs
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\habits.ts (new)
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\api.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\page.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\settings.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\text.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\window\icons.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\src\design\window.css
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\test\windowFixture.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\test\window-habits.test.ts (new)
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\test\window-settings.test.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\test\window-today.test.ts
- C:\Users\seymurglv\Desktop\cue-dev\app\ui\test\window-history.test.ts

## Self-review
- Escapes: the write tool turned `\u00a0` in habits.ts into a literal no-break space; fixed back to the `\u00a0` escape
  with a small `python -I` script (checked: no U+00A0 left in habits.ts). No other line breaks or escapes changed
  (`git diff -U0` read through).
- User text (rule, before, after, app) goes in as text (`textContent`, `append`, `fill`); only the trusted globe and
  trash SVGs go in as markup.
- `Debug` of `SettingsView` prints the habits' count only; the new Rust test checks the example is not in it.
- Muting is color only (`--muted`), never opacity; the trash's opacity 0 → 1 on hover/focus is a reveal, as the
  dictionary chips' cross.
- Labels change with `CUE.setLabel` (count, "Forget all" / "Press again"); reduced motion: rows appear and go at once,
  the highlight and the empty line's rise are off.
- Files are in English and mention no development tools.

## Concerns
- Not checked in the running app (no visual pass of both themes): only jsdom tests. Worth a look at the heading with
  the switch at narrow widths, and the contrast of muted words on the off row's 6% tint.
- `keysChanged` fires on every key-row change (also a check), each one `key_state` call; cheap, but more calls than
  strictly needed.
- `habitsApplied` text is added for Task 9 and is unused for now.

## Fix round 1

### Changes
1. Contrast (`app/ui/src/design/window.css`): `.hb.off:hover,.habl.paused .hb:hover{background:none}`, right after
   `.hb:hover`. A switched-off row, or any row while "Learn from me" is off, gets no hover tint, so its muted "after"
   sits on the 6 % tint alone. The trash still shows on hover.
   Contrast, worked out from the cue.css tokens (WCAG relative luminance; tints mixed as `color-mix` over `--bg`):

   | case | light (`--muted` #6B6B67 on `--bg` #FBFBFA, `--text` #111111) | dark (#85858A on #0E0E0F, `--text` #ECECEA) |
   |---|---|---|
   | muted on bg | 5.17:1 | 5.25:1 |
   | muted on the 4 % hover tint (an on row's example and meta) | 4.76:1 | 4.89:1 |
   | muted "after" on the off 6 % tint (the case now, no hover) | 4.57:1 | 4.67:1 |
   | muted "after" on 6 % + 4 % hover (before the fix) | 4.21:1 | 4.24:1 |

2. `recheck()` (`app/ui/src/window/habits.ts`): a counter, `keyAsk`, goes up on every check, including one that
   finds a local server or no connection. Only the latest check's answer sets the line: a pending earlier reply is
   dropped. A failed read still hides the line, as before.
3. `forget(id)` (habits.ts): a `forgetting` set. A press on a row already being forgotten does nothing. The id leaves
   the set once the core has answered (in a `finally`), so after a refusal the row, drawn back, can be forgotten again.

### Tests (`app/ui/test/window-habits.test.ts`, written first, all three seen failing)
- "a second press on the trash while the row folds away forgets the habit once": mounted with motion, so the row
  folds over 450 ms; two clicks lead to exactly one `habitForget`. RED: `expected [ …(2) ] to deeply equal [ [ { kind: 'habitForget', … } ] ]`.
- "two checks of the key answered out of order: the later one counts": two `keyState` replies are held (the mount's,
  then the window's focus). The later one (saved) is answered first, then the earlier one (not saved); the line stays
  hidden. RED: `expected false to be true`.
- "a cloud check still pending when the cleanup turns to a local server: no line": the core sends a switch to Ollama
  while the cloud check is pending, then that check answers "not saved"; the line stays hidden. RED: `expected false to be true`.

RED, from `app`: `npx vitest run ui/test/window-habits.test.ts` gave `Tests  3 failed | 13 passed (16)`.

GREEN, from `app`:
- `npx vitest run`: `Test Files  15 passed (15)  Tests  396 passed (396)`
- `npx tsc --noEmit -p tsconfig.json`: no output, exit 0

No commits, nothing staged.
