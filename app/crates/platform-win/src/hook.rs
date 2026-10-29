//! Low-level keyboard hook on its own thread, and, only while a Shift that toggled the mode after a tap is held, a
//! low-level mouse hook on the same thread: a click then takes the toggle back. The hook procedures only classify the
//! event and hand it to a channel: Windows removes a hook if its procedure takes too long to answer.
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Sender, SyncSender};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PeekMessageW, PostThreadMessageW, SetTimer,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOWS_HOOK_ID, WM_APP, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_TIMER, WM_XBUTTONDOWN,
};

use crate::chord::{Chord, Decision, HotkeyEvent, VK_ESCAPE, VK_LCONTROL, VK_LWIN, VK_RCONTROL, VK_RWIN, VK_V};
use crate::input::{key_held, send_mask_key, MODIFIERS};

/// Swallow Esc — while CUE is recording, thinking or showing an error. Set by the core.
pub static ESC_ACTIVE: AtomicBool = AtomicBool::new(false);
/// CUE is starting or recording: a fresh Shift press toggles the mode even after the chord was released. Set by the
/// core. Not `ESC_ACTIVE`: that one also covers thinking and errors, when the user may already be typing again, and
/// every capital letter would then wake the core for nothing.
pub static RECORDING: AtomicBool = AtomicBool::new(false);
/// Ctrl + Win + Alt is the "Ask" chord while "Ask" is on (`ask.enabled` in settings.json; the core sets this at
/// startup). Off, Alt keeps making a system shortcut.
pub static ASK_ENABLED: AtomicBool = AtomicBool::new(false);
/// "Pause for an hour": the hook swallows nothing, does not mask the Start menu and sends no events
/// (the Windows Ctrl+Win+V works as usual), but `Chord` still sees every key. Set by the core.
pub static PAUSED: AtomicBool = AtomicBool::new(false);
/// The hook is installed right now. False before `spawn`, after the thread ends, or if `SetWindowsHookExW`
/// failed during `reinstall` (the error is in the log; the next `reinstall` tries again).
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// The hook thread is running (or starting): a second `spawn` gets `HookError::AlreadyRunning`.
static SPAWNED: AtomicBool = AtomicBool::new(false);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);
const WM_REINSTALL: u32 = WM_APP + 1;
/// Install or remove the mouse hook, as `MOUSE_WANTED` says: posted by a hook procedure, which installs no hook itself.
const WM_MOUSE_WATCH: u32 = WM_APP + 2;
/// Scan code of the fake left Ctrl that Windows sends before the right Alt in layouts with AltGr.
const SC_ALTGR_LCONTROL: u32 = 0x21D;
/// Chord timer period: a stuck chord ends no later than two ticks after.
const CHORD_TICK_MS: u32 = 250;

thread_local! {
    static STATE: RefCell<Option<(Chord, Sender<HotkeyEvent>)>> = const { RefCell::new(None) };
    /// Chord timer (thread `SetTimer`), 0 — not set.
    static TIMER: Cell<usize> = const { Cell::new(0) };
    /// Modifiers on the previous timer tick.
    static DEBOUNCE: Cell<Debounce> = const { Cell::new(Debounce::ARMED) };
    /// The mouse hook while it is installed (`follow_mouse`).
    static MOUSE: Cell<Option<HHOOK>> = const { Cell::new(None) };
    /// Whether the mouse hook is wanted, as last asked of the message loop (`want_mouse`).
    static MOUSE_WANTED: Cell<bool> = const { Cell::new(false) };
}

/// The hook did not start.
#[derive(Debug, thiserror::Error)]
pub enum HookError {
    #[error("the keyboard hook is already running")]
    AlreadyRunning,
    #[error("the keyboard hook thread did not start: {0}")]
    Thread(#[from] std::io::Error),
    #[error("Windows did not install the keyboard hook: {0}")]
    Install(#[from] windows::core::Error),
}

/// A hook event parsed from `KBDLLHOOKSTRUCT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyEvent {
    vk: u16,
    scan: u32,
    /// Press (including auto-repeat), otherwise release.
    down: bool,
    /// Sent by a program (`SendInput`), including CUE itself.
    injected: bool,
}

impl KeyEvent {
    /// `msg` — the procedure's `wParam` (`WM_KEYDOWN`, `WM_SYSKEYDOWN`, `WM_KEYUP`, `WM_SYSKEYUP`).
    fn from_hook(msg: u32, kb: &KBDLLHOOKSTRUCT) -> Self {
        Self {
            vk: kb.vkCode as u16,
            scan: kb.scanCode,
            down: msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN,
            injected: kb.flags.0 & LLKHF_INJECTED.0 != 0,
        }
    }
}

/// Which `Chord::resync` to run before the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resync {
    /// No resync: releases, Esc and other keys.
    No,
    /// Before a Ctrl or Win press: everything from `GetAsyncKeyState`.
    Full,
    /// Before any other press: modifiers from `GetAsyncKeyState`, while V and Esc count as held — their marks
    /// are left alone. This way a chord whose Ctrl and Win releases were lost ends on the very first press:
    /// with no false `ChordAborted`, and V does not become "paste". CUE does not swallow Ctrl, Win, Shift or Alt,
    /// so their `GetAsyncKeyState` state is correct; the current key is not in it yet, but `on_key` records it at once.
    Modifiers,
}

/// Where the event goes: `None` — past `Chord` (injected, including CUE's own mask; the fake left Ctrl from AltGr),
/// otherwise into `Chord` after this resync.
fn route(ev: &KeyEvent) -> Option<Resync> {
    if ev.injected || (ev.vk == VK_LCONTROL && ev.scan == SC_ALTGR_LCONTROL) {
        return None;
    }
    Some(match ev.vk {
        _ if !ev.down => Resync::No,
        VK_LCONTROL | VK_RCONTROL | VK_LWIN | VK_RWIN => Resync::Full,
        _ => Resync::Modifiers,
    })
}

fn resync(chord: &mut Chord, how: Resync, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
    match how {
        Resync::No => None,
        Resync::Full => chord.resync(held),
        Resync::Modifiers => chord.resync(|vk| vk == VK_V || vk == VK_ESCAPE || held(vk)),
    }
}

/// What the hook procedure should do with the event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Outcome {
    /// Events for the core, in order: first from the resync, then the undo of a Shift toggle (`Decision::undo_toggle`),
    /// then from the event itself.
    events: [Option<HotkeyEvent>; 3],
    swallow: bool,
    inject_mask_first: bool,
}

/// Resync and `Chord` decision for the event. `held(vk)` — is the key held (without the current event).
/// While paused `Chord` gets the event, but the decision is not carried out.
/// The second value — `Chord` has just formed the chord (`ChordDown`), including while paused: for the chord timer.
fn feed(
    chord: &mut Chord,
    ev: &KeyEvent,
    how: Resync,
    esc_active: bool,
    paused: bool,
    held: impl Fn(u16) -> bool,
) -> (Outcome, bool) {
    let first = resync(chord, how, held);
    // While paused, V and Esc presses go to the system, and `Chord` must not mark them as swallowed: otherwise after
    // the pause the release of a key still held would be swallowed. `Chord` does get the releases — they clear marks
    // left over from before the pause.
    let d = if paused && ev.down && matches!(ev.vk, VK_V | VK_ESCAPE) {
        Decision::default()
    } else {
        chord.on_key(ev.vk, ev.down, ev.injected, esc_active)
    };
    let chord_down = matches!(d.event, Some(HotkeyEvent::ChordDown(_)));
    if paused {
        return (Outcome::default(), chord_down);
    }
    let undo = d.undo_toggle.then_some(HotkeyEvent::ModeToggle);
    let outcome = Outcome { events: [first, undo, d.event], swallow: d.swallow, inject_mask_first: d.inject_mask_first };
    (outcome, chord_down)
}

