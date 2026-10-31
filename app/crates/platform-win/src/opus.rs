//! The recordings of the history as Ogg Opus: the WAV CUE records (16 kHz mono 16-bit, 32 KB a second) encoded for
//! speech at 16 kbit/s, about 2 KB a second — clear, like a good call. The encoder is libopus 1.3.1 translated to Rust
//! (`unsafe-libopus`: no C compiler, nothing to link), the Ogg pages are written here (`ogg`).
//!
//! The sound and the paths are the user's: nothing of them goes to the log or into an error, only codes.
use crate::ogg::OggOpus;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufWriter, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use unsafe_libopus::varargs::{VarArg, VarArgs};
use unsafe_libopus::{
    OPUS_ALLOC_FAIL, OPUS_APPLICATION_VOIP, OPUS_GET_LOOKAHEAD_REQUEST, OPUS_INTERNAL_ERROR, OPUS_OK,
    OPUS_SET_BITRATE_REQUEST, OPUS_SET_COMPLEXITY_REQUEST, OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE, OpusEncoder,
    opus_encode, opus_encoder_create, opus_encoder_ctl_impl, opus_encoder_destroy,
};

/// The sound CUE records (`wav::WavSink`): 16 kHz, mono, 16-bit integers.
const RATE: u32 = 16_000;
/// 20 ms frames: 320 samples at 16 kHz, 960 at the 48 kHz that Ogg Opus counts in.
const FRAME: usize = 320;
const FRAME_48K: u64 = 960;
/// The 48 kHz samples of one 16 kHz sample.
const TO_48K: u64 = 3;
const BITRATE: i32 = 16_000;
/// The slowest setting, the best sound: a recording is encoded in the background, far faster than it was spoken.
const COMPLEXITY: i32 = 10;
/// Room for the largest packet the encoder may make (the size libopus recommends).
const MAX_PACKET: usize = 4000;

