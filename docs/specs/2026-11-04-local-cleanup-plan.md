# Cleanup on this computer or in the cloud: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the cleanup runs either on this computer, with a small GGUF model CUE downloads from Hugging Face and runs
itself on the processor, or in the cloud through a connection and a key; Ollama, LM Studio and the address go.

**Architecture:** `cue-core` gets `refine.runs` / `refine.local` and a pure `llm` module (the GGUF runnable check, the
short list, ids, manifests on disk). `cue-engines` gets the Hugging Face search and pinning for GGUF (`hub`), a
llama.cpp engine behind a `Generate` trait (`local_llm`), and the local refine / translate / habit-rule requests
(`local`), which reuse the cloud prompts and answer checks. The runtime gets an `LlmKeeper` beside `ModelKeeper` and
routes refine by `runs`. The settings tray's Cleanup page becomes variant A; `ModelsPage` serves both lists.

**Tech stack:** Rust 2024 (workspace root `Cargo.toml`), Tauri 2, llama.cpp through the `llama-cpp-2` crate (CPU,
static), TypeScript + vitest (jsdom).

**Spec:** `docs/specs/2026-11-04-local-cleanup.md` (binding; read it first). It builds on
`docs/specs/2026-11-02-model-picker.md` and `docs/specs/2026-11-02-refine-connections.md`. Mockup: variant A of
https://claude.ai/artifact/JfXjgJc3xUE43aTh3qNrLP.

## Global constraints

- Files are in English and say nothing about the tools the code was written with.
- No commits unless the owner asks; each task ends with the checks green, not with a commit.
- Downloads come only from `https://huggingface.co`, pinned at a 40-hex commit, every file checked against its SHA-256
  (`Manifest::sound`, `download::fetch`). Gated repositories are never shown or pinned.
- A dictation's text never reaches disk or the log; the log gets outcomes, latencies, sizes and ids only. New `Debug`
  impls that could hold text print lengths.
- "This computer": no request leaves the computer for the cleanup, translation or a habit's rule.
- Runnable GGUF: one file, quantization in this order `Q4_K_M`, `Q4_K_S`, `Q5_K_M`, `Q8_0`, `Q4_0`, at most
  6 GB (`6_000_000_000` bytes), a chat template, an architecture the linked llama.cpp supports.
- Ids: `llm--<owner>--<repository>--<quant>`, lower case. Folder `models\<id>`, manifest `manifest.json`.
- Local time limit: `refine::timeout_ms(raw) + 20_000` (translation: `translate::timeout_ms(raw) + 20_000`).
- Threads: physical cores minus one, at least 1. CPU only: no CUDA, Vulkan, Metal or DirectML features; no OpenMP
  (it would add `vcomp140.dll`).
- New installs: `runs: "cloud"`. Style stays `Default | Custom`; "Ask" stays on the Anthropic key; translation
  follows `runs`.
- Texts in English and Russian, in the family's voice; tray parts and motion as today (`segc` thumb slide, tray
  height follows, rows as the Model page's).
