//! The recordings of the history's entries: how each one was said, kept as Ogg Opus in `history\audio` for the period
//! the user chose (`HistorySettings::audio_period`), never longer than its entry. Only the core writes and deletes
//! them; the main window plays them (`Playable`, `read`).
//!
//! - The sound of an entry is taken on the core thread — a dictation's once its entry is written, an "Ask" question's
//!   as the question goes out, under an id of its own until its turn has an entry (`encode_as`) — as a snapshot in the
//!   recordings folder (`history::audio_snapshot`): a hard link to the WAV it was recorded into — nothing is copied —
//!   or a copy where a link cannot be made. The WAV may then be replaced or deleted at once: the next dictation's
//!   commit, a follow-up's recording, an "Ask" answer's discard. A snapshot is encoded only once its entry is written.
//! - One worker thread encodes the snapshots, one at a time, into their recordings (`history::audio_file`) and deletes
//!   them whatever came of it; the recordings past their period are deleted in the same queue, so a prune never runs
//!   alongside an encoding. A prune never touches a file still being written: what a crash left half written goes at
//!   startup alone (`sweep`), before anything is recorded.
//! - "Clear history", and recordings or the history turned off, take the whole folder away at once on the core thread
//!   and move the generation on: an encoding of an earlier generation never takes its name (the check comes right
//!   before its rename; a clear after it leaves the rename no folder), and the folder is never made again by one. The
//!   folder is renamed aside (`aside`) and deleted on the worker, so a month of recordings never holds the core up; one
//!   that cannot be renamed — a file in it still open, an encoding under way — is deleted where it is.
//! - The main window reads a recording only from a file of the folder itself, never through a link, and none larger
//!   than `MAX_BYTES`.
//!
//! Neither the sound nor a path nor an entry's id goes to the log: lengths, counts and error codes.
use std::collections::HashSet;
use std::io::{self, Read};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use cue_core::history;
use cue_core::settings::Cutoff;
use cue_platform_win::opus::{self, EncodeError, Encoded};

/// The largest recording the main window gets: more than five hours of speech, far longer than any dictation.
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// What deletes recordings (`HistorySettings::audio_limit`), as the core holds it now: the main window's commands play
/// none past it, even before the next prune deletes it. `None` — nothing is deleted, and what is there plays.
#[derive(Clone, Default)]
pub struct AudioLimit(Arc<Mutex<Option<Cutoff>>>);

impl AudioLimit {
    pub fn get(&self) -> Option<Cutoff> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, limit: Option<Cutoff>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = limit;
    }
}

/// The recordings folder and its worker; only the core thread holds it.
pub struct Recordings {
    dir: PathBuf,
    /// Moves on when the recordings are cleared or turned off: a snapshot taken before is never encoded.
    generation: Arc<AtomicU64>,
    limit: AudioLimit,
    /// The worker's queue; `None` when its thread did not start, and then nothing is recorded.
    worker: Option<mpsc::Sender<Job>>,
}

/// The sound of an entry in the recordings folder, waiting to be encoded into its recording, and the generation it
/// was taken in. No `Debug`: it holds paths.
pub struct Snapshot {
    wav: PathBuf,
    out: PathBuf,
    generation: u64,
}

/// The worker's work, in the order it came: an encoding, a prune of the recordings dated before the `days` before
/// `today`, a folder a clear set aside.
enum Job {
    Encode(Snapshot),
    Prune { days: u32, today: String },
    Remove(PathBuf),
}

