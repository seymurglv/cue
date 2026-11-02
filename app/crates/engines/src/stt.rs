//! Transcription: shared types (audio comes as 16 kHz mono i16 frames) and the session facade over the model on this
//! computer.
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::{AbortHandle, JoinError, JoinHandle};

use cue_core::speech;

use crate::error::EngineError;
use crate::parakeet::model::{Recognized, Recognizer, Running};

/// Samples per millisecond at 16 kHz.
pub const SAMPLES_PER_MS: u64 = 16;

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
    /// The speech's language code when the engine tells it; the model on this computer does not (the runtime reads
    /// it from the text, `cue_core::speech::speech_language`).
    pub language: Option<String>,
}

/// What recognizes a recording on this computer: the speech model, or a stand-in in tests.
pub trait Recognize: Send + Sync + 'static {
    /// A whole recording.
    fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError>;

    /// A recognition fed while the speaker talks, when this model can take one now; `None` recognizes the whole
    /// recording at `Finish`.
    fn start(self: Arc<Self>) -> Option<Box<dyn Partial>> {
        None
    }
}

/// A recognition fed piece by piece: the pieces in order while the speaker talks, the tail once they stop.
pub trait Partial: Send + 'static {
    fn add(&mut self, pcm: &[i16]) -> Result<(), EngineError>;
    fn finish(self: Box<Self>, tail: &[i16]) -> Result<SttOutput, EngineError>;
}

fn output(r: Recognized) -> SttOutput {
    tracing::info!(audio_ms = r.audio_ms, took_ms = r.took_ms, masked = r.masked, "recognized on this computer");
    // The language is told from the text by the runtime (`speech::speech_language`).
    SttOutput { text: r.text, audio_ms: r.audio_ms, language: None }
}

impl Recognize for Recognizer {
    fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError> {
        Recognizer::recognize(self, pcm).map(output)
    }

    fn start(self: Arc<Self>) -> Option<Box<dyn Partial>> {
        Some(Box::new(Running::new(self)))
    }
}

impl Partial for Running {
    fn add(&mut self, pcm: &[i16]) -> Result<(), EngineError> {
        Running::add(self, pcm)
    }

    fn finish(self: Box<Self>, tail: &[i16]) -> Result<SttOutput, EngineError> {
        Running::finish(*self, tail).map(output)
    }
}

/// The engine of a session: the model on this computer, which recognizes pieces of a long recording while the speaker
/// talks and the rest at `finish`.
pub enum Engine {
    Local(Arc<dyn Recognize>),
}

/// One streaming session. The connection opens right away; audio sent before it connects queues up in the channel.
/// A session dropped without `finish` / `abort` counts as cancelled for the engine: the audio sender is gone.
pub struct SttSession {
    tx: UnboundedSender<AudioMsg>,
    handle: JoinHandle<Result<SttOutput, EngineError>>,
}

pub fn start(engine: Engine) -> SttSession {
    match engine {
        Engine::Local(model) => SttSession::spawn(|rx| run_local(model, rx)),
    }
}

/// The recognition fed while the speaker talks.
enum Early {
    /// Waiting for the next piece.
    Idle(Box<dyn Partial>),
    /// A piece is being recognized on a blocking thread.
    Busy(JoinHandle<(Box<dyn Partial>, Result<(), EngineError>)>),
    /// None, or a piece failed: the whole recording is recognized at `Finish`.
    Off,
}

impl Early {
    /// Once the piece being recognized is done: `Idle` again, or `Off` if it failed.
    async fn settle(self) -> Self {
        let Self::Busy(piece) = self else { return self };
        match piece.await {
            Ok((partial, Ok(()))) => Self::Idle(partial),
            Ok((_, Err(e))) => {
                tracing::warn!("a piece was not recognized, the whole recording will be: {e}");
                Self::Off
            }
            Err(e) => {
                tracing::warn!("a piece was not recognized, the whole recording will be: {}", join_failure(&e));
                Self::Off
            }
        }
    }
}

