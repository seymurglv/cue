# Local speech recognition: Parakeet Ultra on this computer

Status: implemented, not yet released; bench and live check pending.

CUE stops sending sound to a speech service. A dictation is recognized on this computer by Parakeet Ultra (Moondream's
post-trained NVIDIA Parakeet TDT 0.6B v3: the same architecture and tokenizer, fewer mistakes, 25 European languages
with Russian and English among them, punctuation and capital letters in its output). Soniox goes completely: its key,
its stream, its batch fallback and its prices. The network is needed only to download the model once and for the
cleanup (Anthropic), which stays as it is.

The design already shows this version (design stage 3, trays "Model did not download" and "Models are ready"; the
welcome window, stage 6).

## The model

- **Files.** The int8 ONNX export of Parakeet Ultra from `Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx` on Hugging Face
  (CC-BY-4.0): `int8/encoder-model.int8.onnx` (≈ 649 MB), `int8/decoder_joint-model.int8.onnx` (≈ 18 MB),
  `vocab.txt` (8 192 tokens and `<blk>`), `config.json`. About 670 MB in all. The URLs name a fixed commit, not `main`,
  and every file has its SHA-256 in CUE's code: a file that does not match is deleted and downloaded again.
- **One loader for both.** NVIDIA's own v3 export (`istupakov/parakeet-tdt-0.6b-v3-onnx`, the same file names) loads
  the same way. CUE ships Ultra; v3 is used only to compare the two on the owner's own recordings before the release
  (the bench). If Ultra turns out worse on them, the pinned files change to v3 and nothing else does.