impl Recordings {
    /// The recordings in `dir`, with the worker started.
    pub fn new(dir: PathBuf) -> Self {
        let (tx, jobs) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let (folder, current) = (dir.clone(), Arc::clone(&generation));
        let spawned = std::thread::Builder::new().name("cue-recordings".into()).spawn(move || {
            for job in jobs {
                // A crash in one job does not stop the others; the panic hook logs where it happened, and a snapshot
                // it left goes at the next startup.
                if catch_unwind(AssertUnwindSafe(|| run(job, &folder, &current))).is_err() {
                    tracing::error!("recordings: a job not done because of a crash, carrying on");
                }
            }
        });
        let worker = match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                tracing::error!("recordings: worker thread not started, nothing will be recorded: {e}");
                None
            }
        };
        Self { dir, generation, limit: AudioLimit::default(), worker }
    }

    /// The limit the main window's commands hold to, which `prune` keeps up to date.
    pub fn limit(&self) -> AudioLimit {
        self.limit.clone()
    }

    /// At startup, before anything is recorded: what a crash left half written goes, and the folders a clear set aside
    /// but did not get to delete (on the worker).
    pub fn sweep(&self) {
        match history::sweep_audio_temps(&self.dir) {
            Ok(0) => {}
            Ok(files) => tracing::info!(files, "recordings: half-written files of an earlier run deleted"),
            Err(e) => tracing::warn!("recordings: half-written files of an earlier run not deleted: {e}"),
        }
        for folder in set_aside(&self.dir) {
            self.remove(folder);
        }
    }

    /// From now on recordings stay until `limit`: past it, counted back from `today` (`YYYY-MM-DD`), they go on the
    /// worker, after the encodings queued before; `All` clears them all at once; `None` deletes nothing.
    pub fn prune(&self, limit: Option<Cutoff>, today: &str) {
        self.limit.set(limit);
        match limit {
            None => {}
            Some(Cutoff::All) => self.clear(),
            Some(Cutoff::DaysBack(days)) => {
                if let Err(job) = self.send(Job::Prune { days, today: today.to_owned() }) {
                    run(job, &self.dir, &self.generation);
                }
            }
        }
    }

    /// Every recording goes at once, with the folder and whatever is in it: renamed aside, then deleted on the worker.
    /// An encoding still under way never takes its name.
    pub fn clear(&self) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let aside = aside(&self.dir, generation);
        match std::fs::rename(&self.dir, &aside) {
            Ok(()) => {
                tracing::info!("recordings cleared");
                self.remove(aside);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            // A file in it is still open (an encoding under way, a read of the main window): deleted where it is.
            Err(_) => match history::clear_audio(&self.dir) {
                Ok(()) => tracing::info!("recordings cleared"),
                Err(e) => tracing::warn!("recordings: not cleared: {e}"),
            },
        }
    }

    /// A folder set aside goes on the worker, or here when it is not running.
    fn remove(&self, folder: PathBuf) {
        if let Err(Job::Remove(folder)) = self.send(Job::Remove(folder)) {
            remove_folder(&folder);
        }
    }

    /// The snapshot of `sound` for the entry `id`, in the recordings folder (made if needed); `None` when it could not
    /// be taken (one log line) or nothing would encode it.
    pub fn snapshot(&self, sound: &Path, id: &str) -> Option<Snapshot> {
        self.worker.as_ref()?;
        let (Some(wav), Some(out)) = (history::audio_snapshot(id), history::audio_file(id)) else {
            tracing::warn!("recording: the entry's id names no recording");
            return None;
        };
        let (wav, out) = (self.dir.join(wav), self.dir.join(out));
        let generation = self.generation.load(Ordering::SeqCst);
        match std::fs::create_dir_all(&self.dir).and_then(|()| link_or_copy(sound, &wav)) {
            Ok(()) => Some(Snapshot { wav, out, generation }),
            Err(e) => {
                if e.kind() == io::ErrorKind::NotFound {
                    tracing::info!("recording: the entry has no sound on disk");
                } else {
                    tracing::warn!("recording: the entry's sound not kept: {e}");
                }
                remove(&wav);
                None
            }
        }
    }

    /// The snapshot, taken under a name of its own before its entry had an id (an "Ask" question's), is encoded into the
    /// recording of the entry `id`, and deleted.
    pub fn encode_as(&self, snapshot: Snapshot, id: &str) {
        match history::audio_file(id) {
            Some(out) => self.encode(Snapshot { out: self.dir.join(out), ..snapshot }),
            None => {
                tracing::warn!("recording: the entry's id names no recording");
                remove(&snapshot.wav);
            }
        }
    }

    /// The snapshot is encoded into its recording on the worker, and deleted.
    pub fn encode(&self, snapshot: Snapshot) {
        if let Err(Job::Encode(snapshot)) = self.send(Job::Encode(snapshot)) {
            tracing::warn!("recording not kept: the worker is not running");
            remove(&snapshot.wav);
        }
    }

    /// The snapshot of an entry that was not written goes.
    pub fn discard(&self, snapshot: Snapshot) {
        remove(&snapshot.wav);
    }

    /// The job to the worker; back when it is not running.
    fn send(&self, job: Job) -> Result<(), Job> {
        match &self.worker {
            Some(tx) => tx.send(job).map_err(|e| e.0),
            None => Err(job),
        }
    }
}

