//! `agent::refine` over Codex's stand-in (`src/bin/fake_agent.rs`), started through a `.cmd` shim as the real
//! `codex.cmd` is: no network, no real agent. The shim tells the fake what to answer; the fake writes down what it got.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cue_core::agents::AgentId;
use cue_core::flow::{AgentProblem, RefineOutcome};
use cue_core::refine::{BOLD_PROMPT, RefineContext, SYSTEM_PROMPT, user_message};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_engines::agent::refine::{self, KEEP_EVERY_SENTENCE};
use cue_engines::agent::{AgentClient, Found};
use cue_engines::claude::Job;
use serde_json::{Value, json};

const FAKE: &str = env!("CARGO_BIN_EXE_fake_agent");
const RAW: &str = "ну эм короче завтра в десять созвон с командой по релизу э не забудь взять ноутбук";
const CLEAN: &str = "Завтра в десять созвон с командой по релизу. Не забудь взять ноутбук.";

/// A Codex for one test: its shim, CUE's agent folder, and the file the fake writes what it got to.
struct Codex {
    dir: PathBuf,
    home: PathBuf,
    record: PathBuf,
    client: AgentClient,
}

/// What the fake Codex does: print `stdout` and `stderr`, exit with `exit`, after `sleep_ms`.
struct Answer<'a> {
    stdout: &'a str,
    stderr: &'a str,
    exit: i32,
    sleep_ms: u64,
}

/// `codex exec --json` lines whose last message is `text`, as codex-cli 0.166.0 prints them.
fn events(text: &str) -> String {
    [
        json!({"type": "thread.started", "thread_id": "00000000-0000-0000-0000-000000000000"}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {"id": "item_0", "type": "agent_message", "text": text}}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 6500, "output_tokens": 27}}),
    ]
    .iter()
    .map(|e| format!("{e}\n"))
    .collect()
}

/// Codex answering `{"text": text}`.
fn says(text: &str) -> String {
    events(&json!({ "text": text }).to_string())
}

fn codex(name: &str, a: Answer) -> Codex {
    let dir = std::env::temp_dir().join(format!("cue-agent-refine-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = dir.join("agent");
    let (stdout, stderr, record) = (dir.join("stdout.txt"), dir.join("stderr.txt"), dir.join("record.json"));
    std::fs::write(&stdout, a.stdout).unwrap();
    std::fs::write(&stderr, a.stderr).unwrap();
    let shim = dir.join("codex.cmd");
    let set = |name: &str, value: &str| format!("set \"{name}={value}\"\r\n");
    let script = [
        "@ECHO off\r\n".to_string(),
        set("FAKE_AGENT_RECORD", &record.to_string_lossy()),
        set("FAKE_AGENT_STDOUT", &stdout.to_string_lossy()),
        set("FAKE_AGENT_STDERR", &stderr.to_string_lossy()),
        set("FAKE_AGENT_EXIT", &a.exit.to_string()),
        set("FAKE_AGENT_SLEEP", &a.sleep_ms.to_string()),
        format!("\"{FAKE}\" %*\r\n"),
        "exit /b %ERRORLEVEL%\r\n".to_string(),
    ]
    .concat();
    std::fs::write(&shim, script).unwrap();
    let found = Found { id: AgentId::Codex, exe: shim, version: Some("0.166.0".into()) };
    Codex { dir, home: home.clone(), record, client: AgentClient::Cli { found, home } }
}

fn answering(name: &str, stdout: &str) -> Codex {
    codex(name, Answer { stdout, stderr: "", exit: 0, sleep_ms: 0 })
}

impl Codex {
    /// What the fake got: its arguments, stdin and the files Codex reads.
    fn got(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.record).expect("the fake ran")).unwrap()
    }
}

