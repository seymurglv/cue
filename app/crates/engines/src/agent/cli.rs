//! Codex's command lines and answers, as checked on codex-cli 0.166.0 (the evidence is in
//! `tests/fixtures/agents/README.md`). The text never appears here: it goes on stdin (`process::run`).
use std::path::Path;

use cue_core::agents::AgentId;
use serde_json::Value;

use super::process::Output;

/// Codex features that hand the model a tool or more turns: the shell (`exec_command`, `write_stdin`), images, apps and
/// plugins (connectors), the browser and computer use; subagents, goals, the sleep tool, hooks, skill search, tool
/// suggestions, the code-mode host and workspace dependencies. With them off the model sees no tool that runs, reads or
/// fetches, and a short cleanup costs about 6,500 input tokens instead of 10,600 (measured 09.11.2026).
const CODEX_TOOLS_OFF: [&str; 16] = [
    "shell_tool",
    "unified_exec",
    "view_image",
    "apps",
    "plugins",
    "browser_use",
    "computer_use",
    "image_generation",
    "multi_agent",
    "goals",
    "sleep_tool",
    "hooks",
    "skill_search",
    "tool_suggest",
    "code_mode_host",
    "workspace_dependencies",
];

/// The feature an "Ask" question with web search keeps on: Codex's live search runs through it.
const CODEX_SEARCH_HOST: &str = "code_mode_host";

/// `codex exec` reading the prompt from stdin (`-`): no session files (`--ephemeral`), events as JSON lines, the
/// read-only sandbox, none of the user's `config.toml` (so none of their MCP servers) or `.rules`, the shell and every
/// other tool off, low reasoning. Web search is off, and live only for an "Ask" question with `web_search` on, with the
/// code-mode host it runs through left on: the only tool Codex then has. `instructions` replaces Codex's own long base instructions with CUE's fixed text (never
/// anything of a dictation or a question: that goes on stdin), and `schema` holds the shape of the answer (`{"text":
/// …}`, or Ask's), the last message in the events; no file is written for the answer.
pub fn codex_args(model: &str, instructions: &Path, schema: &Path, web_search: bool) -> Vec<String> {
    let mut args: Vec<String> = [
        "exec",
        "--ephemeral",
        "--json",
        "-s",
        "read-only",
        "--skip-git-repo-check",
        "--ignore-user-config",
        "--ignore-rules",
        "-m",
        safe_model(AgentId::Codex, model),
    ]
    .map(String::from)
    .into();
    for feature in CODEX_TOOLS_OFF {
        // Codex 0.166.0 reaches the web only through its code-mode host: with it off, live search finds nothing
        // (checked 09.11.2026). It stays off for everything but an "Ask" question with web search on.
        if web_search && feature == CODEX_SEARCH_HOST {
            continue;
        }
        args.extend(["--disable".to_string(), feature.to_string()]);
    }
    // Bare words: Codex takes a value that is not TOML as the literal string (no quotes to survive cmd.exe), and a
    // Windows path with forward slashes is not TOML.
    for setting in [
        (if web_search { "web_search=live" } else { "web_search=disabled" }).to_string(),
        "model_reasoning_effort=low".to_string(),
        format!("model_instructions_file={}", slashed(instructions)),
    ] {
        args.extend(["-c".to_string(), setting]);
    }
    args.extend(["--output-schema".to_string(), slashed(schema)]);
    args.push("-".to_string());
    args
}

/// `path` with forward slashes, which Windows takes as well and neither TOML nor `cmd.exe` reads anything into.
fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `model` when it is a plain model id (letters, digits, `.-_:/`, at most 64), else the agent's default: a model id
/// comes from settings and goes on a command line that `cmd.exe` may parse.
fn safe_model(agent: AgentId, model: &str) -> &str {
    if cue_core::agents::plain_model(model) { model } else { agent.models().first().map(|m| m.0).unwrap_or_default() }
}

/// The JSON values of the lines that are JSON.
fn events(stdout: &str) -> impl Iterator<Item = Value> + '_ {
    stdout.lines().filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
}

/// The items `codex exec --json` reports that are not a tool: the answer, the model's reasoning summary, its to-do
/// list and Codex's own errors (a reconnect).
const CODEX_QUIET_ITEMS: [&str; 4] = ["agent_message", "reasoning", "todo_list", "error"];

/// Codex's last message, when the turn did not fail. `None` as well when any tool ran (a command, a file change, an
/// MCP call, a web search): with CUE's flags none can, so such an answer is not trusted.
pub fn codex_text(stdout: &str) -> Option<String> {
    codex_answer(stdout, false).map(|(text, _)| text)
}

