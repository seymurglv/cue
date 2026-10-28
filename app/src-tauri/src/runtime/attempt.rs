//! Usage accounting of one attempt: audio, transcription path, refine tokens, per-stage latencies: one usage.csv row
//! and one log line.
//!
//! An attempt is a recording (`OpenMic`) and its transcription. "Retry" (`TranscribeSaved`) starts a new attempt
//! of the same dictation: each one costs money, so each gets its own row with its own outcome (`Finished` arrives
//! twice).
//! A retry resets the attempt's fields (refine tokens, latencies, the path: `Batch`, batch transcription of the saved
//! audio only) and keeps the audio and the app: it is the same audio from the same app.
//! "Retry" after a failed translation (`RefineAgain`) is a new attempt on the saved text: its row has no audio (the
//! first attempt paid for it), and the app and the speech language stay.
//! The `latency` runs from the second press (`FinishStt`) to Ctrl+V; after "retry", from the retry; if the dictation
//! was restored after Esc ("restore"), from "restore": time spent in "Cancelled" is not CUE's latency.
//! An attempt without a stream (no Soniox key, the stream never connected) still bills the audio at the stream price:
//! an overestimate in a rare failure, accepted.
//! No text here: only numbers, the path and the outcome.
use std::collections::VecDeque;

use cue_core::flow::{DictationId, Outcome, RefineOutcome, STARTING_MS};
use cue_core::settings::{Pricing, Settings};
use cue_core::usage::{self, SttPath, UsageRecord};
use cue_engines::claude;
use cue_engines::stt::SAMPLES_PER_MS;

#[derive(Clone, Debug, PartialEq)]
pub struct Attempt {
    pub id: DictationId,
    /// The app in focus when the recording started (Soniox context, bench). Never written to the log.
    pub app: String,
    samples: u64,
    pub stt_path: SttPath,
    /// The language of most of the speech from the transcription (a Soniox code): which way a translation goes.
    /// A retry clears it: the new transcription gives its own; a retry of the translation keeps it.
    pub speech_language: Option<String>,
    refine: claude::Usage,
    pub refine_outcome: Option<RefineOutcome>,
    /// The second press (`FinishStt`) or "retry" (`TranscribeSaved`): transcription is timed from here. Core ms.
    finish_at: Option<u64>,
    /// The latency is counted from here: like `finish_at`, but after "restore" it is the moment of "restore".
    latency_from: Option<u64>,
    stt_done_at: Option<u64>,
    refine_at: Option<(u64, Option<u64>)>,
    insert_at: Option<(u64, Option<u64>)>,
    latency_ms: u64,
}

/// Stage latencies of the attempt, ms (`None` if the stage did not happen): transcription from the second press
/// to the text, refine, insert up to Ctrl+V; `latency` is up to Ctrl+V from the second press (after "retry", from
/// the retry; after "restore", from "restore").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stages {
    pub stt: Option<u64>,
    pub refine: Option<u64>,
    pub insert: Option<u64>,
    pub latency: u64,
}

impl Attempt {
    /// A new recording: everything from zero.
    pub fn new(id: DictationId) -> Self {
        Self {
            id,
            app: String::new(),
            samples: 0,
            stt_path: SttPath::Stream,
            speech_language: None,
            refine: claude::Usage::default(),
            refine_outcome: None,
            finish_at: None,
            latency_from: None,
            stt_done_at: None,
            refine_at: None,
            insert_at: None,
            latency_ms: 0,
        }
    }

    /// An audio frame of the recording.
    pub fn add_audio(&mut self, samples: usize) {
        self.samples = self.samples.saturating_add(samples as u64);
    }

    pub fn audio_ms(&self) -> u64 {
        self.samples / SAMPLES_PER_MS
    }

    /// The second press (or Esc while recording): transcription is finishing.
    pub fn finish_stt(&mut self, now: u64) {
        self.finish_at = Some(now);
        self.latency_from = Some(now);
    }

    /// "Restore" after Esc: the latency is counted from here.
    pub fn undo(&mut self, now: u64) {
        self.latency_from = Some(now);
    }

