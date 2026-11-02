//! The speech model on this computer (spec "Local speech recognition"): its state as the windows show it, the
//! download and the load in the background, and the slot recognition waits on. A dictation never waits for a load
//! once CUE has been running a few seconds; one finished while the model is still downloading waits in "Thinking".
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cue_core::model::{Manifest, ModelState, ULTRA};
use cue_engines::EngineError;
use cue_engines::download;
use cue_engines::parakeet::model::{Recognizer, processor_supported};
use cue_engines::stt::{Partial, Recognize, SttOutput};
use tokio::sync::mpsc::UnboundedSender;

use super::CoreEvent;

/// The model CUE ships.
pub const MANIFEST: Manifest = ULTRA;
/// The longest recognition waits for a model still downloading or loading.
pub const WAIT_FOR_MODEL: Duration = Duration::from_secs(30 * 60);
/// The longest a recognition of a ready model may take: a 9-minute dictation takes well under a minute.
pub const RECOGNIZE_TIMEOUT: Duration = Duration::from_secs(120);

/// What happened to the model in the background.
pub enum ModelEvent {
    Progress { done: u64, total: u64 },
    Fetched,
    FetchFailed(EngineError),
    Loaded(Result<Arc<Recognizer>, EngineError>),
    /// A failed load's files were checked again, and the damaged ones deleted.
    Verified,
}

/// The model's state after `event`, from `state`; `present` — bytes of the model already in its folder.
pub fn state_after(state: ModelState, event: &ModelEvent, present: u64, total: u64) -> ModelState {
    match event {
        ModelEvent::Progress { done, total } => ModelState::Downloading { done: *done, total: *total },
        ModelEvent::Fetched => ModelState::Loading,
        ModelEvent::FetchFailed(_) => ModelState::Failed { done: present, total },
        ModelEvent::Loaded(Ok(_)) => ModelState::Ready { total },
        ModelEvent::Loaded(Err(EngineError::Unsupported(_))) => ModelState::Unsupported,
        // A load failure is reported once its files are checked again (`Verified`).
        ModelEvent::Loaded(Err(_)) => state,
        ModelEvent::Verified => ModelState::Failed { done: present, total },
    }
}

/// What recognition waits on: the loaded model, or why there is none.
pub struct ModelSlot {
    slot: Mutex<Slot>,
    changed: Condvar,
}

enum Slot {
    Pending,
    Ready(Arc<dyn Recognize>),
    Failed(EngineError),
}

impl ModelSlot {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { slot: Mutex::new(Slot::Pending), changed: Condvar::new() })
    }

    fn set(&self, slot: Slot) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = slot;
        self.changed.notify_all();
    }

    pub fn pending(&self) {
        self.set(Slot::Pending);
    }

    pub fn ready(&self, model: Arc<dyn Recognize>) {
        self.set(Slot::Ready(model));
    }

    pub fn failed(&self, error: EngineError) {
        self.set(Slot::Failed(error));
    }

    pub fn is_ready(&self) -> bool {
        matches!(*self.slot.lock().unwrap_or_else(PoisonError::into_inner), Slot::Ready(_))
    }

    /// The model once it is ready, waiting while it downloads or loads; the failure that stopped it otherwise.
    fn wait(&self, longest: Duration) -> Result<Arc<dyn Recognize>, EngineError> {
        let deadline = Instant::now() + longest;
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match &*slot {
                Slot::Ready(model) => return Ok(model.clone()),
                Slot::Failed(e) => return Err(e.clone()),
                Slot::Pending => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(EngineError::Model("the model is not ready".into()));
                    }
                    slot = self.changed.wait_timeout(slot, left).unwrap_or_else(PoisonError::into_inner).0;
                }
            }
        }
    }
}

impl Recognize for ModelSlot {
    fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError> {
        self.wait(WAIT_FOR_MODEL)?.recognize(pcm)
    }

    /// Only a model already loaded recognizes while the speaker talks; one still on its way gets the whole recording at
    /// the end.
    fn start(self: Arc<Self>) -> Option<Box<dyn Partial>> {
        let model = match &*self.slot.lock().unwrap_or_else(PoisonError::into_inner) {
            Slot::Ready(model) => model.clone(),
            Slot::Pending | Slot::Failed(_) => return None,
        };
        model.start()
    }
}

