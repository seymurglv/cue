//! Speech model probe: loads Parakeet from `%LOCALAPPDATA%\CUE\models\<model>` and recognizes bench recordings
//! (`%LOCALAPPDATA%\CUE\bench\recordings\*.wav`, every Nth of them), printing the load time and each recording's
//! recognition time. No text is printed. It waits 3 s at the end, so the memory it holds after the runs can be read
//! from outside.
//! Run: cargo run --release -p cue-engines --example stt_probe -- [count] [model]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use cue_engines::parakeet::model::Recognizer;

/// Windows may run a process without a visible window on the efficient cores (EcoQoS); `CUE_THROTTLE=1` leaves it so.
fn no_throttling() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    // SAFETY: the state lives through the call, and its size is the one passed.
    let ok = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    println!("throttling_off {}", ok.is_ok());
}

/// This process's private memory now and at its peak, in MB.
fn memory() -> (u64, u64) {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut m = PROCESS_MEMORY_COUNTERS_EX { cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    // SAFETY: `m` is a counters record of the size its `cb` says.
    let _ = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), std::ptr::from_mut(&mut m).cast(), m.cb) };
    ((m.PrivateUsage / (1 << 20)) as u64, (m.PeakPagefileUsage / (1 << 20)) as u64)
}

fn main() -> anyhow::Result<()> {
    if std::env::var("CUE_THROTTLE").as_deref() != Ok("1") {
        no_throttling();
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let count: usize = args.first().map_or(Ok(40), |a| a.parse()).context("count")?;
    let model = args.get(1).map_or("parakeet-ultra-int8", String::as_str);
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or_else(|| anyhow!("no LOCALAPPDATA"))?).join("CUE");

    let mut wavs: Vec<PathBuf> = std::fs::read_dir(local.join("bench").join("recordings"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "wav"))
        .collect();
    wavs.sort();
    let step = (wavs.len() / count.max(1)).max(1);
    let wavs: Vec<PathBuf> = wavs.into_iter().step_by(step).take(count).collect();

    let started = Instant::now();
    let model = Recognizer::load(&local.join("models").join(model)).map_err(|e| anyhow!("{e}"))?;
    println!("load_ms {} loaded_mb {}", started.elapsed().as_millis(), memory().0);

    let (mut audio, mut took, mut first) = (0u64, 0u64, None);
    for wav in &wavs {
        let mut reader = hound::WavReader::open(wav)?;
        let spec = reader.spec();
        if spec.sample_rate != 16_000 || spec.channels != 1 {
            continue;
        }
        let pcm: Vec<i16> = reader.samples::<i16>().collect::<Result<_, _>>()?;
        let r = model.recognize(&pcm).map_err(|e| anyhow!("{e}"))?;
        first.get_or_insert(r.took_ms);
        audio += r.audio_ms;
        took += r.took_ms;
    }
    println!(
        "recordings {} audio_s {:.1} took_ms {} first_ms {} rtf {:.4}",
        wavs.len(),
        audio as f64 / 1000.0,
        took,
        first.unwrap_or(0),
        took as f64 / audio.max(1) as f64
    );
    let (now, peak) = memory();
    println!("after_mb {now} peak_mb {peak}");
    println!("done");
    if std::env::var("CUE_WAIT").is_ok() {
        std::thread::sleep(Duration::from_secs(3));
    }
    Ok(())
}
