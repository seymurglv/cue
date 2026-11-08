### Task 4: The llama.cpp engine (`local_llm`)

**Files:** create `scripts/dev-env.ps1`, `app/crates/engines/src/local_llm.rs`; `engines/Cargo.toml`, root
`Cargo.toml`, `engines/src/lib.rs`, `scripts/build-release.ps1`.

**Interfaces:**
- Produces:
  - `pub trait Generate: Send + Sync { fn generate(&self, system: &str, user: &str, deadline: std::time::Instant) -> Result<String, EngineError>; fn id(&self) -> &str; }`
  - `pub struct LocalLlm` with `pub fn load(path: &Path, id: &str) -> Result<Arc<LocalLlm>, EngineError>` and
    `impl Generate for LocalLlm`.
  - `pub const TEXT_GRAMMAR: &str` (GBNF for exactly `{"text": <JSON string>}`).
  - `pub struct PromptCache` with `pub fn reusable(&self, system: &str) -> bool` and `pub fn remember(&mut self, system: &str, tokens: usize)`
    (keyed by the system text's hash and the token count evaluated).
  - `pub fn threads(physical: usize) -> usize` (`max(1, physical - 1)`).
  - `EngineError` gains `NotRunnable` (loading failed: architecture, template) and `TimedOut` if absent.

- [ ] **Step 1: toolchain.** `scripts/dev-env.ps1` adds
  `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\IDE\CommonExtensions\Microsoft\CMake\{CMake\bin,Ninja}`
  to PATH (found with `vswhere`, falling back to that path) and sets `LIBCLANG_PATH` to LLVM's `bin`. LLVM is not
  installed on this computer: ask the owner before running `winget install --id LLVM.LLVM -e`.
- [ ] **Step 2:** add `llama-cpp-2` (latest release, pinned with `=`, `default-features = false`, no GPU or OpenMP
  features) to `[workspace.dependencies]` and `cue-engines`. `cargo build -p cue-engines` after `. scripts\dev-env.ps1`
  builds; `build-release.ps1` dot-sources `dev-env.ps1`.
- [ ] **Step 3: failing pure tests:** `grammar_accepts_only_the_text_object` (llama.cpp's grammar parser accepts
  `TEXT_GRAMMAR`; a sample `{"text":"Привет, \"мир\""}` matches and `Sure! {"text":"x"}` does not, through the
  crate's grammar check if it has one, otherwise a parse-only test plus Task 5's answer checks);
  `prompt_cache_is_kept_for_the_same_system_text` and `prompt_cache_is_dropped_when_the_system_text_changes`;
  `threads_leave_one_core` (`threads(8) == 7`, `threads(1) == 1`); `architectures_are_known_to_the_build` (every
  `llm::ARCHITECTURES` name is one the linked llama.cpp knows, if the crate exposes the list; otherwise drop this test
  and note it in `local_llm.rs`).
- [ ] **Step 4: implement `LocalLlm`.** One model and one context, behind a `Mutex`; context length 4096; the chat
  template from the GGUF metadata (no template: `NotRunnable`); the system part evaluated once and kept while
  `PromptCache::reusable`, else the context is cleared and re-evaluated; sampling greedy (temperature 0) under
  `TEXT_GRAMMAR`; generation stops at the grammar's end, at 1,024 tokens, or at `deadline` (`TimedOut`). Thread count
  from `threads(physical cores)`.
- [ ] **Step 5: an ignored smoke test** `local_llm_answers_under_the_grammar`, run with `CUE_TEST_GGUF=<path>`
  `cargo test -p cue-engines -- --ignored local_llm`: loads the file, `generate("Return the user's text.", "привет мир", +60 s)`
  parses as `{"text": …}`.
- [ ] **Step 6:** checks green; `dumpbin /dependents target\release\cue.exe` (after Task 10's release build) lists no
  DLL beyond today's (`DirectML.dll` and system DLLs).

