# Branch `optimize`, the settings redesign and the no-AI layer (parked)

State on 05.11.2026, parked by the owner ("we'll continue after some time"):

- Branch `optimize` (from `main` 883977c, pushed 08.11.2026, not merged):
  - 87aeb32: the speech model rests after 15 min without a dictation, on lock and on sleep; ONNX arena shrink
    (1.8 → 0.75 GB held); Windows throttling off during a dictation; the Drop breathes three times and then holds
    still (WebView2 idle 11% → 0.4%); slower hit test while hidden or far; ONNX Runtime info lines out of the log;
    release profile with fat LTO and stripped symbols.
  - 7e0e9fa: the local cleanup model rests too; the first cleanup after a rest waits up to 15 s for it.
  - a261fe2: two specs.
- The settings redesign, the owner's decisions: Parakeet Ultra for both languages, the language from speech (no
  keyboard-layout modes, no GigaAM for now). Cleanup = two choices: AI in the cloud with a key (all 8 services as
  tiles, Haiku recommended for Russian + English) or a local model (Gemma rejected). A "Set up with an AI agent"
  button copies a prompt (needs CUE to reload `settings.json` when it changes; the agent never handles keys). Layout
  (two cards or a switch) not picked.
- The no-AI layer: the owner said yes, option A (exact matches only). Design
  `docs/specs/2026-11-05-no-ai-layer-design.md` **waits for approval**, then a plan.
- `docs/specs/2026-11-05-cleanup-settings-design.md` is a parked draft, to be rewritten around the decisions above.
- Local cleanup candidates to measure against Haiku: `.superpowers/research/2026-11-05-local-cleanup-candidates.md`.
- Artifacts: review https://claude.ai/artifact/Y82iznYzRpoH8pvTR3eHdR, prototype
  https://claude.ai/artifact/E3FBicdijn4oEnxcpGEVj5. Mockups also in `.superpowers/mockups`.

The owner wants an optimized, simple product before the public release. When work resumes, start from the no-AI
design approval and the settings spec rewrite. **Ask before spending API money on measurements.**
