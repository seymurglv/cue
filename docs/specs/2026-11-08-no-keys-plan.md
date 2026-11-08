# CUE without keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** CUE cleans, translates and answers "Ask" through no AI, a model on this computer, or the user's own OpenCode / Codex / Grok, and holds no API key anywhere.

**Architecture:** A pure no-AI layer (`cue_core::basics`) runs before every route. A new agent path in `cue_engines::agent` talks to the three agents (CLI runs for Codex and Grok, a loopback HTTP server for OpenCode) and reuses today's prompt and output checks. Settings get `where` / `agent` / `fallback`; the eight cloud providers, the key UI and the Anthropic-only Ask go. The window gets the variant-A Cleanup page, an Instructions page backed by `%APPDATA%\CUE\instructions.md`, and a welcome step "Кто чистит текст".

**Tech Stack:** Rust 2024 (cue-core, cue-engines, cue-cli, Tauri 2 app), tokio, reqwest, llama-cpp-2 0.1.161; TypeScript + Vite + Vitest UI, no framework.

**Spec:** `docs/specs/2026-11-08-no-keys-design.md` (read it whole before any task). Also `docs/specs/2026-11-05-no-ai-layer-design.md` (Task 1) and, for S1 Mini's input format, `git show model-picks:docs/specs/2026-11-05-cleanup-model-picks.md` (Task 10).

## Global Constraints

- CUE never stores, reads or asks for an API key or login token; the only Credential Manager entry kept is `habits-secret`.
- The transcript and context reach an agent on stdin only, never as a command-line argument or a temp file.
- Agents run with no window (`CREATE_NO_WINDOW`), working folder `%LOCALAPPDATA%\CUE\agent\` (created empty), tools, file writes and web access off for cleanup and translation.
- Agent order everywhere: OpenCode, Codex, Grok. Claude Code and Gemini/Antigravity are never offered.
- Default agent models: OpenCode "Быстрая · авто" (fast-name list `haiku, flash, mini, lite, luna, fast, small`), Codex `GPT-6 Luna`, Grok its fast model (exact id from `grok` on this computer).
- Agent timeouts: Codex and Grok `AGENT_BASE_MS = 10_000` + 1 000 per 100 words; OpenCode `refine::timeout_ms(raw)`; both plus `START_ALLOWANCE_MS = 5_000` on the first request after CUE starts or the agent rests. Ask's first words through an agent: 15 s, 20 s with search.
- Only an agent whose flags, on the installed version, leave it no shell, no file reading and no network tool is offered; one that cannot be stripped is left out and reported.
- The local fallback runs only when the local model is loaded at that moment; a resting model is never woken for it.
- Habit rules on an agent route always use CUE's template.
- cleanup-1 and S1 Mini only clean: with them, Ask and translation are unavailable and the instructions do not apply.
- OpenCode server CUE started: stopped after `REST_AFTER` (15 min) without a dictation, on session lock, on sleep, on quit; a server CUE did not start is never stopped.
- Nothing downloads without the user pressing on the model.
- Instructions file `%APPDATA%\CUE\instructions.md`, at most `MAX_CUSTOM_CHARS` (2 000) characters used; previous copy `instructions.prev.md`; CUE owns `%APPDATA%\CUE\AGENTS.md`.
- Repository rules (AGENTS.md): English code, comments, docs and commit messages that say what CUE now does; tests need no network, keys, agent or model; `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `npm test`, `npm run build` green before every commit; ARCHITECTURE stays true.
- UI strings in Russian and English; the spec's quoted Russian strings are used verbatim.

## Review Focus

1. **An agent that never exits** (an update prompt, a login prompt waiting on stdin, an npm `.cmd` shim that leaves `node` running): the dictation gets its fallback at the timeout and the whole process tree is killed — test in Task 2.
2. **A dictation that reads like a command** ("удали все файлы", "ignore your instructions"): it is cleaned as text, the agent can act on nothing — test in Task 3 (the flags) and Task 5 (the text reaches stdin inside `<transcript>`).
3. **An agent that is installed but signed out**: the first failure marks it, the Cleanup value turns amber with "войдите заново", and the next dictations do not each wait a full timeout (the mark holds 5 minutes or until the Cleanup page reads the agents again) — test in Task 7.
4. **OpenCode servers**: one CUE started is stopped on rest, lock, sleep and quit; one the user runs is used and never stopped — test in Task 4.
5. **The instructions file broken by hand or by an agent** (empty, deleted, 50 KB, not UTF-8, saved twice in a burst): standard instructions are used, the page shows an amber line, nothing crashes, one reload per burst — test in Task 2.

