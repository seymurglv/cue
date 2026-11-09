//! A stand-in for an agent's CLI in CUE's tests (`agent::process`): no network, no model, no real agent. What it does
//! is its first argument:
//!
//! - `echo-codex`: prints stdin as Codex's last message, in `codex exec --json` lines.
//! - `exec …` (Codex's own first argument): Codex's stand-in for `agent::refine`, told what to do by its environment
//!   (a test's `.cmd` shim sets it, as `codex.cmd` starts the real one): `FAKE_AGENT_RECORD` names a file it writes
//!   what it got to, as JSON (its arguments, stdin, and the contents of the files named by
//!   `-c model_instructions_file=<path>` and `--output-schema <path>`, as Codex reads them); it sleeps
//!   `FAKE_AGENT_SLEEP` ms, prints the files `FAKE_AGENT_STDOUT` and `FAKE_AGENT_STDERR` (a canned answer) and exits
//!   with `FAKE_AGENT_EXIT`. `FAKE_AGENT_PID` names a file it writes its process id to before it sleeps.
//! - `login status` (Codex's): prints `FAKE_AGENT_LOGIN` (default "Logged in using ChatGPT") and exits with
//!   `FAKE_AGENT_LOGIN_EXIT` (default 0).
//! - `echo-grok`: prints stdin as Grok's answer, in `grok -p --output-format json`'s object.
//! - `report`: prints what it was given as JSON: its arguments, stdin, working folder, the folder's entry count and
//!   whether it has a console window.
//! - `sleep <ms>`: sleeps.
//! - `spawn <pid file>`: starts a child of itself that sleeps a minute, writes the child's pid to the file, sleeps.
//! - `fail <code> <bytes>`: writes `<bytes>` characters of noise and then `END` to stderr, exits with `<code>`.
//! - `serve --hostname <h> --port <p>`: OpenCode's server stand-in: listens there and answers every request with a JSON
//!   report of its arguments, the length of `OPENCODE_SERVER_PASSWORD`, its working folder and whether it has a
//!   console window.
//!
//! Copied to a file named `grok.exe` it is Grok's stand-in, told what to do by `fake-grok.json` beside it (Grok is an
//! `.exe`, and its system prompt, which spans lines, cannot pass through a `.cmd` shim): see [`grok`].
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn stdin() -> String {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).expect("stdin");
    s
}

fn has_console_window() -> bool {
    #[cfg(windows)]
    {
        // SAFETY: no arguments; returns a window handle or null.
        !unsafe { windows::Win32::System::Console::GetConsoleWindow() }.is_invalid()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str).unwrap_or("");
    let me = std::env::current_exe().expect("exe");
    if me.file_stem().is_some_and(|s| s.eq_ignore_ascii_case("grok")) {
        return grok(&me, &args);
    }
    match arg(0) {
        "echo-codex" => {
            let text = stdin();
            let item = serde_json::json!({"type": "item.completed", "item": {"id": "item_1", "type": "agent_message", "text": text}});
            println!("{}", serde_json::json!({"type": "thread.started", "thread_id": "00000000-0000-0000-0000-000000000000"}));
            println!("{}", serde_json::json!({"type": "turn.started"}));
            println!("{item}");
            println!("{}", serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 1, "output_tokens": 1}}));
        }
        "exec" => codex(&args),
        "login" => {
            let said = std::env::var("FAKE_AGENT_LOGIN").unwrap_or_else(|_| "Logged in using ChatGPT".into());
            println!("{said}");
            std::process::exit(std::env::var("FAKE_AGENT_LOGIN_EXIT").ok().and_then(|v| v.parse().ok()).unwrap_or(0));
        }
        "echo-grok" => {
            let text = stdin();
            println!("{}", serde_json::json!({"text": text, "stopReason": "end_turn", "sessionId": "0", "requestId": "0"}));
        }
        "report" => {
            let input = stdin();
            let cwd = std::env::current_dir().expect("cwd");
            let entries = std::fs::read_dir(&cwd).map(|d| d.count()).unwrap_or(usize::MAX);
            let report = serde_json::json!({
                "args": args,
                "stdin": input,
                "cwd": cwd,
                "cwd_entries": entries,
                "console_window": has_console_window(),
            });
            println!("{report}");
        }
        "sleep" => std::thread::sleep(Duration::from_millis(arg(1).parse().unwrap_or(60_000))),
        "spawn" => {
            let me = std::env::current_exe().expect("exe");
            let mut child = std::process::Command::new(me)
                .args(["sleep", "60000"])
                .stdin(std::process::Stdio::null())
                .spawn()
                .expect("child");
            std::fs::write(arg(1), child.id().to_string()).expect("pid file");
            std::thread::sleep(Duration::from_secs(60));
            let _ = child.wait();
        }
        "fail" => {
            let n: usize = arg(2).parse().unwrap_or(0);
            let mut err = std::io::stderr();
            err.write_all("x".repeat(n).as_bytes()).ok();
            err.write_all(b"END").ok();
            std::process::exit(arg(1).parse().unwrap_or(1));
        }
        "serve" => {
            let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            let addr = format!("{}:{}", flag("--hostname").unwrap_or_default(), flag("--port").unwrap_or_default());
            let listener = std::net::TcpListener::bind(addr).expect("bind");
            let report = serde_json::json!({
                "args": args,
                "password_len": std::env::var("OPENCODE_SERVER_PASSWORD").map(|p| p.len()).unwrap_or(0),
                "cwd": std::env::current_dir().expect("cwd"),
                "console_window": has_console_window(),
            })
            .to_string();
            for sock in listener.incoming().flatten() {
                serve_one(sock, &report);
            }
        }
        other => {
            eprintln!("fake_agent: unknown mode {other:?}");
            std::process::exit(2);
        }
    }
}

