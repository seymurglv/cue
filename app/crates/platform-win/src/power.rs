//! Sleep and session lock. A hidden top-level window on its own `cue-power` thread (broadcast
//! `WM_POWERBROADCAST` does not reach `HWND_MESSAGE` windows) catches resume, suspend, lock and unlock.
//! After `Resumed` and `Unlocked` the core reinstalls the keyboard hook (Windows may have removed it),
//! and on `Locked` and `Suspending` it ends the recording in progress.
use std::cell::RefCell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, RegisterClassW, SetTimer,
    TranslateMessage, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND, WINDOW_EX_STYLE, WM_POWERBROADCAST, WM_TIMER,
    WM_WTSSESSION_CHANGE, WNDCLASSW, WS_OVERLAPPED, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
};

/// What happened to power or the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerEvent {
    /// Resume from sleep or hibernation (`PBT_APMRESUMEAUTOMATIC`).
    Resumed,
    /// The session was unlocked (`WTS_SESSION_UNLOCK`).
    Unlocked,
    /// The session was locked (`WTS_SESSION_LOCK`), for example Win+L.
    Locked,
    /// The computer is going to sleep (`PBT_APMSUSPEND`).
    Suspending,
}

const CLASS: PCWSTR = w!("CuePowerWatch");

/// Right after signing in to Windows the Remote Desktop services may not be running yet, and
/// `WTSRegisterSessionNotification` answers `RPC_S_INVALID_BINDING`: we try again, this many times in total.
const SESSION_TRIES: u32 = 5;
/// Pause between attempts, ms.
const SESSION_RETRY_MS: u32 = 2_000;
/// Window timer id for the next attempt.
const SESSION_TIMER: usize = 1;

/// `spawn` has already been called: the thread and the window are one per process.
static STARTED: AtomicBool = AtomicBool::new(false);

/// The handler from `spawn`: lives on the `cue-power` thread, where the window procedure calls it.
type Handler = Box<dyn Fn(PowerEvent)>;

thread_local! {
    static ON_EVENT: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

/// The `cue-power` thread with a hidden window; `on_event` is called on it. On `Suspending` Windows does not wait long for
/// the window (about 2 s) — the handler must be fast. Call once: a second call is a warning in the log,
/// and nothing starts. If the window was not created — a warning, the thread ends, CUE works without these events.
pub fn spawn(on_event: impl Fn(PowerEvent) + Send + 'static) {
    if STARTED.swap(true, Ordering::SeqCst) {
        tracing::warn!("power: already running, the second start is skipped");
        return;
    }
    std::thread::Builder::new()
        .name("cue-power".into())
        .spawn(move || run(Box::new(on_event)))
        .expect("power thread");
}

fn run(on_event: Handler) {
    ON_EVENT.with(|f| *f.borrow_mut() = Some(on_event));
    let hwnd = match create_window() {
        Ok(hwnd) => hwnd,
        Err(e) => {
            tracing::warn!(hr = %e.code(), "power: the notification window was not created, sleep and lock are not tracked");
            return;
        }
    };
    let mut tries = 1;
    try_sessions(hwnd, tries);
    let mut msg = MSG::default();
    // SAFETY: an ordinary message loop of our own thread; 0 — WM_QUIT (nobody sends it to this thread), -1 — error.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // A new attempt — between messages: power events keep arriving as usual meanwhile.
        if msg.message == WM_TIMER && msg.hwnd == hwnd && msg.wParam.0 == SESSION_TIMER {
            // SAFETY: the timer was set by this thread on its own window; if an attempt is needed, `try_sessions` sets it again.
            let _ = unsafe { KillTimer(Some(hwnd), SESSION_TIMER) };
            tries += 1;
            try_sessions(hwnd, tries);
            continue;
        }
        // SAFETY: the message has just been received by this thread.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Attempt number `tries` to turn on session lock and unlock notifications. If it failed — after
/// `SESSION_RETRY_MS` a `WM_TIMER` comes for the next one; the last failure — one log entry.
fn try_sessions(hwnd: HWND, tries: u32) {
    // SAFETY: `hwnd` is a live window of this thread.
    let result = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) };
    // SAFETY: a timer of our own window without a callback: `WM_TIMER` arrives in this thread's queue.
    let next = retry_sessions(tries, result.is_ok())
        && unsafe { SetTimer(Some(hwnd), SESSION_TIMER, SESSION_RETRY_MS, None) } != 0;
    if let (Err(e), false) = (result, next) {
        tracing::warn!(hr = %e.code(), tries, "power: session lock and unlock are not tracked");
    }
}

