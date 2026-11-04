# Cleanup on this computer or in the cloud

Status: design, waiting for the owner's approval (04.11.2026). Not built.

## Why

The owner wants CUE as simple to use as Typeless, Wispr Flow or Superwhisper: few settings, and good text out of every
dictation. Today the cleanup reaches an AI through one of ten connections, two of them local servers the user must
install and run (Ollama, LM Studio), each with a key or an address. The owner wants CUE usable for free, with no key,
no account and no other program, and later with a model of their own (a distant plan, not this work).

## What the owner decided (04.11.2026)

- **Cleanup is always on.** There is no "off": plain speech-model text is not offered as a setting.
- **It runs in one of two places:** **on this computer**, free, with a small language model CUE downloads and runs
  itself; or **in the cloud**, through one of the cloud connections and the user's key.
- **The local model is chosen like the speech model:** a short list with the recommended one marked, and a search of
  Hugging Face for others. Downloads come **only from Hugging Face**, pinned and checked.
- **The cloud keeps a model choice** per connection, the recommended one marked (as today).
- **The style stays as today:** «По умолчанию | Свой» / "Default | Custom", the user's instructions replacing CUE's
  cleanup (docs/specs/2026-11-02-refine-connections.md). Three levels, and levels plus the user's rules, were
  considered; the owner kept the two: simpler. The style applies to the cleanup only.
- **"Ask" and translation are unchanged:** their own prompts, never the custom instructions. "Ask" stays on the
  Anthropic key; translation goes wherever the cleanup runs (this computer or the chosen cloud connection), as it
  follows the connection today.
- **No local model testing now.** CUE gets everything needed to choose, download and run a local model; people who
  want it use it. The short list is picked from model cards, not measured.
- **CPU only.** No GPU path in this work.
- **New installs start on Cloud**, as today; "This computer" is one switch away.
- **Languages:** Russian and English, as today. Parakeet Ultra stays the speech model. Making CUE universal across
  languages is not part of this.
