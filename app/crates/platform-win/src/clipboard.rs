//! Paste through the clipboard: wait for the modifiers to be released → remember the clipboard → put the text
//! marked "not for history" → Ctrl+V → after 0.5 s restore the clipboard if nobody changed it. And the opposite way for
//! "Ask": the selection of an app that does not report it, copied with Ctrl+C and the clipboard put back.
use std::time::Duration;

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

/// Reading the clipboard's text (the Ctrl+C fallback of "Ask").
pub trait ReadClipboard {
    /// `None` — no text in the clipboard.
    fn read_text(&mut self) -> Result<Option<String>, String>;
}

/// Pressing Ctrl+C (the Ctrl+C fallback of "Ask").
pub trait CopyKeys {
    /// `false` — the system did not accept all presses.
    fn press_ctrl_c(&mut self) -> bool;
}

/// How long the chord's keys may stay down before the copy gives up: Ctrl + Win + Alt + C would be another shortcut.
const COPY_KEYS_WAIT: Duration = Duration::from_millis(1_500);
/// How long the app has to copy after Ctrl+C.
const COPY_WAIT: Duration = Duration::from_millis(400);
const COPY_POLL: Duration = Duration::from_millis(20);

/// The Ctrl+C fallback of "Ask" (spec 2 §6.1): the selection of an app that does not report it, copied and read, and the
/// user's clipboard put back as it was, with the marks of a paste (not in the Win+V history again). Never in a terminal
/// (the caller decides, `selection::needs_copy_fallback`). `None` — nothing was copied (no selection, the app ignores
/// Ctrl+C, the keys stayed down): the clipboard was not touched. The app's own copy goes into the Win+V history; CUE cannot
/// mark it, and tries to remove it afterwards (`forget_in_history`, decision 27).
pub fn copy_selection(clip: &mut (impl ClipboardApi + ReadClipboard), keys: &mut (impl KeysApi + CopyKeys)) -> Option<String> {
    if !keys.wait_modifiers_released(COPY_KEYS_WAIT) {
        return None;
    }
    // Not read — not risked: an empty "previous" clipboard would wipe the real one.
    let snap = clip.snapshot().ok()?;
    let before = clip.sequence();
    if !keys.press_ctrl_c() {
        return None;
    }
    let mut waited = Duration::ZERO;
    while clip.sequence() == before {
        if waited >= COPY_WAIT {
            return None;
        }
        keys.sleep(COPY_POLL);
        waited += COPY_POLL;
    }
    // An app that writes several formats may put the text a moment after the first one.
    keys.sleep(COPY_POLL);
    let text = clip.read_text().ok().flatten();
    let _ = clip.restore(&snap);
    text.filter(|t| !t.trim().is_empty())
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

/// Which of the newest history items (newest first) holds exactly the copied text. Only the first three are looked at: an
/// older item of the user's with the same text is never removed.
pub fn copied_item(texts: &[Option<String>], copied: &str) -> Option<usize> {
    texts.iter().take(3).position(|t| t.as_deref() == Some(copied))
}

#[cfg(windows)]
mod win {
    use std::time::Duration;

    use windows::core::{w, PCWSTR};
    use windows::ApplicationModel::DataTransfer::{Clipboard, ClipboardHistoryItemsResultStatus, StandardDataFormats};
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL, HWND};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    };

    use super::{ClipboardApi, KeysApi, Snapshot};

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
        let owner = Owner::new()?;
        for attempt in 0..10 {
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
            // SAFETY: the clipboard is open by this thread (`_open`).
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
            // SAFETY: the clipboard is open by this thread (`_open`); CF_UNICODETEXT data is an HGLOBAL owned by the system,
            // read only under GlobalLock and no further than GlobalSize.
            unsafe {
                if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
                    return Ok(None);
                }
                let Ok(h) = GetClipboardData(CF_UNICODETEXT) else { return Ok(None) };
                let g = HGLOBAL(h.0);
                let p = GlobalLock(g).cast::<u16>().cast_const();
                if p.is_null() {
                    return Ok(None);
                }
                let units = GlobalSize(g) / 2;
                let all = std::slice::from_raw_parts(p, units);
                let end = all.iter().position(|&c| c == 0).unwrap_or(units);
                let text = String::from_utf16_lossy(&all[..end]);
                let _ = GlobalUnlock(g);
                Ok(Some(text))
            }
        }
    }

    impl super::CopyKeys for WinKeys {
        fn press_ctrl_c(&mut self) -> bool {
            crate::input::press_ctrl_c()
        }
    }

    /// Decision 27: the selection the Ctrl+C fallback copied leaves the Win+V history, if Windows lets CUE remove it. On a
    /// thread of its own (WinRT, a few hundred ms; the history service takes the item a moment after the copy); the outcome
    /// goes to the log as a kind, never the text.
    pub fn forget_in_history(copied: String) -> Option<std::thread::JoinHandle<super::HistoryForget>> {
        let spawned = std::thread::Builder::new().name("cue-clip-history".into()).spawn(move || {
            // SAFETY: a COM apartment for this thread only (WinRT needs one); it ends with the thread.
            let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let mut outcome = super::HistoryForget::NotFound;
            for _ in 0..3 {
                std::thread::sleep(Duration::from_millis(300));
                outcome = forget_once(&copied).unwrap_or(super::HistoryForget::Refused);
                if outcome != super::HistoryForget::NotFound {
                    break;
                }
            }
            tracing::info!(?outcome, "ask: the copied selection in the clipboard history");
            outcome
        });
        spawned.inspect_err(|e| tracing::warn!("clipboard history thread not started: {e}")).ok()
    }

    fn forget_once(copied: &str) -> windows::core::Result<super::HistoryForget> {
        let result = Clipboard::GetHistoryItemsAsync()?.join()?;
        if result.Status()? != ClipboardHistoryItemsResultStatus::Success {
            return Ok(super::HistoryForget::Refused);
        }
        let items: Vec<_> = result.Items()?.into_iter().take(3).collect();
        let format = StandardDataFormats::Text()?;
        let texts: Vec<Option<String>> = items
            .iter()
            .map(|item| {
                let content = item.Content().ok()?;
                if !content.Contains(&format).ok()? {
                    return None;
                }
                Some(content.GetTextAsync().ok()?.join().ok()?.to_string())
            })
            .collect();
        Ok(match super::copied_item(&texts, copied) {
            Some(i) if Clipboard::DeleteItemFromHistory(&items[i])? => super::HistoryForget::Removed,
            Some(_) => super::HistoryForget::Refused,
            None => super::HistoryForget::NotFound,
        })
    }
}

