//! The agent path (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): finding the user's own OpenCode, Codex or
//! Grok on this computer and running them safely. CUE holds no key: each agent uses the user's own sign-in.
//!
//! - [`discover`]: which agents are installed, in [`cue_core::agents::ORDER`].
//! - [`process::run`]: one run of an agent's CLI with no window, the text on stdin, an empty working folder and a
//!   timeout that ends the whole process tree.
//! - [`cli`]: the flags that leave Codex and Grok no shell, no file reading and no network tool, and reading their
//!   answers.
//! - [`opencode`]: the user's own OpenCode server, or one CUE starts on 127.0.0.1, over loopback HTTP.
//! - [`AgentClient`]: one answer from either; [`refine`]: a cleanup or a translation through it.
pub mod cli;
pub mod discover;
pub mod opencode;
pub mod process;
pub mod refine;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cue_core::agents::AgentId;
use serde::Serialize;

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

/// The file Codex reads CUE's fixed instructions from, in CUE's agent folder.
pub const CODEX_INSTRUCTIONS: &str = "codex-instructions.md";
/// The file that holds Codex's answer to `{"text": …}`, in CUE's agent folder.
pub const CODEX_SCHEMA: &str = "codex-schema.json";
/// The answer CUE asks Codex for, as the cloud path asks for it.
const ANSWER_SCHEMA: &str =
    r#"{"type":"object","properties":{"text":{"type":"string"}},"required":["text"],"additionalProperties":false}"#;

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
                AgentId::Codex => codex(found, home, model, p, timeout).await?,
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
}

/// One `codex exec`: CUE's fixed instructions and the answer's schema in files beside its working folder, the rest of
/// the request on stdin, run off the async threads.
async fn codex(found: &Found, home: &Path, model: &str, p: &Prompt, timeout: Duration) -> Result<String, AgentFailure> {
    let instructions = home.join(CODEX_INSTRUCTIONS);
    let schema = home.join(CODEX_SCHEMA);
    write_if_changed(&instructions, &p.system)?;
    write_if_changed(&schema, ANSWER_SCHEMA)?;
    let args = cli::codex_args(model, &instructions, &schema);
    let (exe, stdin, cwd) = (found.exe.clone(), p.user.clone(), cwd(home, AgentId::Codex));
    let out = tokio::task::spawn_blocking(move || process::run(&exe, &args, &stdin, &cwd, timeout))
        .await
        .map_err(|_| AgentFailure::Failed("join"))??;
    if cli::signed_out(AgentId::Codex, &out) {
        return Err(AgentFailure::SignedOut);
    }
    cli::codex_text(&out.stdout).filter(|t| !t.trim().is_empty()).ok_or(AgentFailure::Failed("no answer"))
}

/// Writes `text` to `path` unless it holds it already (Codex reads it at every run).
fn write_if_changed(path: &Path, text: &str) -> Result<(), AgentFailure> {
    if std::fs::read(path).is_ok_and(|held| held == text.as_bytes()) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|_| AgentFailure::Failed("files"))?;
    }
    std::fs::write(path, text).map_err(|_| AgentFailure::Failed("files"))
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
        let found = Found { id: AgentId::Grok, exe: home.join("grok.exe"), version: None };
        let c = AgentClient::Cli { found, home: home.clone() };
        let p = Prompt { system: "Clean.".into(), user: "<transcript>hi</transcript>".into(), web_search: false };
        let r = c.complete("", &p, Duration::from_secs(5), None).await;
        assert_eq!(r, Err(AgentFailure::Failed("grok needs a pipe")));
        assert!(!home.exists(), "nothing was written or run for Grok");
    }

    #[test]
    fn a_file_is_written_only_when_it_changes() {
        let home = temp_dir("files");
        let file = home.join(CODEX_INSTRUCTIONS);
        write_if_changed(&file, "one").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "one");
        let written = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        write_if_changed(&file, "one").unwrap();
        assert_eq!(std::fs::metadata(&file).unwrap().modified().unwrap(), written, "the same text is not written again");
        write_if_changed(&file, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&home);
    }
}
