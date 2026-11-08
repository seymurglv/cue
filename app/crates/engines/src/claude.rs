//! Refine through Claude. Never "fails": on any problem it returns the raw text and the reason.
use cue_core::flow::RefineOutcome;
use cue_core::refine::{
    max_tokens, sanity_ok_with, strip_markup, habits_prompt, terms_prompt, tidy_list_items, timeout_ms, user_message, RefineContext, BOLD_PROMPT,
    SYSTEM_PROMPT,
};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_core::translate::{self, translation_ok};
use serde::Deserialize;
use serde_json::{json, Value};

pub const API_URL: &str = "https://api.anthropic.com/v1/messages";
pub const API_VERSION: &str = "2026-10-21";

/// `max_tokens` headroom for a model whose thinking cannot be turned off (Opus 5.5): otherwise thinking
/// eats the response budget, the response is cut at `max_tokens`, and the raw text gets pasted.
pub(crate) const THINKING_HEADROOM: u32 = 2_048;

/// How to turn thinking off: Haiku — `disabled`, Sonnet 5.5 — `between_tools`, Opus 5.5 — not possible (the field is omitted).
pub(crate) fn thinking_for(model: &str) -> Option<Value> {
    if model.starts_with("claude-haiku") {
        Some(json!({ "type": "disabled" }))
    } else if model.starts_with("claude-sonnet-5-5") {
        Some(json!({ "type": "between_tools" }))
    } else {
        None
    }
}

/// The request's effort: Haiku 5.5 at its own default (`medium`, which Anthropic tunes it for), the others at `low`.
pub(crate) fn effort_for(model: &str) -> &'static str {
    if model.starts_with("claude-haiku") {
        "medium"
    } else {
        "low"
    }
}

/// What the refine request does with the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Clean up in the spoken language (dictation).
    Clean,
    /// Clean up and turn spoken phrasing into written: CUE's own cleanup of a dictation ([`BOLD_PROMPT`]) or the user's
    /// instructions file in its place.
    Rewrite,
    /// Clean up and write in this language (a code of `cue_core::translate::LANGUAGES`).
    Translate(&'static str),
}

/// Messages API request body: the system prompt with a cache mark, a response strictly in the `{"text": …}` schema,
/// effort ([`effort_for`]) and thinking ([`thinking_for`]) per model.
///
/// The user's dictionary (`dict`) goes in a second system block after the unchanged prompt ([`terms_prompt`]), and the
/// cache mark moves to that last block: the cached prefix covers both. With no usable term the body is the same, byte
/// for byte, as without a dictionary.
pub fn request_body(model: &str, raw: &str, ctx: &RefineContext, dict: &DictionarySettings) -> Value {
    request_body_for(model, raw, ctx, dict, Job::Clean)
}

/// [`request_body`] for a job. A translation adds one system block after the cache mark: the cached prefix is the one
/// plain dictation uses, so translating never writes a second cache. The answer may be longer in tokens than the speech
/// (another alphabet), so the budget doubles.
pub fn request_body_for(model: &str, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, job: Job) -> Value {
    request_body_styled(model, raw, ctx, dict, job, BOLD_PROMPT)
}

