//! An "Ask" question through the cleanup's route (docs/specs/2026-11-08-no-keys-design.md, "Ask and translation"): the
//! user's agent, OpenCode or Codex, or the model on this computer. Each gets CUE's fixed instructions
//! (`AGENT_ASK_PROMPT`) and the whole conversation as one user message (`agent_messages`: an agent keeps no session of
//! its own), and answers with one JSON object, `{"answer", "write", "replace", "sources"}`, which becomes the steps of
//! the answer on screen and its end. Codex searches the web when `ask.web_search` is on (live search, its only tool);
//! OpenCode is asked with no tools, and the model here never searches: the card then says "без поиска". No text of the
//! question, the answer or its sources reaches the log.
use std::sync::Arc;
use std::time::Duration;

use cue_core::ask::{AGENT_ASK_PROMPT, AskTurn, agent_messages, openable, site_of};
use cue_core::flow::AgentProblem;
use serde::Deserialize;

use super::{AgentClient, AgentFailure, Prompt};
use crate::ask::{AskEnd, AskFailure, AskResult, AskUsage, Limits, Step};
use crate::local_llm::{ASK_GRAMMAR, Generate};

/// Where a question goes: the user's agent, or the model loaded on this computer.
#[derive(Clone)]
pub enum AskRoute {
    Agent(AgentClient),
    Local(Arc<dyn Generate>),
}

impl AskRoute {
    /// Whether the question is asked with web search: only when the user wants it and the route can search.
    fn searches(&self, web_search: bool) -> bool {
        web_search && matches!(self, AskRoute::Agent(c) if c.searches())
    }
}

/// How long the question may take: through an agent, its first words (which are the whole answer: an agent brings it
/// whole), longer with web search; on this computer, the whole answer's time, since a model on a processor writes all of
/// it before anything shows.
pub fn wait_for(limits: Limits, route_is_local: bool, searching: bool) -> Duration {
    match (route_is_local, searching) {
        (true, _) => limits.total,
        (false, true) => limits.first_words_search,
        (false, false) => limits.first_words,
    }
}

/// One question (`turns`, the last one the question in flight) along `route`, with `model` (empty: the agent's
/// default), within `limits`. `Step::NoSearch` comes first when the question is answered without web search. Never
/// panics or hangs: every problem ends as `AskEnd::Failed`, with one log line of its kind.
pub async fn run(
    route: AskRoute,
    model: &str,
    web_search: bool,
    turns: &[AskTurn],
    limits: Limits,
    mut on_step: impl FnMut(Step) + Send,
) -> AskResult {
    let searching = route.searches(web_search);
    if !searching {
        on_step(Step::NoSearch);
    }
    let user = agent_messages(turns);
    let reply = match route {
        AskRoute::Agent(c) => {
            let wait = wait_for(limits, false, searching);
            let p = Prompt { system: AGENT_ASK_PROMPT.to_owned(), user, web_search: searching };
            // The agent's own budget, and this one over it: whatever it does, the question ends in time. A request
            // dropped here (or by Esc) ends a running Codex.
            match tokio::time::timeout(wait, c.ask(model, &p, wait)).await {
                Err(_) => Err(AskFailure::Timeout),
                Ok(r) => r.map_err(failure),
            }
        }
        AskRoute::Local(g) => {
            let deadline = std::time::Instant::now() + wait_for(limits, true, false);
            match crate::local::answer(g, AGENT_ASK_PROMPT.to_owned(), user, ASK_GRAMMAR, deadline).await {
                Some(Ok(text)) => Ok((text, 0)),
                Some(Err(crate::EngineError::Timeout)) => Err(AskFailure::Timeout),
                Some(Err(_)) => Err(AskFailure::Failed("local model")),
                None => Err(AskFailure::Failed("local model stopped")),
            }
        }
    };
    match reply {
        Ok((text, searches)) => finish(&text, searches, &mut on_step),
        Err(why) => AskResult::failed(why),
    }
}

/// The answer as the AI wrote it: `answer` and, when it is text for the field, `write` and `replace`; the pages it
/// relies on. Only `answer` is required, so a model that leaves out the rest still answers.
#[derive(Deserialize)]
struct Answer {
    answer: String,
    #[serde(default)]
    write: bool,
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    sources: Vec<Source>,
}

#[derive(Deserialize)]
struct Source {
    url: String,
    #[serde(default)]
    title: String,
}

