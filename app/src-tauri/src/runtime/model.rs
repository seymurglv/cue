//! The speech model on this computer (specs "Local speech recognition" and "Choosing the speech model"): the model in
//! use with its state as the windows show it, its download and load in the background, the slot recognition waits on;
//! and a switch to another model, downloaded and loaded beside it, which takes the slot only once it has loaded. A
//! dictation never waits for a load once CUE has been running a few seconds; one finished while the model is still
//! downloading waits in "Thinking".
//!
//! A model at rest is let go: after [`REST_AFTER`] without a dictation, and when the session locks or the computer
//! sleeps, its memory (about 0.75 GB) is freed, the windows still show it ready, and the next dictation loads it again
//! as it starts (about 1.3 s, while the speaker talks); that dictation is recognized whole at the end.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cue_core::model::{self, EntryState, Manifest, ModelEntry, ModelState, ModelsView};
use cue_engines::EngineError;
use cue_engines::download;
use cue_engines::hub::{self, PinError};
use cue_engines::parakeet::model::{Recognizer, processor_supported};
use cue_engines::stt::{Partial, Recognize, SttOutput};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use super::CoreEvent;

/// The longest recognition waits for a model still downloading or loading.
pub const WAIT_FOR_MODEL: Duration = Duration::from_secs(30 * 60);
/// The longest a recognition of a ready model may take: a 9-minute dictation takes well under a minute.
pub const RECOGNIZE_TIMEOUT: Duration = Duration::from_secs(120);
/// A loaded model is let go after this long without a dictation.
pub const REST_AFTER: Duration = Duration::from_secs(15 * 60);

/// The room a download needs on the drive: its bytes and a tenth more.
pub(crate) fn room_for(bytes: u64) -> u64 {
    bytes + bytes / 10
}

/// What happened to a model in the background.
pub enum ModelEvent {
    Progress { done: u64, total: u64 },
    Fetched,
    FetchFailed(EngineError),
    Loaded(Result<Arc<dyn Recognize>, EngineError>),
    /// A failed load's files were checked again: `removed` damaged ones were deleted.
    Verified { removed: usize },
    /// A found model's files were asked of Hugging Face.
    Pinned(Result<Manifest, PinError>),
    /// The model in use has rested long enough without a dictation: it is let go.
    Rest,
}

/// A background step of one job: the model in use's, or a switch's. A step of a job no longer running (a switch
/// stopped by another choice) is ignored.
pub struct ModelNews {
    pub job: u64,
    pub event: ModelEvent,
}

/// The state of the model in use after `event`, from `state`; `present` — bytes of the model already in its folder.
pub fn state_after(state: ModelState, event: &ModelEvent, present: u64, total: u64) -> ModelState {
    match event {
        ModelEvent::Progress { done, total } => ModelState::Downloading { done: *done, total: *total },
        ModelEvent::Fetched => ModelState::Loading,
        ModelEvent::FetchFailed(_) => ModelState::Failed { done: present, total },
        ModelEvent::Loaded(Ok(_)) => ModelState::Ready { total },
        ModelEvent::Loaded(Err(EngineError::Unsupported(_))) => ModelState::Unsupported,
        // A load failure is reported once its files are checked again (`Verified`).
        // A model let go at rest is still ready for the windows: the next dictation loads it.
        ModelEvent::Loaded(Err(_)) | ModelEvent::Pinned(_) | ModelEvent::Rest => state,
        ModelEvent::Verified { .. } => ModelState::Failed { done: present, total },
    }
}

/// What a background step means for the Drop.
#[derive(Debug, PartialEq, Eq)]
pub enum Told {
    Nothing,
    /// The model in use is ready after a download: "Model is ready".
    Ready,
    /// Another model has taken over: `stt.model` is written, and the Drop names it.
    Switched { id: String, name: String },
}

/// The Model page's choice: a model on its list, or a repository the search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Id(String),
    Repo(String),
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

/// A model being switched to: the Model page's row while it is pinned, downloaded and loaded.
struct Switch {
    id: String,
    repo: String,
    /// `None` while a found model is pinned.
    manifest: Option<Manifest>,
    state: EntryState,
    job: u64,
}

