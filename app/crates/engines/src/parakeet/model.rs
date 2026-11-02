//! The ONNX model: the encoder over a recording's features, then the decoder/joint step by step ([`tdt::decode`]).
//! Files as the onnx-asr exports name them (`encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, `vocab.txt`).
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use cue_core::speech;
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;

use super::mel::{Features, Mel, N_MELS};
use super::tdt::{self, Joint};
use super::vocab::Vocab;
use crate::EngineError;

pub const ENCODER: &str = "encoder-model.int8.onnx";
pub const DECODER: &str = "decoder_joint-model.int8.onnx";
pub const VOCAB: &str = "vocab.txt";
/// Threads of one model run: half the processor's, 2 to 8 — enough for a fast recognition, few enough to leave the
/// rest of the computer alone.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(2, 8)
}
/// The prediction network's LSTM: 2 layers of 640.
const LSTM_LAYERS: usize = 2;
const LSTM_SIZE: usize = 640;

/// A recognized recording: its text, whether the other alphabet was masked out, and how long it took.
#[derive(Clone, PartialEq)]
pub struct Recognized {
    pub text: String,
    pub masked: bool,
    pub audio_ms: u64,
    pub took_ms: u64,
}

/// The text never goes into a log.
impl std::fmt::Debug for Recognized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Recognized")
            .field("chars", &self.text.chars().count())
            .field("masked", &self.masked)
            .field("audio_ms", &self.audio_ms)
            .field("took_ms", &self.took_ms)
            .finish()
    }
}

/// The loaded model. One recognition at a time (the sessions are behind a lock); the features and the vocabulary are
/// shared.
pub struct Recognizer {
    sessions: Mutex<Sessions>,
    vocab: Vocab,
    features: Features,
}

struct Sessions {
    encoder: Session,
    joint: Session,
}

/// ONNX Runtime allows one environment a process.
fn environment() -> Result<&'static Environment, EngineError> {
    static ENV: OnceLock<Environment> = OnceLock::new();
    if let Some(env) = ENV.get() {
        return Ok(env);
    }
    let env = ort::init().with_name("cue").build().map_err(model_error)?;
    Ok(ENV.get_or_init(|| env))
}

fn model_error(e: impl std::fmt::Display) -> EngineError {
    EngineError::Model(e.to_string())
}

/// ONNX Runtime's prebuilt library uses AVX2: without it the process would die on its first instruction, so this is
/// asked before anything loads.
pub fn processor_supported() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("avx2") && std::arch::is_x86_feature_detected!("fma")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        true
    }
}

fn session(path: &Path) -> Result<Session, EngineError> {
    Session::builder(environment()?)
        .map_err(model_error)?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(model_error)?
        .with_intra_threads(threads())
        .map_err(model_error)?
        .commit_from_file(path)
        .map_err(model_error)
}

impl Recognizer {
    /// Loads the model from `dir`. `Unsupported` on a processor ONNX Runtime cannot run on; `Model` for a missing or
    /// damaged file.
    pub fn load(dir: &Path) -> Result<Self, EngineError> {
        if !processor_supported() {
            return Err(EngineError::Unsupported("no AVX2".into()));
        }
        let vocab = std::fs::read_to_string(dir.join(VOCAB)).map_err(|e| model_error(format!("{VOCAB}: {e}")))?;
        let vocab = Vocab::parse(&vocab)?;
        let encoder = session(&dir.join(ENCODER))?;
        let joint = session(&dir.join(DECODER))?;
        Ok(Self { sessions: Mutex::new(Sessions { encoder, joint }), vocab, features: Features::new() })
    }

    /// A whole recording, 16 kHz mono: cut into pieces ([`speech::pieces`]), each recognized, joined with a space. When
    /// the text leaked into the other alphabet ([`speech::leak`]), every piece is decoded again without it, over the
    /// same encoder output.
    pub fn recognize(&self, pcm: &[i16]) -> Result<Recognized, EngineError> {
        let started = Instant::now();
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let pieces = self.pieces(&mut sessions, pcm)?;
        let (text, masked) = self.text(&mut sessions, &pieces)?;
        Ok(Recognized { text, masked, audio_ms: audio_ms(pcm.len()), took_ms: started.elapsed().as_millis() as u64 })
    }

