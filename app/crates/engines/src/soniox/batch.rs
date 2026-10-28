//! Soniox batch transcription (stt-async-v5): the fallback when the stream fails, and retry
//! for a saved recording.
//!
//! The REST API was checked against the documentation on 27.10.2026
//! (<https://soniox.com/docs/stt/api-reference>, <https://soniox.com/docs/stt/async/async-transcription>):
//! `POST /files` (multipart, field `file`) → `POST /transcriptions` with `model`, `file_id`, `language_hints`,
//! `enable_language_identification` (always on, as in the stream)
//! → poll `GET /transcriptions/{id}` until `completed` or `error` (the reason is in `error_message`)
//! → `GET /transcriptions/{id}/transcript` (field `text`; `tokens`, each with its `language`)
//! → `DELETE` the transcription and the file.
//! Soniox does not allow deleting a transcription that is still in progress (409); requests have a per-minute limit (429).
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use cue_core::translate::LanguageTally;
use futures_util::future::join_all;
use reqwest::header::{HeaderValue, AUTHORIZATION, RETRY_AFTER};
use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

use super::bearer;
use crate::error::{checked, net, read_json, EngineError};
use crate::stt::SttOutput;

pub const API: &str = "https://api.soniox.com/v1";

/// Batch transcription deadlines (short in tests).
#[derive(Debug, Clone, Copy)]
struct BatchTiming {
    /// Deadline for the whole transcription (upload, transcription, polling, text) on top of the recording's duration.
    base: Duration,
    /// Pause between polls, and after a transient poll failure.
    poll: Duration,
    /// Pause after a 429 without a usable `Retry-After`.
    limited: Duration,
    /// How long we wait for each delete request.
    cleanup: Duration,
    /// Background delete retries.
    retry: Retry,
}

/// Schedule of background delete retries: first the `first` pauses, then every `every`,
/// until their sum exceeds `limit`.
#[derive(Debug, Clone, Copy)]
struct Retry {
    first: [Duration; 3],
    every: Duration,
    limit: Duration,
}

impl Retry {
    /// Pauses before the retries, in order.
    fn pauses(self) -> impl Iterator<Item = Duration> {
        let mut total = Duration::ZERO;
        self.first.into_iter().chain(std::iter::repeat(self.every)).take_while(move |&p| {
            total += p;
            total <= self.limit
        })
    }
}

const BATCH: BatchTiming = BatchTiming {
    base: Duration::from_secs(60),
    poll: Duration::from_millis(400),
    limited: Duration::from_secs(1),
    cleanup: Duration::from_secs(5),
    // 5, 10, 20 s, then once a minute — about 15 minutes in total.
    retry: Retry {
        first: [Duration::from_secs(5), Duration::from_secs(10), Duration::from_secs(20)],
        every: Duration::from_secs(60),
        limit: Duration::from_secs(15 * 60),
    },
};

/// How long one background cleanup can run: the first attempt and all scheduled retries, each with its own deadline.
/// A short-lived process (the bench) should wait this long before exiting ([`wait_cleanups`]).
pub fn cleanup_window() -> Duration {
    let t = BATCH;
    t.retry.pauses().map(|p| p + t.cleanup).sum::<Duration>() + t.cleanup
}

/// Wait for the Soniox background cleanup (deleting audio and transcriptions after failures and cancels), no longer than `timeout`.
/// Returns how many transcriptions are not cleaned up yet: 0 means everything is cleaned up or retrying is pointless (that
/// gets a warning in the log). A short-lived process needs this before exiting: background retries die together
/// with the tokio runtime. The app does not wait — it lives longer than the retry schedule.
pub async fn wait_cleanups(timeout: Duration) -> usize {
    CLEANUPS.wait(timeout).await
}

/// Background cleanup of all of the process's transcriptions.
static CLEANUPS: Cleanups = Cleanups::new();

/// Count of background cleanup tasks, one per transcription. A task leaves the count when its future is dropped:
/// it finished, panicked, was aborted, or the runtime it ran in shut down.
pub(crate) struct Cleanups {
    running: AtomicUsize,
    changed: tokio::sync::Notify,
}

impl Cleanups {
    pub(crate) const fn new() -> Self {
        Cleanups { running: AtomicUsize::new(0), changed: tokio::sync::Notify::const_new() }
    }

    /// A cleanup in `runtime`. It is counted right away, before the first poll: a wait started right after already sees it.
    fn spawn(&'static self, runtime: &tokio::runtime::Handle, task: impl Future<Output = ()> + Send + 'static) {
        self.running.fetch_add(1, Ordering::SeqCst);
        let counted = Counted(self);
        runtime.spawn(async move {
            let _counted = counted;
            task.await;
        });
    }

    async fn wait(&self, timeout: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            // Subscribe before the check: a task that ends between the check and the wait is not lost.
            changed.as_mut().enable();
            let running = self.running.load(Ordering::SeqCst);
            // Deadline passed (or zero): just the count — a timer with 1 ms granularity would give the cleanup extra time.
            if running == 0 || tokio::time::Instant::now() >= deadline {
                return running;
            }
            if tokio::time::timeout_at(deadline, changed).await.is_err() {
                return self.running.load(Ordering::SeqCst);
            }
        }
    }
}

/// Guard inside a cleanup task's future: once dropped, the task has left the count.
struct Counted(&'static Cleanups);

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
        self.0.changed.notify_waiters();
    }
}

#[derive(Debug, Deserialize)]
pub struct TranscriptionStatus {
    pub id: String,
    pub status: String,
    pub error_message: Option<String>,
}

pub fn create_body(model: &str, file_id: &str, hints: &[String]) -> Value {
    json!({ "model": model, "file_id": file_id, "language_hints": hints, "enable_language_identification": true })
}

