//! The history (spec 3 §2): every result of CUE, kept only on this computer, in `%LOCALAPPDATA%\CUE\history`. One file
//! per local day, `YYYY-MM-DD.jsonl`: an entry is one line of JSON, appended as it comes, so a day is never rewritten,
//! and the retention period deletes whole days. There is no database: a search reads the files. Kept for years, the
//! history makes a search slower; an index can come later without changing the format.
//!
//! Only the core writes; the main window reads, and a read never fails: a day that cannot be read has no entries. Only
//! names `YYYY-MM-DD.jsonl` of a real date count, so any other file in the directory is never read or deleted. A line
//! that does not read as an entry (the tail of a write cut off by a power failure, a line of a later version) is
//! skipped: one bad line never hides the rest of its day. Newest first everywhere: days, and entries within a day.
//!
//! The text of an entry is the user's and never goes to the log: `Debug` of an entry shows only its lengths.
//!
//! The recordings of the entries live in the folder `history\audio`, one file per entry named by its id
//! (`audio_file`), for a period of their own that is never longer than the history's
//! (`settings::HistorySettings::audio_period`); `prune_audio` deletes the older ones, and at startup, before anything
//! is recorded, `sweep_audio_temps` deletes what a crash left half written. There too only CUE's own names count —
//! recordings, and recordings still being written — so nothing else in the folder is ever deleted one by one.
use crate::settings::{AudioKeep, Keep};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What gave the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Dictation,
    Translation,
    Ask,
}

/// One result. `ts` is the local time it came, `YYYY-MM-DDTHH:MM:SS.mmm`, and its date names the day file; `id` is
/// unique and starts with `ts` (`audio_file` has its form). `app` is the app the result went to, empty when not known.
/// For "Ask", `question` is the question and `text` the answer or the text the model wrote. `raw` is the text as it was
/// heard, before CUE cleaned it up — for a translation, the speech before it was translated. "Ask" has none (its
/// question is as it was heard), and neither do the entries of earlier versions.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub ts: String,
    pub mode: Mode,
    pub app: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
}

impl std::fmt::Debug for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entry")
            .field("id", &self.id)
            .field("ts", &self.ts)
            .field("mode", &self.mode)
            .field("app", &self.app)
            .field("text_chars", &self.text.chars().count())
            .field("question_chars", &self.question.as_ref().map(|q| q.chars().count()))
            .field("raw_chars", &self.raw.as_ref().map(|r| r.chars().count()))
            .finish()
    }
}

impl Entry {
    /// Whether the text, the question or the text as it was heard holds `query`, which is already folded (`fold`).
    fn matches(&self, query: &str) -> bool {
        let holds = |s: &str| fold(s).contains(query);
        holds(&self.text) || self.question.as_deref().is_some_and(holds) || self.raw.as_deref().is_some_and(holds)
    }
}

/// Text as a search compares it: lowercase, with ё read as е (dictation writes ё where people type е).
fn fold(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

/// One day of the history: `date` is `YYYY-MM-DD`, the entries are newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day {
    pub date: String,
    pub entries: Vec<Entry>,
}

/// Days with entries, newest first; `more` — an earlier day has entries too ("Show earlier").
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Page {
    pub days: Vec<Day>,
    pub more: bool,
}

const EXTENSION: &str = ".jsonl";

/// Appends an entry to the file of its day, creating the directory and the file. The line goes in one write with its
/// line break. If the previous line was cut off (no line break at the end of the file), the new one starts on a line of
/// its own instead of being glued to the fragment and lost with it.
pub fn append(dir: &Path, entry: &Entry) -> io::Result<()> {
    let Some(date) = entry.ts.get(..10).filter(|d| days_from_date(d).is_some()) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the entry's time does not start with a date"));
    };
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    std::fs::create_dir_all(dir)?;
    let mut file = std::fs::OpenOptions::new().read(true).append(true).create(true).open(day_file(dir, date))?;
    let len = file.metadata()?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        file.seek(SeekFrom::Start(len - 1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            line.insert(0, '\n');
        }
    }
    file.write_all(line.as_bytes())
}

/// Today and yesterday, the first page of the window (`today` is `YYYY-MM-DD`); `more` — an earlier day has entries.
pub fn recent(dir: &Path, today: &str) -> Page {
    let Some(yesterday) = date_minus(today, 1) else { return Page::default() };
    let days = dates(dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| *d == today || *d == yesterday)
        .filter_map(|d| day(dir, d))
        .collect();
    Page { days, more: older(dir, &yesterday, 0).more }
}

/// Up to `n_days` days with entries before the day `before` (`YYYY-MM-DD`; anything else gives nothing), newest
/// first: "Show earlier" after the last day shown. Days without a readable entry are passed over and not counted.
pub fn older(dir: &Path, before: &str, n_days: usize) -> Page {
    let mut page = Page::default();
    if days_from_date(before).is_none() {
        return page;
    }
    for date in dates(dir).unwrap_or_default().into_iter().filter(|d| d.as_str() < before) {
        let Some(day) = day(dir, date) else { continue };
        if page.days.len() == n_days {
            page.more = true;
            break;
        }
        page.days.push(day);
    }
    page
}

/// The entries whose text, question or text as it was heard holds `query` (trimmed, in any case, ё and е alike), newest
/// first, at most `limit` of them. A blank query finds nothing.
pub fn search(dir: &Path, query: &str, limit: usize) -> Vec<Day> {
    let query = fold(query.trim());
    let mut days = Vec::new();
    if query.is_empty() {
        return days;
    }
    let mut left = limit;
    for date in dates(dir).unwrap_or_default() {
        if left == 0 {
            break;
        }
        let entries: Vec<Entry> = read_day(dir, &date).into_iter().filter(|e| e.matches(&query)).take(left).collect();
        if !entries.is_empty() {
            left -= entries.len();
            days.push(Day { date, entries });
        }
    }
    days
}