/// [`codex_text`] for an "Ask" question, with the web searches it ran. With `web_search` (Codex was given live web
/// search) a `web_search` item is allowed; any other tool item still refuses the answer, as does a search Codex was
/// never given.
pub fn codex_answer(stdout: &str, web_search: bool) -> Option<(String, u32)> {
    let (mut text, mut searches) = (None, 0u32);
    for e in events(stdout) {
        match e["type"].as_str() {
            Some("turn.failed") => return None,
            Some("item.started" | "item.updated" | "item.completed") => {
                let kind = e["item"]["type"].as_str().unwrap_or_default();
                if kind == "web_search" && web_search {
                    if e["type"] == "item.completed" {
                        searches = searches.saturating_add(1);
                    }
                    continue;
                }
                if !CODEX_QUIET_ITEMS.contains(&kind) {
                    return None;
                }
                if kind == "agent_message" && e["type"] == "item.completed" {
                    text = e["item"]["text"].as_str().map(String::from);
                }
            }
            _ => {}
        }
    }
    text.map(|t| (t, searches))
}

/// `codex login status`: no text, no model, nothing of the user's.
pub fn codex_login_args() -> Vec<String> {
    vec!["login".to_string(), "status".to_string()]
}

/// What `codex login status` printed: a failure that says "Not logged in" (any case) is signed out, exit 0 ("Logged in
/// using ChatGPT") signed in; any other failure (another message, an older Codex without the command) is unknown, and
/// a run tells.
pub fn codex_login(out: &Output) -> super::Login {
    let said = format!("{}\n{}", out.stdout, out.stderr_tail).to_lowercase();
    if out.status != Some(0) && said.contains("not logged in") {
        super::Login::SignedOut
    } else if out.status == Some(0) {
        super::Login::SignedIn
    } else {
        super::Login::Unknown
    }
}

/// Whether a failed run says the agent is signed out, its sign-in expired, or it has no access. Only the CLI's own
/// error messages and stderr are read, never the answer, so a dictation about signing in is not taken for one.
pub fn signed_out(id: AgentId, out: &Output) -> bool {
    let markers: &[&str] = match id {
        AgentId::Codex => &[
            "401 unauthorized",
            "refresh token",
            "sign in again",
            "log in again",
            "not logged in",
            "missing bearer",
            "invalid_jwt",
        ],
        // OpenCode answers over HTTP; Grok is not offered.
        AgentId::OpenCode | AgentId::Grok => return false,
    };
    failed_saying(out, markers)
}

/// Whether a failed run says the account is out of its allowance for now. codex-cli 0.166.0's own strings: "You've
/// reached your usage limit.", "Usage limit reached.", "Your workspace is out of credits." and "rate limit exceeded".
/// Read as [`signed_out`] reads: the CLI's error events and stderr only, never the answer.
pub fn rate_limited(id: AgentId, out: &Output) -> bool {
    let markers: &[&str] = match id {
        AgentId::Codex => &["usage limit", "rate limit", "out of credits", "too many requests"],
        AgentId::OpenCode | AgentId::Grok => return false,
    };
    failed_saying(out, markers)
}

