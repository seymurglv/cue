# The main window, built in parts

The owner wants the full main window of the design artifact "Макеты CUE" (stage 5), built part by part, each through
spec → plan → code, matching the mockup's style exactly (the merlin-skin design family).

1. Today + header links: done, 4fab00b (`docs/specs/2026-11-01-today.md`).
2. Dictionary (manual): done, 6b04c61.
3. + 4. Corrections watcher and the "Add to dictionary?" suggestion: done, 4fc348d (04.11.2026), checked live in
   Claude. Not yet checked live: the 8 s dot, "Learn from me" off, Brave, Telegram, dark theme.
5. Habits: done, 151e120, spec `docs/specs/2026-11-04-habits.md`, plan `docs/specs/2026-11-05-habits-plan.md`. Owner
   additions 06.11: the hash secret in Credential Manager, opposite corrections change a habit, widening only when all
   forms repeat in another app, ids per app. **Not checked live yet** (stopped mid live check). Deferred: rare
   fold + undo loss, reversing order, runtime tests for the undo and key paths.
6. Remaining settings: hotkey recording, sounds, theme, audio ducking, idle bar.

Order agreed 03.11.2026: 3+4 → 5 → 6 → release 0.2 for Windows; macOS after that. A capital on a sentence's first
word is not a name sign by itself (default taken 03.11.2026).

Decisions (02.11.2026): the suggestion view is B + C of the trays mockup (capsule book · word · + with an 8 s ring;
missed → a green dot until the next dictation); "Learn from me" (`learn.enabled`) is on by default.

Spike: UI Automation reads the field after a paste in Claude and in Brave's address bar; Telegram and page fields
untested.