/// Resync after a gap (sleep, lock): releases during that time never reached the hook.
/// Leaves swallowed V and Esc alone, gives no events while paused.
fn after_gap(chord: &mut Chord, paused: bool, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
    let event = resync(chord, Resync::Modifiers, held);
    if paused { None } else { event }
}

/// A mouse button went down by the user's hand: `msg` — the mouse hook procedure's `wParam`, `flags` — its
/// `MSLLHOOKSTRUCT` flags. Moves, releases and the wheel are no clicks; a press sent by a program passes by, as injected
/// keys do.
fn mouse_button_down(msg: u32, flags: u32) -> bool {
    matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN) && flags & LLMHF_INJECTED == 0
}

/// `Chord`'s answer to a click (`Chord::on_mouse_down`): one more `ModeToggle` when the click takes a Shift toggle back.
/// While paused `Chord` still sees the click, but the core gets nothing.
fn feed_mouse(chord: &mut Chord, paused: bool, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
    let undo = chord.on_mouse_down(held);
    (undo && !paused).then_some(HotkeyEvent::ModeToggle)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MouseChange {
    Install,
    Remove,
    Keep,
}

/// The mouse hook is needed exactly while a click would take a Shift toggle back (`Chord::watches_mouse`): a low-level
/// mouse hook routes every mouse event of the system through this thread, so it lives only for that second or so.
fn mouse_change(wanted: bool, installed: bool) -> MouseChange {
    match (wanted, installed) {
        (true, false) => MouseChange::Install,
        (false, true) => MouseChange::Remove,
        _ => MouseChange::Keep,
    }
}

/// Timer resync while the chord is held: it ends even if the Ctrl and Win releases never reached the hook
/// (UAC prompt, Ctrl+Alt+Del, lock screen, elevated window) and there are no new presses.
/// A key counts as released only if it is not seen held on this tick or the previous one: a tick may come
/// between `Chord` seeing the press and Windows updating `GetAsyncKeyState`. A key the hook saw pressed since the last
/// tick counts as seen (`pressed`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Debounce {
    /// Whether the modifier (`MODIFIERS`) was held on the previous tick or pressed since.
    prev: [bool; 8],
}

impl Debounce {
    /// A freshly set timer: nothing is released on the first tick.
    const ARMED: Self = Self { prev: [true; 8] };

    /// A modifier the hook has just seen go down counts as held on the next tick: Windows may not have recorded the press
    /// yet when the tick reads `GetAsyncKeyState`, and a tick that dropped it would lose what the key did — a Shift's toggle
    /// (its auto-repeat would toggle back) or the Alt of an "Ask" chord (its release would open the app's menu, unmasked).
    fn pressed(&mut self, vk: u16) {
        if let Some(i) = MODIFIERS.iter().position(|&m| m == vk) {
            self.prev[i] = true;
        }
    }

    /// Like `after_gap`, but taking the previous tick into account. While paused `Chord` resyncs, no events.
    fn tick(&mut self, chord: &mut Chord, paused: bool, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
        let now = MODIFIERS.map(&held);
        let prev = std::mem::replace(&mut self.prev, now);
        let seen = |vk| match MODIFIERS.iter().position(|&m| m == vk) {
            Some(i) => now[i] || prev[i],
            None => held(vk),
        };
        let event = resync(chord, Resync::Modifiers, seen);
        if paused { None } else { event }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TimerChange {
    /// Set the timer, the release guard starts over.
    Arm,
    /// Keep the timer, the release guard starts over: a new chord began in the same event where the old one ended.
    Rearm,
    Kill,
    Keep,
}

impl TimerChange {
    fn resets_debounce(self) -> bool {
        matches!(self, Self::Arm | Self::Rearm)
    }
}

/// The chord timer is needed exactly while the chord is held. `chord_down` — this event had a `ChordDown`:
/// the previous tick still saw the old chord, and its snapshot is no good for the new one.
fn timer_change(active: bool, armed: bool, chord_down: bool) -> TimerChange {
    match (active, armed) {
        (true, false) => TimerChange::Arm,
        (true, true) if chord_down => TimerChange::Rearm,
        (false, true) => TimerChange::Kill,
        _ => TimerChange::Keep,
    }
}

/// Sets and kills the chord timer. No logging: called from the hook procedure.
/// If `SetTimer` failed, the timer stays unset, and the next event tries again.
fn follow_chord(active: bool, chord_down: bool) {
    let timer = TIMER.get();
    let change = timer_change(active, timer != 0, chord_down);
    if change.resets_debounce() {
        DEBOUNCE.set(Debounce::ARMED);
    }
    match change {
        TimerChange::Arm => {
            // SAFETY: a thread timer — no window and no callback, it arrives as WM_TIMER in this thread's queue.
            TIMER.set(unsafe { SetTimer(None, 0, CHORD_TICK_MS, None) });
        }
        TimerChange::Kill => {
            // SAFETY: the timer was set by this thread.
            let _ = unsafe { KillTimer(None, timer) };
            TIMER.set(0);
        }
        TimerChange::Rearm | TimerChange::Keep => {}
    }
}

/// Chord timer tick (in the message loop, not in the hook procedure).
fn on_tick() {
    let active = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let (chord, tx) = s.as_mut()?;
        let mut debounce = DEBOUNCE.get();
        let event = debounce.tick(chord, PAUSED.load(Ordering::Relaxed), key_held);
        DEBOUNCE.set(debounce);
        if let Some(event) = event {
            let _ = tx.send(event);
        }
        Some(chord.active())
    });
    follow_chord(active.unwrap_or(false), false);
}

/// Asks the message loop to install or remove the mouse hook (`WM_MOUSE_WATCH`) when the wish has changed. No hook calls
/// and no logging: it runs in a hook procedure. A post that failed leaves the old wish, and the next event asks again.
fn want_mouse(wanted: bool) {
    if wanted == MOUSE_WANTED.get() {
        return;
    }
    // SAFETY: our own thread's id and constants.
    if unsafe { PostThreadMessageW(GetCurrentThreadId(), WM_MOUSE_WATCH, WPARAM(0), LPARAM(0)) }.is_ok() {
        MOUSE_WANTED.set(wanted);
    }
}

/// Installs or removes the mouse hook as `MOUSE_WANTED` says (in the message loop, never in a hook procedure). If Windows
/// did not install it, a click goes unseen and the Shift's toggle stands, as it would without the hook.
fn follow_mouse() {
    match mouse_change(MOUSE_WANTED.get(), MOUSE.get().is_some()) {
        MouseChange::Install => match set_hook(WH_MOUSE_LL, mouse_proc) {
            Ok(hook) => MOUSE.set(Some(hook)),
            Err(e) => tracing::warn!("mouse hook not installed, a Shift + click may switch the mode: {e}"),
        },
        MouseChange::Remove => unhook_mouse(),
        MouseChange::Keep => {}
    }
}

