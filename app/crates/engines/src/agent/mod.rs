//! The agent path (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): finding the user's own OpenCode, Codex or
//! Grok on this computer and running them safely. CUE holds no key: each agent uses the user's own sign-in.
//!
//! - [`discover`]: which agents are installed, in [`cue_core::agents::ORDER`].
//! - [`process::run`]: one run of an agent's CLI with no window, the text on stdin, an empty working folder and a
//!   timeout that ends the whole process tree.
//! - [`cli`]: the flags that leave Codex and Grok no shell, no file reading and no network tool, and reading their
//!   answers.
//! - [`opencode`]: the user's own OpenCode server, or one CUE starts on 127.0.0.1, over loopback HTTP.
//! - [`AgentClient`]: one answer from either; [`refine`]: a cleanup or a translation through it; [`ask`]: an "Ask"
//!   question through it, or through the model on this computer.
pub mod ask;
pub mod cli;
pub mod discover;
pub mod opencode;
pub mod process;
pub mod refine;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cue_core::agents::AgentId;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub use discover::discover;

/// An agent installed on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: AgentId,
    /// The program CUE starts: an `.exe`, or an npm `.cmd` shim (run through `cmd.exe`). None for an OpenCode found
    /// only by the server it announces (`opencode::running`): CUE uses that server and never starts one of its own.
    pub exe: Option<PathBuf>,
    /// As the agent's `--version` says it (`0.166.0`), when it answered; for an OpenCode found only by its server, as
    /// the server announces it.
    pub version: Option<String>,
}

/// Why an agent gave no answer. Each maps to the fallback the user chose and to a word on the Drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentFailure {
    NotInstalled,
    /// Not signed in, the sign-in expired, or no access (the CLI's own message).
    SignedOut,
    Timeout,
    /// The provider refused for now: a rate limit, or a free allowance used up (OpenCode 2.2.5's
    /// `ProviderRequestError` with `reason: "rate_limit"`). Taken as an agent that did not answer.
    RateLimited,
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
    tracing::warn!(agent = id.id(), "agent: no empty folder left among the spare ones");
    Err(AgentFailure::Failed("cwd"))
}

/// The files Codex reads CUE's fixed instructions from are `codex-instructions-<hash>.md` in CUE's agent folder
/// ([`codex_instructions`]): one per text, so a cleanup and a translation (or Ask) never share one, and no file
/// changes under a running Codex.
pub const CODEX_INSTRUCTIONS: &str = "codex-instructions";
/// The file that holds Codex's answer to `{"text": …}`, in CUE's agent folder.
pub const CODEX_SCHEMA: &str = "codex-schema.json";
/// The answer CUE asks Codex for, as every route asks for it.
const ANSWER_SCHEMA: &str =
    r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}"#;
/// The file that holds the shape of Codex's answer to an "Ask" question, in CUE's agent folder.
pub const CODEX_ASK_SCHEMA: &str = "codex-ask-schema.json";
/// An "Ask" answer (`cue_core::ask::AGENT_ASK_PROMPT`): the answer or the text for the field, whether it is one and
/// whether it replaces the selection, and the pages it relies on. Every field required and none other, as Codex's
/// structured output wants.
pub const ASK_SCHEMA: &str = r#"{"type":"object","properties":{"answer":{"type":"string"},"write":{"type":"boolean"},"replace":{"type":"boolean"},"sources":{"type":"array","items":{"type":"object","properties":{"url":{"type":"string"},"title":{"type":"string"}},"required":["url","title"],"additionalProperties":false}}},"required":["answer","write","replace","sources"],"additionalProperties":false}"#;

/// Which answer a Codex run is held to: its schema file and the schema it holds.
#[derive(Clone, Copy)]
struct Shape {
    file: &'static str,
    schema: &'static str,
}

const TEXT_SHAPE: Shape = Shape { file: CODEX_SCHEMA, schema: ANSWER_SCHEMA };
const ASK_SHAPE: Shape = Shape { file: CODEX_ASK_SCHEMA, schema: ASK_SCHEMA };

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
    /// Codex (Grok waits for its pipe). `home` is CUE's agent folder (`%LOCALAPPDATA%\CUE\agent`): the agent runs in
    /// its [`cwd`] there, and Codex's instructions and schema files are written beside that folder.
    Cli { found: Found, home: PathBuf },
}

impl AgentClient {
    pub fn id(&self) -> AgentId {
        match self {
            AgentClient::OpenCode(_) => AgentId::OpenCode,
            AgentClient::Cli { found, .. } => found.id,
        }
    }

