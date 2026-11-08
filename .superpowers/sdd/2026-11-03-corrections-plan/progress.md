# SDD ledger — plan: docs/specs/2026-11-03-corrections-plan.md
Spec: docs/specs/2026-11-02-corrections.md
Snapshot T0 (before Task 1): e7cb4e409397913de002d575edea3bed1e0c9ed8

## Pre-flight scan
| Tasks | Shared file / interface | Finding |
|---|---|---|
| 1↔2↔3 | corrections.rs (one module, tests module grows) | sequential appends; consistent names (Replacement, Watch, Step, Offers, SuggestView) |
| 2↔6 | Watch::new/read/finish, Step, READ_EVERY_MS, SETTLE_MS | consistent |
| 3↔7 | Offers methods, UiAction::Suggest* | consistent; runtime catch-all arm keeps T3 compiling |
| 3↔8 | SuggestView {word, missed} JSON ↔ TS Suggest | consistent |
| 4↔5 | DictionarySettings.learned, LearnedWord | consistent |
| 4↔7 | SettingChange::DictLearn/Learn, learn.enabled() | consistent |
| 4↔9 | SettingChange JSON kinds dictLearn/learn ↔ TS | consistent (TS only sends learn) |
| 5↔7 | runtime.rs refine call sites (terms → dict) | T5 edits runtime lines 1049/1142; T7 later edits other runtime parts; sequential, fine |
| 7↔8 | cue://suggest, actions suggestAdd/Undo/Dismiss/Done | consistent |
| 7↔9 | cue://settings, SettingsView.learned/learn | consistent |
| T1 self | tests vs code | consistent (checked: runs, sentence-start, alphabets) |
| T2 self | tests vs code | consistent (timings checked by hand) |
| T3 self | tests vs code | consistent |
| T4 self | tests vs code | consistent |
| T5 self | signature list from grep | may miss example files; compiler-driven |
| T6 self | tests vs code | consistent; package name to check |
| T7 self | no unit harness for runtime | accepted; logic tested in T1/T3 |
| T8 self | test "undo … ring" re-offered while still 'added' | FIXED in plan before dispatch (core sends null after undo) |
| T9 self | tests use placeholder helper names | implementer maps to the file's own helpers |

Ruling: no commits; each task ends green and uncommitted; review diffs come from working-tree tree snapshots (temp index, real index untouched) — owner's rule "commit only on request" — if wrong: owner commits the whole feature at once instead of per task.
Ruling: work in the main checkout on main, uncommitted — owner's established norm, the installed CUE does not run from the repo — if wrong: changes mix with docs/specs edits, which are docs only.
Ruling: T8 test fixed in plan (ctl.suggest(null) after undo) — mirrors the core's answer — if wrong: one test line.
Ruling: T9 helper names (mountWith/pushSettings/openGeneral/calls/flush) are stand-ins; implementer uses the test files' own helpers — if wrong: test wording differs, behaviour same.