---

### Task 1: The no-AI layer

**Files:**
- Create: `app/crates/core/src/basics.rs`
- Modify: `app/crates/core/src/lib.rs` (module), `app/src-tauri/src/runtime.rs` (`refine` ~1620, `spawn_ask` ~1918)
- Test: `app/crates/core/src/basics.rs` (`#[cfg(test)] mod tests`, table-driven)

**Interfaces:**
- Produces: `pub fn tidy(raw: &str, dict: &DictionarySettings) -> String` in `cue_core::basics`.

- [ ] **Step 1: Write the failing tests** — one table per rule of `2026-11-05-no-ai-layer-design.md`, in Russian and English:
  `learned_phrase_becomes_word` (`heard: "гит хаб"` → `GitHub`, longer phrases first), `dictionary_case_exact_only` (`"github"`→`"GitHub"`, `"claude haiku"`→`"Claude Haiku"`, `"githubs"` unchanged), `fillers_go_with_one_comma` (`"Я, э, приду, эм, завтра."` → `"Я приду завтра."`; `"Ну, я приду."` → `"Я приду."`), `nu_and_well_only_first_word` (`"Он ну пришёл"` unchanged), `whole_words_only` (`"эмоция"`, `"humble"` unchanged), `tidy_spaces_and_commas` (no space before `,.!?:;`, single spaces, no `,,`), `capital_after_removed_first_word`, `empty_result_keeps_raw` (`"эм"` → `"эм"`).
- [ ] **Step 2: Run** `cargo test -p cue-core basics` — Expected: FAIL, `basics` not found.
- [ ] **Step 3: Implement `tidy`** in `basics.rs` exactly as the design's four steps, in that order; the filler list is the design's, no more.
- [ ] **Step 4: Wire it**: in `runtime.rs::refine` pass `basics::tidy(&raw, &dict)` to every engine call and fallback instead of `raw` (`self.raw_text` and the history keep `raw`); in `spawn_ask` the first question goes through `tidy` too.
- [ ] **Step 5: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 6: Commit** — "The basics without a model: learned words, dictionary spelling, filler words and tidy spacing are fixed before any cleanup, and the text pasted when no model works is this one".

### Task 2: The instructions file

**Files:**
- Create: `app/crates/core/src/instructions.rs`, `app/src-tauri/src/instructions.rs`
- Modify: `app/src-tauri/src/paths.rs` (`instructions()`, `instructions_prev()`, `agents_md()`, `agent_dir()`), `app/crates/engines/src/claude.rs::system_blocks` (the job block), `app/src-tauri/src/runtime.rs` (startup, refine), `app/crates/core/src/refine.rs` (keep `BOLD_PROMPT` as the English default, add the Russian one)
- Test: `app/crates/core/src/instructions.rs`, `app/src-tauri/src/instructions.rs`