- **The page:** variant A of the mockup "CUE · Cleanup page, three variants"
  (https://claude.ai/artifact/JfXjgJc3xUE43aTh3qNrLP): a switch on top, its settings under it.

## Goals, non-goals, anti-goals

Goals:
- Cleanup with no key and no other program: one switch and a download.
- A user with a key keeps today's cloud cleanup, with fewer choices around it.
- One decision ("Runs on") decides what the rest of the page shows.

Non-goals: CUE's own model; a GPU path; languages beyond Russian and English; cleanup off; style levels; measuring
local models on the owner's recordings; Ollama, LM Studio or any other local server; a model from a folder on disk.

Anti-goals (a failure even if it works):
- A download that does not come from Hugging Face, or is not checked against the pinned SHA-256.
- A local answer pasted when it changed what was said: every local answer goes through the same checks as a cloud one,
  and a failed check pastes the raw text.
- Text of a dictation leaving the computer when "This computer" is chosen.
- Dictation blocked or the computer frozen by the local model: dictation never waits for a download, and the model
  leaves a core free (see "Threads").

## Approaches considered

The page (shown to the owner as live variants):

| | Trade-off | |
|---|---|---|
| **A · Switch** | The one decision said once on top; the local list is the speech Model page the owner knows; height changes with the switch | **Picked** |
| B · Two rows | Shortest page, but every model change is two clicks and a page deeper | |
| C · One list | One click to anything, but the longest page, local and cloud models mixed | |

Running a local model (the builder's call, stated here):

| | Trade-off | |
|---|---|---|
| **llama.cpp, GGUF** | Almost every small open model on Hugging Face is published as GGUF; grammar-constrained output; prompt-prefix cache; CPU is first-class | **Chosen** |
| ONNX Runtime GenAI | CUE already links ONNX Runtime, but few models are exported for it, so the search would find little | |
| Keep Ollama / LM Studio | Already works, but the user installs and runs another program: what the owner wants gone | |

The style (asked of the owner): today's "Default | Custom" (**picked**); three levels "Casual | Default | Formal";
three levels plus the user's added rules.

## The Cleanup page (variant A)

Title "Cleanup" / «Чистка текста». One group, the model rows, the note. All rows are today's tray parts (`srow`,
`segc`, `mlist`, `msr`, `pk`, `kin`, `dd`, `tft`).

1. **"Style"** / «Стиль»: as today, «По умолчанию | Свой» with the instructions field under it for Custom. (The
   mockup shows no Style row; it stays at the top, as on today's page.)
2. **"Runs on"** / «Где работает»: a segment **"This computer | Cloud"** / «Этот компьютер | Облако». Hint under the
   title: "Free, private" / «Бесплатно, приватно» or "Your key, fastest" / «Ваш ключ, быстрее всего». Switching
   morphs the page below it (the tray's height follows).
3. **This computer:** the group **"Models"** / «Модели», built exactly like the speech Model page
   (docs/specs/2026-11-02-model-picker.md, "What the user sees"): the short list first, then found models already
   downloaded; a row is the name, under it a quiet line ("Recommended · Russian and English", the size once here), a
   check on the one in use, the size on one not downloaded, "Delete" on hover on one downloaded, the one in use too
   (owner's ruling of 04.11.2026: it is let go, and dictations paste as said until another is chosen). Then
   the search row, "Search Hugging Face", which opens in place as on the Model page, and the group "On Hugging Face"
   while it has results. Choosing, downloading, "Loading", Retry and the disk check behave as on the Model page.
4. **Cloud:** the connection card (lamp, name, key state; its list now holds only the cloud connections, no groups),
   the key row with "Check", the model drop-down (recommended marked). As today. Under the note, the group
   "Models on this computer" / «Модели на этом компьютере» with the local models downloaded here, each with its size
   and "Delete"; nothing is chosen or searched there, and the group is not shown when none is downloaded.
5. **The note:** This computer: "Free. The text never leaves this computer; the model runs on its processor."
   Cloud: today's "Text goes to {NAME} for the cleanup; the history stays on this computer only."

**The first page's Cleanup value:** "This computer" / «Этот компьютер» or the connection's name. In amber: "No {NAME}
key" (cloud, no key, as today), "Model not downloaded" / «Модель не скачана» (this computer, nothing downloaded yet),
"Downloading" with the percentage while it downloads.

**Removed from the page:** Ollama, LM Studio and the address row.

## How the local cleanup behaves

- **Until a local model is ready** (not chosen yet, downloading, loading, failed), a dictation pastes the speech
  model's text, as it does today when the cleanup cannot answer (`RawFallback`), and the History marks it as not
  cleaned up. Dictation never waits for a download.
- **The model stays loaded** while CUE runs on "This computer", beside the speech model, so a dictation does not wait
  for it to load. It loads at start (after the speech model) and when chosen. Expected memory: the speech model
  ≈ 1–2 GB plus the local model ≈ its file size + 0.5 GB. **On Cloud no local model is in memory** (owner's ruling of
  04.11.2026): the one in use is let go on the switch, nothing loads at start, and a download finishing there makes its
  model the chosen one, loaded once the cleanup comes back to this computer.
- **The prompt is CUE's cleanup prompt**, the same text the cloud gets: CUE's rules (or the custom instructions), the
  dictionary block, the habits block, the context before the cursor. The system part is evaluated once and its state
  kept (llama.cpp's prompt cache), so each dictation pays only for its own words.
- **The answer** is forced to the `{"text": "…"}` object by a grammar, and goes through the same checks as a cloud
  answer (`sanity_ok`, `strip_markup`, the reply-word rule). A failed check pastes the raw text (`RawFallback`).
- **Long dictations** (owner's ruling of 04.11.2026: wait for the cleanup): a dictation of more than 80 words is cleaned
  in pieces of up to 80 words ending where a sentence does (`refine::pieces`), one after another, each with the text
  before the caret and the pieces already cleaned as its context; there is no limit on the length. A piece that fails
  stays as said; after one runs out of time the rest stays as said. A translation is whole or not at all, and a local
  translation gets no text before the caret (a small model writes it, translated, into its answer).
- **Time limit:** 20 s plus 0.4 s a word for each piece (a minute of speech takes about 30 s on an 8-core desktop,
  two or three times more on a laptop); past it, that piece is pasted as said. Esc cancels the cleanup as today.
- **Threads:** from Windows (`GetLogicalProcessorInformationEx`): on a processor with performance and efficient cores,
  the performance cores (an efficient one would hold the others back at every step: 9 threads on an i7-12700 took
  80 s where its 8 performance cores take 47); otherwise all physical cores but one. The grammar is asked about the
  model's own choice first and filters the whole vocabulary only when it refuses: the same answer, 1.5 times faster.
- **CPU only:** llama.cpp is built for the processor; no CUDA, Vulkan or DirectML in this work.
- **Habits** learned on "This computer" get their rule from the local model or the template, never from the cloud.
- **Usage accounting:** usage.csv's `refine_model` is `local:{model id}`, costing nothing.
- **Log:** latencies and outcomes only, as today; never the text.

## Which local models CUE can run

A Hugging Face repository is **runnable** when it holds a **single-file GGUF** of a text-generation model whose
architecture the linked llama.cpp supports, with a chat template in its metadata, in one of the quantizations
`Q4_K_M`, `Q4_K_S`, `Q5_K_M`, `Q8_0`, `Q4_0` (the first found in that order is used), and that file is at most
**6 GB**. Split GGUFs, vision projectors and models without a chat template are shown pale with the reason ("several
files", "no chat template", "too large", "another architecture"), as on the speech Model page. What can only be known
after the download (the architecture loads, the template applies) is checked when it loads: "Does not run in CUE", and
the model in use keeps working.

The search: `/api/models?search=…&filter=gguf&sort=downloads`, text-generation (or no pipeline named), no sign-in
needed (gated repositories are left out, as `hub.rs` does today), up to 10 rows. A chosen repository is pinned at its
current commit into `models\<id>\manifest.json` (repository, commit, the one file, its size and SHA-256 from Hugging
Face's LFS data, license), downloaded and checked exactly like a speech model (parts, resume, SHA-256, rename). Ids:
`llm--<owner>--<repository>--<quant>`, lower case.

**The short list** is pinned in CUE's code like Parakeet Ultra: three instruction-tuned models of 1.5–4 B parameters,
Q4_K_M, openly licensed, no sign-in, whose model cards claim good Russian and English (for example Gemma 4 E4B,
Qwen3 4B, Qwen3 1.7B, whatever is current on Hugging Face at build time). The first is recommended. They are picked
from the model cards, not measured (the owner's decision). Each model's license goes into NOTICE and the README.

## Settings

In `refine` of settings.json:

| Field | Values | Default |
|---|---|---|
| `runs` | `"local"`, `"cloud"` | `"cloud"` |
| `local` | the local model's id | the recommended one |
| `style`, `custom` | as today | as today |
| `connection` | a cloud connection's id; anything else reads as `anthropic` | `"anthropic"` |
| `models` | as today (cloud connection → model) | `{}` |

Read past and never written again: `addresses`. A `connection` of `ollama` or `lmstudio` reads as `runs: "local"` with
the recommended local model (not downloaded: the first page says so; nothing downloads by itself) and `connection:
"anthropic"`.

## The welcome window

Unchanged for a new install (Cloud: its Keys step asks for the key, as today). With `runs: "local"` it never asks for
a key.

## How it is built

- `cue_core::connections`: Ollama and LM Studio leave the list; the `local` flag and addresses go.
- `cue_core::settings`: `runs`, `local`; the migration above; the `refineAddress` change is removed.
- `cue_core::model`: the catalog gains a second kind of model (language model) beside speech models; the runnable check
  for GGUF repositories is a pure function with its reasons.
- `cue-engines`: a `local_llm` module over llama.cpp (Rust bindings, static link, CPU): load a GGUF, keep the system
  prompt's state, generate under a JSON grammar, stop at the time limit. The Hugging Face search and pinning reuse
  `hub.rs` with the GGUF filter; the download reuses `download.rs` unchanged.
- The runtime: a keeper for the local model like `ModelKeeper` (in use, a switch in progress, events naming their
  model); refine and translation pick the local engine or the cloud one by `runs`.
- The windows: `cue://llms` carries the local list (as `cue://models` does); commands `llms_get`, `llms_search`,
  `llm_choose`, `llm_delete`; the Cleanup page as above; the first-page value.
- The installer stays one file; llama.cpp is linked into cue.exe (its MIT license in NOTICE).

## Testing

- Pure: the GGUF runnable check and its reasons; settings migration (`addresses`, ollama, lmstudio); the first-page
  value in each state.
- Engine: a tiny GGUF fixture loads and answers under the grammar; the time limit stops generation; a broken file says
  "Does not run in CUE".
- UI: the Cleanup page in both places, the switch morph, the local list states (as the Model page tests do), the cloud
  rows, the Style row, both themes.
- Smoke, by hand (the files are large, not in CI): each short-list model downloads, loads, and cleans one RU and one EN
  phrase.
- The cloud path is checked live as before; the local path gets no live test with the owner (the owner's decision).

## Left to the builder

The llama.cpp binding crate and build flags; the exact grammar; thread tuning; how the prompt state is cached; the
three short-list models; whether the local model stays loaded while the setting is on Cloud; row texts not given here,
in the family's voice, RU and EN.
