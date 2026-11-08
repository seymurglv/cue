# Task 10 report

Status: done (steps 1-3; step 4 live check left to the owner). No commits, no git add/stash/checkout.

## README.md
- Privacy: the habit sentence inside the "Your corrections stay on this computer." bullet (40 characters per side, only correction text on disk, sent once with kind and app name).
- Settings list: General is now "(Start at sign-in)"; "Learn from me" moved to the Habits section's heading (also in the "Words from your corrections" bullet title and the `learn.enabled` entry).
- New **Habits** feature bullet after "Words from your corrections" (kinds, card with Undo 7 s/hover pause, applied to that app's cleanup, Translate/"Ask" none, widening, section with switch/Forget on hover/Forget all second press/muted/no-cleanup line, History chips).
- settings.json: `habits.seen`, `habits.list`, `habits.declined` after `learn.enabled`; `learn.enabled` false also stops habits.
- Privacy in detail: new "Habits." bullet (what is on disk, what the rule request sends and when, template fallback with no key / 5 s, log lines, "Learn from me" off).
- Leak-scan paragraph: names the habits' examples and states that `before`/`after` alone are looked for (coordinator ruling).

## docs/ARCHITECTURE.md
- Crates: `habits` (core), `habit` (engines), `habits.ts` (ui).
- New "## Habits" section after Corrections: Corrected{app} -> `habits::correction` over `corrections::runs` -> `Habits::count` (seen, LEARN_AT, widening, declined) -> Draft -> `ask_rule`/`habit::rule` (`rule_messages`, `RULE_WAIT_MS`, `template`) -> `HabitRule` -> `adopt` -> `cue://settings`; card queue (`habit_cards`, `first_notice`/`offer_found`, `at_rest`, `cue://habit-rule` + `newHabit`, `trayAction` -> `HabitForget`, `hold_tray`); `HabitOn`/`HabitsForget`; request side (`habits_for`, `for_app`, `with_habits`, `habits_prompt` after the cache mark, Clean/Rewrite only); `marks` -> `Entry.habits` -> chips; disk/log/Debug rules.
- Settings tray General line no longer lists "Learn from me".

Deviations: README/ARCHITECTURE keep CRLF line endings (all lines, consistent). Edits made through Edit; one python -I round-trip for two small wording fixes (CRLF preserved, verified 100% CRLF).

## Checks (all green)
- `cargo test --workspace`: all suites ok (201, 202, 23, 265, 38, 418 passed; 0 failed).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `npx vitest run`: 15 files, 398 tests passed.
- `npx tsc --noEmit -p tsconfig.json`: clean.
- Leak scan: `APPDATA` pointed at scratchpad `...\scratchpad\appdata` (settings.json with one habit, example "Zorbulon quartz meeting." / "...meeting"), `cue-cli leak-scan`: "habit examples 2" counted (before + after). Clean folder: 0 findings, exit 0. Folder with a file holding the example text: 1 finding "habit example 1" at offset 10, exit 1. (Other counts read the owner's real keys/history/mics as usual; the real settings.json was not used.)

## Concerns
- A habit's JSON in settings uses `forms: [{ "kind": "endMark", ... }]` (serde tag), documented as "the kinds of correction with their details".
- The leak scan's other lines (history texts 672, keys 3) come from the real machine, unchanged behavior.
- Step 4 live check not run.
