# Final review — fix wave (one dispatch). Fix all; rulings are binding.

## Critical
1. A replacement habit makes the cleanup's sanity check reject the result on short dictations ("здравствуйте анна" → "Добрый день, Анна!": 1 of 3 words "said" < 0.7 → RawFallback; raw text pasted, no chip). Files: app/crates/core/src/refine.rs ~328-334 (`sanity_ok`), app/crates/engines/src/claude.rs ~214-218 (`decide` via `settle`), app/src-tauri/src/runtime.rs ~1496-1502. **Ruling:** the words of the sent habits' replacements (`Form::Replacement.to`) count as said in the sanity check, like dictionary terms (e.g. `RefineContext` carries them, or the term list passed to `decide`/`settle` is extended for that request only — never written to the dictionary). Tests: the greeting pair with the habit → Refined; without → RawFallback; translation unchanged.

## Important
2. The OpenRouter rule request (app/crates/engines/src/habit.rs ~55-63, ~109-110) ignores CUE's routing, incl. `data_collection: deny` that the README promises. **Ruling:** use the same routing as the cleanup's first try (`Routing::STRICT`: provider sort latency, data_collection deny, require_parameters when structured — reuse the existing openrouter helpers); on a "no provider" failure just fall back to the template (no relaxed retry). Test the OpenRouter body (and a non-2xx → None).
3. Privacy text and leak scan understate what is sent/kept. **Ruling:** (a) README:~90-92 — say the rule goes with every cleanup request of a dictation in that app (or everywhere), not "once"; the rule request sends the example once; (b) "Privacy in detail" list of what the cleanup receives adds the habit rules; (c) fix the comment at app/crates/cli/src/leak.rs ~187 (rules and short names can hold the user's words); (d) the leak scan also checks each replacement's `from`/`to` and the habit's `short`, with the existing minimum length (MIN_NAME_CHARS), and a leak test with one habit (example found → flagged; template words of an end-mark rule not flagged). Keep rule texts out of the scan (they are mostly template literals).
4. README `learn.enabled` entry (~525-527) still says "The General settings set it too" → "The switch in the Habits section's heading sets it too." Rewrap the three added lines over 120 columns (README leak-scan paragraph; ARCHITECTURE habits.ts crate line and the General line). Add a short note that a habit's `forms` hold a replacement's words, which are words of its example.

## Minor (fix now)
5. Two-app race (habits.rs ~494-563, ~585-592): a form reaching 3 in app B while app A's draft with the same forms waits for its rule → two drafts, one id, app A lost, two cards. **Ruling:** in the runtime, when a rule arrives for a draft whose id is already listed, widen/merge instead of adopting again (or, in `count`, treat forms of pending drafts like listed habits) — pick the smaller change; test it.
6. `habits_sent` tracks one dictation (runtime.rs ~1502, ~1103): keep it per dictation id (map, or on the attempt).
7. Hovering the missed-suggestion dot opens the capsule over an open habit card (drop.js ~80): ignore that hover while a tray card is open (test).
8. History chips over-claim (habits.rs ~105-117): record a habit on an entry only when it shows in the cleaned text and does NOT show in the raw text (`applied(clean) && !applied(raw)`); tests for Digits and EndMark.
9. ARCHITECTURE: note that a habit widened while its card is open keeps the card's old rule, and that pending drafts and queued cards are in memory only (lost on restart; counts already used).
10. Update the spec docs/specs/2026-11-04-habits.md "The habit" JSON to the built shape (`forms: [{kind, …}]` instead of `kind` + `form`), and the line about History `habits` ({id, short}).

## Leave (deferred)
Reversing habits (two contradicting habits can coexist); a per-install salt for form hashes; a runtime-level test seam for the notice queue/hold; the other deferred minors in progress.md.
