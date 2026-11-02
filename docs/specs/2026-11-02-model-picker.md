# Choosing the speech model

Status: agreed with the owner (02.11.2026), built; not yet checked in the running app.

Until now CUE has one speech model, Parakeet Ultra (2026-11-02-local-speech.md), and settings show it without a choice.
This adds a page where the model is chosen: a short list of models CUE knows, and a search over Hugging Face. Parakeet
Ultra stays the default.

## What CUE can run

CUE recognizes speech with its own features (128-band log-mel) and its own TDT greedy decoder
(`cue-engines::parakeet`). So it runs **Parakeet TDT models exported to ONNX the way onnx-asr exports them**, and
nothing else: Whisper, CTC and RNN-T models, Canary and GigaAM each need a recognition path of their own, which is out of
scope. Only the int8 export is used (recognition runs on the processor).

A Hugging Face repository is **runnable** when one of its folders (the root or any other, such as `int8/`) holds all
three of:

- `encoder-model.int8.onnx`
- `decoder_joint-model.int8.onnx`
- `vocab.txt`

and the repository is a TDT one: its tags include `nemo-conformer-tdt`, or its name includes `tdt`. When that folder has
a `config.json`, its `model_type` must be `nemo-conformer-tdt` and its `features_size` 128 (checked when the model is
chosen, since the search does not return file contents).

Anything else is shown but cannot be chosen, with the reason:

| Reason (RU / EN) | When |
|---|---|
| другая архитектура / another architecture | no `decoder_joint-model` file, or not TDT (Whisper, CTC, RNN-T…), or `config.json` names another model type |
| нет int8 / no int8 | the TDT files are there in fp32 or fp16 only |
| другая раскладка файлов / another file layout | TDT in separate encoder, decoder and joiner files (sherpa-onnx) |

A model that passes all of this and still fails to load (a prediction network of another size, a vocabulary without
`<blk>`, a joint narrower than the vocabulary) is **not runnable** after the download: its row says so, and the model
already in use keeps working.

## The models

**The short list**, pinned in CUE's code like Ultra today (repository, commit, every file's size and SHA-256):

| Name | Repository | Languages | Size |
|---|---|---|---|
| Parakeet Ultra (default) | `Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx` | 25 | ≈ 670 MB |
| Parakeet v3 | `istupakov/parakeet-tdt-0.6b-v3-onnx` | 25 | ≈ 670 MB |
| Parakeet v2 | `istupakov/parakeet-tdt-0.6b-v2-onnx` | English only | ≈ 660 MB |

Parakeet v2 recognizes no Russian: its row says "English only", and nothing else changes for it (every dictation comes
out in Latin letters and reads as English).

**A found model** is pinned when it is chosen, never at `main`: CUE asks Hugging Face for the repository's files at its
current commit and writes a manifest of its own, `models\<id>\manifest.json` (repository, commit, folder, the three
files with their sizes and SHA-256, languages, license). The large files' SHA-256 come from Hugging Face's LFS data.
`vocab.txt` is small and not in LFS: it is downloaded at once, checked against its git blob hash from the same listing,
and its SHA-256 written into the manifest. From then on the model is downloaded and checked exactly like Ultra (parts,
resume, SHA-256, rename). Its id is `hf--<owner>--<repository>`, lower case. A folder under `models` without a readable
manifest is not CUE's and is never shown or deleted.

A found repository that is on the short list is shown as that row, never twice.

**Disk.** Before a download CUE compares the free space on the drive of `models` with the bytes still to download plus
10 %. Too little: the row says «Нужно 0,7 ГБ, свободно 0,4» / "Needs 0.7 GB, 0.4 free", and nothing starts.

**Downloaded models stay** until removed by hand: going back to one is instant. Every downloaded model that is not the
chosen one has «Удалить» / "Delete" (its folder goes, the row shows its size again). The chosen one cannot be deleted.

## The settings

`stt.model` in settings.json: the chosen model's id, `parakeet-ultra-int8` when absent. An id with no short-list entry
and no readable manifest reads as Ultra (the log says so). It is written only once the new model has loaded.

## What the user sees

**Speech recognition page.** The model's row stays as it is («Parakeet Ultra · Готово · 668 МБ на этом компьютере»), its
title now the name of the model in use, and gets a chevron: it opens the **Model** page («Модель» / "Model"). The heading line
changes the way it does between other pages.

**The Model page**, top to bottom:

1. The group **«Модели» / "Models"**: the short list, then found models already downloaded. A row: the name; under it,
   small, the languages, and the size once it is here: «25 языков · 668 МБ» / "25 languages · 668 MB", «Только
   английский» / "English only". On the right: a check on the chosen one; the size on one not downloaded; «Удалить» /
   "Delete" on hover on one downloaded and not chosen.