    /// "Retry": a new attempt on the same audio. The audio and the app stay.
    pub fn retry(&mut self, now: u64) {
        *self = Self { app: std::mem::take(&mut self.app), samples: self.samples, ..Self::new(self.id) };
        self.stt_path = SttPath::Batch;
        self.finish_at = Some(now);
        self.latency_from = Some(now);
    }

    /// "Retry" after a failed translation: a new attempt on the same text. No audio is paid again (the row has 0 ms); the
    /// app and the speech language stay.
    pub fn retry_text(&mut self, now: u64) {
        *self = Self {
            app: std::mem::take(&mut self.app),
            speech_language: self.speech_language.take(),
            ..Self::new(self.id)
        };
        self.latency_from = Some(now);
    }

    /// The transcription result arrived; `path` is how the audio was actually transcribed.
    pub fn stt_done(&mut self, path: SttPath, now: u64) {
        self.stt_path = path;
        self.stt_done_at = Some(now);
    }

    pub fn refine_started(&mut self, now: u64) {
        self.refine_at = Some((now, None));
    }

    pub fn refine_done(&mut self, usage: claude::Usage, outcome: RefineOutcome, now: u64) {
        self.refine = usage;
        self.refine_outcome = Some(outcome);
        if let Some((_, end)) = self.refine_at.as_mut() {
            *end = Some(now);
        }
    }

    pub fn insert_started(&mut self, now: u64) {
        self.insert_at = Some((now, None));
    }

    /// Ctrl+V was sent (or "no field" was decided) at `at`.
    pub fn inserted(&mut self, at: u64) {
        if let Some((_, end)) = self.insert_at.as_mut() {
            *end = Some(at);
        }
        if let Some(t) = self.latency_from {
            self.latency_ms = at.saturating_sub(t);
        }
    }

    pub fn stages(&self) -> Stages {
        let span = |s: Option<(u64, Option<u64>)>| s.and_then(|(a, b)| Some(b?.saturating_sub(a)));
        let stt = self.finish_at.zip(self.stt_done_at).map(|(a, b)| b.saturating_sub(a));
        Stages { stt, refine: span(self.refine_at), insert: span(self.insert_at), latency: self.latency_ms }
    }

    /// The usage.csv row for outcome `outcome`; `ts` is the UTC time.
    pub fn record(&self, outcome: Outcome, ts: String, s: &Settings) -> UsageRecord {
        UsageRecord {
            ts,
            audio_ms: self.audio_ms(),
            stt_engine: s.stt.engine,
            stt_path: self.stt_path,
            refine_model: s.refine.model.clone(),
            input_tokens: self.refine.input_tokens,
            cache_read_tokens: self.refine.cache_read_input_tokens,
            cache_write_tokens: self.refine.cache_creation_input_tokens,
            output_tokens: self.refine.output_tokens,
            latency_ms: self.latency_ms,
            outcome,
        }
    }
}

/// The log line about the attempt's result: outcome, audio, transcription path, stage latencies as numbers (no stage,
/// no field), refine outcome. No text and no app.
pub fn log_finished(outcome: Outcome, a: &Attempt) {
    let st = a.stages();
    tracing::info!(
        outcome = usage::outcome_name(outcome),
        audio_ms = a.audio_ms(),
        stt_path = a.stt_path.as_str(),
        stt_ms = st.stt,
        refine_ms = st.refine,
        insert_ms = st.insert,
        latency_ms = st.latency,
        refine = a.refine_outcome.map(tracing::field::debug),
        "dictation finished"
    );
}

/// Whether the attempt needs a usage.csv row. A discard (`Discarded`) gets one only if the audio ran longer than the
/// start-up: a discard mid-recording (lock, pause, a system chord after the start-up) has already been paid for by
/// the stream, a system chord during the start-up has not.
pub fn needs_row(outcome: Outcome, audio_ms: u64) -> bool {
    outcome != Outcome::Discarded || audio_ms >= STARTING_MS
}

/// At most this many rows wait to be written: beyond that the oldest are dropped (the file has been unavailable
/// for very long).
pub const MAX_PENDING: usize = 1_000;

