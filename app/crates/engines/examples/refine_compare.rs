//! Comparing refine models through OpenRouter: speed, price and outcome on a set of synthetic dictations; and the
//! app's translation on the direct path.
//!
//! Run:
//! cargo run -p cue-engines --example refine_compare -- phrases.json out.json [--models a,b,…] [--runs 3] [--max-usd 1.00]
//!   [--terms a,b,…] [--dry-run]
//!
//! - `phrases.json` is an array of `{id, app, raw, before_cursor?, to?}`; `out.json` gets all calls and the summary.
//! - Models are OpenRouter ids (`openai/gpt-6-luna`) and `claude-sonnet-5-5` — the app's current path (Anthropic direct,
//!   `claude::refine`, total time only). By default, the built-in candidate set (`DEFAULT_MODELS`).
//! - `to` translates the phrase into that language, as `Ctrl + Win + Shift` does in the app: a language code
//!   (`translate::known`; `"EN"` reads as `en`, an unknown code stops the run before any request). It runs only on the
//!   direct path (`claude::translate`, the translation check, the 4 s timeout); the OpenRouter models skip such a phrase,
//!   and `out.json` lists it under `skipped` ("translation is direct-only"). Each call in `out.json` carries `to`
//!   (`null` for a cleanup). A translation that fails its check is `raw_fallback`, where the app says "Couldn't
//!   translate".
//! - Each phrase → run 1..runs → each model in turn; everything is sequential. A model's first call is the warm-up
//!   (cold), reported separately in the summary; medians and p90 are over warm calls.
//! - The prompt, the context and the response check are the same as in the app: `cue_core::refine`, then `strip_markup`,
//!   `tidy_list_items` and `sanity_ok` (all via `claude::decide`) with default settings.
//! - `--terms` is a dictionary, comma-separated, sent to every model the way the app sends `dictionary.terms`;
//!   without it, none (as with an empty dictionary).
//! - Stops as soon as more than `--max-usd` has been spent. `--dry-run` only checks the keys, the phrases and the model list,
//!   without paid requests.
//!
//! Keys come from Credential Manager (`cue-cli keys set openrouter`; for the direct path, `anthropic`).
//! The console gets only numbers and ids; response texts go only to `out.json`.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use cue_core::flow::RefineOutcome;
use cue_core::refine::{timeout_ms, RefineContext, BOLD_PROMPT};
use cue_core::settings::{DictionarySettings, Pricing, RefineSettings};
use cue_core::translate;
use cue_engines::claude::{self, Job, Parsed};
use cue_engines::keys::{self, KeyName};
use cue_engines::openrouter::{self, Failure, ModelInfo, Profile, Routing};
use cue_engines::EngineError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The app's current path: its default model (Claude Sonnet 5.5) through Anthropic directly.
const DIRECT: &str = "claude-haiku-5-5";

/// The default set: the direct path, the same Haiku through OpenRouter (the middleman's price) and the candidates.
const DEFAULT_MODELS: [&str; 10] = [
    DIRECT,
    "anthropic/claude-haiku-5.5",
    "openai/gpt-6.1-terra",
    "openai/gpt-6-luna",
    "google/gemini-3.8-flash",
    "google/gemini-3.5-flash-lite",
    "deepseek/deepseek-v4.1-flash",
    "qwen/qwen3.8-flash",
    "z-ai/glm-5.3-flash",
    "openai/gpt-oss-120b",
];

/// Request timeout through OpenRouter — well above the app's timeout (3 s), so the whole latency tail is visible.
const ROUTER_TIMEOUT: Duration = Duration::from_secs(30);

const USAGE: &str = "refine_compare <phrases.json> <out.json> [--models a,b,…] [--runs N] [--max-usd X] [--terms a,b,…] [--dry-run]";

#[derive(Debug, PartialEq)]
struct Args {
    phrases: PathBuf,
    out: PathBuf,
    models: Vec<String>,
    runs: usize,
    max_usd: f64,
    /// The dictionary, as `dictionary.terms` in settings.json.
    terms: Vec<String>,
    dry_run: bool,
}

fn parse_args(args: &[String]) -> anyhow::Result<Args> {
    let mut positional = Vec::new();
    let (mut models, mut runs, mut max_usd, mut terms, mut dry_run) = (None, 3, 1.0, Vec::new(), false);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value\n{USAGE}"));
        match arg.as_str() {
            "--models" => {
                let mut list: Vec<String> = Vec::new();
                for id in value()?.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    if !list.iter().any(|m| m == id) {
                        list.push(id.to_owned());
                    }
                }
                models = Some(list);
            }
            "--runs" => runs = value()?.parse().ok().filter(|n| *n >= 1).context("--runs: an integer from 1")?,
            "--max-usd" => {
                max_usd = value()?.parse().ok().filter(|x: &f64| x.is_finite() && *x > 0.0).context("--max-usd: a number greater than 0")?
            }
            "--terms" => {
                terms = value()?.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
                if terms.is_empty() {
                    bail!("--terms: empty list");
                }
            }
            "--dry-run" => dry_run = true,
            flag if flag.starts_with("--") => bail!("unknown flag {flag}\n{USAGE}"),
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [phrases, out] = <[PathBuf; 2]>::try_from(positional).map_err(|_| anyhow!("two paths are needed\n{USAGE}"))?;
    let models = models.unwrap_or_else(|| DEFAULT_MODELS.map(str::to_owned).to_vec());
    if models.is_empty() {
        bail!("--models: empty list");
    }
    Ok(Args { phrases, out, models, runs, max_usd, terms, dry_run })
}

