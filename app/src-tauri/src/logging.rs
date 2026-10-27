//! Log: events, per-stage latencies, error codes. No dictation text, no audio, no keys.
//! Kept for 7 days and at most 10 MB: pruned at startup and once a day.
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;

const MAX_AGE_DAYS: u64 = 7;
const MAX_TOTAL: u64 = 10 * 1024 * 1024;
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// At trace level tungstenite writes the whole handshake request, including `Authorization: Bearer <key>`.
/// The filter is hard-coded: no environment variable changes it.
const FILTER: &str = "info,tungstenite=off,tokio_tungstenite=off";

/// Guard of the background log writer: lives until exit; `flush` writes the queue to disk.
static GUARD: Mutex<Option<WorkerGuard>> = Mutex::new(None);

/// Deletes log files (`cue.log*`) older than `max_age_days` and everything over `max_total` bytes, oldest first.
/// The newest file, the one being written now, is never deleted for size. Other files in the folder are left alone.
pub fn prune(dir: &Path, max_age_days: u64, max_total: u64) -> std::io::Result<()> {
    let mut logs: Vec<(SystemTime, u64, std::path::PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("cue.log"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then_some(())?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    // Newest first: the size is counted from them, the old ones over the limit go.
    logs.sort_by_key(|l| std::cmp::Reverse(l.0));
    let cutoff = SystemTime::now().checked_sub(Duration::from_secs(max_age_days.saturating_mul(86_400)));
    let mut total = 0u64;
    for (i, (modified, len, path)) in logs.into_iter().enumerate() {
        total = total.saturating_add(len);
        let old = cutoff.is_some_and(|c| modified < c);
        let over = i > 0 && total > max_total;
        if old || over {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// Log in `dir` (one file per day) and its pruning. Call once, before the other threads,
/// after `install_panic_hook`.
pub fn init(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    if let Err(e) = prune(dir, MAX_AGE_DAYS, MAX_TOTAL) {
        eprintln!("CUE: old logs not deleted: {e}");
    }
    let (writer, guard) = tracing_appender::non_blocking(tracing_appender::rolling::daily(dir, "cue.log"));
    *GUARD.lock().unwrap_or_else(|p| p.into_inner()) = Some(guard);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .with_thread_names(true)
        .with_env_filter(tracing_subscriber::EnvFilter::new(FILTER))
        .try_init()
        .map_err(|e| anyhow::anyhow!("log: {e}"))?;
    spawn_daily_prune(dir.to_path_buf());
    Ok(())
}

/// Write the log to disk before exit. Lines after this call are lost.
pub fn flush() {
    drop(GUARD.lock().unwrap_or_else(|p| p.into_inner()).take());
}

/// A panic goes to the log: thread and code location. The panic message is written nowhere: it may contain
/// dictation text. Installed first thing and always, even if the log then fails to open (otherwise Rust's default
/// hook would print the panic message to stderr); without a log the thread and location go to stderr.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        let (file, line) = info.location().map_or(("?", 0), |at| (at.file(), at.line()));
        if tracing::dispatcher::has_been_set() {
            tracing::error!(thread = name, file, line, "crash (panic)");
        } else {
            eprintln!("CUE: crash (panic) in thread {name}, {file}:{line}");
        }
    }));
}

fn spawn_daily_prune(dir: std::path::PathBuf) {
    let spawned = std::thread::Builder::new().name("cue-log-prune".into()).spawn(move || {
        loop {
            std::thread::sleep(PRUNE_EVERY);
            if let Err(e) = prune(&dir, MAX_AGE_DAYS, MAX_TOTAL) {
                tracing::warn!("log: old files not deleted: {e}");
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("log: prune thread not started: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("cue-logs-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn mk(dir: &Path, name: &str, bytes: usize, days_ago: u64) {
        let p = dir.join(name);
        std::fs::write(&p, vec![b'x'; bytes]).unwrap();
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(days_ago * 86_400)).unwrap();
    }

    fn left(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }

    #[test]
    fn removes_old_and_oversized_logs_only() {
        let dir = temp_dir("prune");
        mk(&dir, "cue.log.2026-10-01", 10, 25);
        mk(&dir, "cue.log.2026-10-20", 600, 6);
        mk(&dir, "cue.log.2026-10-25", 600, 1);
        mk(&dir, "cue.log.2026-10-26", 100, 0);
        mk(&dir, "notes.txt", 10, 30);
        prune(&dir, 7, 1_000).unwrap();
        assert_eq!(left(&dir), vec!["cue.log.2026-10-25", "cue.log.2026-10-26", "notes.txt"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn newest_log_stays_even_if_too_big() {
        let dir = temp_dir("newest");
        mk(&dir, "cue.log.2026-10-25", 50, 2);
        mk(&dir, "cue.log.2026-10-26", 5_000, 1);
        prune(&dir, 7, 1_000).unwrap();
        assert_eq!(left(&dir), vec!["cue.log.2026-10-26"], "the file being written is not deleted for size");
        // For age it is deleted too: CUE did not run for a week, a new file starts today.
        mk(&dir, "cue.log.2026-10-26", 10, 8);
        prune(&dir, 7, 1_000).unwrap();
        assert!(left(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_dir_is_an_error_not_a_panic() {
        let dir = temp_dir("missing").join("absent");
        assert!(prune(&dir, 7, 1_000).is_err());
    }
}