/// The entry with this id. The day its id starts with is looked at first, then the other days, newest first.
pub fn find(dir: &Path, id: &str) -> Option<Entry> {
    let mut dates = dates(dir).unwrap_or_default();
    if let Some(at) = dates.iter().position(|d| id.starts_with(d.as_str())) {
        let own = dates.remove(at);
        dates.insert(0, own);
    }
    dates.iter().find_map(|date| read_day(dir, date).into_iter().find(|e| e.id == id))
}

/// Deletes the days the retention period no longer keeps; returns how many. A period counts back from `today`
/// (`YYYY-MM-DD`): a week keeps the 7 days before today, a month 31, a year 366, and earlier days go. `Off` deletes
/// every day, `Forever` none. A day after today (the clock was moved back) is kept.
pub fn prune(dir: &Path, keep: Keep, today: &str) -> io::Result<usize> {
    let back = match keep {
        Keep::Forever => return Ok(0),
        Keep::Off => return remove_days(dir, |_| true),
        Keep::Year => 366,
        Keep::Month => 31,
        Keep::Week => 7,
    };
    let Some(first_kept) = date_minus(today, back) else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "today is not a date"));
    };
    remove_days(dir, |date| date < first_kept.as_str())
}

/// Deletes every day of the history ("Clear history"); files that are not days stay.
pub fn clear(dir: &Path) -> io::Result<()> {
    remove_days(dir, |_| true).map(drop)
}

/// The date `days` days before `date` (`YYYY-MM-DD`), in the same form; `None` if `date` is not a real date or the
/// result falls before the year 0.
pub fn date_minus(date: &str, days: u32) -> Option<String> {
    date_of(days_from_date(date)? - i64::from(days))
}

/// The extension of a recording.
const AUDIO_EXTENSION: &str = ".ogg";
/// The end of the name of the snapshot of an entry's sound, waiting to be encoded into its recording
/// (`audio_snapshot`).
const SNAPSHOT_EXTENSION: &str = ".wav.tmp";
/// The end of the name of a recording being encoded: the encoder writes it under its name with `.tmp` added.
const ENCODING_EXTENSION: &str = ".ogg.tmp";

/// The file of the recording of the entry `id` in the recordings folder: the id with the `:` of its time as `-`
/// (Windows allows no `:` in a file name; the bench names its recordings the same way), then `.ogg`. `None` for
/// anything but an id as CUE makes them — `YYYY-MM-DDTHH:MM:SS.mmm-n` of a real date and time of day, `n` counting
/// from 1 — so an id from a page never names a path.
pub fn audio_file(id: &str) -> Option<String> {
    audio_name(id).map(|name| name + AUDIO_EXTENSION)
}

/// The snapshot of the sound of the entry `id`, encoded into its recording, next to it: `audio_file` with `.wav.tmp`
/// for `.ogg`. Like a recording still being encoded, `sweep_audio_temps` deletes one that was left over.
pub fn audio_snapshot(id: &str) -> Option<String> {
    audio_name(id).map(|name| name + SNAPSHOT_EXTENSION)
}

/// Which of the files `names` of the recordings folder go when recordings are kept for `keep`, counted back from
/// `today` (`YYYY-MM-DD`) by the date of their entry, like the history's days: a day keeps today's recordings, a week
/// the 7 days before today too, a month 31, `Off` none. A recording of a day after today (the clock was moved back) is
/// kept, and with `today` not a date none goes. Any other name is never picked — a file still being written neither,
/// so recordings may be pruned while one is.
pub fn audio_to_delete<'a>(names: impl IntoIterator<Item = &'a str>, keep: AudioKeep, today: &str) -> Vec<&'a str> {
    let back = match keep {
        AudioKeep::Off => None,
        AudioKeep::Day => Some(0),
        AudioKeep::Week => Some(7),
        AudioKeep::Month => Some(31),
    };
    // The first date kept; no date comes before "", so with `today` not a date nothing is old.
    let first_kept = back.map(|days| date_minus(today, days).unwrap_or_default());
    let old = |date: &str| first_kept.as_deref().is_none_or(|first| date < first);
    names.into_iter().filter(|name| recording_date(name).is_some_and(old)).collect()
}

/// Which of the files `names` of the recordings folder were left half written by a crash: the snapshot of an entry's
/// sound (`audio_snapshot`) and a recording being encoded (`audio_file` with `.tmp` added). Only CUE's own names: any
/// other `.tmp` file is never picked.
pub fn audio_temps<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<&'a str> {
    let ours = |name: &str| {
        let stem = name.strip_suffix(SNAPSHOT_EXTENSION).or_else(|| name.strip_suffix(ENCODING_EXTENSION));
        stem.and_then(stem_date).is_some()
    };
    names.into_iter().filter(|name| ours(name)).collect()
}

/// Deletes from the recordings folder `dir` what `audio_to_delete` picks; returns how many. No folder: nothing to
/// delete. A file that cannot be deleted does not stop the others, as with the days.
pub fn prune_audio(dir: &Path, keep: AudioKeep, today: &str) -> io::Result<usize> {
    let names = file_names(dir)?;
    remove_files(audio_to_delete(names.iter().map(String::as_str), keep, today).into_iter().map(|name| dir.join(name)))
}

/// Deletes from the recordings folder `dir` what `audio_temps` picks; returns how many. Only at startup, before
/// anything is recorded: a snapshot or an encoding still under way would lose its recording. No folder: nothing to
/// delete.
pub fn sweep_audio_temps(dir: &Path) -> io::Result<usize> {
    let names = file_names(dir)?;
    remove_files(audio_temps(names.iter().map(String::as_str)).into_iter().map(|name| dir.join(name)))
}

/// Deletes the recordings folder `dir` with all in it: "Clear history", or the recordings turned off. No folder: done.
pub fn clear_audio(dir: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        done => done,
    }
}

/// The id with the `:` of its time as `-`; `None` if it is not an entry id.
fn audio_name(id: &str) -> Option<String> {
    is_entry_id(id).then(|| id.replace(':', "-"))
}

/// The date of the entry whose recording `name` is (`audio_file`); `None` for any other name.
fn recording_date(name: &str) -> Option<&str> {
    stem_date(name.strip_suffix(AUDIO_EXTENSION)?)
}

