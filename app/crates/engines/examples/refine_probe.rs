//! Refine probe: one real Claude Haiku 5.5 refine (a fraction of a cent). Prints the outcome, latency, text and tokens.
//! Run: cargo run -p cue-engines --example refine_probe -- "ну э короче созвон в среду нет в четверг в три часа"
//! The second argument (optional) is the app the text is refined for: `-- "text" "Outlook"`;
//! without it, `Telegram Desktop`.
//! The key comes from Credential Manager (`cue-cli keys set anthropic`).
use anyhow::{anyhow, Context};
use cue_core::refine::RefineContext;
use cue_core::settings::RefineSettings;
use cue_engines::keys::{self, KeyName};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let raw = args.next().context("pass the text to refine")?;
    let app = args.next().unwrap_or_else(|| "Telegram Desktop".to_owned());
    let key = keys::get(KeyName::Anthropic)
        .map_err(|e| anyhow!("{e}\nSave the key: cargo run -p cue-cli -- keys set anthropic"))?;
    let s = RefineSettings::default();
    let ctx = RefineContext::new(app.as_str(), None, s.context_chars);
    let http = reqwest::Client::new();
    let t0 = std::time::Instant::now();
    let r = cue_engines::claude::refine(&http, Some(&key), "claude-haiku-5-5", &raw, &ctx, &s).await;
    println!("{app} — {:?} in {} ms: {}", r.outcome, t0.elapsed().as_millis(), r.text);
    let u = r.usage;
    println!(
        "tokens: input {}, cache read {}, cache write {}, output {}",
        u.input_tokens, u.cache_read_input_tokens, u.cache_creation_input_tokens, u.output_tokens
    );
    Ok(())
}
