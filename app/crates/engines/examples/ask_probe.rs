//! The first probe of "Ask" (spec 2 §13.1), paid (about $0.10): what the Models API says about the model, which web search
//! version it takes, whether the cache warm-up is accepted and writes the cache, how fast the first words come, whether a
//! follow-up after a search reads the cache, and whether a request for text calls write_to_field. Prints numbers and
//! kinds only.
//! Run: cargo run -p cue-engines --example ask_probe [-- <model>]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use cue_core::ask::{first_message, follow_up_message, AskContext};
use cue_engines::ask::{self, AskEnd, AskResult, SearchTool, Thinking};
use cue_engines::claude::{API_URL, API_VERSION};
use cue_engines::keys::{self, KeyName};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::args().nth(1).unwrap_or_else(|| "claude-haiku-5-5".into());
    let key = keys::get(KeyName::Anthropic)?;
    let http = reqwest::Client::new();

    let info: Value = http
        .get(format!("https://api.anthropic.com/v1/models/{model}"))
        .header("x-api-key", &key)
        .header("anthropic-version", API_VERSION)
        .send()
        .await?
        .json()
        .await?;
    let caps = &info["capabilities"];
    // `thinking disabled` is for information only: the app thinks adaptively (decision 8), the bench compares.
    println!(
        "models: web_search {} · programmatic tool calling {} · thinking disabled {}",
        caps["server_tools"]["web_search"]["supported"], caps["code_execution"]["supported"], caps["thinking"]["types"]["disabled"]["supported"]
    );

    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let today = cue_core::usage::utc_timestamp(secs)[..10].to_owned();
    let ctx = |selection: Option<&str>| AskContext {
        app: "Google Chrome".into(),
        before_cursor: None,
        selection: selection.map(str::to_owned),
        today: today.clone(),
    };

    for search in [SearchTool::Basic { max_uses: 2 }, SearchTool::Filtered { max_uses: 2 }] {
        println!("--- {search:?}");
        // The warm-up twice: the second one should read what the first one wrote.
        for round in 1..=2 {
            let resp = http
                .post(API_URL)
                .header("x-api-key", &key)
                .header("anthropic-version", API_VERSION)
                .json(&ask::warm_body(&model, Some(search)))
                .send()
                .await?;
            let status = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or_default();
            let u = &body["usage"];
            println!(
                "warm-up {round}: HTTP {status} · cache write {} · read {} · output {} · error {}",
                u["cache_creation_input_tokens"], u["cache_read_input_tokens"], u["output_tokens"], body["error"]["type"]
            );
        }
        // A question that needs a search, three times; then a follow-up after the last one.
        let mut last = None;
        for run in 1..=3 {
            let messages = vec![ask::user_turn(&first_message("какая сейчас последняя версия Rust", &ctx(None)), None)];
            let (r, first) = timed(&http, &key, &model, Some(search), &messages, None).await;
            println!(
                "search {run}: {} · first words {first:?} ms · searches {} · cache read {} · container {}",
                kind(&r.end), r.usage.web_searches, r.usage.cache_read_tokens, r.container.is_some()
            );
            last = Some((messages, r));
        }
        if let Some((mut messages, r)) = last
            && r.end == AskEnd::Answer
        {
            messages.push(ask::assistant_turn(&r.content));
            messages.push(ask::user_turn(&follow_up_message("а когда выйдет следующая"), None));
            let (f, first) = timed(&http, &key, &model, Some(search), &messages, r.container.clone()).await;
            println!(
                "follow-up: {} · first words {first:?} ms · cache read {} · write {}",
                kind(&f.end), f.usage.cache_read_tokens, f.usage.cache_write_tokens
            );
        }
    }
    let selection = Some("Can you send the report by Friday?");
    let messages = vec![ask::user_turn(&first_message("ответь что пришлю в четверг", &ctx(selection)), None)];
    let (r, first) = timed(&http, &key, &model, ask::search_tool(2), &messages, None).await;
    println!("write: {} · first words {first:?} ms", kind(&r.end));
    // Decision 24 live: a follow-up after a write shown instead of written answers the call first.
    let call = json!({ "type": "tool_use", "id": "toolu_probe", "name": "write_to_field", "input": { "text": "x", "mode": "replace" } });
    let messages = vec![
        ask::user_turn(&first_message("исправь", &ctx(selection)), None),
        ask::assistant_turn(std::slice::from_ref(&call)),
        ask::user_turn(&follow_up_message("а по-другому"), Some(std::slice::from_ref(&call))),
    ];
    let (r, _) = timed(&http, &key, &model, None, &messages, None).await;
    println!("follow-up after a shown write: {}", kind(&r.end));
    Ok(())
}

async fn timed(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    search: Option<SearchTool>,
    messages: &[Value],
    container: Option<String>,
) -> (AskResult, Option<u128>) {
    let t0 = Instant::now();
    let mut first = None;
    let r = ask::run(http, Some(key), model, search, Thinking::DEFAULT, messages, container, |s| {
        let words = s.is_first_words();
        if words && first.is_none() {
            first = Some(t0.elapsed().as_millis());
        }
    })
    .await;
    (r, first)
}

fn kind(end: &AskEnd) -> String {
    match end {
        AskEnd::Answer => "answer".into(),
        AskEnd::Write { replace, .. } => format!("write ({})", if *replace { "replace" } else { "insert" }),
        AskEnd::Refused => "refused".into(),
        AskEnd::Failed(f) => format!("failed {f:?}"),
    }
}
