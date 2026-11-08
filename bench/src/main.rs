//! Bench: the user's recordings → every local speech model → refine models from settings → a report with blind pairs.
//!
//! cue-bench run [--limit N] [--max-usd X]    cue-bench report <run folder>
//!
//! - Recordings live in `%LOCALAPPDATA%\CUE\bench\recordings\` (`<time>-<id>.wav` + `.json` with `{"app": …}`); the app
//!   saves them when settings.json has `"bench": {"save_recordings": true}`. A run goes to `%LOCALAPPDATA%\CUE\bench\runs\<time>\`
//!   (`results.json` and `report.html`, written after every recording): dictation text and audio stay only there, outside the repository.
//!   Recordings are checked before the run: broken ones are skipped and do not count toward `--limit`.
//! - Transcription compares the speech models on this computer: every folder in `%LOCALAPPDATA%\CUE\models\` that loads
//!   is one engine, named by the folder (`parakeet-ultra-int8` is the app's model; another export, such as NVIDIA's v3,
//!   can be put next to it). Each model is loaded once before the run and recognizes every whole recording in turn;
//!   latency is the model's own recognition time, the cost is $0. When transcription starts, like the app on the
//!   second press, the connection to Anthropic is warmed up (HEAD without a key).
//! - Refine runs on the text of `parakeet-ultra-int8` (if it failed or is not installed, on the first model that
//!   succeeded), each model in turn: the same request (with `dictionary.terms`), sanity check and timeout as in the app
//!   (`claude::refine`). Models come from settings.json.
//! - Before the first measurement there is a warm-up: one request per model; it is not in the report, but it is in the spend.
//! - Spending limit `--max-usd` (default $1): once reached, no new paid calls start; what was collected is saved.
//! - Ctrl+C means "finish the current step and stop": the recognition or refine request under way finishes, nothing
//!   new starts, the current recording is not in the results. The next Ctrl+C exits at once.
//! - The console gets numbers, file names, durations, latencies, outcomes, tokens and prices; stderr gets engine
//!   warnings (WARN). No dictation text and no keys there.
mod report;

use anyhow::{Context, Result, anyhow, bail};
use cue_core::model::{DEFAULT_ID, ultra};
use cue_core::refine::RefineContext;
use cue_core::settings::Settings;
use cue_core::usage::{refine_cost, utc_timestamp};
use cue_engines::EngineError;
use cue_engines::claude;
use cue_engines::keys::{self, KeyName};
use cue_engines::parakeet::model::{self as parakeet, Recognized, Recognizer};
use cue_engines::stt::SAMPLES_PER_MS;
use cue_platform_win::focus::UNKNOWN_APP;
use report::{Item, RefineRun, SttRun};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const USAGE: &str = "cue-bench run [--limit N] [--max-usd X] | cue-bench report <run folder>";
const DEFAULT_MAX_USD: f64 = 1.0;
/// Connection warm-up timeout — like the app's (`WARM_TIMEOUT` in runtime.rs).
const WARM_TIMEOUT: Duration = Duration::from_secs(5);
/// Engine log: warnings only. The filter is fixed, so no environment variable can turn on the HTTP stack's
/// debug output, which would carry request headers.
const LOG_FILTER: &str = "warn";
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

/// Refine models: the Anthropic model picked in settings first, then the priced Claude models (not the other
/// connections' ids: the bench calls Anthropic directly) — in order, without repeats.
fn refine_models(s: &Settings) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    let priced = s.pricing.claude.iter().map(|p| p.model.as_str()).filter(|m| m.starts_with("claude-"));
    for m in std::iter::once(s.refine.model_for("anthropic").id).chain(priced) {
        if !m.trim().is_empty() && !models.iter().any(|x| x == m) {
            models.push(m.to_owned());
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

/// What to do when no model loads: where the folders are and what a folder must hold.
fn no_models(root: &Path) -> String {
    format!(
        "no speech model loads from {}: start CUE once so it downloads {}, or put a model folder there \
         ({}, {}, {})",
        root.display(),
        ultra().name,
        parakeet::ENCODER,
        parakeet::DECODER,
        parakeet::VOCAB
    )
}

/// The model folders in `root`, the app's model first, the rest by name — the report's columns keep that order.
/// Files are not models; whether a folder holds one is for the loader to say.
fn model_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!("{}", no_models(root)),
        Err(e) => return Err(e).with_context(|| format!("models folder {}", root.display())),
    };
    let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    dirs.sort_by_key(|d| (file_name(d) != DEFAULT_ID, file_name(d)));
    Ok(dirs)
}