/// One job of the worker.
fn run(job: Job, dir: &Path, generation: &AtomicU64) {
    match job {
        Job::Encode(snapshot) => {
            let current = || generation.load(Ordering::SeqCst) == snapshot.generation;
            // A snapshot taken before a clear is not even started.
            let done = if current() { encode(&snapshot, current) } else { Err(EncodeError::Dropped) };
            log_encoded(&done, !current());
            remove(&snapshot.wav);
        }
        Job::Prune { days, today } => match history::prune_audio(dir, Cutoff::DaysBack(days), &today) {
            Ok(0) => {}
            Ok(recordings) => tracing::info!(recordings, days, "recordings: old ones deleted"),
            Err(e) => tracing::warn!("recordings: old ones not deleted: {e}"),
        },
        Job::Remove(folder) => remove_folder(&folder),
    }
}

/// What came of an encoding, in one line; `cleared` — the recordings were cleared since its snapshot was taken, which
/// takes its file or its folder away while it is written: whatever failed then, it was dropped.
fn log_encoded(done: &Result<Encoded, EncodeError>, cleared: bool) {
    match done {
        Ok(Encoded { duration_ms, bytes }) => tracing::info!(duration_ms, bytes, "recording kept"),
        Err(EncodeError::Dropped) => tracing::info!("recording dropped: the recordings were cleared meanwhile"),
        Err(_) if cleared => tracing::info!("recording dropped: the recordings were cleared meanwhile"),
        Err(e) => tracing::warn!("recording not kept: {e}"),
    }
}

/// Where a clear puts the recordings folder `dir` until it is deleted: next to it (a rename on one volume is done at
/// once), under its name with `.cleared-` and the clear's generation.
fn aside(dir: &Path, generation: u64) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".cleared-{generation}"));
    dir.with_file_name(name)
}

/// The folders set aside next to `dir` (`aside`) that are still there; a link is no such folder.
fn set_aside(dir: &Path) -> Vec<PathBuf> {
    let (Some(parent), Some(name)) = (dir.parent(), dir.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{name}.cleared-");
    let generation = |g: &str| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit());
    let ours = |n: &str| n.strip_prefix(prefix.as_str()).is_some_and(generation);
    let Ok(listing) = std::fs::read_dir(parent) else { return Vec::new() };
    listing
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| e.file_name().to_str().is_some_and(ours))
        .map(|e| e.path())
        .collect()
}

/// Deletes a folder a clear set aside, with all in it; one that cannot be deleted goes at the next startup.
fn remove_folder(folder: &Path) {
    if let Err(e) = std::fs::remove_dir_all(folder)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("recordings: a cleared folder not deleted: {e}");
    }
}

/// The snapshot into its recording, asking `still_wanted` right before the recording takes its name.
fn encode(snapshot: &Snapshot, still_wanted: impl FnOnce() -> bool) -> Result<Encoded, EncodeError> {
    opus::encode_wav_to_ogg_opus_if(&snapshot.wav, &snapshot.out, still_wanted)
}

/// `to` becomes another name of the file `from` — a hard link, nothing copied — or a copy of it where no link can be
/// made (another volume, a file system without links). A missing `from` is not copied either. A file already under the
/// name `to` (left by a run whose startup sweep could not delete it) gives way first.
fn link_or_copy(from: &Path, to: &Path) -> io::Result<()> {
    if let Err(e) = std::fs::remove_file(to)
        && e.kind() != io::ErrorKind::NotFound
    {
        return Err(e);
    }
    match std::fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(e),
        Err(_) => std::fs::copy(from, to).map(drop),
    }
}

/// Deletes a snapshot; one already gone is no error, and one that cannot be deleted goes at the next startup.
fn remove(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("recording: its snapshot not deleted: {e}");
    }
}

/// Whether the recording `file` is past `limit`, counted back from `today`: a prune now would delete it.
fn expired(file: &str, limit: Option<Cutoff>, today: &str) -> bool {
    limit.is_some_and(|cutoff| !history::audio_to_delete([file], cutoff, today).is_empty())
}

/// The recordings the main window may play, from one listing of the folder `dir`: those there, but for any past
/// `limit` (`AudioLimit`), counted back from `today`.
pub struct Playable {
    files: HashSet<String>,
    limit: Option<Cutoff>,
    today: String,
}

