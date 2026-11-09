//! Core tasks (transcription, refine, "Ask") report a result whatever happens: a panic inside a task must not leave
//! the dictation in "Thinking" forever and lose the recording.
use std::future::Future;
use std::panic::AssertUnwindSafe;

use cue_core::flow::RefineOutcome;
use cue_engines::EngineError;
use cue_engines::ask::{AskEnd, AskFailure, AskResult};
use cue_engines::prompt::RefineResult;
use futures_util::FutureExt;

/// The result of `body`; on a panic, the result of `on_panic`. The panic hook logs where it happened; the panic
/// message is not used anywhere.
pub async fn guarded<T>(what: &'static str, body: impl Future<Output = T>, on_panic: impl FnOnce() -> T) -> T {
    match AssertUnwindSafe(body).catch_unwind().await {
        Ok(v) => v,
        Err(_) => {
            tracing::error!("{what}: task failed — using the fallback result");
            on_panic()
        }
    }
}

/// Recognition crashed: for the flow this is a failed recognition, with "retry" from the saved audio.
pub fn stt_panicked() -> EngineError {
    EngineError::Network("transcription task failed".into())
}

/// Refine crashed: the raw text, as with any other refine failure.
pub fn refine_panicked(raw: String) -> RefineResult {
    RefineResult { text: raw, outcome: RefineOutcome::RawFallback, usage: Default::default() }
}

/// "Ask" crashed: for the flow this is a failure with Retry.
pub fn ask_panicked() -> AskResult {
    AskResult { end: AskEnd::Failed(AskFailure::Failed("crashed")), text: String::new(), usage: Default::default() }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn boom() -> u32 {
        let fail = true;
        if fail {
            panic!("test: task failure");
        }
        1
    }

    #[tokio::test]
    async fn result_of_a_normal_task_is_kept() {
        assert_eq!(guarded("test", async { 5 }, || 7).await, 5);
    }

    #[tokio::test]
    async fn panicking_task_still_reports() {
        // As in the app: the task runs on the runtime, the result comes over the core channel.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            let r = guarded("test", boom(), || 7).await;
            let _ = tx.send(r);
        });
        assert_eq!(rx.recv().await, Some(7), "the result arrived although the task body crashed");
    }

    #[test]
    fn panicked_stt_is_a_failure_with_retry() {
        let e = stt_panicked();
        // Not the model's failure (Retry would download it) and not the processor's (no Retry): a plain failure.
        assert!(!matches!(e, EngineError::Model(_) | EngineError::Unsupported(_)), "{e:?}");
        assert!(!e.to_string().contains("test"), "the panic message does not reach the error");
    }

    #[test]
    fn panicked_refine_inserts_raw_text() {
        let r = refine_panicked("сырой текст".into());
        assert_eq!((r.text.as_str(), r.outcome), ("сырой текст", RefineOutcome::RawFallback));
        assert_eq!(r.usage, Default::default());
    }

    #[test]
    fn panicked_ask_is_a_failure_with_retry() {
        assert!(matches!(ask_panicked().end, AskEnd::Failed(AskFailure::Failed(_))));
    }
}