- **Where.** `%LOCALAPPDATA%\CUE\models\parakeet-ultra-int8\`. A file is downloaded as `<name>.part`, resumes from where
  it stopped (an HTTP range request), is checked, then renamed. A folder with every file checked is "ready". The
  installer does not carry the model; uninstalling removes the folder with the rest of CUE's local data.
- **Attribution.** NOTICE and the README name NVIDIA's Parakeet TDT 0.6B v3 (CC-BY-4.0), Moondream's Parakeet Ultra and
  the ONNX export, with links.

## What the user sees

**First start.** The welcome window's Keys step asks for the Anthropic key only. The model starts downloading as soon
as the welcome window opens, in the background; the step after Keys, **Model**, shows its progress as a thin bar with
"412 of 690 MB" (numbers in Bebas) and moves on by itself when it is ready. The Microphone step can be done meanwhile;
"Try it" waits for the model.

**A dictation before the model is ready.** The recording is kept as now; once released, the Drop thinks while the model
finishes downloading and loading, and the text is pasted then. If the download fails, the Drop shows the error with
Retry ("Model did not download" tray: "Stopped at **412 of 690 MB**. Retry continues from there."), and the recording
waits in `current.wav` like any failed attempt.

**Every day.** Nothing changes in how CUE is used: the chord, the Drop, the cleanup, translation, "Ask". While
recording the Drop shows the voice level (it never showed text). Text appears after release: recognition of a
30-second dictation takes well under a second on a current processor; the cleanup's request follows as now.

**When the model is ready after a download** in the background, a quiet tray: "Model is ready" — "Speech is recognized
on this computer." (no button, 5 s).

**Settings.** The Keys page keeps one row, Anthropic. A new row, **Speech recognition**, shows "Parakeet Ultra" and its
state: "Ready", "Downloading 61 %", "Did not download" with Retry. There is no choice of engine.

**Privacy line** (welcome window, settings): "Sound stays on this computer; text goes to Anthropic for the cleanup; the
history stays on this computer only."

## How recognition works

- **Runtime.** ONNX Runtime through the `ort` crate, CPU only, statically linked (no DLL next to CUE). Its prebuilt
  build needs a processor with AVX2 (2013 and later); on one without it, recognition reports "This processor is not
  supported" once and the Drop shows the error.
- **Features and decoding are CUE's own code** (`cue-engines::parakeet`), ported from `parakeet-rs` (MIT/Apache-2.0):
  128-band log-mel the way NeMo computes it (n_fft 512, window 400, hop 160, Hann, Slaney mel, per-feature
  normalization), then TDT greedy decoding (token and duration, at most 10 tokens a frame). Owning the decode loop is
  what lets CUE keep one language per dictation (below).
- **Loaded once.** The model loads in the background at start (≈ 1–2 s) and stays in memory (expected ≈ 1–1.5 GB,
  measured before release); a recording never waits for a load once CUE has been running a few seconds. 4 threads.
- **Long recordings.** A recording is cut into pieces of at most 60 s at the quietest 200 ms between 45 and 60 s;
  pieces are recognized one after another and joined with a space. A dictation is at most 9 minutes.
- **The same interface as now.** Recognition stays a session: frames go in while recording, `finish` returns the text.
  Today the session keeps the frames and recognizes them at `finish`; a streaming mode (pieces recognized at pauses
  while speaking) can be added later behind the same interface without changing the flow or the Drop.
- **Retry** recognizes `current.wav` again, the same way.

## One language per dictation

The model detects the language itself and cannot be told one. CUE's rule stays: a dictation is in Russian or in
English, never mixed.

- **Which language.** The letters of the recognized text are counted by script: more Cyrillic than Latin is Russian,
  the reverse is English (for translation: the language of the pair, target or home, written in that script; a fixed
  `translate.speech` still wins). This replaces Soniox's language tags.
- **A leak of the other script** (the model writing part of a Russian phrase in English, a known weakness): a leak is
  told from names and terms by the little words a sentence needs. When the minority script's words include at least two
  function words of its language ("so", "the", "to", "it"…; «и», «в», «не», «что»…), the decoder runs again over the
  same encoder output with that script's tokens masked out (a mask built from `vocab.txt`). The encoder does not run
  twice. Names and terms in the other script ("запусти cargo test в Visual Studio Code", "call Марисоль") have no such
  words and stay as recognized; the cleanup and the dictionary handle them.
- How often masking runs, and whether it helps, is measured on the owner's recordings in the bench before the release.

## What goes away

- Soniox: the stream (`soniox.rs`), the batch fallback and its uploads (`soniox/batch.rs`), the key and its check, the
  welcome window's Soniox link, the error "Soniox key problem", the settings `stt.soniox_model`,
  `stt.soniox_async_model` and `stt.language_hints` (old files keep them; they are ignored and not written again).
- The Soniox key in Windows Credential Manager is deleted once at the first start of this version (the log says
  "deleted" or "none").
- Prices: recognition costs nothing. usage.csv keeps its columns; new rows write engine `parakeet`, path `local`, STT
  cost `0`. Old rows read as before.
- The bench compares local models (Ultra and v3) on the owner's recordings, with the cleanup after each, instead of
  Soniox's stream and batch.

## Errors

| What | The Drop | Retry |
|---|---|---|
| The model is not downloaded yet and the download fails | error "Model did not download" | downloads again, then recognizes |
| The model files fail to load (damaged) | the same; the folder is checked again and bad files downloaded again | yes |
| Recognition fails (an ONNX Runtime error) | error, "Retry" | recognizes `current.wav` again |
| Processor without AVX2 | error "This processor is not supported" | no |

The log gets: model state changes, download bytes and seconds, load time, recognition time and audio length, the
language picked and whether masking ran. Never the text.

## Out of scope

Streaming text while speaking (later, behind the same session interface); GPU (DirectML) recognition; a choice of
models in settings; models for the cleanup on this computer (the design's "models" plural is for later).

## Testing

- Core: language by script (Russian, English, Latin names in Russian, numbers only), the 25 % share and the masked
  vocabulary, cutting a long recording at quiet points, usage rows of the local engine.
- Engine: log-mel against the reference preprocessor shipped with the v3 export (`nemo128.onnx`) for a synthetic
  clip (stored as a test fixture of numbers, not audio of a person), TDT decoding over a tiny fake model's outputs, the session (frames in, `finish`, abort).
- Download: resume from a `.part` file, a hash mismatch, a server error, checking a ready folder, with a local fake
  server.
- Runtime: a dictation before the model is ready waits and then pastes; retry; the error path.
- Welcome window, settings and the Drop's trays: the Model step, the Speech recognition row, the two trays.
- Bench and live: Ultra vs v3 on the owner's recordings (word error against the texts in the history), speed and
  memory on the owner's computer; short Russian and English phrases for the language rule.
