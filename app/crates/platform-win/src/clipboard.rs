//! Paste through the clipboard: wait for the modifiers to be released → remember the clipboard → put the text
//! marked "not for history" → Ctrl+V → after 0.5 s restore the clipboard if nobody changed it. And the opposite way for
//! "Ask": the selection of an app that does not report it, copied with Ctrl+C and the clipboard put back.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Clipboard contents: (format, bytes) pairs in the order they are put back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub items: Vec<(u32, Vec<u8>)>,
}

pub trait ClipboardApi {
    /// Clipboard sequence number: changes on every write to the clipboard, by anyone.
    fn sequence(&self) -> u32;
    fn snapshot(&mut self) -> Result<Snapshot, String>;
    /// `private` — with marks: not in the Win+V history and not in the cloud clipboard.
    fn write_text(&mut self, text: &str, private: bool) -> Result<(), String>;
    /// With the same marks: the restored content is not a new copy; if it was allowed into the history, it is already there.
    /// An empty snapshot is restored as an empty clipboard, without marks.
    fn restore(&mut self, s: &Snapshot) -> Result<(), String>;
    /// CUE's clipboard operations in this process (one count and one lock for every `WinClipboard`): a paste and a
    /// Ctrl+C copy each begin one, so that an older late-copy guard stands down (`guard_late_copy`).
    fn operations(&self) -> &'static Operations;
}

/// CUE's clipboard operations in this process — pastes and Ctrl+C copies: how many have begun, and one lock. A late-copy
/// guard acts under the lock, and only while its copy is still the newest operation; `begin` takes the same lock. So a
/// newer operation begins either before the guard acts, and the guard stands down, or after it is done — a few ms at
/// most: the guard never waits for the clipboard while it holds the lock (`ReadClipboard::take_back`).
#[derive(Default)]
pub struct Operations {
    count: AtomicU64,
    lock: Mutex<()>,
}

impl Operations {
    pub const fn new() -> Self {
        Self { count: AtomicU64::new(0), lock: Mutex::new(()) }
    }

    /// A newer operation begins (after a guard that is acting now is done); its number.
    pub fn begin(&self) -> u64 {
        let _acting = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.count.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The number of the newest operation.
    pub fn newest(&self) -> u64 {
        self.count.load(Ordering::SeqCst)
    }

    /// `act`, only while `operation` is the newest one, and no newer one begins until it is done. `None` — a newer one has
    /// begun: `act` did not run.
    pub fn while_newest<R>(&self, operation: u64, act: impl FnOnce() -> R) -> Option<R> {
        let _acting = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        (self.newest() == operation).then(act)
    }
}

pub trait KeysApi {
    fn wait_modifiers_released(&mut self, timeout: Duration) -> bool;
    /// `false` — the system did not accept all presses (for example, the window runs as administrator).
    fn press_ctrl_v(&mut self) -> bool;
    fn sleep(&mut self, d: Duration);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteResult {
    /// `restored` — the previous clipboard is restored. If not, someone else changed it, it could not be read,
    /// or it could not be restored.
    Pasted { restored: bool },
    /// No paste happened, the text is left in the clipboard the ordinary way (without marks): the modifiers were not
    /// released within 5 s or Ctrl+V did not reach the window.
    LeftInClipboard,
}

/// `on_pasted` is called once right after Ctrl+V, before the wait and the restore — this is the "done" moment.
/// Without a paste (`LeftInClipboard`, error) it is not called. `Err` — the text could not be put in the clipboard, no paste happened.
pub fn paste_text(
    clip: &mut impl ClipboardApi,
    keys: &mut impl KeysApi,
    text: &str,
    on_pasted: impl FnOnce(),
) -> Result<PasteResult, String> {
    let text = crlf(text);
    // The clipboard is this paste's now: a late-copy guard of an earlier Ctrl+C copy stands down.
    clip.operations().begin();
    if !keys.wait_modifiers_released(Duration::from_secs(5)) {
        clip.write_text(&text, false)?;
        return Ok(PasteResult::LeftInClipboard);
    }
    // Not read — not restored: an empty "previous" clipboard would wipe the real one.
    let snap = clip.snapshot().ok();
    let before = clip.sequence();
    if let Err(e) = clip.write_text(&text, true) {
        // The number changed — the write managed to empty the clipboard: put the previous one back as far as possible.
        // It did not change — the clipboard is intact (usually another app was holding it), and a restore would only overwrite it.
        if let Some(snap) = &snap
            && clip.sequence() != before
        {
            let _ = clip.restore(snap);
        }
        return Err(e);
    }
    // The number once the write is finished and the clipboard is closed: from here on only someone else's write changes it.
    let ours = clip.sequence();
    if !keys.press_ctrl_v() {
        // Like "no field": the text stays in the clipboard and goes into the history, the user pastes it themselves.
        clip.write_text(&text, false)?;
        return Ok(PasteResult::LeftInClipboard);
    }
    on_pasted();
    // The window does not read the clipboard right after Ctrl+V: it must not be changed earlier.
    keys.sleep(Duration::from_millis(500));
    match snap {
        // Not restored — still not an error: the text is already at the caret and "done" has been reported.
        Some(snap) if clip.sequence() == ours => Ok(PasteResult::Pasted { restored: clip.restore(&snap).is_ok() }),
        _ => Ok(PasteResult::Pasted { restored: false }),
    }
}

/// Windows line breaks: `\n` without a `\r` before it becomes `\r\n` (some text fields do not show a bare LF).
fn crlf(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' && prev != '\r' {
            out.push('\r');
        }
        out.push(c);
        prev = c;
    }
    out
}

/// Reading the clipboard for the Ctrl+C fallback of "Ask".
pub trait ReadClipboard {
    /// `None` — no text in the clipboard.
    fn read_text(&mut self) -> Result<Option<String>, String>;
    /// The process of the window that owns the clipboard: the last one to write it. `None` — no owner, or not known.
    fn owner_process(&self) -> Option<u32>;
    /// The late-copy guard's act, with one opening of the clipboard and no wait for it — it runs under the operations
    /// lock: if the clipboard is still at number `seq`, its text is read and `snap` put back.
    fn take_back(&mut self, seq: u32, snap: &Snapshot) -> TakeBack;
}

/// What `ReadClipboard::take_back` did.
pub enum TakeBack {
    /// Another app held the clipboard open: nothing was done; worth another try.
    Busy,
    /// The clipboard was written again after `seq`: nothing was done.
    Changed,
    /// The text that was in it, and whether the snapshot went back.
    Done { text: Option<String>, restored: Result<(), String> },
}

/// Pressing Ctrl+C for the Ctrl+C fallback of "Ask".
pub trait CopyKeys {
    /// `false` — the system did not accept all presses.
    fn press_ctrl_c(&mut self) -> bool;
    /// The process the keys go to: the foreground window's. `None` — no foreground window.
    fn target_process(&self) -> Option<u32>;
    /// When the session's last input came in (a tick in ms): any key, click or mouse move, the user's or injected.
    /// `None` — not known.
    fn last_input(&self) -> Option<u32>;
}

/// How long the chord's keys may stay down before the copy gives up: Ctrl + Win + Alt + C would be another shortcut.
const COPY_KEYS_WAIT: Duration = Duration::from_millis(1_500);
/// How long the app has to copy after Ctrl+C.
const COPY_WAIT: Duration = Duration::from_millis(400);
const COPY_POLL: Duration = Duration::from_millis(20);
/// How long after `COPY_WAIT` a copy of a slow app is still caught and the user's clipboard put back.
const LATE_COPY_WAIT: Duration = Duration::from_secs(2);
/// A clipboard another app holds open: the late-copy guard tries again this many times, this far apart (as `open` does),
/// waiting outside the operations lock.
const TAKE_BACK_TRIES: u32 = 10;
const TAKE_BACK_RETRY: Duration = Duration::from_millis(50);

/// What the Ctrl+C fallback of "Ask" got. `Debug` shows lengths only.
#[derive(Clone, PartialEq)]
pub struct SelectionCopy {
    /// The copied selection; `None` — nothing, or only blanks, was copied in time.
    pub text: Option<String>,
    /// Just before Ctrl+C: a Win+V history item stamped earlier is not this copy (`forget_in_history`).
    pub since: SystemTime,
    /// The app copied, and putting the user's clipboard back failed (logged, without text): the copy is left in it.
    pub restore_failed: bool,
}

