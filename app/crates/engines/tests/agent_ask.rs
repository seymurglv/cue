//! `agent::ask` over Codex's stand-in (`src/bin/fake_agent.rs`), started through a `.cmd` shim as the real `codex.cmd`
//! is: no network, no real agent. The shim tells the fake what to answer; the fake writes down what it got.
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cue_core::agents::AgentId;
use cue_core::ask::{AGENT_ASK_PROMPT, AskTurn, agent_messages};
use cue_core::flow::AgentProblem;
use cue_engines::agent::ask::{AskRoute, run};
use cue_engines::agent::{AgentClient, Found};
use cue_engines::ask::{AskEnd, AskFailure, AskResult, Limits, Step};
use serde_json::{Value, json};

const FAKE: &str = env!("CARGO_BIN_EXE_fake_agent");

/// A Codex for one test: its shim, CUE's agent folder, and the file the fake writes what it got to.
struct Codex {
    dir: PathBuf,
    home: PathBuf,
    record: PathBuf,
    client: AgentClient,
}

/// `codex exec --json` lines: the given items, then the answer `text` as its last message.
fn events(items: &[Value], text: &str) -> String {
    let mut all = vec![json!({"type": "thread.started", "thread_id": "00000000-0000-0000-0000-000000000000"}), json!({"type": "turn.started"})];
    for item in items {
        all.push(json!({"type": "item.started", "item": item}));
        all.push(json!({"type": "item.completed", "item": item}));
    }
    all.push(json!({"type": "item.completed", "item": {"id": "item_9", "type": "agent_message", "text": text}}));
    all.push(json!({"type": "turn.completed", "usage": {"input_tokens": 6500, "output_tokens": 27}}));
    all.iter().map(|e| format!("{e}\n")).collect()
}

fn codex(name: &str, stdout: &str, exit: i32, sleep_ms: u64) -> Codex {
    let dir = std::env::temp_dir().join(format!("cue-agent-ask-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let home = dir.join("agent");
    let (out, record) = (dir.join("stdout.txt"), dir.join("record.json"));
    std::fs::write(&out, stdout).unwrap();
    let set = |name: &str, value: &str| format!("set \"{name}={value}\"\r\n");
    let script = [
        "@ECHO off\r\n".to_string(),
        set("FAKE_AGENT_RECORD", &record.to_string_lossy()),
        set("FAKE_AGENT_STDOUT", &out.to_string_lossy()),
        set("FAKE_AGENT_EXIT", &exit.to_string()),
        set("FAKE_AGENT_SLEEP", &sleep_ms.to_string()),
        format!("\"{FAKE}\" %*\r\n"),
        "exit /b %ERRORLEVEL%\r\n".to_string(),
    ]
    .concat();
    let shim = dir.join("codex.cmd");
    std::fs::write(&shim, script).unwrap();
    let found = Found { id: AgentId::Codex, exe: shim, version: Some("0.166.0".into()) };
    Codex { dir, home: home.clone(), record, client: AgentClient::Cli { found, home } }
}

impl Codex {
    fn got(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.record).expect("the fake ran")).unwrap()
    }

    fn args(&self) -> Vec<String> {
        serde_json::from_value(self.got()["args"].clone()).unwrap()
    }
}