/// The local session. Frames are kept; while the speaker talks, a piece cut at a quiet point
/// ([`speech::early_cut`]) is recognized on a blocking thread whenever the one before is done, so at `Finish` only
/// the tail is left. A failed piece or tail has the whole recording recognized instead. `Abort` or a dropped session
/// is a cancel.
async fn run_local(model: Arc<dyn Recognize>, mut rx: UnboundedReceiver<AudioMsg>) -> Result<SttOutput, EngineError> {
    let mut pcm = Vec::new();
    // Samples already handed to the early recognition.
    let mut fed = 0;
    let mut early = model.clone().start().map_or(Early::Off, Early::Idle);
    while let Some(msg) = rx.recv().await {
        match msg {
            AudioMsg::Pcm(frame) => {
                pcm.extend_from_slice(&frame);
                if matches!(&early, Early::Busy(piece) if piece.is_finished()) {
                    early = early.settle().await;
                }
                if let Early::Idle(_) = early
                    && let Some(cut) = speech::early_cut(&pcm[fed..])
                {
                    let Early::Idle(mut partial) = std::mem::replace(&mut early, Early::Off) else { unreachable!() };
                    let piece = pcm[fed..fed + cut].to_vec();
                    fed += cut;
                    early = Early::Busy(tokio::task::spawn_blocking(move || {
                        let added = partial.add(&piece);
                        (partial, added)
                    }));
                }
            }
            AudioMsg::Finish => {
                if let Early::Idle(partial) = early.settle().await {
                    let tail = pcm[fed..].to_vec();
                    match tokio::task::spawn_blocking(move || partial.finish(&tail)).await {
                        Ok(Ok(out)) => return Ok(out),
                        Ok(Err(e)) => tracing::warn!("the tail was not recognized, the whole recording will be: {e}"),
                        Err(e) => tracing::warn!("the tail was not recognized, the whole recording will be: {}", join_failure(&e)),
                    }
                }
                return recognize(model, pcm).await;
            }
            AudioMsg::Abort => break,
        }
    }
    Err(EngineError::Aborted)
}

/// `pcm` recognized on a blocking thread: the model takes a few hundred milliseconds of CPU.
pub async fn recognize(model: Arc<dyn Recognize>, pcm: Vec<i16>) -> Result<SttOutput, EngineError> {
    tokio::task::spawn_blocking(move || model.recognize(&pcm)).await.unwrap_or_else(|e| Err(join_failure(&e)))
}