impl Playable {
    /// No folder, or one that cannot be listed: none plays.
    pub fn read(dir: &Path, limit: Option<Cutoff>, today: &str) -> Self {
        let files: HashSet<String> = std::fs::read_dir(dir)
            .map(|listing| {
                listing
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default();
        Self { files, limit, today: today.to_owned() }
    }

    /// Whether the entry `id` has a recording to play.
    pub fn has(&self, id: &str) -> bool {
        let plays = |file: &String| self.files.contains(file) && !expired(file, self.limit, &self.today);
        history::audio_file(id).is_some_and(|file| plays(&file))
    }
}

/// The bytes of the recording `file` (`history::audio_file`) of the folder `dir`; `None` when there is none to play —
/// never made, deleted, past `limit` counted back from `today`, or not a file of the folder (a folder, a link). One
/// larger than `MAX_BYTES` is an error: CUE never writes one.
pub fn read(dir: &Path, file: &str, limit: Option<Cutoff>, today: &str) -> io::Result<Option<Vec<u8>>> {
    if expired(file, limit, today) {
        return Ok(None);
    }
    let path = dir.join(file);
    let found = match std::fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() && m.len() <= MAX_BYTES => read_at_most(&path, MAX_BYTES).map(Some),
        Ok(m) if m.is_file() => Err(too_large()),
        Ok(_) => Ok(None),
        Err(e) => Err(e),
    };
    match found {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        other => other,
    }
}

/// The bytes of the file `path`, as long as it holds no more than `max` of them (it may have grown since it was looked
/// at).
fn read_at_most(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max { Err(too_large()) } else { Ok(bytes) }
}

fn too_large() -> io::Error {
    io::Error::other("the recording is larger than any CUE writes")
}

#[cfg(test)]
impl Recordings {
    /// Recordings whose jobs wait in the returned queue until the test runs them on its own thread (`Queue::run`).
    pub fn queued(dir: PathBuf) -> (Self, Queue) {
        let (tx, jobs) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let queue = Queue { jobs, dir: dir.clone(), generation: Arc::clone(&generation) };
        (Self { dir, generation, limit: AudioLimit::default(), worker: Some(tx) }, queue)
    }
}

/// The worker's queue, run by a test.
#[cfg(test)]
pub struct Queue {
    jobs: mpsc::Receiver<Job>,
    dir: PathBuf,
    generation: Arc<AtomicU64>,
}

#[cfg(test)]
impl Queue {
    /// Runs the jobs waiting, in order; returns how many.
    pub fn run(&self) -> usize {
        let mut n = 0;
        while let Ok(job) = self.jobs.try_recv() {
            run(job, &self.dir, &self.generation);
            n += 1;
        }
        n
    }
}

/// A WAV of `ms` milliseconds of a tone, as CUE records it, at `path`.
#[cfg(test)]
pub fn tone(path: &Path, ms: usize) {
    let mut wav = cue_platform_win::wav::WavSink::create(path).unwrap();
    let pcm: Vec<i16> = (0..ms * 16).map(|i| ((i as f32 * 0.07).sin() * 8_000.0) as i16).collect();
    wav.write(&pcm);
    wav.finish().unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::settings::AudioKeep;

    /// A folder of the test's own, deleted with all in it even when the check failed.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cue-test-{}-recordings-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn audio(&self) -> PathBuf {
            self.0.join("audio")
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const ID: &str = "2026-10-31T07:30:00.123-4";

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = match std::fs::read_dir(dir) {
            Ok(listing) => listing.map(|e| e.unwrap().file_name().into_string().unwrap()).collect(),
            Err(_) => return Vec::new(),
        };
        names.sort();
        names
    }

    fn recording(id: &str) -> String {
        history::audio_file(id).unwrap()
    }

