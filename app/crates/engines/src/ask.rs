//! "Ask" through Claude (spec 2 §6.2): the streaming request with the write_to_field tool and web search, the turns of the
//! conversation, the parser of the event stream and what the model decided; the network part is `run` and `warm`. No
//! text of the question, the answer, the selection or the sources reaches the log here.
use std::collections::HashSet;

use cue_core::ask as core_ask;
use serde_json::{json, Value};

use crate::claude::THINKING_HEADROOM;

/// The web search tool (spec 2 §6.2); which version — the first probe's choice (§13.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchTool {
    /// `web_search_20250305`, called by the model directly.
    Basic { max_uses: u32 },
    /// `web_search_20260318`: the model filters the results in code before it reads them.
    Filtered { max_uses: u32 },
}

impl SearchTool {
    /// Searches one request may run (`max_uses` applies per request, continuations included one by one).
    pub fn max_uses(self) -> u32 {
        match self {
            SearchTool::Basic { max_uses } | SearchTool::Filtered { max_uses } => max_uses,
        }
    }

    pub fn json(self) -> Value {
        match self {
            SearchTool::Basic { max_uses } => json!({
                "type": "web_search_20250305", "name": "web_search", "max_uses": max_uses, "allowed_callers": ["direct"]
            }),
            SearchTool::Filtered { max_uses } => {
                json!({ "type": "web_search_20260318", "name": "web_search", "max_uses": max_uses })
            }
        }
    }
}

/// The web search the app sends for `max_uses` searches a question; `None` when the model takes none. The version is the
/// first probe's choice (spec 2 §13.1, `examples/ask_probe.rs`).
pub fn search_tool(max_uses: u32) -> Option<SearchTool> {
    Some(SearchTool::Basic { max_uses })
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
/// or off. The bench (Task 8) compares the two; the app sends `DEFAULT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Thinking {
    Adaptive,
    Off,
}

impl Thinking {
    pub const DEFAULT: Thinking = Thinking::Adaptive;
}

