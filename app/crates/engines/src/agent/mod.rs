//! The agent path (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): finding the user's own OpenCode, Codex or
//! Grok on this computer and running them safely. CUE holds no key: each agent uses the user's own sign-in.
//!
//! - [`discover`]: which agents are installed, in [`cue_core::agents::ORDER`].
//! - [`process::run`]: one run of an agent's CLI with no window, the text on stdin, an empty working folder and a
//!   timeout that ends the whole process tree.
//! - [`cli`]: the flags that leave Codex and Grok no shell, no file reading and no network tool, and reading their
//!   answers.
//! - [`pipe`]: the one-shot named pipe Grok reads the text from (it never reads stdin).
//! - [`opencode`]: the user's own OpenCode server, or one CUE starts on 127.0.0.1, over loopback HTTP.
//! - [`AgentClient`]: one answer from either; [`refine`]: a cleanup or a translation through it.
pub mod cli;
pub mod discover;
pub mod opencode;
pub mod pipe;
pub mod process;
pub mod refine;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use cue_core::agents::AgentId;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use discover::discover;

/// An agent installed on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: AgentId,
    /// The program CUE starts: an `.exe`, or an npm `.cmd` shim (run through `cmd.exe`).
    pub exe: PathBuf,
    /// As the agent's `--version` says it (`0.166.0`), when it answered.
    pub version: Option<String>,
}

/// Why an agent gave no answer. Each maps to the fallback the user chose and to a word on the Drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentFailure {
    NotInstalled,
    /// Not signed in, the sign-in expired, or no access (the CLI's own message).
    SignedOut,
    Timeout,
    /// Anything else, in a word for the log (never the text).
    Failed(&'static str),
}

/// One request to an agent: CUE's system text, the user message (the transcript and context), and whether Ask may
/// search the web.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub system: String,
    pub user: String,
    pub web_search: bool,
}

/// A model an agent offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgentModel {
    pub id: String,
    pub name: String,
}

/// `<home>\<agent id>`: an agent's own working folder inside CUE's agent folder (`home`, `%LOCALAPPDATA%\CUE\agent`),
/// created empty by whoever runs the agent. So a running OpenCode never blocks Codex, and what CUE writes for an
/// agent (Codex's instructions and schema, in `home`) is in no working folder.
pub fn cwd(home: &Path, id: AgentId) -> PathBuf {
    home.join(id.id())
}

/// The most folders [`empty_cwd`] tries beside an agent's own.
const SPARE_FOLDERS: usize = 64;

/// An empty working folder for agent `id`: its own [`cwd`] while that is empty (created when missing), else the first of
/// `<id>-1`, `<id>-2`, … beside it that is absent (created empty) or empty, reused while it stays empty. Nothing in any
/// of them is ever deleted: a stray file in the agent's folder (an `AGENTS.md` with orders, say) never reaches the
/// agent, and never stops it either. The first move to a spare folder writes one warning naming the agent, never a
/// path. `Failed("cwd")` when none can be had.
pub fn empty_cwd(home: &Path, id: AgentId) -> Result<PathBuf, AgentFailure> {
    static WARNED: Mutex<Vec<(AgentId, PathBuf)>> = Mutex::new(Vec::new());
    let own = cwd(home, id);
    let candidates = std::iter::once(own.clone()).chain((1..=SPARE_FOLDERS).map(|n| home.join(format!("{}-{n}", id.id()))));
    for dir in candidates {
        let usable = match std::fs::read_dir(&dir) {
            Ok(mut entries) => entries.next().is_none(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir_all(&dir).is_ok(),
            Err(_) => false,
        };
        if !usable {
            continue;
        }
        if dir != own {
            let mut warned = WARNED.lock().unwrap_or_else(PoisonError::into_inner);
            if !warned.iter().any(|(a, d)| *a == id && *d == dir) {
                warned.push((id, dir.clone()));
                tracing::warn!(agent = id.id(), "agent: folder not empty, runs in a spare empty folder beside it");
            }
        }
        return Ok(dir);
    }
    tracing::warn!(agent = id.id(), "agent: no empty folder left for the agent, it is not run");
    Err(AgentFailure::Failed("cwd"))
}

/// The files Codex reads CUE's fixed instructions from are `codex-instructions-<hash>.md` in CUE's agent folder
/// ([`codex_instructions`]): one per text, so a cleanup and a translation (or Ask) never share one, and no file
/// changes under a running Codex.
pub const CODEX_INSTRUCTIONS: &str = "codex-instructions";
/// The file that holds Codex's answer to `{"text": …}`, in CUE's agent folder.
pub const CODEX_SCHEMA: &str = "codex-schema.json";
/// The answer CUE asks Codex and Grok for, as the cloud path asks for it.
pub(crate) const ANSWER_SCHEMA: &str =
    r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}"#;

