//! The "Ask" bench (spec 2 §11): questions with their context, each asked the way the app asks it, to measure the decision
//! (answer or write text for the field), the first words and the cost. Paid.
//!
//! Run:
//! cargo run -p cue-engines --example ask_compare -- phrases.json out.json [--runs 5] [--max-usd 1.00]
//!   [--model claude-haiku-5-5] [--no-search] [--thinking adaptive|off]
//!
//! - `phrases.json` is an array of `{id, question, app, selection?, before_cursor?, then?, expect, mode?}`. `then` is a
//!   follow-up asked after the first answer, and the follow-up is what is scored; `expect` is `answer` or `write`, `mode`
//!   (`replace` / `insert`) the write's mode when it matters.
//! - The request is the app's: `cue_core::ask::first_message` with today's date (UTC here: a day off only around midnight,
//!   which measures the same), `cue_engines::ask::run` with its deadlines and continuations, the web search of
//!   `ask.max_searches` = 2 (none with `--no-search`), thinking as `--thinking` says (the app's default when left out).
//! - Runs 1..runs, each over all phrases, one call at a time. The very first call is cold (the prompt cache is empty) and
//!   is left out of the medians; first words are split by whether the model searched.
//! - Stops as soon as more than `--max-usd` has been spent.
//!
//! The key comes from Credential Manager (`cue-cli keys set anthropic`). The console gets ids, kinds and numbers only;
//! questions and answers go only to `out.json`.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context};
use cue_core::ask::{first_message, follow_up_message, AskContext};
use cue_core::settings::Pricing;
use cue_core::usage::{refine_cost, utc_timestamp};
use cue_engines::ask::{self, AskEnd, AskResult, SearchTool, Thinking};
use cue_engines::keys::{self, KeyName};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const USAGE: &str =
    "ask_compare <phrases.json> <out.json> [--runs N] [--max-usd X] [--model M] [--no-search] [--thinking adaptive|off]";
/// `ask.max_searches` by default.
const MAX_SEARCHES: u32 = 2;

#[derive(Debug, PartialEq)]
struct Args {
    phrases: PathBuf,
    out: PathBuf,
    runs: usize,
    max_usd: f64,
    model: String,
    search: bool,
    /// The bench's arm (decision 8).
    thinking: Thinking,
}