2. The search, the list's last row and quiet until needed (chosen by the owner from three variants, 02.11.2026): the
   magnifier and «Найти на Hugging Face» / "Search Hugging Face" in the muted color. Pressed (or reached with Tab), the
   row becomes the field where it stands: the words stay in place as the field's hint, a 1 px line grows under it out
   of the magnifier, and once something is typed a cross at its end clears it. No ring, no pill. Esc clears what was
   typed, then closes the field; leaving an empty field closes it.
3. While the field has 2 letters or more (300 ms after the last key): the group **«На Hugging Face» / "On Hugging
   Face"**, up to 10 repositories by downloads: ONNX repositories that are about speech (Hugging Face's
   `automatic-speech-recognition`, or no pipeline named) and need no signing in. A row: `owner/repository`; under it, small, the downloads and the size.
   A row that cannot run is pale, cannot be pressed, and shows its reason instead of the size. No connection:
   «Нет связи с Hugging Face» / "No connection to Hugging Face". Nothing found: «Нет моделей Parakeet по запросу» / "No
   Parakeet models for this search".

**Choosing.** Pressing a runnable row chooses it:

- downloaded: the row says «Загружается» / "Loading" until it is ready;
- not downloaded: the row gets the download's look from today's model row, a 2 px bar and «Качается: **412** из **670**
  МБ» / "Downloading: **412** of **670** MB", then «Загружается»;
- a found one not yet on the list goes into «Модели» as it starts downloading.

The check moves only once the new model is ready. Until then dictations are recognized by the model in use. Then a
quiet tray, as after the first download: «Модель готова» — «Parakeet v3» / "Model is ready" — "Parakeet v3" (no button,
5 s).

The first page's Speech recognition value shows the model in use, as today; a download for a switch does not change it.

## Switching

- **The model in use works until the new one is ready.** The new one is downloaded and loaded in the background; when it
  has loaded, the slot recognition waits on gets it (`ModelSlot::ready`), the old one is let go, and `stt.model` is
  written. No restart. A dictation running at that moment finishes on the old model. While the new one loads (1–2 s)
  both are in memory, ≈ 2–3 GB at the peak.
- **Choosing another model while one downloads** stops that download (its parts stay for later) and starts the new one.
  Choosing the model in use stops any switch.
- **A download that stops**: the row says «Не скачалась · **412** из **670** МБ» with «Повторить» / "Retry"; the model
  in use stays.
- **Damaged files**: they are checked again and the bad ones deleted, as today; the row gets «Повторить».
- **Not runnable** after loading: «Не запускается в CUE» / "Does not run in CUE"; its files can be deleted.
- **At start** the chosen model loads as Ultra does today; one whose files are gone is downloaded again, with today's
  states and Retry.

## How it is built

- `cue_core::model`: `Manifest` and `ModelFile` become owned data, so a manifest can come from a file as well as from
  the code; the short list is `CATALOG` (Ultra first). Deciding whether a repository is runnable, from its file list and
  tags, is a pure function here, with the reason when it is not.
- `cue-engines`: a `models` module asks Hugging Face — the search (`/api/models?search=…&filter=onnx&sort=downloads`,
  with the file list, commit and tags in the same answer) and, on choosing, the repository at that commit with its LFS
  hashes. The download (`download.rs`) does not change beyond taking a manifest by reference. The loader reads the
  prediction network's size from the decoder's inputs instead of assuming 2 × 640, and checks that the joint is wider
  than the vocabulary.
- The runtime: `ModelKeeper` keeps the model in use and, apart, a switch in progress (its manifest, its state). Events
  name the model they are about, so a stopped switch's late events are ignored.
- The windows: `cue://models` carries the list (each model's id, name, languages, size, and state: chosen, downloaded,
  missing, downloading, loading, failed, not runnable); `cue://model` stays for the model in use. Commands, main window
  only: `models_get`, `models_search`, `model_choose`, `model_delete`. The search runs in CUE, not in the page.

## Out of scope

Other architectures (Whisper and the rest); fp32 and fp16 exports; GPU; a model of the user's own from a folder; several
models in memory for different languages.

## Testing

- Core: runnable or not, with each reason, over file lists taken from real repositories (Ultra's `int8/` layout,
  istupakov's root layout, sherpa-onnx, Whisper, fp32 only); the short list's URLs name commits; `stt.model` read and
  written, an unknown id reads as Ultra.
- Engines: Hugging Face's search and repository answers parsed into rows and into a manifest; the vocabulary's blob
  hash; a manifest written and read back; the prediction network's size from a model's inputs. HTTP against a local fake
  server, as the download's tests do.
- Runtime: a switch keeps the old model until the new one loads; a switch stopped by another choice ignores its late
  events; a failed switch keeps the old model and `stt.model`; the free-space check; deleting refuses the chosen model.
- Page: the Model page's rows in their states, choosing, the bar moving in place, the search with pale rows, no
  connection, nothing found, delete on hover.