/// `<home>\codex-instructions-<the first 16 hex digits of the text's SHA-256>.md`: the file for `text`.
pub fn codex_instructions(home: &Path, text: &str) -> PathBuf {
    let digest = Sha256::digest(text.as_bytes());
    let hash = digest.iter().take(8).fold(String::with_capacity(16), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    });
    home.join(format!("{CODEX_INSTRUCTIONS}-{hash}.md"))
}

/// How CUE asks the user's agent: OpenCode's server, or a CLI run once per request.
#[derive(Clone, Debug)]
pub enum AgentClient {
    OpenCode(Arc<opencode::Server>),
    /// Codex. `home` is CUE's agent folder (`%LOCALAPPDATA%\CUE\agent`): the agent runs in its [`cwd`] there, and
    /// Codex's instructions and schema files are written beside that folder.
    Cli { found: Found, home: PathBuf },
    /// Grok, run in its [`cwd`] in `home` as Codex is; `data` is Grok's own folder ([`grok_home`]), where it keeps the
    /// sessions CUE deletes after each run (`None`: not known, only `grok sessions delete` runs).
    Grok { found: Found, home: PathBuf, data: Option<PathBuf> },
}

impl AgentClient {
    pub fn id(&self) -> AgentId {
        match self {
            AgentClient::OpenCode(_) => AgentId::OpenCode,
            AgentClient::Cli { found, .. } | AgentClient::Grok { found, .. } => found.id,
        }
    }

    /// One answer to `p` from `model` (empty: the agent's default), within `timeout`, with no tools. The answer comes
    /// whole: `on_text` gets it once.
    ///
    /// For Codex `p.system` is written to its instructions file, and for Grok it is an argument, so it must hold only
    /// CUE's fixed text and the user's instructions file, never anything of a dictation; `p.user` goes on Codex's stdin
    /// or through Grok's pipe.
    pub async fn complete(
        &self,
        model: &str,
        p: &Prompt,
        timeout: Duration,
        on_text: Option<&mut (dyn FnMut(&str) + Send)>,
    ) -> Result<String, AgentFailure> {
        let text = match self {
            AgentClient::OpenCode(server) => return server.complete(model, p, timeout, on_text).await,
            AgentClient::Cli { found, home } => match found.id {
                AgentId::Codex => codex(found, home, model, p, timeout).await?,
                AgentId::Grok => return Err(AgentFailure::Failed("grok runs as AgentClient::Grok")),
                AgentId::OpenCode => return Err(AgentFailure::Failed("opencode is a server")),
            },
            AgentClient::Grok { found, home, data } => grok(found, home, data.as_deref(), model, p, timeout).await?,
        };
        if let Some(on_text) = on_text {
            on_text(&text);
        }
        Ok(text)
    }
}

/// What Codex says of its sign-in when asked (`codex login status`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Login {
    SignedIn,
    SignedOut,
    /// It said something else, or did not answer in time: a run tells.
    Unknown,
}

/// How long `codex login status` may take; it answers in under a second.
pub const LOGIN_WAIT: Duration = Duration::from_secs(5);

/// Codex's own word on its sign-in, asked with no text at all: `codex login status` in its empty working folder. A
/// Codex never signed in retries its requests for about 15 s before it says so, past CUE's budget, so this is asked
/// first; but it says "Logged in" for an expired sign-in too, which only a run tells (`cli::signed_out`). A missing
/// program is `NotInstalled`.
pub async fn codex_login(found: &Found, home: &Path) -> Result<Login, AgentFailure> {
    let (exe, cwd) = (found.exe.clone(), empty_cwd(home, AgentId::Codex)?);
    let args = cli::codex_login_args();
    let out = tokio::task::spawn_blocking(move || process::run(&exe, &args, "", &cwd, LOGIN_WAIT))
        .await
        .map_err(|_| AgentFailure::Failed("join"));
    match out? {
        Ok(out) => Ok(cli::codex_login(&out)),
        Err(AgentFailure::NotInstalled) => Err(AgentFailure::NotInstalled),
        Err(_) => Ok(Login::Unknown),
    }
}