/// `Ok(true)` — done, `Ok(false)` — still queued or in progress.
pub fn poll_step(s: &TranscriptionStatus) -> Result<bool, EngineError> {
    match s.status.as_str() {
        "completed" => Ok(true),
        "error" | "failed" => Err(EngineError::Service(
            s.error_message.clone().unwrap_or_else(|| "transcription error".into()),
        )),
        _ => Ok(false),
    }
}

/// WAV 16 kHz mono 16-bit: 32 bytes per millisecond after the 44-byte header.
pub fn wav_audio_ms(len_bytes: usize) -> u64 {
    (len_bytes.saturating_sub(44) / 32) as u64
}

/// Batch transcription of a saved WAV: upload → transcription → poll every 400 ms → text.
/// All of it takes no longer than 60 s plus the recording's duration, otherwise `Timeout`. A transient poll failure
/// (network, 5xx, 408, 429) means wait and retry within the same deadline; after a 429 the pause comes
/// from `Retry-After` (otherwise 1 s). An upload or transcription-creation failure is an error right away.
///
/// The audio and the transcription are deleted from Soniox: the recording stays only on this computer, in `last\`.
/// The first attempt runs right after the transcription, for both records together, each no longer than 5 s;
/// the result waits for it, so even a short-lived process cleans up after itself. What was not deleted because of
/// a transient failure (transcription still in progress — 409, network, 5xx, 429) is deleted in the background: retries after
/// 5, 10, 20 s, then once a minute, about 15 minutes in total. A cleanup failure does not change the result.
/// If the future is dropped (a cancel, or retry replaced by a new one), the same cleanup — the first attempt and
/// the retries — starts in the background on drop; outside a tokio runtime there is nothing to run it on.
///
/// The background cleanup can be waited for with [`wait_cleanups`] (the bench, before exiting).
///
/// Cleanup limitations:
/// - the process exited before the background retries and did not wait for them — a record may remain (the window is minutes);
/// - a deadline, network failure or cancel hit the response in which Soniox had already accepted the audio or created
///   the transcription, but the `id` never reached us — we do not know such a record and cannot delete it;
/// - the key was revoked (401/403) — there is nothing left to delete with.
///
/// Neither the key nor the dictation text gets into any error or the log.
pub async fn transcribe_file(
    http: &reqwest::Client,
    key: &str,
    model: &str,
    hints: &[String],
    wav: Vec<u8>,
) -> Result<SttOutput, EngineError> {
    transcribe_at(Service { api: API, t: BATCH, cleanups: &CLEANUPS }, http, key, model, hints, wav).await
}

/// The Soniox REST API for one transcription: the URL, the header with the key, and the key itself — to cut
/// it out of refusal texts. A copy goes to the background cleanup. No `Debug`: it holds the key.
#[derive(Clone)]
struct Rest {
    http: reqwest::Client,
    api: String,
    auth: HeaderValue,
    key: String,
}

impl Rest {
    fn req(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.api))
            .header(AUTHORIZATION, self.auth.clone())
    }

    /// Request → JSON of the success response; any failure is an error right away (upload, transcription creation).
    async fn json<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> Result<T, EngineError> {
        read_json(checked(req.send().await.map_err(net)?, &self.key).await?).await
    }

    /// Polling GET: on a transient failure — network, 5xx and 408 (`Service`), 429 — wait and retry;
    /// the caller holds the overall deadline. Key, money, protocol — an error right away.
    async fn get_patiently<T: DeserializeOwned>(&self, path: &str, t: &BatchTiming) -> Result<T, EngineError> {
        loop {
            let res = match self.req(Method::GET, path).send().await {
                Ok(resp) if resp.status() == StatusCode::TOO_MANY_REQUESTS => {
                    let pause = limited_pause(resp.headers().get(RETRY_AFTER), t);
                    drop(resp); // an unread response does not hold the connection for the whole pause
                    tracing::debug!("Soniox: poll got 429, retrying in {pause:?}");
                    tokio::time::sleep(pause).await;
                    continue;
                }
                Ok(resp) => match checked(resp, &self.key).await {
                    Ok(resp) => read_json(resp).await,
                    Err(e) => Err(e),
                },
                Err(e) => Err(net(e)),
            };
            match res {
                Err(e @ (EngineError::Network(_) | EngineError::Service(_))) => {
                    let what = if matches!(e, EngineError::Network(_)) { "network" } else { "service unavailable" };
                    tracing::debug!("Soniox: poll failed ({what}), retrying");
                    tokio::time::sleep(t.poll).await;
                }
                other => return other,
            }
        }
    }
}

/// Pause after a 429: `Retry-After` in seconds if present and usable (a date is not parsed), otherwise
/// `t.limited`; never shorter than the usual poll — `Retry-After: 0` does not turn polling into a flood of requests.
fn limited_pause(retry_after: Option<&HeaderValue>, t: &BatchTiming) -> Duration {
    let secs = retry_after.and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse::<u64>().ok());
    secs.map_or(t.limited, Duration::from_secs).max(t.poll)
}

/// What was created in Soniox during one transcription — cleaned up whatever the outcome.
#[derive(Debug, Default)]
struct Created {
    file_id: Option<String>,
    transcription_id: Option<String>,
}

/// A record in Soniox to delete: what it is (for the log) and the path (with the `id`; never logged).
#[derive(Debug)]
struct Record {
    what: &'static str,
    path: String,
}

impl Record {
    fn transcription(id: &str) -> Record {
        Record { what: "the transcription", path: format!("/transcriptions/{id}") }
    }

    fn file(id: &str) -> Record {
        Record { what: "the audio", path: format!("/files/{id}") }
    }
}

impl Created {
    fn records(&self) -> Vec<Record> {
        let transcription = self.transcription_id.as_deref().map(Record::transcription);
        transcription.into_iter().chain(self.file_id.as_deref().map(Record::file)).collect()
    }
}

/// What was created in Soniox and what to clean it up with. If the future is dropped before the regular cleanup makes its first
/// attempt, the same cleanup moves to the background on drop (only inside a tokio runtime — otherwise there is nothing to run it on).
/// The background cleanup is counted in `cleanups`, so it can be waited for.
struct Leftovers {
    rest: Rest,
    t: BatchTiming,
    cleanups: &'static Cleanups,
    created: Created,
}