- Checks, from `C:\Users\seymurglv\Desktop\cue-dev`: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`; from `app`: `npx vitest run`,
  `npx tsc --noEmit -p tsconfig.json`. Builds that compile llama.cpp run after `. scripts\dev-env.ps1`.

## Review Focus

1. "This computer" chosen, nothing downloaded, the user dictates: the raw text is pasted at once (no wait like the
   speech slot's `WAIT_FOR_MODEL`). Test in Task 7.
2. The dictionary or a habit changes between two dictations: the cached system prompt must not be reused. Test in
   Task 4 (`PromptCache`).
3. The local model answers late, with broken JSON, or with a reply instead of the text: raw text, outcome
   `RawFallback`, no panic. Tests in Task 5.
4. A repository with several GGUF files (many quants, an `mmproj`, a split model): the right one is picked or the right
   reason shown. Tests in Task 2.
5. Runs switched back to Cloud while a local model downloads: the next dictation goes to the cloud at once; the
   download goes on. Test in Task 7.

## File map

| File | Change |
|---|---|
| `scripts/dev-env.ps1` | new: puts Build Tools' CMake and Ninja on PATH, sets `LIBCLANG_PATH` |
| `scripts/build-release.ps1` | dot-sources `dev-env.ps1` |
| `app/crates/core/src/connections.rs` | Ollama, LM Studio, `local`, `Cost::Disk` go |
| `app/crates/core/src/settings.rs` | `Runs`, `refine.runs`, `refine.local`; `addresses` read past; migration; `RefineRuns`; `RefineAddress` goes |
| `app/crates/core/src/llm.rs` | new: GGUF check, quant order, ids, short list, found models on disk |
| `app/crates/core/src/model.rs` | `found_models` skips `llm--*` (already: lists `hf--*` only; add a test) |
| `app/crates/engines/src/hub.rs` | `search_llms`, `pin_llm`, `LlmFound`, `LlmPinError` |
| `app/crates/engines/src/local_llm.rs` | new: `Generate`, `LocalLlm` (llama.cpp), `PromptCache`, `TEXT_GRAMMAR` |
| `app/crates/engines/src/local.rs` | new: `refine`, `translate`, `rule` over a `Generate` |
| `app/crates/engines/src/{chat,habit,keys}.rs` | local-server branches go |
| `app/crates/engines/Cargo.toml`, root `Cargo.toml` | `llama-cpp-2` |
| `app/src-tauri/src/runtime/llm.rs` | new: `LlmKeeper`, `LlmEvent`, `LlmNews`, `LlmSlot` |
| `app/src-tauri/src/runtime.rs` | `CoreEvent::{Llm, LlmsGet, LlmsSearch, LlmChoose, LlmDelete}`; refine/translate/rule routed by `runs` |
| `app/src-tauri/src/runtime/attempt.rs` | usage id `local:{id}` |
| `app/src-tauri/src/{commands,main,ipc}.rs` | `llms_get`, `llms_search`, `llm_choose`, `llm_delete`; `cue://llms`; `server_check` goes; `SettingsView.refineRuns`, `refineLocal`; `refineAddress` goes |
| `app/src-tauri/src/welcome_window.rs` | no key looked at when `runs` is local |
| `app/crates/cli/src/main.rs` | `pin-llm <repo>` prints a manifest for the short list |
| `app/ui/src/window/{api,models,settings,text}.ts` | the llm API; `ModelsPage` takes a source; the Cleanup page (variant A) |
| `app/ui/src/welcome/{steps,page}.ts` | Keys step skipped when `refineRuns` is `local` |
| `app/ui/test/*` | fixture without local connections; new Cleanup and llm-list tests |
| `NOTICE`, `README.md` | llama.cpp (MIT); the three models and their licenses; "This computer" |

---

### Task 1: The local servers go; `runs` and `local` arrive

**Files:** `connections.rs`, `settings.rs`, `engines/src/{chat,habit,keys}.rs`, `src-tauri/src/{ipc,commands,main,welcome_window}.rs`,
`ui/src/window/{api,settings,text}.ts`, `ui/test/{windowFixture,window-settings.test,window-habits.test}.ts`

