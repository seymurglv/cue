# How CUE works

CUE is a Tauri 2 app: a Rust core that runs in the background and makes every decision, and a small TypeScript UI
(the Drop and its tray cards) rendered by WebView2. The Drop only draws the state the core sends and reports button
clicks back. The dictation logic is a pure state machine, so it is tested without windows, a microphone, a network or
a real clock.

## Crates

- **`cue-core`** (`app/crates/core`): no Windows APIs, no network, no clock (time is passed in).
  - `flow`: the dictation state machine `Flow`. `handle(input, now_ms)` returns a list of `Effect`s.
  - `ui`: `UiState`, `UiPhase`, `ErrorCause`, `TrayKind`, `UiAction`, shared with TypeScript as JSON.
  - `refine`: the system prompt, the user message, timeout and token budget, the sanity check, Markdown stripping,
    list tidying, and the join space before a paste.
  - `audio`: resampling to 16 kHz mono, voice level, the "microphone is silent" detector.
  - `settings`, `usage` (CSV rows, costs, monthly summary), `paths`.
- **`cue-engines`** (`app/crates/engines`): `stt` (session facade, ~100 ms chunks, batch fallback), `soniox`
  (WebSocket stream) and `soniox::batch` (REST, with cleanup of uploads), `claude` (refine request and response),
  `keys` (Windows Credential Manager), `check` (free key probes), `error`. `openrouter` serves only the
  `refine_compare` example.
- **`cue-platform-win`** (`app/crates/platform-win`): `hook` and `chord` (keyboard), `input` (`SendInput`, modifier
  state), `clipboard`, `focus` (UI Automation), `mic` (cpal), `wav`, `dropwin` (window styles), `placement`, `power`.
- **`cue`** (`app/src-tauri`): startup, the `runtime` actor, `ipc`, `drop_window`, `tray` and `trayicon`, `logging`.
- **`app/ui`**: `main.ts` (events, commands, frame loop), `controller.ts` (`DropController`), `trays.ts`,
  `strings.ts`, and `design/` (Drop, tray card and animation code, styles, the Bebas Neue font).
- **`cue-cli`** (`app/crates/cli`) and **`cue-bench`** (`bench`): developer tools.

## Threads

- **Main thread** (Tauri event loop): windows, the tray menu, tray icon redraws.
- **`cue-core`**: the core actor, `Runtime::run`. All events arrive as `CoreEvent`s on one unbounded channel:
  hotkeys, Drop actions, microphone frames, engine results, timers, sleep and lock. It runs on its own thread
  (`block_on`) because it makes short blocking Windows calls (UI Automation up to 150 ms, Credential Manager).
  Network calls and timers are tokio tasks that send their results back as events. A panic while handling one event
  is caught and logged, and the core keeps running.
- **`cue-keyboard`**: the `WH_KEYBOARD_LL` hook with its own message loop at time-critical priority. The handler only
  classifies the key and sends a `HotkeyEvent`; `cue-hotkeys` forwards it to the core. The hook reads two atomics the
  core updates after every event: `ESC_ACTIVE` and `PAUSED`.
- **`cue-focus`**: UI Automation on its own COM thread. Each query has a deadline; a late answer becomes "unknown",
  and queries whose deadline passed while a hung app blocked the thread are skipped.
- **`cue-insert`**: owns all clipboard work, one job at a time. Every job ends within about 7 s; the core also keeps a
  10 s watchdog.
- **`cue-mic`** (cpal) with `cue-mic-events` forwarding frames; `cue-mic-stop` closes a microphone off the core thread.
- **`cue-power`**: a hidden top-level window for `WM_POWERBROADCAST` and session lock notifications.
- **`cue-hit-test`** (~60 Hz while the Drop is shown), **`cue-placement`** (every 0.5 s), **`cue-tray-watch`**
  (every 5 s), **`cue-log-prune`** (daily).

## Dictation flow

```
         Ctrl+Win             Ctrl+Win / release after hold / Done / 9:00     text ready
  Idle ───────────► Starting ──► Recording ───────────────────────► Thinking ───────────► Inserting ──► Done ──► Idle
   ▲                 (380 ms)       │ Esc                              │ Esc                               (1.5 s)
   │                                ▼                                  ▼
   └──────────────────────── Cancelled (Restore, 3.6 s) ◄──────────────┘
      Empty (nothing recognized, 1 s)      Error (cause + Retry; stays until closed, Esc or the next dictation)
```

