//! Cache warm-up probe (spec 2 §6.3): one real warm-up, then a dictation refine and a translation of the same phrase
//! with Claude Haiku 5.5 (a fraction of a cent). The refine and the translation must read the cache the warm-up wrote.
//! Prints only times, token counts and outcomes: no text.
//! Run: cargo run -p cue-engines --example warm_probe
//! The key comes from Credential Manager (`cue-cli keys set anthropic`).
use std::time::Instant;

use anyhow::{anyhow, bail};
use cue_core::refine::RefineContext;
use cue_core::settings::RefineSettings;
use cue_engines::claude::{self, Usage};
use cue_engines::keys::{self, KeyName};

const MODEL: &str = "claude-haiku-5-5";
const PHRASE: &str = "ну привет как дела";

fn tokens(u: &Usage) -> String {
    format!(
        "cache write {}, cache read {}, input {}, output {}",
        u.cache_creation_input_tokens, u.cache_read_input_tokens, u.input_tokens, u.output_tokens
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let key = keys::get(KeyName::Anthropic)
        .map_err(|e| anyhow!("{e}\nSave the key: cargo run -p cue-cli -- keys set anthropic"))?;
    let s = RefineSettings::default();
    let http = reqwest::Client::new();

    let t0 = Instant::now();
    let Some(warm) = claude::warm(&http, &key, MODEL, &Default::default()).await else {
        bail!("the warm-up got no usable answer in {} ms", t0.elapsed().as_millis());
    };
    println!("warm-up in {} ms: {}", t0.elapsed().as_millis(), tokens(&warm));

    let ctx = RefineContext::new("Telegram Desktop", None, s.context_chars);
    let t0 = Instant::now();
    let r = claude::refine(&http, Some(&key), MODEL, PHRASE, &ctx, &Default::default(), &s).await;
    println!("refine {:?} in {} ms: {}", r.outcome, t0.elapsed().as_millis(), tokens(&r.usage));

    let t0 = Instant::now();
    let r = claude::translate(&http, Some(&key), MODEL, PHRASE, &ctx, &Default::default(), &s, "en").await;
    println!("translate to en {:?} in {} ms: {}", r.outcome, t0.elapsed().as_millis(), tokens(&r.usage));
    Ok(())
}
