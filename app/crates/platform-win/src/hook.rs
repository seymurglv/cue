//! Low-level keyboard hook on its own thread. The hook procedure only classifies the event
//! and hands it to a channel: Windows removes the hook if the procedure takes too long to answer.
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
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_INJECTED, MSG,
    PM_NOREMOVE, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_SYSKEYDOWN, WM_TIMER,
};

use crate::chord::{
    Chord, Decision, HotkeyEvent, VK_ESCAPE, VK_LCONTROL, VK_LSHIFT, VK_LWIN, VK_RCONTROL, VK_RSHIFT, VK_RWIN, VK_V,
};
use crate::input::{key_held, send_mask_key, MODIFIERS};

/// Swallow Esc — while CUE is recording, thinking or showing an error. Set by the core.
pub static ESC_ACTIVE: AtomicBool = AtomicBool::new(false);
/// CUE is starting or recording: a fresh Shift press toggles the mode even after the chord was released. Set by the
/// core. Not `ESC_ACTIVE`: that one also covers thinking and errors, when the user may already be typing again, and
/// every capital letter would then wake the core for nothing.
pub static RECORDING: AtomicBool = AtomicBool::new(false);
/// Ctrl + Win + Alt is the "Ask" chord. Off until "Ask" exists (spec 2, queue 2b): Alt keeps making a system shortcut.
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
    /// Events for the core, in order: first from the resync, then from the event itself.
    events: [Option<HotkeyEvent>; 2],
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
    let outcome = Outcome { events: [first, d.event], swallow: d.swallow, inject_mask_first: d.inject_mask_first };
    (outcome, chord_down)
}

/// Resync after a gap (sleep, lock): releases during that time never reached the hook.
/// Leaves swallowed V and Esc alone, gives no events while paused.
fn after_gap(chord: &mut Chord, paused: bool, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
    let event = resync(chord, Resync::Modifiers, held);
    if paused { None } else { event }
}

/// Timer resync while the chord is held: it ends even if the Ctrl and Win releases never reached the hook
/// (UAC prompt, Ctrl+Alt+Del, lock screen, elevated window) and there are no new presses.
/// A key counts as released only if it is not seen held on this tick or the previous one: a tick may come
/// between `Chord` seeing the press and Windows updating `GetAsyncKeyState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Debounce {
    /// Whether the modifier (`MODIFIERS`) was held on the previous tick.
    prev: [bool; 8],
}

impl Debounce {
    /// A freshly set timer: nothing is released on the first tick.
    const ARMED: Self = Self { prev: [true; 8] };

    /// Like `after_gap`, but taking the previous tick into account. While paused `Chord` resyncs, no events.
    fn tick(&mut self, chord: &mut Chord, paused: bool, held: impl Fn(u16) -> bool) -> Option<HotkeyEvent> {
        let now = MODIFIERS.map(&held);
        let prev = std::mem::replace(&mut self.prev, now);
        let seen = |vk| match MODIFIERS.iter().position(|&m| m == vk) {
            // Shift is never dropped here. A Shift pressed inside the held chord is new to the previous tick as well, so
            // a tick that comes before Windows updates its state would drop it, and its auto-repeat would toggle the mode
            // back. The timer only has to end a stuck chord; a Shift whose release was lost is dropped by the resync
            // before the next press, the only place that reads it.
            _ if vk == VK_LSHIFT || vk == VK_RSHIFT => true,
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
                let (out, chord_down) = feed(chord, &ev, how, esc_active, PAUSED.load(Ordering::Relaxed), key_held);
                for event in out.events.into_iter().flatten() {
                    let _ = tx.send(event);
                }
                Some((out, chord.active(), chord_down))
            });
            if let Some((out, active, chord_down)) = outcome {
                follow_chord(active, chord_down);
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
    // SAFETY: the module of our own exe; the procedure is a static `extern "system"` function.
    unsafe {
        let module: HINSTANCE = GetModuleHandleW(None)?.into();
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(module), 0)
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
    let active = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let (chord, tx) = s.as_mut()?;
        if let Some(event) = after_gap(chord, PAUSED.load(Ordering::Relaxed), key_held) {
            let _ = tx.send(event);
        }
        Some(chord.active())
    });
    follow_chord(active.unwrap_or(false), false);
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
    use crate::chord::{VK_MASK, VK_RMENU};
    use cue_core::ui::Mode;
    use windows::Win32::UI::WindowsAndMessaging::{
        KBDLLHOOKSTRUCT_FLAGS, LLKHF_LOWER_IL_INJECTED, LLKHF_UP, WM_KEYUP, WM_SYSKEYUP,
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
        Outcome { events: [a, b], swallow: false, inject_mask_first: false }
    }

    const PASS: Outcome = Outcome { events: [None, None], swallow: false, inject_mask_first: false };
    const SWALLOW: Outcome = Outcome { events: [None, None], swallow: true, inject_mask_first: false };

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
        let paste = Outcome { events: [None, Some(PasteLast)], swallow: true, inject_mask_first: false };
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN])), Some(paste));
        let up = Outcome { events: [None, Some(ChordUp)], swallow: false, inject_mask_first: true };
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
        let esc = Outcome { events: [None, Some(Esc)], swallow: true, inject_mask_first: false };
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
        let up = Outcome { events: [None, Some(ChordUp)], swallow: false, inject_mask_first: true };
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
        assert_eq!(hook(&mut c, key(VK_V, true), held_set(&[VK_LCONTROL, VK_LWIN])).unwrap().events[1], Some(PasteLast));
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
        assert_eq!(g.key(key(VK_LWIN, true), held_set(&[VK_LCONTROL])).events, [None, Some(ChordDown(Mode::Dictate))]);
        // The Ctrl release was lost; the tick already saw Ctrl released.
        assert_eq!(g.tick(held_set(&[VK_LWIN])), None);
        // Ctrl is pressed again: the resync ends the old chord, the press forms a new one — same timer.
        assert_eq!(g.key(key(VK_LCONTROL, true), held_set(&[VK_LWIN])).events, [Some(ChordUp), Some(ChordDown(Mode::Dictate))]);
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
        assert_eq!(g.key(key(VK_LSHIFT, true), held_set(&pair)).events, [None, Some(ModeToggle)]);
        // The tick came before Windows updated the Shift state, and the previous tick did not see Shift either.
        assert_eq!(g.tick(held_set(&pair)), None);
        assert_eq!(g.tick(held_set(&[VK_LCONTROL, VK_LWIN, VK_LSHIFT])), None);
        let repeat = g.key(key(VK_LSHIFT, true), held_set(&[VK_LCONTROL, VK_LWIN, VK_LSHIFT]));
        assert_eq!(repeat.events, [None, None], "the held Shift's auto-repeat does not toggle back");
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
        let up = Outcome { events: [None, Some(ChordUp)], swallow: false, inject_mask_first: true };
        assert_eq!(hook(&mut c, key(VK_RWIN, false), held_set(&[VK_RCONTROL, VK_RWIN])), Some(up));
    }
}
