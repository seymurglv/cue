//! Where the text will go: is there a field, is it a password, what is before the caret, which app.
//! "No field" — only when we are sure there is no field.
// Querying the field exists only on Windows; other builds keep only the rule and the pure helpers.
#![cfg_attr(not(windows), allow(dead_code))]
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub const UIA_EDIT: i32 = 50004;
pub const UIA_DOCUMENT: i32 = 50030;
pub const UIA_LIST_ITEM: i32 = 50007;
pub const UIA_LIST: i32 = 50008;
pub const UIA_TREE: i32 = 50023;
pub const UIA_TREE_ITEM: i32 = 50024;
pub const UIA_PANE: i32 = 50033;
pub const UIA_GROUP: i32 = 50026;
pub const UIA_DATA_ITEM: i32 = 50029;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Editability {
    Yes,
    No,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub struct FocusFacts {
    pub fg_class: String,
    pub process: String,
    pub has_element: bool,
    pub control_type: i32,
    pub value_editable: bool,
    pub has_text_pattern: bool,
    pub has_caret: bool,
}

#[derive(Clone, Default, PartialEq)]
pub struct FocusInfo {
    pub app: String,
    pub editable: Editability,
    pub is_password: bool,
    pub before_cursor: Option<String>,
}

/// Without the text before the caret itself — only its length: `{info:?}` in the log must not carry the document there.
impl std::fmt::Debug for FocusInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FocusInfo")
            .field("app", &self.app)
            .field("editable", &self.editable)
            .field("is_password", &self.is_password)
            .field("before_cursor_chars", &self.before_cursor.as_ref().map(|s| s.chars().count()))
            .finish()
    }
}

/// App name in the "don't know" answer (the query did not make it in time or UI Automation did not start). This is not
/// the name of a real app: the caller compares against this constant, not against the string.
pub const UNKNOWN_APP: &str = "unknown app";

/// "Don't know": there may be a field — paste anyway; no app and no text.
fn unknown() -> FocusInfo {
    FocusInfo { app: UNKNOWN_APP.into(), ..Default::default() }
}

pub fn classify(f: &FocusFacts) -> Editability {
    // The desktop — except the icon rename box.
    if (f.fg_class == "Progman" || f.fg_class == "WorkerW") && f.control_type != UIA_EDIT {
        return Editability::No;
    }
    // File Explorer's file list and folder tree: the element has an editable value (the file name), but there is no field.
    // The rename box is an `Edit`, it does not end up here.
    if f.process.eq_ignore_ascii_case("explorer.exe")
        && matches!(
            f.control_type,
            UIA_LIST | UIA_LIST_ITEM | UIA_TREE | UIA_TREE_ITEM | UIA_PANE | UIA_DATA_ITEM | UIA_GROUP
        )
    {
        return Editability::No;
    }
    if f.value_editable || f.has_text_pattern || f.has_caret || matches!(f.control_type, UIA_EDIT | UIA_DOCUMENT) {
        return Editability::Yes;
    }
    if !f.has_element {
        return Editability::No;
    }
    Editability::Unknown
}

/// The executable's file name from the full path, and the same without `.exe` (in any case) — the app name
/// when the file properties have no description.
fn file_and_stem(path: &str) -> (String, String) {
    let file = path.rsplit('\\').next().unwrap_or(path);
    let stem = match file.len().checked_sub(4) {
        Some(i) if file.get(i..).is_some_and(|ext| ext.eq_ignore_ascii_case(".exe")) => &file[..i],
        _ => file,
    };
    (file.to_string(), stem.to_string())
}

/// The last `n` characters: closer to the caret matters more.
fn last_chars(s: &str, n: usize) -> &str {
    let Some(back) = n.checked_sub(1) else { return "" };
    match s.char_indices().nth_back(back) {
        Some((i, _)) => &s[i..],
        None => s,
    }
}

/// Text before the caret for refine: up to the last `n` characters. One U+FFFD at the start — half of an emoji where
/// the read piece began — is dropped. Empty — no text.
fn context_tail(text: &str, n: usize) -> Option<String> {
    let text = text.strip_prefix('\u{FFFD}').unwrap_or(text);
    Some(last_chars(text, n).to_string()).filter(|s| !s.is_empty())
}

/// An embedded object (picture, emoji picture, attachment): Chromium gives it as this character.
const OBJECT: char = '\u{FFFC}';

/// Whether there may be a line break between the end of `text` and the caret that the field did not give: a text
/// character at the end. A space or a line break at the end — the boundary is already visible. An embedded object right
/// before the caret is not a boundary: Chromium counts the object itself as a "line" too (the caret right after a picture
/// at the start of a line is "at line start"), although on screen the caret is after the picture on the same line;
/// after an object the join space goes as after a word.
fn line_break_may_be_missing(text: &str) -> bool {
    text.chars().next_back().is_some_and(|c| !c.is_whitespace() && c != OBJECT)
}

