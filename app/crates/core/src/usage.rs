//! usage.csv: the numbers of every dictation — its audio, the tokens a model reported, its latency, its outcome, the
//! words it wrote and its day — for the main window's Today (`stats`) and `cue-cli usage`. No dictated text is stored
//! here, and no money: CUE pays for nothing.
//!
//! One row is one attempt. After "Retry" a dictation has two rows (the failed attempt and the retry). In the monthly
//! summary:
//! - audio is the sum over all rows (a retry transcribes the audio again);
//! - "dictations" are the rows whose outcome delivered text: `inserted` and `no_field`;
//! - the average "second press → paste" latency counts only `inserted` rows
//!   (cancelled, empty and failed rows have 0 there and would understate the latency target figure);
//!   separately, over pastes with audio up to `SHORT_AUDIO_MS` (the target figure itself);
//! - rows per mode, in the order the modes first appear.
//!
//! A row of this version has 15 columns ([`HEADER`]). Rows of earlier versions have 13, 16 or 18, with the dollars
//! they were priced at (`stt_usd`, `refine_usd`, `search_usd`): they are read with those columns passed over, whatever
//! they hold ([`fields`]). A row of 13 was written before the modes (no `mode`, `web_searches`) and reads as dictation;
//! one of 16 before Today (no `words`, `day`). The header of an existing file is not rewritten. A cache warm-up of an
//! earlier version is a row of its own (`warmup`, outcome `discarded`), and `answered` (an "Ask" answer shown in full):
//! neither is a dictation.
//!
//! `words` is how many words the attempt wrote (`count_words` of its text; 0 for one that wrote nothing or a question),
//! `day` its local date. `ts` is UTC, and the summary month is a UTC month too.
//!
//! Every column is a number, a time, an outcome, an engine, a transcription path, a model or a mode. String fields are
//! still escaped by CSV rules (the user may type a model name in by hand), and control characters in them
//! are replaced with a space: one record is always one line of the file. The summary skips corrupt and
//! partially written lines.
use crate::flow::Outcome;
use crate::settings::SttEngineKind;
use crate::ui::Mode;
use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// How the attempt's audio was transcribed. Rows of earlier versions name Soniox's paths (`stream`, `stream+batch`,
/// `batch`); they are read as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SttPath {
    /// On this computer.
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

/// The mode the row's dictation ran in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsageMode {
    Dictate,
    Translate,
    Ask,
}

impl UsageMode {
    /// Value of the `mode` column.
    pub fn as_str(self) -> &'static str {
        match self {
            UsageMode::Dictate => DICTATE,
            UsageMode::Translate => "translate",
            UsageMode::Ask => "ask",
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
    /// How many words the attempt wrote ([`count_words`]); 0 when it wrote none, or was a question.
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
    /// "Second press → paste", `inserted` rows only. 0 means there were no pastes.
    pub avg_latency_ms: u64,
    /// The same for audio no longer than `SHORT_AUDIO_MS`: the latency target figure (~30 words in ≤ 1.5 s).
    pub avg_latency_short_ms: u64,
    /// Rows per value of the `mode` column, in the order of first appearance.
    pub modes: Vec<ModeTotal>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModeTotal {
    pub mode: String,
    pub rows: u64,
}

pub const HEADER: &str = "ts,audio_ms,stt_engine,stt_path,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,latency_ms,outcome,mode,web_searches,words,day";

/// The bound of a "short" dictation for the latency target: ~30 words fit into 20 s of audio.
pub const SHORT_AUDIO_MS: u64 = 20_000;

pub(crate) const COLUMNS: usize = 15;
/// A row written with its dollars, after Today.
pub(crate) const COLUMNS_V3: usize = 18;
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
pub fn line(r: &UsageRecord) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        csv_field(&r.ts),
        r.audio_ms,
        engine_name(r.stt_engine),
        r.stt_path.as_str(),
        csv_field(&r.refine_model),
        r.input_tokens,
        r.cache_read_tokens,
        r.cache_write_tokens,
        r.output_tokens,
        r.latency_ms,
        outcome_name(r.outcome),
        r.mode.as_str(),
        r.web_searches,
        r.words,
        csv_field(&r.day)
    )
}

