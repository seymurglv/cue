# Cleanup on this computer: measured picks, plain captions, a recommendation by language

Status: design, for the owner's approval (05.11.2026).

## Why

The local models CUE recommends today were picked from their model cards, never measured (`core/llm.rs`, `catalog`).
The owner tried them: the recommended ones are slow. Superwhisper's S1-mini, found by search, cleans English well and
fast, but its card says English only. People choosing a model need to see, briefly, what each one is good at, backed
by real numbers, and CUE should point each person at the one that suits the languages they dictate in.

## Goals

- A short list of local cleanup models chosen by measurement on the same English and Russian dictations, not by
  their cards.
- Under each model's name, one plain line: what it is good at, how fast it is, its size. The numbers are measured.
- "Recommended for you" goes to the best English model for someone who dictates only in English, and to the best
  Russian-and-English model for someone who dictates in Russian too. CUE learns this from the dictations themselves.
- S1-mini runs the way it was trained, so its quality in CUE matches its card.

## Non-goals

- New speech engines (Whisper large-v3, a Qwen speech model). Speech stays Parakeet; Parakeet Ultra stays the
  recommended speech model (Russian and English).
- Cloud connections and their models.
- The owner measuring anything: the measuring is ours.

## Anti-goals (a failure even if it works)

- A recommendation that jumps around from day to day.
- A caption with a number nobody measured, or a mark that reads like a review score from nowhere.
- A Russian dictation sent to an English-only model and pasted garbled.
- CUE switching or downloading a model by itself because the recommendation moved.
- Rows that grow into paragraphs: one line under the name, the family's quiet style.

## Approaches to learning the languages (the owner picked the first, 05.11.2026)

1. **From dictations (picked).** CUE reads the alphabet of the user's dictations, which it already tells apart
   (`speech::alphabet_of`). No setting, and it follows the user's real habits. Risk: the mark may move once early on;
   settled by deciding after a few dictations and then only on a clear change (below).
2. Ask once, with a row "I dictate in: English / Russian and English". Exact, but one more setting that goes stale.
3. Show both kinds of captions and recommend neither. Simplest, but nobody is told what to pick.

## Design

### 0. What others already measured (the owner, 05.11.2026: start from models proven by benchmarks and real use)

- Scribe's local benchmark (25 dictations, a blind judge 0–100, English): Gemma 4 E4B Q4_K_M 89.2, Gemma 4 E2B 83.5
  (38.8 on a chain of self-corrections, against 77.0 for E4B), Qwen3 4B Instruct 81.7 (83.1 on CPU), Granite 4 3B
  80.7; larger Gemma 4 12B 91.9 and 26B-A4B 94.1 are too big for CUE's processor path.
- vinoWhisper (30 dictations, English): SmolLM3-3B 22/30, Gemma 4 E2B 19/30, Qwen3 4B 2507 19/30 (it obeyed an
  instruction planted in the text and looped).
- S1-mini's card: 94.8 % token accuracy on 7,519 English cases; English only; trained for this one job.
- No public dictation-cleanup benchmark tests Russian. Gemma 4 is reported stronger than Qwen at multilingual
  writing (anecdotal); Russian-tuned Qwen3 4B models exist (Vikhr's QVikhr-3 4B, RuadaptQwen3 4B) without published
  cleanup scores. Our own run (04.11.2026): Qwen3 4B Instruct cleans and translates Russian; Gemma 4 E2B and Qwen3 1.7B
  do not translate, and Qwen3 1.7B mostly leaves the text as recognized.
- So the starting picks are S1-mini for English and Gemma 4 E4B for Russian and English, with Qwen3 4B Instruct kept
  as the one proven to translate Russian here. Our measurement (below) is narrowed to what nobody has shown: Russian,
  and the speed on a processor.

### 1. Measuring

- A fixed set of dictations, half English and half Russian, written to look like Parakeet's output (its casing and
  punctuation, no invented speech errors beyond what Parakeet makes): fillers, a self-correction ("no, Friday"), numbers,
  dates and money, a list said aloud, a dictionary name, a short message, and two long dictations (about 200 words),
  each with a reference cleanup written by hand. None of it comes from the owner's history. It lives in the repo with
  the bench, in English and Russian, and is reused for any model later.
- Each candidate runs through CUE's real local path (`engines/local.rs`, the same prompt, grammar, pieces and
  `claude::settle` checks) on this computer, warm, with the default threads. For each model: the median seconds for a
  dictation of about 30 words and for one of about 200, how many results pass CUE's checks, and the distance of each
  result to its reference (word error rate after normalizing case and punctuation), per language.
- Every output is also read by hand; a model that passes the numbers but changes meaning fails.
- Candidates: the starting picks (S1-mini, Gemma 4 E4B, Qwen3 4B Instruct 2507), and, only to challenge Gemma 4 E4B
  on Russian, Vikhr's QVikhr-3 4B Instruction (Apache-2.0). Each at Q4_K_M. Gemma 4 E2B and Qwen3 1.7B leave the list:
  others and we found them weaker.