/// The model's state, its slot and the background work that moves it.
pub struct ModelKeeper {
    state: ModelState,
    slot: Arc<ModelSlot>,
    dir: PathBuf,
    /// A download ran this time: "Model is ready" is said once it loads.
    downloaded: bool,
}

impl ModelKeeper {
    pub fn new(dir: PathBuf) -> Self {
        Self { state: ModelState::Missing, slot: ModelSlot::new(), dir, downloaded: false }
    }

    pub fn state(&self) -> ModelState {
        self.state
    }

    pub fn slot(&self) -> Arc<ModelSlot> {
        self.slot.clone()
    }

    /// At start: a processor ONNX Runtime cannot run on is said once; a model already here loads; else it downloads.
    pub fn start(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if !processor_supported() {
            tracing::warn!("speech model: this processor has no AVX2, recognition cannot run");
            self.state = ModelState::Unsupported;
            self.slot.failed(EngineError::Unsupported("no AVX2".into()));
            return;
        }
        if download::ready(&MANIFEST, &self.dir) {
            self.load(tx);
        } else {
            self.fetch(tx, http);
        }
    }

    /// Retry after a failure: the download goes on from where it stopped, then the model loads. Nothing to do while
    /// it already downloads or loads, or when it is ready or cannot run.
    pub fn retry(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if matches!(self.state, ModelState::Failed { .. } | ModelState::Missing) {
            self.fetch(tx, http);
        }
    }

    fn fetch(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        let total = MANIFEST.total();
        let done = download::present(&MANIFEST, &self.dir);
        tracing::info!(done, total, "speech model: downloading");
        self.state = ModelState::Downloading { done, total };
        self.slot.pending();
        self.downloaded = true;
        let (tx, http, dir) = (tx.clone(), http.clone(), self.dir.clone());
        tokio::spawn(async move {
            let started = Instant::now();
            // Every percent, not every packet: the windows redraw a bar, not a counter.
            let shown = AtomicU64::new(u64::MAX);
            let progress = |done: u64, total: u64| {
                let percent = done * 100 / total.max(1);
                if shown.swap(percent, Ordering::Relaxed) != percent {
                    let _ = tx.send(CoreEvent::Model(ModelEvent::Progress { done, total }));
                }
            };
            let event = match download::fetch(&http, &MANIFEST, &dir, progress).await {
                Ok(()) => {
                    tracing::info!(seconds = started.elapsed().as_secs(), "speech model: downloaded");
                    ModelEvent::Fetched
                }
                Err(e) => {
                    tracing::warn!("speech model: download stopped: {e}");
                    ModelEvent::FetchFailed(e)
                }
            };
            let _ = tx.send(CoreEvent::Model(event));
        });
    }

