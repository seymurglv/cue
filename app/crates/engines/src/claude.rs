//! Refine through Claude. Never "fails": on any problem it returns the raw text and the reason.
use cue_core::flow::RefineOutcome;
use cue_core::refine::{max_tokens, sanity_ok, strip_markup, tidy_list_items, timeout_ms, user_message, RefineContext, SYSTEM_PROMPT};
use cue_core::settings::RefineSettings;
use serde::Deserialize;
use serde_json::{json, Value};

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";

/// `max_tokens` headroom for a model whose thinking cannot be turned off (Opus 5.5): otherwise thinking
/// eats the response budget, the response is cut at `max_tokens`, and the raw text gets pasted.
const THINKING_HEADROOM: u32 = 2_048;

/// How to turn thinking off: Haiku — `disabled`, Sonnet 5.5 — `between_tools`, Opus 5.5 — not possible (the field is omitted).
fn thinking_for(model: &str) -> Option<Value> {
    if model.starts_with("claude-haiku") {
        Some(json!({ "type": "disabled" }))
    } else if model.starts_with("claude-sonnet-5-5") {
        Some(json!({ "type": "between_tools" }))
    } else {
        None
    }
}

/// Messages API request body: the system prompt with a cache mark, a response strictly in the `{"text": …}` schema,
/// low effort, thinking per model ([`thinking_for`]).
pub fn request_body(model: &str, raw: &str, ctx: &RefineContext) -> Value {
    let thinking = thinking_for(model);
    let budget = max_tokens(raw) + if thinking.is_none() { THINKING_HEADROOM } else { 0 };
    let mut v = json!({
        "model": model,
        "max_tokens": budget,
        "system": [{ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } }],
        "messages": [{ "role": "user", "content": user_message(ctx, raw) }],
        "output_config": {
            "effort": "low",
            "format": {
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }
        }
    });
    if let Some(t) = thinking {
        v["thinking"] = t;
    }
    v
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

