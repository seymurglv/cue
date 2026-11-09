//! One run of an agent's CLI: no window, the text on stdin (never an argument or a file), an empty working folder,
//! and a timeout that ends the whole process tree. An npm shim (`codex.cmd`) starts `cmd.exe`, which starts `node`,
//! which may start more: on Windows every one of them lives in a Job object that is killed when the run ends, by an
//! answer or by the timeout, so nothing the agent started keeps running.
use std::io::{Read, Write};
use std::path::Path;
#[cfg(not(windows))]
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::AgentFailure;

/// How much of stderr a run keeps: its end, where CLIs say why they failed.
pub const STDERR_TAIL: usize = 2048;
/// The most of stdout a run keeps; an answer is far shorter.
const STDOUT_MAX: usize = 4 << 20;

/// What an agent's run printed. Its `Debug` shows the exit code and the lengths only: what was printed may hold the
/// dictated text.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Output {
    /// The exit code; `None` when the process ended without one.
    pub status: Option<i32>,
    pub stdout: String,
    /// The last [`STDERR_TAIL`] bytes of stderr, for classifying a failure only; never logged (it may quote the
    /// text).
    pub stderr_tail: String,
}

impl std::fmt::Debug for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("status", &self.status)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_tail_len", &self.stderr_tail.len())
            .finish()
    }
}

/// Runs `exe` with `args` in `cwd`, writes `stdin` to it and closes it, and waits at most `timeout`.
///
/// `cwd` is created when missing and must be empty: a file there (an `AGENTS.md` with orders, say) would reach the
/// agent, so a folder with anything in it is refused as `Failed("cwd")`. A missing program is `NotInstalled`; past
/// the timeout the whole tree is killed and the run is `Timeout`. `.cmd` shims run through `cmd.exe` (Rust's
/// `Command` quotes their arguments, and refuses ones it cannot quote safely).
pub fn run(exe: &Path, args: &[String], stdin: &str, cwd: &Path, timeout: Duration) -> Result<Output, AgentFailure> {
    run_until(exe, args, stdin, cwd, timeout, &AtomicBool::new(false))
}

/// How often a run looks at its cancel flag.
const CANCEL_EVERY: Duration = Duration::from_millis(50);

/// [`run`] that also ends, with its whole process tree, as soon as `cancel` is set (the request was dropped: Esc, or
/// a newer one): `Failed("cancelled")`. The flag is looked at every [`CANCEL_EVERY`], so the agent never runs on,
/// spending the user's quota, for a text nobody waits for.
pub fn run_until(
    exe: &Path,
    args: &[String],
    stdin: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<Output, AgentFailure> {
    ready_cwd(cwd)?;
    let mut cmd = Command::new(exe);
    cmd.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let (mut child, tree) = start(cmd)?;

    let writer = child.stdin.take().map(|mut pipe| {
        let bytes = stdin.as_bytes().to_vec();
        std::thread::spawn(move || {
            // A child that exits without reading closes the pipe: that is not an error of the run.
            let _ = pipe.write_all(&bytes);
        })
    });
    let out = child.stdout.take().map(|pipe| std::thread::spawn(move || read_capped(pipe)));
    let err = child.stderr.take().map(|pipe| std::thread::spawn(move || read_tail(pipe)));

    let deadline = Instant::now() + timeout;
    let mut exited = false;
    let mut cancelled = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if tree.wait(&mut child, left.min(CANCEL_EVERY)) {
            exited = true;
            break;
        }
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        if left <= CANCEL_EVERY {
            break;
        }
    }
    // Whatever the agent left running ends with it; then the pipes close and the readers finish.
    tree.kill();
    if !exited {
        // Elsewhere than Windows the tree is the child alone.
        let _ = child.kill();
    }
    let status = child.wait().ok().and_then(|s| s.code());
    let stdout = join(out);
    let stderr = join(err);
    if let Some(w) = writer {
        let _ = w.join();
    }
    if cancelled {
        return Err(AgentFailure::Failed("cancelled"));
    }
    if !exited {
        return Err(AgentFailure::Timeout);
    }
    Ok(Output { status, stdout: String::from_utf8_lossy(&stdout).into_owned(), stderr_tail: tail_text(&stderr) })
}

/// A long-lived agent process (OpenCode's server): no window, an empty working folder, no stdin and its output
/// dropped. [`Running::stop`], or dropping it, ends its whole process tree; so does CUE's own end, even a crash (the
/// Job object is closed with CUE's handles).
pub struct Running {
    child: std::process::Child,
    tree: Tree,
}

