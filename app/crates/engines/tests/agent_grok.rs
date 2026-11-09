//! `agent::refine` over Grok's stand-in (`src/bin/fake_agent.rs`, copied to a `grok.exe`, as the real Grok is an
//! `.exe`): no network, no real agent. The fake reads its prompt from the pipe CUE names, keeps the run as Grok does
//! in a Grok folder of the test's own, and writes down what it got.
#![cfg(windows)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cue_core::agents::AgentId;
use cue_core::flow::{AgentProblem, RefineOutcome};
use cue_core::refine::{BOLD_PROMPT, RefineContext, SYSTEM_PROMPT, user_message};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_engines::agent::refine::{self, KEEP_EVERY_SENTENCE};
use cue_engines::agent::{AgentClient, Found, pipe};
use cue_engines::claude::Job;
use serde_json::{Value, json};

const FAKE: &str = env!("CARGO_BIN_EXE_fake_agent");
const RAW: &str = "ну эм короче завтра в десять созвон с командой по релизу э не забудь взять ноутбук";
const CLEAN: &str = "Завтра в десять созвон с командой по релизу. Не забудь взять ноутбук.";

/// A Grok for one test: its program and `fake-grok.json`, CUE's agent folder, Grok's own folder, and the files the
/// fake writes.
struct Grok {
    dir: PathBuf,
    home: PathBuf,
    data: PathBuf,
    record: PathBuf,
    deleted: PathBuf,
    pid: PathBuf,
    client: AgentClient,
}

/// What the fake Grok does.
struct Answer<'a> {
    stdout: &'a str,
    stderr: &'a str,
    exit: i32,
    sleep_ms: u64,
    read_prompt: bool,
}

/// Grok's `--output-format json` object, answering `{"text": text}`, for the session the run was given.
fn says(text: &str) -> String {
    let inner = json!({ "text": text }).to_string();
    json!({"text": inner, "stopReason": "end_turn", "sessionId": "{session}", "requestId": "0", "num_turns": 1}).to_string()
}

fn answering(name: &str, stdout: &str) -> Grok {
    grok(name, Answer { stdout, stderr: "", exit: 0, sleep_ms: 0, read_prompt: true })
}