/// Appends a record; writes the header if the file is missing or empty. If the previous record was cut off
/// (no newline at the end of the file), the new one starts on a new line instead of gluing onto the fragment.
/// Everything appended goes to disk in a single write.
pub fn append(path: &Path, r: &UsageRecord) -> std::io::Result<()> {
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
    out.push_str(&line(r));
    out.push('\n');
    f.write_all(out.as_bytes())
}

/// The columns of a row the summary and Today read, from a row of any width CUE wrote; the dollars of an earlier
/// version's row are passed over. `words` and `day` are `None` in a row written before Today. `None`: a width CUE never
/// wrote.
pub(crate) struct Fields<'a> {
    pub ts: &'a str,
    pub audio_ms: &'a str,
    pub latency_ms: &'a str,
    pub outcome: &'a str,
    pub mode: &'a str,
    pub words: Option<&'a str>,
    pub day: Option<&'a str>,
}

pub(crate) fn fields(c: &[String]) -> Option<Fields<'_>> {
    let f = match c.len() {
        COLUMNS => Fields {
            ts: &c[0],
            audio_ms: &c[1],
            latency_ms: &c[9],
            outcome: &c[10],
            mode: &c[11],
            words: Some(&c[13]),
            day: Some(&c[14]),
        },
        COLUMNS_V1 | COLUMNS_V2 | COLUMNS_V3 => Fields {
            ts: &c[0],
            audio_ms: &c[1],
            latency_ms: &c[11],
            outcome: &c[12],
            mode: if c.len() == COLUMNS_V1 { DICTATE } else { &c[13] },
            words: (c.len() == COLUMNS_V3).then(|| c[16].as_str()),
            day: (c.len() == COLUMNS_V3).then(|| c[17].as_str()),
        },
        _ => return None,
    };
    Some(f)
}

/// The part of a line the summary needs. `None`: the line is corrupt or partially written.
struct Row {
    ts: String,
    audio_ms: u64,
    latency_ms: u64,
    outcome: String,
    mode: String,
}