    /// The snapshot is another name of the sound, so the sound may be replaced or deleted at once; the worker encodes
    /// it into the entry's recording and deletes it.
    #[test]
    fn a_snapshot_outlives_its_sound_and_becomes_the_recording() {
        let d = Dir::new("encode");
        let sound = d.0.join("audio.wav");
        tone(&sound, 1_000);
        let original = std::fs::read(&sound).unwrap();
        let (rec, queue) = Recordings::queued(d.audio());
        let snapshot = rec.snapshot(&sound, ID).unwrap();
        assert_eq!(names(&d.audio()), [history::audio_snapshot(ID).unwrap()]);
        // The next dictation's commit puts its own sound under the name; an answer's discard deletes it.
        tone(&d.0.join("next.wav"), 200);
        std::fs::rename(d.0.join("next.wav"), &sound).unwrap();
        assert_eq!(std::fs::read(&snapshot.wav).unwrap(), original, "the snapshot is this entry's sound");
        std::fs::remove_file(&sound).unwrap();

        rec.encode(snapshot);
        assert_eq!(names(&d.audio()).len(), 1, "nothing is encoded on the core thread");
        assert_eq!(queue.run(), 1);
        assert_eq!(names(&d.audio()), [recording(ID)], "the snapshot is gone");
        let bytes = std::fs::read(d.audio().join(recording(ID))).unwrap();
        assert!(bytes.starts_with(b"OggS") && bytes.len() > 1_000 && bytes.len() < 4_000, "{}", bytes.len());
        let day = Some(AudioKeep::Day.cutoff());
        assert_eq!(read(&d.audio(), &recording(ID), day, "2026-10-31").unwrap(), Some(bytes));
    }

    /// An "Ask" question's snapshot is taken before its entry has an id, under a name of its own the startup sweep
    /// knows; it becomes the recording of the entry it is given to.
    #[test]
    fn a_snapshot_taken_before_its_entry_becomes_that_entrys_recording() {
        let d = Dir::new("encode-as");
        let sound = d.0.join("current.wav");
        tone(&sound, 300);
        let (rec, queue) = Recordings::queued(d.audio());
        let (asked, entry) = ("2026-10-31T07:30:00.000-1", "2026-10-31T07:30:04.500-2");
        let snapshot = rec.snapshot(&sound, asked).unwrap();
        let taken = history::audio_snapshot(asked).unwrap();
        assert_eq!(names(&d.audio()), [taken.as_str()]);
        assert_eq!(history::audio_temps([taken.as_str()]), [taken.as_str()], "a crash leaves it to the sweep");
        rec.encode_as(snapshot, entry);
        assert_eq!(queue.run(), 1);
        assert_eq!(names(&d.audio()), [recording(entry)], "its own name gone, the entry's recording made");

        let snapshot = rec.snapshot(&sound, asked).unwrap();
        rec.encode_as(snapshot, "../not-an-id");
        assert_eq!((queue.run(), names(&d.audio())), (0, vec![recording(entry)]), "no recording, no snapshot left");
    }

    /// "Clear history" between the snapshot and its turn on the worker: the folder is gone at once, and the encoding
    /// neither runs nor makes the folder again.
    #[test]
    fn a_snapshot_taken_before_a_clear_is_never_encoded() {
        let d = Dir::new("clear-before");
        let sound = d.0.join("audio.wav");
        tone(&sound, 300);
        let (rec, queue) = Recordings::queued(d.audio());
        let snapshot = rec.snapshot(&sound, ID).unwrap();
        rec.clear();
        assert!(!d.audio().exists(), "cleared at once");
        rec.encode(snapshot);
        assert_eq!(queue.run(), 2, "the folder set aside is deleted, then the encoding dropped");
        assert!(!d.audio().exists(), "not made again");
        assert_eq!(names(&d.0), ["audio.wav"], "nothing set aside is left");

        // Its snapshot made again by hand (a clear that could not delete it): still never encoded, and deleted.
        let snapshot = rec.snapshot(&sound, ID).unwrap();
        let stale = Snapshot { wav: snapshot.wav, out: snapshot.out, generation: snapshot.generation - 1 };
        rec.encode(stale);
        assert_eq!(queue.run(), 1);
        assert_eq!(names(&d.audio()), Vec::<String>::new());
    }

    /// A clear that comes while the encoding runs: the check right before the rename drops it, and a clear right
    /// after the check leaves the rename no folder to put it in. Either way no recording comes back.
    #[test]
    fn a_clear_during_an_encoding_does_not_bring_the_recording_back() {
        let d = Dir::new("clear-during");
        let sound = d.0.join("audio.wav");
        tone(&sound, 300);
        let (rec, _queue) = Recordings::queued(d.audio());

        let snapshot = rec.snapshot(&sound, ID).unwrap();
        let generation = Arc::clone(&rec.generation);
        let done = encode(&snapshot, || {
            rec.clear();
            generation.load(Ordering::SeqCst) == snapshot.generation
        });
        assert!(matches!(done, Err(EncodeError::Dropped)), "{done:?}");
        assert!(!d.audio().exists());

        let snapshot = rec.snapshot(&sound, ID).unwrap();
        let done = encode(&snapshot, || {
            rec.clear();
            true
        });
        assert!(matches!(done, Err(EncodeError::Write(_))), "{done:?}");
        assert!(!d.audio().exists(), "the rename found no folder, and made none");
    }

