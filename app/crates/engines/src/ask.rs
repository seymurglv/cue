//! "Ask" through Claude (spec 2 §6.2): the streaming request with the write_to_field tool and web search, the turns of the
//! conversation, the parser of the event stream and what the model decided; the network part is `run`. No text of the
//! question, the answer, the selection or the sources reaches the log here. CUE holds no key for it any more: the
//! runtime ends a question at once (`ErrorCause::NeedAiAsk`) until Ask goes through the user's own AI.
// Task 9: Ask's own route replaces the Anthropic request and its stream.
use std::collections::HashSet;
use std::time::Duration;

use cue_core::ask as core_ask;
use serde_json::{json, Value};
use tokio::time::{timeout_at, Instant};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";

/// `max_tokens` headroom for adaptive thinking: otherwise thinking eats the response budget and the answer is cut.
pub(crate) const THINKING_HEADROOM: u32 = 2_048;

/// A Messages API POST with the key and the API version.
fn messages_post(http: &reqwest::Client, url: &str, key: &str) -> reqwest::RequestBuilder {
    http.post(url)
        .header("x-api-key", key)
        .header("anthropic-version", API_VERSION)
        .header("content-type", "application/json")
}

/// SSE stream parsing from byte chunks. Chunks may be cut anywhere — even mid UTF-8 character: a line is parsed whole.
/// An event's `data:` fields are joined with `\n`, and an event ends with an empty line; comments (`:` at the start of
/// a line) and other fields are skipped.
#[derive(Debug, Default)]
pub(crate) struct Sse {
    line: Vec<u8>,
    data: Option<String>,
}

impl Sse {
    /// Data of the events this chunk completed.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Vec<String> {
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
    pub(crate) fn finish(&mut self) -> Vec<String> {
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

/// The web search tool (spec 2 §6.2): the basic `web_search_20250305`, called by the model directly — no filtering of
/// the results in code, so no code execution step that can fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchTool {
    /// Searches one request may run (`max_uses` applies per request, continuations included one by one).
    pub max_uses: u32,
}

impl SearchTool {
    pub fn json(self) -> Value {
        json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": self.max_uses, "allowed_callers": ["direct"] })
    }
}

/// The web search the app sends for `max_uses` searches a question; `None` when the model takes none. The basic search
/// since 2026-10-31: the filtered one (`web_search_20260318`, the first probe's choice) failed live on Haiku 5.5 with
/// thinking off with an invalid-input error, and the answer told the user so. On 2026-10-29 the basic one brought the
/// first words of a searched question in 4.2 / 3.5 / 3.8 s (`examples/ask_probe.rs`), with a cached prefix of 3 981
/// tokens against the filtered one's 7 263.
pub fn search_tool(max_uses: u32) -> Option<SearchTool> {
    Some(SearchTool { max_uses })
}

/// write_to_field (spec 2 §6.2): strict, so its input always matches the schema; buffered (no eager streaming), so the
/// input arrives checked and whole — the app acts on it only then.
pub fn write_tool() -> Value {
    json!({
        "name": core_ask::WRITE_TOOL,
        "description": core_ask::WRITE_TOOL_DESCRIPTION,
        "strict": true,
        "input_schema": {
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": core_ask::WRITE_TEXT_DESCRIPTION },
                "mode": { "type": "string", "enum": ["insert", "replace"], "description": core_ask::WRITE_MODE_DESCRIPTION }
            },
            "required": ["text", "mode"],
            "additionalProperties": false
        }
    })
}

/// The tools in a fixed order (they render first in the cached prefix): write_to_field, then web search if on.
pub fn tools(search: Option<SearchTool>) -> Value {
    let mut list = vec![write_tool()];
    list.extend(search.map(SearchTool::json));
    Value::Array(list)
}

/// Thinking for "Ask" (spec 2 §6.2, decision 8): adaptive — the model's default, left out of the request — at low effort,
/// or off. The bench compares the two; the app sends `DEFAULT`: off, chosen after the bench of 2026-10-29 — adaptive
/// thinking timed out on 3 of 210 calls (its thinking reads as silence to the first-words deadline) and brought the first
/// words later, while off met the same bar with none (see docs/ARCHITECTURE.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thinking {
    Adaptive,
    Off,
}

impl Thinking {
    pub const DEFAULT: Thinking = Thinking::Off;
}

/// The streaming request (spec 2 §6.2): the instruction with a cache mark (it covers the tools before it), low effort,
/// thinking as `thinking` says (adaptive gets room for it on top of the answer's budget), `tool_choice: auto`, and the
/// conversation with a second mark on the last user message, so a follow-up reads what came before from the cache.
pub fn request_body(model: &str, search: Option<SearchTool>, thinking: Thinking, messages: &[Value]) -> Value {
    let budget = core_ask::MAX_TOKENS + if thinking == Thinking::Adaptive { THINKING_HEADROOM } else { 0 };
    let mut v = json!({
        "model": model,
        "max_tokens": budget,
        "stream": true,
        "system": [{ "type": "text", "text": core_ask::ASK_PROMPT, "cache_control": { "type": "ephemeral" } }],
        "tools": tools(search),
        "tool_choice": { "type": "auto" },
        "messages": marked(messages),
        "output_config": { "effort": "low" }
    });
    if thinking == Thinking::Off {
        v["thinking"] = json!({ "type": "disabled" });
    }
    v
}

/// The messages with one cache mark, on the last block of the last user message (a paused turn ends with the assistant's
/// blocks, which take no mark). The stored conversation keeps no marks: a request may carry four at most.
fn marked(messages: &[Value]) -> Value {
    let mut out = messages.to_vec();
    if let Some(last) = out.iter_mut().rev().find(|m| m["role"] == "user")
        && let Some(block) = last["content"].as_array_mut().and_then(|b| b.last_mut())
    {
        block["cache_control"] = json!({ "type": "ephemeral" });
    }
    Value::Array(out)
}

/// What a follow-up says about a write_to_field call of the previous turn that was shown, not written.
const SHOWN_NOT_WRITTEN: &str = "The text was shown to the user instead of being written into the field.";

/// A user message: the text, after a `tool_result` for every write_to_field call of the previous turn (`previous`, its
/// blocks) — the API refuses a message that leaves a call of the previous turn without its result.
pub fn user_turn(text: &str, previous: Option<&[Value]>) -> Value {
    let mut content: Vec<Value> = previous
        .unwrap_or(&[])
        .iter()
        .filter(|b| b["type"] == "tool_use")
        .filter_map(|b| b["id"].as_str())
        .map(|id| json!({ "type": "tool_result", "tool_use_id": id, "content": SHOWN_NOT_WRITTEN }))
        .collect();
    content.push(json!({ "type": "text", "text": text }));
    json!({ "role": "user", "content": content })
}

/// The assistant message for the conversation: the blocks as received (search results with their encrypted content,
/// citations with their indexes, thinking with its signature: spec 2 §6.2), except a server tool call that has no result
/// in them — the API defers a search when the model also calls a client tool, and a follow-up that leaves it unanswered
/// is refused.
pub fn assistant_turn(content: &[Value]) -> Value {
    let answered: HashSet<&str> = content.iter().filter_map(|b| b["tool_use_id"].as_str()).collect();
    let kept: Vec<Value> = content
        .iter()
        .filter(|b| b["type"] != "server_tool_use" || b["id"].as_str().is_some_and(|id| answered.contains(id)))
        .cloned()
        .collect();
    json!({ "role": "assistant", "content": kept })
}

/// What the stream brought, for the answer on screen (spec 2 §7: the thinking line names only real steps).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The model started a web search.
    Searching,
    /// Search results came in.
    Reading,
    /// A text block started.
    TextStart,
    /// More text of the current text block.
    Text(String),
    /// A citation of the current text block: a search result's URL and title.
    Cite { url: String, title: String },
    /// The model started to call write_to_field: a write, not an answer. Counts as its first words.
    WriteStart,
    /// What the question has cost so far, all its responses together (`run` sends it as the counts change): an answer
    /// closed mid-stream still writes what it spent (spec 2 §10).
    Spent(AskUsage),
}

impl Step {
    /// The first words of a question: text that is not blank, or the start of a write (decision 6).
    pub fn is_first_words(&self) -> bool {
        matches!(self, Step::Text(t) if !t.trim().is_empty()) || *self == Step::WriteStart
    }
}