impl Row {
    fn parse(line: &str) -> Option<Row> {
        let c = split_record(line)?;
        let f = fields(&c)?;
        Some(Row {
            audio_ms: f.audio_ms.parse().ok()?,
            latency_ms: f.latency_ms.parse().ok()?,
            outcome: f.outcome.to_owned(),
            ts: f.ts.to_owned(),
            mode: f.mode.to_owned(),
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
        match s.modes.iter_mut().find(|m| m.mode == row.mode) {
            Some(m) => m.rows += 1,
            None => s.modes.push(ModeTotal { mode: row.mode, rows: 1 }),
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

    /// The header before the modes: files written by earlier versions start with it.
    const HEADER_V1: &str = "ts,audio_ms,stt_engine,stt_path,stt_usd,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,refine_usd,latency_ms,outcome";
    /// The header of the version before this one, with its dollars.
    const HEADER_V3: &str = "ts,audio_ms,stt_engine,stt_path,stt_usd,refine_model,input_tokens,cache_read_tokens,cache_write_tokens,output_tokens,refine_usd,latency_ms,outcome,mode,web_searches,search_usd,words,day";
    /// Rows of earlier versions: before the modes (13 columns), before Today (16), with both (18); each with dollars.
    const OLD_V1: &str = "2026-11-01T10:00:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted";
    const OLD_V2: &str = "2026-11-01T10:01:00Z,1000,soniox,stream,0.000033,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted,dictate,0,0.000000";
    const OLD_V3: &str = "2026-11-01T10:01:30Z,1000,parakeet,local,0.000000,claude-haiku-5-5,10,0,0,5,0.000004,900,inserted,dictate,0,0.000000,3,2026-11-01";

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
        assert_eq!(count_words("GitHub-репозиторий\nи\tещё"), 3);
    }

    /// A row holds words and time, never money; a row an earlier version wrote with its dollars still reads, the dollars
    /// left out.
    #[test]
    fn usage_rows_have_no_usd() {
        assert!(!HEADER.contains("usd"), "{HEADER}");
        let l = line(&rec());
        assert_eq!(split_record(&l).unwrap().len(), COLUMNS);
        assert_eq!(l, "2026-10-27T09:00:00Z,30000,parakeet,local,claude-haiku-5-5,1000,2000,0,200,900,inserted,dictate,0,12,2026-10-27");
        let with_cost = "2026-10-27T09:00:00Z,30000,parakeet,local,0.000000,claude-haiku-5-5,1000,2000,0,200,0.000220,900,inserted,dictate,0,0.000000,12,2026-10-27";
        let odd_cost = "2026-10-27T09:01:00Z,30000,soniox,stream,NaN,m,1,2,0,200,-1,900,inserted";
        let s = summarize(&[HEADER_V3, with_cost, odd_cost, &l].join("\n"), "2026-10");
        assert_eq!((s.dictations, s.avg_latency_ms), (3, 900), "a cost field is read past, whatever it holds");
        assert_eq!(s.modes, [ModeTotal { mode: "dictate".into(), rows: 3 }]);
    }

    /// This version's header is the last one's without its dollars, in the same order.
    #[test]
    fn the_header_is_the_old_one_without_its_dollars() {
        let kept: Vec<&str> = HEADER_V3.split(',').filter(|c| !c.ends_with("_usd")).collect();
        assert_eq!(kept.join(","), HEADER);
        assert_eq!(HEADER.split(',').count(), COLUMNS);
        assert_eq!(HEADER_V3.split(',').count(), COLUMNS_V3);
        assert_eq!(HEADER_V1.split(',').count(), COLUMNS_V1);
    }

    #[test]
    fn a_row_ends_with_its_words_and_day() {
        let l = line(&rec());
        assert!(l.ends_with(",12,2026-10-27"), "{l}");
        assert_eq!(split_record(&l).unwrap().len(), COLUMNS);
        assert_eq!(split_record(HEADER).unwrap().len(), COLUMNS);
    }

    #[test]
    fn timestamps() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn csv_line_has_no_text_and_all_columns() {
        let l = line(&rec());
        assert_eq!(l.split(',').count(), HEADER.split(',').count());
        assert!(l.starts_with("2026-10-27T09:00:00Z,30000,parakeet,local,claude-haiku-5-5,1000,2000,0,200,"));
        assert!(l.ends_with(",900,inserted,dictate,0,12,2026-10-27"), "{l}");
    }

    // ---- Modes (spec 2 §10) ----

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

    /// A file of earlier versions — rows of 13, 16 and 18 columns — and today's rows read together: the old rows count
    /// as they did.
    #[test]
    fn the_summary_reads_rows_of_every_width() {
        let now = line(&rec_with("2026-11-01T10:02:00Z", Outcome::Inserted));
        let s = summarize(&[HEADER_V1, OLD_V1, OLD_V2, OLD_V3, &now].join("\n"), "2026-11");
        assert_eq!(s.dictations, 4);
        assert!((s.audio_minutes - 4.0 / 60.0).abs() < 1e-12);
        assert_eq!(s.modes, [ModeTotal { mode: "dictate".into(), rows: 4 }]);
    }

    #[test]
    fn modes_have_stable_names() {
        assert_eq!(UsageMode::Dictate.as_str(), "dictate");
        assert_eq!(UsageMode::Translate.as_str(), "translate");
        assert_eq!(UsageMode::Ask.as_str(), "ask");
        assert_eq!(UsageMode::from(Mode::Dictate), UsageMode::Dictate);
        assert_eq!(UsageMode::from(Mode::Translate), UsageMode::Translate);
        assert_eq!(UsageMode::from(Mode::Ask), UsageMode::Ask);
    }

    #[test]
    fn new_rows_carry_mode_and_searches() {
        let mut r = rec_with("2026-11-01T10:00:00Z", Outcome::Inserted);
        r.mode = UsageMode::Translate;
        r.web_searches = 2;
        let l = line(&r);
        assert!(l.ends_with(",inserted,translate,2,3,2026-11-01"), "{l}");
        let s = summarize(&format!("{HEADER}\n{l}\n"), "2026-11");
        assert_eq!(s.modes, [ModeTotal { mode: "translate".into(), rows: 1 }]);
    }

    #[test]
    fn old_rows_read_as_dictation() {
        let s = summarize(&format!("{HEADER_V1}\n{OLD_V1}\n"), "2026-11");
        assert_eq!(s.dictations, 1);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("dictate", 1)]);
    }

    /// A cache warm-up of an earlier version has a row of its own: its mode reads, it is no dictation.
    #[test]
    fn warmup_rows_of_an_earlier_version_are_not_dictations() {
        let warm = "2026-11-01T10:00:00Z,0,parakeet,local,0.000000,claude-haiku-5-5,0,0,900,0,0.000112,0,discarded,warmup,0,0.000000,0,2026-11-01";
        let s = summarize(&format!("{HEADER_V3}\n{warm}\n"), "2026-11");
        assert_eq!(s.dictations, 0);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("warmup", 1)]);
    }

    #[test]
    fn answered_rows_are_not_dictations() {
        let mut r = rec_with("2026-11-01T10:00:00Z", Outcome::Answered);
        (r.mode, r.web_searches) = (UsageMode::Ask, 1);
        let l = line(&r);
        assert!(l.contains(",answered,ask,1,"), "{l}");
        let s = summarize(&format!("{HEADER}\n{l}\n"), "2026-11");
        assert_eq!(s.dictations, 0);
        assert_eq!(s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect::<Vec<_>>(), vec![("ask", 1)]);
    }

    /// A file started before the modes keeps its old header: old and new rows are counted together, modes in the order
    /// they first appear.
    #[test]
    fn old_and_new_rows_in_one_file_are_counted_by_mode() {
        let warm = "2026-11-02T10:00:00Z,0,parakeet,local,0.000000,claude-haiku-5-5,0,0,900,0,0.000112,0,discarded,warmup,0,0.000000,0,2026-11-02";
        let mut tr = rec_with("2026-11-02T10:01:00Z", Outcome::Inserted);
        tr.mode = UsageMode::Translate;
        let dict = rec_with("2026-11-02T10:02:00Z", Outcome::NoField);
        let csv = [HEADER_V1.to_owned(), OLD_V1.to_owned(), warm.to_owned(), line(&tr), line(&dict)].join("\n");
        let s = summarize(&csv, "2026-11");
        assert_eq!(s.dictations, 3);
        let modes: Vec<_> = s.modes.iter().map(|m| (m.mode.as_str(), m.rows)).collect();
        assert_eq!(modes, [("dictate", 2), ("warmup", 1), ("translate", 1)]);
    }

    /// Only the widths CUE wrote are rows: 13, 16 and 18 columns of earlier versions and this version's 15. What the
    /// dollars of an earlier row hold does not matter.
    #[test]
    fn rows_of_other_widths_are_skipped() {
        for tail in [",dictate", ",dictate,0,0.000000,x", ",dictate,0,0.000000,3,2026-11-01,x", ",dictate,0,0.000000,3,2026-11-01,x,y"] {
            let csv = format!("{HEADER}\n{OLD_V1}{tail}\n");
            assert_eq!(summarize(&csv, "2026-11"), Summary::default(), "{tail}");
        }
        for tail in [",dictate,0,-1", ",dictate,0,NaN"] {
            let csv = format!("{HEADER}\n{OLD_V1}{tail}\n");
            assert_eq!(summarize(&csv, "2026-11").dictations, 1, "{tail}");
        }
    }

    #[test]
    fn append_writes_header_once_and_summary_counts_month() {
        let path = std::env::temp_dir().join(format!("cue-usage-{}.csv", std::process::id()));
        let _ = std::fs::remove_file(&path);
        append(&path, &rec()).unwrap();
        let mut other = rec();
        other.ts = "2026-09-30T23:59:59Z".into();
        append(&path, &other).unwrap();
        let mut slow = rec();
        slow.latency_ms = 1_500;
        append(&path, &slow).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().filter(|l| *l == HEADER).count(), 1);
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.dictations, 2);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9);
        assert_eq!(s.avg_latency_ms, 1_200);
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
        let mut csv = format!("{HEADER}\n");
        for r in rows {
            csv.push_str(&line(r));
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
        let l = line(&with(Outcome::NoField, 0));
        assert!(l.contains(",0,no_field,dictate,0,"), "{l}");
        assert_eq!(summarize(&format!("{HEADER}\n{l}\n"), "2026-10").dictations, 1);
    }

    /// Recognition on this computer; a row of an earlier version names Soniox and still counts.
    #[test]
    fn local_rows_and_soniox_rows_of_earlier_versions_both_count() {
        assert_eq!(SttPath::Local.as_str(), "local");
        let new = line(&rec());
        assert!(new.starts_with("2026-10-27T09:00:00Z,30000,parakeet,local,claude-haiku-5-5,"), "{new}");
        let old = "2026-10-27T09:00:00Z,30000,soniox,stream+batch,0.001833,claude-haiku-5-5,1000,2000,0,200,0.000220,900,inserted";
        let s = summarize(&format!("{HEADER}\n{old}\n{new}\n"), "2026-10");
        assert_eq!(s.dictations, 2);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9);
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

    /// After "Retry" there are two rows for one dictation: the audio counts twice (it was transcribed twice), but there
    /// is one dictation.
    #[test]
    fn a_retry_has_two_rows_but_counts_one_dictation() {
        let failed = UsageRecord {
            refine_model: String::new(),
            input_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            ..with(Outcome::Failed, 0)
        };
        // "Retry" recognizes the saved audio again (TranscribeSaved).
        let retry = with(Outcome::Inserted, 1_100);
        let s = summarize(&csv_of(&[failed, retry]), "2026-10");
        assert_eq!(s.dictations, 1);
        assert!((s.audio_minutes - 1.0).abs() < 1e-9, "audio of both attempts: {}", s.audio_minutes);
        assert_eq!(s.modes, [ModeTotal { mode: "dictate".into(), rows: 2 }]);
        assert_eq!(s.avg_latency_ms, 1_100);
    }

    /// String fields are escaped; one record is one line of the file.
    #[test]
    fn string_fields_are_escaped_and_round_trip() {
        let mut r = rec();
        r.refine_model = "my,\"odd\"\r\nmodel".into();
        let l = line(&r);
        assert!(!l.contains('\n') && !l.contains('\r'), "newline inside a record: {l:?}");
        let fields = split_record(&l).expect("the line parses");
        assert_eq!(fields.len(), COLUMNS);
        assert_eq!(fields[4], "my,\"odd\"  model");
        assert_eq!(fields[10], "inserted");
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

    /// Corrupt and partially written lines are skipped, with no panic; an earlier row's dollars are not looked at.
    #[test]
    fn summarize_skips_malformed_and_partial_lines() {
        let good = line(&rec());
        let huge = line(&with(Outcome::Inserted, u64::MAX));
        let csv = [
            HEADER,
            &good,
            "2026-10-27T09:00:00Z,abc,soniox,stream,0.001000,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,NaN,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,-1,m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,0.001000,\"m,1,2,0,200,0.000220,900,inserted",
            "2026-10-27T09:00:00Z,30000,soniox,stream,0.001000,m,1,2,0,200,0.000220,9x0,inserted",
            "",
            "garbage",
            &huge,
            &huge,
            "2026-10-27T09:00:00Z,30000,son",
        ]
        .join("\r\n");
        let s = summarize(&csv, "2026-10");
        assert_eq!(s.dictations, 5, "only complete lines are counted, whatever their dollars hold");
        assert!((s.audio_minutes - 2.5).abs() < 1e-9);
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
        append(&path, &rec()).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert!(csv.ends_with('\n'));
        assert_eq!(csv.lines().count(), 3, "{csv:?}");
        assert_eq!(summarize(&csv, "2026-10").dictations, 1, "the new record must not glue onto the fragment");
        std::fs::remove_file(path).unwrap();
    }

    /// A file an earlier version started keeps its header; this version's rows go after its rows.
    #[test]
    fn append_to_a_file_of_an_earlier_version_keeps_its_header() {
        let path = tmp("earlier");
        std::fs::write(&path, format!("{HEADER_V3}\n{OLD_V3}\n")).unwrap();
        append(&path, &rec_with("2026-11-01T11:00:00Z", Outcome::Inserted)).unwrap();
        let csv = std::fs::read_to_string(&path).unwrap();
        assert_eq!(csv.lines().next(), Some(HEADER_V3));
        assert_eq!(summarize(&csv, "2026-11").dictations, 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn append_to_empty_file_writes_header_and_creates_dirs() {
        let dir = tmp("dir");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("usage.csv");
        append(&path, &rec()).unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first.lines().next(), Some(HEADER));

        let empty = tmp("empty");
        std::fs::write(&empty, "").unwrap();
        append(&empty, &rec()).unwrap();
        let csv = std::fs::read_to_string(&empty).unwrap();
        assert_eq!(csv.lines().collect::<Vec<_>>(), [HEADER, &line(&rec())]);
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