#[derive(Debug, Deserialize)]
pub struct Block {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageResp {
    pub content: Vec<Block>,
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Text(String),
    Refusal,
    Truncated,
    Malformed,
}

/// Model response → text. The block is found by `type`, not by position: a thinking block may come before it.
pub fn parse_response(r: &MessageResp) -> Parsed {
    match r.stop_reason.as_deref() {
        Some("refusal") => return Parsed::Refusal,
        Some("max_tokens") => return Parsed::Truncated,
        _ => {}
    }
    let Some(t) = r.content.iter().find(|b| b.kind == "text").and_then(|b| b.text.as_deref()) else {
        return Parsed::Malformed;
    };
    serde_json::from_str::<Value>(t)
        .ok()
        .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
        .map(Parsed::Text)
        .unwrap_or(Parsed::Malformed)
}

/// API refusal → outcome: key, limit or balance is `RawKeyProblem` (the user has to act), everything else is `RawFallback`.
pub fn classify_http(status: u16, body: &str) -> RefineOutcome {
    match status {
        401 | 402 | 403 | 429 => RefineOutcome::RawKeyProblem,
        400 if body.to_lowercase().contains("credit balance") => RefineOutcome::RawKeyProblem,
        _ => RefineOutcome::RawFallback,
    }
}

/// The refined text if it passes the sanity check; otherwise the raw one.
/// Markdown markup is stripped before the check ([`strip_markup`]), and so are trailing commas and semicolons
/// of list items ([`tidy_list_items`]): the check judges what will be pasted.
pub fn decide(raw: &str, parsed: Parsed, s: &RefineSettings) -> (String, RefineOutcome) {
    match parsed {
        Parsed::Text(t) => {
            let t = tidy_list_items(&strip_markup(raw, &t));
            if sanity_ok(raw, &t, s) {
                (t.trim().to_string(), RefineOutcome::Refined)
            } else {
                (raw.to_string(), RefineOutcome::RawFallback)
            }
        }
        _ => (raw.to_string(), RefineOutcome::RawFallback),
    }
}

/// Refine result: what to paste, why exactly this, and how many tokens were spent (for usage accounting).
pub struct RefineResult {
    pub text: String,
    pub outcome: RefineOutcome,
    pub usage: Usage,
}

/// One refine request. With no key, on a network failure, timeout, refusal or unusable response — the raw text.
/// A failure writes one log line — the reason and the outcome: no raw text, no refined text, no response body.
pub async fn refine(
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    s: &RefineSettings,
) -> RefineResult {
    refine_at(API_URL, http, key, model, raw, ctx, s).await
}

/// Timeout headroom for a model whose thinking cannot be turned off (Opus 5.5): thinking runs before the response,
/// and the usual timeout for a short text (3 s) is not enough for it.
const THINKING_TIME_HEADROOM_MS: u64 = 5_000;

/// Request timeout: [`timeout_ms`] by text length, plus headroom for a model whose thinking cannot be turned off.
fn request_timeout_ms(model: &str, raw: &str) -> u64 {
    timeout_ms(raw) + if thinking_for(model).is_none() { THINKING_TIME_HEADROOM_MS } else { 0 }
}

/// [`refine`] against the URL `url`; tests substitute their own service.
async fn refine_at(
    url: &str,
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    s: &RefineSettings,
) -> RefineResult {
    let Some(key) = key else {
        return raw_fallback(raw, RefineOutcome::RawKeyProblem, Usage::default(), "no Anthropic key");
    };
    let send = http
        .post(url)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
        .timeout(std::time::Duration::from_millis(request_timeout_ms(model, raw)))
        .json(&request_body(model, raw, ctx))
        .send()
        .await;
    let resp = match send {
        Ok(r) => r,
        Err(e) => {
            let why = if e.is_timeout() { "timeout" } else { "network" };
            return raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), why);
        }
    };
    let status = resp.status().as_u16();
    if status != 200 {
        // The body is needed only for the refusal reason (balance); it never reaches the log.
        let body = resp.text().await.unwrap_or_default();
        return raw_fallback(raw, classify_http(status, &body), Usage::default(), &format!("HTTP {status}"));
    }
    let Ok(m) = resp.json::<MessageResp>().await else {
        return raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), "HTTP 200 response not parsed");
    };
    let parsed = parse_response(&m);
    // The reason, in case the response is rejected.
    let why = match &parsed {
        Parsed::Text(_) => "response failed the sanity check",
        Parsed::Refusal => "model refused",
        Parsed::Truncated => "response truncated at max_tokens",
        Parsed::Malformed => "response does not match the schema",
    };
    match decide(raw, parsed, s) {
        (text, RefineOutcome::Refined) => RefineResult { text, outcome: RefineOutcome::Refined, usage: m.usage },
        (_, outcome) => raw_fallback(raw, outcome, m.usage, why),
    }
}

