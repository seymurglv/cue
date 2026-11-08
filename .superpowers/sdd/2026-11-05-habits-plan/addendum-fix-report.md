# Addendum fix wave: report

Status: DONE. No commits, no git add/stash/checkout/reset. All checks green.

## Per item

### 1. A habit's id includes its app
- `app/crates/core/src/habits.rs:871` `id_of(forms, app: Option<&str>, secret)`: FNV-1a over the forms' hashes joined by
  "," plus `@<app>` for an app habit. For `None` ("везде") the key is exactly the old one, so an everywhere id is the
  same value as before (pinned: `hca6cdd4168133ac4`); the Telegram habit's pinned id is now `hb7a43023672e208b`.
- `learn` (~716): the merge target is a habit whose id is `id_of(forms, Some(draft.app))` or `id_of(forms, None)`, that
  is, the same app or everywhere. Another app's habit is never the target: the draft makes its own app's habit. A new
  habit that holds everywhere (an everywhere opposite was replaced) gets the everywhere id. `opposed` is no longer used
  in `learn`: once `everywhere` is true, every opposite has been removed already, so it was always false there.
- New private `Habits::widen` (~747), used by `count` (~635) and by `learn`'s everywhere merge: clears the app,
  unscopes the rule, gives the habit the everywhere id, folds habits of the *same forms in other apps* into it (their n
  added), and drops the counts of its forms and every `seen` entry keyed by any of those old ids, in every app.
  **The set entries are dropped, not moved to the new id** (this matches the existing widening, which already dropped
  them). Tested in `widening_removes_the_counts_of_its_set_and_its_forms_in_every_app` and in the new two-apps test.
- `Counted::widened` (~573) is now `Vec<(String, String)>`: each id that went, with the everywhere id it has now.
- `rekey` (~837) now sets the secret and calls `reid` (ids include the app).
- Tests: `a_habit_with_an_opposite_in_another_app_never_widens` (~1641) updated as ruled (Notepad gets its own habit,
  the Telegram habit is unchanged at n = 3). New `two_apps_habits_of_the_same_forms_live_apart_with_their_own_ids`
  (~1663): two apps' drafts make two habits with different ids, the same app's second draft merges, and the set
  coming together three times in a third app folds both into one everywhere habit (n 3+3+6, both pairs reported).
  New `a_draft_adds_to_the_habit_of_everywhere_of_its_forms` (~1701). The old
  `a_second_draft_of_the_same_forms_adds_to_the_habit_listed` was removed (its scenario is the one the ruling forbids;
  its same-app half moved into the two-apps test). `a_merged_change_taken_back_leaves_the_habit_it_added_to_as_it_was`
  became `a_merged_draft_taken_back_leaves_the_habit_it_added_to_as_it_was` (~1790), a same-app merge. Widening tests
  now expect the pairs and the everywhere id.

### 2. Undo on a merged card
- `app/src-tauri/src/runtime.rs:2711` new `HabitCard::undo`: with `was` it calls `take_back` (restores `was` plus the
  replaced habits, declines nothing); otherwise `Habits::revert(id, replaced, true)` (forget the new habit, decline its
  forms, restore the replaced). `undo_habit` (~2127) now just calls it, saves, sends the settings and logs
  `habit undone` with `merged`, `changed`, `removed` (no text). The former `HabitForget` path for a new card was the
  same operation (`revert` with no replaced habits is `forget`), so behaviour is unchanged there.
- Tests: `undo_on_a_card_that_learned_a_habit_forgets_it_and_brings_back_what_it_replaced` (~3514, both a
  "changed" and a "new" card) and `undo_on_a_merged_card_restores_the_habit_it_added_to_and_declines_nothing` (~3533,
  also checks learn-off gives the same state). The old `a_merged_change_taken_back_restores_the_habit_it_added_to`
  relied on the cross-app merge and was replaced by these.

### 3. README: the secret's Credential Manager entry
- `README.md:121` Uninstall list: `habits-secret.CUE`, the secret the habits' counts are hashed with.
- `README.md:336` API keys: `habits-secret` (`habits-secret.CUE`) beside the keys, made by CUE itself.

### 4. Ids made again under a kept secret
- `habits.rs:848` new `Habits::reid() -> bool`: every id made again with `id_of(forms, app, secret)` where it differs;
  `seen` entries under a habit's old id follow it. `runtime.rs:998` `key_habits`, `Kept` branch: calls it, logs
  "habits: ids made again under the kept secret" with the count of habits, and saves when one changed.