    fn load(&mut self, tx: &UnboundedSender<CoreEvent>) {
        self.state = ModelState::Loading;
        self.slot.pending();
        let (tx, dir) = (tx.clone(), self.dir.clone());
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let loaded = Recognizer::load(&dir).map(Arc::new);
            match &loaded {
                Ok(_) => tracing::info!(ms = started.elapsed().as_millis() as u64, "speech model: loaded"),
                Err(e) => tracing::warn!("speech model: not loaded: {e}"),
            }
            let _ = tx.send(CoreEvent::Model(ModelEvent::Loaded(loaded)));
        });
    }

    /// A background step came: the state moves on, and the next step starts. `true` when the model has just become
    /// ready after a download (the Drop says so).
    pub fn on_event(&mut self, event: ModelEvent, tx: &UnboundedSender<CoreEvent>) -> bool {
        let present = || download::present(&MANIFEST, &self.dir);
        self.state = state_after(self.state, &event, present(), MANIFEST.total());
        match event {
            ModelEvent::Fetched => self.load(tx),
            ModelEvent::FetchFailed(e) => self.slot.failed(EngineError::Model(e.to_string())),
            ModelEvent::Loaded(Ok(model)) => {
                self.slot.ready(model);
                return std::mem::take(&mut self.downloaded);
            }
            ModelEvent::Loaded(Err(e @ EngineError::Unsupported(_))) => self.slot.failed(e),
            ModelEvent::Loaded(Err(e)) => {
                // A file may be damaged: they are checked again, and a bad one goes, to be downloaded on Retry.
                self.slot.failed(e);
                let (tx, dir) = (tx.clone(), self.dir.clone());
                tokio::task::spawn_blocking(move || {
                    let removed = download::verify(&MANIFEST, &dir);
                    tracing::info!(removed, "speech model: files checked again");
                    let _ = tx.send(CoreEvent::Model(ModelEvent::Verified));
                });
            }
            ModelEvent::Progress { .. } | ModelEvent::Verified => {}
        }
        false
    }

    /// How long recognition may take now: longer while the model is still on its way.
    pub fn timeout(&self) -> Duration {
        if self.slot.is_ready() { RECOGNIZE_TIMEOUT } else { WAIT_FOR_MODEL + RECOGNIZE_TIMEOUT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo;
    impl Recognize for Echo {
        fn recognize(&self, pcm: &[i16]) -> Result<SttOutput, EngineError> {
            Ok(SttOutput { text: pcm.len().to_string(), audio_ms: 0, language: None })
        }
    }

    #[test]
    fn states_follow_the_background_steps() {
        let total = 100;
        let s = ModelState::Missing;
        let s = state_after(s, &ModelEvent::Progress { done: 40, total }, 40, total);
        assert_eq!(s, ModelState::Downloading { done: 40, total });
        assert_eq!(state_after(s, &ModelEvent::FetchFailed(EngineError::Timeout), 41, total), ModelState::Failed { done: 41, total });
        let s = state_after(s, &ModelEvent::Fetched, 100, total);
        assert_eq!(s, ModelState::Loading);
        assert_eq!(state_after(s, &ModelEvent::Loaded(Err(EngineError::Unsupported("x".into()))), 100, total), ModelState::Unsupported);
        let still = state_after(s, &ModelEvent::Loaded(Err(EngineError::Model("x".into()))), 100, total);
        assert_eq!(still, ModelState::Loading, "a load failure waits for the files' check");
        assert_eq!(state_after(still, &ModelEvent::Verified, 70, total), ModelState::Failed { done: 70, total });
    }

    #[test]
    fn recognition_waits_for_the_model() {
        let slot = ModelSlot::new();
        let waiting = {
            let slot = slot.clone();
            std::thread::spawn(move || slot.recognize(&[0; 3]))
        };
        std::thread::sleep(Duration::from_millis(30));
        slot.ready(Arc::new(Echo));
        assert_eq!(waiting.join().unwrap().unwrap().text, "3");
    }

    /// A stand-in that recognizes while the speaker talks.
    struct Streaming;
    impl Recognize for Streaming {
        fn recognize(&self, _: &[i16]) -> Result<SttOutput, EngineError> {
            unreachable!()
        }
        fn start(self: Arc<Self>) -> Option<Box<dyn Partial>> {
            Some(Box::new(Tail))
        }
    }
    struct Tail;
    impl Partial for Tail {
        fn add(&mut self, _: &[i16]) -> Result<(), EngineError> {
            Ok(())
        }
        fn finish(self: Box<Self>, tail: &[i16]) -> Result<SttOutput, EngineError> {
            Ok(SttOutput { text: format!("tail {}", tail.len()), audio_ms: 0, language: None })
        }
    }

    #[test]
    fn early_recognition_starts_only_with_a_ready_model() {
        let slot = ModelSlot::new();
        assert!(slot.clone().start().is_none(), "a model on its way: the whole recording at the end");
        slot.failed(EngineError::Model("x".into()));
        assert!(slot.clone().start().is_none());
        slot.ready(Arc::new(Streaming));
        let partial = slot.clone().start().expect("a ready model recognizes while the speaker talks");
        assert_eq!(partial.finish(&[0; 7]).unwrap().text, "tail 7");
    }

    #[test]
    fn recognition_gets_the_failure() {
        let slot = ModelSlot::new();
        slot.failed(EngineError::Model("did not download".into()));
        assert!(matches!(slot.recognize(&[0]), Err(EngineError::Model(_))));
        slot.pending();
        assert!(matches!(slot.wait(Duration::from_millis(10)), Err(EngineError::Model(_))), "a model never ready");
    }
}
