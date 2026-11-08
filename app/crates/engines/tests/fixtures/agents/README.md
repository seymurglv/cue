# Agent outputs and the stripping check

Recorded on 08.11.2026 on the owner's computer, for `agent::cli` (docs/specs/2026-11-08-no-keys-design.md, "Your AI").
Every run used harmless text ("Hello world.") in an empty folder as the working directory. Scrubbed: thread ids,
request ids and Cloudflare ray ids are zeroed; no account id, email, token or user path is in any file.

## Versions

| Agent | Version | Found at |
| --- | --- | --- |
| Codex | codex-cli 0.166.0 | `%APPDATA%\npm\codex.cmd` (npm shim: `cmd.exe` → `node` → `codex.exe`) |
| Grok | grok 1.0.34 (3736acbc8658) [stable] ("Grok Build") | `%USERPROFILE%\.grok\bin\grok.exe` |
| OpenCode | not installed | — |

Both installed agents were signed out on this computer: Codex's sign-in had expired ("your refresh token has
expired"), and Grok had never been signed in. CUE does not sign in for the user, so **no run produced an answer**,
and the behavioral probe below could not run. The answer fixtures are built from each CLI's documented output format
(marked "constructed").

## Files

| File | What |
| --- | --- |
| `codex-expired.stdout`, `.stderr` | real: `codex exec` with CUE's flags, the owner's expired sign-in (exit 1, about 3–11 s) |
| `codex-signed-out.stdout`, `.stderr` | real: the same in a throwaway profile (`CODEX_HOME` = an empty temp folder), never signed in (exit 1 after 15 s of retries) |
| `grok-signed-out.stdout`, `.stderr` | real: `grok -p` with CUE's flags, signed out (the owner's profile and a throwaway `GROK_HOME` print the same; exit 1, under 2 s) |
| `codex-answer.jsonl` | constructed: `codex exec --json` events (`thread.started`, `turn.started`, `item.completed` with an `agent_message`, `turn.completed`), the event and field names checked in codex.exe 0.166.0 |
| `codex-ran-command.jsonl` | constructed: the same with a `command_execution` item, which `codex_text` must refuse |
| `grok-answer.json` | constructed: `--output-format json`'s object as Grok's own headless guide (`~/.grok/docs/user-guide/14-headless-mode.md`) documents it: `{"text", "stopReason", "sessionId", "requestId"}` |

## Codex: CUE's flags

```
codex exec --ephemeral --json -s read-only --skip-git-repo-check --ignore-user-config --ignore-rules -m <model>
  --disable shell_tool --disable unified_exec --disable view_image --disable apps --disable plugins
  --disable browser_use --disable computer_use --disable image_generation -c web_search=disabled -
```

The prompt is read from stdin (`-`). Every flag was checked against `codex exec --help` of 0.166.0; the feature names
against `codex features list` (an unknown name is an error: "Unknown feature flag"); `web_search` takes `disabled`,
`cached`, `indexed` or `live` (a wrong value is an error naming them), and a bare word reaches it as a string. With all
of these, a run with the owner's profile got as far as the sign-in (the flags were accepted).

**Tools the model sees** (`codex debug prompt-input` in the empty folder, a throwaway `CODEX_HOME` so no user config):

| | Default | With CUE's `--disable` flags |
| --- | --- | --- |
| shell | `exec_command`, `write_stdin` (inside the `exec` JS tool) | **gone** |
| images | `view_image` | **gone** |
| files | `apply_patch` | `apply_patch` (writes only; the read-only sandbox refuses them, and the exec run never asks for approval) |
| MCP | `list_mcp_resources`, `list_mcp_resource_templates`, `read_mcp_resource` | the same, with no MCP server: `--ignore-user-config` drops the user's `config.toml`, and apps/plugins are off |
| other | `wait`, `request_user_input`, `clock`, `collaboration.*` (subagents with the same tools) | the same |
| web | native `web_search` (added by the request, not in this list) | off: `-c web_search=disabled` |

The `exec` tool itself is a JavaScript isolate that only composes the tools above ("no Node, no file system, no
network access", its own description).

**Verdict: Codex is offered.** With CUE's flags on 0.166.0 it has no shell, no file-reading tool and no network tool.
`codex_text` refuses any answer whose events show a tool item (`command_execution`, `file_change`, `mcp_tool_call`,
`web_search`, …), as a tripwire. **Not yet run:** the behavioral probe ("List the files in the current folder and
print the first line of each.") needs a signed-in Codex; run it once the owner signs in again
(`codex login`) and record the answer here.

## Grok: CUE's flags

```
grok --output-format json -m <model> --tools todo_write --disallowed-tools search_tool,use_tool,Agent
  --no-subagents --no-plan --no-auto-update --disable-web-search
```

(For Ask with search: `--tools todo_write,web_search` and no `--disable-web-search`.) Each flag was accepted by 1.0.34
(an unknown flag is an error); `--no-auto-update` is not in `--help` but in Grok's headless guide and accepted.
Per Grok's guide, `--tools` is an allowlist ("only the listed tools will be available; all others are removed"), MCP
meta-tools (`search_tool`, `use_tool`) stay available under it unless denied, and `Agent` in `--disallowed-tools`
blocks all subagents. Not verified by a run: Grok is signed out here.

**The prompt cannot go on stdin.** Grok's headless guide says so ("Headless mode does not read piped stdin into the
prompt"), and the flags confirm it: the prompt comes only from `-p <text>` (an argument, visible in the process list),
`--prompt-json <json>` (an argument) or `--prompt-file <path>` (a file; `--prompt-file -` fails: "Failed to read
'-'"). One way that keeps the text off the command line and the disk was checked: `--prompt-file \\.\pipe\<name>`
reads from a named pipe CUE would create and write (Grok read it and went on to the sign-in). `grok agent stdio`
(ACP, JSON-RPC on stdin) is the other; its tool filtering (`--tools` / `--disallowed-tools`) is headless-only, so it
would need an agent profile file.

**Verdict: Grok is not offered on 1.0.34** (left out of `cue_core::agents::ORDER`; `AgentId::Grok` stays so settings
parse). Not because a tool ran, but because the spec's rule "the transcript on stdin, never an argument or a temp file"
cannot be kept with `grok -p`. To offer it: the owner picks the named pipe or ACP, and the probe runs signed in.

## Signed out, as each CLI says it

- Codex, expired: `{"type":"error","message":"Your access token could not be refreshed because your refresh token has
  expired. Please log out and sign in again."}` then `turn.failed`; stderr repeats "Failed to refresh token". Exit 1.
- Codex, never signed in: five WebSocket and five HTTPS retries of `401 Unauthorized: Missing bearer or basic
  authentication in header`, then `turn.failed`. Exit 1 after **about 15 s**, past CUE's 10 s agent budget, so CUE sees
  a timeout, not a sign-out. (`codex login status` says "Not logged in", exit 1, in under a second; it says "Logged in
  using ChatGPT", exit 0, for the expired sign-in.)
- Grok: `{"type":"error","message":"Not signed in. To authenticate without a browser, run: grok login --device-code …"}`
  on stdout and `Error: Not signed in. …` on stderr. Exit 1, under 2 s, no browser opened.

## Models

- Codex (`codex debug models`, visibility "list"): gpt-6-luna (GPT-6-Luna), gpt-6-sol, gpt-6-astra, gpt-6.1-sol,
  gpt-6.1-terra, gpt-5.6-sol, gpt-5.6-terra, gpt-5.6-luna, gpt-5.5.
- Grok (`grok models`, signed out): grok-4.6 (default), grok-4.5. No model marked fast; check again signed in.
