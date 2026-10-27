//! usage.csv: the cost of every dictation, to price a future subscription. No dictated text is stored here.
//!
//! One row is one attempt. After "Retry" a dictation has two rows (the failed attempt and the retry):
//! both are paid for. In the monthly summary:
//! - cost is the sum over all rows;
//! - audio is the sum over all rows (paid seconds; a retry transcribes the audio again);
//! - "dictations" are the rows whose outcome delivered text: `inserted` and `no_field`;
//! - the average "second press → paste" latency counts only `inserted` rows
//!   (cancelled, empty and failed rows have 0 there and would understate the latency target figure);
//!   separately, over pastes with audio up to `SHORT_AUDIO_MS` (the target figure itself).
//!
//! `ts` is UTC, and the summary month is a UTC month too: that way it matches the providers' invoices.
//!
//! Every column is a number, a time, an outcome, an engine, a transcription path or a model. String fields are
//! still escaped by CSV rules (the user may type a model name in by hand), and control characters in them
//! are replaced with a space: one record is always one line of the file. The summary skips corrupt and
//! partially written lines.
use crate::flow::Outcome;
use crate::settings::{Pricing, SttEngineKind};
use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// How the attempt's audio was transcribed; the price depends on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttPath {
    /// Soniox stream only.
    Stream,
    /// The stream broke off: the same audio once more as a Soniox async batch.
    StreamThenBatch,
    /// Soniox async batch only: "Retry" (saved audio). A stream that never connected
    /// is also recorded as `Batch` on retry.
    Batch,
}

impl SttPath {
    /// Value of the `stt_path` column.
    pub fn as_str(self) -> &'static str {
        match self {
            SttPath::Stream => "stream",
            SttPath::StreamThenBatch => "stream+batch",
            SttPath::Batch => "batch",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageRecord {
    pub ts: String,
    pub audio_ms: u64,
    pub stt_engine: SttEngineKind,
    pub stt_path: SttPath,
    pub refine_model: String,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Summary {
    pub dictations: u64,
    pub audio_minutes: f64,
    pub stt_usd: f64,
    pub refine_usd: f64,
    /// "Second press → paste", `inserted` rows only. 0 means there were no pastes.
    pub avg_latency_ms: u64,
    /// The same for audio no longer than `SHORT_AUDIO_MS`: the latency target figure (~30 words in ≤ 1.5 s).
    pub avg_latency_short_ms: u64,
}

pub const HEADER: &str = "ts,audio_ms,stt_engine,stt_path,stt_usd,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,refine_usd,latency_ms,outcome";

/// The bound of a "short" dictation for the latency target: ~30 words fit into 20 s of audio.
pub const SHORT_AUDIO_MS: u64 = 20_000;

const COLUMNS: usize = 13;
const INSERTED: &str = "inserted";
const NO_FIELD: &str = "no_field";

/// Value of the `outcome` column for a flow outcome. The summary relies on `inserted` and `no_field`,
/// so the outcome string must come from here rather than be spelled out by hand.
pub fn outcome_name(o: Outcome) -> &'static str {
    match o {
        Outcome::Inserted => INSERTED,
        Outcome::NoField => NO_FIELD,
        Outcome::Cancelled => "cancelled",
        Outcome::Empty => "empty",
        Outcome::Failed => "failed",
        Outcome::Discarded => "discarded",
    }
}

/// The stream costs the engine's price, the batch the Soniox async price, "stream + batch" the sum of both.
pub fn stt_cost(p: &Pricing, engine: SttEngineKind, path: SttPath, audio_ms: u64) -> f64 {
    let hours = audio_ms as f64 / 3_600_000.0;
    let stream = match engine {
        SttEngineKind::Soniox => p.soniox_rt_per_hour,
    };
    let per_hour = match path {
        SttPath::Stream => stream,
        SttPath::StreamThenBatch => stream + p.soniox_async_per_hour,
        SttPath::Batch => p.soniox_async_per_hour,
    };
    hours * per_hour
}

/// The cost of a refine request at the model's prices from settings.
/// A model missing from the prices costs 0.0: the core has no log, so the warning (once, when `claude_price` is None)
/// is written by the app itself.
pub fn refine_cost(p: &Pricing, model: &str, input: u64, cache_read: u64, cache_write: u64, output: u64) -> f64 {
    let Some(m) = p.claude_price(model) else { return 0.0 };
    (input as f64 * m.input_per_mtok
        + cache_read as f64 * m.cache_read_per_mtok
        + cache_write as f64 * m.cache_write_per_mtok
        + output as f64 * m.output_per_mtok)
        / 1_000_000.0
}

/// "2023-11-14T22:13:20Z" without external libraries (the civil-from-days algorithm).
pub fn utc_timestamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

fn engine_name(e: SttEngineKind) -> &'static str {
    match e {
        SttEngineKind::Soniox => "soniox",
    }
}

/// A CSV string field: control characters become spaces; a comma or a quote puts the field in quotes, quotes doubled.
fn csv_field(s: &str) -> Cow<'_, str> {
    let s: Cow<'_, str> = if s.chars().any(char::is_control) {
        Cow::Owned(s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect())
    } else {
        Cow::Borrowed(s)
    };
    if s.contains([',', '"']) { Cow::Owned(format!("\"{}\"", s.replace('"', "\"\""))) } else { s }
}