**Interfaces:**
- Produces: `pub enum Runs { Local, Cloud }` in `cue_core::settings` (`#[serde(rename_all = "lowercase")]`,
  `Default` = `Cloud`); `RefineSettings.runs: Runs`; `RefineSettings.local: String` (`skip_serializing_if` empty;
  empty means "the recommended one", resolved in Task 2's `llm::default_id()`); `SettingChange::RefineRuns(Runs)`;
  `SettingsView.refine_runs: Runs`, `SettingsView.refine_local: String` (camelCase `refineRuns`, `refineLocal`).
  `CONNECTIONS: [Connection; 8]`.
- Removed: `Connection.local`, `Cost::Disk`, `RefineSettings.addresses`, `RefineSettings::address()`,
  `SettingChange::RefineAddress`, `commands::server_check`, `api.serverCheck`, `SettingsView.refineAddress`,
  `ConnectionView.local`, `ModelView.gb`, `t.server`, `t.addressRow`, `t.connectionGroups`, `t.onDisk`,
  `t.cleanupNoteLocal`.

- [ ] **Step 1: failing Rust tests** in `settings.rs` `mod tests`:
  - `old_local_servers_read_as_this_computer`: `{"refine":{"connection":"ollama","addresses":{"ollama":"http://x"}}}`
    reads as `runs == Runs::Local`, `connection == "anthropic"`; serialized again it has no `addresses` and has
    `"runs":"local"`. The same for `"lmstudio"`.
  - `runs_is_cloud_unless_local`: absent / `"cloud"` / `"LOCAL "` / `"other"` read as Cloud, Cloud, Local, Cloud.
  - `apply_switches_runs`: `RefineRuns(Local)` changes once, a second time reports no change.
  - In `connections.rs`: rename `ten_connections_two_of_them_local` to `eight_cloud_connections` asserting the ids
    `anthropic, openai, gemini, openrouter, deepseek, mistral, groq, xai`.
  - Drop `refine_address_is_the_local_server_s`; remove the address cases from `apply_reports_no_change` and
    `apply_refuses_what_the_cleanup_could_not_use`.
- [ ] **Step 2:** `cargo test -p cue-core` fails on the new tests.
- [ ] **Step 3: implement.** `connection_id` maps `ollama`/`lmstudio` to the default and the struct's custom
  deserialize (or a `#[serde(from = ...)]` raw form) sets `runs = Local` when the stored connection was one of them;
  `addresses` is accepted and dropped. Remove every `local` use the compiler finds in `chat.rs`
  (`LOCAL_TIME_HEADROOM_MS`, the empty key), `habit.rs`, `keys.rs` tests, `ipc.rs`, `commands.rs` (`server_check`,
  `server_address`, `SERVER_CHECK_TIMEOUT`, its tests and its `COMMANDS` entry, now 27), `main.rs`,
  `welcome_window.rs` tests (`start_key("ollama")`).
- [ ] **Step 4: UI.** `api.ts`: `SettingsView` loses `refineAddress`, gains `refineRuns: 'local' | 'cloud'` and
  `refineLocal: string`; `SettingChange` loses `refineAddress`, gains `{ kind: 'refineRuns'; value: 'local' | 'cloud' }`;
  `ConnectionView` loses `local`; `serverCheck` goes. `settings.ts`: `addressRow`, `serverCheck`, `servers`, the local
  branches of `readiness`, `connectionMenu` (no groups) and the page note go. `windowFixture.ts`: eight connections, no
  `ADDRESSES`, `view()` gets `refineRuns: 'cloud', refineLocal: ''`. Tests: drop `'a local server has an address row…'`
  and the habits tests' ollama cases; `'the connection card lists ten connections in two groups…'` becomes
  `'the connection card lists the eight cloud connections'`.
- [ ] **Step 5:** all checks green.

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

### Task 3: Search and pin GGUF repositories (`hub`)

**Files:** `app/crates/engines/src/hub.rs` (+ tests there).

**Interfaces:**
- Consumes: `llm::{runnable, id_for, GgufMeta, LlmNotRunnable, Pick}`, `model::{Manifest, write_manifest}`.
- Produces:
  - `pub struct LlmFound { pub repo: String, pub downloads: u64, pub size: Option<u64>, pub quant: Option<String>, pub reason: Option<LlmNotRunnable> }` (camelCase)
  - `pub async fn search_llms(http: &reqwest::Client, query: &str) -> Result<Vec<LlmFound>, EngineError>`
  - `pub enum LlmPinError { NotRunnable(LlmNotRunnable), Failed(EngineError) }`
  - `pub async fn pin_llm(http: &reqwest::Client, repo: &str, models: &Path) -> Result<Manifest, LlmPinError>`

- [ ] **Step 1: failing tests** with `testing::fake_api`:
  - `llm_search_asks_for_gguf`: the request path is
    `/api/models?search=qwen&filter=gguf&sort=downloads&limit=30&expand%5B%5D=siblings&expand%5B%5D=downloads&expand%5B%5D=tags&expand%5B%5D=gated&expand%5B%5D=pipeline_tag&expand%5B%5D=gguf`.
  - `llm_search_keeps_open_text_models`: gated, `pipeline_tag: "automatic-speech-recognition"` and
    `"image-text-to-text"` entries are left out; `"text-generation"` and none stay; at most 10.
  - `llm_search_says_why_not`: a split-only repo comes back with `reason: SeveralFiles`, no size.
  - `pin_llm_writes_a_sound_manifest`: commit from `sha`, the picked file with its LFS SHA-256 and size, id from
    `id_for`, license from `cardData`; `models/<id>/manifest.json` exists; no other file is downloaded.
  - `pin_llm_refuses_gated_and_unrunnable`.
- [ ] **Step 2:** `cargo test -p cue-engines hub` fails.
- [ ] **Step 3: implement** beside `search`/`pin`: `Listed` gains `gguf: Option<GgufListed>` (`architecture`,
  `chat_template` presence); `pin_llm` uses `details()` (`?blobs=true`) for the LFS hash. The size of a runnable result
  is the picked file's.
- [ ] **Step 4:** checks green.

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

### Task 5: Local refine, translation and habit rule (`local`)

**Files:** create `app/crates/engines/src/local.rs`; `lib.rs`; tests in the file.

**Interfaces:**
- Consumes: `Generate`; `claude::{system_blocks, decide, decide_translation, raw_fallback, Job, RefineResult}`;
  `habit::{RuleAsk, RuleText}` and the rule messages; `refine::timeout_ms`, `translate::timeout_ms`.
- Produces:
  - `pub async fn refine(llm: Arc<dyn Generate>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings) -> RefineResult`
  - `pub async fn translate(llm: Arc<dyn Generate>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings, to: &'static str) -> RefineResult`
  - `pub async fn rule(llm: Arc<dyn Generate>, s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText>`
  - `pub const LOCAL_HEADROOM_MS: u64 = 20_000;`

- [ ] **Step 1: failing tests** with a fake `Generate` (records `system`/`user`, returns a set answer or error):
  - `local_refine_sends_the_cloud_prompt`: `system` equals the chat connection's joined system blocks for the same
    settings (dictionary, habits, custom style included); `user` equals `refine::user_message(ctx, raw)`.
  - `local_refine_checks_the_answer`: `{"text":"Привет, мир."}` for raw `привет мир` → `Refined`; an answer that adds a
    reply word ("Конечно! …") → `RawFallback` with the raw text; broken JSON → `RawFallback`.
  - `local_refine_stops_at_the_deadline`: the fake returns `TimedOut`; the deadline passed is
    `timeout_ms(raw) + 20_000` from now (±100 ms) → `RawFallback`.
  - `local_refine_costs_nothing`: `usage` is zero.
  - `local_translate_uses_the_translation_check`; `local_rule_reads_the_rule_or_none`.
- [ ] **Step 2:** `cargo test -p cue-engines local` fails.
- [ ] **Step 3: implement** by running `generate` in `tokio::task::spawn_blocking`; a panic or join error is
  `RawFallback`. The system text is built with the same function the chat client uses (make it `pub(crate)` if needed).
- [ ] **Step 4:** checks green.

### Task 6: `cue-cli pin-llm`

**Files:** `app/crates/cli/src/main.rs` (+ its tests).

**Interfaces:** consumes `hub::pin_llm`. Produces the subcommand `pin-llm <owner/repo>`: pins into a temporary folder
and prints the manifest as JSON (id, repo, commit, the file's path, size, SHA-256, license), for Task 10 to copy into
`llm::catalog()`.

- [ ] **Step 1:** test `pin_llm_args_are_one_repository` (no argument / two arguments / a URL are refused).
- [ ] **Step 2:** implement; `cargo run -p cue-cli -- pin-llm Qwen/Qwen3-1.7B-GGUF` prints a manifest.
- [ ] **Step 3:** checks green.

### Task 7: The runtime: `LlmKeeper` and routing by `runs`

**Files:** create `app/src-tauri/src/runtime/llm.rs`; `runtime.rs`, `runtime/attempt.rs`, `commands.rs`, `main.rs`,
`ipc.rs`.

**Interfaces:**
- Consumes: `llm::{catalog, resolve, found, default_id}`, `hub::{search_llms, pin_llm}`, `download::{fetch, verify, ready, present}`,
  `local_llm::{LocalLlm, Generate}`, `local::{refine, translate, rule}`, `model::{EntryState, ModelEntry, ModelsView}`.
- Produces:
  - `pub struct LlmSlot` with `pub fn ready(&self, llm: Arc<dyn Generate>)`, `pub fn clear(&self)`,
    `pub fn get(&self) -> Option<Arc<dyn Generate>>` (never waits).
  - `pub enum LlmEvent { Progress { done: u64, total: u64 }, Fetched, FetchFailed(EngineError), Loaded(Result<Arc<dyn Generate>, EngineError>), Verified { removed: usize }, Pinned(Result<Manifest, LlmPinError>) }`,
    `pub struct LlmNews { pub job: u64, pub event: LlmEvent }`.
  - `pub struct LlmKeeper` with `new(models: PathBuf, chosen: &str)`, `start`, `on_event -> Told`, `choose(Choice)`,
    `delete(&str) -> bool`, `view() -> ModelsView`, `slot() -> Arc<LlmSlot>`, `state() -> ModelState` (same shapes as
    `ModelKeeper`'s; the `Loader` type is `fn(&Path, &str) -> Result<Arc<dyn Generate>, EngineError>`).
  - `CoreEvent::{Llm(LlmNews), LlmsGet(oneshot::Sender<ModelsView>), LlmsSearch { query, reply }, LlmChoose(Choice), LlmDelete { id, reply }}`.
  - Commands `llms_get`, `llms_search(query)`, `llm_choose(id | repo)`, `llm_delete(id)` (main window only, same limits
    as the `models_*` ones); `Ipc::llms(&ModelsView)` emits `cue://llms`; `Ipc::llm(ModelState)` emits `cue://llm`.

- [ ] **Step 1: failing tests** in `runtime/llm.rs` (fake loader and free space, as `runtime/model.rs` tests do):
  `chosen_model_downloads_then_loads`, `a_switch_keeps_the_model_in_use_until_the_new_one_loads`,
  `delete_refuses_the_model_in_use`, `no_space_says_so`, `a_model_that_will_not_load_is_not_runnable`,
  `the_chosen_id_is_written_only_once_loaded` (the keeper reports `Told::Switched`).
- [ ] **Step 2: failing routing tests** in `runtime.rs`' tests (existing fake-engine helpers):
  `local_with_nothing_ready_pastes_raw_at_once` (Review Focus 1: outcome `RawFallback` within 100 ms),
  `local_refine_uses_the_local_engine`, `back_to_cloud_goes_to_the_cloud_at_once` (Review Focus 5),
  `translation_follows_runs`, `ask_stays_on_anthropic`, `local_habit_rule_never_asks_the_cloud`.
- [ ] **Step 3: implement.** The keeper starts at launch after the speech model when `runs` is Local, and when
  `RefineRuns(Local)` is applied; it stays loaded after a switch to Cloud (the builder may unload it; say which in
  `llm.rs`). `refine()` matches `runs` first: Local → `local::refine/translate(slot.get()?)`, else today's match;
  no model ready → `claude::raw_fallback(raw, RawFallback, Usage::default(), "local model not ready")`. `ask_rule` uses
  `local::rule` when Local (no model: the template). `attempt.rs`: `refine_model` is `format!("local:{id}")` when
  Local. `refine.local` is written when `Told::Switched` comes, as `stt.model` is.
- [ ] **Step 4:** checks green.

### Task 8: The Cleanup page (variant A)

**Files:** `app/ui/src/window/{api,models,settings,text}.ts`, `app/ui/src/design/window.css` (only if a part is
missing), tests `app/ui/test/{window-models,window-settings,window-cleanup}.test.ts` (new file for the page).

**Interfaces:**
- Consumes: Task 7's commands and events.
- Produces:
  - `api.ts`: `llmsGet`, `llmsSearch(query) -> LlmFound[]`, `llmChoose({id}|{repo})`, `llmDelete(id)`, `onLlms(cb)`,
    `llmGet`/`onLlm` (`ModelState`), `type LlmFound`, `type LlmNotRunnable`.
  - `models.ts`: `export type ModelSource = { texts: ModelTexts; search(q): Promise<FoundRow[]>; choose(c): Promise<void>; remove(id): Promise<boolean>; reasons: Record<string, string>; line(e: ModelEntry): string }`;
    `ModelsPage` constructor `(ctx, resized, source: ModelSource)`; `speechSource(ctx)` and `llmSource(ctx)` factories.
  - `text.ts`: `runsOn`, `runsHint{local,cloud}`, `runs{local,cloud}`, `llms{list,hint,recommended,…,reasons{severalFiles,noChatTemplate,tooLarge,architecture,noQuant}}`,
    `cleanupNoteLocal` (new text: "Free. The text never leaves this computer; the model runs on its processor."),
    `cleanupValue{local,notDownloaded,downloading}`; EN and RU.

- [ ] **Step 1: failing tests** (`window-cleanup.test.ts`):
  - the page shows Style, then "Runs on" with "This computer | Cloud" (pressed follows `refineRuns`);
  - Cloud shows the card (eight connections, no groups), the key row and the model drop-down; This computer shows the
    "Models" list, the search row and the local note; pressing the other half sends `{kind:'refineRuns'}` and the page
    morphs (the tray's height changes, the thumb moves);
  - the local list: recommended line, check on the one in use, size on one missing, Delete on hover, "Downloading",
    "Loading", Retry, no space, the search results with pale rows and their reasons (as the Model page tests, through
    `llmSource`);
  - the first page's Cleanup value: "This computer", "Model not downloaded" (amber), "Downloading 42 %" (amber),
    the connection name, "No Anthropic key" (amber);
  - both themes render with no missing token (`getComputedStyle` of the row text is not empty in `data-theme=dark`).
  - `window-models.test.ts` keeps passing unchanged through `speechSource`.
- [ ] **Step 2:** `npx vitest run` fails on them.
- [ ] **Step 3: implement.** `settings.ts`: `SettingsTray` holds a second `ModelsPage` with `llmSource`; `cleanupRows`
  is Style, the `runs` segment (`segment('runs', …)`, thumb as `syncSeg`), then the local list rows or the cloud rows;
  `paintCleanupValue` covers the states above; the page subscribes to `onLlms`/`onLlm`.
- [ ] **Step 4:** checks green; look at the page in the browser pane on the mock core (`merlin-skin/tools/tauri-mock.md`)
  in both themes against variant A, and stop the preview server.

### Task 9: The welcome window with `runs: "local"`

**Files:** `app/src-tauri/src/welcome_window.rs`, `main.rs:184`, `app/ui/src/welcome/{steps,page}.ts`,
`app/ui/test/welcome.test.ts`.

**Interfaces:** `open_at_start(app, refine: &RefineSettings, forced)`; `fn start_key(refine: &RefineSettings) -> Option<KeyName>`
returns `None` when `runs` is Local. UI: `stepsFor(view: SettingsView): Step[]` without `'keys'` when `refineRuns`
is `'local'`.

- [ ] **Step 1: failing tests:** `the_key_is_not_looked_at_on_this_computer`; welcome test
  `'with the cleanup on this computer the Keys step is skipped'`.
- [ ] **Step 2:** implement; checks green.

### Task 10: The short list, documents, release

**Files:** `app/crates/core/src/llm.rs` (`catalog()`), `NOTICE`, `README.md`, `scripts/build-release.ps1`.

- [ ] **Step 1: pick three models** from Hugging Face model cards: instruction-tuned, 1.5–4 B parameters, a
  `Q4_K_M` single-file GGUF, openly licensed, not gated, Russian and English named on the card; the strongest first
  (the spec suggests Gemma 4 E4B, Qwen3 4B, Qwen3 1.7B, or what is current). Prefer the publisher's own GGUF
  repository.
- [ ] **Step 2:** `cargo run -p cue-cli -- pin-llm <repo>` for each; copy the printed values into `llm::catalog()`;
  remove the test placeholder; `catalog_is_sound` passes.
- [ ] **Step 3: smoke by hand:** for each model, `CUE_TEST_GGUF=<downloaded file>` runs Task 4's ignored test; then in
  the release build, Cleanup → This computer, choose the model, wait for "Loading" to clear, dictate one Russian and
  one English phrase: cleaned text is pasted, the log shows the local latency. Record each model's latency in this
  plan's notes (no comparison with the cloud: the owner's decision).
- [ ] **Step 4: documents.** NOTICE: llama.cpp (MIT, copyright line from its LICENSE) and each model's name, repository
  and license (Gemma terms or Apache-2.0 as the card says). README: the Cleanup section says "This computer (free, a
  model from Hugging Face) or Cloud (a connection and your key)", the models' sizes, and that Ollama and LM Studio are
  no longer used.
- [ ] **Step 5:** `scripts\build-release.ps1` builds; `cargo run -p cue-cli -- leak-scan` on the installer as in the
  release routine; `dumpbin /dependents` check from Task 4 Step 6.
- [ ] **Step 6: report to the owner** what was built, the three models, the smoke latencies, what was not checked
  (quality on real dictations), and ask whether to commit.
