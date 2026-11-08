### Task 2: Which GGUF models CUE can run (`cue_core::llm`)

**Files:** create `app/crates/core/src/llm.rs`; `lib.rs` (`pub mod llm;`); test in the same file.

**Interfaces:**
- Consumes: `model::{Manifest, ModelFile, RepoFile, read_manifest, MANIFEST_FILE}`.
- Produces:
  - `pub const QUANTS: [&str; 5] = ["Q4_K_M", "Q4_K_S", "Q5_K_M", "Q8_0", "Q4_0"];`
  - `pub const MAX_BYTES: u64 = 6_000_000_000;`
  - `pub const ARCHITECTURES: &[&str]`: the architectures the linked llama.cpp loads (`llama`, `qwen2`, `qwen3`,
    `gemma2`, `gemma3`, `gemma3n`, `phi3`, `mistral3`, plus what the pinned llama.cpp lists; Task 4 checks the list
    against the build).
  - `pub enum LlmNotRunnable { SeveralFiles, NoChatTemplate, TooLarge, Architecture, NoQuant }`
    (`Serialize`, camelCase).
  - `pub struct GgufMeta { pub architecture: Option<String>, pub chat_template: bool }` (from the listing's `gguf`).
  - `pub struct Pick { pub file: RepoFile, pub quant: &'static str }`
  - `pub fn runnable(files: &[RepoFile], meta: &GgufMeta) -> Result<Pick, LlmNotRunnable>`
  - `pub fn id_for(repo: &str, quant: &str) -> String` (`llm--owner--repo--q4_k_m`)
  - `pub fn catalog() -> Vec<Manifest>` (the short list, recommended first; Task 10 fills real values, until then one
    placeholder entry is allowed only behind `#[cfg(test)]`)
  - `pub fn default_id() -> String` (the catalog's first id)
  - `pub fn resolve(models: &Path, id: &str) -> Option<Manifest>`, `pub fn found(models: &Path) -> Vec<Manifest>`
    (folders starting `llm--` with a sound manifest, not in the catalog).

- [ ] **Step 1: failing tests:**
  - `picks_the_first_quant_in_order`: files `x-Q8_0.gguf`, `x-Q4_K_M.gguf`, `x-Q5_K_M.gguf` → `Q4_K_M`.
  - `ignores_projectors_and_non_gguf`: `mmproj-x-f16.gguf`, `README.md`, `x-Q4_0.gguf` → `Q4_0`.
  - `split_models_are_several_files`: only `x-Q4_K_M-00001-of-00002.gguf` and `-00002-of-00002` → `SeveralFiles`.
  - `too_large`: `x-Q4_K_M.gguf` of `6_000_000_001` bytes and nothing smaller → `TooLarge`.
  - `no_template_and_unknown_architecture`: `chat_template: false` → `NoChatTemplate`; `architecture: "bert"` →
    `Architecture`; `architecture: None` passes (checked at load).
  - `ids_are_lower_case`: `id_for("Qwen/Qwen3-4B-GGUF", "Q4_K_M") == "llm--qwen--qwen3-4b-gguf--q4_k_m"`.
  - `found_lists_only_llm_folders`, and in `model.rs` `found_models_skips_language_models`.
  - `catalog_is_sound`: every entry passes `Manifest::sound`, has one file ending `.gguf`, its id is `id_for(repo,
    quant)`, and `default_id()` is the first.
- [ ] **Step 2:** `cargo test -p cue-core llm` fails.
- [ ] **Step 3: implement.** Quant match is case-insensitive on the file stem's last `-` or `.` part; files whose name
  starts with `mmproj` are skipped; the split pattern is `-\d{5}-of-\d{5}\.gguf$`.
- [ ] **Step 4:** checks green.