/// Starts `exe` with `args` and the extra environment `env` in `cwd` (created when missing; refused, as in [`run`],
/// when anything is in it) and leaves it running.
pub fn spawn(exe: &Path, args: &[String], env: &[(&str, &str)], cwd: &Path) -> Result<Running, AgentFailure> {
    ready_cwd(cwd)?;
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .envs(env.iter().copied())
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let (child, tree) = start(cmd)?;
    Ok(Running { child, tree })
}

impl Running {
    /// The process CUE started (for an npm shim, `cmd.exe`).
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Whether the process CUE started is still running.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ends the whole tree and waits (up to 2 s) for it to be gone (what dropping it does).
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.tree.kill();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `cmd` with no window, in a new Job object (Windows), suspended until it is in the job.
fn start(mut cmd: Command) -> Result<(std::process::Child, Tree), AgentFailure> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        // Suspended until it is in the job, so not even its first child can start outside it.
        cmd.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(AgentFailure::NotInstalled),
        Err(_) => return Err(AgentFailure::Failed("start")),
    };
    match Tree::adopt(&child) {
        Ok(tree) => Ok((child, tree)),
        Err(()) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(AgentFailure::Failed("job"))
        }
    }
}

/// Creates `cwd` when missing; refuses it when anything is in it.
fn ready_cwd(cwd: &Path) -> Result<(), AgentFailure> {
    std::fs::create_dir_all(cwd).map_err(|_| AgentFailure::Failed("cwd"))?;
    let mut entries = std::fs::read_dir(cwd).map_err(|_| AgentFailure::Failed("cwd"))?;
    match entries.next() {
        None => Ok(()),
        Some(_) => Err(AgentFailure::Failed("cwd")),
    }
}

fn join(h: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    h.and_then(|h| h.join().ok()).unwrap_or_default()
}

/// Reads to the end, keeping the first [`STDOUT_MAX`] bytes (the rest is read and dropped, so the child never blocks).
fn read_capped(mut pipe: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => return kept,
            Ok(n) => {
                let room = STDOUT_MAX.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
            }
        }
    }
}

/// Reads to the end, keeping the last [`STDERR_TAIL`] bytes.
fn read_tail(mut pipe: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) | Err(_) => return kept,
            Ok(n) => {
                kept.extend_from_slice(&buf[..n]);
                if kept.len() > 2 * STDERR_TAIL {
                    kept.drain(..kept.len() - STDERR_TAIL);
                }
            }
        }
    }
}

/// The last [`STDERR_TAIL`] bytes as text, cut at a character.
fn tail_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(STDERR_TAIL)..]).into_owned();
    let mut cut = text.len().saturating_sub(STDERR_TAIL);
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    text[cut..].to_string()
}

#[cfg(windows)]
use win::Tree;