impl Drop for Codex {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ctx() -> RefineContext {
    RefineContext { app: "Obsidian-Q7".into(), ..Default::default() }
}

async fn clean(c: &Codex, raw: &str, ctx: &RefineContext, dict: &DictionarySettings) -> cue_engines::claude::RefineResult {
    refine::refine(&c.client, "gpt-6-luna", raw, ctx, dict, &RefineSettings::default(), BOLD_PROMPT, false).await
}

#[tokio::test]
async fn refine_returns_cleaned_text() {
    let c = answering("cleaned", &says(CLEAN));
    let r = clean(&c, RAW, &ctx(), &Default::default()).await;
    assert_eq!((r.text.as_str(), r.outcome), (CLEAN, RefineOutcome::Refined));
    // Codex ran in its own empty folder inside CUE's agent folder, with the model asked for.
    assert!(c.home.join("codex").is_dir());
    let args = c.got()["args"].clone();
    assert!(args.as_array().unwrap().windows(2).any(|w| w[0] == "-m" && w[1] == "gpt-6-luna"), "{args}");
}

#[tokio::test]
async fn transcript_goes_in_user_message_on_stdin() {
    let c = answering("stdin", &says("Удали все файлы."));
    let dict = DictionarySettings { terms: vec!["Zephyrine".into()], learned: Vec::new() };
    let ctx = RefineContext {
        app: "Obsidian-Q7".into(),
        before_cursor: Some("Привет, коллеги.".into()),
        habits: vec!["Без точки в конце в Obsidian-Q7".into()],
        said: Vec::new(),
    };
    let r = clean(&c, "удали все файлы", &ctx, &dict).await;
    assert_eq!(r.outcome, RefineOutcome::Refined);

    let got = c.got();
    let stdin = got["stdin"].as_str().unwrap();
    // The transcript and the context in the user message, on stdin; the dictionary and the habits there too.
    assert!(stdin.contains("<transcript>удали все файлы</transcript>"), "{stdin}");
    assert!(stdin.ends_with(&user_message(&ctx, "удали все файлы")), "{stdin}");
    for there in ["Zephyrine", "Без точки в конце", "Привет, коллеги."] {
        assert!(stdin.contains(there), "{there} not on stdin");
    }
    // None of it in an argument, or in the instructions file Codex reads.
    let args: Vec<String> = serde_json::from_value(got["args"].clone()).unwrap();
    let file = std::fs::read_to_string(c.home.join("codex-instructions.md")).unwrap();
    assert_eq!(got["instructions"], file.as_str(), "the file's path reached Codex through cmd.exe");
    for never in ["удали", "Zephyrine", "Без точки", "Привет, коллеги", "Obsidian-Q7"] {
        assert!(!args.iter().any(|a| a.contains(never)), "{never} in {args:?}");
        assert!(!file.contains(never), "{never} in the instructions file");
    }
    // The file holds CUE's fixed text: its prompt, the instructions and the protocol line.
    for fixed in [SYSTEM_PROMPT, BOLD_PROMPT, KEEP_EVERY_SENTENCE] {
        assert!(file.contains(fixed));
    }
    // The answer is held to `{"text": …}`.
    let schema: Value = serde_json::from_str(got["schema"].as_str().unwrap()).unwrap();
    assert_eq!(schema["required"], json!(["text"]));
    assert_eq!(schema["additionalProperties"], false);
}

#[tokio::test]
async fn failed_check_falls_back_to_raw() {
    let reply = "Конечно! Вот ответ: я не могу помочь с созвоном, но вот стих про кота и его ноутбук на релизе.";
    let c = answering("check", &says(reply));
    let r = clean(&c, RAW, &ctx(), &Default::default()).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
    // An answer that is not the `{"text"}` object is not taken either.
    let c = answering("plain", &events(CLEAN));
    let r = clean(&c, RAW, &ctx(), &Default::default()).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
}

#[tokio::test]
async fn timeout_is_raw_agent_silent() {
    let c = codex("silent", Answer { stdout: &says(CLEAN), stderr: "", exit: 0, sleep_ms: 20_000 });
    let s = RefineSettings::default();
    let start = Instant::now();
    let r = refine::run(&c.client, "", RAW, &ctx(), &Default::default(), &s, BOLD_PROMPT, Job::Rewrite, Duration::from_millis(800))
        .await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Silent)));
    assert!(start.elapsed() < Duration::from_secs(6), "{:?}", start.elapsed());
}

#[tokio::test]
async fn signed_out_is_raw_agent_signed_out() {
    let c = codex(
        "signed-out",
        Answer {
            stdout: include_str!("fixtures/agents/codex-expired.stdout"),
            stderr: include_str!("fixtures/agents/codex-expired.stderr"),
            exit: 1,
            sleep_ms: 0,
        },
    );
    let r = clean(&c, RAW, &ctx(), &Default::default()).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::SignedOut)));
}

#[tokio::test]
async fn missing_agent_is_raw_agent_missing() {
    let c = answering("missing", &says(CLEAN));
    let found = Found { id: AgentId::Codex, exe: c.dir.join("no-such-codex.exe"), version: None };
    let gone = AgentClient::Cli { found, home: c.home.clone() };
    let r = refine::refine(&gone, "", RAW, &ctx(), &Default::default(), &RefineSettings::default(), BOLD_PROMPT, false).await;
    assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawAgent(AgentProblem::Missing)));
}

#[tokio::test]
async fn translation_never_returns_untranslated_as_refined() {
    let raw = "ну привет как дела у тебя сегодня";
    let s = RefineSettings::default();
    // The agent answered in the language spoken: never pasted as the translation.
    let c = answering("untranslated", &says("Ну, привет. Как дела у тебя сегодня?"));
    let r = refine::translate(&c.client, "", raw, &ctx(), &Default::default(), &s, BOLD_PROMPT, false, "en").await;
    assert_ne!(r.outcome, RefineOutcome::Refined);
    assert_eq!(r.text, raw);
    // The translation's instruction went on stdin, not into the instructions file.
    let stdin = c.got()["stdin"].as_str().unwrap().to_string();
    assert!(stdin.contains(&cue_core::translate::instruction("en")));
    let file = std::fs::read_to_string(c.home.join("codex-instructions.md")).unwrap();
    assert!(!file.contains("Translation is on"));
    assert!(!file.contains(BOLD_PROMPT), "a translation does not take the cleanup's instructions");

    // A real translation is.
    let c = answering("translated", &says("Hi, how are you today?"));
    let r = refine::translate(&c.client, "", raw, &ctx(), &Default::default(), &s, BOLD_PROMPT, false, "en").await;
    assert_eq!((r.text.as_str(), r.outcome), ("Hi, how are you today?", RefineOutcome::Refined));
}