/// The date of the entry whose recording is named `stem` and an extension (`audio_name`); `None` for any other stem.
fn stem_date(stem: &str) -> Option<&str> {
    // Back to the id: `audio_file` wrote the two `:` of the time as `-`.
    let id: String = stem.char_indices().map(|(at, c)| if at == 13 || at == 16 { ':' } else { c }).collect();
    (audio_name(&id).as_deref() == Some(stem)).then(|| &stem[..10])
}

/// Whether `id` is an entry id as CUE makes them: `YYYY-MM-DDTHH:MM:SS.mmm-n`, a real date and time of day, and `n`
/// counting from 1 in ASCII digits, without leading zeros.
fn is_entry_id(id: &str) -> bool {
    let Some((ts, count)) = id.split_at_checked(23) else { return false };
    let Some(count) = count.strip_prefix('-') else { return false };
    let digits = count.bytes().all(|c| c.is_ascii_digit());
    (1..=20).contains(&count.len()) && !count.starts_with('0') && digits && is_entry_time(ts)
}

/// Whether `ts` is `YYYY-MM-DDTHH:MM:SS.mmm` of a real date and time of day.
fn is_entry_time(ts: &str) -> bool {
    let b = ts.as_bytes();
    let up_to = |at: usize, max: u8| {
        let (tens, ones) = (b[at], b[at + 1]);
        tens.is_ascii_digit() && ones.is_ascii_digit() && (tens - b'0') * 10 + (ones - b'0') <= max
    };
    b.len() == 23
        && ts.get(..10).is_some_and(|date| days_from_date(date).is_some())
        && (b[10], b[13], b[16], b[19]) == (b'T', b':', b':', b'.')
        && up_to(11, 23)
        && up_to(14, 59)
        && up_to(17, 59)
        && b[20..].iter().all(u8::is_ascii_digit)
}

/// Deletes the day files `old` picks; returns how many.
fn remove_days(dir: &Path, old: impl Fn(&str) -> bool) -> io::Result<usize> {
    remove_files(dates(dir)?.into_iter().filter(|d| old(d)).map(|date| day_file(dir, &date)))
}

/// Deletes the files; returns how many. A file that cannot be deleted (another program holds it open) does not stop
/// the others: the first such error is returned, and the next prune tries again. A file already gone is no error.
fn remove_files(files: impl IntoIterator<Item = PathBuf>) -> io::Result<usize> {
    let mut removed = 0;
    let mut failed = None;
    for file in files {
        match std::fs::remove_file(file) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                failed.get_or_insert(e);
            }
        }
    }
    failed.map_or(Ok(removed), Err)
}

/// The dates of the day files, newest first; no directory — none.
fn dates(dir: &Path) -> io::Result<Vec<String>> {
    let names = file_names(dir)?;
    let mut dates: Vec<String> = names.iter().filter_map(|name| file_date(name)).map(str::to_owned).collect();
    dates.sort_unstable_by(|a, b| b.cmp(a));
    Ok(dates)
}

/// The names of the files in `dir`, not its folders; a name that is not Unicode is left out. No directory — none.
fn file_names(dir: &Path) -> io::Result<Vec<String>> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(listing
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect())
}

/// The date in the name of a day file; `None` for any other name.
fn file_date(name: &str) -> Option<&str> {
    name.strip_suffix(EXTENSION).filter(|date| days_from_date(date).is_some())
}

fn day_file(dir: &Path, date: &str) -> PathBuf {
    dir.join(format!("{date}{EXTENSION}"))
}

/// A day with its entries; `None` when it has none.
fn day(dir: &Path, date: String) -> Option<Day> {
    let entries = read_day(dir, &date);
    (!entries.is_empty()).then_some(Day { date, entries })
}

/// The entries of a day, newest first; a file that cannot be read has none. Lines are split as bytes: a cut-off tail
/// may end inside a character, and that must not spoil the lines before it.
fn read_day(dir: &Path, date: &str) -> Vec<Entry> {
    let Ok(bytes) = std::fs::read(day_file(dir, date)) else { return Vec::new() };
    let mut entries: Vec<Entry> =
        bytes.split(|&b| b == b'\n').filter_map(|line| serde_json::from_slice(line).ok()).collect();
    entries.reverse();
    entries
}

