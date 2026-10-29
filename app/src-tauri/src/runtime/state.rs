//! Small executor rules with no OS and no network: timer generations, whose microphone frames to accept,
//! the insert watchdog, when to send the voice level and the answer view, when to warm up the connection and the refine
//! cache.
use std::collections::HashMap;

use cue_core::flow::{DictationId, TimerKind};
use cue_core::ui::UiPhase;

/// Flow timers. Each kind has its own generation: `arm` and `cancel` change it, so a firing of a cancelled
/// or re-armed timer (its task already woke up and the message is queued) never reaches `Flow`.
/// `H` is what cancels a running timer (in the app, a tokio task).
pub struct Timers<H> {
    generation: HashMap<TimerKind, u64>,
    armed: HashMap<TimerKind, (u64, H)>,
}

impl<H> Default for Timers<H> {
    fn default() -> Self {
        Self { generation: HashMap::new(), armed: HashMap::new() }
    }
}

impl<H> Timers<H> {
    /// A new generation for `kind`; `start` launches the timer with that generation.
    /// Returns the previous launch of the same kind if it was still waiting, so it can be cancelled.
    pub fn arm(&mut self, kind: TimerKind, start: impl FnOnce(u64) -> H) -> Option<H> {
        let generation = self.bump(kind);
        self.armed.insert(kind, (generation, start(generation))).map(|(_, h)| h)
    }

    /// Cancels the timer (the generation changes); returns its launch if it was waiting.
    pub fn cancel(&mut self, kind: TimerKind) -> Option<H> {
        self.bump(kind);
        self.armed.remove(&kind).map(|(_, h)| h)
    }

    /// A timer of generation `generation` fired: `true` if it is still current (and it no longer waits).
    pub fn fire(&mut self, kind: TimerKind, generation: u64) -> bool {
        let current = self.armed.get(&kind).is_some_and(|(g, _)| *g == generation);
        if current {
            self.armed.remove(&kind);
        }
        current
    }

    fn bump(&mut self, kind: TimerKind) -> u64 {
        let g = self.generation.entry(kind).or_insert(0);
        *g += 1;
        *g
    }
}

/// Whose microphone frames to accept: only those of the open `OpenMic`, and only until its `StopMic`.
/// Frames still queued after the stop reach neither the WAV, nor transcription, nor the level.
#[derive(Default)]
pub struct MicGate {
    open: Option<DictationId>,
}

impl MicGate {
    pub fn open(&mut self, id: DictationId) {
        self.open = Some(id);
    }

    pub fn close(&mut self, id: DictationId) {
        if self.open == Some(id) {
            self.open = None;
        }
    }

    pub fn accepts(&self, id: DictationId) -> bool {
        self.open == Some(id)
    }
}

/// Insert watchdog: the result must arrive within a limited time. We wait for one insert at a time (the flow does not
/// start the next one until it has the result of the previous one). A late result, after the deadline, is not accepted.
#[derive(Default)]
pub struct InsertWatch {
    next: u64,
    waiting: Option<(DictationId, u64)>,
}

impl InsertWatch {
    /// The insert of `id` has started; returns the generation for the deadline.
    pub fn start(&mut self, id: DictationId) -> u64 {
        self.next += 1;
        self.waiting = Some((id, self.next));
        self.next
    }

    /// The insert result of `id`: `true` if it was awaited (pass it to `Flow`).
    pub fn done(&mut self, id: DictationId) -> bool {
        self.waiting.take_if(|(w, _)| *w == id).is_some()
    }

    /// The deadline of the insert of `id`, generation `generation`, has passed: `true` if no result ever came
    /// (treat it as "no field").
    pub fn expired(&mut self, id: DictationId, generation: u64) -> bool {
        self.waiting.take_if(|w| *w == (id, generation)).is_some()
    }
}

/// At most ~30 times a second.
pub const LEVEL_EVERY_MS: u64 = 33;

/// The voice level is sent only while the Drop is listening: otherwise its frame loop never sleeps (~10 % of a core).
#[derive(Default)]
pub struct LevelGate {
    listening: bool,
    last: Option<u64>,
}

impl LevelGate {
    /// The phase just sent to the window.
    pub fn phase(&mut self, phase: UiPhase) {
        let listening = phase == UiPhase::Listen;
        if listening != self.listening {
            self.listening = listening;
            self.last = None;
        }
    }