fn grok(name: &str, a: Answer) -> Grok {
    let dir = std::env::temp_dir().join(format!("cue-agent-grok-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("grok.exe");
    std::fs::copy(FAKE, &exe).unwrap();
    let (stdout, stderr) = (dir.join("stdout.txt"), dir.join("stderr.txt"));
    std::fs::write(&stdout, a.stdout).unwrap();
    std::fs::write(&stderr, a.stderr).unwrap();
    let (home, data) = (dir.join("agent"), dir.join(".grok"));
    let (record, deleted, pid) = (dir.join("record.json"), dir.join("deleted.txt"), dir.join("pid.txt"));
    let config = json!({
        "record": record, "deleted": deleted, "pid": pid, "data": data, "stdout": stdout, "stderr": stderr,
        "exit": a.exit, "sleep_ms": a.sleep_ms, "read_prompt": a.read_prompt,
    });
    std::fs::write(bin.join("fake-grok.json"), config.to_string()).unwrap();
    let found = Found { id: AgentId::Grok, exe, version: Some("1.0.34".into()) };
    let client = AgentClient::Grok { found, home: home.clone(), data: Some(data.clone()) };
    Grok { dir, home, data, record, deleted, pid, client }
}

impl Grok {
    fn got(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.record).expect("the fake ran")).unwrap()
    }

    fn args(&self) -> Vec<String> {
        serde_json::from_value(self.got()["args"].clone()).unwrap()
    }

    fn flag(&self, name: &str) -> String {
        let args = self.args();
        args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone()).unwrap_or_else(|| panic!("{name} in {args:?}"))
    }

    /// The sessions folder Grok keeps for CUE's Grok folder.
    fn sessions(&self) -> PathBuf {
        let cwd = self.home.join("grok");
        let encoded: String = cwd
            .to_string_lossy()
            .bytes()
            .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") })
            .collect();
        self.data.join("sessions").join(encoded)
    }

    /// Waits (up to 10 s) until the fake was asked to delete `session`; the lines it was asked with.
    fn wait_deleted(&self, session: &str) -> Vec<String> {
        let start = Instant::now();
        loop {
            let lines: Vec<String> =
                std::fs::read_to_string(&self.deleted).unwrap_or_default().lines().map(String::from).collect();
            if lines.iter().any(|l| l.starts_with(session)) && !self.sessions().join("prompt_history.jsonl").exists() {
                return lines;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "the session was not deleted: {lines:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// After a run: Grok was told to delete the run's session, in CUE's Grok folder; the session's folder and the
    /// prompt history are gone, and nothing else of the run is in Grok's folder.
    fn forgot(&self, session: &str) {
        let lines = self.wait_deleted(session);
        let line = lines.iter().find(|l| l.starts_with(session)).unwrap();
        assert_eq!(line.split('\t').nth(1), Some(self.home.join("grok").to_string_lossy().as_ref()), "in the same folder");
        assert!(!self.sessions().join(session).exists(), "the session's folder is gone");
        let left: Vec<String> = std::fs::read_dir(self.sessions())
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "left in Grok's folder: {left:?}");
    }
}

impl Drop for Grok {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ctx() -> RefineContext {
    RefineContext {
        app: "Obsidian-Q7".into(),
        before_cursor: Some("Привет, коллеги.".into()),
        habits: vec!["Без точки в конце в Obsidian-Q7".into()],
        said: Vec::new(),
    }
}

fn dict() -> DictionarySettings {
    DictionarySettings { terms: vec!["Zephyrine".into()], learned: Vec::new() }
}

async fn clean(g: &Grok) -> cue_engines::claude::RefineResult {
    refine::refine(&g.client, "grok-4.7", RAW, &ctx(), &dict(), &RefineSettings::default(), BOLD_PROMPT, false).await
}

/// The cleanup goes through the pipe: Grok read the whole request from it (the dictionary, the habit, the context and
/// the transcript), its arguments hold CUE's fixed text alone, its switches are off, and the answer is pasted.
#[tokio::test]
async fn grok_cleanup_through_the_pipe() {
    let g = answering("cleaned", &says(CLEAN));
    let r = clean(&g).await;
    assert_eq!((r.text.as_str(), r.outcome), (CLEAN, RefineOutcome::Refined));

    let got = g.got();
    let prompt = got["prompt"].as_str().expect("the fake read the pipe");
    assert!(prompt.ends_with(&user_message(&ctx(), RAW)), "{prompt}");
    for there in ["Zephyrine", "Без точки в конце", "Привет, коллеги.", RAW] {
        assert!(prompt.contains(there), "{there} not through the pipe");
    }
    let system = got["system"].as_str().unwrap();
    assert_eq!(system, format!("{SYSTEM_PROMPT}\n\n{BOLD_PROMPT}\n\n{KEEP_EVERY_SENTENCE}"));
    for never in ["ну эм", "Zephyrine", "Без точки", "Привет, коллеги", "Obsidian-Q7"] {
        assert!(!g.args().iter().any(|a| a.contains(never)), "{never} in an argument");
    }
    assert!(g.flag("--prompt-file").starts_with(pipe::PREFIX));
    assert_eq!(g.flag("--max-turns"), "1");
    assert_eq!(g.flag("-m"), "grok-4.7");
    let schema: Value = serde_json::from_str(&g.flag("--json-schema")).expect("the schema reached Grok whole");
    assert_eq!(schema["required"], json!(["text"]));
    for name in ["GROK_TELEMETRY_ENABLED", "GROK_TELEMETRY_TRACE_UPLOAD", "GROK_SESSION_SEARCH", "GROK_SESSION_RECAP", "GROK_MEMORY"] {
        assert_eq!(got["env"][name], "0", "{name}");
    }
    assert_eq!(got["cwd"], g.home.join("grok").to_string_lossy().as_ref(), "Grok's own empty folder");
    g.forgot(&g.flag("--session-id"));
}

/// A translation through Grok: its target goes through the pipe, and the cleanup's instructions stay out.
#[tokio::test]
async fn grok_translation_goes_through_the_pipe() {
    let g = answering("translated", &says("Hi, how are you today?"));
    let raw = "ну привет как дела у тебя сегодня";
    let r = refine::translate(&g.client, "", raw, &ctx(), &dict(), &RefineSettings::default(), BOLD_PROMPT, false, "en").await;
    assert_eq!((r.text.as_str(), r.outcome), ("Hi, how are you today?", RefineOutcome::Refined));
    let got = g.got();
    assert!(got["prompt"].as_str().unwrap().contains(&cue_core::translate::instruction("en")));
    assert_eq!(got["system"], format!("{SYSTEM_PROMPT}\n\n{KEEP_EVERY_SENTENCE}").as_str());
    assert_eq!(g.flag("-m"), "grok-4.7", "the default model");
}

/// `--max-turns 1` cut the turn (the model reached for a tool): exit 1, `stopReason` `cancelled`. Its text, the
/// model's plan, is never pasted: the raw text goes in by the user's fallback.
#[tokio::test]
async fn grok_max_turns_is_not_pasted() {
    let cut = include_str!("fixtures/agents/grok-max-turns.json");
    let g = grok(
        "max-turns",
        Answer { stdout: cut, stderr: include_str!("fixtures/agents/grok-max-turns.stderr"), exit: 1, sleep_ms: 0, read_prompt: true },
    );
    let r = clean(&g).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
    // Even with exit 0 the cut turn is not taken.
    let g = answering("max-turns-0", cut);
    let r = clean(&g).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
}

/// After every run, answer or not, the session Grok keeps is deleted (`grok sessions delete <id>`, in CUE's Grok
/// folder) and the prompt history beside it removed: after an answer, after a timeout, and after Esc.
#[tokio::test]
async fn grok_session_is_deleted_after_every_run() {
    // An answer.
    let g = answering("forget-answer", &says(CLEAN));
    assert_eq!(clean(&g).await.outcome, RefineOutcome::Refined);
    g.forgot(&g.flag("--session-id"));

    // A timeout: Grok is still thinking when the budget ends.
    let g = grok("forget-timeout", Answer { stdout: &says(CLEAN), stderr: "", exit: 0, sleep_ms: 30_000, read_prompt: true });
    let s = RefineSettings::default();
    let start = Instant::now();
    let r = refine::run(&g.client, "", RAW, &ctx(), &dict(), &s, BOLD_PROMPT, Job::Rewrite, Duration::from_millis(1_500)).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Silent)));
    assert!(start.elapsed() < Duration::from_secs(8), "{:?}", start.elapsed());
    g.forgot(&g.flag("--session-id"));

    // Esc: the request is dropped while Grok works.
    let g = grok("forget-esc", Answer { stdout: &says(CLEAN), stderr: "", exit: 0, sleep_ms: 60_000, read_prompt: true });
    let client = g.client.clone();
    let request = tokio::spawn(async move {
        let s = RefineSettings::default();
        refine::run(&client, "", RAW, &ctx(), &dict(), &s, BOLD_PROMPT, Job::Rewrite, Duration::from_secs(30)).await
    });
    let start = Instant::now();
    while !g.pid.exists() {
        assert!(start.elapsed() < Duration::from_secs(15), "the fake did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    request.abort();
    assert!(request.await.is_err_and(|e| e.is_cancelled()), "the request was dropped");
    g.forgot(&g.flag("--session-id"));
    assert!(start.elapsed() < Duration::from_secs(20), "deleted after the drop, not the 30 s timeout: {:?}", start.elapsed());
}

/// Grok signed out, as grok 1.0.34 says it (it reads the prompt, then the sign-in fails): the user is told to sign in.
#[tokio::test]
async fn grok_signed_out_is_raw_agent_signed_out() {
    let out = include_str!("fixtures/agents/grok-signed-out.stdout");
    let err = include_str!("fixtures/agents/grok-signed-out.stderr");
    let g = grok("signed-out", Answer { stdout: out, stderr: err, exit: 1, sleep_ms: 0, read_prompt: true });
    let r = clean(&g).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::SignedOut)));
}

