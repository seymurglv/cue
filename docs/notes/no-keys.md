# CUE without keys (`no-keys`, merged into `main` 09.11.2026)

Spec `docs/specs/2026-11-08-no-keys-design.md`, plan `docs/specs/2026-11-08-no-keys-plan.md`, approved 08.11.2026. The
branch was cut from `optimize` (which carries the model and Drop rests of that line); after the final whole-branch
review it was merged into `main` (fast-forward) and pushed on 09.11.2026 at the owner's word. The installed CUE is this
build (f5317b2). The ledger with every ruling made while the plan ran:
`.superpowers/sdd/2026-11-08-no-keys-plan/progress.md`. Linear project **P-BAN-1** tracks the progress.

## What CUE does on this branch

- CUE holds **no API key**. Cleanup, translation and "Ask" go through one of three things, chosen on the Cleanup page
  (and in the welcome window's "Кто чистит текст" step): **Без ИИ** (CUE's own tidy-up: dictionary and learned
  spellings, no "эм"/"ээ", tidy spaces), **Этот компьютер** (a model on the processor, downloaded only when pressed),
  **Ваш AI** (the user's OpenCode, the default and first, or Codex).
- The eight cloud providers, the key fields, Check and the balance probes are gone, with prices and dollars in usage
  (`usage.csv` keeps words and time). At the first start CUE deletes the nine API keys earlier versions saved.
- Claude Code and Gemini / Antigravity are never offered (their makers forbid a third-party app driving a user's
  subscription). Grok was built and dropped (below).
- Agents run with no window, no tools and no web (except a searched "Ask" through Codex), the text on standard input.
  OpenCode: the user's server (OpenCode Desktop's, announced in its state file) or one CUE starts in its own folder
  and stops after 15 minutes, on lock, on sleep and on quit. "Быстрая · авто" picks a small fast model from OpenCode's
  list.
- **Ваши инструкции**: one plain file, `%APPDATA%\CUE\instructions\instructions.md`, edited on the page or with the
  user's AI ("Изменить с AI"); the page shows before and after on the last dictation and never loses an edit or an AI's
  save.
- "If the AI didn't answer": paste as said, or clean with the local model when it is loaded.
- Docs: README and ARCHITECTURE describe this behaviour (updated at Task 14, 09.11.2026). The bench no longer compares
  cleanups, and the cloud examples are gone.

## The owner's decisions

- 08.11.2026: no keys at all; variant A of the page; OpenCode first; Claude Code and Gemini not offered; the user
  always chooses which local model downloads; instructions like Hex's, in a file.
- 09.11.2026: **Grok dropped.** A named-pipe route was built and passed its checks, but grok-4.7, the only Grok model
  that cleans dictations without acting on them, takes 6-9 s for a short cleanup; "OpenCode и Codex". The route is in
  the history (5999546 and 2054e46, reverted in 821ac09); `AgentId::Grok` stays only so old settings parse.
- 09.11.2026: **Task 10 (the new local model catalog) postponed**: "не хочу добавлять какие-либо модели сейчас по
  дефолту". The This-computer list shows today's three models (Qwen3 4B Instruct recommended, Gemma 4 E2B, Qwen3 1.7B)
  plus Hugging Face search. cleanup-1, S1 Mini and RuadaptQwen3 4B, the speed record and the "Только чистка" rows wait;
  see [model-picks.md](model-picks.md).
- 09.11.2026: OpenCode's "Быстрая · авто" **keeps Haiku**: it may pick Claude Haiku through the owner's own OpenCode
  connection ("Оставить Haiku").
- 09.11.2026: Task 14 includes installing the new build over the everyday CUE.

## Live facts (09.11.2026, the owner's computer)

- OpenCode Desktop 2.2.5: about 2 s for a short dictation, 5-8 s for 165 words, hence the agents' budget of
  10 s + 1 s per 100 words (plus 5 s on the first request).
- Codex 0.166: about 3-4 s; the flags used leave about 39 % fewer tokens and drop no sentence.
- Web search through Codex needs its code-mode host on: with it off Codex has no web access at all, so a searched
  "Ask" keeps the host on and everything else off; cleanup keeps it off. Probes with that flag set (list, read,
  write + whoami) showed no tool item and refused all three.
- OpenCode 2.2.5 keeps every folder a request names as one of the user's projects, so CUE's cleanup names none and its
  own server runs only in `%LOCALAPPDATA%\CUE\agent\opencode`; one leftover project from the first live run is in the
  owner's OpenCode.
- OpenCode lists about 390 models, so the model drop-down has a filter; the list is read only from a server already
  running (opening settings must not start processes).

## What is left

Done on 09.11.2026: installed over the owner's everyday CUE (data backed up first; at startup it moved the settings to
"Ваш AI · OpenCode", deleted the old keys and wrote the standard instructions); the final whole-branch review and its
fix round; the Drop at the top by default with its mark centered (bars on the circle's center, the red lamp inside near
the upper right); "Быстрая · авто" prefers the provider's own Haiku (a measured Gemini 3.5 Flash Lite was faster; the
owner kept Haiku, `examples/opencode_speed.rs` measures); merged into `main` and pushed.

Left, if the owner wants them: a release (version bump and tag are the owner's), Task 10's models, a bench phase over
the agents, Grok again (if it gets faster).