- The results go into a short table in this folder (model, language, seconds, pass rate, distance, size, verdict),
  with the computer they were measured on. Laptops are named as slower, by an estimate, not a number we did not
  measure.

### 2. The short list

- After measuring: the best English model, the best Russian-and-English model, and the fastest model that still
  passes in both languages, if it is a different one. Each pinned at its commit with size and SHA-256, as today.
- "Best" weighs quality first, then speed: a model is out if it fails CUE's checks or changes meaning on more than a
  few dictations, and among the rest the faster wins unless the slower is clearly better in reading.
- The catalog records each model's languages from the measurement (not from its card) and its measured speed, so the
  rows can say them.

### 3. The caption under each model

- One line, three parts at most: what it is for, its speed, its size. For example "English only · about 1 s · 0.5 GB"
  or "Russian and English · about 3 s · 2.5 GB". The speed is the measured median for a 30-word dictation on the
  measuring computer, rounded, written in plain words ("about 1 s").
- "Recommended for you" goes in front of the line of the one recommended; the others carry no mark.
- Models found by search have no measurement: their line says only what Hugging Face says (languages, size), as now.
- The exact look (words, an icon in place of a word, both themes) is decided on a variants page with 2–3 options on
  CUE's real Cleanup page, by the family's process, before it is built.

### 4. The recommendation

- Until the user has five dictations, CUE goes by the interface language: Russian → the Russian-and-English model,
  English → the English one.
- At the fifth dictation it decides once: any Russian among them → Russian-and-English; none → English.
- After that it changes only on a clear change. From English it moves to Russian and English when Russian makes up
  at least a quarter of the last twenty dictations; back to English only when none of the last twenty is Russian (our
  call: a bilingual user who dictates some English must never be pointed at an English-only model). It never switches
  the model in use and never downloads one; only the mark moves.
- The language of a dictation is the alphabet of its text as said (Cyrillic or Latin), read from the history on this
  computer; nothing of it leaves the computer.
- The decided mark is kept in `settings.json` as one value (English, or Russian and English), so a pruned, cleared or
  disabled history does not move it; only the "quarter of the last twenty" rule does, counted from whatever history is
  left (our call: the mark must not jump, and one non-private value is the least that keeps it still).
- The recommendation is only a mark; it never decides the model in use (our call, from the anti-goal):
  - Today an empty `refine.local` resolves to the catalog's first model. On upgrade, CUE writes the model the user is
    running now into `refine.local`, so a new catalog order or a moved mark changes nothing for them.
  - A user who turns on "This computer" with nothing chosen gets the model recommended at that moment, saved as their
    choice, as if they had pressed it; from then on it is theirs.

### 5. S1-mini, run as it was trained

- When the model in use is S1-mini (its repository is Superwhisper's `s1-mini`), CUE sends its system prompt word for
  word and its control line before the text, takes its answer as plain text (no CUE prompt, no JSON grammar), and
  still checks the answer with `claude::settle` (length, overlap) before pasting.
- The control line: `[Styling: semi-formal] [Structure: prose] [Context: general]` by default. Habits and the
  dictionary cannot be passed to S1-mini (it follows no instructions); the row says so in its line only if the
  variants show there is room, otherwise the Cleanup page's note does.
- An English-only model never cleans a dictation in another alphabet: a Russian dictation with S1-mini in use is
  pasted as said, with the normal "Done". "English-only" is what the measurement found, recorded in the catalog, not
  what a model card says: a model found by search is not gated, as today (our call: cards are unreliable, and gating
  by them would stop multilingual models from cleaning Russian).
- S1-mini cannot translate. A translation with it in use ends as a failed translation does today ("Couldn't
  translate", never the untranslated text), and the Cleanup page's note says that S1-mini cleans English and does not
  translate (our call: it keeps the rule that a translation is never pasted untranslated).
- Its license (Apache-2.0 with an added term) requires the name "S1-mini" by "Superwhisper", capitalized exactly so,
  wherever CUE shows it; NOTICE credits it.

### 6. Words

- README: the short list with its measured lines, how the recommendation is decided, what S1-mini cannot do.
- NOTICE: the models of the short list and their licenses.

## Constraints

- This computer: i7-12700KF, 8 performance cores, CPU only. Downloads of each candidate (0.5–3 GB) to measure.
- The repository's rules: English only in code and docs; no mention of development tools.
- The owner does not measure local models.

## Left to the builder

- The file format of the dictation set and the bench command that runs it (an example under `engines/examples`, as
  `refine_compare.rs`, or a `cue-cli` subcommand).
- How the history is read for the recommendation (at start and after each dictation, or from the core's own count).
- Where the measured speed and languages live in the catalog's types.
- The exact numbers of "a few dictations" and "clearly better" in the verdict, written down with the results.