    /// `pcm` cut into pieces ([`speech::pieces`]), each encoded and decoded without a mask.
    fn pieces(&self, sessions: &mut Sessions, pcm: &[i16]) -> Result<Vec<Piece>, EngineError> {
        let mut out = Vec::new();
        for range in speech::pieces(pcm) {
            let audio: Vec<f32> = pcm[range].iter().map(|&s| f32::from(s) / 32_768.0).collect();
            let mel = self.features.log_mel(&audio);
            if mel.frames > 0 {
                let encoded = sessions.encode(&mel)?;
                let ids = self.decode(sessions, &encoded, None)?;
                out.push(Piece { encoded, ids });
            }
        }
        Ok(out)
    }

    fn decode(&self, sessions: &mut Sessions, encoded: &Encoded, mask: Option<&[bool]>) -> Result<Vec<usize>, EngineError> {
        let mut joint = OrtJoint { session: &mut sessions.joint, encoded };
        tdt::decode(&mut joint, encoded.frames, self.vocab.len(), self.vocab.blank, mask)
    }

    /// The pieces' texts joined with a space, and whether the other alphabet was masked out: when it leaked
    /// ([`speech::leak`]), every piece is decoded again without it.
    fn text(&self, sessions: &mut Sessions, pieces: &[Piece]) -> Result<(String, bool), EngineError> {
        let join = |ids: &mut dyn Iterator<Item = Vec<usize>>| {
            ids.map(|ids| self.vocab.text(&ids)).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
        };
        let text = join(&mut pieces.iter().map(|p| p.ids.clone()));
        let Some(alphabet) = speech::leak(&text) else { return Ok((text, false)) };
        let mask = self.vocab.mask_of(alphabet);
        let ids = pieces.iter().map(|p| self.decode(sessions, &p.encoded, Some(&mask))).collect::<Result<Vec<_>, _>>()?;
        Ok((join(&mut ids.into_iter()), true))
    }
}

fn audio_ms(samples: usize) -> u64 {
    samples as u64 * 1_000 / speech::SAMPLE_RATE as u64
}

/// One piece recognized: the encoder's output, kept for a decode with a mask, and its tokens without one.
struct Piece {
    encoded: Encoded,
    ids: Vec<usize>,
}

/// A recognition fed while the speaker talks: each piece is recognized as it comes, so at the end only the tail is
/// left. The same text as [`Recognizer::recognize`] would give with the same cuts.
pub struct Running {
    model: Arc<Recognizer>,
    pieces: Vec<Piece>,
    samples: usize,
}

impl Running {
    pub fn new(model: Arc<Recognizer>) -> Self {
        Self { model, pieces: Vec::new(), samples: 0 }
    }

    /// The next piece of the recording, recognized now.
    pub fn add(&mut self, pcm: &[i16]) -> Result<(), EngineError> {
        let mut sessions = self.model.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        let pieces = self.model.pieces(&mut sessions, pcm)?;
        self.pieces.extend(pieces);
        self.samples += pcm.len();
        Ok(())
    }

    /// The tail recognized, and the text of the whole recording; `took_ms` is this last step's time.
    pub fn finish(mut self, tail: &[i16]) -> Result<Recognized, EngineError> {
        let started = Instant::now();
        let model = self.model.clone();
        let mut sessions = model.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        self.pieces.extend(model.pieces(&mut sessions, tail)?);
        let (text, masked) = model.text(&mut sessions, &self.pieces)?;
        Ok(Recognized {
            text,
            masked,
            audio_ms: audio_ms(self.samples + tail.len()),
            took_ms: started.elapsed().as_millis() as u64,
        })
    }
}

/// The encoder's output for one piece: `dim × frames`, dimension-major as the encoder gives it.
struct Encoded {
    dim: usize,
    frames: usize,
    data: Vec<f32>,
}

