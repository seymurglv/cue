//! The cleanup over an OpenAI-compatible Chat Completions service (docs/specs/2026-11-02-refine-connections.md): OpenAI,
//! Google Gemini, DeepSeek, Mistral, Groq, xAI and the local Ollama and LM Studio. What each needs in a request is in
//! `cue_core::connections`; the stream is read the way OpenRouter's is ([`openrouter::stream`]), and the answer goes
//! through the checks Claude's does ([`claude::settle`]). Never "fails": on any problem the raw text comes back.
//!
//! The key goes only in the `Authorization` header (a local server gets none) and never into a log or an error.
use std::time::Duration;

use cue_core::connections::{Answer, CacheHint, Connection};
use cue_core::flow::RefineOutcome;
use cue_core::refine::{max_tokens, timeout_ms, user_message, RefineContext};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_core::translate;
use serde_json::{json, Value};

use crate::check;
use crate::claude::{self, Job, RefineResult};
use crate::error::EngineError;
use crate::openrouter::{self, parse_content, REASONING_HEADROOM};

/// Said after the prompt where the service holds the answer to a JSON object but not to the schema.
pub const OBJECT_NOTE: &str = "Answer with a JSON object that has exactly one field, \"text\": the cleaned text.";

/// What CUE names its prompt by where a service caches by a name (Mistral's key, xAI's conversation header).
const CACHE_NAME: &str = "cue-cleanup";

/// A local server may still be loading the model into memory: it gets this much more time.
const LOCAL_TIME_HEADROOM_MS: u64 = 10_000;

/// The request body for the connection `c` and its model `model`: the system blocks Claude gets, joined into one system
/// message (`custom` — the custom style's instructions), the transcript, a streamed answer in the `{"text": …}` object.
#[allow(clippy::too_many_arguments)]
pub fn request_body(
    c: &Connection,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    job: Job,
    custom: Option<&str>,
) -> Value {
    let mut budget = max_tokens(raw) + if c.reasons { REASONING_HEADROOM } else { 0 };
    if let Job::Translate(_) = job {
        budget = claude::translation_budget(budget);
    }
    let blocks = claude::system_blocks(dict, &ctx.habits, job, custom);
    let mut texts: Vec<&str> = blocks.as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect();
    if c.answer == Answer::Object {
        texts.push(OBJECT_NOTE);
    }
    let response_format = match c.answer {
        Answer::Schema { strict } => json!({
            "type": "json_schema",
            "json_schema": {
                "name": "refined_text",
                "strict": strict,
                "schema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }
        }),
        Answer::Object => json!({ "type": "json_object" }),
    };
    let mut v = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": texts.join("\n\n") },
            { "role": "user", "content": user_message(ctx, raw) }
        ],
        "stream": true,
        "response_format": response_format,
    });
    v[if c.completion_tokens { "max_completion_tokens" } else { "max_tokens" }] = json!(budget);
    if let Some(effort) = c.reasoning {
        v["reasoning_effort"] = json!(effort);
    }
    if c.include_usage {
        v["stream_options"] = json!({ "include_usage": true });
    }
    if c.cache_hint == Some(CacheHint::BodyKey) {
        v["prompt_cache_key"] = json!(CACHE_NAME);
    }
    v
}

/// Request timeout: the cleanup's or the translation's by text length, plus headroom where the model must reason first
/// and where the server is on this computer.
fn request_timeout_ms(c: &Connection, raw: &str, job: Job) -> u64 {
    let base = match job {
        Job::Clean | Job::Rewrite => timeout_ms(raw),
        Job::Translate(_) => translate::timeout_ms(raw),
    };
    base + if c.reasons { claude::THINKING_TIME_HEADROOM_MS } else { 0 } + if c.local { LOCAL_TIME_HEADROOM_MS } else { 0 }
}