/// The caret is at the start of a line (`at_line_start`), but the text has no line break before it
/// (`line_break_may_be_missing`) — one is appended. Then the join adds no space, and refine knows the phrase
/// starts on a new line. Empty text (start of the field) — as is.
fn mark_line_start(mut text: String, at_line_start: bool) -> String {
    if at_line_start && line_break_may_be_missing(&text) {
        text.push('\n');
    }
    text
}

/// The moment after which nobody waits for the answer to the request any more; `None` — no deadline.
#[derive(Clone, Copy, Debug)]
struct Deadline(Option<Instant>);

impl Deadline {
    fn after(timeout: Duration) -> Self {
        Self(Instant::now().checked_add(timeout))
    }

    fn passed(self) -> bool {
        self.0.is_some_and(|d| Instant::now() >= d)
    }
}

/// A request to the `cue-focus` thread: how many characters before the caret to read, until when the answer is still
/// awaited, and where to reply.
struct Request {
    chars: usize,
    deadline: Deadline,
    reply: Sender<FocusInfo>,
}

/// UI Automation on its own thread: a hung app must not delay the paste.
pub struct FocusWorker {
    tx: Sender<Request>,
}

impl FocusWorker {
    /// The `cue-focus` thread with UI Automation. If it did not start, `query` answers "don't know" at once.
    #[cfg(windows)]
    pub fn spawn() -> Self {
        Self::start(win::setup)
    }

    /// `setup` runs on the new `cue-focus` thread and prepares the field query there; `None` — there will be no query,
    /// the thread ends, and `query` answers "don't know" at once.
    fn start<F: FnMut(usize, Deadline) -> FocusInfo>(setup: impl FnOnce() -> Option<F> + Send + 'static) -> Self {
        let (tx, rx) = channel::<Request>();
        std::thread::Builder::new()
            .name("cue-focus".into())
            .spawn(move || {
                if let Some(inspect) = setup() {
                    serve(rx, inspect);
                }
            })
            .expect("UI Automation thread");
        Self { tx }
    }

    /// Not done within `timeout` — "don't know" (paste anyway).
    pub fn query(&self, context_chars: usize, timeout: Duration) -> FocusInfo {
        let (reply, answer) = channel();
        let deadline = Deadline::after(timeout);
        if self.tx.send(Request { chars: context_chars, deadline, reply }).is_err() {
            return unknown();
        }
        answer.recv_timeout(timeout).unwrap_or_else(|_| unknown())
    }
}