impl Switch {
    fn entry(&self) -> ModelEntry {
        match &self.manifest {
            Some(m) => ModelEntry::new(m, self.state),
            None => ModelEntry {
                id: self.id.clone(),
                name: self.repo.clone(),
                repo: self.repo.clone(),
                languages: Vec::new(),
                size: 0,
                state: self.state,
            },
        }
    }
}

/// Loads a model from its folder.
type Loader = fn(&Path) -> Result<Arc<dyn Recognize>, EngineError>;
/// Free bytes on the drive of a folder; `None` — not known.
pub(crate) type FreeSpace = fn(&Path) -> Option<u64>;

fn load_recognizer(dir: &Path) -> Result<Arc<dyn Recognize>, EngineError> {
    Recognizer::load(dir).map(|r| Arc::new(r) as Arc<dyn Recognize>)
}

/// Free bytes on the drive of `dir`, or of its nearest folder that exists.
#[cfg(windows)]
pub(crate) fn free_space(dir: &Path) -> Option<u64> {
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows::core::HSTRING;
    let existing = dir.ancestors().find(|p| p.exists())?;
    let mut free = 0u64;
    // SAFETY: the path is a valid wide string for the call, and `free` outlives it.
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(existing.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

#[cfg(not(windows))]
pub(crate) fn free_space(_: &Path) -> Option<u64> {
    None
}

/// The model in use with its state and slot, a switch to another, and the background work that moves them.
pub struct ModelKeeper {
    /// `%LOCALAPPDATA%\CUE\models`: a folder per model, named by its id.
    models: PathBuf,
    current: Manifest,
    state: ModelState,
    slot: Arc<ModelSlot>,
    /// A download ran this time: "Model is ready" is said once it loads.
    downloaded: bool,
    /// The job of the model in use: its downloads, loads and checks.
    job: u64,
    jobs: u64,
    /// The model in use's download, stopped when a switch takes over.
    fetching: Option<AbortHandle>,
    switch: Option<Switch>,
    /// The switch's download, stopped by another choice.
    switching: Option<AbortHandle>,
    load: Loader,
    free: FreeSpace,
    /// The model in use was let go at rest: the next dictation loads it again.
    resting: bool,
    /// A dictation records or is worked on: the model is not let go meanwhile.
    busy: bool,
    /// The session locked or the computer went to sleep during a dictation: the model is let go once it ends.
    rest_when_done: bool,
    /// The wait before the model is let go, started when a dictation ends.
    rest_timer: Option<AbortHandle>,
    rest_after: Duration,
}

impl ModelKeeper {
    /// The model `chosen` (`stt.model`) in `models`; one CUE cannot find is Ultra.
    pub fn new(models: PathBuf, chosen: &str) -> Self {
        Self::with(models, chosen, load_recognizer, free_space)
    }

    fn with(models: PathBuf, chosen: &str, load: Loader, free: FreeSpace) -> Self {
        let current = model::resolve(&models, chosen).unwrap_or_else(|| {
            tracing::warn!("speech model: {chosen} not found, Parakeet Ultra instead");
            model::ultra()
        });
        Self {
            models,
            current,
            state: ModelState::Missing,
            slot: ModelSlot::new(),
            downloaded: false,
            job: 0,
            jobs: 0,
            fetching: None,
            switch: None,
            switching: None,
            load,
            free,
            resting: false,
            busy: false,
            rest_when_done: false,
            rest_timer: None,
            rest_after: REST_AFTER,
        }
    }

    /// A dictation started (`true`) or ended (`false`). One starting loads a model let go at rest; one ending starts
    /// the wait after which the model is let go ([`REST_AFTER`]), or lets it go now after a lock or sleep came during it.
    pub fn busy(&mut self, busy: bool, tx: &UnboundedSender<CoreEvent>) {
        self.busy = busy;
        if let Some(timer) = self.rest_timer.take() {
            timer.abort();
        }
        if busy {
            self.wake(tx);
        } else if std::mem::take(&mut self.rest_when_done) {
            self.rest();
        } else {
            self.arm_rest(tx);
        }
    }

    /// The wait before the model in use is let go starts again.
    fn arm_rest(&mut self, tx: &UnboundedSender<CoreEvent>) {
        if let Some(timer) = self.rest_timer.take() {
            timer.abort();
        }
        let (tx, job, after) = (tx.clone(), self.job, self.rest_after);
        self.rest_timer = Some(
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                let _ = tx.send(CoreEvent::Model(ModelNews { job, event: ModelEvent::Rest }));
            })
            .abort_handle(),
        );
    }

    /// The session locked or the computer goes to sleep: the model is let go now, or once the dictation ends.
    pub fn rest_soon(&mut self) {
        if self.busy {
            self.rest_when_done = true;
        } else {
            self.rest();
        }
    }

    /// The model in use, ready and not recognizing, is let go: its memory is freed once a recognition still holding it
    /// ends. The windows still show it ready.
    fn rest(&mut self) {
        if self.busy || self.resting || !matches!(self.state, ModelState::Ready { .. }) || !self.slot.is_ready() {
            return;
        }
        tracing::info!(model = self.current.id, "speech model: let go at rest");
        self.resting = true;
        self.slot.pending();
    }

    /// A model let go at rest loads again.
    fn wake(&mut self, tx: &UnboundedSender<CoreEvent>) {
        if std::mem::take(&mut self.resting) {
            tracing::info!(model = self.current.id, "speech model: loads again for a dictation");
            self.spawn_load(self.job, &self.current.id, tx);
        }
    }

    pub fn state(&self) -> ModelState {
        self.state
    }

    pub fn slot(&self) -> Arc<ModelSlot> {
        self.slot.clone()
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.models.join(id)
    }

    fn new_job(&mut self) -> u64 {
        self.jobs += 1;
        self.jobs
    }

    /// At start: a processor ONNX Runtime cannot run on is said once; a model already here loads; else it downloads.
    pub fn start(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if !processor_supported() {
            tracing::warn!("speech model: this processor has no AVX2, recognition cannot run");
            self.state = ModelState::Unsupported;
            self.slot.failed(EngineError::Unsupported("no AVX2".into()));
            return;
        }
        self.job = self.new_job();
        if download::ready(&self.current, &self.dir(&self.current.id)) {
            self.load_current(tx);
        } else {
            self.fetch_current(tx, http);
        }
    }

    /// Retry after a failure: the download goes on from where it stopped, then the model loads. Nothing to do while
    /// it already downloads or loads, or when it is ready or cannot run.
    pub fn retry(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if matches!(self.state, ModelState::Failed { .. } | ModelState::Missing) {
            self.fetch_current(tx, http);
        }
    }

    fn fetch_current(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        let m = self.current.clone();
        let done = download::present(&m, &self.dir(&m.id));
        tracing::info!(done, total = m.total(), "speech model: downloading");
        self.state = ModelState::Downloading { done, total: m.total() };
        self.slot.pending();
        self.downloaded = true;
        self.fetching = Some(self.spawn_fetch(self.job, m, tx, http));
    }

    fn load_current(&mut self, tx: &UnboundedSender<CoreEvent>) {
        self.state = ModelState::Loading;
        self.slot.pending();
        self.spawn_load(self.job, &self.current.id, tx);
    }

    fn spawn_fetch(&self, job: u64, m: Manifest, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) -> AbortHandle {
        let (tx, http, dir) = (tx.clone(), http.clone(), self.dir(&m.id));
        tokio::spawn(async move {
            let started = Instant::now();
            // Every percent, not every packet: the windows redraw a bar, not a counter.
            let shown = AtomicU64::new(u64::MAX);
            let progress = |done: u64, total: u64| {
                let percent = done * 100 / total.max(1);
                if shown.swap(percent, Ordering::Relaxed) != percent {
                    let _ = tx.send(CoreEvent::Model(ModelNews { job, event: ModelEvent::Progress { done, total } }));
                }
            };
            let event = match download::fetch(&http, &m, &dir, progress).await {
                Ok(()) => {
                    tracing::info!(seconds = started.elapsed().as_secs(), model = m.id, "speech model: downloaded");
                    ModelEvent::Fetched
                }
                Err(e) => {
                    tracing::warn!("speech model {}: download stopped: {e}", m.id);
                    ModelEvent::FetchFailed(e)
                }
            };
            let _ = tx.send(CoreEvent::Model(ModelNews { job, event }));
        })
        .abort_handle()
    }

    fn spawn_load(&self, job: u64, id: &str, tx: &UnboundedSender<CoreEvent>) {
        let (tx, dir, load, id) = (tx.clone(), self.dir(id), self.load, id.to_owned());
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let loaded = load(&dir);
            match &loaded {
                Ok(_) => tracing::info!(ms = started.elapsed().as_millis() as u64, model = id, "speech model: loaded"),
                Err(e) => tracing::warn!("speech model {id}: not loaded: {e}"),
            }
            let _ = tx.send(CoreEvent::Model(ModelNews { job, event: ModelEvent::Loaded(loaded) }));
        });
    }

    /// A model that did not load: its files are checked again, and the damaged ones deleted.
    fn spawn_verify(&self, job: u64, m: Manifest, tx: &UnboundedSender<CoreEvent>) {
        let (tx, dir) = (tx.clone(), self.dir(&m.id));
        tokio::task::spawn_blocking(move || {
            let removed = download::verify(&m, &dir);
            tracing::info!(removed, model = m.id, "speech model: files checked again");
            let _ = tx.send(CoreEvent::Model(ModelNews { job, event: ModelEvent::Verified { removed } }));
        });
    }

    /// A background step came: the state moves on, and the next step starts.
    pub fn on_event(&mut self, news: ModelNews, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) -> Told {
        if news.job == self.job {
            return self.on_current(news.event, tx);
        }
        if self.switch.as_ref().is_some_and(|s| s.job == news.job) {
            return self.on_switch(news.event, tx, http);
        }
        Told::Nothing
    }

    fn on_current(&mut self, event: ModelEvent, tx: &UnboundedSender<CoreEvent>) -> Told {
        let m = &self.current;
        let present = download::present(m, &self.dir(&m.id));
        self.state = state_after(self.state, &event, present, m.total());
        match event {
            ModelEvent::Fetched => {
                self.fetching = None;
                self.load_current(tx);
            }
            ModelEvent::FetchFailed(e) => {
                self.fetching = None;
                self.slot.failed(EngineError::Model(e.to_string()));
            }
            ModelEvent::Loaded(Ok(model)) => {
                self.slot.ready(model);
                // Loaded with no dictation going on (at start, after a download): it rests from now on.
                if !self.busy {
                    self.arm_rest(tx);
                }
                if std::mem::take(&mut self.downloaded) {
                    return Told::Ready;
                }
            }
            ModelEvent::Loaded(Err(e @ EngineError::Unsupported(_))) => self.slot.failed(e),
            ModelEvent::Loaded(Err(e)) => {
                // A file may be damaged: they are checked again, and a bad one goes, to be downloaded on Retry.
                self.slot.failed(e);
                self.spawn_verify(self.job, self.current.clone(), tx);
            }
            ModelEvent::Rest => self.rest(),
            ModelEvent::Progress { .. } | ModelEvent::Verified { .. } | ModelEvent::Pinned(_) => {}
        }
        Told::Nothing
    }

    fn on_switch(&mut self, event: ModelEvent, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) -> Told {
        let Some(s) = self.switch.as_mut() else { return Told::Nothing };
        let job = s.job;
        let present = |m: &Option<Manifest>, models: &Path| {
            m.as_ref().map_or((0, 0), |m| (download::present(m, &models.join(&m.id)), m.total()))
        };
        match event {
            ModelEvent::Pinned(Ok(m)) => {
                s.manifest = Some(m.clone());
                self.begin(m, tx, http);
            }
            ModelEvent::Pinned(Err(PinError::NotRunnable(reason))) => {
                tracing::info!(?reason, "speech model {}: cannot run in CUE", s.id);
                s.state = EntryState::NotRunnable;
            }
            ModelEvent::Pinned(Err(PinError::Failed(e))) => {
                tracing::warn!("speech model {}: not pinned: {e}", s.id);
                s.state = EntryState::Failed { done: 0, total: 0 };
            }
            ModelEvent::Progress { done, total } => s.state = EntryState::Downloading { done, total },
            ModelEvent::Fetched => {
                s.state = EntryState::Loading;
                self.switching = None;
                let id = s.id.clone();
                self.spawn_load(job, &id, tx);
            }
            ModelEvent::FetchFailed(_) => {
                let (done, total) = present(&s.manifest, &self.models);
                s.state = EntryState::Failed { done, total };
                self.switching = None;
            }
            ModelEvent::Loaded(Ok(model)) => {
                let Some(m) = self.switch.take().and_then(|s| s.manifest) else { return Told::Nothing };
                // The model in use goes once nothing recognizes with it; a download of it still running stops.
                self.slot.ready(model);
                if let Some(f) = self.fetching.take() {
                    f.abort();
                }
                tracing::info!(from = self.current.id, to = m.id, "speech model: switched");
                self.state = ModelState::Ready { total: m.total() };
                self.job = job;
                self.downloaded = false;
                // The new model is in the slot, loaded: whatever rested was the old one. It rests from now on.
                self.resting = false;
                if !self.busy {
                    self.arm_rest(tx);
                }
                let told = Told::Switched { id: m.id.clone(), name: m.name.clone() };
                self.current = m;
                return told;
            }
            ModelEvent::Loaded(Err(_)) => {
                if let Some(m) = s.manifest.clone() {
                    self.spawn_verify(job, m, tx);
                }
            }
            ModelEvent::Verified { removed } => {
                let (done, total) = present(&s.manifest, &self.models);
                s.state = if removed > 0 { EntryState::Failed { done, total } } else { EntryState::NotRunnable };
            }
            ModelEvent::Rest => {}
        }
        Told::Nothing
    }

    /// A switch to `m`: loaded when its files are here, else downloaded first, when the drive has room for it.
    fn begin(&mut self, m: Manifest, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        let dir = self.dir(&m.id);
        let Some(job) = self.switch.as_ref().map(|s| s.job) else { return };
        let state = if download::ready(&m, &dir) {
            self.spawn_load(job, &m.id, tx);
            EntryState::Loading
        } else {
            let done = download::present(&m, &dir);
            let needed = room_for(m.total() - done.min(m.total()));
            match (self.free)(&self.models) {
                Some(free) if free < needed => EntryState::NoSpace { needed, free },
                _ => {
                    self.switching = Some(self.spawn_fetch(job, m.clone(), tx, http));
                    EntryState::Downloading { done, total: m.total() }
                }
            }
        };
        if let Some(s) = self.switch.as_mut() {
            s.state = state;
        }
    }

    /// The Model page chose a model: it is switched to; the model in use stops any switch. A switch already on its
    /// way to the same model goes on; one that stopped starts again.
    pub fn choose(&mut self, choice: Choice, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if self.state == ModelState::Unsupported {
            return;
        }
        let (id, repo) = match choice {
            Choice::Id(id) => (id, None),
            Choice::Repo(repo) => (model::id_for_repo(&repo), Some(repo)),
        };
        if id == self.current.id {
            self.stop_switch();
            return;
        }
        let running = |s: &EntryState| matches!(s, EntryState::Preparing | EntryState::Downloading { .. } | EntryState::Loading);
        if self.switch.as_ref().is_some_and(|s| s.id == id && running(&s.state)) {
            return;
        }
        let repo = repo.or_else(|| self.switch.as_ref().filter(|s| s.id == id).map(|s| s.repo.clone()));
        self.stop_switch();
        let job = self.new_job();
        if let Some(m) = model::resolve(&self.models, &id) {
            let repo = m.repo.clone();
            self.switch = Some(Switch { id, repo, manifest: Some(m.clone()), state: EntryState::Loading, job });
            self.begin(m, tx, http);
        } else if let Some(repo) = repo {
            let (tx, http, models) = (tx.clone(), http.clone(), self.models.clone());
            let asked = repo.clone();
            tokio::spawn(async move {
                let pinned = hub::pin(&http, &asked, &models).await;
                let _ = tx.send(CoreEvent::Model(ModelNews { job, event: ModelEvent::Pinned(pinned) }));
            });
            self.switch = Some(Switch { id, repo, manifest: None, state: EntryState::Preparing, job });
        } else {
            tracing::warn!("speech model: {id} is not known");
        }
    }

    fn stop_switch(&mut self) {
        if let Some(task) = self.switching.take() {
            task.abort();
        }
        self.switch = None;
    }

    /// A model's folder deleted from the Model page: never the one in use, nor one a switch is busy with. `true` when
    /// it went.
    pub fn delete(&mut self, id: &str) -> bool {
        let busy = |s: &Switch| s.id == id && matches!(s.state, EntryState::Preparing | EntryState::Downloading { .. } | EntryState::Loading);
        if id == self.current.id || self.switch.as_ref().is_some_and(busy) || model::resolve(&self.models, id).is_none() {
            return false;
        }
        if self.switch.as_ref().is_some_and(|s| s.id == id) {
            self.switch = None;
        }
        let dir = self.dir(id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                tracing::info!("speech model {id}: deleted");
                true
            }
            Err(e) => {
                tracing::warn!("speech model {id}: not deleted: {e}");
                false
            }
        }
    }

    /// The Model page's list: the short list, then found models with a folder here, then a found one being pinned.
    pub fn entries(&self) -> Vec<ModelEntry> {
        let mut list = model::catalog();
        for m in model::found_models(&self.models) {
            if !list.iter().any(|l| l.id == m.id) {
                list.push(m);
            }
        }
        if !list.iter().any(|l| l.id == self.current.id) {
            list.push(self.current.clone());
        }
        let mut out: Vec<ModelEntry> = list.iter().map(|m| ModelEntry::new(m, self.entry_state(m))).collect();
        if let Some(s) = &self.switch {
            match out.iter_mut().find(|e| e.id == s.id) {
                Some(e) => *e = s.entry(),
                None => out.push(s.entry()),
            }
        }
        out
    }

    /// The list with the model in use, for the windows.
    pub fn view(&self) -> ModelsView {
        ModelsView { current: self.current.id.clone(), entries: self.entries(), recommended: None, found: Vec::new() }
    }

    fn entry_state(&self, m: &Manifest) -> EntryState {
        if m.id != self.current.id {
            return if download::ready(m, &self.dir(&m.id)) { EntryState::Downloaded } else { EntryState::Missing };
        }
        match self.state {
            ModelState::Ready { .. } => EntryState::InUse,
            ModelState::Missing => EntryState::Missing,
            ModelState::Downloading { done, total } => EntryState::Downloading { done, total },
            ModelState::Loading => EntryState::Loading,
            ModelState::Failed { done, total } => EntryState::Failed { done, total },
            ModelState::Unsupported => EntryState::NotRunnable,
        }
    }

    /// How long recognition may take now: longer while the model is still on its way.
    pub fn timeout(&self) -> Duration {
        if self.slot.is_ready() { RECOGNIZE_TIMEOUT } else { WAIT_FOR_MODEL + RECOGNIZE_TIMEOUT }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::model::{ModelFile, id_for_repo, write_manifest};
    use tokio::sync::mpsc::UnboundedReceiver;

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
        assert_eq!(state_after(still, &ModelEvent::Verified { removed: 1 }, 70, total), ModelState::Failed { done: 70, total });
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

    /// A model that answers with its folder's name: which one is in the slot.
    struct Named(String);
    impl Recognize for Named {
        fn recognize(&self, _: &[i16]) -> Result<SttOutput, EngineError> {
            Ok(SttOutput { text: self.0.clone(), audio_ms: 0, language: None })
        }
    }

    /// Loads a model as `Named`, or fails for a folder whose name has "broken" in it.
    fn fake_load(dir: &Path) -> Result<Arc<dyn Recognize>, EngineError> {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if name.contains("broken") {
            return Err(EngineError::Model("no".into()));
        }
        Ok(Arc::new(Named(name)))
    }

    const FILE: &[u8] = b"x";
    /// SHA-256 of `FILE`.
    const FILE_SHA: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";

    /// A found model `owner/parakeet-tdt` with a one-byte file, downloaded when `here`.
    fn fake(models: &Path, owner: &str, here: bool) -> Manifest {
        let repo = format!("{owner}/parakeet-tdt");
        let m = Manifest {
            id: id_for_repo(&repo),
            name: repo.clone(),
            repo,
            commit: "c".repeat(40),
            files: vec![ModelFile { path: "w.bin".into(), name: "w.bin".into(), size: 1, sha256: FILE_SHA.into() }],
            languages: vec!["en".into()],
            license: String::new(),
        };
        let dir = models.join(&m.id);
        write_manifest(&dir, &m).unwrap();
        if here {
            std::fs::write(dir.join("w.bin"), FILE).unwrap();
        }
        m
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-keeper-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    struct Bench {
        keeper: ModelKeeper,
        tx: UnboundedSender<CoreEvent>,
        rx: UnboundedReceiver<CoreEvent>,
        http: reqwest::Client,
    }

    impl Bench {
        /// A keeper whose model in use is `chosen`, started and loaded; `free` — the drive's room.
        async fn new(models: &Path, chosen: &str, free: FreeSpace) -> Self {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let keeper = ModelKeeper::with(models.to_owned(), chosen, fake_load, free);
            let mut b = Self { keeper, tx, rx, http: reqwest::Client::new() };
            b.keeper.start(&b.tx, &b.http);
            b.settle().await;
            b
        }

        /// Every background step until none comes for a while, each given to the keeper.
        async fn settle(&mut self) -> Vec<Told> {
            let mut told = Vec::new();
            while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(300), self.rx.recv()).await {
                if let CoreEvent::Model(news) = ev {
                    told.push(self.keeper.on_event(news, &self.tx, &self.http));
                }
            }
            told
        }

        fn choose(&mut self, choice: Choice) {
            self.keeper.choose(choice, &self.tx, &self.http);
        }

        /// The model recognition gets now.
        fn in_slot(&self) -> String {
            self.keeper.slot().recognize(&[]).unwrap().text
        }

        fn state_of(&self, id: &str) -> EntryState {
            self.keeper.entries().into_iter().find(|e| e.id == id).unwrap().state
        }
    }

    fn roomy(_: &Path) -> Option<u64> {
        Some(u64::MAX)
    }

    fn cramped(_: &Path) -> Option<u64> {
        Some(0)
    }

    #[tokio::test]
    async fn a_model_at_rest_is_let_go_and_loads_again_for_the_next_dictation() {
        let models = folder("rest");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        b.keeper.rest_after = Duration::from_millis(50);
        b.keeper.busy(false, &b.tx);
        b.settle().await;
        assert!(!b.keeper.slot().is_ready(), "let go after the wait");
        assert_eq!(b.keeper.state(), ModelState::Ready { total: 1 }, "the windows still show it ready");
        assert_eq!(b.state_of(&m.id), EntryState::InUse);
        b.keeper.busy(true, &b.tx);
        b.settle().await;
        assert_eq!(b.in_slot(), m.id, "a dictation loads it again");
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_model_is_never_let_go_during_a_dictation() {
        let models = folder("busy");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        b.keeper.rest_after = Duration::from_millis(50);
        b.keeper.busy(false, &b.tx);
        b.keeper.busy(true, &b.tx);
        b.settle().await;
        assert!(b.keeper.slot().is_ready(), "a dictation stops the wait");
        b.keeper.rest_soon();
        assert!(b.keeper.slot().is_ready(), "a lock during a dictation waits for its end");
        b.keeper.busy(false, &b.tx);
        assert!(!b.keeper.slot().is_ready(), "and lets the model go as it ends");
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_lock_at_rest_lets_the_model_go_at_once() {
        let models = folder("lock");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        b.keeper.rest_soon();
        assert!(!b.keeper.slot().is_ready());
        b.keeper.busy(true, &b.tx);
        b.settle().await;
        assert_eq!(b.in_slot(), m.id);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn an_unknown_choice_at_start_is_ultra() {
        let models = folder("unknown");
        let k = ModelKeeper::with(models.clone(), "hf--gone--model", fake_load, roomy);
        assert_eq!(k.current, model::ultra());
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn the_model_in_use_works_until_the_new_one_has_loaded() {
        let models = folder("switch");
        let old = fake(&models, "old", true);
        let new = fake(&models, "new", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        assert_eq!((b.in_slot(), b.state_of(&old.id)), (old.id.clone(), EntryState::InUse));
        assert_eq!(b.state_of(&new.id), EntryState::Downloaded);
        b.choose(Choice::Id(new.id.clone()));
        assert_eq!(b.state_of(&new.id), EntryState::Loading);
        assert_eq!(b.in_slot(), old.id, "the old model recognizes while the new one loads");
        let told = b.settle().await;
        assert_eq!(told, [Told::Switched { id: new.id.clone(), name: new.name.clone() }]);
        assert_eq!(b.in_slot(), new.id);
        assert_eq!((b.state_of(&new.id), b.state_of(&old.id)), (EntryState::InUse, EntryState::Downloaded));
        assert_eq!(b.keeper.state(), ModelState::Ready { total: 1 });
        assert_eq!(b.keeper.view().current, new.id);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_switch_stopped_by_another_choice_ignores_its_late_steps() {
        let models = folder("stale");
        let old = fake(&models, "old", true);
        let first = fake(&models, "first", true);
        let second = fake(&models, "second", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        b.choose(Choice::Id(first.id.clone()));
        b.choose(Choice::Id(second.id.clone()));
        let told = b.settle().await;
        assert!(told.contains(&Told::Switched { id: second.id.clone(), name: second.name.clone() }), "{told:?}");
        assert_eq!(b.in_slot(), second.id, "the first one's load came and went unheard");
        assert_eq!(b.state_of(&first.id), EntryState::Downloaded);
        // Choosing the model in use stops a switch.
        b.choose(Choice::Id(old.id.clone()));
        b.choose(Choice::Id(second.id.clone()));
        assert!(b.settle().await.iter().all(|t| *t == Told::Nothing));
        assert_eq!(b.in_slot(), second.id);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_model_that_does_not_load_leaves_the_old_one_in_use() {
        let models = folder("broken");
        let old = fake(&models, "old", true);
        let broken = fake(&models, "broken", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        b.choose(Choice::Id(broken.id.clone()));
        assert!(b.settle().await.iter().all(|t| *t == Told::Nothing));
        assert_eq!(b.state_of(&broken.id), EntryState::NotRunnable, "its files are intact: it cannot run");
        assert_eq!((b.in_slot(), b.state_of(&old.id)), (old.id.clone(), EntryState::InUse));
        // A damaged file instead: it goes, and Retry downloads it again.
        std::fs::write(models.join(&broken.id).join("w.bin"), b"y").unwrap();
        b.choose(Choice::Id(broken.id.clone()));
        b.settle().await;
        assert_eq!(b.state_of(&broken.id), EntryState::Failed { done: 0, total: 1 });
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_download_needs_room_on_the_drive() {
        let models = folder("room");
        let old = fake(&models, "old", true);
        let away = fake(&models, "away", false);
        let mut b = Bench::new(&models, &old.id, cramped).await;
        b.choose(Choice::Id(away.id.clone()));
        assert_eq!(b.state_of(&away.id), EntryState::NoSpace { needed: 1, free: 0 });
        assert_eq!(b.in_slot(), old.id);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn the_model_in_use_cannot_be_deleted() {
        let models = folder("delete");
        let old = fake(&models, "old", true);
        let other = fake(&models, "other", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        assert!(!b.keeper.delete(&old.id));
        assert!(models.join(&old.id).exists());
        assert!(!b.keeper.delete("hf--nobody--here"), "a folder CUE does not know");
        assert!(b.keeper.delete(&other.id));
        assert!(!models.join(&other.id).exists());
        assert!(b.keeper.entries().iter().all(|e| e.id != other.id));
        // Ultra: listed whether or not it is here.
        assert_eq!(b.state_of(model::DEFAULT_ID), EntryState::Missing);
        std::fs::remove_dir_all(models).unwrap();
    }
}
