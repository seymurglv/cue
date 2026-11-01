//! usage.csv: the cost of every dictation, to price a future subscription. No dictated text is stored here.
//!
//! One row is one attempt. After "Retry" a dictation has two rows (the failed attempt and the retry):
//! both are paid for. In the monthly summary:
//! - cost is the sum over all rows;
//! - audio is the sum over all rows (paid seconds; a retry transcribes the audio again);
//! - "dictations" are the rows whose outcome delivered text: `inserted` and `no_field`;
//! - the average "second press → paste" latency counts only `inserted` rows
//!   (cancelled, empty and failed rows have 0 there and would understate the latency target figure);
//!   separately, over pastes with audio up to `SHORT_AUDIO_MS` (the target figure itself);
//! - rows and dollars per mode, in the order the modes first appear.
//!
//! The last three columns, `mode`, `web_searches` and `search_usd`, came with the modes (spec 2 §10). Old rows
//! without these columns read as dictation; the header of an existing file is not rewritten. A refine cache warm-up
//! is a row of its own (`warmup`, outcome `discarded`, no audio): it costs money but is not a dictation; `answered`
//! (an "Ask" answer shown in full) is not a dictation either.
//!
//! The last two, `words` and `day`, came with the main window's Today (`stats`): how many words the attempt wrote
//! (`count_words` of its text; 0 for one that wrote nothing, a question or a warm-up) and its local date. They hold no
//! text either. Rows written before them have 13 or 16 columns, and the summary reads all three widths.
//!
//! `ts` is UTC, and the summary month is a UTC month too: that way it matches the providers' invoices.
//!
//! Every column is a number, a time, an outcome, an engine, a transcription path, a model or a mode. String fields are
//! still escaped by CSV rules (the user may type a model name in by hand), and control characters in them
//! are replaced with a space: one record is always one line of the file. The summary skips corrupt and
//! partially written lines.
use crate::flow::Outcome;
use crate::settings::{Pricing, SttEngineKind};
use crate::ui::Mode;
use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// How the attempt's audio was transcribed. Rows of earlier versions name Soniox's paths (`stream`, `stream+batch`,
/// `batch`); they are read as text and keep the cost they were written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttPath {
    /// On this computer: recognition costs nothing.
    Local,
}

impl SttPath {
    /// Value of the `stt_path` column.
    pub fn as_str(self) -> &'static str {
        match self {
            SttPath::Local => "local",
        }
    }
}

/// What the row paid for: a dictation in one of the modes, or a refine cache warm-up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageMode {
    Dictate,
    Translate,
    Ask,
    Warmup,
}

impl UsageMode {
    /// Value of the `mode` column.
    pub fn as_str(self) -> &'static str {
        match self {
            UsageMode::Dictate => DICTATE,
            UsageMode::Translate => "translate",
            UsageMode::Ask => "ask",
            UsageMode::Warmup => "warmup",
        }
    }
}

impl From<Mode> for UsageMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Dictate => UsageMode::Dictate,
            Mode::Translate => UsageMode::Translate,
            Mode::Ask => UsageMode::Ask,
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
    pub mode: UsageMode,
    pub web_searches: u32,
    /// How many words the attempt wrote ([`count_words`]); 0 when it wrote none, or was a question or a warm-up.
    pub words: u32,
    /// The local date of the attempt, `YYYY-MM-DD` (`ts` is UTC).
    pub day: String,
}

