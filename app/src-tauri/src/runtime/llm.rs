//! The cleanup's language model on this computer (docs/specs/2026-11-04-local-cleanup.md): the model in use in the
//! slot the cleanup takes it from, and a switch to another, pinned, downloaded and loaded beside it, which takes the
//! slot only once it has loaded. Nothing downloads by itself: a model comes when the Cleanup page chooses it, and the
//! chosen one loads at start when its file is here. The cleanup never waits for it: an empty slot pastes the raw text.
//! A model is in memory only while the cleanup runs on this computer: on the cloud none is loaded, and the one in use
//! is let go; a download goes on and its model becomes the chosen one, loaded once the cleanup comes back here.
//!
//! At rest the model in use is let go too, as the speech model is (`model::REST_AFTER` without a dictation, a lock,
//! sleep): it stays in use for the windows, and the next dictation loads it again as it starts; that dictation's
//! cleanup waits for the load ([`LOAD_WAIT`] at most) instead of pasting the text as said.
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use cue_core::llm;
use cue_core::model::{EntryState, Manifest, ModelEntry, ModelsView};
use cue_engines::EngineError;
use cue_engines::download;
use cue_engines::hub::{self, LlmPinError};
use cue_engines::local_llm::{Generate, LocalLlm};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::AbortHandle;

use super::CoreEvent;
use super::model::{Choice, FreeSpace, REST_AFTER, Told, free_space, room_for};

/// The longest a cleanup waits for its model loading again after a rest; past it the text is pasted as said.
pub const LOAD_WAIT: Duration = Duration::from_secs(15);

/// What happened to a language model in the background.
#[derive(Debug)]
pub enum LlmEvent {
    Progress { done: u64, total: u64 },
    Fetched,
    FetchFailed(EngineError),
    Loaded(Result<Arc<dyn Generate>, EngineError>),
    Verified { removed: usize },
    Pinned(Result<Manifest, LlmPinError>),
    /// A model's folder deleted after its file was let go, or not (`ok`).
    Deleted { id: String, ok: bool },
    /// The model in use has rested long enough without a dictation: it is let go.
    Rest,
}

/// A step of the job `job`: events of a switch that was stopped are left alone.
#[derive(Debug)]
pub struct LlmNews {
    pub job: u64,
    pub event: LlmEvent,
}

/// The model the cleanup takes, if one is ready. Never waits.
#[derive(Default)]
pub struct LlmSlot(Mutex<Option<Arc<dyn Generate>>>);

impl LlmSlot {
    pub fn ready(&self, llm: Arc<dyn Generate>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(llm);
    }