impl std::fmt::Debug for SelectionCopy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectionCopy")
            .field("chars", &self.text.as_ref().map(|t| t.chars().count()))
            .field("restore_failed", &self.restore_failed)
            .finish()
    }
}

/// A copy that did not come within `COPY_WAIT`: the app may still copy, and then the user's clipboard must come back.
/// What `guard_late_copy` needs for it.
pub struct LateCopy {
    /// The user's clipboard, to put back.
    snap: Snapshot,
    /// The clipboard's number when Ctrl+C was pressed.
    before: u32,
    /// The process Ctrl+C went to; `None` — not known: no late copy can be told from another app's.
    target: Option<u32>,
    since: SystemTime,
    /// The copy's clipboard operation (`Operations::begin`).
    operation: u64,
    /// The last input once CUE's own Ctrl+C had gone through (`CopyKeys::last_input`): any later is the user's.
    input_mark: Option<u32>,
}

/// The Ctrl+C fallback of "Ask" (spec 2 §6.1): the selection of an app that does not report it, copied and read, and the
/// user's clipboard put back as it was, with the marks of a paste (not in the Win+V history again). Never in a terminal
/// (the caller decides, `selection::needs_copy_fallback`).
///
/// What becomes of the user's clipboard:
/// - the chord's keys stayed down, or the clipboard could not be read first: Ctrl+C is not pressed, it is not touched;
/// - the app copied within `COPY_WAIT`: it is put back; if that fails, `restore_failed` says so, and the app's copy is
///   left in it;
/// - nothing came within `COPY_WAIT` (no selection, or a slow app; `text: None`): `on_late` gets a `LateCopy`, and the
///   caller runs `guard_late_copy` with it on a thread of its own (`spawn_late_guard`): a copy that looks like the app's
///   answer to this Ctrl+C, 0.4 to 2.4 s after it, is put back all the same. One later still, or one that may be the
///   user's own, stays in the clipboard.
///
/// The app's own copy goes into the Win+V history; CUE cannot mark it, and tries to remove it afterwards
/// (`forget_in_history`, decision 27).
pub fn copy_selection(
    clip: &mut (impl ClipboardApi + ReadClipboard),
    keys: &mut (impl KeysApi + CopyKeys),
    on_late: impl FnOnce(LateCopy),
) -> SelectionCopy {
    let nothing = |since| SelectionCopy { text: None, since, restore_failed: false };
    // The clipboard is this copy's now: a late-copy guard of an earlier one stands down.
    let operation = clip.operations().begin();
    if !keys.wait_modifiers_released(COPY_KEYS_WAIT) {
        return nothing(SystemTime::now());
    }
    // Not read — not risked: an empty "previous" clipboard would wipe the real one.
    let Ok(snap) = clip.snapshot() else { return nothing(SystemTime::now()) };
    let target = keys.target_process();
    let since = SystemTime::now();
    let before = clip.sequence();
    if !keys.press_ctrl_c() {
        return nothing(since);
    }
    let mut input_mark = None;
    let mut waited = Duration::ZERO;
    while clip.sequence() == before {
        if waited >= COPY_WAIT {
            on_late(LateCopy { snap, before, target, since, operation, input_mark });
            return nothing(since);
        }
        keys.sleep(COPY_POLL);
        if waited.is_zero() {
            // CUE's own Ctrl+C has gone through the input stream by now: any input after this mark is the user's.
            input_mark = keys.last_input();
        }
        waited += COPY_POLL;
    }
    // An app that writes several formats may put the text a moment after the first one.
    keys.sleep(COPY_POLL);
    read_and_restore(clip, &snap, since)
}

/// After `copy_selection` handed a `LateCopy` over, on a thread of its own: the clipboard is watched from the end of
/// `COPY_WAIT` for `LATE_COPY_WAIT` — 0.4 to 2.4 s after CUE's Ctrl+C. The first write in that time is taken for the
/// app's answer to that Ctrl+C — the user's clipboard put back, the copy returned, and `on_caught` run with it (the start
/// of its removal from the Win+V history) — only when it looks like one:
/// - its owner is the process Ctrl+C went to (another app's write, or one whose owner is not known, is a newer copy the
///   user may want);
/// - there was no input at all since CUE's Ctrl+C went through — no key, click or mouse move: the user's own Ctrl+C,
///   Shift+Delete or a menu's Copy in the same app would look just the same otherwise;
/// - the clipboard was not written again after it;
/// - no newer clipboard operation of CUE's has begun (a paste, the next Ctrl+C copy): the clipboard is that one's. The
///   last of these checks, the restore and `on_caught` happen under the operations lock (`Operations::while_newest`), so
///   none begins in between.
///
/// Otherwise the clipboard is left alone and the watch ends. `None` — nothing was put back. Without the target process
/// nothing can be told apart: no watch at all.
pub fn guard_late_copy<R>(
    clip: &mut (impl ClipboardApi + ReadClipboard),
    keys: &mut (impl KeysApi + CopyKeys),
    late: LateCopy,
    mut on_caught: impl FnMut(&SelectionCopy) -> R,
) -> Option<(SelectionCopy, R)> {
    /// What one try under the lock came to.
    enum Act<T> {
        Retry,
        Leave,
        Done(T),
    }
    let target = late.target?;
    let operations = clip.operations();
    let mut waited = Duration::ZERO;
    let seq = loop {
        if operations.newest() != late.operation {
            return None;
        }
        let seq = clip.sequence();
        if seq != late.before {
            break seq;
        }
        if waited >= LATE_COPY_WAIT {
            return None;
        }
        keys.sleep(COPY_POLL);
        waited += COPY_POLL;
    };
    if clip.owner_process() != Some(target) {
        tracing::info!("ask: another app wrote the clipboard after the copy wait; left alone");
        return None;
    }
    // An app that writes several formats may put the text a moment after the first one.
    keys.sleep(COPY_POLL);
    for attempt in 0..TAKE_BACK_TRIES {
        if attempt > 0 {
            // Outside the lock: another app holding the clipboard never holds up a newer paste of CUE's.
            keys.sleep(TAKE_BACK_RETRY);
        }
        let acted = operations.while_newest(late.operation, || {
            if input_since(&*keys, late.input_mark) {
                tracing::info!("ask: the clipboard changed after the copy wait, with input of the user's; left alone");
                return Act::Leave;
            }
            match clip.take_back(seq, &late.snap) {
                TakeBack::Busy => Act::Retry,
                TakeBack::Changed => Act::Leave,
                TakeBack::Done { text, restored } => {
                    let copy = caught(text, restored, late.since);
                    let then = on_caught(&copy);
                    Act::Done((copy, then))
                }
            }
        });
        match acted {
            Some(Act::Retry) => {}
            Some(Act::Done(done)) => return Some(done),
            // A newer operation of CUE's has begun, there was input, or the clipboard was written again.
            None | Some(Act::Leave) => return None,
        }
    }
    tracing::info!("ask: the clipboard stayed held by another app; a late copy was not put back");
    None
}

/// There was input since `mark` — a key, a click, a mouse move — or it cannot be told.
fn input_since(keys: &impl CopyKeys, mark: Option<u32>) -> bool {
    mark.is_none() || keys.last_input() != mark
}

/// The app's copy is in the clipboard: its text, and the user's clipboard put back.
fn read_and_restore(clip: &mut (impl ClipboardApi + ReadClipboard), snap: &Snapshot, since: SystemTime) -> SelectionCopy {
    let text = clip.read_text().ok().flatten();
    let restored = clip.restore(snap);
    caught(text, restored, since)
}

/// The app's copy as taken: its text unless blank; a failed restore logged, without text.
fn caught(text: Option<String>, restored: Result<(), String>, since: SystemTime) -> SelectionCopy {
    let restore_failed = match restored {
        Ok(()) => false,
        Err(e) => {
            tracing::warn!(error = %e, "ask: the user's clipboard was not put back after the copy");
            true
        }
    };
    SelectionCopy { text: text.filter(|t| !t.trim().is_empty()), since, restore_failed }
}