/// Why a recording was not encoded. No paths and nothing of the sound: only what went wrong.
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    /// The WAV is missing, is not a WAV, or is cut off.
    #[error("the recording could not be read: {0}")]
    Read(#[from] hound::Error),
    /// The WAV is not what CUE records.
    #[error("the recording is not 16 kHz mono 16-bit PCM ({rate} Hz, {channels} channels, {bits}-bit {kind})")]
    Format { rate: u32, channels: u16, bits: u16, kind: &'static str },
    /// libopus refused (its error code); a panic while encoding counts as its internal error, -3.
    #[error("the Opus encoder failed (code {0})")]
    Codec(i32),
    /// The caller no longer wanted the recording when it was ready (`encode_wav_to_ogg_opus_if`).
    #[error("the recording was no longer wanted")]
    Dropped,
    /// The Ogg file could not be written or could not take its name.
    #[error("the Opus recording could not be written: {0}")]
    Write(#[from] io::Error),
}

/// What was encoded: the length of the sound and the size of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub duration_ms: u64,
    pub bytes: u64,
}

/// Encodes the WAV `wav` (16 kHz mono 16-bit, as CUE records it; anything else is an error) into the Ogg Opus file
/// `out`: 20 ms frames, 16 kbit/s, tuned for speech. The file is written next to `out` under `out`'s name with `.tmp`
/// added, then renamed to `out` (replacing it), so `out` is never half written; on any failure that temporary file
/// is deleted and `out` is left as it was. The folder of `out` must exist: it is not created, so an encoding that ends
/// after its folder was deleted brings nothing back.
pub fn encode_wav_to_ogg_opus(wav: &Path, out: &Path) -> Result<Encoded, EncodeError> {
    encode_wav_to_ogg_opus_if(wav, out, || true)
}

/// `encode_wav_to_ogg_opus`, asking `still_wanted` once the recording is written, right before it takes its name
/// `out`: false — the history was cleared meanwhile, say — deletes it and gives `EncodeError::Dropped`.
pub fn encode_wav_to_ogg_opus_if(
    wav: &Path,
    out: &Path,
    still_wanted: impl FnOnce() -> bool,
) -> Result<Encoded, EncodeError> {
    let tmp = temp_path(out)?;
    let reader = hound::WavReader::open(wav)?;
    let spec = reader.spec();
    if (spec.sample_rate, spec.channels, spec.bits_per_sample, spec.sample_format)
        != (RATE, 1, 16, hound::SampleFormat::Int)
    {
        let kind = if spec.sample_format == hound::SampleFormat::Float { "float" } else { "integer" };
        return Err(EncodeError::Format {
            rate: spec.sample_rate,
            channels: spec.channels,
            bits: spec.bits_per_sample,
            kind,
        });
    }
    let mut file = BufWriter::new(File::create(&tmp)?);
    let mut temp = Temp { path: tmp, renamed: false };
    let samples = catch_unwind(AssertUnwindSafe(|| encode(reader, &mut file)))
        .unwrap_or(Err(EncodeError::Codec(OPUS_INTERNAL_ERROR)))?;
    let file = file.into_inner().map_err(io::IntoInnerError::into_error)?;
    // The data is on disk before the rename: a power failure never leaves an empty recording under its name.
    file.sync_all()?;
    let bytes = file.metadata()?.len();
    drop(file);
    if !still_wanted() {
        return Err(EncodeError::Dropped);
    }
    std::fs::rename(&temp.path, out)?;
    temp.renamed = true;
    Ok(Encoded { duration_ms: samples * 1000 / u64::from(RATE), bytes })
}

/// Encodes every sample of `reader` into an Ogg Opus stream on `out`; returns how many samples the sound had. The
/// encoder gives each sample back `lookahead` samples late, so the stream starts with that many to drop (the
/// pre-skip) and the sound is followed by silence until its last sample is out; the last page's position cuts the
/// stream at the sound's end.
fn encode<R: io::Read, W: Write>(reader: hound::WavReader<R>, out: W) -> Result<u64, EncodeError> {
    let mut encoder = Encoder::new()?;
    let lookahead = encoder.lookahead()?;
    let pre_skip = u16::try_from(lookahead * TO_48K).map_err(|_| EncodeError::Codec(OPUS_INTERNAL_ERROR))?;
    let mut ogg = OggOpus::start(out, serial(), pre_skip, RATE)?;
    let mut packet = vec![0u8; MAX_PACKET];
    let mut frame = [0i16; FRAME];
    let (mut filled, mut samples, mut frames) = (0usize, 0u64, 0u64);
    let mut put = |frame: &[i16; FRAME], frames: &mut u64| -> Result<(), EncodeError> {
        let len = encoder.encode(frame, &mut packet)?;
        ogg.packet(&packet[..len], FRAME_48K)?;
        *frames += 1;
        Ok(())
    };
    for sample in reader.into_samples::<i16>() {
        frame[filled] = sample?;
        filled += 1;
        samples += 1;
        if filled == FRAME {
            put(&frame, &mut frames)?;
            filled = 0;
        }
    }
    if filled > 0 {
        frame[filled..].fill(0);
        put(&frame, &mut frames)?;
    }
    frame.fill(0);
    while frames * (FRAME as u64) < samples + lookahead {
        put(&frame, &mut frames)?;
    }
    ogg.finish(u64::from(pre_skip) + samples * TO_48K)?.flush()?;
    Ok(samples)
}

/// One libopus encoder for 16 kHz mono speech at 16 kbit/s, freed when dropped.
struct Encoder(*mut OpusEncoder);

impl Encoder {
    fn new() -> Result<Self, EncodeError> {
        let mut error = 0;
        // SAFETY: valid arguments; `error` outlives the call; a null encoder is not used.
        let raw = unsafe { opus_encoder_create(RATE as i32, 1, OPUS_APPLICATION_VOIP, &mut error) };
        if raw.is_null() {
            return Err(EncodeError::Codec(if error == OPUS_OK { OPUS_ALLOC_FAIL } else { error }));
        }
        let encoder = Self(raw);
        if error != OPUS_OK {
            return Err(EncodeError::Codec(error));
        }
        encoder.set(OPUS_SET_BITRATE_REQUEST, BITRATE)?;
        encoder.set(OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE)?;
        encoder.set(OPUS_SET_COMPLEXITY_REQUEST, COMPLEXITY)?;
        Ok(encoder)
    }

    fn set(&self, request: i32, value: i32) -> Result<(), EncodeError> {
        // SAFETY: a live encoder; a "set" request reads exactly one i32.
        let code = unsafe { opus_encoder_ctl_impl(self.0, request, VarArgs::new(vec![VarArg::I32(value)])) };
        if code == OPUS_OK { Ok(()) } else { Err(EncodeError::Codec(code)) }
    }

    /// The delay of the encoder, in samples at 16 kHz.
    fn lookahead(&self) -> Result<u64, EncodeError> {
        let mut value = 0i32;
        let args = VarArgs::new(vec![VarArg::I32Out(&mut value)]);
        // SAFETY: a live encoder; this "get" request writes exactly one i32, through `args`.
        let code = unsafe { opus_encoder_ctl_impl(self.0, OPUS_GET_LOOKAHEAD_REQUEST, args) };
        if code != OPUS_OK {
            return Err(EncodeError::Codec(code));
        }
        u64::try_from(value).map_err(|_| EncodeError::Codec(OPUS_INTERNAL_ERROR))
    }

    /// One 20 ms frame into `packet`; returns the packet's length.
    fn encode(&mut self, frame: &[i16; FRAME], packet: &mut [u8]) -> Result<usize, EncodeError> {
        let room = i32::try_from(packet.len()).unwrap_or(i32::MAX);
        // SAFETY: a live encoder; `frame` holds the FRAME samples it reads, `packet` the `room` bytes it may write.
        let len = unsafe { opus_encode(self.0, frame.as_ptr(), FRAME as i32, packet.as_mut_ptr(), room) };
        usize::try_from(len).map_err(|_| EncodeError::Codec(len))
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: made by `opus_encoder_create`, freed once.
        unsafe { opus_encoder_destroy(self.0) }
    }
}

/// The file `out` is written as before it takes its name: next to it (a rename across volumes is not atomic), its
/// name with `.tmp` added — the history deletes such files left over by a crash.
fn temp_path(out: &Path) -> io::Result<PathBuf> {
    let Some(name) = out.file_name() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the recording's path has no file name"));
    };
    let mut tmp = name.to_os_string();
    tmp.push(".tmp");
    Ok(out.with_file_name(tmp))
}

