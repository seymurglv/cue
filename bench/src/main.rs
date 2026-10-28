//! Bench: the user's recordings → Soniox stream and batch → refine models from settings → a report with blind pairs.
//!
//! cue-bench run [--limit N] [--max-usd X]    cue-bench report <run folder>
//!
//! - Recordings live in `%LOCALAPPDATA%\CUE\bench\recordings\` (`<time>-<id>.wav` + `.json` with `{"app": …}`); the app
//!   saves them when settings.json has `"bench": {"save_recordings": true}`. A run goes to `%LOCALAPPDATA%\CUE\bench\runs\<time>\`
//!   (`results.json` and `report.html`, written after every recording): dictation text and audio stay only there, outside the repository.
//!   Recordings are checked before the run: broken ones are skipped and do not count toward `--limit`.
//! - Transcription: `soniox` is the stream at speech pace (latency from the end of audio to the final result, like "finalize" in the app),
//!   `soniox-async` is a batch call on the same file (latency is the whole call); both run at once. At the end of audio, like the app
//!   on the second press, the connection to Anthropic is warmed up (HEAD without a key).
//! - Refine runs on the stream text (if the stream failed, on the batch text), each model in turn: the same request (with
//!   `dictionary.terms`), sanity check and timeout as in the app (`claude::refine`). Models come from settings.json.
//! - Before the first measurement there is a warm-up: one request per model; it is not in the report, but it is in the spend.
//! - Spending limit `--max-usd` (default $1): once reached, no new paid calls start; what was collected is saved.
//! - Ctrl+C means "finish what was sent and stop": the current recording's stream is interrupted, its batch call
//!   is awaited (dropped mid-upload, it could leave audio in Soniox under an id we never learn), refine for it
//!   does not run, it is not in the results, no new recordings start. Before exiting (end, limit, Ctrl+C, error)
//!   the bench waits for the recordings' cleanup in Soniox — the engine's whole retry schedule plus a 30 s margin ([`cleanup_wait`]).
//!   The next Ctrl+C exits at once, with a warning.
//! - The console gets numbers, file names, durations, latencies, outcomes, tokens and prices; stderr gets engine
//!   warnings (WARN). No dictation text and no keys there.
mod report;

use anyhow::{Context, Result, anyhow, bail};
use cue_core::refine::RefineContext;
use cue_core::settings::{Settings, SttEngineKind};
use cue_core::usage::{SttPath, refine_cost, stt_cost, utc_timestamp};
use cue_engines::EngineError;
use cue_engines::claude;
use cue_engines::keys::{self, KeyName};
use cue_engines::soniox::{self, SonioxConfig};
use cue_engines::stt::{self, CHUNK_SAMPLES, Engine, Fallback, SAMPLES_PER_MS, SttOutput};
use cue_platform_win::focus::UNKNOWN_APP;
use report::{Item, RefineRun, SttRun};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const USAGE: &str = "cue-bench run [--limit N] [--max-usd X] | cue-bench report <run folder>";
const DEFAULT_MAX_USD: f64 = 1.0;
/// Names of the transcription paths in the report.
const STREAM: &str = "soniox";
const BATCH: &str = "soniox-async";
/// How long we wait for the stream's final result after the end of audio — like the app (`STT_FINISH_TIMEOUT` in runtime.rs).
const STT_FINISH_TIMEOUT: Duration = Duration::from_secs(5);
/// Connection warm-up timeout — like the app's (`WARM_TIMEOUT` in runtime.rs).
const WARM_TIMEOUT: Duration = Duration::from_secs(5);
/// Engine log: warnings only. At trace level tungstenite writes the whole handshake — with
/// `Authorization: Bearer <key>`; the filter is fixed, an environment variable does not change it.
const LOG_FILTER: &str = "warn,tungstenite=off,tokio_tungstenite=off";
const NO_RECORDINGS: &str = "no recordings: set \"bench\": {\"save_recordings\": true} in %APPDATA%\\CUE\\settings.json, \
                             restart CUE and dictate a few times";

#[derive(Debug, PartialEq)]
enum Command {
    /// `limit: None` means all recordings.
    Run { limit: Option<usize>, max_usd: f64 },
    Report { dir: PathBuf },
}

fn parse_args(args: &[String]) -> Result<Command> {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "run" => {
            let (mut limit, mut max_usd) = (None, DEFAULT_MAX_USD);
            let mut it = rest.iter();
            while let Some(arg) = it.next() {
                let mut value = || it.next().with_context(|| format!("{arg} needs a value\n{USAGE}"));
                match arg.as_str() {
                    "--limit" => limit = Some(value()?.parse().ok().filter(|n: &usize| *n >= 1).context("--limit: an integer from 1")?),
                    "--max-usd" => {
                        max_usd = value()?.parse().ok().filter(|x: &f64| x.is_finite() && *x > 0.0).context("--max-usd: a number greater than 0")?
                    }
                    other => bail!("unknown argument {other}\n{USAGE}"),
                }
            }
            Ok(Command::Run { limit, max_usd })
        }
        Some((cmd, [dir])) if cmd == "report" && !dir.starts_with("--") => Ok(Command::Report { dir: PathBuf::from(dir) }),
        _ => bail!("{USAGE}"),
    }
}

/// Refine models: the model from settings first (it may have no price), then the priced models — in order, without repeats.
fn refine_models(s: &Settings) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for m in std::iter::once(&s.refine.model).chain(s.pricing.claude.iter().map(|p| &p.model)) {
        if !m.trim().is_empty() && !models.contains(m) {
            models.push(m.clone());
        }
    }
    models
}

/// The recording's app from `<recording>.json`: a real name as is; `null`, no field, no file or broken json —
/// `UNKNOWN_APP`, the same name the app would have passed.
fn app_name(json: Option<&str>) -> String {
    json.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
        .and_then(|v| v.get("app").and_then(serde_json::Value::as_str).map(str::to_owned))
        .filter(|a| !a.trim().is_empty() && a != UNKNOWN_APP)
        .unwrap_or_else(|| UNKNOWN_APP.to_owned())
}