/// A Grok that exits without opening the pipe is judged by what it said, at once: never a wait for the pipe.
#[tokio::test]
async fn grok_that_never_opens_the_pipe_is_judged_by_its_exit() {
    let out = include_str!("fixtures/agents/grok-signed-out.stdout");
    let err = include_str!("fixtures/agents/grok-signed-out.stderr");
    let g = grok("no-read", Answer { stdout: out, stderr: err, exit: 1, sleep_ms: 0, read_prompt: false });
    let s = RefineSettings::default();
    let start = Instant::now();
    let r = refine::run(&g.client, "", RAW, &ctx(), &dict(), &s, BOLD_PROMPT, Job::Rewrite, Duration::from_secs(20)).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::SignedOut)));
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    assert!(g.got()["prompt"].is_null(), "the fake never read it");

    // An answer from a Grok that never read the text is not one.
    let g = grok("no-read-answer", Answer { stdout: &says(CLEAN), stderr: "", exit: 0, sleep_ms: 0, read_prompt: false });
    let r = clean(&g).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
}

#[tokio::test]
async fn missing_grok_is_raw_agent_missing() {
    let g = answering("missing", &says(CLEAN));
    let found = Found { id: AgentId::Grok, exe: g.dir.join("no-such-grok.exe"), version: None };
    let gone = AgentClient::Grok { found, home: g.home.clone(), data: Some(g.data.clone()) };
    let r = refine::refine(&gone, "", RAW, &ctx(), &dict(), &RefineSettings::default(), BOLD_PROMPT, false).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Missing)));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!g.deleted.exists() && !g.data.exists(), "nothing ran, so nothing was deleted or written");
}

