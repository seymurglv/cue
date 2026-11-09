//! CUE helper command: the usage summary, the leak scan of a build, and pinning a language model for the short list
//! (`llm::catalog`). Not part of the product. CUE holds no API key, and neither does this.
mod leak;

use anyhow::{Context, Result, bail};
use cue_core::usage::Summary;

const USAGE: &str = "cue-cli usage [YYYY-MM]\ncue-cli leak-scan <file or folder>...\ncue-cli pin-llm <owner/repository>";

/// A Hugging Face repository as `pin-llm` takes it: `owner/repository`, nothing else.
fn repository(arg: &str) -> Option<&str> {
    let arg = arg.trim();
    let (owner, name) = arg.split_once('/')?;
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (plain(owner) && plain(name)).then_some(arg)
}

/// `repo` pinned into a temporary folder; its manifest printed for `llm::catalog`.
async fn pin_llm(repo: &str) -> Result<()> {
    let dir = std::env::temp_dir().join(format!("cue-pin-llm-{}", std::process::id()));
    let http = reqwest::Client::new();
    let pinned = cue_engines::hub::pin_llm(&http, repo, &dir).await;
    let _ = std::fs::remove_dir_all(&dir);
    match pinned {
        Ok(m) => println!("{}", serde_json::to_string_pretty(&m)?),
        Err(cue_engines::hub::LlmPinError::NotRunnable(why)) => bail!("{repo} cannot run in CUE: {why:?}"),
        Err(cue_engines::hub::LlmPinError::Failed(e)) => bail!("{repo}: {e}"),
    }
    Ok(())
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

/// The `cue-cli usage` summary for a month: dictations, audio, latency and rows per mode. "—" where there is nothing
/// to divide by.
fn usage_report(month: &str, s: &Summary) -> String {
    let ms = |v: u64| if v == 0 { "—".to_owned() } else { format!("{v} ms") };
    let mut report = format!(
        "Month {month} (UTC): dictations {}, audio {:.1} min\n\
         Average latency to paste: {}, for audio up to {} s: {}\n",
        s.dictations,
        s.audio_minutes,
        ms(s.avg_latency_ms),
        cue_core::usage::SHORT_AUDIO_MS / 1_000,
        ms(s.avg_latency_short_ms)
    );
    if !s.modes.is_empty() {
        report.push_str("By mode:\n");
        for m in &s.modes {
            report.push_str(&format!("  {:<9} {:>5} rows\n", m.mode, m.rows));
        }
    }
    report
}

/// "YYYY-MM" with a month of 01–12.
fn valid_month(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().chain(&b[5..]).all(u8::is_ascii_digit)
        && (1..=12).contains(&(10 * (b[5] - b'0') + (b[6] - b'0')))
}

/// Usage summary for a month from `%LOCALAPPDATA%\CUE\usage.csv`.
fn show_usage(month: &str) -> Result<()> {
    let base = cue_core::paths::local_app_data().context("LOCALAPPDATA is not set or is not an absolute path")?;
    let path = cue_core::paths::usage_csv(&base);
    // No file yet means there were no dictations. A foreign encoding after an edit in a text editor is no reason to fail.
    let csv = match std::fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
    };
    print!("{}", usage_report(month, &cue_core::usage::summarize(&csv, month)));
    Ok(())
}

/// A crash prints where it happened, never its message: a message can quote the string it was working on (a text
/// the leak scan looks for).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let (file, line) = info.location().map_or(("?", 0), |at| (at.file(), at.line()));
        eprintln!("cue-cli: crash (panic) at {file}:{line}");
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["usage"] => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
            show_usage(&cue_core::usage::utc_timestamp(now)[..7])?;
        }
        ["usage", month] if valid_month(month) => show_usage(month)?,
        ["leak-scan", paths @ ..] if !paths.is_empty() => std::process::exit(leak::run(paths)),
        ["pin-llm", repo] => match repository(repo) {
            Some(repo) => pin_llm(repo).await?,
            None => usage(),
        },
        _ => usage(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::usage::ModeTotal;

    #[test]
    fn pin_llm_args_are_one_repository() {
        assert_eq!(repository("Qwen/Qwen3-1.7B-GGUF"), Some("Qwen/Qwen3-1.7B-GGUF"));
        assert_eq!(repository(" unsloth/gemma-4-E4B-it-GGUF "), Some("unsloth/gemma-4-E4B-it-GGUF"));
        for bad in ["", "Qwen", "a/b/c", "https://huggingface.co/a/b", "a /b", "/b", "a/", "a/b?x=1"] {
            assert_eq!(repository(bad), None, "{bad:?}");
        }
        assert!(USAGE.contains("cue-cli pin-llm <owner/repository>"));
    }

    /// `keys` went with the API keys: CUE holds none.
    #[test]
    fn usage_names_no_keys_command() {
        assert!(!USAGE.contains("keys"), "{USAGE}");
        assert!(USAGE.contains("cue-cli usage [YYYY-MM]") && USAGE.contains("cue-cli leak-scan"));
    }

    #[test]
    fn usage_report_empty_month() {
        assert_eq!(
            usage_report("2026-10", &Summary::default()),
            "Month 2026-10 (UTC): dictations 0, audio 0.0 min\n\
             Average latency to paste: —, for audio up to 20 s: —\n"
        );
    }

    fn mode(mode: &str, rows: u64) -> ModeTotal {
        ModeTotal { mode: mode.into(), rows }
    }

    #[test]
    fn usage_report_with_data() {
        let s = Summary {
            dictations: 3,
            audio_minutes: 1.5,
            avg_latency_ms: 1_200,
            avg_latency_short_ms: 1_100,
            modes: vec![mode("dictate", 120), mode("translate", 14), mode("warmup", 9)],
        };
        assert_eq!(
            usage_report("2026-10", &s),
            "Month 2026-10 (UTC): dictations 3, audio 1.5 min\n\
             Average latency to paste: 1200 ms, for audio up to 20 s: 1100 ms\n\
             By mode:\n  \
               dictate     120 rows\n  \
               translate    14 rows\n  \
               warmup        9 rows\n"
        );
        assert!(!usage_report("2026-10", &s).contains('$'), "no money");
    }

    #[test]
    fn month_argument_is_validated() {
        for ok in ["2026-10", "1999-01", "2026-12"] {
            assert!(valid_month(ok), "{ok}");
        }
        for bad in ["", "2026-1", "2026-13", "2026-00", "26-10", "2026/10", "2026-10-01", "abcd-ef", "２０２６-10", " 2026-10"] {
            assert!(!valid_month(bad), "{bad:?}");
        }
    }
}