/// One record, without a trailing newline.
pub fn line(r: &UsageRecord, p: &Pricing) -> String {
    format!(
        "{},{},{},{},{:.6},{},{},{},{},{},{:.6},{},{}",
        csv_field(&r.ts),
        r.audio_ms,
        engine_name(r.stt_engine),
        r.stt_path.as_str(),
        stt_cost(p, r.stt_engine, r.stt_path, r.audio_ms),
        csv_field(&r.refine_model),
        r.input_tokens,
        r.cache_read_tokens,
        r.cache_write_tokens,
        r.output_tokens,
        refine_cost(p, &r.refine_model, r.input_tokens, r.cache_read_tokens, r.cache_write_tokens, r.output_tokens),
        r.latency_ms,
        outcome_name(r.outcome)
    )
}

/// Appends a record; writes the header if the file is missing or empty. If the previous record was cut off
/// (no newline at the end of the file), the new one starts on a new line instead of gluing onto the fragment.
/// Everything appended goes to disk in a single write.
pub fn append(path: &Path, r: &UsageRecord, p: &Pricing) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().read(true).append(true).create(true).open(path)?;
    let len = f.metadata()?.len();
    let mut out = String::new();
    if len == 0 {
        out.push_str(HEADER);
        out.push('\n');
    } else {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(len - 1))?;
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            out.push('\n');
        }
    }
    out.push_str(&line(r, p));
    out.push('\n');
    f.write_all(out.as_bytes())
}

/// The part of a line the summary needs. `None`: the line is corrupt or partially written.
struct Row {
    ts: String,
    audio_ms: u64,
    stt_usd: f64,
    refine_usd: f64,
    latency_ms: u64,
    outcome: String,
}

impl Row {
    fn parse(line: &str) -> Option<Row> {
        let mut c = split_record(line)?;
        if c.len() != COLUMNS {
            return None;
        }
        let usd = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        Some(Row {
            audio_ms: c[1].parse().ok()?,
            stt_usd: usd(&c[4])?,
            refine_usd: usd(&c[10])?,
            latency_ms: c[11].parse().ok()?,
            outcome: std::mem::take(&mut c[12]),
            ts: std::mem::take(&mut c[0]),
        })
    }
}

/// Summary for the month `month`, given as "2026-10". The counting rules are in the module docs.
pub fn summarize(csv: &str, month: &str) -> Summary {
    let mut s = Summary::default();
    let (mut all, mut short) = (Average::default(), Average::default());
    for row in csv.lines().filter_map(Row::parse) {
        if !row.ts.strip_prefix(month).is_some_and(|rest| rest.starts_with('-')) {
            continue;
        }
        s.audio_minutes += row.audio_ms as f64 / 60_000.0;
        s.stt_usd += row.stt_usd;
        s.refine_usd += row.refine_usd;
        match row.outcome.as_str() {
            INSERTED => {
                s.dictations += 1;
                all.add(row.latency_ms);
                if row.audio_ms <= SHORT_AUDIO_MS {
                    short.add(row.latency_ms);
                }
            }
            NO_FIELD => s.dictations += 1,
            _ => {}
        }
    }
    s.avg_latency_ms = all.get();
    s.avg_latency_short_ms = short.get();
    s
}

/// Average of whole ms; a huge garbage value does not overflow the sum.
#[derive(Default)]
struct Average {
    sum: u64,
    count: u64,
}

impl Average {
    fn add(&mut self, v: u64) {
        self.sum = self.sum.saturating_add(v);
        self.count += 1;
    }

    /// 0 if there were no values.
    fn get(&self) -> u64 {
        self.sum.checked_div(self.count).unwrap_or(0)
    }
}

