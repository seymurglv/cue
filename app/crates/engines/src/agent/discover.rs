//! Finding the agents installed on this computer: each agent of [`ORDER`] by its program name on `PATH`, then in its
//! usual install folders (npm's global folder, Grok's and OpenCode's own `bin`, WinGet's and Scoop's links). Only
//! these names are looked for, so Claude Code or Gemini is never found, whatever is installed.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cue_core::agents::{AgentId, ORDER};

use super::{Found, process};

/// How long an agent may take to say its version.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// The program forms CUE starts, the first found in a folder in this order. An npm folder also holds an
/// extensionless shell script and a `.ps1` beside the `.cmd`: neither is run (one needs a Unix shell, the other
/// PowerShell and its execution policy).
const EXTENSIONS: [&str; 3] = ["exe", "cmd", "bat"];
/// Grok's only form: it installs `grok.exe`, and its system prompt, which spans lines, cannot pass through a batch
/// file (Rust refuses such an argument), so a `grok.cmd` or `grok.bat` is never run.
const GROK_EXTENSIONS: [&str; 1] = ["exe"];

/// The agents installed here, in [`ORDER`], each with its version when it answers `--version` in time.
pub fn discover() -> Vec<Found> {
    let path = std::env::var_os("PATH");
    let on_path = path_dirs(path.as_deref());
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let (home, roaming, local) = (var("USERPROFILE"), var("APPDATA"), var("LOCALAPPDATA"));
    let dirs = |id: AgentId| {
        let mut d = on_path.clone();
        d.extend(usual_dirs(id, home.as_deref(), roaming.as_deref(), local.as_deref()));
        d
    };
    let home = match cue_core::paths::local_app_data() {
        Some(l) => cue_core::paths::local_dir(&l).join("agent"),
        None => std::env::temp_dir().join("CUE-agent"),
    };
    discover_in(&dirs, &home)
}

/// Where agent `id`'s program is, by the same search as [`discover`] but without asking its version.
pub fn program(id: AgentId) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut dirs = path_dirs(path.as_deref());
    dirs.extend(usual_dirs(id, var("USERPROFILE").as_deref(), var("APPDATA").as_deref(), var("LOCALAPPDATA").as_deref()));
    dirs.iter().find_map(|d| program_in(d, id))
}