#[derive(Debug, Deserialize)]
struct Phrase {
    id: String,
    app: String,
    raw: String,
    #[serde(default)]
    before_cursor: Option<String>,
    /// `to` as written in the file; [`read_phrases`] checks it and fills [`Phrase::to`].
    #[serde(default, rename = "to")]
    to_code: Option<String>,
    /// Translate into this language instead of cleaning up: the language code (`translate::known`), the form the app
    /// sends.
    #[serde(skip)]
    to: Option<&'static str>,
}

fn read_phrases(path: &Path) -> anyhow::Result<Vec<Phrase>> {
    let bytes = std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    // A parse error gives only the position: serde quotes values.
    let mut phrases: Vec<Phrase> = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow!("{}: cannot parse (line {}, column {})", path.display(), e.line(), e.column()))?;
    if phrases.is_empty() {
        bail!("{}: no phrases", path.display());
    }
    for i in 0..phrases.len() {
        let p = &phrases[i];
        if p.id.trim().is_empty() || p.raw.trim().is_empty() {
            bail!("{}: phrase #{} has an empty id or raw", path.display(), i + 1);
        }
        if phrases[..i].iter().any(|q| q.id == p.id) {
            bail!("{}: id {} is repeated", path.display(), p.id);
        }
        // In settings.json an unknown code reads as the default; in a phrase file it is a typo, and the run would
        // measure another pair than the one meant.
        let to = match p.to_code.as_deref() {
            None => None,
            Some(code) => Some(
                translate::known(code)
                    .with_context(|| format!("{}: phrase {} has `to` that is not a language code CUE knows", path.display(), p.id))?,
            ),
        };
        phrases[i].to = to;
    }
    Ok(phrases)
}

/// How the model is called.
enum Engine {
    Direct,
    Router { info: Box<ModelInfo>, profile: Profile },
}

/// Why a model does not run a phrase, if it does not: a translation runs only on the direct path, the app's own
/// (comparing models for translation is not this bench's job). `direct` — the model is [`DIRECT`].
fn skip_reason(direct: bool, p: &Phrase) -> Option<&'static str> {
    (p.to.is_some() && !direct).then_some("translation is direct-only")
}

/// A phrase a model does not run, once per phrase and model — for `out.json`.
#[derive(Debug, Serialize)]
struct Skipped {
    model: String,
    phrase: String,
    reason: &'static str,
}

struct Lane {
    id: String,
    engine: Engine,
    /// Routing conditions: strict at first, relaxed ([`Routing::relaxed`]) if no provider matches them
    /// but one matches the relaxed ones.
    routing: Routing,
    calls: usize,
}

impl Lane {
    /// [`skip_reason`] for this model.
    fn skips(&self, p: &Phrase) -> Option<&'static str> {
        skip_reason(matches!(self.engine, Engine::Direct), p)
    }
}

/// One call — one entry in `out.json`.
#[derive(Clone, Debug, Default, Serialize)]
struct Call {
    seq: usize,
    model: String,
    phrase: String,
    /// The phrase's `to`: a translation into this language; `None` for a cleanup.
    to: Option<&'static str>,
    run: usize,
    /// A model's first call is the warm-up.
    cold: bool,
    ttft_ms: Option<u64>,
    total_ms: u64,
    /// All input, including the cache.
    input_tokens: u64,
    /// All output, including reasoning.
    output_tokens: u64,
    cached_tokens: u64,
    reasoning_tokens: u64,
    cost_usd: f64,
    /// `openrouter` (`usage.cost`), `estimate` (model list prices), `pricing` (the app's prices — direct path), `none`.
    cost_source: &'static str,
    finish_reason: Option<String>,
    /// `text`, `truncated`, `refusal`, `malformed`, `error`; for the direct path, `n/a`.
    parsed: &'static str,
    /// As in the app: `refined`, `raw_fallback`, `raw_key_problem`. A translation that fails its check is
    /// `raw_fallback`: the app shows "Couldn't translate" there.
    outcome: &'static str,
    /// Longer than the app's timeout (`timeout_ms`: 3 s for phrases under 100 words; a translation gets a second more).
    over_timeout: bool,
    /// When `outcome` is `refined`, the text that would be pasted at the caret. Otherwise the raw transcript: a cleanup
    /// pastes it, a translation pastes nothing (the app shows "Couldn't translate").
    output: String,
    /// The model's response as is; `None` on the direct path, which returns only the decided text.
    content: Option<String>,
    provider: Option<String>,
    served_model: Option<String>,
    /// `deny`, `allow` or `n/a` (direct path).
    data_collection: &'static str,
    /// The routing's `require_parameters`; for the direct path, `None`.
    require_parameters: Option<bool>,
    /// Over 1 means there were retries with relaxed routing.
    attempts: usize,
    error_kind: Option<String>,
    error: Option<String>,
    /// "No provider" refusals before the retries: the routing conditions and the error.
    routing_errors: Vec<String>,
}