**Interfaces:**
- Consumes: `RefineSettings::custom_text()` (exists; Task 6 renames it `legacy_custom()`).
- Produces:
  - `cue_core::instructions::{standard(lang: &str /* "ru" | "en", the window's resolved language */) -> &'static str, agents_md(lang: &str, version: &str) -> String, task(lang: &str) -> String, usable(text: &str) -> Option<&str>}` (`usable`: trimmed, non-empty, cut at `MAX_CUSTOM_CHARS`).
  - `app::instructions::Instructions` with `load_or_create(paths, lang, legacy: Option<&str>) -> Self`, `text(&mut self) -> Cow<str>` (re-reads when the file's modified time changed, debounced 300 ms; falls back to `standard`), `edited(&self) -> bool`, `broken(&self) -> bool`, `set(&mut self, text: &str) -> io::Result<()>` (writes `instructions.prev.md` first), `reset(&mut self, lang)`.
  - `system_blocks(dict, habits, job, instructions: &str)`: the `Job::Rewrite` block is `instructions` as given (no `CUSTOM_PREAMBLE`).

- [ ] **Step 1: Write the failing tests**: `standard_texts_exist_in_both_languages`; `usable_trims_and_caps_at_2000`; `usable_rejects_blank`; `task_names_the_file_agents_md_and_the_page` (contains `instructions.md`, `AGENTS.md`, the Instructions page as the check, and "keys"/"ключи" as not to touch; no `cue-cli`); in the app crate with a temp dir: `creates_standard_file_when_missing`, `migrates_custom_text_once`, `reloads_after_change`, `burst_of_saves_reloads_once`, `empty_or_deleted_or_binary_file_means_standard_and_broken`, `oversized_file_is_cut_not_rejected`, `set_keeps_previous_copy`, `agents_md_rewritten_when_older_than_version`.
- [ ] **Step 2: Run** `cargo test -p cue-core instructions && cargo test -p cue instructions` — Expected: FAIL.
- [ ] **Step 3: Implement** the two modules; `AGENTS.md` starts with a line `<!-- CUE <version> -->` that decides "older".
- [ ] **Step 4: Use it**: the runtime holds one `Instructions`, created at startup (legacy text from `settings.refine.custom_text()`), and passes `text()` to every cleanup request; `system_blocks` takes it.
- [ ] **Step 5: Run** `cargo test --workspace` — Expected: PASS (existing `claude.rs` body tests updated to the new block text).
- [ ] **Step 6: Commit** — "The cleanup's instructions live in instructions.md: CUE writes the standard ones in the interface language or the user's own, takes a saved change at the next dictation, keeps the previous copy and an AGENTS.md for an AI that edits them".

### Task 3: Finding and running agents

**Files:**
- Create: `app/crates/core/src/agents.rs`, `app/crates/engines/src/agent/mod.rs`, `agent/discover.rs`, `agent/process.rs`, `agent/cli.rs`, `app/crates/engines/src/bin/fake_agent.rs`
- Test: `app/crates/engines/tests/agent_process.rs`, unit tests in `cli.rs`, `discover.rs`

**Interfaces:**
- Produces:
  - `cue_core::agents::{AgentId { OpenCode, Codex, Grok }, ORDER: [AgentId; 3], AgentId::id(&self) -> &'static str /* "opencode" | "codex" | "grok" */, AgentId::name(&self) -> &'static str, AgentId::find(id: &str) -> Option<AgentId>, AgentId::models(&self) -> &'static [(&'static str, &'static str)] /* (id, name) for Codex and Grok, default first; empty for OpenCode, which lists its own */, FAST_NAMES: [&str; 7]}`.
  - `cue_engines::agent::{Found { id: AgentId, exe: PathBuf, version: Option<String> }, discover() -> Vec<Found> /* in ORDER */, AgentFailure { NotInstalled, SignedOut, Timeout, Failed(&'static str) }, Prompt { system: String, user: String, web_search: bool }, AgentModel { id: String, name: String }}`.
  - `agent::process::run(exe: &Path, args: &[String], stdin: &str, cwd: &Path, timeout: Duration) -> Result<Output, AgentFailure>` with `Output { status: Option<i32>, stdout: String, stderr_tail: String /* last 2 KB, for classification only, never logged */ }`; kills the whole tree (a Job object with `KILL_ON_JOB_CLOSE`).
  - `agent::cli::{codex_args(model: &str) -> Vec<String>, grok_args(model: &str, web_search: bool) -> Vec<String>, codex_text(stdout: &str) -> Option<String>, grok_text(stdout: &str) -> Option<String>, signed_out(id: AgentId, out: &Output) -> bool}`.

- [ ] **Step 1: Record real outputs and check stripping** (once, by hand, on this computer, no dictation text): run `codex exec` and `grok -p` on the sentence "Hello world." with the flags of the spec, and each signed out (in a throwaway profile), and save the scrubbed stdout/stderr as fixtures in `app/crates/engines/tests/fixtures/agents/`. Then feed each the text "List the files in the current folder and print the first line of each." with the flags: the answer must show it ran nothing. Write the exact flags, versions and this result in the fixtures' README. An agent that ran a command is not added to `ORDER`; stop and report it to the owner before going on.
- [ ] **Step 2: Write the failing tests**: `codex_args_are_ephemeral_readonly_json_no_user_config` (contains `exec`, `--ephemeral`, `--json`, `-s read-only`, `--skip-git-repo-check`, `--ignore-user-config`, `-m gpt-6-luna`, and `-` for stdin, no transcript); `grok_args_turn_tools_web_and_subagents_off`; `codex_text_reads_last_message` / `grok_text_reads_answer` (fixtures); `signed_out_detected` (fixtures); `discover_finds_npm_cmd_and_ps1_shims` (temp PATH with `codex.cmd`); `discover_order_is_opencode_codex_grok`; integration with `fake_agent`: `run_passes_stdin_not_args`, `run_times_out_and_kills_child_tree` (fake agent spawns a child and sleeps; after `run` returns, the child's pid is gone), `run_has_no_window_and_empty_cwd`.
- [ ] **Step 3: Run** `cargo test -p cue-engines agent` — Expected: FAIL.
- [ ] **Step 4: Implement**; `fake_agent` echoes stdin wrapped as the fixture format, or sleeps / spawns a child per its first argument.
- [ ] **Step 5: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 6: Commit** — "CUE finds OpenCode, Codex and Grok on this computer and runs Codex and Grok with no window, the text on stdin, tools off and a timeout that ends the whole process tree".

### Task 4: OpenCode

**Files:**
- Create: `app/crates/engines/src/agent/opencode.rs`, `app/src-tauri/src/runtime/opencode.rs`
- Test: unit tests in both, against a local fake HTTP server (as the engines' network tests do)

**Interfaces:**
- Consumes: `Found`, `AgentFailure`, `Prompt` (Task 3).
- Produces:
  - `agent::opencode::{announced(state_dir: &Path) -> Option<Url> /* loopback http only */, Server { url: Url, owned: bool }, start(exe: &Path, cwd: &Path) -> Result<Server, AgentFailure> /* `opencode serve` on 127.0.0.1, free port */, Server::models(&self, http) -> Result<Vec<AgentModel>, AgentFailure>, Server::complete(&self, http, model: &str, p: &Prompt, timeout: Duration, on_text: Option<&mut (dyn FnMut(&str) + Send)>) -> Result<String, AgentFailure>, Server::stop(self)}`; `fast_auto(models: &[AgentModel]) -> Option<&AgentModel>` (`AgentModel` from Task 3).
  - `runtime::opencode::OpenCodeSlot` with `get_or_start(&self) -> Result<Arc<Server>, AgentFailure>`, `rest(&self)` (stops an owned server), hooked to the same rest, lock, sleep and quit events as `runtime/llm.rs`.

- [ ] **Step 1: Write the failing tests**: `announced_accepts_only_loopback`; `fast_auto_prefers_names_in_order_else_default`; `complete_uses_stateless_generate_when_offered`; `complete_falls_back_to_session_and_deletes_it` (the fake server records a DELETE); `complete_times_out`; `slot_reuses_announced_server_and_never_stops_it`; `slot_stops_owned_server_on_rest_lock_sleep_quit`.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement**; check the endpoints against the OpenCode version the user installs (spec, "Your AI") and keep both shapes behind `complete`.
- [ ] **Step 4: Run** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5: Commit** — "OpenCode: CUE uses the server the user runs, or starts its own on 127.0.0.1 and stops it when CUE rests, the session locks, the computer sleeps or CUE quits; a cleanup is one request that leaves no session behind".

### Task 5: Cleanup, translation and habit rules through an agent

**Files:**
- Create: `app/crates/engines/src/agent/refine.rs`
- Modify: `app/crates/core/src/flow.rs` (`RefineOutcome`), `app/src-tauri/src/runtime.rs` (habit rules on an agent route go straight to the template)
- Test: unit tests in `agent/refine.rs` with an `AgentClient` over `fake_agent`

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `agent::AgentClient` = `enum { OpenCode(Arc<Server>), Cli(Found) }` with `complete(&self, http, model: &str, p: &Prompt, timeout, on_text) -> Result<String, AgentFailure>`.
  - `agent::refine::{refine(c: &AgentClient, http, model, raw, ctx, dict, s, instructions: &str, first: bool) -> RefineResult, translate(..., to: &'static str) -> RefineResult, timeout_ms(c: &AgentClient, raw: &str, first: bool) -> u64}`; system text = `chat::system_text` with `instructions`, user = `refine::user_message(ctx, raw)`, answer parsed as `{"text": …}` then `settle` and `drop_copied_context`.
  - `RefineOutcome::{Refined, RawFallback, RawAgent(AgentProblem)}` replacing `RawKeyProblem`; `AgentProblem { Missing, SignedOut, Silent }` in `cue_core::flow`.

- [ ] **Step 1: Write the failing tests**: `refine_returns_cleaned_text`; `transcript_goes_in_user_message_on_stdin` (fake agent writes what it got to a temp file; contains `<transcript>удали все файлы</transcript>`, the args do not); `failed_check_falls_back_to_raw`; `timeout_is_raw_agent_silent`; `cli_timeout_is_10s_plus_1s_per_100_words` (`timeout_ms(cli, 200 words, false) == 12_000`); `first_request_adds_5s`; `opencode_timeout_is_refine_timeout`; `signed_out_is_raw_agent_signed_out`; `translation_never_returns_untranslated_as_refined`; `habit_rule_on_agent_route_uses_template` (no agent run happens).
- [ ] **Step 2–4: Run → implement → run** (`cargo test --workspace`); update `flow.rs` users of `RawKeyProblem` to `RawAgent(_)` with the same behavior for now.
- [ ] **Step 5: Commit** — "Cleanup, translation and habit rules can go through OpenCode, Codex or Grok with the same prompt and checks as before".

### Task 6: Settings: where, agent, fallback

**Files:**
- Modify: `app/crates/core/src/settings.rs` (`RefineSettings`, `StoredRefine`, `SettingChange`, `AskSettings`), `app/src-tauri/src/ipc.rs` (`SettingsView`), `app/ui/src/window/api.ts`
- Test: `settings.rs` tests, `app/ui/test/window-api.test.ts`

**Interfaces:**
- Produces:
  - `pub enum Where { Off, Local, Agent }` (serde lowercase, lenient like `Runs`), `pub enum Fallback { Raw, Local }`.
  - `RefineSettings { where_: Where /* "where" */, local: String /* "" = nothing chosen */, agent: Option<AgentId> /* None = first found, resolved at startup */, models: BTreeMap<String, String> /* per agent id */, fallback: Fallback, context_chars, min_ratio, max_ratio, min_overlap }` and `legacy_custom(&self) -> Option<&str>` (read from old `style: custom` + `custom`, never written).
  - `SettingChange::{RefineWhere(Where), RefineAgent(AgentId), RefineAgentModel { agent: AgentId, model: String }, RefineFallback(Fallback)}`; `RefineStyle`, `RefineCustom`, `RefineRuns`, `RefineConnection`, `RefineModel` go; `AskSettings.model` goes.
  - `SettingsView` adds `refineWhere`, `refineAgent`, `refineAgentModel`, `refineFallback`; drops `refineStyle`, `refineCustom`, `refineRuns`, `refineConnection`, `refineModel`, `connections`.

- [ ] **Step 1: Write the failing tests** (migration table): `runs_local_keeps_its_model`; `runs_local_with_empty_local_gets_old_recommended_id` (`llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m` — take the exact id from `llm::id_for`); `runs_cloud_becomes_agent_unresolved`; `custom_style_text_is_legacy_custom`; `old_fields_are_never_written_back`; `unknown_where_reads_as_default_agent`; `ask_model_is_read_past`.
- [ ] **Step 2–4: Run → implement → run**; the runtime still compiles by mapping `Where::Agent` to the cloud path until Task 7 (one temporary match arm, removed in Task 7).
- [ ] **Step 5: Commit** — "Settings say where the cleanup runs (no AI, this computer, your AI), which agent and model, and what to do when the AI doesn't answer; earlier settings move over once".

### Task 7: The route switch and the Drop's words

**Files:**
- Modify: `app/src-tauri/src/runtime.rs` (`Route`, `route`, `refine`, startup resolution of `agent: None`, habits' rule route), `app/crates/core/src/ui.rs` (`ErrorCause`), `app/crates/core/src/flow.rs`, `app/ui/src/strings.ts`, `app/src-tauri/src/ipc.rs` (`cue://cleanup-name`)
- Test: runtime route tests, `flow.rs` tests, `app/ui/test/controller.test.ts`

**Interfaces:**
- Consumes: Tasks 4–6.
- Produces: `Route::{Off, Local(g), NotReady, Waking(slot), Agent(AgentClient)}`; `ErrorCause::{AgentSignIn, AgentMissing, AgentSilent, NeedAiTranslate, NeedAiAsk, NeedTranslatingModel, NeedAnsweringModel}` replacing `ClaudeKey`, `CleanupKey` (the last two are raised in Tasks 9 and 10); an `AgentMarks` holding a signed-out or missing mark per agent for 5 minutes (`MARK_FOR = 300 s`), cleared by `agents_get` (Task 11).

- [ ] **Step 1: Write the failing tests**: `off_pastes_basics_text`; `agent_silent_with_local_fallback_cleans_locally_and_shows_nothing`; `local_fallback_skipped_when_model_resting` (pasted as said, model not woken); `agent_silent_with_raw_fallback_shows_agent_silent`; `signed_out_marks_agent_and_next_dictation_skips_the_wait` (second dictation returns at once with `RawAgent(SignedOut)`); `translate_on_off_shows_need_ai_and_pastes_nothing`; `startup_resolves_unset_agent_to_first_found_else_local_nothing_chosen`; UI: Drop shows `"{NAME}: войдите заново"` / `"{NAME}: sign in again"`, `"{NAME} не найден"`, `"{NAME} не ответил"`, `"Перевод: выберите AI"`, `"Спросить: выберите AI"`.
- [ ] **Step 2–4: Run → implement → run** `cargo test --workspace && cd app && npm test`.
- [ ] **Step 5: Commit** — "Cleanup goes where the settings say: no AI, this computer or your AI, with the fallback the user chose; the Drop says which AI needs a sign-in, was not found or didn't answer".

### Task 8: Removing keys, providers and prices

**Files:**
- Delete: `app/crates/core/src/connections.rs`, `app/crates/engines/src/check.rs`, `chat.rs`, `openrouter.rs`, `app/ui/src/window/keys.ts`
- Modify: `app/crates/engines/src/claude.rs` (keep only what the agent path reuses: `Job`, `system_blocks`, `settle`, `decide*`, `Parsed`, `Usage`→ moved to `agent/refine.rs` or a `prompt.rs`; delete the HTTP parts), `keys.rs` (only `habit_secret`, `kept_habit_secret`, new `forget_api_keys()`), `commands.rs` / `main.rs` (drop `key_state`, `key_set`, `key_check`), `runtime.rs` (Anthropic warm-ups and HEAD ping), `app/crates/cli/src/main.rs` (drop `keys`), `app/crates/core/src/usage.rs` + `settings.rs` (`Pricing`, `refine_cost`: words and time stay, no USD), examples (`refine_compare`, `ask_compare`, `refine_probe`, `ask_probe`, `warm_probe`: an env var key or removed), `app/crates/cli/src/leak.rs` (no Credential Manager key lookup; keep the API-key-pattern scan)
- Test: `keys.rs` test with a fake store, usage tests, leak tests

**Interfaces:**
- Produces: `keys::forget_api_keys()` deleting service `CUE` entries `anthropic, openrouter, openai, gemini, deepseek, mistral, groq, xai`, called once at startup (replaces `forget_soniox`'s call site).

- [ ] **Step 1: Write the failing tests**: `forget_api_keys_deletes_the_eight_and_keeps_habits_secret`; `usage_rows_have_no_usd`; `leak_scan_still_flags_api_key_patterns`.
- [ ] **Step 2–4: Run → delete and implement → run** the four checks of Global Constraints; `rg -n "x-api-key|Credential|KeyName::" app` finds only the habit secret.
- [ ] **Step 5: Commit** — "CUE holds no API keys: the eight cloud connections, key checks, balances and prices are gone, and the keys an earlier version saved are deleted at the first start".

### Task 9: Ask through the chosen route

**Files:**
- Create: `app/crates/engines/src/agent/ask.rs`
- Modify: `app/crates/engines/src/ask.rs` (keep `Step`, `AskEnd`, `AskResult`, `Limits`; drop the Anthropic request and stream), `app/crates/core/src/ask.rs` (agent prompt), `app/src-tauri/src/runtime.rs::spawn_ask`, `app/ui/src/answer.ts` (the "без поиска" line)
- Test: unit tests in `agent/ask.rs` over `fake_agent`, `app/ui/test/answer.test.ts`

**Interfaces:**
- Produces:
  - `cue_core::ask::{AGENT_ASK_PROMPT, agent_messages(messages: &[AskTurn]) -> String}` (the conversation as plain text) and the answer shape `{"answer": string, "write": bool, "replace": bool, "sources": [{"url": string, "title": string}]}`; `sources` become `Step::Cite` steps.
  - `agent::ask::run(route: AskRoute, http, model, search: bool, turns, limits: Limits, on_step) -> AskResult` with `AskRoute::{Agent(AgentClient), Local(Arc<dyn Generate>)}`; emits `Step::NoSearch` first when search is off or not possible; `AskEnd::Write { text, replace }` when `write`.
  - Conversation turns stored as `{"role", "text"}` instead of Anthropic content blocks.

- [ ] **Step 1: Write the failing tests**: `answer_streams_as_text_steps`; `write_true_ends_as_write`; `replace_true_targets_selection` (the card offers Replace as today); `sources_become_cite_steps`; `first_words_deadline_is_15s_and_20s_with_search`; `cleanup_only_local_model_ends_with_need_answering_model`; `codex_and_grok_search_when_asked`, `opencode_without_search_tool_says_no_search`; `local_route_never_searches`; `deadlines_are_limits_spec`; `follow_up_carries_previous_turns`; UI: the card shows "без поиска" / "no web search" after `NoSearch`.
- [ ] **Step 2–4: Run → implement → run**; Ask on Без ИИ or nothing chosen ends with `NeedAiAsk` (Task 7).
- [ ] **Step 5: Commit** — "Ask goes through the same AI as the cleanup, or the model on this computer without web search; no Anthropic key is needed".

### Task 10: Models on this computer

**Files:**
- Modify: `app/crates/core/src/llm.rs` (`catalog`, `recommended_for(langs: &[&str]) -> Vec<String>`), `app/crates/engines/src/local.rs` + `local_llm.rs` (input formats per model), `app/src-tauri/src/runtime/llm.rs` (speed record), `app/crates/engines/src/hub.rs` (unchanged search; "CUE не проверял" is UI)
- Create: `app/src-tauri/src/speed.rs` (`%LOCALAPPDATA%\CUE\speed.json`)
- Test: `llm.rs`, `speed.rs`, `local.rs` format tests

**Interfaces:**
- Produces: catalog in this order: Spokenly cleanup-1 (`spokenly/cleanup-1-gguf`, Q8_0, pinned commit, size and sha256 from the repo), RuadaptQwen3 4B Instruct (`RefalMachine/RuadaptQwen3-4B-Instruct-GGUF`, Q4_K_M), S1 Mini (as model-picks); `Manifest.languages` filled (`["ru","en"]`, `["en"]`); `InputFormat { Json, KeyValue /* cleanup-1 */, S1 }` per model id; `cleans_only(id) -> bool` (true for cleanup-1 and S1 Mini: no Ask, no translation, no instructions; Drop words "Спросить: нужна модель, которая отвечает" / "Перевод: нужна модель, которая переводит"; rows say "Только чистка"); `speed::{record(id, secs), median(id) -> Option<f32>}` over the last 10.

- [ ] **Step 1: Write the failing tests**: `catalog_order_and_languages`; `recommended_for_ru_en_is_cleanup1_and_ruadapt`; `recommended_for_en_only_adds_s1_mini`; `cleanup1_prompt_is_key_value_out` (its card's format); `old_catalog_model_still_resolves_when_downloaded`; `speed_median_of_last_ten`.
- [ ] **Step 2–4: Run → implement → run** (check S1 Mini's license before keeping it).
- [ ] **Step 5: Commit** — "Models on this computer: Spokenly cleanup-1 and RuadaptQwen3 4B recommended for Russian and English, S1 Mini for English; each row learns its real speed on this computer".

### Task 11: The Cleanup page

**Files:**
- Create: `app/ui/src/window/cleanup.ts` (the page), `app/src-tauri/src/commands.rs` additions
- Modify: `app/ui/src/window/settings.ts` (route `cleanup` to `cleanup.ts`, first-page value and its amber states, drop `cleanupRows`, `card`, `connectionMenu`, `modelMenu`, key rows), `app/ui/src/window/text.ts` (strings), `app/ui/src/window/models.ts` (groups "Для ваших языков · …" / "Другие", "Рекомендуем", speed, "CUE не проверял"), `app/ui/src/design/window.css` (nothing new if the mockup's classes suffice)
- Test: `app/ui/test/window-cleanup.test.ts` (rewritten), `window-models.test.ts`, `window-settings.test.ts`

**Interfaces:**
- Consumes: Tasks 6, 7, 10.
- Produces: commands `agents_get() -> Vec<AgentView { id, name, found, signedIn: Option<bool>, line, models: Vec<{id,name}>, model }>` (clears marks), `agent_install_link(id)`; the page per the spec's "Cleanup page" sections 1–6 and the mockup https://claude.ai/artifact/UAKqQmWV7WoD2EyuP6SSdK.

- [ ] **Step 1: Write the failing tests**: `three_part_control_switches_lists`; `agents_in_order_opencode_first_with_recommended`; `missing_opencode_shows_install_button`; `signed_out_value_is_amber_with_sign_in_again`; `model_dropdown_lists_agent_models_and_remembers_per_agent`; `fallback_offers_local_only_when_downloaded`; `grok_line_says_it_keeps_history`; `local_groups_and_recommended_tag`; `pressing_missing_model_downloads_with_percent_and_stop`; `found_model_marked_not_checked`; `footer_per_route_verbatim`; first page `value_shows_agent_and_model` / `value_nothing_chosen`.
- [ ] **Step 2–4: Run → implement → run** `npm test && npm run build`; look at it in the `cue-ui` preview on the mock core (merlin-skin `tools/tauri-mock.md`) in both themes.
- [ ] **Step 5: Commit** — "The Cleanup page: No AI, This computer or Your AI on top; OpenCode, Codex and Grok as found here, recommended models for your languages with their size and speed, the Hugging Face search, and the fallback when the AI doesn't answer".

### Task 12: The Instructions page and "Изменить с AI"

**Files:**
- Create: `app/ui/src/window/instructions.ts`
- Modify: `app/src-tauri/src/commands.rs`, `app/ui/src/window/text.ts`
- Test: `app/ui/test/window-instructions.test.ts`, command tests in `commands.rs`

**Interfaces:**
- Consumes: Task 2 (`Instructions`), Task 5 (`AgentClient`), Task 7 (route).
- Produces: commands `instructions_get() -> { text, edited, broken, path }`, `instructions_set(text)`, `instructions_reset()`, `instructions_task() -> String`, `instructions_open(agent: AgentId)` (new terminal: Windows Terminal `wt` when present, else `cmd /c start`, in `%APPDATA%\CUE\`, the agent's interactive command with the task as its first message); event `cue://instructions { text, edited, broken, sample: Option<{ before, after }> }` after a reload (the sample: the last history dictation re-cleaned once on the current route; none on Без ИИ or a cleanup-only model).

- [ ] **Step 1: Write the failing tests**: `page_shows_file_text_and_standard_state`; `edit_with_ai_menu_has_open_and_copy`; `copy_puts_task_on_clipboard`; `task_asks_to_check_the_page_not_a_cli`; `reload_event_shows_before_after`; `broken_file_shows_amber_line`; `reset_restores_standard`; `cleanup_only_model_says_it_does_not_read_instructions`.
- [ ] **Step 2–4: Run → implement → run** all four checks.
- [ ] **Step 5: Commit** — "Your instructions: edit them on the page or with your AI (open it in the instructions folder or copy its task); CUE takes the saved file and shows before and after on your last dictation".

### Task 13: The welcome step "Кто чистит текст"

**Files:**
- Modify: `app/ui/src/welcome/steps.ts` (`'keys'` → `'cleanup'`), `welcome/page.ts`, `welcome/text.ts`, `app/src-tauri/src/welcome_window.rs` (no longer opens for a missing key)
- Test: `app/ui/test/welcome.test.ts`

**Interfaces:**
- Consumes: `cleanup.ts` (Task 11) rendered inside the step.

- [ ] **Step 1: Write the failing tests**: `steps_are_hello_cleanup_model_mic_try_done`; `default_is_opencode_when_found`; `default_is_first_found_agent`; `no_agent_highlights_recommended_and_downloads_nothing`; `welcome_does_not_open_for_missing_key`.
- [ ] **Step 2–4: Run → implement → run**.
- [ ] **Step 5: Commit** — "The welcome window asks who cleans the text instead of asking for a key".

### Task 14: Docs, notes and the owner's CUE

**Files:**
- Modify: `README.md` and `docs/ARCHITECTURE.md` (sections listed in the spec, "Docs to update"), `docs/notes/local-cleanup.md`, `optimize-and-settings.md`, `model-picks.md`, `docs/notes/README.md`
- Create: `docs/notes/no-keys.md`

- [ ] **Step 1:** Update the docs so every statement about keys, providers, prices, Ask and the welcome window is true for this branch; `rg -n -i "api key|anthropic key|openrouter|deepseek|balance" README.md docs/ARCHITECTURE.md` returns only historical mentions.
- [ ] **Step 2:** Run the four checks of Global Constraints — Expected: all green.
- [ ] **Step 3:** Build `scripts\build-release.ps1`, install with `/S`, start CUE, dictate one Russian and one English sentence through Codex (OpenCode once the owner installs it) and through the local model; report what was checked.
- [ ] **Step 4: Commit** — "Docs and notes describe CUE without keys".