    /// The model is let go: its memory is freed once a cleanup still running with it ends.
    pub fn clear(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    pub fn get(&self) -> Option<Arc<dyn Generate>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The model once it is here, waiting at most `longest` for one loading again after a rest.
    pub async fn wait(&self, longest: Duration) -> Option<Arc<dyn Generate>> {
        let deadline = Instant::now() + longest;
        loop {
            if let Some(g) = self.get() {
                return Some(g);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// How often, and how many times, a model's folder still held is tried again: a minute in all, the longest a warm-up
/// runs.
const DELETE_EVERY: std::time::Duration = std::time::Duration::from_millis(250);
const DELETE_TRIES: u32 = 240;

/// Loads a model's GGUF file; `id` is its folder's name.
type Loader = fn(&Path, &str) -> Result<Arc<dyn Generate>, EngineError>;

/// A GGUF file loaded by llama.cpp (`cue_engines::local_llm`).
fn load_llm(file: &Path, id: &str) -> Result<Arc<dyn Generate>, EngineError> {
    LocalLlm::load(file, id).map(|l| l as Arc<dyn Generate>)
}

/// A model being switched to: its row while it is pinned, downloaded and loaded.
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
            None => ModelEntry { id: self.id.clone(), name: self.repo.clone(), repo: self.repo.clone(), languages: Vec::new(), size: 0, state: self.state },
        }
    }
}

/// The language model in use, a switch to another, and the background work that moves them.
pub struct LlmKeeper {
    /// `%LOCALAPPDATA%\CUE\models`: a folder per model, named by its id.
    models: PathBuf,
    /// `refine.local`, or the recommended one: what loads at start when its file is here.
    chosen: String,
    in_use: Option<Manifest>,
    slot: Arc<LlmSlot>,
    switch: Option<Switch>,
    /// The switch's download, stopped by another choice.
    switching: Option<AbortHandle>,
    jobs: u64,
    /// Models being deleted while their file is still held (a cleanup or a warm-up finishing with it): not listed.
    deleting: Vec<String>,
    /// The cleanup runs on this computer: only then is a model loaded.
    active: bool,
    load: Loader,
    free: FreeSpace,
    /// The model in use was let go at rest: the next dictation loads it again.
    resting: bool,
    /// The job of the model in use loading again after a rest.
    waking: Option<u64>,
    /// A dictation records or is worked on: the model is not let go meanwhile.
    busy: bool,
    /// The session locked or the computer went to sleep during a dictation: the model is let go once it ends.
    rest_when_done: bool,
    /// The wait before the model is let go, started when a dictation ends.
    rest_timer: Option<AbortHandle>,
    rest_after: Duration,
}

impl LlmKeeper {
    /// The model `chosen` (`refine.local`; empty — the recommended one) in `models`; `active` — the cleanup runs on
    /// this computer.
    pub fn new(models: PathBuf, chosen: &str, active: bool) -> Self {
        Self::with(models, chosen, active, load_llm, free_space)
    }

    fn with(models: PathBuf, chosen: &str, active: bool, load: Loader, free: FreeSpace) -> Self {
        let chosen = if chosen.is_empty() { llm::default_id().unwrap_or_default() } else { chosen.to_owned() };
        Self {
            models,
            chosen,
            in_use: None,
            slot: Arc::new(LlmSlot::default()),
            switch: None,
            switching: None,
            jobs: 0,
            deleting: Vec::new(),
            active,
            load,
            free,
            resting: false,
            waking: None,
            busy: false,
            rest_when_done: false,
            rest_timer: None,
            rest_after: REST_AFTER,
        }
    }

    /// A dictation started (`true`) or ended (`false`). One starting loads a model let go at rest; one ending starts
    /// the wait after which the model is let go, or lets it go now after a lock or sleep came during it.
    pub fn busy(&mut self, busy: bool, tx: &UnboundedSender<CoreEvent>) {
        self.busy = busy;
        if busy {
            self.stop_rest_timer();
            self.wake(tx);
        } else if std::mem::take(&mut self.rest_when_done) {
            self.stop_rest_timer();
            self.rest();
        } else {
            self.arm_rest(tx);
        }
    }

    /// The session locked or the computer goes to sleep: the model is let go now, or once the dictation ends.
    pub fn rest_soon(&mut self) {
        if self.busy {
            self.rest_when_done = true;
        } else {
            self.rest();
        }
    }

    /// The model in use is loading again after a rest: a cleanup waits for it.
    pub fn waking(&self) -> bool {
        self.waking.is_some()
    }

    fn stop_rest_timer(&mut self) {
        if let Some(timer) = self.rest_timer.take() {
            timer.abort();
        }
    }

    /// The wait before the model in use is let go starts again.
    fn arm_rest(&mut self, tx: &UnboundedSender<CoreEvent>) {
        self.stop_rest_timer();
        let (tx, after) = (tx.clone(), self.rest_after);
        self.rest_timer = Some(
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                let _ = tx.send(CoreEvent::Llm(LlmNews { job: 0, event: LlmEvent::Rest }));
            })
            .abort_handle(),
        );
    }

    /// The model in use, loaded and not cleaning up, is let go: its memory is freed once a cleanup or a warm-up still
    /// holding it ends. The windows still show it in use.
    fn rest(&mut self) {
        if self.busy || self.resting || !self.active || self.in_use.is_none() || self.slot.get().is_none() {
            return;
        }
        tracing::info!(model = self.in_use().unwrap_or(""), "language model: let go at rest");
        self.resting = true;
        self.slot.clear();
    }