`Flow` emits effects (`OpenMic`, `StopMic`, `StartStt`, `FinishStt`, `AbortStt`, `TranscribeSaved`, `Refine`,
`AbortRefine`, `Insert`, `PasteLast`, `Ui`, `Shake`, `Tray`, `SetTimer`, `CancelTimer`, `HoldHintShown`, `Finished`)
and the runtime executes them. Late results never reach `Flow`: every timer kind and every transcription or refine
task has a generation number, microphone frames are tagged with the dictation id and dropped after `StopMic`, and a
transcription result is accepted only after `FinishStt` or `TranscribeSaved`. A task that crashes still reports:
transcription as a network error (so Retry is offered), refine as the raw text, paste as "no field".

### Hotkey chord

- The chord is exactly Ctrl + Win (left or right, any order). If Shift or Alt is already held when the pair forms, it
  is a system shortcut and CUE stays out of it.
- Ctrl and Win always pass through to Windows. CUE swallows only `V` while Ctrl + Win are held (paste last; the
  Windows `Ctrl + Win + V` panel does not open) and `Esc` while CUE is starting, recording, thinking, pasting or
  showing an error. A swallowed key's auto-repeat and release are swallowed too.
- Before the Win key-up passes after a chord, CUE sends an unused key (VK `0xE8`), so Windows does not open Start.
- A third key while the starting chord is still held, within 1.5 s of the press (`Ctrl + Win + D`, arrows), means
  a system shortcut: the recording is dropped silently and the key reaches Windows. A later key is taken as a slip
  during a held dictation, and the recording goes on.
- Tap and release starts tap mode: the next chord press finishes. A chord held for 0.6 s starts hold mode: releasing
  finishes, and the first two times a "Press and release" card follows. Presses within 0.3 s of the last are ignored.
  A chord while thinking only makes the Drop shake.
