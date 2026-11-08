//! A stand-in for an agent's CLI in CUE's tests (`agent::process`): no network, no model, no real agent. What it does
//! is its first argument:
//!
//! - `echo-codex`: prints stdin as Codex's last message, in `codex exec --json` lines.
//! - `echo-grok`: prints stdin as Grok's answer, in `grok -p --output-format json`'s object.
//! - `report`: prints what it was given as JSON: its arguments, stdin, working folder, the folder's entry count and
//!   whether it has a console window.
//! - `sleep <ms>`: sleeps.
//! - `spawn <pid file>`: starts a child of itself that sleeps a minute, writes the child's pid to the file, sleeps.
//! - `fail <code> <bytes>`: writes `<bytes>` characters of noise and then `END` to stderr, exits with `<code>`.
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
        "echo-grok" => {
            let text = stdin();
            println!("{}", serde_json::json!({"text": text, "stopReason": "EndTurn", "sessionId": "0", "requestId": "0"}));
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
        other => {
            eprintln!("fake_agent: unknown mode {other:?}");
            std::process::exit(2);
        }
    }
}
