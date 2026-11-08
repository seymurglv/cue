# Branch `model-picks`: which local cleanup models to offer (postponed)

Spec `docs/specs/2026-11-05-cleanup-model-picks.md` and plan `docs/specs/2026-11-05-cleanup-model-picks-plan.md`,
committed 05.11.2026 as 06b179a on branch `model-picks` (from `main` 883977c; pushed 08.11.2026, not merged).

The owner's rulings: recommend by language, learned from the dictations; start from models proven by others'
benchmarks (S1-mini for English, Gemma 4 E4B for Russian + English, Qwen3 4B Instruct kept as the one that translates
Russian); measure only Russian and processor speed ourselves. S1-mini (Superwhisper's) is English only and needs its
own input format.

05.11.2026: postponed ("потом начнем"). When the owner says to start: execute the plan task by task, and stop at its
two checkpoints (the measurement table; the caption variants) for the owner. macOS comes after this
([macos.md](macos.md)). Note: [optimize-and-settings.md](optimize-and-settings.md) later rejected Gemma for the
redesign; reconcile before starting.