impl Leftovers {
    /// The regular cleanup: the first attempt for all records together (we wait for it), transient failures go to background
    /// retries. A record leaves the latch as soon as it reaches an outcome (deleted or hopeless);
    /// if dropped mid-attempt, the drop retries only the unfinished ones.
    async fn clean(mut self) {
        let (rest, wait) = (&self.rest, self.t.cleanup);
        let Created { file_id, transcription_id } = &mut self.created;
        let (a, b) = futures_util::future::join(
            settle_slot(rest, transcription_id, Record::transcription, wait),
            settle_slot(rest, file_id, Record::file, wait),
        )
        .await;
        // The rest now belong to the background retries; the latch must not repeat them.
        self.created = Created::default();
        let again: Vec<_> = a.into_iter().chain(b).collect();
        if !again.is_empty() {
            self.cleanups.spawn(&tokio::runtime::Handle::current(), retry_later(self.rest.clone(), again, self.t));
        }
    }
}

/// The first attempt for a record from the latch: once it reaches an outcome, the slot is emptied right away.
async fn settle_slot(
    rest: &Rest,
    slot: &mut Option<String>,
    record: fn(&str) -> Record,
    wait: Duration,
) -> Option<(Record, String)> {
    let again = settle(rest, record(slot.as_deref()?), wait, 1).await;
    if again.is_none() {
        *slot = None;
    }
    again
}

impl Drop for Leftovers {
    fn drop(&mut self) {
        let records = self.created.records();
        if records.is_empty() {
            return;
        }
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            self.cleanups.spawn(&runtime, clean_in_background(self.rest.clone(), records, self.t));
        }
    }
}

/// Cleanup after a cancel: the first attempt and, if needed, the retries — entirely in the background.
async fn clean_in_background(rest: Rest, records: Vec<Record>, t: BatchTiming) {
    let again = attempt(&rest, records, t.cleanup, 1).await;
    if !again.is_empty() {
        retry_later(rest, again, t).await;
    }
}

/// Where and by which rules the transcription runs: the REST API URL, the deadlines and the background cleanup count (tests substitute their own).
#[derive(Clone, Copy)]
struct Service<'a> {
    api: &'a str,
    t: BatchTiming,
    cleanups: &'static Cleanups,
}

async fn transcribe_at(
    service: Service<'_>,
    http: &reqwest::Client,
    key: &str,
    model: &str,
    hints: &[String],
    wav: Vec<u8>,
) -> Result<SttOutput, EngineError> {
    let Service { api, t, cleanups } = service;
    let audio_ms = wav_audio_ms(wav.len());
    // A `bearer` refusal is a fixed text without the key; nothing has been created in Soniox yet.
    let rest = Rest { http: http.clone(), api: api.to_owned(), auth: bearer(key)?, key: key.to_owned() };
    let mut left = Leftovers { rest, t, cleanups, created: Created::default() };
    let deadline = t.base + Duration::from_millis(audio_ms);
    let res = tokio::time::timeout(deadline, recognize(&left.rest, &t, model, hints, wav, &mut left.created))
        .await
        .unwrap_or(Err(EngineError::Timeout));
    left.clean().await;
    res.map(|(text, language)| SttOutput { text, audio_ms, language }).map_err(|e| e.redact(key))
}

/// Upload, transcription, polling, text and its language. Whatever is created in Soniox is recorded in `created`
/// right away — the cleanup finds it both after a failure and after the deadline.
async fn recognize(
    rest: &Rest,
    t: &BatchTiming,
    model: &str,
    hints: &[String],
    wav: Vec<u8>,
    created: &mut Created,
) -> Result<(String, Option<String>), EngineError> {
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| EngineError::Protocol(e.to_string()))?;
    let form = reqwest::multipart::Form::new().part("file", part);
    let file: Value = rest.json(rest.req(Method::POST, "/files").multipart(form)).await?;
    let file_id = file["id"].as_str().ok_or_else(|| EngineError::Protocol("no file id".into()))?;
    created.file_id = Some(file_id.to_owned());

    let body = create_body(model, file_id, hints);
    let tr: TranscriptionStatus = rest.json(rest.req(Method::POST, "/transcriptions").json(&body)).await?;
    let path = format!("/transcriptions/{}", tr.id);
    created.transcription_id = Some(tr.id);
    loop {
        let st: TranscriptionStatus = rest.get_patiently(&path, t).await?;
        if poll_step(&st)? {
            break;
        }
        tokio::time::sleep(t.poll).await;
    }
    let transcript: Value = rest.get_patiently(&format!("{path}/transcript"), t).await?;
    transcript_output(&transcript)
}

/// The transcript text and the language of most of it. Per the documentation `text` is always present: if it is
/// missing or not a string, the API has changed. An empty string is a normal result (silence). The language comes from
/// the `tokens` (each may carry `text` and `language`); without them it is `None`, not an error. The response body
/// never gets into the error.
fn transcript_output(v: &Value) -> Result<(String, Option<String>), EngineError> {
    let text = v
        .get("text")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_owned())
        .ok_or_else(|| EngineError::Protocol("the Soniox response has no transcript text".into()))?;
    let mut languages = LanguageTally::default();
    for token in v.get("tokens").and_then(Value::as_array).into_iter().flatten() {
        if let (Some(piece), Some(language)) = (token["text"].as_str(), token["language"].as_str()) {
            languages.add(language, piece);
        }
    }
    Ok((text, languages.dominant().map(str::to_owned)))
}

/// Outcome of one attempt to delete a record.
enum Deleted {
    /// Deleted — or already gone (404).
    Done,
    /// A transient failure — 409 (still in progress), 408, 429, 5xx, network, timeout: retry later.
    Again(String),
    /// Retrying is pointless: 401/403 (key revoked) and other refusals.
    Never(String),
}

