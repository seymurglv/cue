//! CUE helper command: API keys, the usage summary and the leak scan of a build. Not part of the product.
//!
//! A key's value is never printed — only its mask.
use std::io::Write;

mod leak;

use anyhow::{Context, Result, bail};
use cue_core::usage::Summary;
use cue_engines::EngineError;
use cue_engines::keys::{self, KeyName};

const USAGE: &str = "cue-cli keys set <soniox|anthropic|openrouter>\ncue-cli keys show\ncue-cli keys check\n\
                     cue-cli usage [YYYY-MM]\ncue-cli leak-scan <file or folder>...";

fn usage() -> ! {
    eprintln!("{USAGE}");
    std::process::exit(2)
}

/// A `keys show` line: the mask, "—" if there is no key, or the store failure text (it holds no secret).
fn show_line(name: KeyName, read: &Result<Option<String>, EngineError>) -> String {
    let shown = match read {
        Ok(Some(value)) => keys::mask(value),
        Ok(None) => "—".to_owned(),
        Err(e) => format!("error: {e}"),
    };
    format!("{:<11} {shown}", name.as_str())
}

/// Time limit for one `keys check` probe: a network that "swallows" requests gives a "timeout" line, not an endless wait.
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A `keys check` line: the result of reading the key, with the result of a test request in place of the key itself.
/// "works" or the refusal reason; "—" if there is no key; the store failure text.
fn check_line(name: KeyName, probed: &Result<Option<Result<(), EngineError>>, EngineError>) -> String {
    let shown = match probed {
        Ok(Some(Ok(()))) => "works".to_owned(),
        Ok(Some(Err(e))) => e.to_string(),
        Ok(None) => "—".to_owned(),
        Err(e) => format!("error: {e}"),
    };
    format!("{:<11} {shown}", name.as_str())
}