/// The JSON object of a reply, without a code fence around it.
fn unfenced(text: &str) -> &str {
    let t = text.trim();
    let Some(rest) = t.strip_prefix("```") else { return t };
    let body = rest.split_once('\n').map_or("", |(_, b)| b);
    body.trim_end().strip_suffix("```").unwrap_or(body).trim()
}

/// The steps and the end of a reply `text` from a route that ran `searches` web searches. A reply that is no JSON
/// object at all is an answer as it stands; a broken object, or one with nothing in it, is no answer. Sources count
/// only after a search (one named without a search is made up) and only web pages; one without a title goes under its
/// site. One log line: the kind of end and the counts.
fn finish(text: &str, searches: u32, on_step: &mut (impl FnMut(Step) + ?Sized)) -> AskResult {
    let body = unfenced(text);
    let a = if body.starts_with('{') {
        match serde_json::from_str::<Answer>(body) {
            Ok(a) => a,
            Err(_) => return AskResult::failed(AskFailure::Failed("answer")),
        }
    } else {
        Answer { answer: body.to_owned(), write: false, replace: false, sources: Vec::new() }
    };
    if a.answer.trim().is_empty() {
        return AskResult::failed(AskFailure::Failed("empty"));
    }
    let usage = AskUsage { web_searches: searches, ..AskUsage::default() };
    if a.write {
        tracing::info!(replace = a.replace, searches, "ask: the AI wrote text for the field");
        on_step(Step::WriteStart);
        return AskResult { end: AskEnd::Write { text: a.answer.clone(), replace: a.replace }, text: a.answer, usage };
    }
    on_step(Step::TextStart);
    on_step(Step::Text(a.answer.clone()));
    let sources: Vec<Source> = if searches > 0 { a.sources.into_iter().filter(|s| openable(&s.url)).collect() } else { Vec::new() };
    tracing::info!(sources = sources.len(), searches, "ask: answered");
    for s in sources {
        let title = if s.title.trim().is_empty() { site_of(&s.url) } else { s.title.trim().to_owned() };
        on_step(Step::Cite { url: s.url, title });
    }
    AskResult { end: AskEnd::Answer, text: a.answer, usage }
}

