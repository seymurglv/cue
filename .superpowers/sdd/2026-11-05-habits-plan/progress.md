# SDD ledger — plan: docs/specs/2026-11-05-habits-plan.md
Spec: docs/specs/2026-11-04-habits.md
Snapshot T0 (before Task 1): e0ce33dd5cb7dd59c7aebf36058fb233755a9b14
Ruling: the plan's 17 rulings (end of plan) accepted as written — the owner approved the plan; each is reasonable against the spec — if wrong: the owner reads them in the final list.
Pre-flight: preflight.md (17 findings: 3 blocking, 6 should-fix, 8 notes); plan Rust for Tasks 1–5 compiled and passed in a scratch copy.
Ruling: B-1 leak scan checks only before/after — template words are in cue.exe — if wrong: a rule text on disk is not scanned (it holds no field text anyway).
Ruling: B-2 show_notice() after every suggestion answer — a queued habit card must open when the capsule closes — if wrong: an extra no-op call.
Ruling: B-3 clippy for core deferred from Task 1 to Task 2 — helpers become used there — if wrong: none.
Ruling: S-2 the every-card test fills {RULE} for newHabit — keeps coverage — if wrong: one test edit.
Ruling: S-3 Habits empty line uses .hbempty — .hempty is the History's — if wrong: a class name.
Ruling: S-4/5/6 existing tests updated for the moved switch, four header links, scoped .bmore — they test old layout — if wrong: test edits.
Ruling: S-7 Learn off also drops saved-but-unannounced habits — global constraint "nothing learned" — if wrong: a habit found just before switching off is lost.
Notes (8) in preflight.md left to the task reviews.
Task 1: dispatched (base snapshot e0ce33dd5cb7dd59c7aebf36058fb233755a9b14)
Task 1: reviewed — Approved with 1 Important (plan-mandated): Digits branch catches name/term edits ("гпт четыре" → "GPT-4").
Ruling: Digits requires that `to` holds no capital letter and no letter of the other alphabet (so "12 %", "5 минут" count; "GPT-4" does not) — spec: dictionary candidates are not habits — if wrong: a digits edit with a capitalised unit ("5 Мб") is not counted.
Task 1: minor (deferred): short shared word lets runs through; FirstLetter::shows_in first alphabetic char; no tests for shows_in/has_run/MAX_TOKENS/example fallback (Task 2 to cover).
Task 1: fix round 1/5 (1 addressed, 0 open — Digits excludes terms; snapshots ffdd365..d1dbc93)
Task 1: complete (snapshots e0ce33d..d1dbc93, review clean)
Task 2: dispatched (base snapshot d1dbc936617834d923fbee72e4ad48a3629980d7)
Task 2: reviewed — Needs fixes: (1) multi-form habit n grows per form, not per dictation (brief's code); (2) several replacements: the one outside the example still stored in forms/rule (privacy constraint) + missing example tests (ruling).
Ruling: a habit's n grows once per dictation (by 1, or by the other app's count when it widened) — "how many times" is per correction — if wrong: counts read lower.
Ruling: replacements whose words are not in the stored example are left out of the draft (their counts stay at 3 and draft on their next correction) — the example is the only correction text on disk — if wrong: such a replacement is learned one correction later.
Ruling: also fix two minors now: unscoped() cuts the app name only at a word boundary (else template); widening drops the seen counts of all the habit's hashes — cheap, user-visible wording / dead slots — if wrong: a few lines.
Task 2: note for the owner: a multi-form habit widens to everywhere when any one of its forms repeats 3 times in another app (spec wording).
Task 2: minor (deferred): read_rule fence in upper case falls to the template; template coverage partial (EndMark !?…, FirstLetter upper, RU Digits, EN Dash, U+2028).
Ruling: unscoped() cuts the app name only before a colon, dash or comma (a rule like 'In Telegram write…' falls to the template) — avoids cutting 'Telegram Desktop' — if wrong: some model rules get the template's wording when widened.
Task 2: fix round 1/5 (4 addressed, 0 open; snapshots aac305a..b85a517)
Task 2: complete (snapshots d1dbc93..b85a517, review clean)
Task 2: minor (deferred): unscoped treats a hyphen in "Telegram-X" as the separator.
Task 3: dispatched (base snapshot b85a5177eed6f55db244280f203ee3659974de35)
Task 3: complete (snapshots b85a517..f0e5344, review clean; ⚠️ summary text: workspace run green, resolved)
Task 3: minor (deferred): long lines in leak.rs comment/summary and settings.rs module doc; no leak test for habit examples.
Task 4: dispatched (base snapshot f0e5344db8a6e0e9c74e2c57d2d40a1747e3b5e4)
Note: tauri dev CUE exited when a mid-task build failed; the installed CUE 0.1.2 was restarted for the owner.
Task 4: complete (snapshots f0e5344..fc4eee0, review clean)
Task 4: minor (deferred): import order in claude.rs; redundant test import; missing blank line before a test in refine.rs; one_line lets format chars (Cf) through; chat/openrouter tests lack translation/empty cases.
Task 5: dispatched (base snapshot fc4eee054eb3217d6578de25482e295591680199)
Task 5: complete (snapshots fc4eee0..2e79202, review clean; ⚠️ system text checked: constant RULE_PROMPT + language only)
Task 5: minor (deferred): duplicated rule_messages call in a test; OpenRouter request built inline; no tests for OpenRouter kind / non-2xx / chat body branches.
Task 6: dispatched (base snapshot 2e7920205a01ab537a8479eda5b7b03e09b26dd2)
Task 6: reviewed — Needs fixes: Important (i) Undo opens the next queued notice during the "Removed" hold. Minors 1–4.
Ruling: core side: after a tray action on a kind with a done label (NewHabit, NoField), the Drop stays busy for the page's done hold + fold (1100 + 430 ms) through a timed CoreEvent, then the tray clears and show_notice runs; the page's contract is unchanged — the spec wants "Removed" seen and never two notices together — if wrong: the next notice comes ~1.5 s later.
Ruling: fix minors 1–3 now (drop forgotten ids from habit_cards on HabitForget/HabitsForget; HabitsForget also clears drafts and habit_cards; forgetting the open card's habit from the main window closes the card) and 4 (extract a pure habits_for(mode, learn_on, job) with a test) — correctness and cheap — if wrong: a few lines.
Task 6: fix round 1/5 (5 addressed, 0 open; snapshots 9e637b9..4559461)
Task 6: complete (snapshots 2e79202..4559461, review clean)
Task 6: minor (deferred): a card closed from the main window lets the next notice open during its 430 ms fold (same as the cross today); no unit test for the timed TrayDone hold; runtime paths otherwise untested.
Task 7: dispatched (base snapshot 4559461c0e6c4a0c63673ecd7a05da31cbf47dcc)
Task 7: reviewed — Needs fixes: a card without a rule shows ". CUE writes…" and a stale rule could show.
Ruling: the page clears the rule once a card used it (and on another kind / null); an empty rule renders the body sentence alone; no "." added after a rule that already ends with . ! ? … — defensive, the core normally always sends a rule — if wrong: a few lines.
Task 7: fix round 1/5 (1 addressed, 0 open; snapshots 2323840..10b98ae)
Task 7: complete (snapshots 4559461..10b98ae, review clean)
Task 7: minor (deferred): a rule ending in a quote after its stop gets a second "."; no tests for RU bare sentence, clear-on-other-kind, . ! … variants.
Task 8: dispatched (base snapshot 10b98ae1deff96932e10e256f8c86e001b322634)
Task 8: reviewed — Approved, 7 minors.
Ruling: fix minors 1 (muted "after" on a hovered off row falls to 4.2:1 — no hover tint on off/paused rows), 2 (recheck drops stale key_state replies by a sequence counter) and 4 (a second trash click during the fold is ignored) now — contrast ≥ 4.5:1 is a family hard rule; the other two are user-visible glitches — if wrong: a few lines.
Task 8: minor (deferred): keysChanged fires on every KeyRows.refresh (extra IPC); test gaps (reduced motion, refused forget, rebuild while armed); redundant .hdel transition; habitsApplied text for Task 9.
Task 8: fix round 1/5 (3 addressed, 0 open; snapshots 9675311..2efa2b2)
Task 8: complete (snapshots 10b98ae..2efa2b2, review clean)
Task 9: dispatched (base snapshot 2efa2b2a2a446aee3e693a80b628424581af70e4)
Task 9: complete (snapshots 2efa2b2..23dc93f, review clean; ⚠️ Entry.habits only applied ones: verified in Task 6 review (f); chip contrast: live check)
Task 9: minor (deferred): defensive ?. on habits; no test for several chips.
Task 10: dispatched (base snapshot 23dc93f6c010812119e4f5c7568c6a46cf46309a)
Task 10: implemented (snapshot 75311a0fa210305d7cafda414935c6d5f7c93956); task review and final review dispatched in parallel
Task 10: reviewed — Needs fixes: README learn.enabled entry still says "The General settings set it too". Minors: 3 lines over 120 cols; README could say forms hold the example's words; leak-scan min length unstated. Held for the final fix wave.
FINAL REVIEW: With fixes — 1 Critical (replacement habit breaks the sanity check → raw text), 2 Important (OpenRouter rule request without data_collection deny; privacy text/leak scan understate), + Task 10 docs + minors. Rulings in final-findings.md.
Ruling: habit replacement words count as said in the sanity check for that request only — otherwise every replacement habit throws away the cleanup on short dictations — if wrong: the check is a little looser for words the user taught.
Ruling: the rule request through OpenRouter uses STRICT routing and falls back to the template on "no provider" — README promises data_collection deny — if wrong: fewer model-made rules through OpenRouter.
Ruling: leak scan adds replacement from/to and short (≥ MIN_NAME_CHARS), not rules — user words may sit there; rules are mostly template literals — if wrong: a model rule quoting the example is not scanned (the example itself is).
Ruling: History chips only when the effect is in the cleaned text and not in the raw — otherwise chips over-claim — if wrong: a habit that coincided with speech shows no chip.
Ruling: two-app race, habits_sent per id, dot hover over a card — fix now (small, user-visible) — if wrong: a few lines.
Ruling: reversing habits and a hash salt left for later — design decisions for the owner — if wrong: contradicting habits can coexist until then.
Final fix wave dispatched (base snapshot 75311a0fa210305d7cafda414935c6d5f7c93956).
Final fix wave: done (snapshot 7ebe9f9d90f12caec3d063bbe575bd0bb9a3237d). Departures: said words carried separately (RefineContext.said); two drafts merge only with exactly the same forms; habits_applied per id too.
Final fix wave re-review: all 10 addressed, no new breakage (snapshots 75311a0..7ebe9f9). Next: live check with the owner, then commit on request.
OWNER 06.11: (1) contradicting habits → "Changed": 3 opposite corrections replace the old habit, card "Привычка изменилась", Undo restores; (2) hash secret in Windows Credential Manager; (3) widening only when all the habit's forms come together 3 times in another app. Spec updated. Addendum tasks 11–13 to be planned and run the same way.
Ruling: the addendum's rulings 19–28 (plan) accepted — HMAC-SHA-256 for the secret hash, Restored/«Возвращено» label, a habit with an opposite in some app never widens, etc. — sound against the spec — if wrong: the owner reads them in the final list. Task 11 dispatched (base snapshot ed56b4e8b363a411583502487a91976374aedc07).
Ruling: only a missing or malformed secret entry makes a new secret; any other store read failure leaves the secret unset for the session (nothing counted, nothing overwritten) — a passing failure must not wipe declined forms — if wrong: habits do not learn until the next start after a store failure.
Task 11: reviewed — Needs fixes: (1) any store read failure makes a new secret (against the ruling); (2) forget without a secret writes zero-key hashes to declined; (3) leak scan skips the secret silently on a read error (plan-mandated). Minors: long comment line, no long-key HMAC test, odd leak label.
Ruling: forget without a secret removes the habit but declines nothing; the leak scan exits with an error when the secret cannot be read (missing/malformed → None) — zero-key hashes are guessable; a blind scan must not pass — if wrong: a forgotten habit may be learned again after a store failure.
Task 11: fix round 1/5 (4 addressed, 0 open; snapshots 1e984af..8d83860). Spec aligned: "missing or malformed".
Task 11: complete (snapshots ed56b4e..8d83860, review clean)
Task 12: dispatched (base snapshot 8d83860eaf14ff5701ff3e352cd430475438e515)
Ruling: ruling 25 (no widening for a habit with an opposite) moves to Task 13, where opposites exist — Task 12 had nothing to check against — if wrong: none.
Task 12: complete (snapshots 8d83860..8de1234, review clean)
Task 12: minor → carried into Task 13: a habit with empty forms widens vacuously in count (add !hashes.is_empty()); rewrap the long ARCHITECTURE step 3 line and README line 29.
Task 13: dispatched (base snapshot 8de1234ba326c98e32f790b2ffa1d4e7f9bf2201)
Task 13: reviewed — Needs fixes: Important: an Undo on "Habit changed" can restore a habit beside a later opposite one. Minors 1–5.
Ruling: the Undo wins — restoring the old habit removes (does not decline) any habit listed since that opposes it in the same app or everywhere; the restore itself is never skipped — Undo is the user's explicit, latest choice — if wrong: a habit learned while the card waited is lost and relearns after 3 more corrections.
Ruling: fix minors now: learn-off revert of a merged card restores the merged habit to its pre-merge state (not deleted); a test for newest-first revert; reflow count's doc; ARCHITECTURE step 5 says Learned.merged and Habits::listed becomes test-only or goes; the everywhere-change test compares whole habits — small and correctness-related — if wrong: a few lines.
Task 13: fix round 1/5 (5 addressed, 0 open; snapshots 5b0b6b1..fc70b86)
Task 13: complete (snapshots 8de1234..fc70b86, review clean)
Task 13: minor (deferred): restored order after a learn-off revert of a merged card is [x, y]; the plan file still shows old revert/listed signatures.
Addendum final review: running (snapshots ed56b4e..fc70b86).
ADDENDUM FINAL REVIEW: With fixes — Important: (1) per-form-set ids make an opposite-blocked habit swallow another app's corrections; (2) Undo on a merged "changed" card deletes+declines an unrelated habit; (3) README misses the habits-secret credential. Rulings in addendum-findings.md.
Ruling: a habit's id includes its app (None for everywhere) — the owner wants each app to have its own counts and habits — if wrong: ids change once (counts kept by their own hashes).
Ruling: Undo on a merged change restores the habit as it was and declines nothing — the user never undid that habit — if wrong: the changed form can be learned again sooner.
Ruling: also fix re-id on a kept secret, carry a waiting card's chain into a newer change, and the private doc link — small, keep invariants — if wrong: a few lines.
Addendum fix wave dispatched (base snapshot fc70b8609a3b505b77d832a0989da568dc85d6be).
Addendum fix wave: done (snapshot 2122e03cd2351610faa6b5c931a61d9808c82e0b). Additions: widening folds same-form habits of other apps; cards follow a widened id; the row changes in place on widening.
Addendum fix wave re-review: all 7 addressed, no new Critical/Important (snapshots fc70b86..2122e03).
Addendum: parked — Undo or learn-off on a card that followed a fold removes the other apps' folded habits — Ruling: left for now (needs a widening during a card's open/waiting window, rare; no second fix wave) — if wrong: an old habit of another app is lost in that rare case and relearns after 3 corrections.
Addendum: minor (deferred): fold matches forms in order only; two long ARCHITECTURE lines.
Next: live check with the owner, then commit on request.
