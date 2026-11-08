### Task 5: the rule request through the cleanup's connection (`cue-engines::habit`)

**Files:**
- Create: `app/crates/engines/src/habit.rs`
- Modify: `app/crates/engines/src/lib.rs` (`pub mod habit;` after `pub mod error;`), `claude.rs` (`thinking_for`,
  line 20, and `effort_for`, line 31, become `pub(crate)`), `chat.rs` (`post`, line 100, becomes `pub(crate)`),
  `openrouter.rs` (`TITLE`, line 23, becomes `pub(crate)`)

**Interfaces:**
- Consumes: `habits::{RuleAsk, RuleText, rule_messages, read_rule}` (Task 2).
- Produces: `pub async fn rule(http: &reqwest::Client, key: Option<&str>, s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText>`
  (`None`: no key, refused, no answer within `timeout`, or none usable; one log line with the reason, never text);
  `pub fn anthropic_body(model: &str, system: &str, user: &str) -> Value`; `pub fn chat_body(s: &RefineSettings, system: &str, user: &str) -> Value`.

- [ ] **Step 1: write the failing tests** (end of `habit.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{capture_logs, fake_api, Reply};
    use cue_core::habits::{Draft, Form};

    const KEY: &str = "sk-HABIT-TEST-0123456789";
    const WAIT: Duration = Duration::from_secs(5);

    fn ask() -> RuleAsk {
        let forms = vec![Form::FirstLetter { lower: true }, Form::EndMark { mark: '.', removed: true }];
        Draft { forms, app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 }.ask()
    }
    fn over(id: &str, base: &str) -> RefineSettings {
        let mut s = RefineSettings { connection: id.into(), ..RefineSettings::default() };
        s.addresses.insert(id.into(), base.into());
        s
    }
    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }
    /// The only words of the user's in a request: the example and the app, in the user message.
    fn only_the_ask(user: &str) {
        let v: Value = serde_json::from_str(user).unwrap();
        assert_eq!(v, json!({ "kind": ["firstLetter", "endMark"], "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram" }));
    }

    #[tokio::test]
    async fn anthropic_gets_the_kind_the_example_and_the_app_and_its_rule_comes_back() {
        let text = json!({ "rule": "В Telegram — с маленькой буквы и без точки", "short": "без точки" }).to_string();
        let answer = json!({ "content": [{ "type": "text", "text": text }], "stop_reason": "end_turn", "usage": {} }).to_string();
        let api = fake_api(move |_, _, _| Reply::Send(200, answer.clone(), "")).await;
        let s = over("anthropic", &format!("{}/v1", api.base));
        let r = rule(&http(), Some(KEY), &s, &ask(), "ru", WAIT).await.unwrap();
        assert_eq!((r.rule.as_str(), r.short.as_str()), ("В Telegram — с маленькой буквы и без точки", "без точки"));
        assert_eq!(api.calls(), ["POST /v1/messages"]);
        let sent: Value = serde_json::from_slice(&api.body(0)).unwrap();
        let (system, user) = habits::rule_messages(&ask(), "ru");
        assert_eq!(sent, anthropic_body(s.model().id, &system, &user));
        assert_eq!(sent["messages"].as_array().unwrap().len(), 1);
        only_the_ask(sent["messages"][0]["content"].as_str().unwrap());
    }

    #[tokio::test]
    async fn a_chat_service_gets_the_same_and_streams_its_rule() {
        let chunk = |content: &str, finish: Option<&str>| {
            json!({ "id": "c1", "model": "gpt-6-luna", "choices": [{ "index": 0, "delta": { "content": content }, "finish_reason": finish }] }).to_string()
        };
        let text = json!({ "rule": "In Telegram — no full stop", "short": "no full stop" }).to_string();
        let body: String = [chunk(&text, None), chunk("", Some("stop")), "[DONE]".to_owned()].iter().map(|e| format!("data: {e}\n\n")).collect();
        let api = fake_api(move |_, _, _| Reply::Send(200, body.clone(), "")).await;
        let s = over("openai", &format!("{}/v1", api.base));
        let r = rule(&http(), Some(KEY), &s, &ask(), "en", WAIT).await.unwrap();
        assert_eq!(r.short, "no full stop");
        assert_eq!(api.calls(), ["POST /v1/chat/completions"]);
        let sent: Value = serde_json::from_slice(&api.body(0)).unwrap();
        assert_eq!(sent, chat_body(&s, &habits::rule_messages(&ask(), "en").0, &habits::rule_messages(&ask(), "en").1));
        assert_eq!(sent["messages"].as_array().unwrap().len(), 2);
        only_the_ask(sent["messages"][1]["content"].as_str().unwrap());
        assert_eq!(sent["response_format"]["json_schema"]["name"], "habit_rule");
    }

    #[tokio::test]
    async fn no_key_no_answer_in_time_or_an_unusable_one_give_none_and_log_no_text() {
        let (logs, _guard) = capture_logs();
        assert!(rule(&http(), None, &RefineSettings::default(), &ask(), "en", WAIT).await.is_none(), "no key: not even asked");
        let api = fake_api(|_, _, _| Reply::Hang).await;
        let s = over("anthropic", &format!("{}/v1", api.base));
        let t0 = std::time::Instant::now();
        assert!(rule(&http(), Some(KEY), &s, &ask(), "en", Duration::from_millis(200)).await.is_none());
        assert!(t0.elapsed() < Duration::from_secs(2), "the wait ends the request");
        let bad = json!({ "content": [{ "type": "text", "text": "{\"rule\":\"\",\"short\":\"\"}" }], "stop_reason": "end_turn" }).to_string();
        let api = fake_api(move |_, _, _| Reply::Send(200, bad.clone(), "")).await;
        assert!(rule(&http(), Some(KEY), &over("anthropic", &format!("{}/v1", api.base)), &ask(), "en", WAIT).await.is_none());
        for line in logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::habit")) {
            assert!(!line.contains("Скинь") && !line.contains("скинь") && !line.contains(KEY), "{line}");
        }
    }
}
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-engines habit`.

- [ ] **Step 3: implement** (`habit.rs`, above `mod tests`):

```rust
//! The rule of a habit just learned (docs/specs/2026-11-04-habits.md), asked of the cleanup's connection: one short
//! request with the kind of the correction, its example and the app's name, nothing else (`habits::rule_messages`).
//! Never fails: `None` on any problem, with one log line of the reason, and the caller writes the rule from the
//! template. The key goes only in its header; the answer never reaches the log.
use std::time::Duration;

use cue_core::connections::{Answer, Kind};
use cue_core::habits::{self, RuleAsk, RuleText};
use cue_core::settings::RefineSettings;
use serde_json::{json, Value};

use crate::{chat, claude, openrouter};

/// The answer is a sentence and a few words.
const RULE_TOKENS: u32 = 200;

/// The answer's shape: the rule and its short name.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "rule": { "type": "string" }, "short": { "type": "string" } },
        "required": ["rule", "short"],
        "additionalProperties": false
    })
}

/// The Messages API body: the instructions, the user message, the answer in the schema, effort and thinking as the
/// cleanup sets them for the model.
pub fn anthropic_body(model: &str, system: &str, user: &str) -> Value {
    let thinking = claude::thinking_for(model);
    let budget = RULE_TOKENS + if thinking.is_none() { claude::THINKING_HEADROOM } else { 0 };
    let mut v = json!({
        "model": model,
        "max_tokens": budget,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "output_config": { "effort": claude::effort_for(model), "format": { "type": "json_schema", "schema": schema() } }
    });
    if let Some(t) = thinking {
        v["thinking"] = t;
    }
    v
}

/// The Chat Completions body for OpenRouter and the OpenAI-compatible services, with what each needs as the cleanup's
/// requests have it: the reasoning setting, the budget's headroom, the answer's format.
pub fn chat_body(s: &RefineSettings, system: &str, user: &str) -> Value {
    let (c, model) = (s.connection(), s.model().id);
    let named = |strict: bool| json!({ "type": "json_schema", "json_schema": { "name": "habit_rule", "strict": strict, "schema": schema() } });
    let mut v = json!({
        "model": model,
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
        "stream": true
    });
    if matches!(c.kind, Kind::OpenRouter) {
        let p = openrouter::model_profile(model);
        v["max_tokens"] = json!(RULE_TOKENS + if p.reasoning.mandatory() { openrouter::REASONING_HEADROOM } else { 0 });
        if let Some(r) = p.reasoning.param() {
            v["reasoning"] = r;
        }
        if p.structured {
            v["response_format"] = named(true);
        }
    } else {
        let budget = RULE_TOKENS + if c.reasons { openrouter::REASONING_HEADROOM } else { 0 };
        v[if c.completion_tokens { "max_completion_tokens" } else { "max_tokens" }] = json!(budget);
        if let Some(effort) = c.reasoning {
            v["reasoning_effort"] = json!(effort);
        }
        v["response_format"] = match c.answer {
            Answer::Schema { strict } => named(strict),
            Answer::Object => json!({ "type": "json_object" }),
        };
    }
    v
}

/// No rule from the connection: why, in the log, and the template writes it.
fn skip(why: &str) -> Option<RuleText> {
    tracing::info!("habit rule: {why}, written from the template");
    None
}

/// The rule from the cleanup's connection in `s` (its address, its model), within `timeout`; a local server needs no
/// key, a cloud one without a key is not asked.
pub async fn rule(http: &reqwest::Client, key: Option<&str>, s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText> {
    let c = s.connection();
    let key = match key {
        Some(k) => k,
        None if c.local => "",
        None => return skip(&format!("no {} key", c.name)),
    };
    let (system, user) = habits::rule_messages(ask, lang);
    let content = match c.kind {
        Kind::Anthropic => {
            let url = format!("{}/messages", s.address());
            let body = anthropic_body(s.model().id, &system, &user);
            let resp = match claude::messages_post(http, &url, key).timeout(timeout).json(&body).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => return skip(&format!("HTTP {}", r.status().as_u16())),
                Err(e) => return skip(if e.is_timeout() { "timeout" } else { "network" }),
            };
            match resp.json::<claude::MessageResp>().await {
                Ok(m) => m.content.into_iter().find(|b| b.kind == "text").and_then(|b| b.text).unwrap_or_default(),
                Err(_) => return skip("response not parsed"),
            }
        }
        Kind::OpenRouter | Kind::Chat => {
            let req = if matches!(c.kind, Kind::OpenRouter) {
                http.post(format!("{}/chat/completions", s.address())).bearer_auth(key).header("X-Title", openrouter::TITLE)
            } else {
                chat::post(http, c, s.address(), key)
            };
            match openrouter::stream(req, key, &chat_body(s, &system, &user), timeout).await {
                Ok(done) => done.content,
                Err(f) => return skip(&openrouter::reason(&f)),
            }
        }
    };
    habits::read_rule(&content).or_else(|| skip("the answer is not a usable rule"))
}
```

  `claude::MessageResp` and its `Block` are already `pub`; `openrouter::stream` and `reason` are `pub(crate)`.

- [ ] **Step 4: run** `cargo test -p cue-engines` and `cargo clippy --workspace --all-targets -- -D warnings`: green.

---