/// A POST to the service's chat completions with the key (if there is one) and the connection's own headers.
pub(crate) fn post(http: &reqwest::Client, c: &Connection, address: &str, key: &str) -> reqwest::RequestBuilder {
    let mut req = http.post(format!("{address}/chat/completions"));
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    if c.cache_hint == Some(CacheHint::GrokHeader) {
        req = req.header("x-grok-conv-id", CACHE_NAME);
    }
    req
}

/// One cleanup over the connection and model of `s`, in its style.
pub async fn refine(http: &reqwest::Client, key: Option<&str>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings) -> RefineResult {
    refine_job(http, key, raw, ctx, dict, s, Job::Rewrite).await
}

/// One translation over the connection and model of `s` (see [`claude::translate`]).
pub async fn translate(
    http: &reqwest::Client,
    key: Option<&str>,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    to: &'static str,
) -> RefineResult {
    refine_job(http, key, raw, ctx, dict, s, Job::Translate(to)).await
}

/// [`refine`] or [`translate`]; the address is the connection's in `s` (tests point it at their own service).
async fn refine_job(
    http: &reqwest::Client,
    key: Option<&str>,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    job: Job,
) -> RefineResult {
    let c = s.connection();
    let key = match key {
        Some(k) => k,
        // A local server needs no key.
        None if c.local => "",
        None => {
            return claude::raw_fallback(raw, RefineOutcome::RawKeyProblem, claude::Usage::default(), &format!("no {} key", c.name));
        }
    };
    let timeout = request_timeout_ms(c, raw, job);
    let body = request_body(c, s.model().id, raw, ctx, dict, job, s.custom_text());
    // As for Claude: a failure worth it is asked once more, with twice the time.
    let mut why = String::new();
    let mut last = None;
    for ms in [timeout, timeout * claude::RETRY_TIMEOUT_FACTOR] {
        if !why.is_empty() {
            tracing::info!("refine: {why}, asking again");
        }
        match openrouter::stream(post(http, c, s.address(), key), key, &body, Duration::from_millis(ms)).await {
            Err(f) if openrouter::worth_asking_again(&f) => why = openrouter::reason(&f),
            done => {
                last = Some(done);
                break;
            }
        }
    }
    match last {
        Some(Ok(done)) => {
            let usage = done.usage.map(openrouter::claude_usage).unwrap_or_default();
            claude::settle(raw, parse_content(&done.content, done.finish_reason.as_deref(), true), dict, &ctx.said, s, job, usage)
        }
        // A refusal before the stream: the key, a limit or the balance is the user's to fix.
        Some(Err(f)) => {
            let outcome = f.status.map_or(RefineOutcome::RawFallback, |code| claude::classify_http(code, ""));
            claude::raw_fallback(raw, outcome, claude::Usage::default(), &openrouter::reason(&f))
        }
        None => claude::raw_fallback(raw, RefineOutcome::RawFallback, claude::Usage::default(), &why),
    }
}