/// Tokens and searches paid for (one response, or a whole question with its continuations).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AskUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub web_searches: u32,
}

impl AskUsage {
    pub fn add(&mut self, o: AskUsage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
        self.web_searches += o.web_searches;
    }
}

/// The stream failed: an `error` event (the service's error type, e.g. `overloaded_error`), or data the protocol does not
/// allow (not JSON, a delta of a block never started).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamError {
    Api(String),
    Malformed,
}

/// One response's event stream → its content blocks as the API would return them whole: text with its citations, tool
/// calls with their input parsed, search results as they came (encrypted content included), thinking with its signature.
/// Events and deltas it does not know are skipped: the API may add some.
#[derive(Debug, Default)]
pub struct StreamState {
    blocks: Vec<Value>,
    /// The `input_json_delta` pieces of each block, parsed at its stop.
    json: Vec<String>,
    stop_reason: Option<String>,
    usage: AskUsage,
    /// Web searches the stream started (`server_tool_use` blocks): what the response has searched before its end reports
    /// the count (`ended`), which only `message_delta` does.
    searches_seen: u32,
    ended: bool,
    stopped: bool,
}

/// One response, complete.
#[derive(Debug, Default, PartialEq)]
pub struct StreamTurn {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: AskUsage,
}

impl StreamState {
    /// The data of one event; what it shows on screen.
    pub fn feed(&mut self, data: &str) -> Result<Vec<Step>, StreamError> {
        let v: Value = serde_json::from_str(data).map_err(|_| StreamError::Malformed)?;
        let mut steps = Vec::new();
        match v["type"].as_str().unwrap_or("") {
            "message_start" => self.read_usage(&v["message"]["usage"]),
            "content_block_start" => {
                let i = block_index(&v)?;
                let block = v["content_block"].clone();
                let name = block["name"].as_str().unwrap_or("");
                match block["type"].as_str().unwrap_or("") {
                    "server_tool_use" if name == "web_search" => {
                        self.searches_seen = self.searches_seen.saturating_add(1);
                        steps.push(Step::Searching);
                    }
                    "web_search_tool_result" => steps.push(Step::Reading),
                    "tool_use" if name == core_ask::WRITE_TOOL => steps.push(Step::WriteStart),
                    "text" => {
                        steps.push(Step::TextStart);
                        if let Some(t) = block["text"].as_str().filter(|t| !t.is_empty()) {
                            steps.push(Step::Text(t.to_owned()));
                        }
                    }
                    _ => {}
                }
                self.put(i, block);
            }
            "content_block_delta" => {
                let i = block_index(&v)?;
                let d = &v["delta"];
                let block = self.blocks.get_mut(i).filter(|b| !b.is_null()).ok_or(StreamError::Malformed)?;
                match d["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        let t = d["text"].as_str().unwrap_or("");
                        append(block, "text", t);
                        if !t.is_empty() {
                            steps.push(Step::Text(t.to_owned()));
                        }
                    }
                    "citations_delta" => {
                        let c = d["citation"].clone();
                        // The title is optional in the API: without one the source goes under its site.
                        if c["type"] == "web_search_result_location"
                            && let Some(url) = c["url"].as_str()
                        {
                            let title = c["title"].as_str().map_or_else(|| core_ask::site_of(url), str::to_owned);
                            steps.push(Step::Cite { url: url.to_owned(), title });
                        }
                        match block["citations"].as_array_mut() {
                            Some(list) => list.push(c),
                            None => block["citations"] = json!([c]),
                        }
                    }
                    "input_json_delta" => self.json[i].push_str(d["partial_json"].as_str().unwrap_or("")),
                    "thinking_delta" => append(block, "thinking", d["thinking"].as_str().unwrap_or("")),
                    "signature_delta" => block["signature"] = d["signature"].clone(),
                    _ => {}
                }
            }
            "content_block_stop" => {
                let i = block_index(&v)?;
                let raw = std::mem::take(self.json.get_mut(i).ok_or(StreamError::Malformed)?);
                if let Some(block) = self.blocks.get_mut(i)
                    && matches!(block["type"].as_str(), Some("tool_use" | "server_tool_use"))
                    && !raw.is_empty()
                {
                    // An input that does not parse (cut at max_tokens, or broken) is kept empty: `decide` never carries
                    // it out, and the conversation can still go on.
                    block["input"] = serde_json::from_str::<Value>(&raw).ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
                }
            }
            "message_delta" => {
                if let Some(r) = v["delta"]["stop_reason"].as_str() {
                    self.stop_reason = Some(r.to_owned());
                }
                self.read_usage(&v["usage"]);
                self.ended = true;
            }
            "message_stop" => self.stopped = true,
            "error" => return Err(StreamError::Api(v["error"]["type"].as_str().unwrap_or("error").to_owned())),
            _ => {}
        }
        Ok(steps)
    }

    /// `message_stop` came: the response is complete.
    pub fn done(&self) -> bool {
        self.stopped
    }

    pub fn stop_reason(&self) -> Option<&str> {
        self.stop_reason.as_deref()
    }

    /// The counts of this response so far. The searches are the ones the stream started until the response's end reports
    /// them (spec 2 §10: a question closed or cut after it searched still pays for the search); then the reported count,
    /// which leaves out a search the API deferred and never ran. The tokens are the ones reported so far.
    pub fn usage(&self) -> AskUsage {
        let mut usage = self.usage;
        if !self.ended {
            usage.web_searches = usage.web_searches.max(self.searches_seen);
        }
        usage
    }

    pub fn finish(self) -> StreamTurn {
        let usage = self.usage();
        StreamTurn { content: self.blocks.into_iter().filter(|b| !b.is_null()).collect(), stop_reason: self.stop_reason, usage }
    }

    fn put(&mut self, i: usize, block: Value) {
        if self.blocks.len() <= i {
            self.blocks.resize(i + 1, Value::Null);
            self.json.resize(i + 1, String::new());
        }
        self.blocks[i] = block;
        self.json[i].clear();
    }

    /// The counts are cumulative within a response: the latest value of each field wins.
    fn read_usage(&mut self, u: &Value) {
        let n = |k: &str| u[k].as_u64();
        if let Some(x) = n("input_tokens") {
            self.usage.input_tokens = x;
        }
        if let Some(x) = n("output_tokens") {
            self.usage.output_tokens = x;
        }
        if let Some(x) = n("cache_read_input_tokens") {
            self.usage.cache_read_tokens = x;
        }
        if let Some(x) = n("cache_creation_input_tokens") {
            self.usage.cache_write_tokens = x;
        }
        if let Some(x) = u["server_tool_use"]["web_search_requests"].as_u64() {
            self.usage.web_searches = u32::try_from(x).unwrap_or(u32::MAX);
        }
    }
}

fn block_index(v: &Value) -> Result<usize, StreamError> {
    v["index"].as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < 4_096).ok_or(StreamError::Malformed)
}

fn append(block: &mut Value, key: &str, more: &str) {
    let mut s = block[key].as_str().unwrap_or("").to_owned();
    s.push_str(more);
    block[key] = Value::String(s);
}

/// What the model did with the question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Text to read.
    Answer,
    /// Text for the field, complete and valid; `replace` — over the selection.
    Write { text: String, replace: bool },
    /// The model declined (`stop_reason: refusal`, spec 2 §6.2: "Не могу ответить на это").
    Refused,
    /// Nothing to show: no text and no valid write.
    Empty,
}

/// Spec 2 §6.2: a refusal wins over anything the stream already had (a call it cut must not run); a write_to_field call
/// counts only complete and valid (not cut at `max_tokens`, a mode from the schema, some text); otherwise any text is an
/// answer — a broken or empty call too is shown as an ordinary answer.
pub fn decide(content: &[Value], stop_reason: Option<&str>) -> Decision {
    if stop_reason == Some("refusal") {
        return Decision::Refused;
    }
    if stop_reason != Some("max_tokens")
        && let Some(call) = content.iter().find(|b| b["type"] == "tool_use" && b["name"] == core_ask::WRITE_TOOL)
        && let (Some(text), Some(mode)) = (call["input"]["text"].as_str(), call["input"]["mode"].as_str())
        && !text.trim().is_empty()
        && matches!(mode, "insert" | "replace")
    {
        return Decision::Write { text: text.to_owned(), replace: mode == "replace" };
    }
    if content.iter().any(|b| b["type"] == "text" && b["text"].as_str().is_some_and(|t| !t.trim().is_empty())) {
        Decision::Answer
    } else {
        Decision::Empty
    }
}