/// One delete, no longer than `wait`. The reason holds only the status or the failure kind (the reqwest text
/// contains the URL, and the URL contains the `id`).
async fn delete(rest: &Rest, path: &str, wait: Duration) -> Deleted {
    let resp = match tokio::time::timeout(wait, rest.req(Method::DELETE, path).send()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(_)) => return Deleted::Again("network".into()),
        Err(_) => return Deleted::Again("timeout".into()),
    };
    let status = resp.status();
    if status.is_success() || status == StatusCode::NOT_FOUND {
        Deleted::Done
    } else if status.is_server_error() || matches!(status.as_u16(), 408 | 409 | 429) {
        Deleted::Again(status.as_u16().to_string())
    } else {
        Deleted::Never(status.as_u16().to_string())
    }
}

/// Attempt number `n` for one record: deleted, or retrying is pointless (then a warning) —
/// `None`; a transient failure — the record and the reason, for a retry.
async fn settle(rest: &Rest, r: Record, wait: Duration, n: usize) -> Option<(Record, String)> {
    match delete(rest, &r.path, wait).await {
        Deleted::Done => None,
        Deleted::Again(why) => Some((r, why)),
        Deleted::Never(why) => {
            tracing::warn!("Soniox: could not delete {} ({why}, attempts: {n}), giving up", r.what);
            None
        }
    }
}

/// Attempt number `n` for all records at once. Returns the ones worth retrying, with the reason.
async fn attempt(rest: &Rest, records: Vec<Record>, wait: Duration, n: usize) -> Vec<(Record, String)> {
    join_all(records.into_iter().map(|r| settle(rest, r, wait, n))).await.into_iter().flatten().collect()
}

