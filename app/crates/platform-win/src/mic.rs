//! The pinned microphone, or the default one → 20 ms frames, 16 kHz mono i16, with peak and level for the Drop.
//!
//! A pinned microphone is found by its name as Windows shows it (spec 3 §3), on the `cue-mic` thread: whoever opens
//! the microphone never waits for the devices to be listed. With nothing pinned the default microphone opens as it
//! always did, with no list at all.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cue_core::audio::{level, peak_dbfs, Resampler};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

pub const FRAME: usize = 320;

#[derive(Clone, PartialEq)]
pub enum MicEvent {
    Frame { pcm: Vec<i16>, peak_dbfs: f32, level: f32 },
    /// The pinned microphone is not there: this `open` records from the default one, named `used` (empty when its name
    /// could not be read). At most once per `open`, before its first frame.
    Fallback { used: String },
    /// At most once per `open`; after it this `open` gives no more audio.
    Failed(String),
}

/// Without the samples themselves — only their count — and without a device's name — only its length: `{ev:?}` in the
/// log must not carry audio or names there.
impl std::fmt::Debug for MicEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Frame { pcm, peak_dbfs, level } => f
                .debug_struct("Frame")
                .field("samples", &pcm.len())
                .field("peak_dbfs", peak_dbfs)
                .field("level", level)
                .finish(),
            Self::Fallback { used } => f.debug_struct("Fallback").field("used_chars", &used.chars().count()).finish(),
            Self::Failed(e) => f.debug_tuple("Failed").field(e).finish(),
        }
    }
}

/// Which of `names` (as Windows shows them) is the pinned `wanted`: the same name once both are trimmed, else the first
/// one that differs only in case. `None`: it is not there.
pub fn pick(names: &[String], wanted: &str) -> Option<usize> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return None;
    }
    names.iter().position(|n| n.trim() == wanted).or_else(|| {
        let wanted = wanted.to_lowercase();
        names.iter().position(|n| n.trim().to_lowercase() == wanted)
    })
}

/// Channel downmix, resampling and slicing into frames. Lives inside the cpal audio callback.
pub struct Pipe {
    ch: usize,
    rs: Resampler,
    buf: Vec<i16>,
    tx: Sender<MicEvent>,
}

impl Pipe {
    pub fn new(rate: u32, ch: usize, tx: Sender<MicEvent>) -> Self {
        Self { ch: ch.max(1), rs: Resampler::new(rate), buf: Vec::with_capacity(FRAME * 4), tx }
    }

    pub fn push_f32(&mut self, data: &[f32]) {
        let mono: Vec<f32> = data.chunks(self.ch).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
        let out = self.rs.process(&mono);
        self.buf.extend_from_slice(&out);
        while self.buf.len() >= FRAME {
            let pcm: Vec<i16> = self.buf.drain(..FRAME).collect();
            let _ = self.tx.send(MicEvent::Frame { peak_dbfs: peak_dbfs(&pcm), level: level(&pcm), pcm });
        }
    }

    pub fn push_i16(&mut self, data: &[i16]) {
        let f: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
        self.push_f32(&f);
    }
}

pub struct MicHandle {
    stop: Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

impl MicHandle {
    pub fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.thread.join();
    }
}

/// The cpal stream lives on its own `cue-mic` thread until `stop`; an open failure arrives as a `Failed` event.
/// `device` is the pinned microphone's name (`None` — the default one); when it is not there, the default one records
/// and a `Fallback` says so first.
/// A `MicHandle` dropped without `stop` closes the microphone too: `recv` gets an error, the stream closes, the thread ends.
pub fn open(device: Option<String>, events: Sender<MicEvent>) -> MicHandle {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let fail = Fail::new(events);
    let thread = std::thread::Builder::new()
        .name("cue-mic".into())
        .spawn(move || hold(&fail, &stop_rx, |fail| build(fail, device.as_deref())))
        .expect("microphone thread");
    MicHandle { stop: stop_tx, thread }
}

/// The names of the microphones Windows has now, as it shows them (a name it cannot read is left out). It lists the
/// devices: never on a thread that must not wait.
pub fn input_names() -> Vec<String> {
    devices(&cpal::default_host()).1.into_iter().map(|(_, name)| name).collect()
}

