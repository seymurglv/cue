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
//! - `report`: prints what it was given as JSON: its arguments, stdin, working folder, the folder's entry count and
//!   whether it has a console window.
//! - `sleep <ms>`: sleeps.
//! - `spawn <pid file>`: starts a child of itself that sleeps a minute, writes the child's pid to the file, sleeps.
//! - `fail <code> <bytes>`: writes `<bytes>` characters of noise and then `END` to stderr, exits with `<code>`.
//! - `serve --hostname <h> --port <p>`: OpenCode's server stand-in: listens there and answers every request with a JSON
//!   report of its arguments, the length of `OPENCODE_SERVER_PASSWORD`, its working folder and whether it has a
//!   console window.
use std::io::{Read, Write};
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