fn parse_args(args: &[String]) -> anyhow::Result<Args> {
    let mut positional = Vec::new();
    let (mut runs, mut max_usd, mut model, mut search) = (5, 1.0, "claude-haiku-5-5".to_owned(), true);
    let mut thinking = Thinking::DEFAULT;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().with_context(|| format!("{a}: a value is missing"));
        match a.as_str() {
            "--runs" => runs = value()?.parse::<usize>().ok().filter(|n| *n > 0).context("--runs: a whole number above 0")?,
            "--max-usd" => {
                max_usd = value()?.parse::<f64>().ok().filter(|x| x.is_finite() && *x > 0.0).context("--max-usd: a number above 0")?
            }
            "--model" => model = value()?.clone(),
            "--no-search" => search = false,
            "--thinking" => {
                thinking = match value()?.as_str() {
                    "adaptive" => Thinking::Adaptive,
                    "off" => Thinking::Off,
                    other => bail!("--thinking: adaptive or off, not {other}"),
                }
            }
            flag if flag.starts_with("--") => bail!("unknown flag {flag}\n{USAGE}"),
            _ => positional.push(PathBuf::from(a)),
        }
    }
    let [phrases, out] = <[PathBuf; 2]>::try_from(positional).map_err(|_| anyhow!(USAGE))?;
    Ok(Args { phrases, out, runs, max_usd, model, search, thinking })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Expect {
    Answer,
    Write,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum WriteMode {
    Replace,
    Insert,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Phrase {
    id: String,
    question: String,
    app: String,
    #[serde(default)]
    selection: Option<String>,
    #[serde(default)]
    before_cursor: Option<String>,
    /// A follow-up asked after the first answer: the follow-up is scored.
    #[serde(default)]
    then: Option<String>,
    expect: Expect,
    #[serde(default)]
    mode: Option<WriteMode>,
}

fn read_phrases(path: &Path) -> anyhow::Result<Vec<Phrase>> {
    let text = std::fs::read_to_string(path).with_context(|| path.display().to_string())?;
    let phrases: Vec<Phrase> = serde_json::from_str(&text).context("phrases.json")?;
    let mut seen = HashSet::new();
    for p in &phrases {
        if !seen.insert(p.id.as_str()) {
            bail!("phrase id {} is there twice", p.id);
        }
        if p.mode.is_some() && p.expect != Expect::Write {
            bail!("{}: a mode is only for expect \"write\"", p.id);
        }
    }
    Ok(phrases)
}

/// One scored question as `out.json` keeps it.
#[derive(Debug, Serialize)]
struct Call {
    id: String,
    run: usize,
    cold: bool,
    expect: Expect,
    expect_mode: Option<WriteMode>,
    /// `answer`, `write`, `refused` or `failed: <kind>`.
    decision: String,
    mode: Option<WriteMode>,
    /// The answer's Markdown or the written text (refused and failed: empty).
    text: String,
    first_words_ms: Option<u64>,
    total_ms: u64,
    searches: u32,
    /// The question and, for a follow-up phrase, the first answer too.
    cost_usd: f64,
}

struct Asked {
    result: AskResult,
    first_words_ms: Option<u64>,
    total_ms: u64,
}

/// One request as the app sends it; the first words are the first non-blank text or the start of a write.
async fn ask_once(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    search: Option<SearchTool>,
    thinking: Thinking,
    messages: &[Value],
    container: Option<String>,
) -> Asked {
    let t0 = Instant::now();
    let mut first = None;
    let result = ask::run(http, Some(key), model, search, thinking, messages, container, |s| {
        let words = s.is_first_words();
        if words && first.is_none() {
            first = Some(t0.elapsed().as_millis() as u64);
        }
    })
    .await;
    Asked { result, first_words_ms: first, total_ms: t0.elapsed().as_millis() as u64 }
}

fn cost(p: &Pricing, model: &str, r: &AskResult) -> f64 {
    let u = r.usage;
    refine_cost(p, model, u.input_tokens, u.cache_read_tokens, u.cache_write_tokens, u.output_tokens)
        + f64::from(u.web_searches) * p.web_search_usd
}

fn scored(p: &Phrase, run: usize, cold: bool, asked: &Asked, cost_usd: f64) -> Call {
    let r = &asked.result;
    let (decision, mode, text) = match &r.end {
        AskEnd::Answer => {
            let text = r.content.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<String>();
            ("answer".to_owned(), None, text)
        }
        AskEnd::Write { text, replace } => {
            ("write".to_owned(), Some(if *replace { WriteMode::Replace } else { WriteMode::Insert }), text.clone())
        }
        AskEnd::Refused => ("refused".to_owned(), None, String::new()),
        AskEnd::Failed(f) => (format!("failed: {f:?}"), None, String::new()),
    };
    Call {
        id: p.id.clone(),
        run,
        cold,
        expect: p.expect,
        expect_mode: p.mode,
        decision,
        mode,
        text,
        first_words_ms: asked.first_words_ms,
        total_ms: asked.total_ms,
        searches: r.usage.web_searches,
        cost_usd,
    }
}

#[derive(Debug, Serialize)]
struct Summary {
    calls: usize,
    decisions_right: usize,
    modes_right: usize,
    modes_scored: usize,
    first_words_ms_no_search: Option<u64>,
    first_words_ms_search: Option<u64>,
    cost_usd: f64,
}

fn median(mut v: Vec<u64>) -> Option<u64> {
    v.sort_unstable();
    (!v.is_empty()).then(|| v[v.len() / 2])
}

fn summarize(calls: &[Call]) -> Summary {
    let right = |c: &&Call| match c.expect {
        Expect::Answer => c.decision == "answer",
        Expect::Write => c.decision == "write",
    };
    let warm = |searched: bool| {
        median(calls.iter().filter(|c| !c.cold && (c.searches > 0) == searched).filter_map(|c| c.first_words_ms).collect())
    };
    Summary {
        calls: calls.len(),
        decisions_right: calls.iter().filter(right).count(),
        modes_right: calls.iter().filter(|c| c.expect_mode.is_some() && c.mode == c.expect_mode).count(),
        modes_scored: calls.iter().filter(|c| c.expect_mode.is_some()).count(),
        first_words_ms_no_search: warm(false),
        first_words_ms_search: warm(true),
        cost_usd: calls.iter().map(|c| c.cost_usd).sum(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&argv)?;
    let phrases = read_phrases(&args.phrases)?;
    let key = keys::get(KeyName::Anthropic)?;
    let http = reqwest::Client::new();
    let search = if args.search { ask::search_tool(MAX_SEARCHES) } else { None };
    let today = utc_timestamp(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())[..10].to_owned();
    let pricing = Pricing::default();
    let (mut calls, mut spent, mut stopped) = (Vec::new(), 0.0, false);
    'runs: for run in 1..=args.runs {
        for p in &phrases {
            if spent > args.max_usd {
                stopped = true;
                break 'runs;
            }
            let ctx = AskContext {
                app: p.app.clone(),
                before_cursor: p.before_cursor.clone(),
                selection: p.selection.clone(),
                today: today.clone(),
            };
            let mut messages = vec![ask::user_turn(&first_message(&p.question, &ctx), None)];
            let mut asked = ask_once(&http, &key, &args.model, search, args.thinking, &messages, None).await;
            let mut usd = cost(&pricing, &args.model, &asked.result);
            if let Some(then) = &p.then
                && asked.result.end == AskEnd::Answer
            {
                let previous = asked.result.content.clone();
                let container = asked.result.container.clone();
                messages.push(ask::assistant_turn(&previous));
                messages.push(ask::user_turn(&follow_up_message(then), Some(&previous)));
                asked = ask_once(&http, &key, &args.model, search, args.thinking, &messages, container).await;
                usd += cost(&pricing, &args.model, &asked.result);
            }
            spent += usd;
            let c = scored(p, run, calls.is_empty(), &asked, usd);
            println!(
                "{} run {run}: {}{} · first words {:?} ms · searches {} · ${usd:.4}",
                c.id,
                c.decision,
                c.mode.map(|m| format!(" ({m:?})")).unwrap_or_default(),
                c.first_words_ms,
                c.searches
            );
            calls.push(c);
        }
    }
    let summary = summarize(&calls);
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if stopped {
        println!("stopped: more than ${} spent", args.max_usd);
    }
    let thinking = format!("{:?}", args.thinking).to_lowercase();
    let out = json!({
        "model": args.model, "runs": args.runs, "search": args.search, "thinking": thinking, "stopped": stopped,
        "calls": calls, "summary": summary
    });
    std::fs::write(&args.out, serde_json::to_string_pretty(&out)?).with_context(|| args.out.display().to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| (*x).to_owned()).collect()
    }

    #[test]
    fn arguments_with_defaults_and_flags() {
        let a = parse_args(&s(&["p.json", "o.json"])).unwrap();
        assert_eq!(
            a,
            Args {
                phrases: "p.json".into(),
                out: "o.json".into(),
                runs: 5,
                max_usd: 1.0,
                model: "claude-haiku-5-5".into(),
                search: true,
                thinking: Thinking::DEFAULT,
            }
        );
        let a = parse_args(&s(&["p.json", "o.json", "--runs", "2", "--max-usd", "0.4", "--model", "m", "--no-search", "--thinking", "off"]))
            .unwrap();
        assert_eq!((a.runs, a.max_usd, a.model.as_str(), a.search, a.thinking), (2, 0.4, "m", false, Thinking::Off));
        assert_eq!(parse_args(&s(&["p.json", "o.json", "--thinking", "adaptive"])).unwrap().thinking, Thinking::Adaptive);
        assert!(parse_args(&s(&["p.json"])).is_err());
        assert!(parse_args(&s(&["p.json", "o.json", "--runs", "0"])).is_err());
        assert!(parse_args(&s(&["p.json", "o.json", "--fast"])).is_err());
        assert!(parse_args(&s(&["p.json", "o.json", "--thinking", "deep"])).is_err());
    }

    #[test]
    fn phrases_are_read_and_checked() {
        let dir = std::env::temp_dir().join(format!("cue-ask-compare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.json");
        std::fs::write(
            &path,
            r#"[{"id":"a","question":"q","app":"Notepad","expect":"answer"},
                {"id":"b","question":"q","app":"Notepad","selection":"x","expect":"write","mode":"replace"},
                {"id":"c","question":"q","app":"Notepad","then":"and?","expect":"answer"}]"#,
        )
        .unwrap();
        let p = read_phrases(&path).unwrap();
        assert_eq!((p.len(), p[1].mode, p[2].then.as_deref()), (3, Some(WriteMode::Replace), Some("and?")));
        std::fs::write(&path, r#"[{"id":"a","question":"q","app":"N","expect":"answer"},{"id":"a","question":"q","app":"N","expect":"answer"}]"#)
            .unwrap();
        assert!(read_phrases(&path).is_err(), "an id twice");
        std::fs::write(&path, r#"[{"id":"a","question":"q","app":"N","expect":"answer","mode":"insert"}]"#).unwrap();
        assert!(read_phrases(&path).is_err(), "a mode on an answer");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn call(expect: Expect, decision: &str, mode: Option<WriteMode>, want: Option<WriteMode>, first: Option<u64>, searches: u32, cold: bool) -> Call {
        Call {
            id: "x".into(),
            run: 1,
            cold,
            expect,
            expect_mode: want,
            decision: decision.into(),
            mode,
            text: String::new(),
            first_words_ms: first,
            total_ms: 0,
            searches,
            cost_usd: 0.25,
        }
    }

    #[test]
    fn the_summary_scores_decisions_modes_and_warm_first_words() {
        use Expect::*;
        use WriteMode::*;
        let calls = [
            call(Answer, "answer", None, None, Some(9_000), 0, true),
            call(Answer, "answer", None, None, Some(1_000), 0, false),
            call(Answer, "answer", None, None, Some(2_000), 0, false),
            call(Answer, "write", Some(Insert), None, Some(1_500), 0, false),
            call(Write, "write", Some(Replace), Some(Replace), Some(1_200), 0, false),
            call(Write, "write", Some(Insert), Some(Replace), Some(1_300), 0, false),
            call(Answer, "answer", None, None, Some(6_000), 2, false),
            call(Answer, "failed: Timeout", None, None, None, 1, false),
        ];
        let s = summarize(&calls);
        assert_eq!((s.calls, s.decisions_right, s.modes_right, s.modes_scored), (8, 6, 1, 2));
        assert_eq!(s.first_words_ms_no_search, Some(1_300), "the cold call is left out");
        assert_eq!(s.first_words_ms_search, Some(6_000));
        assert!((s.cost_usd - 2.0).abs() < 1e-9);
    }

    #[test]
    fn median_takes_the_upper_middle() {
        assert_eq!(median(vec![]), None);
        assert_eq!(median(vec![3, 1, 2]), Some(2));
        assert_eq!(median(vec![4, 1, 3, 2]), Some(3));
    }
}
