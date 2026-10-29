//! The first probe of page reading in "Ask" (spec 2 §12.12), paid (usually about $0.05; it stops before the next request
//! once past $0.08, so at most about $0.10): what the Models API says, which web_fetch version Haiku 5.5 takes next to
//! the filtered web search, whether the warm-up writes and reads the new prefix, whether the end of a response reports its
//! page reads, how a read page, a page that cannot be read and a PDF come back, which page a citation's `document_index`
//! names (in one response and after a follow-up that reads one more), and whether a site named in the question is opened.
//! Prints kinds, counts, codes and numbers, and the API's own message of a refused request (no text of ours is in it).
//! Run: cargo run -p cue-engines --example fetch_probe [-- <model>]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::bail;
use cue_core::ask::{first_message, follow_up_message, AskContext};
use cue_core::settings::Pricing;
use cue_core::usage::{refine_cost, utc_timestamp};
use cue_engines::ask::{self, AskEnd, AskResult, FetchTool, Step, Thinking, WebTools};
use cue_engines::claude::{API_URL, API_VERSION};
use cue_engines::keys::{self, KeyName};
use serde_json::Value;

/// The probe stops before a request once it has spent more than this: one request past it stays within $0.10.
const CAP_USD: f64 = 0.08;
const PAGE: &str = "https://en.wikipedia.org/wiki/Hyper_Text_Coffee_Pot_Control_Protocol";
const TWO: &str = "https://www.rfc-editor.org/rfc/rfc2324.txt https://www.rfc-editor.org/rfc/rfc7168.txt";
const MORE: &str = "https://en.wikipedia.org/wiki/Larry_Masinter";
/// A PDF of one page. A PDF is read whole, past `max_content_tokens`: on 2026-10-29 RFC 9110's PDF came in as about
/// 524 thousand tokens, which alone spent more than the cap (and past 100 thousand a prompt is priced higher than
/// `Pricing` counts). RFC 2324, the coffee pot's, has no PDF at the RFC Editor (404).
const PDF: &str = "https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf";
const SCRIPTED: &str = "https://excalidraw.com/";

struct Probe {
    http: reqwest::Client,
    key: String,
    model: String,
    today: String,
    pricing: Pricing,
    spent: f64,
}

/// What one question showed: its steps by kind and when its first words came.
#[derive(Default)]
struct Seen {
    fetching: usize,
    fetched: usize,
    cites: usize,
    first_words_ms: Option<u128>,
}

impl Probe {
    fn ctx(&self, selection: Option<&str>) -> AskContext {
        AskContext { app: "Google Chrome".into(), before_cursor: None, selection: selection.map(str::to_owned), today: self.today.clone() }
    }

    fn pay(&mut self, input: u64, cache_read: u64, cache_write: u64, output: u64, searches: u32) {
        self.spent += refine_cost(&self.pricing, &self.model, input, cache_read, cache_write, output)
            + f64::from(searches) * self.pricing.web_search_usd;
    }

    /// Spent past the cap: no more requests.
    fn spent_up(&self) -> bool {
        let up = self.spent > CAP_USD;
        if up {
            println!("stopped: more than ${CAP_USD} spent");
        }
        up
    }

    /// The warm-up as the app sends it: its status, the cache it wrote and read, the API's error if any. `false` once the
    /// cap is spent (nothing sent).
    async fn warm(&mut self, web: WebTools) -> anyhow::Result<bool> {
        if self.spent_up() {
            return Ok(false);
        }
        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", API_VERSION)
            .json(&ask::warm_body(&self.model, web))
            .send()
            .await?;
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or_default();
        let u = &body["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        println!(
            "warm-up: HTTP {status} · cache write {} · read {} · error {} {}",
            n("cache_creation_input_tokens"),
            n("cache_read_input_tokens"),
            body["error"]["type"],
            body["error"]["message"]
        );
        self.pay(n("input_tokens"), n("cache_read_input_tokens"), n("cache_creation_input_tokens"), n("output_tokens"), 0);
        Ok(true)
    }

