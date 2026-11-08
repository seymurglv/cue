//! The agent path (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): finding the user's own OpenCode, Codex or
//! Grok on this computer and running them safely. CUE holds no key: each agent uses the user's own sign-in.
//!
//! - [`discover`]: which agents are installed, in [`cue_core::agents::ORDER`].
//! - [`process::run`]: one run of an agent's CLI with no window, the text on stdin, an empty working folder and a
//!   timeout that ends the whole process tree.
//! - [`cli`]: the flags that leave Codex and Grok no shell, no file reading and no network tool, and reading their
//!   answers.
//! - [`opencode`]: the user's own OpenCode server, or one CUE starts on 127.0.0.1, over loopback HTTP.
pub mod cli;
pub mod discover;
pub mod opencode;
pub mod process;

use std::path::PathBuf;

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
