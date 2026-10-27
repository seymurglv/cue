//! OpenRouter: models from different providers through one API (Chat Completions, SSE stream) — refine candidates.
//! The prompt, the response budget and the result check are the same as for Claude: `cue_core::refine`
//! and [`crate::claude::decide`].
//!
//! The key goes only in the `Authorization` header and never appears in error texts. Success bodies (they hold
//! dictation text) never get into errors; from a refusal only `error.message` is taken, without `metadata`
//! (a moderation refusal quotes the input there).
use std::time::{Duration, Instant};

use cue_core::refine::{max_tokens, user_message, RefineContext, SYSTEM_PROMPT};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::claude::Parsed;
use crate::error::{checked, net, read_json, EngineError};

pub const API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// The public model list: no key, free.
pub const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
/// The app name in the `X-Title` header (OpenRouter statistics).
const TITLE: &str = "CUE";

/// Response budget headroom for a model whose reasoning cannot be turned off: it goes into the same `max_tokens`,
/// otherwise the response is cut by length (like `THINKING_HEADROOM` for Claude).
pub const REASONING_HEADROOM: u32 = 2_048;

/// Reasoning efforts, from lowest to highest.
const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// How to handle the model's reasoning (the request's `reasoning` field).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reasoning {
    /// The model does not reason (`reasoning` is not in `supported_parameters`) — the field is omitted.
    Absent,
    /// Can be turned off: `{"effort": "none"}`.
    EffortNone,
    /// Can be turned off, but the model lists no efforts: `{"enabled": false}`.
    Disabled,
    /// Mandatory: the model's lowest effort (`None` — the model lists none), and the reasoning itself
    /// is not sent in the stream (`exclude`). The response budget needs [`REASONING_HEADROOM`].
    Lowest(Option<String>),
}

impl Reasoning {
    /// The request's `reasoning` field; `None` — omitted.
    pub fn param(&self) -> Option<Value> {
        match self {
            Reasoning::Absent => None,
            Reasoning::EffortNone => Some(json!({ "effort": "none" })),
            Reasoning::Disabled => Some(json!({ "enabled": false })),
            Reasoning::Lowest(Some(effort)) => Some(json!({ "effort": effort, "exclude": true })),
            Reasoning::Lowest(None) => Some(json!({ "exclude": true })),
        }
    }

    /// Reasoning cannot be turned off.
    pub fn mandatory(&self) -> bool {
        matches!(self, Reasoning::Lowest(_))
    }
}

/// What we send to the model, based on its entry in the public list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// A response strictly in the `{"text": …}` schema (`structured_outputs` in `supported_parameters`); otherwise plain text.
    pub structured: bool,
    pub reasoning: Reasoning,
    /// A cache mark on the system prompt. Anthropic and Qwen cache only by marks; OpenAI, DeepSeek, Z.AI
    /// and Gemini cache on their own.
    pub cache_mark: bool,
}

/// A model from the public list `GET /api/v1/models` — only what the request and the price estimate need.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default, deserialize_with = "nullable")]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub reasoning: Option<ReasoningInfo>,
    #[serde(default, deserialize_with = "nullable")]
    pub pricing: Pricing,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ReasoningInfo {
    #[serde(default, deserialize_with = "nullable")]
    pub mandatory: bool,
    #[serde(default, deserialize_with = "nullable")]
    pub supported_efforts: Vec<String>,
}

/// Prices in dollars per token — OpenRouter gives them as strings ("0.0000001").
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Pricing {
    #[serde(default)]
    pub prompt: Option<Value>,
    #[serde(default)]
    pub completion: Option<Value>,
    #[serde(default)]
    pub input_cache_read: Option<Value>,
}

/// `null` in the list counts as a missing field.
fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Option::unwrap_or_default)
}

impl ModelInfo {
    /// Reasoning: optional reasoning is turned off (`effort: none` if the model lists it, otherwise
    /// `enabled: false`); mandatory reasoning gets the lowest effort. The schema is used if the model supports `structured_outputs`.
    pub fn profile(&self) -> Profile {
        let has = |p: &str| self.supported_parameters.iter().any(|s| s == p);
        let reasoning = if !has("reasoning") {
            Reasoning::Absent
        } else {
            let info = self.reasoning.clone().unwrap_or_default();
            let listed = |e: &str| info.supported_efforts.iter().any(|s| s == e);
            if !info.mandatory {
                if listed("none") { Reasoning::EffortNone } else { Reasoning::Disabled }
            } else {
                Reasoning::Lowest(EFFORTS.into_iter().find(|e| listed(e)).map(str::to_owned))
            }
        };
        Profile {
            structured: has("structured_outputs"),
            reasoning,
            cache_mark: ["anthropic/", "qwen/"].iter().any(|p| self.id.starts_with(p)),
        }
    }

    /// Request price from the list prices — for when the stream's final chunk has no `cost`. Cache reads are priced at the cache
    /// price, or at the input price if there is none. `None` — no input and output prices.
    pub fn estimate_cost(&self, u: &Usage) -> Option<f64> {
        let (input, output) = (per_token(&self.pricing.prompt)?, per_token(&self.pricing.completion)?);
        let cache_read = per_token(&self.pricing.input_cache_read).unwrap_or(input);
        let cached = u.cached_tokens.min(u.prompt_tokens);
        Some((u.prompt_tokens - cached) as f64 * input + cached as f64 * cache_read + u.completion_tokens as f64 * output)
    }
}

/// Per-token price from the list: a string or a number; a negative one ("-1" means the price floats) is unknown.
fn per_token(v: &Option<Value>) -> Option<f64> {
    let p = match v.as_ref()? {
        Value::String(s) => s.trim().parse().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    (p >= 0.0 && p.is_finite()).then_some(p)
}

/// Routing conditions (`provider`). Among the matching providers the fastest is always taken (`sort: latency`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Routing {
    /// `data_collection: deny` — only providers that do not store requests for training.
    pub deny_data_collection: bool,
    /// `require_parameters` — only providers that support every request parameter (the schema, turning
    /// reasoning off). Without it an unsupported parameter is silently skipped.
    pub require_parameters: bool,
}

impl Routing {
    pub const STRICT: Routing = Routing { deny_data_collection: true, require_parameters: true };

    /// Conditions for the next attempt when no provider matches these ([`Failure::no_provider`]):
    /// first without `deny`, then also without `require_parameters`; `None` — nothing left to relax.
    pub fn relaxed(self) -> Option<Routing> {
        if self.deny_data_collection {
            Some(Routing { deny_data_collection: false, ..self })
        } else if self.require_parameters {
            Some(Routing { require_parameters: false, ..self })
        } else {
            None
        }
    }
}

