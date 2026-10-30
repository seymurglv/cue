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
use crate::settings::Keep;
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
/// unique and starts with `ts`. `app` is the app the result went to, empty when not known. For "Ask", `question` is the
/// question and `text` the answer or the text the model wrote.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub ts: String,
    pub mode: Mode,
    pub app: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
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
            .finish()
    }
}

impl Entry {
    /// Whether the text or the question holds `query`, which is already folded (`fold`).
    fn matches(&self, query: &str) -> bool {
        let holds = |s: &str| fold(s).contains(query);
        holds(&self.text) || self.question.as_deref().is_some_and(holds)
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

/// The entries whose text or question holds `query` (trimmed, in any case, ё and е alike), newest first, at most
/// `limit` of them. A blank query finds nothing.
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

/// Deletes the day files `old` picks; returns how many. A file that cannot be deleted (another program holds it open)
/// does not stop the others: the first such error is returned, and the next prune tries again.
fn remove_days(dir: &Path, old: impl Fn(&str) -> bool) -> io::Result<usize> {
    let mut removed = 0;
    let mut failed = None;
    for date in dates(dir)?.into_iter().filter(|d| old(d)) {
        match std::fs::remove_file(day_file(dir, &date)) {
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
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut dates: Vec<String> = listing
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().and_then(file_date).map(str::to_owned))
        .collect();
    dates.sort_unstable_by(|a, b| b.cmp(a));
    Ok(dates)
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
        Entry { id: format!("{ts}-1"), ts: ts.into(), mode, app: "Notepad".into(), text: text.into(), question: None }
    }

    fn ask(ts: &str, question: &str, answer: &str) -> Entry {
        Entry { question: Some(question.into()), ..entry(ts, Mode::Ask, answer) }
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
        // Exactly as many days as asked for and none left: no more.
        let page = older(&dir, "2026-10-29", 2);
        assert_eq!(texts(&page.days), [("2026-10-27", vec!["day 27"]), ("2026-10-25", vec!["day 25"])]);
        assert!(page.more);
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
        let page = Page { days: vec![Day { date: "2026-10-31".into(), entries: vec![e] }], more: false };
        assert!(!format!("{page:?}").contains("private"));
    }
}