/// The words of a text: whitespace-separated runs that hold a letter or a digit ("—" and "…" alone are not words).
pub fn count_words(text: &str) -> u32 {
    let n = text.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Summary {
    pub dictations: u64,
    pub audio_minutes: f64,
    pub stt_usd: f64,
    pub refine_usd: f64,
    pub search_usd: f64,
    /// "Second press → paste", `inserted` rows only. 0 means there were no pastes.
    pub avg_latency_ms: u64,
    /// The same for audio no longer than `SHORT_AUDIO_MS`: the latency target figure (~30 words in ≤ 1.5 s).
    pub avg_latency_short_ms: u64,
    /// Rows and dollars per value of the `mode` column, in the order of first appearance.
    pub modes: Vec<ModeTotal>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModeTotal {
    pub mode: String,
    pub rows: u64,
    /// Transcription, refine and search together.
    pub usd: f64,
}

pub const HEADER: &str = "ts,audio_ms,stt_engine,stt_path,stt_usd,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,refine_usd,latency_ms,outcome,mode,web_searches,search_usd,words,day";

/// The bound of a "short" dictation for the latency target: ~30 words fit into 20 s of audio.
pub const SHORT_AUDIO_MS: u64 = 20_000;

pub(crate) const COLUMNS: usize = 18;
/// A row written before Today: no `words`, `day`.
pub(crate) const COLUMNS_V2: usize = 16;
/// A row written before the modes: no `mode`, `web_searches`, `search_usd`.
pub(crate) const COLUMNS_V1: usize = 13;
pub(crate) const DICTATE: &str = "dictate";
pub(crate) const INSERTED: &str = "inserted";
pub(crate) const NO_FIELD: &str = "no_field";

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
        Outcome::Answered => "answered",
    }
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
        SttEngineKind::Parakeet => "parakeet",
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
        "{},{},{},{},{:.6},{},{},{},{},{},{:.6},{},{},{},{},{:.6},{},{}",
        csv_field(&r.ts),
        r.audio_ms,
        engine_name(r.stt_engine),
        r.stt_path.as_str(),
        0.0,
        csv_field(&r.refine_model),
        r.input_tokens,
        r.cache_read_tokens,
        r.cache_write_tokens,
        r.output_tokens,
        refine_cost(p, &r.refine_model, r.input_tokens, r.cache_read_tokens, r.cache_write_tokens, r.output_tokens),
        r.latency_ms,
        outcome_name(r.outcome),
        r.mode.as_str(),
        r.web_searches,
        f64::from(r.web_searches) * p.web_search_usd,
        r.words,
        csv_field(&r.day)
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
    search_usd: f64,
    latency_ms: u64,
    outcome: String,
    mode: String,
}

