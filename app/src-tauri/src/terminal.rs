//! "Открыть в <agent>" on the Instructions page (docs/specs/2026-11-08-no-keys-design.md, "Instructions page"): the
//! user's agent started interactively in a new terminal window, in the instructions' own folder, with CUE's task as its
//! first message. Windows Terminal (`wt -w new -d <folder> …`) when it is here, else the agent in a console window of
//! its own.
//!
//! The command lines, checked on this computer (codex-cli 0.166.0, OpenCode Desktop's opencode-cli 2.2.6):
//! - Codex: `codex -C <folder> <task>` (its `[PROMPT]` starts the session; `-C` is its working root).
//! - OpenCode: `opencode-cli --prompt <task> <folder>` (the TUI in that folder, the prompt sent first).
//!
//! The task is CUE's own fixed text (never a dictation), so it may go on a command line. An argument that a console or
//! Windows Terminal would read otherwise (a double quote, `%`, a line break, a trailing backslash) is never sent: the
//! agent then opens without the task, and the caller puts the task in the clipboard. Through Windows Terminal every
//! argument is quoted and a `;` goes as `\;` (it splits its commands at `;` wherever it stands); in a console the
//! agent is started directly, Rust quoting its arguments (an npm `.cmd` shim through `cmd.exe`'s own rules), not
//! through `start`, whose `cmd /K` would strip the quotes. The log gets the agent and the terminal, never the task.
use std::path::{Path, PathBuf};

use cue_core::agents::AgentId;

/// A new console window for the agent when Windows Terminal is not here.
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// Where the agent's window comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    /// Windows Terminal, by its program (an app execution alias in `WindowsApps`).
    Wt(PathBuf),
    /// A console window of the agent's own.
    Console,
}

/// How the agent was opened: with the task as its first message, or without it (the caller copies the task).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Opened {
    WithTask,
    TaskCopied,
}

/// What to start: a program with its command line as it is (Windows Terminal), or the agent itself with its
/// arguments, quoted by Rust, in a new console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    Raw { program: PathBuf, line: String },
    Console { program: PathBuf, args: Vec<String> },
}

/// Windows Terminal when it is here (on `PATH`, else in `%LOCALAPPDATA%\Microsoft\WindowsApps`), else a console.
pub fn find() -> Terminal {
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).collect()).unwrap_or_default();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join("Microsoft").join("WindowsApps"));
    }
    find_in(&dirs)
}

fn find_in(dirs: &[PathBuf]) -> Terminal {
    // An app execution alias is a reparse point no metadata call follows: it is there when its own entry is.
    dirs.iter().map(|d| d.join("wt.exe")).find(|p| std::fs::symlink_metadata(p).is_ok()).map_or(Terminal::Console, Terminal::Wt)
}

/// The agent's arguments: in `dir`, with `task` as its first message when given.
pub fn agent_args(id: AgentId, dir: &Path, task: Option<&str>) -> Vec<String> {
    let dir = dir.to_string_lossy().into_owned();
    let mut args = Vec::new();
    match id {
        AgentId::Codex => {
            args.extend(["-C".to_owned(), dir]);
            args.extend(task.map(str::to_owned));
        }
        // OpenCode (and nothing else is offered): its flags, then the folder it starts in.
        _ => {
            if let Some(task) = task {
                args.extend(["--prompt".to_owned(), task.to_owned()]);
            }
            args.push(dir);
        }
    }
    args
}

/// Whether `arg` reaches the program as it is through Windows Terminal's command line or an npm shim's `cmd.exe`: no
/// double quote, no `%` (a console expands it even in quotes), no line break or NUL, no backslash at the end (it would
/// escape the closing quote).
fn passable(arg: &str) -> bool {
    !arg.contains(['"', '%', '\r', '\n', '\0']) && !arg.ends_with('\\')
}

/// What to start to open `exe` with `args` in `dir`; `None` when an argument cannot pass ([`passable`]).
pub fn launch(term: &Terminal, dir: &Path, exe: &Path, args: &[String]) -> Option<Launch> {
    let dir = dir.to_string_lossy();
    let exe_s = exe.to_string_lossy();
    let mut all = [&*dir, &*exe_s].into_iter().chain(args.iter().map(String::as_str));
    if !all.all(passable) {
        return None;
    }
    match term {
        Terminal::Wt(wt) => {
            // `;` separates Windows Terminal's commands wherever it stands; `\;` is a plain one.
            let quoted = |s: &str| format!("\"{}\"", s.replace(';', "\\;"));
            let mut line = format!("-w new -d {} {}", quoted(&dir), quoted(&exe_s));
            for a in args {
                line.push(' ');
                line.push_str(&quoted(a));
            }
            Some(Launch::Raw { program: wt.clone(), line })
        }
        Terminal::Console => Some(Launch::Console { program: exe.to_owned(), args: args.to_vec() }),
    }
}

