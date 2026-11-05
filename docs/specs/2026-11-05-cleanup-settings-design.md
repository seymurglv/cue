# Cleanup and speech settings, variant B: design

Date: 2026-11-05. Status: draft, not approved and not built; parked. Later the same day the owner moved on from it:
cleanup becomes two choices (AI in the cloud with a key, or a model on this computer) with all eight services shown as
tiles, a "Set up with an AI agent" button that copies a prompt, Parakeet Ultra for both languages, and Gemma dropped.
Prototype of that: https://claude.ai/artifact/E3FBicdijn4oEnxcpGEVj5 (`.superpowers/mockups/simple-cleanup.html`). This
document is to be rewritten around those decisions. Prototype the owner first picked from (variant B):
https://claude.ai/artifact/8KQJLtFJPL4HUmyLvaFRq5 (private; file `.superpowers/mockups/cleanup-variants.html`, not in the
repository). Related: `docs/specs/2026-11-05-cleanup-model-picks.md` (on branch `model-picks`, not merged), which decides
which local model is recommended for the languages a user dictates.

## Why

The owner's words: speech recognition and cleanup "turned into a big mess"; they want it "as simple as possible and very
practical", and before the clean public release. A first simplification hid every model choice under "Advanced"; the
owner rejected it ("we limited it too much"; "the selector must be there in any case") and asked to learn from Wispr
Flow, Handy, Superwhisper, FluidVoice and Typeless and do better. Of two clickable variants the owner picked B: where
cleanup runs first, then the model, close to today's CUE.

What the other apps taught (research of 2026-11-05):
- Wispr Flow controls cleanup by strength (None / Light / Medium), not by model.
- Superwhisper shows the language model right where AI is switched on; hiding it (Handy: under Experimental) is what
  reviews criticize.
- Handy and FluidVoice group models by fit ("for your languages") and say each model's languages and size.
- VoiceInk and FluidVoice check a key when it is entered and show "Connected".
- Typeless has almost no settings and is criticized for the lack of control.

## Goals

- One page per subject, each choice in one place, the model always visible on the Cleanup page.
- A user who never opens settings gets good results; one who does sees at a glance where text goes and with which model.
- Cleanup has a strength the user understands without knowing what a model is.
- Downloaded models are managed in one place, with their sizes.

## Non-goals

