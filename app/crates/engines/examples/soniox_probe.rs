//! Soniox probe: 6 seconds of speech from the microphone → Soniox → finalize. Prints the text and the latency after finalize.
//! Run: cargo run -p cue-engines --example soniox_probe
//! The key comes from Credential Manager (`cue-cli keys set soniox`); without a key the microphone is not opened.
use anyhow::{anyhow, bail, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use cue_engines::keys::{self, KeyName};
use cue_engines::soniox::{run_rt, SonioxConfig};
use cue_engines::stt::AudioMsg;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // The key first: without it the microphone is left alone.
    let key = keys::get(KeyName::Soniox)
        .map_err(|e| anyhow!("{e}\nSave the key: cargo run -p cue-cli -- keys set soniox"))?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SonioxConfig {
        model: "stt-rt-v5".into(),
        language_hints: vec!["ru".into(), "en".into()],
        app: Some("Probe".into()),
        terms: vec![],
    };
    let session = tokio::spawn(run_rt(key, cfg, rx));

    let device = cpal::default_host().default_input_device().context("no microphone")?;
    let supported = device.default_input_config()?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let name = device.description().map(|d| d.name().to_owned()).unwrap_or_else(|e| e.to_string());
    println!("Microphone: {name}, {} Hz, {} channels, {format}", config.sample_rate, config.channels);
    let stream = match format {
        cpal::SampleFormat::F32 => capture::<f32>(&device, config, tx.clone())?,
        cpal::SampleFormat::I16 => capture::<i16>(&device, config, tx.clone())?,
        other => bail!("the probe does not support microphone format {other} (needs f32 or i16)"),
    };
    stream.play()?;
    println!("Speak for 6 seconds…");
    tokio::time::sleep(Duration::from_secs(6)).await;
    drop(stream);
    let t0 = Instant::now();
    tx.send(AudioMsg::Finish)?;
    let out = session.await??;
    println!("Text: {}", out.text);
    println!("Audio: {} ms; finalize to text: {} ms", out.audio_ms, t0.elapsed().as_millis());
    Ok(())
}

/// Crude conversion to 16 kHz mono (nearest sample) — for the probe only; the app uses a proper resampler.
/// Frames go out as they are (one per microphone callback): `run_rt` gathers them into ~100 ms.
fn capture<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    tx: UnboundedSender<AudioMsg>,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let ch = config.channels as usize;
    let step = config.sample_rate as f64 / 16_000.0;
    let mut pos = 0.0f64;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let frames = data.len() / ch;
            let mut pcm = Vec::with_capacity((frames as f64 / step) as usize + 1);
            while (pos as usize) < frames {
                let i = pos as usize * ch;
                let m = data[i..i + ch].iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / ch as f32;
                pcm.push((m.clamp(-1.0, 1.0) * 32767.0) as i16);
                pos += step;
            }
            pos -= frames as f64;
            if !pcm.is_empty() {
                let _ = tx.send(AudioMsg::Pcm(pcm));
            }
        },
        |e| eprintln!("microphone: {e}"),
        None,
    )?;
    Ok(stream)
}
