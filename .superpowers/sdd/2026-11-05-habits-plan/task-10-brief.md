### Task 10: documents, the checks, the live check

**Files:**
- Modify: `README.md`, `docs/ARCHITECTURE.md`

- [ ] **Step 1: README.**
  - Privacy (line 85), after the "Your corrections stay on this computer." bullet, a sentence in it: "When you correct
    the same thing a few times, CUE keeps it as a habit: the words around the correction (at most 40 characters before
    and after it) are the only text of a correction it writes to disk, and the cleanup's connection receives them once,
    with the kind of the correction and the app's name, to word the habit's rule; it already received that text when
    you dictated it."
  - The settings list (line 255): "**General** (Start at sign-in)"; the switch "**Learn from me**" is now "in the Habits
    section's heading".
  - After "Words from your corrections" (line 257), a new bullet **Habits**: the three corrections in one app (the end
    mark, the first letter's case, a word written another way, numbers as digits, a dash for a hyphen), the "New habit"
    card with **Undo** (7 s, hovering pauses it), the habit going into the cleanup of that app's dictations (Translate
    and "Ask" take none), widening to every app when it repeats in another, the main window's Habits section (switch,
    Forget on hover, Forget all on a second press, muted while "Learn from me" is off, the line when no cleanup is on),
    and the History's chips.
  - The settings.json part (after `learn.enabled`, line 506): `habits.seen` (counts by hash, no text, 30 days, at most
    300), `habits.list` (`{ id, forms, rule, short, before, after, app, n, on }`, newest first, at most 50),
    `habits.declined` (hashes of forms never to learn again); each list written only when not empty, anything that does
    not read is skipped. `learn.enabled` `false` also stops habits from being counted, learned and applied.
  - Privacy in detail (line 706, "Learn from me"): what the habit keeps on disk (the example, the rule, the short name;
    counts by hash), what the rule request sends and when, that it is not sent without a cleanup connection (the rule
    then comes from a template), and that the log gets "habit learned" with kinds and numbers.