    /// A prune waits for the encodings queued before it; off clears at once; unknown deletes nothing. The main
    /// window's limit follows each.
    #[test]
    fn prunes_run_on_the_worker_in_turn_with_encodings() {
        let d = Dir::new("prune");
        let sound = d.0.join("audio.wav");
        tone(&sound, 200);
        let (rec, queue) = Recordings::queued(d.audio());
        let old = "2026-10-29T09:00:00.000-1";
        rec.encode(rec.snapshot(&sound, old).unwrap());
        let day = AudioKeep::Day.cutoff();
        rec.prune(Some(day), "2026-10-31");
        assert_eq!(rec.limit().get(), Some(day));
        assert_eq!(queue.run(), 2);
        assert_eq!(names(&d.audio()), Vec::<String>::new(), "encoded, then deleted as older than a day");

        rec.encode(rec.snapshot(&sound, ID).unwrap());
        assert_eq!(queue.run(), 1);
        rec.prune(None, "2026-11-30");
        assert_eq!((rec.limit().get(), queue.run()), (None, 0), "unknown: nothing to do");
        assert_eq!(names(&d.audio()), [recording(ID)]);
        rec.prune(Some(AudioKeep::Week.cutoff()), "2026-11-30");
        assert_eq!(queue.run(), 1);
        assert_eq!(names(&d.audio()), Vec::<String>::new());

        // The history's own year while the recordings' period is unknown: 366 days back.
        rec.encode(rec.snapshot(&sound, "2025-10-30T09:00:00.000-2").unwrap());
        rec.encode(rec.snapshot(&sound, "2025-10-29T09:00:00.000-3").unwrap());
        rec.prune(Some(Cutoff::DaysBack(366)), "2026-10-31");
        assert_eq!(queue.run(), 3);
        assert_eq!(names(&d.audio()), [recording("2025-10-30T09:00:00.000-2")]);

        std::fs::write(d.audio().join("notes.txt"), "x").unwrap();
        rec.prune(Some(Cutoff::All), "2026-11-30");
        assert_eq!(rec.limit().get(), Some(Cutoff::All));
        assert!(!d.audio().exists(), "off: cleared at once");
        assert_eq!(queue.run(), 1, "and deleted on the worker");
        assert_eq!(names(&d.0), ["audio.wav"]);
    }

    /// A clear renames the folder aside and the worker deletes it; one that cannot be renamed is deleted where it is;
    /// at startup the folders a clear left aside go on the worker — only CUE's own.
    #[test]
    fn a_clear_sets_the_folder_aside_and_the_worker_deletes_it() {
        let d = Dir::new("aside");
        let (rec, queue) = Recordings::queued(d.audio());
        std::fs::create_dir_all(d.audio()).unwrap();
        for n in 0..50 {
            std::fs::write(d.audio().join(recording(&format!("2026-10-31T07:30:00.000-{}", n + 1))), b"OggS").unwrap();
        }
        rec.clear();
        assert_eq!(names(&d.0), ["audio.cleared-1"], "gone at once, set aside");
        assert_eq!(queue.run(), 1);
        assert_eq!(names(&d.0), Vec::<String>::new());

        // A file in it held open: Windows renames no such folder, and it is deleted where it is.
        std::fs::create_dir_all(d.audio()).unwrap();
        let open = std::fs::File::create(d.audio().join(recording(ID) + ".tmp")).unwrap();
        rec.clear();
        drop(open);
        let left: Vec<String> = names(&d.0);
        assert!(!d.audio().exists(), "{left:?}");
        queue.run();
        assert_eq!(names(&d.0), Vec::<String>::new());

        for folder in ["audio.cleared-7", "audio.cleared-", "audio.cleared-x", "other.cleared-2", "audio"] {
            std::fs::create_dir_all(d.0.join(folder)).unwrap();
        }
        std::fs::write(d.0.join("audio.cleared-7").join("x.ogg"), b"OggS").unwrap();
        std::fs::write(d.0.join("audio.cleared-3"), b"a file").unwrap();
        rec.sweep();
        assert_eq!(queue.run(), 1);
        let kept = ["audio", "audio.cleared-", "audio.cleared-3", "audio.cleared-x", "other.cleared-2"];
        assert_eq!(names(&d.0), kept);
    }

