# CUE

CUE is a voice-dictation app for Windows 11. Press `Ctrl + Win`, speak, and the cleaned-up text appears at the caret
in whatever app you are typing in. Speech is transcribed in real time by Soniox, then lightly edited by Claude Haiku 5.5
(filler words, self-corrections, punctuation, lists, numbers) while keeping your own words, tone and mix of languages.
Add `Shift` (`Ctrl + Win + Shift`) and the text is pasted translated: into English, and English into your home
language (Russian, say); any pair of the languages Soniox recognizes can be set. It runs quietly in the background:
a small indicator at the bottom of the screen (the Drop; drag it to the top or to either side if you prefer) shows
what CUE is doing, and a tray icon next to the clock holds a short menu. This first version is built for personal use
with your own Soniox and Anthropic API keys.

## Features

- **Two chords.** `Ctrl + Win` (left or right keys, any order) starts dictation; `Ctrl + Win + Shift` (any order too)
  starts a translated one. Tap and release: CUE records until you press the chord again (with or without `Shift`; the
  mode no longer changes then). Hold for 0.6 s or longer: recording ends when you let go. While the starting chord is
  still held, adding `Shift` turns the dictation into a translation and the Drop changes its icon at once; once you
  let go, the mode stays until the recording ends.
  `Ctrl + Win` with `Shift` and `Alt` together is the Office key, and `Ctrl + Win + Alt` is kept for the "Ask" mode
  of a later version: CUE leaves both to Windows.
- **Translation** (`Ctrl + Win + Shift`). Speech that is not in your target language (`translate.target`, English
  by default) is translated into it; speech in the target language goes to your home language (`translate.home`, by
  default the Windows display language). The speech language is the one most of your words were in, so
  "Открой Telegram" counts as Russian. The cleanup rules stay as in dictation (self-corrections, lists, numbers, one
  line in terminals), and names from `dictionary.terms`, product names and code are kept as written. If the
  translation fails (no network, a timeout, or an answer that does not look like a translation), nothing is pasted:
  the Drop shows "Couldn't translate", and **Retry** translates the saved text again.
- **Text at the caret in any app**, pasted through the clipboard. Your previous clipboard content comes back about
  half a second later, and dictated text is kept out of Windows clipboard history and the cloud clipboard.
- **Russian, English, and both mixed** in one sentence.
- **Light-touch cleanup:** fillers, stutters and abandoned starts are removed, "Wednesday, no, Thursday" becomes
  "Thursday", punctuation and paragraphs are added, lists go on separate lines, numbers are written as digits.
  The tone adapts to the target app; in terminals the result always stays on one line, so nothing runs by itself.
- **Safe fallbacks:** if cleanup is slow, refused or looks wrong, the raw transcript is pasted instead. If the
  Soniox stream drops, the saved recording is transcribed as a whole. If there is no network, the recording is kept
  and **Retry** sends it again.
- **`Esc`** cancels; **Restore** (for 3.6 s) brings the dictation back. **`Ctrl + Win + V`** pastes the last result
  again, a translation included.