fn unhook_mouse() {
    if let Some(hook) = MOUSE.take() {
        // SAFETY: the hook was installed by this thread.
        let _ = unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// The mouse hook procedure, installed only while `Chord::watches_mouse`: a button press takes a Shift toggle back.
/// It never swallows the click — Shift + click still extends the selection — and does not log.
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: with HC_ACTION lparam points to an MSLLHOOKSTRUCT that is alive for the duration of the call.
        let flags = unsafe { (*(lparam.0 as *const MSLLHOOKSTRUCT)).flags };
        if mouse_button_down(wparam.0 as u32, flags) {
            let watch = STATE.with(|s| {
                let mut s = s.try_borrow_mut().ok()?;
                let (chord, tx) = s.as_mut()?;
                let paused = PAUSED.load(Ordering::Relaxed);
                if let Some(event) = feed_mouse(chord, paused, key_held) {
                    let _ = tx.send(event);
                }
                Some(chord.watches_mouse() && !paused)
            });
            // After a click nothing is left to take back: the hook goes.
            want_mouse(watch.unwrap_or(false));
        }
    }
    // SAFETY: the arguments are the ones Windows called the procedure with.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: with HC_ACTION lparam points to a KBDLLHOOKSTRUCT that is alive for the duration of the call.
        let ev = KeyEvent::from_hook(wparam.0 as u32, unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) });
        if let Some(how) = route(&ev) {
            let outcome = STATE.with(|s| {
                // There is no re-entry here (the mask is sent below), but a panic in the procedure would bring the process down.
                let mut s = s.try_borrow_mut().ok()?;
                let (chord, tx) = s.as_mut()?;
                chord.set_ask(ASK_ENABLED.load(Ordering::Relaxed));
                chord.set_recording(RECORDING.load(Ordering::Relaxed));
                let esc_active = ESC_ACTIVE.load(Ordering::Relaxed);
                let paused = PAUSED.load(Ordering::Relaxed);
                let (out, chord_down) = feed(chord, &ev, how, esc_active, paused, key_held);
                for event in out.events.into_iter().flatten() {
                    let _ = tx.send(event);
                }
                Some((out, chord.active(), chord_down, chord.watches_mouse() && !paused))
            });
            if let Some((out, active, chord_down, watch)) = outcome {
                follow_chord(active, chord_down);
                // The mouse hook lives while a Shift that toggled after a tap is held, when a click takes the toggle back.
                // The message loop installs it right after this procedure returns, long before a hand can click.
                want_mouse(watch);
                if ev.down {
                    let mut debounce = DEBOUNCE.get();
                    debounce.pressed(ev.vk);
                    DEBOUNCE.set(debounce);
                }
                // Outside the STATE borrow: SendInput from this thread calls the procedure again right away
                // (for the injected mask, which goes past `Chord`), and the mask reaches the system before this event.
                // The result is not checked: there is no logging in the procedure.
                if out.inject_mask_first {
                    send_mask_key();
                }
                if out.swallow {
                    return LRESULT(1);
                }
            }
        }
    }
    // SAFETY: the arguments are the ones Windows called the procedure with.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn install() -> windows::core::Result<HHOOK> {
    set_hook(WH_KEYBOARD_LL, hook_proc)
}

/// A low-level hook of this thread: its procedure runs here, while the message loop waits for messages.
fn set_hook(
    id: WINDOWS_HOOK_ID,
    procedure: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT,
) -> windows::core::Result<HHOOK> {
    // SAFETY: the module of our own exe; the procedure is a static `extern "system"` function.
    unsafe {
        let module: HINSTANCE = GetModuleHandleW(None)?.into();
        SetWindowsHookExW(id, Some(procedure), Some(module), 0)
    }
}

/// Starts the hook thread and waits until the hook is installed. A second call — `HookError::AlreadyRunning`.
/// If Windows did not install the hook, the error is returned here, the thread ends, and `spawn` can be called again.
pub fn spawn(tx: Sender<HotkeyEvent>) -> Result<JoinHandle<()>, HookError> {
    if SPAWNED.swap(true, Ordering::SeqCst) {
        return Err(HookError::AlreadyRunning);
    }
    let started = start(tx);
    if started.is_err() {
        SPAWNED.store(false, Ordering::SeqCst);
    }
    started
}

fn start(tx: Sender<HotkeyEvent>) -> Result<JoinHandle<()>, HookError> {
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let handle = std::thread::Builder::new().name("cue-keyboard".into()).spawn(move || run(tx, ready_tx))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(HookError::Install(e))
        }
        Err(_) => {
            let _ = handle.join();
            Err(HookError::Thread(std::io::Error::other("the hook thread ended before installing the hook")))
        }
    }
}

fn run(tx: Sender<HotkeyEvent>, ready: SyncSender<windows::core::Result<()>>) {
    // Windows silently removes a hook that takes too long to answer: the thread must not wait for its turn.
    // SAFETY: pseudo-handle of our own thread.
    if let Err(e) = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) } {
        tracing::warn!("keyboard hook: could not raise the thread priority: {e}");
    }
    let mut msg = MSG::default();
    // SAFETY: only creates the thread's message queue so that `reinstall` can post to it.
    let _ = unsafe { PeekMessageW(&mut msg, None, WM_APP, WM_APP, PM_NOREMOVE) };
    STATE.with(|s| *s.borrow_mut() = Some((Chord::default(), tx)));
    let mut hook = match install() {
        Ok(hook) => Some(hook),
        Err(e) => {
            tracing::error!("keyboard hook not installed: {e}");
            let _ = ready.send(Err(e));
            return;
        }
    };
    INSTALLED.store(true, Ordering::SeqCst);
    // SAFETY: no arguments.
    THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    tracing::info!("keyboard hook installed");
    let _ = ready.send(Ok(()));

    // SAFETY: an ordinary message loop of our own thread; 0 — WM_QUIT, -1 — error.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        if msg.message == WM_REINSTALL {
            hook = install_again(hook);
            continue;
        }
        if msg.message == WM_MOUSE_WATCH {
            follow_mouse();
            continue;
        }
        if msg.message == WM_TIMER && msg.wParam.0 != 0 && msg.wParam.0 == TIMER.get() {
            on_tick();
            continue;
        }
        // SAFETY: the message has just been received by this thread.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    THREAD_ID.store(0, Ordering::SeqCst);
    INSTALLED.store(false, Ordering::SeqCst);
    follow_chord(false, false);
    unhook_mouse();
    if let Some(hook) = hook {
        // SAFETY: the hook was installed by this thread.
        let _ = unsafe { UnhookWindowsHookEx(hook) };
    }
}

fn install_again(old: Option<HHOOK>) -> Option<HHOOK> {
    if let Some(old) = old {
        // SAFETY: the hook was installed by this thread; the procedure is not called while the thread is not waiting for messages.
        let _ = unsafe { UnhookWindowsHookEx(old) };
    }
    // Windows may have removed the mouse hook too: it goes in again below if it is still wanted.
    unhook_mouse();
    let state = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let (chord, tx) = s.as_mut()?;
        let paused = PAUSED.load(Ordering::Relaxed);
        if let Some(event) = after_gap(chord, paused, key_held) {
            let _ = tx.send(event);
        }
        Some((chord.active(), chord.watches_mouse() && !paused))
    });
    let (active, watch) = state.unwrap_or((false, false));
    follow_chord(active, false);
    MOUSE_WANTED.set(watch);
    follow_mouse();
    match install() {
        Ok(hook) => {
            INSTALLED.store(true, Ordering::SeqCst);
            tracing::info!("keyboard hook reinstalled");
            Some(hook)
        }
        Err(e) => {
            INSTALLED.store(false, Ordering::SeqCst);
            tracing::error!("keyboard hook not reinstalled, CUE does not hear the keys: {e}");
            None
        }
    }
}