/// The `cue-cli usage` summary for a month. "—" where there is nothing to divide by.
///
/// "Per dictation" is the month's whole cost (failed attempts and cache warm-ups included) divided by the dictations
/// that produced text: what a user really costs. "Per hour of audio" is for comparing with providers' prices.
/// Web search is named only when there was some: until "Ask" it is always 0.
fn usage_report(month: &str, s: &Summary) -> String {
    let ms = |v: u64| if v == 0 { "—".to_owned() } else { format!("{v} ms") };
    let total = s.stt_usd + s.refine_usd + s.search_usd;
    let search = if s.search_usd > 0.0 { format!(", web search ${:.4}", s.search_usd) } else { String::new() };
    let per_dictation = if s.dictations == 0 { "—".to_owned() } else { format!("${:.5}", total / s.dictations as f64) };
    let per_hour = if s.audio_minutes > 0.0 { format!("${:.4}", total * 60.0 / s.audio_minutes) } else { "—".to_owned() };
    let mut report = format!(
        "Month {month} (UTC): dictations {}, audio {:.1} min\n\
         Transcription ${:.4}, refine ${:.4}{search}, total ${total:.4}\n\
         Per dictation {per_dictation}, per hour of audio {per_hour}\n\
         Average latency to paste: {}, for audio up to {} s: {}\n",
        s.dictations,
        s.audio_minutes,
        s.stt_usd,
        s.refine_usd,
        ms(s.avg_latency_ms),
        cue_core::usage::SHORT_AUDIO_MS / 1_000,
        ms(s.avg_latency_short_ms)
    );
    if !s.modes.is_empty() {
        report.push_str("By mode:\n");
        for m in &s.modes {
            report.push_str(&format!("  {:<9} {:>5} rows   ${:.4}\n", m.mode, m.rows, m.usd));
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

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["keys", "set", name] => {
            // The typed name is not echoed back: if a key was pasted instead of the name by mistake, it stays off the screen.
            let Some(k) = KeyName::parse(name) else {
                bail!(
                    "unknown key name; expected one of: {}",
                    KeyName::ALL.map(KeyName::as_str).join(", ")
                )
            };
            // The prompt is printed by std (WriteConsoleW), not by rpassword: rpassword writes UTF-8 bytes to CONOUT$,
            // and with console code page 866 non-ASCII text turns into mojibake.
            eprint!("{} API key (input hidden): ", k.as_str());
            std::io::stderr().flush().context("could not print the prompt")?;
            let raw = rpassword::read_password().context("could not read the input")?;
            let value = raw.trim();
            if value.is_empty() {
                bail!("an empty key is not saved");
            }
            keys::set(k, value)?;
            println!("Saved: {} = {}", k.as_str(), keys::mask(value));
        }
        ["keys", "show"] => {
            let mut failed = false;
            for k in KeyName::ALL {
                let read = keys::lookup(k);
                failed |= read.is_err();
                println!("{}", show_line(k, &read));
            }
            if failed {
                bail!("could not read keys from Windows Credential Manager");
            }
        }
        ["keys", "check"] => {
            let http = reqwest::Client::builder().timeout(CHECK_TIMEOUT).build().context("could not create the HTTP client")?;
            let mut failed = false;
            for k in KeyName::ALL {
                // The key is not in a check error's text: `cue_engines::check` strips it out.
                let probed = match keys::lookup(k) {
                    Ok(Some(key)) => Ok(Some(match k {
                        KeyName::Soniox => cue_engines::check::soniox(&http, &key).await,
                        KeyName::Anthropic => cue_engines::check::anthropic(&http, &key).await,
                        KeyName::OpenRouter => cue_engines::check::openrouter(&http, &key).await,
                    })),
                    Ok(None) => Ok(None),
                    Err(e) => Err(e),
                };
                failed |= matches!(probed, Err(_) | Ok(Some(Err(_))));
                println!("{}", check_line(k, &probed));
            }
            if failed {
                bail!("not all keys passed the check");
            }
        }
        ["usage"] => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
            show_usage(&cue_core::usage::utc_timestamp(now)[..7])?;
        }
        ["usage", month] if valid_month(month) => show_usage(month)?,
        ["leak-scan", paths @ ..] if !paths.is_empty() => std::process::exit(leak::run(paths)),
        _ => usage(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::usage::ModeTotal;

    #[test]
    fn show_line_present_key_is_masked() {
        let key = "sk-ant-api03-abcdefghijklmnop-f3a9".to_string();
        let line = show_line(KeyName::Anthropic, &Ok(Some(key.clone())));
        assert_eq!(line, "anthropic   sk-ant…f3a9");
        assert!(!line.contains(&key));
    }

    #[test]
    fn show_line_openrouter_key_is_masked() {
        let key = "sk-or-v1-0123456789abcdef0123456789abcdef".to_string();
        let line = show_line(KeyName::OpenRouter, &Ok(Some(key.clone())));
        assert_eq!(line, "openrouter  sk-or-…cdef");
        assert!(!line.contains(&key));
        assert_eq!(check_line(KeyName::OpenRouter, &Ok(None)), "openrouter  —");
    }

    #[test]
    fn usage_names_every_key() {
        for k in KeyName::ALL {
            assert!(USAGE.contains(k.as_str()), "usage text lacks {}: {USAGE}", k.as_str());
        }
    }

    #[test]
    fn show_line_missing_key_is_dash() {
        assert_eq!(show_line(KeyName::Soniox, &Ok(None)), "soniox      —");
    }

    #[test]
    fn show_line_store_failure_is_visible() {
        let err = EngineError::Service("could not read the soniox key: boom".into());
        assert_eq!(
            show_line(KeyName::Soniox, &Err(err)),
            "soniox      error: service unavailable: could not read the soniox key: boom"
        );
    }

    #[test]
    fn check_line_working_key() {
        assert_eq!(check_line(KeyName::Soniox, &Ok(Some(Ok(())))), "soniox      works");
    }

    #[test]
    fn check_line_refused_key_shows_reason() {
        let refused = EngineError::KeyProblem("401: invalid x-api-key".into());
        assert_eq!(
            check_line(KeyName::Anthropic, &Ok(Some(Err(refused)))),
            "anthropic   key, balance or limit: 401: invalid x-api-key"
        );
    }

    #[test]
    fn check_line_timeout_is_visible() {
        assert_eq!(check_line(KeyName::Soniox, &Ok(Some(Err(EngineError::Timeout)))), "soniox      timeout");
    }

    #[test]
    fn check_line_missing_key_is_dash() {
        assert_eq!(check_line(KeyName::Anthropic, &Ok(None)), "anthropic   —");
    }

    #[test]
    fn check_line_store_failure_is_visible() {
        let err = EngineError::Service("could not read the soniox key: boom".into());
        assert_eq!(
            check_line(KeyName::Soniox, &Err(err)),
            "soniox      error: service unavailable: could not read the soniox key: boom"
        );
    }

    #[test]
    fn usage_report_empty_month() {
        assert_eq!(
            usage_report("2026-10", &Summary::default()),
            "Month 2026-10 (UTC): dictations 0, audio 0.0 min\n\
             Transcription $0.0000, refine $0.0000, total $0.0000\n\
             Per dictation —, per hour of audio —\n\
             Average latency to paste: —, for audio up to 20 s: —\n"
        );
    }

    fn mode(mode: &str, rows: u64, usd: f64) -> ModeTotal {
        ModeTotal { mode: mode.into(), rows, usd }
    }

    #[test]
    fn usage_report_with_data() {
        let s = Summary {
            dictations: 3,
            audio_minutes: 1.5,
            stt_usd: 0.0123,
            refine_usd: 0.0456,
            search_usd: 0.0,
            avg_latency_ms: 1_200,
            avg_latency_short_ms: 1_100,
            modes: vec![mode("dictate", 120, 0.0412), mode("translate", 14, 0.0051), mode("warmup", 9, 0.0013)],
        };
        // Per dictation: 0.0579 / 3; per hour of audio: 0.0579 / (1.5 / 60).
        assert_eq!(
            usage_report("2026-10", &s),
            "Month 2026-10 (UTC): dictations 3, audio 1.5 min\n\
             Transcription $0.0123, refine $0.0456, total $0.0579\n\
             Per dictation $0.01930, per hour of audio $2.3160\n\
             Average latency to paste: 1200 ms, for audio up to 20 s: 1100 ms\n\
             By mode:\n  \
               dictate     120 rows   $0.0412\n  \
               translate    14 rows   $0.0051\n  \
               warmup        9 rows   $0.0013\n"
        );
    }

    /// Web searches of "Ask" are part of the total; the line names them only when there were any.
    #[test]
    fn usage_report_counts_web_search_in_the_total() {
        let s = Summary { dictations: 2, stt_usd: 0.01, refine_usd: 0.02, search_usd: 0.03, ..Summary::default() };
        let report = usage_report("2026-11", &s);
        assert!(report.contains("Transcription $0.0100, refine $0.0200, web search $0.0300, total $0.0600\n"), "{report}");
        assert!(report.contains("Per dictation $0.03000,"), "{report}");
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