/// [`request_body_for`] with the cleanup's `instructions` (the instructions file) as the job's block.
pub fn request_body_styled(model: &str, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, job: Job, instructions: &str) -> Value {
    let thinking = thinking_for(model);
    let budget = max_tokens(raw) + if thinking.is_none() { THINKING_HEADROOM } else { 0 };
    let mut v = json!({
        "model": model,
        "max_tokens": budget,
        "system": system_blocks(dict, &ctx.habits, job, instructions),
        "messages": [{ "role": "user", "content": user_message(ctx, raw) }],
        "output_config": {
            "effort": effort_for(model),
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
    if let Job::Translate(_) = job {
        v["max_tokens"] = json!(translation_budget(budget));
    }
    v
}

/// The system blocks of a refine request: the prompt, the dictionary block ([`terms_prompt`]) with the cache mark on the
/// last of them; then, for a cleanup, the habits block ([`habits_prompt`]); then the job's own block: CUE's cleanup
/// (the `instructions`, the instructions file's text as given), or the translation's. Everything after the mark changes
/// with the app or the job, so the cached prefix is always the same.
pub(crate) fn system_blocks(dict: &DictionarySettings, habits: &[String], job: Job, instructions: &str) -> Value {
    let mut system = match terms_prompt(&dict.terms, &dict.learned) {
        None => vec![json!({ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } })],
        Some(block) => vec![
            json!({ "type": "text", "text": SYSTEM_PROMPT }),
            json!({ "type": "text", "text": block, "cache_control": { "type": "ephemeral" } }),
        ],
    };
    if matches!(job, Job::Clean | Job::Rewrite)
        && let Some(block) = habits_prompt(habits)
    {
        system.push(json!({ "type": "text", "text": block }));
    }
    match job {
        Job::Clean => {}
        Job::Rewrite => {
            system.push(json!({ "type": "text", "text": instructions }))
        }
        Job::Translate(to) => system.push(json!({ "type": "text", "text": translate::instruction(to) })),
    }
    Value::Array(system)
}

/// A translation may be longer in tokens than the speech (another alphabet): twice the budget, up to 16 000.
pub(crate) fn translation_budget(budget: u32) -> u32 {
    budget.saturating_mul(2).min(16_000)
}

/// Whether a cache warm-up is possible for the model: a model whose thinking cannot be turned off (Opus 5.5) would need a
/// thinking budget in a one-token answer.
pub fn can_warm(model: &str) -> bool {
    thinking_for(model).is_some()
}

/// The cache warm-up (spec 2 §6.3): the dictation request with a one-character transcript and `max_tokens: 1`.
/// `max_tokens: 0` is refused together with `output_config.format`, and the format has to stay: the warm-up must build
/// the exact prefix real refine reads.
pub fn warm_body(model: &str, dict: &DictionarySettings) -> Value {
    let ctx = RefineContext::default();
    let mut v = request_body(model, ".", &ctx, dict);
    v["max_tokens"] = json!(1);
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
/// of list items ([`tidy_list_items`]): the check judges what will be pasted. Words of the dictionary terms count as
/// said ([`sanity_ok`]).
pub fn decide(raw: &str, parsed: Parsed, dict: &DictionarySettings, s: &RefineSettings) -> (String, RefineOutcome) {
    decide_with(raw, parsed, dict, &[], s)
}

/// [`decide`] for a request that sent habits: the new words of their replacements (`said`, `RefineContext::said`)
/// count as said too ([`sanity_ok_with`]), for this answer only.
pub fn decide_with(raw: &str, parsed: Parsed, dict: &DictionarySettings, said: &[String], s: &RefineSettings) -> (String, RefineOutcome) {
    match parsed {
        Parsed::Text(t) => {
            let t = tidy_list_items(&strip_markup(raw, &t));
            if sanity_ok_with(raw, &t, &dict.terms, said, s) {
                (t.trim().to_string(), RefineOutcome::Refined)
            } else {
                (raw.to_string(), RefineOutcome::RawFallback)
            }
        }
        _ => (raw.to_string(), RefineOutcome::RawFallback),
    }
}

/// The translation if it passes [`translation_ok`]; markup and list punctuation are stripped first, as in [`decide`].
pub fn decide_translation(raw: &str, parsed: Parsed, dict: &DictionarySettings, to: &str) -> (String, RefineOutcome) {
    match parsed {
        Parsed::Text(t) => {
            let t = tidy_list_items(&strip_markup(raw, &t));
            if translation_ok(raw, &t, &dict.terms, to) {
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
/// `dict` is the user's dictionary (`settings.dictionary`), see [`request_body`].
#[allow(clippy::too_many_arguments)]
pub async fn refine(
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    instructions: &str,
) -> RefineResult {
    refine_at(API_URL, http, key, model, raw, ctx, dict, s, instructions, Job::Rewrite).await
}

/// One translation request (spec 2 §5): the refine request with the translation block. Never "fails": on any problem
/// the raw text comes back with `RawFallback` / `RawKeyProblem`, and the flow shows "Couldn't translate" instead of
/// pasting it.
#[allow(clippy::too_many_arguments)]
pub async fn translate(
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    instructions: &str,
    to: &'static str,
) -> RefineResult {
    refine_at(API_URL, http, key, model, raw, ctx, dict, s, instructions, Job::Translate(to)).await
}

/// Timeout headroom for a model whose thinking cannot be turned off (Opus 5.5): thinking runs before the response,
/// and the usual timeout for a short text (3 s) is not enough for it.
pub(crate) const THINKING_TIME_HEADROOM_MS: u64 = 5_000;

/// Request timeout: [`timeout_ms`] by text length ([`translate::timeout_ms`] for a translation), plus headroom for a
/// model whose thinking cannot be turned off.
fn request_timeout_ms(model: &str, raw: &str, job: Job) -> u64 {
    let base = match job {
        Job::Clean | Job::Rewrite => timeout_ms(raw),
        Job::Translate(_) => translate::timeout_ms(raw),
    };
    base + if thinking_for(model).is_none() { THINKING_TIME_HEADROOM_MS } else { 0 }
}

/// The second request gets this many times the first one's timeout.
pub(crate) const RETRY_TIMEOUT_FACTOR: u64 = 2;

/// A refusal that a second request may not get: the service is overloaded or failed on its side. Key, limit and
/// balance problems (4xx) are not asked again.
fn is_transient(status: u16) -> bool {
    matches!(status, 500 | 502 | 503 | 504 | 529)
}

/// [`refine`] or [`translate`] against the URL `url`; tests substitute their own service.
#[allow(clippy::too_many_arguments)]
async fn refine_at(
    url: &str,
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    instructions: &str,
    job: Job,
) -> RefineResult {
    let Some(key) = key else {
        return raw_fallback(raw, RefineOutcome::RawKeyProblem, Usage::default(), "no Anthropic key");
    };
    let body = request_body_styled(model, raw, ctx, dict, job, instructions);
    let timeout = request_timeout_ms(model, raw, job);
    // A timeout, a network failure or an overloaded service is asked once more, with twice the time: the raw text is
    // pasted only when the second request fails too.
    let (mut resp, mut why) = (None, String::new());
    for ms in [timeout, timeout * RETRY_TIMEOUT_FACTOR] {
        if !why.is_empty() {
            tracing::info!("refine: {why}, asking again");
        }
        match messages_post(http, url, key).timeout(std::time::Duration::from_millis(ms)).json(&body).send().await {
            Ok(r) if !is_transient(r.status().as_u16()) => {
                resp = Some(r);
                break;
            }
            Ok(r) => why = format!("HTTP {}", r.status().as_u16()),
            Err(e) => why = if e.is_timeout() { "timeout" } else { "network" }.to_owned(),
        }
    }
    // A transient refusal is never a key problem (`classify_http`), so both failed requests mean `RawFallback`.
    let Some(resp) = resp else {
        return raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), &why);
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
    settle(raw, parse_response(&m), dict, &ctx.said, s, job, m.usage)
}

/// A parsed answer → what to paste: the answer if it passes its job's check, otherwise the raw text and one log line
/// with the reason. The tokens are counted either way: they were spent. `said` are the words the habits sent with a
/// cleanup write ([`decide_with`]); a translation's check takes none.
pub(crate) fn settle(
    raw: &str,
    parsed: Parsed,
    dict: &DictionarySettings,
    said: &[String],
    s: &RefineSettings,
    job: Job,
    usage: Usage,
) -> RefineResult {
    // The reason, in case the response is rejected.
    let why = match (&parsed, job) {
        (Parsed::Text(_), Job::Clean | Job::Rewrite) => "response failed the sanity check",
        (Parsed::Text(_), Job::Translate(_)) => "translation failed the check",
        (Parsed::Refusal, _) => "model refused",
        (Parsed::Truncated, _) => "response truncated at max_tokens",
        (Parsed::Malformed, _) => "response does not match the schema",
    };
    let decided = match job {
        Job::Clean | Job::Rewrite => decide_with(raw, parsed, dict, said, s),
        Job::Translate(to) => decide_translation(raw, parsed, dict, to),
    };
    match decided {
        (text, RefineOutcome::Refined) => RefineResult { text, outcome: RefineOutcome::Refined, usage },
        (_, outcome) => raw_fallback(raw, outcome, usage, why),
    }
}

/// The raw text instead of the refined one, plus one log line: the reason and the outcome.
pub(crate) fn raw_fallback(raw: &str, outcome: RefineOutcome, usage: Usage, why: &str) -> RefineResult {
    tracing::warn!("refine: {why} → {outcome:?}");
    RefineResult { text: raw.to_string(), outcome, usage }
}

/// A Messages API POST with the key and the API version.
pub(crate) fn messages_post(http: &reqwest::Client, url: &str, key: &str) -> reqwest::RequestBuilder {
    http.post(url)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
}

/// The warm-up runs while the user speaks; one that has not answered in 5 s is of no use to this dictation.
const WARM_TIMEOUT_MS: u64 = 5_000;

/// Of a warm-up answer only the tokens matter: they are paid for.
#[derive(Deserialize)]
struct WarmResp {
    usage: Usage,
}

/// One cache warm-up request ([`warm_body`]): the tokens paid for on HTTP 200. On any problem, one log line with the
/// reason only (neither the key nor the response body) and `None`: the dictation goes on, its refine writes the cache.
pub async fn warm(http: &reqwest::Client, key: &str, model: &str, dict: &DictionarySettings) -> Option<Usage> {
    warm_at(API_URL, http, key, model, dict).await
}

/// [`warm`] against the URL `url`; tests substitute their own service.
async fn warm_at(url: &str, http: &reqwest::Client, key: &str, model: &str, dict: &DictionarySettings) -> Option<Usage> {
    let send = messages_post(http, url, key)
        .timeout(std::time::Duration::from_millis(WARM_TIMEOUT_MS))
        .json(&warm_body(model, dict))
        .send()
        .await;
    let why = match send {
        Err(e) if e.is_timeout() => "timeout".to_owned(),
        Err(_) => "network".to_owned(),
        Ok(r) if r.status().as_u16() != 200 => format!("HTTP {}", r.status().as_u16()),
        Ok(r) => match r.json::<WarmResp>().await {
            Ok(w) => return Some(w.usage),
            Err(_) => "HTTP 200 response not parsed".to_owned(),
        },
    };
    tracing::warn!("cache warm-up: {why}");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::refine::{habits_prompt, sanity_ok};
    use crate::testing::{capture_logs, fake_api, reply, FakeApi, Reply};

    fn dict(terms: &[&str]) -> DictionarySettings {
        DictionarySettings { terms: terms.iter().map(|t| t.to_string()).collect(), ..Default::default() }
    }

    fn ctx() -> RefineContext {
        RefineContext { app: "Telegram Desktop".into(), before_cursor: None, ..Default::default() }
    }

    /// Habits go after the dictionary, behind the cache mark, for a cleanup only; with none the body is the same.
    #[test]
    fn habits_go_after_the_dictionary_for_a_cleanup_only() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned()];
        let with = RefineContext { habits: rules.clone(), ..ctx() };
        let block = habits_prompt(&rules).unwrap();
        let v = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&["Soniox"]), Job::Rewrite);
        let texts: Vec<&str> = v["system"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!((texts.len(), texts[0], texts[2], texts[3]), (4, SYSTEM_PROMPT, block.as_str(), BOLD_PROMPT));
        assert!(v["system"][1]["cache_control"].is_object() && v["system"][2].get("cache_control").is_none(), "the cached prefix is the one without habits");
        assert_eq!(request_body("claude-haiku-5-5", "привет", &with, &dict(&[]))["system"][1]["text"], block);
        let to_en = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&[]), Job::Translate("en"));
        assert!(!to_en["system"].to_string().contains("без точки"), "a translation takes none");
        let none = RefineContext { habits: Vec::new(), ..ctx() };
        assert_eq!(request_body("claude-haiku-5-5", "привет", &none, &dict(&[])), request_body("claude-haiku-5-5", "привет", &ctx(), &dict(&[])));
    }

    #[test]
    fn body_for_haiku() {
        let v = request_body("claude-haiku-5-5", "ну привет", &ctx(), &Default::default());
        assert_eq!(v["model"], "claude-haiku-5-5");
        assert_eq!(v["max_tokens"], 521);
        assert_eq!(v["thinking"], json!({"type": "disabled"}));
        // Haiku 5.5 at its own default effort, with thinking off (accepted at `high` or below).
        assert_eq!(v["output_config"]["effort"], "medium");
        assert_eq!(v["output_config"]["format"]["type"], "json_schema");
        assert_eq!(v["output_config"]["format"]["schema"]["required"], json!(["text"]));
        assert_eq!(v["system"][0]["cache_control"], json!({"type": "ephemeral"}));
        assert_eq!(v["system"][0]["text"], SYSTEM_PROMPT);
        assert_eq!(v["messages"][0]["role"], "user");
        assert!(v.get("temperature").is_none() && v.get("top_p").is_none() && v.get("top_k").is_none());
    }

    /// The body as sent before the dictionary existed, byte for byte (but for Haiku's effort, now its own default).
    fn golden_body() -> String {
        format!(
            concat!(
                r#"{{"max_tokens":521,"messages":[{{"content":"<app>Telegram Desktop</app>\n<transcript>ну привет</transcript>","role":"user"}}],"#,
                r#""model":"claude-haiku-5-5","output_config":{{"effort":"medium","format":{{"schema":{{"additionalProperties":false,"#,
                r#""properties":{{"text":{{"type":"string"}}}},"required":["text"],"type":"object"}},"type":"json_schema"}}}},"#,
                r#""system":[{{"cache_control":{{"type":"ephemeral"}},"text":{prompt},"type":"text"}}],"thinking":{{"type":"disabled"}}}}"#
            ),
            prompt = serde_json::to_string(SYSTEM_PROMPT).unwrap()
        )
    }

    /// With no usable term the body is the one sent before the dictionary existed, byte for byte: the cached prefix
    /// stays the same, so an update brings no cache write.
    #[test]
    fn body_without_terms_is_unchanged() {
        let long = "x".repeat(61);
        for terms in [dict(&[]), dict(&["", "  ", "Claude\nHaiku", &long])] {
            let body = serde_json::to_string(&request_body("claude-haiku-5-5", "ну привет", &ctx(), &terms)).unwrap();
            assert_eq!(body, golden_body(), "{:?}", terms.terms);
        }
    }

    #[test]
    fn a_learned_word_reaches_the_request_with_how_it_sounds() {
        let dict = DictionarySettings {
            terms: vec!["Marisol".to_owned()],
            learned: vec![cue_core::settings::LearnedWord { word: "Marisol".into(), heard: "Марисоль".into() }],
        };
        let body = request_body("claude-haiku-5-5", "ну привет", &ctx(), &dict);
        let text = body["system"][1]["text"].as_str().unwrap();
        assert!(text.contains("Marisol — sounds like «Марисоль»"), "{text}");
    }

    #[test]
    fn terms_go_in_a_second_system_block_with_the_cache_mark() {
        let terms = dict(&["Soniox", " GitHub "]);
        let with = request_body("claude-haiku-5-5", "ну привет", &ctx(), &terms);
        // The prompt block comes first, unchanged; the mark is on the last block, so the cached prefix covers both.
        let block = "Names and terms the user often says. When the speech clearly refers to one of them, even if the transcript \
                     spells it differently, splits it into parts or mixes alphabets, write it exactly as listed. Don't add a term \
                     the user didn't say, and don't change other words because of this list.\n\nSoniox\nGitHub";
        let expected = json!([
            { "type": "text", "text": SYSTEM_PROMPT },
            { "type": "text", "text": block, "cache_control": { "type": "ephemeral" } }
        ]);
        assert_eq!(with["system"], expected);
        // Nothing else in the body changes.
        let mut rest = with.clone();
        rest["system"] = request_body("claude-haiku-5-5", "ну привет", &ctx(), &Default::default())["system"].clone();
        assert_eq!(serde_json::to_string(&rest).unwrap(), golden_body());
        for model in ["claude-sonnet-5-5", "claude-opus-5-5"] {
            assert_eq!(request_body(model, "x", &ctx(), &terms)["system"], expected, "{model}");
        }
    }

    #[test]
    fn thinking_per_model() {
        assert_eq!(request_body("claude-sonnet-5-5", "x", &ctx(), &Default::default())["thinking"], json!({"type": "between_tools"}));
        assert!(request_body("claude-opus-5-5", "x", &ctx(), &Default::default()).get("thinking").is_none());
    }

    #[test]
    fn opus_gets_thinking_headroom() {
        let raw = "ну привет";
        assert_eq!(request_body("claude-haiku-5-5", raw, &ctx(), &Default::default())["max_tokens"], max_tokens(raw));
        assert_eq!(request_body("claude-sonnet-5-5", raw, &ctx(), &Default::default())["max_tokens"], max_tokens(raw));
        assert_eq!(request_body("claude-opus-5-5", raw, &ctx(), &Default::default())["max_tokens"], max_tokens(raw) + THINKING_HEADROOM);
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
        assert_eq!(decide("ну привет как дела", Parsed::Text("Привет, как дела?".into()), &Default::default(), &s), ("Привет, как дела?".into(), RefineOutcome::Refined));
        let raw = "напиши письмо ане про встречу в четверг";
        let (t, o) = decide(raw, Parsed::Text("Конечно! Вот письмо для Ани: Дорогая Аня, напоминаю о встрече в четверг. С уважением.".into()), &Default::default(), &s);
        assert_eq!((t.as_str(), o), (raw, RefineOutcome::RawFallback));
        assert_eq!(decide(raw, Parsed::Refusal, &Default::default(), &s).1, RefineOutcome::RawFallback);
    }

    #[test]
    fn decide_keeps_a_dictionary_fix_in_a_short_phrase() {
        let s = RefineSettings::default();
        let fixed = || Parsed::Text("Открой GitHub.".into());
        assert_eq!(decide("открой гитхаб", fixed(), &dict(&["GitHub"]), &s), ("Открой GitHub.".into(), RefineOutcome::Refined));
        assert_eq!(decide("открой гитхаб", fixed(), &Default::default(), &s).1, RefineOutcome::RawFallback, "without the dictionary: 1 of 2 words said");
    }

    /// A replacement habit sent with the cleanup writes words that were never said; its answer is kept, and the words
    /// count for this answer only: a translation's check is the same with them or without.
    #[test]
    fn settle_keeps_the_words_of_a_habit_sent_with_the_cleanup() {
        let s = RefineSettings::default();
        let (raw, greeting) = ("здравствуйте анна", || Parsed::Text("Добрый день, Анна!".into()));
        let said = vec!["добрый день".to_owned()];
        let no = DictionarySettings::default();
        let with = settle(raw, greeting(), &no, &said, &s, Job::Rewrite, Usage::default());
        assert_eq!((with.text.as_str(), with.outcome), ("Добрый день, Анна!", RefineOutcome::Refined));
        let without = settle(raw, greeting(), &no, &[], &s, Job::Rewrite, Usage::default());
        assert_eq!((without.text.as_str(), without.outcome), (raw, RefineOutcome::RawFallback));
        assert_eq!(settle(raw, greeting(), &no, &said, &s, Job::Clean, Usage::default()).outcome, RefineOutcome::Refined);
        for answer in ["Hello, Anna!", "Добрый день, Анна!"] {
            let translated = |said: &[String]| {
                let r = settle(raw, Parsed::Text(answer.into()), &no, said, &s, Job::Translate("en"), Usage::default());
                (r.text, r.outcome)
            };
            assert_eq!(translated(&said), translated(&[]), "{answer}");
        }
        assert_eq!(no, DictionarySettings::default(), "nothing went into the dictionary");
    }

    #[test]
    fn decide_strips_markdown_before_the_sanity_check() {
        let s = RefineSettings::default();
        // The heading and bold are stripped: plain text is pasted.
        let raw = "ну итоги встречи релиз в четверг и бюджет согласован";
        let answer = "# Итоги встречи\n**Релиз** в четверг, бюджет согласован.";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Итоги встречи\nРелиз в четверг, бюджет согласован.".into(), RefineOutcome::Refined)
        );
        // The check judges what will be pasted: with the asterisks the response is over 25 % longer than what was said.
        let raw = "ну купить молоко хлеб сыр масло";
        let answer = "Купить:\n* **молоко**\n* **хлеб**\n* **сыр**\n* **масло**";
        assert!(!sanity_ok(raw, answer, &[], &s), "with the markup the check fails");
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
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
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос.".into(), RefineOutcome::Refined)
        );
        // Together with markup stripping: `* ` → `- `, then the trailing commas.
        let answer = "Взять с собой:\n* **зонт**,\n* **куртку**,\n* **термос**";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос".into(), RefineOutcome::Refined)
        );
    }

    #[test]
    fn opus_gets_time_headroom() {
        let raw = "ну привет";
        assert_eq!(request_timeout_ms("claude-haiku-5-5", raw, Job::Clean), timeout_ms(raw));
        assert_eq!(request_timeout_ms("claude-sonnet-5-5", raw, Job::Clean), timeout_ms(raw));
        assert_eq!(request_timeout_ms("claude-opus-5-5", raw, Job::Clean), timeout_ms(raw) + THINKING_TIME_HEADROOM_MS);
        assert_eq!(THINKING_TIME_HEADROOM_MS, 5_000);
    }

    // ---- Translation (spec 2 §5) ----

    #[test]
    fn translation_adds_a_block_after_the_cache_mark() {
        let plain = request_body("claude-haiku-5-5", "ну привет", &ctx(), &Default::default());
        let tr = request_body_for("claude-haiku-5-5", "ну привет", &ctx(), &Default::default(), Job::Translate("en"));
        let system = tr["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0], plain["system"][0], "the cached prompt block is the same, mark included");
        assert!(system[1].get("cache_control").is_none(), "the instruction is not cached");
        assert!(system[1]["text"].as_str().unwrap().contains("English"));
        assert_eq!(tr["max_tokens"], plain["max_tokens"].as_u64().unwrap() * 2);
        assert_eq!(tr["output_config"], plain["output_config"]);
        assert_eq!(tr["messages"], plain["messages"]);
    }

    #[test]
    fn translation_with_terms_keeps_the_mark_on_the_terms_block() {
        let terms = dict(&["Soniox"]);
        let tr = request_body_for("claude-haiku-5-5", "x", &ctx(), &terms, Job::Translate("ru"));
        let system = tr["system"].as_array().unwrap();
        assert_eq!(system.len(), 3);
        assert!(system[1].get("cache_control").is_some());
        assert!(system[2]["text"].as_str().unwrap().contains("Russian"));
    }

    #[test]
    fn a_cleanup_job_is_the_plain_body() {
        let terms = dict(&["Soniox"]);
        for model in ["claude-haiku-5-5", "claude-opus-5-5"] {
            assert_eq!(request_body_for(model, "x", &ctx(), &terms, Job::Clean), request_body(model, "x", &ctx(), &terms));
        }
    }

    #[test]
    fn translation_gets_a_second_more() {
        let raw = "ну привет";
        assert_eq!(request_timeout_ms("claude-haiku-5-5", raw, Job::Translate("en")), translate::timeout_ms(raw));
        assert_eq!(
            request_timeout_ms("claude-opus-5-5", raw, Job::Translate("en")),
            translate::timeout_ms(raw) + THINKING_TIME_HEADROOM_MS
        );
    }

    #[test]
    fn bold_refine_adds_its_block_after_the_cache_mark() {
        for t in [&dict(&[]), &dict(&["Soniox"])] {
            let clean = request_body_for("claude-sonnet-5-5", "ну привет", &ctx(), t, Job::Clean);
            let bold = request_body_for("claude-sonnet-5-5", "ну привет", &ctx(), t, Job::Rewrite);
            let mut system = clean["system"].as_array().unwrap().clone();
            system.push(json!({ "type": "text", "text": BOLD_PROMPT }));
            assert_eq!(bold["system"], json!(system), "the cached prefix is the careful one");
            let mut rest = bold.clone();
            rest["system"] = clean["system"].clone();
            assert_eq!(rest, clean, "nothing else changes");
        }
    }

    #[test]
    fn sonnet_stays_at_low_effort_with_thinking_between_tools() {
        let v = request_body("claude-sonnet-5-5", "ну привет", &ctx(), &Default::default());
        assert_eq!((v["output_config"]["effort"].as_str(), &v["thinking"]), (Some("low"), &json!({"type": "between_tools"})));
        assert_eq!(request_timeout_ms("claude-sonnet-5-5", "ну привет", Job::Rewrite), timeout_ms("ну привет"));
    }

    /// The instructions file's text is the job's block as given, after the cache mark; a translation is the same
    /// whatever the instructions say.
    #[test]
    fn the_instructions_are_the_cleanup_block_as_given() {
        for t in [&dict(&[]), &dict(&["Soniox"])] {
            let bold = request_body_for("claude-haiku-5-5", "ну привет", &ctx(), t, Job::Rewrite);
            let custom = request_body_styled("claude-haiku-5-5", "ну привет", &ctx(), t, Job::Rewrite, "пиши коротко");
            let mut system = bold["system"].as_array().unwrap().clone();
            *system.last_mut().unwrap() = json!({ "type": "text", "text": "пиши коротко" });
            assert_eq!(custom["system"], json!(system));
            let mut rest = custom.clone();
            rest["system"] = bold["system"].clone();
            assert_eq!(rest, bold, "nothing else changes");
            let to_en = Job::Translate("en");
            let translation = request_body_for("claude-haiku-5-5", "ну привет", &ctx(), t, to_en);
            assert_eq!(request_body_styled("claude-haiku-5-5", "ну привет", &ctx(), t, to_en, "пиши коротко"), translation);
        }
    }

    #[test]
    fn decide_translation_checks_the_script_not_the_words() {
        let raw = "ну привет";
        assert_eq!(decide_translation(raw, Parsed::Text("**Hello.**".into()), &Default::default(), "en"), ("Hello.".into(), RefineOutcome::Refined));
        assert_eq!(decide_translation(raw, Parsed::Text("Ну привет.".into()), &Default::default(), "en"), (raw.into(), RefineOutcome::RawFallback));
        assert_eq!(decide_translation(raw, Parsed::Truncated, &Default::default(), "en"), (raw.into(), RefineOutcome::RawFallback));
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
        run_with(key, RAW, Job::Clean, route).await
    }

    /// The speech of the translation tests.
    const SPEECH: &str = "ну привет";

    /// A translation of [`SPEECH`] into English, as [`refine_with`].
    async fn translate_with<F>(key: Option<&str>, route: F) -> (RefineResult, Vec<String>, FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        run_with(key, SPEECH, Job::Translate("en"), route).await
    }

    async fn run_with<F>(key: Option<&str>, raw: &str, job: Job, route: F) -> (RefineResult, Vec<String>, FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        let api = fake_api(route).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let (logs, _guard) = capture_logs();
        let url = format!("{}/v1/messages", api.base);
        let r = refine_at(&url, &http, key, "claude-haiku-5-5", raw, &ctx(), &Default::default(), &RefineSettings::default(), BOLD_PROMPT, job).await;
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
        assert_eq!(body, request_body("claude-haiku-5-5", RAW, &ctx(), &Default::default()));
        assert_eq!(req.body, serde_json::to_vec(&body).unwrap(), "the body goes out exactly as `request_body` serializes");
        for banned in ["temperature", "top_p", "top_k"] {
            assert!(body.get(banned).is_none(), "{banned} is not sent");
        }
    }

    #[tokio::test]
    async fn refine_sends_the_dictionary() {
        let api = fake_api(|_, _, _| reply(200, &message("Привет, как дела?", "end_turn"))).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/v1/messages", api.base);
        let terms = dict(&["Soniox"]);
        let r = refine_at(&url, &http, Some(KEY), "claude-haiku-5-5", RAW, &ctx(), &terms, &RefineSettings::default(), BOLD_PROMPT, Job::Clean).await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        assert_eq!(api.body(0), serde_json::to_vec(&request_body("claude-haiku-5-5", RAW, &ctx(), &terms)).unwrap());
    }

    #[tokio::test]
    async fn refine_checks_the_answer_against_the_dictionary_too() {
        let api = fake_api(|_, _, _| reply(200, &message("Открой GitHub.", "end_turn"))).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/v1/messages", api.base);
        let s = RefineSettings::default();
        let r = refine_at(&url, &http, Some(KEY), "claude-haiku-5-5", "открой гитхаб", &ctx(), &dict(&["GitHub"]), &s, BOLD_PROMPT, Job::Clean).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Открой GitHub.", RefineOutcome::Refined));
        let r = refine_at(&url, &http, Some(KEY), "claude-haiku-5-5", "открой гитхаб", &ctx(), &Default::default(), &s, BOLD_PROMPT, Job::Clean).await;
        assert_eq!((r.text.as_str(), r.outcome), ("открой гитхаб", RefineOutcome::RawFallback));
    }

    #[tokio::test]
    async fn refine_http_refusal_returns_raw_text() {
        for (code, outcome) in [(400, RefineOutcome::RawFallback), (401, RefineOutcome::RawKeyProblem), (402, RefineOutcome::RawKeyProblem)] {
            let body = format!(r#"{{"type":"error","error":{{"message":"BODY-MARK привет {KEY}"}}}}"#);
            let (r, lines, api) = refine_with(Some(KEY), move |_, _, _| reply(code, &body)).await;
            assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, outcome, Usage::default()), "HTTP {code}");
            assert_eq!(api.calls().len(), 1, "HTTP {code} is not asked again");
            assert_one_warning(&lines, &format!("HTTP {code}"));
        }
    }

    /// Two log lines of a request asked again: the first failure (INFO) and the final outcome (WARN).
    fn assert_asked_again(lines: &[String], why: &str) {
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("INFO") && lines[0].contains(why) && lines[0].contains("asking again"), "{}", lines[0]);
        assert_one_warning(&lines[1..], why);
    }

    #[tokio::test]
    async fn refine_asks_an_overloaded_service_again() {
        for code in [500, 502, 503, 504, 529] {
            let (r, lines, api) = refine_with(Some(KEY), move |_, _, n| {
                if n == 0 { reply(code, r#"{"type":"error"}"#) } else { reply(200, &message("Привет, как дела?", "end_turn")) }
            })
            .await;
            assert_eq!((r.text.as_str(), r.outcome, r.usage), ("Привет, как дела?", RefineOutcome::Refined, USAGE), "HTTP {code}");
            assert_eq!(api.calls().len(), 2, "HTTP {code}");
            assert_eq!(api.body(0), api.body(1), "the same request goes again");
            assert_eq!(lines.len(), 1, "{lines:?}");
            assert!(lines[0].starts_with("INFO") && lines[0].contains(&format!("HTTP {code}")), "{}", lines[0]);
        }
    }

    #[tokio::test]
    async fn refine_returns_raw_text_when_the_second_request_fails_too() {
        let (r, lines, api) = refine_with(Some(KEY), |_, _, _| reply(529, r#"{"type":"error"}"#)).await;
        assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, RefineOutcome::RawFallback, Usage::default()));
        assert_eq!(api.calls().len(), 2, "asked once more, not more");
        assert_asked_again(&lines, "HTTP 529");
    }

    #[tokio::test]
    async fn refine_asks_again_after_a_network_failure() {
        let (r, lines, api) =
            refine_with(Some(KEY), |_, _, n| if n == 0 { Reply::Close } else { reply(200, &message("Привет, как дела?", "end_turn")) }).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Привет, как дела?", RefineOutcome::Refined));
        assert_eq!(api.calls().len(), 2);
        assert_eq!(lines.len(), 1, "{lines:?}");
    }

    #[test]
    fn limits_and_bad_requests_are_not_asked_again() {
        for code in [400, 401, 402, 403, 404, 429] {
            assert!(!is_transient(code), "HTTP {code}");
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
        let (r, lines, api) = refine_with(Some(KEY), |_, _, _| Reply::Close).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        assert_eq!(api.calls().len(), 2);
        assert_asked_again(&lines, "network");
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

    #[tokio::test]
    async fn translate_returns_the_translation() {
        let (r, lines, api) = translate_with(Some(KEY), |_, _, _| reply(200, &message("Hello.", "end_turn"))).await;
        assert_eq!((r.text.as_str(), r.outcome, r.usage), ("Hello.", RefineOutcome::Refined, USAGE));
        assert!(lines.is_empty(), "a success leaves the log alone: {lines:?}");
        let body: Value = serde_json::from_slice(&api.body(0)).unwrap();
        assert_eq!(body, request_body_for("claude-haiku-5-5", SPEECH, &ctx(), &Default::default(), Job::Translate("en")));
        assert_eq!(body["system"][1]["text"], translate::instruction("en"), "the translation block is sent");
    }

    #[tokio::test]
    async fn untranslated_answer_returns_the_raw_text_with_its_reason() {
        let (r, lines, _api) = translate_with(Some(KEY), |_, _, _| reply(200, &message("Ну привет.", "end_turn"))).await;
        assert_eq!((r.text.as_str(), r.outcome, r.usage), (SPEECH, RefineOutcome::RawFallback, USAGE));
        assert_one_warning(&lines, "translation failed the check");
    }

    /// A listed name is no evidence of translation: respelling the listed names is a cleanup, not a translation.
    #[tokio::test]
    async fn translate_checks_the_answer_against_the_dictionary_too() {
        let api = fake_api(|_, _, _| reply(200, &message("Kubernetes и Docker.", "end_turn"))).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/v1/messages", api.base);
        let (s, raw, job) = (RefineSettings::default(), "кубернетес и докер", Job::Translate("en"));
        let terms = dict(&["Kubernetes", "Docker"]);
        let r = refine_at(&url, &http, Some(KEY), "claude-haiku-5-5", raw, &ctx(), &terms, &s, BOLD_PROMPT, job).await;
        assert_eq!((r.text.as_str(), r.outcome), (raw, RefineOutcome::RawFallback));
    }

    #[tokio::test]
    async fn translate_key_problem_returns_the_raw_text() {
        let (r, lines, _api) = translate_with(Some(KEY), |_, _, _| reply(401, r#"{"type":"error"}"#)).await;
        assert_eq!((r.text.as_str(), r.outcome), (SPEECH, RefineOutcome::RawKeyProblem));
        assert_one_warning(&lines, "HTTP 401");
    }

    // ---- Cache warm-up (spec 2 §6.3) ----

    /// The warm-up writes the prefix real refine reads: the same model, system blocks with the cache mark, thinking and
    /// answer format; a translation adds its block after that prefix.
    #[test]
    fn warm_body_builds_the_dictation_prefix() {
        for t in [&dict(&[]), &dict(&["Soniox"])] {
            for model in ["claude-haiku-5-5", "claude-sonnet-5-5"] {
                let warm = warm_body(model, t);
                let real = request_body(model, "ну привет", &ctx(), t);
                for field in ["model", "system", "thinking", "output_config"] {
                    assert_eq!(warm[field], real[field], "{model}: {field}");
                }
                assert_eq!(warm["max_tokens"], 1, "{model}");
                let tr = request_body_for(model, "ну привет", &ctx(), t, Job::Translate("en"));
                let prefix = warm["system"].as_array().unwrap();
                assert_eq!(&tr["system"].as_array().unwrap()[..prefix.len()], &prefix[..], "{model}: translation reads it too");
            }
        }
    }

    #[test]
    fn no_warm_up_for_a_model_whose_thinking_cannot_be_turned_off() {
        assert!(can_warm("claude-haiku-5-5"));
        assert!(can_warm("claude-sonnet-5-5"));
        assert!(!can_warm("claude-opus-5-5"));
    }

    /// A warm-up against a fake Messages API: the result, the claude log lines and the service itself.
    async fn warm_with<F>(route: F) -> (Option<Usage>, Vec<String>, FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        let api = fake_api(route).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let (logs, _guard) = capture_logs();
        let url = format!("{}/v1/messages", api.base);
        let u = warm_at(&url, &http, KEY, "claude-haiku-5-5", &Default::default()).await;
        let lines = logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::claude")).cloned().collect();
        (u, lines, api)
    }

    #[tokio::test]
    async fn warm_returns_the_tokens_paid_for() {
        let body = json!({
            "content": [{ "type": "text", "text": "{" }],
            "stop_reason": "max_tokens",
            "usage": { "input_tokens": 12, "output_tokens": 1, "cache_creation_input_tokens": 900, "cache_read_input_tokens": 0 }
        })
        .to_string();
        let (u, lines, api) = warm_with(move |_, _, _| reply(200, &body)).await;
        let paid = Usage { input_tokens: 12, output_tokens: 1, cache_read_input_tokens: 0, cache_creation_input_tokens: 900 };
        assert_eq!(u, Some(paid));
        assert!(lines.is_empty(), "a success leaves the log alone: {lines:?}");
        let req = &api.seen()[0];
        assert_eq!(api.calls(), ["POST /v1/messages"]);
        assert_eq!(req.header("x-api-key"), [KEY]);
        assert_eq!(req.header("anthropic-version"), ["2026-10-21"]);
        assert_eq!(req.body, serde_json::to_vec(&warm_body("claude-haiku-5-5", &Default::default())).unwrap());
    }

    #[tokio::test]
    async fn warm_failure_is_one_log_line_and_no_tokens() {
        let refusal = format!(r#"{{"type":"error","error":{{"message":"BODY-MARK {KEY}"}}}}"#);
        let (u, lines, _api) = warm_with(move |_, _, _| reply(500, &refusal)).await;
        assert_eq!(u, None);
        assert_one_warning(&lines, "cache warm-up: HTTP 500");
        let (u, lines, _api) = warm_with(|_, _, _| Reply::Close).await;
        assert_eq!(u, None);
        assert_one_warning(&lines, "cache warm-up: network");
        let (u, lines, _api) = warm_with(|_, _, _| reply(200, r#"{"content":[]}"#)).await;
        assert_eq!(u, None);
        assert_one_warning(&lines, "cache warm-up: HTTP 200 response not parsed");
    }
}