/// What became of the copied selection in the Windows clipboard history (decision 27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryForget {
    /// Gone from Win+V.
    Removed,
    /// Not among the newest items (the history is off, or the item did not show up in time).
    NotFound,
    /// Windows refused: clipboard history access is not given to every app.
    Refused,
}

/// What a late-copy guard caught (`spawn_late_guard`): the app's late copy, and what became of it in the Win+V history
/// (`None` — its text was blank, or the history thread did not run).
pub type LateCaught = (SelectionCopy, Option<HistoryForget>);

/// How much earlier than `since` the history may stamp the copy: its clock may round.
const HISTORY_CLOCK_SLACK: Duration = Duration::from_secs(1);

/// Which of the newest history items (newest first: its text, its stamp in `history_time`) is the copy: exactly the copied
/// text, stamped no earlier than just before Ctrl+C (`since`). Only the first three are looked at, and an older item of the
/// user's with the same text — pinned, or copied before the question — is never taken for it.
pub fn copied_item(items: &[(Option<String>, i64)], copied: &str, since: SystemTime) -> Option<usize> {
    let since = history_time(since.checked_sub(HISTORY_CLOCK_SLACK).unwrap_or(since));
    items.iter().take(3).position(|(text, at)| *at >= since && text.as_deref() == Some(copied))
}

/// A moment as the clipboard history stamps its items (`DateTime::UniversalTime`): 100 ns since 1601-01-01 UTC.
fn history_time(t: SystemTime) -> i64 {
    const UNIX_EPOCH_AT: i64 = 116_444_736_000_000_000;
    let ticks = |d: Duration| i64::try_from(d.as_nanos() / 100).unwrap_or(i64::MAX);
    match t.duration_since(UNIX_EPOCH) {
        Ok(after) => UNIX_EPOCH_AT.saturating_add(ticks(after)),
        Err(e) => UNIX_EPOCH_AT.saturating_sub(ticks(e.duration())),
    }
}

#[cfg(windows)]
mod win {
    use std::time::{Duration, SystemTime};