- **Nowhere to paste?** When the focus is clearly not a text field (desktop, File Explorer's file list), the text
  stays in the clipboard and a card offers to copy it.
- **Recording limit:** at 8:00 a card says one minute is left, at 9:00 recording ends on its own and the text is pasted
  as usual. The Drop shows no timer while recording, only the voice bars.
- **The Drop stays where you put it.** Hover over the thin strip to open it, then drag it: when you let go it snaps
  to the nearest of four places: the bottom (the default) or the top edge, centered, or the left or the right edge,
  halfway up. The place is remembered (`drop.position` in `settings.json`). A plain click without moving does what it
  always did, and dragging never takes the focus from the app you are typing in. The Drop can be dragged only while
  idle, and it always stays on top of ordinary windows.
- **Tray icon menu:** Pause for an hour, Start at sign-in, Open log, Quit.
- **Interface language:** English, or Russian when the Windows display language is Russian; `ui.language` in
  `settings.json` can fix it to either.
- **Usage accounting:** every dictation's cost is recorded locally, with its mode; `cue-cli usage` shows a monthly
  summary.
- **Comparison bench** (a developer tool): runs your saved recordings through Soniox stream and batch and through
  several Claude models, and builds a blind A/B report.

Not in this version: the "Ask" mode (`Ctrl + Win + Alt`), a settings window, history, a custom vocabulary, an
installer, macOS.

## Requirements

- Windows 11 (x64).
- Rust stable with the MSVC toolchain, 1.90 or newer (edition 2024):
  `rustup default stable-x86_64-pc-windows-msvc`.
- Visual Studio Build Tools with the "Desktop development with C++" workload.
- Microsoft Edge WebView2 Runtime (included with Windows 11).
- Node.js 24 or newer, with npm (`"node": ">=24"` in `app/package.json`).
- A Soniox API key and an Anthropic API key.

## API keys

Keys live in Windows Credential Manager under the service name `CUE`. They are never written to files, never
printed in full and never logged. Enter them once with the command-line tool (input is hidden):

```powershell
cargo run -p cue-cli -- keys set soniox
cargo run -p cue-cli -- keys set anthropic
cargo run -p cue-cli -- keys show     # masked values, e.g. sk-ant…f3a9
cargo run -p cue-cli -- keys check    # a free test request per key
```

`keys set openrouter` is also accepted; that key is used only by the `refine_compare` developer example, not by the app.

## Build and run

Development build, with live reload of the Drop UI (`npm run dev` alone serves `ui/preview.html` too: every state
of the Drop at each of the four places, side by side):

```powershell
cd app
npm install
npm run tauri dev
```

Release build:

```powershell
cd app
npx tauri build --no-bundle
```

The result is `target\release\cue.exe` at the repository root. Run it directly; use **Start at sign-in** in the tray
menu to launch it with Windows. A second launch does nothing: only one instance runs.

Tests and lints:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd app; npm test; npm run build
```

The tests need no microphone, network or API keys (network code is tested against a local fake server).

## Settings

`%APPDATA%\CUE\settings.json`. CUE reads it at startup, so restart CUE after editing it. A missing file or missing
fields mean defaults; unknown fields are ignored. If the file cannot be read or parsed, CUE runs on defaults for that
session and does not overwrite it. The defaults:

```json
{
  "stt": {
    "engine": "soniox",
    "soniox_model": "stt-rt-v5",
    "soniox_async_model": "stt-async-v5",
    "language_hints": ["ru", "en"]
  },
  "refine": {
    "model": "claude-haiku-5-5",
    "context_chars": 500,
    "min_ratio": 0.35,
    "max_ratio": 1.25,
    "min_overlap": 0.7
  },
  "hold_hints_left": 2,
  "bench": { "save_recordings": false }
}
```

- `stt.soniox_model` / `soniox_async_model`: the real-time model and the batch model used for fallback and Retry.
  `language_hints` are hints, not limits. `engine` has a single value today, `soniox`.
- `refine.model`: the Claude model for cleanup. `context_chars`: how much text before the caret is sent as context
  (0 turns it off). `min_ratio`, `max_ratio`, `min_overlap`: the sanity check (see
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)).
- `hold_hints_left`: how many more times the "Press and release" card appears after a held chord.
- `bench.save_recordings`: copy each successful dictation's audio and app name for the bench.
- `ui.language`: the language of the tray menu and the Drop. `"auto"` (the default) follows the Windows display
  language, `"en"` and `"ru"` fix it (case and surrounding spaces do not matter), any other value means `"auto"`. The
  value is kept in the file only when it is `"en"` or `"ru"`. Example: `"ui": { "language": "ru" }`.
- `drop.position`: where the Drop sits: `"bottom"` (the default; centered, 12 px above the taskbar), `"top"`
  (centered, 12 px below the top of the screen), `"left"` or `"right"` (halfway up, 12 px from the edge). Dragging the
  Drop sets it. Case and surrounding spaces do not matter; any other value means `"bottom"`. The value is kept in the
  file only when it is not `"bottom"`. Example: `"drop": { "position": "right" }`.
- `dictionary.terms`: names and terms you often say, written as they should appear. Example:
  `"dictionary": { "terms": ["Soniox", "Claude Haiku", "GitHub"] }`. When the transcript clearly means one of them but
  spells it differently, splits it or mixes alphabets (`Со niox`, `гитхаб`, `git hub`), the cleanup writes it as
  listed. The list affects only the Claude cleanup, where it rides along with the cached system prompt and costs
  almost nothing. It is not sent to Soniox as recognition context: Soniox bills context tokens on every dictation,
  which would roughly double the cost of recognition. Entries are trimmed; empty ones, ones with a line break, ones
  longer than 60 characters and exact repeats (case-sensitive) are skipped, and only the first 200 are used. A value
  that is not a list reads as an empty list. The block is kept in the file only when the list is not empty.
- `translate`: the language pair of `Ctrl + Win + Shift`, as the ISO 639-1 codes of the 60 languages Soniox
  recognizes (`en`, `ru`, `de`, `zh`, …). Case and surrounding spaces do not matter, and a locale name reads as its
  language (`"ru-RU"` is `ru`).
  - `target` (default `en`): speech in any other language is translated into it.
  - `home`: speech in the target language is translated into it. By default the Windows display language, else the
    first `stt.language_hints` entry; one equal to the target is skipped. With no home language, speech in the target
    language is only cleaned up.
  - `speech`: `"auto"` (the default) lets Soniox name the language of every word, and the language of most words
    decides the direction. A code fixes the speech language and goes to Soniox as the only language hint of a
    translation (dictation keeps `stt.language_hints`).

  An unknown code reads as the default, and defaults are not written to the file. Example for a Russian speaker who
  writes to German colleagues: `"translate": { "target": "de", "home": "ru" }`. Russian (or English) speech is pasted
  in German, German speech in Russian.
- `pricing` (not written unless you change it): `soniox_rt_per_hour`, `soniox_async_per_hour`, and a `claude` list
  of `{model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok}` in USD. Defaults:
  Soniox $0.12 / hour (stream) and $0.10 / hour (batch); Haiku 5.5 $0.10 / $0.50, Sonnet 5.5 $2 / $10,
  Opus 5.5 $4 / $20 per million input / output tokens. `web_search_usd` ($0.01) prices a web search of the "Ask"
  mode to come.

Start at sign-in is not a setting: it is the Windows `Run` registry entry, toggled from the tray menu.

## Local data

Everything else lives in `%LOCALAPPDATA%\CUE`:

| Path | What |
|---|---|
| `last\audio.wav`, `last\result.txt` | The last successful dictation (16 kHz mono WAV and the final text), for `Ctrl + Win + V` and Copy. Replaced by the next one. |
| `last\current.wav` | Audio of the dictation in progress; after a failure it waits there for Retry. Deleted when a dictation is cancelled or comes out empty. |
| `logs\cue.log.YYYY-MM-DD` | Daily log: events, per-stage timings, error kinds. No dictation text, no audio, no keys. Kept 7 days and 10 MB at most. **Open log** in the tray menu opens this folder. |
| `usage.csv` | One row per attempt: time (UTC), audio length, transcription path, cost, refine model, tokens, latency, outcome, mode. A refine cache warm-up gets a row of its own. No text. |
| `bench\recordings\` | Recordings for the bench, only when `bench.save_recordings` is `true`. |
| `bench\runs\<timestamp>\` | Bench runs: `results.json` and `report.html`. |

WebView2 keeps the Drop window's browser profile (cache and storage) separately, in
`%LOCALAPPDATA%\app.cue.dictation\EBWebView` (`app.cue.dictation` is the app identifier in `tauri.conf.json`).

## Costs

```powershell
cargo run -p cue-cli -- usage            # current month (UTC)
cargo run -p cue-cli -- usage 2026-10
```

The summary shows the number of dictations, minutes of audio, transcription and refine cost, the cost per dictation
and per hour of audio, the average latency from the second key press to the paste (overall and for recordings of
20 s or less), and the rows and dollars of each mode. Prices come from `settings.json`. A Retry is a second paid
attempt and gets its own row; the Retry of a failed translation translates the saved text again, so its row has no
audio.

The last three columns of `usage.csv` are `mode`, `web_searches` and `search_usd`. `mode` is `dictate`, `translate`
or `warmup`. A `warmup` row is a refine cache warm-up (see Privacy): paid tokens, no audio, outcome `discarded`; it
counts in the cost, not as a dictation. `web_searches` and `search_usd` belong to the "Ask" mode to come and stay 0.
Rows written before these columns existed read as dictation, and the header of an existing file is not rewritten:
it keeps the 13 old names while new rows have 16 fields.

`usage.csv` is comma-separated. In Excel with a locale whose list separator is not a comma (Russian, for example), do not
open the file directly; use **Data → From Text/CSV** and choose the comma delimiter.

## Comparison bench

The bench compares transcription and cleanup models on your own speech and shows how the sanity check behaves on
it. It does not tune the thresholds.

1. Set `"bench": { "save_recordings": true }` in `settings.json`, restart CUE and dictate as usual.
2. Run it (needs the Soniox and Anthropic keys):

   ```powershell
   cargo run -p cue-bench -- run [--limit N] [--max-usd X]
   ```

   Each recording is transcribed twice in parallel: Soniox stream at real-time pace and Soniox batch. The transcript
   is then cleaned by `refine.model` and every model in the `pricing.claude` list, with the same request (including
   `dictionary.terms`), sanity check and timeout as the app. `--max-usd` (default $1) stops new paid calls once
   reached. The first `Ctrl+C` finishes the work already sent and stops; a second one exits at once. Results are saved
   after every recording.
3. Open `report.html` from the run folder. It shows blind A/B pairs for transcription and for refine, then the scores,
   latency and cost per dictation. **Save choices** downloads your answers as `choices.json`.

`cargo run -p cue-bench -- report <run folder>` rebuilds `report.html` from `results.json`.

## Privacy

- **Soniox** receives the audio of each dictation while you record, plus the name of the app you are dictating into
  as context. On fallback or Retry, the saved WAV is uploaded to the batch API; CUE deletes the uploaded file and its
  transcription right after use and retries the deletion in the background for about 15 minutes if it fails.
- **Anthropic** receives the transcript, the app name, up to `refine.context_chars` (500) characters of text
  before the caret, and your `dictionary.terms`. Nothing is read from password fields, and the window title is never
  sent. A translation goes in the same cleanup request, with the target language added. While you record, CUE also
  sends an empty `HEAD` request without a key to open the connection early, so cleanup starts faster. When a
  dictation starts four minutes or more after the last cleanup, CUE sends a cache warm-up: the cleanup request with
  your `dictionary.terms` and a one-character placeholder instead of a transcript, without the app name or any text
  before the caret, so the first cleanup after a pause reads the prompt from Anthropic's cache.
- The microphone is open only while CUE is recording.
- **Stays on this computer:** the last dictation's audio and text, recordings saved for the bench, settings, logs and
  usage rows. Logs and `usage.csv` never contain dictation text; a crash records only the thread and code location,
  not the panic message.
- Check each provider's own data-retention policy for the audio and text they receive.

## Known limitations

- Windows does not let a normal app intercept keys in, or paste into, windows running as administrator; CUE does not
  react there. The lock screen and UAC prompts are out of reach too.
- Clipboard restore covers text, HTML, RTF, images (PNG and DIB) and file lists; rarer formats are not restored.
- The terminal one-line rule recognizes Windows Terminal, PowerShell, Command Prompt, conhost, mintty and WSL by
  name; the VS Code integrated terminal looks like the editor.
- A translation is checked by length and by script, not by meaning. Between two languages written in the same script
  (English and German) only the length is checked, so untranslated text can pass there; other limits are in
  [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Repository layout

```
Cargo.toml                Cargo workspace
app/
  package.json            front-end tooling: Vite, TypeScript, Vitest, Tauri CLI
  crates/core/            cue-core: dictation state machine, settings, refine and translation rules, audio helpers,
                          usage
  crates/engines/         cue-engines: Soniox stream and batch, Claude refine and translation, API keys and checks
  crates/platform-win/    cue-platform-win: keyboard hook, clipboard paste, focus via UI Automation,
                          microphone, Drop window styles and placement, sleep and lock events
  crates/cli/             cue-cli: API keys and monthly usage
  src-tauri/              the cue app: runtime, Drop window, tray icon, logging
  ui/                     the Drop and its tray cards: TypeScript + Vite, no framework
bench/                    cue-bench: comparison bench
docs/ARCHITECTURE.md      how CUE works inside
```

`app/crates/platform-win/examples` and `app/crates/engines/examples` hold small probes for checking the keyboard
hook, microphone, focus, paste, sleep events, the Drop's z-order (`zorder_probe`, run while CUE is running), Soniox
and refine on a real machine
(`cargo run -p <crate> --example <name>`). `refine_compare` (in `cue-engines`) runs a phrase file of your own through
cleanup models with the app's prompt and checks; a phrase with `"to": "en"` is translated on the app's own path
instead (see the example's header).

## License

All rights reserved. No license has been chosen yet.

The Bebas Neue font in `app/ui/src/design/fonts` is distributed under the SIL Open Font License 1.1 (see `OFL.txt`).