/// One CSV line (RFC 4180, no newlines inside fields). `None`: the quoting is broken.
fn split_record(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut chars = line.chars().peekable();
    loop {
        let mut field = String::new();
        let at_end = if chars.peek() == Some(&'"') {
            chars.next();
            loop {
                match chars.next()? {
                    '"' if chars.peek() == Some(&'"') => {
                        chars.next();
                        field.push('"');
                    }
                    '"' => break,
                    c => field.push(c),
                }
            }
            match chars.next() {
                None => true,
                Some(',') => false,
                Some(_) => return None,
            }
        } else {
            loop {
                match chars.next() {
                    None => break true,
                    Some(',') => break false,
                    Some('"') => return None,
                    Some(c) => field.push(c),
                }
            }
        };
        fields.push(field);
        if at_end {
            return Some(fields);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> UsageRecord {
        UsageRecord {
            ts: "2026-10-27T09:00:00Z".into(),
            audio_ms: 30_000,
            stt_engine: SttEngineKind::Soniox,
            stt_path: SttPath::Stream,
            refine_model: "claude-haiku-5-5".into(),
            input_tokens: 1_000,
            cache_read_tokens: 2_000,
            cache_write_tokens: 0,
            output_tokens: 200,
            latency_ms: 900,
            outcome: Outcome::Inserted,
        }
    }

    #[test]
    fn costs() {
        let p = Pricing::default();
        let hour = 3_600_000;
        assert!((stt_cost(&p, SttEngineKind::Soniox, SttPath::Stream, hour) - 0.12).abs() < 1e-9);
        assert!((stt_cost(&p, SttEngineKind::Soniox, SttPath::StreamThenBatch, hour) - 0.22).abs() < 1e-9, "stream + batch");
        assert!((stt_cost(&p, SttEngineKind::Soniox, SttPath::Batch, hour) - 0.10).abs() < 1e-9, "batch");
        let c = refine_cost(&p, "claude-haiku-5-5", 1_000_000, 1_000_000, 0, 1_000_000);
        assert!((c - (0.10 + 0.01 + 0.50)).abs() < 1e-9);
        assert_eq!(refine_cost(&p, "unknown-model", 1_000, 0, 0, 1_000), 0.0);
    }

    #[test]
    fn timestamps() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn csv_line_has_no_text_and_all_columns() {
        let l = line(&rec(), &Pricing::default());
        assert_eq!(l.split(',').count(), HEADER.split(',').count());
        assert!(l.starts_with("2026-10-27T09:00:00Z,30000,soniox,stream,0.001000,claude-haiku-5-5,1000,2000,0,200,"));
        assert!(l.ends_with(",900,inserted"));
    }

    #[test]
    fn append_writes_header_once_and_summary_counts_month() {
        let path = std::env::temp_dir().join(format!("cue-usage-{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let p = Pricing::default();
        append(&path, &rec(), &p).unwrap();
        let mut other = rec();
        other.ts = "2026-09-30T23:59:59Z".into();
        append(&path, &other, &p).unwrap();
        let mut slow = rec();
        slow.latency_ms = 1_500;
        append(&path, &slow, &p).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().filter(|l| *l == HEADER).count(), 1);
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.dictations, 2);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9);
        assert_eq!(s.avg_latency_ms, 1_200);
        assert!(s.stt_usd > 0.0 && s.refine_usd > 0.0);
        std::fs::remove_file(path).unwrap();
    }

    // ---- Counting rules and edge cases ----

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cue-usage-{}-{name}.csv", std::process::id()))
    }

    fn with(outcome: Outcome, latency_ms: u64) -> UsageRecord {
        UsageRecord { outcome, latency_ms, ..rec() }
    }

    fn csv_of(rows: &[UsageRecord]) -> String {
        let p = Pricing::default();
        let mut csv = format!("{HEADER}\n");
        for r in rows {
            csv.push_str(&line(r, &p));
            csv.push('\n');
        }
        csv
    }

    #[test]
    fn outcome_names_match_flow_outcomes() {
        assert_eq!(outcome_name(Outcome::Inserted), "inserted");
        assert_eq!(outcome_name(Outcome::NoField), "no_field");
        assert_eq!(outcome_name(Outcome::Cancelled), "cancelled");
        assert_eq!(outcome_name(Outcome::Empty), "empty");
        assert_eq!(outcome_name(Outcome::Failed), "failed");
        assert_eq!(outcome_name(Outcome::Discarded), "discarded");
    }

    /// The record holds the flow's outcome type; the file gets its stable name.
    #[test]
    fn outcome_is_written_by_its_stable_name() {
        let l = line(&with(Outcome::NoField, 0), &Pricing::default());
        assert!(l.ends_with(",0,no_field"), "{l}");
        assert_eq!(summarize(&format!("{HEADER}\n{l}\n"), "2026-10").dictations, 1);
    }

    /// Transcription path: stream, stream + batch, batch only; each has its own price.
    #[test]
    fn stt_paths_have_stable_names_and_prices() {
        let p = Pricing::default();
        let hour = 3_600_000;
        assert_eq!(SttPath::Stream.as_str(), "stream");
        assert_eq!(SttPath::StreamThenBatch.as_str(), "stream+batch");
        assert_eq!(SttPath::Batch.as_str(), "batch");
        // Default prices are checked in `costs`; here, that each path takes its own price from settings:
        // stream uses realtime, batch uses async, "stream + batch" uses both.
        let edited = Pricing { soniox_rt_per_hour: 1.0, soniox_async_per_hour: 0.25, ..Pricing::default() };
        let cost = |path| stt_cost(&edited, SttEngineKind::Soniox, path, hour);
        assert!((cost(SttPath::Stream) - 1.0).abs() < 1e-9);
        assert!((cost(SttPath::StreamThenBatch) - 1.25).abs() < 1e-9);
        assert!((cost(SttPath::Batch) - 0.25).abs() < 1e-9);
        let l = line(&UsageRecord { stt_path: SttPath::StreamThenBatch, ..rec() }, &p);
        assert!(l.starts_with("2026-10-27T09:00:00Z,30000,soniox,stream+batch,0.001833,"), "{l}");
    }

    /// A separate average latency for pastes with up to 20 s of audio: the latency target figure.
    #[test]
    fn short_dictation_latency_is_averaged_separately() {
        let at = |audio_ms, outcome, latency_ms| UsageRecord { audio_ms, ..with(outcome, latency_ms) };
        let csv = csv_of(&[
            at(SHORT_AUDIO_MS, Outcome::Inserted, 1_000),
            at(SHORT_AUDIO_MS + 1, Outcome::Inserted, 3_000),
            at(5_000, Outcome::Inserted, 1_400),
            at(5_000, Outcome::NoField, 9_999),
            at(5_000, Outcome::Failed, 0),
        ]);
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.avg_latency_ms, 1_800);
        assert_eq!(s.avg_latency_short_ms, 1_200);
        let long_only = summarize(&csv_of(&[at(60_000, Outcome::Inserted, 2_000)]), "2026-10");
        assert_eq!((long_only.avg_latency_ms, long_only.avg_latency_short_ms), (2_000, 0));
    }

    /// The average latency counts only `inserted` rows; cancelled, empty and failed rows have 0 there.
    #[test]
    fn average_latency_counts_only_inserted_rows() {
        let csv = csv_of(&[
            with(Outcome::Inserted, 900),
            with(Outcome::Cancelled, 0),
            with(Outcome::Empty, 0),
            with(Outcome::Failed, 0),
            with(Outcome::Discarded, 0),
            with(Outcome::NoField, 5_000),
            with(Outcome::Inserted, 1_500),
        ]);
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.avg_latency_ms, 1_200, "latency skewed by rows without a paste");
        // "Dictations" are rows where the text went somewhere: two pastes and one "no field".
        assert_eq!(s.dictations, 3);
    }

    #[test]
    fn no_inserted_rows_means_zero_latency_without_panic() {
        let s = summarize(&csv_of(&[with(Outcome::Cancelled, 0), with(Outcome::NoField, 700)]), "2026-10");
        assert_eq!((s.dictations, s.avg_latency_ms), (1, 0));
        assert_eq!(summarize("", "2026-10"), Summary::default());
        assert_eq!(summarize(HEADER, "2026-10"), Summary::default());
    }

    /// After "Retry" there are two rows for one dictation. Both attempts are paid for,
    /// the audio counts twice (it was paid for twice), but there is one dictation.
    #[test]
    fn retry_costs_both_attempts_but_counts_one_dictation() {
        let p = Pricing::default();
        let failed = UsageRecord {
            refine_model: String::new(),
            input_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            ..with(Outcome::Failed, 0)
        };
        // "Retry" transcribes the saved audio as a batch only (TranscribeSaved), with no stream.
        let retry = UsageRecord { stt_path: SttPath::Batch, ..with(Outcome::Inserted, 1_100) };
        let s = summarize(&csv_of(&[failed.clone(), retry.clone()]), "2026-10");
        assert_eq!(s.dictations, 1);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9, "audio of both attempts: {}", s.audio_minutes);
        let half_minute_h = 30_000.0 / 3_600_000.0;
        let stt = half_minute_h * (p.soniox_rt_per_hour + p.soniox_async_per_hour);
        assert!((s.stt_usd - stt).abs() < 1e-6, "{} != {stt}", s.stt_usd);
        let refine = refine_cost(&p, &retry.refine_model, 1_000, 2_000, 0, 200);
        assert!((s.refine_usd - refine).abs() < 1e-6, "{} != {refine}", s.refine_usd);
        assert_eq!(s.avg_latency_ms, 1_100);
    }

    /// String fields are escaped; one record is one line of the file.
    #[test]
    fn string_fields_are_escaped_and_round_trip() {
        let p = Pricing::default();
        let mut r = rec();
        r.refine_model = "my,\"odd\"\r\nmodel".into();
        let l = line(&r, &p);
        assert!(!l.contains('\n') && !l.contains('\r'), "newline inside a record: {l:?}");
        let fields = split_record(&l).expect("the line parses");
        assert_eq!(HEADER.split(',').count(), COLUMNS);
        assert_eq!(fields.len(), COLUMNS);
        assert_eq!(fields[5], "my,\"odd\"  model");
        assert_eq!(fields[12], "inserted");
        let s = summarize(&format!("{HEADER}\n{l}\n"), "2026-10");
        assert_eq!(s.dictations, 1);
        assert_eq!(s.avg_latency_ms, 900);
    }

    #[test]
    fn split_record_follows_csv_quoting() {
        assert_eq!(split_record("a,b,,c").unwrap(), ["a", "b", "", "c"]);
        assert_eq!(split_record("\"a,b\",\"x\"\"y\",\"\"").unwrap(), ["a,b", "x\"y", ""]);
        assert_eq!(split_record("a,").unwrap(), ["a", ""]);
        assert_eq!(split_record("\"unterminated,a"), None);
        assert_eq!(split_record("\"a\"b,c"), None);
        assert_eq!(split_record("a\"b,c"), None);
    }

    /// Corrupt and partially written lines are skipped, with no panic.
    #[test]
    fn summarize_skips_malformed_and_partial_lines() {
        let good = line(&rec(), &Pricing::default());
        let huge = line(&with(Outcome::Inserted, u64::MAX), &Pricing::default());
        let csv = [
            HEADER,
            &good,
            "2026-10-27T09:00:00Z,abc,soniox,stream,0.001000,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,NaN,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,-1,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,0.001000,\"m,1,2,0,200,0.000220,900,inserted",
            "",
            "garbage",
            &huge,
            &huge,
            "2026-10-27T09:00:00Z,30000,son",
        ]
        .join("\r\n");
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.dictations, 3, "only complete lines are counted");
        assert!((s.audio_minutes - 1.5).abs() < 1e-9);
        assert!(s.stt_usd.is_finite() && s.stt_usd > 0.0);
    }

    #[test]
    fn month_must_match_whole_year_and_month() {
        let mut nov = rec();
        nov.ts = "2026-11-01T00:00:00Z".into();
        let csv = csv_of(&[rec(), nov]);
        assert_eq!(summarize(&csv, "2026-10").dictations, 1);
        assert_eq!(summarize(&csv, "2026-11").dictations, 1);
        assert_eq!(summarize(&csv, "2026-1").dictations, 0);
        assert_eq!(summarize(&csv, "2025-10").dictations, 0);
    }

    #[test]
    fn append_after_partial_last_line_starts_a_new_line() {
        let path = tmp("partial");
        std::fs::write(&path, format!("{HEADER}\n2026-10-27T09:00:00Z,30000,son")).unwrap();
        append(&path, &rec(), &Pricing::default()).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert!(csv.ends_with('\n'));
        assert_eq!(csv.lines().count(), 3, "{csv:?}");
        assert_eq!(summarize(&csv, "2026-10").dictations, 1, "the new record must not glue onto the fragment");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn append_to_empty_file_writes_header_and_creates_dirs() {
        let dir = tmp("dir");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("usage.csv");
        append(&path, &rec(), &Pricing::default()).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first.lines().next(), Some(HEADER));

        let empty = tmp("empty");
        std::fs::write(&empty, "").unwrap();
        append(&empty, &rec(), &Pricing::default()).unwrap();
        let csv = std::fs::read_to_string(&empty).unwrap();
        assert_eq!(csv.lines().collect::<Vec<_>>(), [HEADER, &line(&rec(), &Pricing::default())]);
        std::fs::remove_file(empty).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn timestamps_extremes_do_not_panic() {
        assert_eq!(utc_timestamp(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(utc_timestamp(1_792_972_800), "2026-10-26T00:00:00Z");
        assert!(utc_timestamp(u64::MAX).ends_with('Z'));
    }
}