#[cfg(windows)]
pub use win::{forget_in_history, WinClipboard, WinKeys};

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
    use std::cell::RefCell;
    use std::rc::Rc;

    /// The clipboard and the app behind it: on Ctrl+C the app copies `selection` (if any) after `copy_after` sleeps.
    #[derive(Default)]
    struct World {
        seq: u32,
        content: Snapshot,
        text: Option<String>,
        selection: Option<String>,
        copy_after: u32,
        sleeps: u32,
        pressed_c: bool,
        stuck: bool,
        snapshot_fails: bool,
        restored: Option<Snapshot>,
    }

    impl World {
        fn maybe_copy(&mut self) {
            if self.pressed_c
                && self.sleeps >= self.copy_after
                && self.text != self.selection
                && let Some(s) = self.selection.clone()
            {
                self.text = Some(s);
                self.seq += 1;
            }
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
        fn write_text(&mut self, _text: &str, _private: bool) -> Result<(), String> {
            panic!("the copy never writes to the clipboard");
        }
        fn restore(&mut self, s: &Snapshot) -> Result<(), String> {
            let mut w = self.0.borrow_mut();
            w.restored = Some(s.clone());
            w.seq += 1;
            Ok(())
        }
    }

    impl ReadClipboard for Clip {
        fn read_text(&mut self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().text.clone())
        }
    }

    impl KeysApi for Keys {
        fn wait_modifiers_released(&mut self, _t: Duration) -> bool {
            !self.0.borrow().stuck
        }
        fn press_ctrl_v(&mut self) -> bool {
            panic!("the copy never pastes");
        }
        fn sleep(&mut self, _d: Duration) {
            let mut w = self.0.borrow_mut();
            w.sleeps += 1;
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
    }

    fn user_clip() -> Snapshot {
        Snapshot { items: vec![(13, vec![65, 0, 0, 0])] }
    }

    fn copy(w: World) -> (Option<String>, World) {
        let w = Rc::new(RefCell::new(w));
        let got = copy_selection(&mut Clip(w.clone()), &mut Keys(w.clone()));
        let w = Rc::try_unwrap(w).ok().unwrap().into_inner();
        (got, w)
    }

    #[test]
    fn copies_the_selection_and_puts_the_clipboard_back() {
        let (got, w) = copy(World { content: user_clip(), selection: Some("выделенный текст".into()), ..Default::default() });
        assert_eq!(got.as_deref(), Some("выделенный текст"));
        assert_eq!(w.restored, Some(user_clip()), "the user's clipboard is back");
    }

    #[test]
    fn waits_for_a_slow_app() {
        let (got, _) = copy(World { selection: Some("медленно".into()), copy_after: 5, ..Default::default() });
        assert_eq!(got.as_deref(), Some("медленно"));
    }

    /// Spec 2 §13.5: with nothing selected the clipboard is not touched.
    #[test]
    fn nothing_selected_leaves_the_clipboard_untouched() {
        let (got, w) = copy(World { content: user_clip(), ..Default::default() });
        assert_eq!(got, None);
        assert_eq!(w.restored, None, "nothing to put back");
        assert!(w.sleeps * 20 <= 400 + 20, "gives up after 0.4 s: {} sleeps", w.sleeps);
    }

    #[test]
    fn held_keys_or_an_unreadable_clipboard_mean_no_copy() {
        let (got, w) = copy(World { stuck: true, selection: Some("x".into()), ..Default::default() });
        assert_eq!((got, w.pressed_c), (None, false), "Ctrl + Win + Alt + C would be another shortcut");
        let (got, w) = copy(World { snapshot_fails: true, selection: Some("x".into()), ..Default::default() });
        assert_eq!((got, w.pressed_c), (None, false), "a clipboard that cannot be put back is not risked");
    }

    #[test]
    fn a_blank_copy_is_no_selection_but_the_clipboard_comes_back() {
        let (got, w) = copy(World { content: user_clip(), selection: Some(" \r\n".into()), ..Default::default() });
        assert_eq!(got, None);
        assert_eq!(w.restored, Some(user_clip()));
    }

    /// Decision 27: only a fresh history item with exactly the copied text is removed, never an older one of the user's.
    #[test]
    fn only_a_fresh_history_item_with_the_copied_text_is_removed() {
        let items = [Some("другое".to_owned()), None, Some("выделенный текст".to_owned())];
        assert_eq!(copied_item(&items, "выделенный текст"), Some(2));
        assert_eq!(copied_item(&items, "нет такого"), None);
        let old = [None, None, None, Some("выделенный текст".to_owned())];
        assert_eq!(copied_item(&old, "выделенный текст"), None, "older than the copy: the user's own");
    }
}
