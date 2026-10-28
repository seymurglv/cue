//! Soniox: streaming transcription (stt-rt-v5, here) and batch transcription (stt-async-v5, module `batch`) —
//! the fallback when the stream fails, and retry for a saved recording.
//!
//! The streaming protocol was checked against the documentation on 26.10.2026
//! (<https://soniox.com/docs/stt/api-reference/websocket-api>):
//! - the key goes only in the `Authorization: Bearer …` header (the `api_key` field in the first message is deprecated);
//! - the first message is the JSON config, then audio as binary frames (ours are ~100 ms);
//! - `{"type":"finalize"}` → all tokens become final, then the `<fin>` token
//!   (<https://soniox.com/docs/stt/rt/manual-finalization>);
//! - end of audio is an empty text frame (an empty binary frame does not end the stream);
//! - with no audio for over 20 s the stream closes with 408, so during pauses we send `{"type":"keepalive"}`
//!   (<https://soniox.com/docs/stt/rt/connection-keepalive>);
//! - an error is a frame with `error_code` / `error_type` / `error_message`; after it the server closes the connection.
use std::future::Future;
use std::time::Duration;

use cue_core::translate::LanguageTally;
use futures_util::{FutureExt, Sink, SinkExt, Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_tungstenite::tungstenite::{self, client::IntoClientRequest, http, Message};

use crate::error::EngineError;
use crate::stt::{pcm_bytes, AudioMsg, Chunker, SttOutput, SAMPLES_PER_MS};

mod batch;
pub use batch::{
    cleanup_window, create_body, poll_step, transcribe_file, wait_cleanups, wav_audio_ms, TranscriptionStatus, API,
};

pub const RT_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
pub const FINALIZE: &str = r#"{"type":"finalize"}"#;
pub const KEEPALIVE: &str = r#"{"type":"keepalive"}"#;
/// Before finalize we append 200 ms of silence, as the Soniox documentation advises.
const TAIL_SILENCE: usize = 3_200;
/// Session deadlines (short in tests).
#[derive(Debug, Clone, Copy)]
struct Timing {
    /// A pause in sending audio after which we send keepalive (Soniox waits no longer than 20 s).
    keepalive: Duration,
    /// How long after finalize we wait for `<fin>`: past that it is `Timeout`, and the connection closes.
    /// Without this deadline a stuck session would live until the server or the network closed it.
    fin_wait: Duration,
    /// How long we wait for each send step (frame, socket flush); past that it is `Network`.
    send: Duration,
}

const TIMING: Timing = Timing {
    keepalive: Duration::from_secs(5),
    fin_wait: Duration::from_secs(15),
    send: Duration::from_secs(5),
};
/// How long we wait for the goodbye frames to go out: closing must not delay the result or a cancel.
const CLOSE_WAIT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq)]
pub struct SonioxConfig {
    pub model: String,
    pub language_hints: Vec<String>,
    pub app: Option<String>,
    pub terms: Vec<String>,
}

pub fn start_message(cfg: &SonioxConfig) -> Value {
    let mut v = json!({
        "model": cfg.model,
        "audio_format": "pcm_s16le",
        "sample_rate": 16000,
        "num_channels": 1,
        "language_hints": cfg.language_hints,
        "enable_endpoint_detection": false,
        // Language identification is bundled into the hourly price (soniox.com/pricing, FAQ), and the mode may change
        // after the stream starts, so it is always on.
        "enable_language_identification": true,
    });
    let mut ctx = serde_json::Map::new();
    if let Some(app) = &cfg.app {
        ctx.insert("general".into(), json!([{ "key": "app", "value": app }]));
    }
    if !cfg.terms.is_empty() {
        ctx.insert("terms".into(), json!(cfg.terms));
    }
    if !ctx.is_empty() {
        v["context"] = Value::Object(ctx);
    }
    v
}

#[derive(Debug, Deserialize)]
pub struct RtResponse {
    #[serde(default)]
    pub tokens: Vec<RtToken>,
    #[serde(default)]
    pub finished: bool,
    pub error_code: Option<u16>,
    pub error_type: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RtToken {
    pub text: String,
    #[serde(default)]
    pub is_final: bool,
    #[serde(default)]
    pub language: Option<String>,
}

/// Builds the text from final tokens and tallies their languages; `fin_seen` — the end marker after finalize has
/// arrived; `finished` — the server has ended the stream (all audio processed).
#[derive(Debug, Default)]
pub struct Transcript {
    finals: String,
    languages: LanguageTally,
    pub fin_seen: bool,
    pub finished: bool,
}

impl Transcript {
    pub fn apply(&mut self, r: &RtResponse) -> Result<(), EngineError> {
        if let Some(code) = r.error_code {
            let what = format!(
                "{} {}",
                r.error_type.as_deref().unwrap_or(""),
                r.error_message.as_deref().unwrap_or("")
            );
            return Err(EngineError::from_status(code, what.trim()));
        }
        for t in r.tokens.iter().filter(|t| t.is_final) {
            if t.text == "<fin>" {
                self.fin_seen = true;
            } else {
                self.finals.push_str(&t.text);
                if let Some(l) = &t.language {
                    self.languages.add(l, &t.text);
                }
            }
        }
        self.finished |= r.finished;
        Ok(())
    }