    /// The worker not running: nothing is recorded, a prune runs where it is asked for.
    #[test]
    fn without_the_worker_nothing_is_recorded() {
        let d = Dir::new("no-worker");
        let sound = d.0.join("audio.wav");
        tone(&sound, 200);
        let (rec, queue) = Recordings::queued(d.audio());
        let snapshot = rec.snapshot(&sound, ID).unwrap();
        drop(queue);
        rec.encode(snapshot);
        assert_eq!(names(&d.audio()), Vec::<String>::new(), "its snapshot deleted");
        std::fs::write(d.audio().join(recording("2026-10-01T09:00:00.000-1")), b"OggS").unwrap();
        rec.prune(Some(AudioKeep::Week.cutoff()), "2026-10-31");
        assert_eq!(names(&d.audio()), Vec::<String>::new());

        let none = Recordings { worker: None, ..Recordings::queued(d.audio()).0 };
        assert!(none.snapshot(&sound, ID).is_none());
        assert_eq!(names(&d.audio()), Vec::<String>::new());
    }

    /// An entry without sound on disk gets no recording, and leaves nothing behind.
    #[test]
    fn no_sound_no_snapshot() {
        let d = Dir::new("no-sound");
        let (rec, queue) = Recordings::queued(d.audio());
        assert!(rec.snapshot(&d.0.join("missing.wav"), ID).is_none());
        assert!(rec.snapshot(&d.0.join("missing.wav"), "../../x").is_none(), "not an entry id");
        assert_eq!((names(&d.audio()), queue.run()), (Vec::<String>::new(), 0));
    }

    /// The main window plays what is there and not past the limit; a name that is no entry id, a folder named like a
    /// recording, or a recording already deleted plays nothing.
    #[test]
    fn only_recordings_within_the_limit_play() {
        let d = Dir::new("playable");
        let audio = d.audio();
        std::fs::create_dir_all(&audio).unwrap();
        let (today, yesterday, folder) =
            ("2026-10-31T09:00:00.000-1", "2026-10-30T09:00:00.000-2", "2026-10-31T10:00:00.000-3");
        for id in [today, yesterday] {
            std::fs::write(audio.join(recording(id)), id).unwrap();
        }
        std::fs::create_dir_all(audio.join(recording(folder))).unwrap();
        let missing = "2026-10-31T11:00:00.000-4";

        let at = |limit| Playable::read(&audio, limit, "2026-10-31");
        let plays = |p: &Playable| [today, yesterday, folder, missing, "..", ""].map(|id| p.has(id));
        assert_eq!(plays(&at(Some(AudioKeep::Week.cutoff()))), [true, true, false, false, false, false]);
        assert_eq!(plays(&at(None)), [true, true, false, false, false, false], "unknown: what is there");
        assert_eq!(plays(&at(Some(AudioKeep::Day.cutoff()))), [true, false, false, false, false, false]);
        assert_eq!(plays(&at(Some(AudioKeep::Off.cutoff()))), [false; 6]);
        assert_eq!(plays(&Playable::read(&d.0.join("none"), None, "2026-10-31")), [false; 6]);

        let bytes = |id: &str, limit| read(&audio, &recording(id), limit, "2026-10-31").unwrap();
        assert_eq!(bytes(today, Some(AudioKeep::Day.cutoff())), Some(today.as_bytes().to_vec()));
        assert_eq!(bytes(yesterday, Some(AudioKeep::Week.cutoff())), Some(yesterday.as_bytes().to_vec()));
        assert_eq!(bytes(yesterday, Some(AudioKeep::Day.cutoff())), None, "past the limit, not yet deleted");
        assert_eq!(bytes(yesterday, None), Some(yesterday.as_bytes().to_vec()));
        assert_eq!(bytes(folder, None), None);
        assert_eq!(bytes(missing, None), None);
    }