struct Recording {
    app: String,
    pcm: Vec<i16>,
}

/// A `<recording>.wav` + `<recording>.json` pair. Audio as the app writes it: 16 kHz mono 16-bit; anything else or empty is an error.
fn read_recording(wav: &Path) -> Result<Recording> {
    let mut r = hound::WavReader::open(wav).context("could not open the audio")?;
    let spec = r.spec();
    if (spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format) != (16_000, 1, 16, hound::SampleFormat::Int) {
        bail!("audio is not 16 kHz mono 16-bit");
    }
    let pcm = r.samples::<i16>().collect::<Result<Vec<_>, _>>().context("could not read the audio")?;
    if pcm.is_empty() {
        bail!("empty recording");
    }
    let json = std::fs::read_to_string(wav.with_extension("json")).ok();
    Ok(Recording { app: app_name(json.as_deref()), pcm })
}

/// All recordings in name order (a name starts with the time — that is also the dictation order).
fn recordings(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!("{NO_RECORDINGS}"),
        Err(e) => return Err(e).with_context(|| format!("recordings folder {}", dir.display())),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav")))
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("{NO_RECORDINGS}");
    }
    Ok(files)
}

fn file_name(p: &Path) -> String {
    p.file_name().map_or_else(String::new, |f| f.to_string_lossy().into_owned())
}

/// Recordings that pass `check`, at most `limit`: broken ones are skipped (a console line) and do not count toward the limit.
/// The list is ready before the run, so "n of N" in the recording lines and in the summary agree; nothing past the limit is checked.
fn readable(files: &[PathBuf], limit: Option<usize>, mut check: impl FnMut(&Path) -> Result<()>) -> Vec<PathBuf> {
    files
        .iter()
        .filter(|f| match check(f) {
            Ok(()) => true,
            Err(e) => {
                println!("skipped {}: {e:#}", file_name(f));
                false
            }
        })
        .take(limit.unwrap_or(usize::MAX))
        .cloned()
        .collect()
}

/// Spending limit: a paid call starts only while the spend is below the limit. A call started before the limit
/// may go past it — by the price of one call (cents).
#[derive(Debug)]
struct Budget {
    max_usd: f64,
    spent: f64,
}

impl Budget {
    fn new(max_usd: f64) -> Self {
        Budget { max_usd, spent: 0.0 }
    }

    fn allows(&self) -> bool {
        self.spent < self.max_usd
    }

    fn add(&mut self, usd: f64) {
        self.spent += usd;
    }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Audio at speech pace: each 100 ms chunk is sent once it has "sounded" (from the start up to its last sample).
/// Returns right after the last chunk — that is the end of audio, the second press in the app.
async fn pace(pcm: &[i16], mut push: impl FnMut(Vec<i16>)) {
    let start = tokio::time::Instant::now();
    let mut sent = 0u64;
    for chunk in pcm.chunks(CHUNK_SAMPLES) {
        sent += chunk.len() as u64;
        tokio::time::sleep_until(start + Duration::from_millis(sent / SAMPLES_PER_MS)).await;
        push(chunk.to_vec());
    }
}

/// Soniox stream at speech pace. Latency is from the end of audio to the `finish` result.
async fn stream(engine: Engine, pcm: &[i16], http: &reqwest::Client) -> (Result<SttOutput, EngineError>, u64) {
    let session = stt::start(engine);
    pace(pcm, |chunk| session.push(chunk)).await;
    // The end of audio is the "second press": like the app, the connection to Anthropic warms up by the time the text arrives.
    drop(warm_at(http, claude::API_URL.to_owned()));
    let t0 = Instant::now();
    let r = session.finish(STT_FINISH_TIMEOUT).await;
    (r, ms(t0.elapsed()))
}

/// Warm-up, like the app's `warm_up`: HEAD to the API address without a key and without text — for a TLS connection
/// in the shared client's pool, so refine does not pay for the handshake after a long transcription. The response
/// (a refusal) is not needed; a failure is a log line.
fn warm_at(http: &reqwest::Client, url: String) -> tokio::task::JoinHandle<()> {
    let http = http.clone();
    tokio::spawn(async move {
        if let Err(e) = http.head(url).timeout(WARM_TIMEOUT).send().await {
            tracing::warn!("Anthropic connection warm-up: {}", if e.is_timeout() { "timeout" } else { "network" });
        }
    })
}

/// Soniox batch call on the recording file. Latency is the whole call (upload, transcription, polling, cleanup), like the batch fallback.
async fn batch(fallback: &Fallback, wav: &Path) -> (Result<SttOutput, EngineError>, u64) {
    let t0 = Instant::now();
    let r = fallback.transcribe(wav).await;
    (r, ms(t0.elapsed()))
}

/// A transcription result and its latency.
type Heard = (Result<SttOutput, EngineError>, u64);

/// A request to stop — the first Ctrl+C.
#[derive(Clone)]
struct Stop(tokio::sync::watch::Receiver<bool>);

impl Stop {
    fn new() -> (tokio::sync::watch::Sender<bool>, Stop) {
        let (tx, rx) = tokio::sync::watch::channel(false);
        (tx, Stop(rx))
    }

    fn requested(&self) -> bool {
        *self.0.borrow()
    }