    use windows::core::{w, PCWSTR};
    use windows::ApplicationModel::DataTransfer::{Clipboard, ClipboardHistoryItemsResultStatus, StandardDataFormats};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber,
        IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetForegroundWindow, GetWindowThreadProcessId, HWND_MESSAGE, WINDOW_EX_STYLE,
        WINDOW_STYLE,
    };

    use super::{ClipboardApi, HistoryForget, KeysApi, LateCopy, Operations, Snapshot, TakeBack};

    /// CUE's clipboard operations in this process: every `WinClipboard` counts here, the late-copy guard's own too.
    static OPERATIONS: Operations = Operations::new();

    const CF_DIB: u32 = 8;
    const CF_UNICODETEXT: u32 = 13;
    const CF_HDROP: u32 = 15;

    /// The Windows clipboard. Keeps no window between operations: an owner window is created for each operation (`Owner`).
    pub struct WinClipboard {
        /// Saved formats — from rich to simple: they go back into the clipboard in this order, and apps
        /// that walk the formats in order take the first one they know. Only formats with data in an `HGLOBAL`
        /// (the system builds `CF_BITMAP` and the other GDI handles from `CF_DIB` itself).
        formats: [u32; 6],
        /// "Not in the Win+V history and not in the cloud clipboard" marks, each with the DWORD value 0:
        /// `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory`, `CanUploadToCloudClipboard`.
        marks: [u32; 3],
    }

    /// Clipboard owner window for one operation (without an owner `SetClipboardData` is not reliable). While the window
    /// owns the clipboard, someone else's copy waits for its thread to answer `WM_DESTROYCLIPBOARD` — for a thread without
    /// a message loop up to 5 s. A destroyed window holds nobody up, and the data stays in the clipboard: there is no delayed rendering.
    struct Owner(HWND);

    impl Owner {
        fn new() -> Result<Self, String> {
            // SAFETY: a message-only window of the system STATIC class: no menu, module or creation parameters.
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("cue-clipboard"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    None,
                    None,
                )
            }
            .map_err(|e| format!("CreateWindowExW: {e}"))?;
            Ok(Self(hwnd))
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            // SAFETY: the window was created by this thread in `Owner::new` (`HWND` is not `Send`) and is stored nowhere else.
            let _ = unsafe { DestroyWindow(self.0) };
        }
    }

    /// The open clipboard. When it goes out of scope the clipboard is closed, then the `_owner` field destroys
    /// the owner window — after `CloseClipboard`.
    struct Open {
        _owner: Owner,
    }

    impl Drop for Open {
        fn drop(&mut self) {
            // SAFETY: the clipboard was opened by this thread in `open`.
            let _ = unsafe { CloseClipboard() };
        }
    }

    /// Our own owner window and up to 10 attempts 50 ms apart: another app may be holding the clipboard open.
    /// If it did not open, the window is destroyed right away.
    fn open() -> Result<Open, String> {
        open_tries(10)
    }

    fn open_tries(tries: u32) -> Result<Open, String> {
        let owner = Owner::new()?;
        for attempt in 0..tries {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            // SAFETY: `owner` is a live window of this thread.
            if unsafe { OpenClipboard(Some(owner.0)) }.is_ok() {
                return Ok(Open { _owner: owner });
            }
        }
        Err("the clipboard is held by another app".into())
    }

    /// Format number by name (the same for all apps).
    fn register(name: &str) -> Result<u32, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a null-terminated string that lives until the end of the call.
        match unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) } {
            0 => Err(format!("RegisterClipboardFormatW: {}", windows::core::Error::from_thread())),
            fmt => Ok(fmt),
        }
    }

    impl WinClipboard {
        pub fn new() -> Result<Self, String> {
            let formats = [
                register("Rich Text Format")?,
                register("HTML Format")?,
                register("PNG")?,
                CF_DIB,
                CF_HDROP,
                CF_UNICODETEXT,
            ];
            let marks = [
                register("ExcludeClipboardContentFromMonitorProcessing")?,
                register("CanIncludeInClipboardHistory")?,
                register("CanUploadToCloudClipboard")?,
            ];
            Ok(Self { formats, marks })
        }

        /// Marks go first: if one of them could not be put, there will be no unmarked content in the clipboard.
        fn put_marks(&self) -> Result<(), String> {
            for fmt in self.marks {
                put(fmt, &0u32.to_le_bytes())?;
            }
            Ok(())
        }

        /// Puts `s` back into the clipboard, which this thread holds open.
        fn put_back(&self, s: &Snapshot) -> Result<(), String> {
            // SAFETY: the caller holds the clipboard open on this thread.
            unsafe { EmptyClipboard() }.map_err(|e| format!("EmptyClipboard: {e}"))?;
            // An empty clipboard is restored as empty — not as marks alone.
            if !s.items.is_empty() {
                self.put_marks()?;
            }
            for (fmt, bytes) in &s.items {
                put(*fmt, bytes)?;
            }
            Ok(())
        }
    }

    /// The text in the clipboard, which this thread holds open; `None` — no text.
    fn text_now() -> Option<String> {
        // SAFETY: the caller holds the clipboard open on this thread; CF_UNICODETEXT data is an HGLOBAL owned by the system,
        // read only under GlobalLock and no further than GlobalSize.
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
                return None;
            }
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g).cast::<u16>().cast_const();
            if p.is_null() {
                return None;
            }
            let units = GlobalSize(g) / 2;
            let all = std::slice::from_raw_parts(p, units);
            let end = all.iter().position(|&c| c == 0).unwrap_or(units);
            let text = String::from_utf16_lossy(&all[..end]);
            let _ = GlobalUnlock(g);
            Some(text)
        }
    }

    /// Puts a copy of `bytes` into a clipboard opened and emptied by this thread. The memory passes to the system only
    /// after a successful `SetClipboardData`; until then we free it ourselves.
    fn put(fmt: u32, bytes: &[u8]) -> Result<(), String> {
        // SAFETY: an ordinary movable memory allocation; a zero-size block cannot be locked — at least one byte.
        let h = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }.map_err(|e| format!("GlobalAlloc: {e}"))?;
        // SAFETY: `h` was just allocated and is still ours.
        let p = unsafe { GlobalLock(h) }.cast::<u8>();
        if p.is_null() {
            let e = windows::core::Error::from_thread();
            // SAFETY: `h` is ours and not locked.
            let _ = unsafe { GlobalFree(Some(h)) };
            return Err(format!("GlobalLock: {e}"));
        }
        // SAFETY: the block has at least `bytes.len()` bytes, the block and the slice do not overlap; after the copy the block
        // is unlocked and the pointer is no longer used.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            // `windows` returns releasing the last lock as `Err` (FALSE with NO_ERROR) — this is not an error.
            let _ = GlobalUnlock(h);
        }
        // SAFETY: `h` is an unlocked movable block from `GlobalAlloc`; without an open clipboard the call returns an error.
        if let Err(e) = unsafe { SetClipboardData(fmt, Some(HANDLE(h.0))) } {
            // SAFETY: the system did not accept the block — it is still ours.
            let _ = unsafe { GlobalFree(Some(h)) };
            return Err(format!("SetClipboardData: {e}"));
        }
        Ok(())
    }

    impl ClipboardApi for WinClipboard {
        fn sequence(&self) -> u32 {
            // SAFETY: no arguments, the clipboard does not need to be open.
            unsafe { GetClipboardSequenceNumber() }
        }

        fn snapshot(&mut self) -> Result<Snapshot, String> {
            let _open = open()?;
            let mut items = Vec::new();
            for &fmt in &self.formats {
                // SAFETY: the clipboard is open by this thread (`_open`); data of the listed formats is an `HGLOBAL` owned by
                // the system, and we read it only under `GlobalLock`, no further than `GlobalSize`.
                unsafe {
                    if IsClipboardFormatAvailable(fmt).is_err() {
                        continue;
                    }
                    let Ok(h) = GetClipboardData(fmt) else { continue };
                    let g = HGLOBAL(h.0);
                    let p = GlobalLock(g).cast::<u8>().cast_const();
                    if p.is_null() {
                        continue;
                    }
                    let size = GlobalSize(g);
                    if size > 0 {
                        items.push((fmt, std::slice::from_raw_parts(p, size).to_vec()));
                    }
                    let _ = GlobalUnlock(g);
                }
            }
            Ok(Snapshot { items })
        }

        fn write_text(&mut self, text: &str, private: bool) -> Result<(), String> {
            let utf16: Vec<u8> = text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
            let _open = open()?;
            // SAFETY: the clipboard is open by this thread (`_open`).
            unsafe { EmptyClipboard() }.map_err(|e| format!("EmptyClipboard: {e}"))?;
            if private {
                self.put_marks()?;
            }
            put(CF_UNICODETEXT, &utf16)
        }

        fn restore(&mut self, s: &Snapshot) -> Result<(), String> {
            let _open = open()?;
            self.put_back(s)
        }

        fn operations(&self) -> &'static Operations {
            &OPERATIONS
        }
    }

    /// Keys for the paste — through `crate::input`.
    pub struct WinKeys;

    impl KeysApi for WinKeys {
        fn wait_modifiers_released(&mut self, timeout: Duration) -> bool {
            crate::input::wait_modifiers_released(timeout)
        }
        fn press_ctrl_v(&mut self) -> bool {
            crate::input::press_ctrl_v()
        }
        fn sleep(&mut self, d: Duration) {
            std::thread::sleep(d);
        }
    }

    impl super::ReadClipboard for WinClipboard {
        fn read_text(&mut self) -> Result<Option<String>, String> {
            let _open = open()?;
            Ok(text_now())
        }

        fn take_back(&mut self, seq: u32, snap: &Snapshot) -> TakeBack {
            // One try, no wait: the caller holds the operations lock, and waits outside it.
            let Ok(_open) = open_tries(1) else { return TakeBack::Busy };
            // Open, the clipboard cannot be written by anyone else: the number checked now holds until it is closed.
            if self.sequence() != seq {
                return TakeBack::Changed;
            }
            TakeBack::Done { text: text_now(), restored: self.put_back(snap) }
        }

        fn owner_process(&self) -> Option<u32> {
            // SAFETY: no arguments; the clipboard does not need to be open.
            window_process(unsafe { GetClipboardOwner() }.ok()?)
        }
    }

    impl super::CopyKeys for WinKeys {
        fn press_ctrl_c(&mut self) -> bool {
            crate::input::press_ctrl_c()
        }

        fn target_process(&self) -> Option<u32> {
            // SAFETY: no arguments.
            window_process(unsafe { GetForegroundWindow() })
        }

        fn last_input(&self) -> Option<u32> {
            let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            // SAFETY: `info` with `cbSize` filled in lives until the end of the call.
            unsafe { GetLastInputInfo(&mut info) }.as_bool().then_some(info.dwTime)
        }
    }

    /// The process of a window; `None` for none or a closed one.
    fn window_process(hwnd: HWND) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: `pid` lives until the end of the call; an empty or closed window leaves it 0.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        (pid != 0).then_some(pid)
    }

    /// `guard_late_copy` on a thread of its own, for `copy_selection`'s `on_late`: it watches for about 2 s. A late copy
    /// it took out of the clipboard leaves the Win+V history too (`forget_in_history`), and the thread waits for that, so
    /// the removal is not cut short. The outcome goes to the log as a kind, never the text; the handle is for a probe —
    /// the copy and what became of it in the history — the app does not wait for it.
    pub fn spawn_late_guard(late: LateCopy) -> Option<std::thread::JoinHandle<Option<super::LateCaught>>> {
        let spawned = std::thread::Builder::new().name("cue-late-copy".into()).spawn(move || {
            let mut clip = WinClipboard::new().inspect_err(|e| tracing::warn!("late copy: no clipboard: {e}")).ok()?;
            // The removal starts under the operations lock, together with the restore.
            let (copy, removal) = super::guard_late_copy(&mut clip, &mut WinKeys, late, |copy| {
                copy.text.clone().and_then(|text| forget_in_history(text, copy.since))
            })?;
            tracing::info!(restore_failed = copy.restore_failed, "ask: the app copied the selection late; caught");
            Some((copy, removal.and_then(|h| h.join().ok())))
        });
        spawned.inspect_err(|e| tracing::warn!("late copy thread not started: {e}")).ok()
    }

    /// Decision 27: the selection the Ctrl+C fallback copied leaves the Win+V history, if Windows lets CUE remove it. On a
    /// thread of its own (WinRT, a few hundred ms; the history service takes the item a moment after the copy); the outcome
    /// goes to the log as a kind, never the text.
    /// `since` — `SelectionCopy::since`: only an item stamped after it can be the copy.
    pub fn forget_in_history(copied: String, since: SystemTime) -> Option<std::thread::JoinHandle<HistoryForget>> {
        let spawned = std::thread::Builder::new().name("cue-clip-history".into()).spawn(move || {
            // SAFETY: a COM apartment for this thread only (WinRT needs one); it ends with the thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let mut outcome = HistoryForget::NotFound;
            // History off: the copy is not there, and there is nothing to wait for.
            if Clipboard::IsHistoryEnabled().unwrap_or(true) {
                for _ in 0..3 {
                    std::thread::sleep(Duration::from_millis(300));
                    outcome = forget_once(&copied, since).unwrap_or(HistoryForget::Refused);
                    if outcome != HistoryForget::NotFound {
                        break;
                    }
                }
            }
            tracing::info!(?outcome, "ask: the copied selection in the clipboard history");
            outcome
        });
        spawned.inspect_err(|e| tracing::warn!("clipboard history thread not started: {e}")).ok()
    }

    fn forget_once(copied: &str, since: SystemTime) -> windows::core::Result<HistoryForget> {
        let result = Clipboard::GetHistoryItemsAsync()?.join()?;
        let status = result.Status()?;
        if status == ClipboardHistoryItemsResultStatus::ClipboardHistoryDisabled {
            return Ok(HistoryForget::NotFound);
        }
        if status != ClipboardHistoryItemsResultStatus::Success {
            return Ok(HistoryForget::Refused);
        }
        let items: Vec<_> = result.Items()?.into_iter().take(3).collect();
        let format = StandardDataFormats::Text()?;
        let entries: Vec<(Option<String>, i64)> = items
            .iter()
            .map(|item| {
                let text = || {
                    let content = item.Content().ok()?;
                    if !content.Contains(&format).ok()? {
                        return None;
                    }
                    Some(content.GetTextAsync().ok()?.join().ok()?.to_string())
                };
                // No stamp: not known to be the copy.
                (text(), item.Timestamp().map_or(i64::MIN, |t| t.UniversalTime))
            })
            .collect();
        Ok(match super::copied_item(&entries, copied, since) {
            Some(i) if Clipboard::DeleteItemFromHistory(&items[i])? => HistoryForget::Removed,
            Some(_) => HistoryForget::Refused,
            None => HistoryForget::NotFound,
        })
    }
}

