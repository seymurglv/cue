//! Paste through the clipboard: wait for the modifiers to be released → remember the clipboard → put the text
//! marked "not for history" → Ctrl+V → after 0.5 s restore the clipboard if nobody changed it. And the opposite way for
//! "Ask": the selection of an app that does not report it, copied with Ctrl+C and the clipboard put back.
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use crate::focus::WindowRef;

/// The clipboard's Unicode text format.
const CF_UNICODETEXT: u32 = 13;

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
    /// CUE's clipboard operations in this process (one count and one lock for every `WinClipboard`): a paste, a text
    /// left in the clipboard and a Ctrl+C copy each begin one, so that an older late-copy guard stands down
    /// (`guard_late_copy`).
    fn operations(&self) -> &'static Operations;
}

/// CUE's clipboard operations in this process — pastes, texts left in the clipboard and Ctrl+C copies: how many have
/// begun, and one lock. A late-copy guard acts under the lock, and only while its copy is still the newest operation;
/// `begin` takes the same lock. So a newer operation begins either before the guard acts, and the guard stands down, or
/// after it is done. The guard never waits for another app to let the clipboard go while it holds the lock
/// (`ReadClipboard::take_back`: one try), but under it the copying app may be asked to render its text
/// (`WM_RENDERFORMAT`) and to let its copy go (`WM_DESTROYCLIPBOARD`): a slow app keeps a newer `begin` waiting that
/// long, with no timeout. It happens at most once per copy, 0.4–2.4 s after its Ctrl+C, so a paste would have to begin,
/// or a text be left in the clipboard, at that very moment.
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
    /// No paste happened because the window in front may not take this text (`paste_text_if`'s `allowed`): the text is
    /// left in the clipboard the ordinary way, as with `LeftInClipboard`.
    NotAllowed,
}

/// `on_pasted` is called once right after Ctrl+V, before the wait and the restore — this is the "done" moment.
/// Without a paste (`LeftInClipboard`, `NotAllowed`, error) it is not called. `Err` — the text could not be put in the
/// clipboard, no paste happened.
pub fn paste_text(
    clip: &mut impl ClipboardApi,
    keys: &mut impl KeysApi,
    text: &str,
    on_pasted: impl FnOnce(),
) -> Result<PasteResult, String> {
    paste_text_if(clip, keys, text, || true, on_pasted)
}

/// Whether an "Ask" paste may go into the window in front now (spec 2 §6.2): it is still `target`, the window the
/// question was asked in — the same check as before the Ctrl+C fallback's Ctrl+C (`WindowRef::same_window`) — and, for a
/// text that starts with a line break or spans lines, it is not a terminal, where a line break is Enter. A write lands
/// seconds after the question, and the user may have moved on to another window by then.
pub fn may_paste(keys: &impl CopyKeys, target: WindowRef, text: &str) -> bool {
    let spans_lines = text.contains(['\n', '\r', '\u{0B}', '\u{0C}', '\u{85}', '\u{2028}', '\u{2029}']);
    keys.foreground().is_some_and(|front| front.same_window(target)) && !(spans_lines && keys.terminal(target))
}