    pub fn text(&self) -> String {
        self.finals.trim().to_string()
    }

    /// The language of most of the final text; `None` if no final token named a known language.
    pub fn language(&self) -> Option<String> {
        self.languages.dominant().map(str::to_owned)
    }
}

/// Server frame → response. A parse error holds only the position: serde's text quotes values
/// from the frame, and that may be dictation text (the log must not see it).
fn parse(frame: &str) -> Result<RtResponse, EngineError> {
    serde_json::from_str(frame).map_err(|e| {
        EngineError::Protocol(format!(
            "unexpected Soniox response (line {}, column {})",
            e.line(),
            e.column()
        ))
    })
}

/// Whether the result is ready: after `Finish`, on `<fin>` (or `finished`).
/// `finished` before the recording ends means the server dropped the stream: the text is incomplete, an error (the fallback kicks in).
fn done(tr: &Transcript, finishing: bool) -> Result<bool, EngineError> {
    if tr.finished && !finishing {
        return Err(EngineError::Protocol("Soniox ended the stream before the recording ended".into()));
    }
    Ok(finishing && (tr.fin_seen || tr.finished))
}

/// Audio from the channel → frames for Soniox. Pure logic, no network.
#[derive(Debug, Default)]
struct Uplink {
    chunker: Chunker,
    /// Samples of real audio (without the appended silence).
    samples: u64,
    /// `Finish` has arrived: the remainder, the silence and finalize are already out; no new audio is needed.
    finishing: bool,
    /// The sender left after `Finish`: we no longer listen to the channel.
    closed: bool,
}

impl Uplink {
    /// Frames to send in response to a channel message (`None` means the sender left).
    /// `Pcm` accumulates up to ~100 ms; `Finish` → remainder, 200 ms of silence, finalize.
    /// `Abort` or the sender leaving before `Finish` gives `Err(Aborted)`.
    fn on(&mut self, msg: Option<AudioMsg>) -> Result<Vec<Message>, EngineError> {
        Ok(match msg {
            Some(AudioMsg::Abort) => return Err(EngineError::Aborted),
            None if !self.finishing => return Err(EngineError::Aborted),
            None => {
                self.closed = true;
                Vec::new()
            }
            Some(AudioMsg::Pcm(_) | AudioMsg::Finish) if self.finishing => Vec::new(),
            Some(AudioMsg::Pcm(pcm)) => {
                self.samples += pcm.len() as u64;
                self.chunker.push(&pcm).iter().map(|c| binary(c)).collect()
            }
            Some(AudioMsg::Finish) => {
                self.finishing = true;
                let mut chunks = self.chunker.push(&[0; TAIL_SILENCE]);
                chunks.extend(self.chunker.flush());
                let mut frames: Vec<Message> = chunks.iter().map(|c| binary(c)).collect();
                frames.push(Message::text(FINALIZE));
                frames
            }
        })
    }

    fn audio_ms(&self) -> u64 {
        self.samples / SAMPLES_PER_MS
    }
}

fn binary(pcm: &[i16]) -> Message {
    Message::Binary(pcm_bytes(pcm).into())
}

/// The `Authorization` header with the key, marked sensitive (Debug will not show it).
/// Shared by the streaming and batch paths.
fn bearer(key: &str) -> Result<http::HeaderValue, EngineError> {
    let mut auth = http::HeaderValue::try_from(format!("Bearer {key}"))
        .map_err(|_| EngineError::KeyProblem("the Soniox key has invalid characters".into()))?;
    auth.set_sensitive(true);
    Ok(auth)
}

/// Connection request: the key goes only in the header.
fn rt_request(key: &str) -> Result<http::Request<()>, EngineError> {
    let mut req = RT_URL.into_client_request().map_err(|e| EngineError::Protocol(e.to_string()))?;
    req.headers_mut().insert(http::header::AUTHORIZATION, bearer(key)?);
    Ok(req)
}

/// One dictation: the connection opens right away; audio sent before it connects queues up in the channel `rx`.
/// `Finish` → 200 ms of silence → finalize → wait for `<fin>` → empty text frame (end of audio) → close.
/// `Abort` (or the sender leaving before `Finish`) → close → `Aborted`. A drop before `<fin>` is an error,
/// not a truncated text; no `<fin>` 15 s after finalize is `Timeout`; a send stuck for 5 s is
/// `Network` (or the server's error frame, if it has already arrived). Closing runs in the background; the result does not wait for it.
/// Neither the key nor the dictation text gets into any error.
pub async fn run_rt(
    key: String,
    cfg: SonioxConfig,
    rx: UnboundedReceiver<AudioMsg>,
) -> Result<SttOutput, EngineError> {
    let req = rt_request(&key)?;
    let connect = async {
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .map_err(|e| EngineError::from_ws_redacted(e, &key))?;
        Ok(ws.split())
    };
    session(connect, cfg, rx, TIMING).await.map_err(|e| e.redact(&key))
}

/// How many already-arrived frames we look through for an explanation of a send failure.
const PEEK_FRAMES: usize = 16;

/// Sends the frames as one batch (one socket flush). Each step takes no longer than `wait`:
/// otherwise a link stuck without a drop would hold the session (cancel, timers and reading) indefinitely.
async fn send_all<S>(tx: &mut S, frames: Vec<Message>, wait: Duration) -> Result<(), EngineError>
where
    S: Sink<Message, Error = tungstenite::Error> + Unpin,
{
    let stalled = |_| EngineError::Network("Soniox is not accepting data (send timeout)".into());
    for f in frames {
        tokio::time::timeout(wait, tx.feed(f)).await.map_err(stalled)?.map_err(EngineError::from_ws)?;
    }
    tokio::time::timeout(wait, tx.flush()).await.map_err(stalled)?.map_err(EngineError::from_ws)
}

/// The send failed. If the server managed to send an error frame (401, 402…), that is the reason:
/// otherwise a wrong key would look like a network failure and lead to a useless fallback.
/// Reads only what has already arrived; does not wait.
fn explain<R>(err: EngineError, rxw: &mut R) -> EngineError
where
    R: Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    for _ in 0..PEEK_FRAMES {
        match rxw.next().now_or_never() {
            Some(Some(Ok(Message::Text(t)))) => {
                if let Ok(Err(server)) = parse(&t).map(|r| Transcript::default().apply(&r)) {
                    return server;
                }
            }
            Some(Some(Ok(_))) => {}
            _ => break, // nothing arrived, the stream ended or broke
        }
    }
    err
}

