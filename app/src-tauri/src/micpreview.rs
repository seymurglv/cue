//! The microphone's live level in the main window's settings (spec 3 §3). Only the level goes to the page, at most ~30
//! times a second; the audio itself is never kept and never leaves this module. One preview at a time: a new one
//! replaces the old. It ends by the page's command (`commands::mic_preview`) and when the main window is destroyed
//! (`main_window`). It opens the microphone in the shared mode, as a recording does, so a dictation meanwhile records
//! as usual.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use cue_platform_win::mic::{self, MicEvent, MicHandle};
use tauri::{AppHandle, Emitter};

use crate::ipc::MAIN_LABEL;

/// At most ~30 levels a second, as the Drop gets.
const LEVEL_EVERY: Duration = Duration::from_millis(33);

/// The preview running now.
static PREVIEW: Mutex<Option<Preview>> = Mutex::new(None);

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
/// to the main window as `cue://mic-level`. Returns at once: the microphone opens on its own thread.
pub fn start(app: &AppHandle, device: Option<String>) {
    stop();
    let live = Arc::new(AtomicBool::new(true));
    let (events, rx) = std::sync::mpsc::channel();
    let (app, on) = (app.clone(), Arc::clone(&live));
    let forward = std::thread::Builder::new().name("cue-mic-preview".into()).spawn(move || forward(&app, &rx, &on));
    if let Err(e) = forward {
        tracing::warn!("microphone preview not started: {e}");
        return;
    }
    tracing::info!(pinned = device.is_some(), "microphone preview started");
    let mic = mic::open(device, events);
    // Another start in between (the page asks one at a time, so only in a race): the last one stays.
    if let Some(other) = slot().replace(Preview { mic, live }) {
        other.end();
    }
}

/// The preview ends, if one runs.
pub fn stop() {
    let ended = slot().take();
    if let Some(preview) = ended {
        preview.end();
        tracing::info!("microphone preview stopped");
    }
}

fn slot() -> MutexGuard<'static, Option<Preview>> {
    PREVIEW.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Frames → their level, to the main window; the samples go with the event. Ends when the preview has ended or the
/// microphone's thread has.
fn forward(app: &AppHandle, events: &Receiver<MicEvent>, live: &AtomicBool) {
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
                send(app, level);
            }
            // The page shows a pinned microphone that is not there itself; meanwhile the level is the Windows one's.
            MicEvent::Fallback { .. } => {}
            MicEvent::Failed(e) => {
                tracing::warn!("microphone preview: {e}");
                send(app, 0.0);
            }
        }
    }
}

fn send(app: &AppHandle, level: f32) {
    if let Err(e) = app.emit_to(MAIN_LABEL, "cue://mic-level", level) {
        tracing::debug!("main window: event cue://mic-level not sent: {e}");
    }
}