    /// The main window never gets a recording larger than CUE writes, nor one through a link.
    #[test]
    fn a_recording_too_large_or_behind_a_link_is_not_read() {
        let d = Dir::new("read-limits");
        let audio = d.audio();
        std::fs::create_dir_all(&audio).unwrap();
        let (large, linked) = ("2026-10-31T09:00:00.000-1", "2026-10-31T09:01:00.000-2");
        std::fs::File::create(audio.join(recording(large))).unwrap().set_len(MAX_BYTES + 1).unwrap();
        let e = read(&audio, &recording(large), None, "2026-10-31").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Other, "{e}");
        let small = d.0.join("small");
        std::fs::write(&small, b"OggS").unwrap();
        assert_eq!(read_at_most(&small, 4).unwrap(), b"OggS");
        assert!(read_at_most(&small, 3).is_err(), "grown past the size it was looked at with");
        // Where Windows lets this account make a link: one under a recording's name is no recording.
        if std::os::windows::fs::symlink_file(&small, audio.join(recording(linked))).is_ok() {
            assert_eq!(read(&audio, &recording(linked), None, "2026-10-31").unwrap(), None);
            assert!(!Playable::read(&audio, None, "2026-10-31").has(linked));
        }
    }

    /// The line an encoding gives: a failure after a clear is a drop, not a fault.
    #[test]
    fn an_encoding_that_fails_after_a_clear_is_dropped() {
        // A second subscriber alive, as in `the_log_has_no_path_and_no_id`.
        let _second = tracing::Dispatch::new(tracing_subscriber::fmt().with_writer(io::sink).finish());
        let line = |cleared: bool| {
            let cap = Capture::default();
            let writer = cap.clone();
            let sub =
                tracing_subscriber::fmt().with_writer(move || writer.clone()).with_ansi(false).without_time().finish();
            tracing::subscriber::with_default(sub, || {
                tracing::callsite::rebuild_interest_cache();
                log_encoded(&Err(EncodeError::Write(io::Error::from(io::ErrorKind::NotFound))), cleared);
            });
            String::from_utf8(cap.0.lock().unwrap().clone()).unwrap()
        };
        let (after, before) = (line(true), line(false));
        assert!(after.contains("recording dropped") && !after.contains("not kept"), "{after}");
        assert!(before.contains("recording not kept"), "{before}");
    }

    /// A log writer into memory, to check the lines.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Capture {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Each outcome is one line of counts and codes: no path, no file name, no id.
    #[test]
    fn the_log_has_no_path_and_no_id() {
        let d = Dir::new("log");
        let cap = Capture::default();
        let writer = cap.clone();
        let sub =
            tracing_subscriber::fmt().with_writer(move || writer.clone()).with_ansi(false).without_time().finish();
        // tracing works out once per line of the log who wants it, and while a single subscriber is alive it asks only
        // the thread that meets the line first: another test's, which has none. With a second one alive it asks them
        // all, this test's too.
        let _second = tracing::Dispatch::new(tracing_subscriber::fmt().with_writer(io::sink).finish());
        tracing::subscriber::with_default(sub, || {
            tracing::callsite::rebuild_interest_cache();
            let sound = d.0.join("audio.wav");
            tone(&sound, 300);
            let (rec, queue) = Recordings::queued(d.audio());
            rec.encode(rec.snapshot(&sound, ID).unwrap());
            queue.run();
            std::fs::write(d.0.join("broken.wav"), b"RIFF not a wave").unwrap();
            rec.encode(rec.snapshot(&d.0.join("broken.wav"), "2026-10-31T07:31:00.000-5").unwrap());
            queue.run();
            let snapshot = rec.snapshot(&sound, "2026-10-31T07:32:00.000-6").unwrap();
            rec.clear();
            rec.encode(snapshot);
            queue.run();
            assert!(rec.snapshot(&d.0.join("missing.wav"), ID).is_none());
            rec.encode(rec.snapshot(&sound, "2026-10-01T07:30:00.000-7").unwrap());
            rec.prune(Some(AudioKeep::Day.cutoff()), "2026-10-31");
            queue.run();
            std::fs::write(d.audio().join(history::audio_snapshot(ID).unwrap()), b"left").unwrap();
            rec.sweep();
        });
        let log = String::from_utf8(cap.0.lock().unwrap().clone()).unwrap();
        for line in ["recording kept", "recording not kept", "recording dropped", "no sound", "old ones deleted"] {
            assert!(log.contains(line), "{line}: {log}");
        }
        assert!(log.contains("half-written files of an earlier run deleted"), "{log}");
        let dir = d.0.to_string_lossy().into_owned();
        let private = ["cue-test", "audio.wav", "broken", "missing", "2026-10", ".ogg", ".wav", "Users"];
        for private in private.into_iter().chain([dir.as_str()]) {
            assert!(!log.contains(private), "{private}: {log}");
        }
    }
}
