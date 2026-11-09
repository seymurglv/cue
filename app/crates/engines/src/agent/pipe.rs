//! A one-shot named pipe for Grok (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): grok 1.0.34's headless mode
//! reads the prompt only from an argument or a file, never from stdin, so CUE hands it the text through
//! `--prompt-file \\.\pipe\cue-grok-<random>`: a pipe CUE creates before Grok starts, that only the current user can
//! open (no other account, no network logon, no remote client), with room for one client; CUE writes the text once and
//! closes it. The text is never an argument and never a file on disk.
use std::fmt::Write as _;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::AgentFailure;

/// Every pipe CUE makes for Grok is named this and 32 random hex digits.
pub const PREFIX: &str = r"\\.\pipe\cue-grok-";

/// A pipe waiting for its one client. Dropped unsent, it is closed and gone.
pub struct OneShot {
    name: String,
    #[cfg(windows)]
    pipe: win::Pipe,
}

impl OneShot {
    /// `\\.\pipe\cue-grok-<128 random bits in hex>`, outbound only, one instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`: a
    /// pipe of that name made by anyone else first is an error, never joined), remote clients refused, and a
    /// descriptor that lets the current user alone open it (network logons denied).
    pub fn create() -> Result<OneShot, AgentFailure> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| AgentFailure::Failed("random"))?;
        let name = bytes.iter().fold(String::from(PREFIX), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        });
        #[cfg(windows)]
        {
            let pipe = win::Pipe::create(&name)?;
            Ok(OneShot { name, pipe })
        }
        #[cfg(not(windows))]
        {
            let _ = name;
            Err(AgentFailure::Failed("pipe"))
        }
    }

    /// The pipe's path, for `--prompt-file`.
    pub fn path(&self) -> String {
        self.name.clone()
    }

    /// Waits for the one client to connect, at most `timeout`, writes `text` (UTF-8) once and closes the pipe: the
    /// client reads what is left, then the end. No client within `timeout` is `Timeout`.
    pub fn send(self, text: &str, timeout: Duration) -> Result<(), AgentFailure> {
        self.send_until(text, timeout, &AtomicBool::new(false))
    }

    /// [`send`](Self::send) that also gives up, as `Failed("stopped")`, as soon as `stop` is set (the client's
    /// process ended without reading, or the request was dropped). `stop` is looked at every 50 ms.
    pub fn send_until(self, text: &str, timeout: Duration, stop: &AtomicBool) -> Result<(), AgentFailure> {
        let deadline = Instant::now() + timeout;
        #[cfg(windows)]
        {
            self.pipe.connect(deadline, stop)?;
            self.pipe.write_all(text.as_bytes(), deadline, stop)
        }
        #[cfg(not(windows))]
        {
            let _ = (text, deadline, stop);
            Err(AgentFailure::Failed("pipe"))
        }
    }

    /// The pipe's DACL as SDDL, as Windows holds it.
    #[cfg(all(test, windows))]
    fn dacl_sddl(&self) -> String {
        self.pipe.dacl_sddl()
    }
}