fn outcome_name(o: RefineOutcome) -> &'static str {
    match o {
        RefineOutcome::Refined => "refined",
        RefineOutcome::RawFallback => "raw_fallback",
        RefineOutcome::RawAgent(_) => "raw_agent",
        RefineOutcome::Covered(_) => "covered",
        RefineOutcome::NoAi => "no_ai",
    }
}

fn parsed_name(p: &Parsed) -> &'static str {
    match p {
        Parsed::Text(_) => "text",
        Parsed::Truncated => "truncated",
        Parsed::Refusal => "refusal",
        Parsed::Malformed => "malformed",
    }
}

/// Failure kind — for the console and the summary, without text.
fn error_kind(f: &Failure) -> String {
    if let Some(code) = f.status {
        return format!("http_{code}");
    }
    match f.error {
        EngineError::Timeout => "timeout",
        EngineError::Network(_) => "network",
        EngineError::KeyProblem(_) | EngineError::NoBalance(_) => "stream_limit",
        EngineError::Service(_) => "stream_error",
        EngineError::Protocol(_) => "stream_protocol",
        EngineError::Aborted => "aborted",
        EngineError::Model(_) | EngineError::Unsupported(_) | EngineError::Llm(_) => "local",
    }
    .to_owned()
}

/// Routing conditions in words: "deny+require", "allow+require", "allow".
fn routing_name(r: Routing) -> String {
    let dc = if r.deny_data_collection { "deny" } else { "allow" };
    if r.require_parameters { format!("{dc}+require") } else { dc.to_owned() }
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

/// The app's direct path — `claude::refine` as is (its timeout is `timeout_ms`, 3 s), or `claude::translate` for a
/// phrase with `to` (`translate::timeout_ms`, a second more); total time only.
async fn direct_call(http: &reqwest::Client, key: &str, p: &Phrase, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings) -> Call {
    let t0 = Instant::now();
    let (r, timeout) = match p.to {
        None => (claude::refine(http, Some(key), DIRECT, &p.raw, ctx, dict, s, BOLD_PROMPT).await, timeout_ms(&p.raw)),
        Some(to) => (claude::translate(http, Some(key), DIRECT, &p.raw, ctx, dict, s, BOLD_PROMPT, to).await, translate::timeout_ms(&p.raw)),
    };
    let total_ms = ms(t0.elapsed());
    let u = r.usage;
    let cost = cue_core::usage::refine_cost(
        &Pricing::default(),
        DIRECT,
        u.input_tokens,
        u.cache_read_input_tokens,
        u.cache_creation_input_tokens,
        u.output_tokens,
    );
    Call {
        total_ms,
        input_tokens: u.input_tokens + u.cache_read_input_tokens + u.cache_creation_input_tokens,
        output_tokens: u.output_tokens,
        cached_tokens: u.cache_read_input_tokens,
        cost_usd: cost,
        cost_source: "pricing",
        parsed: "n/a",
        outcome: outcome_name(r.outcome),
        over_timeout: total_ms > timeout,
        output: r.text,
        provider: Some("anthropic-direct".into()),
        served_model: Some(DIRECT.into()),
        data_collection: "n/a",
        attempts: 1,
        ..Call::default()
    }
}

/// A call through OpenRouter. If no provider matches the routing conditions, retry with relaxed ones
/// (without `data_collection: deny`, then also without `require_parameters`); on success the model keeps them from then on.
#[allow(clippy::too_many_arguments)]
async fn router_call(
    http: &reqwest::Client,
    key: &str,
    id: &str,
    info: &ModelInfo,
    profile: &Profile,
    routing: &mut Routing,
    p: &Phrase,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
) -> Call {
    let body = |r: Routing| openrouter::request_body_styled(id, profile, r, &p.raw, ctx, dict, Job::Rewrite, BOLD_PROMPT);
    let routed = openrouter::complete_routed(http, key, *routing, ROUTER_TIMEOUT, body).await;
    let (result, used, elapsed) = (routed.result, routed.routing, ms(routed.elapsed));
    if result.is_ok() {
        *routing = used;
    }
    let base = Call {
        data_collection: if used.deny_data_collection { "deny" } else { "allow" },
        require_parameters: Some(used.require_parameters),
        attempts: routed.refusals.len() + 1,
        routing_errors: routed.refusals.iter().map(|(r, f)| format!("{}: {}", routing_name(*r), f.error)).collect(),
        ..Call::default()
    };
    match result {
        Ok(c) => {
            let parsed = openrouter::parse_content(&c.content, c.finish_reason.as_deref(), profile.structured);
            let parsed_kind = parsed_name(&parsed);
            let (output, outcome) = claude::decide(&p.raw, parsed, dict, s);
            let u = c.usage.unwrap_or_default();
            let (cost_usd, cost_source) = match (c.usage, u.cost) {
                (_, Some(cost)) => (cost, "openrouter"),
                (Some(u), None) => info.estimate_cost(&u).map_or((0.0, "none"), |cost| (cost, "estimate")),
                (None, None) => (0.0, "none"),
            };
            let total_ms = ms(c.total);
            Call {
                ttft_ms: c.ttft.map(ms),
                total_ms,
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                cached_tokens: u.cached_tokens,
                reasoning_tokens: u.reasoning_tokens,
                cost_usd,
                cost_source,
                finish_reason: c.finish_reason,
                parsed: parsed_kind,
                outcome: outcome_name(outcome),
                over_timeout: total_ms > timeout_ms(&p.raw),
                output,
                content: Some(c.content),
                provider: c.provider,
                served_model: c.model,
                ..base
            }
        }
        Err(f) => Call {
            total_ms: elapsed,
            cost_source: "none",
            parsed: "error",
            outcome: if f.error.is_key_problem() { "raw_key_problem" } else { "raw_fallback" },
            over_timeout: elapsed > timeout_ms(&p.raw),
            output: p.raw.clone(),
            error_kind: Some(error_kind(&f)),
            error: Some(f.error.to_string()),
            ..base
        },
    }
}

/// Median; for an even count, the mean of the two middle values.
fn median(values: &[u64]) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    let n = v.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(v[n / 2] as f64),
        _ => Some((v[n / 2 - 1] + v[n / 2]) as f64 / 2.0),
    }
}