/// `codex exec` as a test sets it up (see the module's comment).
fn codex(args: &[String]) {
    let input = stdin();
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let read = |path: Option<&str>| path.and_then(|p| std::fs::read_to_string(p).ok());
    if let Some(record) = env("FAKE_AGENT_RECORD") {
        let instructions = args.iter().find_map(|a| a.strip_prefix("model_instructions_file="));
        let schema = args.iter().position(|a| a == "--output-schema").and_then(|i| args.get(i + 1));
        let got = serde_json::json!({
            "args": args,
            "stdin": input,
            "instructions": read(instructions),
            "schema": read(schema.map(String::as_str)),
        });
        std::fs::write(record, got.to_string()).expect("record");
    }
    if let Some(pid) = env("FAKE_AGENT_PID") {
        std::fs::write(pid, std::process::id().to_string()).expect("pid file");
    }
    if let Some(ms) = env("FAKE_AGENT_SLEEP").and_then(|v| v.parse().ok()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    if let Some(out) = read(env("FAKE_AGENT_STDOUT").as_deref()) {
        print!("{out}");
    }
    if let Some(err) = read(env("FAKE_AGENT_STDERR").as_deref()) {
        eprint!("{err}");
    }
    std::process::exit(env("FAKE_AGENT_EXIT").and_then(|v| v.parse().ok()).unwrap_or(0));
}

/// Grok as a test sets it up in `fake-grok.json` beside the program (every field optional):
///
/// - `grok sessions delete <id>`: appends `<id>` and its working folder, tab-separated, to the file `deleted`, and
///   removes `<data>\sessions\*\<id>` as Grok does; exit 0, or 1 when there was no such session.
/// - any other run (headless): reads the prompt from `--prompt-file` unless `read_prompt` is false; with `data` set,
///   keeps the run as Grok does: `<data>\sessions\<working folder, percent-encoded>\<--session-id>\chat_history.jsonl`
///   and a line in `prompt_history.jsonl` beside it; writes what it got to `record` (its arguments, the prompt, the
///   system prompt, its `GROK_*` variables, its working folder); writes its pid to `pid`; sleeps `sleep_ms`; prints the
///   files `stdout` (`{session}` replaced by the session id) and `stderr`; exits with `exit`.
fn grok(me: &Path, args: &[String]) {
    let config: serde_json::Value = std::fs::read_to_string(me.with_file_name("fake-grok.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let path = |k: &str| config[k].as_str().map(PathBuf::from);
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let cwd = std::env::current_dir().expect("cwd");
    if args.first().map(String::as_str) == Some("sessions") && args.get(1).map(String::as_str) == Some("delete") {
        let id = args.get(2).cloned().unwrap_or_default();
        if let Some(deleted) = path("deleted") {
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(deleted).expect("deleted");
            writeln!(f, "{id}\t{}", cwd.display()).expect("deleted");
        }
        let mut found = false;
        if let Some(sessions) = path("data").map(|d| d.join("sessions"))
            && let Ok(groups) = std::fs::read_dir(sessions)
        {
            for g in groups.flatten() {
                let dir = g.path().join(&id);
                if !id.is_empty() && dir.is_dir() {
                    std::fs::remove_dir_all(&dir).expect("delete the session");
                    found = true;
                }
            }
        }
        std::process::exit(if found { 0 } else { 1 });
    }
    let session = flag("--session-id").unwrap_or_default();
    let prompt = if config["read_prompt"].as_bool() != Some(false) {
        flag("--prompt-file").map(|p| std::fs::read_to_string(p).expect("the prompt file reads"))
    } else {
        None
    };
    if let Some(data) = path("data") {
        let group = data.join("sessions").join(encode(&cwd.to_string_lossy()));
        std::fs::create_dir_all(group.join(&session)).expect("session folder");
        std::fs::write(group.join(&session).join("chat_history.jsonl"), prompt.clone().unwrap_or_default()).expect("chat");
        let mut h = std::fs::OpenOptions::new().create(true).append(true).open(group.join("prompt_history.jsonl")).expect("history");
        writeln!(h, "{}", serde_json::json!({ "prompt": prompt })).expect("history");
    }
    if let Some(record) = path("record") {
        let env: serde_json::Map<String, serde_json::Value> =
            std::env::vars().filter(|(k, _)| k.starts_with("GROK_")).map(|(k, v)| (k, v.into())).collect();
        let got = serde_json::json!({
            "args": args,
            "prompt": prompt,
            "system": flag("--system-prompt-override"),
            "env": env,
            "cwd": cwd,
        });
        std::fs::write(record, got.to_string()).expect("record");
    }
    if let Some(pid) = path("pid") {
        std::fs::write(pid, std::process::id().to_string()).expect("pid file");
    }
    if let Some(ms) = config["sleep_ms"].as_u64() {
        std::thread::sleep(Duration::from_millis(ms));
    }
    if let Some(out) = path("stdout").and_then(|p| std::fs::read_to_string(p).ok()) {
        print!("{}", out.replace("{session}", &session));
    }
    if let Some(err) = path("stderr").and_then(|p| std::fs::read_to_string(p).ok()) {
        eprint!("{err}");
    }
    std::process::exit(config["exit"].as_i64().unwrap_or(0) as i32);
}

/// A working folder as Grok names its sessions folder: percent-encoded, letters, digits and `-_.~` kept.
fn encode(path: &str) -> String {
    path.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }
        })
        .collect()
}

/// Reads one request's head and answers it with `body`.
fn serve_one(mut sock: std::net::TcpStream, body: &str) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match sock.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let reply = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(reply.as_bytes());
}