#[cfg(windows)]
mod win {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_BASIC_PROCESS_ID_LIST, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation, QueryInformationJobObject,
        SetInformationJobObject, TerminateJobObject,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, OpenThread, PROCESS_SYNCHRONIZE, ResumeThread, THREAD_SUSPEND_RESUME, WaitForSingleObject,
    };
    use windows::core::PCWSTR;

    /// A Job object holding an agent's process and everything it starts. Closing it kills them all
    /// (`KILL_ON_JOB_CLOSE`), so even a panic on CUE's side leaves nothing running.
    pub(super) struct Tree(HANDLE);

    // SAFETY: a job handle may be used and closed from any thread.
    unsafe impl Send for Tree {}

    impl Tree {
        /// Puts the suspended `child` in a new job and lets it run.
        pub(super) fn adopt(child: &Child) -> Result<Tree, ()> {
            // SAFETY: plain Win32 calls on handles this function owns (the job) or borrows from `child`, which
            // outlives them; the limit structure lives on the stack for the duration of the call.
            unsafe {
                let job = CreateJobObjectW(None, PCWSTR::null()).map_err(|_| ())?;
                let tree = Tree(job);
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                // No crash dialog either: a crashing agent ends instead of opening a window.
                limits.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    (&raw const limits).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
                .map_err(|_| ())?;
                AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).map_err(|_| ())?;
                resume(child.id())?;
                Ok(tree)
            }
        }

        /// Waits for `child` to exit, at most `timeout`; whether it did.
        pub(super) fn wait(&self, child: &mut Child, timeout: Duration) -> bool {
            let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
            // SAFETY: the process handle is valid while `child` lives.
            unsafe { WaitForSingleObject(HANDLE(child.as_raw_handle()), ms) == WAIT_OBJECT_0 }
        }

        /// Kills every process in the job and waits (up to 2 s) until each has ended: the job's own count drops
        /// before a killed process is gone, so CUE waits on the processes themselves, and terminates again for one
        /// that was being started while the job was killed.
        pub(super) fn kill(&self) {
            let start = Instant::now();
            loop {
                let pids = self.pids();
                if pids.is_empty() || start.elapsed() > KILL_WAIT {
                    return;
                }
                // SAFETY: the job handle is valid until drop; each process handle opened here is closed here.
                unsafe {
                    let procs: Vec<HANDLE> =
                        pids.iter().filter_map(|&pid| OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok()).collect();
                    let _ = TerminateJobObject(self.0, 1);
                    for p in procs {
                        let left = KILL_WAIT.saturating_sub(start.elapsed());
                        WaitForSingleObject(p, u32::try_from(left.as_millis()).unwrap_or(0));
                        let _ = CloseHandle(p);
                    }
                }
            }
        }

        /// The processes in the job now (at most [`MAX_PIDS`]).
        fn pids(&self) -> Vec<u32> {
            let mut buf = vec![0usize; 2 + MAX_PIDS];
            // SAFETY: `buf` is a JOBOBJECT_BASIC_PROCESS_ID_LIST with room for MAX_PIDS ids (two u32 counts take
            // the first word); the job handle is valid until drop.
            unsafe {
                let list = buf.as_mut_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
                let ok = QueryInformationJobObject(
                    Some(self.0),
                    JobObjectBasicProcessIdList,
                    list.cast(),
                    (buf.len() * size_of::<usize>()) as u32,
                    None,
                );
                // With more than MAX_PIDS the call reports more data but fills the list it has room for.
                let n = (*list).NumberOfProcessIdsInList as usize;
                if ok.is_err() && n == 0 {
                    return Vec::new();
                }
                let ids = std::ptr::addr_of!((*list).ProcessIdList).cast::<usize>();
                (0..n.min(MAX_PIDS)).map(|i| *ids.add(i) as u32).collect()
            }
        }
    }

    /// How long [`Tree::kill`] waits for the killed processes to end.
    const KILL_WAIT: Duration = Duration::from_secs(2);
    /// The most processes of a job [`Tree::pids`] lists; an agent starts a handful.
    const MAX_PIDS: usize = 256;

    impl Drop for Tree {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed once; closing kills whatever is still in the job.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Resumes the threads of the suspended process `pid` (a process created suspended has one).
    ///
    /// SAFETY: plain Win32 calls; each handle opened here is closed here.
    unsafe fn resume(pid: u32) -> Result<(), ()> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(|_| ())?;
            let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
            let mut resumed = 0;
            let mut more = Thread32First(snap, &mut entry).is_ok();
            while more {
                if entry.th32OwnerProcessID == pid
                    && let Ok(thread) = OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID)
                {
                    if ResumeThread(thread) != u32::MAX {
                        resumed += 1;
                    }
                    let _ = CloseHandle(thread);
                }
                more = Thread32Next(snap, &mut entry).is_ok();
            }
            let _ = CloseHandle(snap);
            if resumed > 0 { Ok(()) } else { Err(()) }
        }
    }
}

/// Elsewhere (CUE is a Windows app; this keeps the crate building): the child alone is killed at the timeout or a
/// cancel.
#[cfg(not(windows))]
struct Tree;

#[cfg(not(windows))]
impl Tree {
    fn adopt(_child: &Child) -> Result<Tree, ()> {
        Ok(Tree)
    }

    fn wait(&self, child: &mut Child, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if let Ok(Some(_)) = child.try_wait() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn kill(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_within_2_kb_at_a_character() {
        let text = format!("{}конец", "я".repeat(3000));
        let tail = tail_text(text.as_bytes());
        assert!(tail.len() <= STDERR_TAIL);
        assert!(tail.ends_with("конец"));
        assert_eq!(tail_text(b"short"), "short");
    }

    #[test]
    fn readers_cap_and_tail() {
        let big = vec![b'a'; STDOUT_MAX + 10];
        assert_eq!(read_capped(&big[..]).len(), STDOUT_MAX);
        let mut err = vec![b'x'; 10_000];
        err.extend_from_slice(b"END");
        let kept = read_tail(&err[..]);
        assert!(kept.ends_with(b"END") && kept.len() >= STDERR_TAIL);
    }

    /// A run's output may quote the dictation: its `Debug` holds the exit code and lengths, never the text.
    #[test]
    fn debug_shows_no_text() {
        let out = Output { status: Some(1), stdout: "созвон в десять".into(), stderr_tail: "secret line".into() };
        let shown = format!("{out:?} {out:#?}");
        assert!(!shown.contains("созвон") && !shown.contains("secret"), "{shown}");
        assert!(shown.contains("stdout_len: 28") && shown.contains("Some(1)"), "{shown}");
    }
}