/// Days since 1970-01-01 of a real date written `YYYY-MM-DD` (the days-from-civil algorithm, the inverse of the one in
/// `usage::utc_timestamp`); anything else is `None`.
fn days_from_date(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let number = |digits: &[u8]| {
        digits.iter().try_fold(0i64, |n, &c| c.is_ascii_digit().then(|| n * 10 + i64::from(c - b'0')))
    };
    let (y, m, d) = (number(&b[..4])?, number(&b[5..7])?, number(&b[8..])?);
    if !(1..=12).contains(&m) || !(1..=days_in_month(y, m)).contains(&d) {
        return None;
    }
    // The year starts in March, so that the leap day is the last day of a year.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// `YYYY-MM-DD` of a day counted from 1970-01-01 (the civil-from-days algorithm, as in `usage::utc_timestamp`); `None`
/// outside the years 0–9999, which the form cannot hold.
fn date_of(days: i64) -> Option<String> {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (0..=9999).contains(&y).then(|| format!("{y:04}-{m:02}-{d:02}"))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A fresh directory for one test; a leftover of an earlier run is removed first.
    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-history-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(ts: &str, mode: Mode, text: &str) -> Entry {
        let (id, app) = (format!("{ts}-1"), "Notepad".into());
        Entry { id, ts: ts.into(), mode, app, text: text.into(), question: None, raw: None }
    }

    fn ask(ts: &str, question: &str, answer: &str) -> Entry {
        Entry { question: Some(question.into()), ..entry(ts, Mode::Ask, answer) }
    }

    /// A dictation that gave `text`, heard as `raw`.
    fn heard(ts: &str, text: &str, raw: &str) -> Entry {
        Entry { raw: Some(raw.into()), ..entry(ts, Mode::Dictation, text) }
    }

    fn add(dir: &Path, entries: &[Entry]) {
        for e in entries {
            append(dir, e).unwrap();
        }
    }

    /// Dates and texts of the days, in the order given.
    fn texts(days: &[Day]) -> Vec<(&str, Vec<&str>)> {
        days.iter().map(|d| (d.date.as_str(), d.entries.iter().map(|e| e.text.as_str()).collect())).collect()
    }

    /// The names of the files in the directory, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> =
            std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn append_then_recent_groups_by_day_newest_first() {
        let dir = tmp("recent");
        assert_eq!(recent(&dir, "2026-10-31"), Page::default(), "no directory yet is an empty history");
        add(
            &dir,
            &[
                entry("2026-10-29T09:00:00.000", Mode::Dictation, "two days ago"),
                entry("2026-10-30T08:00:00.000", Mode::Dictation, "yesterday morning"),
                entry("2026-10-30T22:15:00.500", Mode::Translation, "yesterday evening"),
                entry("2026-10-31T07:00:00.000", Mode::Dictation, "today first"),
                ask("2026-10-31T07:30:00.000", "what time is it", "half past seven"),
                entry("2026-10-31T08:00:00.000", Mode::Dictation, "today last"),
            ],
        );
        assert_eq!(names(&dir), ["2026-10-29.jsonl", "2026-10-30.jsonl", "2026-10-31.jsonl"]);
        let page = recent(&dir, "2026-10-31");
        assert_eq!(
            texts(&page.days),
            [
                ("2026-10-31", vec!["today last", "half past seven", "today first"]),
                ("2026-10-30", vec!["yesterday evening", "yesterday morning"]),
            ]
        );
        assert!(page.more, "an earlier day is left");
        let answer = &page.days[0].entries[1];
        assert_eq!(answer, &ask("2026-10-31T07:30:00.000", "what time is it", "half past seven"));

        // One line per entry, the text on it as JSON, a line break inside the text escaped.
        add(&dir, &[entry("2026-11-01T00:00:01.000", Mode::Dictation, "line one\nline two")]);
        let file = std::fs::read_to_string(dir.join("2026-11-01.jsonl")).unwrap();
        assert_eq!(file.lines().count(), 1);
        assert!(file.ends_with('\n'));
        let page = recent(&dir, "2026-11-01");
        assert_eq!(
            texts(&page.days),
            [("2026-11-01", vec!["line one\nline two"]), ("2026-10-31", vec!["today last", "half past seven", "today first"])]
        );
        assert!(page.more);

        // Only today: no yesterday group, and nothing earlier.
        let only = tmp("recent-only-today");
        let alone = entry("2026-10-31T07:00:00.000", Mode::Dictation, "alone");
        add(&only, std::slice::from_ref(&alone));
        let day = Day { date: "2026-10-31".into(), entries: vec![alone] };
        assert_eq!(recent(&only, "2026-10-31"), Page { days: vec![day], more: false });
        // An entry whose time does not start with a date has no file to go to.
        let undated = entry("31.10.2026 07:00", Mode::Dictation, "x");
        assert_eq!(append(&only, &undated).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        std::fs::remove_dir_all(dir).unwrap();
        std::fs::remove_dir_all(only).unwrap();
    }

    #[test]
    fn older_continues_before_a_day_and_reports_more() {
        let dir = tmp("older");
        // Days with entries: 20, 22, 25, 27, 29, 30, 31 October; the 26th has a file with no readable entry.
        for day in [20, 22, 25, 27, 29, 30, 31] {
            add(&dir, &[entry(&format!("2026-10-{day}T10:00:00.000"), Mode::Dictation, &format!("day {day}"))]);
        }
        std::fs::write(dir.join("2026-10-26.jsonl"), "").unwrap();

        let page = older(&dir, "2026-10-30", 3);
        assert_eq!(
            texts(&page.days),
            [("2026-10-29", vec!["day 29"]), ("2026-10-27", vec!["day 27"]), ("2026-10-25", vec!["day 25"])]
        );
        assert!(page.more);
        let page = older(&dir, "2026-10-25", 3);
        assert_eq!(texts(&page.days), [("2026-10-22", vec!["day 22"]), ("2026-10-20", vec!["day 20"])]);
        assert!(!page.more, "nothing is left before the 20th");
        let page = older(&dir, "2026-10-22", 1);
        assert_eq!(texts(&page.days), [("2026-10-20", vec!["day 20"])]);
        assert!(!page.more);
        assert_eq!(older(&dir, "2026-10-20", 7), Page::default());
        // Fewer days asked for than there are: more.
        let page = older(&dir, "2026-10-29", 2);
        assert_eq!(texts(&page.days), [("2026-10-27", vec!["day 27"]), ("2026-10-25", vec!["day 25"])]);
        assert!(page.more);
        // Exactly as many days as asked for and none left: no more.
        assert!(!older(&dir, "2026-10-21", 1).more);
        // Not a date: nothing.
        assert_eq!(older(&dir, "zzzz", 7), Page::default());
        assert_eq!(older(&tmp("older-missing"), "2026-10-31", 7), Page::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A write cut off by a power failure leaves half a line: it is skipped, the rest of the day still reads, and the
    /// next entry starts on a line of its own instead of being glued to it.
    #[test]
    fn half_written_line_is_skipped() {
        let dir = tmp("half-written");
        add(&dir, &[entry("2026-10-31T07:00:00.000", Mode::Dictation, "first")]);
        let file = dir.join("2026-10-31.jsonl");
        let mut bytes = std::fs::read(&file).unwrap();
        bytes.extend_from_slice(b"{\"id\":\"2026-10-31T07:05:00.000-2\",\"ts\":\"2026-10-31T07:05:00.000\",\"text\":\"\xD0");
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(texts(&recent(&dir, "2026-10-31").days), [("2026-10-31", vec!["first"])]);

        let next = entry("2026-10-31T07:10:00.000", Mode::Dictation, "next");
        add(&dir, std::slice::from_ref(&next));
        assert_eq!(texts(&recent(&dir, "2026-10-31").days), [("2026-10-31", vec!["next", "first"])]);
        assert_eq!(find(&dir, &next.id), Some(next));

        // Blank lines, lines of another shape and a line with an unknown mode are skipped too.
        std::fs::write(
            dir.join("2026-10-30.jsonl"),
            "\n[1,2]\n{\"id\":\"a\"}\n{\"id\":\"b\",\"ts\":\"2026-10-30T09:00:00.000\",\"mode\":\"summary\",\"app\":\"\",\"text\":\"x\"}\n",
        )
        .unwrap();
        assert_eq!(texts(&recent(&dir, "2026-10-31").days), [("2026-10-31", vec!["next", "first"])]);
        assert!(!recent(&dir, "2026-10-31").more);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Only `YYYY-MM-DD.jsonl` of a real date is a day of the history: anything else in the directory is never read,
    /// counted or deleted.
    #[test]
    fn foreign_files_are_ignored() {
        let dir = tmp("foreign");
        add(&dir, &[entry("2026-10-31T07:00:00.000", Mode::Dictation, "ours")]);
        let line = serde_json::to_string(&entry("2026-10-30T07:00:00.000", Mode::Dictation, "foreign")).unwrap() + "\n";
        let foreign = [
            "notes.txt",
            "2026-10-30.json",
            "2026-10-30.jsonl.bak",
            "2026-10-30.jsonl.tmp",
            "x2026-10-30.jsonl",
            "2026-13-01.jsonl",
            "2026-02-30.jsonl",
            "2026-1-30.jsonl",
            "26-10-30.jsonl",
        ];
        for name in foreign {
            std::fs::write(dir.join(name), &line).unwrap();
        }
        std::fs::create_dir_all(dir.join("2026-10-29.jsonl")).unwrap();

        let page = recent(&dir, "2026-10-31");
        assert_eq!(texts(&page.days), [("2026-10-31", vec!["ours"])]);
        assert!(!page.more);
        assert_eq!(older(&dir, "2026-11-30", 7), page);
        assert_eq!(texts(&search(&dir, "o", 10)), [("2026-10-31", vec!["ours"])]);
        assert_eq!(find(&dir, "2026-10-30T07:00:00.000-1"), None);

        assert_eq!(prune(&dir, Keep::Off, "2026-10-31").unwrap(), 1);
        clear(&dir).unwrap();
        let mut left: Vec<String> = foreign.iter().map(|n| (*n).to_owned()).collect();
        left.push("2026-10-29.jsonl".into());
        left.sort();
        assert_eq!(names(&dir), left);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn search_is_case_insensitive_over_text_and_question() {
        let dir = tmp("search");
        add(
            &dir,
            &[
                entry("2026-10-29T09:00:00.000", Mode::Dictation, "Отправь отчёт в ПЯТНИЦУ"),
                ask("2026-10-30T09:00:00.000", "Когда пятница?", "Через два дня."),
                entry("2026-10-30T10:00:00.000", Mode::Translation, "See you on Friday"),
                entry("2026-10-31T09:00:00.000", Mode::Dictation, "nothing here"),
            ],
        );
        assert_eq!(
            texts(&search(&dir, "  пятниц ", 10)),
            [("2026-10-30", vec!["Через два дня."]), ("2026-10-29", vec!["Отправь отчёт в ПЯТНИЦУ"])]
        );
        assert_eq!(texts(&search(&dir, "FRIDAY", 10)), [("2026-10-30", vec!["See you on Friday"])]);
        assert_eq!(texts(&search(&dir, "ДВА", 10)), [("2026-10-30", vec!["Через два дня."])]);
        for blank in ["", "   ", "\t\n"] {
            assert_eq!(search(&dir, blank, 10), Vec::<Day>::new(), "{blank:?}");
        }
        assert_eq!(search(&dir, "absent", 10), Vec::<Day>::new());
        assert_eq!(search(&tmp("search-missing"), "x", 10), Vec::<Day>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Dictation writes ё where people type е, and the other way round.
    #[test]
    fn search_finds_yo_and_ye_alike() {
        let dir = tmp("search-yo");
        add(
            &dir,
            &[
                entry("2026-10-29T09:00:00.000", Mode::Dictation, "Отчёт готов"),
                entry("2026-10-30T09:00:00.000", Mode::Dictation, "отчет отправлен"),
                ask("2026-10-31T09:00:00.000", "Где ЁЛКА?", "Во дворе."),
            ],
        );
        let both = [("2026-10-30", vec!["отчет отправлен"]), ("2026-10-29", vec!["Отчёт готов"])];
        assert_eq!(texts(&search(&dir, "отчет", 10)), both);
        assert_eq!(texts(&search(&dir, "ОТЧЁТ", 10)), both);
        assert_eq!(texts(&search(&dir, "елка", 10)), [("2026-10-31", vec!["Во дворе."])]);
        assert_eq!(texts(&search(&dir, "Ёлка", 10)), [("2026-10-31", vec!["Во дворе."])]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The text as it was heard is kept with its entry, and only when there is one: the line of an entry without it,
    /// like every entry of an earlier version, has no such field and reads back without it.
    #[test]
    fn raw_text_is_written_only_when_there_is_one() {
        let dir = tmp("raw");
        let cleaned = heard("2026-10-31T07:00:00.000", "Send the report on Friday.", "uh send the report on\nfriday");
        let plain = entry("2026-10-31T07:05:00.000", Mode::Dictation, "Plain.");
        add(&dir, &[cleaned.clone(), plain.clone()]);
        let file = std::fs::read_to_string(dir.join("2026-10-31.jsonl")).unwrap();
        assert_eq!(file.lines().count(), 2, "a line break in the raw text stays inside its line");
        let lines: Vec<serde_json::Value> = file.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0]["raw"], "uh send the report on\nfriday");
        assert!(lines[1].get("raw").is_none(), "{file}");
        assert_eq!(recent(&dir, "2026-10-31").days[0].entries, [plain, cleaned.clone()]);
        assert_eq!(find(&dir, &cleaned.id), Some(cleaned));

        let line = concat!(
            r#"{"id":"2026-10-30T09:00:00.000-1","ts":"2026-10-30T09:00:00.000","#,
            r#""mode":"dictation","app":"","text":"Earlier."}"#,
            "\n",
        );
        std::fs::write(dir.join("2026-10-30.jsonl"), line).unwrap();
        let earlier = find(&dir, "2026-10-30T09:00:00.000-1").expect("an entry of an earlier version reads");
        assert_eq!((earlier.text.as_str(), earlier.raw), ("Earlier.", None));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A search looks in the text as it was heard too, with ё and е alike, and finds each entry once.
    #[test]
    fn search_looks_in_the_text_as_heard() {
        let dir = tmp("search-raw");
        let translated = heard("2026-10-31T09:00:00.000", "See you on Friday.", "Увидимся в пятницу");
        let translated = Entry { mode: Mode::Translation, ..translated };
        add(
            &dir,
            &[
                heard("2026-10-30T09:00:00.000", "Отчёт будет в пятницу.", "ну отчет будет э в пятницу"),
                translated,
                entry("2026-10-31T10:00:00.000", Mode::Dictation, "nothing here"),
            ],
        );
        assert_eq!(texts(&search(&dir, "НУ ОТЧЁТ", 10)), [("2026-10-30", vec!["Отчёт будет в пятницу."])]);
        assert_eq!(texts(&search(&dir, "увидимся", 10)), [("2026-10-31", vec!["See you on Friday."])]);
        assert_eq!(
            texts(&search(&dir, "пятниц", 10)),
            [("2026-10-31", vec!["See you on Friday."]), ("2026-10-30", vec!["Отчёт будет в пятницу."])]
        );
        assert_eq!(texts(&search(&dir, "friday", 10)), [("2026-10-31", vec!["See you on Friday."])]);
        assert_eq!(search(&dir, "absent", 10), Vec::<Day>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// An entry id names its recording, with the `:` Windows does not allow as `-`; anything else names none, so no id
    /// from a page reaches outside the folder.
    #[test]
    fn audio_file_takes_entry_ids_alone() {
        for (id, file) in [
            ("2026-10-31T07:30:00.123-1", "2026-10-31T07-30-00.123-1.ogg"),
            ("2026-10-31T23:59:59.999-42", "2026-10-31T23-59-59.999-42.ogg"),
            ("2024-02-29T00:00:00.000-18446744073709551615", "2024-02-29T00-00-00.000-18446744073709551615.ogg"),
        ] {
            assert_eq!(audio_file(id).as_deref(), Some(file), "{id}");
            assert_eq!(audio_snapshot(id), Some(file.replace(".ogg", ".wav.tmp")), "{id}");
        }
        let e = entry("2026-10-31T07:30:00.123", Mode::Dictation, "x");
        assert_eq!(audio_file(&e.id).as_deref(), Some("2026-10-31T07-30-00.123-1.ogg"), "the ids of the entries");

        for id in [
            "",
            "..",
            r"C:\Windows\win.ini",
            "2026-10-31T07:30:00.123",
            "2026-10-31T07:30:00.123-",
            "2026-10-31T07:30:00.123-0",
            "2026-10-31T07:30:00.123-01",
            "2026-10-31T07:30:00.123-+1",
            "2026-10-31T07:30:00.123--1",
            "2026-10-31T07:30:00.123-1a",
            "2026-10-31T07:30:00.123-123456789012345678901",
            "2026-10-31T07:30:00.123-1.ogg",
            "2026-10-31T07:30:00.123-1/../../x",
            r"2026-10-31T07:30:00.123-1\..\..\x",
            "2026-10-31T07:30:00.123-1\0",
            "2026-10-31T07:30:00.123-1 ",
            " 2026-10-31T07:30:00.123-1",
            "x2026-10-31T07:30:00.123-1",
            "2026-10-31T07-30-00.123-1",
            "2026-10-31 07:30:00.123-1",
            "2026-10-31T07:30:00,123-1",
            "2026-10-31T07:30:00.12-1",
            "2026-10-31T07:30:00.1234-1",
            "2026-10-31T7:30:00.123-1",
            "2026-10-31T24:00:00.000-1",
            "2026-10-31T07:60:00.000-1",
            "2026-10-31T07:30:60.000-1",
            "2026-10-31T0a:30:00.000-1",
            "2026-02-30T07:30:00.000-1",
            "2026-13-01T07:30:00.000-1",
            "２026-10-31T07:30:00.123-1",
            "2026-10-31T07:30:00.12３-1",
            "2026-10-31T07:30:00.123-１",
        ] {
            assert_eq!(audio_file(id), None, "{id:?}");
            assert_eq!(audio_snapshot(id), None, "{id:?}");
        }
    }

    /// Recordings go by the date of their entry, counted back like the days; a file still being written, and any
    /// other name, is never picked.
    #[test]
    fn audio_to_delete_keeps_the_period() {
        let dates = ["2026-09-29", "2026-09-30", "2026-10-23", "2026-10-24", "2026-10-30", "2026-10-31", "2026-11-02"];
        let recording = |date: &str| audio_file(&format!("{date}T12:00:00.000-1")).unwrap();
        let recordings: Vec<String> = dates.iter().map(|d| recording(d)).collect();
        let snapshot = audio_snapshot("2026-09-01T12:00:00.000-2").unwrap();
        let foreign = [
            snapshot.as_str(),
            "2026-09-01T12-00-00.000-3.ogg.tmp",
            "x.tmp",
            "notes.txt",
            "2026-10-30.jsonl",
            ".ogg",
            "x.ogg",
            "2026-10-01.ogg",
            "2026-10-01T12-00-00.000-1.wav",
            "2026-10-01T12-00-00.000-1.ogg.bak",
            "2026-10-01T12-00-00.000-1.OGG",
            "2026-10-01T12:00:00.000-1.ogg",
            "2026-10-01T12-00-00.000-0.ogg",
            "2026-10-01T12-00-00.000-1-1.ogg",
            "2026-02-30T12-00-00.000-1.ogg",
        ];
        let names: Vec<&str> = recordings.iter().map(String::as_str).chain(foreign).collect();
        let pick = |keep, today| audio_to_delete(names.iter().copied(), keep, today);
        // The recordings before `first_kept`.
        let gone = |first_kept: usize| -> Vec<&str> { recordings[..first_kept].iter().map(String::as_str).collect() };
        let today = "2026-10-31";
        assert_eq!(pick(AudioKeep::Off, today), gone(dates.len()), "off: every recording, also one after today");
        // A month keeps 31 days back: 2026-09-30 stays.
        assert_eq!(pick(AudioKeep::Month, today), gone(1));
        // A week keeps 7 days back: 2026-10-24 stays.
        assert_eq!(pick(AudioKeep::Week, today), gone(3));
        // A day keeps today's, and one after today (the clock was moved back).
        assert_eq!(pick(AudioKeep::Day, today), gone(5));
        assert_eq!(pick(AudioKeep::Day, "2026-11-03"), gone(dates.len()));
        // With no date to count from, no recording is old; off needs none.
        assert_eq!(pick(AudioKeep::Day, "today"), gone(0));
        assert_eq!(pick(AudioKeep::Off, "today"), gone(dates.len()));
        assert_eq!(audio_to_delete(foreign, AudioKeep::Off, today), Vec::<&str>::new());
    }

    /// Only CUE's own half-written files are swept: the snapshot of an entry's sound and a recording being encoded,
    /// named by an entry id. Any other `.tmp`, and every recording, stays.
    #[test]
    fn audio_temps_are_only_cues_own() {
        let id = "2026-10-31T12:00:00.000-2";
        let (snapshot, recording) = (audio_snapshot(id).unwrap(), audio_file(id).unwrap());
        let encoding = format!("{recording}.tmp");
        assert_eq!(snapshot, "2026-10-31T12-00-00.000-2.wav.tmp");
        assert_eq!(encoding, "2026-10-31T12-00-00.000-2.ogg.tmp", "the encoder's name for its file");
        let others = [
            recording.as_str(),
            "x.tmp",
            "notes.tmp",
            ".tmp",
            ".wav.tmp",
            ".ogg.tmp",
            "2026-10-31T12-00-00.000-2.tmp",
            "2026-10-31T12-00-00.000-2.mp3.tmp",
            "2026-10-31T12-00-00.000-2.wav",
            "2026-10-31T12-00-00.000-2.WAV.TMP",
            "2026-10-31T12-00-00.000-2.ogg.tmp.bak",
            "2026-10-31T12-00-00.000-2.wav.wav.tmp",
            "2026-10-31T12:00:00.000-2.wav.tmp",
            "2026-10-31T12-00-00.000-0.wav.tmp",
            "2026-02-30T12-00-00.000-2.ogg.tmp",
            "x2026-10-31T12-00-00.000-2.wav.tmp",
            "2026-10-31T12-00-00.000-2 .ogg.tmp",
        ];
        let names: Vec<&str> = [snapshot.as_str(), &encoding].into_iter().chain(others).collect();
        assert_eq!(audio_temps(names.iter().copied()), [snapshot.as_str(), &encoding]);
        assert_eq!(audio_temps(others), Vec::<&str>::new());
    }

    /// On disk: a prune deletes old recordings alone, the sweep CUE's half-written files alone; other files and
    /// folders stay; clearing removes the whole folder. The history's own deletions never touch the recordings folder
    /// inside it.
    #[test]
    fn prune_audio_and_clear_audio_on_disk() {
        let history = tmp("audio");
        let dir = history.join("audio");
        assert_eq!(prune_audio(&dir, AudioKeep::Day, "2026-10-31").unwrap(), 0, "no folder: nothing to delete");
        assert_eq!(sweep_audio_temps(&dir).unwrap(), 0);
        clear_audio(&dir).unwrap();

        add(&history, &[entry("2026-10-30T12:00:00.000", Mode::Dictation, "yesterday")]);
        std::fs::create_dir_all(&dir).unwrap();
        let old = audio_file("2026-10-30T12:00:00.000-1").unwrap();
        let kept = audio_file("2026-10-31T08:00:00.000-2").unwrap();
        let left = audio_snapshot("2026-10-31T08:00:00.000-3").unwrap();
        let encoding = audio_file("2026-10-31T08:00:00.000-4").unwrap() + ".tmp";
        for name in [old.as_str(), &kept, &left, &encoding, "notes.txt", "notes.tmp"] {
            std::fs::write(dir.join(name), b"OggS").unwrap();
        }
        let folder = audio_file("2026-10-01T12:00:00.000-1").unwrap();
        let temp_folder = audio_snapshot("2026-10-01T12:00:00.000-1").unwrap();
        for name in [&folder, &temp_folder] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }

        assert_eq!(prune_audio(&dir, AudioKeep::Day, "2026-10-31").unwrap(), 1, "not the files being written");
        let folders = [folder.as_str(), &temp_folder];
        let notes = ["notes.tmp", "notes.txt"];
        let left_over = [&kept, &left, &encoding].map(String::as_str);
        assert_eq!(names(&dir), [&folders[..], &left_over, &notes].concat());
        assert_eq!(prune_audio(&dir, AudioKeep::Day, "2026-10-31").unwrap(), 0, "nothing more to delete");
        assert_eq!(sweep_audio_temps(&dir).unwrap(), 2);
        let swept = [&folders[..], &[kept.as_str()], &notes].concat();
        assert_eq!(names(&dir), swept, "folders named like ours stay");
        assert_eq!(sweep_audio_temps(&dir).unwrap(), 0);

        assert_eq!(prune(&history, Keep::Off, "2026-10-31").unwrap(), 1);
        clear(&history).unwrap();
        assert_eq!(names(&history), ["audio"]);
        assert_eq!(names(&dir), swept, "the days are gone, the recordings are not");

        assert_eq!(prune_audio(&dir, AudioKeep::Off, "2026-10-31").unwrap(), 1);
        assert_eq!(names(&dir), [&folders[..], &notes].concat());
        clear_audio(&dir).unwrap();
        assert!(!dir.exists());
        clear_audio(&dir).unwrap();
        std::fs::remove_dir_all(history).unwrap();
    }

    #[test]
    fn search_respects_the_limit() {
        let dir = tmp("search-limit");
        for (day, hour) in [(29, 9), (30, 9), (30, 10), (30, 11), (31, 9), (31, 10)] {
            let ts = format!("2026-10-{day}T{hour:02}:00:00.000");
            add(&dir, &[entry(&ts, Mode::Dictation, &format!("note {day} {hour}"))]);
        }
        assert_eq!(
            texts(&search(&dir, "note", 3)),
            [("2026-10-31", vec!["note 31 10", "note 31 9"]), ("2026-10-30", vec!["note 30 11"])]
        );
        assert_eq!(texts(&search(&dir, "note", 2)), [("2026-10-31", vec!["note 31 10", "note 31 9"])]);
        assert_eq!(search(&dir, "note", 100).iter().map(|d| d.entries.len()).sum::<usize>(), 6);
        assert_eq!(search(&dir, "note", 0), Vec::<Day>::new());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn prune_keeps_the_period_and_off_deletes_all() {
        let dir = tmp("prune");
        let today = "2026-10-31";
        let days = [
            "2025-10-29", "2025-10-30", "2026-09-29", "2026-09-30", "2026-10-23", "2026-10-24", "2026-10-31", "2026-11-02",
        ];
        let fill = || {
            for date in days {
                add(&dir, &[entry(&format!("{date}T12:00:00.000"), Mode::Dictation, date)]);
            }
        };
        let files = |dates: &[&str]| dates.iter().map(|d| format!("{d}.jsonl")).collect::<Vec<_>>();
        fill();
        assert_eq!(prune(&dir, Keep::Forever, today).unwrap(), 0);
        assert_eq!(names(&dir), files(&days));
        // A year keeps 366 days back: 2025-10-30 stays, the day before goes.
        assert_eq!(prune(&dir, Keep::Year, today).unwrap(), 1);
        assert_eq!(names(&dir), files(&days[1..]));
        // A month keeps 31 days back: 2026-09-30 stays.
        assert_eq!(prune(&dir, Keep::Month, today).unwrap(), 2);
        assert_eq!(names(&dir), files(&days[3..]));
        // A week keeps 7 days back: 2026-10-24 stays. A day after today (the clock was moved) is not old.
        assert_eq!(prune(&dir, Keep::Week, today).unwrap(), 2);
        assert_eq!(names(&dir), files(&days[5..]));
        assert_eq!(prune(&dir, Keep::Week, today).unwrap(), 0, "nothing more to delete");
        assert_eq!(prune(&dir, Keep::Off, today).unwrap(), 3);
        assert_eq!(names(&dir), Vec::<String>::new());

        fill();
        clear(&dir).unwrap();
        assert_eq!(names(&dir), Vec::<String>::new());
        assert!(prune(&dir, Keep::Week, "today").is_err(), "a period needs a date to count from");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(prune(&dir, Keep::Week, today).unwrap(), 0, "no directory: nothing to delete");
        clear(&dir).unwrap();
    }

    #[test]
    fn date_minus_crosses_months_and_leap_years() {
        for (date, days, expected) in [
            ("2026-10-31", 0, "2026-10-31"),
            ("2026-10-31", 1, "2026-10-30"),
            ("2026-11-01", 1, "2026-10-31"),
            ("2026-01-01", 1, "2025-12-31"),
            ("2024-03-01", 1, "2024-02-29"),
            ("2026-03-01", 1, "2026-02-28"),
            ("2000-03-01", 1, "2000-02-29"),
            ("1900-03-01", 1, "1900-02-28"),
            ("2026-10-31", 7, "2026-10-24"),
            ("2026-10-31", 31, "2026-09-30"),
            ("2026-10-31", 366, "2025-10-30"),
            ("2025-03-01", 366, "2024-02-29"),
            ("2026-10-31", 3652, "2016-10-31"),
            ("1970-01-01", 1, "1969-12-31"),
        ] {
            assert_eq!(date_minus(date, days).as_deref(), Some(expected), "{date} - {days}");
        }
        for date in [
            "",
            "2026-10",
            "2026-10-31T07:00",
            "2026-13-01",
            "2026-02-29",
            "2026-04-31",
            "2026-00-10",
            "2026-10-00",
            "2026/10/31",
            "+026-10-31",
            "２026-10-31",
            "abcd-ef-gh",
        ] {
            assert_eq!(date_minus(date, 1), None, "{date:?}");
        }
        assert_eq!(date_minus("0000-01-01", 1), None, "before year 0 there is no four-digit date");
    }

    /// The text of an entry is the user's: a log shows only its length.
    #[test]
    fn entry_debug_has_no_text() {
        let e = ask("2026-10-31T07:30:00.000", "secret question", "private answer");
        let text = format!("{e:?}");
        assert!(!text.contains("secret") && !text.contains("private"), "{text}");
        assert!(text.contains("2026-10-31T07:30:00.000") && text.contains("Ask") && text.contains("Notepad"), "{text}");
        assert!(text.contains("14") && text.contains("Some(15)"), "the lengths are shown: {text}");
        assert!(text.contains("raw_chars: None"), "{text}");
        let page = Page { days: vec![Day { date: "2026-10-31".into(), entries: vec![e] }], more: false };
        assert!(!format!("{page:?}").contains("private"));

        let dictated = heard("2026-10-31T07:31:00.000", "Clean.", "secret raw speech");
        let text = format!("{dictated:?}");
        assert!(!text.contains("secret") && !text.contains("speech") && !text.contains("Clean"), "{text}");
        assert!(text.contains("raw_chars: Some(17)"), "the length is shown: {text}");
    }
}