impl Sessions {
    fn encode(&mut self, mel: &Mel) -> Result<Encoded, EngineError> {
        let signal = Tensor::from_array(([1usize, N_MELS, mel.frames], mel.data.clone())).map_err(model_error)?;
        let length = Tensor::from_array(([1usize], vec![mel.frames as i64])).map_err(model_error)?;
        let outputs = self.encoder.run(ort::inputs!["audio_signal" => signal, "length" => length]).map_err(model_error)?;
        let (shape, data) = outputs["outputs"].try_extract_tensor::<f32>().map_err(model_error)?;
        let (_, lengths) = outputs["encoded_lengths"].try_extract_tensor::<i64>().map_err(model_error)?;
        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let [1, dim, total] = dims[..] else { return Err(model_error(format!("encoder output shape {dims:?}"))) };
        let frames = lengths.first().map_or(total, |&n| (n.max(0) as usize).min(total));
        Ok(Encoded { dim, frames, data: data.to_vec() })
    }
}

/// The decoder/joint session over one piece's encoder output.
struct OrtJoint<'a> {
    session: &'a mut Session,
    encoded: &'a Encoded,
}

/// The LSTM's hidden and cell states, `LSTM_LAYERS × 1 × LSTM_SIZE` each.
#[derive(Clone)]
struct LstmState {
    h: Vec<f32>,
    c: Vec<f32>,
}