    /// A model let go at rest loads again.
    fn wake(&mut self, tx: &UnboundedSender<CoreEvent>) {
        if !std::mem::take(&mut self.resting) || !self.active {
            return;
        }
        let Some(m) = self.in_use.clone() else { return };
        tracing::info!(model = m.id, "language model: loads again for a dictation");
        let job = self.new_job();
        self.waking = Some(job);
        self.spawn_load(job, &m, tx);
    }

    /// The model in use loaded again after a rest, or not: in the slot, or let go when the cleanup left this computer or
    /// another model took over meanwhile. One that did not load is tried again by the next dictation.
    fn on_woken(&mut self, loaded: Result<Arc<dyn Generate>, EngineError>, tx: &UnboundedSender<CoreEvent>) {
        self.waking = None;
        match loaded {
            Ok(model) if self.active && self.in_use() == Some(model.id()) && self.slot.get().is_none() => {
                self.slot.ready(model);
                if !self.busy {
                    self.arm_rest(tx);
                }
            }
            Ok(_) => {}
            Err(_) => self.resting = self.active && self.in_use.is_some(),
        }
    }

    pub fn slot(&self) -> Arc<LlmSlot> {
        self.slot.clone()
    }

    /// The model in use's id, if one is.
    pub fn in_use(&self) -> Option<&str> {
        self.in_use.as_ref().map(|m| m.id.as_str())
    }

    fn dir(&self, id: &str) -> PathBuf {
        self.models.join(id)
    }

    fn new_job(&mut self) -> u64 {
        self.jobs += 1;
        self.jobs
    }

    /// At start, and when the cleanup moves to this computer: the chosen model loads when its file is here.
    pub fn start(&mut self, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        if !self.active || self.in_use.is_some() || self.switch.is_some() {
            return;
        }
        if let Some(m) = llm::resolve(&self.models, &self.chosen)
            && download::ready(&m, &self.dir(&m.id))
        {
            self.choose(Choice::Id(m.id), tx, http);
        }
    }

    fn spawn_fetch(&self, job: u64, m: Manifest, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) -> AbortHandle {
        let (tx, http, dir) = (tx.clone(), http.clone(), self.dir(&m.id));
        tokio::spawn(async move {
            let started = Instant::now();
            let shown = AtomicU64::new(u64::MAX);
            let progress = |done: u64, total: u64| {
                let percent = done * 100 / total.max(1);
                if shown.swap(percent, Ordering::Relaxed) != percent {
                    let _ = tx.send(CoreEvent::Llm(LlmNews { job, event: LlmEvent::Progress { done, total } }));
                }
            };
            let event = match download::fetch(&http, &m, &dir, progress).await {
                Ok(()) => {
                    tracing::info!(seconds = started.elapsed().as_secs(), model = m.id, "language model: downloaded");
                    LlmEvent::Fetched
                }
                Err(e) => {
                    tracing::warn!("language model {}: download stopped: {e}", m.id);
                    LlmEvent::FetchFailed(e)
                }
            };
            let _ = tx.send(CoreEvent::Llm(LlmNews { job, event }));
        })
        .abort_handle()
    }

    fn spawn_load(&self, job: u64, m: &Manifest, tx: &UnboundedSender<CoreEvent>) {
        let file = self.dir(&m.id).join(&m.files[0].name);
        let (tx, load, id) = (tx.clone(), self.load, m.id.clone());
        tokio::task::spawn_blocking(move || {
            let started = Instant::now();
            let loaded = load(&file, &id);
            match &loaded {
                Ok(_) => tracing::info!(ms = started.elapsed().as_millis() as u64, model = id, "language model: loaded"),
                Err(e) => tracing::warn!("language model {id}: not loaded: {e}"),
            }
            let _ = tx.send(CoreEvent::Llm(LlmNews { job, event: LlmEvent::Loaded(loaded) }));
        });
    }

    fn spawn_verify(&self, job: u64, m: Manifest, tx: &UnboundedSender<CoreEvent>) {
        let (tx, dir) = (tx.clone(), self.dir(&m.id));
        tokio::task::spawn_blocking(move || {
            let removed = download::verify(&m, &dir);
            tracing::info!(removed, model = m.id, "language model: files checked again");
            let _ = tx.send(CoreEvent::Llm(LlmNews { job, event: LlmEvent::Verified { removed } }));
        });
    }