/// A run that failed and whose error events (`error`, `turn.failed`) or stderr hold one of `markers`, in any case.
fn failed_saying(out: &Output, markers: &[&str]) -> bool {
    if out.status == Some(0) {
        return false;
    }
    let mut said: Vec<String> = events(&out.stdout)
        .filter(|e| matches!(e["type"].as_str(), Some("error" | "turn.failed")))
        .map(|e| format!("{} {}", e["message"], e["error"]["message"]))
        .collect();
    said.push(out.stderr_tail.clone());
    said.iter().map(|s| s.to_lowercase()).any(|s| markers.iter().any(|m| s.contains(m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX_ANSWER: &str = include_str!("../../tests/fixtures/agents/codex-answer.jsonl");
    const CODEX_RAN_COMMAND: &str = include_str!("../../tests/fixtures/agents/codex-ran-command.jsonl");
    const CODEX_EXPIRED_OUT: &str = include_str!("../../tests/fixtures/agents/codex-expired.stdout");
    const CODEX_EXPIRED_ERR: &str = include_str!("../../tests/fixtures/agents/codex-expired.stderr");
    const CODEX_SIGNED_OUT_OUT: &str = include_str!("../../tests/fixtures/agents/codex-signed-out.stdout");
    const CODEX_SIGNED_OUT_ERR: &str = include_str!("../../tests/fixtures/agents/codex-signed-out.stderr");

    /// As `process::run` keeps it: the stderr's last 2 KB.
    fn output(status: i32, stdout: &str, stderr: &str) -> Output {
        let mut cut = stderr.len().saturating_sub(2048);
        while !stderr.is_char_boundary(cut) {
            cut += 1;
        }
        Output { status: Some(status), stdout: stdout.to_string(), stderr_tail: stderr[cut..].to_string() }
    }

    fn has_pair(args: &[String], a: &str, b: &str) -> bool {
        args.windows(2).any(|w| w[0] == a && w[1] == b)
    }

    /// No argument carries text: each is a flag or a short fixed value, never a sentence.
    fn no_text_in(args: &[String]) {
        for a in args {
            assert!(!a.contains(char::is_whitespace), "{a:?} looks like text");
        }
    }

    const INSTRUCTIONS: &str = r"C:\Users\Ann\AppData\Local\CUE\agent\codex-instructions.md";
    const SCHEMA: &str = r"C:\Users\Ann\AppData\Local\CUE\agent\codex-schema.json";

    fn codex(model: &str) -> Vec<String> {
        codex_args(model, Path::new(INSTRUCTIONS), Path::new(SCHEMA), false)
    }

    #[test]
    fn codex_args_are_ephemeral_readonly_json_no_user_config() {
        let args = codex("gpt-6-luna");
        assert_eq!(args[0], "exec");
        for flag in ["--ephemeral", "--json", "--skip-git-repo-check", "--ignore-user-config", "--ignore-rules"] {
            assert!(args.iter().any(|a| a == flag), "{flag} missing from {args:?}");
        }
        assert!(has_pair(&args, "-s", "read-only"));
        assert!(has_pair(&args, "-m", "gpt-6-luna"));
        // The tools off: no shell, no web search, no images or apps; nothing that hands the model more tools or turns
        // (subagents, goals, hooks, skills, tool suggestions, the code-mode host, workspace dependencies).
        for feature in [
            "shell_tool",
            "unified_exec",
            "view_image",
            "apps",
            "plugins",
            "browser_use",
            "computer_use",
            "image_generation",
            "multi_agent",
            "goals",
            "sleep_tool",
            "hooks",
            "skill_search",
            "tool_suggest",
            "code_mode_host",
            "workspace_dependencies",
        ] {
            assert!(has_pair(&args, "--disable", feature), "{feature} not disabled");
        }
        assert!(has_pair(&args, "-c", "web_search=disabled"));
        assert!(has_pair(&args, "-c", "model_reasoning_effort=low"));
        // The prompt is read from stdin; no file is written for the answer.
        assert_eq!(args.last().map(String::as_str), Some("-"));
        for never in ["-o", "--output-last-message", "--search", "--full-auto", "--approve-for-me"] {
            assert!(!args.iter().any(|a| a == never), "{never} in {args:?}");
        }
        assert!(!args.iter().any(|a| a.starts_with("--dangerously")));
        no_text_in(&args);
    }

    /// CUE's fixed instructions replace Codex's own (a file), and the answer is held to `{"text": …}` (a schema file).
    /// Both paths go with forward slashes and no quotes: the value of `-c` is read as TOML, and an npm shim passes the
    /// arguments through `cmd.exe`.
    #[test]
    fn codex_args_name_the_instructions_and_schema_files() {
        let args = codex("gpt-6-luna");
        assert!(
            has_pair(&args, "-c", "model_instructions_file=C:/Users/Ann/AppData/Local/CUE/agent/codex-instructions.md"),
            "{args:?}"
        );
        assert!(has_pair(&args, "--output-schema", "C:/Users/Ann/AppData/Local/CUE/agent/codex-schema.json"), "{args:?}");
        for a in &args {
            assert!(!a.contains(['\\', '"', '%', '^', '&', '|', '<', '>', '!']), "{a:?} may not survive cmd.exe");
        }
    }

    #[test]
    fn codex_args_take_the_default_model_for_an_unsafe_one() {
        for bad in ["", "gpt\" & calc", "a b", "x%PATH%", "m\n-c"] {
            assert!(has_pair(&codex(bad), "-m", "gpt-6-luna"), "{bad:?}");
        }
        assert!(has_pair(&codex("gpt-6.1-sol"), "-m", "gpt-6.1-sol"));
    }

    #[test]
    fn codex_text_reads_last_message() {
        assert_eq!(codex_text(CODEX_ANSWER).as_deref(), Some("{\"text\": \"Hello world.\"}"));
        assert_eq!(codex_text(CODEX_EXPIRED_OUT), None);
        assert_eq!(codex_text(CODEX_SIGNED_OUT_OUT), None);
        assert_eq!(codex_text(""), None);
        assert_eq!(codex_text("not json\n"), None);
    }

    /// Real runs of codex-cli 0.166.0 with CUE's flags, signed in (09.11.2026): a cleanup, and the stripping probes,
    /// where the model first says what it will try and then that it cannot. No tool item in any of them.
    #[test]
    fn codex_text_reads_real_signed_in_runs() {
        let cleanup = include_str!("../../tests/fixtures/agents/codex-cleanup.jsonl");
        assert_eq!(
            codex_text(cleanup).as_deref(),
            Some("Завтра в десять созвон с командой по релизу. Не забудь взять ноутбук.")
        );
        let list = include_str!("../../tests/fixtures/agents/codex-probe-list.jsonl");
        assert!(codex_text(list).is_some_and(|t| t.starts_with("I can’t access the current folder")));
        for probe in [
            list,
            include_str!("../../tests/fixtures/agents/codex-probe-read.jsonl"),
            include_str!("../../tests/fixtures/agents/codex-probe-write.jsonl"),
        ] {
            assert!(!probe.contains("CANARY"), "the canary file was read");
            assert!(codex_text(probe).is_some(), "no tool item, so the answer is trusted");
        }
    }

    #[test]
    fn codex_text_refuses_an_answer_after_a_tool_ran() {
        assert_eq!(codex_text(CODEX_RAN_COMMAND), None);
    }

    /// "Ask" with web search on: live search replaces `disabled`, and the code-mode host it runs through stays on;
    /// every other flag stays as it is. Off, the arguments are the cleanup's own, the host disabled.
    #[test]
    fn codex_searches_when_asked() {
        let search = codex_args("gpt-6-luna", Path::new(INSTRUCTIONS), Path::new(SCHEMA), true);
        assert!(has_pair(&search, "-c", "web_search=live"), "{search:?}");
        assert!(!has_pair(&search, "-c", "web_search=disabled"));
        assert!(!has_pair(&search, "--disable", "code_mode_host"), "live search needs the code-mode host");
        let quiet = codex("gpt-6-luna");
        assert!(has_pair(&quiet, "-c", "web_search=disabled"));
        assert!(has_pair(&quiet, "--disable", "code_mode_host"));
        // Exactly those two differ: the quiet run has the host's pair, and its search setting reads `disabled`.
        let mut without_host = quiet.clone();
        let at = without_host.windows(2).position(|w| w[0] == "--disable" && w[1] == "code_mode_host").unwrap();
        without_host.drain(at..at + 2);
        let differ: Vec<(&String, &String)> = search.iter().zip(&without_host).filter(|(a, b)| a != b).collect();
        assert_eq!(differ.len(), 1, "only the search setting changes beside the host: {differ:?}");
        assert_eq!(search.len(), without_host.len());
        for never in ["--search", "--full-auto"] {
            assert!(!search.iter().any(|a| a == never), "{never} in {search:?}");
        }
        no_text_in(&search);
    }

    fn item(kind: &str, extra: serde_json::Value) -> String {
        let mut item = serde_json::json!({ "id": "item_1", "type": kind });
        if let (Some(o), Some(more)) = (item.as_object_mut(), extra.as_object()) {
            o.extend(more.clone());
        }
        let started = serde_json::json!({ "type": "item.started", "item": item });
        let completed = serde_json::json!({ "type": "item.completed", "item": item });
        format!("{started}\n{completed}\n")
    }

    /// A searched answer: its web searches are allowed only when Codex was given search, and any other tool still
    /// refuses the answer.
    #[test]
    fn codex_search_allows_only_web_search_items() {
        let answer = format!(
            "{}{}{}",
            item("web_search", serde_json::json!({ "query": "rust latest version" })),
            item("web_search", serde_json::json!({ "query": "rust 1.95" })),
            item("agent_message", serde_json::json!({ "text": "{\"answer\": \"1.95\"}" })),
        );
        assert_eq!(codex_answer(&answer, true), Some(("{\"answer\": \"1.95\"}".to_string(), 2)));
        assert_eq!(codex_answer(&answer, false), None, "a search Codex was never given");
        assert_eq!(codex_text(&answer), None);
        for tool in ["command_execution", "file_change", "mcp_tool_call"] {
            let ran = format!("{answer}{}", item(tool, serde_json::json!({})));
            assert_eq!(codex_answer(&ran, true), None, "{tool} with search on");
        }
        assert_eq!(codex_answer(CODEX_RAN_COMMAND, true), None);
        assert_eq!(codex_answer(CODEX_ANSWER, true), Some(("{\"text\": \"Hello world.\"}".to_string(), 0)));
    }

    /// A real searched run of codex-cli 0.166.0 (09.11.2026, the code-mode host on, live search): one `web_search`
    /// item, then the answer; accepted only when search was granted.
    #[test]
    fn codex_answer_reads_a_real_searched_run() {
        let run = include_str!("../../tests/fixtures/agents/codex-ask-search.jsonl");
        let (text, searches) = codex_answer(run, true).expect("a searched answer is trusted when search was granted");
        assert!(text.starts_with("The latest stable Rust version is **1.99.1**"), "{text}");
        assert_eq!(searches, 1);
        assert_eq!(codex_answer(run, false), None, "a search Codex was never given");
        assert_eq!(codex_text(run), None);
    }

    /// `codex login status` as codex-cli 0.166.0 answers it (fixtures README, "Signed out, as each CLI says it").
    #[test]
    fn codex_login_status_reads_signed_out_and_in() {
        use crate::agent::Login;
        assert_eq!(codex_login_args(), ["login", "status"]);
        assert_eq!(codex_login(&output(1, "", "Not logged in\n")), Login::SignedOut);
        assert_eq!(codex_login(&output(1, "Not logged in\n", "")), Login::SignedOut);
        assert_eq!(codex_login(&output(0, "Logged in using ChatGPT\n", "")), Login::SignedIn);
        assert_eq!(codex_login(&output(1, "", "NOT LOGGED IN")), Login::SignedOut, "any case");
        assert_eq!(codex_login(&output(2, "", "error: unrecognized subcommand 'status'")), Login::Unknown);
        assert_eq!(codex_login(&output(1, "", "")), Login::Unknown, "exit 1 alone is not a sign-out");
        assert_eq!(codex_login(&output(1, "", "Error: could not read auth.json")), Login::Unknown);
    }

    #[test]
    fn signed_out_detected() {
        assert!(signed_out(AgentId::Codex, &output(1, CODEX_EXPIRED_OUT, CODEX_EXPIRED_ERR)));
        assert!(signed_out(AgentId::Codex, &output(1, CODEX_SIGNED_OUT_OUT, CODEX_SIGNED_OUT_ERR)));
        assert!(!signed_out(AgentId::Codex, &output(0, CODEX_ANSWER, "")));
        assert!(!signed_out(AgentId::Codex, &output(1, "", "panicked at main.rs")));
        // A dictation about signing in, echoed in an answer, is not a sign-in failure.
        let echoed = "{\"type\":\"item.completed\",\"item\":{\"id\":\"1\",\"type\":\"agent_message\",\"text\":\"Please sign in again.\"}}\n";
        assert!(!signed_out(AgentId::Codex, &output(0, echoed, "")));
        assert!(!signed_out(AgentId::Codex, &output(1, CODEX_ANSWER, "Error: You've reached your usage limit.")));
    }

    /// codex-cli 0.166.0's own words for an account out of its allowance, in an error event or on stderr, any case; never
    /// from the answer, a run that succeeded, or a sign-in failure.
    #[test]
    fn rate_limit_detected() {
        let event = |m: &str| format!("{}\n", serde_json::json!({ "type": "error", "message": m }));
        let failed = |m: &str| format!("{}\n", serde_json::json!({ "type": "turn.failed", "error": { "message": m } }));
        for said in [
            "You've reached your usage limit. Increase your limits to continue using codex.",
            "Usage limit reached.",
            "Your workspace is out of credits. Add credits to continue using Codex.",
            "stream error: rate limit exceeded",
            "unexpected status 429 Too Many Requests",
        ] {
            assert!(rate_limited(AgentId::Codex, &output(1, &event(said), "")), "{said}");
            assert!(rate_limited(AgentId::Codex, &output(1, &failed(said), "")), "{said}");
            assert!(rate_limited(AgentId::Codex, &output(1, "", &said.to_uppercase())), "{said}");
            assert!(!signed_out(AgentId::Codex, &output(1, &event(said), "")), "{said}");
        }
        assert!(!rate_limited(AgentId::Codex, &output(0, &event("Usage limit reached."), "")), "the run succeeded");
        let echoed = "{\"type\":\"item.completed\",\"item\":{\"id\":\"1\",\"type\":\"agent_message\",\"text\":\"rate limit\"}}\n";
        assert!(!rate_limited(AgentId::Codex, &output(1, echoed, "")), "the answer is never read");
        assert!(!rate_limited(AgentId::Codex, &output(1, CODEX_EXPIRED_OUT, CODEX_EXPIRED_ERR)));
        assert!(!rate_limited(AgentId::Codex, &output(1, "", "panicked at main.rs")));
        assert!(!rate_limited(AgentId::OpenCode, &output(1, "", "rate limit")));
    }
}
