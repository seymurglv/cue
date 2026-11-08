# Task 9 report: green book, hint, "Learn from me"

## What
- api.ts: `SettingsView.learned`, `SettingsView.learn`, `SettingChange` `{kind:'learn'}`, `onSettings(cb)` (listens `cue://settings`).
- page.ts: `api.onSettings((v) => this.took(v))` next to `onModel`.
- text.ts: `dictHintLearned`, `dictHeard`, `learn`, `learnHint` (en + ru, brief's strings).
- dictionary.ts: hint = text + book icon + learned text; learned chips get `<i class="g">` with the book and a `title` from `dictHeard` ({HEARD} replaced inline; `fill` only knows {NAME}).
- settings.ts: second switch row (`data-sw="learn"`, srow with the hint as small); click sends `{kind:'learn', value: !current}`. The switch state follows the view because `settingsChanged()` redraws the open page.
- window.css: added `.dch .g` and `.dch .g svg`. The `.dhint svg` rule already existed (12px); kept that one, did not add a second (13px in the brief).
- Fixture: `view()` gets `learned: []`, `learn: true`; `feeds.settings` keeps the `onSettings` callback; `dictRemove` also drops from `learned`; `learn` works through the generic fallback.
- Tests: 3 in window-dictionary.test.ts, 1 in window-settings.test.ts; the existing General-page row list expectation now includes 'Learn from me'.

## TDD evidence
Deviation: I wrote the implementation before the tests (order slip), so there is no separate RED run for the new tests. The first test run after both existed was the only run: 365 pass, 1 fail, the hint test, because of a double space (text + ' ' then ' ' + text around the icon, the zero-width svg text), which the brief allows adjusting in the expectation only. Fixed the expectation.

## GREEN
`npx vitest run`: 14 files, 366 tests passed. `npx tsc --noEmit -p tsconfig.json`: clean.

## Deviations
- The learn test re-queries the switch after the click (the page redraws, so the old node is detached) instead of holding one reference.
- Hint expectation has two spaces before the dash.
- No RED run (see above).

## Files
app/ui/src/window/{api,page,text,dictionary,settings}.ts, app/ui/src/design/window.css, app/ui/test/{windowFixture,window-dictionary,window-settings}.test.ts (fixture: windowFixture.ts). CRLF line endings preserved.

## Self-review
Diff is small; no added line ends inside an open string; no git writes. `learned?.` optional access kept for a settings view without the field. Remaining concern: none beyond the missing RED run.