    /// One question without a stream: what the end of the whole response reports of its server tools (the app counts the
    /// reads from the stream too, but the docs say the end reports them). `false` once the cap is spent.
    async fn reported(&mut self, web: WebTools, messages: &[Value]) -> anyhow::Result<bool> {
        if self.spent_up() {
            return Ok(false);
        }
        let mut body = ask::request_body(&self.model, web, Thinking::DEFAULT, messages, None);
        if let Some(o) = body.as_object_mut() {
            o.remove("stream");
        }
        let resp = self.http.post(API_URL).header("x-api-key", &self.key).header("anthropic-version", API_VERSION).json(&body).send().await?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().await.unwrap_or_default();
        let u = &v["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let calls = v["content"].as_array().into_iter().flatten().filter(|b| b["type"] == "server_tool_use" && b["name"] == "web_fetch").count();
        println!(
            "no stream: HTTP {status} · fetch calls {calls} · the end reports web_fetch_requests {} · web_search_requests {}",
            u["server_tool_use"]["web_fetch_requests"], u["server_tool_use"]["web_search_requests"]
        );
        let searches = u32::try_from(u["server_tool_use"]["web_search_requests"].as_u64().unwrap_or(0)).unwrap_or(u32::MAX);
        self.pay(n("input_tokens"), n("cache_read_input_tokens"), n("cache_creation_input_tokens"), n("output_tokens"), searches);
        Ok(true)
    }

    /// One question as the app asks it; `None` once the cap is spent.
    async fn ask(&mut self, web: WebTools, messages: &[Value], container: Option<String>) -> Option<(AskResult, Seen)> {
        if self.spent_up() {
            return None;
        }
        let t0 = Instant::now();
        let mut seen = Seen::default();
        let r = ask::run(&self.http, Some(&self.key), &self.model, web, Thinking::DEFAULT, messages, container, |s| {
            match &s {
                Step::Fetching => seen.fetching += 1,
                Step::Fetched { .. } => seen.fetched += 1,
                Step::Cite { .. } => seen.cites += 1,
                _ => {}
            }
            if s.is_first_words() && seen.first_words_ms.is_none() {
                seen.first_words_ms = Some(t0.elapsed().as_millis());
            }
        })
        .await;
        let u = r.usage;
        self.pay(u.input_tokens, u.cache_read_tokens, u.cache_write_tokens, u.output_tokens, u.web_searches);
        Some((r, seen))
    }
}

/// The titles of the pages read in these messages, in order (what a citation's `document_index` may count).
fn page_titles(blocks: &[Value]) -> Vec<Option<String>> {
    blocks
        .iter()
        .filter(|b| b["type"] == "web_fetch_tool_result" && b["content"]["type"] == "web_fetch_result")
        .map(|b| b["content"]["content"]["title"].as_str().map(str::to_owned))
        .collect()
}

fn history_blocks(messages: &[Value]) -> Vec<Value> {
    messages.iter().filter_map(|m| m["content"].as_array()).flatten().cloned().collect()
}

/// A question by kinds and numbers: the reads asked, what they brought (a page with its source kind, or an error code),
/// the kinds of the citations, each page citation's `document_index` and whether the page there — among the history's and
/// this answer's pages, in order — has the citation's title; the steps, the counts, the first words, the cache.
fn report(label: &str, r: &AskResult, seen: &Seen, history: &[Value]) {
    let mut titles = page_titles(&history_blocks(history));
    let (mut results, mut kinds, mut cites) = (Vec::new(), Vec::new(), Vec::new());
    for b in &r.content {
        match b["type"].as_str() {
            Some("web_fetch_tool_result") => {
                let c = &b["content"];
                if c["type"] == "web_fetch_result" {
                    let src = &c["content"]["source"];
                    results.push(format!("page ({} {})", src["type"].as_str().unwrap_or("?"), src["media_type"].as_str().unwrap_or("?")));
                    titles.push(c["content"]["title"].as_str().map(str::to_owned));
                } else {
                    results.push(format!("error {}", c["error_code"].as_str().unwrap_or("?")));
                }
            }
            Some("text") => {
                for c in b["citations"].as_array().into_iter().flatten() {
                    let kind = c["type"].as_str().unwrap_or("?").to_owned();
                    if !kinds.contains(&kind) {
                        kinds.push(kind);
                    }
                    if matches!(c["type"].as_str(), Some("char_location" | "page_location" | "content_block_location")) {
                        let i = c["document_index"].as_u64();
                        let there = i.and_then(|i| titles.get(usize::try_from(i).ok()?));
                        let fit = match there {
                            Some(t) if t.as_deref() == c["document_title"].as_str() => "title ok",
                            Some(_) => "other title",
                            None => "no such page",
                        };
                        cites.push(format!("{i:?} {fit}"));
                    }
                }
            }
            _ => {}
        }
    }
    let calls = r.content.iter().filter(|b| b["type"] == "server_tool_use" && b["name"] == "web_fetch").count();
    println!(
        "{label}: {} · fetch calls {calls} · results [{}] · citation kinds [{}] · page cites [{}] · steps fetching {} \
         fetched {} cites {} · reads {} · searches {} · first words {:?} ms · cache read {} write {}",
        kind(&r.end),
        results.join(", "),
        kinds.join(", "),
        cites.join(", "),
        seen.fetching,
        seen.fetched,
        seen.cites,
        r.usage.web_fetches,
        r.usage.web_searches,
        seen.first_words_ms,
        r.usage.cache_read_tokens,
        r.usage.cache_write_tokens
    );
}

fn kind(end: &AskEnd) -> String {
    match end {
        AskEnd::Answer => "answer".into(),
        AskEnd::Write { replace, .. } => format!("write ({})", if *replace { "replace" } else { "insert" }),
        AskEnd::Refused => "refused".into(),
        AskEnd::Failed(f) => format!("failed {f:?}"),
    }
}

/// The cap counts tokens at the default prices (`refine_cost`, which prices a model it does not know at 0): a model without
/// one would cost $0 a request, and the cap would never stop the probe (as `ask_compare`'s check).
fn check_priced(p: &Pricing, model: &str) -> anyhow::Result<()> {
    if p.claude_price(model).is_none() {
        let known: Vec<&str> = p.claude.iter().map(|m| m.model.as_str()).collect();
        bail!("{model} has no default price, so the cap could not stop the probe; priced: {}", known.join(", "));
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let model = std::env::args().nth(1).unwrap_or_else(|| "claude-haiku-5-5".into());
    // Before any request, the free Models API read included.
    check_priced(&Pricing::default(), &model)?;
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
    let tools = &info["capabilities"]["server_tools"];
    println!("models: web_fetch {} · web_search {}", tools["web_fetch"]["supported"], tools["web_search"]["supported"]);
    let secs = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let today = utc_timestamp(secs)[..10].to_owned();
    let mut p = Probe { http, key, model, today, pricing: Pricing::default(), spent: 0.0 };
    let search = ask::search_tool(2);
    let first = vec![ask::user_turn(&first_message("о чём эта страница", &p.ctx(Some(PAGE))), None)];
    for fetch in [FetchTool::Basic { max_uses: 2 }, FetchTool::Filtered { max_uses: 2 }] {
        println!("--- {fetch:?}");
        let web = WebTools { search, fetch: Some(fetch) };
        // Twice: the second should read what the first wrote.
        if !p.warm(web).await? || !p.warm(web).await? {
            break;
        }
        let Some((r, seen)) = p.ask(web, &first, None).await else { break };
        report("one page", &r, &seen, &first);
    }
    let web = WebTools::on(2);
    println!("--- the app's tools: {web:?}");
    // The same question once without a stream: does the end report its page reads?
    p.reported(web, &first).await?;
    let mut messages = vec![ask::user_turn(&first_message("чем отличаются эти два документа", &p.ctx(Some(TWO))), None)];
    if let Some((r, seen)) = p.ask(web, &messages, None).await {
        report("two pages", &r, &seen, &messages);
        if r.end == AskEnd::Answer {
            messages.push(ask::assistant_turn(&r.content));
            let follow = follow_up_message(&format!("а теперь сравни их с этой страницей {MORE}"));
            messages.push(ask::user_turn(&follow, Some(&r.content)));
            if let Some((f, seen)) = p.ask(web, &messages, r.container.clone()).await {
                report("follow-up with one more page", &f, &seen, &messages);
            }
        }
    }
    for (label, selection, question) in [
        ("pdf", Some(PDF), "кратко о чём этот документ"),
        ("scripted page", Some(SCRIPTED), "что на этой странице"),
        ("site in the question", None, "что написано на сайте example.com"),
    ] {
        let m = vec![ask::user_turn(&first_message(question, &p.ctx(selection)), None)];
        let Some((r, seen)) = p.ask(web, &m, None).await else { break };
        report(label, &r, &seen, &m);
    }
    println!("spent about ${:.3}", p.spent);
    Ok(())
}