/// The connection answers at `address`: its model list, free, with the key if there is one (a local server needs none).
pub async fn check(http: &reqwest::Client, address: &str, key: Option<&str>) -> Result<(), EngineError> {
    check::models_list(http, &format!("{address}/models"), key).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{capture_logs, fake_api, reply, Reply};
    use cue_core::connections::find;
    use cue_core::refine::{BOLD_PROMPT, CUSTOM_PREAMBLE, SYSTEM_PROMPT};
    use cue_core::settings::RefineStyle;

    const KEY: &str = "sk-CHAT-TEST-0123456789";
    const RAW: &str = "ну привет как дела";

    fn ctx() -> RefineContext {
        RefineContext { app: "Telegram Desktop".into(), before_cursor: None, ..Default::default() }
    }

    fn body(id: &str, job: Job) -> Value {
        let c = find(id).unwrap();
        request_body(c, c.recommended().id, RAW, &ctx(), &Default::default(), job, None)
    }

    #[test]
    fn openai_body() {
        let v = body("openai", Job::Rewrite);
        assert_eq!(v["model"], "gpt-6-luna");
        assert_eq!(v["messages"][0], json!({ "role": "system", "content": format!("{SYSTEM_PROMPT}\n\n{BOLD_PROMPT}") }));
        assert_eq!(v["messages"][1], json!({ "role": "user", "content": user_message(&ctx(), RAW) }));
        assert_eq!(v["stream"], true);
        assert_eq!(v["max_completion_tokens"], max_tokens(RAW));
        assert!(v.get("max_tokens").is_none());
        assert_eq!(v["reasoning_effort"], "none");
        assert_eq!(v["stream_options"], json!({ "include_usage": true }));
        let rf = &v["response_format"];
        assert_eq!((rf["type"].as_str(), rf["json_schema"]["strict"].as_bool()), (Some("json_schema"), Some(true)));
        let claude = claude::request_body("claude-haiku-5-5", RAW, &ctx(), &Default::default());
        assert_eq!(rf["json_schema"]["schema"], claude["output_config"]["format"]["schema"], "the same schema as for Claude");
        for absent in ["prompt_cache_key", "temperature", "provider", "reasoning"] {
            assert!(v.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    fn each_service_gets_what_it_needs() {
        // Gemini: thinking only down to minimal, so headroom; the schema without `strict`.
        let v = body("gemini", Job::Rewrite);
        assert_eq!((v["reasoning_effort"].as_str(), v["max_tokens"].as_u64()), (Some("minimal"), Some(u64::from(max_tokens(RAW) + REASONING_HEADROOM))));
        assert_eq!(v["response_format"]["json_schema"]["strict"], false);
        // DeepSeek: a JSON object, said in the prompt too; reasoning off (it is on by default).
        let v = body("deepseek", Job::Rewrite);
        assert_eq!(v["response_format"], json!({ "type": "json_object" }));
        assert!(v["messages"][0]["content"].as_str().unwrap().ends_with(OBJECT_NOTE));
        assert_eq!(v["reasoning_effort"], "none");
        // Mistral: a cache key; its stream reports usage without being asked.
        let v = body("mistral", Job::Rewrite);
        assert_eq!(v["prompt_cache_key"], CACHE_NAME);
        assert!(v.get("stream_options").is_none());
        // Groq: a JSON object (a strict schema cannot be streamed), the lowest reasoning gpt-oss takes.
        let v = body("groq", Job::Rewrite);
        assert_eq!((v["response_format"]["type"].as_str(), v["reasoning_effort"].as_str()), (Some("json_object"), Some("low")));
        // LM Studio does not document `reasoning_effort`: it is not sent.
        assert!(body("lmstudio", Job::Rewrite).get("reasoning_effort").is_none());
        assert_eq!(body("ollama", Job::Rewrite)["model"], "gemma4:12b");
    }

    #[test]
    fn the_style_and_the_translation_blocks_are_claude_s() {
        let c = find("openai").unwrap();
        let custom = request_body(c, "gpt-6-luna", RAW, &ctx(), &Default::default(), Job::Rewrite, Some("пиши коротко"));
        assert_eq!(custom["messages"][0]["content"], format!("{SYSTEM_PROMPT}\n\n{CUSTOM_PREAMBLE}пиши коротко"));
        let to_en = body("openai", Job::Translate("en"));
        assert_eq!(to_en["messages"][0]["content"], format!("{SYSTEM_PROMPT}\n\n{}", translate::instruction("en")));
        assert_eq!(to_en["max_completion_tokens"], u64::from(max_tokens(RAW)) * 2);
    }

    #[test]
    fn habits_ride_in_the_system_message() {
        let c = find("openai").unwrap();
        let with = RefineContext { habits: vec!["No full stop".into()], ..ctx() };
        let v = request_body(c, "gpt-6-luna", RAW, &with, &Default::default(), Job::Rewrite, None);
        let block = cue_core::refine::habits_prompt(&with.habits).unwrap();
        assert_eq!(v["messages"][0]["content"], format!("{SYSTEM_PROMPT}\n\n{block}\n\n{BOLD_PROMPT}"));
    }

    #[test]
    fn time_limits() {
        let t = |id| request_timeout_ms(find(id).unwrap(), RAW, Job::Rewrite);
        assert_eq!(t("openai"), timeout_ms(RAW));
        assert_eq!(t("groq"), timeout_ms(RAW) + 5_000);
        assert_eq!(t("ollama"), timeout_ms(RAW) + 10_000);
        assert_eq!(request_timeout_ms(find("openai").unwrap(), RAW, Job::Translate("en")), translate::timeout_ms(RAW));
    }

    // ---- against a fake service ----

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    /// An OpenAI stream: the text in the schema, the finish reason, then usage in a chunk with no choices.
    fn answer(text: &str) -> String {
        let chunk = |content: &str, finish: Option<&str>| {
            json!({ "id": "c1", "model": "gpt-6-luna", "choices": [{ "index": 0, "delta": { "content": content }, "finish_reason": finish }] })
        };
        let usage = json!({ "id": "c1", "choices": [], "usage": { "prompt_tokens": 1200, "completion_tokens": 12,
            "prompt_tokens_details": { "cached_tokens": 1024 } } });
        let events = [
            chunk(&json!({ "text": text }).to_string(), None).to_string(),
            chunk("", Some("stop")).to_string(),
            usage.to_string(),
            "[DONE]".to_owned(),
        ];
        events.iter().map(|e| format!("data: {e}\n\n")).collect()
    }

    /// The cleanup over the connection `id`, pointed at `base`.
    fn over(id: &str, base: &str) -> RefineSettings {
        let mut s = RefineSettings { connection: id.into(), ..RefineSettings::default() };
        s.addresses.insert(id.into(), base.into());
        s
    }

    async fn run<F>(id: &str, key: Option<&str>, s: impl FnOnce(RefineSettings) -> RefineSettings, route: F) -> (RefineResult, Vec<String>, crate::testing::FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        let api = fake_api(route).await;
        let (logs, _guard) = capture_logs();
        let r = refine(&http(), key, RAW, &ctx(), &Default::default(), &s(over(id, &format!("{}/v1", api.base)))).await;
        let lines = logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::")).cloned().collect();
        (r, lines, api)
    }

    fn assert_no_leak(lines: &[String]) {
        for line in lines {
            for leak in ["привет", "Привет", "CHAT-TEST"] {
                assert!(!line.contains(leak), "\"{leak}\" in the log: {line}");
            }
        }
    }

    #[tokio::test]
    async fn refine_returns_the_cleanup_and_its_tokens() {
        let (r, lines, api) = run("openai", Some(KEY), |s| s, |_, _, _| Reply::Send(200, answer("Ну, привет. Как дела?"), "")).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Ну, привет. Как дела?", RefineOutcome::Refined));
        let usage = claude::Usage { input_tokens: 176, output_tokens: 12, cache_read_input_tokens: 1024, cache_creation_input_tokens: 0 };
        assert_eq!(r.usage, usage);
        assert!(lines.is_empty(), "{lines:?}");
        assert_eq!(api.calls(), ["POST /v1/chat/completions"]);
        let req = &api.seen()[0];
        assert_eq!(req.header("authorization"), [format!("Bearer {KEY}").as_str()]);
        let sent: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent, body("openai", Job::Rewrite));
    }

    #[tokio::test]
    async fn refine_sends_the_picked_model_the_custom_style_and_the_service_s_header() {
        let pick = |mut s: RefineSettings| {
            s.models.insert("xai".into(), "grok-4.3".into());
            (s.style, s.custom) = (RefineStyle::Custom, "пиши коротко".into());
            s
        };
        let (r, _, api) = run("xai", Some(KEY), pick, |_, _, _| Reply::Send(200, answer("Ну, привет. Как дела?"), "")).await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        let req = &api.seen()[0];
        assert_eq!(req.header("x-grok-conv-id"), [CACHE_NAME]);
        let sent: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(sent, request_body(find("xai").unwrap(), "grok-4.3", RAW, &ctx(), &Default::default(), Job::Rewrite, Some("пиши коротко")));
    }

    #[tokio::test]
    async fn a_local_server_needs_no_key() {
        let (r, lines, api) = run("ollama", None, |s| s, |_, _, _| Reply::Send(200, answer("Ну, привет. Как дела?"), "")).await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        assert!(lines.is_empty(), "{lines:?}");
        assert!(api.seen()[0].header("authorization").is_empty());
    }

    #[tokio::test]
    async fn no_key_sends_nothing() {
        let (r, lines, api) = run("deepseek", None, |s| s, |_, _, _| Reply::Send(200, answer("Привет"), "")).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawKeyProblem));
        assert!(api.calls().is_empty());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("no DeepSeek key"), "{lines:?}");
    }

    #[tokio::test]
    async fn a_refused_key_a_balance_or_a_limit_is_a_key_problem() {
        for code in [401, 402, 403, 429] {
            let body = json!({ "error": { "message": format!("refused {KEY}") } }).to_string();
            let (r, lines, api) = run("mistral", Some(KEY), |s| s, move |_, _, _| reply(code, &body)).await;
            assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawKeyProblem), "HTTP {code}");
            assert_eq!(api.calls().len(), 1, "HTTP {code} is not asked again");
            assert!(lines.last().unwrap().contains(&format!("HTTP {code}")), "{lines:?}");
            assert_no_leak(&lines);
        }
    }

    #[tokio::test]
    async fn a_failing_service_is_asked_once_more() {
        let (r, lines, api) = run("groq", Some(KEY), |s| s, |_, _, n| {
            if n == 0 { reply(502, "BODY-MARK") } else { Reply::Send(200, answer("Ну, привет. Как дела?"), "") }
        })
        .await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        assert_eq!(api.calls().len(), 2);
        assert!(lines[0].contains("HTTP 502") && lines[0].contains("asking again"), "{lines:?}");

        let (r, lines, _) = run("groq", Some(KEY), |s| s, |_, _, _| Reply::Close).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        assert!(lines.last().unwrap().starts_with("WARN") && lines.last().unwrap().contains("network"), "{lines:?}");
        assert_no_leak(&lines);
    }

    #[tokio::test]
    async fn an_unusable_answer_returns_the_raw_text() {
        let insane = "Конечно! Вот ответ: привет, у меня всё отлично, спасибо, а как дела у тебя сегодня вечером?";
        let (r, lines, _) = run("gemini", Some(KEY), |s| s, move |_, _, _| Reply::Send(200, answer(insane), "")).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        assert_eq!(r.usage.output_tokens, 12, "tokens were spent, so they are counted");
        assert!(lines.last().unwrap().contains("sanity check"), "{lines:?}");
        assert_no_leak(&lines);
    }

    #[tokio::test]
    async fn check_reads_the_model_list() {
        let api = fake_api(|_, _, _| reply(200, r#"{"data":[]}"#)).await;
        assert_eq!(check(&http(), &format!("{}/v1", api.base), Some(KEY)).await, Ok(()));
        assert_eq!(api.calls(), ["GET /v1/models"]);
        assert_eq!(api.seen()[0].header("authorization"), [format!("Bearer {KEY}").as_str()]);
        // A local server: no key.
        let local = fake_api(|_, _, _| reply(200, r#"{"data":[]}"#)).await;
        assert_eq!(check(&http(), &local.base, None).await, Ok(()));
        assert!(local.seen()[0].header("authorization").is_empty());
        // A refused key is a key problem, without the key.
        let refused = fake_api(|_, _, _| reply(401, &format!(r#"{{"error":{{"message":"bad key {KEY}"}}}}"#))).await;
        let e = check(&http(), &refused.base, Some(KEY)).await.unwrap_err();
        assert!(e.is_key_problem() && !format!("{e} {e:?}").contains("CHAT-TEST"), "{e:?}");
    }
}
