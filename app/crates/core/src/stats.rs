//! The main window's Today: words by day, words a minute, days in a row and the total, counted from usage.csv (its
//! `words` and `day` columns; no text is read). Counted are the dictations and translations that wrote text
//! (outcomes `inserted` and `no_field`). A row written before those columns gets an estimate: its audio at the speaking
//! speed measured on the rows that have them (`ESTIMATE_WPM` while there are none), and the date of its UTC `ts` moved
//! by the local offset now.
use serde::Serialize;

use crate::usage::{DICTATE, INSERTED, NO_FIELD, fields, split_record};

/// Days the bars show, today the last.
pub const DAYS: usize = 30;
/// Typing speed, words a minute: "minutes without the keyboard" are today's words at this speed.
pub const TYPING_WPM: u64 = 40;
/// Speaking speed for the estimate of old rows while no row has its words.
pub const ESTIMATE_WPM: f64 = 150.0;
/// The round numbers of words a milestone marks before every 100 000.
const MILESTONES: [u64; 6] = [1_000, 5_000, 10_000, 25_000, 50_000, 100_000];

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayWords {
    pub date: String,
    pub words: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub today_words: u64,
    pub saved_minutes: u64,
    /// `DAYS` local days, oldest first, today the last.
    pub days: Vec<DayWords>,
    /// Words a minute over those days; `None` without audio.
    pub wpm: Option<u64>,
    /// Days with words in a row, ending today, or yesterday while today has none.
    pub streak: u32,
    pub total: u64,
    /// The largest round number the total has reached.
    pub milestone: Option<u64>,
}

/// The largest milestone not above `total`: 1 000, 5 000, 10 000, 25 000, 50 000, 100 000, then every 100 000.
pub fn milestone(total: u64) -> Option<u64> {
    if total >= 100_000 {
        return Some(total / 100_000 * 100_000);
    }
    MILESTONES.iter().rev().copied().find(|&m| total >= m)
}

