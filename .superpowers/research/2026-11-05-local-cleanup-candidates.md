# Local cleanup model candidates (to replace Gemma), researched 2026-11-05

Parked for step 5 (after language modes, GigaAM check, Haiku in both modes, settings).

Latency note (estimate, no CPU measurements found): a 2.5 GB dense Q4 model gives ~15–25 tok/s on an i7-12700KF,
so an 80-word Russian piece (130–200 tokens) takes 6–10 s. 1–2 s needs ≤1.7B dense, an MoE with 1–2B active
parameters, or a tokenizer that spends fewer tokens on Cyrillic.

Ranked shortlist to benchmark against Haiku's outputs on the owner's recordings:

1. GigaChat 3.1 Lightning, MoE 10B / 1.8B active, MIT. GGUF ai-sage/GigaChat3.1-10B-A1.8B-GGUF (Q4_K_M 6.47 GB,
   llama.cpp build ≥ 8495). Vendor Russian scores above Qwen3-4B-2507. Unknown: CPU speed, Cyrillic tokens, 6.5 GB RAM.
2. RuadaptQwen3-4B-Instruct (RefalMachine), 4B, Apache-2.0, Russian-extended tokenizer (claims up to 2x faster on
   Russian). GGUF RefalMachine/RuadaptQwen3-4B-Instruct-GGUF (2.49 GB). Pin a commit; temp 0–0.3.
3. Qwen3.5-4B / 2B, Apache-2.0, 250k vocab (10–60% fewer tokens). GGUF unsloth/Qwen3.5-4B-GGUF (2.74 GB). Thinking
   on by default (enable_thinking false); Gated DeltaNet reportedly slow on CPU in mainline llama.cpp.
4. T-lite-it-2.1 (T-Bank), 8B on Qwen3-8B, Apache-2.0. GGUF t-tech/T-lite-it-2.1-GGUF (5.03 GB). Best-documented
   Russian quality (Ru Arena Hard 83.95 vs 57.23 Qwen3-8B); ~2x slower.
5. Spokenly cleanup-1, Qwen3-0.6B base, Apache-2.0, built for fillers and spoken corrections; Russian supported but
   weaker; custom "key: value … out:" prompt. The fast floor.
6. Optional: YandexGPT-5-Lite-8B-instruct (license: 10M output tokens/month cap, Russian law, attribution).

Ruled out: Vikhr (no recent text instruct), LFM2-8B-A1B (no Russian), Ministral 3 3B, Granite 4.0, SmolLM3, Phi-4-mini,
Llama 3.2 small.

Method: tokens per word on bench transcripts; llama-bench prompt and generation tok/s; quality against Haiku.