#[cfg(windows)]
mod win {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{
        CloseHandle, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
        WAIT_OBJECT_0,
    };
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_OUTBOUND, WriteFile,
    };
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, OpenProcessToken, ResetEvent, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::AgentFailure;

    /// How often a wait looks at its stop flag.
    const STEP: Duration = Duration::from_millis(50);
    /// The pipe's buffer, and the most one write hands it.
    const CHUNK: usize = 64 * 1024;

    /// The server end of the pipe and the event its overlapped calls signal; both closed on drop.
    pub(super) struct Pipe {
        handle: HANDLE,
        event: HANDLE,
    }

    // SAFETY: the handles may be used and closed from any thread; the pipe is used by one thread at a time (`send`
    // takes it by value).
    unsafe impl Send for Pipe {}

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn failed<T>(_: T) -> AgentFailure {
        AgentFailure::Failed("pipe")
    }

    /// The SID of the user CUE runs as, as a string (`S-1-5-21-…`).
    pub(super) fn current_user_sid() -> Result<String, AgentFailure> {
        // SAFETY: plain Win32 calls; the token handle and the SID string are owned here and released here, and the
        // buffer (u64-aligned, as TOKEN_USER needs) outlives every read of it.
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(failed)?;
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let got = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
            let _ = CloseHandle(token);
            got.map_err(failed)?;
            let user = &*buf.as_ptr().cast::<TOKEN_USER>();
            let mut text = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut text).map_err(failed)?;
            let sid = text.to_string().map_err(failed);
            let _ = LocalFree(Some(HLOCAL(text.0.cast())));
            sid
        }
    }

    impl Pipe {
        pub(super) fn create(name: &str) -> Result<Pipe, AgentFailure> {
            // A protected DACL (nothing inherited): network logons denied, then full access for the current user and
            // no one else; everyone else is denied by having no entry.
            let sddl = format!("D:P(D;;GA;;;NU)(A;;GA;;;{})", current_user_sid()?);
            let (sddl, name) = (wide(&sddl), wide(name));
            // SAFETY: the strings outlive the calls; the descriptor is freed here once the pipe holds its copy; the
            // event and pipe handles are owned by the returned `Pipe` (or closed here on failure).
            unsafe {
                let mut sd = PSECURITY_DESCRIPTOR::default();
                ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut sd, None)
                    .map_err(failed)?;
                let attrs = SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: sd.0,
                    bInheritHandle: false.into(),
                };
                let handle = CreateNamedPipeW(
                    PCWSTR(name.as_ptr()),
                    PIPE_ACCESS_OUTBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    CHUNK as u32,
                    0,
                    0,
                    Some(&attrs),
                );
                let _ = LocalFree(Some(HLOCAL(sd.0)));
                if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
                    return Err(AgentFailure::Failed("pipe"));
                }
                match CreateEventW(None, true, false, PCWSTR::null()) {
                    Ok(event) => Ok(Pipe { handle, event }),
                    Err(_) => {
                        let _ = CloseHandle(handle);
                        Err(AgentFailure::Failed("pipe"))
                    }
                }
            }
        }

        /// Waits for the client to connect (one that connected already counts).
        pub(super) fn connect(&self, deadline: Instant, stop: &AtomicBool) -> Result<(), AgentFailure> {
            let mut ov = OVERLAPPED { hEvent: self.event, ..Default::default() };
            // SAFETY: `ov` lives until the call is complete: `wait` returns only once it completed or was cancelled
            // and finished.
            unsafe {
                let _ = ResetEvent(self.event);
                match ConnectNamedPipe(self.handle, Some(&mut ov)) {
                    Ok(()) => Ok(()),
                    Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(()),
                    Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => self.wait(&mut ov, deadline, stop).map(|_| ()),
                    Err(_) => Err(AgentFailure::Failed("pipe")),
                }
            }
        }

        /// Writes all of `bytes`, a chunk at a time; a client that goes away first is `Failed("pipe")`.
        pub(super) fn write_all(&self, bytes: &[u8], deadline: Instant, stop: &AtomicBool) -> Result<(), AgentFailure> {
            let mut done = 0;
            while done < bytes.len() {
                let chunk = &bytes[done..bytes.len().min(done + CHUNK)];
                let mut ov = OVERLAPPED { hEvent: self.event, ..Default::default() };
                // SAFETY: `chunk` and `ov` live until the write is complete (`wait` returns only then).
                let n = unsafe {
                    let _ = ResetEvent(self.event);
                    match WriteFile(self.handle, Some(chunk), None, Some(&mut ov)) {
                        Ok(()) => {
                            let mut n = 0u32;
                            GetOverlappedResult(self.handle, &ov, &mut n, true).map_err(failed)?;
                            n
                        }
                        Err(e) if e.code() == ERROR_IO_PENDING.to_hresult() => self.wait(&mut ov, deadline, stop)?,
                        Err(_) => return Err(AgentFailure::Failed("pipe")),
                    }
                };
                if n == 0 {
                    return Err(AgentFailure::Failed("pipe"));
                }
                done += n as usize;
            }
            Ok(())
        }

        /// Waits for the overlapped call `ov` to complete: its byte count; past `deadline` (`Timeout`) or once `stop`
        /// is set (`Failed("stopped")`) the call is cancelled and waited for, so `ov` is free when this returns.
        ///
        /// SAFETY: `ov` must be the pending call's own structure, on this pipe.
        unsafe fn wait(&self, ov: &mut OVERLAPPED, deadline: Instant, stop: &AtomicBool) -> Result<u32, AgentFailure> {
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let ms = u32::try_from(left.min(STEP).as_millis()).unwrap_or(0);
                // SAFETY: the event handle is valid while `self` lives; `ov` is the pending call's (the caller's
                // promise).
                unsafe {
                    if WaitForSingleObject(self.event, ms) == WAIT_OBJECT_0 {
                        let mut n = 0u32;
                        return GetOverlappedResult(self.handle, ov, &mut n, false).map(|()| n).map_err(failed);
                    }
                    let why = if stop.load(Ordering::Relaxed) {
                        AgentFailure::Failed("stopped")
                    } else if left.is_zero() {
                        AgentFailure::Timeout
                    } else {
                        continue;
                    };
                    let _ = CancelIoEx(self.handle, Some(ov));
                    let mut n = 0u32;
                    // Waits for the cancelled call to finish (it may have completed meanwhile: then it is done).
                    let _ = GetOverlappedResult(self.handle, ov, &mut n, true);
                    return Err(why);
                }
            }
        }

        /// The DACL Windows holds for the pipe, as SDDL.
        #[cfg(test)]
        pub(super) fn dacl_sddl(&self) -> String {
            use windows::Win32::Security::Authorization::{
                ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
            };
            use windows::Win32::Security::DACL_SECURITY_INFORMATION;
            // SAFETY: plain Win32 calls; the descriptor and the string are freed here.
            unsafe {
                let mut sd = PSECURITY_DESCRIPTOR::default();
                let r = GetSecurityInfo(
                    self.handle,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    None,
                    None,
                    Some(&mut sd),
                );
                assert!(r.is_ok(), "GetSecurityInfo: {r:?}");
                let mut text = PWSTR::null();
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    None,
                )
                .unwrap();
                let sddl = text.to_string().unwrap();
                let _ = LocalFree(Some(HLOCAL(text.0.cast())));
                let _ = LocalFree(Some(HLOCAL(sd.0)));
                sddl
            }
        }
    }

    impl Drop for Pipe {
        fn drop(&mut self) {
            // SAFETY: both handles are owned and closed once. Closing the server end without disconnecting leaves
            // what the client has not read yet for it to read, then the end.
            unsafe {
                let _ = CloseHandle(self.handle);
                let _ = CloseHandle(self.event);
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::io::Read;
    use std::time::Instant;

    use super::*;

    /// The pipe's client, as Grok opens `--prompt-file`: a file opened for reading, read to the end.
    fn read_all(path: &str) -> std::thread::JoinHandle<String> {
        let path = path.to_string();
        std::thread::spawn(move || {
            let mut f = std::fs::File::open(&path).expect("the pipe opens for its user");
            let mut s = String::new();
            f.read_to_string(&mut s).expect("read to the end");
            s
        })
    }

    #[test]
    fn pipe_name_is_random_and_local() {
        let (a, b) = (OneShot::create().unwrap(), OneShot::create().unwrap());
        assert_ne!(a.path(), b.path());
        for p in [a.path(), b.path()] {
            // `\\.\pipe\`: this computer's pipes, never a server's.
            let random = p.strip_prefix(PREFIX).unwrap_or_else(|| panic!("{p}"));
            assert_eq!(random.len(), 32, "{p}");
            assert!(random.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{p}");
        }
        assert!(PREFIX.starts_with(r"\\.\pipe\"));
    }

    /// The descriptor CUE gives the pipe: one allow entry, for the current user's SID alone (no Everyone, no users'
    /// group, no administrators, no SYSTEM), a deny of network logons before it, and no inherited entry.
    #[test]
    fn pipe_denies_other_users() {
        let p = OneShot::create().unwrap();
        let sddl = p.dacl_sddl();
        let user = win::current_user_sid().unwrap();
        assert!(user.starts_with("S-1-5-"), "{user}");
        let dacl = sddl.strip_prefix("D:P").unwrap_or_else(|| panic!("protected, nothing inherited: {sddl}"));
        let aces: Vec<&str> = dacl.trim_start_matches('(').trim_end_matches(')').split(")(").collect();
        let allows: Vec<&&str> = aces.iter().filter(|a| a.starts_with("A;")).collect();
        assert_eq!(allows.len(), 1, "{sddl}");
        assert!(allows[0].ends_with(&format!(";;;{user}")), "{sddl}");
        assert!(aces.iter().any(|a| a.starts_with("D;") && a.ends_with(";;;NU")), "network logons denied: {sddl}");
        for other in ["WD", "AU", "BU", "BA", "SY", "AN", "IU"] {
            assert!(!aces.iter().any(|a| a.starts_with("A;") && a.ends_with(&format!(";;;{other}"))), "{other}: {sddl}");
        }
    }

    #[test]
    fn pipe_send_times_out_without_a_client() {
        let p = OneShot::create().unwrap();
        let start = Instant::now();
        assert_eq!(p.send("текст", Duration::from_millis(300)), Err(AgentFailure::Timeout));
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
    }

    /// A stop (Grok exited without opening the pipe, or the request was dropped) ends the wait at once.
    #[test]
    fn pipe_send_stops_when_told() {
        let p = OneShot::create().unwrap();
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let start = Instant::now();
        assert_eq!(p.send_until("текст", Duration::from_secs(30), &stop), Err(AgentFailure::Failed("stopped")));
        assert!(start.elapsed() < Duration::from_secs(3), "{:?}", start.elapsed());
    }

    /// The client gets every byte, in UTF-8, then the end; also a text far larger than the pipe's buffer, and also
    /// when the client opens the pipe before CUE waits for it.
    #[test]
    fn pipe_delivers_the_whole_text_once() {
        let text = format!("<transcript>{}</transcript>", "ну эм завтра в десять созвон ".repeat(20_000));
        let p = OneShot::create().unwrap();
        let client = read_all(&p.path());
        std::thread::sleep(Duration::from_millis(100));
        p.send(&text, Duration::from_secs(10)).unwrap();
        assert_eq!(client.join().unwrap(), text);

        let p = OneShot::create().unwrap();
        let path = p.path();
        let sender = std::thread::spawn(move || p.send("Привет.", Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(read_all(&path).join().unwrap(), "Привет.");
        sender.join().unwrap().unwrap();
        // One shot: once sent, the pipe is gone.
        assert!(std::fs::File::open(&path).is_err());
    }

    /// One client only: a second one cannot open the pipe while the first holds it.
    #[test]
    fn pipe_takes_one_client() {
        let p = OneShot::create().unwrap();
        let path = p.path();
        let first = std::fs::File::open(&path).unwrap();
        assert!(std::fs::File::open(&path).is_err(), "a second client opened the pipe");
        drop(first);
        drop(p);
    }
}