impl Drop for Codex {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn has_pair(args: &[String], a: &str, b: &str) -> bool {
    args.windows(2).any(|w| w[0] == a && w[1] == b)
}

fn turns() -> Vec<AskTurn> {
    vec![AskTurn::user("<question>какая последняя версия Rust</question>\n<app>Notepad</app>\n<today>2026-11-09</today>")]
}

async fn ask(c: &Codex, search: bool, limits: Limits) -> (AskResult, Vec<Step>) {
    let mut steps = Vec::new();
    let r = run(AskRoute::Agent(c.client.clone()), "gpt-6-luna", search, &turns(), limits, |s| steps.push(s)).await;
    (r, steps)
}

const SEARCHED: &str = r#"{"answer":"Последняя версия — **1.95**.","write":false,"replace":false,"sources":[{"url":"https://blog.rust-lang.org/","title":"Rust Blog"}]}"#;

fn searches() -> Vec<Value> {
    vec![json!({"id": "item_1", "type": "web_search", "query": "rust latest version"})]
}

/// With web search on, Codex gets live search (its only tool), its search items are allowed, and the answer's sources
/// become the card's; nothing says "без поиска".
#[tokio::test]
async fn codex_searches_when_asked() {
    let c = codex("search", &events(&searches(), SEARCHED), 0, 0);
    let (r, steps) = ask(&c, true, Limits::SPEC).await;
    assert_eq!(
        steps,
        [
            Step::TextStart,
            Step::Text("Последняя версия — **1.95**.".into()),
            Step::Cite { url: "https://blog.rust-lang.org/".into(), title: "Rust Blog".into() },
        ]
    );
    assert_eq!((r.end, r.usage.web_searches), (AskEnd::Answer, 1));
    let args = c.args();
    assert!(has_pair(&args, "-c", "web_search=live"), "{args:?}");
    for feature in ["shell_tool", "unified_exec", "apps", "plugins", "browser_use", "multi_agent"] {
        assert!(has_pair(&args, "--disable", feature), "{feature} stays off");
    }
    assert!(has_pair(&args, "-s", "read-only"));

    // Off: no search, "без поиска" first, and a search item in the events refuses the answer.
    let c = codex("no-search", &events(&[], SEARCHED), 0, 0);
    let (r, steps) = ask(&c, false, Limits::SPEC).await;
    assert_eq!(steps.first(), Some(&Step::NoSearch));
    assert!(!steps.iter().any(|s| matches!(s, Step::Cite { .. })), "no search, no sources: {steps:?}");
    assert_eq!(r.end, AskEnd::Answer);
    assert!(has_pair(&c.args(), "-c", "web_search=disabled"));
    let c = codex("unasked-search", &events(&searches(), SEARCHED), 0, 0);
    assert_eq!(ask(&c, false, Limits::SPEC).await.0.end, AskEnd::Failed(AskFailure::Failed("no answer")));
}

/// A searched answer is still refused when any other tool ran.
#[tokio::test]
async fn codex_search_refuses_any_other_tool() {
    let mut items = searches();
    items.push(json!({"id": "item_2", "type": "command_execution", "command": "dir", "status": "completed"}));
    let c = codex("other-tool", &events(&items, SEARCHED), 0, 0);
    assert_eq!(ask(&c, true, Limits::SPEC).await.0.end, AskEnd::Failed(AskFailure::Failed("no answer")));
}

/// CUE's fixed instructions go in Codex's own instructions file and the answer's shape in its own schema file; the
/// question, its context and the earlier turns go on stdin only.
#[tokio::test]
async fn codex_ask_keeps_the_question_on_stdin() {
    let c = codex("stdin", &events(&[], r#"{"answer":"Пришлю в пятницу.","write":true,"replace":true,"sources":[]}"#), 0, 0);
    let (r, steps) = ask(&c, false, Limits::SPEC).await;
    assert_eq!(steps, [Step::NoSearch, Step::WriteStart]);
    assert_eq!(r.end, AskEnd::Write { text: "Пришлю в пятницу.".into(), replace: true });
    let got = c.got();
    assert_eq!(got["instructions"], AGENT_ASK_PROMPT);
    assert_eq!(got["stdin"], agent_messages(&turns()).as_str());
    let schema: Value = serde_json::from_str(got["schema"].as_str().expect("the schema file was read")).unwrap();
    assert_eq!(schema["required"], json!(["answer", "write", "replace", "sources"]));
    let args = c.args();
    assert!(args.iter().any(|a| a.contains("codex-ask-schema.json")), "{args:?}");
    for a in &args {
        assert!(!a.contains("Rust") && !a.contains("Notepad"), "{a:?} carries the question");
    }
    let instructions = args.iter().find_map(|a| a.strip_prefix("model_instructions_file=")).unwrap();
    assert!(instructions.contains("codex-instructions-"), "{instructions}");
    assert!(c.home.join("codex").is_dir(), "Codex ran in its own empty folder");
}

/// Codex says it is signed out: the question ends with the agent's word, which the Drop shows.
#[tokio::test]
async fn a_signed_out_codex_ends_with_its_word() {
    let out = "{\"type\":\"error\",\"message\":\"Please log out and sign in again.\"}\n{\"type\":\"turn.failed\",\"error\":{\"message\":\"sign in again\"}}\n";
    let c = codex("signed-out", out, 1, 0);
    assert_eq!(ask(&c, true, Limits::SPEC).await.0.end, AskEnd::Failed(AskFailure::Agent(AgentProblem::SignedOut)));
}

/// The first words through Codex: within the first-words deadline without search, the longer one with it.
#[tokio::test]
async fn codex_waits_longer_for_a_searched_answer() {
    let short = Limits { first_words: Duration::from_millis(400), first_words_search: Duration::from_secs(20), total: Duration::from_secs(30) };
    let c = codex("slow", &events(&searches(), SEARCHED), 0, 1_500);
    let start = Instant::now();
    assert_eq!(ask(&c, false, short).await.0.end, AskEnd::Failed(AskFailure::Timeout));
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
    assert_eq!(ask(&c, true, short).await.0.end, AskEnd::Answer, "with search, 20 s");
}