/// After attempt number `tries` (from 1): whether to try again.
fn retry_sessions(tries: u32, registered: bool) -> bool {
    !registered && tries < SESSION_TRIES
}

/// A hidden top-level window: without `WS_VISIBLE` it is neither on the taskbar nor in Alt+Tab.
fn create_window() -> windows::core::Result<HWND> {
    // SAFETY: the module of our own exe; the window procedure is a static `extern "system"` function, the names are 'static strings.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: module.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread());
        }
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            w!("CUE"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        )
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(event) = power_event(msg, wparam.0) {
        ON_EVENT.with(|f| {
            if let Some(f) = f.borrow().as_ref() {
                // A panic cannot cross `extern "system"` — the whole process would go down.
                if catch_unwind(AssertUnwindSafe(|| f(event))).is_err() {
                    tracing::warn!(?event, "power: the event handler panicked");
                }
            }
        });
    }
    // SAFETY: the arguments are the ones Windows called the window procedure with.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Window message → event. Other codes give `None`, including `PBT_APMRESUMESUSPEND`: after a resume
/// triggered by a key press it comes right after `PBT_APMRESUMEAUTOMATIC`.
fn power_event(msg: u32, wparam: usize) -> Option<PowerEvent> {
    match (msg, u32::try_from(wparam).ok()?) {
        (WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC) => Some(PowerEvent::Resumed),
        (WM_POWERBROADCAST, PBT_APMSUSPEND) => Some(PowerEvent::Suspending),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK) => Some(PowerEvent::Unlocked),
        (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => Some(PowerEvent::Locked),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, GetWindowThreadProcessId, SendMessageTimeoutW, PBT_APMRESUMESUSPEND, PBT_POWERSETTINGCHANGE,
        SMTO_NORMAL, WTS_SESSION_LOGON,
    };

    #[test]
    fn resume_from_sleep() {
        assert_eq!(power_event(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC as usize), Some(PowerEvent::Resumed));
    }

    #[test]
    fn going_to_sleep() {
        assert_eq!(power_event(WM_POWERBROADCAST, PBT_APMSUSPEND as usize), Some(PowerEvent::Suspending));
    }

    #[test]
    fn session_unlocked() {
        assert_eq!(power_event(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK as usize), Some(PowerEvent::Unlocked));
    }

    #[test]
    fn session_locked() {
        assert_eq!(power_event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize), Some(PowerEvent::Locked));
    }

    /// A resume triggered by a key press comes as a second message after `PBT_APMRESUMEAUTOMATIC` — there is no second `Resumed`.
    #[test]
    fn other_power_and_session_codes_are_ignored() {
        assert_eq!(power_event(WM_POWERBROADCAST, PBT_APMRESUMESUSPEND as usize), None);
        assert_eq!(power_event(WM_POWERBROADCAST, PBT_POWERSETTINGCHANGE as usize), None);
        assert_eq!(power_event(WM_WTSSESSION_CHANGE, WTS_SESSION_LOGON as usize), None);
    }

    /// Sleep and session codes mean nothing in a different message.
    #[test]
    fn same_codes_in_other_messages_are_ignored() {
        assert_eq!(power_event(WM_TIMER, PBT_APMRESUMEAUTOMATIC as usize), None);
        assert_eq!(power_event(WM_TIMER, WTS_SESSION_UNLOCK as usize), None);
        assert_eq!(power_event(WM_WTSSESSION_CHANGE, PBT_APMRESUMEAUTOMATIC as usize), None);
        assert_eq!(power_event(WM_POWERBROADCAST, WTS_SESSION_LOCK as usize), None);
    }

    /// The high bits of `wparam` are not dropped: this is not `PBT_APMRESUMEAUTOMATIC`.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn high_bits_of_wparam_are_not_truncated() {
        let wparam = (1usize << 32) | PBT_APMRESUMEAUTOMATIC as usize;
        assert_eq!(power_event(WM_POWERBROADCAST, wparam), None);
    }

    /// Session notifications are on — no more attempts.
    #[test]
    fn session_registration_stops_after_success() {
        assert!(!retry_sessions(1, true));
        assert!(!retry_sessions(SESSION_TRIES - 1, true));
    }

    /// Not on — try again, exactly `SESSION_TRIES` times in total.
    #[test]
    fn session_registration_gives_up_after_last_try() {
        assert!(retry_sessions(1, false));
        assert_eq!((1..).find(|&tries| !retry_sessions(tries, false)), Some(SESSION_TRIES));
    }

    /// The `CuePowerWatch` window of this process (a running probe may have one too).
    fn own_window() -> Option<HWND> {
        let mut after = None;
        // SAFETY: only searches top-level windows by class; the class name is a 'static string.
        while let Ok(hwnd) = unsafe { FindWindowExW(None, after, CLASS, PCWSTR::null()) } {
            let mut pid = 0;
            // SAFETY: only reads the window's process id; `pid` lives until the end of the call.
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pid == std::process::id() {
                return Some(hwnd);
            }
            after = Some(hwnd);
        }
        None
    }

    /// Log entries made on this thread while `f` runs.
    fn logged(f: impl FnOnce()) -> String {
        #[derive(Clone, Default)]
        struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Capture {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt().with_ansi(false).with_writer(move || writer.clone()).finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = capture.0.lock().unwrap().clone();
        String::from_utf8(bytes).unwrap()
    }

    /// The window gets the same messages Windows sends; the computer itself does not sleep or lock.
    /// The only test with `spawn`: the thread and the window are one per process. It also checks the guard against a second start:
    /// the second `spawn` writes "already running" to the calling thread's log (without the guard it would start a new thread,
    /// and this log would have nothing), and the events still go to the first handler.
    #[test]
    fn window_passes_events_to_handler() {
        let (tx, rx) = std::sync::mpsc::channel();
        spawn(move |event| {
            let _ = tx.send(event);
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let hwnd = loop {
            if let Some(hwnd) = own_window() {
                break hwnd;
            }
            assert!(std::time::Instant::now() < deadline, "the cue-power window did not appear");
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        let log = logged(|| spawn(|_| {}));
        assert!(log.contains("already running"), "no warning about the second start, log: {log:?}");
        let send = |msg: u32, code: u32| {
            // SAFETY: a window of this process; the parameters contain no pointers. The call waits until the window thread
            // handles the message — by then the event is already in the channel; a stuck thread fails the test instead of hanging.
            let sent = unsafe {
                SendMessageTimeoutW(hwnd, msg, WPARAM(code as usize), LPARAM(0), SMTO_NORMAL, 5_000, None)
            };
            assert_ne!(sent.0, 0, "the cue-power thread did not answer within 5 s");
        };
        send(WM_POWERBROADCAST, PBT_APMSUSPEND);
        send(WM_POWERBROADCAST, PBT_APMRESUMEAUTOMATIC);
        send(WM_POWERBROADCAST, PBT_APMRESUMESUSPEND);
        send(WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK);
        send(WM_WTSSESSION_CHANGE, WTS_SESSION_UNLOCK);
        let got: Vec<PowerEvent> = rx.try_iter().collect();
        assert_eq!(got, [PowerEvent::Suspending, PowerEvent::Resumed, PowerEvent::Locked, PowerEvent::Unlocked]);
    }
}