/// A saved recording (16 kHz mono 16-bit WAV, as CUE writes `current.wav`) recognized: "Retry".
pub async fn recognize_wav(model: Arc<dyn Recognize>, wav: &Path) -> Result<SttOutput, EngineError> {
    let wav = wav.to_owned();
    let pcm = tokio::task::spawn_blocking(move || -> Result<Vec<i16>, EngineError> {
        let reader = hound::WavReader::open(&wav).map_err(|e| EngineError::Protocol(format!("no recording: {e}")))?;
        reader.into_samples::<i16>().collect::<Result<_, _>>().map_err(|e| EngineError::Protocol(format!("recording: {e}")))
    })
    .await
    .unwrap_or_else(|e| Err(join_failure(&e)))?;
    recognize(model, pcm).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;
    use tokio::time::Instant;

    /// Fake engine: counts the audio it receives and returns its duration on `Finish`.
    async fn counting(mut rx: UnboundedReceiver<AudioMsg>) -> Result<SttOutput, EngineError> {
        let mut samples = 0;
        while let Some(msg) = rx.recv().await {
            match msg {
                AudioMsg::Pcm(pcm) => samples += pcm.len() as u64,
                AudioMsg::Finish => {
                    return Ok(SttOutput { text: "готово".into(), audio_ms: samples / SAMPLES_PER_MS, language: None });
                }
                AudioMsg::Abort => break,
            }
        }
        Err(EngineError::Aborted)
    }

    /// A stand-in model: the text says how many samples it got.
    struct Counting;
    impl Recognize for Counting {
        fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError> {
            Ok(SttOutput { text: format!("{} samples", pcm.len()), audio_ms: pcm.len() as u64 / SAMPLES_PER_MS, language: None })
        }
    }

    #[tokio::test]
    async fn a_local_session_recognizes_every_frame_at_finish() {
        let s = start(Engine::Local(Arc::new(Counting)));
        s.push(vec![0; 1_600]);
        s.push(vec![0; 320]);
        let out = s.finish(Duration::from_secs(5)).await.unwrap();
        assert_eq!((out.text.as_str(), out.audio_ms), ("1920 samples", 120));
    }

    /// A stand-in model that recognizes while the speaker talks: the text lists the pieces it got and the tail; a whole
    /// recording says so.
    struct Streaming {
        fail_add: bool,
        fail_finish: bool,
    }
    impl Recognize for Streaming {
        fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError> {
            Ok(SttOutput { text: format!("whole {}", pcm.len()), audio_ms: pcm.len() as u64 / SAMPLES_PER_MS, language: None })
        }
        fn start(self: Arc<Self>) -> Option<Box<dyn Partial>> {
            Some(Box::new(StreamingRun { model: self, pieces: Vec::new() }))
        }
    }
    struct StreamingRun {
        model: Arc<Streaming>,
        pieces: Vec<usize>,
    }
    impl Partial for StreamingRun {
        fn add(&mut self, pcm: &[i16]) -> Result<(), EngineError> {
            if self.model.fail_add {
                return Err(EngineError::Model("piece".into()));
            }
            self.pieces.push(pcm.len());
            Ok(())
        }
        fn finish(self: Box<Self>, tail: &[i16]) -> Result<SttOutput, EngineError> {
            if self.model.fail_finish {
                return Err(EngineError::Model("tail".into()));
            }
            let samples = self.pieces.iter().sum::<usize>() + tail.len();
            Ok(SttOutput {
                text: format!("pieces {:?} tail {}", self.pieces, tail.len()),
                audio_ms: samples as u64 / SAMPLES_PER_MS,
                language: None,
            })
        }
    }

    /// `seconds` of audio in 20 ms frames, then the result.
    async fn dictate(model: Streaming, seconds: usize) -> SttOutput {
        let s = start(Engine::Local(Arc::new(model)));
        for _ in 0..seconds * 50 {
            s.push(vec![1_000; 320]);
        }
        s.finish(Duration::from_secs(5)).await.unwrap()
    }

    #[tokio::test]
    async fn a_long_dictation_is_recognized_in_pieces_while_it_is_spoken() {
        let out = dictate(Streaming { fail_add: false, fail_finish: false }, 40).await;
        assert_eq!(out.audio_ms, 40_000, "every sample once: {}", out.text);
        let (pieces, tail) = out.text.strip_prefix("pieces [").and_then(|t| t.split_once("] tail ")).expect(&out.text);
        let pieces: Vec<usize> = pieces.split(", ").map(|n| n.parse().unwrap()).collect();
        assert!(!pieces.is_empty(), "{}", out.text);
        assert!(pieces.iter().all(|&n| (160_000..=240_000).contains(&n)), "10 to 15 s each: {pieces:?}");
        assert_eq!(pieces.iter().sum::<usize>() + tail.parse::<usize>().unwrap(), 640_000);
    }

    #[tokio::test]
    async fn a_short_dictation_is_all_tail() {
        let out = dictate(Streaming { fail_add: false, fail_finish: false }, 5).await;
        assert_eq!(out.text, "pieces [] tail 80000");
    }

    #[tokio::test]
    async fn a_failed_piece_or_tail_falls_back_to_the_whole_recording() {
        let out = dictate(Streaming { fail_add: true, fail_finish: false }, 40).await;
        assert_eq!(out.text, "whole 640000");
        let out = dictate(Streaming { fail_add: false, fail_finish: true }, 40).await;
        assert_eq!(out.text, "whole 640000");
    }

    #[tokio::test]
    async fn a_local_session_aborted_recognizes_nothing() {
        struct Never;
        impl Recognize for Never {
            fn recognize(&self, _: &[i16]) -> Result<SttOutput, EngineError> {
                panic!("an aborted recording was recognized")
            }
        }
        let s = start(Engine::Local(Arc::new(Never)));
        s.push(vec![0; 320]);
        s.abort();
        let (tx, rx) = unbounded_channel();
        drop(tx);
        assert_eq!(run_local(Arc::new(Never), rx).await, Err(EngineError::Aborted), "a dropped session is a cancel");
    }

    #[tokio::test]
    async fn a_saved_recording_is_recognized_from_its_wav() {
        let path = std::env::temp_dir().join(format!("cue-stt-{}.wav", std::process::id()));
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for _ in 0..1_600 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        let out = recognize_wav(Arc::new(Counting), &path).await.unwrap();
        assert_eq!(out.text, "1600 samples");
        std::fs::remove_file(&path).unwrap();
        let e = recognize_wav(Arc::new(Counting), &path).await.unwrap_err();
        let EngineError::Protocol(m) = &e else { panic!("expected Protocol, not {e:?}") };
        assert!(m.starts_with("no recording") && !m.contains("cue-stt"), "the path stays out of the error: {m}");
    }

    #[tokio::test(start_paused = true)]
    async fn finish_returns_engine_result() {
        let s = SttSession::spawn(counting);
        s.push(vec![0; 1_600]);
        s.push(vec![0; 320]);
        let out = s.finish(Duration::from_secs(5)).await;
        assert_eq!(out, Ok(SttOutput { text: "готово".into(), audio_ms: 120, language: None }));
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
}