/// Opens agent `id` (its program `exe`) in a new terminal window in `dir`, with `task` as its first message when it
/// can pass; otherwise without it (`TaskCopied`: the caller copies the task). Blocking only for the start.
pub fn open(id: AgentId, exe: &Path, dir: &Path, task: &str) -> std::io::Result<Opened> {
    let term = find();
    let (opened, how) = match launch(&term, dir, exe, &agent_args(id, dir, Some(task))) {
        Some(l) => (Opened::WithTask, l),
        None => {
            let bare = launch(&term, dir, exe, &agent_args(id, dir, None));
            let l = bare.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "a path cannot pass"))?;
            (Opened::TaskCopied, l)
        }
    };
    spawn(&how, dir)?;
    let terminal = if matches!(term, Terminal::Wt(_)) { "windows terminal" } else { "console" };
    tracing::info!(agent = id.id(), terminal, ?opened, "instructions: the agent opened in a terminal");
    Ok(opened)
}

/// Starts what `how` says in `dir`, and lets it run.
fn spawn(how: &Launch, dir: &Path) -> std::io::Result<()> {
    let cmd = match how {
        Launch::Raw { program, line } => {
            let mut cmd = std::process::Command::new(program);
            #[cfg(windows)]
            std::os::windows::process::CommandExt::raw_arg(&mut cmd, line);
            #[cfg(not(windows))]
            cmd.arg(line);
            cmd
        }
        Launch::Console { program, args } => {
            let mut cmd = std::process::Command::new(program);
            cmd.args(args);
            #[cfg(windows)]
            std::os::windows::process::CommandExt::creation_flags(&mut cmd, CREATE_NEW_CONSOLE);
            cmd
        }
    };
    let mut cmd = cmd;
    cmd.current_dir(dir).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = r"C:\Users\Ann\AppData\Roaming\CUE\instructions";
    const TASK: &str = "Нужно поправить инструкции CUE; правь только instructions.md. Спроси меня «что изменить».";

    fn wt() -> Terminal {
        Terminal::Wt(PathBuf::from(r"C:\Users\Ann\AppData\Local\Microsoft\WindowsApps\wt.exe"))
    }

    #[test]
    fn codex_gets_its_folder_and_the_task_as_its_prompt() {
        let args = agent_args(AgentId::Codex, Path::new(DIR), Some(TASK));
        assert_eq!(args, ["-C", DIR, TASK]);
        assert_eq!(agent_args(AgentId::Codex, Path::new(DIR), None), ["-C", DIR]);
    }

    #[test]
    fn opencode_gets_the_task_as_its_prompt_and_starts_in_the_folder() {
        let args = agent_args(AgentId::OpenCode, Path::new(DIR), Some(TASK));
        assert_eq!(args, ["--prompt", TASK, DIR]);
        assert_eq!(agent_args(AgentId::OpenCode, Path::new(DIR), None), [DIR]);
    }

    /// Windows Terminal: a new window in the folder; every argument quoted, `;` escaped (it would start another tab).
    #[test]
    fn windows_terminal_line_quotes_and_escapes_semicolons() {
        let exe = Path::new(r"C:\Users\Ann\AppData\Roaming\npm\codex.cmd");
        let args = agent_args(AgentId::Codex, Path::new(DIR), Some(TASK));
        let Some(Launch::Raw { program, line }) = launch(&wt(), Path::new(DIR), exe, &args) else { panic!("raw") };
        assert!(program.ends_with("wt.exe"));
        let task = TASK.replace(';', "\\;");
        assert_eq!(line, format!("-w new -d \"{DIR}\" \"{}\" \"-C\" \"{DIR}\" \"{task}\"", exe.display()));
        assert!(!line.replace("\\;", "").contains(';'), "no bare semicolon");
    }

    /// A console: the agent itself, its arguments as they are (Rust quotes them).
    #[test]
    fn console_starts_the_agent_itself() {
        let exe = Path::new(r"C:\Users\Ann\AppData\Roaming\ai.opencode.desktop\cli\2.2.6-x64\opencode-cli.exe");
        let args = agent_args(AgentId::OpenCode, Path::new(DIR), Some(TASK));
        assert_eq!(launch(&Terminal::Console, Path::new(DIR), exe, &args), Some(Launch::Console { program: exe.to_owned(), args }));
    }

    /// What a console or Windows Terminal would read otherwise is never sent: the agent then opens without the task.
    #[test]
    fn an_argument_that_cannot_pass_is_refused() {
        let exe = Path::new(r"C:\npm\codex.cmd");
        for bad in ["say \"hi\"", "100% sure", "two\nlines", "cr\rhere", "ends with \\", "nul\0"] {
            let args = agent_args(AgentId::Codex, Path::new(DIR), Some(bad));
            assert_eq!(launch(&wt(), Path::new(DIR), exe, &args), None, "{bad:?}");
            assert_eq!(launch(&Terminal::Console, Path::new(DIR), exe, &args), None, "{bad:?}");
        }
        // CUE's own tasks pass in both languages.
        for lang in ["ru", "en"] {
            let task = cue_core::instructions::task(lang);
            let args = agent_args(AgentId::Codex, Path::new(DIR), Some(&task));
            assert!(launch(&wt(), Path::new(DIR), exe, &args).is_some(), "{lang}");
            assert!(launch(&Terminal::Console, Path::new(DIR), exe, &args).is_some(), "{lang}");
        }
    }

    /// Windows Terminal is found by its alias's own entry, else a console is used.
    #[test]
    fn windows_terminal_is_found_by_its_entry() {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-terminal", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(find_in(std::slice::from_ref(&dir)), Terminal::Console);
        std::fs::write(dir.join("wt.exe"), b"").unwrap();
        assert_eq!(find_in(std::slice::from_ref(&dir)), Terminal::Wt(dir.join("wt.exe")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Live check, not run by default (it opens windows): a fake agent — Node with a script that writes the arguments
    /// it got — opened as an `.exe` (like opencode-cli) and as an npm `.cmd` shim (like codex), through Windows Terminal
    /// and in a console; each must receive CUE's Russian task as one argument, unchanged, in the folder. Run with
    /// `cargo test -p cue --bin cue -- --ignored terminal::tests::live`.
    #[test]
    #[ignore]
    fn live_fake_agent_receives_the_task_unchanged() {
        let node = std::env::var_os("PATH")
            .and_then(|p| std::env::split_paths(&p).map(|d| d.join("node.exe")).find(|p| p.is_file()))
            .expect("node.exe on PATH");
        let dir = std::env::temp_dir().join(format!("cue-live-terminal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("argv.js");
        // Each run writes a file of its own, named by its first argument.
        std::fs::write(
            &script,
            "const a=process.argv.slice(2);require('fs').writeFileSync(require('path').join(__dirname,a[0]+'.json'),\
             JSON.stringify({argv:a.slice(1),cwd:process.cwd()}));",
        )
        .unwrap();
        let shim = dir.join("fake.cmd");
        std::fs::write(&shim, format!("@\"{}\" \"%~dp0argv.js\" %*\r\n", node.display())).unwrap();
        let task = cue_core::instructions::task("ru");
        let terms = match find() {
            Terminal::Console => vec![Terminal::Console],
            wt => vec![wt, Terminal::Console],
        };
        for (i, term) in terms.iter().enumerate() {
            for (j, (exe, through_node)) in [(&node, true), (&shim, false)].into_iter().enumerate() {
                let name = format!("run{i}{j}");
                let out = dir.join(format!("{name}.json"));
                // Node.exe takes the script first; the shim names it itself.
                let mut args = if through_node { vec![script.to_string_lossy().into_owned()] } else { Vec::new() };
                args.push(name);
                let agent = agent_args(AgentId::Codex, &dir, Some(&task));
                args.extend(agent.iter().cloned());
                spawn(&launch(term, &dir, exe, &args).unwrap(), &dir).unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                while !out.is_file() && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                std::thread::sleep(std::time::Duration::from_millis(300));
                let bytes = std::fs::read(&out).unwrap_or_else(|_| panic!("the fake agent did not run: {term:?} {}", exe.display()));
                let got: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                let argv: Vec<String> = serde_json::from_value(got["argv"].clone()).unwrap();
                assert_eq!(argv, agent, "{term:?} {}", exe.display());
                let cwd = std::fs::canonicalize(got["cwd"].as_str().unwrap()).unwrap();
                assert_eq!(cwd, std::fs::canonicalize(&dir).unwrap(), "{term:?} {}", exe.display());
                eprintln!("ok: {term:?} {}", exe.display());
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
