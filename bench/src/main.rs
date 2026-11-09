//! Bench: the user's recordings → every local speech model → a report with blind pairs.
//!
//! cue-bench run [--limit N]    cue-bench report <run folder>
//!
//! - Recordings live in `%LOCALAPPDATA%\CUE\bench\recordings\` (`<time>-<id>.wav` + `.json` with `{"app": …}`); the app
//!   saves them when settings.json has `"bench": {"save_recordings": true}`. A run goes to `%LOCALAPPDATA%\CUE\bench\runs\<time>\`
//!   (`results.json` and `report.html`, written after every recording): dictation text and audio stay only there, outside the repository.
//!   Recordings are checked before the run: broken ones are skipped and do not count toward `--limit`.
//! - Transcription compares the speech models on this computer: every folder in `%LOCALAPPDATA%\CUE\models\` that loads
//!   is one engine, named by the folder (`parakeet-ultra-int8` is the app's model; another export, such as NVIDIA's v3,
//!   can be put next to it). Each model is loaded once before the run and recognizes every whole recording in turn;
//!   latency is the model's own recognition time.
//! - The cleanup is not compared: CUE holds no API key, and nothing here calls a cloud service. A report rebuilt from a
//!   run of an earlier version still shows that run's cleanup.
//! - Ctrl+C means "finish the current step and stop": the recognition under way finishes, nothing new starts, the
//!   current recording is not in the results. The next Ctrl+C exits at once.
//! - The console gets numbers, file names, durations, latencies and outcomes; stderr gets engine warnings (WARN). No
//!   dictation text there.
mod report;

use anyhow::{Context, Result, anyhow, bail};
use cue_core::model::{DEFAULT_ID, ultra};
use cue_core::usage::utc_timestamp;
use cue_engines::EngineError;
use cue_engines::parakeet::model::{self as parakeet, Recognized, Recognizer};
use cue_engines::stt::SAMPLES_PER_MS;
use cue_platform_win::focus::UNKNOWN_APP;
use report::{Item, SttRun};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const USAGE: &str = "cue-bench run [--limit N] | cue-bench report <run folder>";
/// Engine log: warnings only. The filter is fixed, so no environment variable can turn on the HTTP stack's
/// debug output, which would carry request headers.
const LOG_FILTER: &str = "warn";
const NO_RECORDINGS: &str = "no recordings: set \"bench\": {\"save_recordings\": true} in %APPDATA%\\CUE\\settings.json, \
                             restart CUE and dictate a few times";

#[derive(Debug, PartialEq)]
enum Command {
    /// `limit: None` means all recordings.
    Run { limit: Option<usize> },
    Report { dir: PathBuf },
}