/// Requests one by one until `FocusWorker` is closed. One whose deadline passed while the thread was busy with a hung app
/// is skipped without touching UI Automation: nobody waits for the answer any more, and the queue does not pile up behind
/// the hung app. A query that finished after the deadline (and may have been cut short halfway) is not sent.
fn serve(rx: Receiver<Request>, mut inspect: impl FnMut(usize, Deadline) -> FocusInfo) {
    for req in rx {
        if req.deadline.passed() {
            continue;
        }
        let info = inspect(req.chars, req.deadline);
        if !req.deadline.passed() {
            let _ = req.reply.send(info);
        }
    }
}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use windows::core::{w, Interface, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement, IUIAutomationTextPattern,
        IUIAutomationTextRange, IUIAutomationValuePattern, TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start,
        TextUnit_Character, TextUnit_Line, UIA_TextPatternId, UIA_ValuePatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongW, GetWindowThreadProcessId,
        RealGetWindowClassW, SendMessageTimeoutW, ES_PASSWORD, GUITHREADINFO, GWL_STYLE, SMTO_ABORTIFHUNG,
        WM_GETTEXT,
    };

    use super::{
        classify, context_tail, file_and_stem, line_break_may_be_missing, mark_line_start, unknown, Deadline, FocusFacts,
        FocusInfo,
    };

    /// How long UI Automation waits for a hung app (the system default is 2 and 20 s): any longer and the thread would
    /// not answer the following requests either.
    const UIA_TIMEOUT_MS: u32 = 1_000;
    /// `FrameworkId` of Chromium web content elements: Edge, Chrome and Electron apps.
    const CHROMIUM: &str = "Chrome";
    /// App names kept in the thread's memory — with room for everything opened in a day.
    const NAMES_CAP: usize = 64;
    /// The frame window of Store apps: the app itself is in another process, and that is the one we name.
    const FRAME_HOST: &str = "ApplicationFrameHost.exe";
    /// `EM_GETSEL` (the `Win32_UI_Controls` feature is not enabled for a single constant).
    const EM_GETSEL: u32 = 0x00B0;

    /// COM of the `cue-focus` thread. Closed last — after the UI Automation objects (`Uia`).
    struct Com;

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: paired with the successful `CoInitializeEx` of this same thread in `setup`; the thread has no COM objects left.
            unsafe { CoUninitialize() };
        }
    }

    /// UI Automation of the `cue-focus` thread and app names by exe path. Fields are dropped in declaration
    /// order: `uia` before COM.
    struct Uia {
        uia: IUIAutomation,
        names: HashMap<Vec<u16>, String>,
        _com: Com,
    }

    /// Runs on the `cue-focus` thread: COM and UI Automation of that thread. If it fails — one log entry
    /// and no queries.
    pub(super) fn setup() -> Option<impl FnMut(usize, Deadline) -> FocusInfo> {
        // SAFETY: the first COM call on a new thread; the paired `CoUninitialize` is in `Com::drop`.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if hr.is_err() {
            tracing::warn!(%hr, "text field: COM did not start, the field is not checked");
            return None;
        }
        let com = Com;
        // `CUIAutomation8`, not `CUIAutomation`: only it has `IUIAutomation2` with timeouts.
        // SAFETY: COM of this thread is running (`com`), the object is created in it.
        let uia: IUIAutomation = match unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) } {
            Ok(uia) => uia,
            Err(e) => {
                tracing::warn!(hr = %e.code(), "text field: UI Automation is unavailable, the field is not checked");
                return None;
            }
        };
        if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
            // SAFETY: configuring our own UI Automation object; a failure does not get in the way of the query.
            unsafe {
                let _ = uia2.SetConnectionTimeout(UIA_TIMEOUT_MS);
                let _ = uia2.SetTransactionTimeout(UIA_TIMEOUT_MS);
            }
        }
        let mut uia = Uia { uia, names: HashMap::new(), _com: com };
        // The method call captures the whole `uia`: COM is closed only together with the closure.
        Some(move |chars, deadline| uia.inspect(chars, deadline))
    }

    impl Uia {
        /// After the deadline the app is not asked anything new: the result will not be sent anyway (`serve`).
        fn inspect(&mut self, context_chars: usize, deadline: Deadline) -> FocusInfo {
            let mut facts = FocusFacts::default();
            let mut info = FocusInfo::default();
            // SAFETY: no arguments. The window may close at any moment — further calls with it return an error or 0.
            let fg = unsafe { GetForegroundWindow() };
            facts.fg_class = class_name(fg);
            let (pid, tid) = window_process(fg);
            let fg_path = exe_path(pid);
            if let Some(path) = &fg_path {
                facts.process = file_and_stem(&String::from_utf16_lossy(path)).0;
            }
            let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
            // SAFETY: `gti` with `cbSize` filled in lives until the end of the call.
            if unsafe { GetGUIThreadInfo(tid, &mut gti) }.is_ok() {
                facts.has_caret = !gti.hwndCaret.is_invalid();
            }
            if deadline.passed() {
                return unknown();
            }
            let mut app_pid = None;
            // SAFETY: a call on our own UI Automation object on the thread where it was created.
            match unsafe { self.uia.GetFocusedElement() } {
                Ok(el) => {
                    facts.has_element = true;
                    let readable = read_element(&el, context_chars, deadline, &mut facts, &mut info);
                    if readable && info.before_cursor.is_none() && !facts.has_text_pattern && !deadline.passed() {
                        // "Not a password" was said about the UIA element — read the window only if it is that element's own window.
                        // SAFETY: as above.
                        let same = unsafe { el.CurrentNativeWindowHandle() }
                            .is_ok_and(|h| !h.is_invalid() && h == gti.hwndFocus);
                        if same && !deadline.passed() {
                            info.before_cursor = edit_text_before_caret(gti.hwndFocus, context_chars, deadline);
                        }
                    }
                    if facts.process.eq_ignore_ascii_case(FRAME_HOST) && !deadline.passed() {
                        // SAFETY: as above.
                        app_pid = unsafe { el.CurrentProcessId() }.ok().and_then(|p| u32::try_from(p).ok());
                    }
                }
                // A UI Automation error is not yet "no field": with a real foreground window it is "don't know" (or "yes"
                // by the caret); "no" — only when there is no foreground window at all.
                Err(_) => facts.has_element = !fg.is_invalid(),
            }
            // The File Explorer rule goes by the window's process; the name — by the element's process if the window is just a frame.
            let app_path = app_pid.filter(|&p| p != pid).and_then(exe_path).or(fg_path);
            info.app = app_path.map(|p| self.app_name(&p)).filter(|a| !a.is_empty()).unwrap_or_else(|| unknown().app);
            info.editable = classify(&facts);
            info
        }

        /// The description from the file properties, otherwise the file name without `.exe`; remembered by path.
        fn app_name(&mut self, path: &[u16]) -> String {
            if let Some(name) = self.names.get(path) {
                return name.clone();
            }
            let name = file_description(path).unwrap_or_else(|| file_and_stem(&String::from_utf16_lossy(path)).1);
            if self.names.len() >= NAMES_CAP {
                self.names.clear();
            }
            self.names.insert(path.to_vec(), name.clone());
            name
        }
    }

    fn class_name(hwnd: HWND) -> String {
        let mut class = [0u16; 128];
        // SAFETY: a buffer on the stack, the function takes its length from the slice.
        let n = unsafe { GetClassNameW(hwnd, &mut class) };
        String::from_utf16_lossy(&class[..(n.max(0) as usize).min(class.len())])
    }

    /// The window's process and thread; zeros for an empty or closed window.
    fn window_process(hwnd: HWND) -> (u32, u32) {
        let mut pid = 0u32;
        // SAFETY: `pid` lives until the end of the call.
        let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
        (pid, tid)
    }

    /// Type, patterns and password flag of the focused element; the text before the caret — through the text pattern.
    /// `true` — it is known for sure that this is not a password, and the text was asked for: it may be read another way too.
    fn read_element(
        el: &IUIAutomationElement,
        context_chars: usize,
        deadline: Deadline,
        facts: &mut FocusFacts,
        info: &mut FocusInfo,
    ) -> bool {
        // SAFETY: calls on a live element on the UI Automation thread. If the element is gone or the app did not answer — `Err`.
        unsafe {
            facts.control_type = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
            if deadline.passed() {
                return false;
            }
            let password = el.CurrentIsPassword().map(|b| b.as_bool());
            info.is_password = matches!(password, Ok(true));
            let readable = matches!(password, Ok(false)) && context_chars > 0;
            if deadline.passed() {
                return false;
            }
            if let Ok(v) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
                facts.value_editable = matches!(v.CurrentIsReadOnly().map(|b| b.as_bool()), Ok(false));
            }
            if deadline.passed() {
                return false;
            }
            if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
                facts.has_text_pattern = true;
                if readable && !deadline.passed() {
                    let chromium = el.CurrentFrameworkId().is_ok_and(|f| f == CHROMIUM);
                    info.before_cursor = text_before_caret(&tp, context_chars, chromium, deadline);
                }
            }
            readable
        }
    }

    /// Up to `n` characters before the caret (before the start of the selection, if something is selected).
    ///
    /// Chromium (`chromium`: Edge, Chrome and Electron apps) counts both in its own way:
    /// - the range is not limited to the field: at the start of the field (and in an empty field) it reaches into the page
    ///   text before the field — its start is pulled in to the start of the field itself;
    /// - the caret at the start of a new paragraph (Enter in an editor on `<p>`/`<div>`) is at the same place as the end of
    ///   the previous paragraph: the text has no line break before it — `mark_line_start` adds one if the caret is at line start.
    ///
    /// Deadline passed — the app is not asked anything more (`None`: the answer will not go out anyway, and a hung app
    /// would hold the thread for up to `UIA_TIMEOUT_MS` more on every call).
    fn text_before_caret(tp: &IUIAutomationTextPattern, n: usize, chromium: bool, deadline: Deadline) -> Option<String> {
        let n32 = i32::try_from(n).unwrap_or(i32::MAX);
        let in_time = || (!deadline.passed()).then_some(());
        // SAFETY: calls on the pattern of a live element and its ranges on the UI Automation thread; errors — `Err`.
        unsafe {
            let sel = tp.GetSelection().ok()?;
            if sel.Length().ok()? < 1 {
                return None;
            }
            let caret = sel.GetElement(0).ok()?;
            // An empty range at the start of the selection is the caret.
            let at = caret.Clone().ok()?;
            at.MoveEndpointByRange(TextPatternRangeEndpoint_End, &caret, TextPatternRangeEndpoint_Start).ok()?;
            let r = at.Clone().ok()?;
            r.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -n32).ok()?;
            if chromium {
                in_time()?;
                keep_inside_field(tp, &r)?;
            }
            in_time()?;
            // A UI Automation "character" can be longer than one UTF-16 unit (emoji, CRLF): with room to spare, and trimmed
            // from the start so as not to lose what is closest to the caret.
            let text = r.GetText(n32.saturating_mul(2)).ok()?.to_string();
            // Line start is checked only when something depends on it.
            let line_start = chromium && line_break_may_be_missing(&text) && in_time().is_some() && at_line_start(&at);
            context_tail(&mark_line_start(text, line_start), n)
        }
    }

    /// The start of range `r` (it ends at the caret) — no earlier than the start of the field's text. If it could not be
    /// pulled in — `None`: text outside the field is not read.
    unsafe fn keep_inside_field(tp: &IUIAutomationTextPattern, r: &IUIAutomationTextRange) -> Option<()> {
        // SAFETY: as in `text_before_caret`.
        unsafe {
            let field = tp.DocumentRange().ok()?;
            let before = r.CompareEndpoints(TextPatternRangeEndpoint_Start, &field, TextPatternRangeEndpoint_Start).ok()?;
            if before < 0 {
                r.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &field, TextPatternRangeEndpoint_Start).ok()?;
            }
            Some(())
        }
    }

    /// The caret (empty range `at`) is at line start: the line it belongs to starts with it.
    /// An error — "no": that is how it was before this check.
    unsafe fn at_line_start(at: &IUIAutomationTextRange) -> bool {
        // SAFETY: as in `text_before_caret`.
        unsafe {
            let Ok(line) = at.Clone() else { return false };
            line.ExpandToEnclosingUnit(TextUnit_Line).is_ok()
                && line.CompareEndpoints(TextPatternRangeEndpoint_Start, at, TextPatternRangeEndpoint_Start).is_ok_and(|c| c == 0)
        }
    }

    /// A classic Win32 field (`Edit` and its descendants, for example the WinForms `TextBox`). Such a field often answers
    /// the system itself through MSAA, and UI Automation exposes it through an MSAA proxy that has no text
    /// pattern. Then the selection start and the text before it are asked from the field itself with messages that the
    /// system marshals between processes. A password field (`ES_PASSWORD`) and CUE's own windows are not read.
    fn edit_text_before_caret(hwnd: HWND, n: usize, deadline: Deadline) -> Option<String> {
        // SAFETY: no arguments.
        let own = unsafe { GetCurrentProcessId() };
        if hwnd.is_invalid() || window_process(hwnd).0 == own || !is_edit(hwnd) {
            return None;
        }
        // SAFETY: only reads the window style.
        if unsafe { GetWindowLongW(hwnd, GWL_STYLE) } & ES_PASSWORD != 0 {
            return None;
        }
        // The selection start is in the low word of the answer; −1 — the selection start or end is beyond the 65,535th
        // character, then no text.
        let sel = send(hwnd, EM_GETSEL, 0, 0, deadline)? as u32;
        if sel == u32::MAX {
            return None;
        }
        let start = (sel & 0xFFFF) as usize;
        if start == 0 {
            return None;
        }
        // `WM_GETTEXT` copies the start of the text, one character less than the buffer size: exactly what is before the caret.
        let mut buf = vec![0u16; start + 1];
        let copied = send(hwnd, WM_GETTEXT, buf.len(), buf.as_mut_ptr() as isize, deadline)?;
        context_tail(&String::from_utf16_lossy(&buf[..copied.min(start)]), n)
    }

    /// The real window class is `Edit`, even if the field is subclassed under another name.
    fn is_edit(hwnd: HWND) -> bool {
        let mut class = [0u16; 32];
        // SAFETY: a buffer on the stack, the function takes its length from the slice.
        let n = unsafe { RealGetWindowClassW(hwnd, &mut class) } as usize;
        String::from_utf16_lossy(&class[..n.min(class.len())]).eq_ignore_ascii_case("Edit")
    }

    /// A message to a window of another process with a timeout (no longer than `UIA_TIMEOUT_MS` and the request deadline);
    /// a hung window — `None` at once.
    fn send(hwnd: HWND, msg: u32, wparam: usize, lparam: isize, deadline: Deadline) -> Option<usize> {
        let cap = Duration::from_millis(UIA_TIMEOUT_MS.into());
        let left = deadline.0.map_or(cap, |d| d.saturating_duration_since(Instant::now()));
        let ms = left.min(cap).as_millis().max(1) as u32;
        let mut result = 0usize;
        // SAFETY: a window of another process (checked in `edit_text_before_caret`), so the system copies the `WM_GETTEXT`
        // data through its own buffer and does not write to `lparam` after the timeout; `lparam` is a live buffer of
        // `wparam` characters or 0; `result` lives until the end of the call.
        let ok = unsafe {
            SendMessageTimeoutW(hwnd, msg, WPARAM(wparam), LPARAM(lparam), SMTO_ABORTIFHUNG, ms, Some(&raw mut result))
        };
        (ok.0 != 0).then_some(result)
    }

    /// The full path of the process executable, without the trailing null.
    fn exe_path(pid: u32) -> Option<Vec<u16>> {
        // SAFETY: only a query for process information; the handle is closed below.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        // SAFETY: `h` is opened above; `buf` and `len` (its length in characters) live until the end of the call.
        let ok =
            unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }.is_ok();
        // SAFETY: `h` is opened above and no longer used.
        let _ = unsafe { CloseHandle(h) };
        ok.then(|| buf[..(len as usize).min(buf.len())].to_vec())
    }

    /// The description from the file properties ("Telegram Desktop"): in the first language from the file's translation
    /// list, otherwise in English (US) and in the neutral language, in Unicode.
    fn file_description(path: &[u16]) -> Option<String> {
        let wide: Vec<u16> = path.iter().copied().chain(std::iter::once(0)).collect();
        let name = PCWSTR(wide.as_ptr());
        // SAFETY: `name` is the null-terminated string `wide`, it lives until the end of the function.
        let size = unsafe { GetFileVersionInfoSizeW(name, None) } as usize;
        if size == 0 {
            return None;
        }
        // A block of u32: its entries are aligned to 4 bytes from its start.
        let mut data = vec![0u32; size.div_ceil(4)];
        // SAFETY: `data` has at least `size` bytes.
        unsafe { GetFileVersionInfoW(name, None, size as u32, data.as_mut_ptr().cast()) }.ok()?;
        // SAFETY: the first `size` bytes of `data` are the block just filled; u8 has no alignment requirements.
        let block = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), size) };
        let mut keys = Vec::with_capacity(3);
        if let Some(t) = ver_value(block, w!("\\VarFileInfo\\Translation"), 1).filter(|t| t.len() >= 4) {
            let (lang, cp) = (u16::from_le_bytes([t[0], t[1]]), u16::from_le_bytes([t[2], t[3]]));
            keys.push(format!("{lang:04X}{cp:04X}"));
        }
        keys.extend(["040904B0".to_string(), "000004B0".to_string()]);
        keys.iter().find_map(|lang_cp| description(block, lang_cp))
    }

    fn description(block: &[u8], lang_cp: &str) -> Option<String> {
        let key: Vec<u16> = format!("\\StringFileInfo\\{lang_cp}\\FileDescription\0").encode_utf16().collect();
        let bytes = ver_value(block, PCWSTR(key.as_ptr()), 2)?;
        let units: Vec<u16> =
            bytes.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&c| c != 0).collect();
        let s = String::from_utf16_lossy(&units);
        Some(s.trim().to_string()).filter(|s| !s.is_empty())
    }

    /// The value `sub` from the version information block — bytes inside `block`, no further than its end, even if the file
    /// lies about the length. `unit` — what `VerQueryValueW` counts the length in: 1 — bytes (binary values), 2 — UTF-16 characters.
    fn ver_value(block: &[u8], sub: PCWSTR, unit: usize) -> Option<&[u8]> {
        let mut p = std::ptr::null_mut();
        let mut n = 0u32;
        // SAFETY: `block` is a version information block from `GetFileVersionInfoW`, `sub` is a null-terminated string;
        // the function only writes a pointer into the block to `p` and the length to `n`.
        if !unsafe { VerQueryValueW(block.as_ptr().cast(), sub, &mut p, &mut n) }.as_bool() || p.is_null() {
            return None;
        }
        let off = (p as usize).checked_sub(block.as_ptr() as usize)?;
        let end = off.checked_add((n as usize).saturating_mul(unit))?.min(block.len());
        block.get(off..end)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn wide(s: &str) -> Vec<u16> {
            s.encode_utf16().collect()
        }

        #[test]
        fn own_exe_path_is_found() {
            let path = String::from_utf16_lossy(&exe_path(std::process::id()).unwrap());
            assert!(path.contains('\\') && path.to_ascii_lowercase().ends_with(".exe"), "{path}");
            assert_eq!(exe_path(0), None, "there is no process 0");
        }

        #[test]
        fn description_comes_from_file_properties() {
            let explorer = std::env::var("SystemRoot").unwrap() + r"\explorer.exe";
            assert!(file_description(&wide(&explorer)).is_some_and(|d| !d.is_empty() && !d.contains('\0')));
            assert_eq!(file_description(&wide(r"C:\нет\такого.exe")), None);
        }

        #[test]
        fn ui_automation_starts_and_answers_without_text_when_not_asked() {
            let mut inspect = setup().expect("UI Automation");
            let info = inspect(0, Deadline::after(Duration::from_secs(5)));
            assert!(!info.app.is_empty());
            assert_eq!(info.before_cursor, None);
        }

        #[test]
        fn nothing_is_asked_after_the_deadline() {
            let mut inspect = setup().expect("UI Automation");
            let info = inspect(500, Deadline(Some(Instant::now())));
            assert_eq!(info, unknown());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> FocusFacts {
        FocusFacts { fg_class: "Notepad".into(), process: "notepad.exe".into(), has_element: true, ..Default::default() }
    }

    #[test]
    fn desktop_and_nothing_focused_are_no() {
        assert_eq!(classify(&FocusFacts { fg_class: "Progman".into(), ..facts() }), Editability::No);
        assert_eq!(classify(&FocusFacts { fg_class: "WorkerW".into(), ..facts() }), Editability::No);
        assert_eq!(classify(&FocusFacts { has_element: false, ..facts() }), Editability::No);
    }

    #[test]
    fn edits_documents_caret_are_yes() {
        assert_eq!(classify(&FocusFacts { control_type: UIA_EDIT, ..facts() }), Editability::Yes);
        assert_eq!(classify(&FocusFacts { control_type: UIA_DOCUMENT, ..facts() }), Editability::Yes);
        assert_eq!(classify(&FocusFacts { value_editable: true, ..facts() }), Editability::Yes);
        assert_eq!(classify(&FocusFacts { has_text_pattern: true, ..facts() }), Editability::Yes);
        assert_eq!(classify(&FocusFacts { has_caret: true, has_element: false, ..facts() }), Editability::Yes);
    }

    #[test]
    fn explorer_file_list_is_no_but_rename_box_is_yes() {
        let ex = FocusFacts { process: "explorer.exe".into(), fg_class: "CabinetWClass".into(), ..facts() };
        assert_eq!(classify(&FocusFacts { control_type: UIA_LIST_ITEM, ..ex.clone() }), Editability::No);
        assert_eq!(classify(&FocusFacts { control_type: UIA_LIST, ..ex.clone() }), Editability::No);
        assert_eq!(classify(&FocusFacts { control_type: UIA_EDIT, ..ex }), Editability::Yes);
    }

    #[test]
    fn explorer_items_are_no_even_with_an_editable_name() {
        // As seen in a live check: a file in the list (`ListItem`, class `UIItem`) with an editable value — its name.
        let item = FocusFacts {
            fg_class: "CabinetWClass".into(),
            process: "explorer.exe".into(),
            has_element: true,
            control_type: UIA_LIST_ITEM,
            value_editable: true,
            ..Default::default()
        };
        assert_eq!(classify(&item), Editability::No);
        for control_type in [UIA_LIST, UIA_TREE, UIA_TREE_ITEM, UIA_PANE, UIA_DATA_ITEM, UIA_GROUP] {
            let f = FocusFacts { control_type, has_text_pattern: true, has_caret: true, ..item.clone() };
            assert_eq!(classify(&f), Editability::No, "{control_type}");
        }
        let rename = FocusFacts { control_type: UIA_EDIT, has_text_pattern: true, ..item.clone() };
        assert_eq!(classify(&rename), Editability::Yes, "rename box");
        let other = FocusFacts { process: "notepad.exe".into(), fg_class: "Notepad".into(), ..item };
        assert_eq!(classify(&other), Editability::Yes, "an editable list item in another app");
    }

    #[test]
    fn desktop_rename_box_is_yes_but_icons_are_no() {
        let desktop = FocusFacts { fg_class: "Progman".into(), process: "explorer.exe".into(), ..facts() };
        let rename = FocusFacts { control_type: UIA_EDIT, has_text_pattern: true, ..desktop.clone() };
        assert_eq!(classify(&rename), Editability::Yes);
        let icon = FocusFacts { control_type: UIA_LIST_ITEM, value_editable: true, ..desktop.clone() };
        assert_eq!(classify(&icon), Editability::No);
        let worker = FocusFacts { fg_class: "WorkerW".into(), has_caret: true, ..desktop };
        assert_eq!(classify(&worker), Editability::No);
    }

    #[test]
    fn ui_automation_error_is_not_nowhere_to_paste() {
        // This is how `inspect` translates a `GetFocusedElement` error: there is a foreground window — the element "sort of exists".
        let error_with_window = FocusFacts { has_element: true, ..facts() };
        assert_eq!(classify(&error_with_window), Editability::Unknown);
        let error_with_caret = FocusFacts { has_caret: true, ..error_with_window };
        assert_eq!(classify(&error_with_caret), Editability::Yes);
        let no_window = FocusFacts { has_element: false, fg_class: String::new(), process: String::new(), ..facts() };
        assert_eq!(classify(&no_window), Editability::No);
    }

    #[test]
    fn custom_drawn_apps_are_unknown() {
        assert_eq!(classify(&FocusFacts { control_type: UIA_PANE, process: "game.exe".into(), ..facts() }), Editability::Unknown);
    }

    #[test]
    fn unknown_is_app_without_text() {
        let u = unknown();
        assert_eq!(
            (u.app.as_str(), u.editable, u.is_password, u.before_cursor),
            ("unknown app", Editability::Unknown, false, None)
        );
    }

    #[test]
    fn app_name_falls_back_to_file_name_without_exe() {
        let pair = |f: &str, s: &str| (f.to_string(), s.to_string());
        assert_eq!(
            file_and_stem(r"C:\Users\me\AppData\Roaming\Telegram Desktop\Telegram.EXE"),
            pair("Telegram.EXE", "Telegram")
        );
        assert_eq!(file_and_stem(r"D:\x.Exe"), pair("x.Exe", "x"));
        assert_eq!(file_and_stem(r"C:\Программы\Редактор.exe"), pair("Редактор.exe", "Редактор"));
        assert_eq!(file_and_stem("notepad.exe"), pair("notepad.exe", "notepad"), "a path without a backslash");
        assert_eq!(file_and_stem(r"C:\tools\run"), pair("run", "run"), "no extension");
        assert_eq!(file_and_stem("€€"), pair("€€", "€€"), "four bytes from the end are not a character boundary");
        assert_eq!(file_and_stem(""), pair("", ""));
    }

    #[test]
    fn context_is_the_last_chars_before_caret() {
        assert_eq!(last_chars("Привет, мир", 3), "мир");
        assert_eq!(last_chars("a😀b", 2), "😀b");
        assert_eq!(last_chars("коротко", 500), "коротко");
        assert_eq!(last_chars("что-то", 0), "");
        assert_eq!(last_chars("", 5), "");
    }

    #[test]
    fn context_drops_one_broken_half_at_the_start() {
        assert_eq!(context_tail("\u{FFFD}привет", 500).as_deref(), Some("привет"));
        assert_eq!(context_tail("\u{FFFD}\u{FFFD}x", 500).as_deref(), Some("\u{FFFD}x"), "only one");
        assert_eq!(context_tail("a\u{FFFD}b", 500).as_deref(), Some("a\u{FFFD}b"), "not at the start — left alone");
        assert_eq!(context_tail("Привет, мир", 3).as_deref(), Some("мир"));
        assert_eq!(context_tail("\u{FFFD}", 500), None);
        assert_eq!(context_tail("", 500), None);
    }

    #[test]
    fn missing_line_break_at_line_start_is_added() {
        // Chromium: the caret at the start of a new paragraph (Enter in an editor on <p>), the text has no line break.
        assert_eq!(mark_line_start("через 5 минут.".into(), true), "через 5 минут.\n");
        assert_eq!(mark_line_start("слово".into(), true), "слово\n");
        // The caret right after an embedded object (Chromium: "\n" and U+FFFC) is not line start, even though UIA says "yes".
        assert_eq!(mark_line_start("минут.\n\u{FFFC}".into(), true), "минут.\n\u{FFFC}", "a picture at line start");
        assert_eq!(mark_line_start("\u{FFFC}".into(), true), "\u{FFFC}");
        // The line break is already there (Shift+Enter, <br>, textarea) or the line wrapped at a space — as is.
        assert_eq!(mark_line_start("строка.\n".into(), true), "строка.\n");
        assert_eq!(mark_line_start("строка.\r".into(), true), "строка.\r");
        assert_eq!(mark_line_start("длинная строка ".into(), true), "длинная строка ");
        assert_eq!(mark_line_start("строка\u{A0}".into(), true), "строка\u{A0}", "contenteditable no-break space");
        // Start of the field: no text before the caret.
        assert_eq!(mark_line_start(String::new(), true), "");
        // Mid-line — as is.
        assert_eq!(mark_line_start("Итак,".into(), false), "Итак,");
        assert_eq!(mark_line_start("минут.".into(), false), "минут.");
    }

    #[test]
    fn new_line_gets_no_join_space_but_mid_line_does() {
        use cue_core::refine::join_for_insert;
        let join = |before: &str, line_start: bool| {
            let seen = context_tail(&mark_line_start(before.into(), line_start), 16);
            join_for_insert(seen.as_deref(), "Привет")
        };
        assert_eq!(join("через 5 минут.", true), "Привет", "a new paragraph in Chromium");
        assert_eq!(join("через 5 минут.\n", true), "Привет", "after Shift+Enter");
        assert_eq!(join("через 5 минут.", false), " Привет", "continuing the same line");
        assert_eq!(join("Итак,", false), " Привет");
        assert_eq!(join("Итак, ", false), "Привет");
        assert_eq!(join("", true), "Привет", "empty field");
        assert_eq!(join("минут.\n\u{FFFC}", true), " Привет", "after a picture — as after a word");
        assert_eq!(join("Итак\n\u{FFFC}", false), " Привет", "a picture mid-line");
    }

    #[test]
    fn debug_shows_length_not_text() {
        let info = FocusInfo { before_cursor: Some("секретное письмо".into()), ..notepad() };
        for s in [format!("{info:?}"), format!("{info:#?}")] {
            assert!(!s.contains("секрет"), "{s}");
            assert!(s.contains("16") && s.contains("Блокнот"), "{s}");
        }
    }

    fn notepad() -> FocusInfo {
        let before_cursor = Some("Привет, ".into());
        FocusInfo { app: "Блокнот".into(), editable: Editability::Yes, is_password: false, before_cursor }
    }

    #[test]
    fn fast_reply_is_returned() {
        let (seen_tx, seen) = channel();
        let w = FocusWorker::start(move || {
            Some(move |chars, deadline: Deadline| {
                let _ = seen_tx.send((chars, deadline.0.is_some() && !deadline.passed()));
                notepad()
            })
        });
        assert_eq!(w.query(500, Duration::from_secs(5)), notepad());
        assert_eq!(seen.try_iter().collect::<Vec<_>>(), vec![(500, true)], "the query gets the request deadline");
    }

    #[test]
    fn hung_app_gives_unknown_within_timeout() {
        let w = FocusWorker::start(|| {
            Some(|_, _| {
                std::thread::sleep(Duration::from_secs(1));
                notepad()
            })
        });
        let t = Instant::now();
        assert_eq!(w.query(500, Duration::from_millis(100)), unknown());
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(100) && took < Duration::from_millis(600), "{took:?}");
    }

    #[test]
    fn request_expired_while_busy_is_skipped() {
        let (seen_tx, seen) = channel();
        let (release_tx, release) = channel::<()>();
        let w = FocusWorker::start(move || {
            Some(move |chars, _| {
                let _ = seen_tx.send(chars);
                if chars == 1 {
                    let _ = release.recv(); // a "hung app" — until the test lets go
                }
                notepad()
            })
        });
        assert_eq!(w.query(0, Duration::from_secs(5)), notepad(), "the thread is running and free");
        assert_eq!(w.query(1, Duration::from_millis(200)), unknown(), "the app hung");
        assert_eq!(w.query(2, Duration::from_millis(200)), unknown(), "the thread is still busy");
        release_tx.send(()).unwrap();
        assert_eq!(w.query(3, Duration::from_secs(5)), notepad());
        assert_eq!(seen.try_iter().collect::<Vec<_>>(), vec![0, 1, 3], "expired request 2 — never asked");
    }

    #[test]
    fn answer_finished_after_the_deadline_is_not_sent() {
        let (tx, rx) = channel();
        let (reply, answer) = channel();
        tx.send(Request { chars: 500, deadline: Deadline::after(Duration::from_millis(50)), reply }).unwrap();
        drop(tx);
        serve(rx, |_, _| {
            std::thread::sleep(Duration::from_millis(100));
            notepad()
        });
        assert!(answer.try_recv().is_err(), "a query that was cut short or late does not go out");
    }

    #[test]
    fn worker_without_ui_automation_answers_unknown_at_once() {
        let w = FocusWorker::start(|| None::<fn(usize, Deadline) -> FocusInfo>);
        let t = Instant::now();
        assert_eq!(w.query(500, Duration::from_secs(5)), unknown());
        assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    }
}