/// 90th percentile by nearest rank: the value that 90 % of the sample does not exceed.
fn p90(values: &[u64]) -> Option<u64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    let rank = (9 * v.len()).div_ceil(10);
    rank.checked_sub(1).map(|i| v[i])
}

#[derive(Debug, PartialEq, Serialize)]
struct Summary {
    model: String,
    calls: usize,
    warm_calls: usize,
    /// Warm calls without an error.
    total_ms_median: Option<f64>,
    total_ms_p90: Option<u64>,
    ttft_ms_median: Option<f64>,
    ttft_ms_p90: Option<u64>,
    cold_total_ms: Option<u64>,
    cold_ttft_ms: Option<u64>,
    /// Share of warm calls longer than the app's timeout (3 s).
    share_over_timeout: Option<f64>,
    refined: usize,
    /// Everything that is not `refined`: a fallback to the raw text, including because of errors.
    fallbacks: usize,
    errors: usize,
    cost_usd: f64,
    cost_per_dictation_usd: Option<f64>,
    /// Routing conditions at the end of the run: `deny+require`, `allow+require`, `allow`; for the direct path, `n/a`.
    routing: String,
    providers: BTreeMap<String, usize>,
}

fn summarize(model: &str, calls: &[Call], routing: &str) -> Summary {
    let mine: Vec<&Call> = calls.iter().filter(|c| c.model == model).collect();
    let warm: Vec<&Call> = mine.iter().copied().filter(|c| !c.cold).collect();
    let ok_warm: Vec<&Call> = warm.iter().copied().filter(|c| c.error_kind.is_none()).collect();
    let totals: Vec<u64> = ok_warm.iter().map(|c| c.total_ms).collect();
    let ttfts: Vec<u64> = ok_warm.iter().filter_map(|c| c.ttft_ms).collect();
    let cold = mine.iter().find(|c| c.cold);
    let cost_usd: f64 = mine.iter().map(|c| c.cost_usd).sum();
    let refined = mine.iter().filter(|c| c.outcome == "refined").count();
    let mut providers = BTreeMap::new();
    for p in mine.iter().filter_map(|c| c.provider.clone()) {
        *providers.entry(p).or_insert(0) += 1;
    }
    Summary {
        model: model.to_owned(),
        calls: mine.len(),
        warm_calls: warm.len(),
        total_ms_median: median(&totals),
        total_ms_p90: p90(&totals),
        ttft_ms_median: median(&ttfts),
        ttft_ms_p90: p90(&ttfts),
        cold_total_ms: cold.map(|c| c.total_ms),
        cold_ttft_ms: cold.and_then(|c| c.ttft_ms),
        share_over_timeout: (!warm.is_empty())
            .then(|| warm.iter().filter(|c| c.over_timeout).count() as f64 / warm.len() as f64),
        refined,
        fallbacks: mine.len() - refined,
        errors: mine.iter().filter(|c| c.error_kind.is_some()).count(),
        cost_usd,
        cost_per_dictation_usd: (!mine.is_empty()).then(|| cost_usd / mine.len() as f64),
        routing: routing.to_owned(),
        providers,
    }
}

