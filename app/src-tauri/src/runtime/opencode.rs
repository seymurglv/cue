//! The OpenCode server CUE cleans through (docs/specs/2026-11-08-no-keys-design.md, "Your AI", "OpenCode (default)"):
//! the one the user runs, when it announces itself in OpenCode's state file, else one CUE starts with
//! `opencode serve` on 127.0.0.1 when a cleanup first needs it.
//!
//! A server CUE started rests as the local models do (`llm.rs`, `model.rs`): it is stopped after
//! [`REST_AFTER`] without a dictation, when the session locks or the computer sleeps (once the dictation then under way
//! ends), and when CUE quits; the next cleanup starts it again. A server the user runs is never stopped: CUE only lets
//! go of it.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use cue_core::agents::AgentId;
use cue_engines::agent::opencode::{self, Server};
use cue_engines::agent::{self, AgentFailure, discover};
use tokio::task::AbortHandle;

use super::model::REST_AFTER;

type Finder = Box<dyn Fn() -> Option<Server> + Send + Sync>;
type Starter = Box<dyn Fn() -> Result<Server, AgentFailure> + Send + Sync>;

/// The OpenCode server in use, and when it rests.
pub struct OpenCodeSlot {
    held: Mutex<Option<Arc<Server>>>,
    /// One start at a time; `held` is not locked meanwhile, so a rest never waits for a start.
    starting: Mutex<()>,
    find: Finder,
    start: Starter,
    rest: Mutex<Rest>,
    rest_after: Duration,
    /// Bumped by every rest: a start that a rest came during stops the server it started.
    rests: AtomicU64,
}