- [ ] **Step 2: ARCHITECTURE.** After "## Corrections", a "## Habits" section: `CoreEvent::Corrected { …, app }` →
  `habits::correction` (the kinds over `corrections::runs`, the forms and their FNV-1a hashes) → `Habits::count`
  (`seen` by app and hash, `LEARN_AT`, widening, `declined`) → `Draft` → `cue_engines::habit::rule` through the
  cleanup's connection (`habits::rule_messages`: kind, example, app; `RULE_WAIT_MS`, else `habits::template`) →
  `CoreEvent::HabitRule` → `Habits::adopt` → `cue://settings`; the card queue (`habit_cards`, `first_notice` with the
  suggestion's `offer_found`, `at_rest`) → `cue://habit-rule` + `cue://tray` `newHabit` → `trayAction` →
  `SettingChange::HabitForget`; and the request side: `Habits::for_app` → `RefineContext::with_habits` →
  `refine::habits_prompt` after the cache mark (Clean and Rewrite only) → `habits::marks` on a refined text →
  `Entry.habits` → the History's chips.
- [ ] **Step 3: all checks** from the global constraints: green. The leak scan (`cargo run -p cue-cli -- leak-scan …`,
  as `scripts/build-release.ps1 -ScanBuild` runs it) still passes on a settings.json with a habit.
- [ ] **Step 4: live, with the owner** (build the app the usual way, `npx tauri dev` from `app` or the release build;
  cleanup on Anthropic with a key):
  1. Telegram: dictate "Скинь файл", change it to "скинь файл" (lower case, no full stop) three times: after the third
     the card "Новая привычка" with the rule; it closes after 7 s; hovering pauses it. The next dictation in Telegram
     comes without the capital and the full stop; its History entry shows the chip «без точки».
  2. "Отменить" on a new card: «Убрано», the row leaves the Habits section, the same correction is never learned again.
  3. Почта: "Здравствуйте, …" → "Добрый день, …" three times: a replacement habit; its rule names both.
  4. Документы: "клиенты - в основном" → "клиенты — в основном" three times: a dash habit; then three times in another
     app: the row says «везде», no card.
  5. With no key (or the network off): the rule comes from the template within 5 s.
  6. "Learn from me" off in the Habits heading: rows muted, nothing learned, the next dictation's request has no habits
     (the log has no "habit forms counted").
  7. A dictionary suggestion and a habit from one correction: the capsule first, the card after it folds.
  8. Both themes; reduced motion.
- [ ] **Step 5: report to the owner** what passed, what did not, and ask whether to commit.

---

## Rulings

1. **The app is the history's name for it** ("Telegram Desktop", from the file description), not the exe the spec's
   example shows: the spec says "the dictation's app as the history records it", and the history keeps
   `stored_app(attempt.app)`. It is also the name the section and the rule show. A dictation whose app is not known is
   not counted, and gets only the habits of everywhere.
2. **A habit keeps `forms` (each kind with its detail) instead of one `kind` and one `form` hash.** A merged habit (lower
   case and no full stop) has several forms, and the applied check and widening need their details, not only hashes;
   the hashes are computed from the forms. A replacement's words are always words of the example (a replacement longer
   than 40 characters is no form, and the example falls back to the replacement's own words), so no correction text
   beyond the example reaches disk.
3. **The history entry keeps `habits: [{ id, short }]`, not `[id]`:** "a forgotten habit's chip shows its short name as
   it was" needs the short name on the entry.
4. **`newHabit` is a plain tray kind; the rule goes just before the card on `cue://habit-rule`.** `TrayKind` is `Copy`
   and goes to the page as a bare string; the missing microphone's names and the switched model's name already come
   the same way.
5. **The rule request names the interface language in its instructions;** the user message holds exactly the kind(s),
   the example and the app (the test checks the keys). The model is asked for a rule that starts with the app; on
   widening the "In <app>" / «В <app>» start is cut off, and a rule that does not start so is written anew from the
   template's everywhere form.
6. **Template wording** follows the spec's examples; a scoped habit's template always starts with the app ("В Почта:
   писать «добрый день» вместо «здравствуйте»"), and the spec's replacement example is read as the everywhere form. App
   names are not declined in Russian.
7. **The habits block goes after the cache mark** (after the dictionary block, before the style's block): it changes with
   the app, and the cached prefix stays the one every dictation reads.
8. **"Forget all" also clears the counts (`seen`)**, besides the habits and the declined forms: a form at two would
   otherwise become a habit at the next correction right after the user cleared everything.
9. **A habit's `n` keeps growing** with further corrections of its form in its app (or anywhere, once everywhere), and
   widening adds the other app's count — the row's "how many times".
10. **Only dictations count and take habits.** The watch also runs after a translation (for dictionary words), but a
    translation's correction is not counted, and Translate and "Ask" requests take no habits.
11. **The applied check runs only on a refined text:** a raw fallback marks no habit, though its text may happen to
    show one.
12. **Notices in order:** a habit is "found" when its forms reach three (its number is taken then), though its card can
    open only once its rule came (≤ 5 s). A card waits while the suggestion's capsule is open or shows "undo"; a missed
    suggestion's dot does not hold it back.
13. **"No cleanup connected"** means the chosen connection is a cloud one with no saved key (a local server counts as
    connected). The line is checked when the section is drawn, when the connection changes and when the window gets
    the focus back — not the moment a key is typed on the Keys page (the window has no event for that).
14. **The rule request is not written to usage.csv** (at most one per habit learned, a sentence long).
15. **End mark and first letter need the same last / first word:** one mark swapped for another is neither; a
    correction that shares no word with the paste holds no forms.
16. **"Learn from me" off** drops the habits waiting for their rule and the cards waiting for rest; a card already
    open stays until it closes, and its Undo still forgets.
17. **`RefineContext` gets a hand-written `Debug`** (lengths and a count): it now carries the rules, and its derived
    `Debug` printed the app and the text before the caret.
18. **Hard in the code:** `corrections::changes` cuts a run with more words written than heard to its leading names, so
    "здравствуйте → добрый день" never reached it; the alignment is split out (`corrections::runs`) and habits read the
    raw runs, with the dictionary's behavior unchanged. std's hasher is not stable between versions, so the form hash is
    FNV-1a written out.

## Coordinator rulings for this task (binding, override the text above)
- The leak scan's habit check covers `before`/`after` only (Task 3 ruling); document that.