/// [`discover`] over the folders `dirs` gives for each agent, asking each version in that agent's empty folder in
/// `home`, CUE's agent folder ([`super::empty_cwd`]).
pub(crate) fn discover_in(dirs: &dyn Fn(AgentId) -> Vec<PathBuf>, home: &Path) -> Vec<Found> {
    let programs: Vec<(AgentId, PathBuf)> =
        ORDER.iter().filter_map(|&id| dirs(id).iter().find_map(|d| program_in(d, id)).map(|exe| (id, exe))).collect();
    // Asked together: an npm shim starts Node, which takes a moment.
    std::thread::scope(|s| {
        let asks: Vec<_> = programs
            .into_iter()
            .map(|(id, exe)| {
                s.spawn(move || {
                    let version = super::empty_cwd(home, id)
                        .and_then(|cwd| process::run(&exe, &["--version".to_string()], "", &cwd, VERSION_TIMEOUT))
                        .ok()
                        .filter(|o| o.status == Some(0))
                        .and_then(|o| version_of(&o.stdout));
                    Found { id, exe, version }
                })
            })
            .collect();
        asks.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// Where each agent's installer puts its program, besides `PATH` (a program started at sign-in may not see a `PATH`
/// the installer changed later).
fn usual_dirs(id: AgentId, home: Option<&Path>, roaming: Option<&Path>, local: Option<&Path>) -> Vec<PathBuf> {
    let npm = roaming.map(|r| r.join("npm"));
    let winget = local.map(|l| l.join("Microsoft").join("WinGet").join("Links"));
    let scoop = home.map(|h| h.join("scoop").join("shims"));
    match id {
        AgentId::OpenCode => vec![home.map(|h| h.join(".opencode").join("bin")), npm, winget, scoop],
        AgentId::Codex => vec![npm, winget, scoop],
        AgentId::Grok => vec![home.map(|h| h.join(".grok").join("bin"))],
    }
    .into_iter()
    .flatten()
    .collect()
}

/// `<dir>\<agent>.exe`, `.cmd` or `.bat`, the first that is a file; for Grok `grok.exe` only.
fn program_in(dir: &Path, id: AgentId) -> Option<PathBuf> {
    let extensions: &[&str] = if id == AgentId::Grok { &GROK_EXTENSIONS } else { &EXTENSIONS };
    extensions.iter().map(|ext| dir.join(format!("{}.{ext}", id.id()))).find(|p| p.is_file())
}

/// The version in `--version`'s first line: its first word that is a dotted number (`v` before it dropped).
fn version_of(stdout: &str) -> Option<String> {
    stdout.lines().next()?.split_whitespace().find_map(|w| {
        let w = w.strip_prefix('v').unwrap_or(w);
        let dotted = w.contains('.') && w.starts_with(|c: char| c.is_ascii_digit());
        (dotted && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))).then(|| w.to_string())
    })
}

/// The absolute folders of a `PATH` value. A relative entry (`.`, `bin`, `C:foo`) is skipped: it would resolve
/// against CUE's own working folder.
fn path_dirs(path: Option<&OsStr>) -> Vec<PathBuf> {
    path.map(|p| std::env::split_paths(p).filter(|d| d.is_absolute()).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::agents::ORDER;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-discover-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A batch file that prints `line` for `--version`, as an npm shim answers.
    fn shim(dir: &Path, name: &str, line: &str) {
        std::fs::write(dir.join(name), format!("@ECHO off\r\necho {line}\r\n")).unwrap();
    }

    #[test]
    fn discover_finds_npm_cmd_and_ps1_shims() {
        let base = temp_dir("npm");
        let bin = base.join("npm");
        std::fs::create_dir_all(&bin).unwrap();
        // npm writes three shims: a shell script, a .cmd and a .ps1. CUE takes the .cmd.
        std::fs::write(bin.join("codex"), "#!/bin/sh\nexec node codex.js \"$@\"\n").unwrap();
        shim(&bin, "codex.cmd", "codex-cli 0.166.0");
        std::fs::write(bin.join("codex.ps1"), "& node codex.js $args\r\n").unwrap();
        let found = discover_in(&|_| vec![bin.clone()], &base.join("agent"));
        assert_eq!(
            found,
            vec![Found { id: AgentId::Codex, exe: bin.join("codex.cmd"), version: Some("0.166.0".into()) }]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_lone_ps1_or_shell_script_is_not_run() {
        let base = temp_dir("ps1");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("codex.ps1"), "& node codex.js $args\r\n").unwrap();
        std::fs::write(bin.join("codex"), "#!/bin/sh\n").unwrap();
        assert!(discover_in(&|_| vec![bin.clone()], &base.join("agent")).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn discover_order_is_opencode_codex_grok() {
        let base = temp_dir("order");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // Written in the opposite order, with Claude Code and Gemini beside them: never offered. Grok is an `.exe`
        // (this one does not start, so it has no version).
        std::fs::write(bin.join("grok.exe"), b"MZ").unwrap();
        for name in ["codex", "opencode", "claude", "gemini"] {
            shim(&bin, &format!("{name}.cmd"), &format!("{name} 1.2.3"));
        }
        let found = discover_in(&|_| vec![bin.clone()], &base.join("agent"));
        let ids: Vec<AgentId> = found.iter().map(|f| f.id).collect();
        assert_eq!(ids, ORDER.to_vec());
        assert!(found.iter().filter(|f| f.id != AgentId::Grok).all(|f| f.version.as_deref() == Some("1.2.3")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn earlier_folder_wins_and_an_exe_beats_a_cmd() {
        let base = temp_dir("first");
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        shim(&a, "codex.cmd", "codex-cli 1.0.0");
        shim(&b, "codex.cmd", "codex-cli 2.0.0");
        let found = discover_in(&|_| vec![a.clone(), b.clone()], &base.join("agent"));
        assert_eq!(found[0].exe, a.join("codex.cmd"));
        assert_eq!(super::program_in(&a, AgentId::Codex), Some(a.join("codex.cmd")));
        std::fs::write(a.join("codex.exe"), b"MZ").unwrap();
        assert_eq!(super::program_in(&a, AgentId::Codex), Some(a.join("codex.exe")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Grok only as `grok.exe`: a `grok.cmd` or `grok.bat` found first is passed over for the `.exe` in a later
    /// folder (Grok's own `bin`), and alone it is not Grok at all.
    #[test]
    fn grok_is_found_only_as_an_exe() {
        let base = temp_dir("grok-exe");
        let (path, own) = (base.join("path"), base.join(".grok").join("bin"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::create_dir_all(&own).unwrap();
        shim(&path, "grok.cmd", "grok 9.9.9");
        shim(&path, "grok.bat", "grok 9.9.9");
        let dirs = |_| vec![path.clone(), own.clone()];
        assert!(discover_in(&dirs, &base.join("agent")).iter().all(|f| f.id != AgentId::Grok), "a grok.cmd is not Grok");
        assert_eq!(super::program_in(&path, AgentId::Grok), None);
        std::fs::write(own.join("grok.exe"), b"MZ").unwrap();
        let found = discover_in(&dirs, &base.join("agent"));
        assert_eq!(found.iter().find(|f| f.id == AgentId::Grok).map(|f| f.exe.clone()), Some(own.join("grok.exe")));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn version_is_the_number_in_the_first_line() {
        assert_eq!(super::version_of("codex-cli 0.166.0\r\n"), Some("0.166.0".into()));
        assert_eq!(super::version_of("grok 1.0.34 (3736acbc8658) [stable]\n"), Some("1.0.34".into()));
        assert_eq!(super::version_of("opencode v1.2.3"), Some("1.2.3".into()));
        assert_eq!(super::version_of("no version here"), None);
        assert_eq!(super::version_of(""), None);
    }

    #[test]
    fn relative_path_entries_are_skipped() {
        let dirs = super::path_dirs(Some(std::ffi::OsStr::new(r".;bin;C:\Tools;;C:\Program Files\X")));
        assert_eq!(dirs, vec![PathBuf::from(r"C:\Tools"), PathBuf::from(r"C:\Program Files\X")]);
    }
}
