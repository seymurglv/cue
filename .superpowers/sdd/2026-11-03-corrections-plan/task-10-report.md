# Task 10, steps 1-3: report

No commits, no git index/stash/checkout. Only README.md and docs/ARCHITECTURE.md were edited (uncommitted).

## README.md
- Privacy (top list): new bullet "Your corrections stay on this computer" (Learn from me on by default, field read up to a
  minute after a paste, text in memory only, nothing written or sent, word enters the dictionary only on "+", password
  fields / CUE's own windows / Ask answers never read).
- Features: new bullet "Words from your corrections" (capsule, 8 s ring, "+", Undo 6 s, green dot until the next dictation,
  cleanup gets "Marisol — sounds like «Марисоль»", off = no field read, Telegram / web page fields may give nothing);
  Dictionary sub-bullet notes the green book and the hint; settings tray list: General now "Start at sign-in; Learn from me".
- Settings: bullets `dictionary.learned` and `learn.enabled` (defaults, what is written, lenient reading, Dictionary
  removal removes both).
- Privacy in detail: bullet "Learn from me" (UI Automation, 0.4 s, 60 s, stop conditions, memory only, log gets
  found/none and numbers, Undo or Dictionary removal takes out the word and how it was heard).
- Known limitations: Telegram / web page fields, and only replaced name-like words are noticed.

## docs/ARCHITECTURE.md
- Crates: `corrections` (core), `fieldwatch` (platform-win). Threads: `cue-field`.
- New section "Corrections" (before "Ask"): insert/`watch_next` -> `InsertDone` -> `fieldwatch::spawn` -> `Watch` ->
  `CoreEvent::Corrected` -> `corrections::candidates` -> `Offers` -> `cue://suggest` -> Drop capsule -> `suggestAdd` ->
  `SettingChange::DictLearn` -> `cue://settings`; `suggestUndo` -> `DictRemove`; `FieldReader` / `Pace` as the plug-in
  point for another system's reader; memory-only/log rules.
- Events and actions (`cue://suggest`, `suggest*`), Settings tray General (`learn.enabled`), Settings (learned, learn),
  Logging (what is logged for the watch; field text and learned words never logged).
- Long lines rewrapped to the files' 120 columns. No mention of tools, assistants, plans or reviews.

## Checks (all from the stated directories)
- `cargo test --workspace`: all `test result: ok` (197, 23, 38, 378, 196 (4 ignored), 264 passed; 0 failed). Output in
  scratchpad `cargo-test.txt`.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0, no warnings.
- `npx vitest run` (app): 14 files, 366 tests passed.
- `npx tsc --noEmit -p tsconfig.json` (app): exit 0.
- Leak-scan: `cue-cli leak-scan <paths>` reads settings via `%APPDATA%\CUE\settings.json`. I ran it with APPDATA set (for
  that shell only) to a scratchpad folder holding a settings.json with term "Marisolkin" and learned
  {Marisolkin, Марисолкин}; the owner's real settings.json was not touched (history, keys read-only as always).
  Scan summary: "dictionary terms 3" (term + learned word + heard). A clean folder: exit 0, "nothing private found". A
  folder with the word and with the "heard" text: exit 1, findings reported as "dictionary term 1/2" with file and
  offset, never the value. So learned words and how they were heard are scanned. The full `-ScanBuild` was not run.

## Concerns
- Side effect: a first background cargo test wrote its log to `%LOCALAPPDATA%\scratch_test.txt` by my mistake; moved to the
  scratchpad, nothing left behind.
- README says field reading works in Claude and in a Brave address bar (spike result) and that Telegram / web page
  fields may give nothing; Step 4 will confirm or correct that.
