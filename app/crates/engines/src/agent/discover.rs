//! Finding the agents installed on this computer: each agent of [`ORDER`] by its program name on `PATH`, then in its
//! usual install folders (npm's global folder, Grok's and OpenCode's own `bin`, WinGet's and Scoop's links), then, for
//! OpenCode, OpenCode Desktop's own CLI; an OpenCode whose program is nowhere there still counts when a server it runs
//! announces itself (`opencode::running`). Only these names are looked for, so Claude Code or Gemini is never found,
//! whatever is installed.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cue_core::agents::{AgentId, ORDER};

use super::opencode::{self, Announced};
use super::{Found, process};

/// How long an agent may take to say its version.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// The program forms CUE starts, the first found in a folder in this order. An npm folder also holds an
/// extensionless shell script and a `.ps1` beside the `.cmd`: neither is run (one needs a Unix shell, the other
/// PowerShell and its execution policy).
const EXTENSIONS: [&str; 3] = ["exe", "cmd", "bat"];
/// OpenCode Desktop's folder in `%APPDATA%`, its CLIs in `cli\<version>-<arch>\` under it.
const DESKTOP_DIR: &str = "ai.opencode.desktop";
/// The one program CUE runs from OpenCode Desktop's folder.
const DESKTOP_CLI: &str = "opencode-cli.exe";

/// The agents installed here, in [`ORDER`], each with its version when it answers `--version` in time.
pub fn discover() -> Vec<Found> {
    let home = match cue_core::paths::local_app_data() {
        Some(l) => cue_core::paths::local_dir(&l).join("agent"),
        None => std::env::temp_dir().join("CUE-agent"),
    };
    let state = opencode::state_dir();
    discover_in(&program, &|| state.as_deref().and_then(opencode::running), &home)
}

/// Where agent `id`'s program is, by the same search as [`discover`] but without asking its version: on `PATH`, in
/// its usual install folders, then, for OpenCode, OpenCode Desktop's CLI.
pub fn program(id: AgentId) -> Option<PathBuf> {
    let path = std::env::var_os("PATH");
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let roaming = var("APPDATA");
    let mut dirs = path_dirs(path.as_deref());
    dirs.extend(usual_dirs(id, var("USERPROFILE").as_deref(), roaming.as_deref(), var("LOCALAPPDATA").as_deref()));
    find(id, &dirs, roaming.as_deref())
}

/// Agent `id`'s program in `dirs` (the first folder that has one of its forms), else, for OpenCode, OpenCode
/// Desktop's CLI in `roaming` ([`desktop_cli`]).
fn find(id: AgentId, dirs: &[PathBuf], roaming: Option<&Path>) -> Option<PathBuf> {
    let desktop = || roaming.filter(|_| id == AgentId::OpenCode).and_then(desktop_cli);
    dirs.iter().find_map(|d| program_in(d, id.id())).or_else(desktop)
}