/// Writing to usage.csv. A row that could not be appended (the file is open in Excel) stays in memory
/// and is appended before the next one, in order.
#[derive(Default)]
pub struct UsageLog {
    pending: VecDeque<UsageRecord>,
    warned_no_price: bool,
    dropped: u64,
}

impl UsageLog {
    /// Queues `rec` and appends the queue through `append`. The first failure stops the writing and is
    /// returned; the rows not yet written wait for the next time.
    pub fn write(
        &mut self,
        rec: UsageRecord,
        mut append: impl FnMut(&UsageRecord) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        self.pending.push_back(rec);
        let mut written = Ok(());
        while let Some(next) = self.pending.front() {
            if let Err(e) = append(next) {
                written = Err(e);
                break;
            }
            self.pending.pop_front();
        }
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
            self.dropped += 1;
        }
        written
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// How many rows were dropped because the queue overflowed.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// `true` if the model has no price in the settings and no warning has been given yet (once per session).
    pub fn warn_no_price(&mut self, pricing: &Pricing, model: &str) -> bool {
        if self.warned_no_price || pricing.claude_price(model).is_some() {
            return false;
        }
        self.warned_no_price = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    fn usage(input: u64, output: u64) -> claude::Usage {
        claude::Usage { input_tokens: input, output_tokens: output, cache_read_input_tokens: 7, cache_creation_input_tokens: 3 }
    }

    #[test]
    fn plain_dictation_record() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(4));
        a.app = "Telegram Desktop".into();
        for _ in 0..100 {
            a.add_audio(320);
        }
        assert_eq!(a.audio_ms(), 2_000, "100 frames of 20 ms");
        a.finish_stt(10_000);
        a.stt_done(SttPath::Stream, 10_350);
        a.refine_started(10_350);
        a.refine_done(usage(120, 15), RefineOutcome::Refined, 11_000);
        a.insert_started(11_000);
        a.inserted(11_090);
        assert_eq!(
            a.stages(),
            Stages { stt: Some(350), refine: Some(650), insert: Some(90), latency: 1_090 }
        );
        let r = a.record(Outcome::Inserted, "2026-10-27T10:00:00Z".into(), &s);
        assert_eq!(r.ts, "2026-10-27T10:00:00Z");
        assert_eq!(r.audio_ms, 2_000);
        assert_eq!(r.stt_path, SttPath::Stream);
        assert_eq!(r.refine_model, s.refine.model);
        assert_eq!((r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens), (120, 15, 7, 3));
        assert_eq!(r.latency_ms, 1_090);
        assert_eq!(r.outcome, Outcome::Inserted);
    }

    #[test]
    fn retry_is_a_new_attempt_of_the_same_audio() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(9));
        a.app = "Outlook".into();
        a.add_audio(16 * 4_000);
        a.finish_stt(5_000);
        // The stream broke off, and the batch fallback failed too.
        a.stt_done(SttPath::StreamThenBatch, 9_000);
        let failed = a.record(Outcome::Failed, "t1".into(), &s);
        assert_eq!((failed.stt_path, failed.audio_ms, failed.latency_ms), (SttPath::StreamThenBatch, 4_000, 0));
        assert_eq!(failed.outcome, Outcome::Failed);

        a.retry(20_000);
        assert_eq!(a.stages(), Stages::default(), "the previous attempt's latencies do not carry over to the retry");
        a.stt_done(SttPath::Batch, 21_000);
        a.refine_started(21_000);
        a.refine_done(usage(50, 9), RefineOutcome::Refined, 21_500);
        a.insert_started(21_500);
        a.inserted(21_550);
        let ok = a.record(Outcome::Inserted, "t2".into(), &s);
        assert_eq!(ok.stt_path, SttPath::Batch, "a retry is batch transcription only");
        assert_eq!(ok.audio_ms, 4_000, "the same audio");
        assert_eq!((ok.input_tokens, ok.output_tokens), (50, 9));
        assert_eq!(ok.latency_ms, 1_550, "from \"retry\" to Ctrl+V");
        assert_eq!(a.app, "Outlook");
        assert_eq!(a.id, DictationId(9));
    }

    #[test]
    fn retry_clears_refine_of_failed_attempt() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(1));
        a.finish_stt(1_000);
        a.stt_done(SttPath::Stream, 1_200);
        a.speech_language = Some("ru".into());
        a.refine_started(1_200);
        a.refine_done(usage(80, 8), RefineOutcome::RawKeyProblem, 1_300);
        a.retry(5_000);
        let r = a.record(Outcome::Failed, "t".into(), &s);
        assert_eq!((r.input_tokens, r.output_tokens, r.cache_read_tokens, r.cache_write_tokens), (0, 0, 0, 0));
        assert_eq!(a.refine_outcome, None);
        assert_eq!(a.speech_language, None, "the new transcription gives its own speech language");
        assert_eq!(r.stt_path, SttPath::Batch);
    }

    #[test]
    fn text_retry_pays_no_audio_and_keeps_the_language() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(7));
        a.app = "Telegram Desktop".into();
        a.add_audio(16 * 3_000);
        a.finish_stt(1_000);
        a.stt_done(SttPath::Stream, 1_300);
        a.speech_language = Some("ru".into());
        a.refine_started(1_300);
        a.refine_done(usage(90, 30), RefineOutcome::RawFallback, 2_000);

        a.retry_text(5_000);
        assert_eq!(a.audio_ms(), 0, "the audio was paid for by the first attempt");
        assert_eq!(a.stages(), Stages::default(), "no transcription and no latencies of the failed attempt");
        assert_eq!((a.app.as_str(), a.speech_language.as_deref(), a.id), ("Telegram Desktop", Some("ru"), DictationId(7)));
        assert_eq!(a.refine_outcome, None);
        a.refine_started(5_000);
        a.refine_done(usage(40, 12), RefineOutcome::Refined, 5_600);
        a.insert_started(5_600);
        a.inserted(5_650);
        assert_eq!(a.stages().stt, None);
        let r = a.record(Outcome::Inserted, "t".into(), &s);
        assert_eq!((r.audio_ms, r.input_tokens, r.output_tokens), (0, 40, 12));
        assert_eq!(r.latency_ms, 650, "from \"retry\" to Ctrl+V");
    }

    #[test]
    fn cancelled_or_empty_attempt_has_no_latency() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(2));
        a.add_audio(16 * 700);
        a.finish_stt(3_000);
        a.stt_done(SttPath::Stream, 3_300);
        let r = a.record(Outcome::Cancelled, "t".into(), &s);
        assert_eq!((r.latency_ms, r.audio_ms, r.outcome), (0, 700, Outcome::Cancelled));
        assert_eq!(a.stages(), Stages { stt: Some(300), refine: None, insert: None, latency: 0 });
    }

    #[test]
    fn latency_after_undo_counts_from_undo() {
        let s = Settings::default();
        let mut a = Attempt::new(DictationId(5));
        a.finish_stt(1_000); // Esc while recording: the text is still transcribed
        a.stt_done(SttPath::Stream, 1_300);
        a.undo(4_000); // "restore" after 3 s
        a.refine_started(4_000);
        a.refine_done(usage(10, 2), RefineOutcome::Refined, 4_500);
        a.insert_started(4_500);
        a.inserted(4_520);
        assert_eq!(a.stages(), Stages { stt: Some(300), refine: Some(500), insert: Some(20), latency: 520 });
        assert_eq!(a.record(Outcome::Inserted, "t".into(), &s).latency_ms, 520, "time in \"Cancelled\" is not CUE latency");
    }

    #[test]
    fn discarded_rows_only_after_starting() {
        assert!(!needs_row(Outcome::Discarded, 0));
        assert!(!needs_row(Outcome::Discarded, STARTING_MS - 1), "a system chord during the start-up");
        assert!(needs_row(Outcome::Discarded, STARTING_MS), "lock or pause mid-recording: the stream is paid for");
        for o in [Outcome::Inserted, Outcome::NoField, Outcome::Cancelled, Outcome::Empty, Outcome::Failed] {
            assert!(needs_row(o, 0), "{o:?}");
        }
    }

    /// A log writer into memory, to check the line.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn logged(outcome: Outcome, a: &Attempt) -> String {
        let cap = Capture::default();
        let writer = cap.clone();
        let sub = tracing_subscriber::fmt().with_writer(move || writer.clone()).with_ansi(false).without_time().finish();
        tracing::subscriber::with_default(sub, || log_finished(outcome, a));
        String::from_utf8(cap.0.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn finish_log_has_plain_numbers_and_omits_missing_stages() {
        let mut a = Attempt::new(DictationId(3));
        a.app = "Secret App".into();
        a.add_audio(16 * 1_280);
        a.finish_stt(1_000);
        a.stt_done(SttPath::Stream, 1_287);
        let line = logged(Outcome::Empty, &a);
        assert!(line.contains("outcome=\"empty\"") && line.contains("audio_ms=1280"), "{line}");
        assert!(line.contains("stt_ms=287") && line.contains("latency_ms=0"), "{line}");
        assert!(!line.contains("Some(") && !line.contains("None"), "{line}");
        assert!(!line.contains("refine_ms") && !line.contains("insert_ms") && !line.contains("refine="), "{line}");
        assert!(!line.contains("Secret"), "the app is never written to the log: {line}");

        a.refine_started(1_287);
        a.refine_done(usage(1, 1), RefineOutcome::RawKeyProblem, 1_900);
        let line = logged(Outcome::Failed, &a);
        assert!(line.contains("refine_ms=613") && line.contains("refine=RawKeyProblem"), "{line}");
    }

    fn rec(ts: &str) -> UsageRecord {
        Attempt::new(DictationId(1)).record(Outcome::Inserted, ts.into(), &Settings::default())
    }

    #[test]
    fn rows_that_failed_to_write_are_retried_in_order() {
        let mut log = UsageLog::default();
        let mut file: Vec<String> = Vec::new();
        let locked = |_: &UsageRecord| -> io::Result<()> { Err(io::Error::other("file is busy")) };
        assert!(log.write(rec("1"), locked).is_err());
        assert!(log.write(rec("2"), locked).is_err());
        assert_eq!(log.pending(), 2, "the rows wait in memory");
        log.write(rec("3"), |r| {
            file.push(r.ts.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(file, ["1", "2", "3"], "the pending ones first, in order");
        assert_eq!(log.pending(), 0);
    }

    #[test]
    fn write_stops_at_first_failure_and_keeps_the_rest() {
        let mut log = UsageLog::default();
        let _ = log.write(rec("1"), |_| Err(io::Error::other("busy")));
        let mut calls = 0;
        let mut file: Vec<String> = Vec::new();
        // The first one was appended; on the second the file was busy again.
        let res = log.write(rec("2"), |r| {
            calls += 1;
            if calls == 1 {
                file.push(r.ts.clone());
                Ok(())
            } else {
                Err(io::Error::other("busy again"))
            }
        });
        assert!(res.is_err());
        assert_eq!(file, ["1"]);
        assert_eq!(log.pending(), 1);
        log.write(rec("3"), |r| {
            file.push(r.ts.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(file, ["1", "2", "3"]);
    }

    #[test]
    fn pending_queue_is_bounded() {
        let mut log = UsageLog::default();
        for i in 0..MAX_PENDING + 5 {
            let _ = log.write(rec(&i.to_string()), |_| Err(io::Error::other("busy")));
        }
        assert_eq!(log.pending(), MAX_PENDING);
        assert_eq!(log.dropped(), 5);
        let mut first = None;
        log.write(rec("last"), |r| {
            first.get_or_insert(r.ts.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(first.as_deref(), Some("5"), "the oldest were dropped");
    }

    #[test]
    fn missing_price_is_warned_once() {
        let p = Pricing::default();
        let mut log = UsageLog::default();
        assert!(!log.warn_no_price(&p, "claude-haiku-5-5"), "the price is there");
        assert!(log.warn_no_price(&p, "claude-no-such-model"));
        assert!(!log.warn_no_price(&p, "claude-no-such-model"), "no second warning in a session");
    }
}
