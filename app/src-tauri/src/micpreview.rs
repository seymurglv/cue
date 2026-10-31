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
    end(running);
    let live = Arc::new(AtomicBool::new(true));
    let (events, rx) = std::sync::mpsc::channel();
    let (app, to, on) = (app.clone(), owner.to_owned(), Arc::clone(&live));
    let forward =
        std::thread::Builder::new().name("cue-mic-preview".into()).spawn(move || forward(&app, &to, &rx, &on));
    if let Err(e) = forward {
        tracing::warn!("microphone preview not started: {e}");
        return;
    }
    tracing::info!(pinned = device.is_some(), window = owner, "microphone preview started");
    let mic = mic::open(device, events);
    // Another start in between (each page asks one at a time, so only in a race): the last one stays.
    if let Some(other) = slot().replace(Owned { owner: owner.to_owned(), preview: Preview { mic, live } }) {
        other.preview.end();
    }
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

        // A start replaces the one running, whoever started it; from then on it is the new window's.
        let mut slot = Some(Owned { owner: MAIN_LABEL.to_owned(), preview: 2 });
        let replaced = slot.replace(Owned { owner: WELCOME_LABEL.to_owned(), preview: 3 });
        assert_eq!(replaced.map(|p| p.owner), Some(MAIN_LABEL.to_owned()));
        assert!(take_owned(&mut slot, MAIN_LABEL).is_none());
        assert_eq!(take_owned(&mut slot, WELCOME_LABEL).map(|p| p.preview), Some(3));
    }
}