/// The streaming request (spec 2 §6.2): the instruction with a cache mark (it covers the tools before it), low effort,
/// thinking as `thinking` says (adaptive gets room for it on top of the answer's budget), `tool_choice: auto`, and the
/// conversation with a second mark on the last user message, so a follow-up reads what came before from the cache.
/// `container` — the code execution container the API gave the conversation (filtered search), sent back.
pub fn request_body(
    model: &str,
    search: Option<SearchTool>,
    thinking: Thinking,
    messages: &[Value],
    container: Option<&str>,
) -> Value {
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
    if let Some(c) = container {
        v["container"] = json!(c);
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

/// `max_tokens` of the cache warm-up (spec 2 §6.3): no output is paid for. The first probe (§13.1) keeps it at 0 or, if the
/// API refuses that, sets 1.
pub const WARM_MAX_TOKENS: u32 = 0;

/// The cache warm-up (spec 2 §6.3): the request's prefix — the same tools, instruction, thinking, effort and tool choice —
/// with a one-character placeholder, no stream, no output. The mark stays only on the instruction: on the placeholder it
/// would key the cache to it.
pub fn warm_body(model: &str, search: Option<SearchTool>) -> Value {
    let mut v = request_body(model, search, Thinking::DEFAULT, &[user_turn(".", None)], None);
    if let Some(o) = v.as_object_mut() {
        o.remove("stream");
    }
    v["max_tokens"] = json!(WARM_MAX_TOKENS);
    if let Some(block) = v["messages"][0]["content"][0].as_object_mut() {
        block.remove("cache_control");
    }
    v
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
    container: Option<String>,
    stopped: bool,
}

/// One response, complete.
#[derive(Debug, Default, PartialEq)]
pub struct StreamTurn {
    pub content: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: AskUsage,
    pub container: Option<String>,
}

impl StreamState {
    /// The data of one event; what it shows on screen.
    pub fn feed(&mut self, data: &str) -> Result<Vec<Step>, StreamError> {
        let v: Value = serde_json::from_str(data).map_err(|_| StreamError::Malformed)?;
        let mut steps = Vec::new();
        match v["type"].as_str().unwrap_or("") {
            "message_start" => {
                self.read_usage(&v["message"]["usage"]);
                self.read_container(&v["message"]["container"]);
            }
            "content_block_start" => {
                let i = block_index(&v)?;
                let block = v["content_block"].clone();
                let name = block["name"].as_str().unwrap_or("");
                match block["type"].as_str().unwrap_or("") {
                    "server_tool_use" if name == "web_search" => steps.push(Step::Searching),
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
                        if c["type"] == "web_search_result_location"
                            && let (Some(url), Some(title)) = (c["url"].as_str(), c["title"].as_str())
                        {
                            steps.push(Step::Cite { url: url.to_owned(), title: title.to_owned() });
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
                self.read_container(&v["delta"]["container"]);
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

    /// The counts of this response so far.
    pub fn usage(&self) -> AskUsage {
        self.usage
    }

    pub fn finish(self) -> StreamTurn {
        StreamTurn {
            content: self.blocks.into_iter().filter(|b| !b.is_null()).collect(),
            stop_reason: self.stop_reason,
            usage: self.usage,
            container: self.container,
        }
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

    /// The code execution container of a filtered search, to send back with the next request of the conversation.
    fn read_container(&mut self, c: &Value) {
        if let Some(id) = c["id"].as_str().or_else(|| c.as_str()) {
            self.container = Some(id.to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::ask::ASK_PROMPT;

    fn question() -> Value {
        user_turn("<question>что такое HTTP 418</question>", None)
    }

    #[test]
    fn the_request_streams_with_both_tools_and_two_cache_marks() {
        let v = request_body("claude-haiku-5-5", Some(SearchTool::Basic { max_uses: 2 }), Thinking::DEFAULT, &[question()], None);
        assert_eq!(v["model"], "claude-haiku-5-5");
        assert_eq!(v["stream"], true);
        assert_eq!(v["max_tokens"], 4_048, "the answer's budget and room for thinking");
        assert!(v.get("thinking").is_none(), "adaptive thinking is the model's default (decision 8)");
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
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &[question()], None);
        assert_eq!(v["tools"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn filtered_search_lets_the_model_filter_in_code() {
        assert_eq!(
            SearchTool::Filtered { max_uses: 3 }.json(),
            json!({ "type": "web_search_20260318", "name": "web_search", "max_uses": 3 })
        );
        assert_eq!((SearchTool::Filtered { max_uses: 3 }.max_uses(), SearchTool::Basic { max_uses: 2 }.max_uses()), (3, 2));
    }

    /// The cache mark goes on the last user message only (four marks at most), also while a paused turn is continued;
    /// the stored conversation keeps none.
    #[test]
    fn only_the_last_user_message_carries_the_mark() {
        let answer = json!({ "role": "assistant", "content": [{ "type": "text", "text": "Ответ." }] });
        let follow = user_turn("<question>а подробнее</question>", None);
        let msgs = vec![question(), answer.clone(), follow];
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &msgs, None);
        assert!(v["messages"][0]["content"][0].get("cache_control").is_none());
        assert!(v["messages"][1]["content"][0].get("cache_control").is_none());
        assert_eq!(v["messages"][2]["content"][0]["cache_control"], json!({ "type": "ephemeral" }));
        assert!(msgs[2]["content"][0].get("cache_control").is_none(), "the conversation itself is untouched");
        let paused = vec![question(), answer];
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &paused, None);
        assert_eq!(v["messages"][0]["content"][0]["cache_control"], json!({ "type": "ephemeral" }));
        assert!(v["messages"][1]["content"][0].get("cache_control").is_none());
    }

    /// The bench's other arm (decision 8): thinking off, and no room kept for it.
    #[test]
    fn thinking_off_is_sent_as_disabled_without_headroom() {
        let v = request_body("claude-haiku-5-5", None, Thinking::Off, &[question()], None);
        assert_eq!(v["max_tokens"], 2_000);
        assert_eq!(v["thinking"], json!({ "type": "disabled" }));
    }

    #[test]
    fn the_container_goes_back_when_there_is_one() {
        let v = request_body("claude-haiku-5-5", None, Thinking::DEFAULT, &[question()], Some("container_011"));
        assert_eq!(v["container"], "container_011");
    }

    /// The warm-up builds the prefix the real request reads (spec 2 §6.3): the same tools, instruction, thinking, effort
    /// and tool choice; no stream, no output, the mark on the instruction only (a mark on the placeholder would key the
    /// cache to it).
    #[test]
    fn the_warm_up_has_the_prefix_and_nothing_else() {
        let search = Some(SearchTool::Basic { max_uses: 2 });
        let warm = warm_body("claude-haiku-5-5", search);
        let real = request_body("claude-haiku-5-5", search, Thinking::DEFAULT, &[question()], None);
        for k in ["model", "system", "tools", "tool_choice", "thinking", "output_config"] {
            assert_eq!(warm[k], real[k], "{k}");
        }
        assert!(warm.get("stream").is_none());
        assert_eq!(warm["max_tokens"], WARM_MAX_TOKENS);
        assert_eq!(warm["messages"], json!([{ "role": "user", "content": [{ "type": "text", "text": "." }] }]));
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

    /// A refusal wins over whatever came before it: a call it cut must not run (spec 2 §6.2).
    #[test]
    fn a_refusal_wins() {
        let mut events = write_answer("x", "insert");
        events.truncate(4);
        events.extend(end("refusal", json!({ "output_tokens": 3 })));
        let (_, turn) = feed_all(&events);
        assert_eq!(decide(&turn.content, turn.stop_reason.as_deref()), Decision::Refused);
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

    #[test]
    fn the_container_is_remembered() {
        let mut st = StreamState::default();
        st.feed(&json!({ "type": "message_start", "message": { "usage": {}, "container": { "id": "container_011", "expires_at": "x" } } }).to_string()).unwrap();
        assert_eq!(st.finish().container.as_deref(), Some("container_011"));
    }

    #[test]
    fn usage_adds_up_over_continuations() {
        let mut u = AskUsage { input_tokens: 1, output_tokens: 2, cache_read_tokens: 3, cache_write_tokens: 4, web_searches: 1 };
        u.add(AskUsage { input_tokens: 10, output_tokens: 20, cache_read_tokens: 30, cache_write_tokens: 40, web_searches: 1 });
        assert_eq!(u, AskUsage { input_tokens: 11, output_tokens: 22, cache_read_tokens: 33, cache_write_tokens: 44, web_searches: 2 });
    }
}