/// A loaded speech model under its folder's name, which is its engine name in the report.
struct Model<T> {
    name: String,
    recognizer: T,
}

/// Every folder `load` accepts, once, with a console line each: the load time, or why the folder is skipped (a
/// partial download, a different export, a processor ONNX Runtime cannot run on).
fn load_models<T>(dirs: &[PathBuf], mut load: impl FnMut(&Path) -> Result<T, EngineError>) -> Vec<Model<T>> {
    let mut models = Vec::new();
    for dir in dirs {
        let name = file_name(dir);
        let t0 = Instant::now();
        let loaded = load(dir);
        println!("{}", load_line(&name, loaded.as_ref().map(|_| ms(t0.elapsed()))));
        if let Ok(recognizer) = loaded {
            models.push(Model { name, recognizer });
        }
    }
    models
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

/// Warm-up, like the app's `warm_up`: HEAD to the API address without a key and without text — for a TLS connection
/// in the shared client's pool, so refine does not pay for the handshake after the transcription. The response
/// (a refusal) is not needed; a failure is a log line.
fn warm_at(http: &reqwest::Client, url: String) -> tokio::task::JoinHandle<()> {
    let http = http.clone();
    tokio::spawn(async move {
        if let Err(e) = http.head(url).timeout(WARM_TIMEOUT).send().await {
            tracing::warn!("Anthropic connection warm-up: {}", if e.is_timeout() { "timeout" } else { "network" });
        }
    })
}

/// A recognition result and the wall time of the call (the latency of a failure, which has no `took_ms`).
type Heard = (Result<Recognized, EngineError>, u64);

/// The whole recording through one model, on a blocking thread: the runtime stays free to see Ctrl+C meanwhile.
async fn recognize(model: Arc<Recognizer>, pcm: Arc<[i16]>) -> Heard {
    let t0 = Instant::now();
    let r = tokio::task::spawn_blocking(move || model.recognize(&pcm))
        .await
        // A panic's message may hold dictation text; the report gets a fixed one.
        .unwrap_or_else(|_| Err(EngineError::Model("the recognition thread failed".into())));
    (r, ms(t0.elapsed()))
}

/// A request to stop — the first Ctrl+C. It is looked at between steps: a recognition cannot be interrupted, and a
/// refine request under way is already paid for.
#[derive(Clone, Default)]
struct Stop(Arc<AtomicBool>);

impl Stop {
    fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn requested(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Engine result → report row and whether the other alphabet was masked out. An error goes through the engine's
/// `Display` (it holds no dictation text); the text is then empty.
fn stt_run(engine: &str, (result, elapsed_ms): Heard) -> (SttRun, bool) {
    let (text, latency_ms, error, masked) = match result {
        Ok(r) => (r.text, r.took_ms, None, r.masked),
        Err(e) => (String::new(), elapsed_ms, Some(e.to_string()), false),
    };
    (SttRun { engine: engine.into(), text, latency_ms, cost_usd: 0.0, error }, masked)
}

/// Transcription of a recording: each model in turn (`recognize(i)` is model `i` of `names`), a console line each.
/// `None` if a stop request came between models: the recording is not complete.
async fn stt_phase<Fut: Future<Output = Heard>>(names: &[String], stop: &Stop, mut recognize: impl FnMut(usize) -> Fut) -> Option<Vec<SttRun>> {
    let mut runs = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if stop.requested() {
            return None;
        }
        let (run, masked) = stt_run(name, recognize(i).await);
        println!("{}", stt_line(&run, masked));
        runs.push(run);
    }
    Some(runs)
}

/// Text for refine — as in the app: the app's model's text; if it failed or is not installed, the first model that
/// succeeded. The app does not refine empty text (outcome "empty") — neither does the bench.
fn refine_input(runs: &[SttRun]) -> Option<String> {
    let ok = |r: &&SttRun| r.error.is_none();
    let source = runs.iter().filter(ok).find(|r| r.engine == DEFAULT_ID).or_else(|| runs.iter().find(ok))?;
    (!source.text.trim().is_empty()).then(|| source.text.clone())
}

/// One refine request — the same as in the app (`claude::refine`: request with the dictionary from settings, check,
/// timeout). Latency is the whole call.
async fn refine_once(http: &reqwest::Client, key: &str, model: &str, raw: &str, ctx: &RefineContext, s: &Settings) -> RefineRun {
    let t0 = Instant::now();
    let r = claude::refine(http, Some(key), model, raw, ctx, &s.dictionary, &s.refine, cue_core::refine::BOLD_PROMPT).await;
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

/// Refine for one recording: warm-up (if `warm_up`), then a measurement per model — in turn, while the limit allows
/// and no stop was requested. `call(model)` is one request. Returns the measurements (warm-up is not among them) and
/// why the phase ended early: `Some(End::Budget)` or `Some(End::Interrupted)`.
async fn refine_phase<'a, Fut: Future<Output = RefineRun>>(
    models: &'a [String],
    warm_up: bool,
    budget: &mut Budget,
    stop: &Stop,
    mut call: impl FnMut(&'a str) -> Fut,
) -> (Vec<RefineRun>, Option<End>) {
    let warm: &[String] = if warm_up { models } else { &[] };
    let mut runs = Vec::new();
    for (model, warm) in warm.iter().map(|m| (m, true)).chain(models.iter().map(|m| (m, false))) {
        if stop.requested() {
            return (runs, Some(End::Interrupted));
        }
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

fn load_line(name: &str, loaded: Result<u64, &EngineError>) -> String {
    match loaded {
        Ok(ms) => format!("speech model {name}: loaded in {ms} ms"),
        Err(e) => format!("speech model {name}: skipped — {e}"),
    }
}

fn item_line(n: usize, total: usize, name: &str, audio_ms: u64) -> String {
    format!("{n}/{total} · {name} · {:.1} s", audio_ms as f64 / 1000.0)
}

fn stt_line(r: &SttRun, masked: bool) -> String {
    match &r.error {
        None => format!("  {}: {} ms, other alphabet masked: {}", r.engine, r.latency_ms, if masked { "yes" } else { "no" }),
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

/// The recording where we stopped.
const INTERRUPTED_ITEM: &str = "  stopped by Ctrl+C: this recording is not in the results";

/// The first Ctrl+C.
const STOPPING: &str = "Ctrl+C: stopping after the current step — this recording will not be in the results, no new \
                        recordings will start. Ctrl+C again to exit at once.";

/// The second Ctrl+C: what was collected is already on disk, saved after every recording.
fn exit_line(report: &Path) -> String {
    format!("Exiting at once; recordings finished so far are in the report: {}", report.display())
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

/// Everything the run needs before the first paid call. No `Debug`: it holds a key.
struct Plan {
    settings: Settings,
    /// The run's recordings: readable and at most `--limit`. "n of N" comes from this list.
    files: Vec<PathBuf>,
    speech: Vec<Model<Arc<Recognizer>>>,
    models: Vec<String>,
    anthropic_key: String,
    /// One client for the whole run — as in the app: connections are reused.
    http: reqwest::Client,
    dir: PathBuf,
}

/// Settings, recordings, refine models, the key, the speech models and the run folder — no paid calls yet.
/// The models load last of the checks: it takes seconds, and a missing key should not wait for it.
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
    let anthropic_key = key(KeyName::Anthropic)?;

    // The same folder the app keeps its model in (`paths::model_dir`).
    let models_root = cue_core::paths::local_dir(&base).join("models");
    let speech = load_models(&model_dirs(&models_root)?, |d| Recognizer::load(d).map(Arc::new));
    if speech.is_empty() {
        bail!("{}", no_models(&models_root));
    }

    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let dir = bench.join("runs").join(run_stamp(secs));
    std::fs::create_dir_all(&dir).with_context(|| format!("run folder {}", dir.display()))?;
    // An empty report right away: writing into the run folder is checked before the first paid request.
    save(&dir, &[])?;
    let plan = Plan { settings, files, speech, models, anthropic_key, http: reqwest::Client::new(), dir };

    let speech_names: Vec<&str> = plan.speech.iter().map(|m| m.name.as_str()).collect();
    println!(
        "recordings {}, speech models: {}; refine models: {}; limit ${max_usd:.2}",
        plan.files.len(),
        speech_names.join(", "),
        plan.models.join(", ")
    );
    for m in plan.models.iter().filter(|m| plan.settings.pricing.claude_price(m).is_none()) {
        println!("model {m} has no price in settings.json — its spend counts as $0, the limit does not see it");
    }
    Ok(plan)
}

/// Recordings in turn: transcription by every model, warm-up (once), refine. After every recording — results.json and
/// report.html. On a stop request the step under way finishes and nothing new starts; the current recording is not in
/// the results (a model it was not heard by would skew the failure count, an incomplete refine the choice count); its
/// spend is counted in the total.
async fn work(plan: &Plan, items: &mut Vec<Item>, budget: &mut Budget, stop: &Stop) -> Result<End> {
    let s = &plan.settings;
    let names: Vec<String> = plan.speech.iter().map(|m| m.name.clone()).collect();
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

        // The start of recognition is the "second press": like the app, the connection to Anthropic warms up by the time the text is there.
        drop(warm_at(&plan.http, claude::API_URL.to_owned()));
        let pcm: Arc<[i16]> = rec.pcm.into();
        let phase = stt_phase(&names, stop, |i| recognize(plan.speech[i].recognizer.clone(), pcm.clone())).await;
        let Some(stt) = phase else {
            println!("{INTERRUPTED_ITEM}");
            return Ok(End::Interrupted);
        };
        let raw = refine_input(&stt);
        let mut item = Item { file: file.display().to_string(), app: rec.app.clone(), audio_ms, stt, refine: vec![] };

        let mut end = None;
        if let Some(raw) = raw {
            let ctx = RefineContext::new(rec.app.as_str(), None, s.refine.context_chars);
            // Warm-up on the first transcript, before the first measurement: a cold connection and the prompt cache write.
            let warm_up = !warmed;
            warmed = true;
            let (runs, phase_end) = refine_phase(&plan.models, warm_up, budget, stop, |model| {
                refine_once(&plan.http, &plan.anthropic_key, model, &raw, &ctx, s)
            })
            .await;
            if phase_end == Some(End::Interrupted) {
                println!("{INTERRUPTED_ITEM}");
                return Ok(End::Interrupted);
            }
            item.refine = runs;
            end = phase_end;
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

async fn run(limit: Option<usize>, max_usd: f64) -> Result<()> {
    let plan = prepare(limit, max_usd)?;
    let (mut items, mut budget, stop) = (Vec::new(), Budget::new(max_usd), Stop::default());
    // The first Ctrl+C is a stop request: `work` finishes the step under way and returns on its own.
    // The next one exits at once.
    let ctrl_c = async {
        interrupted().await;
        stop.request();
        println!("{STOPPING}");
        interrupted().await;
    };
    let report = plan.dir.join("report.html");
    let end = tokio::select! {
        end = work(&plan, &mut items, &mut budget, &stop) => end,
        () = ctrl_c => {
            // `work` is dropped at an await, never inside `save`, so the files on disk are whole. The process ends
            // here rather than by returning: the runtime's shutdown would wait for a recognition under way.
            println!("{}", exit_line(&report));
            std::process::exit(130);
        }
    };
    if let Ok(end) = end {
        println!("{}", summary_line(items.len(), plan.files.len(), budget.spent, max_usd, end, &report));
    }
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

/// Engine warnings (refine fallback reasons, model load problems) go to stderr. Their texts hold no dictation.
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
    use cue_platform_win::focus::UNKNOWN_APP;
    use report::{RefineRun, SttRun};
    use std::path::{Path, PathBuf};

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
        s.refine.models.insert("anthropic".into(), "claude-sonnet-5-5".into());
        assert_eq!(refine_models(&s), ["claude-sonnet-5-5", "claude-haiku-5-5", "claude-opus-5-5"], "the refine model comes first");

        // The picked model without a price is still listed; a repeat in prices, an empty name and another connection's
        // id are skipped.
        let mut s = Settings::default();
        s.pricing.claude =
            vec![price("claude-sonnet-5-5"), price(" "), price("claude-sonnet-5-5"), price("openai:gpt-6-luna"), price("claude-opus-5-5")];
        assert_eq!(refine_models(&s), ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]);

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

    const V3: &str = "parakeet-v3-int8";

    #[test]
    fn model_folders_are_found_app_model_first_and_files_are_not_models() {
        let root = temp_dir("models");
        for d in [V3, "aaa-export", DEFAULT_ID, "zz-partial"] {
            std::fs::create_dir(root.join(d)).unwrap();
        }
        std::fs::write(root.join("notes.txt"), b"").unwrap();
        let names = |v: &[PathBuf]| v.iter().map(|p| file_name(p)).collect::<Vec<_>>();
        let dirs = model_dirs(&root).unwrap();
        assert_eq!(names(&dirs), [DEFAULT_ID, "aaa-export", V3, "zz-partial"], "the app's model first, then by name");

        // A folder the loader refuses (here: no vocabulary) is skipped; the rest keep their order and folder names.
        for d in [V3, "aaa-export", DEFAULT_ID] {
            std::fs::write(root.join(d).join(parakeet::VOCAB), d).unwrap();
        }
        let mut tried = Vec::new();
        let models = load_models(&dirs, |d| {
            tried.push(file_name(d));
            std::fs::read_to_string(d.join(parakeet::VOCAB)).map_err(|e| EngineError::Model(e.to_string()))
        });
        assert_eq!(tried, [DEFAULT_ID, "aaa-export", V3, "zz-partial"], "each folder is loaded once");
        let loaded: Vec<(&str, &str)> = models.iter().map(|m| (m.name.as_str(), m.recognizer.as_str())).collect();
        assert_eq!(loaded, [(DEFAULT_ID, DEFAULT_ID), ("aaa-export", "aaa-export"), (V3, V3)]);

        // An empty folder is no error here — `prepare` says "no model loads"; a missing folder already says what to do.
        let empty = temp_dir("no-models");
        assert!(model_dirs(&empty).unwrap().is_empty());
        std::fs::remove_dir(&empty).unwrap();
        let e = model_dirs(&empty).unwrap_err().to_string();
        assert!(e.contains("no speech model") && e.contains(parakeet::ENCODER) && e.contains(&empty.display().to_string()), "{e}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn load_lines_have_the_time_or_the_reason() {
        assert_eq!(load_line(DEFAULT_ID, Ok(1_234)), "speech model parakeet-ultra-int8: loaded in 1234 ms");
        let line = load_line(V3, Err(&EngineError::Unsupported("no AVX2".into())));
        assert!(line.contains(V3) && line.contains("skipped") && line.contains("no AVX2"), "{line}");
    }

    fn recognized(text: &str, masked: bool) -> Recognized {
        Recognized { text: text.into(), masked, audio_ms: 4_000, took_ms: 310 }
    }

    #[test]
    fn stt_run_keeps_text_or_error() {
        let (ok, masked) = stt_run(DEFAULT_ID, (Ok(recognized("привет", true)), 330));
        assert_eq!(ok, SttRun { engine: DEFAULT_ID.into(), text: "привет".into(), latency_ms: 310, cost_usd: 0.0, error: None }, "latency is the model's own time");
        assert!(masked);
        let (failed, masked) = stt_run(V3, (Err(EngineError::Model("encoder: broken".into())), 45));
        assert_eq!(failed, SttRun { engine: V3.into(), text: String::new(), latency_ms: 45, cost_usd: 0.0, error: Some("speech model: encoder: broken".into()) });
        assert!(!masked);
    }

    #[tokio::test]
    async fn every_model_hears_the_recording_until_a_stop() {
        let names = vec![DEFAULT_ID.to_owned(), V3.to_owned()];
        let stop = Stop::default();
        let mut asked = Vec::new();
        let runs = stt_phase(&names, &stop, |i| {
            asked.push(i);
            std::future::ready((Ok(recognized(&format!("text {i}"), false)), 300))
        })
        .await
        .unwrap();
        assert_eq!(asked, [0, 1]);
        assert_eq!(runs.iter().map(|r| (r.engine.as_str(), r.text.as_str())).collect::<Vec<_>>(), [(DEFAULT_ID, "text 0"), (V3, "text 1")]);

        // A stop during the first model: it finishes, the next one does not start, the recording is incomplete.
        let mut asked = 0;
        let phase = stt_phase(&names, &stop, |_| {
            asked += 1;
            stop.request();
            std::future::ready((Ok(recognized("", false)), 300))
        });
        assert_eq!(phase.await, None);
        assert_eq!(asked, 1);
    }

    fn priced(model: &str, cost_usd: f64) -> RefineRun {
        RefineRun { model: model.into(), cost_usd, ..refine_run("") }
    }

    #[tokio::test]
    async fn refine_is_not_started_after_stop_and_the_one_under_way_finishes() {
        let models = vec!["claude-haiku-5-5".to_owned(), "claude-sonnet-5-5".to_owned()];
        let mut budget = Budget::new(1.0);

        // The stop request is already there (Ctrl+C during transcription) — not a single request.
        let stop = Stop::default();
        stop.request();
        let mut calls = 0;
        let (runs, end) = refine_phase(&models, true, &mut budget, &stop, |m| {
            calls += 1;
            std::future::ready(priced(m, 0.0))
        })
        .await;
        assert_eq!((calls, runs.len(), end), (0, 0, Some(End::Interrupted)), "refine is not called");

        // A stop request during a request — that one is awaited and paid for, the next one is not started.
        let stop = Stop::default();
        let mut calls = 0;
        let (runs, end) = refine_phase(&models, false, &mut budget, &stop, |m| {
            calls += 1;
            stop.request();
            std::future::ready(priced(m, 0.25))
        })
        .await;
        assert_eq!((calls, runs.len(), end), (1, 1, Some(End::Interrupted)));
        assert!((budget.spent - 0.25).abs() < 1e-12, "its spend is counted: {}", budget.spent);
    }

    #[tokio::test]
    async fn refine_phase_warms_up_then_measures_within_budget() {
        let models = vec!["claude-haiku-5-5".to_owned(), "claude-sonnet-5-5".to_owned()];
        let stop = Stop::default();
        let mut called = Vec::new();
        let mut budget = Budget::new(1.0);
        let (runs, end) = refine_phase(&models, true, &mut budget, &stop, |m| {
            called.push(m.to_owned());
            std::future::ready(priced(m, 0.1))
        })
        .await;
        assert_eq!(called, ["claude-haiku-5-5", "claude-sonnet-5-5", "claude-haiku-5-5", "claude-sonnet-5-5"], "warm-up first");
        assert_eq!(runs.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(), ["claude-haiku-5-5", "claude-sonnet-5-5"], "warm-up is not in the report");
        assert_eq!(end, None);
        assert!((budget.spent - 0.4).abs() < 1e-12, "warm-up is in the spend: {}", budget.spent);

        // The limit ran out in the middle of the warm-up — no new requests, the result is "limit".
        let mut called = 0;
        let mut budget = Budget::new(0.5);
        let (runs, end) = refine_phase(&models, true, &mut budget, &stop, |m| {
            called += 1;
            std::future::ready(priced(m, 0.3))
        })
        .await;
        assert_eq!((called, runs.len(), end), (2, 0, Some(End::Budget)));
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
    fn refine_takes_the_app_model_text_then_the_first_that_succeeded() {
        let ultra = |t: &str| stt(DEFAULT_ID, t, None);
        let v3 = |t: &str| stt(V3, t, None);
        let other = |t: &str| stt("aaa-export", t, None);
        let failed = |engine: &str| stt(engine, "", Some("speech model: broken"));
        assert_eq!(refine_input(&[ultra("ultra"), v3("v3")]).as_deref(), Some("ultra"));
        // The app's model is not first in the list — it is still the one refined.
        assert_eq!(refine_input(&[other("other"), ultra("ultra")]).as_deref(), Some("ultra"));
        assert_eq!(refine_input(&[failed(DEFAULT_ID), other("other"), v3("v3")]).as_deref(), Some("other"), "the app's model failed — the first that succeeded");
        assert_eq!(refine_input(&[failed("aaa-export"), v3("v3")]).as_deref(), Some("v3"), "the app's model is not installed");
        assert_eq!(refine_input(&[failed(DEFAULT_ID), failed(V3)]), None);
        assert_eq!(refine_input(&[]), None);
        // The app's model gave empty text — for the app that is "empty", refine does not run.
        assert_eq!(refine_input(&[ultra("  "), v3("v3")]), None);
        assert_eq!(refine_input(&[failed(DEFAULT_ID), v3(" "), other("other")]), None);
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
        let line = stt_line(&stt(DEFAULT_ID, SECRET, None), false);
        assert_eq!(line, "  parakeet-ultra-int8: 10 ms, other alphabet masked: no");
        let line = stt_line(&stt(V3, SECRET, None), true);
        assert!(!line.contains(SECRET) && line.contains(V3) && line.ends_with("masked: yes"), "{line}");
        let line = stt_line(&stt(V3, "", Some("speech model: encoder: broken")), false);
        assert!(line.contains(V3) && line.contains("failed — speech model: encoder: broken") && line.contains("10 ms"), "{line}");
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
    fn stop_wording_is_about_steps_and_the_exit_points_to_the_report() {
        assert!(STOPPING.contains("current step") && STOPPING.contains("again"), "{STOPPING}");
        let line = exit_line(Path::new(r"C:\x\report.html"));
        assert!(line.contains("at once") && line.contains(r"C:\x\report.html"), "{line}");
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