/// Days since 1970-01-01 of a civil date (the days-from-civil algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `YYYY-MM-DD` of a day number from [`days_from_civil`].
pub fn date_of(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day number of `YYYY-MM-DD`; `None` if it is not one.
fn day_number(date: &str) -> Option<i64> {
    let b = date.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (date[..4].parse().ok()?, date[5..7].parse().ok()?, date[8..].parse().ok()?);
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

/// The local day of a UTC `YYYY-MM-DDTHH:MM:SSZ` moved by `offset_minutes`.
fn local_day_of(ts: &str, offset_minutes: i32) -> Option<i64> {
    let day = day_number(ts.get(..10)?)?;
    let (h, m): (i64, i64) = (ts.get(11..13)?.parse().ok()?, ts.get(14..16)?.parse().ok()?);
    Some(day + (h * 60 + m + i64::from(offset_minutes)).div_euclid(1_440))
}

/// A counted row: its local day, its words (`None` — an old row, to estimate) and its audio.
struct Counted {
    day: i64,
    words: Option<u64>,
    audio_ms: u64,
}

fn counted(line: &str, offset_minutes: i32) -> Option<Counted> {
    let c = split_record(line)?;
    let f = fields(&c)?;
    if !matches!(f.mode, DICTATE | "translate") || !matches!(f.outcome, INSERTED | NO_FIELD) {
        return None;
    }
    let words = match f.words {
        Some(w) => Some(w.parse().ok()?),
        None => None,
    };
    let day = match f.day.and_then(day_number) {
        Some(d) => d,
        None => local_day_of(f.ts, offset_minutes)?,
    };
    Some(Counted { day, words, audio_ms: f.audio_ms.parse().ok()? })
}

/// Today's numbers from the text of usage.csv; `today` is the local date, `offset_minutes` local time minus UTC.
pub fn compute(csv: &str, today: &str, offset_minutes: i32) -> Stats {
    let rows: Vec<Counted> = csv.lines().filter_map(|l| counted(l, offset_minutes)).collect();
    let (said, minutes) = rows
        .iter()
        .filter_map(|r| r.words.map(|w| (w, r.audio_ms)))
        .filter(|&(_, ms)| ms > 0)
        .fold((0u64, 0f64), |(w, m), (rw, ms)| (w + rw, m + ms as f64 / 60_000.0));
    let speed = if said > 0 && minutes > 0.0 { said as f64 / minutes } else { ESTIMATE_WPM };
    let words_of = |r: &Counted| r.words.unwrap_or_else(|| (r.audio_ms as f64 / 60_000.0 * speed).round() as u64);

    let last = day_number(today).unwrap_or_default();
    let first = last - (DAYS as i64 - 1);
    let mut by_day = vec![0u64; DAYS];
    let (mut total, mut window_words, mut window_minutes) = (0u64, 0u64, 0f64);
    for r in &rows {
        let w = words_of(r);
        total += w;
        if (first..=last).contains(&r.day) {
            by_day[(r.day - first) as usize] += w;
            if r.audio_ms > 0 {
                window_words += w;
                window_minutes += r.audio_ms as f64 / 60_000.0;
            }
        }
    }
    let today_words = by_day[DAYS - 1];
    // The run may be longer than the bars: it walks back over every day that has words.
    let wrote: std::collections::HashSet<i64> = rows.iter().filter(|r| words_of(r) > 0).map(|r| r.day).collect();
    let mut d = if today_words > 0 { last } else { last - 1 };
    let mut streak = 0u32;
    while wrote.contains(&d) {
        streak += 1;
        d -= 1;
    }
    Stats {
        today_words,
        saved_minutes: (today_words as f64 / TYPING_WPM as f64).round() as u64,
        days: (0..DAYS).map(|i| DayWords { date: date_of(first + i as i64), words: by_day[i] }).collect(),
        wpm: (window_minutes > 0.0).then(|| (window_words as f64 / window_minutes).round() as u64),
        streak,
        total,
        milestone: milestone(total),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::HEADER;

    /// A row of today's width: ts, audio, outcome, mode, words, day.
    fn row(ts: &str, audio_ms: u64, outcome: &str, mode: &str, words: u64, day: &str) -> String {
        format!("{ts},{audio_ms},parakeet,local,agent:codex:gpt-6-luna,0,0,0,0,900,{outcome},{mode},0,{words},{day}")
    }
    /// A row of the version before, with its dollars: 18 columns.
    fn priced(ts: &str, audio_ms: u64, outcome: &str, mode: &str, words: u64, day: &str) -> String {
        format!("{ts},{audio_ms},soniox,stream,0.0,claude-sonnet-5-5,0,0,0,0,0.0,900,{outcome},{mode},0,0.000000,{words},{day}")
    }
    /// A row before the words: 16 columns.
    fn old(ts: &str, audio_ms: u64, outcome: &str, mode: &str) -> String {
        format!("{ts},{audio_ms},soniox,stream,0.0,claude-haiku-5-5,0,0,0,0,0.0,900,{outcome},{mode},0,0.000000")
    }
    fn file(rows: &[String]) -> String {
        std::iter::once(HEADER.to_owned()).chain(rows.iter().cloned()).collect::<Vec<_>>().join("\n")
    }
    const TODAY: &str = "2026-11-01";

    #[test]
    fn dates_round_trip() {
        for d in ["1970-01-01", "2000-02-29", "2026-10-31", "2026-11-01", "2027-01-01"] {
            assert_eq!(date_of(day_number(d).unwrap()), d);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(day_number("2026-13-01"), None);
    }

    #[test]
    fn nothing_written_is_all_zeros() {
        let s = compute("", TODAY, 240);
        assert_eq!((s.today_words, s.saved_minutes, s.wpm, s.streak, s.total, s.milestone), (0, 0, None, 0, 0, None));
        assert_eq!(s.days.len(), DAYS);
        assert_eq!((s.days[0].date.as_str(), s.days[DAYS - 1].date.as_str()), ("2026-10-03", TODAY));
    }

    #[test]
    fn counts_what_dictations_and_translations_wrote() {
        let csv = file(&[
            row("2026-11-01T08:00:00Z", 60_000, "inserted", "dictate", 100, TODAY),
            row("2026-11-01T08:05:00Z", 30_000, "no_field", "translate", 20, TODAY),
            row("2026-11-01T08:06:00Z", 30_000, "inserted", "ask", 50, TODAY),
            row("2026-11-01T08:07:00Z", 30_000, "cancelled", "dictate", 0, TODAY),
            row("2026-11-01T08:08:00Z", 0, "discarded", "warmup", 0, TODAY),
        ]);
        let s = compute(&csv, TODAY, 240);
        assert_eq!((s.today_words, s.saved_minutes, s.total), (120, 3, 120));
        assert_eq!(s.wpm, Some(80), "120 words in 1.5 minutes");
        assert_eq!(s.days[DAYS - 1].words, 120);
    }

    #[test]
    fn old_rows_are_estimated_at_the_measured_speed() {
        let measured = row("2026-11-01T08:00:00Z", 120_000, "inserted", "dictate", 300, TODAY);
        let csv = file(&[old("2026-10-31T10:00:00Z", 60_000, "inserted", "dictate"), measured]);
        let s = compute(&csv, TODAY, 0);
        assert_eq!(s.days[DAYS - 2].words, 150, "60 s at 150 words a minute");
        assert_eq!(s.total, 450);
        let alone = compute(&file(&[old("2026-10-31T10:00:00Z", 30_000, "inserted", "dictate")]), TODAY, 0);
        assert_eq!(alone.total, 75, "the default speed while no row has its words");
        let v1 = "2026-10-31T10:00:00Z,60000,soniox,stream,0.0,claude-haiku-5-5,0,0,0,0,0.0,900,inserted";
        assert_eq!(compute(&file(&[v1.to_owned()]), TODAY, 0).total, 150, "rows before the modes are dictations");
    }

    #[test]
    fn an_old_row_takes_its_local_day() {
        let late = old("2026-10-31T21:30:00Z", 60_000, "inserted", "dictate");
        assert_eq!(compute(&file(std::slice::from_ref(&late)), TODAY, 240).today_words, 150, "01:30 local, the next day");
        assert_eq!(compute(&file(&[late]), TODAY, 0).today_words, 0);
    }

    #[test]
    fn thirty_days_end_today_across_months() {
        let csv = file(&[
            row("2026-10-03T08:00:00Z", 1_000, "inserted", "dictate", 7, "2026-10-03"),
            row("2026-10-02T08:00:00Z", 1_000, "inserted", "dictate", 9, "2026-10-02"),
        ]);
        let s = compute(&csv, TODAY, 0);
        assert_eq!(s.days[0], DayWords { date: "2026-10-03".into(), words: 7 });
        assert_eq!(s.total, 16, "the total counts days before the bars too");
    }

    #[test]
    fn days_in_a_row() {
        let on = |d: &str| row(&format!("{d}T08:00:00Z"), 1_000, "inserted", "dictate", 5, d);
        assert_eq!(compute(&file(&[on(TODAY), on("2026-10-31"), on("2026-10-30")]), TODAY, 0).streak, 3);
        assert_eq!(compute(&file(&[on("2026-10-31"), on("2026-10-30")]), TODAY, 0).streak, 2, "today not yet");
        assert_eq!(compute(&file(&[on(TODAY), on("2026-10-30")]), TODAY, 0).streak, 1, "a gap ends it");
        assert_eq!(compute(&file(&[on("2026-10-30")]), TODAY, 0).streak, 0);
        let long: Vec<String> = (0..40).map(|i| on(&date_of(day_number(TODAY).unwrap() - i))).collect();
        assert_eq!(compute(&file(&long), TODAY, 0).streak, 40, "longer than the bars");
    }

    #[test]
    fn milestones() {
        assert_eq!(milestone(999), None);
        assert_eq!(milestone(1_000), Some(1_000));
        assert_eq!(milestone(24_999), Some(10_000));
        assert_eq!(milestone(100_000), Some(100_000));
        assert_eq!(milestone(260_000), Some(200_000));
    }

    /// A row written with its dollars counts its words as it did.
    #[test]
    fn rows_with_dollars_of_the_version_before_count_their_words() {
        let csv = file(&[
            priced("2026-11-01T07:00:00Z", 60_000, "inserted", "dictate", 40, TODAY),
            row("2026-11-01T08:00:00Z", 60_000, "inserted", "dictate", 60, TODAY),
        ]);
        let s = compute(&csv, TODAY, 0);
        assert_eq!((s.today_words, s.total, s.wpm), (100, 100, Some(50)));
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let csv = file(&["garbage".into(), "\"broken,1".into(), row("2026-11-01T08:00:00Z", 1_000, "inserted", "dictate", 4, TODAY)]);
        assert_eq!(compute(&csv, TODAY, 0).total, 4);
    }
}