/// A temporary file deleted when dropped — on an error, or a panic — unless it took its name (`renamed`).
struct Temp {
    path: PathBuf,
    renamed: bool,
}

impl Drop for Temp {
    fn drop(&mut self) {
        if !self.renamed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The stream's serial number: any number, different from one recording to the next, so recordings can be chained.
fn serial() -> u32 {
    std::hash::RandomState::new().build_hasher().finish() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ogg::read::{Page, pages};
    use unsafe_libopus::{opus_decode, opus_decoder_create, opus_decoder_destroy};

    /// A folder of the test's own, deleted with all in it even when the check failed.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("cue-opus-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        /// The names of the files in the folder, sorted.
        fn names(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_wav(path: &Path, spec: hound::WavSpec, samples: &[i16]) {
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for &s in samples {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    const CUE_WAV: hound::WavSpec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    /// A made-up tone: `seconds` of a sine at `hz`, half of full scale, plus `extra` samples.
    fn sine(hz: f64, seconds: usize, extra: usize) -> Vec<i16> {
        (0..seconds * 16_000 + extra)
            .map(|i| (16_384.0 * (std::f64::consts::TAU * hz * i as f64 / 16_000.0).sin()) as i16)
            .collect()
    }

    /// A made-up signal shaped like speech: a voice whose pitch glides between 100 and 220 Hz, with harmonics under
    /// moving formants, in syllables of about a quarter of a second with short pauses between words, and hiss for
    /// the consonants. Nobody's voice.
    fn speech_like(seconds: usize) -> Vec<i16> {
        let mut noise = 0x2545_F491_4F6C_DD1Du64;
        let mut hiss = move || {
            noise ^= noise << 13;
            noise ^= noise >> 7;
            noise ^= noise << 17;
            (noise >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        let tau = std::f64::consts::TAU;
        let mut phase = 0.0f64;
        (0..seconds * 16_000)
            .map(|i| {
                let t = i as f64 / 16_000.0;
                let pitch = 160.0 + 60.0 * (tau * 0.3 * t).sin();
                phase += tau * pitch / 16_000.0;
                let word = t % 0.8;
                if word > 0.65 {
                    return 0; // the pause between words
                }
                let syllable = (std::f64::consts::PI * word / 0.65 * 3.0).sin().powi(2);
                let (f1, f2) = (500.0 + 300.0 * (tau * 1.1 * t).sin(), 1500.0 + 700.0 * (tau * 0.7 * t).cos());
                let voice: f64 = (1..=20)
                    .map(|k| {
                        let f = pitch * f64::from(k);
                        let near = |formant: f64, width: f64| 1.0 / (1.0 + ((f - formant) / width).powi(2));
                        let formants = near(f1, 150.0) + 0.6 * near(f2, 200.0);
                        (formants + 0.05) / f64::from(k) * (phase * f64::from(k)).sin()
                    })
                    .sum();
                let consonant = if word < 0.06 { 0.3 * hiss() } else { 0.02 * hiss() };
                (9_000.0 * (syllable * voice + consonant)).clamp(-32_768.0, 32_767.0) as i16
            })
            .collect()
    }

    /// A stream read back, its pages checked: CRCs, order, flags, the two headers, the positions.
    struct Stream {
        pre_skip: u64,
        /// The granule position of the last page.
        end: u64,
        /// The audio packets, in order.
        packets: Vec<Vec<u8>>,
    }

    fn read_stream(bytes: &[u8]) -> Stream {
        assert_eq!(&bytes[..4], b"OggS");
        let pages: Vec<Page> = pages(bytes);
        assert!(pages.len() >= 3, "two header pages and some sound");
        let serial = pages[0].serial;
        for (i, page) in pages.iter().enumerate() {
            assert_eq!(page.serial, serial, "one stream");
            assert_eq!(page.sequence, i as u32, "pages in order");
            assert_eq!(page.flags & 0x02 != 0, i == 0, "only the first page begins the stream");
            assert_eq!(page.flags & 0x04 != 0, i == pages.len() - 1, "only the last page ends it");
            assert_eq!(page.flags & 0x01, 0, "no packet goes on from a page before");
        }
        let head = &pages[0].packets;
        assert_eq!(head.len(), 1);
        assert_eq!(&head[0][..8], b"OpusHead");
        assert_eq!((head[0][8], head[0][9]), (1, 1), "version 1, mono");
        assert_eq!(u32::from_le_bytes(head[0][12..16].try_into().unwrap()), 16_000, "the rate recorded");
        let pre_skip = u64::from(u16::from_le_bytes([head[0][10], head[0][11]]));
        assert_eq!(pages[1].packets, vec![b"OpusTags\x03\0\0\0CUE\0\0\0\0".to_vec()], "vendor CUE, no comments");
        assert_eq!((pages[0].granule, pages[1].granule), (0, 0));
        let audio = &pages[2..];
        assert!(audio.windows(2).all(|w| w[0].granule <= w[1].granule), "positions never go back");
        let mut done = 0;
        for page in &audio[..audio.len() - 1] {
            done += page.packets.len() as u64 * FRAME_48K;
            assert_eq!(page.granule, done, "a page's position counts the sound of its packets");
        }
        let packets: Vec<Vec<u8>> = audio.iter().flat_map(|p| p.packets.iter().cloned()).collect();
        Stream { pre_skip, end: audio.last().unwrap().granule, packets }
    }

    /// The packets decoded by libopus at `rate`, one after another.
    fn decode(packets: &[Vec<u8>], rate: i32) -> Vec<i16> {
        let mut error = 0;
        // SAFETY: valid arguments; the decoder is checked, used here only and freed once.
        let decoder = unsafe { opus_decoder_create(rate, 1, &mut error) };
        assert!(!decoder.is_null() && error == OPUS_OK);
        let mut pcm = Vec::new();
        let mut buf = vec![0i16; 5760];
        for p in packets {
            // SAFETY: `p` holds `p.len()` bytes, `buf` room for 5760 samples, the most a packet decodes to.
            let n = unsafe { opus_decode(decoder, p.as_ptr(), p.len() as i32, buf.as_mut_ptr(), 5760, 0) };
            assert_eq!(n, rate / 50, "every packet is one 20 ms frame");
            pcm.extend_from_slice(&buf[..n as usize]);
        }
        // SAFETY: made above, freed once.
        unsafe { opus_decoder_destroy(decoder) };
        pcm
    }

    /// Encodes `samples` from a WAV in `dir` and checks the stream, its length, and that no temporary file is left;
    /// gives the stream back with the size of the file.
    fn round_trip(dir: &Dir, samples: &[i16]) -> (Stream, u64) {
        let (wav, out) = (dir.join("in.wav"), dir.join("2026-10-31T12-00-00.000-1.ogg"));
        write_wav(&wav, CUE_WAV, samples);
        let encoded = encode_wav_to_ogg_opus(&wav, &out).unwrap();
        assert_eq!(dir.names(), ["2026-10-31T12-00-00.000-1.ogg", "in.wav"], "no temporary file is left");
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(encoded, Encoded { duration_ms: samples.len() as u64 * 1000 / 16_000, bytes: bytes.len() as u64 });
        let stream = read_stream(&bytes);
        let n = samples.len() as u64;
        assert_eq!(stream.pre_skip, 312, "libopus 1.3.1 at 16 kHz is 104 samples late, 312 at 48 kHz");
        assert_eq!(stream.end, stream.pre_skip + 3 * n, "the last position: pre-skip and the sound, at 48 kHz");
        let decoded = decode(&stream.packets, 48_000).len() as u64;
        assert!(decoded >= stream.end, "the packets hold all of the sound");
        let length = decoded - stream.pre_skip;
        assert!(length.abs_diff(3 * n) < FRAME_48K, "decoded {length} samples for {} at 48 kHz", 3 * n);
        (stream, bytes.len() as u64)
    }

    #[test]
    fn silence_encodes() {
        let dir = Dir::new("silence");
        let (stream, bytes) = round_trip(&dir, &vec![0; 3 * 16_000 + 123]);
        assert!(bytes < 3 * 1_000, "{bytes} bytes for 3 s of silence");
        let pcm = decode(&stream.packets, 16_000);
        assert!(pcm.iter().all(|s| s.abs() < 64), "silence comes back silent");
    }

    /// A tone comes back as itself: as loud, and in step with the WAV once the pre-skip is dropped.
    #[test]
    fn a_sine_comes_back_in_step() {
        let dir = Dir::new("sine");
        let tone = sine(440.0, 2, 77);
        let (stream, _) = round_trip(&dir, &tone);
        let pcm = decode(&stream.packets, 16_000);
        let back = &pcm[(stream.pre_skip / 3) as usize..][..tone.len()];
        // Away from the edges, where the encoder starts and stops.
        let (a, b) = (&tone[3_200..28_800], &back[3_200..28_800]);
        let dot = |x: &[i16], y: &[i16]| x.iter().zip(y).map(|(&p, &q)| f64::from(p) * f64::from(q)).sum::<f64>();
        let (aa, bb, ab) = (dot(a, a), dot(b, b), dot(a, b));
        let loudness = (bb / aa).sqrt();
        assert!((0.8..1.25).contains(&loudness), "loudness {loudness}");
        let correlation = ab / (aa * bb).sqrt();
        assert!(correlation > 0.9, "correlation {correlation}");
    }

    /// Speech at 16 kbit/s: about 2 KB a second, sixteen times smaller than the WAV.
    #[test]
    fn speech_is_about_two_kilobytes_a_second() {
        let dir = Dir::new("speech");
        let (_, bytes) = round_trip(&dir, &speech_like(10));
        let per_second = bytes / 10;
        assert!((1_200..=3_000).contains(&per_second), "{per_second} bytes a second");
    }

    #[test]
    fn a_short_or_empty_recording_still_makes_a_stream() {
        let dir = Dir::new("short");
        round_trip(&dir, &sine(300.0, 0, 5));
        round_trip(&dir, &[]);
    }

    /// The loudest sounds a microphone gives — full-scale noise, a clipped square wave, the extremes back to back —
    /// encode like any other: the translated encoder has no overflow on them.
    #[test]
    fn full_scale_sound_encodes() {
        let dir = Dir::new("loud");
        let mut noise = 0x9E37_79B9_7F4A_7C15u64;
        let loud: Vec<i16> = (0..2 * 16_000)
            .map(|i| match i / 8_000 {
                0 => {
                    noise ^= noise << 13;
                    noise ^= noise >> 7;
                    noise ^= noise << 17;
                    (noise >> 48) as u16 as i16
                }
                1 if i % 40 < 20 => i16::MAX, // a square wave at 400 Hz
                2 if i % 2 == 0 => i16::MAX, // the extremes back to back
                _ => i16::MIN, // the other halves of those, then the lowest value held
            })
            .collect();
        round_trip(&dir, &loud);
    }

    /// The error of a failed encoding, with nothing left behind: no recording, no temporary file, and no path in
    /// what the error says.
    fn fails(dir: &Dir, wav: &Path) -> EncodeError {
        let before = dir.names();
        let out = dir.join("2026-10-31T12-00-00.000-2.ogg");
        let e = encode_wav_to_ogg_opus(wav, &out).unwrap_err();
        assert_eq!(dir.names(), before, "nothing is left behind");
        let said = format!("{e} {e:?}");
        assert!(!said.contains("cue-opus") && !said.contains("2026-10-31"), "no path in the error: {said}");
        e
    }

    #[test]
    fn a_cut_off_wav_is_an_error() {
        let dir = Dir::new("cut");
        let wav = dir.join("in.wav");
        write_wav(&wav, CUE_WAV, &sine(440.0, 1, 0));
        let whole = std::fs::read(&wav).unwrap();
        // Cut in the sound: the header promises more than there is; a temporary file was begun and goes.
        std::fs::write(&wav, &whole[..44 + 10_001]).unwrap();
        assert!(matches!(fails(&dir, &wav), EncodeError::Read(_)));
        // Cut in the header.
        std::fs::write(&wav, &whole[..30]).unwrap();
        assert!(matches!(fails(&dir, &wav), EncodeError::Read(_)));
        std::fs::write(&wav, b"this is not a recording at all").unwrap();
        assert!(matches!(fails(&dir, &wav), EncodeError::Read(_)));
        assert!(matches!(fails(&dir, &dir.join("missing.wav")), EncodeError::Read(_)));
    }

    #[test]
    fn a_wav_of_another_kind_is_an_error() {
        let dir = Dir::new("kind");
        let wav = dir.join("in.wav");
        let float = hound::WavSpec { bits_per_sample: 32, sample_format: hound::SampleFormat::Float, ..CUE_WAV };
        let mut w = hound::WavWriter::create(&wav, float).unwrap();
        for _ in 0..1600 {
            w.write_sample(0.25f32).unwrap();
        }
        w.finalize().unwrap();
        let e = fails(&dir, &wav);
        assert!(matches!(e, EncodeError::Format { rate: 16_000, channels: 1, bits: 32, kind: "float" }), "{e:?}");
        for spec in [
            hound::WavSpec { channels: 2, ..CUE_WAV },
            hound::WavSpec { sample_rate: 48_000, ..CUE_WAV },
            hound::WavSpec { bits_per_sample: 8, ..CUE_WAV },
        ] {
            let mut w = hound::WavWriter::create(&wav, spec).unwrap();
            for _ in 0..1600 {
                if spec.bits_per_sample == 8 { w.write_sample(0i8) } else { w.write_sample(0i16) }.unwrap();
            }
            w.finalize().unwrap();
            assert!(matches!(fails(&dir, &wav), EncodeError::Format { .. }), "{spec:?}");
        }
    }

    #[test]
    fn a_recording_that_cannot_be_written_is_an_error() {
        let dir = Dir::new("unwritable");
        let wav = dir.join("in.wav");
        write_wav(&wav, CUE_WAV, &sine(440.0, 1, 0));
        let e = encode_wav_to_ogg_opus(&wav, &dir.join("gone").join("a.ogg")).unwrap_err();
        assert!(matches!(e, EncodeError::Write(_)), "a missing folder is not created: {e:?}");
        assert!(matches!(encode_wav_to_ogg_opus(&wav, Path::new("")), Err(EncodeError::Write(_))));
        assert_eq!(dir.names(), ["in.wav"]);
    }

    /// The recording is written whole next to its name, `.tmp` added, and only then takes the name, replacing what
    /// was there; a recording no longer wanted at that moment goes.
    #[test]
    fn the_recording_takes_its_name_last() {
        let dir = Dir::new("rename");
        let wav = dir.join("in.wav");
        write_wav(&wav, CUE_WAV, &sine(440.0, 1, 0));
        let out = dir.join("2026-10-31T12-00-00.000-3.ogg");
        std::fs::write(&out, b"old").unwrap();
        let mut asked = 0;
        let e = encode_wav_to_ogg_opus_if(&wav, &out, || {
            asked += 1;
            let tmp = dir.join("2026-10-31T12-00-00.000-3.ogg.tmp");
            assert!(std::fs::read(&tmp).unwrap().starts_with(b"OggS"), "written whole under the temporary name");
            assert_eq!(std::fs::read(&out).unwrap(), b"old", "the name is not taken yet");
            false
        })
        .unwrap_err();
        assert!(matches!(e, EncodeError::Dropped));
        assert_eq!(asked, 1);
        assert_eq!(dir.names(), ["2026-10-31T12-00-00.000-3.ogg", "in.wav"], "the temporary file goes");
        assert_eq!(std::fs::read(&out).unwrap(), b"old");
        encode_wav_to_ogg_opus_if(&wav, &out, || true).unwrap();
        assert!(std::fs::read(&out).unwrap().starts_with(b"OggS"), "replaced");
        assert_eq!(dir.names(), ["2026-10-31T12-00-00.000-3.ogg", "in.wav"]);
    }
}