    /// Wait for the request. If the sender is gone without sending it, the request will never come.
    async fn wait(&self) {
        if self.0.clone().wait_for(|stop| *stop).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The result of `fut` — or `None` if a stop request came first: then `fut` is dropped (and one not yet started is never started).
async fn unless_stopped<T>(fut: impl Future<Output = T>, stop: &Stop) -> Option<T> {
    if stop.requested() {
        return None;
    }
    tokio::select! {
        biased;
        v = fut => Some(v),
        () = stop.wait() => None,
    }
}

/// Transcription of a recording: stream and batch at once. A stop request interrupts only the stream — dropping it
/// leaves nothing in Soniox. The batch call is always awaited: dropped in the middle of `POST /files` or
/// `POST /transcriptions`, it could leave audio or a transcription in Soniox whose ids we would never learn — there
/// would be nothing to delete. The engine bounds its duration.
async fn stt_phase(stream: impl Future<Output = Heard>, batch: impl Future<Output = Heard>, stop: &Stop) -> (Heard, Heard) {
    let stream = async { unless_stopped(stream, stop).await.unwrap_or((Err(EngineError::Aborted), 0)) };
    tokio::join!(stream, batch)
}

/// Engine result → report row. An error goes through the engine's `Display` (it holds no dictation text); the text is then empty.
fn stt_run(engine: &str, (result, latency_ms): (Result<SttOutput, EngineError>, u64), cost_usd: f64) -> SttRun {
    let (text, error) = match result {
        Ok(o) => (o.text, None),
        Err(e) => (String::new(), Some(e.to_string())),
    };
    SttRun { engine: engine.into(), text, latency_ms, cost_usd, error }
}

/// Text for refine — as in the app: the stream text; if the stream failed, the batch text. The app does not refine
/// empty text (outcome "empty") — neither does the bench.
fn refine_input(stream: &SttRun, batch: &SttRun) -> Option<String> {
    let source = [stream, batch].into_iter().find(|r| r.error.is_none())?;
    (!source.text.trim().is_empty()).then(|| source.text.clone())
}

/// One refine request — the same as in the app (`claude::refine`: request with the dictionary from settings, check,
/// timeout). Latency is the whole call.
async fn refine_once(http: &reqwest::Client, key: &str, model: &str, raw: &str, ctx: &RefineContext, s: &Settings) -> RefineRun {
    let t0 = Instant::now();
    let r = claude::refine(http, Some(key), model, raw, ctx, &s.dictionary.terms, &s.refine).await;
    let latency_ms = ms(t0.elapsed());
    let u = r.usage;
    RefineRun {
        model: model.into(),
        text: r.text,
        outcome: format!("{:?}", r.outcome),
        latency_ms,
        cost_usd: refine_cost(&s.pricing, model, u.input_tokens, u.cache_read_input_tokens, u.cache_creation_input_tokens, u.output_tokens),
        input_tokens: u.input_tokens,
        cache_read_tokens: u.cache_read_input_tokens,
        cache_write_tokens: u.cache_creation_input_tokens,
        output_tokens: u.output_tokens,
    }
}

/// Refine for one recording: warm-up (if `warm_up`), then a measurement per model — in turn, while the limit allows.
/// `call(model)` is one request. Returns the measurements (warm-up is not among them) and `Some(End::Budget)` if the limit was hit.
async fn refine_phase<'a, Fut: Future<Output = RefineRun>>(
    models: &'a [String],
    warm_up: bool,
    budget: &mut Budget,
    mut call: impl FnMut(&'a str) -> Fut,
) -> (Vec<RefineRun>, Option<End>) {
    let warm: &[String] = if warm_up { models } else { &[] };
    let mut runs = Vec::new();
    for (model, warm) in warm.iter().map(|m| (m, true)).chain(models.iter().map(|m| (m, false))) {
        if !budget.allows() {
            return (runs, Some(End::Budget));
        }
        let r = call(model).await;
        budget.add(r.cost_usd);
        if warm {
            println!("{}", warm_line(&r));
        } else {
            println!("{}", refine_line(&r));
            runs.push(r);
        }
    }
    (runs, None)
}

/// How the run ended.
#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    Done,
    /// Reached the spending limit.
    Budget,
    /// Ctrl+C.
    Interrupted,
}

// Console lines: only numbers, names, outcomes — no dictation text.

fn item_line(n: usize, total: usize, name: &str, audio_ms: u64) -> String {
    format!("{n}/{total} · {name} · {:.1} s", audio_ms as f64 / 1000.0)
}

fn stt_line(r: &SttRun) -> String {
    match &r.error {
        None => format!("  {}: {} ms, ${:.5}", r.engine, r.latency_ms, r.cost_usd),
        Some(e) => format!("  {}: failed — {e} ({} ms)", r.engine, r.latency_ms),
    }
}

fn refine_line(r: &RefineRun) -> String {
    format!(
        "  {}: {} ms, {}, ${:.5}; tokens: input {}, cache {}/{}, output {}",
        r.model, r.latency_ms, r.outcome, r.cost_usd, r.input_tokens, r.cache_read_tokens, r.cache_write_tokens, r.output_tokens
    )
}

fn warm_line(r: &RefineRun) -> String {
    format!("  warm-up {}: {} ms, {}, ${:.5}", r.model, r.latency_ms, r.outcome, r.cost_usd)
}

fn summary_line(done: usize, total: usize, spent: f64, max_usd: f64, end: End, report: &Path) -> String {
    let head = match end {
        End::Done => "Done".to_owned(),
        End::Budget => format!("Spending limit ${max_usd:.2} reached — run stopped"),
        End::Interrupted => "Interrupted (Ctrl+C)".to_owned(),
    };
    format!("{head}: recordings {done} of {total}, spent ${spent:.4}; report: {}", report.display())
}

/// Cleanup left undone in Soniox — only the count of recordings.
fn cleanup_left_line(left: usize) -> Option<String> {
    (left > 0).then(|| {
        format!(
            "Warning: Soniox cleanup did not finish for recordings: {left}. Their audio or transcription may remain \
             in the Soniox account — delete them in the Soniox console."
        )
    })
}

/// Margin over the worst case of the engine's background cleanup: the wait must not end a moment before the last attempt.
const CLEANUP_MARGIN: Duration = Duration::from_secs(30);

/// How long the bench waits for the Soniox cleanup before exiting: the engine's whole retry schedule plus a margin.
fn cleanup_wait() -> Duration {
    soniox::cleanup_window() + CLEANUP_MARGIN
}

fn cleanup_wait_line(wait: Duration) -> String {
    format!("cleaning up recordings in Soniox… (up to {} min; Ctrl+C to exit without waiting)", wait.as_secs().div_ceil(60))
}

/// The recording where we stopped.
const INTERRUPTED_ITEM: &str = "  stopped by Ctrl+C: this recording is not in the results";

/// The first Ctrl+C.
const STOPPING: &str = "Ctrl+C: stopping — waiting for the recording already sent to Soniox and cleaning it up (it will \
                        not be in the results); no new recordings will start. Ctrl+C again to exit at once, without cleanup.";

/// A second Ctrl+C while waiting for the sent recording: exit at once, do not wait for the cleanup — only the count.
fn abandon_line(left: usize) -> String {
    format!(
        "Exiting without Soniox cleanup: recordings not cleaned up — {left}; a recording interrupted mid-upload may remain too. \
         Delete the leftovers in the Soniox console."
    )
}

/// Run folder name: UTC time without colons (Windows file names cannot contain them).
fn run_stamp(unix_secs: u64) -> String {
    utc_timestamp(unix_secs).replace(':', "-")
}

/// A key from Credential Manager; if it is missing, a hint on how to save it. Neither the key nor its mask is in the text.
fn key(name: KeyName) -> Result<String> {
    match keys::lookup(name) {
        Ok(Some(key)) => Ok(key),
        Ok(None) => bail!("no {0} key\nSave the key: cargo run -p cue-cli -- keys set {0}", name.as_str()),
        Err(e) => bail!("{e}"),
    }
}

/// `results.json` and `report.html` after every recording: an interrupted run loses nothing it collected.
fn save(dir: &Path, items: &[Item]) -> Result<()> {
    std::fs::write(dir.join("results.json"), serde_json::to_string_pretty(items)?).context("results.json not written")?;
    std::fs::write(dir.join("report.html"), report::embed(items)).context("report.html not written")?;
    Ok(())
}

/// Everything the run needs before the first paid call. No `Debug`: it holds keys.
struct Plan {
    settings: Settings,
    /// The run's recordings: readable and at most `--limit`. "n of N" comes from this list.
    files: Vec<PathBuf>,
    models: Vec<String>,
    soniox_key: String,
    anthropic_key: String,
    /// One client for the whole run — as in the app: connections are reused.
    http: reqwest::Client,
    dir: PathBuf,
}

/// Settings, recordings, models, keys and the run folder — no paid calls yet.
fn prepare(limit: Option<usize>, max_usd: f64) -> Result<Plan> {
    let app_data = cue_core::paths::app_data().context("no %APPDATA% (or the path is not absolute)")?;
    let settings_file = cue_core::paths::settings_json(&app_data);
    let settings = cue_core::settings::load(&settings_file).with_context(|| format!("settings {}", settings_file.display()))?;
    let base = cue_core::paths::local_app_data().context("no %LOCALAPPDATA% (or the path is not absolute)")?;
    let bench = cue_core::paths::bench_dir(&base);
    let all = recordings(&bench.join("recordings"))?;
    let files = readable(&all, limit, |f| read_recording(f).map(drop));
    if files.is_empty() {
        bail!("no recording is readable (skipped: {})", all.len());
    }
    let models = refine_models(&settings);
    let soniox_key = key(KeyName::Soniox)?;
    let anthropic_key = key(KeyName::Anthropic)?;

    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let dir = bench.join("runs").join(run_stamp(secs));
    std::fs::create_dir_all(&dir).with_context(|| format!("run folder {}", dir.display()))?;
    // An empty report right away: writing into the run folder is checked before the first paid request.
    save(&dir, &[])?;
    let plan = Plan { settings, files, models, soniox_key, anthropic_key, http: reqwest::Client::new(), dir };

    println!("recordings {}, refine models: {}; limit ${max_usd:.2}", plan.files.len(), plan.models.join(", "));
    for m in plan.models.iter().filter(|m| plan.settings.pricing.claude_price(m).is_none()) {
        println!("model {m} has no price in settings.json — its spend counts as $0, the limit does not see it");
    }
    Ok(plan)
}

/// Recordings in turn: transcription, warm-up (once), refine. After every recording — results.json and report.html.
/// On a stop request no new recordings start; for the current one the stream is interrupted, the batch is awaited,
/// refine does not run (a running one is dropped). The current recording is not in the results: an interrupted stream
/// would skew the failure count, an incomplete refine the choice count; its spend is counted in the total.
async fn work(plan: &Plan, items: &mut Vec<Item>, budget: &mut Budget, stop: &Stop) -> Result<End> {
    let s = &plan.settings;
    let fallback = Fallback {
        http: plan.http.clone(),
        soniox_key: plan.soniox_key.clone(),
        model: s.stt.soniox_async_model.clone(),
        hints: s.stt.language_hints.clone(),
    };
    let mut warmed = false;
    for (n, file) in plan.files.iter().enumerate() {
        if stop.requested() {
            return Ok(End::Interrupted);
        }
        if !budget.allows() {
            return Ok(End::Budget);
        }
        // Checked during preparation; unreadable now means the file was touched during the run.
        let rec = match read_recording(file) {
            Ok(rec) => rec,
            Err(e) => {
                println!("skipped {}: {e:#}", file_name(file));
                continue;
            }
        };
        let audio_ms = rec.pcm.len() as u64 / SAMPLES_PER_MS;
        println!("{}", item_line(n + 1, plan.files.len(), &file_name(file), audio_ms));

        let cfg = SonioxConfig {
            model: s.stt.soniox_model.clone(),
            language_hints: s.stt.language_hints.clone(),
            // As in the app: "unknown" is not an app name, so there is no context.
            app: (rec.app != UNKNOWN_APP).then(|| rec.app.clone()),
            terms: vec![],
        };
        let engine = Engine::Soniox { key: plan.soniox_key.clone(), cfg };
        let (streamed, batched) = stt_phase(stream(engine, &rec.pcm, &plan.http), batch(&fallback, file), stop).await;
        let stream_run = stt_run(STREAM, streamed, stt_cost(&s.pricing, SttEngineKind::Soniox, SttPath::Stream, audio_ms));
        let batch_run = stt_run(BATCH, batched, stt_cost(&s.pricing, SttEngineKind::Soniox, SttPath::Batch, audio_ms));
        budget.add(stream_run.cost_usd + batch_run.cost_usd);
        println!("{}\n{}", stt_line(&stream_run), stt_line(&batch_run));
        let raw = refine_input(&stream_run, &batch_run);
        let mut item = Item { file: file.display().to_string(), app: rec.app.clone(), audio_ms, stt: vec![stream_run, batch_run], refine: vec![] };

        let mut end = None;
        if let Some(raw) = raw {
            let ctx = RefineContext::new(rec.app.as_str(), None, s.refine.context_chars);
            // Warm-up on the first transcript, before the first measurement: a cold connection and the prompt cache write.
            let warm_up = !warmed;
            warmed = true;
            let phase = refine_phase(&plan.models, warm_up, budget, |model| {
                refine_once(&plan.http, &plan.anthropic_key, model, &raw, &ctx, s)
            });
            let Some((runs, budget_end)) = unless_stopped(phase, stop).await else {
                println!("{INTERRUPTED_ITEM}");
                return Ok(End::Interrupted);
            };
            item.refine = runs;
            end = budget_end;
        } else if stop.requested() {
            println!("{INTERRUPTED_ITEM}");
            return Ok(End::Interrupted);
        } else {
            println!("  refine: nothing to refine (empty, or transcription failed)");
        }
        items.push(item);
        save(&plan.dir, items)?;
        if let Some(end) = end {
            return Ok(end);
        }
    }
    Ok(End::Done)
}

/// Ctrl+C. If subscribing failed, it is "never": the run goes on as if there were no handler.
async fn interrupted() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Before exiting, wait for the background Soniox cleanup ([`cleanup_wait`]). Ctrl+C exits without waiting.
async fn wait_for_cleanup() {
    if soniox::wait_cleanups(Duration::ZERO).await == 0 {
        return;
    }
    let wait = cleanup_wait();
    println!("{}", cleanup_wait_line(wait));
    let left = tokio::select! {
        left = soniox::wait_cleanups(wait) => left,
        () = interrupted() => soniox::wait_cleanups(Duration::ZERO).await,
    };
    if let Some(line) = cleanup_left_line(left) {
        eprintln!("{line}");
    }
}

async fn run(limit: Option<usize>, max_usd: f64) -> Result<()> {
    let plan = prepare(limit, max_usd)?;
    let (mut items, mut budget) = (Vec::new(), Budget::new(max_usd));
    let (stop_tx, stop) = Stop::new();
    // The first Ctrl+C is a stop request: `work` finishes what was sent to Soniox and returns on its own.
    // The next one exits at once.
    let ctrl_c = async {
        interrupted().await;
        let _ = stop_tx.send(true);
        println!("{STOPPING}");
        interrupted().await;
    };
    let end = tokio::select! {
        end = work(&plan, &mut items, &mut budget, &stop) => end,
        () = ctrl_c => {
            // The run is already dropped: the abandoned batch call handed the cleanup of what it knows to the background; we do not wait for it.
            eprintln!("{}", abandon_line(soniox::wait_cleanups(Duration::ZERO).await));
            return Ok(());
        }
    };
    if let Ok(end) = end {
        println!("{}", summary_line(items.len(), plan.files.len(), budget.spent, max_usd, end, &plan.dir.join("report.html")));
    }
    // Whatever the outcome, the Soniox cleanup comes first; main prints the run error after it.
    wait_for_cleanup().await;
    end.map(|_| ())
}

/// Rebuild the report from a finished run's `results.json` (for example after editing the template).
fn write_report(dir: &Path) -> Result<()> {
    let path = dir.join("results.json");
    let json = std::fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
    // A parse error gives only the location: serde quotes values, and they hold dictation text.
    let items: Vec<Item> = serde_json::from_str(&json)
        .map_err(|e| anyhow!("{} not parsed: line {}, column {}", path.display(), e.line(), e.column()))?;
    let out = dir.join("report.html");
    std::fs::write(&out, report::embed(&items)).with_context(|| format!("could not write {}", out.display()))?;
    println!("Done: recordings {}; report: {}", items.len(), out.display());
    Ok(())
}

/// Engine warnings (refine fallback reasons, Soniox cleanup failures) go to stderr. Their texts hold no dictation.
fn init_log() {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .with_env_filter(tracing_subscriber::EnvFilter::new(LOG_FILTER))
        .try_init();
}

/// A panic shows only the code location, as in the app: the panic message may contain dictation text.
fn hide_panic_text() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map_or_else(String::new, |l| format!(" at {}:{}", l.file(), l.line()));
        eprintln!("cue-bench: panic{at} (the panic message is not shown)");
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    hide_panic_text();
    init_log();
    match parse_args(&std::env::args().skip(1).collect::<Vec<_>>())? {
        Command::Run { limit, max_usd } => run(limit, max_usd).await,
        Command::Report { dir } => write_report(&dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::settings::{ModelPrice, Settings};
    use cue_engines::EngineError;
    use cue_engines::stt::SttOutput;
    use cue_platform_win::focus::UNKNOWN_APP;
    use report::{RefineRun, SttRun};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    fn args(list: &[&str]) -> anyhow::Result<Command> {
        parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn args_run_and_report() {
        assert_eq!(args(&["run"]).unwrap(), Command::Run { limit: None, max_usd: 1.0 });
        assert_eq!(args(&["run", "--limit", "2"]).unwrap(), Command::Run { limit: Some(2), max_usd: 1.0 });
        assert_eq!(args(&["run", "--max-usd", "0.25"]).unwrap(), Command::Run { limit: None, max_usd: 0.25 });
        assert_eq!(args(&["run", "--limit", "3", "--max-usd", "2"]).unwrap(), Command::Run { limit: Some(3), max_usd: 2.0 });
        assert_eq!(args(&["run", "--max-usd", "2", "--limit", "3"]).unwrap(), Command::Run { limit: Some(3), max_usd: 2.0 });
        assert_eq!(args(&["report", r"C:\runs\x"]).unwrap(), Command::Report { dir: PathBuf::from(r"C:\runs\x") });
    }

    #[test]
    fn args_errors() {
        for bad in [
            &[][..],
            &["walk"],
            &["run", "--limit"],
            &["run", "--limit", "0"],
            &["run", "--limit", "two"],
            &["run", "--limit", "-1"],
            &["run", "--max-usd"],
            &["run", "--max-usd", "0"],
            &["run", "--max-usd", "-1"],
            &["run", "--max-usd", "NaN"],
            &["run", "--max-usd", "inf"],
            &["run", "--fast"],
            &["run", "extra"],
            &["report"],
            &["report", "a", "b"],
            &["report", "--limit", "2"],
        ] {
            assert!(args(bad).is_err(), "{bad:?} should be an error");
        }
        let e = args(&["run", "--fast"]).unwrap_err().to_string();
        assert!(e.contains("--fast") && e.contains("cue-bench run"), "{e}");
    }

    fn price(model: &str) -> ModelPrice {
        ModelPrice { model: model.into(), input_per_mtok: 1.0, output_per_mtok: 1.0, cache_read_per_mtok: 0.1, cache_write_per_mtok: 1.25 }
    }

    #[test]
    fn models_from_settings_refine_model_first_without_duplicates() {
        let s = Settings::default();
        assert_eq!(refine_models(&s), ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]);

        let mut s = Settings::default();
        s.refine.model = "claude-sonnet-5-5".into();
        assert_eq!(refine_models(&s), ["claude-sonnet-5-5", "claude-haiku-5-5", "claude-opus-5-5"], "the refine model comes first");

        // A refine model without a price is still listed; a repeat in prices and an empty name are skipped.
        let mut s = Settings::default();
        s.refine.model = "claude-new-6".into();
        s.pricing.claude = vec![price("claude-haiku-5-5"), price(" "), price("claude-haiku-5-5"), price("claude-opus-5-5")];
        assert_eq!(refine_models(&s), ["claude-new-6", "claude-haiku-5-5", "claude-opus-5-5"]);

        let mut s = Settings::default();
        s.pricing.claude.clear();
        assert_eq!(refine_models(&s), ["claude-haiku-5-5"]);
    }

    #[test]
    fn app_name_from_recording_json() {
        assert_eq!(app_name(Some(r#"{"app":"Telegram Desktop"}"#)), "Telegram Desktop");
        assert_eq!(app_name(Some(r#"{"app":"Блокнот","x":1}"#)), "Блокнот");
        // The app was not recognized, there is no json or it is broken — the same name the app would have passed.
        for json in [Some(r#"{"app":null}"#), Some("{}"), Some(r#"{"app":""}"#), Some(r#"{"app":"  "}"#), Some(r#"{"app":5}"#), Some("[1]"), Some("{broken"), Some(""), None] {
            assert_eq!(app_name(json), UNKNOWN_APP, "{json:?}");
        }
        assert_eq!(app_name(Some(&format!(r#"{{"app":"{UNKNOWN_APP}"}}"#))), UNKNOWN_APP);
    }

    /// A fresh folder of our own in the temp directory (`create_dir` fails if the folder already exists).
    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("cue-bench-test-{}-{name}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn write_wav(path: &Path, spec: hound::WavSpec, samples: &[i16]) {
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for s in samples {
            w.write_sample(*s).unwrap();
        }
        w.finalize().unwrap();
    }

    const SPEC: hound::WavSpec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };

    #[test]
    fn recording_pair_is_read() {
        let dir = temp_dir("pair");
        let wav = dir.join("2026-10-27T10-00-00Z-1.wav");
        write_wav(&wav, SPEC, &[1, -2, 3]);
        std::fs::write(dir.join("2026-10-27T10-00-00Z-1.json"), r#"{"app":"Telegram Desktop"}"#).unwrap();
        let r = read_recording(&wav).unwrap();
        assert_eq!((r.app.as_str(), r.pcm.as_slice()), ("Telegram Desktop", &[1, -2, 3][..]));

        // Without json — `UNKNOWN_APP`.
        let lone = dir.join("lone.wav");
        write_wav(&lone, SPEC, &[0; 16]);
        assert_eq!(read_recording(&lone).unwrap().app, UNKNOWN_APP);

        // A wrong format and an empty recording are errors (the recording is skipped).
        let stereo = dir.join("stereo.wav");
        write_wav(&stereo, hound::WavSpec { channels: 2, ..SPEC }, &[0; 4]);
        assert!(read_recording(&stereo).is_err());
        let empty = dir.join("empty.wav");
        write_wav(&empty, SPEC, &[]);
        assert!(read_recording(&empty).is_err());
        assert!(read_recording(&dir.join("missing.wav")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recordings_are_sorted_filtered_and_limited() {
        let dir = temp_dir("list");
        for name in ["b.wav", "a.wav", "c.WAV", "a.json", "notes.txt"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.join("sub.wav")).unwrap();
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(recordings(&dir).unwrap()), ["a.wav", "b.wav", "c.WAV"]);
        std::fs::remove_dir_all(&dir).unwrap();

        // No folder, or no recordings in it — a hint on how to collect them.
        let e = recordings(&dir).unwrap_err().to_string();
        assert!(e.contains("save_recordings"), "{e}");
        let empty = temp_dir("empty");
        assert!(recordings(&empty).unwrap_err().to_string().contains("save_recordings"));
        std::fs::remove_dir(empty).unwrap();
    }

    #[test]
    fn limit_counts_only_readable_recordings_and_checks_no_further() {
        let files: Vec<PathBuf> = ["bad1.wav", "a.wav", "bad2.wav", "b.wav", "c.wav"].iter().map(PathBuf::from).collect();
        let mut checked = Vec::new();
        let check = |p: &Path| {
            let name = p.to_string_lossy().into_owned();
            checked.push(name.clone());
            if name.starts_with("bad") { Err(anyhow::anyhow!("broken")) } else { Ok(()) }
        };
        // The list is ready in advance: "n of N" in the recording lines and in the summary agree.
        assert_eq!(readable(&files, Some(2), check), [PathBuf::from("a.wav"), PathBuf::from("b.wav")], "broken ones are skipped and do not count toward the limit");
        assert_eq!(checked, ["bad1.wav", "a.wav", "bad2.wav", "b.wav"], "nothing past the limit is checked");
        assert_eq!(readable(&files, None, |_: &Path| Ok(())).len(), 5);
    }

    fn heard(text: &str) -> SttOutput {
        SttOutput { text: text.into(), audio_ms: 1_000 }
    }

    /// A guard inside a future: records the time since `t0` when it is dropped.
    struct DroppedAt(tokio::time::Instant, std::sync::Arc<std::sync::Mutex<Option<Duration>>>);

    impl Drop for DroppedAt {
        fn drop(&mut self) {
            *self.1.lock().unwrap() = Some(self.0.elapsed());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn stop_interrupts_the_stream_but_waits_for_the_batch() {
        let (tx, stop) = Stop::new();
        let t0 = tokio::time::Instant::now();
        let dropped = std::sync::Arc::new(std::sync::Mutex::new(None));
        let guard = DroppedAt(t0, dropped.clone());
        let stream = async move {
            let _guard = guard;
            tokio::time::sleep(Duration::from_secs(10)).await;
            (Ok(heard("stream")), 10_000)
        };
        let batch = async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            (Ok(heard("batch")), 3_000)
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tx.send(true).unwrap();
        });
        let (streamed, batched) = stt_phase(stream, batch, &stop).await;
        assert_eq!(streamed.0, Err(EngineError::Aborted));
        assert_eq!(batched, (Ok(heard("batch")), 3_000), "the batch call is not dropped — its result is awaited");
        assert_eq!(t0.elapsed(), Duration::from_secs(3));
        assert_eq!(*dropped.lock().unwrap(), Some(Duration::from_secs(1)), "the stream is interrupted right on request");
    }

    #[tokio::test(start_paused = true)]
    async fn without_stop_both_recognitions_finish() {
        let (_tx, stop) = Stop::new();
        let stream = async { (Ok(heard("stream")), 300) };
        let batch = async { (Ok(heard("batch")), 2_000) };
        assert_eq!(stt_phase(stream, batch, &stop).await, ((Ok(heard("stream")), 300), (Ok(heard("batch")), 2_000)));
    }

    fn priced(model: &str, cost_usd: f64) -> RefineRun {
        RefineRun { model: model.into(), cost_usd, ..refine_run("") }
    }

    #[tokio::test(start_paused = true)]
    async fn refine_is_not_started_after_stop_and_a_running_one_is_dropped() {
        let models = vec!["claude-haiku-5-5".to_owned(), "claude-sonnet-5-5".to_owned()];
        let mut budget = Budget::new(1.0);

        // The stop request is already there (Ctrl+C during transcription) — not a single request.
        let (tx, stop) = Stop::new();
        tx.send(true).unwrap();
        let mut calls = 0;
        let phase = refine_phase(&models, true, &mut budget, |m| {
            calls += 1;
            std::future::ready(priced(m, 0.0))
        });
        assert!(unless_stopped(phase, &stop).await.is_none());
        assert_eq!(calls, 0, "refine is not called");

        // A stop request in the middle of a request — it is dropped, the next one is not started.
        let (tx, stop) = Stop::new();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            tx.send(true).unwrap();
        });
        let mut calls = 0;
        let phase = refine_phase(&models, false, &mut budget, |m| {
            calls += 1;
            let run = priced(m, 0.0);
            async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                run
            }
        });
        assert!(unless_stopped(phase, &stop).await.is_none());
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn refine_phase_warms_up_then_measures_within_budget() {
        let models = vec!["claude-haiku-5-5".to_owned(), "claude-sonnet-5-5".to_owned()];
        let (_tx, stop) = Stop::new();
        let mut called = Vec::new();
        let mut budget = Budget::new(1.0);
        let phase = refine_phase(&models, true, &mut budget, |m| {
            called.push(m.to_owned());
            std::future::ready(priced(m, 0.1))
        });
        let (runs, end) = unless_stopped(phase, &stop).await.unwrap();
        assert_eq!(called, ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-sonnet-5-5"], "warm-up first");
        assert_eq!(runs.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(), ["claude-haiku-5-5", "claude-sonnet-5-5"], "warm-up is not in the report");
        assert_eq!(end, None);
        assert!((budget.spent - 0.4).abs() < 1e-12, "warm-up is in the spend: {}", budget.spent);

        // The limit ran out in the middle of the warm-up — no new requests, the result is "limit".
        let mut called = 0;
        let mut budget = Budget::new(0.5);
        let (runs, end) = refine_phase(&models, true, &mut budget, |m| {
            called += 1;
            std::future::ready(priced(m, 0.3))
        })
        .await;
        assert_eq!((called, runs.len(), end), (2, 0, Some(End::Budget)));
    }

    #[test]
    fn cleanup_wait_has_a_margin_and_one_wording() {
        assert_eq!(cleanup_wait(), cue_engines::soniox::cleanup_window() + Duration::from_secs(30));
        assert!(cleanup_wait_line(Duration::from_secs(995)).contains("up to 17 min"), "{}", cleanup_wait_line(Duration::from_secs(995)));
        assert!(cleanup_wait_line(Duration::from_secs(960)).contains("up to 16 min"));
    }

    #[test]
    fn abandon_line_has_a_count_only() {
        for left in [0, 3] {
            let line = abandon_line(left);
            assert!(line.contains("Soniox") && line.contains(&left.to_string()), "{line}");
        }
    }

    #[test]
    fn budget_stops_new_calls_at_the_limit() {
        let mut b = Budget::new(0.01);
        assert!(b.allows());
        b.add(0.004);
        b.add(0.004);
        assert!(b.allows(), "spent 0.008 of 0.01");
        b.add(0.002);
        assert!(!b.allows(), "the limit is reached — no new calls");
        b.add(0.003);
        assert!(!b.allows());
        assert!((b.spent - 0.013).abs() < 1e-12, "what went past the limit is counted too: {}", b.spent);
    }

    fn stt(engine: &str, text: &str, error: Option<&str>) -> SttRun {
        SttRun { engine: engine.into(), text: text.into(), latency_ms: 10, cost_usd: 0.0, error: error.map(str::to_owned) }
    }

    #[test]
    fn refine_takes_stream_text_then_batch() {
        let ok = |t: &str| stt(STREAM, t, None);
        let batch = |t: &str| stt(BATCH, t, None);
        let failed = |e: &str| stt(STREAM, "", Some(e));
        assert_eq!(refine_input(&ok("stream"), &batch("batch")).as_deref(), Some("stream"));
        assert_eq!(refine_input(&failed("timeout"), &batch("batch")).as_deref(), Some("batch"), "the stream failed — the batch text");
        assert_eq!(refine_input(&failed("timeout"), &stt(BATCH, "", Some("network"))), None);
        // The stream gave empty text — for the app that is "empty", refine does not run.
        assert_eq!(refine_input(&ok("  "), &batch("batch")), None);
        assert_eq!(refine_input(&failed("network"), &batch(" ")), None);
    }

    #[test]
    fn stt_run_keeps_text_or_error() {
        let ok = stt_run(STREAM, (Ok(SttOutput { text: "привет".into(), audio_ms: 1_000 }), 320), 0.5);
        assert_eq!(ok, SttRun { engine: STREAM.into(), text: "привет".into(), latency_ms: 320, cost_usd: 0.5, error: None });
        let failed = stt_run(BATCH, (Err(EngineError::Timeout), 5_000), 0.25);
        assert_eq!(failed, SttRun { engine: BATCH.into(), text: String::new(), latency_ms: 5_000, cost_usd: 0.25, error: Some("timeout".into()) });
    }

    #[tokio::test(start_paused = true)]
    async fn stream_is_paced_like_speech() {
        let t0 = tokio::time::Instant::now();
        let mut pushed = Vec::new();
        // Two full 100 ms chunks and a 300-sample tail (18.75 ms).
        pace(&[0; 3_500], |chunk| pushed.push((t0.elapsed(), chunk.len()))).await;
        let ms = Duration::from_millis;
        assert_eq!(pushed, [(ms(100), 1_600), (ms(200), 1_600), (ms(218), 300)], "a chunk is sent once it has sounded");
        assert_eq!(t0.elapsed(), ms(218), "the end of audio is right after the last chunk, with no extra pause");
    }

    const SECRET: &str = "секретная диктовка";

    fn refine_run(text: &str) -> RefineRun {
        RefineRun {
            model: "claude-haiku-5-5".into(),
            text: text.into(),
            outcome: "Refined".into(),
            latency_ms: 1_234,
            cost_usd: 0.00012,
            input_tokens: 120,
            cache_read_tokens: 950,
            cache_write_tokens: 0,
            output_tokens: 40,
        }
    }

    #[test]
    fn console_lines_have_numbers_but_no_dictation_text() {
        let line = stt_line(&stt(STREAM, SECRET, None));
        assert!(!line.contains(SECRET) && line.contains("soniox") && line.contains("10 ms"), "{line}");
        let line = stt_line(&stt(BATCH, "", Some("network: connection reset")));
        assert!(line.contains("soniox-async") && line.contains("network: connection reset"), "{line}");
        for line in [refine_line(&refine_run(SECRET)), warm_line(&refine_run(SECRET))] {
            assert!(!line.contains(SECRET), "{line}");
            assert!(line.contains("claude-haiku-5-5") && line.contains("1234 ms") && line.contains("Refined"), "{line}");
        }
        assert!(warm_line(&refine_run(SECRET)).contains("warm-up"));
        let line = item_line(3, 30, "2026-10-27T10-00-00Z-7.wav", 4_210);
        assert_eq!(line, "3/30 · 2026-10-27T10-00-00Z-7.wav · 4.2 s");
    }

    #[test]
    fn summary_line_tells_why_the_run_ended() {
        let path = Path::new(r"C:\x\report.html");
        let done = summary_line(2, 2, 0.0123, 1.0, End::Done, path);
        assert!(done.contains("recordings 2 of 2") && done.contains("$0.0123") && done.contains(r"C:\x\report.html"), "{done}");
        assert!(!done.contains("limit") && !done.contains("Ctrl+C"), "{done}");
        let stopped = summary_line(12, 30, 1.0034, 1.0, End::Budget, path);
        assert!(stopped.contains("limit") && stopped.contains("$1.00") && stopped.contains("$1.0034"), "{stopped}");
        assert!(stopped.contains("recordings 12 of 30"), "{stopped}");
        let interrupted = summary_line(3, 30, 0.02, 1.0, End::Interrupted, path);
        assert!(interrupted.contains("Ctrl+C") && interrupted.contains("recordings 3 of 30"), "{interrupted}");
    }

    #[test]
    fn cleanup_notice_has_a_count_only() {
        assert_eq!(cleanup_left_line(0), None);
        let line = cleanup_left_line(2).unwrap();
        assert!(line.contains('2') && line.contains("Soniox"), "{line}");
    }

    /// A fake server on this computer: accepts one request, answers 405 and returns the request line.
    fn one_request_server() -> (String, std::thread::JoinHandle<String>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 2 {
                line.clear();
            }
            let mut stream = stream;
            stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").unwrap();
            first.trim_end().to_owned()
        });
        (url, server)
    }

    #[tokio::test]
    async fn warm_up_sends_head_to_the_api_address() {
        let (url, server) = one_request_server();
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        warm_at(&http, url).await.unwrap();
        assert_eq!(server.join().unwrap(), "HEAD /v1/messages HTTP/1.1", "no body and no key — only the connection");
    }

    #[test]
    fn run_folder_name_is_a_safe_utc_time() {
        assert_eq!(run_stamp(1_700_000_000), "2023-11-14T22-13-20Z");
    }
}
