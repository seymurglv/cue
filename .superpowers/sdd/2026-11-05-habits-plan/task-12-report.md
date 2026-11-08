# Task 12 report: stricter widening

Status: done. No commits, no git add/stash/checkout.

## Changes
- `app/crates/core/src/habits.rs`: `Habits::count` rewritten as in the brief (habit sets counted under the habit's id; forms already on a habit of this app or everywhere belong to no set; other forms count for the app; widening drops set and form counts in every app). `forget` now drops `seen` entries under the habit's id. Comments of `Seen`, `Counted.widened`, `count`, `forget` updated.
- Tests: renamed `a_habit_of_one_form_widens_when_it_comes_three_times_in_another_app` (+ doc comment); replaced `widening_removes_the_counts_of_its_set_and_its_forms_in_every_app`; added `all_its_forms_together_three_times_in_another_app_widen_a_habit`, `one_form_of_a_habit_alone_in_another_app_makes_that_apps_own_habit`, `a_forgotten_habit_takes_its_sets_counts_along`. Seen failing first (4 failed), then green.
- `README.md` (Habits bullet, `habits.seen`) and `docs/ARCHITECTURE.md` (Habits step 3; CRLF preserved).

## Checks
`cargo test -p cue-core` 430 passed; `cargo test --workspace` all ok; `cargo clippy --workspace --all-targets -- -D warnings` clean.

## Deviations / notes
- New tests placed right after the replaced test (immediately before `a_replacement_outside_the_stored_example...`), as the brief intends.
- Ruling 25 (a habit with an opposite never widens) is not in the brief's code and the real code has no "opposite" logic yet (ruling 24's `learn` change is not built), so widening does not check it; belongs to the opposite task.