/// The name of the microphone Windows records from by default, if there is one.
pub fn default_name() -> Option<String> {
    cpal::default_host().default_input_device().as_ref().and_then(name_of)
}

/// The default input device, and every input device with the name Windows shows for it. The default one is asked for
/// first, as with nothing pinned: cpal sets up COM on this thread then, and the list needs it too.
fn devices(host: &cpal::Host) -> (Option<cpal::Device>, Vec<(cpal::Device, String)>) {
    let default = host.default_input_device();
    let Ok(all) = host.input_devices() else { return (default, Vec::new()) };
    (default, all.filter_map(|d| name_of(&d).map(|name| (d, name))).collect())
}

/// A device's name as Windows shows it. The driver's property store can fail, and cpal panics then: that device has no
/// name, the others still do.
fn name_of(device: &cpal::Device) -> Option<String> {
    catch_unwind(AssertUnwindSafe(|| device.description())).ok()?.ok().map(|d| d.name().to_owned())
}

/// The pinned microphone `wanted`; when it is not there, the default one, with a `Fallback` naming it before any frame.
fn pinned(host: &cpal::Host, wanted: &str, fail: &Fail) -> Result<cpal::Device, String> {
    let (default, mut found) = devices(host);
    let names: Vec<String> = found.iter().map(|(_, name)| name.clone()).collect();
    if let Some(i) = pick(&names, wanted) {
        return Ok(found.swap_remove(i).0);
    }
    let device = default.ok_or("no microphone")?;
    let used = name_of(&device).unwrap_or_default();
    let _ = fail.tx.send(MicEvent::Fallback { used });
    Ok(device)
}

/// Body of the `cue-mic` thread: open the stream and hold it until `stop`. An open error or panic — `Failed`, otherwise
/// the consumer would wait for frames forever; the panic text does not go into the event.
fn hold<S>(fail: &Fail, stop: &Receiver<()>, build: impl FnOnce(&Fail) -> Result<S, String>) {
    match catch_unwind(AssertUnwindSafe(|| build(fail))) {
        Ok(Ok(stream)) => {
            let _ = stop.recv();
            drop(stream);
        }
        Ok(Err(e)) => fail.send(e),
        Err(_) => fail.send("microphone failure".into()),
    }
}

/// Sender of `Failed` for one `open`: a flag shared by both cpal callbacks (errors and audio) and the `cue-mic` thread.
#[derive(Clone)]
struct Fail {
    tx: Sender<MicEvent>,
    sent: Arc<AtomicBool>,
}

impl Fail {
    fn new(tx: Sender<MicEvent>) -> Self {
        Self { tx, sent: Arc::new(AtomicBool::new(false)) }
    }

    fn send(&self, msg: String) {
        if !self.sent.swap(true, Ordering::Relaxed) {
            let _ = self.tx.send(MicEvent::Failed(msg));
        }
    }

    fn is_set(&self) -> bool {
        self.sent.load(Ordering::Relaxed)
    }

    fn on_stream_error(&self, e: cpal::Error) {
        if is_fatal(e.kind()) {
            self.send(e.to_string());
        } else {
            tracing::debug!(kind = ?e.kind(), "microphone: glitch without stopping the recording");
        }
    }
}

/// After a dropped packet, a move to another device and a refused priority cpal keeps recording — the dictation is not
/// cut off. Everything else, including future kinds (`ErrorKind` is non-exhaustive), ends the recording.
fn is_fatal(kind: cpal::ErrorKind) -> bool {
    !matches!(kind, cpal::ErrorKind::Xrun | cpal::ErrorKind::DeviceChanged | cpal::ErrorKind::RealtimeDenied)
}

/// cpal audio callback on top of `Pipe`. After `Failed` the audio is dropped: when the default microphone changes, WASAPI
/// reports an error but keeps delivering audio from the old one. cpal calls both callbacks from one thread, so the flag
/// is seen at once. A panic in processing is `Failed` too, otherwise the cpal thread would end silently and there would be
/// no frames until `stop`.
struct Feed {
    pipe: Pipe,
    fail: Fail,
}

impl Feed {
    fn push(&mut self, push: impl FnOnce(&mut Pipe)) {
        if self.fail.is_set() {
            return;
        }
        if catch_unwind(AssertUnwindSafe(|| push(&mut self.pipe))).is_err() {
            self.fail.send("microphone failure".into());
        }
    }
}