/// Refine request body: the app's prompt, the [`max_tokens`] budget (with headroom if reasoning is mandatory),
/// streaming, reasoning per [`Profile::reasoning`], the `{"text": …}` schema (the same as for Claude), routing per [`Routing`].
pub fn request_body(model: &str, p: &Profile, routing: Routing, raw: &str, ctx: &RefineContext) -> Value {
    let budget = max_tokens(raw) + if p.reasoning.mandatory() { REASONING_HEADROOM } else { 0 };
    let system = if p.cache_mark {
        json!([{ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } }])
    } else {
        json!(SYSTEM_PROMPT)
    };
    let mut provider = json!({ "sort": "latency" });
    if routing.deny_data_collection {
        provider["data_collection"] = json!("deny");
    }
    if routing.require_parameters {
        provider["require_parameters"] = json!(true);
    }
    let mut v = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_message(ctx, raw) }
        ],
        "max_tokens": budget,
        "stream": true,
        "provider": provider,
    });
    if let Some(r) = p.reasoning.param() {
        v["reasoning"] = r;
    }
    if p.structured {
        v["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {
                "name": "refined_text",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }
        });
    }
    v
}

/// Request usage, from the last stream chunk.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    /// All input, including cache reads.
    pub prompt_tokens: u64,
    /// All output, including reasoning.
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    /// Charged to the balance, in dollars; `None` — OpenRouter did not report it.
    pub cost: Option<f64>,
}

/// The whole model response.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    pub content: String,
    /// Normalized finish reason: `stop`, `length`, `content_filter`…
    pub finish_reason: Option<String>,
    /// The finish reason as the provider named it.
    pub native_finish_reason: Option<String>,
    /// `None` — there was no chunk with usage.
    pub usage: Option<Usage>,
    /// Who responded (`provider` of the stream chunks).
    pub provider: Option<String>,
    /// Which model responded (`model` of the stream chunks).
    pub model: Option<String>,
    /// From sending the request to the first chunk of response text; `None` — there was no text.
    pub ttft: Option<Duration>,
    /// From sending the request to the end of the stream.
    pub total: Duration,
}

/// A request failure.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub error: EngineError,
    /// HTTP status of a refusal before the stream started; `None` — network, deadline or a failure mid-stream.
    pub status: Option<u16>,
}

impl Failure {
    /// No provider matches the routing conditions (404 or 503 before the stream started): with `data_collection: deny`
    /// it is worth retrying without it.
    pub fn no_provider(&self) -> bool {
        matches!(self.status, Some(404 | 503))
    }
}

/// Response text → parse result (like [`crate::claude::parse_response`]): cut by length is `Truncated`,
/// the provider's filter is `Refusal`; otherwise the `text` field of a JSON object (also inside a ```` ``` ```` fence).
/// Without the schema (`structured == false`) the whole plain text is accepted too — except an empty one and a JSON fragment.
pub fn parse_content(content: &str, finish_reason: Option<&str>, structured: bool) -> Parsed {
    match finish_reason {
        Some("length") => return Parsed::Truncated,
        Some("content_filter") => return Parsed::Refusal,
        _ => {}
    }
    let content = content.trim();
    let json = unfence(content);
    if let Some(text) = serde_json::from_str::<Value>(json).ok().and_then(|v| v.get("text")?.as_str().map(str::to_owned)) {
        return Parsed::Text(text);
    }
    if structured || content.is_empty() || json.starts_with('{') {
        return Parsed::Malformed;
    }
    Parsed::Text(content.to_owned())
}

/// JSON inside a ```` ```json … ``` ```` fence → what is inside; otherwise as is.
fn unfence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else { return text };
    let Some((_, body)) = rest.split_once('\n') else { return text };
    body.trim_end().strip_suffix("```").map(str::trim).unwrap_or(text)
}

/// SSE stream parsing from byte chunks. Chunks may be cut anywhere — even mid UTF-8 character: a line
/// is parsed whole. An event's `data:` fields are joined with `\n`, and an event ends with an empty line;
/// comments (`:` at the start of a line, OpenRouter's `: OPENROUTER PROCESSING`) and other fields are skipped.
#[derive(Debug, Default)]
struct Sse {
    line: Vec<u8>,
    data: Option<String>,
}

impl Sse {
    /// Data of the events this chunk completed.
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                let line = std::mem::take(&mut self.line);
                self.line_done(&line, &mut out);
            } else {
                self.line.push(b);
            }
        }
        out
    }

    /// End of stream: an unfinished line and an event with no empty line after it.
    fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let line = std::mem::take(&mut self.line);
        if !line.is_empty() {
            self.line_done(&line, &mut out);
        }
        out.extend(self.data.take());
        out
    }

    fn line_done(&mut self, line: &[u8], out: &mut Vec<String>) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            out.extend(self.data.take());
            return;
        }
        if line.starts_with(b":") {
            return;
        }
        let line = String::from_utf8_lossy(line);
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        if field == "data" {
            match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            }
        }
    }
}

/// What the next stream chunk brought.
#[derive(Debug, PartialEq)]
enum Step {
    More,
    /// The first non-empty chunk of response text.
    FirstText,
    /// `[DONE]` — the stream has ended.
    Done,
}

/// Stream chunks → response.
#[derive(Debug, Default)]
struct Acc {
    c: Completion,
}

impl Acc {
    /// Data of one event. A failure mid-stream (`error` in a chunk or finish reason `error`) is an error;
    /// non-JSON data is skipped (as the OpenRouter documentation advises).
    fn apply(&mut self, data: &str, key: &str) -> Result<Step, EngineError> {
        if data.trim() == "[DONE]" {
            return Ok(Step::Done);
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { return Ok(Step::More) };
        if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
            return Err(stream_error(e, key));
        }
        let c = &mut self.c;
        for (slot, field) in [(&mut c.provider, "provider"), (&mut c.model, "model")] {
            if slot.is_none() {
                *slot = v.get(field).and_then(Value::as_str).map(str::to_owned);
            }
        }
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            c.usage = serde_json::from_value::<RawUsage>(u.clone()).ok().map(Usage::from);
        }
        let choice = &v["choices"][0];
        if let Some(reason) = choice["finish_reason"].as_str() {
            if reason == "error" {
                return Err(EngineError::Service("the provider cut the response short (finish_reason: error)".into()));
            }
            c.finish_reason = Some(reason.to_owned());
        }
        if let Some(reason) = choice["native_finish_reason"].as_str() {
            c.native_finish_reason = Some(reason.to_owned());
        }
        let piece = choice["delta"]["content"].as_str().unwrap_or("");
        let first = !piece.is_empty() && c.content.is_empty();
        c.content.push_str(piece);
        Ok(if first { Step::FirstText } else { Step::More })
    }

    /// The whole response: the stream ended `total` after sending.
    fn done(self, total: Duration) -> Completion {
        Completion { total, ..self.c }
    }
}