/// [`discover`] with each agent's program from `program`, and the OpenCode server `announced` gives when OpenCode's
/// program is not found, asking each version in that agent's empty folder in `home`, CUE's agent folder
/// ([`super::empty_cwd`]). An OpenCode found only by its server has no program and the version the server announces.
pub(crate) fn discover_in(
    program: &dyn Fn(AgentId) -> Option<PathBuf>,
    announced: &dyn Fn() -> Option<Announced>,
    home: &Path,
) -> Vec<Found> {
    let found: Vec<(AgentId, Option<PathBuf>, Option<String>)> = ORDER
        .iter()
        .filter_map(|&id| match program(id) {
            Some(exe) => Some((id, Some(exe), None)),
            None if id == AgentId::OpenCode => {
                announced().map(|a| (id, None, a.version.as_deref().and_then(version_of)))
            }
            None => None,
        })
        .collect();
    // Asked together: an npm shim starts Node, which takes a moment.
    std::thread::scope(|s| {
        let asks: Vec<_> = found
            .into_iter()
            .map(|(id, exe, announced)| {
                s.spawn(move || {
                    let Some(program) = exe.clone() else { return Found { id, exe, version: announced } };
                    let version = super::empty_cwd(home, id)
                        .and_then(|cwd| process::run(&program, &["--version".to_string()], "", &cwd, VERSION_TIMEOUT))
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

/// OpenCode Desktop's CLI: `<roaming>\ai.opencode.desktop\cli\<version>-<arch>\opencode-cli.exe`, from the folder of
/// the highest version that has it (a release above a pre-release of the same numbers). Nothing else there is ever run.
pub(crate) fn desktop_cli(roaming: &Path) -> Option<PathBuf> {
    let root = roaming.join(DESKTOP_DIR).join("cli");
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let key = desktop_version(e.file_name().to_str()?)?;
            let exe = e.path().join(DESKTOP_CLI);
            exe.is_file().then_some((key, exe))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, exe)| exe)
}

/// The order of a Desktop CLI folder `<version>-<arch>` (`2.2.5-x64`, `2.3.0-beta.1-arm64`): its version's numbers,
/// then whether it is a release. None for a name that is not one.
fn desktop_version(folder: &str) -> Option<(Vec<u64>, bool)> {
    let (version, arch) = folder.rsplit_once('-')?;
    if arch.is_empty() || !arch.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let (numbers, pre) = match version.split_once('-') {
        Some((n, pre)) => (n, Some(pre)),
        None => (version, None),
    };
    let numbers: Vec<u64> = numbers.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
    (numbers.len() >= 2).then_some((numbers, pre.is_none()))
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

/// `<dir>\<name>.exe`, `.cmd` or `.bat`, the first that is a file.
fn program_in(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|p| p.is_file())
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

    /// Each agent's program in `dirs` only.
    fn in_dirs(dirs: &[PathBuf]) -> impl Fn(AgentId) -> Option<PathBuf> + use<> {
        let dirs = dirs.to_vec();
        move |id| find(id, &dirs, None)
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
        let found = discover_in(&in_dirs(std::slice::from_ref(&bin)), &|| None, &base.join("agent"));
        assert_eq!(
            found,
            vec![Found { id: AgentId::Codex, exe: Some(bin.join("codex.cmd")), version: Some("0.166.0".into()) }]
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
        assert!(discover_in(&in_dirs(std::slice::from_ref(&bin)), &|| None, &base.join("agent")).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn discover_order_is_opencode_then_codex() {
        let base = temp_dir("order");
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // Written in the opposite order, with Claude Code and Gemini beside them: never offered.
        for name in ["grok", "codex", "opencode", "claude", "gemini"] {
            shim(&bin, &format!("{name}.cmd"), &format!("{name} 1.2.3"));
        }
        let found = discover_in(&in_dirs(std::slice::from_ref(&bin)), &|| None, &base.join("agent"));
        let ids: Vec<AgentId> = found.iter().map(|f| f.id).collect();
        assert_eq!(ids, ORDER.to_vec());
        assert!(found.iter().all(|f| f.version.as_deref() == Some("1.2.3")));
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
        let found = discover_in(&in_dirs(&[a.clone(), b.clone()]), &|| None, &base.join("agent"));
        assert_eq!(found[0].exe, Some(a.join("codex.cmd")));
        assert_eq!(super::program_in(&a, "codex"), Some(a.join("codex.cmd")));
        std::fs::write(a.join("codex.exe"), b"MZ").unwrap();
        assert_eq!(super::program_in(&a, "codex"), Some(a.join("codex.exe")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// OpenCode Desktop's CLI: the highest version folder that holds `opencode-cli.exe`; nothing else there.
    #[test]
    fn desktop_cli_is_the_highest_version_folder() {
        let base = temp_dir("desktop");
        let cli = base.join("ai.opencode.desktop").join("cli");
        assert_eq!(desktop_cli(&base), None, "no Desktop app");
        for folder in ["2.2.5-x64", "2.10.0-x64", "2.10.0-beta.1-x64", "3.0.0-x64", "9.9.9", "latest-x64", "2.11-x64"] {
            std::fs::create_dir_all(cli.join(folder)).unwrap();
        }
        for folder in ["2.2.5-x64", "2.10.0-x64", "2.10.0-beta.1-x64", "9.9.9", "latest-x64"] {
            std::fs::write(cli.join(folder).join("opencode-cli.exe"), b"MZ").unwrap();
        }
        // 3.0.0 has no CLI in it and 2.11 holds another program: passed over. "9.9.9" names no arch, "latest" no
        // version.
        std::fs::write(cli.join("2.11-x64").join("opencode.exe"), b"MZ").unwrap();
        assert_eq!(desktop_cli(&base), Some(cli.join("2.10.0-x64").join("opencode-cli.exe")));
        // A release above its own pre-release, numbers compared as numbers.
        assert_eq!(desktop_version("2.10.0-x64"), Some((vec![2, 10, 0], true)));
        assert_eq!(desktop_version("2.10.0-beta.1-arm64"), Some((vec![2, 10, 0], false)));
        assert!(desktop_version("2.10.0-beta.1-x64") < desktop_version("2.10.0-x64"));
        assert!(desktop_version("2.9.9-x64") < desktop_version("2.10.0-x64"));
        assert_eq!(desktop_version("x64"), None);
        assert_eq!(desktop_version("2-x64"), None);
        // Only OpenCode is looked for there, and only after its usual folders.
        assert_eq!(find(AgentId::OpenCode, &[], Some(&base)), Some(cli.join("2.10.0-x64").join("opencode-cli.exe")));
        assert_eq!(find(AgentId::Codex, &[], Some(&base)), None);
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        shim(&bin, "opencode.cmd", "opencode v2.2.5");
        assert_eq!(find(AgentId::OpenCode, std::slice::from_ref(&bin), Some(&base)), Some(bin.join("opencode.cmd")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// An OpenCode with no program CUE can find counts when a server it runs announces itself, with the version it
    /// announces and no program: CUE uses that server and never starts one.
    #[test]
    fn opencode_counts_when_its_server_is_announced() {
        let base = temp_dir("announced");
        let url: reqwest::Url = "http://127.0.0.1:28894/".parse().unwrap();
        let announced = || Some(Announced { url: url.clone(), version: Some("2.2.5".into()) });
        let found = discover_in(&|_| None, &announced, &base.join("agent"));
        assert_eq!(found, vec![Found { id: AgentId::OpenCode, exe: None, version: Some("2.2.5".into()) }]);
        // Nothing announced, nothing found.
        assert!(discover_in(&|_| None, &|| None, &base.join("agent")).is_empty());
        // A program found wins: the server is not asked about, and the version is the program's own.
        let bin = base.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        shim(&bin, "opencode.cmd", "opencode v2.2.6");
        let asked = std::sync::atomic::AtomicBool::new(false);
        let announced = || {
            asked.store(true, std::sync::atomic::Ordering::SeqCst);
            None
        };
        let found = discover_in(&in_dirs(std::slice::from_ref(&bin)), &announced, &base.join("agent"));
        assert_eq!(found, vec![Found { id: AgentId::OpenCode, exe: Some(bin.join("opencode.cmd")), version: Some("2.2.6".into()) }]);
        assert!(!asked.load(std::sync::atomic::Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The Desktop CLI found is the program, its version from `--version` (a program that does not run has none).
    #[test]
    fn desktop_cli_is_found_as_opencode() {
        let base = temp_dir("desktop-found");
        let dir = base.join("ai.opencode.desktop").join("cli").join("2.2.5-x64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("opencode-cli.exe"), b"MZ").unwrap();
        let roaming = base.clone();
        let found = discover_in(&move |id| find(id, &[], Some(&roaming)), &|| None, &base.join("agent"));
        assert_eq!(found, vec![Found { id: AgentId::OpenCode, exe: Some(dir.join("opencode-cli.exe")), version: None }]);
        assert_eq!(super::version_of("opencode v2.2.5\n"), Some("2.2.5".into()));
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