    /// Whether to send the level now (`now` in ms).
    pub fn allow(&mut self, now: u64) -> bool {
        if !self.listening || self.last.is_some_and(|t| now.saturating_sub(t) < LEVEL_EVERY_MS) {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// Warm-up of the Anthropic connection: at most once every 20 s.
pub const WARM_EVERY_MS: u64 = 20_000;

#[derive(Default)]
pub struct WarmUp {
    last: Option<u64>,
}

impl WarmUp {
    /// `true` if it is time to warm up (and the next 20 s are counted from now).
    pub fn due(&mut self, now: u64) -> bool {
        if self.last.is_some_and(|t| now.saturating_sub(t) < WARM_EVERY_MS) {
            return false;
        }
        self.last = Some(now);
        true
    }

    /// After sleep or unlock the pooled connections are most likely dead: the next warm-up happens without waiting.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// The refine prompt cache lives 5 minutes from the start of the last request that used it (spec 2 §6.3).
pub const CACHE_TTL_MS: u64 = 5 * 60_000;
/// A warm-up goes out once the cache may expire before refine starts: the recording itself takes time, a minute of margin.
pub const CACHE_WARM_AFTER_MS: u64 = CACHE_TTL_MS - 60_000;

/// When to warm the refine cache: on a new dictation, if no refine or warm-up started in the last `CACHE_WARM_AFTER_MS`.
#[derive(Debug, Default)]
pub struct CacheWarm {
    last: Option<u64>,
}

impl CacheWarm {
    /// A request that reads the cache starts now.
    pub fn touched(&mut self, now: u64) {
        self.last = Some(now);
    }

    /// Whether to warm now; a `true` counts as a touch.
    pub fn due(&mut self, now: u64) -> bool {
        if self.last.is_some_and(|t| now.saturating_sub(t) < CACHE_WARM_AFTER_MS) {
            return false;
        }
        self.last = Some(now);
        true
    }

    /// After sleep the clock may have skipped: warm on the next dictation.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// A task in flight (transcription or refine). Each launch has its own generation: by it, the result of a cancelled
/// or replaced task of the same dictation is recognized, does not go to the flow, and does not clear the mark of
/// the new task.
/// `H` is what cancels the task (in the app, `Option<JoinHandle>`).
pub struct TaskSlot<H> {
    next: u64,
    current: Option<(DictationId, u64, H)>,
}

impl<H> Default for TaskSlot<H> {
    fn default() -> Self {
        Self { next: 0, current: None }
    }
}

impl<H> TaskSlot<H> {
    /// A new task of dictation `id`; `start` gets its generation. Returns the previous task if it was still running.
    pub fn start(&mut self, id: DictationId, start: impl FnOnce(u64) -> H) -> Option<H> {
        self.next += 1;
        let generation = self.next;
        self.current.replace((id, generation, start(generation))).map(|(_, _, h)| h)
    }

    /// A result of the task of generation `generation` arrived: `true` if it is the current task (the mark is
    /// cleared), `false` for a late result.
    pub fn finished(&mut self, generation: u64) -> bool {
        self.current.take_if(|(_, g, _)| *g == generation).is_some()
    }

    /// Cancel the task of dictation `id` if it is running; its result is not accepted after that.
    pub fn take_for(&mut self, id: DictationId) -> Option<H> {
        self.current.take_if(|(i, _, _)| *i == id).map(|(_, _, h)| h)
    }

    /// Whether the task of generation `generation` is the current one (a step of a stream that is still wanted).
    #[cfg_attr(not(test), expect(dead_code))]
    pub fn running(&self, generation: u64) -> bool {
        self.current.as_ref().is_some_and(|(_, g, _)| *g == generation)
    }

    /// Whether the task in flight belongs to dictation `id` (an answer still printing when a follow-up starts).
    #[cfg_attr(not(test), expect(dead_code))]
    pub fn is_for(&self, id: DictationId) -> bool {
        self.current.as_ref().is_some_and(|(i, _, _)| *i == id)
    }
}

/// While the words of an answer stream, the view goes to the windows at most this often: every delta would redraw the
/// answer up to 50 times a second (spec 2 §7 reveals words at its own pace anyway).
#[cfg_attr(not(test), expect(dead_code))]
pub const VIEW_EVERY_MS: u64 = 50;

/// What to do with a change of the answer view.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewSend {
    Now,
    /// Send it in this many ms (one flush).
    Later(u64),
    /// A flush is already on its way: it will carry this change too.
    Wait,
}

/// When the answer view goes to the windows while it streams.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug, Default)]
pub struct ViewGate {
    last: Option<u64>,
    flush_armed: bool,
}

#[cfg_attr(not(test), expect(dead_code))]
impl ViewGate {
    /// A change at `now`.
    pub fn change(&mut self, now: u64) -> ViewSend {
        match self.last {
            Some(t) if now.saturating_sub(t) < VIEW_EVERY_MS => {
                if self.flush_armed {
                    ViewSend::Wait
                } else {
                    self.flush_armed = true;
                    ViewSend::Later(VIEW_EVERY_MS - now.saturating_sub(t))
                }
            }
            _ => {
                self.last = Some(now);
                ViewSend::Now
            }
        }
    }

    /// The flush fired at `now`: the view goes out.
    pub fn flushed(&mut self, now: u64) {
        self.flush_armed = false;
        self.last = Some(now);
    }

    /// The view went out at once (a new turn, the end, the opening).
    pub fn sent(&mut self, now: u64) {
        self.last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TimerKind as K;

    #[test]
    fn timer_fires_only_with_its_generation() {
        let mut t: Timers<u64> = Timers::default();
        assert_eq!(t.arm(K::DoneHide, |g| g), None);
        let first = 1;
        // Re-armed: the previous launch is returned so it can be cancelled.
        assert_eq!(t.arm(K::DoneHide, |g| g), Some(first));
        let second = 2;
        assert!(!t.fire(K::DoneHide, first), "the previous launch woke up");
        assert!(t.fire(K::DoneHide, second));
        assert!(!t.fire(K::DoneHide, second), "a fired timer no longer waits");
    }

    #[test]
    fn cancelled_timer_never_reaches_flow() {
        let mut t: Timers<u64> = Timers::default();
        t.arm(K::UndoExpire, |g| g);
        assert_eq!(t.cancel(K::UndoExpire), Some(1));
        assert!(!t.fire(K::UndoExpire, 1), "the message of the cancelled timer was already queued");
        assert_eq!(t.cancel(K::UndoExpire), None, "nothing to cancel");
        // A new launch after a cancel is a new generation; the old one still does not count.
        // Generations: arm 1, cancel 2, cancel 3, arm 4.
        t.arm(K::UndoExpire, |g| g);
        for stale in [1, 2, 3] {
            assert!(!t.fire(K::UndoExpire, stale), "generation {stale}");
        }
        assert!(t.fire(K::UndoExpire, 4), "cancel changes the generation too");
    }

    #[test]
    fn timer_kinds_are_independent() {
        let mut t: Timers<()> = Timers::default();
        let gen_of = |t: &mut Timers<()>, k| {
            let mut got = 0;
            t.arm(k, |g| got = g);
            got
        };
        let a = gen_of(&mut t, K::StartingDone);
        let b = gen_of(&mut t, K::HoldCheck);
        t.cancel(K::StartingDone);
        assert!(!t.fire(K::StartingDone, a));
        assert!(t.fire(K::HoldCheck, b), "cancelling one kind does not touch another");
        assert!(!t.fire(K::AutoFinish, 1), "a timer that was never armed");
    }

    #[test]
    fn mic_frames_only_from_open_dictation() {
        let (one, two) = (DictationId(1), DictationId(2));
        let mut g = MicGate::default();
        assert!(!g.accepts(one), "the microphone is not open");
        g.open(one);
        assert!(g.accepts(one));
        assert!(!g.accepts(two), "another dictation's id");
        g.close(two);
        assert!(g.accepts(one), "stopping another dictation does not close the current one");
        g.close(one);
        assert!(!g.accepts(one), "frames still queued after StopMic");
        g.open(two);
        assert!(!g.accepts(one) && g.accepts(two), "a new recording: only its frames");
    }

    #[test]
    fn insert_result_is_taken_once_and_not_after_timeout() {
        let (one, two) = (DictationId(1), DictationId(2));
        let mut w = InsertWatch::default();
        assert!(!w.done(one), "there was no insert");
        let g1 = w.start(one);
        assert!(!w.done(two), "another dictation's result");
        assert!(w.done(one));
        assert!(!w.done(one), "a repeated result");
        assert!(!w.expired(one, g1), "the result came in time: the deadline does nothing");

        let g2 = w.start(two);
        assert_ne!(g1, g2);
        assert!(!w.expired(two, g1), "the deadline of the previous insert");
        assert!(w.expired(two, g2), "no result: \"no field\"");
        assert!(!w.done(two), "a late result is not accepted");
        assert!(!w.expired(two, g2));
    }

    #[test]
    fn level_only_while_listening_and_at_most_30_per_second() {
        let mut l = LevelGate::default();
        assert!(!l.allow(0), "idle");
        l.phase(UiPhase::Start);
        assert!(!l.allow(10), "start-up: not listening yet");
        l.phase(UiPhase::Listen);
        assert!(l.allow(400));
        assert!(!l.allow(420), "more than 30 times a second");
        assert!(l.allow(433));
        l.phase(UiPhase::Think);
        assert!(!l.allow(500), "the recording is over: the level does not wake the Drop");
        l.phase(UiPhase::Listen);
        assert!(l.allow(510), "a new recording starts without waiting");
        for p in [UiPhase::Idle, UiPhase::Done, UiPhase::Empty, UiPhase::Cancel, UiPhase::Error] {
            l.phase(p);
            assert!(!l.allow(10_000), "{p:?}");
        }
    }

    #[test]
    fn warm_up_at_most_every_20_seconds() {
        let mut w = WarmUp::default();
        assert!(w.due(1_000));
        assert!(!w.due(5_000));
        assert!(!w.due(20_999));
        assert!(w.due(21_000));
        assert!(!w.due(21_001));
        // Woke up: the pooled connections are most likely dead, so warm up right away.
        w.reset();
        assert!(w.due(22_000));
        assert!(!w.due(22_001));
    }

    #[test]
    fn cache_warm_up_after_a_pause_only() {
        const MIN: u64 = 60_000;
        let mut c = CacheWarm::default();
        assert!(c.due(0), "the first dictation of the session");
        assert!(!c.due(3 * MIN), "the warm-up 3 minutes ago still holds");
        let mut c = CacheWarm::default();
        assert!(c.due(0));
        c.touched(3 * MIN);
        assert!(!c.due(6 * MIN), "a refine 3 minutes ago kept the cache");
        assert!(c.due(8 * MIN), "5 minutes since the refine: the cache may expire while the user speaks");
        assert!(!c.due(8 * MIN + 1), "a warm-up counts as a touch");
        c.reset();
        assert!(c.due(8 * MIN + 2), "after sleep the clock may have skipped");
        assert_eq!(CACHE_WARM_AFTER_MS, 4 * MIN);
    }

    #[test]
    fn stale_task_result_keeps_the_newer_task() {
        let (one, two) = (DictationId(1), DictationId(2));
        let mut s: TaskSlot<u64> = TaskSlot::default();
        assert!(!s.finished(1), "there were no tasks");
        let mut first = 0;
        assert_eq!(s.start(one, |g| { first = g; g }), None);
        // Esc → "restore": the same dictation, a new task; the previous one is returned so it can be cancelled.
        let mut second = 0;
        assert_eq!(s.start(one, |g| { second = g; g }), Some(first));
        assert!(!s.finished(first), "the result of a cancelled task of the same dictation");
        assert_eq!(s.take_for(two), None, "another dictation");
        assert!(s.finished(second), "the result of the new task is accepted");
        assert!(!s.finished(second), "and only once");
        let third = {
            let mut g = 0;
            s.start(two, |x| { g = x; x });
            g
        };
        assert_eq!(s.take_for(two), Some(third), "cancel");
        assert!(!s.finished(third), "the result of a cancelled task is not accepted");
    }

    #[test]
    fn the_answer_view_goes_out_at_most_every_50_ms() {
        let mut g = ViewGate::default();
        assert_eq!(g.change(0), ViewSend::Now);
        assert_eq!(g.change(20), ViewSend::Later(30));
        assert_eq!(g.change(30), ViewSend::Wait, "the armed flush carries it");
        g.flushed(50);
        assert_eq!(g.change(60), ViewSend::Later(40));
        g.flushed(100);
        assert_eq!(g.change(160), ViewSend::Now);
        g.sent(170);
        assert_eq!(g.change(180), ViewSend::Later(40));
    }

    #[test]
    fn a_running_task_is_recognized_without_being_cleared() {
        let mut slot: TaskSlot<()> = TaskSlot::default();
        slot.start(DictationId(1), |_| ());
        assert!(slot.running(1) && !slot.running(2));
        assert!(slot.is_for(DictationId(1)) && !slot.is_for(DictationId(2)));
        assert!(slot.running(1), "asking does not clear");
        assert!(slot.finished(1));
        assert!(!slot.running(1) && !slot.is_for(DictationId(1)));
    }
}