impl Row {
    fn parse(line: &str) -> Option<Row> {
        let mut c = split_record(line)?;
        let usd = |s: &str| s.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        let (mode, search_usd) = match c.len() {
            COLUMNS_V1 => (DICTATE.to_owned(), 0.0),
            COLUMNS_V2 | COLUMNS => (std::mem::take(&mut c[13]), usd(&c[15])?),
            _ => return None,
        };
        Some(Row {
            audio_ms: c[1].parse().ok()?,
            stt_usd: usd(&c[4])?,
            refine_usd: usd(&c[10])?,
            search_usd,
            latency_ms: c[11].parse().ok()?,
            outcome: std::mem::take(&mut c[12]),
            ts: std::mem::take(&mut c[0]),
            mode,
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
        s.search_usd += row.search_usd;
        let usd = row.stt_usd + row.refine_usd + row.search_usd;
        match s.modes.iter_mut().find(|m| m.mode == row.mode) {
            Some(m) => {
                m.rows += 1;
                m.usd += usd;
            }
            None => s.modes.push(ModeTotal { mode: row.mode, rows: 1, usd }),
        }
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
pub(crate) fn split_record(line: &str) -> Option<Vec<String>> {
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
            stt_engine: SttEngineKind::Parakeet,
            stt_path: SttPath::Local,
            refine_model: "claude-haiku-5-5".into(),
            input_tokens: 1_000,
            cache_read_tokens: 2_000,
            cache_write_tokens: 0,
            output_tokens: 200,
            latency_ms: 900,
            outcome: Outcome::Inserted,
            mode: UsageMode::Dictate,
            web_searches: 0,
            words: 12,
            day: "2026-10-27".into(),
        }
    }

    #[test]
    fn words_are_runs_with_a_letter_or_a_digit() {
        assert_eq!(count_words("Привет, мир! 2 340 — ok…"), 5);
        assert_eq!(count_words(" — … "), 0);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("GitHub-репозиторий
и	ещё"), 3);
    }

    #[test]
    fn a_row_ends_with_its_words_and_day() {
        let l = line(&rec(), &Pricing::default());
        assert!(l.ends_with(",12,2026-10-27"), "{l}");
        assert_eq!(split_record(&l).unwrap().len(), COLUMNS);
        assert_eq!(split_record(HEADER).unwrap().len(), COLUMNS);
    }

    #[test]
    fn costs() {
        let p = Pricing::default();
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
        assert!(l.starts_with("2026-10-27T09:00:00Z,30000,parakeet,local,0.000000,claude-haiku-5-5,1000,2000,0,200,"));
        assert!(l.ends_with(",900,inserted,dictate,0,0.000000,12,2026-10-27"), "{l}");
    }

    // ---- Modes (spec 2 §10) ----

    /// The header before the modes: files written by earlier versions start with it.
    const HEADER_V1: &str = "ts,audio_ms,stt_engine,stt_path,stt_usd,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,refine_usd,latency_ms,outcome";

    fn rec_with(ts: &str, outcome: Outcome) -> UsageRecord {
        UsageRecord {
            ts: ts.into(),
            audio_ms: 1_000,
            stt_engine: SttEngineKind::Parakeet,
            stt_path: SttPath::Local,
            refine_model: "claude-haiku-5-5".into(),
            input_tokens: 10,
            cache_read_tokens: 900,
            cache_write_tokens: 0,
            output_tokens: 5,
            latency_ms: 900,
            outcome,
            mode: UsageMode::Dictate,
            web_searches: 0,
            words: 3,
            day: ts.get(..10).unwrap_or_default().into(),
        }
    }

    /// A file of earlier versions — rows of 13 and 16 columns — and today's rows read together: the old rows count as
    /// they did.
    #[test]
    fn the_summary_reads_rows_of_every_width() {
        let p = Pricing::default();
        let v1 = "2026-11-01T10:00:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted";
        let v2 = "2026-11-01T10:01:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted,dictate,0,0.000000";
        let now = line(&rec_with("2026-11-01T10:02:00Z", Outcome::Inserted), &p);
        let s = summarize(&[HEADER_V1, v1, v2, &now].join("\n"), "2026-11");
        assert_eq!(s.dictations, 3);
        assert_eq!((s.modes.len(), s.modes[0].mode.as_str(), s.modes[0].rows), (1, "dictate", 3));
        assert!((s.modes[0].usd - (s.stt_usd + s.refine_usd)).abs() < 1e-12);
    }

    #[test]
    fn modes_have_stable_names() {
        assert_eq!(UsageMode::Dictate.as_str(), "dictate");
        assert_eq!(UsageMode::Translate.as_str(), "translate");
        assert_eq!(UsageMode::Ask.as_str(), "ask");
        assert_eq!(UsageMode::Warmup.as_str(), "warmup");
        assert_eq!(UsageMode::from(Mode::Dictate), UsageMode::Dictate);
        assert_eq!(UsageMode::from(Mode::Translate), UsageMode::Translate);
        assert_eq!(UsageMode::from(Mode::Ask), UsageMode::Ask);
        assert_eq!(HEADER.split(',').count(), COLUMNS);
        assert!(HEADER.starts_with(HEADER_V1), "the new columns go at the end");
    }

    #[test]
    fn new_rows_carry_mode_and_searches() {
        let p = Pricing::default();
        let mut r = rec_with("2026-11-01T10:00:00Z", Outcome::Inserted);
        r.mode = UsageMode::Translate;
        r.web_searches = 2;
        let l = line(&r, &p);
        assert!(l.ends_with(",inserted,translate,2,0.020000,3,2026-11-01"), "{l}");
        let s = summarize(&format!("{HEADER}\n{l}\n"), "2026-11");
        assert!((s.search_usd - 0.02).abs() < 1e-9, "{}", s.search_usd);
        let row_usd = s.stt_usd + s.refine_usd + s.search_usd;
        assert_eq!(s.modes.len(), 1);
        assert_eq!((s.modes[0].mode.as_str(), s.modes[0].rows), ("translate", 1));
        assert!((s.modes[0].usd - row_usd).abs() < 1e-9, "the mode's dollars include the search: {}", s.modes[0].usd);
    }

    #[test]
    fn old_rows_read_as_dictation() {
        let csv = format!("{HEADER_V1}\n2026-11-01T10:00:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted\n");
        let s = summarize(&csv, "2026-11");
        assert_eq!(s.dictations, 1);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("dictate", 1)]);
        assert_eq!(s.search_usd, 0.0);
    }

    #[test]
    fn warmup_rows_cost_money_but_are_not_dictations() {
        let p = Pricing::default();
        let mut r = rec_with("2026-11-01T10:00:00Z", Outcome::Discarded);
        (r.audio_ms, r.latency_ms, r.cache_read_tokens, r.cache_write_tokens, r.mode) = (0, 0, 0, 900, UsageMode::Warmup);
        let csv = format!("{HEADER}\n{}\n", line(&r, &p));
        let s = summarize(&csv, "2026-11");
        assert_eq!(s.dictations, 0);
        assert!(s.refine_usd > 0.0);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("warmup", 1)]);
    }

    #[test]
    fn answered_rows_cost_money_but_are_not_dictations() {
        let p = Pricing::default();
        let mut r = rec_with("2026-11-01T10:00:00Z", Outcome::Answered);
        (r.mode, r.web_searches) = (UsageMode::Ask, 1);
        let l = line(&r, &p);
        assert!(l.contains(",answered,ask,1,0.010000"), "{l}");
        let s = summarize(&format!("{HEADER}\n{l}\n"), "2026-11");
        assert_eq!(s.dictations, 0);
        assert!((s.search_usd - 0.01).abs() < 1e-9);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("ask", 1)]);
    }

    /// A file started before the modes keeps its old header: old and new rows are summed together, modes in the order
    /// they first appear.
    #[test]
    fn old_and_new_rows_in_one_file_are_summed_by_mode() {
        let p = Pricing::default();
        let old = "2026-11-01T10:00:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted";
        let mut warm = rec_with("2026-11-02T10:00:00Z", Outcome::Discarded);
        warm.mode = UsageMode::Warmup;
        let mut tr = rec_with("2026-11-02T10:01:00Z", Outcome::Inserted);
        tr.mode = UsageMode::Translate;
        let dict = rec_with("2026-11-02T10:02:00Z", Outcome::NoField);
        let csv = [HEADER_V1.to_owned(), old.to_owned(), line(&warm, &p), line(&tr, &p), line(&dict, &p)].join("\n");
        let s = summarize(&csv, "2026-11");
        assert_eq!(s.dictations, 3);
        let modes: Vec<_> = s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect();
        assert_eq!(modes, [("dictate", 2), ("warmup", 1), ("translate", 1)]);
        let by_mode: f64 = s.modes.iter().map(|m| m.usd).sum();
        assert!((by_mode - (s.stt_usd + s.refine_usd + s.search_usd)).abs() < 1e-9, "modes add up to the total");
    }

    /// Only the two known widths are rows: 13 columns (before the modes) and 16.
    #[test]
    fn rows_of_other_widths_are_skipped() {
        let old = "2026-11-01T10:00:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted";
        for tail in [",dictate", ",dictate,0", ",dictate,0,0.000000,x", ",dictate,0,-1", ",dictate,0,NaN"] {
            let csv = format!("{HEADER}\n{old}{tail}\n");
            assert_eq!(summarize(&csv, "2026-11"), Summary::default(), "{tail}");
        }
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
        assert!(s.stt_usd == 0.0 && s.refine_usd > 0.0, "recognition is free, the cleanup is paid");
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
        assert!(l.contains(",0,no_field,dictate,0,0.000000,"), "{l}");
        assert_eq!(summarize(&format!("{HEADER}\n{l}\n"), "2026-10").dictations, 1);
    }

    /// Recognition on this computer costs nothing; a row of an earlier version keeps the cost it was written with.
    #[test]
    fn local_rows_cost_nothing_for_recognition_and_old_rows_keep_their_cost() {
        assert_eq!(SttPath::Local.as_str(), "local");
        let new = line(&rec(), &Pricing::default());
        assert!(new.starts_with("2026-10-27T09:00:00Z,30000,parakeet,local,0.000000,"), "{new}");
        let old = "2026-10-27T09:00:00Z,30000,soniox,stream+batch,0.001833,claude-haiku-5-5,1000,2000,0,200,0.000220,900,inserted";
        let s = summarize(&format!("{HEADER}
{old}
{new}
"), "2026-10");
        assert_eq!(s.dictations, 2);
        assert!((s.stt_usd - 0.001833).abs() < 1e-9, "{}", s.stt_usd);
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
        // "Retry" recognizes the saved audio again (TranscribeSaved).
        let retry = with(Outcome::Inserted, 1_100);
        let s = summarize(&csv_of(&[failed.clone(), retry.clone()]), "2026-10");
        assert_eq!(s.dictations, 1);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9, "audio of both attempts: {}", s.audio_minutes);
        assert_eq!(s.stt_usd, 0.0, "recognition on this computer");
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
        assert!(s.refine_usd.is_finite() && s.refine_usd > 0.0);
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