## Tasks
Task 1: implemented (snapshot 862bf45), reviewing
Ruling: Task 1 test very_long_texts_are_not_diffed is vacuous (plan-mandated) — fix it so the guard is what blocks a real candidate, plus a positive case under the cap — spec wants the size cap to matter — if wrong: one test.
Task 1: minor (deferred): redundant-looking closure |t| key(t) (needed for &&str); re-lowercasing dictionary per change (harmless).
Task 1: fix round 1/5 (1 addressed, 0 open — vacuous long-text test; snapshots 862bf45..d7c2706)
Task 1: complete (snapshots e7cb4e4..d7c2706, review clean)
Task 2: dispatched (base snapshot d7c2706baf80bf0b78b03e670647e2517952c207)
Task 2: complete (snapshots d7c2706..0019e8b, review clean)
Task 2: minor (deferred): paste at field end → typed-after text lands in span (pure insertion, candidates ignore it); rfind picks last occurrence; test gaps (60 s cap after change, long read gap, empty before anchors).
Task 3: dispatched (base snapshot 0019e8bc29e1ca0099937ada89e68623f0e7eee4)
Ruling: Task 3 added a no-op arm for Suggest* in core flow.rs (exhaustive match) — the flow does not own the suggestion, like TrayAction — if wrong: the flow would need its own handling later.
Task 3: complete (snapshots 0019e8b..2132bd3, review clean)
Ruling: Task 7 on_corrected resends the view right after offers.offer() (a replaced missed dot must leave the page) — reviewer's Task 3 ⚠️ — plan + task-7 brief patched — if wrong: one extra emit.
Task 3: minor (deferred): done() could use step(); offer() doc line about resend.
Task 4: dispatched (base snapshot 2132bd3daa6c11a263117aa3d3a69a5a48eda288)
Task 4: complete (snapshots 2132bd3..63b2b5a, review clean)
Task 4: minor (deferred): LearnedWord derives full Debug (like DictionarySettings); apply() doc comment omits DictLearn/Learn; leak.rs scans learned word twice (count inflated).
Task 5: dispatched (base snapshot 63b2b5a5ee545a6fbfa1a9cbeaf2db5a0a6dfa44)
Ruling: Task 5 terms_prompt adds the heard suffix only when is_term(heard) — a hand-edited settings.json can hold an empty or multi-line heard, which would inject lines into the system prompt — if wrong: a learned word loses its hint.
Task 5: fix round 1/5 (3 addressed, 1 open — raw line breaks in string literals; snapshots db60100..2ce88e7)
Task 5: fix round 2/5 (1 addressed, 0 open — raw breaks fixed via script; snapshots 2ce88e7..674994f)
Task 5: complete (snapshots 63b2b5a..674994f, review clean)
Task 5: note: implementer's edit tool turns backslash-n into real newlines; later dispatches warn and ask for a grep check. Temp script C:\Users\SEYMUR~1\AppData\Local\Temp\fixnl.py left (deletes need the owner's yes).
Task 6: dispatched (base snapshot 674994ffdbf5aef3baf9eab049f90a5c3c29b599)
Task 6: complete (snapshots 674994f..2ecc8b8, review clean)
Task 6: minor (deferred): own-process check by PID may miss the webview process; GetText(20000) cuts long fields silently; test hardcodes 400.
Task 6: for Task 7: element is captured when the cue-field thread starts — spawn right after InsertDone(Pasted); check learn.enabled gates spawn.
PAUSED 2026-11-03 at owner's request after Task 6. Resume at Task 7 (base snapshot 2ecc8b8cf58aa11f4edc298e3a30fb00f1647ec0).
RESUMED 2026-11-04: snapshot matches 2ecc8b8; Task 7 dispatched (opus)
Task 7: reviewed — Approved, 4 minors.
Ruling: cue://settings goes to the main window only (emit_to MAIN_LABEL) — only it draws the dictionary; Drop/card/welcome need none of the words — if wrong: another window would have to fetch settings itself.
Ruling: fix minors 1 (watch identity by the stop flag itself, not the dictation id) and 2 (drop watch_next on every insert outcome) now — both are privacy hygiene on the "off means no read" rule — if wrong: a few extra lines.
Task 7: minor (deferred): no unit test for the rest rule (optional at_rest helper).
Task 7: fix round 1/5 (3 addressed, 0 open — settings to main only, watch identity by flag, watch_next dropped on every outcome; snapshots 28fea95..669cb98)
Task 7: complete (snapshots 2ecc8b8..669cb98, review clean)
Task 7: minor (deferred): insert reads attempt.mode without checking attempt.id == id; runtime watch paths have no unit tests.
Task 8: dispatched (base snapshot 669cb98195a2f20a5adfff92ce6c3eb91f3aeec2)
Task 8: reviewed — Approved. ⚠️ added-state on dictation start: resolved — core OpenMic calls offers.dictation_started (clears Added) and sends suggest(None); consistent.
Ruling: fix minor 1 (dot stays visible while the Drop is hidden: idle-hidden / away) — a visible glitch the owner would see — if wrong: two CSS lines.
Task 8: minor (deferred): duplicated onSuggest guard; answer phase folds an open capsule locally; frame() rects not tested.
Task 8: fix round 1/5 (2 addressed, 0 open — dot hides with the Drop, peek ring sends nothing; snapshots 6a8a892..6210382; rebuilt test file verified: no lines lost)
Task 8: complete (snapshots 669cb98..6210382, review clean)
Task 9: dispatched (base snapshot 6210382c4551e66b28e795abca8bc1a610dd0a9c)
Task 9: complete (snapshots 6210382..fcfd3a0, review clean)
Task 9: minor (deferred): no RED run recorded; duplicated switch-building lines; no test for a push flipping the switch; defensive ?. on learned.
Task 10: dispatched (base snapshot fcfd3a027850f67031f1f2dcb6f791e594ba737f)
Task 10: implemented (snapshot b1f8405628744cb28e5b52fa10ed2c2620e7a6a5), reviewing; final review dispatched in parallel
Task 10: reviewed — Needs fixes: ARCHITECTURE says "the windows get cue://settings" (main window only; also runtime.rs:746 comment); minors: two >120-col lines, is_term applies to heard too, cue-field thread line omits Learn off. Held to batch with the final review's fix wave.
FINAL REVIEW: With fixes — 4 Important (reduced-motion rings, capsule over full-screen/paused, own windows via WebView2, typed-on words join the word) + minors. Rulings in final-findings.md.
Ruling: rings timed by JS timers paused on hover (CSS ring decorative) — spec timing must not depend on animation settings — if wrong: ring and timer could drift visually.
Ruling: "at rest" = idle, no tray, no answer, not paused, not idle-hidden, as one tested helper; page also hides sugg/added under idle-hidden — if wrong: a suggestion waits longer.
Ruling: own-window check adds the foreground window's process — WebView2 provider PID differs — if wrong: a field in another app with CUE in front is skipped (CUE never pastes there anyway).
Ruling: a longer new run keeps only leading name-like words — typed-on text after a correction is common in chats — if wrong: a multi-word term with a lowercase word inside is cut (e.g. "Visual studio").
Ruling: a cue://settings push redraws only the Dictionary and the Learn switch — keeps unsaved drafts in the tray — if wrong: another setting changed by the core would not redraw (only the dictionary changes this way today).
Final fix wave dispatched (base snapshot b1f8405628744cb28e5b52fa10ed2c2620e7a6a5).
Final fix wave: done (snapshot 621228eb330a8632033c5254fc9385d69c6bd4f1), re-review dispatched. Deviations: timers in drop.js; new CoreEvent::IdleHidden; typed-on words cut before MAX_RUN check.
Final fix wave re-review: all 10 addressed, no new breakage (snapshots b1f8405..621228e). Deferred: capsule timer keeps running while the Drop is hidden. Next: live check with the owner.
LIVE CHECK 2026-11-04 (owner): Claude — capsule, +, green book, tooltip, hint, undo all work; Marisol removed by the owner. Not checked live: 8 s dot, Learn off, Brave, Telegram, dark theme. Next: commit on the owner's yes.