/// Send with an explanation of a failure (see [`explain`]).
async fn send<S, R>(tx: &mut S, rxw: &mut R, frames: Vec<Message>, wait: Duration) -> Result<(), EngineError>
where
    S: Sink<Message, Error = tungstenite::Error> + Unpin,
    R: Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    send_all(tx, frames, wait).await.map_err(|e| explain(e, rxw))
}

/// Closing in the background: the result or a cancel does not wait for the goodbye. `end_of_audio` sends an empty
/// text frame first (end of audio), then Close. We do not wait for the server's reply; after [`CLOSE_WAIT`]
/// the sending half is dropped — the session has already released the other half, so the socket closes.
fn close_in_background<S>(mut tx: S, end_of_audio: bool)
where
    S: Sink<Message, Error = tungstenite::Error> + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let bye = async {
            if end_of_audio {
                tx.feed(Message::text("")).await?;
            }
            tx.send(Message::Close(None)).await
        };
        let _ = tokio::time::timeout(CLOSE_WAIT, bye).await;
    });
}

/// The server closed the connection before the result: the code and reason go to the log.
fn closed_by_server(frame: Option<tungstenite::protocol::CloseFrame>) -> EngineError {
    let why = match frame {
        Some(f) if f.reason.trim().is_empty() => format!(" ({})", f.code),
        Some(f) => format!(" ({}: {})", f.code, f.reason.trim()),
        None => String::new(),
    };
    EngineError::Network(format!("Soniox closed the connection{why}"))
}

