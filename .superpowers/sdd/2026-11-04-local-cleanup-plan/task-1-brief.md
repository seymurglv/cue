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