/// `paste_text`, with `allowed` asked once more right before Ctrl+V, the clipboard already holding the text: `false` —
/// no Ctrl+V, the text is left in the clipboard the ordinary way (`NotAllowed`).
pub fn paste_text_if(
    clip: &mut impl ClipboardApi,
    keys: &mut impl KeysApi,
    text: &str,
    allowed: impl FnOnce() -> bool,
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
    // The last moment before Ctrl+V: the window in front may have changed while the keys were held.
    if !allowed() {
        clip.write_text(&text, false)?;
        return Ok(PasteResult::NotAllowed);
    }
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

/// The text left in the clipboard the ordinary way, without marks, for the user to paste themselves.
pub fn leave_text(clip: &mut impl ClipboardApi, text: &str) -> Result<(), String> {
    // A clipboard operation of its own: once CUE's owner window is gone this write has no owner, just like an app's
    // copy with none, and a late-copy guard of an earlier Ctrl+C must not take it for one (`the_apps_copy`).
    clip.operations().begin();
    clip.write_text(text, false)
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

/// Whether Windows uploads text copied on this computer to the cloud clipboard by itself, from the user's two values
/// under `HKCU\Software\Microsoft\Clipboard` (Settings → System → Clipboard): `EnableCloudClipboard` (1: "Clipboard
/// history across your devices" on; missing: off, the default) and `CloudClipboardAutomaticUpload` (1: "Automatically
/// sync text that I copy", 0: "Manually sync text that I copy"). A missing upload value with sync on counts as automatic:
/// what Windows does then is not documented, and a selection must not reach other devices on a guess.
pub fn uploads_to_cloud(sync: Option<u32>, automatic: Option<u32>) -> bool {
    sync.is_some_and(|on| on != 0) && automatic != Some(0)
}

/// Reading the clipboard for the Ctrl+C fallback of "Ask".
pub trait ReadClipboard {
    /// The process of the window that owns the clipboard: the last one to write it. `None` — no owner, or not known.
    fn owner_process(&self) -> Option<u32>;
    /// The app's copy taken back, with one opening of the clipboard and no wait for it (the late-copy guard runs it
    /// under the operations lock): if the clipboard may still be the copy of `target`'s app (`the_apps_copy`) — its
    /// owner checked while it is open, so nobody can write in between — its text is read and `snap` put back.
    fn take_back(&mut self, target: WindowRef, snap: &Snapshot) -> TakeBack;
}

/// Whether a clipboard owned by `owner` (`ReadClipboard::owner_process`) may hold the answer of `target`'s app to
/// CUE's Ctrl+C: when the owner is that window's process, or the process of the app in it when the window is only a
/// frame — a Store app's copy comes from one of the app's own windows, not the frame host's — or when there is no owner
/// at all. An app may copy with no window (`OpenClipboard(NULL)`), and such a copy, appearing in the wait right after
/// Ctrl+C into the window just checked, is that app's in practice; a clipboard listener re-rendering the user's own
/// clipboard with no owner is caught by the identical-text rule (`caught`). Any other process's write is a copy the
/// user may want, and stays.
fn the_apps_copy(target: WindowRef, owner: Option<u32>) -> bool {
    owner.is_none_or(|owner| owner == target.process || Some(owner) == target.app_process)
}

/// What `ReadClipboard::take_back` did.
pub enum TakeBack {
    /// Another app held the clipboard open: nothing was done; worth another try.
    Busy,
    /// CUE's own owner window could not be made: nothing was done, and another try would fail the same way.
    NoWindow(String),
    /// The clipboard is another process's (`the_apps_copy`): another app wrote it. Nothing was done.
    Other,
    /// The text that was in it, and whether the snapshot went back.
    Done { text: Option<String>, restored: Result<(), String> },
}

/// Pressing Ctrl+C for the Ctrl+C fallback of "Ask".
pub trait CopyKeys {
    /// `false` — the system did not accept all presses.
    fn press_ctrl_c(&mut self) -> bool;
    /// The window the keys go to — the foreground one — and its process. `None` — no foreground window.
    fn foreground(&self) -> Option<WindowRef>;
    /// The window is a terminal, by its class or its executable (`terminal::terminal_window`).
    fn terminal(&self, window: WindowRef) -> bool;
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
    /// The window Ctrl+C went to: the one the selection was read in.
    target: WindowRef,
    since: SystemTime,
    /// The copy's clipboard operation (`Operations::begin`).
    operation: u64,
    /// The last input once CUE's own Ctrl+C had gone through (`CopyKeys::last_input`): any later is the user's.
    input_mark: Option<u32>,
}

/// The Ctrl+C fallback of "Ask" (spec 2 §6.1): the selection of an app that does not report it, copied and read, and the
/// user's clipboard put back as it was, with the marks of a paste (not in the Win+V history again). Only into `target`,
/// the window the selection was read in at the chord, and never into a terminal: the caller decides from the app named
/// at the chord (`selection::needs_copy_fallback`), and right before Ctrl+C the window in front is looked at again.
///
/// What becomes of the user's clipboard:
/// - the chord's keys stayed down, the clipboard could not be read first, or the window in front is no longer `target`
///   (or is a terminal): Ctrl+C is not pressed, it is not touched;
/// - the app copied within `COPY_WAIT`: it is put back; if that fails, `restore_failed` says so, and the app's copy is
///   left in it. The app's copy is a write owned by its window's process, by the app in it when that window is a
///   Store app's frame, or by no window (`the_apps_copy`). A change by any other process — another app's copy, a
///   clipboard listener rendering a format of the user's own clipboard — is never taken for the selection, and stays;
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
    target: WindowRef,
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
    // The last moment before Ctrl+C: the user may have switched windows since the chord, to a terminal even.
    if !keys.foreground().is_some_and(|front| front.same_window(target)) || keys.terminal(target) {
        tracing::info!("ask: the window in front is not the one read at the chord, or is a terminal: no Ctrl+C");
        return nothing(SystemTime::now());
    }
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
    for attempt in 0..TAKE_BACK_TRIES {
        if attempt > 0 {
            keys.sleep(TAKE_BACK_RETRY);
        }
        // Only a write that may be the app's is its answer to Ctrl+C: the user's clipboard never becomes "the selection".
        match clip.take_back(target, &snap) {
            TakeBack::Busy => {}
            TakeBack::Done { text, restored } => return caught(text, restored, since, &snap),
            TakeBack::Other => {
                tracing::info!("ask: the clipboard changed after Ctrl+C, but by another app; left alone");
                return nothing(since);
            }
            TakeBack::NoWindow(e) => return not_put_back(&e, since),
        }
    }
    not_put_back("the clipboard stayed held by another app", since)
}

/// The app copied, but its copy could not be read and the user's clipboard put back: the copy is left in the clipboard.
fn not_put_back(why: &str, since: SystemTime) -> SelectionCopy {
    tracing::warn!(error = %why, "ask: the user's clipboard was not put back after the copy");
    SelectionCopy { text: None, since, restore_failed: true }
}

/// After `copy_selection` handed a `LateCopy` over, on a thread of its own: the clipboard is watched from the end of
/// `COPY_WAIT` for `LATE_COPY_WAIT` — 0.4 to 2.4 s after CUE's Ctrl+C. The first write in that time is taken for the
/// app's answer to that Ctrl+C — the user's clipboard put back, the copy returned, and `on_caught` run with it (the start
/// of its removal from the Win+V history) — only when it looks like one:
/// - its owner may be the app's (`the_apps_copy`): the process of the window Ctrl+C went to, the app in it when that
///   window is a frame, or none (another app's write is a newer copy the user may want);
/// - there was no input at all since CUE's Ctrl+C went through — no key, click or mouse move: the user's own Ctrl+C,
///   Shift+Delete or a menu's Copy in the same app would look just the same otherwise;
/// - the clipboard was not written again after it by another process: its owner is checked once more, with the
///   clipboard open, right before the take-back. A second write of the app's own, or a second one with no owner, in
///   between is taken back like the first: the owner is all that tells writes apart;
/// - no newer clipboard operation of CUE's has begun (a paste, a text left in the clipboard, the next Ctrl+C copy): the
///   clipboard is that one's. The last of these checks, the restore and `on_caught` happen under the operations lock
///   (`Operations::while_newest`), so none begins in between.
///
/// Otherwise the clipboard is left alone and the watch ends. `None` — nothing was put back. The owner is checked again
/// with the clipboard open, right before the take-back: the number may have moved since the poll without a new copy
/// (a second write of the app's, a listener rendering a format), and another process's write in between stays.
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
    let target = late.target;
    let operations = clip.operations();
    let mut waited = Duration::ZERO;
    loop {
        if operations.newest() != late.operation {
            return None;
        }
        if clip.sequence() != late.before {
            break;
        }
        if waited >= LATE_COPY_WAIT {
            return None;
        }
        keys.sleep(COPY_POLL);
        waited += COPY_POLL;
    }
    if !the_apps_copy(target, clip.owner_process()) {
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
            match clip.take_back(target, &late.snap) {
                TakeBack::Busy => Act::Retry,
                TakeBack::Other => {
                    tracing::info!("ask: the clipboard was written again after the late copy; left alone");
                    Act::Leave
                }
                TakeBack::NoWindow(e) => {
                    tracing::warn!(error = %e, "ask: a late copy was not put back");
                    Act::Leave
                }
                TakeBack::Done { text, restored } => {
                    let copy = caught(text, restored, late.since, &late.snap);
                    let then = on_caught(&copy);
                    Act::Done((copy, then))
                }
            }
        });
        match acted {
            Some(Act::Retry) => {}
            Some(Act::Done(done)) => return Some(done),
            // A newer operation of CUE's has begun, there was input, or the clipboard is not the app's any more.
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

/// The app's copy as taken: its text, unless blank or the very text the user had in the clipboard before Ctrl+C
/// (`saved`): a clipboard listener re-rendering the user's own clipboard moves the number with the owner unchanged, and
/// that must never become "the selection". A selection of exactly that text is lost as context, accepted. A failed
/// restore is logged, without text.
fn caught(text: Option<String>, restored: Result<(), String>, since: SystemTime, saved: &Snapshot) -> SelectionCopy {
    let restore_failed = match restored {
        Ok(()) => false,
        Err(e) => {
            tracing::warn!(error = %e, "ask: the user's clipboard was not put back after the copy");
            true
        }
    };
    let before = saved_text(saved);
    let text = text.filter(|t| !t.trim().is_empty() && Some(t) != before.as_ref());
    SelectionCopy { text, since, restore_failed }
}

/// The text of a saved clipboard (`CF_UNICODETEXT`, UTF-16 up to its null); `None` — there was none.
fn saved_text(snap: &Snapshot) -> Option<String> {
    let (_, bytes) = snap.items.iter().find(|(fmt, _)| *fmt == CF_UNICODETEXT)?;
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|&b| u16::from_le_bytes(b)).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&units))
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
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetForegroundWindow, GetWindowThreadProcessId, HWND_MESSAGE, WINDOW_EX_STYLE,
        WINDOW_STYLE,
    };

    use super::{
        CF_UNICODETEXT, ClipboardApi, HistoryForget, KeysApi, LateCopy, Operations, Snapshot, TakeBack, WindowRef,
    };

    /// CUE's clipboard operations in this process: every `WinClipboard` counts here, the late-copy guard's own too.
    static OPERATIONS: Operations = Operations::new();

    const CF_DIB: u32 = 8;
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
        open_with(Owner::new()?, tries).ok_or_else(|| "the clipboard is held by another app".into())
    }

    /// The clipboard opened with `owner`, in up to `tries` attempts; `None` — another app kept it open.
    fn open_with(owner: Owner, tries: u32) -> Option<Open> {
        for attempt in 0..tries {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(50));
            }
            // SAFETY: `owner` is a live window of this thread.
            if unsafe { OpenClipboard(Some(owner.0)) }.is_ok() {
                return Some(Open { _owner: owner });
            }
        }
        None
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
        fn take_back(&mut self, target: WindowRef, snap: &Snapshot) -> TakeBack {
            let window = match Owner::new() {
                Ok(window) => window,
                Err(e) => return TakeBack::NoWindow(e),
            };
            // One try, no wait: the guard holds the operations lock, and waits outside it.
            let Some(_open) = open_with(window, 1) else { return TakeBack::Busy };
            // Open, the clipboard cannot be written by anyone else: the owner checked now holds until it is closed.
            // Opening it does not make CUE its owner; only a write would.
            if !super::the_apps_copy(target, self.owner_process()) {
                return TakeBack::Other;
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

        fn foreground(&self) -> Option<WindowRef> {
            // SAFETY: no arguments.
            let window = unsafe { GetForegroundWindow() };
            window_process(window).map(|process| WindowRef { window: window.0 as isize, process, app_process: None })
        }

        fn terminal(&self, window: WindowRef) -> bool {
            let (class, exe) = crate::focus::window_kind(window);
            crate::terminal::terminal_window(&class, &exe)
        }

        fn last_input(&self) -> Option<u32> {
            let mut info = LASTINPUTINFO { cbSize: size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
            // SAFETY: `info` with `cbSize` filled in lives until the end of the call.
            unsafe { GetLastInputInfo(&mut info) }.as_bool().then_some(info.dwTime)
        }
    }

    /// `super::uploads_to_cloud` with the user's two values as the registry has them now: two short reads, one per "Ask"
    /// chord. A value that cannot be read counts as missing.
    pub fn cloud_clipboard_uploads() -> bool {
        super::uploads_to_cloud(clipboard_value(w!("EnableCloudClipboard")), clipboard_value(w!("CloudClipboardAutomaticUpload")))
    }

    /// A DWORD under `HKCU\Software\Microsoft\Clipboard`; `None` — missing, of another type, or not readable.
    fn clipboard_value(name: PCWSTR) -> Option<u32> {
        let mut value: u32 = 0;
        let mut len = size_of::<u32>() as u32;
        // SAFETY: the key and the name are `w!` constants; the data goes into our own `u32`, `len` is its size, and
        // RRF_RT_REG_DWORD lets nothing larger be written.
        let read = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Clipboard"),
                name,
                RRF_RT_REG_DWORD,
                None,
                Some((&raw mut value).cast()),
                Some(&mut len),
            )
        };
        read.is_ok().then_some(value)
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
pub use win::{cloud_clipboard_uploads, forget_in_history, spawn_late_guard, WinClipboard, WinKeys};

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

    /// Only sync on with automatic upload — or with no upload value at all — sends copied text to the cloud by itself.
    #[test]
    fn copied_text_goes_to_the_cloud_only_with_sync_and_automatic_upload() {
        for (sync, automatic, want) in [
            (None, None, false),
            (None, Some(1), false),
            (Some(0), Some(1), false),
            (Some(1), Some(1), true),
            (Some(1), Some(0), false),
            (Some(1), None, true),
            (Some(2), Some(2), true),
        ] {
            assert_eq!(uploads_to_cloud(sync, automatic), want, "{sync:?} {automatic:?}");
        }
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

    /// The app Ctrl+C goes to, another one, and the frame host of Store apps.
    const APP: u32 = 100;
    const OTHER: u32 = 200;
    const FRAME_HOST: u32 = 400;
    /// The window the selection was read in at the chord, and another one.
    const TARGET: WindowRef = WindowRef { window: 0x10, process: APP, app_process: None };
    const ELSEWHERE: WindowRef = WindowRef { window: 0x20, process: OTHER, app_process: None };
    /// A Store app's window as read at the chord: the frame, and the app in it, known by the focused element.
    const FRAMED: WindowRef = WindowRef { window: 0x30, process: FRAME_HOST, app_process: Some(APP) };

    /// The clipboard, the app behind it and the user, counted in sleeps: on Ctrl+C the app copies `selection` (if any)
    /// after `copy_after` sleeps, owned by its own process or by `copy_owner` (`Some(None)`: by no window); another app
    /// writes the clipboard at sleep `other_writes_at`; a clipboard listener renders a format of what is there at
    /// `render_at` (the number moves, the owner stays); the user copies in the app at `user_copies_at`, or just types or
    /// moves the mouse at `user_input_at`; a newer clipboard operation of CUE's begins at `newer_begins_at`; the
    /// clipboard is held open by another app for the first `busy_takes` take-backs. The window read at the chord is a
    /// Store app's frame once `framed` (`FRAMED`); another window is in front once `moved`, none once `gone`; the window
    /// is a `terminal`; the last input cannot be read once `no_input_info`; CUE's own owner window cannot be made once
    /// `no_owner_window`.
    #[derive(Default)]
    struct World {
        seq: u32,
        content: Snapshot,
        text: Option<String>,
        owner: Option<u32>,
        selection: Option<String>,
        copy_after: u32,
        copy_owner: Option<Option<u32>>,
        framed: bool,
        other_writes_at: Option<u32>,
        render_at: Option<u32>,
        user_copies_at: Option<u32>,
        user_input_at: Option<u32>,
        newer_begins_at: Option<u32>,
        busy_takes: u32,
        take_backs: u32,
        input_tick: u32,
        moved: bool,
        gone: bool,
        terminal: bool,
        no_input_info: bool,
        no_owner_window: bool,
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

        /// The window Ctrl+C is meant for, as read at the chord.
        fn target(&self) -> WindowRef {
            if self.framed { FRAMED } else { TARGET }
        }

        fn app_copies_now(&mut self) {
            if let Some(s) = self.selection.clone() {
                self.text = Some(s);
                self.owner = self.copy_owner.unwrap_or(Some(APP));
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
            // CUE's owner window is gone once the write is done: the clipboard is left with no owner.
            w.owner = None;
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
        fn owner_process(&self) -> Option<u32> {
            self.0.borrow().owner
        }
        fn take_back(&mut self, target: WindowRef, snap: &Snapshot) -> TakeBack {
            let mut w = self.0.borrow_mut();
            w.take_backs += 1;
            if w.no_owner_window {
                return TakeBack::NoWindow("CreateWindowExW: refused".into());
            }
            if w.busy_takes > 0 {
                w.busy_takes -= 1;
                return TakeBack::Busy;
            }
            if !the_apps_copy(target, w.owner) {
                return TakeBack::Other;
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
            if w.render_at == now {
                w.seq += 1;
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
        fn foreground(&self) -> Option<WindowRef> {
            let w = self.0.borrow();
            if w.gone {
                None
            } else if w.moved {
                Some(ELSEWHERE)
            } else if w.framed {
                // As the system tells it: the frame and its host's process, with nothing known of the app in it.
                Some(WindowRef { app_process: None, ..FRAMED })
            } else {
                Some(TARGET)
            }
        }
        fn terminal(&self, _window: WindowRef) -> bool {
            self.0.borrow().terminal
        }
        fn last_input(&self) -> Option<u32> {
            let w = self.0.borrow();
            (!w.no_input_info).then_some(w.input_tick)
        }
    }

    fn user_clip() -> Snapshot {
        Snapshot { items: vec![(13, vec![65, 0, 0, 0])] }
    }

    /// An "Ask" paste on `w`, checked against the window read at the chord as the insert thread checks it.
    fn ask_paste(w: World, text: &str) -> (PasteResult, World) {
        let w = Rc::new(RefCell::new(w));
        let keys = Keys(w.clone());
        let allowed = || may_paste(&keys, TARGET, text);
        let got = paste_text_if(&mut Clip(w.clone()), &mut Keys(w.clone()), text, allowed, || {}).unwrap();
        drop(keys);
        (got, Rc::try_unwrap(w).ok().unwrap().into_inner())
    }

    /// Ruling 6b, for a paste too: only into the window the question was asked in, and never a text that spans lines (or
    /// starts with a line break) into a terminal. Otherwise nothing is pasted, and the text stays in the clipboard.
    #[test]
    fn an_ask_paste_goes_only_into_the_chords_window_and_no_line_break_into_a_terminal() {
        let (got, w) = ask_paste(World { content: user_clip(), ..Default::default() }, "ответ");
        assert_eq!((got, w.ctrl_v), (PasteResult::Pasted { restored: true }, 1));
        for (world, text) in [
            (World { moved: true, ..Default::default() }, "ответ"),
            (World { gone: true, ..Default::default() }, "ответ"),
            (World { terminal: true, ..Default::default() }, "git status\ngit diff"),
            (World { terminal: true, ..Default::default() }, "\ngit status"),
        ] {
            let (got, w) = ask_paste(World { content: user_clip(), ..world }, text);
            assert_eq!((got, w.ctrl_v), (PasteResult::NotAllowed, 0), "{text:?}");
            assert_eq!(w.text.as_deref(), Some(crlf(text).as_str()), "left in the clipboard for the user");
            assert_eq!(w.restored, None, "nothing to put back over it");
        }
        let (got, w) = ask_paste(World { terminal: true, content: user_clip(), ..Default::default() }, "git status");
        assert_eq!((got, w.ctrl_v), (PasteResult::Pasted { restored: true }, 1), "one line into a terminal is a paste");
    }

    /// The world after a copy and its guard: neither ever pastes or writes the clipboard itself.
    fn world(w: Rc<RefCell<World>>) -> World {
        let w = Rc::try_unwrap(w).ok().unwrap().into_inner();
        assert_eq!((w.ctrl_v, w.writes.len()), (0, 0), "the copy and the guard never paste or write");
        w
    }

    /// `copy_selection` on `w`, and whether it handed a late copy over.
    fn copy(w: World) -> (SelectionCopy, bool, World) {
        let target = w.target();
        let w = Rc::new(RefCell::new(w));
        let mut late = false;
        let got = copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), target, |_| late = true);
        (got, late, world(w))
    }

    /// The same, and the late copy it handed over watched right after (in the app: on a thread of its own).
    fn copy_and_watch(w: World) -> (SelectionCopy, Option<SelectionCopy>, World) {
        let target = w.target();
        let w = Rc::new(RefCell::new(w));
        let mut late = None;
        let got = copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), target, |l| late = Some(l));
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
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), TARGET, |l| late = Some(l));
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
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), TARGET, |l| late = Some(l));
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), TARGET, |_| {});
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
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), TARGET, |l| late = Some(l));
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

    fn selected() -> World {
        World { content: user_clip(), selection: Some("выделенный текст".into()), ..Default::default() }
    }

    /// Ctrl+C goes only to the window the selection was read in at the chord (spec 2 §6.1): once another window is in
    /// front, or none, or the window turns out to be a terminal (Ctrl+C there stops the running program), nothing is
    /// pressed, the clipboard is not touched, and the question goes without a selection.
    #[test]
    fn ctrl_c_goes_only_to_the_chords_window_and_never_to_a_terminal() {
        for (w, why) in [
            (World { moved: true, ..selected() }, "another window in front"),
            (World { gone: true, ..selected() }, "no window in front"),
            (World { terminal: true, ..selected() }, "a terminal"),
        ] {
            let (got, late, w) = copy(w);
            assert_eq!((got.text, w.pressed_c, late, w.restored), (None, false, false, None), "{why}");
        }
    }

    /// Only the app's own write is its answer to Ctrl+C. Another app's copy, or a clipboard listener rendering a format
    /// of the user's own clipboard, moves the number too: never taken for the selection, and left alone, unwatched.
    #[test]
    fn a_change_that_is_not_the_apps_is_never_taken_for_the_selection() {
        let (got, late, w) = copy(World { content: user_clip(), other_writes_at: Some(3), ..Default::default() });
        assert_eq!((got.text, late, w.restored), (None, false, None));
        assert_eq!(w.text.as_deref(), Some("чужое"), "the other app's copy stays");

        let users = World { text: Some("своё".into()), owner: Some(OTHER), render_at: Some(3), ..World::default() };
        let (got, late, w) = copy(World { content: user_clip(), ..users });
        assert_eq!((got.text, late, w.restored), (None, false, None), "the user's own clipboard is not the selection");
    }

    /// The number may move again after the app's write — a second write of its own, or a listener rendering a format —
    /// and the copy is still the app's: its owner, checked with the clipboard open, is what counts.
    #[test]
    fn the_apps_copy_is_taken_though_the_number_moves_again() {
        let (got, _, w) = copy(World { render_at: Some(1), ..selected() });
        assert_eq!(got.text.as_deref(), Some("выделенный текст"));
        assert_eq!(w.restored, Some(user_clip()));
        let (_, watched, w) = copy_and_watch(World { render_at: Some(41), ..late_app() });
        assert_eq!(watched.and_then(|c| c.text).as_deref(), Some("поздно"));
        assert_eq!(w.restored, Some(user_clip()));
    }

    /// The number moved with the owner one a copy of the app's may have — the target, or none — but the text is the one
    /// the user had before Ctrl+C: a listener re-rendering the user's own clipboard, not a copy. Never the selection —
    /// on the main path or a late one.
    #[test]
    fn the_users_own_text_moved_again_is_never_the_selection() {
        for owner in [Some(APP), None] {
            let own = || World { content: user_clip(), text: Some("A".into()), owner, ..World::default() };
            let (got, late, w) = copy(World { render_at: Some(3), ..own() });
            assert_eq!((got.text, late, got.restore_failed), (None, false, false), "owner {owner:?}");
            assert_eq!(w.restored, Some(user_clip()), "the user's clipboard stays as it was, owner {owner:?}");

            let (got, watched, _) = copy_and_watch(World { render_at: Some(30), ..own() });
            assert_eq!(got.text, None);
            assert_eq!(watched.map(|c| c.text), Some(None), "a late move of the same text is no copy either, {owner:?}");
        }

        assert_eq!(saved_text(&user_clip()).as_deref(), Some("A"), "the saved text, up to its null");
        assert_eq!(saved_text(&Snapshot::default()), None);
    }

    fn framed() -> World {
        World { framed: true, ..selected() }
    }

    /// A Store app's window is a frame of another process (`ApplicationFrameHost`), and its copy is owned by a window of
    /// the app's own process, or of the frame's: either is its answer to Ctrl+C — taken, and the user's clipboard put
    /// back — on the main path and a late one. Ctrl+C still goes only to the frame read at the chord.
    #[test]
    fn a_store_apps_copy_is_taken_from_the_app_or_its_frame() {
        for owner in [APP, FRAME_HOST] {
            let (got, _, w) = copy(World { copy_owner: Some(Some(owner)), ..framed() });
            assert_eq!(got.text.as_deref(), Some("выделенный текст"), "owned by {owner}");
            assert_eq!(w.restored, Some(user_clip()), "the user's clipboard is back, owned by {owner}");

            let (_, watched, w) = copy_and_watch(World { framed: true, copy_owner: Some(Some(owner)), ..late_app() });
            assert_eq!(watched.and_then(|c| c.text).as_deref(), Some("поздно"), "late, owned by {owner}");
            assert_eq!(w.restored, Some(user_clip()), "late, owned by {owner}");
        }
        let (got, late, w) = copy(World { moved: true, ..framed() });
        assert_eq!((got.text, w.pressed_c, late), (None, false, false), "another window in front");
    }

    /// An app may copy with no window of its own (`OpenClipboard(NULL)`): the clipboard has no owner. Right after Ctrl+C
    /// into the window just checked, that is the app's copy — taken, and the user's clipboard put back — on the main path
    /// and a late one.
    #[test]
    fn a_copy_with_no_owner_is_the_apps() {
        let (got, _, w) = copy(World { copy_owner: Some(None), ..selected() });
        assert_eq!(got.text.as_deref(), Some("выделенный текст"));
        assert_eq!(w.restored, Some(user_clip()));

        let (_, watched, w) = copy_and_watch(World { copy_owner: Some(None), ..late_app() });
        assert_eq!(watched.and_then(|c| c.text).as_deref(), Some("поздно"));
        assert_eq!(w.restored, Some(user_clip()));
    }

    /// A copy owned by any other process — a helper of the app's, another app — is still not the answer to Ctrl+C, a
    /// Store app's frame or not: never taken, and left in the clipboard, on the main path and a late one.
    #[test]
    fn a_copy_of_another_process_is_still_left_alone() {
        for framed in [true, false] {
            let (got, late, w) = copy(World { framed, copy_owner: Some(Some(OTHER)), ..selected() });
            assert_eq!((got.text, late, w.restored), (None, false, None), "framed: {framed}");
            assert_eq!(w.text.as_deref(), Some("выделенный текст"), "the copy stays, framed: {framed}");

            let (_, watched, w) = copy_and_watch(World { framed, copy_owner: Some(Some(OTHER)), ..late_app() });
            assert_eq!((watched, w.restored), (None, None), "late, framed: {framed}");
            assert_eq!(w.text.as_deref(), Some("поздно"), "the late copy stays, framed: {framed}");
        }
    }

    /// CUE leaves a text in the clipboard (no field) while a late-copy guard still watches: once CUE's window is gone
    /// that write has no owner, like an app's copy with none, so it must stand the guard down — the text stays, and is
    /// never taken for a late copy (nor removed from Win+V).
    #[test]
    fn a_text_left_in_the_clipboard_stands_a_late_guard_down() {
        let w = Rc::new(RefCell::new(World { content: user_clip(), ..World::default() }));
        let mut late = None;
        copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()), TARGET, |l| late = Some(l));
        leave_text(&mut Clip(w.clone()), "ответ").unwrap();
        let watched = guard_late_copy(&mut Clip(w.clone()), &mut Keys(w.clone()), late.expect("watched for"), |_| ());
        assert_eq!(watched, None);
        let w = w.borrow();
        assert_eq!((w.restores, w.text.as_deref(), w.writes.len()), (0, Some("ответ"), 1));
    }

    /// Another app holds the clipboard open when the app's copy is to be read: tried again; held for good, the copy is
    /// left in the clipboard, and that is reported as a restore that failed.
    #[test]
    fn a_busy_clipboard_after_the_copy_is_tried_again() {
        let (got, _, w) = copy(World { busy_takes: 2, ..selected() });
        assert_eq!((got.text.as_deref(), w.restored), (Some("выделенный текст"), Some(user_clip())));
        let (got, _, w) = copy(World { busy_takes: 100, ..selected() });
        assert_eq!((got.text, got.restore_failed, w.restored), (None, true, None));
        assert_eq!(w.take_backs, 10);
    }

    /// The last input cannot be read: the user's own input cannot be ruled out, so the guard stands down.
    #[test]
    fn an_unreadable_last_input_stands_the_guard_down() {
        let (_, watched, w) = copy_and_watch(World { no_input_info: true, ..late_app() });
        assert_eq!((watched, w.restored), (None, None));
    }

    /// CUE's own owner window cannot be made: not a clipboard another app holds, so not tried ten times over.
    #[test]
    fn no_owner_window_is_its_own_failure_not_a_busy_clipboard() {
        let (_, watched, w) = copy_and_watch(World { no_owner_window: true, ..late_app() });
        assert_eq!((watched, w.restored, w.take_backs), (None, None, 1));
        let (got, _, w) = copy(World { no_owner_window: true, ..selected() });
        assert_eq!((got.text, got.restore_failed, w.take_backs), (None, true, 1));
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