/// The model's routing conditions at the end of the run; for the direct path, `n/a`.
fn lane_routing(lane: &Lane) -> String {
    match lane.engine {
        Engine::Direct => "n/a".into(),
        Engine::Router { .. } => routing_name(lane.routing),
    }
}

/// What each model is sent — for `out.json`.
fn lanes_meta(lanes: &[Lane]) -> Vec<Value> {
    lanes
        .iter()
        .map(|l| match &l.engine {
            Engine::Direct => json!({ "id": l.id, "engine": "anthropic-direct" }),
            Engine::Router { info, profile } => json!({
                "id": l.id,
                "engine": "openrouter",
                "structured_output": profile.structured,
                "reasoning": profile.reasoning.param(),
                "reasoning_mandatory": profile.reasoning.mandatory(),
                "cache_mark": profile.cache_mark,
                "pricing_per_token": { "prompt": info.pricing.prompt, "completion": info.pricing.completion,
                                       "input_cache_read": info.pricing.input_cache_read },
            }),
        })
        .collect()
}

struct Report<'a> {
    args: &'a Args,
    lanes: &'a [Lane],
    calls: &'a [Call],
    skipped: &'a [Skipped],
    planned: usize,
    spent: f64,
    stopped_by_budget: bool,
}

impl Report<'_> {
    fn summaries(&self) -> Vec<Summary> {
        self.lanes.iter().map(|l| summarize(&l.id, self.calls, &lane_routing(l))).collect()
    }

    fn write(&self) -> anyhow::Result<()> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let v = json!({
            "generated_at": cue_core::usage::utc_timestamp(now),
            "phrases_file": self.args.phrases.display().to_string(),
            "runs": self.args.runs,
            "max_usd": self.args.max_usd,
            "terms": self.args.terms,
            "router_timeout_ms": ms(ROUTER_TIMEOUT),
            "planned_calls": self.planned,
            "done_calls": self.calls.len(),
            "spent_usd": self.spent,
            "stopped_by_budget": self.stopped_by_budget,
            "models": lanes_meta(self.lanes),
            "summary": self.summaries(),
            "skipped": self.skipped,
            "calls": self.calls,
        });
        let path = &self.args.out;
        std::fs::write(path, serde_json::to_vec_pretty(&v)?).with_context(|| format!("could not write {}", path.display()))
    }
}

fn opt_ms(v: Option<u64>) -> String {
    v.map_or("—".into(), |v| v.to_string())
}

fn opt_f(v: Option<f64>, digits: usize) -> String {
    v.map_or("—".into(), |v| format!("{v:.digits$}"))
}

/// A call's console line: only numbers and ids.
fn call_line(c: &Call, planned: usize) -> String {
    let mut line = format!(
        "{:>4}/{planned} {} r{} {:<30} ttft {:>5} ms, total {:>5} ms, {:<15} ${:.6}",
        c.seq,
        c.phrase,
        c.run,
        c.model,
        opt_ms(c.ttft_ms),
        c.total_ms,
        c.outcome,
        c.cost_usd
    );
    if let Some(to) = c.to {
        line.push_str(&format!(" to {to}"));
    }
    if c.cold {
        line.push_str(" cold");
    }
    if let Some(kind) = &c.error_kind {
        line.push_str(&format!(" error {kind}"));
    }
    if c.attempts > 1 {
        let require = if c.require_parameters == Some(true) { "+require" } else { "" };
        line.push_str(&format!(" (retries {}, routing {}{require})", c.attempts - 1, c.data_collection));
    }
    line
}

fn print_summary(summaries: &[Summary]) {
    println!(
        "\n{:<30} {:>5} {:>7} {:>6} {:>8} {:>8} {:>7} {:>6} {:>6} {:>6} {:>9} {:>10}",
        "model", "warm", "median", "p90", "ttft med", "ttft p90", "cold", ">3 s", "raw", "errors", "$ total", "$/dictation"
    );
    for s in summaries {
        println!(
            "{:<30} {:>5} {:>7} {:>6} {:>8} {:>8} {:>7} {:>6} {:>6} {:>6} {:>9.5} {:>10}",
            s.model,
            s.warm_calls,
            opt_f(s.total_ms_median, 0),
            opt_ms(s.total_ms_p90),
            opt_f(s.ttft_ms_median, 0),
            opt_ms(s.ttft_ms_p90),
            opt_ms(s.cold_total_ms),
            s.share_over_timeout.map_or("—".into(), |v| format!("{:.0}%", v * 100.0)),
            s.fallbacks,
            s.errors,
            s.cost_usd,
            opt_f(s.cost_per_dictation_usd, 6)
        );
    }
}