/// Every file under `dir` changed since `since`.
fn changed_since(dir: &std::path::Path, since: std::time::SystemTime, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        // Grok's sign-in is never read.
        let name = e.file_name().to_string_lossy().to_lowercase();
        if ["auth", "credential", "token"].iter().any(|n| name.contains(n)) {
            continue;
        }
        if p.is_dir() {
            changed_since(&p, since, out);
        } else if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since) {
            out.push(p);
        }
    }
}

/// CUE's own Grok path against the real, signed-in Grok on this computer (grok-4.7): the cleaned text comes back, the
/// session Grok kept and the prompt history are gone afterwards, the working folder stays empty, and no file Grok
/// changed during the run holds the text. Never run by default; run it explicitly:
/// `cargo test -p cue-engines --test agent_grok live_grok -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "runs the real, signed-in Grok"]
async fn live_grok_cleans_and_leaves_nothing() {
    // CUE's log (never a text) says why a run fell back.
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).try_init();
    let exe = PathBuf::from(std::env::var("USERPROFILE").unwrap()).join(r".grok\bin\grok.exe");
    let data = cue_engines::agent::grok_home().unwrap();
    let dir = std::env::temp_dir().join(format!("cue-live-grok-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let home = dir.join("agent");
    let found = Found { id: AgentId::Grok, exe, version: None };
    let client = AgentClient::Grok { found, home: home.clone(), data: Some(data.clone()) };
    let raw = "ну эм завтра в десять созвон";
    let since = std::time::SystemTime::now();
    let start = Instant::now();
    let r = refine::refine(&client, "grok-4.7", raw, &RefineContext::default(), &Default::default(), &RefineSettings::default(), "", true)
        .await;
    eprintln!("live: {:?} {:?} in {:?}", r.outcome, r.text, start.elapsed());
    assert_eq!(r.outcome, RefineOutcome::Refined);
    assert!(r.text.contains("десять") || r.text.contains("10"), "{}", r.text);

    // The deletion runs on its own thread after the answer: wait until the run's sessions folder holds no session and
    // no prompt history.
    let tag = format!("cue-live-grok-{}", std::process::id());
    let left = || -> Vec<String> {
        std::fs::read_dir(data.join("sessions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(&tag))
            .flat_map(|g| std::fs::read_dir(g.path()).into_iter().flatten().flatten())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    while left().iter().any(|n| n == "prompt_history.jsonl" || !n.contains('.')) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let sessions_left = left();
    eprintln!("live: left in the run's sessions folder after {:?}: {sessions_left:?}", start.elapsed());
    assert!(!sessions_left.iter().any(|n| n == "prompt_history.jsonl"), "{sessions_left:?}");
    assert!(sessions_left.iter().all(|n| n.contains('.')), "a session folder is left: {sessions_left:?}");
    // No file Grok changed during the run holds the text, as UTF-8 or JSON-escaped.
    let mut changed = Vec::new();
    changed_since(&data, since, &mut changed);
    let escaped: String = "созвон".chars().map(|c| format!("\\u{:04x}", c as u32)).collect();
    let holding: Vec<&PathBuf> = changed
        .iter()
        .filter(|p| {
            std::fs::read(p).is_ok_and(|b| {
                let s = String::from_utf8_lossy(&b).to_lowercase();
                s.contains("созвон") || s.contains(&escaped)
            })
        })
        .collect();
    eprintln!("live: files Grok changed during the run, still there: {changed:#?}");
    assert!(holding.is_empty(), "files that still hold the text: {holding:#?}");
    let cwd_entries = std::fs::read_dir(home.join("grok")).map(|d| d.count()).unwrap_or(usize::MAX);
    assert_eq!(cwd_entries, 0, "Grok wrote into its working folder");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Nothing outside the run's own sessions folder in Grok's folder is touched, even a prompt history of another folder.
#[tokio::test]
async fn grok_cleanup_touches_nothing_else_in_groks_folder() {
    let g = answering("others", &says(CLEAN));
    let other = g.data.join("sessions").join("C%3A%5Cwork%5Cproject");
    std::fs::create_dir_all(other.join("0000")).unwrap();
    std::fs::write(other.join("prompt_history.jsonl"), "{}\n").unwrap();
    std::fs::write(g.data.join("config.toml"), "model = \"grok-4.7\"\n").unwrap();
    assert_eq!(clean(&g).await.outcome, RefineOutcome::Refined);
    g.forgot(&g.flag("--session-id"));
    assert!(other.join("prompt_history.jsonl").is_file() && other.join("0000").is_dir());
    assert!(g.data.join("config.toml").is_file());
}
