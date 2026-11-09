//! The user's own AI on the dictation path (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): which client a
//! cleanup or a translation goes to, the model it asks for, whether it is the agent's first request (the start
//! allowance), the marks that let a dictation skip an agent known to be signed out or missing, and the local fallback
//! the user may choose for when the AI does not answer.
//!
//! - Codex is asked `codex login status` (no text) before its first run and every [`MARK_FOR`] after, since a Codex
//!   never signed in retries for about 15 s and would read as a timeout; an expired sign-in, which that check does not
//!   tell, is marked from the run itself (`cli::signed_out`). A mark held answers the next dictation at once.
//! - OpenCode's server is found or started off the core's thread; the dictation waits for it within its own budget.
//! - The log gets the agent and a reason, never a text.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use cue_core::agents::{AgentId, ORDER};
use cue_core::flow::{AgentProblem, RefineOutcome};
use cue_core::refine::RefineContext;
use cue_core::settings::{DictionarySettings, Fallback, RefineSettings, Where};
use cue_engines::agent::opencode::{Server, fast_auto};
use cue_engines::agent::{self, AgentClient, AgentFailure, Found, Login};
use cue_engines::prompt::{Job, RefineResult, Usage};
use cue_engines::local;

use super::llm::LlmSlot;
use super::model::REST_AFTER;
use super::opencode::OpenCodeSlot;

/// How long a mark holds: a signed-out or missing agent is not asked again meanwhile, and a Codex that said it is
/// signed in is not asked again.
pub const MARK_FOR: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    /// Codex's sign-in was checked (it said it is signed in, said nothing CUE knows, or a run answered).
    Checked,
    SignedOut,
    Missing,
}

/// A mark per agent, each for [`MARK_FOR`].
#[derive(Default)]
pub struct AgentMarks(Mutex<HashMap<AgentId, (Mark, Instant)>>);

impl AgentMarks {
    fn held(&self, id: AgentId) -> Option<Mark> {
        let marks = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        marks.get(&id).filter(|(_, at)| at.elapsed() < MARK_FOR).map(|(m, _)| *m)
    }

    /// What a mark held says the agent needs: a sign-in, or to be installed.
    pub fn problem(&self, id: AgentId) -> Option<AgentProblem> {
        match self.held(id)? {
            Mark::SignedOut => Some(AgentProblem::SignedOut),
            Mark::Missing => Some(AgentProblem::Missing),
            Mark::Checked => None,
        }
    }

    fn checked(&self, id: AgentId) -> bool {
        self.held(id) == Some(Mark::Checked)
    }

    fn set(&self, id: AgentId, mark: Mark) {
        self.set_at(id, mark, Instant::now());
    }

    fn set_at(&self, id: AgentId, mark: Mark, at: Instant) {
        let was = self.0.lock().unwrap_or_else(PoisonError::into_inner).insert(id, (mark, at)).map(|(m, _)| m);
        if was != Some(mark) && mark != Mark::Checked {
            let mark = if mark == Mark::SignedOut { "signed out" } else { "missing" };
            tracing::info!(agent = id.id(), mark, "agent: marked for 5 minutes");
        }
    }

    /// A signed-out or missing mark of `id` goes (a "checked" one stays): the user's Retry right after signing in or
    /// installing asks the agent again. A new dictation keeps the mark for its 5 minutes.
    pub fn forget_problem(&self, id: AgentId) {
        let mut marks = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if marks.get(&id).is_some_and(|(m, _)| *m != Mark::Checked) {
            marks.remove(&id);
        }
    }

    /// Every mark goes: the agents are looked at again (the Cleanup page's list).
    // Task 11: `agents_get` clears them.
    #[allow(dead_code)]
    pub fn clear(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }
}

type Finder = Box<dyn Fn(AgentId) -> Option<PathBuf> + Send + Sync>;