/// The key from the store read result (`keys::lookup`); a missing key gives "no <name> key" and a hint on how to save it.
/// Neither the key nor its mask appears in the text.
fn need_key(name: KeyName, read: Result<Option<String>, EngineError>) -> anyhow::Result<String> {
    match read {
        Ok(Some(key)) => Ok(key),
        Ok(None) => bail!("no {0} key\nSave the key: cargo run -p cue-cli -- keys set {0}", name.as_str()),
        Err(e) => bail!("{e}"),
    }
}

fn key_or_hint(name: KeyName) -> anyhow::Result<String> {
    need_key(name, keys::lookup(name))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = parse_args(&std::env::args().skip(1).collect::<Vec<_>>())?;
    let phrases = read_phrases(&args.phrases)?;
    let needs_router = args.models.iter().any(|m| m != DIRECT);
    let router_key = if needs_router { Some(key_or_hint(KeyName::OpenRouter)?) } else { None };
    let direct_key = if args.models.iter().any(|m| m == DIRECT) { Some(key_or_hint(KeyName::Anthropic)?) } else { None };

    // One client for everything, as in the app: connections are reused, only the first call warms up.
    let http = reqwest::Client::new();
    let listed = if needs_router {
        openrouter::models(&http).await.map_err(|e| anyhow!("OpenRouter model list: {e}"))?
    } else {
        Vec::new()
    };
    let mut lanes = Vec::new();
    for id in &args.models {
        let engine = if id == DIRECT {
            Engine::Direct
        } else {
            let info = listed.iter().find(|m| &m.id == id).cloned().with_context(|| format!("model {id} is not in the OpenRouter list"))?;
            Engine::Router { profile: info.profile(), info: Box::new(info) }
        };
        lanes.push(Lane { id: id.clone(), engine, routing: Routing::STRICT, calls: 0 });
    }

    let planned = args.runs * phrases.iter().map(|p| lanes.iter().filter(|l| l.skips(p).is_none()).count()).sum::<usize>();
    let translations = phrases.iter().filter(|p| p.to.is_some()).count();
    println!(
        "phrases {} ({translations} to translate), models {}, runs {} — up to {planned} calls; budget ${:.2}",
        phrases.len(),
        lanes.len(),
        args.runs,
        args.max_usd
    );
    let dict = DictionarySettings { terms: args.terms.clone(), ..Default::default() };
    if !args.terms.is_empty() {
        println!("dictionary: {} entries", args.terms.len());
    }
    for l in &lanes {
        match &l.engine {
            Engine::Direct => println!("  {:<30} Anthropic direct (claude::refine, claude::translate)", l.id),
            Engine::Router { profile, .. } => println!(
                "  {:<30} schema {}, reasoning {}, cache mark {}",
                l.id,
                if profile.structured { "yes" } else { "no" },
                profile.reasoning.param().map_or("—".into(), |v| v.to_string()),
                if profile.cache_mark { "yes" } else { "no" }
            ),
        }
    }
    if args.dry_run {
        println!("--dry-run: no paid requests");
        return Ok(());
    }

    let s = RefineSettings::default();
    let (mut calls, mut spent, mut stopped_by_budget) = (Vec::new(), 0.0, false);
    let mut skipped = Vec::new();
    // An empty report right away: the output path is checked before the first paid request.
    Report { args: &args, lanes: &lanes, calls: &calls, skipped: &skipped, planned, spent, stopped_by_budget }.write()?;
    'phrases: for p in &phrases {
        let ctx = RefineContext::new(p.app.as_str(), p.before_cursor.as_deref(), s.context_chars);
        for lane in &lanes {
            if let Some(reason) = lane.skips(p) {
                println!("          {} {:<30} skipped: {reason}", p.id, lane.id);
                skipped.push(Skipped { model: lane.id.clone(), phrase: p.id.clone(), reason });
            }
        }
        for run in 1..=args.runs {
            for lane in lanes.iter_mut() {
                if lane.skips(p).is_some() {
                    continue;
                }
                let cold = lane.calls == 0;
                lane.calls += 1;
                let mut call = match &lane.engine {
                    Engine::Direct => direct_call(&http, direct_key.as_deref().unwrap_or_default(), p, &ctx, &dict, &s).await,
                    Engine::Router { info, profile } => {
                        let key = router_key.as_deref().unwrap_or_default();
                        router_call(&http, key, &lane.id, info, profile, &mut lane.routing, p, &ctx, &dict, &s).await
                    }
                };
                call.seq = calls.len() + 1;
                call.model = lane.id.clone();
                call.phrase = p.id.clone();
                call.to = p.to;
                call.run = run;
                call.cold = cold;
                spent += call.cost_usd;
                println!("{}", call_line(&call, planned));
                calls.push(call);
                if spent > args.max_usd {
                    stopped_by_budget = true;
                    println!("spent ${spent:.4} — over the ${:.2} budget, stopping", args.max_usd);
                    break 'phrases;
                }
            }
        }
        Report { args: &args, lanes: &lanes, calls: &calls, skipped: &skipped, planned, spent, stopped_by_budget }.write()?;
    }
    let report = Report { args: &args, lanes: &lanes, calls: &calls, skipped: &skipped, planned, spent, stopped_by_budget };
    report.write()?;
    print_summary(&report.summaries());
    println!("\n{} of {planned} calls, spent ${spent:.4}; report: {}", calls.len(), args.out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> anyhow::Result<Args> {
        parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn args_defaults() {
        let a = args(&["p.json", "o.json"]).unwrap();
        assert_eq!((a.phrases, a.out), (PathBuf::from("p.json"), PathBuf::from("o.json")));
        assert_eq!(a.models, DEFAULT_MODELS.map(str::to_owned).to_vec());
        assert_eq!((a.runs, a.max_usd, a.dry_run), (3, 1.0, false));
        assert!(a.terms.is_empty(), "no dictionary by default");
        assert_eq!(DEFAULT_MODELS.len(), 10);
        assert_eq!(DIRECT, RefineSettings::default().model().id, "the direct path is the app's model");
    }

    #[test]
    fn args_flags() {
        let a = args(&["--runs", "5", "p.json", "--models", " openai/gpt-6-luna, claude-haiku-5-5,openai/gpt-6-luna,", "o.json", "--max-usd", "0.25", "--dry-run"]).unwrap();
        assert_eq!(a.models, ["openai/gpt-6-luna", "claude-haiku-5-5"], "spaces and repeats are removed");
        assert_eq!((a.runs, a.max_usd, a.dry_run), (5, 0.25, true));
        let a = args(&["p.json", "o.json", "--terms", " Soniox, Claude Haiku,,GitHub "]).unwrap();
        assert_eq!(a.terms, ["Soniox", "Claude Haiku", "GitHub"], "spaces around entries and empty entries are removed");
    }

    #[test]
    fn args_errors() {
        for bad in [
            &["p.json"][..],
            &["p.json", "o.json", "x.json"],
            &["p.json", "o.json", "--runs", "0"],
            &["p.json", "o.json", "--runs"],
            &["p.json", "o.json", "--max-usd", "-1"],
            &["p.json", "o.json", "--max-usd", "NaN"],
            &["p.json", "o.json", "--models", " , "],
            &["p.json", "o.json", "--terms"],
            &["p.json", "o.json", "--terms", " , "],
            &["p.json", "o.json", "--fast"],
        ] {
            assert!(args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn missing_key_message() {
        let e = need_key(KeyName::OpenRouter, Ok(None)).unwrap_err().to_string();
        assert_eq!(e, "no openrouter key\nSave the key: cargo run -p cue-cli -- keys set openrouter");
        let e = need_key(KeyName::Anthropic, Err(EngineError::Service("could not read the anthropic key: boom".into())));
        assert!(e.unwrap_err().to_string().contains("boom"));
        assert_eq!(need_key(KeyName::OpenRouter, Ok(Some("k".into()))).unwrap(), "k");
    }

    #[test]
    fn median_and_p90() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5]), Some(5.0));
        assert_eq!(median(&[9, 1, 5]), Some(5.0));
        assert_eq!(median(&[4, 1, 3, 2]), Some(2.5));
        assert_eq!(p90(&[]), None);
        assert_eq!(p90(&[7]), Some(7));
        assert_eq!(p90(&(1..=10).rev().collect::<Vec<_>>()), Some(9));
        assert_eq!(p90(&(1..=20).collect::<Vec<_>>()), Some(18));
        assert_eq!(p90(&(1..=11).collect::<Vec<_>>()), Some(10), "rank ⌈0.9 × 11⌉ = 10");
    }

    fn call(model: &str, cold: bool, total_ms: u64, ttft_ms: Option<u64>, outcome: &'static str, error: bool, cost: f64) -> Call {
        Call {
            model: model.into(),
            cold,
            total_ms,
            ttft_ms,
            outcome,
            over_timeout: total_ms > 3_000,
            error_kind: error.then(|| "http_503".into()),
            cost_usd: cost,
            provider: (!error).then(|| "OpenAI".into()),
            ..Call::default()
        }
    }

    #[test]
    fn summary_separates_cold_and_skips_errors_in_latency() {
        let calls = [
            call("a", true, 2_500, Some(900), "refined", false, 0.001),
            call("a", false, 800, Some(300), "refined", false, 0.001),
            call("b", true, 100, Some(50), "refined", false, 0.5),
            call("a", false, 3_200, Some(400), "raw_fallback", false, 0.001),
            call("a", false, 40, None, "raw_fallback", true, 0.0),
            call("a", false, 1_000, Some(350), "refined", false, 0.001),
        ];
        let s = summarize("a", &calls, "deny+require");
        assert_eq!((s.calls, s.warm_calls), (5, 4));
        assert_eq!(s.total_ms_median, Some(1_000.0), "warm calls without an error: 800, 1000, 3200");
        assert_eq!(s.total_ms_p90, Some(3_200));
        assert_eq!((s.ttft_ms_median, s.ttft_ms_p90), (Some(350.0), Some(400)));
        assert_eq!((s.cold_total_ms, s.cold_ttft_ms), (Some(2_500), Some(900)));
        assert_eq!(s.share_over_timeout, Some(0.25), "1 of 4 warm calls is over 3 s");
        assert_eq!((s.refined, s.fallbacks, s.errors), (3, 2, 1));
        assert!((s.cost_usd - 0.004).abs() < 1e-12 && (s.cost_per_dictation_usd.unwrap() - 0.0008).abs() < 1e-12);
        assert_eq!(s.providers, BTreeMap::from([("OpenAI".to_owned(), 4)]));
        assert_eq!(s.routing, "deny+require");

        let empty = summarize("c", &calls, "allow");
        assert_eq!((empty.calls, empty.total_ms_median, empty.share_over_timeout, empty.cost_per_dictation_usd), (0, None, None, None));
    }

    #[test]
    fn call_line_has_numbers_and_ids_only() {
        let c = Call {
            seq: 7,
            model: "openai/gpt-6-luna".into(),
            phrase: "chat-time-fix".into(),
            to: Some("en"),
            run: 2,
            ttft_ms: Some(312),
            total_ms: 640,
            outcome: "refined",
            cost_usd: 0.000123,
            output: "SECRET-OUTPUT".into(),
            content: Some("SECRET-CONTENT".into()),
            error: Some("SECRET-ERROR".into()),
            error_kind: Some("http_503".into()),
            attempts: 2,
            data_collection: "allow",
            require_parameters: Some(true),
            ..Call::default()
        };
        let line = call_line(&c, 600);
        assert!(line.contains("7/600") && line.contains("chat-time-fix") && line.contains("312") && line.contains("http_503"), "{line}");
        assert!(line.contains("retries 1, routing allow+require") && line.contains(" to en"), "{line}");
        assert!(!line.contains("SECRET"), "neither text nor the error message: {line}");
    }

    #[test]
    fn phrases_file_is_validated() {
        let dir = std::env::temp_dir().join(format!("cue-refine-compare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let ok = write("ok.json", r#"[{"id":"a","app":"Slack","raw":"ну привет"},{"id":"b","app":"X","raw":"да","before_cursor":"Итак, "}]"#);
        let got = read_phrases(&ok).unwrap();
        assert_eq!((got.len(), got[1].before_cursor.as_deref()), (2, Some("Итак, ")));
        for (name, body) in [
            ("empty.json", "[]"),
            ("dup.json", r#"[{"id":"a","app":"X","raw":"x"},{"id":"a","app":"X","raw":"y"}]"#),
            ("blank.json", r#"[{"id":"a","app":"X","raw":"  "}]"#),
        ] {
            assert!(read_phrases(&write(name, body)).is_err(), "{name}");
        }
        let broken = read_phrases(&write("broken.json", r#"[{"id":"a","app":"X","raw":SECRET}]"#)).unwrap_err().to_string();
        assert!(broken.contains("line 1") && !broken.contains("SECRET"), "{broken}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn phrase_target_language_is_normalized_or_refused() {
        let dir = std::env::temp_dir().join(format!("cue-refine-compare-to-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.json");
        std::fs::write(&ok, r#"[{"id":"a","app":"X","raw":"привет","to":"EN"},{"id":"b","app":"X","raw":"hello"}]"#).unwrap();
        let got = read_phrases(&ok).unwrap();
        assert_eq!((got[0].to, got[1].to), (Some("en"), None), "the code is normalized; no `to` means a cleanup");
        let bad = dir.join("bad.json");
        std::fs::write(&bad, r#"[{"id":"a","app":"X","raw":"x"},{"id":"q7","app":"X","raw":"y","to":"xx"}]"#).unwrap();
        let e = read_phrases(&bad).unwrap_err().to_string();
        assert!(e.contains("q7"), "the error names the phrase: {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn translations_run_only_on_the_direct_path() {
        let phrase = |to| Phrase { id: "a".into(), app: "X".into(), raw: "x".into(), before_cursor: None, to_code: None, to };
        assert_eq!(skip_reason(true, &phrase(Some("en"))), None);
        assert_eq!(skip_reason(false, &phrase(Some("en"))), Some("translation is direct-only"));
        assert_eq!((skip_reason(true, &phrase(None)), skip_reason(false, &phrase(None))), (None, None), "a cleanup runs everywhere");
    }
}