/// Sets its flag when dropped: a request whose future is dropped (Esc, a newer request) ends the process it started.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// One `codex exec`: CUE's fixed instructions and the answer's schema in files beside its working folder, the rest of
/// the request on stdin, run off the async threads. A run that failed is never read for an answer: a half-written last
/// message is not a cleanup. Dropping the request (Esc) ends the run and every process it started.
async fn codex(found: &Found, home: &Path, model: &str, p: &Prompt, timeout: Duration) -> Result<String, AgentFailure> {
    let instructions = codex_instructions(home, &p.system);
    let schema = home.join(CODEX_SCHEMA);
    // Named after its text: a file that is there holds it.
    if !instructions.is_file() {
        write_whole(&instructions, &p.system)?;
    }
    if !std::fs::read(&schema).is_ok_and(|held| held == ANSWER_SCHEMA.as_bytes()) {
        write_whole(&schema, ANSWER_SCHEMA)?;
    }
    let args = cli::codex_args(model, &instructions, &schema);
    let (exe, stdin, cwd) = (found.exe.clone(), p.user.clone(), empty_cwd(home, AgentId::Codex)?);
    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = CancelOnDrop(cancel.clone());
    let out = tokio::task::spawn_blocking(move || process::run_until(&exe, &args, &stdin, &cwd, timeout, &cancel))
        .await
        .map_err(|_| AgentFailure::Failed("join"))??;
    if cli::signed_out(AgentId::Codex, &out) {
        return Err(AgentFailure::SignedOut);
    }
    if out.status != Some(0) {
        return Err(AgentFailure::Failed("exit"));
    }
    cli::codex_text(&out.stdout).filter(|t| !t.trim().is_empty()).ok_or(AgentFailure::Failed("no answer"))
}

/// How long `grok sessions delete` may take; it takes under a second.
pub const GROK_FORGET_WAIT: Duration = Duration::from_secs(5);
/// The file beside Grok's sessions of one working folder that lists the prompts run there.
pub const GROK_PROMPT_HISTORY: &str = "prompt_history.jsonl";

/// Grok's own folder, as Grok's docs name it: `GROK_HOME` when set, else `%USERPROFILE%\.grok`.
pub fn grok_home() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    var("GROK_HOME").or_else(|| var("USERPROFILE").map(|h| h.join(".grok")))
}

/// One `grok` headless run: the pipe made first, then Grok started with CUE's fixed text as its system prompt and the
/// pipe as its prompt file, then the rest of the request written through the pipe once Grok opens it (within the
/// timeout). Off the async threads; dropping the request (Esc) ends the run and every process it started. A Grok
/// that exits before it opens the pipe is judged by what it said (signed out, …), never as a pipe timeout. Whatever
/// came of it, the session Grok keeps of the run is deleted afterwards ([`GrokRun::forget`]), on a thread of its own.
async fn grok(
    found: &Found,
    home: &Path,
    data: Option<&Path>,
    model: &str,
    p: &Prompt,
    timeout: Duration,
) -> Result<String, AgentFailure> {
    let cwd = empty_cwd(home, AgentId::Grok)?;
    // The first Grok run after CUE starts also deletes what an earlier CUE left (it quit or crashed before a run's
    // session was deleted): sessions made before this moment only, so never this run's, on a thread of its own.
    if let Some(data) = data
        && !GROK_SWEPT.swap(true, Ordering::Relaxed)
    {
        let (exe, home, data, before) = (found.exe.clone(), home.to_path_buf(), data.to_path_buf(), SystemTime::now());
        std::thread::spawn(move || sweep_grok_sessions(&exe, &home, &data, before));
    }
    let pipe = pipe::OneShot::create()?;
    let session = session_id()?;
    let args = cli::grok_args(model, p.web_search, &p.system, &pipe.path(), &session);
    let run = GrokRun {
        exe: found.exe.clone(),
        cwd,
        home: home.to_path_buf(),
        data: data.map(Path::to_path_buf),
        session,
    };
    let text = p.user.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    let _stop = CancelOnDrop(cancel.clone());
    // This runs to its end even when the request is dropped: the flag ends Grok, then the session goes.
    let (out, sent) = tokio::task::spawn_blocking(move || {
        let ran = run.ask(pipe, &args, &text, timeout, &cancel);
        // A Grok that never started (missing, or its folder or job refused) kept nothing.
        let started = !matches!(ran, Err(AgentFailure::NotInstalled | AgentFailure::Failed("start" | "cwd" | "job")));
        if started {
            std::thread::spawn(move || run.forget());
        }
        ran
    })
    .await
    .map_err(|_| AgentFailure::Failed("join"))??;
    grok_answer(&out, sent)
}