/// An error mid-stream (`error` in a chunk) → kind by `code` (a number is treated as an HTTP status, otherwise a service failure).
/// The text holds only `message` without the key; `metadata` is not taken.
fn stream_error(e: &Value, key: &str) -> EngineError {
    let message = e["message"].as_str().unwrap_or("");
    let code = e["code"].as_u64().filter(|c| (400..=599).contains(c)).map_or(502, |c| c as u16);
    EngineError::refusal(code, Some("failure mid-response"), message, key)
}

/// A refusal before the stream started → error by status. From the body, only `error.message` (a non-JSON body gives
/// just the status reason): `metadata` may quote the input.
fn refused(code: u16, reason: Option<&str>, body: &str, key: &str) -> EngineError {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_default();
    EngineError::refusal(code, reason, &message, key)
}

/// A failure of the request or of reading the stream: a passed deadline is `Timeout`, anything else is network.
fn transport(e: &reqwest::Error) -> EngineError {
    if e.is_timeout() { EngineError::Timeout } else { net(e) }
}

/// `usage` from the stream. `null` instead of a number or details counts as a missing field.
#[derive(Deserialize)]
struct RawUsage {
    #[serde(default, deserialize_with = "nullable")]
    prompt_tokens: u64,
    #[serde(default, deserialize_with = "nullable")]
    completion_tokens: u64,
    #[serde(default)]
    cost: Option<f64>,
    #[serde(default, deserialize_with = "nullable")]
    prompt_tokens_details: PromptDetails,
    #[serde(default, deserialize_with = "nullable")]
    completion_tokens_details: CompletionDetails,
}

#[derive(Default, Deserialize)]
struct PromptDetails {
    #[serde(default, deserialize_with = "nullable")]
    cached_tokens: u64,
    #[serde(default, deserialize_with = "nullable")]
    cache_write_tokens: u64,
}

#[derive(Default, Deserialize)]
struct CompletionDetails {
    #[serde(default, deserialize_with = "nullable")]
    reasoning_tokens: u64,
}

impl From<RawUsage> for Usage {
    fn from(u: RawUsage) -> Self {
        Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            cached_tokens: u.prompt_tokens_details.cached_tokens,
            cache_write_tokens: u.prompt_tokens_details.cache_write_tokens,
            reasoning_tokens: u.completion_tokens_details.reasoning_tokens,
            cost: u.cost,
        }
    }
}

/// One streaming refine request: the whole response and the time to first text. The `timeout` covers the whole request,
/// including reading the stream.
pub async fn complete(http: &reqwest::Client, key: &str, body: &Value, timeout: Duration) -> Result<Completion, Failure> {
    complete_at(API_URL, http, key, body, timeout).await
}

/// [`complete`] against the URL `url`; tests substitute their own service.
async fn complete_at(
    url: &str,
    http: &reqwest::Client,
    key: &str,
    body: &Value,
    timeout: Duration,
) -> Result<Completion, Failure> {
    let fail = |error: EngineError, status: Option<u16>| Failure { error: error.redact(key), status };
    let t0 = Instant::now();
    let send = http.post(url).bearer_auth(key).header("X-Title", TITLE).timeout(timeout).json(body).send().await;
    let mut resp = send.map_err(|e| fail(transport(&e), None))?;
    let status = resp.status();
    if !status.is_success() {
        // Before the stream starts, a refusal comes as plain JSON (OpenRouter documentation, "Streaming").
        let text = resp.text().await.unwrap_or_default();
        let code = status.as_u16();
        return Err(fail(refused(code, status.canonical_reason(), &text, key), Some(code)));
    }
    let (mut sse, mut acc) = (Sse::default(), Acc::default());
    loop {
        let (events, end) = match resp.chunk().await {
            Ok(Some(bytes)) => (sse.push(&bytes), false),
            Ok(None) => (sse.finish(), true),
            Err(e) => return Err(fail(transport(&e), None)),
        };
        for data in events {
            match acc.apply(&data, key).map_err(|e| fail(e, None))? {
                Step::FirstText => acc.c.ttft = Some(t0.elapsed()),
                Step::Done => {
                    let done = acc.done(t0.elapsed());
                    // The end of the body (the final empty chunk) may arrive after `[DONE]`. A response dropped
                    // unread closes the connection, and the next request pays for a new TLS connection;
                    // we finish reading in the background — the result does not wait for it. The request deadline applies here too.
                    tokio::spawn(async move { while let Ok(Some(_)) = resp.chunk().await {} });
                    return Ok(done);
                }
                Step::More => {}
            }
        }
        if end {
            break;
        }
    }
    // The stream ended without `[DONE]`: the response is usable if a finish reason arrived; otherwise the connection dropped.
    if acc.c.finish_reason.is_some() {
        Ok(acc.done(t0.elapsed()))
    } else {
        Err(fail(EngineError::Network("the stream dropped before the response ended".into()), None))
    }
}

/// Result of [`complete_routed`].
#[derive(Debug)]
pub struct Routed {
    pub result: Result<Completion, Failure>,
    /// Routing conditions of the last attempt.
    pub routing: Routing,
    /// "No provider" refusals before the retries, with the conditions they were received under.
    pub refusals: Vec<(Routing, Failure)>,
    /// Time of the last attempt (for a success, the same as [`Completion::total`]).
    pub elapsed: Duration,
}

/// [`complete`] with routing relaxation: if no provider matches `routing` ([`Failure::no_provider`]), retry
/// under [`Routing::relaxed`] while there is something left to relax. The request body is `body(conditions)`. Such refusals are free.
pub async fn complete_routed(
    http: &reqwest::Client,
    key: &str,
    routing: Routing,
    timeout: Duration,
    body: impl Fn(Routing) -> Value,
) -> Routed {
    complete_routed_at(API_URL, http, key, routing, timeout, body).await
}