/// An agent's failure as a question's: the ones the user has to act on carry the agent's word.
pub fn failure(f: AgentFailure) -> AskFailure {
    match f {
        AgentFailure::NotInstalled => AskFailure::Agent(AgentProblem::Missing),
        AgentFailure::SignedOut => AskFailure::Agent(AgentProblem::SignedOut),
        AgentFailure::Timeout => AskFailure::Timeout,
        AgentFailure::Failed(why) => AskFailure::Failed(why),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use cue_core::ask::{AnswerAction, SelectionKind, WriteTarget, answer_actions, first_message, follow_up_message, write_target};
    use cue_core::flow::PasteHow;
    use reqwest::Url;

    use super::*;
    use crate::EngineError;
    use crate::agent::opencode::Server;
    use crate::testing::{Reply, fake_api, reply};

    /// A model on this computer that answers `answer`, and keeps what it was asked.
    struct Here {
        answer: Result<String, EngineError>,
        asked: Mutex<Option<(String, String, String)>>,
    }

    impl Here {
        fn new(answer: Result<&str, EngineError>) -> Arc<Here> {
            Arc::new(Here { answer: answer.map(str::to_owned), asked: Mutex::new(None) })
        }

        fn asked(&self) -> (String, String, String) {
            self.asked.lock().unwrap().clone().expect("asked")
        }
    }

    impl Generate for Here {
        fn generate(&self, system: &str, user: &str, grammar: &str, _: Instant, _: &AtomicBool) -> Result<String, EngineError> {
            *self.asked.lock().unwrap() = Some((system.into(), user.into(), grammar.into()));
            self.answer.clone()
        }
        fn id(&self) -> &str {
            "llm--here--model--q4_k_m"
        }
    }

    fn question() -> Vec<AskTurn> {
        vec![AskTurn::user("<question>что такое HTTP 418</question>\n<app>Notepad</app>\n<today>2026-11-09</today>")]
    }

    async fn ask(route: AskRoute, search: bool, turns: &[AskTurn]) -> (AskResult, Vec<Step>) {
        let mut steps = Vec::new();
        let r = run(route, "", search, turns, Limits::SPEC, |s| steps.push(s)).await;
        (r, steps)
    }

    const V2_INFO: &str = r#"{"version":"0.0.0-dev-19272","pid":42,"urls":[],"paths":{"tmp":"x"}}"#;

    /// An OpenCode V2 server whose stateless generation answers `text` (or hangs); its requests are kept.
    async fn opencode(text: Option<&'static str>) -> (AskRoute, crate::testing::FakeApi) {
        let api = fake_api(move |method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => match text {
                Some(t) => reply(200, &serde_json::json!({ "data": { "text": t } }).to_string()),
                None => Reply::Hang,
            },
            _ => reply(500, "{}"),
        })
        .await;
        let server = Server::running_at(Url::parse(&api.base).unwrap(), None, None);
        (AskRoute::Agent(AgentClient::OpenCode(Arc::new(server))), api)
    }

    fn sent_prompt(api: &crate::testing::FakeApi) -> String {
        let body: serde_json::Value = serde_json::from_slice(&api.body(2)).unwrap();
        body["prompt"].as_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn answer_streams_as_text_steps() {
        let (route, api) = opencode(Some(r#"{"answer":"**I'm a teapot.**","write":false,"replace":false,"sources":[]}"#)).await;
        let (r, steps) = ask(route, false, &question()).await;
        assert_eq!(steps, [Step::NoSearch, Step::TextStart, Step::Text("**I'm a teapot.**".into())]);
        assert_eq!((r.end, r.text.as_str()), (AskEnd::Answer, "**I'm a teapot.**"));
        assert_eq!(r.usage, AskUsage::default());
        // CUE's instructions, then the question as the user message.
        let prompt = sent_prompt(&api);
        assert!(prompt.starts_with(AGENT_ASK_PROMPT), "{prompt}");
        assert!(prompt.ends_with(&question()[0].text));
    }

    #[tokio::test]
    async fn write_true_ends_as_write() {
        let here = Here::new(Ok(r#"{"answer":"Пришлю отчёт в пятницу.","write":true,"replace":false,"sources":[]}"#));
        let (r, steps) = ask(AskRoute::Local(here), false, &question()).await;
        assert_eq!(steps, [Step::NoSearch, Step::WriteStart], "a write never shows as an answer's words");
        assert!(Step::WriteStart.is_first_words());
        assert_eq!(r.end, AskEnd::Write { text: "Пришлю отчёт в пятницу.".into(), replace: false });
        assert_eq!(r.text, "Пришлю отчёт в пятницу.", "the conversation keeps what was written");
        // A write with no text is no write: nothing to show.
        let blank = Here::new(Ok(r#"{"answer":"  ","write":true,"replace":false,"sources":[]}"#));
        assert_eq!(ask(AskRoute::Local(blank), false, &question()).await.0.end, AskEnd::Failed(AskFailure::Failed("empty")));
    }

    /// `replace` writes over the selection, and the card offers Replace over a writable one, as before.
    #[tokio::test]
    async fn replace_true_targets_selection() {
        let here = Here::new(Ok(r#"{"answer":"Fixed text.","write":true,"replace":true,"sources":[]}"#));
        let (r, _) = ask(AskRoute::Local(here), false, &question()).await;
        assert_eq!(r.end, AskEnd::Write { text: "Fixed text.".into(), replace: true });
        assert_eq!(write_target(true, SelectionKind::Writable, false, false), WriteTarget::Paste(PasteHow::Replace));
        assert_eq!(answer_actions(SelectionKind::Writable, false, false), [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy]);
    }

    /// The sources of a searched answer follow its words, one `Cite` each, a title-less one under its site; only web
    /// pages, and only when the AI did search: a source it names without a search is made up.
    #[test]
    fn sources_become_cite_steps() {
        let text = r#"{"answer":"Rust 1.95.","write":false,"replace":false,"sources":[
            {"url":"https://blog.rust-lang.org/","title":"Rust Blog"},
            {"url":"https://www.example.com/a","title":" "},
            {"url":"file:///C:/x.exe","title":"Bad"}]}"#;
        let mut steps = Vec::new();
        let r = finish(text, 1, &mut |s| steps.push(s));
        assert_eq!(
            steps,
            [
                Step::TextStart,
                Step::Text("Rust 1.95.".into()),
                Step::Cite { url: "https://blog.rust-lang.org/".into(), title: "Rust Blog".into() },
                Step::Cite { url: "https://www.example.com/a".into(), title: "example.com".into() },
            ]
        );
        assert_eq!((r.end, r.usage.web_searches), (AskEnd::Answer, 1));
        let mut unsearched = Vec::new();
        finish(text, 0, &mut |s| unsearched.push(s));
        assert_eq!(unsearched, [Step::TextStart, Step::Text("Rust 1.95.".into())], "no search, no sources");
    }

    /// What an AI may write around its JSON: a code fence is taken off; plain text with no JSON at all is an answer;
    /// a broken object is no answer.
    #[test]
    fn the_answer_is_read_leniently() {
        let mut none = |_| {};
        let fenced = "```json\n{\"answer\":\"Да.\",\"write\":false,\"replace\":false,\"sources\":[]}\n```";
        assert_eq!(finish(fenced, 0, &mut none).text, "Да.");
        let plain = finish("Просто ответ.", 0, &mut none);
        assert_eq!((plain.end, plain.text.as_str()), (AskEnd::Answer, "Просто ответ."));
        assert_eq!(finish("{\"answer\": \"обрыв", 0, &mut none).end, AskEnd::Failed(AskFailure::Failed("answer")));
        assert_eq!(finish("  ", 0, &mut none).end, AskEnd::Failed(AskFailure::Failed("empty")));
        // Only `answer` is needed.
        assert_eq!(finish("{\"answer\":\"Да.\"}", 0, &mut none).end, AskEnd::Answer);
    }

    #[test]
    fn deadlines_are_limits_spec() {
        assert_eq!(
            Limits::SPEC,
            Limits { first_words: Duration::from_secs(15), first_words_search: Duration::from_secs(20), total: Duration::from_secs(30) }
        );
        let later = Limits::SPEC.later_by(Duration::from_secs(5));
        assert_eq!((later.first_words, later.first_words_search, later.total), (Duration::from_secs(20), Duration::from_secs(25), Duration::from_secs(30)));
    }

    /// Through an agent the first words come within 15 s, 20 s with search; on this computer the whole answer's 30 s.
    #[tokio::test]
    async fn first_words_deadline_is_15s_and_20s_with_search() {
        assert_eq!(wait_for(Limits::SPEC, false, false), Duration::from_secs(15));
        assert_eq!(wait_for(Limits::SPEC, false, true), Duration::from_secs(20));
        assert_eq!(wait_for(Limits::SPEC, true, false), Duration::from_secs(30));
        // An agent that never answers: a timeout at the first-words deadline, not later.
        let (route, _api) = opencode(None).await;
        let short = Limits { first_words: Duration::from_millis(300), first_words_search: Duration::from_secs(30), total: Duration::from_secs(30) };
        let start = Instant::now();
        let r = run(route, "", true, &question(), short, |_| {}).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Timeout));
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
    }

    /// OpenCode's API does not say whether a model can search, so it is asked with no tools: "без поиска" first, even
    /// with web search on, and any sources it names are dropped.
    #[tokio::test]
    async fn opencode_without_search_tool_says_no_search() {
        let said = r#"{"answer":"Сейчас +18.","write":false,"replace":false,"sources":[{"url":"https://a.io/","title":"A"}]}"#;
        let (route, api) = opencode(Some(said)).await;
        let (r, steps) = ask(route, true, &question()).await;
        assert_eq!(steps.first(), Some(&Step::NoSearch));
        assert!(!steps.iter().any(|s| matches!(s, Step::Cite { .. } | Step::Searching)), "{steps:?}");
        assert_eq!((r.end, r.usage.web_searches), (AskEnd::Answer, 0));
        assert_eq!(api.calls().len(), 3, "the probes, then one stateless generation (no tools)");
    }

    /// The model on this computer never searches: "без поиска" first, its answer held to Ask's grammar, the same
    /// instructions and conversation an agent gets.
    #[tokio::test]
    async fn local_route_never_searches() {
        let here = Here::new(Ok(r#"{"answer":"Чайник.","write":false,"replace":false,"sources":[]}"#));
        let (r, steps) = ask(AskRoute::Local(here.clone()), true, &question()).await;
        assert_eq!(steps, [Step::NoSearch, Step::TextStart, Step::Text("Чайник.".into())]);
        assert_eq!(r.end, AskEnd::Answer);
        let (system, user, grammar) = here.asked();
        assert_eq!((system.as_str(), user.as_str(), grammar.as_str()), (AGENT_ASK_PROMPT, question()[0].text.as_str(), ASK_GRAMMAR));
        // Its own failures: a timeout is a timeout; anything else a failure in a word.
        let late = Here::new(Err(EngineError::Timeout));
        assert_eq!(ask(AskRoute::Local(late), false, &question()).await.0.end, AskEnd::Failed(AskFailure::Timeout));
        let broken = Here::new(Err(EngineError::Llm("x".into())));
        assert_eq!(ask(AskRoute::Local(broken), false, &question()).await.0.end, AskEnd::Failed(AskFailure::Failed("local model")));
    }

    /// A follow-up sends the whole conversation again: the first question with its context, the answer, the follow-up.
    #[tokio::test]
    async fn follow_up_carries_previous_turns() {
        let ctx = cue_core::ask::AskContext {
            app: "Telegram Desktop".into(),
            before_cursor: None,
            selection: Some("Нужно выбрать оркестратор.".into()),
            today: "2026-11-09".into(),
        };
        let turns = vec![
            AskTurn::user(first_message("объясни", &ctx)),
            AskTurn::assistant("Оркестратор управляет контейнерами."),
            AskTurn::user(follow_up_message("а короче")),
        ];
        let (route, api) = opencode(Some(r#"{"answer":"Управляет контейнерами.","write":false,"replace":false,"sources":[]}"#)).await;
        let (r, _) = ask(route, false, &turns).await;
        assert_eq!(r.text, "Управляет контейнерами.");
        let prompt = sent_prompt(&api);
        assert!(prompt.ends_with(&agent_messages(&turns)), "{prompt}");
        for part in ["<selection>Нужно выбрать оркестратор.</selection>", "<your_answer>\nОркестратор управляет контейнерами.", "<question>а короче</question>"] {
            assert!(prompt.contains(part), "{part}");
        }
    }

    /// The log gets kinds and counts: never the question, the answer, a written text or a source's URL or title.
    #[tokio::test]
    async fn the_log_holds_no_question_answer_or_url() {
        let (lines, _guard) = crate::testing::capture_logs();
        let mut none = |_| {};
        finish(r#"{"answer":"Секретный ответ","sources":[{"url":"https://secret.example/page","title":"Тайна"}]}"#, 1, &mut none);
        finish(r#"{"answer":"Секретный текст","write":true,"replace":true}"#, 0, &mut none);
        finish("{\"answer\": \"Секретный обрыв", 0, &mut none);
        let here = Here::new(Ok(r#"{"answer":"Секретный локальный","write":false,"replace":false,"sources":[]}"#));
        let turns = vec![AskTurn::user("<question>секретный вопрос</question>")];
        run(AskRoute::Local(here), "", true, &turns, Limits::SPEC, |_| {}).await;
        let lines = lines.lock().unwrap().clone();
        assert!(!lines.is_empty(), "{lines:?}");
        for line in &lines {
            for never in ["Секрет", "секрет", "secret.example", "Тайна"] {
                assert!(!line.contains(never), "{line}");
            }
        }
    }

    /// An agent's failures: the ones the user has to act on carry its word; a Codex that is not there is missing.
    #[tokio::test]
    async fn agent_failures_map_to_the_agents_word() {
        assert_eq!(failure(AgentFailure::NotInstalled), AskFailure::Agent(AgentProblem::Missing));
        assert_eq!(failure(AgentFailure::SignedOut), AskFailure::Agent(AgentProblem::SignedOut));
        assert_eq!(failure(AgentFailure::Timeout), AskFailure::Timeout);
        assert_eq!(failure(AgentFailure::Failed("exit")), AskFailure::Failed("exit"));
        // A Codex that is not there.
        let home = std::env::temp_dir().join(format!("cue-agent-ask-{}-missing", std::process::id()));
        let found = super::super::Found { id: cue_core::agents::AgentId::Codex, exe: PathBuf::from(&home).join("codex.exe"), version: None };
        let route = AskRoute::Agent(AgentClient::Cli { found, home: home.clone() });
        let (r, steps) = ask(route, true, &question()).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Agent(AgentProblem::Missing)));
        assert!(!steps.contains(&Step::NoSearch), "Codex was to search");
        let _ = std::fs::remove_dir_all(&home);
    }
}