/// Reinstall the hook (after sleep and unlock — Windows may have removed it).
/// Whether it worked is visible from `installed()` and in the log.
pub fn reinstall() {
    let id = THREAD_ID.load(Ordering::SeqCst);
    if id != 0 {
        // SAFETY: only a thread id and constants.
        if let Err(e) = unsafe { PostThreadMessageW(id, WM_REINSTALL, WPARAM(0), LPARAM(0)) } {
            tracing::error!("keyboard hook: could not request a reinstall: {e}");
        }
    }
}

/// Whether the hook is installed right now.
pub fn installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::{VK_LMENU, VK_LSHIFT, VK_MASK, VK_RMENU, VK_RSHIFT};
    use cue_core::flow::{Effect, Flow, Input, Ms};
    use cue_core::ui::Mode;
    use windows::Win32::UI::WindowsAndMessaging::{
        KBDLLHOOKSTRUCT_FLAGS, LLKHF_LOWER_IL_INJECTED, LLKHF_UP, LLMHF_LOWER_IL_INJECTED, WM_KEYUP, WM_LBUTTONUP,
        WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONUP, WM_SYSKEYUP,
    };
    use HotkeyEvent::*;

    const VK_A: u16 = 0x41;
    const SC_LCONTROL: u32 = 0x1D;

    fn key(vk: u16, down: bool) -> KeyEvent {
        KeyEvent { vk, scan: 0, down, injected: false }
    }

    /// The event through `route` and `feed`, as in the procedure; `None` — past `Chord`.
    fn hook_with(
        c: &mut Chord,
        ev: KeyEvent,
        esc_active: bool,
        paused: bool,
        held: impl Fn(u16) -> bool,
    ) -> Option<Outcome> {
        route(&ev).map(|how| feed(c, &ev, how, esc_active, paused, held).0)
    }

    /// The procedure and the message loop together, with the chord timer — like `follow_chord` and `on_tick`, but without Windows.
    struct Glue {
        chord: Chord,
        armed: bool,
        debounce: Debounce,
    }

    impl Glue {
        fn new() -> Self {
            Self { chord: Chord::default(), armed: false, debounce: Debounce::ARMED }
        }

        fn key(&mut self, ev: KeyEvent, held: impl Fn(u16) -> bool) -> Outcome {
            let how = route(&ev).expect("the event goes into Chord");
            let (out, chord_down) = feed(&mut self.chord, &ev, how, false, false, held);
            self.follow(chord_down);
            if ev.down {
                self.debounce.pressed(ev.vk);
            }
            out
        }

        fn tick(&mut self, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
            assert!(self.armed, "tick without a timer");
            let event = self.debounce.tick(&mut self.chord, false, held);
            self.follow(false);
            event
        }

        fn follow(&mut self, chord_down: bool) {
            let change = timer_change(self.chord.active(), self.armed, chord_down);
            if change.resets_debounce() {
                self.debounce = Debounce::ARMED;
            }
            match change {
                TimerChange::Arm => self.armed = true,
                TimerChange::Kill => self.armed = false,
                TimerChange::Rearm | TimerChange::Keep => {}
            }
        }
    }

    fn hook(c: &mut Chord, ev: KeyEvent, held: impl Fn(u16) -> bool) -> Option<Outcome> {
        hook_with(c, ev, false, false, held)
    }

    fn held_set(keys: &[u16]) -> impl Fn(u16) -> bool + '_ {
        move |vk| keys.contains(&vk)
    }

    fn events(a: Option<HotkeyEvent>, b: Option<HotkeyEvent>) -> Outcome {
        Outcome { events: [a, None, b], swallow: false, inject_mask_first: false }
    }

    const PASS: Outcome = Outcome { events: [None, None, None], swallow: false, inject_mask_first: false };
    const SWALLOW: Outcome = Outcome { events: [None, None, None], swallow: true, inject_mask_first: false };

    fn kb(vk: u16, scan: u32, flags: KBDLLHOOKSTRUCT_FLAGS) -> KBDLLHOOKSTRUCT {
        KBDLLHOOKSTRUCT { vkCode: u32::from(vk), scanCode: scan, flags, time: 0, dwExtraInfo: 0 }
    }

    #[test]
    fn from_hook_reads_direction_and_injected() {
        let none = KBDLLHOOKSTRUCT_FLAGS(0);
        assert_eq!(
            KeyEvent::from_hook(WM_KEYDOWN, &kb(VK_LCONTROL, SC_LCONTROL, none)),
            KeyEvent { vk: VK_LCONTROL, scan: SC_LCONTROL, down: true, injected: false }
        );
        assert!(KeyEvent::from_hook(WM_SYSKEYDOWN, &kb(VK_LWIN, 0x5B, none)).down, "press with Alt held");
        assert!(!KeyEvent::from_hook(WM_KEYUP, &kb(VK_V, 0x2F, LLKHF_UP)).down);
        assert!(!KeyEvent::from_hook(WM_SYSKEYUP, &kb(VK_LWIN, 0x5B, LLKHF_UP)).down);
        assert!(KeyEvent::from_hook(WM_KEYDOWN, &kb(VK_MASK, 0, LLKHF_INJECTED)).injected);
        let lower = KBDLLHOOKSTRUCT_FLAGS(LLKHF_INJECTED.0 | LLKHF_LOWER_IL_INJECTED.0);
        assert!(KeyEvent::from_hook(WM_KEYDOWN, &kb(VK_A, 0x1E, lower)).injected);
        assert!(!KeyEvent::from_hook(WM_KEYUP, &kb(VK_A, 0x1E, LLKHF_UP)).injected);
    }

    #[test]
    fn injected_events_bypass_chord() {
        for vk in [VK_LCONTROL, VK_LWIN, VK_LSHIFT, VK_V, VK_ESCAPE, VK_MASK] {
            for down in [true, false] {
                let ev = KeyEvent { vk, scan: 0, down, injected: true };
                assert_eq!(route(&ev), None, "{vk:#04x} down={down}");
            }
        }
    }

    #[test]
    fn altgr_fake_ctrl_bypasses_chord() {
        for down in [true, false] {
            assert_eq!(route(&KeyEvent { vk: VK_LCONTROL, scan: SC_ALTGR_LCONTROL, down, injected: false }), None);
        }
        let real = KeyEvent { vk: VK_LCONTROL, scan: SC_LCONTROL, down: true, injected: false };
        assert_eq!(route(&real), Some(Resync::Full));
    }

    #[test]
    fn altgr_with_win_is_not_a_chord() {
        // The fake LCtrl comes before the right Alt: if it reached Chord, the pair would form without Alt.
        let mut c = Chord::default();
        assert_eq!(hook(&mut c, key(VK_LWIN, true), held_set(&[])), Some(PASS));
        let fake = |down| KeyEvent { vk: VK_LCONTROL, scan: SC_ALTGR_LCONTROL, down, injected: false };
        assert_eq!(hook(&mut c, fake(true), held_set(&[VK_LWIN])), None);
        assert_eq!(hook(&mut c, key(VK_RMENU, true), held_set(&[VK_LWIN, VK_LCONTROL])), Some(PASS));
        assert_eq!(hook(&mut c, key(VK_RMENU, false), held_set(&[VK_LWIN, VK_LCONTROL, VK_RMENU])), Some(PASS));
        assert_eq!(hook(&mut c, fake(false), held_set(&[VK_LWIN, VK_LCONTROL])), None);
        assert_eq!(hook(&mut c, key(VK_LWIN, false), held_set(&[VK_LWIN])), Some(PASS), "no ChordUp and no mask");
    }

    #[test]
    fn ctrl_and_win_presses_resync_fully() {
        for vk in [VK_LCONTROL, VK_RCONTROL, VK_LWIN, VK_RWIN] {
            assert_eq!(route(&key(vk, true)), Some(Resync::Full), "press {vk:#04x}");
            assert_eq!(route(&key(vk, false)), Some(Resync::No), "release {vk:#04x}");
        }
    }

    #[test]
    fn v_press_resyncs_modifiers_only() {
        assert_eq!(route(&key(VK_V, true)), Some(Resync::Modifiers));
        assert_eq!(route(&key(VK_V, false)), Some(Resync::No));
    }

    #[test]
    fn other_presses_resync_modifiers_only() {
        for vk in [VK_ESCAPE, VK_A, VK_LSHIFT, VK_RMENU, VK_MASK] {
            assert_eq!(route(&key(vk, true)), Some(Resync::Modifiers), "press {vk:#04x}");
            assert_eq!(route(&key(vk, false)), Some(Resync::No), "release {vk:#04x}");
        }
    }

    #[test]
    fn lost_releases_then_letter_ends_chord() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        // UAC prompt: the Ctrl and Win releases went to the secure desktop, it was answered with the mouse, then ordinary typing.
        assert_eq!(hook(&mut c, key(VK_A, true), held_set(&[])), Some(events(Some(ChordUp), None)), "no ChordAborted");
        assert!(!c.active());
        assert_eq!(hook(&mut c, key(VK_A, false), held_set(&[VK_A])), Some(PASS));
    }

    #[test]
    fn shift_presses_inside_held_chord_toggle_the_mode() {
        // The resync before Shift leaves the held pair alone: every fresh Shift press toggles, its auto-repeat does not.
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        let pair = [VK_LCONTROL, VK_LWIN];
        let with_shift = [VK_LCONTROL, VK_LWIN, VK_LSHIFT];
        let toggle = Some(events(None, Some(ModeToggle)));
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&pair)), toggle);
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&with_shift)), Some(PASS));
        assert_eq!(hook(&mut c, key(VK_LSHIFT, false), held_set(&with_shift)), Some(PASS));
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&pair)), toggle);
        assert!(c.active(), "ChordUp only when the pair breaks up");
    }

    #[test]
    fn shift_after_a_tap_toggles_while_recording() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        hook(&mut c, key(VK_LWIN, false), held_set(&[VK_LCONTROL, VK_LWIN]));
        hook(&mut c, key(VK_LCONTROL, false), held_set(&[VK_LCONTROL]));
        let toggle = Some(events(None, Some(ModeToggle)));
        // The tap is over and the core records: the hook procedure passes the flag to `Chord` before every event.
        c.set_recording(true);
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[])), toggle);
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[VK_LSHIFT])), Some(PASS), "auto-repeat");
        // The Shift release was lost (UAC prompt): the resync before the next press drops the stale Shift.
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[])), toggle);
        assert_eq!(hook(&mut c, key(VK_LSHIFT, false), held_set(&[VK_LSHIFT])), Some(PASS));
        c.set_recording(false);
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[])), Some(PASS), "not recording");
        c.set_recording(true);
        let paused = hook_with(&mut c, key(VK_RSHIFT, true), false, true, held_set(&[VK_LSHIFT]));
        assert_eq!(paused, Some(PASS), "nothing while paused");
    }

    #[test]
    fn a_key_after_a_lost_shift_release_takes_nothing_back() {
        let mut c = Chord::default();
        c.set_recording(true);
        let toggle = Some(events(None, Some(ModeToggle)));
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[])), toggle);
        assert_eq!(hook(&mut c, key(VK_A, true), held_set(&[VK_LSHIFT])), Some(undo(None)), "Shift is still held");
        assert_eq!(hook(&mut c, key(VK_LSHIFT, true), held_set(&[VK_A])), toggle, "A was released, Shift pressed again");
        // The Shift release went to the secure desktop: the resync before A drops it, and the toggle stands.
        assert_eq!(hook(&mut c, key(VK_A, true), held_set(&[])), Some(PASS));
    }

    /// A Shift toggle taken back by this event, then the event's own `event`.
    fn undo(event: Option<HotkeyEvent>) -> Outcome {
        Outcome { events: [None, Some(ModeToggle), event], swallow: false, inject_mask_first: false }
    }

    /// Keys through the hook into a real `Flow`, joined the way the runtime joins them: the flow's answer sets the
    /// hook flags before the next key.
    struct Live {
        chord: Chord,
        flow: Flow,
        now: Ms,
        held: Vec<u16>,
    }

    impl Live {
        /// A recording started with a tap of `keys` (Shift among them makes it a translation).
        fn tapped(keys: &[u16]) -> Self {
            let mut live = Self { chord: Chord::default(), flow: Flow::new(0, false), now: 0, held: vec![] };
            live.keys(keys);
            live.now += 2_000;
            live
        }

        fn key(&mut self, vk: u16, down: bool) {
            self.now += 40;
            self.chord.set_recording(self.flow.recording_now());
            let held = self.held.clone();
            let ev = key(vk, down);
            let how = route(&ev).expect("the event goes into Chord");
            let (out, _) = feed(&mut self.chord, &ev, how, self.flow.esc_active(), false, |k| held.contains(&k));
            for event in out.events.into_iter().flatten() {
                self.flow.handle(input(event), self.now);
            }
            self.held.retain(|&k| k != vk);
            if down {
                self.held.push(vk);
            }
        }

        /// A mouse button press, through `feed_mouse` as in the mouse hook procedure.
        fn click(&mut self) {
            self.now += 40;
            let held = self.held.clone();
            if let Some(event) = feed_mouse(&mut self.chord, false, |k| held.contains(&k)) {
                self.flow.handle(input(event), self.now);
            }
        }

        /// Presses the keys in order, then releases them in the same order.
        fn keys(&mut self, keys: &[u16]) {
            keys.iter().for_each(|&vk| self.key(vk, true));
            keys.iter().for_each(|&vk| self.key(vk, false));
        }

        /// The mode the recording is refined in, once the finishing chord has been pressed.
        fn refined_in(&mut self) -> Option<Mode> {
            let id = self.flow.current_id();
            let fx = self.flow.handle(Input::SttFinal { id, text: "текст".into() }, self.now + 200);
            fx.into_iter().find_map(|e| if let Effect::Refine { mode, .. } = e { Some(mode) } else { None })
        }
    }

    /// What `runtime::input_of` does.
    fn input(event: HotkeyEvent) -> Input {
        match event {
            ChordDown(mode) => Input::ChordDown(mode),
            ModeChange(mode) => Input::ModeChange(mode),
            ModeToggle => Input::ModeToggle,
            ChordUp => Input::ChordUp,
            ChordAborted => Input::ChordAborted,
            Esc => Input::Esc,
            PasteLast => Input::PasteLast,
        }
    }

    #[test]
    fn the_finishing_chord_keeps_the_mode_whatever_the_order() {
        let starts = [(&[VK_LCONTROL, VK_LWIN][..], Mode::Dictate), (&[VK_LCONTROL, VK_LSHIFT, VK_LWIN][..], Mode::Translate)];
        let finishes = [[VK_LSHIFT, VK_LCONTROL, VK_LWIN], [VK_LCONTROL, VK_LSHIFT, VK_LWIN], [VK_LWIN, VK_LSHIFT, VK_LCONTROL]];
        for (start, mode) in starts {
            for finish in finishes {
                let mut live = Live::tapped(start);
                assert_eq!(live.flow.mode(), mode, "{start:x?}");
                live.keys(&finish);
                assert_eq!(live.refined_in(), Some(mode), "started with {start:x?}, finished with {finish:x?}");
            }
        }
    }

    #[test]
    fn shift_alone_switches_a_tapped_recording_and_shift_with_a_key_does_not() {
        let mut live = Live::tapped(&[VK_LCONTROL, VK_LWIN]);
        live.keys(&[VK_LSHIFT, VK_A]);
        assert_eq!(live.flow.mode(), Mode::Dictate, "a capital letter");
        live.keys(&[VK_LSHIFT]);
        assert_eq!(live.flow.mode(), Mode::Translate, "Shift on its own");
        live.keys(&[VK_RMENU, VK_LSHIFT]);
        live.keys(&[VK_LSHIFT, VK_RMENU]);
        assert_eq!(live.flow.mode(), Mode::Translate, "Alt + Shift and Shift + Alt");
        live.keys(&[VK_LCONTROL, VK_LWIN]);
        assert_eq!(live.refined_in(), Some(Mode::Translate));
    }

    /// Spec 2 §4: Shift + click extends a selection while CUE records; it does not switch the mode.
    #[test]
    fn shift_click_in_a_tapped_recording_keeps_its_mode() {
        for (start, mode) in [(&[VK_LCONTROL, VK_LWIN][..], Mode::Dictate), (&[VK_LCONTROL, VK_LSHIFT, VK_LWIN][..], Mode::Translate)] {
            let mut live = Live::tapped(start);
            assert_eq!(live.flow.mode(), mode, "{start:x?}");
            live.key(VK_LSHIFT, true);
            assert!(live.chord.watches_mouse(), "the mouse hook goes in");
            live.click();
            live.key(VK_LSHIFT, false);
            assert_eq!(live.flow.mode(), mode, "Shift + click, {start:x?}");
            assert!(!live.chord.watches_mouse(), "and out again");
            live.keys(&[VK_LCONTROL, VK_LWIN]);
            assert_eq!(live.refined_in(), Some(mode), "{start:x?}");
        }
        // A lone Shift press after a Shift + click still switches.
        let mut live = Live::tapped(&[VK_LCONTROL, VK_LWIN]);
        live.key(VK_LSHIFT, true);
        live.click();
        live.key(VK_LSHIFT, false);
        live.keys(&[VK_LSHIFT]);
        assert_eq!(live.flow.mode(), Mode::Translate);
        live.click();
        assert_eq!(live.flow.mode(), Mode::Translate, "a click with no Shift held takes nothing back");
        live.keys(&[VK_LCONTROL, VK_LWIN]);
        assert_eq!(live.refined_in(), Some(Mode::Translate));
    }

    #[test]
    fn only_a_real_button_press_is_a_click() {
        for msg in [WM_LBUTTONDOWN, WM_RBUTTONDOWN, WM_MBUTTONDOWN, WM_XBUTTONDOWN] {
            assert!(mouse_button_down(msg, 0), "{msg:#06x}");
        }
        for msg in [WM_MOUSEMOVE, WM_LBUTTONUP, WM_RBUTTONUP, WM_MOUSEWHEEL, WM_MOUSEHWHEEL] {
            assert!(!mouse_button_down(msg, 0), "{msg:#06x}");
        }
        assert!(!mouse_button_down(WM_LBUTTONDOWN, LLMHF_INJECTED), "a program's click");
        assert!(!mouse_button_down(WM_LBUTTONDOWN, LLMHF_INJECTED | LLMHF_LOWER_IL_INJECTED));
    }

    #[test]
    fn the_mouse_hook_lives_only_while_a_toggling_shift_is_held() {
        assert_eq!(mouse_change(true, false), MouseChange::Install);
        assert_eq!(mouse_change(false, true), MouseChange::Remove);
        assert_eq!(mouse_change(true, true), MouseChange::Keep);
        assert_eq!(mouse_change(false, false), MouseChange::Keep);
        let mut c = Chord::default();
        c.set_recording(true);
        assert!(!c.watches_mouse(), "recording, no Shift");
        hook(&mut c, key(VK_LSHIFT, true), held_set(&[]));
        assert!(c.watches_mouse(), "the Shift toggled");
        hook(&mut c, key(VK_LSHIFT, false), held_set(&[VK_LSHIFT]));
        assert!(!c.watches_mouse(), "released: its toggle stands");
        hook(&mut c, key(VK_LSHIFT, true), held_set(&[]));
        assert_eq!(hook(&mut c, key(VK_A, true), held_set(&[VK_LSHIFT])), Some(undo(None)));
        assert!(!c.watches_mouse(), "a key took the toggle back");
        let mut c = Chord::default();
        c.set_recording(true);
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        hook(&mut c, key(VK_LSHIFT, true), held_set(&[VK_LCONTROL, VK_LWIN]));
        assert!(!c.watches_mouse(), "inside a held chord a click takes nothing back");
    }

    #[test]
    fn a_paused_click_does_nothing() {
        let mut c = Chord::default();
        c.set_recording(true);
        hook(&mut c, key(VK_LSHIFT, true), held_set(&[]));
        assert_eq!(feed_mouse(&mut c, true, held_set(&[VK_LSHIFT])), None);
        assert!(!c.watches_mouse(), "Chord saw the click");
        hook(&mut c, key(VK_LSHIFT, false), held_set(&[VK_LSHIFT]));
        hook(&mut c, key(VK_LSHIFT, true), held_set(&[]));
        assert_eq!(feed_mouse(&mut c, false, held_set(&[VK_LSHIFT])), Some(ModeToggle), "not paused");
    }

    #[test]
    fn resync_event_comes_before_key_event() {
        let mut c = Chord::default();
        assert_eq!(hook(&mut c, key(VK_LCONTROL, true), held_set(&[])), Some(PASS));
        assert_eq!(hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL])), Some(events(None, Some(ChordDown(Mode::Dictate)))));
        // The Ctrl release went to the secure desktop (UAC prompt), Ctrl is pressed again.
        assert_eq!(
            hook(&mut c, key(VK_LCONTROL, true), held_set(&[VK_LWIN])),
            Some(events(Some(ChordUp), Some(ChordDown(Mode::Dictate))))
        );
    }

    #[test]
    fn lost_releases_end_chord_before_v() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        // The Ctrl and Win releases were lost; V is an ordinary letter.
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(events(Some(ChordUp), None)));
        assert_eq!(hook(&mut c, key(VK_V, false), held_set(&[VK_V])), Some(PASS));
    }

    #[test]
    fn held_paste_key_stays_swallowed() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        let paste = Outcome { events: [None, None, Some(PasteLast)], swallow: true, inject_mask_first: false };
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN])), Some(paste));
        let up = Outcome { events: [None, None, Some(ChordUp)], swallow: false, inject_mask_first: true };
        assert_eq!(hook(&mut c, key(VK_LWIN, false), held_set(&[VK_LCONTROL, VK_LWIN])), Some(up));
        assert_eq!(hook(&mut c, key(VK_LCONTROL, false), held_set(&[VK_LCONTROL])), Some(PASS));
        // V is still held; GetAsyncKeyState did not see the swallowed press.
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(SWALLOW), "V auto-repeat does not reach the app");
        assert_eq!(hook(&mut c, key(VK_V, false), held_set(&[])), Some(SWALLOW));
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(PASS), "a new V press passes");
    }

    #[test]
    fn esc_and_mask_are_carried_out() {
        let mut c = Chord::default();
        let esc = Outcome { events: [None, None, Some(Esc)], swallow: true, inject_mask_first: false };
        assert_eq!(hook_with(&mut c, key(VK_ESCAPE, true), true, false, held_set(&[])), Some(esc));
        assert_eq!(hook_with(&mut c, key(VK_ESCAPE, false), true, false, held_set(&[])), Some(SWALLOW));
        assert_eq!(hook_with(&mut c, key(VK_ESCAPE, true), false, false, held_set(&[])), Some(PASS));
    }

    #[test]
    fn paused_feeds_chord_but_acts_on_nothing() {
        let mut c = Chord::default();
        let paused = |c: &mut Chord, ev, held: &[u16]| hook_with(c, ev, true, true, held_set(held));
        assert_eq!(paused(&mut c, key(VK_LCONTROL, true), &[]), Some(PASS));
        assert_eq!(paused(&mut c, key(VK_LWIN, true), &[VK_LCONTROL]), Some(PASS), "no ChordDown");
        assert_eq!(paused(&mut c, key(VK_V, true), &[VK_LCONTROL, VK_LWIN]), Some(PASS), "the Windows Ctrl+Win+V works");
        assert_eq!(paused(&mut c, key(VK_V, false), &[VK_LCONTROL, VK_LWIN, VK_V]), Some(PASS));
        assert_eq!(paused(&mut c, key(VK_LWIN, false), &[VK_LCONTROL, VK_LWIN]), Some(PASS), "no mask");
        assert_eq!(paused(&mut c, key(VK_ESCAPE, true), &[VK_LCONTROL]), Some(PASS), "Esc is not swallowed");
        assert_eq!(paused(&mut c, key(VK_ESCAPE, false), &[VK_LCONTROL, VK_ESCAPE]), Some(PASS));
        // The pause is over, Ctrl is still held: Chord saw its press during the pause.
        assert_eq!(hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL])), Some(events(None, Some(ChordDown(Mode::Dictate)))));
    }

    #[test]
    fn paused_drops_resync_event() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        // Pause; the Ctrl release was lost, Ctrl is pressed again.
        assert_eq!(hook_with(&mut c, key(VK_LCONTROL, true), false, true, held_set(&[VK_LWIN])), Some(PASS));
        // Chord resynced during the pause: after it the Win release gives ChordUp for the chord formed during the pause.
        let up = Outcome { events: [None, None, Some(ChordUp)], swallow: false, inject_mask_first: true };
        assert_eq!(hook(&mut c, key(VK_LWIN, false), held_set(&[VK_LCONTROL, VK_LWIN])), Some(up));
    }

    #[test]
    fn after_gap_ends_chord_and_keeps_swallowed_v() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN]));
        // Sleep: the Ctrl and Win releases were lost, V is still held.
        assert_eq!(after_gap(&mut c, false, held_set(&[])), Some(ChordUp));
        assert_eq!(after_gap(&mut c, false, held_set(&[])), None, "ChordUp once");
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(SWALLOW), "V auto-repeat does not reach the app");
        assert_eq!(hook(&mut c, key(VK_V, false), held_set(&[])), Some(SWALLOW));

        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert_eq!(after_gap(&mut c, true, held_set(&[])), None, "no events while paused");
        assert_eq!(hook(&mut c, key(VK_LWIN, true), held_set(&[])), Some(PASS), "but Chord resynced: a lone Win");
    }

    #[test]
    fn paused_press_of_v_or_esc_is_not_marked_swallowed() {
        let mut c = Chord::default();
        let both = [VK_LCONTROL, VK_LWIN];
        hook_with(&mut c, key(VK_LCONTROL, true), true, true, held_set(&[]));
        hook_with(&mut c, key(VK_LWIN, true), true, true, held_set(&[VK_LCONTROL]));
        assert_eq!(hook_with(&mut c, key(VK_V, true), true, true, held_set(&both)), Some(PASS), "Windows panel");
        assert_eq!(hook_with(&mut c, key(VK_ESCAPE, true), true, true, held_set(&both)), Some(PASS));
        // The pause is over, V and Esc are still held: their releases reach the app, just like the presses.
        assert_eq!(hook_with(&mut c, key(VK_V, false), true, false, held_set(&[VK_LCONTROL, VK_LWIN, VK_V])), Some(PASS));
        assert_eq!(hook_with(&mut c, key(VK_ESCAPE, false), true, false, held_set(&both)), Some(PASS));
    }

    #[test]
    fn paused_release_clears_swallow_from_before_pause() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN])).unwrap().events[2], Some(PasteLast));
        hook(&mut c, key(VK_LWIN, false), held_set(&[VK_LCONTROL, VK_LWIN]));
        hook(&mut c, key(VK_LCONTROL, false), held_set(&[VK_LCONTROL]));
        // Pause; V is released during the pause — Chord sees it.
        assert_eq!(hook_with(&mut c, key(VK_V, false), false, true, held_set(&[])), Some(PASS));
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(PASS), "after the pause V is an ordinary letter");
    }

    #[test]
    fn timer_ends_lost_chord_on_second_tick() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert!(c.active());
        let mut d = Debounce::ARMED;
        // The releases were lost, no new presses.
        assert_eq!(d.tick(&mut c, false, held_set(&[])), None, "the first tick after arming releases nothing");
        assert!(c.active());
        assert_eq!(d.tick(&mut c, false, held_set(&[])), Some(ChordUp));
        assert!(!c.active());
        assert_eq!(d.tick(&mut c, false, held_set(&[])), None, "ChordUp once");
    }

    #[test]
    fn timer_ignores_one_tick_glitch() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        let mut d = Debounce::ARMED;
        // The tick came before Windows updated the Ctrl state.
        assert_eq!(d.tick(&mut c, false, held_set(&[VK_LWIN])), None);
        for held in [&[VK_LCONTROL, VK_LWIN][..], &[VK_LWIN], &[VK_LCONTROL, VK_LWIN], &[VK_LCONTROL], &[VK_LCONTROL, VK_LWIN]] {
            assert_eq!(d.tick(&mut c, false, held_set(held)), None, "{held:?}");
            assert!(c.active(), "{held:?}");
        }
        assert_eq!(d.tick(&mut c, false, held_set(&[VK_LWIN])), None);
        assert_eq!(d.tick(&mut c, false, held_set(&[VK_LWIN])), Some(ChordUp), "Ctrl not seen for two ticks in a row");
    }

    #[test]
    fn timer_keeps_swallowed_v() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_LCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN]));
        let mut d = Debounce::ARMED;
        d.tick(&mut c, false, held_set(&[]));
        assert_eq!(d.tick(&mut c, false, held_set(&[])), Some(ChordUp));
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[])), Some(SWALLOW), "auto-repeat of a held V does not reach the app");
        assert_eq!(hook(&mut c, key(VK_V, false), held_set(&[])), Some(SWALLOW));
    }

    #[test]
    fn timer_on_pause_syncs_silently() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_RCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_RWIN, true), held_set(&[VK_RCONTROL]));
        let mut d = Debounce::ARMED;
        assert_eq!(d.tick(&mut c, true, held_set(&[])), None);
        assert_eq!(d.tick(&mut c, true, held_set(&[])), None, "no events while paused");
        assert!(!c.active(), "but Chord resynced");
    }

    #[test]
    fn timer_follows_chord() {
        for chord_down in [false, true] {
            assert_eq!(timer_change(true, false, chord_down), TimerChange::Arm);
            assert_eq!(timer_change(false, true, chord_down), TimerChange::Kill);
            assert_eq!(timer_change(false, false, chord_down), TimerChange::Keep);
        }
        assert_eq!(timer_change(true, true, false), TimerChange::Keep);
        assert_eq!(timer_change(true, true, true), TimerChange::Rearm, "the chord formed again");
        assert!(TimerChange::Arm.resets_debounce() && TimerChange::Rearm.resets_debounce());
        assert!(!TimerChange::Keep.resets_debounce() && !TimerChange::Kill.resets_debounce());
    }

    #[test]
    fn feed_reports_chord_down_even_on_pause() {
        for paused in [false, true] {
            let mut c = Chord::default();
            let ctrl = feed(&mut c, &key(VK_LCONTROL, true), Resync::Full, false, paused, held_set(&[]));
            assert!(!ctrl.1, "paused={paused}");
            let win = feed(&mut c, &key(VK_LWIN, true), Resync::Full, false, paused, held_set(&[VK_LCONTROL]));
            assert!(win.1, "paused={paused}");
            let repeat = feed(&mut c, &key(VK_LWIN, true), Resync::Full, false, paused, held_set(&[VK_LCONTROL, VK_LWIN]));
            assert!(!repeat.1, "auto-repeat is not a new chord, paused={paused}");
        }
    }

    #[test]
    fn chord_restarted_in_one_event_rearms_debounce() {
        let mut g = Glue::new();
        g.key(key(VK_LCONTROL, true), held_set(&[]));
        assert_eq!(g.key(key(VK_LWIN, true), held_set(&[VK_LCONTROL])).events, [None, None, Some(ChordDown(Mode::Dictate))]);
        // The Ctrl release was lost; the tick already saw Ctrl released.
        assert_eq!(g.tick(held_set(&[VK_LWIN])), None);
        // Ctrl is pressed again: the resync ends the old chord, the press forms a new one — same timer.
        assert_eq!(g.key(key(VK_LCONTROL, true), held_set(&[VK_LWIN])).events, [Some(ChordUp), None, Some(ChordDown(Mode::Dictate))]);
        // The tick came before Windows updated the Ctrl state.
        assert_eq!(g.tick(held_set(&[VK_LWIN])), None, "no false ChordUp right after ChordDown");
        assert!(g.chord.active());
        assert_eq!(g.tick(held_set(&[VK_LCONTROL, VK_LWIN])), None);
        assert!(g.chord.active());
    }

    #[test]
    fn timer_does_not_turn_shift_auto_repeat_into_a_toggle() {
        let mut g = Glue::new();
        let pair = [VK_LCONTROL, VK_LWIN];
        g.key(key(VK_LCONTROL, true), held_set(&[]));
        g.key(key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(g.key(key(VK_LSHIFT, true), held_set(&pair)).events, [None, None, Some(ModeToggle)]);
        // The tick came before Windows updated the Shift state, and the previous tick did not see Shift either.
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(g.tick(held_set(&[VK_LCONTROL, VK_LWIN, VK_LSHIFT])), None);
        let repeat = g.key(key(VK_LSHIFT, true), held_set(&[VK_LCONTROL, VK_LWIN, VK_LSHIFT]));
        assert_eq!(repeat.events, [None, None, None], "the held Shift's auto-repeat does not toggle back");
    }

    #[test]
    fn timer_does_not_drop_a_fresh_alt_or_its_mask() {
        let mut g = Glue::new();
        g.chord.set_ask(true);
        let pair = [VK_LCONTROL, VK_LWIN];
        g.key(key(VK_LCONTROL, true), held_set(&[]));
        g.key(key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(g.key(key(VK_LMENU, true), held_set(&pair)).events, [None, None, Some(ModeChange(Mode::Ask))]);
        // The tick came before Windows recorded the Alt press, and the previous tick did not see Alt either.
        assert_eq!(g.tick(held_set(&pair)), None);
        let up = g.key(key(VK_LMENU, false), held_set(&[VK_LCONTROL, VK_LWIN, VK_LMENU]));
        assert!(up.inject_mask_first, "the Alt release is masked: the app's menu does not open");
    }

    #[test]
    fn timer_still_drops_an_alt_whose_release_was_lost() {
        let mut g = Glue::new();
        g.chord.set_ask(true);
        let pair = [VK_LCONTROL, VK_LWIN];
        g.key(key(VK_LCONTROL, true), held_set(&[]));
        g.key(key(VK_LWIN, true), held_set(&[VK_LCONTROL]));
        assert_eq!(g.tick(held_set(&pair)), None);
        g.key(key(VK_LMENU, true), held_set(&pair));
        // Alt's release never reached the hook: not seen on two ticks in a row, it is dropped.
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(
            g.key(key(VK_LMENU, true), held_set(&pair)).events,
            [None, None, Some(ModeChange(Mode::Ask))],
            "a new Alt press is fresh again"
        );
    }

    #[test]
    fn second_spawn_is_refused() {
        // The hook is not installed: the "already running" flag is set beforehand (there are no other spawn calls in tests).
        SPAWNED.store(true, Ordering::SeqCst);
        let (tx, _rx) = std::sync::mpsc::channel();
        let result = spawn(tx);
        SPAWNED.store(false, Ordering::SeqCst);
        assert!(matches!(result, Err(HookError::AlreadyRunning)), "{result:?}");
    }

    #[test]
    fn after_gap_keeps_held_pair() {
        let mut c = Chord::default();
        hook(&mut c, key(VK_RCONTROL, true), held_set(&[]));
        hook(&mut c, key(VK_RWIN, true), held_set(&[VK_RCONTROL]));
        assert_eq!(after_gap(&mut c, false, held_set(&[VK_RCONTROL, VK_RWIN])), None);
        let up = Outcome { events: [None, None, Some(ChordUp)], swallow: false, inject_mask_first: true };
        assert_eq!(hook(&mut c, key(VK_RWIN, false), held_set(&[VK_RCONTROL, VK_RWIN])), Some(up));
    }
}