    /// A background step came: the switch moves on, and the next step starts.
    pub fn on_event(&mut self, news: LlmNews, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) -> Told {
        if let LlmEvent::Deleted { id, ok } = &news.event {
            self.deleting.retain(|d| d != id);
            if *ok {
                tracing::info!("language model {id}: deleted");
            } else {
                tracing::warn!("language model {id}: not deleted, its file stays in use");
            }
            return Told::Nothing;
        }
        if let LlmEvent::Rest = news.event {
            self.rest();
            return Told::Nothing;
        }
        if self.waking == Some(news.job) {
            if let LlmEvent::Loaded(loaded) = news.event {
                self.on_woken(loaded, tx);
            }
            return Told::Nothing;
        }
        let Some(s) = self.switch.as_mut().filter(|s| s.job == news.job) else { return Told::Nothing };
        let job = s.job;
        let present = |m: &Option<Manifest>, models: &Path| m.as_ref().map_or((0, 0), |m| (download::present(m, &models.join(&m.id)), m.total()));
        match news.event {
            LlmEvent::Pinned(Ok(m)) => {
                s.manifest = Some(m.clone());
                self.begin(m, tx, http);
            }
            LlmEvent::Pinned(Err(LlmPinError::NotRunnable(reason))) => {
                tracing::info!(?reason, "language model {}: cannot run in CUE", s.id);
                s.state = EntryState::NotRunnable;
            }
            LlmEvent::Pinned(Err(LlmPinError::Failed(e))) => {
                tracing::warn!("language model {}: not pinned: {e}", s.id);
                s.state = EntryState::Failed { done: 0, total: 0 };
            }
            LlmEvent::Progress { done, total } => s.state = EntryState::Downloading { done, total },
            LlmEvent::Fetched => {
                self.switching = None;
                if !self.active {
                    return self.chosen_not_loaded();
                }
                s.state = EntryState::Loading;
                if let Some(m) = s.manifest.clone() {
                    self.spawn_load(job, &m, tx);
                }
            }
            LlmEvent::FetchFailed(e) => {
                tracing::info!(timeout = e == EngineError::Timeout, "language model {}: Retry continues the download", s.id);
                let (done, total) = present(&s.manifest, &self.models);
                s.state = EntryState::Failed { done, total };
                self.switching = None;
            }
            LlmEvent::Loaded(Ok(model)) => {
                if !self.active {
                    // Loaded as the cleanup left this computer: let go at once.
                    drop(model);
                    return self.chosen_not_loaded();
                }
                let Some(m) = self.switch.take().and_then(|s| s.manifest) else { return Told::Nothing };
                self.slot.ready(model);
                // The new model is in the slot: whatever rested or was loading again was the old one. It rests from now on.
                self.resting = false;
                self.waking = None;
                if !self.busy {
                    self.arm_rest(tx);
                }
                tracing::info!(from = self.in_use().unwrap_or("none"), to = m.id, "language model: switched");
                let told = Told::Switched { id: m.id.clone(), name: m.name.clone() };
                self.chosen = m.id.clone();
                self.in_use = Some(m);
                return told;
            }
            LlmEvent::Loaded(Err(_)) => {
                if let Some(m) = s.manifest.clone() {
                    self.spawn_verify(job, m, tx);
                }
            }
            LlmEvent::Verified { removed } => {
                let (done, total) = present(&s.manifest, &self.models);
                s.state = if removed > 0 { EntryState::Failed { done, total } } else { EntryState::NotRunnable };
            }
            // Taken above, before a switch's job is looked at.
            LlmEvent::Deleted { .. } | LlmEvent::Rest => {}
        }
        Told::Nothing
    }

    /// The switch's model is here but the cleanup runs in the cloud: it becomes the chosen one (`refine.local` is
    /// written), loaded once the cleanup comes back to this computer.
    fn chosen_not_loaded(&mut self) -> Told {
        let Some(m) = self.switch.take().and_then(|s| s.manifest) else { return Told::Nothing };
        tracing::info!(model = m.id, "language model: chosen, loads when the cleanup runs on this computer");
        self.chosen = m.id.clone();
        Told::Switched { id: m.id, name: m.name }
    }