/// What a finished Grok run gave: signed out, a turn cut because the model reached for a tool (never pasted), a failed
/// run, a run that did not get the whole text, or the answer.
fn grok_answer(out: &process::Output, sent: Result<(), AgentFailure>) -> Result<String, AgentFailure> {
    if sent == Err(AgentFailure::Failed("pipe client")) {
        tracing::warn!(agent = "grok", "agent: another process opened grok's pipe; nothing was written to it");
        return Err(AgentFailure::Failed("pipe client"));
    }
    if cli::signed_out(AgentId::Grok, out) {
        return Err(AgentFailure::SignedOut);
    }
    if cli::grok_tried_a_tool(&out.stdout) {
        return Err(AgentFailure::Failed("tried a tool"));
    }
    if out.status != Some(0) {
        return Err(AgentFailure::Failed("exit"));
    }
    if sent.is_err() {
        return Err(AgentFailure::Failed("pipe"));
    }
    cli::grok_text(&out.stdout).filter(|t| !t.trim().is_empty()).ok_or(AgentFailure::Failed("no answer"))
}

/// A Grok run's program, its working folder in CUE's agent folder `home`, Grok's own folder and the session id CUE
/// gave the run.
struct GrokRun {
    exe: PathBuf,
    cwd: PathBuf,
    home: PathBuf,
    data: Option<PathBuf>,
    session: String,
}

impl GrokRun {
    /// Starts Grok, hands it `text` through `pipe` while it runs, and waits for it: what it printed, and whether the
    /// text went through whole. Only the process started here may read it (the pipe checks the client's process id);
    /// the pipe gives up as soon as Grok has ended.
    fn ask(
        &self,
        pipe: pipe::OneShot,
        args: &[String],
        text: &str,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<(process::Output, Result<(), AgentFailure>), AgentFailure> {
        let ended = Arc::new(AtomicBool::new(false));
        let grok = Arc::new(AtomicU32::new(0));
        let (flag, pid, text) = (ended.clone(), grok.clone(), text.to_owned());
        let sender = std::thread::spawn(move || pipe.send_until(&text, timeout, &flag, &pid));
        let out = process::run_env_until(&self.exe, args, &cli::GROK_ENV, "", &self.cwd, timeout, cancel, Some(&grok));
        ended.store(true, Ordering::Relaxed);
        let sent = sender.join().unwrap_or(Err(AgentFailure::Failed("pipe")));
        Ok((out?, sent))
    }

    /// Deletes what Grok kept of the run: `grok sessions delete <id>` (the session's folder and the text in it),
    /// then the one prompt-history file Grok keeps beside the sessions of the run's working folder. Nothing else in
    /// Grok's folder is touched. CUE chose the session id (`--session-id`), so a run cut by a timeout or Esc is deleted
    /// too; one cut before Grok made its session has none, and `sessions delete` finds nothing. The log says neither
    /// the id nor a path.
    fn forget(&self) {
        if !grok_delete(&self.exe, &self.home, &self.session) {
            tracing::info!(agent = "grok", "agent: grok deleted no session (none was made, or it did not answer)");
        }
        if let Some(dir) = self.data.as_deref().and_then(|d| grok_sessions_of(d, &self.home, &self.cwd)) {
            remove_prompt_history(&dir);
        }
    }
}

/// Whether this CUE has looked for the sessions an earlier one left ([`sweep_grok_sessions`]).
static GROK_SWEPT: AtomicBool = AtomicBool::new(false);

/// `grok sessions delete <id>`, whether Grok said it is done. It carries no text, so it runs in an empty folder of
/// its own ([`empty_cwd`]), whatever has turned up in the run's folder since.
fn grok_delete(exe: &Path, home: &Path, id: &str) -> bool {
    let Ok(cwd) = empty_cwd(home, AgentId::Grok) else { return false };
    let none = AtomicBool::new(false);
    let args = cli::grok_delete_args(id);
    process::run_env_until(exe, &args, &cli::GROK_ENV, "", &cwd, GROK_FORGET_WAIT, &none, None)
        .is_ok_and(|o| o.status == Some(0))
}

/// Removes the prompt history in `dir`, Grok's sessions folder of one of CUE's working folders.
fn remove_prompt_history(dir: &Path) {
    match std::fs::remove_file(dir.join(GROK_PROMPT_HISTORY)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => tracing::warn!(agent = "grok", "agent: grok's prompt history was not removed"),
    }
}

/// Deletes what earlier Grok runs left in Grok's own folder `data` (CUE quit or crashed before a run's
/// [`GrokRun::forget`]): in the sessions folder of each of CUE's Grok working folders in `home` (`grok`, `grok-1` …,
/// found as [`grok_sessions_of`] finds them), every session folder made before `before`, through
/// `grok sessions delete <id>`, and the prompt history. A folder whose name is not a session id is left alone, and
/// nothing else in Grok's folder is touched. How many sessions it deleted.
pub fn sweep_grok_sessions(exe: &Path, home: &Path, data: &Path, before: SystemTime) -> usize {
    let mut deleted = 0;
    let folders = std::iter::once(cwd(home, AgentId::Grok))
        .chain((1..=SPARE_FOLDERS).map(|n| home.join(format!("{}-{n}", AgentId::Grok.id()))));
    for dir in folders.filter_map(|f| grok_sessions_of(data, home, &f)) {
        let old = |e: &std::fs::DirEntry| e.metadata().and_then(|m| m.created().or_else(|_| m.modified())).is_ok_and(|t| t < before);
        let ids: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && old(e))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.is_empty() && n.len() <= 64 && n.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
            .collect();
        for id in ids {
            if grok_delete(exe, home, &id) {
                deleted += 1;
            }
        }
        remove_prompt_history(&dir);
    }
    if deleted > 0 {
        tracing::info!(agent = "grok", deleted, "agent: deleted the grok sessions an earlier run left");
    }
    deleted
}