fn parse_args(args: &[String]) -> Result<Command> {
    match args.split_first() {
        Some((cmd, rest)) if cmd == "run" => {
            let mut limit = None;
            let mut it = rest.iter();
            while let Some(arg) = it.next() {
                let mut value = || it.next().with_context(|| format!("{arg} needs a value\n{USAGE}"));
                match arg.as_str() {
                    "--limit" => limit = Some(value()?.parse().ok().filter(|n: &usize| *n >= 1).context("--limit: an integer from 1")?),
                    other => bail!("unknown argument {other}\n{USAGE}"),
                }
            }
            Ok(Command::Run { limit })
        }
        Some((cmd, [dir])) if cmd == "report" && !dir.starts_with("--") => Ok(Command::Report { dir: PathBuf::from(dir) }),
        _ => bail!("{USAGE}"),
    }
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

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
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

/// A request to stop — the first Ctrl+C. It is looked at between steps: a recognition cannot be interrupted.
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
    (SttRun { engine: engine.into(), text, latency_ms, error }, masked)
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

/// How the run ended.
#[derive(Debug, Clone, Copy, PartialEq)]
enum End {
    Done,
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

fn summary_line(done: usize, total: usize, end: End, report: &Path) -> String {
    let head = match end {
        End::Done => "Done",
        End::Interrupted => "Interrupted (Ctrl+C)",
    };
    format!("{head}: recordings {done} of {total}; report: {}", report.display())
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

/// `results.json` and `report.html` after every recording: an interrupted run loses nothing it collected.
fn save(dir: &Path, items: &[Item]) -> Result<()> {
    std::fs::write(dir.join("results.json"), serde_json::to_string_pretty(items)?).context("results.json not written")?;
    std::fs::write(dir.join("report.html"), report::embed(items)).context("report.html not written")?;
    Ok(())
}

/// Everything the run needs before the first recognition.
struct Plan {
    /// The run's recordings: readable and at most `--limit`. "n of N" comes from this list.
    files: Vec<PathBuf>,
    speech: Vec<Model<Arc<Recognizer>>>,
    dir: PathBuf,
}

/// Recordings, the speech models and the run folder. The models load last of the checks: it takes seconds.
fn prepare(limit: Option<usize>) -> Result<Plan> {
    let base = cue_core::paths::local_app_data().context("no %LOCALAPPDATA% (or the path is not absolute)")?;
    let bench = cue_core::paths::bench_dir(&base);
    let all = recordings(&bench.join("recordings"))?;
    let files = readable(&all, limit, |f| read_recording(f).map(drop));
    if files.is_empty() {
        bail!("no recording is readable (skipped: {})", all.len());
    }

    // The same folder the app keeps its model in (`paths::model_dir`).
    let models_root = cue_core::paths::local_dir(&base).join("models");
    let speech = load_models(&model_dirs(&models_root)?, |d| Recognizer::load(d).map(Arc::new));
    if speech.is_empty() {
        bail!("{}", no_models(&models_root));
    }

    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
    let dir = bench.join("runs").join(run_stamp(secs));
    std::fs::create_dir_all(&dir).with_context(|| format!("run folder {}", dir.display()))?;
    // An empty report right away: writing into the run folder is checked before the first recognition.
    save(&dir, &[])?;
    let plan = Plan { files, speech, dir };

    let speech_names: Vec<&str> = plan.speech.iter().map(|m| m.name.as_str()).collect();
    println!("recordings {}, speech models: {}", plan.files.len(), speech_names.join(", "));
    Ok(plan)
}

/// Recordings in turn: transcription by every model. After every recording — results.json and report.html. On a stop
/// request the step under way finishes and nothing new starts; the current recording is not in the results (a model it
/// was not heard by would skew the failure count).
async fn work(plan: &Plan, items: &mut Vec<Item>, stop: &Stop) -> Result<End> {
    let names: Vec<String> = plan.speech.iter().map(|m| m.name.clone()).collect();
    for (n, file) in plan.files.iter().enumerate() {
        if stop.requested() {
            return Ok(End::Interrupted);
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

        let pcm: Arc<[i16]> = rec.pcm.into();
        let phase = stt_phase(&names, stop, |i| recognize(plan.speech[i].recognizer.clone(), pcm.clone())).await;
        let Some(stt) = phase else {
            println!("{INTERRUPTED_ITEM}");
            return Ok(End::Interrupted);
        };
        items.push(Item { file: file.display().to_string(), app: rec.app.clone(), audio_ms, stt, refine: vec![] });
        save(&plan.dir, items)?;
    }
    Ok(End::Done)
}

/// Ctrl+C. If subscribing failed, it is "never": the run goes on as if there were no handler.
async fn interrupted() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

async fn run(limit: Option<usize>) -> Result<()> {
    let plan = prepare(limit)?;
    let (mut items, stop) = (Vec::new(), Stop::default());
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
        end = work(&plan, &mut items, &stop) => end,
        () = ctrl_c => {
            // `work` is dropped at an await, never inside `save`, so the files on disk are whole. The process ends
            // here rather than by returning: the runtime's shutdown would wait for a recognition under way.
            println!("{}", exit_line(&report));
            std::process::exit(130);
        }
    };
    if let Ok(end) = end {
        println!("{}", summary_line(items.len(), plan.files.len(), end, &report));
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

/// Engine warnings (model load problems) go to stderr. Their texts hold no dictation.
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
        Command::Run { limit } => run(limit).await,
        Command::Report { dir } => write_report(&dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_engines::EngineError;
    use cue_platform_win::focus::UNKNOWN_APP;
    use report::SttRun;
    use std::path::{Path, PathBuf};

    fn args(list: &[&str]) -> anyhow::Result<Command> {
        parse_args(&list.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn args_run_and_report() {
        assert_eq!(args(&["run"]).unwrap(), Command::Run { limit: None });
        assert_eq!(args(&["run", "--limit", "2"]).unwrap(), Command::Run { limit: Some(2) });
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
            &["run", "--max-usd", "1"],
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
        assert_eq!(ok, SttRun { engine: DEFAULT_ID.into(), text: "привет".into(), latency_ms: 310, error: None }, "latency is the model's own time");
        assert!(masked);
        let (failed, masked) = stt_run(V3, (Err(EngineError::Model("encoder: broken".into())), 45));
        assert_eq!(failed, SttRun { engine: V3.into(), text: String::new(), latency_ms: 45, error: Some("speech model: encoder: broken".into()) });
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

    fn stt(engine: &str, text: &str, error: Option<&str>) -> SttRun {
        SttRun { engine: engine.into(), text: text.into(), latency_ms: 10, error: error.map(str::to_owned) }
    }

    const SECRET: &str = "секретная диктовка";

    #[test]
    fn console_lines_have_numbers_but_no_dictation_text() {
        let line = stt_line(&stt(DEFAULT_ID, SECRET, None), false);
        assert_eq!(line, "  parakeet-ultra-int8: 10 ms, other alphabet masked: no");
        let line = stt_line(&stt(V3, SECRET, None), true);
        assert!(!line.contains(SECRET) && line.contains(V3) && line.ends_with("masked: yes"), "{line}");
        let line = stt_line(&stt(V3, "", Some("speech model: encoder: broken")), false);
        assert!(line.contains(V3) && line.contains("failed — speech model: encoder: broken") && line.contains("10 ms"), "{line}");
        let line = item_line(3, 30, "2026-10-27T10-00-00Z-7.wav", 4_210);
        assert_eq!(line, "3/30 · 2026-10-27T10-00-00Z-7.wav · 4.2 s");
    }

    #[test]
    fn summary_line_tells_why_the_run_ended() {
        let path = Path::new(r"C:\x\report.html");
        let done = summary_line(2, 2, End::Done, path);
        assert_eq!(done, r"Done: recordings 2 of 2; report: C:\x\report.html");
        let interrupted = summary_line(3, 30, End::Interrupted, path);
        assert!(interrupted.contains("Ctrl+C") && interrupted.contains("recordings 3 of 30"), "{interrupted}");
    }

    #[test]
    fn stop_wording_is_about_steps_and_the_exit_points_to_the_report() {
        assert!(STOPPING.contains("current step") && STOPPING.contains("again"), "{STOPPING}");
        let line = exit_line(Path::new(r"C:\x\report.html"));
        assert!(line.contains("at once") && line.contains(r"C:\x\report.html"), "{line}");
    }

    #[test]
    fn run_folder_name_is_a_safe_utc_time() {
        assert_eq!(run_stamp(1_700_000_000), "2023-11-14T22-13-20Z");
    }
}