async fn complete_routed_at(
    url: &str,
    http: &reqwest::Client,
    key: &str,
    mut routing: Routing,
    timeout: Duration,
    body: impl Fn(Routing) -> Value,
) -> Routed {
    let mut refusals = Vec::new();
    loop {
        let t0 = Instant::now();
        let result = complete_at(url, http, key, &body(routing), timeout).await;
        let elapsed = t0.elapsed();
        match (result, routing.relaxed()) {
            (Err(f), Some(next)) if f.no_provider() => {
                refusals.push((routing, f));
                routing = next;
            }
            (result, _) => return Routed { result, routing, refusals, elapsed },
        }
    }
}

/// The public model list (no key). Entries that cannot be parsed are skipped.
pub async fn models(http: &reqwest::Client) -> Result<Vec<ModelInfo>, EngineError> {
    models_at(MODELS_URL, http).await
}

async fn models_at(url: &str, http: &reqwest::Client) -> Result<Vec<ModelInfo>, EngineError> {
    #[derive(Deserialize)]
    struct List {
        data: Vec<Value>,
    }
    let resp = http.get(url).send().await.map_err(|e| transport(&e))?;
    let list: List = read_json(checked(resp, "").await?).await?;
    Ok(list.data.into_iter().filter_map(|m| serde_json::from_value(m).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fake_api, reply, Reply};

    const KEY: &str = "sk-or-v1-TEST-0123456789";

    fn ctx() -> RefineContext {
        RefineContext { app: "Telegram Desktop".into(), before_cursor: None }
    }

    // ---- SSE ----

    /// An OpenRouter stream chunk: text `content`, finish reason `finish` (`None` — still going).
    fn chunk(content: &str, finish: Option<&str>) -> String {
        json!({
            "id": "gen-1", "provider": "OpenAI", "model": "openai/gpt-6-luna", "object": "chat.completion.chunk",
            "created": 1_790_000_000,
            "choices": [{ "index": 0, "delta": { "role": "assistant", "content": content },
                          "finish_reason": finish, "native_finish_reason": finish.map(|_| "completed") }]
        })
        .to_string()
    }

    /// The final chunk with usage: it repeats the stream's finish reason (that is how OpenRouter does it).
    fn usage_chunk() -> String {
        json!({
            "id": "gen-1", "provider": "OpenAI", "model": "openai/gpt-6-luna", "object": "chat.completion.chunk",
            "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "stop", "native_finish_reason": "completed" }],
            "usage": { "prompt_tokens": 905, "completion_tokens": 14, "total_tokens": 919, "cost": 0.0000973,
                       "prompt_tokens_details": { "cached_tokens": 768, "cache_write_tokens": 0 },
                       "completion_tokens_details": { "reasoning_tokens": 3 } }
        })
        .to_string()
    }

    /// A whole stream as OpenRouter sends it: a comment, the text in two chunks, the finish reason, usage, `[DONE]`.
    fn stream(nl: &str) -> String {
        let events = [
            ": OPENROUTER PROCESSING".to_owned(),
            format!("data: {}", chunk("", None)),
            format!("data: {}", chunk(r#"{"text":"Привет,"#, None)),
            ": OPENROUTER PROCESSING".to_owned(),
            format!("data: {}", chunk(r#" как дела?"}"#, None)),
            format!("data: {}", chunk("", Some("stop"))),
            format!("data: {}", usage_chunk()),
            "data: [DONE]".to_owned(),
        ];
        events.iter().map(|e| format!("{e}{nl}{nl}")).collect()
    }

    fn expected_events() -> Vec<String> {
        vec![
            chunk("", None),
            chunk(r#"{"text":"Привет,"#, None),
            chunk(r#" как дела?"}"#, None),
            chunk("", Some("stop")),
            usage_chunk(),
            "[DONE]".to_owned(),
        ]
    }

    #[test]
    fn sse_reads_data_and_skips_comments() {
        let mut sse = Sse::default();
        let mut got = sse.push(stream("\n").as_bytes());
        got.extend(sse.finish());
        assert_eq!(got, expected_events());
    }

    #[test]
    fn sse_split_anywhere_gives_same_events() {
        // Every cut point — including the middle of a Cyrillic letter (two bytes each) and between `\r` and `\n`.
        for nl in ["\n", "\r\n"] {
            let bytes = stream(nl).into_bytes();
            for cut in 0..=bytes.len() {
                let mut sse = Sse::default();
                let mut got = sse.push(&bytes[..cut]);
                got.extend(sse.push(&bytes[cut..]));
                got.extend(sse.finish());
                assert_eq!(got, expected_events(), "cut at byte {cut}, line break {nl:?}");
            }
            // Byte by byte.
            let mut sse = Sse::default();
            let mut got: Vec<String> = bytes.iter().flat_map(|b| sse.push(std::slice::from_ref(b))).collect();
            got.extend(sse.finish());
            assert_eq!(got, expected_events(), "byte by byte, line break {nl:?}");
        }
    }

    #[test]
    fn sse_multiline_data_other_fields_and_unterminated_tail() {
        let mut sse = Sse::default();
        let got = sse.push(b"event: message\nid: 7\ndata: a\ndata:b\nretry: 10\n\n:keep-alive\n\ndata: x");
        assert_eq!(got, ["a\nb"], "data fields are joined with a line break, other fields are skipped");
        assert_eq!(sse.finish(), ["x"], "the end of the stream completes both the unfinished line and the event");
        assert!(sse.finish().is_empty());
        assert!(Sse::default().push(b": OPENROUTER PROCESSING\n\n\n\n").is_empty(), "only comments, no events");
    }

    // ---- stream chunks ----

    fn accumulate(events: &[String]) -> (Acc, Vec<Step>) {
        let mut acc = Acc::default();
        let steps = events.iter().map(|d| acc.apply(d, KEY).unwrap()).collect();
        (acc, steps)
    }

    #[test]
    fn stream_collects_text_finish_usage_and_provider() {
        let (acc, steps) = accumulate(&expected_events());
        use Step::*;
        assert_eq!(steps, [More, FirstText, More, More, More, Done], "the first text is the second chunk: the first one is empty");
        let c = acc.c;
        assert_eq!(c.content, r#"{"text":"Привет, как дела?"}"#);
        assert_eq!(c.finish_reason.as_deref(), Some("stop"));
        assert_eq!(c.native_finish_reason.as_deref(), Some("completed"));
        assert_eq!(c.provider.as_deref(), Some("OpenAI"));
        assert_eq!(c.model.as_deref(), Some("openai/gpt-6-luna"));
        let u = Usage {
            prompt_tokens: 905,
            completion_tokens: 14,
            cached_tokens: 768,
            cache_write_tokens: 0,
            reasoning_tokens: 3,
            cost: Some(0.0000973),
        };
        assert_eq!(c.usage, Some(u));
    }

    #[test]
    fn stream_usage_without_details_or_cost() {
        let (acc, _) = accumulate(&[json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 2,
            "prompt_tokens_details": null } })
        .to_string()]);
        let u = acc.c.usage.unwrap();
        assert_eq!((u.prompt_tokens, u.completion_tokens, u.cached_tokens, u.cost), (10, 2, 0, None));
    }

    #[test]
    fn stream_skips_garbage_and_reasoning_deltas() {
        let reasoning = json!({ "choices": [{ "delta": { "content": null, "reasoning": "thinking" } }] }).to_string();
        let (acc, steps) = accumulate(&["not json".into(), reasoning, "{}".into(), chunk("Да", None)]);
        assert_eq!(steps, [Step::More, Step::More, Step::More, Step::FirstText], "time to text is measured by text, not by reasoning");
        assert_eq!(acc.c.content, "Да");
    }

    #[test]
    fn stream_error_event_is_failure_without_key() {
        let mut acc = Acc::default();
        let event = json!({ "id": "gen-1", "provider": "OpenAI",
            "error": { "code": 502, "message": format!("Provider disconnected unexpectedly {KEY}"), "metadata": { "raw": "BODY-MARK" } },
            "choices": [{ "index": 0, "delta": { "content": "" }, "finish_reason": "error" }] });
        let e = acc.apply(&event.to_string(), KEY).unwrap_err();
        assert!(matches!(e, EngineError::Service(_)), "{e:?}");
        let shown = e.to_string();
        assert!(shown.contains("502") && shown.contains("Provider disconnected"), "{shown}");
        assert!(!shown.contains("TEST-01") && !shown.contains("BODY-MARK"), "neither the key nor metadata: {shown}");

        let named = json!({ "error": { "code": "server_error", "message": "boom" } }).to_string();
        assert!(matches!(Acc::default().apply(&named, KEY), Err(EngineError::Service(m)) if m.contains("boom")));
        let limit = json!({ "error": { "code": 429, "message": "rate limited" } }).to_string();
        assert!(Acc::default().apply(&limit, KEY).unwrap_err().is_key_problem());
        let bare = json!({ "choices": [{ "delta": { "content": "" }, "finish_reason": "error" }] }).to_string();
        assert!(matches!(Acc::default().apply(&bare, KEY), Err(EngineError::Service(_))), "finish reason error without an error object");
    }

    // ---- text parsing ----

    #[test]
    fn parse_content_reads_text_field() {
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, Some("stop"), true), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content(" \n{\"text\": \"Привет.\"}\n", Some("stop"), false), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content("```json\n{\"text\":\"Привет.\"}\n```", Some("stop"), true), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, None, true), Parsed::Text("Привет.".into()), "no finish reason, the text is still usable");
    }

    #[test]
    fn parse_content_plain_text_fallback_only_without_schema() {
        assert_eq!(parse_content("  Привет, как дела?\n", Some("stop"), false), Parsed::Text("Привет, как дела?".into()));
        assert_eq!(parse_content("Привет, как дела?", Some("stop"), true), Parsed::Malformed, "the schema was requested, so plain text is not accepted");
        for broken in [r#"{"text":"Привет"#, r#"{"answer":"Привет"}"#, "", "  \n"] {
            assert_eq!(parse_content(broken, Some("stop"), false), Parsed::Malformed, "{broken:?}");
            assert_eq!(parse_content(broken, Some("stop"), true), Parsed::Malformed, "{broken:?}");
        }
    }

    #[test]
    fn parse_content_truncated_and_filtered() {
        assert_eq!(parse_content(r#"{"text":"Прив"#, Some("length"), true), Parsed::Truncated);
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, Some("length"), false), Parsed::Truncated, "cut off, even if the JSON is intact");
        assert_eq!(parse_content("", Some("content_filter"), true), Parsed::Refusal);
    }

    // ---- model and request body ----

    fn model(v: Value) -> ModelInfo {
        serde_json::from_value(v).unwrap()
    }

    /// `supported_parameters`: the usual ones plus `extra`.
    fn sp(extra: &[&str]) -> Vec<String> {
        ["max_tokens", "response_format", "seed"].iter().chain(extra).map(|s| s.to_string()).collect()
    }

    #[test]
    fn profile_from_model_list() {
        let off = model(json!({ "id": "openai/gpt-6.1-terra", "supported_parameters": sp(&["reasoning", "structured_outputs"]),
            "reasoning": { "mandatory": false, "default_enabled": true, "supported_efforts": ["max", "high", "low", "none"] } }));
        assert_eq!(off.profile(), Profile { structured: true, reasoning: Reasoning::EffortNone, cache_mark: false });

        let qwen = model(json!({ "id": "qwen/qwen3.8-flash", "supported_parameters": sp(&["reasoning", "structured_outputs"]),
            "reasoning": { "mandatory": false, "default_enabled": true, "supports_max_tokens": true } }));
        assert_eq!(qwen.profile(), Profile { structured: true, reasoning: Reasoning::Disabled, cache_mark: true });

        let lite = model(json!({ "id": "google/gemini-3.5-flash-lite", "supported_parameters": sp(&["reasoning", "structured_outputs"]),
            "reasoning": { "mandatory": true, "supported_efforts": ["high", "medium", "low", "minimal"] } }));
        assert_eq!(lite.profile().reasoning, Reasoning::Lowest(Some("minimal".into())));
        assert!(!lite.profile().cache_mark, "Gemini caches on its own");

        let glm = model(json!({ "id": "z-ai/glm-5.3-flash", "supported_parameters": sp(&["reasoning", "structured_outputs"]),
            "reasoning": { "mandatory": true, "supported_efforts": ["max", "high", "low"], "default_effort": "max" } }));
        assert_eq!(glm.profile().reasoning, Reasoning::Lowest(Some("low".into())));

        let mandatory_unlisted = model(json!({ "id": "x/y", "supported_parameters": sp(&["reasoning"]), "reasoning": { "mandatory": true } }));
        assert_eq!(mandatory_unlisted.profile(), Profile { structured: false, reasoning: Reasoning::Lowest(None), cache_mark: false });

        let plain = model(json!({ "id": "anthropic/claude-haiku-5.5", "supported_parameters": null, "reasoning": null, "pricing": null }));
        assert_eq!(plain.profile(), Profile { structured: false, reasoning: Reasoning::Absent, cache_mark: true });
    }

    #[test]
    fn reasoning_param_shapes() {
        assert_eq!(Reasoning::Absent.param(), None);
        assert_eq!(Reasoning::EffortNone.param(), Some(json!({ "effort": "none" })));
        assert_eq!(Reasoning::Disabled.param(), Some(json!({ "enabled": false })));
        assert_eq!(Reasoning::Lowest(Some("low".into())).param(), Some(json!({ "effort": "low", "exclude": true })));
        assert_eq!(Reasoning::Lowest(None).param(), Some(json!({ "exclude": true })));
        assert!(Reasoning::Lowest(None).mandatory() && !Reasoning::EffortNone.mandatory());
    }

    fn profile(structured: bool, reasoning: Reasoning, cache_mark: bool) -> Profile {
        Profile { structured, reasoning, cache_mark }
    }

    #[test]
    fn body_uses_app_prompt_budget_stream_and_routing() {
        let raw = "ну привет как дела";
        let v = request_body("openai/gpt-6-luna", &profile(true, Reasoning::EffortNone, false), Routing::STRICT, raw, &ctx());
        assert_eq!(v["model"], "openai/gpt-6-luna");
        assert_eq!(v["messages"][0], json!({ "role": "system", "content": SYSTEM_PROMPT }));
        assert_eq!(v["messages"][1], json!({ "role": "user", "content": user_message(&ctx(), raw) }));
        assert_eq!(v["max_tokens"], max_tokens(raw));
        assert_eq!(v["stream"], true);
        assert_eq!(v["reasoning"], json!({ "effort": "none" }));
        assert_eq!(v["provider"], json!({ "sort": "latency", "data_collection": "deny", "require_parameters": true }));
        let rf = &v["response_format"];
        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["json_schema"]["strict"], true);
        assert_eq!(rf["json_schema"]["name"], "refined_text");
        let claude = crate::claude::request_body("claude-haiku-5-5", raw, &ctx());
        assert_eq!(rf["json_schema"]["schema"], claude["output_config"]["format"]["schema"], "the same schema as for Claude");
        // Usage always comes (`usage: {include: true}` is deprecated); sampling is left alone, as for Claude.
        for absent in ["usage", "temperature", "top_p", "top_k", "stream_options"] {
            assert!(v.get(absent).is_none(), "{absent} is not sent");
        }
    }

    #[test]
    fn routing_relaxes_deny_first_then_required_parameters() {
        let allow = Routing { deny_data_collection: false, require_parameters: true };
        let loose = Routing { deny_data_collection: false, require_parameters: false };
        assert_eq!(Routing::STRICT, Routing { deny_data_collection: true, require_parameters: true });
        assert_eq!(Routing::STRICT.relaxed(), Some(allow));
        assert_eq!(allow.relaxed(), Some(loose));
        assert_eq!(loose.relaxed(), None);
        let deny_only = Routing { deny_data_collection: true, require_parameters: false };
        assert_eq!(deny_only.relaxed(), Some(loose));
    }

    #[test]
    fn body_plain_text_mandatory_reasoning_cache_mark_and_allow() {
        let raw = "ну привет как дела";
        let allow = Routing { deny_data_collection: false, require_parameters: true };
        let v = request_body("x/y", &profile(false, Reasoning::Lowest(Some("low".into())), true), allow, raw, &ctx());
        assert!(v.get("response_format").is_none(), "no schema means plain text");
        assert_eq!(v["max_tokens"], max_tokens(raw) + REASONING_HEADROOM, "headroom for mandatory reasoning");
        assert_eq!(REASONING_HEADROOM, 2_048);
        assert_eq!(v["reasoning"], json!({ "effort": "low", "exclude": true }));
        assert_eq!(v["provider"], json!({ "sort": "latency", "require_parameters": true }), "without deny the field is omitted");
        assert_eq!(
            v["messages"][0]["content"],
            json!([{ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } }])
        );
        let loose = Routing { deny_data_collection: false, require_parameters: false };
        let v = request_body("x/y", &profile(false, Reasoning::Absent, false), loose, raw, &ctx());
        assert_eq!(v["provider"], json!({ "sort": "latency" }));
        assert!(v.get("reasoning").is_none());
        assert_eq!(v["max_tokens"], max_tokens(raw));
    }

    #[test]
    fn estimate_cost_from_list_prices() {
        let m = model(json!({ "id": "a/b", "pricing": { "prompt": "0.0000001", "completion": "0.0000005", "input_cache_read": "0.00000001" } }));
        let u = Usage { prompt_tokens: 1_000, completion_tokens: 100, cached_tokens: 800, ..Usage::default() };
        // 200 × $0.1/M + 800 × $0.01/M + 100 × $0.5/M.
        let cost = m.estimate_cost(&u).unwrap();
        assert!((cost - (200.0 * 0.1 + 800.0 * 0.01 + 100.0 * 0.5) / 1e6).abs() < 1e-12, "{cost}");
        let no_cache_price = model(json!({ "id": "a/b", "pricing": { "prompt": 0.000001, "completion": "0.000002" } }));
        assert!((no_cache_price.estimate_cost(&u).unwrap() - (1_000.0 + 200.0) / 1e6).abs() < 1e-12, "cache at the input price");
        assert_eq!(model(json!({ "id": "a/b" })).estimate_cost(&u), None);
        assert_eq!(model(json!({ "id": "a/b", "pricing": { "prompt": "-1", "completion": "-1" } })).estimate_cost(&u), None);
    }

    // ---- the whole request against a fake service ----

    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    const LONG: Duration = Duration::from_secs(10);

    async fn call(api: &crate::testing::FakeApi, timeout: Duration) -> Result<Completion, Failure> {
        let body = request_body("openai/gpt-6-luna", &profile(true, Reasoning::EffortNone, false), Routing::STRICT, "ну привет как дела", &ctx());
        complete_at(&format!("{}/api/v1/chat/completions", api.base), &http(), KEY, &body, timeout).await
    }

    #[tokio::test]
    async fn complete_streams_text_and_usage() {
        let api = fake_api(|_, _, _| Reply::Send(200, stream("\n"), "")).await;
        let c = call(&api, LONG).await.unwrap();
        assert_eq!(c.content, r#"{"text":"Привет, как дела?"}"#);
        assert_eq!(c.finish_reason.as_deref(), Some("stop"));
        assert_eq!(c.usage.unwrap().cost, Some(0.0000973));
        assert_eq!(c.provider.as_deref(), Some("OpenAI"));
        assert!(c.ttft.unwrap() <= c.total);
        assert_eq!(parse_content(&c.content, c.finish_reason.as_deref(), true), Parsed::Text("Привет, как дела?".into()));

        assert_eq!(api.calls(), ["POST /api/v1/chat/completions"]);
        let req = &api.seen()[0];
        assert_eq!(req.header("authorization"), [format!("Bearer {KEY}").as_str()]);
        assert_eq!(req.header("x-title"), ["CUE"]);
        assert_eq!(req.header("content-type"), ["application/json"]);
        let sent: Value = serde_json::from_slice(&req.body).unwrap();
        let body = request_body("openai/gpt-6-luna", &profile(true, Reasoning::EffortNone, false), Routing::STRICT, "ну привет как дела", &ctx());
        assert_eq!(sent, body);
    }

    /// No provider (`refusals` times — 404 first, then 503), then a response; routing per the conditions in the request body.
    async fn routed_after(refusals: usize, first: Routing) -> (Routed, crate::testing::FakeApi) {
        let api = fake_api(move |_, _, nth| match nth {
            n if n < refusals => reply(if n == 0 { 404 } else { 503 }, r#"{"error":{"code":404,"message":"No endpoints found"}}"#),
            _ => Reply::Send(200, stream("\n"), ""),
        })
        .await;
        let url = format!("{}/api/v1/chat/completions", api.base);
        let p = profile(true, Reasoning::EffortNone, false);
        let body = |r: Routing| request_body("openai/gpt-6-luna", &p, r, "ну привет", &ctx());
        let routed = complete_routed_at(&url, &http(), KEY, first, LONG, body).await;
        (routed, api)
    }

    fn sent_provider(api: &crate::testing::FakeApi, i: usize) -> Value {
        serde_json::from_slice::<Value>(&api.body(i)).unwrap()["provider"].clone()
    }

    #[tokio::test]
    async fn routed_relaxes_deny_then_required_parameters() {
        let (r, _api) = routed_after(0, Routing::STRICT).await;
        assert!(r.result.is_ok() && r.refusals.is_empty());
        assert_eq!(r.routing, Routing::STRICT);

        let (r, api1) = routed_after(1, Routing::STRICT).await;
        assert!(r.result.is_ok());
        assert_eq!(r.routing, Routing { deny_data_collection: false, require_parameters: true });
        assert_eq!(r.refusals.iter().map(|(rt, f)| (*rt, f.status)).collect::<Vec<_>>(), [(Routing::STRICT, Some(404))]);
        assert_eq!(sent_provider(&api1, 0)["data_collection"], "deny");
        assert!(sent_provider(&api1, 1).get("data_collection").is_none(), "the retry goes without deny");

        let (r, api2) = routed_after(2, Routing::STRICT).await;
        assert!(r.result.is_ok());
        assert_eq!(r.routing, Routing { deny_data_collection: false, require_parameters: false });
        assert_eq!(sent_provider(&api2, 2), json!({ "sort": "latency" }));
        assert_eq!(api2.calls().len(), 3);

        // Nothing left to relax: the refusal stays a refusal.
        let (r, api3) = routed_after(5, Routing::STRICT).await;
        assert_eq!(r.result.unwrap_err().status, Some(503));
        assert_eq!((r.refusals.len(), api3.calls().len()), (2, 3));
    }

    #[tokio::test]
    async fn routed_does_not_retry_other_refusals() {
        let api = fake_api(|_, _, _| reply(401, r#"{"error":{"code":401,"message":"bad key"}}"#)).await;
        let url = format!("{}/api/v1/chat/completions", api.base);
        let p = profile(true, Reasoning::EffortNone, false);
        let r = complete_routed_at(&url, &http(), KEY, Routing::STRICT, LONG, |rt| request_body("m", &p, rt, "да", &ctx())).await;
        assert!(r.result.unwrap_err().error.is_key_problem());
        assert_eq!((r.routing, r.refusals.len(), api.calls().len()), (Routing::STRICT, 0, 1));
    }

    #[tokio::test]
    async fn complete_leaves_connection_for_the_next_request() {
        // The final empty chunk arrives 50 ms after `[DONE]`. The body is read to the end in the background: otherwise
        // the connection does not return to the pool, and every "warm" request pays for a new TLS connection —
        // while the direct path to Anthropic does not. The result does not wait for that read.
        let pieces = vec![stream("\n").into_bytes()];
        let api = fake_api(move |_, _, _| Reply::Stream(200, pieces.clone(), Duration::from_millis(50))).await;
        let (http, url) = (http(), format!("{}/api/v1/chat/completions", api.base));
        let body = request_body("openai/gpt-6-luna", &profile(true, Reasoning::EffortNone, false), Routing::STRICT, "ну привет", &ctx());
        for _ in 0..3 {
            let t0 = Instant::now();
            complete_at(&url, &http, KEY, &body, LONG).await.unwrap();
            assert!(t0.elapsed() < Duration::from_millis(45), "the result does not wait for the end of the body: {:?}", t0.elapsed());
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(api.seen().iter().map(|s| s.conn).collect::<Vec<_>>(), [0, 0, 0], "all three over one connection");
    }

    #[tokio::test]
    async fn complete_measures_first_text_before_the_end() {
        // Three writes 300 ms apart: the first ends mid the first Cyrillic letter (two bytes), the second completes the chunk
        // with the first text, the third brings the rest. Time to text comes from the second write, the end from the third.
        let s = stream("\n").into_bytes();
        let find = |what: &str| s.windows(what.len()).position(|w| w == what.as_bytes()).unwrap();
        let mid_letter = find("Привет") + 1;
        let next_event = find(&format!("data: {}", chunk(r#" как дела?"}"#, None)));
        let pieces = vec![s[..mid_letter].to_vec(), s[mid_letter..next_event].to_vec(), s[next_event..].to_vec()];
        let api = fake_api(move |_, _, _| Reply::Stream(200, pieces.clone(), Duration::from_millis(300))).await;
        let c = call(&api, LONG).await.unwrap();
        assert_eq!(c.content, r#"{"text":"Привет, как дела?"}"#, "a letter split between writes stays intact");
        let ttft = c.ttft.unwrap();
        assert!(ttft >= Duration::from_millis(250), "the first text is in the second write: {ttft:?}");
        assert!(c.total >= ttft + Duration::from_millis(250), "ttft {ttft:?}, total {:?}", c.total);
    }

    #[tokio::test]
    async fn complete_refusal_maps_status_without_key_or_text() {
        for (code, kind) in [(401, "key"), (402, "key"), (429, "key"), (404, "none"), (503, "none"), (400, "protocol"), (502, "service")] {
            let body = json!({ "error": { "code": code, "message": format!("refused {KEY}"),
                "metadata": { "flagged_input": "BODY-MARK привет" } } })
            .to_string();
            let api = fake_api(move |_, _, _| reply(code, &body)).await;
            let f = call(&api, LONG).await.unwrap_err();
            assert_eq!(f.status, Some(code));
            assert_eq!(f.no_provider(), kind == "none", "HTTP {code}");
            match kind {
                "key" => assert!(f.error.is_key_problem(), "HTTP {code}: {f:?}"),
                "protocol" => assert!(matches!(f.error, EngineError::Protocol(_)), "HTTP {code}: {f:?}"),
                _ => assert!(matches!(f.error, EngineError::Service(_) | EngineError::Protocol(_)), "HTTP {code}: {f:?}"),
            }
            let shown = format!("{} {:?}", f.error, f);
            assert!(shown.contains(&code.to_string()) && shown.contains("refused"), "{shown}");
            assert!(!shown.contains("TEST-01") && !shown.contains("BODY-MARK") && !shown.contains("привет"), "{shown}");
        }
        // A non-JSON body (a page, an echo of the request) does not get into the error — only the status reason.
        let api = fake_api(|_, _, _| reply(500, "<html>BODY-MARK ну привет как дела</html>")).await;
        let f = call(&api, LONG).await.unwrap_err();
        assert_eq!(f.error, EngineError::Service("500: Internal Server Error".into()));
    }

    #[tokio::test]
    async fn complete_mid_stream_error_is_failure() {
        let events = [
            format!("data: {}\n\n", chunk(r#"{"text":"При"#, None)),
            format!(
                "data: {}\n\n",
                json!({ "error": { "code": 502, "message": "Provider disconnected unexpectedly" },
                        "choices": [{ "delta": { "content": "" }, "finish_reason": "error" }] })
            ),
        ];
        let api = fake_api(move |_, _, _| Reply::Send(200, events.concat(), "")).await;
        let f = call(&api, LONG).await.unwrap_err();
        assert_eq!(f.status, None, "an error mid-stream: the status was already 200");
        assert!(matches!(&f.error, EngineError::Service(m) if m.contains("Provider disconnected")), "{f:?}");
        assert!(!format!("{f:?}").contains("При"), "no response text in the error: {f:?}");
    }

    #[tokio::test]
    async fn complete_cut_stream_is_network() {
        // The connection closed mid-response: neither a finish reason nor `[DONE]`.
        let first = format!("data: {}\n\n", chunk(r#"{"text":"При"#, None)).into_bytes();
        let api = fake_api(move |_, _, _| Reply::Stream(200, vec![first.clone()], Duration::ZERO)).await;
        let f = call(&api, LONG).await.unwrap_err();
        assert!(matches!(f.error, EngineError::Network(_)), "{f:?}");
        let api = fake_api(|_, _, _| Reply::Cut).await;
        assert!(matches!(call(&api, LONG).await.unwrap_err().error, EngineError::Network(_)));
        let api = fake_api(|_, _, _| Reply::Close).await;
        assert!(matches!(call(&api, LONG).await.unwrap_err().error, EngineError::Network(_)));
    }

    #[tokio::test]
    async fn complete_finished_stream_without_done_is_accepted() {
        let s = stream("\n").replace("data: [DONE]\n\n", "").into_bytes();
        let api = fake_api(move |_, _, _| Reply::Stream(200, vec![s.clone()], Duration::ZERO)).await;
        let c = call(&api, LONG).await.unwrap();
        assert_eq!((c.content.as_str(), c.finish_reason.as_deref()), (r#"{"text":"Привет, как дела?"}"#, Some("stop")));
    }

    #[tokio::test]
    async fn complete_silent_service_is_timeout() {
        let api = fake_api(|_, _, _| Reply::Hang).await;
        let f = call(&api, Duration::from_millis(200)).await.unwrap_err();
        assert_eq!(f, Failure { error: EngineError::Timeout, status: None });
        // The deadline covers the whole request, including reading the stream: a stream that goes silent halfway also ends.
        let pieces = vec![format!("data: {}\n\n", chunk(r#"{"text":"При"#, None)).into_bytes(), b"data: [DONE]\n\n".to_vec()];
        let api = fake_api(move |_, _, _| Reply::Stream(200, pieces.clone(), Duration::from_secs(5))).await;
        let f = call(&api, Duration::from_millis(300)).await.unwrap_err();
        assert_eq!(f.error, EngineError::Timeout);
    }

    #[tokio::test]
    async fn models_list_parses_and_skips_broken_rows() {
        let list = json!({ "data": [
            { "id": "openai/gpt-6-luna", "supported_parameters": ["reasoning", "structured_outputs"],
              "reasoning": { "mandatory": false, "supported_efforts": ["low", "none"] },
              "pricing": { "prompt": "0.0000001", "completion": "0.0000005" }, "context_length": 1_050_000 },
            { "id": 42 },
            { "id": "z-ai/glm-5.3-flash", "supported_parameters": ["reasoning"], "reasoning": { "mandatory": true, "supported_efforts": ["low"] } }
        ] })
        .to_string();
        let api = fake_api(move |_, _, _| reply(200, &list)).await;
        let got = models_at(&format!("{}/api/v1/models", api.base), &http()).await.unwrap();
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["openai/gpt-6-luna", "z-ai/glm-5.3-flash"]);
        assert_eq!(got[0].profile().reasoning, Reasoning::EffortNone);
        assert!(api.seen()[0].header("authorization").is_empty(), "the list is public, no key needed");
        assert_eq!(MODELS_URL, "https://openrouter.ai/api/v1/models");
        assert_eq!(API_URL, "https://openrouter.ai/api/v1/chat/completions");

        let api = fake_api(|_, _, _| reply(503, "down")).await;
        assert!(matches!(models_at(&api.base, &http()).await, Err(EngineError::Service(_))));
    }
}