/// A new random session id for Grok (a UUID, version 4, in lower case).
fn session_id() -> Result<String, AgentFailure> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|_| AgentFailure::Failed("random"))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = b.iter().fold(String::with_capacity(32), |mut s, x| {
        let _ = write!(s, "{x:02x}");
        s
    });
    Ok(format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]))
}

/// `path` as Grok names a sessions folder after its working folder: percent-encoded whole (UTF-8 bytes, upper-case
/// hex), only letters, digits and `-_.~` left as they are (`C:\Users\…` → `C%3A%5CUsers%5C…`).
fn grok_encode(path: &Path) -> String {
    let mut s = String::new();
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            s.push(char::from(b));
        } else {
            let _ = write!(s, "%{b:02X}");
        }
    }
    s
}

/// A folder name percent-decoded; `None` when it is not valid UTF-8 or not percent-encoding.
fn grok_decode(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The folder in Grok's own folder `data` that holds the sessions of working folder `cwd`: `sessions\<cwd encoded>`,
/// else the one whose name decodes to `cwd` or whose `.cwd` file names it (Grok's form for a path too long to encode).
/// Only for a working folder inside CUE's agent folder `home`, so it holds CUE's runs alone.
fn grok_sessions_of(data: &Path, home: &Path, cwd: &Path) -> Option<PathBuf> {
    if cwd == home || !cwd.starts_with(home) {
        return None;
    }
    let sessions = data.join("sessions");
    // The folder as CUE named it, and its full form (long names, not `SEYMUR~1`), as Grok may name it.
    let mut forms = vec![cwd.to_string_lossy().into_owned()];
    if let Ok(full) = std::fs::canonicalize(cwd) {
        let full = full.to_string_lossy().into_owned();
        forms.push(full.strip_prefix(r"\\?\").map(String::from).unwrap_or(full));
    }
    for form in &forms {
        let named = sessions.join(grok_encode(Path::new(form)));
        if named.is_dir() {
            return Some(named);
        }
    }
    let same = |s: &str| forms.iter().any(|f| s.trim().eq_ignore_ascii_case(f));
    std::fs::read_dir(&sessions).ok()?.flatten().map(|e| e.path()).find(|dir| {
        dir.is_dir()
            && (dir.file_name().and_then(|n| grok_decode(&n.to_string_lossy())).is_some_and(|n| same(&n))
                || std::fs::read_to_string(dir.join(".cwd")).is_ok_and(|c| same(&c)))
    })
}

/// Writes `text` to `path` whole: under a temporary name in the same folder, then renamed into place, so a reader never
/// sees half of it. A rename that fails while `path` holds `text` (another request wrote it meanwhile) is fine.
fn write_whole(path: &Path, text: &str) -> Result<(), AgentFailure> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or(AgentFailure::Failed("files"))?;
    std::fs::create_dir_all(dir).map_err(|_| AgentFailure::Failed("files"))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    if std::fs::write(&temp, text).and_then(|()| std::fs::rename(&temp, path)).is_err() {
        let _ = std::fs::remove_file(&temp);
        if !std::fs::read(path).is_ok_and(|held| held == text.as_bytes()) {
            return Err(AgentFailure::Failed("files"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-agent-client-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn each_agent_works_in_its_own_folder() {
        let home = Path::new(r"C:\U\AppData\Local\CUE\agent");
        assert_eq!(cwd(home, AgentId::OpenCode), Path::new(r"C:\U\AppData\Local\CUE\agent\opencode"));
        assert_eq!(cwd(home, AgentId::Codex), Path::new(r"C:\U\AppData\Local\CUE\agent\codex"));
    }

    /// Grok names a sessions folder after its working folder, percent-encoded whole (fixtures README, "Signed-in
    /// run"), and CUE finds it that way, by the decoded name, or by a `.cwd` file; never for a folder outside CUE's.
    #[test]
    fn grok_sessions_folder_is_named_after_the_working_folder() {
        let seen = r"C:\Users\seymurglv\AppData\Local\Temp\claude\220d5e011641d0ef\53ca95ca-54cd-404a-8ac1-b2e6074a88e9\scratchpad\codexprobe2\g1";
        assert_eq!(
            grok_encode(Path::new(seen)),
            "C%3A%5CUsers%5Cseymurglv%5CAppData%5CLocal%5CTemp%5Cclaude%5C220d5e011641d0ef%5C53ca95ca-54cd-404a-8ac1-b2e6074a88e9%5Cscratchpad%5Ccodexprobe2%5Cg1"
        );
        let odd = Path::new(r"C:\Users\Пользователь Тест\AppData\Local\CUE\agent\grok-1");
        assert_eq!(grok_decode(&grok_encode(odd)).as_deref(), Some(odd.to_str().unwrap()));
        assert_eq!(grok_decode("C%3"), None);

        let dir = temp_dir("sessions-of");
        let (home, data) = (dir.join("CUE").join("agent"), dir.join(".grok"));
        let cwd = home.join("grok");
        assert_eq!(grok_sessions_of(&data, &home, &cwd), None, "no folder yet");
        let named = data.join("sessions").join(grok_encode(&cwd));
        std::fs::create_dir_all(&named).unwrap();
        assert_eq!(grok_sessions_of(&data, &home, &cwd), Some(named.clone()));
        // Grok's form for a long path: a slug and a `.cwd` file naming the folder.
        let spare = home.join("grok-1");
        let long = data.join("sessions").join("grok-1-0f3a");
        std::fs::create_dir_all(&long).unwrap();
        std::fs::write(long.join(".cwd"), spare.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(grok_sessions_of(&data, &home, &spare), Some(long));
        // Never CUE's agent folder itself, nor a folder outside it.
        assert_eq!(grok_sessions_of(&data, &home, &home), None);
        assert_eq!(grok_sessions_of(&data, &home, &dir.join("elsewhere")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grok_session_ids_are_random_uuids() {
        let (a, b) = (session_id().unwrap(), session_id().unwrap());
        assert_ne!(a, b);
        for id in [a, b] {
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(parts.iter().map(|p| p.len()).collect::<Vec<_>>(), [8, 4, 4, 4, 12], "{id}");
            assert!(parts[2].starts_with('4') && id.chars().all(|c| c == '-' || c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        }
    }

    /// A Cli client never runs Grok: Grok has its own, with its pipe and its own folder.
    #[tokio::test]
    async fn grok_is_not_run_as_a_codex_client() {
        let home = temp_dir("grok");
        let found = Found { id: AgentId::Grok, exe: home.join("grok.exe"), version: None };
        let c = AgentClient::Cli { found, home: home.clone() };
        let p = Prompt { system: "Clean.".into(), user: "<transcript>hi</transcript>".into(), web_search: false };
        let r = c.complete("", &p, Duration::from_secs(5), None).await;
        assert_eq!(r, Err(AgentFailure::Failed("grok runs as AgentClient::Grok")));
        assert!(!home.exists(), "nothing was written or run");
    }

    /// The cap of spare folders: no folder, no run.
    #[test]
    fn no_empty_folder_left_is_failed_cwd() {
        let home = temp_dir("cap");
        std::fs::create_dir_all(&home).unwrap();
        for name in std::iter::once("grok".to_string()).chain((1..=SPARE_FOLDERS).map(|n| format!("grok-{n}"))) {
            std::fs::write(home.join(name), "a file").unwrap();
        }
        assert_eq!(empty_cwd(&home, AgentId::Grok), Err(AgentFailure::Failed("cwd")));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A stray file in the agent's folder: the next empty folder beside it, created empty and reused; nothing deleted.
    #[test]
    fn a_folder_that_is_not_empty_is_left_for_an_empty_one_beside_it() {
        let home = temp_dir("spare");
        assert_eq!(empty_cwd(&home, AgentId::Codex).unwrap(), home.join("codex"), "created empty");
        assert!(home.join("codex").is_dir());
        std::fs::write(home.join("codex").join("AGENTS.md"), "orders").unwrap();
        assert_eq!(empty_cwd(&home, AgentId::Codex).unwrap(), home.join("codex-1"));
        assert_eq!(empty_cwd(&home, AgentId::Codex).unwrap(), home.join("codex-1"), "reused while empty");
        std::fs::write(home.join("codex-1").join("x.txt"), "x").unwrap();
        // A file where a folder would be is passed over too.
        std::fs::write(home.join("codex-2"), "a file").unwrap();
        assert_eq!(empty_cwd(&home, AgentId::Codex).unwrap(), home.join("codex-3"));
        assert_eq!(std::fs::read_to_string(home.join("codex").join("AGENTS.md")).unwrap(), "orders", "nothing deleted");
        assert_eq!(empty_cwd(&home, AgentId::OpenCode).unwrap(), home.join("opencode"), "each agent its own");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn each_text_has_its_own_instructions_file() {
        let home = Path::new(r"C:\U\AppData\Local\CUE\agent");
        let clean = codex_instructions(home, "Clean.\n\nBold.");
        let translate = codex_instructions(home, "Clean.");
        assert_ne!(clean, translate, "a cleanup and a translation never share a file");
        assert_eq!(clean, codex_instructions(home, "Clean.\n\nBold."));
        assert_eq!(clean.parent(), Some(home));
        let name = clean.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("codex-instructions-") && name.ends_with(".md") && name.len() == 19 + 16 + 3, "{name}");
    }

    #[tokio::test]
    async fn an_instructions_file_that_is_there_is_not_rewritten() {
        let home = temp_dir("files");
        let p = Prompt { system: "Clean.".into(), user: "<transcript>hi</transcript>".into(), web_search: false };
        let file = codex_instructions(&home, &p.system);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(&file, "already here").unwrap();
        // Codex itself is missing: the run fails once the files are ready.
        let found = Found { id: AgentId::Codex, exe: home.join("codex.exe"), version: None };
        let c = AgentClient::Cli { found, home: home.clone() };
        assert_eq!(c.complete("", &p, Duration::from_secs(5), None).await, Err(AgentFailure::NotInstalled));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "already here");
        assert_eq!(std::fs::read_to_string(home.join(CODEX_SCHEMA)).unwrap(), ANSWER_SCHEMA);
        // Another text gets a file of its own, written whole; no temporary file stays behind.
        let other = Prompt { system: "Translate.".into(), ..p.clone() };
        assert_eq!(c.complete("", &other, Duration::from_secs(5), None).await, Err(AgentFailure::NotInstalled));
        assert_eq!(std::fs::read_to_string(codex_instructions(&home, "Translate.")).unwrap(), "Translate.");
        let left: Vec<String> =
            std::fs::read_dir(&home).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(!left.iter().any(|n| n.ends_with(".tmp")), "{left:?}");
        let _ = std::fs::remove_dir_all(&home);
    }
}