/// A session over any connection (a fake one in tests). While `connect` is still running, audio queues up
/// in ~100 ms frames, and a cancel interrupts the connection without waiting for it.
async fn session<C, S, R>(
    connect: C,
    cfg: SonioxConfig,
    mut rx: UnboundedReceiver<AudioMsg>,
    timing: Timing,
) -> Result<SttOutput, EngineError>
where
    C: Future<Output = Result<(S, R), EngineError>>,
    S: Sink<Message, Error = tungstenite::Error> + Unpin + Send + 'static,
    R: Stream<Item = Result<Message, tungstenite::Error>> + Unpin,
{
    let mut up = Uplink::default();
    let mut queued: Vec<Message> = Vec::new();
    tokio::pin!(connect);
    let (mut tx, mut rxw) = loop {
        tokio::select! {
            ws = &mut connect => break ws?,
            msg = rx.recv(), if !up.closed => queued.extend(up.on(msg)?),
        }
    };

    let mut frames = vec![Message::text(start_message(&cfg).to_string())];
    frames.append(&mut queued);
    send(&mut tx, &mut rxw, frames, timing.send).await?;

    let mut tr = Transcript::default();
    // One timer: while recording, keepalive after a pause in sending;
    // after finalize, the deadline for `<fin>` (no more keepalive; a stuck stream is not kept alive).
    let wait = |up: &Uplink| if up.finishing { timing.fin_wait } else { timing.keepalive };
    let timer = tokio::time::sleep(wait(&up));
    tokio::pin!(timer);
    loop {
        tokio::select! {
            msg = rx.recv(), if !up.closed => match up.on(msg) {
                Ok(frames) if frames.is_empty() => {}
                Ok(frames) => {
                    send(&mut tx, &mut rxw, frames, timing.send).await?;
                    timer.as_mut().reset(tokio::time::Instant::now() + wait(&up));
                }
                Err(e) => {
                    close_in_background(tx, false);
                    return Err(e);
                }
            },
            () = &mut timer => {
                if up.finishing {
                    close_in_background(tx, false);
                    return Err(EngineError::Timeout);
                }
                send(&mut tx, &mut rxw, vec![Message::text(KEEPALIVE)], timing.send).await?;
                timer.as_mut().reset(tokio::time::Instant::now() + wait(&up));
            }
            frame = rxw.next() => match frame {
                Some(Ok(Message::Text(t))) => {
                    tr.apply(&parse(&t)?)?;
                    if done(&tr, up.finishing)? {
                        close_in_background(tx, true);
                        return Ok(SttOutput { text: tr.text(), audio_ms: up.audio_ms(), language: tr.language() });
                    }
                }
                Some(Ok(Message::Close(frame))) => return Err(closed_by_server(frame)),
                None => return Err(EngineError::Network("the Soniox connection dropped".into())),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(EngineError::from_ws(e)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, Ordering};
    use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
    use tokio::sync::oneshot;

    fn cfg() -> SonioxConfig {
        SonioxConfig {
            model: "stt-rt-v5".into(),
            language_hints: vec!["ru".into(), "en".into()],
            app: Some("Telegram Desktop".into()),
            terms: vec![],
        }
    }

    #[test]
    fn start_message_shape() {
        let v = start_message(&cfg());
        assert_eq!(v["model"], "stt-rt-v5");
        assert_eq!(v["audio_format"], "pcm_s16le");
        assert_eq!(v["sample_rate"], 16000);
        assert_eq!(v["num_channels"], 1);
        assert_eq!(v["language_hints"], json!(["ru", "en"]));
        assert_eq!(v["enable_endpoint_detection"], false);
        assert_eq!(v["enable_language_identification"], true);
        assert_eq!(v["context"]["general"], json!([{"key": "app", "value": "Telegram Desktop"}]));
        assert!(v["context"].get("terms").is_none(), "an empty vocabulary is not sent");
        assert!(v.get("api_key").is_none(), "the key goes only in the header");
    }

    #[test]
    fn no_context_without_app_and_terms() {
        let mut c = cfg();
        c.app = None;
        assert!(start_message(&c).get("context").is_none());
        c.terms = vec!["Kubernetes".into()];
        assert_eq!(start_message(&c)["context"]["terms"], json!(["Kubernetes"]));
    }

    #[test]
    fn transcript_keeps_only_final_tokens_and_fin() {
        let mut t = Transcript::default();
        let r1: RtResponse = serde_json::from_str(
            r#"{"tokens":[{"text":"При","is_final":true},{"text":"вет","is_final":true},{"text":" как","is_final":false}],"final_audio_proc_ms":10,"total_audio_proc_ms":20}"#,
        ).unwrap();
        t.apply(&r1).unwrap();
        assert_eq!(t.text(), "Привет");
        assert!(!t.fin_seen);
        let r2: RtResponse = serde_json::from_str(
            r#"{"tokens":[{"text":" как дела","is_final":true},{"text":"<fin>","is_final":true}]}"#,
        ).unwrap();
        t.apply(&r2).unwrap();
        assert_eq!(t.text(), "Привет как дела");
        assert!(t.fin_seen);
    }

    #[test]
    fn transcript_reports_the_language_of_most_words() {
        let mut tr = Transcript::default();
        let r: RtResponse = serde_json::from_str(
            r#"{"tokens":[{"text":"Скажи","is_final":true,"language":"ru"},{"text":" Soniox","is_final":true,"language":"en"},
                {"text":" всё","is_final":false,"language":"ru"},{"text":"<fin>","is_final":true}]}"#,
        ).unwrap();
        tr.apply(&r).unwrap();
        // One final word each: a tie goes to the language heard first.
        assert_eq!(tr.language().as_deref(), Some("ru"));
        assert!(Transcript::default().language().is_none());
    }

    #[test]
    fn transcript_language_skips_non_final_tokens() {
        let mut tr = Transcript::default();
        let r: RtResponse = serde_json::from_str(
            r#"{"tokens":[{"text":"Да","is_final":true,"language":"ru"},{"text":" Soniox","is_final":false,"language":"en"},
                {"text":" rocks","is_final":false,"language":"en"}]}"#,
        ).unwrap();
        tr.apply(&r).unwrap();
        assert_eq!(tr.language().as_deref(), Some("ru"), "counting the non-final tokens would make it en");
    }

    #[test]
    fn error_response_maps_to_engine_error() {
        let mut t = Transcript::default();
        let r: RtResponse = serde_json::from_str(
            r#"{"tokens":[],"error_code":401,"error_type":"unauthenticated","error_message":"bad key"}"#,
        ).unwrap();
        assert!(t.apply(&r).unwrap_err().is_key_problem());
    }

    #[test]
    fn pcm_is_little_endian() {
        assert_eq!(pcm_bytes(&[1, -2]), vec![1, 0, 0xFE, 0xFF]);
    }

    // ---- 100 ms frames, finishing, cancel, keepalive, errors. ----

    fn rt(json: &str) -> RtResponse {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn error_frame_with_documented_extra_fields() {
        // The full error shape per the documentation: also more_info and request_id.
        let r = rt(r#"{"tokens":[],"error_code":402,"error_type":"organization_balance_exhausted","error_message":"Balance exhausted","more_info":"https://soniox.com/docs","request_id":"r-1"}"#);
        let e = Transcript::default().apply(&r).unwrap_err();
        assert!(e.is_key_problem());
        assert!(e.to_string().contains("organization_balance_exhausted"), "{e}");
        let r = rt(r#"{"tokens":[],"error_code":408,"error_type":"request_timeout","error_message":"Request timeout."}"#);
        assert!(matches!(Transcript::default().apply(&r), Err(EngineError::Service(_))));
    }

    #[test]
    fn done_waits_for_fin_after_finish_only() {
        let mut t = Transcript::default();
        assert_eq!(done(&t, false), Ok(false));
        assert_eq!(done(&t, true), Ok(false), "after finalize we wait for <fin>");
        t.apply(&rt(r#"{"tokens":[{"text":"<fin>","is_final":true}]}"#)).unwrap();
        assert_eq!(done(&t, true), Ok(true));
        let mut t = Transcript::default();
        t.apply(&rt(r#"{"tokens":[],"finished":true}"#)).unwrap();
        assert_eq!(done(&t, true), Ok(true), "the stream has ended, nothing to wait for");
        assert!(
            matches!(done(&t, false), Err(EngineError::Protocol(_))),
            "the stream ending before the recording ends is an error, not a truncated text"
        );
    }

    fn sizes(frames: &[Message]) -> Vec<usize> {
        frames
            .iter()
            .map(|m| match m {
                Message::Binary(b) => b.len(),
                other => panic!("expected a binary frame, not {other:?}"),
            })
            .collect()
    }

    #[test]
    fn uplink_sends_100ms_frames_then_tail_and_finalize() {
        let mut up = Uplink::default();
        for _ in 0..4 {
            assert!(up.on(Some(AudioMsg::Pcm(vec![5; 320]))).unwrap().is_empty());
        }
        let first = up.on(Some(AudioMsg::Pcm(vec![5; 320]))).unwrap();
        assert_eq!(sizes(&first), vec![3_200], "100 ms = 1,600 samples = 3,200 bytes");
        assert!(up.on(Some(AudioMsg::Pcm(vec![6; 320]))).unwrap().is_empty());

        // Remainder 320 + silence 3,200 = 1,600 + 1,600 + 320, then finalize.
        let mut tail = up.on(Some(AudioMsg::Finish)).unwrap();
        assert_eq!(tail.pop(), Some(Message::text(FINALIZE)));
        assert_eq!(sizes(&tail), vec![3_200, 3_200, 640]);
        let Message::Binary(b) = &tail[0] else { unreachable!() };
        assert_eq!(&b[..2], &6i16.to_le_bytes(), "the audio remainder first");
        assert!(b[640..].iter().all(|&x| x == 0), "then silence");
        assert_eq!(up.audio_ms(), 120, "silence does not count toward the duration");

        // After Finish, audio and a repeated Finish send nothing.
        assert!(up.on(Some(AudioMsg::Pcm(vec![1; 3_200]))).unwrap().is_empty());
        assert!(up.on(Some(AudioMsg::Finish)).unwrap().is_empty());
        assert_eq!(up.audio_ms(), 120);
    }

    #[test]
    fn uplink_abort_and_sender_gone() {
        let mut up = Uplink::default();
        assert_eq!(up.on(Some(AudioMsg::Abort)), Err(EngineError::Aborted));
        assert_eq!(Uplink::default().on(None), Err(EngineError::Aborted), "left without Finish: cancel");

        let mut up = Uplink::default();
        up.on(Some(AudioMsg::Finish)).unwrap();
        assert_eq!(up.on(None), Ok(vec![]), "left after Finish: wait for the result");
        assert!(up.closed);
        let mut up = Uplink::default();
        up.on(Some(AudioMsg::Finish)).unwrap();
        assert_eq!(up.on(Some(AudioMsg::Abort)), Err(EngineError::Aborted), "cancel works after Finish too");
    }

    #[test]
    fn request_carries_key_only_in_sensitive_header() {
        let req = rt_request("sk-test-123").unwrap();
        assert_eq!(req.uri().to_string(), RT_URL);
        let auth = &req.headers()[http::header::AUTHORIZATION];
        assert_eq!(auth, "Bearer sk-test-123");
        assert!(auth.is_sensitive());
        assert!(!format!("{req:?}").contains("sk-test-123"), "the request's Debug does not show the key");
        let err = rt_request("sk-bad\nkey").unwrap_err();
        assert!(err.is_key_problem());
        assert!(!err.to_string().contains("sk-bad"), "{err}");
    }

    // ---- A session over a fake connection: no network. ----

    type FakeSink = Pin<Box<dyn Sink<Message, Error = tungstenite::Error> + Send>>;
    type FakeStream = Pin<Box<dyn Stream<Item = Result<Message, tungstenite::Error>> + Send>>;

    /// How the fake network accepts the client's frames.
    const ACCEPT: u8 = 0;
    /// Frames get "stuck": a send never completes (the network hangs without a drop).
    const STALL: u8 = 1;
    /// A send fails with an I/O error (connection reset).
    const FAIL: u8 = 2;

    /// Fake server: sees everything the client sent and replies on the test's command.
    struct Server {
        sent: UnboundedReceiver<Message>,
        /// `None` means the connection dropped.
        reply: Option<UnboundedSender<Result<Message, tungstenite::Error>>>,
        uplink: Arc<AtomicU8>,
    }

    impl Server {
        fn stall(&self) {
            self.uplink.store(STALL, Ordering::SeqCst);
        }

        fn fail(&self) {
            self.uplink.store(FAIL, Ordering::SeqCst);
        }

        async fn next_sent(&mut self) -> Message {
            tokio::time::timeout(Duration::from_secs(2), self.sent.recv())
                .await
                .expect("the client sent nothing")
                .expect("the client closed its sending side")
        }

        fn say(&self, json: &str) {
            self.reply.as_ref().unwrap().send(Ok(Message::text(json))).unwrap();
        }

        fn nothing_sent(&mut self) -> bool {
            self.sent.try_recv().is_err()
        }
    }

    /// The connection is "being established" until the test opens `gate`.
    fn fake_ws(
        gate: oneshot::Receiver<()>,
    ) -> (impl Future<Output = Result<(FakeSink, FakeStream), EngineError>> + Send, Server) {
        let (sent_tx, sent) = unbounded_channel::<Message>();
        let uplink = Arc::new(AtomicU8::new(ACCEPT));
        let sink = futures_util::sink::unfold((sent_tx, uplink.clone()), |(tx, mode), m: Message| async move {
            match mode.load(Ordering::SeqCst) {
                STALL => std::future::pending::<()>().await,
                FAIL => {
                    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
                    return Err(tungstenite::Error::Io(reset));
                }
                _ => {}
            }
            tx.send(m).map_err(|_| tungstenite::Error::ConnectionClosed)?;
            Ok::<_, tungstenite::Error>((tx, mode))
        });
        let (reply, replies) = unbounded_channel();
        let stream = futures_util::stream::unfold(replies, |mut rx| async move {
            rx.recv().await.map(|m| (m, rx))
        });
        let connect = async move {
            gate.await.map_err(|_| EngineError::Network("no network".into()))?;
            Ok((Box::pin(sink) as FakeSink, Box::pin(stream) as FakeStream))
        };
        (connect, Server { sent, reply: Some(reply), uplink })
    }

    struct Rig {
        audio: UnboundedSender<AudioMsg>,
        srv: Server,
        task: tokio::task::JoinHandle<Result<SttOutput, EngineError>>,
        gate: Option<oneshot::Sender<()>>,
    }

    fn rig(timing: Timing) -> Rig {
        let (gate_tx, gate) = oneshot::channel();
        let (connect, srv) = fake_ws(gate);
        let (audio, rx) = unbounded_channel();
        let task = tokio::spawn(session(connect, cfg(), rx, timing));
        Rig { audio, srv, task, gate: Some(gate_tx) }
    }

    impl Rig {
        /// Opens the connection and checks that the config message went out first.
        async fn connect(&mut self) {
            self.gate.take().unwrap().send(()).unwrap();
            let Message::Text(start) = self.srv.next_sent().await else { panic!("the config goes first") };
            let v: Value = serde_json::from_str(&start).unwrap();
            assert_eq!(v, start_message(&cfg()));
        }

        async fn result(self) -> Result<SttOutput, EngineError> {
            tokio::time::timeout(Duration::from_secs(2), self.task).await.expect("the session hung").unwrap()
        }
    }

    const SLOW: Timing = Timing {
        keepalive: Duration::from_secs(60),
        fin_wait: Duration::from_secs(60),
        send: Duration::from_secs(60),
    };

    async fn settle() {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    const FIN: &str = r#"{"tokens":[{"text":"Привет","is_final":true,"language":"ru"},{"text":" мир","is_final":true,"language":"ru"},{"text":"<fin>","is_final":true}]}"#;

    #[tokio::test(start_paused = true)]
    async fn session_buffers_while_connecting_then_finishes_on_fin() {
        let mut r = rig(SLOW);
        for _ in 0..6 {
            r.audio.send(AudioMsg::Pcm(vec![7; 320])).unwrap();
        }
        settle().await;
        assert!(r.srv.nothing_sent(), "nothing goes out before the connection");
        r.connect().await;
        assert!(matches!(r.srv.next_sent().await, Message::Binary(b) if b.len() == 3_200));
        r.audio.send(AudioMsg::Finish).unwrap();
        for len in [3_200, 3_200, 640] {
            assert!(matches!(r.srv.next_sent().await, Message::Binary(b) if b.len() == len));
        }
        assert_eq!(r.srv.next_sent().await, Message::text(FINALIZE));
        r.srv.say(r#"{"tokens":[{"text":"Привет","is_final":false}]}"#);
        r.srv.say(FIN);
        assert_eq!(r.srv.next_sent().await, Message::text(""), "end of audio is an empty text frame");
        assert!(matches!(r.srv.next_sent().await, Message::Close(_)), "then the close");
        let out = r.result().await.unwrap();
        assert_eq!(out, SttOutput { text: "Привет мир".into(), audio_ms: 120, language: Some("ru".into()) });
    }

    #[tokio::test(start_paused = true)]
    async fn finish_before_connection_is_kept() {
        let mut r = rig(SLOW);
        r.audio.send(AudioMsg::Pcm(vec![1; 320])).unwrap();
        r.audio.send(AudioMsg::Finish).unwrap();
        // The sender may even leave: Finish is already in the channel, so this is not a cancel.
        drop(std::mem::replace(&mut r.audio, unbounded_channel().0));
        settle().await;
        r.connect().await;
        for len in [3_200, 3_200, 640] {
            assert!(matches!(r.srv.next_sent().await, Message::Binary(b) if b.len() == len));
        }
        assert_eq!(r.srv.next_sent().await, Message::text(FINALIZE));
        r.srv.say(FIN);
        assert_eq!(r.result().await.unwrap().audio_ms, 20);
    }

    #[tokio::test(start_paused = true)]
    async fn abort_while_connecting_sends_nothing() {
        let mut r = rig(SLOW);
        r.audio.send(AudioMsg::Pcm(vec![1; 3_200])).unwrap();
        r.audio.send(AudioMsg::Abort).unwrap();
        let gate = r.gate.take(); // the connection never got established
        let mut srv_sent = std::mem::replace(&mut r.srv.sent, unbounded_channel().1);
        assert_eq!(r.result().await, Err(EngineError::Aborted));
        assert_eq!(srv_sent.recv().await, None, "the connection was dropped, nothing was sent");
        drop(gate);
    }

    #[tokio::test(start_paused = true)]
    async fn sender_gone_before_finish_is_abort() {
        let mut r = rig(SLOW);
        r.connect().await;
        let Rig { audio, mut srv, task, .. } = r;
        drop(audio);
        assert!(matches!(srv.next_sent().await, Message::Close(_)));
        let res = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert_eq!(res, Err(EngineError::Aborted));
    }

    #[tokio::test(start_paused = true)]
    async fn abort_after_finish_closes() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.audio.send(AudioMsg::Finish).unwrap();
        for _ in 0..2 {
            r.srv.next_sent().await; // silence
        }
        assert_eq!(r.srv.next_sent().await, Message::text(FINALIZE));
        r.audio.send(AudioMsg::Abort).unwrap();
        assert!(matches!(r.srv.next_sent().await, Message::Close(_)));
        assert_eq!(r.result().await, Err(EngineError::Aborted));
    }

    #[tokio::test(start_paused = true)]
    async fn sender_gone_after_finish_still_gets_text() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.audio.send(AudioMsg::Finish).unwrap();
        let Rig { audio, mut srv, task, .. } = r;
        drop(audio);
        for _ in 0..3 {
            srv.next_sent().await; // silence, silence, finalize
        }
        srv.say(FIN);
        let res = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert_eq!(res.unwrap().text, "Привет мир");
    }

    #[tokio::test(start_paused = true)]
    async fn server_error_frame_ends_session() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.srv.say(r#"{"tokens":[],"error_code":401,"error_type":"unauthenticated","error_message":"Invalid API key."}"#);
        let err = r.result().await.unwrap_err();
        assert!(err.is_key_problem(), "{err:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn connection_lost_before_fin_is_network_error() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.srv.say(r#"{"tokens":[{"text":"Привет","is_final":true}]}"#);
        r.audio.send(AudioMsg::Finish).unwrap();
        for _ in 0..3 {
            r.srv.next_sent().await;
        }
        r.srv.reply = None; // a drop: there will be no final tokens and no <fin>
        assert!(
            matches!(r.result().await, Err(EngineError::Network(_))),
            "an incomplete text is not passed off as the result; the fallback kicks in"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn close_frame_before_fin_is_network_error() {
        use tungstenite::protocol::{frame::coding::CloseCode, CloseFrame};
        let mut r = rig(SLOW);
        r.connect().await;
        let close = CloseFrame { code: CloseCode::Away, reason: "idle".into() };
        r.srv.reply.as_ref().unwrap().send(Ok(Message::Close(Some(close)))).unwrap();
        let err = r.result().await.unwrap_err();
        let EngineError::Network(msg) = &err else { panic!("expected Network, not {err:?}") };
        assert!(msg.contains("1001"), "{msg}");
    }

    #[tokio::test(start_paused = true)]
    async fn transport_error_maps_through_from_ws() {
        let mut r = rig(SLOW);
        r.connect().await;
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        r.srv.reply.as_ref().unwrap().send(Err(tungstenite::Error::Io(io))).unwrap();
        assert!(matches!(r.result().await, Err(EngineError::Network(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn early_finished_is_error() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.srv.say(r#"{"tokens":[],"finished":true}"#);
        assert!(matches!(r.result().await, Err(EngineError::Protocol(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn no_fin_in_time_is_timeout_without_keepalive() {
        let quick = Timing { keepalive: Duration::from_millis(50), fin_wait: Duration::from_millis(300), ..SLOW };
        let mut r = rig(quick);
        r.connect().await;
        r.audio.send(AudioMsg::Finish).unwrap();
        for _ in 0..2 {
            assert!(matches!(r.srv.next_sent().await, Message::Binary(_)));
        }
        assert_eq!(r.srv.next_sent().await, Message::text(FINALIZE));
        let t0 = tokio::time::Instant::now();
        // <fin> does not arrive. No keepalive after finalize: a stuck stream is not kept open.
        assert!(
            matches!(r.srv.next_sent().await, Message::Close(_)),
            "after finalize, only the close on the deadline"
        );
        assert_eq!(t0.elapsed(), Duration::from_millis(300), "exactly fin_wait");
        assert_eq!(r.result().await, Err(EngineError::Timeout));
    }

    #[tokio::test(start_paused = true)]
    async fn keepalive_in_pauses_only() {
        let mut r = rig(Timing { keepalive: Duration::from_millis(150), ..SLOW });
        r.connect().await;
        let t0 = tokio::time::Instant::now();
        assert_eq!(r.srv.next_sent().await, Message::text(KEEPALIVE), "a pause gives keepalive");
        assert_eq!(t0.elapsed(), Duration::from_millis(150), "exactly after the keepalive pause");
        // Audio is flowing: no keepalive needed.
        for _ in 0..6 {
            r.audio.send(AudioMsg::Pcm(vec![0; 1_600])).unwrap();
            assert!(matches!(r.srv.next_sent().await, Message::Binary(_)));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        r.audio.send(AudioMsg::Abort).unwrap();
        assert!(matches!(r.srv.next_sent().await, Message::Close(_)));
        assert_eq!(r.result().await, Err(EngineError::Aborted));
    }

    // ---- Dictation text stays out of errors, send timeout, result before the close. ----

    #[test]
    fn parse_error_keeps_only_position() {
        for frame in [
            r#"{"tokens":"секретная диктовка"}"#,
            r#"{"tokens":[{"text":"секретная","is_final":"диктовка"}]}"#,
            r#"{"tokens":[{"text":"секретная диктовка","is_final":true}"#,
        ] {
            let err = parse(frame).unwrap_err();
            let EngineError::Protocol(msg) = &err else { panic!("expected Protocol, not {err:?}") };
            assert!(!msg.contains("секрет") && !msg.contains("диктовк"), "dictation text in the error: {msg}");
            assert!(msg.contains("line 1, column"), "{msg}");
        }
        assert!(parse(FIN).is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_send_times_out_as_network() {
        let mut r = rig(Timing { send: Duration::from_secs(1), ..SLOW });
        r.connect().await;
        r.srv.stall();
        let t0 = tokio::time::Instant::now();
        r.audio.send(AudioMsg::Pcm(vec![0; 1_600])).unwrap();
        let err = r.result().await.unwrap_err();
        let EngineError::Network(msg) = &err else { panic!("expected Network, not {err:?}") };
        assert!(msg.contains("timeout"), "{msg}");
        assert_eq!(t0.elapsed(), Duration::from_secs(1), "exactly the send deadline");
    }

    #[tokio::test(start_paused = true)]
    async fn send_failure_prefers_buffered_server_error() {
        let mut r = rig(Timing { send: Duration::from_secs(1), ..SLOW });
        r.connect().await;
        r.srv.stall();
        r.audio.send(AudioMsg::Pcm(vec![0; 1_600])).unwrap();
        settle().await; // the client is stuck sending and does not read replies
        r.srv.say(r#"{"tokens":[{"text":"При","is_final":true}]}"#);
        r.srv.say(r#"{"tokens":[],"error_code":402,"error_type":"organization_balance_exhausted","error_message":"Balance exhausted"}"#);
        let err = r.result().await.unwrap_err();
        assert!(err.is_key_problem(), "the server's error frame wins over the send failure: {err:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn failed_send_without_server_word_is_network() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.srv.fail();
        r.audio.send(AudioMsg::Pcm(vec![0; 1_600])).unwrap();
        assert!(matches!(r.result().await, Err(EngineError::Network(_))));
    }

    #[tokio::test(start_paused = true)]
    async fn text_is_returned_before_close_completes() {
        let mut r = rig(SLOW);
        r.connect().await;
        r.audio.send(AudioMsg::Finish).unwrap();
        for _ in 0..3 {
            r.srv.next_sent().await; // silence, silence, finalize
        }
        r.srv.stall(); // the goodbye frames get stuck in the network
        r.srv.say(FIN);
        let Rig { mut srv, task, .. } = r;
        let out = tokio::time::timeout(Duration::from_millis(1), task)
            .await
            .expect("the result waits for the connection to close")
            .unwrap();
        assert_eq!(out.unwrap().text, "Привет мир");
        assert!(srv.reply.as_ref().unwrap().is_closed(), "reading is released at once");
        // The socket does not leak: within CLOSE_WAIT the background close drops the sending side too.
        let t0 = tokio::time::Instant::now();
        let closed = tokio::time::timeout(Duration::from_secs(2), srv.sent.recv()).await;
        assert_eq!(closed.expect("the connection is not closed"), None);
        assert_eq!(t0.elapsed(), CLOSE_WAIT);
    }
}