fn build(fail: &Fail, wanted: Option<&str>) -> Result<cpal::Stream, String> {
    let host = cpal::default_host();
    let device = match wanted {
        // Nothing pinned: the default microphone, as it always was — no device list.
        None => host.default_input_device().ok_or("no microphone")?,
        Some(wanted) => pinned(&host, wanted, fail)?,
    };
    let config = device.default_input_config().map_err(|e| e.to_string())?;
    let rate = config.sample_rate();
    // `Resampler::new` panics on zero and would bring the thread down silently; an error instead becomes "microphone is silent".
    if rate == 0 {
        return Err("the microphone reported a sample rate of 0".into());
    }
    let mut feed = Feed { pipe: Pipe::new(rate, config.channels() as usize, fail.tx.clone()), fail: fail.clone() };
    let on_err = {
        let fail = fail.clone();
        move |e: cpal::Error| fail.on_stream_error(e)
    };
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config.into(),
            move |d: &[f32], _: &cpal::InputCallbackInfo| feed.push(|p| p.push_f32(d)),
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config.into(),
            move |d: &[i16], _: &cpal::InputCallbackInfo| feed.push(|p| p.push_i16(d)),
            on_err,
            None,
        ),
        f => return Err(format!("microphone format {f:?} is not supported")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_48k_becomes_20ms_frames() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut p = Pipe::new(48_000, 2, tx);
        p.push_f32(&vec![0.25f32; 48_000 * 2]); // 1 s of stereo
        let frames: Vec<MicEvent> = rx.try_iter().collect();
        assert_eq!(frames.len(), 50, "16,000 samples / 320");
        let MicEvent::Frame { pcm, peak_dbfs, level } = &frames[49] else { panic!() };
        assert_eq!(pcm.len(), FRAME);
        assert!((pcm[100] as i32 - 8_191).abs() <= 2);
        assert!(*peak_dbfs > -13.0 && *level > 0.0);
    }

    #[test]
    fn i16_input_is_accepted() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut p = Pipe::new(16_000, 1, tx);
        p.push_i16(&[1_000; 640]);
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn channels_are_averaged_not_picked() {
        // Left channel 0.5, right silence → 0.25 in every sample of the frame (0.25 · 32,767 with the fraction dropped).
        let (tx, rx) = std::sync::mpsc::channel();
        let mut p = Pipe::new(16_000, 2, tx);
        p.push_f32(&[0.5, 0.0].repeat(FRAME));
        let frames: Vec<MicEvent> = rx.try_iter().collect();
        assert_eq!(frames.len(), 1);
        let MicEvent::Frame { pcm, .. } = &frames[0] else { panic!() };
        assert!(pcm.iter().all(|&s| s == 8_191));
    }

    #[test]
    fn remainder_waits_for_next_callback() {
        // cpal delivers audio in chunks of any length: 10 ms of 48 kHz stereo is half a frame, the rest waits for the next call.
        let (tx, rx) = std::sync::mpsc::channel();
        let mut p = Pipe::new(48_000, 2, tx);
        p.push_f32(&[0.25; 960]);
        assert_eq!(rx.try_iter().count(), 0);
        p.push_f32(&[0.25; 960]);
        assert_eq!(rx.try_iter().count(), 1);
    }

    #[test]
    fn only_glitches_keep_recording() {
        use cpal::ErrorKind as K;
        for k in [K::Xrun, K::DeviceChanged, K::RealtimeDenied] {
            assert!(!is_fatal(k), "{k:?}");
        }
        for k in [
            K::DeviceBusy,
            K::DeviceNotAvailable,
            K::HostUnavailable,
            K::InvalidInput,
            K::PermissionDenied,
            K::ResourceExhausted,
            K::StreamInvalidated,
            K::UnsupportedConfig,
            K::UnsupportedOperation,
            K::BackendError,
            K::Other,
        ] {
            assert!(is_fatal(k), "{k:?}");
        }
    }

    #[test]
    fn failed_is_sent_once_and_glitches_are_silent() {
        let (tx, rx) = std::sync::mpsc::channel();
        let fail = Fail::new(tx);
        fail.on_stream_error(cpal::ErrorKind::Xrun.into());
        fail.on_stream_error(cpal::ErrorKind::DeviceChanged.into());
        assert_eq!(rx.try_iter().count(), 0, "a glitch without stopping the recording does not cut the dictation off");
        // WASAPI reports a lost device twice; one more error may come from the open.
        fail.on_stream_error(cpal::Error::with_message(cpal::ErrorKind::DeviceNotAvailable, "gone"));
        fail.clone().on_stream_error(cpal::ErrorKind::DeviceNotAvailable.into());
        fail.send("more".into());
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![MicEvent::Failed("gone".into())]);
    }

    #[test]
    fn panic_while_opening_becomes_failed() {
        let (tx, rx) = std::sync::mpsc::channel();
        let (_stop, stop_rx) = std::sync::mpsc::channel();
        hold::<()>(&Fail::new(tx), &stop_rx, |_| panic!("samples 1 2 3"));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![MicEvent::Failed("microphone failure".into())]);
    }

    #[test]
    fn no_frames_after_failed() {
        // When the default microphone changes, WASAPI sends `StreamInvalidated` but keeps delivering audio from the old one.
        let (tx, rx) = std::sync::mpsc::channel();
        let fail = Fail::new(tx.clone());
        let mut feed = Feed { pipe: Pipe::new(16_000, 1, tx), fail: fail.clone() };
        feed.push(|p| p.push_i16(&[0; FRAME]));
        fail.on_stream_error(cpal::Error::with_message(cpal::ErrorKind::StreamInvalidated, "changed"));
        feed.push(|p| p.push_i16(&[0; FRAME]));
        let events: Vec<MicEvent> = rx.try_iter().collect();
        assert_eq!(events.len(), 2, "{events:?}");
        assert!(matches!(events[0], MicEvent::Frame { .. }));
        assert_eq!(events[1], MicEvent::Failed("changed".into()));
    }

    #[test]
    fn panic_in_audio_callback_becomes_failed() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut feed = Feed { pipe: Pipe::new(16_000, 1, tx.clone()), fail: Fail::new(tx) };
        feed.push(|_| panic!("samples 1 2 3"));
        feed.push(|p| p.push_i16(&[0; FRAME]));
        feed.push(|_| panic!("more"));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), vec![MicEvent::Failed("microphone failure".into())]);
    }

    #[test]
    fn debug_shows_sample_count_not_samples() {
        let ev = MicEvent::Frame { pcm: vec![12_345; FRAME], peak_dbfs: -1.5, level: 0.5 };
        for s in [format!("{ev:?}"), format!("{ev:#?}")] {
            assert!(s.contains("samples") && s.contains("320"), "{s}");
            assert!(!s.contains("12345"), "{s}");
        }
        let ev = MicEvent::Fallback { used: "Headset of Jane".into() };
        for s in [format!("{ev:?}"), format!("{ev:#?}")] {
            assert!(s.contains("15") && !s.contains("Jane"), "the name's length, never the name: {s}");
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|&n| n.to_owned()).collect()
    }

    #[test]
    fn pick_prefers_exact_then_case_insensitive() {
        let list = names(&["Microphone (Realtek Audio)", "Headset (JABRA)", "headset (Jabra)", "Микрофон (USB Audio)"]);
        assert_eq!(pick(&list, "headset (Jabra)"), Some(2), "the same name wins over an earlier one in another case");
        assert_eq!(pick(&list, "  Headset (JABRA)\t"), Some(1), "trimmed");
        assert_eq!(pick(&list, "HEADSET (jabra)"), Some(1), "only the case differs: the first such");
        assert_eq!(pick(&list, "микрофон (usb audio)"), Some(3), "case in any script");
        assert_eq!(pick(&names(&[" Microphone (Realtek Audio) "]), "Microphone (Realtek Audio)"), Some(0));
    }

    #[test]
    fn pick_is_none_when_absent() {
        let list = names(&["Microphone (Realtek Audio)", "Headset (JABRA)"]);
        assert_eq!(pick(&list, "Blue Yeti"), None);
        assert_eq!(pick(&list, "Headset"), None, "a part of a name is not the name");
        assert_eq!(pick(&list, "Headset (JABRA) 2"), None);
        assert_eq!(pick(&[], "Headset (JABRA)"), None, "no microphones at all");
        assert_eq!(pick(&names(&["", "  "]), " "), None, "a blank name is never a device");
    }
}