    /// Where the cleanup runs changed. To this computer: the chosen model loads when its file is here. To the cloud:
    /// the model in use is let go, so its memory is freed; a download goes on.
    pub fn set_active(&mut self, active: bool, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        self.active = active;
        if active {
            self.start(tx, http);
            return;
        }
        if let Some(m) = self.in_use.take() {
            tracing::info!(model = m.id, "language model: let go, the cleanup runs in the cloud");
            self.chosen = m.id;
        }
        self.slot.clear();
        self.resting = false;
        self.waking = None;
        self.stop_rest_timer();
    }

    /// A switch to `m`: loaded when its file is here, else downloaded first, when the drive has room for it.
    fn begin(&mut self, m: Manifest, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        let dir = self.dir(&m.id);
        let Some(job) = self.switch.as_ref().map(|s| s.job) else { return };
        let state = if download::ready(&m, &dir) {
            // On the cloud a downloaded model is not loaded: the Cleanup page chooses only on this computer.
            if !self.active {
                self.switch = None;
                return;
            }
            self.spawn_load(job, &m, tx);
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

    /// The Cleanup page chose a model (from the list, or a repository the search found): it is switched to; the model
    /// in use stops any switch. A switch already on its way to the same model goes on; one that stopped starts again.
    pub fn choose(&mut self, choice: Choice, tx: &UnboundedSender<CoreEvent>, http: &reqwest::Client) {
        let (id, repo) = match choice {
            Choice::Id(id) => (id, None),
            // The repository's id is known once it is pinned (its quantization): until then the switch is named by it.
            Choice::Repo(repo) => (format!("repo:{}", repo.to_lowercase()), Some(repo)),
        };
        if self.in_use() == Some(id.as_str()) {
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
        if let Some(m) = llm::resolve(&self.models, &id) {
            let repo = m.repo.clone();
            self.switch = Some(Switch { id, repo, manifest: Some(m.clone()), state: EntryState::Loading, job });
            self.begin(m, tx, http);
        } else if let Some(repo) = repo {
            let (tx, http, models) = (tx.clone(), http.clone(), self.models.clone());
            let asked = repo.clone();
            tokio::spawn(async move {
                let pinned = hub::pin_llm(&http, &asked, &models).await;
                let _ = tx.send(CoreEvent::Llm(LlmNews { job, event: LlmEvent::Pinned(pinned) }));
            });
            self.switch = Some(Switch { id, repo, manifest: None, state: EntryState::Preparing, job });
        } else if !id.is_empty() {
            tracing::warn!("language model: {id} is not known");
        }
    }

    fn stop_switch(&mut self) {
        if let Some(task) = self.switching.take() {
            task.abort();
        }
        self.switch = None;
    }

    /// A model's folder deleted from the Cleanup page, the one in use too: it is let go first, and the cleanup pastes
    /// the text as said until another is chosen. Never one a switch is busy with. A file still held by a cleanup or a
    /// warm-up finishing with it (Windows keeps a mapped file) is deleted once they let it go, within a minute; until
    /// then the model is not listed as downloaded.
    pub fn delete(&mut self, id: &str, tx: &UnboundedSender<CoreEvent>) -> bool {
        let busy = |s: &Switch| s.id == id && matches!(s.state, EntryState::Preparing | EntryState::Downloading { .. } | EntryState::Loading);
        if self.switch.as_ref().is_some_and(busy) || llm::resolve(&self.models, id).is_none() {
            return false;
        }
        if self.switch.as_ref().is_some_and(|s| s.id == id) {
            self.switch = None;
        }
        if self.in_use() == Some(id) {
            tracing::info!("language model {id}: let go to be deleted");
            self.in_use = None;
            self.slot.clear();
            self.resting = false;
            self.waking = None;
        }
        let dir = self.dir(id);
        if std::fs::remove_dir_all(&dir).is_ok() {
            tracing::info!("language model {id}: deleted");
            return true;
        }
        self.deleting.push(id.to_owned());
        let (tx, id) = (tx.clone(), id.to_owned());
        tokio::task::spawn_blocking(move || {
            let mut ok = false;
            for _ in 0..DELETE_TRIES {
                std::thread::sleep(DELETE_EVERY);
                if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                    ok = true;
                    break;
                }
            }
            let _ = tx.send(CoreEvent::Llm(LlmNews { job: 0, event: LlmEvent::Deleted { id, ok } }));
        });
        true
    }

    /// The Cleanup page's list: the short list, then found models with a folder here, then a switch being pinned.
    pub fn view(&self) -> ModelsView {
        let mut list = llm::catalog();
        for m in llm::found(&self.models) {
            if !list.iter().any(|l| l.id == m.id) {
                list.push(m);
            }
        }
        if let Some(m) = &self.in_use
            && !list.iter().any(|l| l.id == m.id)
        {
            list.push(m.clone());
        }
        let mut entries: Vec<ModelEntry> = list
            .iter()
            .map(|m| {
                let state = if self.in_use() == Some(m.id.as_str()) {
                    EntryState::InUse
                } else if self.deleting.contains(&m.id) {
                    EntryState::Missing
                } else if download::ready(m, &self.dir(&m.id)) {
                    EntryState::Downloaded
                } else {
                    EntryState::Missing
                };
                ModelEntry::new(m, state)
            })
            .collect();
        if let Some(s) = &self.switch {
            let e = s.entry();
            match entries.iter_mut().find(|x| x.id == e.id || (s.manifest.is_none() && x.repo.eq_ignore_ascii_case(&s.repo))) {
                Some(x) => *x = e,
                None => entries.push(e),
            }
        }
        ModelsView { current: self.in_use().unwrap_or_default().to_owned(), entries, recommended: llm::default_id() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::llm::id_for;
    use cue_core::model::{ModelFile, write_manifest};
    use std::time::Duration;
    use tokio::sync::mpsc::UnboundedReceiver;

    /// A model that answers with its folder's name: which one is in the slot.
    struct Named(String);
    impl Generate for Named {
        fn generate(&self, _: &str, _: &str, _: &str, _: Instant, _: &std::sync::atomic::AtomicBool) -> Result<String, EngineError> {
            Ok(self.0.clone())
        }
        fn id(&self) -> &str {
            &self.0
        }
    }

    /// Loads a model as `Named`, or fails for one whose id has "broken" in it.
    fn fake_load(_: &Path, id: &str) -> Result<Arc<dyn Generate>, EngineError> {
        if id.contains("broken") {
            return Err(EngineError::Llm("does not load".into()));
        }
        Ok(Arc::new(Named(id.to_owned())))
    }

    const FILE: &[u8] = b"x";
    /// SHA-256 of `FILE`.
    const FILE_SHA: &str = "2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881";

    /// A found language model `owner/model-GGUF` with a one-byte file, downloaded when `here`.
    fn fake(models: &Path, owner: &str, here: bool) -> Manifest {
        let repo = format!("{owner}/model-GGUF");
        let m = Manifest {
            id: id_for(&repo, "Q4_K_M"),
            name: repo.clone(),
            repo,
            commit: "c".repeat(40),
            files: vec![ModelFile { path: "m-Q4_K_M.gguf".into(), name: "m-Q4_K_M.gguf".into(), size: 1, sha256: FILE_SHA.into() }],
            languages: vec![],
            license: String::new(),
        };
        let dir = models.join(&m.id);
        write_manifest(&dir, &m).unwrap();
        if here {
            std::fs::write(dir.join("m-Q4_K_M.gguf"), FILE).unwrap();
        }
        m
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-llm-keeper-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn roomy(_: &Path) -> Option<u64> {
        Some(u64::MAX)
    }
    fn cramped(_: &Path) -> Option<u64> {
        Some(0)
    }

    struct Bench {
        keeper: LlmKeeper,
        tx: UnboundedSender<CoreEvent>,
        rx: UnboundedReceiver<CoreEvent>,
        http: reqwest::Client,
    }

    impl Bench {
        async fn new(models: &Path, chosen: &str, free: FreeSpace) -> Self {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let keeper = LlmKeeper::with(models.to_owned(), chosen, true, fake_load, free);
            let mut b = Self { keeper, tx, rx, http: reqwest::Client::new() };
            b.keeper.start(&b.tx, &b.http);
            b.settle().await;
            b
        }
        async fn settle(&mut self) -> Vec<Told> {
            let mut told = Vec::new();
            while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(300), self.rx.recv()).await {
                if let CoreEvent::Llm(news) = ev {
                    told.push(self.keeper.on_event(news, &self.tx, &self.http));
                }
            }
            told
        }
        fn choose(&mut self, choice: Choice) {
            self.keeper.choose(choice, &self.tx, &self.http);
        }
        /// The model the cleanup gets now.
        fn in_slot(&self) -> Option<String> {
            self.keeper.slot().get().map(|g| g.id().to_owned())
        }
        fn state_of(&self, id: &str) -> EntryState {
            self.keeper.view().entries.into_iter().find(|e| e.id == id).unwrap().state
        }
    }

    #[tokio::test]
    async fn a_model_at_rest_is_let_go_and_the_next_cleanup_waits_for_it_to_load_again() {
        let models = folder("rest");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        b.keeper.rest_after = Duration::from_millis(50);
        b.keeper.busy(false, &b.tx);
        b.settle().await;
        assert_eq!(b.in_slot(), None, "let go after the wait");
        assert_eq!(b.keeper.view().current, m.id, "still the model in use for the windows");
        assert!(b.keeper.slot().wait(Duration::from_millis(20)).await.is_none(), "nothing loads by itself");
        b.keeper.busy(true, &b.tx);
        assert!(b.keeper.waking());
        let slot = b.keeper.slot();
        let cleanup = tokio::spawn(async move { slot.wait(Duration::from_secs(5)).await.map(|g| g.id().to_owned()) });
        b.settle().await;
        assert_eq!(cleanup.await.unwrap(), Some(m.id.clone()), "the cleanup got the model loaded again");
        assert!(!b.keeper.waking());
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_local_model_is_never_let_go_during_a_dictation_and_a_lock_waits_for_its_end() {
        let models = folder("busy");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        b.keeper.rest_after = Duration::from_millis(50);
        b.keeper.busy(false, &b.tx);
        b.keeper.busy(true, &b.tx);
        b.settle().await;
        assert_eq!(b.in_slot(), Some(m.id.clone()), "a dictation stops the wait");
        b.keeper.rest_soon();
        assert_eq!(b.in_slot(), Some(m.id.clone()), "a lock during a dictation waits for its end");
        b.keeper.busy(false, &b.tx);
        assert_eq!(b.in_slot(), None, "and lets the model go as it ends");
        b.keeper.set_active(false, &b.tx, &b.http);
        b.keeper.busy(true, &b.tx);
        assert!(!b.keeper.waking(), "on the cloud nothing loads again");
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn nothing_chosen_or_downloaded_waits_and_downloads_nothing() {
        let models = folder("nothing");
        let m = fake(&models, "here", false);
        let b = Bench::new(&models, &m.id, roomy).await;
        assert_eq!(b.in_slot(), None);
        assert_eq!(b.keeper.view().current, "", "nothing downloads by itself");
        assert_eq!(b.state_of(&m.id), EntryState::Missing);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_downloaded_choice_loads_at_start() {
        let models = folder("start");
        let m = fake(&models, "here", true);
        let b = Bench::new(&models, &m.id, roomy).await;
        assert_eq!(b.in_slot(), Some(m.id.clone()));
        assert_eq!(b.state_of(&m.id), EntryState::InUse);
        assert_eq!(b.keeper.view().current, m.id);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_switch_keeps_the_model_in_use_until_the_new_one_loads() {
        let models = folder("switch");
        let old = fake(&models, "old", true);
        let new = fake(&models, "new", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        b.choose(Choice::Id(new.id.clone()));
        assert_eq!(b.state_of(&new.id), EntryState::Loading);
        assert_eq!(b.in_slot(), Some(old.id.clone()), "the old model cleans up while the new one loads");
        let told = b.settle().await;
        assert_eq!(told, [Told::Switched { id: new.id.clone(), name: new.name.clone() }], "the chosen id is written only now");
        assert_eq!(b.in_slot(), Some(new.id.clone()));
        assert_eq!(b.state_of(&old.id), EntryState::Downloaded);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn the_cloud_lets_the_model_go_and_this_computer_loads_it_again() {
        let models = folder("cloud");
        let m = fake(&models, "here", true);
        let mut b = Bench::new(&models, &m.id, roomy).await;
        assert_eq!(b.in_slot(), Some(m.id.clone()));
        b.keeper.set_active(false, &b.tx, &b.http);
        assert_eq!(b.in_slot(), None, "nothing stays in memory on the cloud");
        assert_eq!(b.keeper.view().current, "");
        assert_eq!(b.state_of(&m.id), EntryState::Downloaded, "and it can be deleted");
        b.keeper.set_active(true, &b.tx, &b.http);
        b.settle().await;
        assert_eq!(b.in_slot(), Some(m.id.clone()), "the chosen model comes back");
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn nothing_loads_at_start_on_the_cloud() {
        let models = folder("cloud-start");
        let m = fake(&models, "here", true);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let http = reqwest::Client::new();
        let mut keeper = LlmKeeper::with(models.clone(), &m.id, false, fake_load, roomy);
        keeper.start(&tx, &http);
        assert!(tokio::time::timeout(Duration::from_millis(300), rx.recv()).await.is_err(), "no load was started");
        assert_eq!(keeper.slot().get().map(|g| g.id().to_owned()), None);
        assert_eq!(keeper.view().entries.iter().find(|e| e.id == m.id).unwrap().state, EntryState::Downloaded);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_model_that_loads_as_the_cleanup_leaves_is_chosen_not_kept() {
        let models = folder("cloud-switch");
        let old = fake(&models, "old", true);
        let new = fake(&models, "new", true);
        let mut b = Bench::new(&models, &old.id, roomy).await;
        b.choose(Choice::Id(new.id.clone()));
        b.keeper.set_active(false, &b.tx, &b.http);
        let told = b.settle().await;
        assert_eq!(told, [Told::Switched { id: new.id.clone(), name: new.name.clone() }], "the new choice is written");
        assert_eq!(b.in_slot(), None);
        b.keeper.set_active(true, &b.tx, &b.http);
        b.settle().await;
        assert_eq!(b.in_slot(), Some(new.id.clone()), "the new choice loads on this computer");
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn delete_lets_the_model_in_use_go_first() {
        let models = folder("delete");
        let used = fake(&models, "used", true);
        let other = fake(&models, "other", true);
        let mut b = Bench::new(&models, &used.id, roomy).await;
        assert!(b.keeper.delete(&other.id, &b.tx));
        assert!(!models.join(&other.id).exists());
        assert!(b.keeper.delete(&used.id, &b.tx));
        assert!(!models.join(&used.id).exists());
        assert_eq!(b.in_slot(), None, "the cleanup pastes as said until another is chosen");
        assert_eq!(b.keeper.view().current, "");
        assert!(!b.keeper.delete("llm--nobody--nothing--q4_k_m", &b.tx));
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn no_space_says_so() {
        let models = folder("space");
        let m = fake(&models, "big", false);
        let mut b = Bench::new(&models, "", cramped).await;
        b.choose(Choice::Id(m.id.clone()));
        assert!(matches!(b.state_of(&m.id), EntryState::NoSpace { .. }));
        assert_eq!(b.in_slot(), None);
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_model_that_will_not_load_is_not_runnable() {
        let models = folder("broken");
        let m = fake(&models, "broken", true);
        let mut b = Bench::new(&models, "", roomy).await;
        b.choose(Choice::Id(m.id.clone()));
        let told = b.settle().await;
        assert!(told.iter().all(|t| *t == Told::Nothing), "{told:?}");
        assert_eq!(b.state_of(&m.id), EntryState::NotRunnable);
        assert_eq!(b.in_slot(), None);
        std::fs::remove_dir_all(models).unwrap();
    }
}
