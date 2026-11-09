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

09.11.2026: the spec of `no-keys` took this idea over as its Task 10 (a catalog of cleanup-1, RuadaptQwen3 4B and S1
Mini, recommendations by the user's languages, a speed record per model), and the owner **postponed it again**: "не хочу
добавлять какие-либо модели сейчас по дефолту". Nothing of it is written; the This-computer list shows today's three
models. The moved test `cleanup_only_local_model_ends_with_need_answering_model` and the `spawn_ask` seam wait with it.
When the owner says to start: run the plan's Task 10, and reconcile with this note's rulings. See [no-keys.md](no-keys.md).