- Microphone, Languages, History and General pages do not change.
- No new cloud providers; all eight stay (Anthropic, OpenAI, Google Gemini, OpenRouter, DeepSeek, Mistral, Groq, xAI).
- No styles per app (Wispr's Personal / Work / Email): a later idea; CUE's habits already learn per app.
- No "Undo AI edit" now (history keeps the raw text already): a later idea.
- Ask and its Claude key do not change.

## Anti-goals (a failure even if it works)

- The model choice hidden behind a page or a toggle.
- One setting reachable from two places (for example deleting a model on two pages).
- A choice whose effect the page does not say (where text goes, what a strength does).
- Blue anywhere but Ask; a status shown by color alone.

## Approaches

| | What it is | Trade-off | Outcome |
|---|---|---|---|
| Hidden | Off / Computer / Cloud and one Anthropic key; models under Advanced | Fewest choices, but the model is hidden and providers are cut | Rejected by the owner |
| A, one model | Strength and one Model row; local and cloud models in one list; where it runs follows from the model | One choice fewer; less familiar | Not picked |
| **B, where then model** | Off / This computer / Cloud, then strength, then the models or the connection right below | Familiar to today's CUE; two choices before the model | **Picked by the owner** |

## Today (what changes from)

- First settings page: Speech recognition (value "Ready", opens the speech model page), Cleanup (value: the connection
  or "This computer"), Microphone, Languages, History, General.
- Cleanup page: Style (Default / Custom; Custom puts the user's instructions in place of CUE's prompt), Runs on (This
  computer / Cloud); on this computer a model list with Hugging Face search; in the cloud the connection picker, its key
  with Check, the model, and "Models on this computer" to delete downloaded ones.
- Settings (`settings.json`, `refine`): `runs`, `local`, `connection`, `models` (per connection), `style`, `custom`, and
  thresholds. Keys are in Windows Credential Manager; Check answers works / bad key / no money on the account / missing.

## The design

Copy below is English; Russian follows the prototype's strings. Every new string exists in both languages.

### First settings page

1. Speech recognition. Value: the model in use ("Parakeet Ultra"), or its download state. Opens the Speech page.
2. Cleanup. Value: "Off"; or the local model ("Gemma 4 E4B", "Gemma 4 E4B · download", "Downloading 42 %"); or the cloud
   model ("Claude Haiku 5.5"). Opens the Cleanup page.
3. Microphone, Languages, History, General: as today.
4. A divider, then **Models on this computer**. Value: their total size ("6.2 GB"). Opens the Library page. There is no
   "Advanced".

New icon for the Library row: a drive, drawn on CUE's 20-unit grid with the 1.8 stroke (`icons.ts`).

### Cleanup page

Rows from top to bottom:

1. **Where**: a full-width three-part control **Off · This computer · Cloud**.
2. **Strength** (hidden when Off): **Light · Medium**, with a one-line description that follows the choice.
   - Light: "Punctuation, capitals, no "um" or slips". It keeps the speaker's wording.
   - Medium: "Also drops repeats and smooths phrasing". This is today's CUE cleanup.
3. **This computer**: a group "Model" listing the local models, each a row with a radio mark, its name, a line of tags
   (languages: "RU · EN" or "English only"; "translates"; "recommended") and its size.
   - A model not downloaded says so in its tag line; choosing it starts its download in place: a progress bar under the
     name, the percent on the right, and Stop.
   - The catalog models come first, then found models already downloaded. Today's catalog (`core/llm.rs`) is Qwen3 4B
     Instruct, Gemma 4 E2B and Qwen3 1.7B; the model-picks spec replaces it with S1 Mini, Gemma 4 E4B and Qwen3 4B
     Instruct, the list the prototype shows.
   - "recommended" marks the model the model-picks spec recommends for the user's languages.
   - Below the group: "Find a model on Hugging Face", as today.
4. **Cloud**:
   - The connection, a wide button with the provider's name and a lamp; it opens a menu of the eight providers, each
     with a lamp that is green when its key is saved and its last check passed.
   - The key row: a password field and Check.
     - The key is checked by itself once it is saved (Enter or paste and leave the field), as well as on Check.
     - Its line says Connected, Key needed, or today's failures (bad key; "No money on the account" in amber).
     - For Anthropic it adds "It also works for Ask and translation".
   - Model: a dropdown of the provider's models, as today (the model is remembered per provider).
5. **Your instructions** (hidden when Off): opens a page with one text box. Its line says "Optional: how to write, what
   to leave alone", or "Yours added" when there is text.
6. Footer, one line that says where text goes:
   - This computer: "Free. Text never leaves this computer; the model runs on its processor." While the chosen model is
     not downloaded, it adds "Until then text is pasted as recognized."
   - Cloud: "Text goes to {provider} ({model}) for cleanup; history stays on this computer."
   - Off: "Text is pasted as recognized: Parakeet adds punctuation and capitals itself. Translation and Ask work with an
     Anthropic key."

Switching Off → This computer or Cloud brings back the last strength (Medium the first time).

### Your instructions page

- One text box (CUE's `.cta`) and the footer "CUE adds this to its cleanup at the strength you chose."
- The text is **added** to CUE's prompt, at either strength. Today's Custom style **replaces** CUE's prompt instead.

### Speech page

- Today's speech model page, kept: Parakeet Ultra (recommended, 25 languages), Parakeet v3, Parakeet v2 (English only),
  found models, and Hugging Face search.
- Each row: radio mark, tags, size, download state.
- Deleting a downloaded model moves to the Library page; this page only chooses.

### Library page: "Models on this computer"

- Every downloaded model, speech and cleanup.
- Each row: its size and role ("speech", "cleanup"), "in use" where it is, and Delete.
  - The speech model in use has no Delete: today's rule that it cannot be deleted stays.
  - The cleanup model in use can be deleted, as today; cleanup then pastes the text as recognized until another is
    chosen.
- Footer: "{total} in all".

## Behavior changes

- **Strength.**
  - Light is a new, careful prompt: punctuation, capitals, fillers and self-corrections, and the wording kept. It is
    close to the "careful" style CUE had before commit fd9261a merged its styles.
  - Medium is today's prompt (`refine::BOLD_PROMPT`).
  - Both strengths go to every connection and to the local models, except S1 Mini, which runs with its own trained
    prompt and control line (model-picks spec). With S1 Mini chosen, the Strength row and Your instructions are
    replaced by one line: "S1 Mini cleans up its own way, English only" (our call).
  - The output checks (`min_ratio`, `max_ratio`, `min_overlap`) stay as they are; Light may warrant tighter ones, the
    builder measures.
- **Off.**
  - Dictation is pasted as recognized, with no model loaded for it; the local model is let go as on the cloud.
  - The cleanup's `runs`, `connection` and models stay stored, so switching back restores them.
  - Translation and Ask keep working through their own routes: Ask through Anthropic as today. Translation uses the
    stored connection (cloud) or the local model (this computer) as today, and needs a key or a downloaded model.
  - Habits (our call): corrections are still counted, but a habit's rule comes from CUE's template only, never from a
    model, so no dictated text leaves the computer while Off. The History page's existing note ("Habits work when
    cleanup is on") shows, since found habits are not applied without cleanup.
- **Instructions are added, not substituted** (above).
- **The key is checked by itself** when saved.

## Settings

- `refine.on`: `true | false`, default `true`. Off is `on: false`; `runs` keeps meaning where cleanup runs when on.
  (Our call: a separate switch, so the last strength survives Off and a restart.)
- `refine.strength`: `"light" | "medium"`, default `"medium"`. (Our call: not named `level`, because older files carry
  a `refine.level` that is read past today, see `old_refine_fields_are_read_past` in `settings.rs`; it stays ignored.)
- `refine.custom` keeps the instructions; `refine.style` is read only to migrate and then dropped from what is written.
- Migration of an existing `settings.json`:
  - `style: default` → `on: true`, `strength: medium`.
  - `style: custom` → `on: true`, `strength: medium`, with `custom` kept as the instructions, now added rather than
    substituted. It is logged once.
- Unknown values read as the default, as CUE's settings already do.

## Welcome window

- Its Keys step follows the cleanup as today (skipped when cleanup runs on this computer), and is skipped when cleanup
  is Off too.
- Its Model step stays the speech model's.

## Constraints

- The owner's design family (merlin-skin): system type at 11 / 12 / 13 px; contrast at least 4.5:1 in both themes;
  amber for warnings; every motion has a cause; one icon set.
  - The segment thumb slides.
  - A page slides in from the side it comes from, and the tray's height eases to the new page.
  - A deleted row folds away.
- English-only code and comments. Keys stay in Windows Credential Manager.
- Today's behaviors stay: local models let go on the cloud and at rest; downloads resume; the cleanup never waits for a
  model except after a rest (`llm::LOAD_WAIT`).
- Tests:
  - UI tests (`app/ui/test`) cover the new rows, the three-part control, strength, the library and both languages.
  - Rust tests cover `level`, the migration, and that instructions are added.

## Left to the builder

- File and module layout, the exact animation timings within the family's rules, and the polish of the copy in both
  languages.
- Whether found models that are not downloaded appear in the cleanup's list (today they do not).
- The Light prompt's wording, measured on the bench against Medium.