- Test: `ids_made_otherwise_are_made_again_and_their_sets_follow` (~1721).

### 5. A replaced habit's waiting card carries over
- `runtime.rs:2722` new `carry(waiting, replaced)`: each replaced habit whose card still waits is replaced by that
  card's `was` then its `replaced`, and that card leaves the queue. Called in `on_habit_rule` (~960) before deciding
  whether a card is due, so a change that only undid an unannounced new habit becomes a "New habit" card.
- Test: `a_change_of_a_habit_whose_card_waits_carries_that_cards_chain` (~3554): the chain card's take-back and Undo
  both end at the original habit; the order `was`, `replaced`, others is checked too.

### 6. `cargo doc` warnings in habits.rs
- The `[Habits::opposed]` links in `count`/`learn` are gone (plain `opposed`). Two pre-existing "unclosed HTML tag
  `app`" warnings (the `In <app>` docs of `template` and `unscoped`, ~911 and ~931) were fixed by putting the
  placeholders in code spans. `cargo doc --no-deps -p cue-core` now warns only in corrections.rs and translate.rs (3,
  pre-existing, outside this change).

### 7. Docs
- Spec `docs/specs/2026-11-04-habits.md`: "The hash's secret" (ids made again too; `reid` under a kept secret),
  "Wider" (set under the habit's id, the everywhere id, set counts dropped, same-forms habits fold in), "Changed"
  (merged card's Undo; a waiting card's chain carried), "The habit" (id made from the forms, the app and the secret;
  two apps' drafts make two habits; a same-app draft or one whose habit widened adds to it).
- `docs/ARCHITECTURE.md` (Habits steps 2, 3, 5, 6; CRLF kept): `id_of`, `reid`, `Habits::widen`, the widened pairs
  and `follow`, the merge rule, `carry`, `HabitCard::undo`.
- `README.md` ~283-289: two apps make two habits, one once it holds everywhere; the merged card's Undo.

## Departures and additions (beyond the rulings)
- **Fold on widening.** With per-app ids, two apps' habits of the same forms both widen in a third app at once and
  would get the same everywhere id (duplicate ids). So `widen` folds habits of the same forms in other apps into the
  widened one (counts added; a folded set's own count is not added, since it counted the same corrections). The spec
  now says so.
- **Cards follow a widened id.** A widened habit's id changes, so an open or waiting card would lose its habit (Undo
  would do nothing). `Counted::widened` carries (old, new) pairs and `follow` (`runtime.rs:2735`, called in
  `count_habits` ~922) moves the cards. Test: `the_cards_of_a_widened_habit_follow_its_new_id` (~3582).
- **The row changes in place.** The section keys rows by id, so a widened habit would fold away and grow in anew,
  against the spec ("the row changes in place"). `app/ui/src/window/habits.ts` ~100: a new everywhere habit takes
  the row of an app habit gone in the same update with the same example; `update` now writes the row's ids
  (`data-w`, `data-forget`, `data-hsw`) and remembers the habit shown (a `WeakMap`). Test in
  `app/ui/test/window-habits.test.ts` ~149.

## Checks (from the repo root / from app)
- `cargo test --workspace`: all ok (core 442, app 208 + 1 ignored, engines 265 + 1 ignored, others ok), 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: finished, no warnings.
- `cargo doc --no-deps -p cue-core`: no warnings for habits.rs (3 elsewhere, pre-existing).
- `npx vitest run`: 15 files, 404 tests passed.
- `npx tsc --noEmit -p tsconfig.json`: clean.
- Self-review: `git diff -U0` of runtime.rs shows no added line with an odd quote count; no edit used a backslash
  escape. README/spec LF and ARCHITECTURE CRLF kept as they were.

## Concerns
- A merged card with replaced habits cannot arise from a consistent list now (the merge target and an opposite in the
  same app or everywhere would clash). The merged-card Undo is implemented and tested on a hand-made state, as ruled.
- If a widening folds two habits that each had a card, both cards follow to the same id and both may show. Rare
  (needs two cards waiting while three corrections happen in a third app); left as it is.
- The fold keeps the widened habit's `on`; a folded habit that was switched off disappears into it.
- `cargo fmt --check` is not clean in the repository as a whole (pre-existing, e.g. leak.rs); not one of the checks.
