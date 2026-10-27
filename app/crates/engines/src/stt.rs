//! Transcription: shared types (audio comes as 16 kHz mono i16 frames), the streaming session facade
//! and the batch fallback.
use std::future::Future;
use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::{AbortHandle, JoinError, JoinHandle};

use crate::error::EngineError;
use crate::soniox::SonioxConfig;

/// Samples per millisecond at 16 kHz.
pub const SAMPLES_PER_MS: u64 = 16;
/// A frame sent to the service is ~100 ms: 1,600 samples at 16 kHz.
pub const CHUNK_SAMPLES: usize = 1_600;

#[derive(Debug, Clone, PartialEq)]
pub enum AudioMsg {
    Pcm(Vec<i16>),
    Finish,
    Abort,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SttOutput {
    pub text: String,
    pub audio_ms: u64,
}

pub fn pcm_bytes(pcm: &[i16]) -> Vec<u8> {
    pcm.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// Gathers incoming frames of any length (usually 20 ms) into chunks of exactly `size` samples.
/// A remainder shorter than `size` waits for more frames or is taken by [`Chunker::flush`].
#[derive(Debug)]
pub struct Chunker {
    buf: Vec<i16>,
    size: usize,
}

impl Default for Chunker {
    /// Chunks of ~100 ms.
    fn default() -> Self {
        Chunker::new(CHUNK_SAMPLES)
    }
}

impl Chunker {
    pub fn new(size: usize) -> Self {
        assert!(size > 0, "chunk size must be greater than zero");
        Chunker { buf: Vec::with_capacity(size), size }
    }

    /// Adds samples; returns the ready chunks (each exactly `size`), possibly none.
    pub fn push(&mut self, pcm: &[i16]) -> Vec<Vec<i16>> {
        self.buf.extend_from_slice(pcm);
        let full = self.buf.len() - self.buf.len() % self.size;
        let chunks = self.buf[..full].chunks(self.size).map(<[i16]>::to_vec).collect();
        self.buf.drain(..full);
        chunks
    }

    /// The remainder shorter than `size`, if any — before finalize / end of audio.
    pub fn flush(&mut self) -> Option<Vec<i16>> {
        (!self.buf.is_empty()).then(|| std::mem::take(&mut self.buf))
    }
}

/// The fallback makes sense only if the problem is not the key and the recording was not cancelled.
pub fn should_fallback(err: &EngineError) -> bool {
    !matches!(err, EngineError::KeyProblem(_) | EngineError::Aborted)
}

/// Streaming engine: only Soniox for now (ElevenLabs was removed); an enum leaves room for a second one if it comes back.
pub enum Engine {
    Soniox { key: String, cfg: SonioxConfig },
}

/// One streaming session. The connection opens right away; audio sent before it connects queues up in the channel.
/// A session dropped without `finish` / `abort` counts as cancelled for the engine: the audio sender is gone.
pub struct SttSession {
    tx: UnboundedSender<AudioMsg>,
    handle: JoinHandle<Result<SttOutput, EngineError>>,
}

pub fn start(engine: Engine) -> SttSession {
    match engine {
        Engine::Soniox { key, cfg } => SttSession::spawn(|rx| crate::soniox::run_rt(key, cfg, rx)),
    }
}

impl SttSession {
    /// The session task: `run` gets the audio receiver and returns the result.
    fn spawn<F, Fut>(run: F) -> Self
    where
        F: FnOnce(UnboundedReceiver<AudioMsg>) -> Fut,
        Fut: Future<Output = Result<SttOutput, EngineError>> + Send + 'static,
    {
        let (tx, rx) = unbounded_channel();
        SttSession { tx, handle: tokio::spawn(run(rx)) }
    }

    /// An audio frame. If the session has already ended (error, drop), the audio is discarded — `finish` reports the result.
    pub fn push(&self, pcm: Vec<i16>) {
        let _ = self.tx.send(AudioMsg::Pcm(pcm));
    }

    /// Cancel: the engine closes the connection itself; we do not wait for a result.
    pub fn abort(self) {
        let _ = self.tx.send(AudioMsg::Abort);
    }

    /// The final text or an error; no result within `timeout` gives `Timeout`. Once we stop waiting — on the deadline
    /// or by dropping a started `finish` future — the session task is aborted: the result is no longer needed, and
    /// a connection carrying the recording must not live on by itself. A `finish` never polled (dropped before the
    /// first poll) is like a dropped session: `Finish` was not sent, and the engine ends by itself on the closed channel.
    pub async fn finish(mut self, timeout: Duration) -> Result<SttOutput, EngineError> {
        let _ = self.tx.send(AudioMsg::Finish);
        let _abort = AbortOnDrop(self.handle.abort_handle());
        match tokio::time::timeout(timeout, &mut self.handle).await {
            Ok(joined) => joined.unwrap_or_else(|e| Err(join_failure(&e))),
            Err(_) => Err(EngineError::Timeout), // `_abort` aborts the task
        }
    }
}

/// Aborts the session task on drop, whenever `finish` stops waiting. `abort` does not affect
/// a task that has already finished — its result is already taken.
struct AbortOnDrop(AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The session task panicked or was aborted. The panic message stays out of the error: it could hold dictation text.
fn join_failure(e: &JoinError) -> EngineError {
    let what = if e.is_panic() { "transcription task failed" } else { "transcription task aborted" };
    EngineError::Protocol(what.into())
}

/// Soniox batch transcription of the saved recording — both as the fallback and for retry.
/// No `Debug`: it holds the key.
#[derive(Clone)]
pub struct Fallback {
    pub http: reqwest::Client,
    pub soniox_key: String,
    pub model: String,
    pub hints: Vec<String>,
}

impl Fallback {
    pub async fn transcribe(&self, wav_path: &Path) -> Result<SttOutput, EngineError> {
        let wav = tokio::fs::read(wav_path).await.map_err(|e| EngineError::Protocol(format!("no recording: {e}")))?;
        crate::soniox::transcribe_file(&self.http, &self.soniox_key, &self.model, &self.hints, wav).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;
    use tokio::time::Instant;

    #[test]
    fn chunker_joins_20ms_frames_into_100ms() {
        let mut c = Chunker::default();
        for i in 0..4 {
            assert!(c.push(&[i; 320]).is_empty(), "frame {i}: under 100 ms, too early");
        }
        let out = c.push(&[4; 320]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), CHUNK_SAMPLES);
        // Sample order is kept.
        assert_eq!(&out[0][..2], &[0, 0]);
        assert_eq!(out[0][320], 1);
        assert_eq!(out[0][CHUNK_SAMPLES - 1], 4);
        assert_eq!(c.flush(), None, "no remainder after an exact chunk");
    }

    #[test]
    fn chunker_splits_long_frames_and_keeps_remainder() {
        let mut c = Chunker::new(4);
        let out = c.push(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(out, vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
        assert_eq!(c.push(&[11]), Vec::<Vec<i16>>::new());
        assert_eq!(c.push(&[12, 13]), vec![vec![9, 10, 11, 12]]);
        assert_eq!(c.flush(), Some(vec![13]));
        assert_eq!(c.flush(), None, "the remainder is handed out once");
        assert!(c.push(&[]).is_empty());
    }

    #[test]
    fn fallback_rules() {
        assert!(should_fallback(&EngineError::Network("x".into())));
        assert!(should_fallback(&EngineError::Service("x".into())));
        assert!(should_fallback(&EngineError::Timeout));
        assert!(should_fallback(&EngineError::Protocol("x".into())));
        assert!(!should_fallback(&EngineError::KeyProblem("x".into())));
        assert!(!should_fallback(&EngineError::Aborted));
    }

    /// Fake engine: counts the audio it receives and returns its duration on `Finish`.
    async fn counting(mut rx: UnboundedReceiver<AudioMsg>) -> Result<SttOutput, EngineError> {
        let mut samples = 0;
        while let Some(msg) = rx.recv().await {
            match msg {
                AudioMsg::Pcm(pcm) => samples += pcm.len() as u64,
                AudioMsg::Finish => return Ok(SttOutput { text: "готово".into(), audio_ms: samples / SAMPLES_PER_MS }),
                AudioMsg::Abort => break,
            }
        }
        Err(EngineError::Aborted)
    }

    #[tokio::test(start_paused = true)]
    async fn finish_returns_engine_result() {
        let s = SttSession::spawn(counting);
        s.push(vec![0; CHUNK_SAMPLES]);
        s.push(vec![0; 320]);
        let out = s.finish(Duration::from_secs(5)).await;
        assert_eq!(out, Ok(SttOutput { text: "готово".into(), audio_ms: 120 }));
    }

    #[tokio::test(start_paused = true)]
    async fn finish_timeout_aborts_session_task() {
        let (alive, gone) = oneshot::channel::<()>();
        let s = SttSession::spawn(|rx| async move {
            let _held = (rx, alive); // the audio channel and the guard live as long as the task
            std::future::pending().await
        });
        let t0 = Instant::now();
        assert_eq!(s.finish(Duration::from_secs(3)).await, Err(EngineError::Timeout));
        assert_eq!(t0.elapsed(), Duration::from_secs(3), "exactly the deadline");
        // An aborted task is dropped along with the guard; a merely abandoned one would hang on and the deadline would fire.
        let dropped = tokio::time::timeout(Duration::from_secs(60), gone).await;
        assert!(matches!(dropped, Ok(Err(_))), "the session task outlived the timeout");
    }

    #[tokio::test(start_paused = true)]
    async fn failed_session_task_is_protocol_without_panic_text() {
        let s = SttSession::spawn(|mut rx| async move {
            while let Some(msg) = rx.recv().await {
                if msg == AudioMsg::Finish {
                    panic!("секретная диктовка");
                }
            }
            Err(EngineError::Aborted)
        });
        let e = s.finish(Duration::from_secs(5)).await.unwrap_err();
        assert_eq!(e, EngineError::Protocol("transcription task failed".into()));
        assert!(!e.to_string().contains("секрет"), "panic text in the error: {e}");
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_session_task_is_protocol() {
        let task = tokio::spawn(std::future::pending::<()>());
        task.abort();
        let e = join_failure(&task.await.unwrap_err());
        assert_eq!(e, EngineError::Protocol("transcription task aborted".into()));
    }

    #[tokio::test(start_paused = true)]
    async fn push_after_session_end_is_harmless() {
        let s = SttSession::spawn(|rx| async move {
            drop(rx);
            Err(EngineError::Network("dropped".into()))
        });
        s.tx.closed().await;
        assert!(s.handle.is_finished(), "the session task has already ended");
        s.push(vec![0; 320]);
        assert_eq!(s.finish(Duration::from_secs(5)).await, Err(EngineError::Network("dropped".into())));
    }

    #[tokio::test(start_paused = true)]
    async fn dropped_finish_aborts_session_task() {
        let (alive, gone) = oneshot::channel::<()>();
        let s = SttSession::spawn(|rx| async move {
            let _held = (rx, alive); // the audio channel and the guard live as long as the task
            std::future::pending().await
        });
        let t0 = Instant::now();
        // Wait one second of ten for the result, then drop the wait.
        let waited = tokio::time::timeout(Duration::from_secs(1), s.finish(Duration::from_secs(10))).await;
        assert!(waited.is_err(), "no result, the wait was dropped");
        let dropped = tokio::time::timeout(Duration::from_secs(60), gone).await;
        assert!(matches!(dropped, Ok(Err(_))), "the session task outlived the dropped finish");
        assert_eq!(t0.elapsed(), Duration::from_secs(1), "aborted at once, not at the finish deadline");
    }

    /// Fake engine: records everything that arrives until the channel closes, then reports.
    async fn recording(
        mut rx: UnboundedReceiver<AudioMsg>,
        report: oneshot::Sender<Vec<AudioMsg>>,
    ) -> Result<SttOutput, EngineError> {
        let mut got = Vec::new();
        while let Some(msg) = rx.recv().await {
            got.push(msg);
        }
        let _ = report.send(got);
        Err(EngineError::Aborted)
    }

    #[tokio::test(start_paused = true)]
    async fn abort_reaches_engine() {
        let (report, seen) = oneshot::channel();
        let s = SttSession::spawn(|rx| recording(rx, report));
        s.push(vec![1; 4]);
        s.abort();
        assert_eq!(seen.await.unwrap(), vec![AudioMsg::Pcm(vec![1; 4]), AudioMsg::Abort]);
    }

    #[tokio::test(start_paused = true)]
    async fn dropped_session_is_left_to_engine() {
        // Without finish the task is not aborted: the engine sees the closed channel and ends by itself, gently.
        let (report, seen) = oneshot::channel();
        let s = SttSession::spawn(|rx| recording(rx, report));
        s.push(vec![1; 4]);
        drop(s);
        assert_eq!(seen.await.unwrap(), vec![AudioMsg::Pcm(vec![1; 4])]);
    }

    #[tokio::test]
    async fn fallback_without_recording_is_protocol() {
        // A proxy to nowhere: had it come to the network, the request would not leave this computer.
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:9").unwrap())
            .build()
            .unwrap();
        let fb = Fallback { http, soniox_key: "test-key".into(), model: "stt-async-v5".into(), hints: vec![] };
        // Our own freshly created folder: the file surely is not there (`create_dir` fails if the folder exists).
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("cue-stt-test-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("no-recording.wav");
        let e = fb.transcribe(&path).await.unwrap_err();
        std::fs::remove_dir(&dir).unwrap();
        let EngineError::Protocol(m) = &e else { panic!("expected Protocol, not {e:?}") };
        assert!(m.starts_with("no recording"), "{m}");
        assert!(!m.contains(path.to_str().unwrap()) && !m.contains("no-recording"), "path in the error: {m}");
    }
}
