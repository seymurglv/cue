//! Microphone probe: N seconds (3 by default) from the default microphone, or the one named with `--device`, through
//! `mic::open`, then `stop`.
//! Run: cargo run -p cue-platform-win --example mic_probe -- [seconds] [--wav <path>] [--device <name>]
//! Prints the microphone config, the frame count, frames per second (≈ 50), peak, mean level and failures; never the audio.
use cpal::traits::{DeviceTrait, HostTrait};
use cue_platform_win::mic::{self, MicEvent};
use cue_platform_win::wav::WavSink;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Stats {
    frames: usize,
    max_peak: Option<f32>,
    level_sum: f32,
    first: Option<Instant>,
}

fn main() {
    let mut secs = 3u64;
    let mut wav: Option<PathBuf> = None;
    let mut device: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--wav" {
            wav = Some(args.next().unwrap_or_else(|| fail("--wav needs a path")).into());
        } else if a == "--device" {
            device = Some(args.next().unwrap_or_else(|| fail("--device needs a name")));
        } else {
            secs = a
                .parse()
                .ok()
                .filter(|&s| s > 0)
                .unwrap_or_else(|| fail(&format!("need a whole number of seconds greater than zero: {a}")));
        }
    }

    match cpal::default_host().default_input_device() {
        Some(d) => {
            let name = d.description().map(|x| x.name().to_owned()).unwrap_or_else(|e| e.to_string());
            match d.default_input_config() {
                Ok(c) => println!(
                    "Microphone: {name}, {} Hz, {} channels, {}",
                    c.sample_rate(),
                    c.channels(),
                    c.sample_format()
                ),
                Err(e) => println!("Microphone: {name}, config unavailable: {e}"),
            }
        }
        None => println!("No default microphone"),
    }
    let mut sink = wav.as_deref().map(|p| WavSink::create(p).unwrap_or_else(|e| fail(&e)));

    let (tx, rx) = std::sync::mpsc::channel();
    let deadline = Instant::now() + Duration::from_secs(secs);
    let handle = mic::open(device, tx);
    println!("Listening for {secs} s…");
    let mut st = Stats::default();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ev) => take(&mut st, sink.as_mut(), ev),
            // Time is up or the microphone thread has already ended (after `Failed`).
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
        }
    }
    handle.stop();
    let stopped = Instant::now();
    for ev in rx.try_iter() {
        take(&mut st, sink.as_mut(), ev);
    }

    println!("Frames: {}", st.frames);
    if let (Some(first), Some(peak)) = (st.first, st.max_peak) {
        let span = stopped.duration_since(first).as_secs_f64();
        println!("Frames per second: {:.1}", st.frames as f64 / span);
        println!("Peak: {peak:.1} dBFS, mean level: {:.3}", st.level_sum / st.frames as f32);
    }

    if let (Some(sink), Some(path)) = (sink, wav) {
        sink.finish().unwrap_or_else(|e| fail(&format!("WAV: {e}")));
        let r = hound::WavReader::open(&path).unwrap_or_else(|e| fail(&format!("could not read the WAV: {e}")));
        let rate = r.spec().sample_rate;
        println!("WAV: {} samples, {:.2} s", r.len(), r.duration() as f64 / rate as f64);
    }
}

fn take(st: &mut Stats, sink: Option<&mut WavSink>, ev: MicEvent) {
    match ev {
        MicEvent::Frame { pcm, peak_dbfs, level } => {
            st.first.get_or_insert_with(Instant::now);
            st.frames += 1;
            st.max_peak = Some(st.max_peak.map_or(peak_dbfs, |p| p.max(peak_dbfs)));
            st.level_sum += level;
            if let Some(s) = sink {
                s.write(&pcm);
            }
        }
        MicEvent::Fallback { used } => println!("That microphone is not there, recording from: {used}"),
        MicEvent::Failed(e) => println!("Failure: {e}"),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}