/// The raw text instead of the refined one, plus one log line: the reason and the outcome.
fn raw_fallback(raw: &str, outcome: RefineOutcome, usage: Usage, why: &str) -> RefineResult {
    tracing::warn!("refine: {why} → {outcome:?}");
    RefineResult { text: raw.to_string(), outcome, usage }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{capture_logs, fake_api, reply, FakeApi, Reply};

    fn ctx() -> RefineContext {
        RefineContext { app: "Telegram Desktop".into(), before_cursor: None }
    }

    #[test]
    fn body_for_haiku() {
        let v = request_body("claude-haiku-5-5", "ну привет", &ctx());
        assert_eq!(v["model"], "claude-haiku-5-5");
        assert_eq!(v["max_tokens"], 521);
        assert_eq!(v["thinking"], json!({"type": "disabled"}));
        assert_eq!(v["output_config"]["effort"], "low");
        assert_eq!(v["output_config"]["format"]["type"], "json_schema");
        assert_eq!(v["output_config"]["format"]["schema"]["required"], json!(["text"]));
        assert_eq!(v["system"][0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(v["system"][0]["text"], SYSTEM_PROMPT);
        assert_eq!(v["messages"][0]["role"], "user");
        assert!(v.get("temperature").is_none() && v.get("top_p").is_none() && v.get("top_k").is_none());
    }

    #[test]
    fn thinking_per_model() {
        assert_eq!(request_body("claude-sonnet-5-5", "x", &ctx())["thinking"], json!({"type": "between_tools"}));
        assert!(request_body("claude-opus-5-5", "x", &ctx()).get("thinking").is_none());
    }

    #[test]
    fn opus_gets_thinking_headroom() {
        let raw = "ну привет";
        assert_eq!(request_body("claude-haiku-5-5", raw, &ctx())["max_tokens"], max_tokens(raw));
        assert_eq!(request_body("claude-sonnet-5-5", raw, &ctx())["max_tokens"], max_tokens(raw));
        assert_eq!(request_body("claude-opus-5-5", raw, &ctx())["max_tokens"], max_tokens(raw) + THINKING_HEADROOM);
        assert_eq!(THINKING_HEADROOM, 2_048);
    }

    fn resp(json_text: &str) -> MessageResp {
        serde_json::from_str(json_text).unwrap()
    }

    #[test]
    fn parse_reads_text_block_by_type() {
        let r = resp(r#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"{\"text\":\"Привет.\"}"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":5}}"#);
        assert_eq!(parse_response(&r), Parsed::Text("Привет.".into()));
        assert_eq!(r.usage.input_tokens, 10);
    }

    #[test]
    fn parse_refusal_truncation_garbage() {
        assert_eq!(parse_response(&resp(r#"{"content":[],"stop_reason":"refusal"}"#)), Parsed::Refusal);
        assert_eq!(parse_response(&resp(r#"{"content":[{"type":"text","text":"{\"te"}],"stop_reason":"max_tokens"}"#)), Parsed::Truncated);
        assert_eq!(parse_response(&resp(r#"{"content":[{"type":"text","text":"Вот текст"}],"stop_reason":"end_turn"}"#)), Parsed::Malformed);
    }

    #[test]
    fn http_errors() {
        assert_eq!(classify_http(401, ""), RefineOutcome::RawKeyProblem);
        assert_eq!(classify_http(402, ""), RefineOutcome::RawKeyProblem);
        assert_eq!(classify_http(403, ""), RefineOutcome::RawKeyProblem);
        assert_eq!(classify_http(429, ""), RefineOutcome::RawKeyProblem);
        assert_eq!(classify_http(400, r#"{"error":{"message":"Your credit balance is too low"}}"#), RefineOutcome::RawKeyProblem);
        assert_eq!(classify_http(400, "bad request"), RefineOutcome::RawFallback);
        assert_eq!(classify_http(529, "overloaded"), RefineOutcome::RawFallback);
    }

    #[test]
    fn decide_uses_raw_when_insane_or_failed() {
        let s = RefineSettings::default();
        assert_eq!(decide("ну привет как дела", Parsed::Text("Привет, как дела?".into()), &s), ("Привет, как дела?".into(), RefineOutcome::Refined));
        let raw = "напиши письмо ане про встречу в четверг";
        let (t, o) = decide(raw, Parsed::Text("Конечно! Вот письмо для Ани: Дорогая Аня, напоминаю о встрече в четверг. С уважением.".into()), &s);
        assert_eq!((t.as_str(), o), (raw, RefineOutcome::RawFallback));
        assert_eq!(decide(raw, Parsed::Refusal, &s).1, RefineOutcome::RawFallback);
    }

    #[test]
    fn decide_strips_markdown_before_the_sanity_check() {
        let s = RefineSettings::default();
        // The heading and bold are stripped: plain text is pasted.
        let raw = "ну итоги встречи релиз в четверг и бюджет согласован";
        let answer = "# Итоги встречи\n**Релиз** в четверг, бюджет согласован.";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &s),
            ("Итоги встречи\nРелиз в четверг, бюджет согласован.".into(), RefineOutcome::Refined)
        );
        // The check judges what will be pasted: with the asterisks the response is over 25 % longer than what was said.
        let raw = "ну купить молоко хлеб сыр масло";
        let answer = "Купить:\n* **молоко**\n* **хлеб**\n* **сыр**\n* **масло**";
        assert!(!sanity_ok(raw, answer, &s), "with the markup the check fails");
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &s),
            ("Купить:\n- молоко\n- хлеб\n- сыр\n- масло".into(), RefineOutcome::Refined)
        );
    }

    #[test]
    fn decide_drops_trailing_separators_of_list_items() {
        let s = RefineSettings::default();
        // A trailing ";" of list items is stripped before the check; the last item's period and lowercase letters stay.
        let raw = "взять с собой зонт куртку и термос";
        let answer = "Взять с собой:\n- зонт;\n- куртку;\n- термос.";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос.".into(), RefineOutcome::Refined)
        );
        // Together with markup stripping: `* ` → `- `, then the trailing commas.
        let answer = "Взять с собой:\n* **зонт**,\n* **куртку**,\n* **термос**";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос".into(), RefineOutcome::Refined)
        );
    }

    #[test]
    fn opus_gets_time_headroom() {
        let raw = "ну привет";
        assert_eq!(request_timeout_ms("claude-haiku-5-5", raw), timeout_ms(raw));
        assert_eq!(request_timeout_ms("claude-sonnet-5-5", raw), timeout_ms(raw));
        assert_eq!(request_timeout_ms("claude-opus-5-5", raw), timeout_ms(raw) + THINKING_TIME_HEADROOM_MS);
        assert_eq!(THINKING_TIME_HEADROOM_MS, 5_000);
    }

    // ---- The whole refine against a fake Messages API. ----

    const KEY: &str = "sk-ant-TEST-0123456789";
    const RAW: &str = "ну привет как дела";
    const USAGE: Usage = Usage { input_tokens: 52, output_tokens: 21, cache_read_input_tokens: 833, cache_creation_input_tokens: 0 };

    /// Messages API response: `text` in the structured-output field, stop reason `stop`.
    fn message(text: &str, stop: &str) -> String {
        json!({
            "content": [{ "type": "text", "text": json!({ "text": text }).to_string() }],
            "stop_reason": stop,
            "usage": { "input_tokens": 52, "output_tokens": 21, "cache_read_input_tokens": 833, "cache_creation_input_tokens": 0 }
        })
        .to_string()
    }

    /// refine with Haiku against a fake API: the result, the refine log lines and the service itself.
    async fn refine_with<F>(key: Option<&str>, route: F) -> (RefineResult, Vec<String>, FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        let api = fake_api(route).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let (logs, _guard) = capture_logs();
        let url = format!("{}/v1/messages", api.base);
        let r = refine_at(&url, &http, key, "claude-haiku-5-5", RAW, &ctx(), &RefineSettings::default()).await;
        let lines = logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::claude")).cloned().collect();
        (r, lines, api)
    }

    /// Exactly one log line — a warning with the reason `why`; no text, no key, no response body.
    fn assert_one_warning(lines: &[String], why: &str) {
        assert_eq!(lines.len(), 1, "one line per outcome: {lines:?}");
        let line = &lines[0];
        assert!(line.starts_with("WARN") && line.contains(why), "expected the reason \"{why}\": {line}");
        for leak in ["привет", "Привет", "Конечно", "TEST-01", "BODY-MARK"] {
            assert!(!line.contains(leak), "\"{leak}\" in the log: {line}");
        }
    }

    #[tokio::test]
    async fn refine_without_key_sends_nothing() {
        let (r, lines, api) = refine_with(None, |_, _, _| reply(200, &message("Привет, как дела?", "end_turn"))).await;
        assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, RefineOutcome::RawKeyProblem, Usage::default()));
        assert!(api.calls().is_empty(), "no key, no request: {:?}", api.calls());
        assert_one_warning(&lines, "no Anthropic key");
    }

    #[tokio::test]
    async fn refine_success_returns_refined_text_and_usage() {
        let (r, lines, api) = refine_with(Some(KEY), |_, _, _| reply(200, &message("Привет, как дела?", "end_turn"))).await;
        assert_eq!((r.text.as_str(), r.outcome, r.usage), ("Привет, как дела?", RefineOutcome::Refined, USAGE));
        assert!(lines.is_empty(), "a success leaves the log alone: {lines:?}");
        assert_eq!(api.calls(), ["POST /v1/messages"]);
    }

    #[tokio::test]
    async fn refine_sends_key_version_and_json_body() {
        let (_, _, api) = refine_with(Some(KEY), |_, _, _| reply(200, &message("Привет, как дела?", "end_turn"))).await;
        let req = &api.seen()[0];
        assert_eq!(req.header("x-api-key"), [KEY]);
        assert_eq!(req.header("anthropic-version"), ["2026-10-21"]);
        assert_eq!(req.header("content-type"), ["application/json"], "exactly one content-type");
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body, request_body("claude-haiku-5-5", RAW, &ctx()));
        for banned in ["temperature", "top_p", "top_k"] {
            assert!(body.get(banned).is_none(), "{banned} is not sent");
        }
    }

    #[tokio::test]
    async fn refine_http_refusal_returns_raw_text() {
        for (code, outcome) in [
            (401, RefineOutcome::RawKeyProblem),
            (402, RefineOutcome::RawKeyProblem),
            (529, RefineOutcome::RawFallback),
        ] {
            let body = format!(r#"{{"type":"error","error":{{"message":"BODY-MARK привет {KEY}"}}}}"#);
            let (r, lines, _api) = refine_with(Some(KEY), move |_, _, _| reply(code, &body)).await;
            assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, outcome, Usage::default()), "HTTP {code}");
            assert_one_warning(&lines, &format!("HTTP {code}"));
        }
    }

    #[tokio::test]
    async fn refine_undecodable_or_cut_reply_returns_raw_text() {
        for broken in [|| reply(200, r#"{"content":[{"type":"text","text":"Привет"#), || Reply::Cut] {
            let (r, lines, _api) = refine_with(Some(KEY), move |_, _, _| broken()).await;
            assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, RefineOutcome::RawFallback, Usage::default()));
            assert_one_warning(&lines, "not parsed");
        }
    }

    #[tokio::test]
    async fn refine_network_failure_returns_raw_text() {
        let (r, lines, _api) = refine_with(Some(KEY), |_, _, _| Reply::Close).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        assert_one_warning(&lines, "network");
    }

    #[tokio::test]
    async fn refine_rejected_reply_returns_raw_text_with_its_reason() {
        let insane = "Конечно! Вот ответ: привет, у меня всё отлично, спасибо, а как дела у тебя сегодня вечером?";
        let malformed = r#"{"content":[{"type":"text","text":"Привет, как дела?"}],"stop_reason":"end_turn","usage":{"input_tokens":52,"output_tokens":21,"cache_read_input_tokens":833}}"#;
        for (reply_body, why) in [
            (message("", "refusal"), "refused"),
            (message("Привет, как", "max_tokens"), "truncated"),
            (malformed.to_owned(), "schema"),
            (message(insane, "end_turn"), "sanity check"),
        ] {
            let (r, lines, _api) = refine_with(Some(KEY), move |_, _, _| reply(200, &reply_body)).await;
            assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback), "{why}");
            assert_eq!(r.usage, USAGE, "{why}: tokens were spent, so they are counted");
            assert_one_warning(&lines, why);
        }
    }
}