#[derive(Default)]
struct Rest {
    /// A dictation records or is worked on: the server is not stopped meanwhile.
    busy: bool,
    /// The session locked or the computer went to sleep during a dictation: stopped once it ends.
    when_done: bool,
    /// The wait before the server is stopped, started when a dictation ends.
    timer: Option<AbortHandle>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl OpenCodeSlot {
    /// The slot for this computer: the user's server from OpenCode's state folder (OpenCode Desktop's, say), else
    /// `opencode serve` (OpenCode Desktop's `opencode-cli.exe` when that is the one found) started in OpenCode's own
    /// empty folder in `home`, `%LOCALAPPDATA%\CUE\agent\opencode` only (`opencode::server_cwd`: OpenCode keeps a
    /// server's folder as a project, so never a spare; something in it and no server is started). A V1 session on a
    /// server the user runs is told to work in that folder, or a spare empty one beside it (`agent::empty_cwd`).
    pub fn new(home: PathBuf) -> Arc<Self> {
        let dir = home.clone();
        let find: Finder = Box::new(move || {
            let cwd = agent::empty_cwd(&dir, AgentId::OpenCode).ok()?;
            opencode::state_dir().and_then(|s| Server::announced(&s, &cwd))
        });
        let start: Starter = Box::new(move || {
            let exe = discover::program(AgentId::OpenCode).ok_or(AgentFailure::NotInstalled)?;
            opencode::start(&exe, &opencode::server_cwd(&home)?)
        });
        Self::with(find, start, REST_AFTER)
    }

    pub(super) fn with(find: Finder, start: Starter, rest_after: Duration) -> Arc<Self> {
        Arc::new(Self {
            held: Mutex::new(None),
            starting: Mutex::new(()),
            find,
            start,
            rest: Mutex::new(Rest::default()),
            rest_after,
            rests: AtomicU64::new(0),
        })
    }

    /// The server to send a cleanup to: the one in use while it runs, else the user's own, else one CUE starts now
    /// (blocking up to `opencode::START_WAIT`: call it off the core's thread). A rest during the start stops the
    /// server it started: `Failed("rest")`.
    pub fn get_or_start(&self) -> Result<Arc<Server>, AgentFailure> {
        let _one = lock(&self.starting);
        let held = lock(&self.held).clone();
        if let Some(s) = held {
            if s.alive() {
                return Ok(s);
            }
            self.let_go(&s);
        }
        let rests = self.rests.load(Ordering::SeqCst);
        let server = match (self.find)().filter(Server::alive) {
            Some(s) => {
                tracing::info!("opencode: uses the server the user runs");
                s
            }
            None => (self.start)()?,
        };
        let server = Arc::new(server);
        // Checked under `held`, which `rest` takes after bumping the count: a rest either sees this server or is seen.
        let mut held = lock(&self.held);
        if self.rests.load(Ordering::SeqCst) != rests {
            drop(held);
            // CUE rested, the session locked or CUE quit while the server started: it does not run on.
            server.halt();
            return Err(AgentFailure::Failed("rest"));
        }
        *held = Some(server.clone());
        Ok(server)
    }

    /// A server already running, never one started for it: the one in use while it runs, else the one the user runs
    /// (held from then on, and never stopped by CUE). None when neither runs. For the Cleanup page's list of OpenCode's
    /// models; it finds by OpenCode's state file only, so it is quick (call it off the core's thread all the same).
    pub fn running(&self) -> Option<Arc<Server>> {
        let _one = lock(&self.starting);
        let held = lock(&self.held).clone();
        if let Some(s) = held {
            if s.alive() {
                return Some(s);
            }
            self.let_go(&s);
        }
        let server = Arc::new((self.find)().filter(Server::alive)?);
        tracing::info!("opencode: the Cleanup page uses the server the user runs");
        *lock(&self.held) = Some(server.clone());
        Some(server)
    }

    /// A server is held: the next cleanup goes to it without finding or starting one. None after CUE starts and after a
    /// rest, so the next request is OpenCode's first and gets the start allowance.
    pub fn holds(&self) -> bool {
        lock(&self.held).is_some()
    }

    /// `s` is no longer used: stopped if CUE started it.
    fn let_go(&self, s: &Arc<Server>) {
        let mut held = lock(&self.held);
        if held.as_ref().is_some_and(|h| Arc::ptr_eq(h, s)) {
            *held = None;
        }
        drop(held);
        s.halt();
    }

    /// A dictation started (`true`) or ended (`false`). One starting stops the wait; one ending starts it, or stops
    /// the server now after a lock or sleep came during it.
    pub fn busy(self: &Arc<Self>, busy: bool) {
        let mut rest = lock(&self.rest);
        rest.busy = busy;
        if let Some(timer) = rest.timer.take() {
            timer.abort();
        }
        if busy {
            return;
        }
        if std::mem::take(&mut rest.when_done) {
            drop(rest);
            self.rest();
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
        let (me, after) = (Arc::downgrade(self), self.rest_after);
        rest.timer = Some(rt.spawn(rest_later(me, after)).abort_handle());
    }

    /// The session locked or the computer goes to sleep: the server CUE started is stopped now, or once the dictation
    /// ends.
    pub fn rest_soon(&self) {
        let mut rest = lock(&self.rest);
        if rest.busy {
            rest.when_done = true;
        } else {
            drop(rest);
            self.rest();
        }
    }

    /// CUE rests or quits: the server CUE started is stopped (its whole process tree), the user's is let go.
    pub fn rest(&self) {
        self.rests.fetch_add(1, Ordering::SeqCst);
        let held = lock(&self.held).take();
        if let Some(s) = held {
            s.halt();
        }
    }
}

/// After `after`, the slot rests unless a dictation started meanwhile.
async fn rest_later(slot: Weak<OpenCodeSlot>, after: Duration) {
    tokio::time::sleep(after).await;
    let Some(slot) = slot.upgrade() else { return };
    // Stopping waits for the processes to end: not on the async threads.
    let _ = tokio::task::spawn_blocking(move || {
        if !lock(&slot.rest).busy {
            slot.rest();
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cue_engines::agent::process;
    use reqwest::Url;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-opencode-slot-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A server "CUE started": a process that runs two minutes unless stopped.
    fn owned(cwd: &Path) -> Server {
        let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        let ping = root.join("System32").join("PING.EXE");
        let args: Vec<String> = ["-n", "120", "127.0.0.1"].iter().map(|a| a.to_string()).collect();
        let running = process::spawn(&ping, &args, &[], cwd).unwrap();
        Server::owned(Url::parse("http://127.0.0.1:9/").unwrap(), None, running)
    }

    fn counting(starts: &Arc<AtomicUsize>, cwd: PathBuf) -> Starter {
        let starts = starts.clone();
        Box::new(move || {
            starts.fetch_add(1, Ordering::SeqCst);
            Ok(owned(&cwd))
        })
    }

    #[test]
    fn slot_reuses_announced_server_and_never_stops_it() {
        let (state, cwd) = (temp_dir("state"), temp_dir("cwd"));
        // The user's own server: something listening on loopback, registered in OpenCode's state file.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::fs::write(state.join("service.json"), format!(r#"{{"url":"http://127.0.0.1:{port}","pid":1}}"#)).unwrap();
        let starts = Arc::new(AtomicUsize::new(0));
        let (s, c) = (state.clone(), cwd.clone());
        let started = starts.clone();
        let slot = OpenCodeSlot::with(
            Box::new(move || Server::announced(&s, &c)),
            Box::new(move || {
                started.fetch_add(1, Ordering::SeqCst);
                Err(AgentFailure::NotInstalled)
            }),
            Duration::from_secs(60),
        );
        assert!(!slot.holds(), "nothing held before the first cleanup");
        let a = slot.get_or_start().unwrap();
        let b = slot.get_or_start().unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(slot.holds());
        assert!(!a.owned);
        assert_eq!(a.url.port(), Some(port));
        // Rest, lock, sleep and quit leave it running; CUE only lets go of it.
        slot.busy(false);
        slot.rest_soon();
        slot.rest();
        assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
        let c = slot.get_or_start().unwrap();
        assert!(!c.owned);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "CUE started a server while the user's one ran");
        // Once it is gone, CUE starts its own.
        drop(listener);
        assert_eq!(slot.get_or_start().unwrap_err(), AgentFailure::NotInstalled);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&state);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    async fn gone(s: &Server) {
        let start = std::time::Instant::now();
        while s.alive() {
            assert!(start.elapsed() < Duration::from_secs(5), "the server CUE started was not stopped");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn slot_stops_owned_server_on_rest_lock_sleep_quit() {
        let cwd = temp_dir("owned");
        let starts = Arc::new(AtomicUsize::new(0));
        let slot = OpenCodeSlot::with(Box::new(|| None), counting(&starts, cwd.clone()), Duration::from_millis(100));

        // The rest after a dictation (REST_AFTER, shortened here).
        let s1 = slot.get_or_start().unwrap();
        assert!(s1.owned && s1.alive());
        assert!(Arc::ptr_eq(&s1, &slot.get_or_start().unwrap()));
        slot.busy(true);
        slot.busy(false);
        gone(&s1).await;
        assert!(!slot.holds(), "after a rest the next request is the first again");

        // A lock or sleep between dictations (both come as the session lock): stopped now.
        let s2 = slot.get_or_start().unwrap();
        assert!(!Arc::ptr_eq(&s1, &s2) && s2.alive());
        slot.rest_soon();
        assert!(!s2.alive());

        // A lock during a dictation: stopped once it ends.
        slot.busy(true);
        let s3 = slot.get_or_start().unwrap();
        slot.rest_soon();
        assert!(s3.alive());
        slot.busy(false);
        assert!(!s3.alive());

        // A dictation starting stops the wait; quitting stops the server.
        let s4 = slot.get_or_start().unwrap();
        slot.busy(true);
        slot.busy(false);
        slot.busy(true);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(s4.alive());
        slot.rest();
        assert!(!s4.alive());
        assert_eq!(starts.load(Ordering::SeqCst), 4);
        let _ = std::fs::remove_dir_all(&cwd);
    }

    /// Whether process `pid` still runs (`tasklist` lists it).
    fn running(pid: u32) -> bool {
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
    }

    #[test]
    fn a_rest_during_a_start_stops_the_server_it_started() {
        let cwd = temp_dir("slow");
        let pid = Arc::new(Mutex::new(None));
        let (seen, dir) = (pid.clone(), cwd.clone());
        // A slow start: the server runs, then half a second passes before the slot gets it.
        let slot = OpenCodeSlot::with(
            Box::new(|| None),
            Box::new(move || {
                let s = owned(&dir);
                *seen.lock().unwrap() = s.pid();
                std::thread::sleep(Duration::from_millis(500));
                Ok(s)
            }),
            Duration::from_secs(60),
        );
        let starting = {
            let slot = slot.clone();
            std::thread::spawn(move || slot.get_or_start())
        };
        let start = std::time::Instant::now();
        while pid.lock().unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(10), "the start did not begin");
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = pid.lock().unwrap().unwrap();
        assert!(running(pid));
        // The session locks (no dictation under way) while the server starts.
        slot.rest_soon();
        assert_eq!(starting.join().unwrap().unwrap_err(), AgentFailure::Failed("rest"));
        assert!(!running(pid), "the server started during a rest runs on");
        // The next cleanup starts it again, and keeps it.
        let s = slot.get_or_start().unwrap();
        assert!(s.alive());
        slot.rest();
        assert!(!s.alive());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    /// The Cleanup page's look never starts a server: nothing running is none; the one in use is that one.
    #[test]
    fn running_never_starts_a_server() {
        let cwd = temp_dir("running");
        let starts = Arc::new(AtomicUsize::new(0));
        let slot = OpenCodeSlot::with(Box::new(|| None), counting(&starts, cwd.clone()), Duration::from_secs(60));
        assert!(slot.running().is_none());
        assert!(!slot.holds());
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        let s = slot.get_or_start().unwrap();
        assert!(Arc::ptr_eq(&slot.running().unwrap(), &s), "the server in use");
        s.halt();
        assert!(slot.running().is_none(), "a server that died is not started again");
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        slot.rest();
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn a_server_that_died_is_started_again() {
        let cwd = temp_dir("died");
        let starts = Arc::new(AtomicUsize::new(0));
        let slot = OpenCodeSlot::with(Box::new(|| None), counting(&starts, cwd.clone()), Duration::from_secs(60));
        let s1 = slot.get_or_start().unwrap();
        s1.halt();
        let s2 = slot.get_or_start().unwrap();
        assert!(!Arc::ptr_eq(&s1, &s2) && s2.alive());
        assert_eq!(starts.load(Ordering::SeqCst), 2);
        slot.rest();
        let _ = std::fs::remove_dir_all(&cwd);
    }
}