#[cfg(windows)]
pub use win::{forget_in_history, spawn_late_guard, WinClipboard, WinKeys};

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// What the paste did and in what order — a shared log of the fake clipboard, keys and `on_pasted`.
    #[derive(Debug, Clone, PartialEq)]
    enum Ev {
        Snapshot,
        Wait,
        Write { private: bool },
        CtrlV,
        OnPasted,
        Sleep(Duration),
        Restore,
    }

    type Log = Rc<RefCell<Vec<Ev>>>;

    #[derive(Default)]
    struct FakeClip {
        seq: u32,
        reads_after_write: Cell<u32>,
        someone_copies: bool,
        snapshot_fails: bool,
        /// The write failed, the clipboard is untouched (usually another app is holding it).
        write_fails: bool,
        /// The write managed to empty the clipboard (the number changed) and failed.
        write_fails_after_emptying: bool,
        restore_fails: bool,
        content: Snapshot,
        writes: Vec<(String, bool)>,
        restored: Option<Snapshot>,
        ops: std::cell::OnceCell<&'static Operations>,
        log: Log,
    }

    const WRITE_ERROR: &str = "SetClipboardData: refused";

    impl ClipboardApi for FakeClip {
        fn sequence(&self) -> u32 {
            let n = self.reads_after_write.get();
            self.reads_after_write.set(n + 1);
            if self.someone_copies && n >= 1 { self.seq + 1 } else { self.seq }
        }
        fn snapshot(&mut self) -> Result<Snapshot, String> {
            self.log.borrow_mut().push(Ev::Snapshot);
            if self.snapshot_fails {
                return Err("the clipboard is held by another app".into());
            }
            Ok(self.content.clone())
        }
        fn write_text(&mut self, text: &str, private: bool) -> Result<(), String> {
            self.log.borrow_mut().push(Ev::Write { private });
            if self.write_fails {
                return Err(WRITE_ERROR.into());
            }
            self.seq += 1;
            self.reads_after_write.set(0);
            if self.write_fails_after_emptying {
                return Err(WRITE_ERROR.into());
            }
            self.writes.push((text.into(), private));
            Ok(())
        }
        fn restore(&mut self, s: &Snapshot) -> Result<(), String> {
            self.log.borrow_mut().push(Ev::Restore);
            if self.restore_fails {
                return Err("EmptyClipboard: refused".into());
            }
            self.restored = Some(s.clone());
            Ok(())
        }
        fn operations(&self) -> &'static Operations {
            self.ops.get_or_init(|| Box::leak(Box::default()))
        }
    }

    #[derive(Default)]
    struct FakeKeys {
        stuck: bool,
        ctrl_v_fails: bool,
        pressed: u32,
        slept: Duration,
        log: Log,
    }

    impl KeysApi for FakeKeys {
        fn wait_modifiers_released(&mut self, _t: Duration) -> bool {
            self.log.borrow_mut().push(Ev::Wait);
            !self.stuck
        }
        fn press_ctrl_v(&mut self) -> bool {
            self.log.borrow_mut().push(Ev::CtrlV);
            self.pressed += 1;
            !self.ctrl_v_fails
        }
        fn sleep(&mut self, d: Duration) {
            self.log.borrow_mut().push(Ev::Sleep(d));
            self.slept += d;
        }
    }

    fn user_clip() -> Snapshot {
        Snapshot { items: vec![(13, vec![65, 0, 0, 0])] }
    }

    #[test]
    fn pastes_privately_and_restores_users_clipboard() {
        let mut c = FakeClip { content: user_clip(), ..Default::default() };
        let mut k = FakeKeys::default();
        let called = Cell::new(0);
        let r = paste_text(&mut c, &mut k, "Привет.", || called.set(called.get() + 1));
        assert_eq!(r.unwrap(), PasteResult::Pasted { restored: true });
        assert_eq!(c.writes, vec![("Привет.".to_string(), true)]);
        assert_eq!(k.pressed, 1);
        assert_eq!(called.get(), 1, "on_pasted — exactly once");
        assert_eq!(k.slept, Duration::from_millis(500));
        assert_eq!(c.restored, Some(user_clip()));
    }

    #[test]
    fn on_pasted_runs_right_after_ctrl_v_before_the_wait_and_restore() {
        let log = Log::default();
        let mut c = FakeClip { content: user_clip(), log: log.clone(), ..Default::default() };
        let mut k = FakeKeys { log: log.clone(), ..Default::default() };
        let r = paste_text(&mut c, &mut k, "x", || log.borrow_mut().push(Ev::OnPasted));
        assert_eq!(r.unwrap(), PasteResult::Pasted { restored: true });
        assert_eq!(
            *log.borrow(),
            vec![
                Ev::Wait,
                Ev::Snapshot,
                Ev::Write { private: true },
                Ev::CtrlV,
                Ev::OnPasted,
                Ev::Sleep(Duration::from_millis(500)),
                Ev::Restore,
            ]
        );
    }

    #[test]
    fn does_not_overwrite_a_newer_copy() {
        let mut c = FakeClip { content: user_clip(), someone_copies: true, ..Default::default() };
        let mut k = FakeKeys::default();
        let called = Cell::new(false);
        let r = paste_text(&mut c, &mut k, "x", || called.set(true));
        assert_eq!(r.unwrap(), PasteResult::Pasted { restored: false });
        assert!(called.get());
        assert_eq!(c.restored, None);
    }

    #[test]
    fn empty_clipboard_is_restored_as_empty() {
        let mut c = FakeClip::default();
        let mut k = FakeKeys::default();
        assert_eq!(paste_text(&mut c, &mut k, "x", || {}).unwrap(), PasteResult::Pasted { restored: true });
        assert_eq!(c.restored, Some(Snapshot::default()));
    }

    #[test]
    fn stuck_modifiers_leave_text_in_clipboard() {
        let log = Log::default();
        let mut c = FakeClip { content: user_clip(), log: log.clone(), ..Default::default() };
        let mut k = FakeKeys { stuck: true, log: log.clone(), ..Default::default() };
        let r = paste_text(&mut c, &mut k, "текст", || log.borrow_mut().push(Ev::OnPasted));
        assert_eq!(r.unwrap(), PasteResult::LeftInClipboard);
        assert_eq!(c.writes, vec![("текст".to_string(), false)]);
        assert_eq!(k.pressed, 0);
        assert_eq!(c.restored, None);
        assert!(!log.borrow().contains(&Ev::OnPasted), "without a paste there is no \"done\"");
    }

    #[test]
    fn failed_ctrl_v_leaves_text_in_clipboard_without_marks() {
        let log = Log::default();
        let mut c = FakeClip { content: user_clip(), log: log.clone(), ..Default::default() };
        let mut k = FakeKeys { ctrl_v_fails: true, log: log.clone(), ..Default::default() };
        let r = paste_text(&mut c, &mut k, "текст", || log.borrow_mut().push(Ev::OnPasted));
        assert_eq!(r.unwrap(), PasteResult::LeftInClipboard);
        assert_eq!(c.writes, vec![("текст".to_string(), true), ("текст".to_string(), false)]);
        assert_eq!(c.restored, None);
        assert_eq!(k.slept, Duration::ZERO);
        assert!(!log.borrow().contains(&Ev::OnPasted), "Ctrl+V did not get through — no \"done\"");
    }

    #[test]
    fn failed_snapshot_pastes_but_does_not_wipe_users_clipboard() {
        let mut c = FakeClip { content: user_clip(), snapshot_fails: true, ..Default::default() };
        let mut k = FakeKeys::default();
        let called = Cell::new(false);
        let r = paste_text(&mut c, &mut k, "x", || called.set(true));
        assert_eq!(r.unwrap(), PasteResult::Pasted { restored: false });
        assert_eq!(c.writes, vec![("x".to_string(), true)]);
        assert_eq!(k.pressed, 1);
        assert!(called.get());
        assert_eq!(c.restored, None, "an empty \"previous\" clipboard is not written over the real one");
    }

    #[test]
    fn failed_restore_after_the_paste_is_not_an_error() {
        let mut c = FakeClip { content: user_clip(), restore_fails: true, ..Default::default() };
        let mut k = FakeKeys::default();
        let called = Cell::new(0);
        let r = paste_text(&mut c, &mut k, "x", || called.set(called.get() + 1));
        assert_eq!(r.unwrap(), PasteResult::Pasted { restored: false }, "the text is already at the caret, \"done\" has been reported");
        assert_eq!(called.get(), 1);
        assert_eq!(k.pressed, 1);
    }

    #[test]
    fn failed_private_write_that_touched_nothing_restores_nothing() {
        let log = Log::default();
        let mut c = FakeClip { content: user_clip(), write_fails: true, log: log.clone(), ..Default::default() };
        let mut k = FakeKeys { log: log.clone(), ..Default::default() };
        let r = paste_text(&mut c, &mut k, "x", || log.borrow_mut().push(Ev::OnPasted));
        assert_eq!(r, Err(WRITE_ERROR.to_string()));
        assert_eq!(k.pressed, 0);
        assert_eq!(c.restored, None, "the clipboard is intact — a restore would only overwrite it");
        assert_eq!(*log.borrow(), vec![Ev::Wait, Ev::Snapshot, Ev::Write { private: true }]);
    }

    #[test]
    fn failed_private_write_after_emptying_puts_users_clipboard_back() {
        let log = Log::default();
        let mut c =
            FakeClip { content: user_clip(), write_fails_after_emptying: true, log: log.clone(), ..Default::default() };
        let mut k = FakeKeys { log: log.clone(), ..Default::default() };
        let r = paste_text(&mut c, &mut k, "x", || log.borrow_mut().push(Ev::OnPasted));
        assert_eq!(r, Err(WRITE_ERROR.to_string()));
        assert_eq!(k.pressed, 0);
        assert_eq!(c.restored, Some(user_clip()), "the write emptied the clipboard — the previous one is put back");
        assert_eq!(*log.borrow(), vec![Ev::Wait, Ev::Snapshot, Ev::Write { private: true }, Ev::Restore]);

        // The restore failed too — the write error still goes out.
        let log = Log::default();
        let mut c = FakeClip {
            content: user_clip(),
            write_fails_after_emptying: true,
            restore_fails: true,
            log: log.clone(),
            ..Default::default()
        };
        let mut k = FakeKeys::default();
        let r = paste_text(&mut c, &mut k, "x", || panic!("there was no paste"));
        assert_eq!(r, Err(WRITE_ERROR.to_string()));
        assert_eq!(k.pressed, 0);
        assert!(log.borrow().contains(&Ev::Restore));

        // No snapshot — nothing to restore.
        let log = Log::default();
        let mut c =
            FakeClip { snapshot_fails: true, write_fails_after_emptying: true, log: log.clone(), ..Default::default() };
        let mut k = FakeKeys::default();
        let r = paste_text(&mut c, &mut k, "x", || panic!("there was no paste"));
        assert_eq!(r, Err(WRITE_ERROR.to_string()));
        assert!(!log.borrow().contains(&Ev::Restore));
    }

    #[test]
    fn bare_lf_becomes_crlf() {
        assert_eq!(crlf("a\nb"), "a\r\nb");
        assert_eq!(crlf("a\r\nb"), "a\r\nb");
        assert_eq!(crlf("\n\n"), "\r\n\r\n");
        assert_eq!(crlf("\r\n\n"), "\r\n\r\n");
        assert_eq!(crlf("a\rb\n"), "a\rb\r\n");
        assert_eq!(crlf("Привет.\nПока."), "Привет.\r\nПока.");
        assert_eq!(crlf(""), "");
    }

    #[test]
    fn both_private_and_plain_writes_use_crlf() {
        let mut c = FakeClip::default();
        let mut k = FakeKeys::default();
        paste_text(&mut c, &mut k, "раз\nдва", || {}).unwrap();
        assert_eq!(c.writes, vec![("раз\r\nдва".to_string(), true)]);

        let mut c = FakeClip::default();
        let mut k = FakeKeys { stuck: true, ..Default::default() };
        paste_text(&mut c, &mut k, "раз\nдва", || {}).unwrap();
        assert_eq!(c.writes, vec![("раз\r\nдва".to_string(), false)]);

        let mut c = FakeClip::default();
        let mut k = FakeKeys { ctrl_v_fails: true, ..Default::default() };
        paste_text(&mut c, &mut k, "раз\nдва", || {}).unwrap();
        assert_eq!(c.writes, vec![("раз\r\nдва".to_string(), true), ("раз\r\nдва".to_string(), false)]);
    }
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    use std::cell::{OnceCell, RefCell};
    use std::rc::Rc;

    /// The app Ctrl+C goes to, another one, and CUE.
    const APP: u32 = 100;
    const OTHER: u32 = 200;
    const CUE: u32 = 300;

    /// The clipboard, the app behind it and the user, counted in sleeps: on Ctrl+C the app copies `selection` (if any)
    /// after `copy_after` sleeps; another app writes the clipboard at sleep `other_writes_at`; the user copies in the app
    /// at `user_copies_at`, or just types or moves the mouse at `user_input_at`; a newer clipboard operation of CUE's
    /// begins at `newer_begins_at`; the clipboard is held open by another app for the first `busy_takes` take-backs.
    #[derive(Default)]
    struct World {
        seq: u32,
        content: Snapshot,
        text: Option<String>,
        owner: Option<u32>,
        selection: Option<String>,
        copy_after: u32,
        other_writes_at: Option<u32>,
        user_copies_at: Option<u32>,
        user_input_at: Option<u32>,
        newer_begins_at: Option<u32>,
        busy_takes: u32,
        input_tick: u32,
        no_target: bool,
        sleeps: u32,
        pressed_c: bool,
        app_copied: bool,
        ctrl_v: u32,
        stuck: bool,
        snapshot_fails: bool,
        restore_fails: bool,
        restored: Option<Snapshot>,
        restores: u32,
        writes: Vec<String>,
        ops: OnceCell<&'static Operations>,
    }

    impl World {
        fn ops(&self) -> &'static Operations {
            self.ops.get_or_init(|| Box::leak(Box::default()))
        }

        /// The app answers Ctrl+C once.
        fn maybe_copy(&mut self) {
            if self.pressed_c && !self.app_copied && self.sleeps >= self.copy_after {
                self.app_copies_now();
            }
        }

        fn app_copies_now(&mut self) {
            if let Some(s) = self.selection.clone() {
                self.text = Some(s);
                self.owner = Some(APP);
                self.seq += 1;
                self.app_copied = true;
            }
        }

        fn restore_now(&mut self, s: &Snapshot) -> Result<(), String> {
            if self.restore_fails {
                return Err("EmptyClipboard: refused".into());
            }
            self.restored = Some(s.clone());
            self.restores += 1;
            self.owner = None;
            self.seq += 1;
            Ok(())
        }
    }

    struct Clip(Rc<RefCell<World>>);
    struct Keys(Rc<RefCell<World>>);

    impl ClipboardApi for Clip {
        fn sequence(&self) -> u32 {
            self.0.borrow().seq
        }
        fn snapshot(&mut self) -> Result<Snapshot, String> {
            let w = self.0.borrow();
            if w.snapshot_fails { Err("held by another app".into()) } else { Ok(w.content.clone()) }
        }
        fn write_text(&mut self, text: &str, _private: bool) -> Result<(), String> {
            let mut w = self.0.borrow_mut();
            w.writes.push(text.into());
            w.text = Some(text.into());
            w.owner = Some(CUE);
            w.seq += 1;
            Ok(())
        }
        fn restore(&mut self, s: &Snapshot) -> Result<(), String> {
            self.0.borrow_mut().restore_now(s)
        }
        fn operations(&self) -> &'static Operations {
            self.0.borrow().ops()
        }
    }

    impl ReadClipboard for Clip {
        fn read_text(&mut self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().text.clone())
        }
        fn owner_process(&self) -> Option<u32> {
            self.0.borrow().owner
        }
        fn take_back(&mut self, seq: u32, snap: &Snapshot) -> TakeBack {
            let mut w = self.0.borrow_mut();
            if w.busy_takes > 0 {
                w.busy_takes -= 1;
                return TakeBack::Busy;
            }
            if w.seq != seq {
                return TakeBack::Changed;
            }
            let text = w.text.clone();
            TakeBack::Done { text, restored: w.restore_now(snap) }
        }
    }

    impl KeysApi for Keys {
        fn wait_modifiers_released(&mut self, _t: Duration) -> bool {
            !self.0.borrow().stuck
        }
        fn press_ctrl_v(&mut self) -> bool {
            self.0.borrow_mut().ctrl_v += 1;
            true
        }
        fn sleep(&mut self, _d: Duration) {
            let mut w = self.0.borrow_mut();
            w.sleeps += 1;
            let now = Some(w.sleeps);
            if w.other_writes_at == now {
                w.text = Some("чужое".into());
                w.owner = Some(OTHER);
                w.seq += 1;
            }
            if w.user_copies_at == now {
                w.input_tick += 1;
                w.text = Some("своё".into());
                w.owner = Some(APP);
                w.seq += 1;
            }
            if w.user_input_at == now {
                w.input_tick += 1;
            }
            if w.newer_begins_at == now {
                w.ops().begin();
            }
            w.maybe_copy();
        }
    }

    impl CopyKeys for Keys {
        fn press_ctrl_c(&mut self) -> bool {
            let mut w = self.0.borrow_mut();
            w.pressed_c = true;
            w.maybe_copy();
            true
        }
        fn target_process(&self) -> Option<u32> {
            (!self.0.borrow().no_target).then_some(APP)
        }
        fn last_input(&self) -> Option<u32> {
            Some(self.0.borrow().input_tick)
        }
    }

    fn user_clip() -> Snapshot {
        Snapshot { items: vec![(13, vec![65, 0, 0, 0])] }
    }

    /// The world after a copy and its guard: neither ever pastes or writes the clipboard itself.
    fn world(w: Rc<RefCell<World>>) -> World {
        let w = Rc::try_unwrap(w).ok().unwrap().into_inner();
        assert_eq!((w.ctrl_v, w.writes.len()), (0, 0), "the copy and the guard never paste or write");
        w
    }

    /// `copy_selection` on `w`, and whether it handed a late copy over.
    fn copy(w: World) -> (SelectionCopy, bool, World) {
        let w = Rc::new(RefCell::new(w));
        let mut late = false;
        let got = copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |_| late = true);
        (got, late, world(w))
    }

    /// The same, and the late copy it handed over watched right after (in the app: on a thread of its own).
    fn copy_and_watch(w: World) -> (SelectionCopy, Option<SelectionCopy>, World) {
        let w = Rc::new(RefCell::new(w));
        let mut late = None;
        let got = copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |l| late = Some(l));
        let watched = late.and_then(|l| guard_late_copy(&mut Clip(w.clone()), &mut Keys(w.clone()), l, |_| ()));
        (got, watched.map(|(copy, ())| copy), world(w))
    }

    fn late_app() -> World {
        World { content: user_clip(), selection: Some("поздно".into()), copy_after: 40, ..Default::default() }
    }

    #[test]
    fn copies_the_selection_and_puts_the_clipboard_back() {
        let (got, late, w) =
            copy(World { content: user_clip(), selection: Some("выделенный текст".into()), ..Default::default() });
        assert_eq!(got.text.as_deref(), Some("выделенный текст"));
        assert!(!got.restore_failed && !late);
        assert_eq!(w.restored, Some(user_clip()), "the user's clipboard is back");
        assert_eq!(w.ops().newest(), 1, "a clipboard operation of its own");
    }

    #[test]
    fn waits_for_a_slow_app() {
        let (got, _, _) = copy(World { selection: Some("медленно".into()), copy_after: 5, ..Default::default() });
        assert_eq!(got.text.as_deref(), Some("медленно"));
    }

    /// Spec 2 §13.5: with nothing selected the clipboard is not touched.
    #[test]
    fn nothing_selected_leaves_the_clipboard_untouched() {
        let (got, late, w) = copy(World { content: user_clip(), ..Default::default() });
        assert_eq!(got.text, None);
        assert!(late, "the app may still copy: that is watched for");
        assert_eq!(w.restored, None, "nothing to put back");
        assert!(w.sleeps * 20 <= 400 + 20, "gives up after 0.4 s: {} sleeps", w.sleeps);

        let (_, watched, w) = copy_and_watch(World { content: user_clip(), ..Default::default() });
        assert_eq!(watched, None);
        assert_eq!(w.restored, None, "the watch ends with nothing put back");
        assert!(w.sleeps * 20 <= 400 + 2_000 + 20, "after about 2 s more: {} sleeps", w.sleeps);
    }

    /// A busy app copies after the 0.4 s wait: the user's clipboard comes back all the same, and the copy's text is there
    /// to leave the Win+V history.
    #[test]
    fn a_late_copy_still_gets_the_clipboard_back() {
        let (got, watched, w) = copy_and_watch(late_app());
        assert_eq!(got.text, None, "too late for the question");
        let watched = watched.expect("the late copy is caught");
        assert_eq!(watched.text.as_deref(), Some("поздно"));
        assert!(!watched.restore_failed);
        assert_eq!(w.restored, Some(user_clip()), "the user's clipboard is back");
    }

    #[test]
    fn a_late_write_of_another_app_is_left_alone() {
        let other = World { selection: Some("x".into()), copy_after: 1_000, other_writes_at: Some(30), ..late_app() };
        let (_, watched, w) = copy_and_watch(other);
        assert_eq!(watched, None);
        assert_eq!(w.restored, None, "a newer copy of another app is not overwritten");
    }

    #[test]
    fn a_failed_restore_is_reported() {
        let (got, _, w) =
            copy(World { content: user_clip(), selection: Some("x".into()), restore_fails: true, ..Default::default() });
        assert_eq!((got.text.as_deref(), got.restore_failed), (Some("x"), true));
        assert_eq!(w.restored, None);
        let (_, watched, _) = copy_and_watch(World { restore_fails: true, ..late_app() });
        assert!(watched.is_some_and(|c| c.restore_failed), "a late copy that could not be put back says so too");
    }

    /// The user copies something themselves in the same app 0.4–2.4 s after CUE's Ctrl+C (nothing was selected): it
    /// is not taken for the app's late answer — not overwritten, and not removed from Win+V.
    #[test]
    fn the_users_own_copy_in_the_same_app_is_left_alone() {
        let (_, watched, w) = copy_and_watch(World { content: user_clip(), user_copies_at: Some(30), ..Default::default() });
        assert_eq!(watched, None, "no copy is returned, so none is removed from the history");
        assert_eq!(w.restored, None);
        assert_eq!(w.text.as_deref(), Some("своё"), "the user's copy stays in the clipboard");
    }

    /// Any input of the user's after CUE's Ctrl+C — a key (Shift+Delete, a keyboard menu), a click, a mouse move — and the
    /// change may be theirs: left alone, even when it is in fact the app's late copy.
    #[test]
    fn any_input_of_the_users_stands_the_guard_down() {
        let (_, watched, w) = copy_and_watch(World { user_input_at: Some(30), ..late_app() });
        assert_eq!(watched, None);
        assert_eq!(w.restored, None);
    }

    /// An older guard and a newer paste (or Ctrl+C copy) would react to the same write: the older one stands down as soon
    /// as a newer clipboard operation of CUE's has begun.
    #[test]
    fn an_older_guard_stands_down_once_a_newer_paste_or_copy_has_started() {
        // A paste: the app's late copy lands right after its Ctrl+V, while the older guard still watches.
        let w = Rc::new(RefCell::new(World { copy_after: 1_000, ..late_app() }));
        let mut late = None;
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |l| late = Some(l));
        let late = late.expect("the late copy is watched for");
        let mut watched = None;
        let pasted = paste_text(&mut Clip(w.clone()), &mut Keys(w.clone()), "ответ", || {
            w.borrow_mut().app_copies_now();
            watched = Some(guard_late_copy(&mut Clip(w.clone()), &mut Keys(w.clone()), late, |_| ()));
        });
        assert!(pasted.is_ok());
        assert_eq!(watched, Some(None), "the older guard stood down");
        assert_eq!(w.borrow().restores, 0, "nothing was put back over the paste");
        assert_eq!(w.borrow().ops().newest(), 2, "the paste is a clipboard operation of its own");

        // A newer Ctrl+C copy has begun.
        let w = Rc::new(RefCell::new(World { copy_after: 1_000, ..late_app() }));
        let mut late = None;
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |l| late = Some(l));
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |_| {});
        w.borrow_mut().app_copies_now();
        let late = late.expect("watched for");
        assert_eq!(guard_late_copy(&mut Clip(w.clone()), &mut Keys(w.clone()), late, |_| ()), None);
        assert_eq!(w.borrow().restores, 0);
    }

    /// The last moment: a newer operation begins after the guard's last poll, while it is about to put the clipboard back.
    #[test]
    fn a_newer_operation_as_the_guard_is_about_to_act_stands_it_down() {
        // Sleep 40: the app's late copy; 41: the moment the formats get to settle.
        let (_, watched, w) = copy_and_watch(World { newer_begins_at: Some(41), ..late_app() });
        assert_eq!(watched, None);
        assert_eq!(w.restores, 0);
    }

    /// Another app holds the clipboard open: the guard tries again, waiting outside the lock — and a newer operation that
    /// begins meanwhile still stands it down.
    #[test]
    fn a_busy_clipboard_is_tried_again_outside_the_lock() {
        let (_, watched, w) = copy_and_watch(World { busy_takes: 2, ..late_app() });
        assert!(watched.is_some());
        assert_eq!(w.restored, Some(user_clip()));
        assert_eq!(w.sleeps, 43, "two waits for the clipboard after the settle");

        let (_, watched, w) = copy_and_watch(World { busy_takes: 1, newer_begins_at: Some(42), ..late_app() });
        assert_eq!(watched, None);
        assert_eq!(w.restores, 0);
    }

    /// The clipboard is written again between the late change and the take-back: that write is left alone.
    #[test]
    fn a_write_after_the_late_copy_is_left_alone() {
        let (_, watched, w) = copy_and_watch(World { other_writes_at: Some(41), ..late_app() });
        assert_eq!(watched, None);
        assert_eq!(w.restored, None);
        assert_eq!(w.text.as_deref(), Some("чужое"));
    }

    /// What the guard does once it has taken the copy back — the start of the history removal — happens under the lock
    /// too: no newer operation can begin before it.
    #[test]
    fn the_history_removal_starts_under_the_lock() {
        let w = Rc::new(RefCell::new(late_app()));
        let ops = w.borrow().ops();
        let mut late = None;
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), |l| late = Some(l));
        let mut calls = 0;
        let caught = guard_late_copy(&mut Clip(w.clone()), &mut Keys(w.clone()), late.expect("watched for"), |copy| {
            calls += 1;
            (copy.text.clone(), ops.lock.try_lock().is_err())
        });
        let (_, (text, locked)) = caught.expect("caught");
        assert_eq!((calls, text.as_deref(), locked), (1, Some("поздно"), true));
    }

    /// With real threads: a newer operation that begins while a guard acts waits for it, and the guard, once done, is
    /// older — it would stand down from then on.
    #[test]
    fn a_newer_operation_begins_only_after_an_acting_guard_is_done() {
        use std::sync::{mpsc, Arc, Mutex};
        let ops: &'static Operations = Box::leak(Box::default());
        let first = ops.begin();
        let (inside_tx, inside) = mpsc::channel();
        let order = Arc::new(Mutex::new(Vec::new()));
        let guard_order = Arc::clone(&order);
        let guard = std::thread::spawn(move || {
            ops.while_newest(first, || {
                inside_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                guard_order.lock().unwrap().push("put back");
            })
        });
        inside.recv().unwrap();
        let newer = ops.begin();
        order.lock().unwrap().push("newer began");
        assert_eq!(guard.join().unwrap(), Some(()));
        assert_eq!(*order.lock().unwrap(), ["put back", "newer began"]);
        assert_eq!(newer, first + 1);
        assert_eq!(ops.while_newest(first, || ()), None, "an older guard stands down");
    }

    #[test]
    fn without_the_target_process_there_is_no_watch() {
        let (_, watched, w) = copy_and_watch(World { no_target: true, ..late_app() });
        assert_eq!(watched, None);
        assert_eq!(w.sleeps, 20, "the guard returns at once");
        assert_eq!(w.restored, None);
    }

    #[test]
    fn held_keys_or_an_unreadable_clipboard_mean_no_copy() {
        let (got, late, w) = copy(World { stuck: true, selection: Some("x".into()), ..Default::default() });
        assert_eq!((got.text, w.pressed_c, late), (None, false, false), "Ctrl + Win + Alt + C would be another shortcut");
        let (got, late, w) = copy(World { snapshot_fails: true, selection: Some("x".into()), ..Default::default() });
        assert_eq!((got.text, w.pressed_c, late), (None, false, false), "a clipboard that cannot be put back is not risked");
    }

    #[test]
    fn a_blank_copy_is_no_selection_but_the_clipboard_comes_back() {
        let (got, _, w) = copy(World { content: user_clip(), selection: Some(" \r\n".into()), ..Default::default() });
        assert_eq!(got.text, None);
        assert_eq!(w.restored, Some(user_clip()));
    }

    #[test]
    fn copy_debug_shows_lengths_not_text() {
        let (got, _, _) = copy(World { selection: Some("секретный текст".into()), ..Default::default() });
        let shown = format!("{got:?}");
        assert!(!shown.contains("секрет") && shown.contains("15"), "{shown}");
    }

    /// Decision 27: only the history item of this copy is removed — exactly its text, among the newest three, stamped no
    /// earlier than just before Ctrl+C — never an older one of the user's with the same text, pinned or not.
    #[test]
    fn only_the_fresh_history_item_with_the_copied_text_is_removed() {
        let since = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let fresh = history_time(since + Duration::from_millis(30));
        let item = |text: &str, at: i64| (Some(text.to_owned()), at);
        let items = [item("другое", fresh), (None, fresh), item("выделенный текст", fresh)];
        assert_eq!(copied_item(&items, "выделенный текст", since), Some(2));
        assert_eq!(copied_item(&items, "нет такого", since), None);
        let fourth = [(None, fresh), (None, fresh), (None, fresh), item("выделенный текст", fresh)];
        assert_eq!(copied_item(&fourth, "выделенный текст", since), None, "beyond the newest three");
        let older = [item("выделенный текст", history_time(since - Duration::from_secs(5)))];
        assert_eq!(copied_item(&older, "выделенный текст", since), None, "stamped before Ctrl+C: the user's own");
        let rounded = [item("выделенный текст", history_time(since - Duration::from_millis(500)))];
        assert_eq!(copied_item(&rounded, "выделенный текст", since), Some(0), "a history clock that rounds");
        let unstamped = [item("выделенный текст", i64::MIN)];
        assert_eq!(copied_item(&unstamped, "выделенный текст", since), None, "no stamp: not known to be this copy");
    }

    #[test]
    fn history_time_counts_from_1601_in_100_ns() {
        assert_eq!(history_time(UNIX_EPOCH), 116_444_736_000_000_000);
        assert_eq!(history_time(UNIX_EPOCH + Duration::from_micros(1)), 116_444_736_000_000_010);
        assert_eq!(history_time(UNIX_EPOCH - Duration::from_secs(1)), 116_444_735_990_000_000);
    }
}