    /// One answer to `p` from `model` (empty: the agent's default), within `timeout`, with no tools. The answer comes
    /// whole: `on_text` gets it once.
    ///
    /// For Codex `p.system` is written to its instructions file, so it must hold only CUE's fixed text and the user's
    /// instructions file, never anything of a dictation; `p.user` goes on stdin.
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
                AgentId::Codex => {
                    let out = codex(found, home, model, p, timeout, TEXT_SHAPE).await?;
                    cli::codex_text(&out.stdout).filter(|t| !t.trim().is_empty()).ok_or(AgentFailure::Failed("no answer"))?
                }
                // grok 1.0.34 reads the prompt only from an argument or a file; its named pipe comes later.
                AgentId::Grok => return Err(AgentFailure::Failed("grok needs a pipe")),
                AgentId::OpenCode => return Err(AgentFailure::Failed("opencode is a server")),
            },
        };
        if let Some(on_text) = on_text {
            on_text(&text);
        }
        Ok(text)
    }

    /// One "Ask" answer to `p` from `model` (empty: the agent's default), within `timeout`, as the JSON text the agent
    /// wrote, with the web searches it ran. Codex searches only when `p.web_search` (live search, its only tool) and is
    /// held to [`ASK_SCHEMA`]; OpenCode is asked with no tools at all, so it never searches.
    pub async fn ask(&self, model: &str, p: &Prompt, timeout: Duration) -> Result<(String, u32), AgentFailure> {
        match self {
            AgentClient::OpenCode(server) => {
                let quiet = Prompt { web_search: false, ..p.clone() };
                Ok((server.complete(model, &quiet, timeout, None).await?, 0))
            }
            AgentClient::Cli { found, home } => match found.id {
                AgentId::Codex => {
                    let out = codex(found, home, model, p, timeout, ASK_SHAPE).await?;
                    cli::codex_answer(&out.stdout, p.web_search)
                        .filter(|(t, _)| !t.trim().is_empty())
                        .ok_or(AgentFailure::Failed("no answer"))
                }
                AgentId::Grok => Err(AgentFailure::Failed("grok needs a pipe")),
                AgentId::OpenCode => Err(AgentFailure::Failed("opencode is a server")),
            },
        }
    }

    /// Whether this client can search the web for "Ask" ([`can_search`]).
    pub fn searches(&self) -> bool {
        can_search(self.id())
    }
}

/// Whether agent `id` can search the web for "Ask": Codex can (live search); OpenCode's API does not say whether a
/// model has a search tool, so CUE asks it with none; Grok is not offered. The one place this is decided: the
/// question's deadline and its run both ask here.
pub fn can_search(id: AgentId) -> bool {
    id == AgentId::Codex
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
    let exe = found.exe.clone().ok_or(AgentFailure::NotInstalled)?;
    let cwd = empty_cwd(home, AgentId::Codex)?;
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

/// One `codex exec`: CUE's fixed instructions and the answer's schema (`shape`) in files beside its working folder, the
/// rest of the request on stdin, run off the async threads; web search live only when `p.web_search`. A run that failed
/// is never read for an answer: a half-written last message is not a cleanup. Dropping the request (Esc) ends the run
/// and every process it started.
async fn codex(
    found: &Found,
    home: &Path,
    model: &str,
    p: &Prompt,
    timeout: Duration,
    shape: Shape,
) -> Result<process::Output, AgentFailure> {
    let instructions = codex_instructions(home, &p.system);
    let schema = home.join(shape.file);
    // Named after its text: a file that is there holds it.
    if !instructions.is_file() {
        write_whole(&instructions, &p.system)?;
    }
    if !std::fs::read(&schema).is_ok_and(|held| held == shape.schema.as_bytes()) {
        write_whole(&schema, shape.schema)?;
    }
    let args = cli::codex_args(model, &instructions, &schema, p.web_search);
    let exe = found.exe.clone().ok_or(AgentFailure::NotInstalled)?;
    let (stdin, cwd) = (p.user.clone(), empty_cwd(home, AgentId::Codex)?);
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
    Ok(out)
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

    #[tokio::test]
    async fn grok_is_not_run_without_a_pipe() {
        let home = temp_dir("grok");
        // A program that is not there: had it been run, the answer would be `NotInstalled`.
        let found = Found { id: AgentId::Grok, exe: Some(home.join("grok.exe")), version: None };
        let c = AgentClient::Cli { found, home: home.clone() };
        let p = Prompt { system: "Clean.".into(), user: "<transcript>hi</transcript>".into(), web_search: false };
        let r = c.complete("", &p, Duration::from_secs(5), None).await;
        assert_eq!(r, Err(AgentFailure::Failed("grok needs a pipe")));
        assert!(!home.exists(), "nothing was written or run for Grok");
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
        let found = Found { id: AgentId::Codex, exe: Some(home.join("codex.exe")), version: None };
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