/// Why an "Ask" request failed. Never any text of the request or the response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskFailure {
    NoKey,
    /// 401 / 402 / 403, or a 400 about the credit balance.
    Key(u16),
    /// HTTP 404: the model in `ask.model` does not exist (a typo in settings.json).
    NoModel,
    Timeout,
    Network,
    /// Another refusal before the stream: 429, 5xx, a 400 of another kind.
    Http(u16),
    /// An `error` event in the stream (its type, e.g. `overloaded_error`).
    Stream(String),
    /// A stream against the protocol, or one that ended with nothing to show.
    Malformed,
}

impl AskFailure {
    /// A key, balance or limit problem: the user has to act, and Retry would fail the same way (spec 2 §8). A 429 is not
    /// one here: §8 counts it with the network.
    pub fn is_key(&self) -> bool {
        matches!(self, AskFailure::NoKey | AskFailure::Key(_))
    }

    /// The reason for the log: kinds and codes only.
    fn reason(&self) -> String {
        match self {
            AskFailure::NoKey => "no Anthropic key".into(),
            AskFailure::Key(code) => format!("key, balance or limit (HTTP {code})"),
            AskFailure::NoModel => "the model in ask.model was not found (HTTP 404): check settings.json".into(),
            AskFailure::Timeout => "timeout".into(),
            AskFailure::Network => "network".into(),
            AskFailure::Http(code) => format!("HTTP {code}"),
            AskFailure::Stream(kind) => format!("stream error {kind}"),
            AskFailure::Malformed => "no usable response".into(),
        }
    }
}

/// How a question ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskEnd {
    Answer,
    Write { text: String, replace: bool },
    Refused,
    Failed(AskFailure),
}

/// The end of a question, the assistant's blocks for the conversation (all continuations, one turn), and the tokens and
/// searches paid for.
#[derive(Clone, Debug, PartialEq)]
pub struct AskResult {
    pub end: AskEnd,
    pub content: Vec<Value>,
    pub usage: AskUsage,
}

/// The deadlines of one question (spec 2 §6.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub first_words: Duration,
    pub first_words_search: Duration,
    pub total: Duration,
}

impl Limits {
    pub const SPEC: Limits = Limits {
        first_words: Duration::from_millis(core_ask::FIRST_WORDS_MS),
        first_words_search: Duration::from_millis(core_ask::FIRST_WORDS_SEARCH_MS),
        total: Duration::from_millis(core_ask::ANSWER_MS),
    };
}

/// One question, streamed (spec 2 §6.2): the steps go to `on_step` as they come, and with them what the question has cost
/// so far (`Step::Spent`); a paused turn is continued with its blocks and the same tools, at most `MAX_CONTINUATIONS` times
/// and only while its searches are not spent (decision 9); a key without web search is asked again without it (§8); a
/// write cut at `max_tokens` is asked again once with more room, under deadlines of its own that start when the retry is
/// sent (the cut call has usually used most of the whole answer's time); the deadlines are `Limits::SPEC`. Never "fails"
/// by panicking or hanging: every problem ends as `AskEnd::Failed`, with one log line of its reason, and with what the
/// question spent up to it, the response that failed part-way included.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    search: Option<SearchTool>,
    thinking: Thinking,
    messages: &[Value],
    on_step: impl FnMut(Step) + Send,
) -> AskResult {
    run_at(API_URL, http, key, model, search, thinking, messages, Limits::SPEC, on_step).await
}

fn failed(why: AskFailure, content: Vec<Value>, usage: AskUsage) -> AskResult {
    tracing::warn!("ask: {}", why.reason());
    AskResult { end: AskEnd::Failed(why), content, usage }
}

/// A refusal before the stream → its kind (spec 2 §8). The body is read only for the balance reason; it never reaches
/// the log.
fn classify(status: u16, body: &str) -> AskFailure {
    match status {
        401..=403 => AskFailure::Key(status),
        400 if body.to_lowercase().contains("credit balance") => AskFailure::Key(status),
        404 => AskFailure::NoModel,
        _ => AskFailure::Http(status),
    }
}

/// A 400 that says web search is not available to this key or organization.
fn search_refused(body: &str) -> bool {
    let b = body.to_lowercase();
    b.contains("web search") || b.contains("web_search")
}

/// Room added once for a write cut at `max_tokens` (a long email): the same question again; the cut call never runs.
const LONG_WRITE_TOKENS: u32 = 2 * core_ask::MAX_TOKENS;

/// [`run`] against the URL `url` with the deadlines `limits`; tests substitute their own.
#[allow(clippy::too_many_arguments)]
async fn run_at(
    url: &str,
    http: &reqwest::Client,
    key: Option<&str>,
    model: &str,
    mut search: Option<SearchTool>,
    thinking: Thinking,
    messages: &[Value],
    limits: Limits,
    mut on_step: impl FnMut(Step) + Send,
) -> AskResult {
    let mut content: Vec<Value> = Vec::new();
    let mut usage = AskUsage::default();
    let Some(key) = key else { return failed(AskFailure::NoKey, content, usage) };
    let mut t0 = Instant::now();
    let mut total = t0 + limits.total;
    let mut first_by = t0 + limits.first_words;
    let mut first_seen = false;
    let mut round = 0;
    let mut widened = false;
    let mut last_spent = AskUsage::default();
    loop {
        // Until the first words: the first-words deadline (longer once a search began); then the whole answer's.
        let by = |seen: bool, first_by: Instant| if seen { total } else { first_by.min(total) };
        let mut msgs = messages.to_vec();
        if !content.is_empty() {
            msgs.push(json!({ "role": "assistant", "content": content }));
        }
        let mut body = request_body(model, search, thinking, &msgs);
        if widened {
            body["max_tokens"] = json!(body["max_tokens"].as_u64().unwrap_or(0) + u64::from(LONG_WRITE_TOKENS));
        }
        let mut resp = match timeout_at(by(first_seen, first_by), messages_post(http, url, key).json(&body).send()).await {
            Err(_) => return failed(AskFailure::Timeout, content, usage),
            Ok(Err(e)) if e.is_timeout() => return failed(AskFailure::Timeout, content, usage),
            Ok(Err(_)) => return failed(AskFailure::Network, content, usage),
            Ok(Ok(r)) => r,
        };
        let status = resp.status().as_u16();
        if status != 200 {
            // The refusal's body is read under the same deadline: a service that stalls after its status must not hang
            // the question.
            let text = timeout_at(by(first_seen, first_by), resp.text()).await.ok().and_then(Result::ok).unwrap_or_default();
            if status == 400 && search.is_some() && content.is_empty() && search_refused(&text) {
                tracing::warn!("ask: web search is not available to this key: asked without it");
                search = None;
                continue;
            }
            return failed(classify(status, &text), content, usage);
        }
        let (mut sse, mut st) = (Sse::default(), StreamState::default());
        let broke = loop {
            let chunk = match timeout_at(by(first_seen, first_by), resp.chunk()).await {
                Err(_) => break Some(AskFailure::Timeout),
                Ok(Err(_)) => break Some(AskFailure::Network),
                Ok(Ok(c)) => c,
            };
            let (events, end) = match chunk {
                Some(bytes) => (sse.push(&bytes), false),
                None => (sse.finish(), true),
            };
            let mut broke = None;
            for data in events {
                let steps = match st.feed(&data) {
                    Ok(steps) => steps,
                    Err(StreamError::Api(kind)) => {
                        broke = Some(AskFailure::Stream(kind));
                        break;
                    }
                    Err(StreamError::Malformed) => {
                        broke = Some(AskFailure::Malformed);
                        break;
                    }
                };
                for step in steps {
                    match &step {
                        Step::Searching if !first_seen => first_by = t0 + limits.first_words_search,
                        s if s.is_first_words() => first_seen = true,
                        _ => {}
                    }
                    on_step(step);
                }
            }
            // Reported before a failure too: the events before an `error` event in the same chunk were paid for.
            let mut spent = usage;
            spent.add(st.usage());
            if spent != last_spent {
                last_spent = spent;
                on_step(Step::Spent(spent));
            }
            if broke.is_some() {
                break broke;
            }
            if st.done() || end {
                // The body ended before the response did (no stop reason): the connection dropped.
                break (!st.done() && st.stop_reason().is_none()).then_some(AskFailure::Network);
            }
        };
        if let Some(why) = broke {
            // A response that failed part-way is paid for as far as it came (spec 2 §10).
            usage.add(st.usage());
            return failed(why, content, usage);
        }
        let turn = st.finish();
        usage.add(turn.usage);
        content.extend(turn.content);
        let reason = turn.stop_reason.as_deref();
        let searches_left = search.is_none_or(|s| usage.web_searches < s.max_uses);
        if reason == Some("pause_turn") && round < core_ask::MAX_CONTINUATIONS && searches_left {
            round += 1;
            // The next request follows at once: with the rest of this body read, it goes over the same connection.
            let _ = timeout_at(by(first_seen, first_by), rest(&mut resp)).await;
            continue;
        }
        let cut_write = content.iter().any(|b| b["type"] == "tool_use" && b["name"] == core_ask::WRITE_TOOL);
        if reason == Some("max_tokens") && cut_write && !widened {
            widened = true;
            content.clear();
            let _ = timeout_at(by(first_seen, first_by), rest(&mut resp)).await;
            // The retry has deadlines of its own, from when it is sent: the cut call has usually used most of the whole
            // answer's time, and a retry left with the rest of it would end in a timeout with both calls paid for.
            t0 = Instant::now();
            total = t0 + limits.total;
            first_by = t0 + limits.first_words;
            first_seen = false;
            continue;
        }
        let end = match decide(&content, reason) {
            Decision::Answer => AskEnd::Answer,
            Decision::Write { text, replace } => AskEnd::Write { text, replace },
            Decision::Refused => AskEnd::Refused,
            Decision::Empty => return failed(AskFailure::Malformed, content, usage),
        };
        // A response dropped unread closes the connection, and the next request pays for a new TLS handshake: the rest of
        // the body (the final empty chunk) is read in the background, for `DRAIN` at most.
        tokio::spawn(async move {
            let _ = tokio::time::timeout(DRAIN, rest(&mut resp)).await;
        });
        return AskResult { end, content, usage };
    }
}