impl Joint for OrtJoint<'_> {
    type State = LstmState;

    fn initial(&self) -> LstmState {
        LstmState { h: vec![0.0; LSTM_LAYERS * LSTM_SIZE], c: vec![0.0; LSTM_LAYERS * LSTM_SIZE] }
    }

    fn step(&mut self, frame: usize, last: usize, state: &LstmState) -> Result<(Vec<f32>, LstmState), EngineError> {
        let e = self.encoded;
        let column: Vec<f32> = (0..e.dim).map(|d| e.data[d * (e.data.len() / e.dim) + frame]).collect();
        let states = |v: &Vec<f32>| Tensor::from_array(([LSTM_LAYERS, 1usize, LSTM_SIZE], v.clone())).map_err(model_error);
        let outputs = self
            .session
            .run(ort::inputs![
                "encoder_outputs" => Tensor::from_array(([1usize, e.dim, 1], column)).map_err(model_error)?,
                "targets" => Tensor::from_array(([1usize, 1], vec![last as i32])).map_err(model_error)?,
                "target_length" => Tensor::from_array(([1usize], vec![1i32])).map_err(model_error)?,
                "input_states_1" => states(&state.h)?,
                "input_states_2" => states(&state.c)?,
            ])
            .map_err(model_error)?;
        let (_, logits) = outputs["outputs"].try_extract_tensor::<f32>().map_err(model_error)?;
        let (_, h) = outputs["output_states_1"].try_extract_tensor::<f32>().map_err(model_error)?;
        let (_, c) = outputs["output_states_2"].try_extract_tensor::<f32>().map_err(model_error)?;
        Ok((logits.to_vec(), LstmState { h: h.to_vec(), c: c.to_vec() }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The model's folder for the tests that need the real files (`CUE_MODEL_DIR`), else none.
    fn model_dir() -> Option<std::path::PathBuf> {
        std::env::var_os("CUE_MODEL_DIR").map(Into::into)
    }

    /// The export's inputs and outputs by name and type: the names `encode` and `step` use.
    #[test]
    #[ignore = "needs the model files (CUE_MODEL_DIR)"]
    fn the_export_has_the_inputs_and_outputs_used() {
        let dir = model_dir().expect("CUE_MODEL_DIR");
        for file in [ENCODER, DECODER] {
            let s = session(&dir.join(file)).unwrap();
            let inputs: Vec<_> = s.inputs().iter().map(|o| format!("{} {:?}", o.name(), o.dtype())).collect();
            let outputs: Vec<_> = s.outputs().iter().map(|o| format!("{} {:?}", o.name(), o.dtype())).collect();
            println!("{file}\n  in:  {inputs:?}\n  out: {outputs:?}");
        }
    }

    /// A WAV of speech (`CUE_TEST_WAV`, 16 kHz mono) recognized end to end: prints the time, never asserted text.
    #[test]
    #[ignore = "needs the model files (CUE_MODEL_DIR) and a recording (CUE_TEST_WAV)"]
    fn a_recording_is_recognized() {
        let dir = model_dir().expect("CUE_MODEL_DIR");
        let wav = std::env::var("CUE_TEST_WAV").expect("CUE_TEST_WAV");
        let pcm: Vec<i16> = hound::WavReader::open(wav).unwrap().samples::<i16>().map(Result::unwrap).collect();
        let t = Instant::now();
        let r = Recognizer::load(&dir).unwrap();
        println!("loaded in {} ms", t.elapsed().as_millis());
        let out = r.recognize(&pcm).unwrap();
        println!("{out:?}\n{}", out.text);
        assert!(!out.text.is_empty());
    }

    /// The same WAV fed in pieces as a dictation feeds them ([`speech::early_cut`]): prints the time left after the
    /// last frame and both texts, the whole recording's and the pieces', never asserted.
    #[test]
    #[ignore = "needs the model files (CUE_MODEL_DIR) and a recording (CUE_TEST_WAV)"]
    fn a_recording_fed_in_pieces_is_recognized() {
        let dir = model_dir().expect("CUE_MODEL_DIR");
        let wav = std::env::var("CUE_TEST_WAV").expect("CUE_TEST_WAV");
        let pcm: Vec<i16> = hound::WavReader::open(wav).unwrap().samples::<i16>().map(Result::unwrap).collect();
        let model = Arc::new(Recognizer::load(&dir).unwrap());
        println!("threads {}", threads());
        let whole = model.recognize(&pcm).unwrap();
        let mut running = Running::new(model.clone());
        let mut fed = 0;
        while let Some(cut) = speech::early_cut(&pcm[fed..]) {
            let t = Instant::now();
            running.add(&pcm[fed..fed + cut]).unwrap();
            println!("piece of {} ms in {} ms", audio_ms(cut), t.elapsed().as_millis());
            fed += cut;
        }
        let out = running.finish(&pcm[fed..]).unwrap();
        println!("whole {whole:?}\n{}\npieces {out:?}\n{}", whole.text, out.text);
        assert!(!out.text.is_empty());
    }

    /// Our log-mel against the export's own preprocessor (`nemo128.onnx`, path in `CUE_NEMO128`) on a synthetic chirp.
    #[test]
    #[ignore = "needs the export's nemo128.onnx (CUE_NEMO128)"]
    fn features_match_the_reference_preprocessor() {
        let path = std::env::var("CUE_NEMO128").expect("CUE_NEMO128");
        let mut s = session(Path::new(&path)).unwrap();
        let audio: Vec<f32> = (0..24_000)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                0.4 * (2.0 * std::f32::consts::PI * (200.0 + 1_800.0 * t) * t).sin()
            })
            .collect();
        let names: Vec<String> = s.inputs().iter().map(|o| o.name().to_owned()).collect();
        let wave = Tensor::from_array(([1usize, audio.len()], audio.clone())).unwrap();
        let lens = Tensor::from_array(([1usize], vec![audio.len() as i64])).unwrap();
        let out = s.run(ort::inputs![names[0].as_str() => wave, names[1].as_str() => lens]).unwrap();
        let (shape, reference) = out[0].try_extract_tensor::<f32>().unwrap();
        let ours = Features::new().log_mel(&audio);
        let frames = shape[2] as usize;
        println!("reference {shape:?}, ours {} frames", ours.frames);
        let common = frames.min(ours.frames);
        let mut worst = 0.0f32;
        for band in 0..N_MELS {
            for f in 0..common {
                worst = worst.max((reference[band * frames + f] - ours.data[band * ours.frames + f]).abs());
            }
        }
        println!("largest difference {worst}");
        assert!(worst < 1e-3, "features differ from the reference by {worst}");
    }

    #[test]
    fn a_missing_folder_is_a_model_error() {
        if !processor_supported() {
            return;
        }
        let err = Recognizer::load(Path::new("no such folder")).err().unwrap();
        assert!(matches!(err, EngineError::Model(_)), "{err:?}");
    }
}