/// The user's agents as the dictation path uses them.
pub struct Agents {
    /// CUE's agent folder, `%LOCALAPPDATA%\CUE\agent`: each agent runs in its own empty folder there.
    home: PathBuf,
    opencode: Arc<OpenCodeSlot>,
    /// Where an agent's program is (`discover::program`).
    find: Finder,
    pub marks: AgentMarks,
    /// When each agent run once per request (Codex) was last asked: its first request after CUE starts, or after
    /// [`REST_AFTER`] without one, gets the start allowance.
    used: Mutex<HashMap<AgentId, Instant>>,
    /// OpenCode's "Быстрая · авто" for the server in use, picked once from the models it lists.
    auto: Mutex<Option<(Weak<Server>, String)>>,
    /// Tests only: a shorter budget than an agent's.
    budget: Option<Duration>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Agents {
    pub fn new(home: PathBuf, opencode: Arc<OpenCodeSlot>) -> Arc<Self> {
        Self::with(home, opencode, Box::new(agent::discover::program), None)
    }

    fn with(home: PathBuf, opencode: Arc<OpenCodeSlot>, find: Finder, budget: Option<Duration>) -> Arc<Self> {
        Arc::new(Self {
            home,
            opencode,
            find,
            marks: AgentMarks::default(),
            used: Mutex::new(HashMap::new()),
            auto: Mutex::new(None),
            budget,
        })
    }

    /// A cleanup or a translation (`job`) of `raw` through agent `id`, within the agent's budget from the moment it is
    /// asked: finding or starting OpenCode's server and Codex's sign-in check come out of it. Never fails: the raw text
    /// comes back with the outcome the problem maps to. A mark held answers at once, without the agent.
    #[allow(clippy::too_many_arguments)]
    pub async fn refine(
        &self,
        id: AgentId,
        raw: &str,
        ctx: &RefineContext,
        dict: &DictionarySettings,
        s: &RefineSettings,
        instructions: &str,
        job: Job,
    ) -> RefineResult {
        if let Some(p) = self.marks.problem(id) {
            let reason = if p == AgentProblem::SignedOut { "signed out (marked)" } else { "not installed (marked)" };
            tracing::info!(agent = id.id(), reason, "cleanup: the agent is not asked");
            return RefineResult { text: raw.to_owned(), outcome: RefineOutcome::RawAgent(p), usage: Usage::default() };
        }
        // The agent cleans with CUE's instructions, as the cloud did.
        let job = if job == Job::Clean { Job::Rewrite } else { job };
        let first = self.first(id);
        let budget = self.budget.unwrap_or_else(|| agent::refine::budget(id, raw, first, job));
        let deadline = tokio::time::Instant::now() + budget;
        let client = match self.client(id, deadline).await {
            Ok(c) => c,
            Err(f) => {
                let r = agent::refine::failed(id, raw, f);
                self.note(id, r.outcome);
                return r;
            }
        };
        let model = self.model(&client, s, deadline).await;
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let r = agent::refine::run(&client, &model, raw, ctx, dict, s, instructions, job, left).await;
        self.note(id, r.outcome);
        r
    }

    /// Whether this request is the agent's first: after CUE starts, or after it rested (OpenCode's server let go, or
    /// [`REST_AFTER`] without a request to a CLI agent). The request is noted.
    fn first(&self, id: AgentId) -> bool {
        let now = Instant::now();
        let last = lock(&self.used).insert(id, now);
        match id {
            AgentId::OpenCode => !self.opencode.holds(),
            AgentId::Codex | AgentId::Grok => last.is_none_or(|t| now.duration_since(t) > REST_AFTER),
        }
    }

    /// The client for agent `id`, by `deadline`: OpenCode's server (found or started off the core's thread), or Codex
    /// once its sign-in is known not to be missing.
    async fn client(&self, id: AgentId, deadline: tokio::time::Instant) -> Result<AgentClient, AgentFailure> {
        match id {
            AgentId::OpenCode => {
                let slot = self.opencode.clone();
                let start = tokio::task::spawn_blocking(move || slot.get_or_start());
                match tokio::time::timeout_at(deadline, start).await {
                    Ok(Ok(server)) => server.map(AgentClient::OpenCode),
                    Ok(Err(_)) => Err(AgentFailure::Failed("join")),
                    // The start goes on: the next cleanup finds the server it started.
                    Err(_) => Err(AgentFailure::Timeout),
                }
            }
            AgentId::Codex => {
                let exe = (self.find)(id).ok_or(AgentFailure::NotInstalled)?;
                let found = Found { id, exe, version: None };
                if !self.marks.checked(id) {
                    match tokio::time::timeout_at(deadline, agent::codex_login(&found, &self.home)).await {
                        Ok(Ok(Login::SignedOut)) => return Err(AgentFailure::SignedOut),
                        Ok(Ok(Login::SignedIn | Login::Unknown)) => self.marks.set(id, Mark::Checked),
                        Ok(Err(f)) => return Err(f),
                        Err(_) => return Err(AgentFailure::Timeout),
                    }
                }
                Ok(AgentClient::Cli { found, home: self.home.clone() })
            }
            // Grok is offered only through its named pipe, which comes later.
            AgentId::Grok => Err(AgentFailure::Failed("not offered")),
        }
    }

    /// The model the request asks for: the one the user picked; else Codex's default; else OpenCode's "Быстрая ·
    /// авто", the first of its models whose name marks a small fast one (asked once per server, by `deadline`), else
    /// OpenCode's own default (empty). OpenCode's model goes in the request's JSON body, never on a command line.
    async fn model(&self, c: &AgentClient, s: &RefineSettings, deadline: tokio::time::Instant) -> String {
        if let Some(m) = s.agent_model(c.id()) {
            return m.to_owned();
        }
        let AgentClient::OpenCode(server) = c else { return String::new() };
        if let Some((held, m)) = lock(&self.auto).as_ref()
            && held.upgrade().is_some_and(|h| Arc::ptr_eq(&h, server))
        {
            return m.clone();
        }
        let Ok(Ok(models)) = tokio::time::timeout_at(deadline, server.models()).await else { return String::new() };
        let m = fast_auto(&models).map(|m| m.id.clone()).unwrap_or_default();
        *lock(&self.auto) = Some((Arc::downgrade(server), m.clone()));
        m
    }

    /// What a request's outcome says of the agent: signed out or missing is marked; a Codex that answered is signed in.
    fn note(&self, id: AgentId, outcome: RefineOutcome) {
        match outcome {
            RefineOutcome::RawAgent(AgentProblem::SignedOut) => self.marks.set(id, Mark::SignedOut),
            RefineOutcome::RawAgent(AgentProblem::Missing) => self.marks.set(id, Mark::Missing),
            RefineOutcome::Refined if id == AgentId::Codex => self.marks.set(id, Mark::Checked),
            _ => {}
        }
    }
}

/// The agent's result, or the local model's work in its place when the user chose "Почистить: <local model>" and the
/// model is in its slot at this moment: a resting one is never woken or waited for, and the text goes in as said.
/// Cleaned in place of an agent that did not answer, nothing is shown; in place of one that is signed out or missing,
/// the Drop still says so (`Covered`). The local model's own failure leaves the agent's result.
#[allow(clippy::too_many_arguments)]
pub async fn or_local(
    agent: RefineResult,
    fallback: Fallback,
    slot: &LlmSlot,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    instructions: &str,
    job: Job,
) -> RefineResult {
    if agent.outcome.cleaned() || fallback != Fallback::Local {
        return agent;
    }
    let Some(g) = slot.get() else {
        tracing::info!("cleanup: the local fallback's model is not loaded, the text goes in as said");
        return agent;
    };
    // Task 10: a model that only cleans (`cleans_only`) is no fallback for a translation; skip it here.
    let here = match job {
        Job::Clean | Job::Rewrite => local::refine(g, raw, ctx, dict, s, instructions).await,
        Job::Translate(to) => local::translate(g, raw, ctx, dict, s, instructions, to).await,
    };
    if here.outcome != RefineOutcome::Refined {
        return agent;
    }
    tracing::info!("cleanup: the local fallback cleaned the text");
    let outcome = match agent.outcome {
        RefineOutcome::RawAgent(p @ (AgentProblem::SignedOut | AgentProblem::Missing)) => RefineOutcome::Covered(p),
        _ => RefineOutcome::Refined,
    };
    RefineResult { outcome, ..here }
}

/// The cleanup goes to the user's agent and none is set: the agents on this computer are to be looked for (at startup,
/// and when the main window moves the cleanup to "your AI").
pub fn wants_agents(s: &RefineSettings) -> bool {
    s.where_ == Where::Agent && s.agent.is_none()
}

/// At startup, `refine.agent` not set on the agent route: the first agent found in [`ORDER`]; none found, the cleanup
/// runs on this computer, with nothing chosen there unless a model was. Whether the settings changed.
pub fn resolve_agent(s: &mut RefineSettings, found: &[AgentId]) -> bool {
    if !wants_agents(s) {
        return false;
    }
    match ORDER.iter().find(|a| found.contains(a)) {
        Some(&a) => s.agent = Some(a),
        None => s.where_ = Where::Local,
    }
    true
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    use cue_engines::EngineError;
    use cue_engines::local_llm::Generate;

    use super::*;

    const RAW: &str = "ну эм завтра в десять созвон";

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-agents-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A Codex `.cmd` shim: `login status` says `login` with exit `login_exit`; `exec` writes a line to `runs.txt` and
    /// runs `exec` (cmd lines).
    fn shim(dir: &Path, login: &str, login_exit: i32, exec: &str) -> PathBuf {
        let runs = dir.join("runs.txt");
        let script = format!(
            "@ECHO off\r\nif \"%1\"==\"login\" (\r\n  echo {login}\r\n  exit /b {login_exit}\r\n)\r\necho run>> \"{}\"\r\n{exec}\r\n",
            runs.display()
        );
        let shim = dir.join("codex.cmd");
        std::fs::write(&shim, script).unwrap();
        shim
    }

    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("runs.txt")).map(|s| s.lines().count()).unwrap_or(0)
    }

    fn agents(dir: &Path, exe: Option<PathBuf>, budget: Option<Duration>) -> Arc<Agents> {
        let slot = OpenCodeSlot::new(dir.join("agent"));
        Agents::with(dir.join("agent"), slot, Box::new(move |_| exe.clone()), budget)
    }

    async fn clean(a: &Agents) -> RefineResult {
        let s = RefineSettings::default();
        a.refine(AgentId::Codex, RAW, &RefineContext::default(), &Default::default(), &s, "", Job::Clean).await
    }

    /// A local model that answers `text`.
    struct Here(&'static str);
    impl Generate for Here {
        fn generate(&self, _: &str, _: &str, _: &str, _: Instant, _: &AtomicBool) -> Result<String, EngineError> {
            Ok(format!("{{\"text\": \"{}\"}}", self.0))
        }
        fn id(&self) -> &str {
            "llm--here--model--q4_k_m"
        }
    }

    fn loaded(text: &'static str) -> LlmSlot {
        let slot = LlmSlot::default();
        slot.ready(Arc::new(Here(text)));
        slot
    }

    fn silent() -> RefineResult {
        RefineResult { text: RAW.into(), outcome: RefineOutcome::RawAgent(AgentProblem::Silent), usage: Usage::default() }
    }

    async fn fallback(agent: RefineResult, fallback: Fallback, slot: &LlmSlot, job: Job) -> RefineResult {
        let s = RefineSettings::default();
        or_local(agent, fallback, slot, RAW, &RefineContext::default(), &Default::default(), &s, "", job).await
    }

    #[test]
    fn marks_hold_for_five_minutes() {
        let m = AgentMarks::default();
        assert_eq!(m.problem(AgentId::Codex), None);
        m.set(AgentId::Codex, Mark::SignedOut);
        assert_eq!(m.problem(AgentId::Codex), Some(AgentProblem::SignedOut));
        assert_eq!(m.problem(AgentId::OpenCode), None, "a mark is the agent's own");
        m.set_at(AgentId::Codex, Mark::Missing, Instant::now() - MARK_FOR - Duration::from_secs(1));
        assert_eq!(m.problem(AgentId::Codex), None, "an old mark is gone");
        m.set(AgentId::Codex, Mark::Checked);
        assert!(m.checked(AgentId::Codex));
        assert_eq!(m.problem(AgentId::Codex), None);
        m.set(AgentId::OpenCode, Mark::Missing);
        m.clear();
        assert_eq!(m.problem(AgentId::OpenCode), None);
    }

    /// The run says the sign-in expired: the agent is marked, and the next dictation goes in at once without it.
    #[tokio::test]
    async fn signed_out_marks_agent_and_next_dictation_skips_the_wait() {
        let dir = temp_dir("expired");
        let out = dir.join("out.jsonl");
        std::fs::write(
            &out,
            "{\"type\":\"error\",\"message\":\"Your access token could not be refreshed because your refresh token has expired. \
             Please log out and sign in again.\"}\n{\"type\":\"turn.failed\",\"error\":{\"message\":\"sign in again\"}}\n",
        )
        .unwrap();
        let exe = shim(&dir, "Logged in using ChatGPT", 0, &format!("type \"{}\"\r\nexit /b 1", out.display()));
        let a = agents(&dir, Some(exe), None);
        let r = clean(&a).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::SignedOut)));
        assert_eq!(runs(&dir), 1);
        let start = Instant::now();
        let r = clean(&a).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::SignedOut)));
        assert!(start.elapsed() < Duration::from_millis(200), "{:?}", start.elapsed());
        assert_eq!(runs(&dir), 1, "the agent was not run again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A Codex never signed in says so to `codex login status` at once: no run, no 15 s of retries.
    #[tokio::test]
    async fn never_signed_in_codex_is_told_by_its_login_status() {
        let dir = temp_dir("never");
        let exe = shim(&dir, "Not logged in", 1, "ping -n 30 127.0.0.1 >nul");
        let a = agents(&dir, Some(exe), None);
        let start = Instant::now();
        let r = clean(&a).await;
        assert_eq!(r.outcome, RefineOutcome::RawAgent(AgentProblem::SignedOut));
        assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
        assert_eq!(runs(&dir), 0, "nothing of the dictation went to Codex");
        assert_eq!(a.marks.problem(AgentId::Codex), Some(AgentProblem::SignedOut));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Retry of a dictation the mark answered: the mark goes and the agent is asked again; a new dictation without a
    /// Retry keeps the mark.
    #[tokio::test]
    async fn a_retry_clears_the_mark_and_asks_the_agent_again() {
        let dir = temp_dir("retry");
        let out = dir.join("out.jsonl");
        std::fs::write(&out, "{\"type\":\"error\",\"message\":\"Please log out and sign in again.\"}\n").unwrap();
        let exe = shim(&dir, "Logged in using ChatGPT", 0, &format!("type \"{}\"\r\nexit /b 1", out.display()));
        let a = agents(&dir, Some(exe), None);
        assert_eq!(clean(&a).await.outcome, RefineOutcome::RawAgent(AgentProblem::SignedOut));
        assert_eq!(clean(&a).await.outcome, RefineOutcome::RawAgent(AgentProblem::SignedOut));
        assert_eq!(runs(&dir), 1, "a new dictation keeps the mark");
        a.marks.forget_problem(AgentId::Codex);
        assert_eq!(clean(&a).await.outcome, RefineOutcome::RawAgent(AgentProblem::SignedOut));
        assert_eq!(runs(&dir), 2, "the Retry asked Codex again");
        // A "checked" mark is not a problem: it stays.
        a.marks.set(AgentId::Codex, Mark::Checked);
        a.marks.forget_problem(AgentId::Codex);
        assert!(a.marks.checked(AgentId::Codex));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_move_to_your_ai_with_none_set_looks_for_agents() {
        let s = RefineSettings { where_: Where::Agent, agent: None, ..RefineSettings::default() };
        assert!(wants_agents(&s));
        assert!(!wants_agents(&RefineSettings { agent: Some(AgentId::Codex), ..s.clone() }));
        assert!(!wants_agents(&RefineSettings { where_: Where::Local, ..s.clone() }));
        assert!(!wants_agents(&RefineSettings { where_: Where::Off, ..s }));
    }

    #[tokio::test]
    async fn a_missing_codex_is_marked_missing() {
        let dir = temp_dir("missing");
        let a = agents(&dir, None, None);
        assert_eq!(clean(&a).await.outcome, RefineOutcome::RawAgent(AgentProblem::Missing));
        assert_eq!(a.marks.problem(AgentId::Codex), Some(AgentProblem::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Codex that answers: signed in, so the next request skips the check; its first request gets the allowance.
    #[tokio::test]
    async fn an_answer_marks_codex_signed_in_and_only_the_first_request_is_first() {
        let dir = temp_dir("answers");
        let out = dir.join("out.jsonl");
        let answer = serde_json::json!({"type": "item.completed", "item": {"id": "0", "type": "agent_message", "text": "{\"text\": \"Завтра в десять созвон.\"}"}});
        std::fs::write(&out, format!("{answer}\n")).unwrap();
        let exe = shim(&dir, "Logged in using ChatGPT", 0, &format!("type \"{}\"", out.display()));
        let a = agents(&dir, Some(exe), None);
        let r = clean(&a).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Завтра в десять созвон.", RefineOutcome::Refined));
        assert!(a.marks.checked(AgentId::Codex));
        assert!(!a.first(AgentId::Codex), "asked a moment ago");
        lock(&a.used).insert(AgentId::Codex, Instant::now() - REST_AFTER - Duration::from_secs(1));
        assert!(a.first(AgentId::Codex), "after a rest it starts again");
        assert!(a.first(AgentId::OpenCode), "no OpenCode server held yet");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The agent did not answer and the user chose the local fallback with its model loaded: cleaned here, and the Drop
    /// shows nothing (`Refined`).
    #[tokio::test]
    async fn agent_silent_with_local_fallback_cleans_locally_and_shows_nothing() {
        let dir = temp_dir("silent");
        let exe = shim(&dir, "Logged in using ChatGPT", 0, "ping -n 30 127.0.0.1 >nul");
        let a = agents(&dir, Some(exe), Some(Duration::from_millis(1_500)));
        let start = Instant::now();
        let r = clean(&a).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Silent)));
        assert!(start.elapsed() < Duration::from_secs(8), "{:?}", start.elapsed());
        let slot = loaded("Завтра в десять созвон.");
        let r = fallback(r, Fallback::Local, &slot, Job::Rewrite).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Завтра в десять созвон.", RefineOutcome::Refined));
        // Paste as said: the agent's word stays.
        assert_eq!(fallback(silent(), Fallback::Raw, &slot, Job::Rewrite).await.outcome, RefineOutcome::RawAgent(AgentProblem::Silent));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The local model rests (its slot is empty): it is not woken or waited for, and the text goes in as said.
    #[tokio::test]
    async fn local_fallback_skipped_when_model_resting() {
        let slot = LlmSlot::default();
        let start = Instant::now();
        let r = fallback(silent(), Fallback::Local, &slot, Job::Rewrite).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Silent)));
        assert!(start.elapsed() < Duration::from_millis(100), "the fallback waited for the model: {:?}", start.elapsed());
        assert!(slot.get().is_none(), "nothing was loaded for it");
    }

    /// In place of a signed-out or missing agent the local model's text goes in, and the Drop still says so; a
    /// cleanup the agent made, or a local model that fails, is left as it was.
    #[tokio::test]
    async fn local_fallback_covers_a_signed_out_agent_and_keeps_its_word() {
        let slot = loaded("Завтра в десять созвон.");
        for p in [AgentProblem::SignedOut, AgentProblem::Missing] {
            let agent = RefineResult { text: RAW.into(), outcome: RefineOutcome::RawAgent(p), usage: Usage::default() };
            let r = fallback(agent, Fallback::Local, &slot, Job::Rewrite).await;
            assert_eq!((r.text.as_str(), r.outcome), ("Завтра в десять созвон.", RefineOutcome::Covered(p)));
        }
        let fine = RefineResult { text: "Агент.".into(), outcome: RefineOutcome::Refined, usage: Usage::default() };
        assert_eq!(fallback(fine, Fallback::Local, &slot, Job::Rewrite).await.text, "Агент.");
        // The local model answered in the language spoken: no translation, the agent's result stays.
        let r = fallback(silent(), Fallback::Local, &slot, Job::Translate("en")).await;
        assert_eq!(r.outcome, RefineOutcome::RawAgent(AgentProblem::Silent));
    }

    #[test]
    fn startup_resolves_unset_agent_to_first_found_else_local_nothing_chosen() {
        let unset = || RefineSettings { where_: Where::Agent, agent: None, local: String::new(), ..RefineSettings::default() };
        let mut s = unset();
        assert!(resolve_agent(&mut s, &[AgentId::Codex, AgentId::OpenCode]));
        assert_eq!((s.where_, s.agent), (Where::Agent, Some(AgentId::OpenCode)), "OpenCode comes first");
        let mut s = unset();
        assert!(resolve_agent(&mut s, &[AgentId::Codex]));
        assert_eq!(s.agent, Some(AgentId::Codex));
        let mut s = unset();
        assert!(resolve_agent(&mut s, &[AgentId::Grok]), "Grok is not offered yet");
        assert_eq!((s.where_, s.agent, s.local.as_str()), (Where::Local, None, ""));
        let mut s = unset();
        assert!(resolve_agent(&mut s, &[]));
        assert_eq!((s.where_, s.agent, s.local.as_str()), (Where::Local, None, ""), "nothing chosen");
        // A chosen agent, or another route, is left alone.
        let mut s = RefineSettings { agent: Some(AgentId::Codex), ..unset() };
        assert!(!resolve_agent(&mut s, &[AgentId::OpenCode]));
        assert_eq!(s.agent, Some(AgentId::Codex));
        let mut s = RefineSettings { where_: Where::Off, ..unset() };
        assert!(!resolve_agent(&mut s, &[AgentId::OpenCode]));
        assert_eq!((s.where_, s.agent), (Where::Off, None));
    }
}