/// Background delete retries on the `t.retry` schedule. If even the last one fails, one warning per record.
async fn retry_later(rest: Rest, mut again: Vec<(Record, String)>, t: BatchTiming) {
    let mut n = 1;
    for pause in t.retry.pauses() {
        tokio::time::sleep(pause).await;
        n += 1;
        again = attempt(&rest, again.into_iter().map(|(r, _)| r).collect(), t.cleanup, n).await;
        if again.is_empty() {
            return;
        }
    }
    for (r, why) in again {
        tracing::warn!("Soniox: could not delete {} ({why}, attempts: {n}); the record remains on the server", r.what);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{capture_logs, fake_api, reply, FakeApi, Reply};
    use tokio::time::Instant;

    #[test]
    fn async_create_body() {
        let v = create_body("stt-async-v5", "f-1", &["ru".into(), "en".into()]);
        assert_eq!(
            v,
            json!({
                "model": "stt-async-v5",
                "file_id": "f-1",
                "language_hints": ["ru", "en"],
                "enable_language_identification": true,
            })
        );
    }

    #[test]
    fn async_poll_states() {
        let st = |s: &str| TranscriptionStatus { id: "t".into(), status: s.into(), error_message: Some("bad file".into()) };
        assert_eq!(poll_step(&st("queued")), Ok(false));
        assert_eq!(poll_step(&st("processing")), Ok(false));
        assert_eq!(poll_step(&st("completed")), Ok(true));
        assert_eq!(poll_step(&st("error")), Err(EngineError::Service("bad file".into())));
    }

    #[test]
    fn wav_duration() {
        assert_eq!(wav_audio_ms(44 + 32_000), 1_000);
        assert_eq!(wav_audio_ms(10), 0);
    }

    #[test]
    fn transcript_text_is_required() {
        let text = |v: &Value| transcript_output(v).map(|(text, _)| text);
        assert_eq!(text(&json!({"id": "t", "text": " привет "})), Ok("привет".into()));
        assert_eq!(text(&json!({"text": ""})), Ok(String::new()), "silence is a result too");
        for v in [json!({"id": "t", "tokens": []}), json!({"text": 5}), json!({"text": null})] {
            assert!(matches!(transcript_output(&v), Err(EngineError::Protocol(_))), "{v}");
        }
    }

    #[test]
    fn transcript_language_comes_from_the_tokens() {
        let v = json!({"id": "t-1", "text": "привет", "tokens": [{"text": "привет", "language": "ru"}]});
        assert_eq!(transcript_output(&v), Ok(("привет".into(), Some("ru".into()))));
        assert_eq!(transcript_output(&json!({"id": "t-1", "text": "привет"})), Ok(("привет".into(), None)));
        // Tokens without a language or text, and a `tokens` that is not a list, give no language and no error.
        let v = json!({"text": "ok", "tokens": [{"text": "ok"}, {"language": "en"}, 5]});
        assert_eq!(transcript_output(&v), Ok(("ok".into(), None)));
        assert_eq!(transcript_output(&json!({"text": "ok", "tokens": "en"})), Ok(("ok".into(), None)));
    }

    #[test]
    fn pause_after_429_honors_retry_after() {
        let header = HeaderValue::from_static;
        assert_eq!(limited_pause(None, &BATCH), Duration::from_secs(1));
        assert_eq!(limited_pause(Some(&header("3")), &BATCH), Duration::from_secs(3));
        assert_eq!(
            limited_pause(Some(&header("Tue, 27 Oct 2026 10:00:00 GMT")), &BATCH),
            Duration::from_secs(1),
            "a date instead of seconds gives the usual pause"
        );
        assert_eq!(limited_pause(Some(&header("0")), &BATCH), BATCH.poll, "no more often than the usual poll");
    }

    #[test]
    fn retry_schedule_grows_then_steady_for_about_15_minutes() {
        let s = Duration::from_secs;
        let pauses: Vec<Duration> = BATCH.retry.pauses().collect();
        assert_eq!(pauses[..4], [s(5), s(10), s(20), s(60)]);
        assert!(pauses[3..].iter().all(|&p| p == s(60)));
        let total: Duration = pauses.iter().sum();
        assert!(total > s(14 * 60) && total <= s(15 * 60), "{total:?}");
        // The cleanup window is the pauses plus the deadline of each attempt (the first and all retries).
        let attempts = u32::try_from(pauses.len()).unwrap() + 1;
        assert_eq!(cleanup_window(), total + BATCH.cleanup * attempts);
        assert!(cleanup_window() < s(17 * 60), "{:?}", cleanup_window());
    }

    // ---- Waiting for the background cleanup: virtual time, no network. ----

    #[tokio::test(start_paused = true)]
    async fn cleanup_wait_returns_when_background_tasks_end() {
        let c = own_cleanups();
        let rt = tokio::runtime::Handle::current();
        c.spawn(&rt, tokio::time::sleep(Duration::from_secs(30)));
        c.spawn(&rt, tokio::time::sleep(Duration::from_secs(10)));
        let t0 = Instant::now();
        assert_eq!(c.wait(Duration::from_secs(15 * 60)).await, 0);
        assert_eq!(t0.elapsed(), Duration::from_secs(30), "exactly until the last task ends");
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_wait_reports_what_is_left_at_the_deadline() {
        let c = own_cleanups();
        let rt = tokio::runtime::Handle::current();
        c.spawn(&rt, tokio::time::sleep(Duration::from_secs(10 * 60)));
        c.spawn(&rt, tokio::time::sleep(Duration::from_secs(1)));
        c.spawn(&rt, std::future::pending());
        let t0 = Instant::now();
        assert_eq!(c.wait(Duration::from_secs(5)).await, 2, "one ended, two are still running");
        assert_eq!(t0.elapsed(), Duration::from_secs(5), "exactly the deadline");
        assert_eq!(c.wait(Duration::ZERO).await, 2, "a zero deadline gives just the count");
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_wait_without_tasks_returns_at_once() {
        let c = own_cleanups();
        let t0 = Instant::now();
        assert_eq!(c.wait(Duration::from_secs(3600)).await, 0);
        assert_eq!(t0.elapsed(), Duration::ZERO);
    }

    #[test]
    fn cleanup_task_dropped_with_its_runtime_leaves_the_count() {
        // The runtime running the cleanup shut down (the process is exiting): the task is dropped and leaves the count.
        let c = own_cleanups();
        let gone = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        c.spawn(gone.handle(), std::future::pending());
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        assert_eq!(rt.block_on(c.wait(Duration::ZERO)), 1);
        drop(gone);
        assert_eq!(rt.block_on(c.wait(Duration::ZERO)), 0);
    }

    // ---- A fake Soniox REST API: no network and no new dependencies. ----

    const TEST_KEY: &str = "sk-test-BATCH-0123456789";

    const UPLOADED: &str = r#"{"id":"f-1","filename":"audio.wav","size":48044,"created_at":"2026-10-27T10:00:00Z"}"#;

    fn status(s: &str) -> String {
        format!(r#"{{"id":"t-1","status":"{s}","model":"stt-async-v5","filename":"audio.wav","error_type":null,"error_message":null}}"#)
    }

    /// A normal Soniox: the transcription is queued, ready on the second poll; everything gets deleted.
    fn happy(method: &str, path: &str, nth: usize) -> Reply {
        match (method, path) {
            ("POST", "/files") => reply(201, UPLOADED),
            ("POST", "/transcriptions") => reply(201, &status("queued")),
            ("GET", "/transcriptions/t-1") => reply(200, &status(if nth == 0 { "queued" } else { "completed" })),
            ("GET", "/transcriptions/t-1/transcript") => {
                reply(200, r#"{"id":"t-1","text":" привет ","tokens":[{"text":" привет ","start_ms":0,"end_ms":900,"language":"ru"}]}"#)
            }
            ("DELETE", "/transcriptions/t-1" | "/files/f-1") => reply(204, ""),
            _ => reply(404, r#"{"error_type":"not_found","message":"unknown path"}"#),
        }
    }

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Deadlines for tests on real sockets. The time here is real, not virtual: when paused, tokio
    /// advances the clock to the nearest timer whenever the runtime is idle — including while waiting
    /// for a socket reply; the transcription deadline could fire in the middle of an ordinary request.
    const FAST: BatchTiming = BatchTiming {
        base: Duration::from_secs(60),
        poll: ms(10),
        limited: ms(10),
        cleanup: Duration::from_secs(5),
        // Retries at 20, 40, 60, 80, 100 ms after the first attempt — five of them.
        retry: Retry { first: [ms(20); 3], every: ms(20), limit: ms(100) },
    };

    /// The deadline passes mid-polling: a recording with no audio and 1 s for everything. Upload and transcription
    /// creation finish in time even on a loaded machine — without an `id` there would be nothing to delete, and the test would check the wrong thing.
    const EXPIRED: BatchTiming = BatchTiming { base: Duration::from_secs(1), ..FAST };

    /// One and a half seconds of audio.
    const WAV_LEN: usize = 44 + 32 * 1_500;

    fn hints() -> Vec<String> {
        vec!["ru".into(), "en".into()]
    }

    async fn batch(api: &FakeApi, timing: BatchTiming, wav_len: usize) -> Result<SttOutput, EngineError> {
        batch_counted(api, timing, wav_len, own_cleanups()).await
    }

    /// Like [`batch`], with a given background cleanup count — to wait for exactly that cleanup.
    async fn batch_counted(
        api: &FakeApi,
        timing: BatchTiming,
        wav_len: usize,
        cleanups: &'static Cleanups,
    ) -> Result<SttOutput, EngineError> {
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let service = Service { api: &api.base, t: timing, cleanups };
        transcribe_at(service, &http, TEST_KEY, "stt-async-v5", &hints(), vec![0; wav_len]).await
    }

    /// A cleanup count per test: tests run in parallel, and the shared process count would see other tests' tasks.
    fn own_cleanups() -> &'static Cleanups {
        Box::leak(Box::new(Cleanups::new()))
    }

    /// The error holds no key — neither whole nor in part.
    fn assert_no_key(e: &EngineError) {
        let shown = format!("{e} {e:?}");
        assert!(!shown.contains("BATCH-01"), "key in the error: {shown}");
    }

    /// The first cleanup attempt: both deletes together, in any order.
    fn cleanup_calls(calls: &[String]) -> Vec<String> {
        let mut v = calls.to_vec();
        v.sort();
        v
    }

    const CLEANUP: [&str; 2] = ["DELETE /files/f-1", "DELETE /transcriptions/t-1"];

    fn processing_forever(method: &str, path: &str, nth: usize) -> Reply {
        match (method, path) {
            ("GET", "/transcriptions/t-1") => reply(200, &status("processing")),
            ("DELETE", "/transcriptions/t-1") => reply(409, r#"{"error_type":"transcription_invalid_state"}"#),
            _ => happy(method, path, nth),
        }
    }

    #[tokio::test]
    async fn batch_success_returns_trimmed_text_and_cleans_up() {
        let api = fake_api(happy).await;
        let out = batch(&api, FAST, WAV_LEN).await.unwrap();
        assert_eq!(out, SttOutput { text: "привет".into(), audio_ms: 1_500, language: Some("ru".into()) });
        let calls = api.calls();
        assert_eq!(
            calls[..5],
            [
                "POST /files",
                "POST /transcriptions",
                "GET /transcriptions/t-1",
                "GET /transcriptions/t-1",
                "GET /transcriptions/t-1/transcript",
            ]
        );
        assert_eq!(cleanup_calls(&calls[5..]), CLEANUP);
        assert!(api.every_request_has("authorization", &format!("Bearer {TEST_KEY}")), "the key is in every request's header");
        let upload = api.body(0);
        assert!(upload.len() > WAV_LEN, "the whole recording is uploaded");
        let upload = String::from_utf8_lossy(&upload);
        for part in [r#"name="file""#, r#"filename="audio.wav""#, "audio/wav"] {
            assert!(upload.contains(part), "the upload has no {part}");
        }
        let create: Value = serde_json::from_slice(&api.body(1)).unwrap();
        assert_eq!(create, create_body("stt-async-v5", "f-1", &hints()));
        // Everything was deleted on the first attempt: no background retries.
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.calls().len(), 7);
    }

    #[tokio::test]
    async fn batch_error_status_is_service_error_and_cleans_up() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("GET", "/transcriptions/t-1") => reply(
                200,
                r#"{"id":"t-1","status":"error","error_type":"invalid_audio","error_message":"Audio file could not be decoded."}"#,
            ),
            _ => happy(m, p, n),
        })
        .await;
        let err = batch(&api, FAST, WAV_LEN).await.unwrap_err();
        assert_eq!(err, EngineError::Service("Audio file could not be decoded.".into()));
        assert_eq!(cleanup_calls(&api.calls()[3..]), CLEANUP);
    }

    #[tokio::test]
    async fn batch_refused_create_is_key_problem_and_audio_is_deleted() {
        // The refusal echoes the key; it still does not get into the error.
        let api = fake_api(|m, p, n| match (m, p) {
            ("POST", "/transcriptions") => reply(
                401,
                &format!(r#"{{"status_code":401,"error_type":"unauthenticated","message":"Invalid API key {TEST_KEY}"}}"#),
            ),
            _ => happy(m, p, n),
        })
        .await;
        let err = batch(&api, FAST, WAV_LEN).await.unwrap_err();
        assert!(err.is_key_problem(), "{err:?}");
        assert!(err.to_string().contains("unauthenticated"), "{err}");
        assert_no_key(&err);
        assert_eq!(
            api.calls(),
            ["POST /files", "POST /transcriptions", "DELETE /files/f-1"],
            "there is no transcription, so only the audio is deleted"
        );
    }

    #[tokio::test]
    async fn batch_refused_upload_leaves_nothing_to_delete() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("POST", "/files") => reply(402, r#"{"error_type":"organization_balance_exhausted"}"#),
            _ => happy(m, p, n),
        })
        .await;
        assert!(batch(&api, FAST, WAV_LEN).await.unwrap_err().is_key_problem());
        assert_eq!(api.calls(), ["POST /files"]);
    }

    #[tokio::test]
    async fn batch_deadline_is_timeout_and_cleans_up() {
        let api = fake_api(processing_forever).await;
        let quick = BatchTiming { base: ms(200), ..FAST };
        let t0 = Instant::now();
        let err = batch(&api, quick, 44 + 32 * 300).await.unwrap_err();
        let took = t0.elapsed();
        assert_eq!(err, EngineError::Timeout);
        assert!(took >= ms(500), "the deadline is 200 ms plus 300 ms of recording, but {took:?} passed");
        assert!(took < Duration::from_secs(5), "{took:?}");
        let calls = api.calls();
        assert!(api.count("GET /transcriptions/t-1") > 3, "polling went on until the deadline");
        assert_eq!(cleanup_calls(&calls[calls.len() - 2..]), CLEANUP);
    }

    #[tokio::test]
    async fn batch_leftover_transcription_is_deleted_later() {
        // The deadline passed with the transcription still in progress: the first delete gets 409, the background retry deletes it.
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", "/transcriptions/t-1") if n > 0 => reply(204, ""),
            _ => processing_forever(m, p, n),
        })
        .await;
        assert_eq!(batch(&api, EXPIRED, 44).await, Err(EngineError::Timeout));
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 1, "the result does not wait for the retries");
        assert_eq!(api.count("DELETE /files/f-1"), 1);
        api.wait_for("DELETE /transcriptions/t-1", 2).await;
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 2, "deleted, so no more retries");
        assert_eq!(api.count("DELETE /files/f-1"), 1, "the audio was deleted on the first attempt");
    }

    #[tokio::test]
    async fn batch_background_retries_can_be_waited_for() {
        // The transcription is still in progress: the first delete gets 409, the background retry deletes it; the wait ends with it.
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", "/transcriptions/t-1") if n > 0 => reply(204, ""),
            _ => processing_forever(m, p, n),
        })
        .await;
        let c = own_cleanups();
        assert_eq!(batch_counted(&api, EXPIRED, 44, c).await, Err(EngineError::Timeout));
        assert_eq!(c.wait(Duration::ZERO).await, 1, "the delete retry runs in the background");
        assert_eq!(c.wait(Duration::from_secs(5)).await, 0);
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 2, "we waited for exactly that delete");
    }

    #[tokio::test]
    async fn batch_wait_ends_after_the_last_retry_even_if_it_failed() {
        // The delete was never allowed: after the last attempt the task ended (with a warning), nothing left to wait for.
        let api = fake_api(processing_forever).await;
        let c = own_cleanups();
        assert_eq!(batch_counted(&api, EXPIRED, 44, c).await, Err(EngineError::Timeout));
        assert_eq!(c.wait(Duration::from_secs(5)).await, 0);
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 1 + FAST.retry.pauses().count());
    }

    #[tokio::test]
    async fn batch_wait_reports_unfinished_cleanup_at_the_deadline() {
        let api = fake_api(processing_forever).await;
        let c = own_cleanups();
        // The retry comes only after a minute: at the wait deadline the cleanup is still running.
        let slow = BatchTiming { retry: Retry { first: [Duration::from_secs(60); 3], every: Duration::from_secs(60), limit: Duration::from_secs(600) }, ..EXPIRED };
        assert_eq!(batch_counted(&api, slow, 44, c).await, Err(EngineError::Timeout));
        assert_eq!(c.wait(ms(100)).await, 1);
    }

    #[tokio::test]
    async fn batch_cancelled_call_cleanup_is_counted_at_once() {
        // The future was dropped mid-polling (Ctrl+C in the bench): the cleanup moves to the background and the wait sees it at once.
        let api = fake_api(|m, p, n| match (m, p) {
            ("GET", "/transcriptions/t-1") => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let c = own_cleanups();
        let mut call = Box::pin(batch_counted(&api, FAST, WAV_LEN, c));
        tokio::select! {
            _ = &mut call => panic!("polling hung, the call must not end"),
            () = api.wait_for("GET /transcriptions/t-1", 1) => {}
        }
        drop(call);
        assert_eq!(c.wait(Duration::ZERO).await, 1);
        assert_eq!(c.wait(Duration::from_secs(5)).await, 0);
        assert_eq!(cleanup_calls(&api.calls()[3..]), CLEANUP);
    }

    #[tokio::test]
    async fn batch_retries_stop_after_the_last_attempt() {
        let api = fake_api(processing_forever).await;
        assert_eq!(batch(&api, EXPIRED, 44).await, Err(EngineError::Timeout));
        let attempts = 1 + FAST.retry.pauses().count();
        assert_eq!(attempts, 6);
        api.wait_for("DELETE /transcriptions/t-1", attempts).await;
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), attempts, "silence after the last attempt");
    }

    #[tokio::test]
    async fn batch_revoked_key_or_missing_record_is_not_retried() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", "/transcriptions/t-1") => reply(401, r#"{"error_type":"unauthenticated"}"#),
            ("DELETE", "/files/f-1") => reply(404, r#"{"error_type":"file_not_found"}"#),
            _ => processing_forever(m, p, n),
        })
        .await;
        assert_eq!(batch(&api, EXPIRED, 44).await, Err(EngineError::Timeout));
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 1, "the key is revoked, nothing to retry with");
        assert_eq!(api.count("DELETE /files/f-1"), 1, "404 means the record is already gone");
    }

    #[tokio::test]
    async fn batch_hung_cleanup_does_not_hold_result() {
        // Both deletes hang: they run together, so the result waits one cleanup deadline, not two.
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", _) => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let t0 = Instant::now();
        let out = batch(&api, BatchTiming { cleanup: Duration::from_secs(1), ..FAST }, WAV_LEN).await;
        let took = t0.elapsed();
        assert_eq!(out.unwrap().text, "привет", "a cleanup failure does not change the result");
        // Together: one deadline (1 s) plus the transcription itself; one after the other would be two deadlines, ≥ 2 s.
        // The 600 ms margin covers the transcription and a loaded machine.
        assert!(took >= ms(1_000) && took < ms(1_600), "one cleanup deadline for both records, but {took:?} passed");
        assert_eq!(cleanup_calls(&api.calls()[5..7]), CLEANUP);
    }

    #[tokio::test]
    async fn batch_cancelled_while_polling_still_cleans_up() {
        // Polling hung and the future was dropped (a cancel): the audio and the transcription are still deleted, in the background.
        let api = fake_api(|m, p, n| match (m, p) {
            ("GET", "/transcriptions/t-1") => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let mut call = Box::pin(batch(&api, FAST, WAV_LEN));
        tokio::select! {
            _ = &mut call => panic!("polling hung, the call must not end"),
            () = api.wait_for("GET /transcriptions/t-1", 1) => {}
        }
        assert_eq!(api.count("DELETE /files/f-1"), 0);
        drop(call);
        api.wait_for("DELETE /transcriptions/t-1", 1).await;
        api.wait_for("DELETE /files/f-1", 1).await;
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 1, "deleted, so no retries");
        assert_eq!(api.count("DELETE /files/f-1"), 1);
    }

    #[tokio::test]
    async fn batch_cancelled_during_cleanup_cleans_up_again() {
        // The first cleanup attempt hung, and then the future was dropped: the unfinished deletes are retried in the background.
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", _) if n == 0 => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let mut call = Box::pin(batch(&api, FAST, WAV_LEN));
        tokio::select! {
            _ = &mut call => panic!("the cleanup hung, the call must not end"),
            () = async {
                api.wait_for("DELETE /transcriptions/t-1", 1).await;
                api.wait_for("DELETE /files/f-1", 1).await;
            } => {}
        }
        drop(call);
        api.wait_for("DELETE /transcriptions/t-1", 2).await;
        api.wait_for("DELETE /files/f-1", 2).await;
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 2);
        assert_eq!(api.count("DELETE /files/f-1"), 2);
    }

    #[tokio::test]
    async fn batch_cancelled_mid_cleanup_does_not_resend_finished_records() {
        // Deleting the transcription is hopeless (401), the audio delete hung — and the future was dropped.
        // The drop retries only the audio; the transcription gets one warning, without the id or the key.
        let (logs, _guard) = capture_logs();
        let api = fake_api(|m, p, n| match (m, p) {
            ("DELETE", "/transcriptions/t-1") => reply(401, r#"{"error_type":"unauthenticated"}"#),
            ("DELETE", "/files/f-1") if n == 0 => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let warned = || logs.lock().unwrap().iter().any(|l| l.starts_with("WARN"));
        let mut call = Box::pin(batch(&api, FAST, WAV_LEN));
        tokio::select! {
            _ = &mut call => panic!("the cleanup hung, the call must not end"),
            () = async {
                // Drop once the transcription refusal has been handled while the audio still waits for a reply.
                api.wait_for("DELETE /files/f-1", 1).await;
                while !warned() {
                    tokio::time::sleep(ms(5)).await;
                }
            } => {}
        }
        drop(call);
        api.wait_for("DELETE /files/f-1", 2).await;
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.count("DELETE /transcriptions/t-1"), 1, "it already had an outcome, so no retry");
        assert_eq!(api.count("DELETE /files/f-1"), 2);
        let logs = logs.lock().unwrap();
        let warns: Vec<_> = logs.iter().filter(|l| l.starts_with("WARN")).collect();
        assert_eq!(warns.len(), 1, "{logs:?}");
        assert!(warns[0].contains("transcription") && warns[0].contains("401"), "{warns:?}");
        assert!(
            logs.iter().all(|l| !l.contains("t-1") && !l.contains("f-1") && !l.contains("BATCH-01")),
            "id or key in the log: {logs:?}"
        );
    }

    #[tokio::test]
    async fn batch_cancelled_before_anything_was_created_sends_nothing_more() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("POST", "/files") => Reply::Hang,
            _ => happy(m, p, n),
        })
        .await;
        let mut call = Box::pin(batch(&api, FAST, WAV_LEN));
        tokio::select! {
            _ = &mut call => panic!("the upload hung, the call must not end"),
            () = api.wait_for("POST /files", 1) => {}
        }
        drop(call);
        tokio::time::sleep(ms(200)).await;
        assert_eq!(api.calls(), ["POST /files"], "no id arrived, so nothing to delete");
    }

    #[tokio::test]
    async fn batch_broken_transcript_is_protocol_error_without_text() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("GET", "/transcriptions/t-1/transcript") => reply(200, r#"{"id":"t-1","text":"секретная диктовка"#),
            _ => happy(m, p, n),
        })
        .await;
        let err = batch(&api, FAST, WAV_LEN).await.unwrap_err();
        assert!(matches!(err, EngineError::Protocol(_)), "a broken body is protocol, not network: {err:?}");
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("секрет") && !shown.contains("диктовк"), "dictation text in the error: {shown}");
        assert_eq!(api.count("GET /transcriptions/t-1/transcript"), 1, "protocol is not a transient failure");
        assert_eq!(cleanup_calls(&api.calls()[5..]), CLEANUP);
    }

    #[tokio::test]
    async fn batch_transient_poll_failures_are_waited_out() {
        let api = fake_api(|m, p, n| match (m, p, n) {
            ("GET", "/transcriptions/t-1", 0) => reply(503, r#"{"error_type":"internal_error"}"#),
            ("GET", "/transcriptions/t-1", 1) => reply(429, r#"{"error_type":"limit_exceeded"}"#),
            ("GET", "/transcriptions/t-1", 2) => Reply::Cut,
            ("GET", "/transcriptions/t-1", _) => reply(200, &status("completed")),
            ("GET", "/transcriptions/t-1/transcript", 0) => reply(502, ""),
            _ => happy(m, p, n),
        })
        .await;
        let t0 = Instant::now();
        let out = batch(&api, BatchTiming { limited: ms(300), ..FAST }, WAV_LEN).await.unwrap();
        assert_eq!(out.text, "привет");
        assert!(t0.elapsed() >= ms(300), "after 429 comes the limit pause, {:?} passed", t0.elapsed());
        assert_eq!(api.count("GET /transcriptions/t-1"), 4, "503, 429, a drop — then done");
        assert_eq!(api.count("GET /transcriptions/t-1/transcript"), 2);
    }

    #[tokio::test]
    async fn batch_retry_after_header_sets_the_pause() {
        let api = fake_api(|m, p, n| match (m, p, n) {
            ("GET", "/transcriptions/t-1", 0) => Reply::Send(429, "{}".into(), "retry-after: 0\r\n"),
            _ => happy(m, p, n),
        })
        .await;
        // Without the header we would wait `limited` (10 s); `Retry-After: 0` gives just the usual poll.
        let t0 = Instant::now();
        let out = batch(&api, BatchTiming { limited: Duration::from_secs(10), ..FAST }, WAV_LEN).await;
        assert_eq!(out.unwrap().text, "привет");
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    }

    #[tokio::test]
    async fn batch_key_problem_during_polling_is_fatal_and_cleans_up() {
        let api = fake_api(|m, p, n| match (m, p) {
            ("GET", "/transcriptions/t-1") => reply(401, r#"{"error_type":"unauthenticated","message":"Invalid API key."}"#),
            _ => happy(m, p, n),
        })
        .await;
        let err = batch(&api, FAST, WAV_LEN).await.unwrap_err();
        assert!(err.is_key_problem(), "{err:?}");
        assert_eq!(api.count("GET /transcriptions/t-1"), 1, "a key problem is not a transient failure");
        assert_eq!(cleanup_calls(&api.calls()[3..]), CLEANUP);
    }
}
