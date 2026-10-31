//! The microphone's live level in a window: the main window's settings (spec 3 §3) and the welcome window's microphone
//! step (spec 4a §1). Only the level goes to the page, at most ~30 times a second; the audio itself is never kept and
//! never leaves this module. One preview at a time: a new one replaces the old, whichever window started it. It belongs
//! to the window that started it: its level goes there, a microphone that fails is told there (`cue://mic-failed`),
//! and only that window ends it — by its page's command (`commands::mic_preview`) or when it is destroyed
//! (`main_window`, `welcome_window`). It opens the microphone in the shared mode, as a recording does, so a dictation
//! meanwhile records as usual.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cue_platform_win::mic::{self, MicEvent, MicHandle};
use tauri::{AppHandle, Emitter};

/// At most ~30 levels a second, as the Drop gets.
const LEVEL_EVERY: Duration = Duration::from_millis(33);

/// The preview running now, and the window it belongs to.
static PREVIEW: Mutex<Option<Owned<Preview>>> = Mutex::new(None);

/// A preview and the label of the window that started it.
struct Owned<P> {
    owner: String,
    preview: P,
}

struct Preview {
    mic: MicHandle,
    /// Cleared when it ends: levels still on their way are not sent.
    live: Arc<AtomicBool>,
}

impl Preview {
    /// No more levels at once; the microphone closes off this thread (its driver may take a while), and if that thread
    /// does not start, the dropped handle closes it.
    fn end(self) {
        self.live.store(false, Ordering::Relaxed);
        let mic = self.mic;
        if let Err(e) = std::thread::Builder::new().name("cue-mic-stop".into()).spawn(move || mic.stop()) {
            tracing::warn!("microphone preview: stop thread not started ({e}), the microphone will close by itself");
        }
    }
}

/// The level of `device` (`None` — the Windows microphone; a pinned one that is not there — the Windows one too) goes
/// to the window `owner` as `cue://mic-level`. Returns at once: the microphone opens on its own thread.
pub fn start(app: &AppHandle, owner: &str, device: Option<String>) {
    // One at a time: the one running ends, whichever window started it.
    let running = slot().take();
    take_over(app, owner, running);
    let live = Arc::new(AtomicBool::new(true));
    let (events, rx) = std::sync::mpsc::channel();
    let (handle, to, on) = (app.clone(), owner.to_owned(), Arc::clone(&live));
    let forward =
        std::thread::Builder::new().name("cue-mic-preview".into()).spawn(move || forward(&handle, &to, &rx, &on));
    if let Err(e) = forward {
        tracing::warn!("microphone preview not started: {e}");
        return;
    }
    tracing::info!(pinned = device.is_some(), window = owner, "microphone preview started");
    let mic = mic::open(device, events);
    // Another start in between (each page asks one at a time, so only in a race): the last one stays.
    let other = slot().replace(Owned { owner: owner.to_owned(), preview: Preview { mic, live } });
    take_over(app, owner, other);
}

/// Ends `ended`, which a preview of the window `owner` replaces. If another window started it, that window gets a
/// level of 0, so its meter drops rather than freezing at its last level.
fn take_over(app: &AppHandle, owner: &str, ended: Option<Owned<Preview>>) {
    let other = taken_from(ended.as_ref(), owner).map(str::to_owned);
    end(ended);
    if let Some(other) = other {
        send(app, &other, "cue://mic-level", 0.0);
    }
}

/// The window whose preview `owner` takes over, if it is another one.
fn taken_from<'a, P>(ended: Option<&'a Owned<P>>, owner: &str) -> Option<&'a str> {
    ended.map(|p| p.owner.as_str()).filter(|&other| other != owner)
}

/// The preview ends, if one runs and the window `owner` started it.
pub fn stop(owner: &str) {
    let ended = take_owned(&mut slot(), owner);
    end(ended);
}

fn end(ended: Option<Owned<Preview>>) {
    if let Some(Owned { owner, preview }) = ended {
        preview.end();
        tracing::info!(window = owner.as_str(), "microphone preview stopped");
    }
}

/// The preview in `slot`, if the window `owner` started it: another window's stays.
fn take_owned<P>(slot: &mut Option<Owned<P>>, owner: &str) -> Option<Owned<P>> {
    slot.take_if(|p| p.owner == owner)
}

fn slot() -> MutexGuard<'static, Option<Owned<Preview>>> {
    PREVIEW.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Frames → their level, to the window `owner`; the samples are dropped with it. Ends when the preview has ended or the
/// microphone's thread has.
fn forward(app: &AppHandle, owner: &str, events: &Receiver<MicEvent>, live: &AtomicBool) {
    let mut last: Option<Instant> = None;
    for ev in events {
        if !live.load(Ordering::Relaxed) {
            break;
        }
        match ev {
            MicEvent::Frame { level, .. } => {
                if last.is_some_and(|t| t.elapsed() < LEVEL_EVERY) {
                    continue;
                }
                last = Some(Instant::now());
                // Taken over since this frame was read: its level would come after the 0 the takeover sent.
                if !live.load(Ordering::Relaxed) {
                    break;
                }
                send(app, owner, "cue://mic-level", level);
            }
            // The page shows a pinned microphone that is not there itself; meanwhile the level is the Windows one's.
            MicEvent::Fallback { .. } => {}
            // The meter drops, and the page learns that it is no silence: the microphone did not open, or stopped
            // (Windows may keep apps off it). Nothing more about it.
            MicEvent::Failed(e) => {
                tracing::warn!("microphone preview: {e}");
                send(app, owner, "cue://mic-level", 0.0);
                send(app, owner, "cue://mic-failed", ());
            }
        }
    }
}

fn send<S: serde::Serialize + Clone>(app: &AppHandle, owner: &str, event: &str, payload: S) {
    if let Err(e) = app.emit_to(owner, event, payload) {
        tracing::debug!(window = owner, "event {event} not sent: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{MAIN_LABEL, WELCOME_LABEL};

    /// The welcome window's microphone step and the main window's settings may both be open: either window's "off" or
    /// end stops only a preview it started itself.
    #[test]
    fn preview_belongs_to_the_window_that_started_it() {
        let mut slot = Some(Owned { owner: WELCOME_LABEL.to_owned(), preview: 1 });
        assert!(take_owned(&mut slot, MAIN_LABEL).is_none(), "the main window closing leaves the welcome window's");
        assert!(take_owned(&mut slot, "drop").is_none());
        assert_eq!(slot.as_ref().map(|p| p.preview), Some(1));
        assert_eq!(take_owned(&mut slot, WELCOME_LABEL).map(|p| p.preview), Some(1));
        assert!(slot.is_none() && take_owned(&mut slot, WELCOME_LABEL).is_none(), "ended once");
    }

    /// A start replaces the one running, whoever started it: the window it is taken from is told its level is 0, the
    /// window that starts it again (another microphone chosen) is not.
    #[test]
    fn a_preview_taken_over_is_told_to_the_window_it_was_taken_from() {
        let main = Owned { owner: MAIN_LABEL.to_owned(), preview: 2 };
        assert_eq!(taken_from(Some(&main), WELCOME_LABEL), Some(MAIN_LABEL));
        assert_eq!(taken_from(Some(&main), MAIN_LABEL), None);
        assert_eq!(taken_from::<u8>(None, WELCOME_LABEL), None);
    }
}