- Injected key events (including CUE's own) pass untouched. Before each key-down the remembered modifiers are
  checked against `GetAsyncKeyState`, so key-ups lost to the secure desktop or UAC do not leave a stuck chord; a
  250 ms timer ends a chord whose keys are gone.
- The hook is installed again after resume from sleep and after unlock, since Windows may remove it.
- Pause for an hour sets `PAUSED`: the hook swallows and masks nothing and sends no events.

### Recording

The default microphone opens on the first press. Frames are resampled to 16 kHz mono `i16` in 20 ms frames. Each
frame goes to the WAV file (`last\current.wav`), to the Soniox stream, and, at most ~30 times a second while
listening, to the Drop as a voice level. Four seconds of digital silence from the start (below −70 dBFS: a muted or
blocked microphone, not a quiet room) stop the recording and show the "Microphone is silent" card, as does a
microphone that fails before any sound. One that fails after sound was heard ends the recording normally. At 8:00
a card says one minute is left; at 9:00 the recording finishes on its own. Locking the session or sleep discards a
recording in progress.

### Transcription: Soniox stream and batch fallback

- The WebSocket to `stt-rt-v5` opens on the first press; audio queues in a channel until it connects. The key goes
  only in the `Authorization` header. The first message is the config (PCM 16 kHz mono, `language_hints`, endpoint
  detection off, the focused app's name as context); audio follows in ~100 ms binary frames, with keepalives in pauses.
- On finish CUE sends 200 ms of silence, `finalize`, waits for the `<fin>` token, then ends the audio. The text is
  built from final tokens.
- If the stream has no result within 5 s of the second press, or fails for any reason other than the key or a cancel,
  the saved WAV goes to the batch model `stt-async-v5`: upload, create a transcription, poll every 400 ms, fetch the
  text, delete the transcription and the file. Deletions that fail (still processing, network, rate limit) are
  retried in the background for about 15 minutes.
- Retry after an error transcribes the saved WAV through the same batch path.
- A key, balance or limit problem (401 / 402 / 403 / 429, or no key) becomes the "Soniox key problem" error; anything
  else becomes "No connection". Both keep the recording for Retry.

### Refine (cleanup)

- At the start of recording and again at the second press, the runtime warms up a TLS connection to the Anthropic API
  (a key-less `HEAD`, at most every 20 s, at once after wake). Right after the second press it also asks `cue-focus`
  for the focused app and up to `refine.context_chars` characters before the caret, in parallel with transcription.
  Password fields are never read; the app name comes from the executable's file description, never the window title.
- The request: the system prompt as a cached block, a user message with `<app>`, `<before_cursor>` and `<transcript>`
  tags (closing tags inside the data are defused), effort `low`, thinking disabled where the model allows it, and a
  JSON schema that forces `{"text": …}`. Timeout: 3 s plus 1 s per 100 words. When the app is unknown (neither
  the query at the start of recording nor the one after the second press named it), the request carries
  `<app>unknown app</app>`; Soniox gets no app context in that case.
- The answer is used only when it is a complete `{"text": …}` object: a refusal, `max_tokens`, a timeout, a network
  error or a bad response all mean the raw transcript is pasted with the normal "Done". Explicit Markdown the speaker
  did not dictate is stripped, and stray commas and semicolons at the end of list items are removed.
- **Sanity check** on what would be pasted: lengths are compared in characters without numbers (digits and number
  words), and only for transcripts of at least four words with two or more that are not numbers; the ratio must stay
  within `min_ratio`…`max_ratio` (0.35–1.25). At least `min_overlap` (70 %) of the result's non-number words must
  appear in the transcript. An empty result always fails. A failed check pastes the raw transcript.
- A key, balance or limit problem at Anthropic pastes the raw transcript and then shows "Claude key problem" without
  Retry.

### Paste

The same path serves dictation and `Ctrl + Win + V`:

1. One short focus query: is there a field, which app, the last few characters before the caret.
2. Trim the text's edges, then add a join space only when the caret sits right after a word.
3. In a terminal (Windows Terminal, PowerShell, Command Prompt, conhost, mintty, WSL) every line break becomes a
   space, so pasting never presses Enter. If the query was late, the last known app or the app at recording start
   decides.
4. On `cue-insert`: wait until Ctrl and Win are physically released (up to 5 s), open the clipboard (up to 10 tries,
   50 ms apart), snapshot RTF, HTML, PNG, DIB, file list and text, put the text with the
   `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0` and
   `CanUploadToCloudClipboard = 0` markers, press Ctrl+V with `SendInput` (virtual key and scan code, so any keyboard
   layout works), and report "pasted". After 0.5 s the snapshot is restored, unless the clipboard sequence number
   shows someone else changed it.

**No field.** CUE says "nowhere to paste" only when it is sure: the focus is on the desktop, in File Explorer's file
list or folder tree, or nowhere at all. Apps that expose nothing to UI Automation still get the paste. The text is
then put in the clipboard normally, without markers, and the "Nowhere to paste" card offers Copy. The same happens
if the keys are not released in time, if Ctrl+V is not delivered (an elevated window), or if the 10 s watchdog fires.

## The Drop and tray cards

The Drop window is transparent, borderless, always on top, absent from the taskbar and Alt+Tab, and never takes focus:
`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` are enforced by a window subclass, because the windowing library rebuilds the
extended style on every flag change. The window is larger than the capsule; the UI reports the capsule and card
rectangles (`drop_hit_rects`) and `cue-hit-test` lets the mouse through everywhere else. `cue-placement` keeps the
Drop centered 12 px above the bottom of the work area on the monitor of the foreground window, follows DPI changes,
and hides the idle strip over full-screen apps (`SHQueryUserNotificationState`) and while paused.

Core → window events: `cue://state` (phase, error cause, retry, paused), `cue://level`, `cue://tray` (card kind or
none), `cue://shake`, `cue://idle-hidden`. Window → core commands: `drop_action` (`cancel`, `finish`, `undo`, `retry`,
`dismiss`, `trayAction`, `trayClose`), `drop_hit_rects`, and `drop_ready`, after which the core resends the current
state. No dictation text crosses this boundary.

Phases: `idle` (a thin strip; on hover the mark and the hotkey), `start`, `listen` (voice bars and no timer: the mark
grows from 24 to 28 px and the Drop stays a circle, 44 px instead of 40; cancel and done buttons on hover), `think`
(three dots), `done` (a check, gone after 1.5 s), `empty`, `cancel` (Restore with a 3.6 s ring), `error` (amber "!",
the cause on hover, Retry when available). The frame loop sleeps when nothing moves; with reduced motion turned on in
Windows, states change instantly.

Tray cards grow out of the Drop, one at a time; a new card replaces the old one, a new recording closes it, and hovering
pauses the countdown ring:

| Card | Button | Closes |
|---|---|---|
| Nowhere to paste | Copy → Copied | after 9 s |
| Microphone is silent | Settings (opens `ms-settings:privacy-microphone`) | manually |
| Press and release | Got it | after 7 s |
| One minute left | — | after 5 s |

A card button arrives as `trayAction`; the core acts on the last card it sent.

The interface is in English, or in Russian when the Windows display language is Russian; `ui.language` in
`settings.json` (`en` or `ru`) overrides Windows. The core makes that choice once at startup, so the tray menu and
the Drop always match.

## Tray icon

A monochrome caret mark drawn in code, light or dark to match the taskbar and sized for its DPI (both rechecked every
5 s). It shows a red lamp while recording and turns pale while paused. The menu:

- **Pause for an hour** (checkmark): a recording in progress is discarded, hotkeys and the idle strip are off, and the
  pause ends by itself after an hour.
- **Start at sign-in** (checkmark): the Windows `Run` entry, re-read every 5 s in case it was changed elsewhere.
- **Open log**: opens the log folder.
- **Quit**.

A second launch of `cue.exe` does nothing.

## Settings

`%APPDATA%\CUE\settings.json` is read once at startup. Missing fields take defaults, unknown fields are ignored, a
UTF-8 BOM is accepted, and any engine name reads as `soniox`. If the file cannot be read, CUE runs on defaults and
never writes it during that session. Writes (the hold-hint counter) are atomic: a temp file in the same folder, then a
rename. Default prices are not written to the file, so price fixes in code reach existing installs; once any price is
edited, the whole block is written. `ui.language` (`auto`, `en`, `ru`, in any case and with spaces trimmed; any other
value reads as `auto`) is likewise written only when it is not `auto`. Model names are settings, so a new model
version needs no code change.

## Usage accounting

An attempt is a recording and its transcription; Retry starts a second attempt of the same dictation, and each gets
its own row in `%LOCALAPPDATA%\CUE\usage.csv`:
`ts, audio_ms, stt_engine, stt_path, stt_usd, refine_model, input_tokens, cache_read_tokens, cache_write_tokens,
output_tokens, refine_usd, latency_ms, outcome`. `stt_path` is `stream`, `stream+batch` or `batch`; outcomes are
`inserted`, `no_field`, `cancelled`, `empty`, `failed`, `discarded` (written only when audio ran past 380 ms).
Latency runs from the second press to Ctrl+V (from Retry or Restore when used); the target for a ~30-word phrase is
1.5 s or less. Rows that fail to write, for example
while the file is open in Excel, wait in memory and go out before the next one. `cue-cli usage` sums a UTC month:
dictations are `inserted` plus `no_field` rows, average latency counts only `inserted` rows, and a separate average
covers recordings of 20 s or less.

## Logging and privacy rules

- Logs go to `%LOCALAPPDATA%\CUE\logs\cue.log.YYYY-MM-DD` through a non-blocking writer at `info` level; old files are
  pruned at startup and daily (7 days, 10 MB).
- Logged: events, per-stage timings (`stt_ms`, `refine_ms`, `insert_ms`, `latency_ms`), outcomes and error kinds.
  Never logged: dictation text, text before the caret, audio, API keys, response bodies.
- WebSocket handshake tracing is switched off by a fixed filter, since it would include the `Authorization` header.
- The panic hook is installed first and records only the thread and code location, never the message, which could
  contain dictated text. Debug output of focus results and microphone frames shows only lengths.
- Keys are read from Credential Manager on use, appear in the CLI only masked, and are cut out of engine error texts.
- Soniox uploads are deleted after use. The last dictation (`last\`) and bench recordings stay on this computer only.

## Bench

`cue-bench run` reads recordings saved by the app (`%LOCALAPPDATA%\CUE\bench\recordings`, a WAV plus a JSON with the
app name), and for each one runs the Soniox stream at real-time pace and the batch model at the same time, then
refines the stream text (or the batch text if the stream failed) with `refine.model` and every model in the price
list. It uses the app's own code paths, so the request, sanity check and timeouts match. One warm-up call per model
is excluded from the report but counted in spend. A `--max-usd` budget stops new paid calls; `Ctrl+C` finishes the
work already sent, and before exiting the bench waits for Soniox cleanup. `results.json` and `report.html` are
rewritten after each recording in `%LOCALAPPDATA%\CUE\bench\runs\<timestamp>`. The report is a single HTML page with
blind A/B pairs for transcription and for refine, scores, latency and cost per dictation, and saves the answers as
`choices.json`.
