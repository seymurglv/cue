# Cleanup on this computer or in the cloud (the local half lives on in `no-keys`)

Spec `docs/specs/2026-11-04-local-cleanup.md`, plan `docs/specs/2026-11-04-local-cleanup-plan.md`, both approved
04.11.2026; in `main` as 694351a. The ledger with every ruling:
`.superpowers/sdd/2026-11-04-local-cleanup-plan/progress.md`.

The owner's decisions:

- Cleanup is always on. It runs on **This computer** (free, a GGUF from Hugging Face only, llama.cpp, processor only)
  or in the **Cloud** (a connection and its key). New installs start on Cloud. *(Superseded 08-09.11.2026 on branch
  `no-keys`: no cloud and no keys; the choice is Без ИИ, Этот компьютер or Ваш AI, see [no-keys.md](no-keys.md).)*
- Style stays Default | Custom. "Ask" stays on Anthropic; translation follows where cleanup runs. *(Superseded: the
  style is the instructions file, and "Ask" and translation follow the cleanup's route.)*
- Ollama and LM Studio removed. Russian and English only; Parakeet Ultra stays.
- The page is variant A (the switch on top) of the mockup artifact https://claude.ai/artifact/JfXjgJc3xUE43aTh3qNrLP.
- Short list: Qwen3 4B Instruct 2507 (recommended), Gemma 4 E2B, Qwen3 1.7B, all unsloth GGUF, Apache-2.0. Real-model
  smoke: Gemma 4 needed a hand-written `<|turn>` prompt; small models copy `before_cursor` (hence
  `refine::drop_copied_context`); Qwen3 1.7B is weak; only Qwen3 4B translates.
- Follow-ups (04.11.2026): threads = the performance cores Windows reports; no local model in memory on Cloud; any
  model can be deleted, the one in use too, and from the Cloud page; long dictations wait and are cleaned in 80-word
  pieces.

The owner wants CUE usable free, with no key, and simple settings. The models offered will change: see
[model-picks.md](model-picks.md) and [optimize-and-settings.md](optimize-and-settings.md).

State 09.11.2026: the local model stays as built here, now one of the three choices of [no-keys.md](no-keys.md); the
cloud half is gone on that branch. The short list is unchanged because the new catalog was postponed.