/// How long the rest of an answered response's body is read in the background: a service that never ends it must not
/// keep a task and a connection.
const DRAIN: Duration = Duration::from_secs(5);

/// Reads the rest of a response's body (the final empty chunk, after `message_stop`), so its connection can be reused.
async fn rest(resp: &mut reqwest::Response) {
    while let Ok(Some(_)) = resp.chunk().await {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::ask::ASK_PROMPT;

    fn question() -> Value {
        user_turn("<question>что такое HTTP 418</question>", None)
    }

    #[test]
    fn the_request_streams_with_both_tools_and_two_cache_marks() {
        let v = request_body("claude-haiku-5-5", Some(SearchTool { max_uses: 2 }), Thinking::DEFAULT, &[question()]);
        assert_eq!(v["model"], "claude-haiku-5-5");
        assert_eq!(v["stream"], true);
        assert_eq!(v["max_tokens"], 2_000, "the answer's budget: no room kept for thinking");
        assert_eq!(v["thinking"], json!({ "type": "disabled" }), "thinking is off (decision 8, after the bench)");
        assert_eq!(v["output_config"], json!({ "effort": "low" }), "no output format: the answer streams as text");
        assert_eq!(v["tool_choice"], json!({ "type": "auto" }));
        assert_eq!(v["system"], json!([{ "type": "text", "text": ASK_PROMPT, "cache_control": { "type": "ephemeral" } }]));
        let write = &v["tools"][0];
        assert_eq!((write["name"].as_str(), write["strict"].as_bool()), (Some("write_to_field"), Some(true)));
        assert_eq!(write["input_schema"]["required"], json!(["text", "mode"]));
        assert_eq!(write["input_schema"]["additionalProperties"], false);
        assert_eq!(write["input_schema"]["properties"]["mode"]["enum"], json!(["insert", "replace"]));
        assert!(write.get("eager_input_streaming").is_none(), "the call is acted on whole: its input stays buffered and checked");
        assert_eq!(
            v["tools"][1],
            json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 2, "allowed_callers": ["direct"] })
        );
        assert_eq!(v["messages"][0]["content"][0]["cache_control"], json!({ "type": "ephemeral" }));
        for banned in ["temperature", "top_p", "top_k", "container"] {
            assert!(v.get(banned).is_none(), "{banned}");
        }
    }

    #[test]
    fn without_search_only_the_write_tool() {
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &[question()]);
        assert_eq!(v["tools"].as_array().map(Vec::len), Some(1));
    }

    /// The app's search is the basic one, called by the model directly: no filtering in code, no code execution.
    #[test]
    fn the_app_searches_with_the_basic_search() {
        assert_eq!(search_tool(3), Some(SearchTool { max_uses: 3 }));
        let v = request_body("claude-haiku-5-5", search_tool(3), Thinking::DEFAULT, &[question()]);
        assert_eq!(
            v["tools"][1],
            json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 3, "allowed_callers": ["direct"] })
        );
        assert_eq!(v["tools"].as_array().map(Vec::len), Some(2), "no code execution tool");
    }

    /// The cache mark goes on the last user message only (four marks at most), also while a paused turn is continued;
    /// the stored conversation keeps none.
    #[test]
    fn only_the_last_user_message_carries_the_mark() {
        let answer = json!({ "role": "assistant", "content": [{ "type": "text", "text": "Ответ." }] });
        let follow = user_turn("<question>а подробнее</question>", None);
        let msgs = vec![question(), answer.clone(), follow];
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &msgs);
        assert!(v["messages"][0]["content"][0].get("cache_control").is_none());
        assert!(v["messages"][1]["content"][0].get("cache_control").is_none());
        assert_eq!(v["messages"][2]["content"][0]["cache_control"], json!({ "type": "ephemeral" }));
        assert!(msgs[2]["content"][0].get("cache_control").is_none(), "the conversation itself is untouched");
        let paused = vec![question(), answer];
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &paused);
        assert_eq!(v["messages"][0]["content"][0]["cache_control"], json!({ "type": "ephemeral" }));
        assert!(v["messages"][1]["content"][0].get("cache_control").is_none());
    }

    /// The bench's other arm (decision 8): adaptive thinking, the model's default, left out of the request, with room
    /// for it on top of the answer's budget.
    #[test]
    fn adaptive_thinking_is_left_out_with_headroom() {
        let v = request_body("claude-haiku-5-5", None, Thinking::Adaptive, &[question()]);
        assert_eq!(v["max_tokens"], 4_048);
        assert!(v.get("thinking").is_none());
    }

    /// A whole stream: a comment, the events with their names, `[DONE]`.
    fn sse_stream(nl: &str) -> String {
        let events = [
            ": keep-alive".to_owned(),
            format!(r#"event: message_start{nl}data: {{"type":"message_start"}}"#),
            r#"data: {"type":"content_block_delta","delta":{"text":"Привет,"}}"#.to_owned(),
            ": keep-alive".to_owned(),
            r#"data: {"type":"content_block_delta","delta":{"text":" как дела?"}}"#.to_owned(),
            "data: [DONE]".to_owned(),
        ];
        events.iter().map(|e| format!("{e}{nl}{nl}")).collect()
    }

    fn sse_events() -> Vec<String> {
        vec![
            r#"{"type":"message_start"}"#.to_owned(),
            r#"{"type":"content_block_delta","delta":{"text":"Привет,"}}"#.to_owned(),
            r#"{"type":"content_block_delta","delta":{"text":" как дела?"}}"#.to_owned(),
            "[DONE]".to_owned(),
        ]
    }

    #[test]
    fn sse_reads_data_and_skips_comments() {
        let mut sse = Sse::default();
        let mut got = sse.push(sse_stream("\n").as_bytes());
        got.extend(sse.finish());
        assert_eq!(got, sse_events());
    }

    #[test]
    fn sse_split_anywhere_gives_same_events() {
        // Every cut point — including the middle of a Cyrillic letter (two bytes each) and between `\r` and `\n`.
        for nl in ["\n", "\r\n"] {
            let bytes = sse_stream(nl).into_bytes();
            for cut in 0..=bytes.len() {
                let mut sse = Sse::default();
                let mut got = sse.push(&bytes[..cut]);
                got.extend(sse.push(&bytes[cut..]));
                got.extend(sse.finish());
                assert_eq!(got, sse_events(), "cut at byte {cut}, line break {nl:?}");
            }
            // Byte by byte.
            let mut sse = Sse::default();
            let mut got: Vec<String> = bytes.iter().flat_map(|b| sse.push(std::slice::from_ref(b))).collect();
            got.extend(sse.finish());
            assert_eq!(got, sse_events(), "byte by byte, line break {nl:?}");
        }
    }

    #[test]
    fn sse_multiline_data_other_fields_and_unterminated_tail() {
        let mut sse = Sse::default();
        let got = sse.push(b"event: message\nid: 7\ndata: a\ndata:b\nretry: 10\n\n:keep-alive\n\ndata: x");
        assert_eq!(got, ["a\nb"], "data fields are joined with a line break, other fields are skipped");
        assert_eq!(sse.finish(), ["x"], "the end of the stream completes both the unfinished line and the event");
        assert!(sse.finish().is_empty());
        assert!(Sse::default().push(b": keep-alive\n\n\n\n").is_empty(), "only comments, no events");
    }

    /// A write shown instead of written (spec 2 §6.2) left a call without its result: the follow-up answers it first.
    #[test]
    fn a_follow_up_answers_the_calls_of_the_previous_turn() {
        let previous = vec![
            json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": { "text": "x", "mode": "replace" } }),
        ];
        assert_eq!(
            user_turn("<question>а короче</question>", Some(&previous)),
            json!({ "role": "user", "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1", "content": SHOWN_NOT_WRITTEN },
                { "type": "text", "text": "<question>а короче</question>" }
            ] })
        );
        assert_eq!(user_turn("<question>q</question>", Some(&[json!({ "type": "text", "text": "a" })]))["content"].as_array().map(Vec::len), Some(1));
    }

    /// The blocks go back as they came (search results with their encrypted content), except a search the API never ran:
    /// a follow-up that leaves it unanswered is refused.
    #[test]
    fn the_history_keeps_the_blocks_but_drops_a_search_that_never_ran() {
        let content = vec![
            json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": { "query": "q" } }),
            json!({ "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [{ "type": "web_search_result", "url": "https://a.io", "title": "A", "encrypted_content": "EqgfCio" }] }),
            json!({ "type": "text", "text": "Ответ", "citations": [{ "type": "web_search_result_location", "url": "https://a.io", "title": "A", "encrypted_index": "Eo8B", "cited_text": "a" }] }),
            json!({ "type": "server_tool_use", "id": "srvtoolu_2", "name": "web_search", "input": { "query": "q2" } }),
            json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": { "text": "x", "mode": "insert" } }),
        ];
        let turn = assistant_turn(&content);
        assert_eq!(turn["role"], "assistant");
        let kept = turn["content"].as_array().unwrap();
        assert_eq!(kept.len(), 4);
        assert_eq!(kept[..3], content[..3], "unchanged, encrypted content and citations included");
        assert_eq!(kept[3], content[4]);
    }

    fn start(usage: Value) -> Value {
        json!({ "type": "message_start", "message": { "id": "msg_1", "type": "message", "role": "assistant", "content": [], "usage": usage } })
    }
    fn block_start(i: u64, block: Value) -> Value {
        json!({ "type": "content_block_start", "index": i, "content_block": block })
    }
    fn delta(i: u64, d: Value) -> Value {
        json!({ "type": "content_block_delta", "index": i, "delta": d })
    }
    fn stop(i: u64) -> Value {
        json!({ "type": "content_block_stop", "index": i })
    }
    fn end(reason: &str, usage: Value) -> Vec<Value> {
        vec![
            json!({ "type": "message_delta", "delta": { "stop_reason": reason, "stop_sequence": null }, "usage": usage }),
            json!({ "type": "message_stop" }),
        ]
    }
    fn text_answer(text: &str) -> Vec<Value> {
        let mut v = vec![
            start(json!({ "input_tokens": 40, "cache_read_input_tokens": 900, "cache_creation_input_tokens": 0, "output_tokens": 1 })),
            block_start(0, json!({ "type": "text", "text": "" })),
            delta(0, json!({ "type": "text_delta", "text": text })),
            stop(0),
        ];
        v.extend(end("end_turn", json!({ "output_tokens": 12 })));
        v
    }
    fn search_answer() -> Vec<Value> {
        let mut v = vec![
            start(json!({ "input_tokens": 50, "output_tokens": 1 })),
            block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            delta(0, json!({ "type": "input_json_delta", "partial_json": "{\"query\":" })),
            delta(0, json!({ "type": "input_json_delta", "partial_json": " \"rust latest version\"}" })),
            stop(0),
            block_start(1, json!({ "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
                { "type": "web_search_result", "url": "https://blog.rust-lang.org/", "title": "Rust Blog", "encrypted_content": "EqgfCio", "page_age": null }
            ] })),
            stop(1),
            block_start(2, json!({ "type": "text", "text": "" })),
            delta(2, json!({ "type": "text_delta", "text": "Последняя версия — 1.95" })),
            delta(2, json!({ "type": "citations_delta", "citation": { "type": "web_search_result_location", "url": "https://blog.rust-lang.org/", "title": "Rust Blog", "encrypted_index": "Eo8B", "cited_text": "Rust 1.95 is out" } })),
            stop(2),
            block_start(3, json!({ "type": "text", "text": "." })),
            stop(3),
        ];
        v.extend(end("end_turn", json!({ "input_tokens": 3_000, "output_tokens": 30, "server_tool_use": { "web_search_requests": 1 } })));
        v
    }
    fn write_answer(text: &str, mode: &str) -> Vec<Value> {
        let input = json!({ "text": text, "mode": mode }).to_string();
        // Two pieces, cut between characters (the text may be Cyrillic).
        let mid = input.char_indices().nth(input.chars().count() / 2).map_or(0, |(i, _)| i);
        let (a, b) = input.split_at(mid);
        let mut v = vec![
            start(json!({ "input_tokens": 60, "output_tokens": 1 })),
            block_start(0, json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": {} })),
            delta(0, json!({ "type": "input_json_delta", "partial_json": a })),
            delta(0, json!({ "type": "input_json_delta", "partial_json": b })),
            stop(0),
        ];
        v.extend(end("tool_use", json!({ "output_tokens": 20 })));
        v
    }

    fn feed_all(events: &[Value]) -> (Vec<Step>, StreamTurn) {
        let mut st = StreamState::default();
        let mut steps = Vec::new();
        for e in events {
            steps.extend(st.feed(&e.to_string()).unwrap());
        }
        assert!(st.done(), "message_stop was seen");
        (steps, st.finish())
    }

    /// The events with their block indexes moved by `by` (blocks put before them).
    fn shifted(events: &[Value], by: u64) -> Vec<Value> {
        events
            .iter()
            .map(|e| {
                let mut e = e.clone();
                if let Some(i) = e["index"].as_u64() {
                    e["index"] = json!(i + by);
                }
                e
            })
            .collect()
    }

    #[test]
    fn a_text_answer_streams_its_words() {
        let (steps, turn) = feed_all(&text_answer("I'm a teapot."));
        assert_eq!(steps, [Step::TextStart, Step::Text("I'm a teapot.".into())]);
        assert_eq!(turn.content, [json!({ "type": "text", "text": "I'm a teapot." })]);
        assert_eq!(turn.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(
            turn.usage,
            AskUsage { input_tokens: 40, output_tokens: 12, cache_read_tokens: 900, cache_write_tokens: 0, web_searches: 0 }
        );
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Answer);
    }

    #[test]
    fn a_searched_answer_keeps_its_results_and_citations() {
        let (steps, turn) = feed_all(&search_answer());
        assert_eq!(
            steps,
            [
                Step::Searching,
                Step::Reading,
                Step::TextStart,
                Step::Text("Последняя версия — 1.95".into()),
                Step::Cite { url: "https://blog.rust-lang.org/".into(), title: "Rust Blog".into() },
                Step::TextStart,
                Step::Text(".".into()),
            ]
        );
        assert_eq!(turn.content[0]["input"], json!({ "query": "rust latest version" }), "the call's input parsed whole");
        assert_eq!(turn.content[1]["content"][0]["encrypted_content"], "EqgfCio", "results go back unchanged");
        assert_eq!(turn.content[2]["citations"][0]["encrypted_index"], "Eo8B");
        assert_eq!(turn.content[2]["text"], "Последняя версия — 1.95");
        assert_eq!(turn.usage.web_searches, 1);
        assert_eq!(turn.usage.input_tokens, 3_000, "the counts are cumulative: the latest wins");
    }

    #[test]
    fn a_write_comes_back_whole() {
        let (steps, turn) = feed_all(&write_answer("Привет, Аня! Встреча переносится на пятницу.", "insert"));
        assert_eq!(steps, [Step::WriteStart]);
        assert_eq!(
            decide(&turn.content, turn.stop_reason.as_deref()),
            Decision::Write { text: "Привет, Аня! Встреча переносится на пятницу.".into(), replace: false }
        );
        let (_, turn) = feed_all(&write_answer("Fixed.", "replace"));
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Write { text: "Fixed.".into(), replace: true });
    }

    /// A broken or empty call is shown as an ordinary answer (spec 2 §6.2): its text, if any; nothing to show otherwise.
    #[test]
    fn a_broken_or_empty_call_is_not_carried_out() {
        let mut events = vec![
            start(json!({ "input_tokens": 60 })),
            block_start(0, json!({ "type": "text", "text": "" })),
            delta(0, json!({ "type": "text_delta", "text": "Вот текст:" })),
            stop(0),
            block_start(1, json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": {} })),
            delta(1, json!({ "type": "input_json_delta", "partial_json": "{\"text\": \"обрыв" })),
            stop(1),
        ];
        events.extend(end("max_tokens", json!({ "output_tokens": 2_000 })));
        let (_, turn) = feed_all(&events);
        assert_eq!(turn.content[1]["input"], json!({}), "an input that does not parse is kept empty");
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Answer);
        let (_, turn) = feed_all(&write_answer("   ", "insert"));
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Empty);
        let call = [json!({ "type": "tool_use", "name": "write_to_field", "input": { "text": "x", "mode": "append" } })];
        assert_eq!(decide(&call, Some("tool_use")), Decision::Empty, "a mode outside the schema");
        let cut = [json!({ "type": "tool_use", "name": "write_to_field", "input": { "text": "x", "mode": "insert" } })];
        assert_eq!(decide(&cut, Some("max_tokens")), Decision::Empty, "cut at max_tokens: never carried out");
    }

    /// A refusal wins over whatever came before it, a complete and valid call too: that call must not run (spec 2 §6.2).
    #[test]
    fn a_refusal_wins() {
        let mut events = write_answer("x", "insert");
        events.truncate(5);
        events.extend(end("refusal", json!({ "output_tokens": 3 })));
        let (_, turn) = feed_all(&events);
        let write = Decision::Write { text: "x".into(), replace: false };
        assert_eq!(decide(&turn.content, Some("tool_use")), write, "the call itself is whole and valid");
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Refused);
    }

    /// Decision 6: the deadlines wait for text that is not blank, or for the start of a write.
    #[test]
    fn the_first_words_are_text_or_the_start_of_a_write() {
        assert!(Step::Text("Да".into()).is_first_words() && Step::WriteStart.is_first_words());
        assert!(!Step::Text(" \n".into()).is_first_words() && !Step::TextStart.is_first_words());
        assert!([Step::Searching, Step::Reading, Step::Cite { url: "https://a.io".into(), title: "A".into() }].iter().all(|s| !s.is_first_words()));
    }

    /// A cited result's title is optional in the API: a citation without one still numbers its source, under its site.
    #[test]
    fn a_citation_without_a_title_is_still_a_source() {
        let mut st = StreamState::default();
        st.feed(&block_start(0, json!({ "type": "text", "text": "" })).to_string()).unwrap();
        let cite = json!({ "type": "web_search_result_location", "url": "https://www.rust-lang.org/learn", "title": null, "encrypted_index": "Eo8B", "cited_text": "a" });
        assert_eq!(
            st.feed(&delta(0, json!({ "type": "citations_delta", "citation": cite })).to_string()),
            Ok(vec![Step::Cite { url: "https://www.rust-lang.org/learn".into(), title: "rust-lang.org".into() }])
        );
        assert_eq!(st.finish().content[0]["citations"], json!([cite]), "the citation goes back as it came");
    }

    #[test]
    fn thinking_comes_back_with_its_signature() {
        let mut events = vec![
            start(json!({ "input_tokens": 10 })),
            block_start(0, json!({ "type": "thinking", "thinking": "" })),
            delta(0, json!({ "type": "thinking_delta", "thinking": "" })),
            delta(0, json!({ "type": "signature_delta", "signature": "EqQBCgIYAhIM" })),
            stop(0),
        ];
        events.extend(shifted(&text_answer("Ответ.")[1..], 1));
        let (_, turn) = feed_all(&events);
        assert_eq!(turn.content[0], json!({ "type": "thinking", "thinking": "", "signature": "EqQBCgIYAhIM" }));
        assert_eq!(turn.content[1]["text"], "Ответ.");
    }

    /// A failed search inside the answer is no error (spec 2 §8): the model answers without it.
    #[test]
    fn a_failed_search_is_not_an_error() {
        let mut events = vec![
            start(json!({ "input_tokens": 10 })),
            block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            stop(0),
            block_start(1, json!({ "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": { "type": "web_search_tool_result_error", "error_code": "unavailable" } })),
            stop(1),
        ];
        events.extend(shifted(&text_answer("Без поиска: …")[1..], 2));
        let (steps, turn) = feed_all(&events);
        assert!(steps.contains(&Step::Reading));
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Answer);
    }

    #[test]
    fn an_error_event_fails_the_stream_and_unknown_events_are_skipped() {
        let mut st = StreamState::default();
        assert_eq!(st.feed(r#"{"type":"ping"}"#), Ok(vec![]));
        assert_eq!(st.feed(r#"{"type":"some_future_event","x":1}"#), Ok(vec![]));
        st.feed(&block_start(0, json!({ "type": "text", "text": "" })).to_string()).unwrap();
        assert_eq!(st.feed(&delta(0, json!({ "type": "future_delta", "x": 1 })).to_string()), Ok(vec![]));
        assert_eq!(
            st.feed(r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#),
            Err(StreamError::Api("overloaded_error".into()))
        );
        assert_eq!(st.feed("not json"), Err(StreamError::Malformed));
        assert_eq!(st.feed(&delta(7, json!({ "type": "text_delta", "text": "x" })).to_string()), Err(StreamError::Malformed), "a delta of a block never started");
    }

    /// The API reports a response's searches only at its end: until then each search the stream started counts, so a
    /// question closed or cut after it searched still writes the search (spec 2 §10). The end's count is the one billed.
    #[test]
    fn searches_count_from_the_stream_until_the_end_reports_them() {
        let events = search_answer();
        let mut st = StreamState::default();
        for e in &events[..2] {
            st.feed(&e.to_string()).unwrap();
        }
        assert_eq!(st.usage().web_searches, 1, "the search began");
        assert_eq!(st.usage().input_tokens, 50, "tokens stay as known");
        for e in &events[2..] {
            st.feed(&e.to_string()).unwrap();
        }
        assert_eq!(st.usage().web_searches, 1, "the end's count, not one more");
        let mut deferred = StreamState::default();
        deferred.feed(&start(json!({ "input_tokens": 5 })).to_string()).unwrap();
        deferred.feed(&block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })).to_string()).unwrap();
        assert_eq!(deferred.usage().web_searches, 1);
        for e in end("tool_use", json!({ "output_tokens": 9 })) {
            deferred.feed(&e.to_string()).unwrap();
        }
        assert_eq!(deferred.usage().web_searches, 0, "a search the end does not report was never run");
    }

    #[test]
    fn usage_adds_up_over_continuations() {
        let mut u = AskUsage { input_tokens: 1, output_tokens: 2, cache_read_tokens: 3, cache_write_tokens: 4, web_searches: 1 };
        u.add(AskUsage { input_tokens: 10, output_tokens: 20, cache_read_tokens: 30, cache_write_tokens: 40, web_searches: 1 });
        assert_eq!(u, AskUsage { input_tokens: 11, output_tokens: 22, cache_read_tokens: 33, cache_write_tokens: 44, web_searches: 2 });
    }

    use crate::testing::{capture_logs, fake_api, reply, FakeApi, Reply};
    use std::time::Duration;

    const KEY: &str = "sk-ant-TEST-KEY";
    const FAST: Limits = Limits {
        first_words: Duration::from_millis(400),
        first_words_search: Duration::from_millis(1_200),
        total: Duration::from_millis(2_500),
    };

    /// Events as the service sends them: an `event:` and a `data:` line, an empty line after each.
    fn sse(events: &[Value]) -> Vec<u8> {
        events
            .iter()
            .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("message")))
            .collect::<String>()
            .into_bytes()
    }

    fn stream(events: &[Value]) -> Reply {
        Reply::Stream(200, vec![sse(events)], Duration::ZERO)
    }

    /// `run_at` with Haiku and basic search against a fake API: the result, the steps (without the cost reports), the ask
    /// log lines, the service. Whatever the end, the last cost report is the whole cost of the result (spec 2 §10).
    async fn ask_with<F>(key: Option<&str>, limits: Limits, route: F) -> (AskResult, Vec<Step>, Vec<String>, FakeApi)
    where
        F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
    {
        let api = fake_api(route).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let (logs, _guard) = capture_logs();
        let url = format!("{}/v1/messages", api.base);
        let (mut steps, mut spent) = (Vec::new(), AskUsage::default());
        let search = Some(SearchTool { max_uses: 2 });
        let r = run_at(&url, &http, key, "claude-haiku-5-5", search, Thinking::DEFAULT, &[question()], limits, |s| match s {
            Step::Spent(u) => spent = u,
            s => steps.push(s),
        })
        .await;
        assert_eq!(spent, r.usage, "the last cost report is the result's cost: {:?}", r.end);
        let lines = logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::ask")).cloned().collect();
        (r, steps, lines, api)
    }

    /// No text of the question, the answer or the key in a log line.
    fn assert_clean(lines: &[String]) {
        for line in lines {
            for leak in ["HTTP 418", "teapot", "1.95", "rust", "Аня", "TEST-KEY"] {
                assert!(!line.contains(leak), "\"{leak}\" in the log: {line}");
            }
        }
    }

    #[tokio::test]
    async fn an_answer_streams_its_steps_and_returns_its_blocks() {
        let (r, steps, lines, api) = ask_with(Some(KEY), FAST, |_, _, _| stream(&text_answer("I'm a teapot."))).await;
        assert_eq!(r.end, AskEnd::Answer);
        assert_eq!(steps, [Step::TextStart, Step::Text("I'm a teapot.".into())]);
        assert_eq!(r.content, [json!({ "type": "text", "text": "I'm a teapot." })]);
        assert_eq!((r.usage.input_tokens, r.usage.cache_read_tokens, r.usage.output_tokens), (40, 900, 12));
        assert!(lines.is_empty(), "an answer leaves the log alone: {lines:?}");
        let req = &api.seen()[0];
        assert_eq!(req.header("x-api-key"), [KEY]);
        assert_eq!(req.header("anthropic-version"), ["2026-10-21"]);
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let search = Some(SearchTool { max_uses: 2 });
        assert_eq!(body, request_body("claude-haiku-5-5", search, Thinking::DEFAULT, &[question()]));
    }

    #[tokio::test]
    async fn a_paused_turn_is_continued_with_its_blocks() {
        let mut paused = vec![
            start(json!({ "input_tokens": 50 })),
            block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            delta(0, json!({ "type": "input_json_delta", "partial_json": "{\"query\":\"q\"}" })),
            stop(0),
        ];
        paused.extend(end("pause_turn", json!({ "output_tokens": 5, "server_tool_use": { "web_search_requests": 1 } })));
        let (r, steps, _, api) = ask_with(Some(KEY), FAST, move |_, _, nth| {
            if nth == 0 { stream(&paused) } else { stream(&text_answer("Готово.")) }
        })
        .await;
        assert_eq!(r.end, AskEnd::Answer);
        assert_eq!(api.calls().len(), 2);
        let second: Value = serde_json::from_slice(&api.body(1)).unwrap();
        assert_eq!(second["messages"][1]["role"], "assistant", "the paused turn goes back as it came");
        assert_eq!(second["messages"][1]["content"][0]["id"], "srvtoolu_1");
        assert_eq!(second["tools"], tools(Some(SearchTool { max_uses: 2 })), "the same tools");
        assert_eq!(r.content.len(), 2, "both responses' blocks: one turn");
        assert_eq!((r.usage.input_tokens, r.usage.output_tokens, r.usage.web_searches), (90, 17, 1));
        assert_eq!(steps[0], Step::Searching);
    }

    #[tokio::test]
    async fn continuations_stop_after_three() {
        let mut paused = text_answer("часть ");
        let n = paused.len();
        paused[n - 2] = json!({ "type": "message_delta", "delta": { "stop_reason": "pause_turn" }, "usage": { "output_tokens": 3 } });
        let (r, _, _, api) = ask_with(Some(KEY), FAST, move |_, _, _| stream(&paused)).await;
        assert_eq!(api.calls().len(), 4, "the question and three continuations");
        assert_eq!(r.end, AskEnd::Answer, "the text so far is the answer");
    }

    /// `max_uses` counts per request: a paused turn is not continued once the question's searches are spent, or every
    /// continuation could search as much again (decision 9).
    #[tokio::test]
    async fn no_continuation_once_the_searches_are_spent() {
        let mut paused = text_answer("По данным поиска ");
        let n = paused.len();
        paused[n - 2] = json!({ "type": "message_delta", "delta": { "stop_reason": "pause_turn" },
            "usage": { "output_tokens": 3, "server_tool_use": { "web_search_requests": 2 } } });
        let (r, _, _, api) = ask_with(Some(KEY), FAST, move |_, _, _| stream(&paused)).await;
        assert_eq!(api.calls().len(), 1, "two searches of two: the answer is what came");
        assert_eq!((r.end, r.usage.web_searches), (AskEnd::Answer, 2));
    }

    /// Spec 2 §8: a failed search is no error. A key or organization without web search is refused with HTTP 400 on every
    /// question: the question goes again without the tool, once.
    #[tokio::test]
    async fn a_key_without_web_search_asks_again_without_it() {
        let refused = r#"{"type":"error","error":{"type":"invalid_request_error","message":"Web search is not enabled for this organization"}}"#;
        let (r, _, lines, api) = ask_with(Some(KEY), FAST, move |_, _, nth| {
            if nth == 0 { reply(400, refused) } else { stream(&text_answer("Ответ.")) }
        })
        .await;
        assert_eq!(r.end, AskEnd::Answer);
        assert_eq!(api.calls().len(), 2);
        let second: Value = serde_json::from_slice(&api.body(1)).unwrap();
        assert_eq!(second["tools"], tools(None), "the second request has no search");
        assert_eq!(lines.len(), 1, "one line says why: {lines:?}");
        assert_clean(&lines);
    }

    /// A write cut at `max_tokens` never runs; once, the same question goes again with room for a long text.
    #[tokio::test]
    async fn a_write_cut_at_max_tokens_is_asked_once_more_with_room() {
        let mut cut = write_answer("Длинное письмо", "insert");
        let n = cut.len();
        cut[n - 2] = json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" }, "usage": { "output_tokens": 4_048 } });
        let whole = write_answer("Длинное письмо целиком.", "insert");
        let (r, _, _, api) = ask_with(Some(KEY), FAST, move |_, _, nth| if nth == 0 { stream(&cut) } else { stream(&whole) }).await;
        assert_eq!(r.end, AskEnd::Write { text: "Длинное письмо целиком.".into(), replace: false });
        let first: Value = serde_json::from_slice(&api.body(0)).unwrap();
        let second: Value = serde_json::from_slice(&api.body(1)).unwrap();
        assert_eq!(second["max_tokens"].as_u64(), first["max_tokens"].as_u64().map(|t| t + 4_000));
        assert_eq!(second["messages"], first["messages"], "the same question; the cut call is left out");
        assert_eq!(r.content.len(), 1, "only the whole call");
        assert_eq!(r.usage.output_tokens, 4_068, "both responses are paid for");
    }

    /// Spec 2 §10: an answer closed mid-stream still writes what it spent, so `run` reports the cost as it grows.
    #[tokio::test]
    async fn the_cost_so_far_is_reported_as_it_grows() {
        let api = fake_api(|_, _, _| stream(&text_answer("I'm a teapot."))).await;
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/v1/messages", api.base);
        let mut spent = Vec::new();
        let r = run_at(&url, &http, Some(KEY), "claude-haiku-5-5", None, Thinking::DEFAULT, &[question()], FAST, |s| {
            if let Step::Spent(u) = s {
                spent.push(u);
            }
        })
        .await;
        assert_eq!(spent.first().map(|u| u.input_tokens), Some(40), "known from the first event");
        assert_eq!(spent.last(), Some(&r.usage), "the last report is the whole cost");
    }

    #[tokio::test]
    async fn refusals_before_the_stream_are_a_key_problem_or_worth_a_retry() {
        for (code, body, want) in [
            (401, "{}", AskFailure::Key(401)),
            (403, "{}", AskFailure::Key(403)),
            (400, r#"{"error":{"message":"Your credit balance is too low"}}"#, AskFailure::Key(400)),
            (429, "{}", AskFailure::Http(429)),
            (529, "{}", AskFailure::Http(529)),
            (400, r#"{"error":{"message":"bad"}}"#, AskFailure::Http(400)),
            (404, r#"{"error":{"type":"not_found_error"}}"#, AskFailure::NoModel),
        ] {
            let (r, _, lines, _) = ask_with(Some(KEY), FAST, move |_, _, _| reply(code, body)).await;
            assert_eq!(r.end, AskEnd::Failed(want), "HTTP {code}");
            assert_eq!(lines.len(), 1, "one line: {lines:?}");
            assert!(lines[0].starts_with("WARN"), "{lines:?}");
            assert_clean(&lines);
        }
        let (r, _, lines, api) = ask_with(None, FAST, |_, _, _| stream(&text_answer("x"))).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::NoKey));
        assert!(AskFailure::NoKey.is_key() && AskFailure::Key(402).is_key());
        assert!(!AskFailure::Http(429).is_key() && !AskFailure::Timeout.is_key(), "429 counts with the network (spec 2 §8)");
        assert!(!AskFailure::NoModel.is_key());
        assert!(api.calls().is_empty(), "no key, no request");
        assert_eq!(lines.len(), 1);
    }

    #[tokio::test]
    async fn an_error_event_mid_stream_fails_with_its_type() {
        let mut events = text_answer("част")[..3].to_vec();
        events.push(json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } }));
        let (r, _, lines, _) = ask_with(Some(KEY), FAST, move |_, _, _| stream(&events)).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Stream("overloaded_error".into())));
        assert_eq!((r.usage.input_tokens, r.usage.cache_read_tokens), (40, 900), "what came before the error is paid for");
        assert_clean(&lines);
    }

    #[tokio::test]
    async fn no_first_words_in_time_is_a_timeout() {
        let t0 = std::time::Instant::now();
        let (r, _, _, _) = ask_with(Some(KEY), FAST, |_, _, _| Reply::Hang).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Timeout));
        assert!(t0.elapsed() < Duration::from_millis(1_500), "{:?}", t0.elapsed());
    }

    /// Spec 2 §6.2: the first words wait longer once the model searches.
    #[tokio::test]
    async fn a_search_gives_the_first_words_more_time() {
        let searching = vec![
            start(json!({ "input_tokens": 50 })),
            block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            stop(0),
        ];
        // The answer's text blocks after the search call: block indexes shifted by one.
        let rest = shifted(&text_answer("Ответ.")[1..], 1);
        let later = rest.clone();
        let (r, _, _, _) = ask_with(Some(KEY), FAST, move |_, _, _| {
            Reply::Stream(200, vec![sse(&searching), sse(&later)], Duration::from_millis(700))
        })
        .await;
        assert_eq!(r.end, AskEnd::Answer, "a search began: 1.2 s for the first words");
        let silent = vec![start(json!({ "input_tokens": 50 }))];
        let (r, _, _, _) = ask_with(Some(KEY), FAST, move |_, _, _| {
            Reply::Stream(200, vec![sse(&silent), sse(&rest)], Duration::from_millis(700))
        })
        .await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Timeout), "no search: 0.4 s");
    }

    /// A search began, then the deadline cut the question: the search is in what it spent.
    #[tokio::test]
    async fn a_question_cut_after_its_search_still_counts_the_search() {
        let searching = vec![
            start(json!({ "input_tokens": 50, "cache_read_input_tokens": 900 })),
            block_start(0, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": {} })),
            stop(0),
        ];
        let (r, _, _, _) = ask_with(Some(KEY), FAST, move |_, _, _| Reply::Stream(200, vec![sse(&searching)], Duration::from_secs(5))).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Timeout));
        assert_eq!((r.usage.web_searches, r.usage.cache_read_tokens), (1, 900));
    }

    #[tokio::test]
    async fn the_whole_answer_has_a_deadline() {
        let mut pieces = vec![sse(&text_answer("Начало")[..3])];
        for _ in 0..5 {
            pieces.push(sse(&[delta(0, json!({ "type": "text_delta", "text": " ещё" }))]));
        }
        let (r, steps, _, _) = ask_with(Some(KEY), FAST, move |_, _, _| Reply::Stream(200, pieces.clone(), Duration::from_millis(600))).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Timeout));
        assert!(steps.contains(&Step::Text("Начало".into())), "the words that came were shown");
        assert_eq!((r.usage.input_tokens, r.usage.cache_read_tokens), (40, 900), "an answer cut by the deadline is paid for");
    }

    /// The retry of a cut write has deadlines of its own, from when it is sent: the cut call has usually used most of the
    /// whole answer's time, and a retry left with the rest of it would end in a timeout with both calls paid for.
    #[tokio::test]
    async fn the_retry_of_a_cut_write_has_its_own_deadlines() {
        let mut cut = write_answer("Длинное письмо", "insert");
        let n = cut.len();
        cut[n - 2] = json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" }, "usage": { "output_tokens": 4_048 } });
        let whole = write_answer("Длинное письмо целиком.", "insert");
        // Each call: the start of the write at once, the rest 0.8 s later. The two together take longer than the whole
        // answer's 1.2 s.
        fn slow(events: &[Value]) -> Reply {
            Reply::Stream(200, vec![sse(&events[..2]), sse(&events[2..])], Duration::from_millis(800))
        }
        let limits = Limits { total: Duration::from_millis(1_200), ..FAST };
        let (r, _, _, api) = ask_with(Some(KEY), limits, move |_, _, nth| if nth == 0 { slow(&cut) } else { slow(&whole) }).await;
        assert_eq!(api.calls().len(), 2);
        assert_eq!(r.end, AskEnd::Write { text: "Длинное письмо целиком.".into(), replace: false });
        assert_eq!(r.usage.output_tokens, 4_068);
    }

    #[tokio::test]
    async fn a_dropped_connection_is_a_network_failure() {
        let (r, _, lines, _) = ask_with(Some(KEY), FAST, |_, _, _| Reply::Cut).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Network));
        assert_eq!(lines.len(), 1);
        let (r, _, _, _) = ask_with(Some(KEY), FAST, |_, _, _| Reply::Close).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Network));
    }

    #[tokio::test]
    async fn a_write_a_refusal_and_nothing() {
        let (r, steps, _, _) = ask_with(Some(KEY), FAST, |_, _, _| stream(&write_answer("Пришлю в четверг.", "insert"))).await;
        assert_eq!(r.end, AskEnd::Write { text: "Пришлю в четверг.".into(), replace: false });
        assert_eq!(steps, [Step::WriteStart]);
        let mut refused = text_answer("Я не");
        let n = refused.len();
        refused[n - 2] = json!({ "type": "message_delta", "delta": { "stop_reason": "refusal" }, "usage": { "output_tokens": 2 } });
        let (r, _, lines, _) = ask_with(Some(KEY), FAST, move |_, _, _| stream(&refused)).await;
        assert_eq!(r.end, AskEnd::Refused);
        assert!(lines.is_empty(), "a refusal is an answer, not a failure");
        let mut nothing = vec![start(json!({ "input_tokens": 5 }))];
        nothing.extend(end("end_turn", json!({ "output_tokens": 0 })));
        let (r, _, _, _) = ask_with(Some(KEY), FAST, move |_, _, _| stream(&nothing)).await;
        assert_eq!(r.end, AskEnd::Failed(AskFailure::Malformed));
    }
}
