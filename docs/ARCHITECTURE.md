# How CUE works

CUE is a Tauri 2 app: a Rust core that runs in the background and makes every decision, and a small TypeScript UI (the
Drop with its tray cards, the "Ask" card at the selection, the main window with the history and the settings, and the
welcome window of the first start) rendered by WebView2. The Drop and the card only draw the state the core sends and
report button clicks back; the main window reads the history and changes the settings through commands, the welcome
window saves the key and picks the microphone the same way, and the core stays the only one that writes the history and
the settings. Speech is recognized on this computer by a speech model the core downloads once and keeps loaded (see
Recognition); the network is needed only for that download and for Anthropic. The dictation logic is a pure state
machine, so it is tested without windows, a microphone, a model, a network or a real clock.

## Crates

- **`cue-core`** (`app/crates/core`): no Windows APIs, no network, no clock (time is passed in).
  - `flow`: the dictation state machine `Flow`, one for every mode. `handle(input, now_ms)` returns a list of
    `Effect`s.
  - `ui`: `UiState`, `UiPhase`, `Mode`, `ErrorCause`, `TrayKind`, `UiAction`, shared with TypeScript as JSON.
  - `refine`: the system prompt, the dictionary block, the user message, timeout and token budget, the sanity check,
    Markdown stripping, list tidying, and the join space before a paste.
  - `connections`: the cleanup's ten connections (the Messages API, OpenRouter, eight OpenAI-compatible services, two of
    them local servers) with their models (the first recommended), prices, and what each service needs in a request.
  - `translate`: the 60 languages a translation can go to, the translation direction, the instruction block, the
    timeout and the translation check.
  - `speech`: the rules of recognition: the alphabet of a dictation, a leak of the other alphabet, the speech language
    of a translation, and where a long recording is cut into pieces.
  - `model`: the speech model's files (`Manifest`: a Hugging Face repository at a fixed commit, each file's size and
    SHA-256; `ULTRA`, the one CUE ships) and `ModelState`, the model's state as the windows show it.
  - `ask`: the "Ask" instructions and user message, the conversation's turns and their text for an AI
    (`AskTurn`, `agent_messages`), the selection limit, where a written text goes, the buttons of an answer, its
    sources, the plain text for pasting, and `AnswerView`, the answer as the windows draw it.
  - `audio`: resampling to 16 kHz mono, voice level, the "microphone is silent" detector.
  - `history`: the history's day files (`Entry`, `append`, the pages, `search`, `find`, `prune` by the retention
    period, `clear`) and the names and deletions of its recordings folder (`audio_file`, `audio_snapshot`,
    `audio_to_delete`, `prune_audio`, `sweep_audio_temps`, `clear_audio`).
  - `corrections`: which replaced words could go into the dictionary (`candidates`), how a watched field's text
    becomes the corrected span (`Watch`), and the one suggestion the Drop offers (`Offers`).
  - `habits`: the corrections that repeat (see Habits): the kinds and their forms (`Form`, `correction`), the counts, the
    habits and the rule's request and template (`Habits`, `Habit`, `Draft`), and the check of a result (`marks`).
  - `settings` (with `SettingChange`, one change from the main window), `usage` (CSV rows, costs, monthly summary),
    `paths`.
- **`cue-engines`** (`app/crates/engines`): `stt` (the session facade over the model on this computer, and Retry's
  recognition of a saved WAV), `parakeet` (the model: `mel`, the log-mel features; `vocab`, the vocabulary and the
  masks of one alphabet; `tdt`, TDT greedy decoding; `model`, the ONNX Runtime sessions and `Recognizer`), `download`
  (the model's files: resumed, checked, renamed), `claude` (refine and translation requests and responses, the cache
  warm-up), `ask` (an "Ask" question's steps, end and deadlines; `agent::ask` asks the route),
  `keys` (Windows Credential Manager, and the deletion of the Soniox key of earlier versions), `check` (free key
  probes), `habit` (the rule request of a habit just learned), `error`. `openrouter` is the cleanup's OpenRouter
  connection (`refine`, `translate`: the same system blocks, checks and fallbacks as `claude`, each model's profile in
  `model_profile`) and holds the server-sent-events reader that "Ask", `chat` and the `refine_compare` example use too;
  `chat` is the cleanup over any other OpenAI-compatible service (OpenAI, Gemini, DeepSeek, Mistral, Groq, xAI, Ollama,
  LM Studio), each service's needs read from `cue_core::connections`.
- **`cue-platform-win`** (`app/crates/platform-win`): `hook` and `chord` (keyboard, and the mouse while a Shift that
  switched the mode is held), `input` (`SendInput`, modifier state), `clipboard` (paste, and the `Ctrl + C` fallback
  of "Ask"), `focus` (UI Automation: the field, the text before the caret, the selection; and the window under the
  main window, for Paste again), `selection` (when to copy the selection, where it is on screen), `cardplace` (where
  the "Ask" card goes), `fieldwatch` (the field pasted into, read again to notice a correction), `mic` (cpal; a
  pinned microphone found by its name; whether Windows denies CUE the microphone), `wav`, `opus` and `ogg` (a WAV
  into Ogg Opus: libopus 1.3.1 in its Rust translation `unsafe-libopus`, the pages written here), `dropwin` (window
  styles, moves, the z-order guard), `placement` (the four places, the nearest place, monitors), `power`.
- **`cue`** (`app/src-tauri`): startup, the `runtime` actor (with `runtime::ask`, the conversation of an open answer,
  `runtime::recordings`, the history's recordings, and `runtime::model`, the speech model's download, load and state),
  `ipc`, `drop_window`, `card_window`, `main_window`, `welcome_window` and `commands` (the main and welcome windows and
  their commands), `micpreview` (the microphone level of the settings and of the welcome window), `tray` and
  `trayicon`, `logging`; `nsis/` holds the installer's script and hooks.
- **`app/ui`**: `main.ts` (events, commands, frame loop), `controller.ts` (`DropController`), `trays.ts`, `answer.ts`
  (the "Ask" answer, one view for the tray and the card), `markdown.ts` (its safe Markdown), `card.ts` and `cardPage.ts`
  (the card window), `strings.ts`, `window.html` with `window/` (the main window: `page.ts`, `history.ts`, `said.ts`
  (how an entry was said, and its player), `habits.ts` (the Habits section), `settings.ts`, `text.ts`, `icons.ts`,
  `api.ts`, and `model.ts`, `keys.ts` and `mic.ts`, the speech model's block and the key and microphone rows the welcome
  window shares), `welcome.html` with `welcome/` (`page.ts`, `steps.ts`, `text.ts`, `welcome.css`), and `design/` (Drop,
  tray card, answer and animation code, styles including the main and welcome windows' `window.css`, the Bebas Neue and
  Montserrat ExtraBold fonts).
- **`cue-cli`** (`app/crates/cli`: keys, usage and `leak-scan`, see Installer and release build) and **`cue-bench`**
  (`bench`): developer tools. `scripts/build-release.ps1` builds the installer.

## Threads

- **Main thread** (Tauri event loop): windows, the tray menu, tray icon redraws, the `mic_preview` command (so a
  preview's start and stop run in order with its window's end) and the welcome window's `welcome_step` (so its steps
  reach the core in order).
- **`cue-core`**: the core actor, `Runtime::run`. All events arrive as `CoreEvent`s on one unbounded channel: hotkeys,
  Drop actions, microphone frames, engine results, the speech model's download and load steps, timers, sleep and lock,
  the main and welcome windows' settings and model requests, the main window's history requests and the welcome window's
  hold of the hotkey. It runs on its own thread (`block_on`) because it makes short blocking Windows calls (UI
  Automation up to 150 ms, Credential Manager). Network calls and timers are tokio tasks that send their results back as
  events, and so are the model's download and the recognition sessions; recognition itself, the model's load and the
  check of its files run on tokio's blocking threads, and one recognition uses 4 ONNX Runtime threads. A panic while
  handling one event is caught and logged, and the core keeps running.
- **`cue-keyboard`**: the `WH_KEYBOARD_LL` hook (and, while a Shift that toggled the mode is held, `WH_MOUSE_LL`: see
  Hotkey chord) with its own message loop at time-critical priority. The handler only classifies the key and sends a
  `HotkeyEvent`; `cue-hotkeys` forwards it to the core. The hook reads three atomics
  the core updates as soon as the flow has handled an event: `ESC_ACTIVE`, `RECORDING` and `PAUSED`; and
  `ASK_ENABLED`, set once at startup from `ask.enabled`.
- **`cue-focus`**: UI Automation on its own COM thread. Each query has a deadline; a late answer becomes "unknown",
  and queries whose deadline passed while a hung app blocked the thread are skipped.
- **`cue-insert`**: owns all clipboard work, one job at a time: pastes, texts left in the clipboard and the `Ctrl + C`
  fallback of "Ask". Every job ends within about 7 s ("Insert below" within about 9 s); the core also keeps a 10 s
  watchdog. A late copy of the fallback is watched on `cue-late-copy`, and its removal from the Win+V history runs on
  `cue-clip-history` (WinRT); both are short-lived.
- **`cue-field`**: after a paste, UI Automation on a COM thread of its own reads the field pasted into every 0.4 s, for
  60 s at most, to notice a correction (see Corrections). One at a time: a new dictation, a new watch or "Learn from
  me" turned off ends the one running. It sends the core a `CoreEvent::Corrected` when it is over.
- **`cue-card`**: moves, shows and hides the "Ask" card window.
- **`cue-mic`** (cpal; it also lists the devices to find a pinned microphone) with `cue-mic-events` forwarding
  frames; `cue-mic-stop` closes a microphone off the core thread (and off the main thread for the preview).
- **`cue-mic-preview`**: while the main window's Microphone settings or the welcome window's Microphone step are
  shown, sends that window the level of the preview's own `cue-mic` (see Main window).
- **`cue-recordings`**: encodes the history's recordings, one at a time, and in the same queue deletes those past
  their period and the folders a clear set aside (see Recordings).
- **`cue-main-window`** and **`cue-welcome-window`**: each builds its window and ends; a WebView built on the main
  thread from a tray or menu handler deadlocks (wry#583). The windows' other commands are async: their file,
  device-list, registry and Credential Manager work runs on tokio's blocking threads.
- **`cue-power`**: a hidden top-level window for `WM_POWERBROADCAST` and session lock notifications.
- **`cue-hit-test`** (one for the Drop and one for the card, ~60 Hz while the window is shown), **`cue-placement`**
  (every 0.5 s; every 8 ms while the Drop is dragged or snapping into place), **`cue-tray-watch`** (every 5 s),
  **`cue-log-prune`** (daily), **`cue-open-source`** (a short thread for each click on a source),
  **`cue-open-link`** (a short thread for each link of the welcome window).

## Dictation flow

```
         Ctrl+Win             Ctrl+Win / release after hold / Done / 9:00     text ready
  Idle ───────────► Starting ──► Recording ───────────────────────► Thinking ───────────► Inserting ──► Done ──► Idle
   ▲                 (380 ms)       │ Esc                              │ Esc                               (1.5 s)
   │                                ▼                                  ▼
   └──────────────────────── Cancelled (Restore, 3.6 s) ◄──────────────┘
      Empty (nothing recognized, 1 s)      Error (cause + Retry; stays until closed, Esc or the next dictation)

  "Ask": Thinking ──(first real step, or the end)──► Answer ──(Esc, the cross)──► Idle
                                                       │ ▲
                                Ctrl+Win: a follow-up  ▼ │  its answer shows, or it is dropped
                                                 Starting → Recording → Thinking
```

`Flow` emits effects (`OpenMic`, `StopMic`, `StartStt`, `FinishStt`, `AbortStt`, `TranscribeSaved`, `Refine`,
`RefineAgain`, `AbortRefine`, `Ask`, `AskAgain`, `AbortAsk`, `AskPaste`, `CloseAnswer`, `Insert`, `PasteLast`, `Ui`,
`Shake`, `Tray`, `SetTimer`, `CancelTimer`, `HoldHintShown`, `Finished`) and the runtime executes them. Late results
never reach `Flow`: every timer kind and every transcription, refine or "Ask" task has a generation number, microphone
frames are tagged with the dictation id and dropped after `StopMic`, and a transcription result is accepted only after
`FinishStt` or `TranscribeSaved` (a failure reported while still recording, such as a model that did not download, is
shown when the recording ends). A task that crashes still reports: recognition as a failure with Retry, refine as the
raw text, a question as a failure with Retry, paste as "no field".

**Modes.** `Flow` keeps the mode of the dictation (`Mode`: `Dictate`, `Translate`, `Ask`). `ChordDown(mode)` starts a
dictation in the mode the chord picked. `ModeToggle` (a fresh Shift press) swaps dictation and translation ("Ask"
becomes a translation) while starting or recording, whether the chord is still held or was tapped (then only a Shift on
its own, see below): people often tap the chord and decide on a translation only while speaking, and a second Shift
takes a slip back. A translation goes back to the mode it was switched from (`toggled_from`: "Ask" too; a recording
that started as a translation becomes a dictation; Alt picking "Ask" forgets it), so every toggle is undone by the next
one, and a toggle the hook takes back restores a question as well. `ModeChange(mode)` (Alt, for "Ask") counts only while the starting chord is still held (in hold mode
too). Once the finishing press has moved the flow to thinking, neither
changes the mode, and neither does anything outside a recording. The mode reaches the Drop in `UiState` and the
runtime in `Refine { mode }` and `RefineAgain { mode }`; every path into Idle resets it to `Dictate`. A translation
that comes back as anything but a refined, non-empty text is never pasted: the attempt ends as `failed`, and the flow
shows the `translate` error (the agent's word, `agentSignIn`, `agentMissing` or `agentSilent`, when the user's AI is
the cause) with Retry. That Retry emits `RefineAgain` with the saved transcript: the text is translated again, the audio
is not transcribed again. With no AI (`RefineOutcome::NoAi`) there is nothing to translate with: `needAiTranslate`
("Перевод: выберите AI"), nothing pasted, no Retry.

**"Ask" in the flow.** A question is recorded and transcribed like a dictation; then, instead of `Refine`, the flow
emits `Ask { id, question, follow_up }` and stays in Thinking (the Drop's three dots) until the answer has something to
show. The runtime reports back with four inputs: `AskOpened` (the first real step: a search began, its results came,
or the first words), `AskAnswered` (the answer is complete; a refusal, or a written text shown instead of written,
too), `AskWrite { how }` (the AI wrote text for the field and the runtime holds it; `PasteHow` is `AtCaret`,
`Replace` or `Below`) and `AskFailed { error }` (`AskError::NeedAi`, `Agent(problem)` or `Failed`). An answer that opens or ends moves the
flow to the `Answer` phase: the Drop rests (it looks idle), the answer shows in the tray or the card, `Esc` stays
swallowed, and a chord records a follow-up. A write goes to `Inserting` at once with `AskPaste { id, how }` and ends
the conversation (`CloseAnswer`); the answer's buttons (Insert, Insert below, Replace) do the same for an answer that
is complete on screen, and Retry after a failure emits `AskAgain { id }`. `AbortAsk { id }` drops the request in
flight. `NeedAi` (no AI chosen) closes the answer and shows the Drop's "Спросить: выберите AI" (`needAiAsk`) without
Retry, and `Agent` (the user's agent signed out or missing) closes it with the agent's word, as a cleanup's; any other
failure keeps the answer open with the error and Retry, and the question is kept.

- `asking` is the dictation whose request is in flight, and it can be an earlier one than the current dictation: a
  follow-up may be recorded while the answer it follows is still printing. Results are matched against it, so a late
  one of a dropped request never reaches the flow. `asked` is the request whose answer the open answer shows: Retry and
  the paste buttons act on it, and a follow-up takes it over only once its own answer shows.
- `held` is a follow-up's question, transcribed while the answer it follows still prints: it goes (`Ask` with
  `follow_up: true`) as soon as that answer has ended, answered or failed. A follow-up that could not be transcribed
  shows the error with Retry inside the open answer ("Couldn't make out the follow-up"); dismissing it, `Esc`, or an
  empty follow-up goes back to the answer, which stays.
- `Esc` by phase: while a first question records, it is cancelled with Restore, as a dictation; a first question
  being transcribed or waiting for its answer is aborted and cancelled with Restore, which asks the same question again
  with the same context (the runtime keeps it, `KeptTaken`); a follow-up recording, being transcribed or waiting is
  dropped and the answer stays, still printing if it was, with no Restore; in `Answer` it closes the answer, aborts a
  request still in flight and forgets the conversation; during a paste it is only swallowed.
- A write that ends while a follow-up is under way (recording, being transcribed, held or missed) is never pasted
  under the user's hands, and CUE never injects input on its own while paused (the main window's Paste again is a
  click): the runtime asks `Flow::shows_write` and shows such a text in the answer instead. A pause closes an open
  answer (with a follow-up under way and the request in flight), since `Esc` does not reach CUE then.
- Shift and Alt do nothing to a follow-up (it stays "Ask"), and a dictation needs the answer closed first. A
  microphone that fails during a follow-up closes the answer for its own card (one card at a time).
- Accounting: an answer shown in full ends its attempt as `answered` (paid, nothing pasted), a written text as
  `inserted` or `no_field`, a pasted answer adds no second `Finished` (it was counted when it was complete). An answer
  still printing when a follow-up starts keeps its attempt aside (`answering` in the runtime) until it ends, with its
  own row; an answer closed or aborted mid-stream still writes what it spent.

### Hotkey chord

- The chord is Ctrl + Win (left or right, any order) and at most one of Shift / Alt, pressed in any order: Ctrl + Win
  alone dictates, with Shift it translates, with Alt it asks. Alt picks "Ask" only while `hook::ASK_ENABLED` is set (the
  core sets it at startup from `ask.enabled`, on by default). If Shift and Alt together (Ctrl + Shift + Alt + Win is the
  Office key), or Alt while "Ask" is off, are held when the pair forms, it is a system shortcut: no events, no swallowed
  `V`, no mask, and Windows sees every key as is.
- Every fresh Shift press (left or right; not its auto-repeat, not an injected one) toggles the mode while the chord
  is held, and also after the chord was released while CUE is starting or recording (then only a Shift on its own, and
  a key pressed while it is held takes the toggle back: see the guards below): the core sets `hook::RECORDING`
  for that (not `ESC_ACTIVE`, which also covers thinking and errors, when every capital letter typed would wake the
  core for nothing), and refreshes the flags right after the flow decides, before the effects run, so a Shift pressed
  just after a tap already counts. Inside a held chord the event goes out without the flag (the core may not have
  heard of the new recording yet), and the flow drops it when nothing records. A press is fresh when its side's bit
  was not set yet; the resync before every press drops a Shift whose release was lost. A release never toggles, so
  the order in which the keys come up does not matter. The Drop changes its icon at once.
- After a tap, two guards keep a Shift that belongs to other keys from switching the mode; a finishing chord pressed
  in any order (Shift + Ctrl + Win, Ctrl + Shift + Win, Win + Shift + Ctrl) must not turn a translation into a
  dictation. First, Shift toggles only on its own: with Ctrl, Win or Alt down (the finishing chord started with Ctrl
  or Win, Ctrl + Shift + a key, Alt + Shift, Win + Shift + S) it changes nothing. Second, when any other key goes down
  while the Shift that toggled is still held (a capital letter, Shift + Alt, a finishing chord started with Shift,
  Shift + Esc), the chord takes the toggle back: `Decision::undo_toggle` puts a second `ModeToggle` before that key's
  own event, so the flow is back in its mode before the finishing `ChordDown` or the cancel. A mouse button pressed
  while that Shift is held (Shift + click extends a selection) takes it back the same way (`Chord::on_mouse_down`).
  A `WH_MOUSE_LL` hook on the same thread sees the click; it is installed only while such a Shift is held, since a
  low-level mouse hook routes every mouse event of the system through CUE's thread. The hook procedures post
  `WM_MOUSE_WATCH` to their own loop, which installs or removes it (a hook procedure installs no hook). The mouse hook
  counts button presses only, lets injected ones pass and never swallows a click; it is removed after the click, the
  Shift's release or the next key, set again after a reinstall if still needed, and removed when the thread ends. Each
  Shift press is taken back at most once (both Shifts held: two toggles cancel out); a released Shift's toggle stands,
  also when the release was lost and the resync drops it. The Drop shows the brief switch and back. Inside a held
  chord neither guard applies.
- An Alt press inside a held chord, once "Ask" is on, picks "Ask". Shift and Alt together inside a held chord (the
  Office key pressed late) count as a third key (below) and also drop the Win and Alt masks whatever the order of the
  presses, so Windows sees the Office key as is.
- Ctrl, Win, Shift and Alt always pass through to Windows. CUE swallows only `V` while the chord is held (paste last;
  the Windows `Ctrl + Win + V` panel does not open) and `Esc` while CUE is starting, recording, thinking, pasting or
  showing an error. A swallowed key's auto-repeat and release are swallowed too.
- Before the Win key-up passes after a chord, CUE sends an unused key (VK `0xE8`), so Windows does not open Start.
  Alt that took part in an "Ask" chord gets the same key before its key-up, so the app's menu does not open.
- A third key while the starting chord is still held, within 1.5 s of the press (`Ctrl + Win + D`, arrows, Alt while
  "Ask" is off), means a system shortcut: the recording is dropped silently and the key reaches Windows. A later key
  is taken as a slip during a held dictation, and the recording goes on.
- Tap and release starts tap mode: the next chord press finishes. A chord held for 0.6 s starts hold mode: releasing
  finishes, and the first two times a "Press and release" card follows. Presses within 0.3 s of the last are ignored.
  A chord while thinking only makes the Drop shake.
- Injected key events (including CUE's own) pass untouched. Before each key-down the remembered modifiers are
  checked against `GetAsyncKeyState`, so key-ups lost to the secure desktop or UAC do not leave a stuck chord; a
  250 ms timer ends a chord whose keys are gone. A modifier counts as released only when it is seen up on this tick and
  the one before, and a modifier the hook has just seen go down counts as held on the next tick (`Debounce::pressed`):
  a tick may come before Windows records the press in `GetAsyncKeyState`, and dropping it then would lose what the key
  did — a Shift's toggle (its auto-repeat would toggle back) or the Alt of an "Ask" chord (its release would reach the
  app unmasked and open its menu). The check before the next key-down drops a modifier whose key-up was lost.
- The hook is installed again after resume from sleep and after unlock, since Windows may remove it.
- Pause for an hour sets `PAUSED`: the hook swallows and masks nothing and sends no events.
- While the welcome window holds the hotkey (`Input::WelcomeGate`, see Welcome window), a chord starts nothing: no
  recording, no follow-up over an open answer, no new recording in place of a "restore". It brings the welcome
  window forward instead (`Effect::BringWelcome`), and does not count for the 0.3 s debounce. Only a press that ends
  a recording already starting or running counts as usual; Esc, the release of a held chord and the Drop's buttons
  are untouched. `Ctrl + Win + V` pastes nothing, though it still drops a start-up begun before the hold. The hold is
  no pause: `PAUSED` stays clear, so the hook still swallows the chord and Start does not open.

### Recording

The microphone opens on the first press. Frames are resampled to 16 kHz mono `i16` in 20 ms frames. Each
frame goes to the WAV file (`last\current.wav`), to the recognition session, and, at most ~30 times a second while
listening, to the Drop as a voice level. Four seconds of digital silence from the start (below −70 dBFS: a muted or
blocked microphone, not a quiet room) stop the recording and show the "Microphone is silent" card, as does a
microphone that fails before any sound. One that fails after sound was heard ends the recording normally. At 8:00
a card says one minute is left; at 9:00 the recording finishes on its own. Locking the session or sleep discards a
recording in progress.

**Which microphone.** The one Windows uses by default, or the one pinned in the main window (`audio.device`, read
each time a recording opens the microphone). A pinned one is looked for on the `cue-mic` thread, so the core never
waits for the list: among the input devices with the names Windows shows, `mic::pick` takes the trimmed name exactly,
else the first that differs only in case. It is opened as that device, and cpal follows the Windows default only for a
stream opened on the default device, so Windows changing its default (a headset plugged in) does not move a pinned
recording. A pinned device that is not there leaves the recording to the Windows default: `MicEvent::Fallback` comes
before the first frame, and the flow (`MicFallback`, while starting or recording, and not while an "Ask" answer is
open) shows the "Another microphone" card with both names, which go to the Drop alone (`cue://mic-names`), never to the
log. Each loss is told once (`state::MicTold`): it counts as told when the card goes out, and a recording that has the
pinned device again, or a change of the microphone setting, forgets it. With nothing pinned no device list is read.

### Recognition: Parakeet on this computer

- **The model.** Parakeet Ultra: Moondream's post-trained NVIDIA Parakeet TDT 0.6B v3 (the same architecture and
  tokenizer, 25 European languages with Russian and English among them, punctuation and capital letters in its
  output), in the int8 ONNX export of `Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx`: `encoder-model.int8.onnx`,
  `decoder_joint-model.int8.onnx` and `vocab.txt` (8,192 tokens and the blank `<blk>`). NVIDIA's own v3 export
  (`istupakov/parakeet-tdt-0.6b-v3-onnx`) has the same file names and loads the same way; it is used only in the bench.
- **Runtime.** ONNX Runtime through the `ort` crate, CPU only, linked statically into the program. Its prebuilt
  library needs AVX2 and FMA, without which the process would die on its first instruction, so
  `parakeet::model::processor_supported` is asked before anything loads (see The speech model). Each session runs
  with graph optimization level 3 and 4 threads. The two sessions sit behind one lock: one recognition at a time.
- **The session** (`stt::start(Engine::Local(slot))`) opens at `StartStt`, whether the model is ready or not. It only
  keeps the frames; `finish` sends `Finish`, and the whole recording is recognized on a blocking thread. `Abort`, or a
  dropped session, is a cancel. `finish` waits 120 s for a ready model (`RECOGNIZE_TIMEOUT`; a 9-minute dictation
  takes well under a minute), and 30 minutes more while the model is still downloading or loading (`WAIT_FOR_MODEL`):
  the session's recognizer is the model's slot (`ModelSlot`), which waits for the model and then recognizes, or
  returns the failure that stopped it. Recognition of a 30-second dictation takes well under a second on a current
  processor; the text comes only after the recording ends. A streaming mode (pieces recognized at pauses while
  speaking) can come later behind the same session without changing the flow or the Drop.
- **Features** (`parakeet::mel`, ported from parakeet-rs): the 128-band log-mel the way NeMo computes it for Parakeet:
  pre-emphasis 0.97, a centered STFT (n_fft 512, a 400-sample Hann window, hop 160: 10 ms a frame), Slaney mel bands,
  a log with an additive guard of 2^-24, then every band normalized over the piece. A wrong spectrogram makes the model
  write nothing, so each step follows NeMo; an ignored test compares it with the export's own preprocessor
  (`nemo128.onnx`).
- **Decoding** (`parakeet::tdt`, ported from parakeet-rs): TDT greedy decoding over the encoder's output. At each
  frame the decoder/joint (a 2-layer LSTM of 640) gives token logits and duration logits; the best allowed token is
  emitted unless it is the blank, the decoder's state changes only when a token is emitted, and the best duration says
  how many frames to move on; at most 10 tokens on one frame. The text is the tokens joined, "▁" a space, special
  tokens written as nothing, and a number glued to a word given its space ("version 2"). Owning the decode loop is
  what lets CUE keep one language a dictation (below).
- **Long recordings** (`speech::pieces`): a recording longer than 60 s is cut into pieces of at most 60 s, each in the
  middle of the quietest 200 ms between 45 and 60 s into it (the window moving in 50 ms steps), so a word is rarely
  split. Each piece goes through the encoder; the pieces' texts are joined with a space.
- **Retry** (`TranscribeSaved`) recognizes `current.wav` again, the same way. After a model failure it first lets the
  model's download go on from where it stopped (`ModelKeeper::retry`), and recognition waits for the model.
- **Errors** (`stt_error`): the model missing, not downloaded or damaged (`EngineError::Model`) is the Drop's "Model did
  not download" with Retry; a processor without AVX2 (`Unsupported`) is "Processor not supported" without Retry;
  anything else, an ONNX Runtime error or a crashed task included, is "Couldn't recognize" with Retry. Each keeps the
  recording in `current.wav`. Recognition never needs the network.
- The log gets the audio length, the recognition time and whether the other alphabet was masked out (`Recognized`'s
  `Debug` shows the text's length alone), never the text.

### One language a dictation

The model detects the language itself and cannot be told one. CUE's rule: a dictation is in Russian or in English,
never mixed (`cue_core::speech`). Letters are counted by alphabet (`Alphabet`: Cyrillic or Latin, by
`translate::script_of`).

- **Which alphabet** (`alphabet_of`): the one whose language's function words the text holds more of (fixed lists:
  "the", "to", "it", "please"…; «и», «в», «не», «что»…), so Latin names and terms in a Russian sentence
  ("запусти cargo test в Visual Studio Code") do not make it English; with as many of each, the one most letters are
  in; none without letters of either (numbers only).
- **A leak** (`leak`): the model writing part of a Russian phrase in English (a known weakness), or the other way
  round. A leak is told from names and terms by the little words a sentence needs: when both alphabets hold at least
  two function words of their language (`LEAK_WORDS`), the alphabet with fewer letters is masked out
  (`Vocab::mask_of`: every token whose text is in that alphabet; special tokens never) and every piece is decoded
  again over the same encoder output: the encoder does not run twice. Names and terms in the other alphabet
  ("call Марисоль", one "the" in a film title) have no such words and stay as recognized; the cleanup and the
  dictionary handle them.
- **The speech language of a translation** (`speech_language`): the language of the pair, the target first and then
  home, written in the text's alphabet (`translate::script_of_language`); none when neither is, and unknown speech
  goes to the target. A fixed `translate.speech` wins. The recognition result carries no language of its own
  (`SttOutput::language` is `None`), so the runtime reads it from the text when the result comes.

### The speech model

`runtime::model` keeps the model: its state, its background download and load, and the slot recognition waits on.
The folder is `%LOCALAPPDATA%\CUE\models\parakeet-ultra-int8\` (`paths::model_dir`, named by the manifest's id).

- **Download** (`cue_engines::download`, on a tokio task): every file the folder lacks comes from
  `https://huggingface.co/<repo>/resolve/<commit>/<path>`, the commit fixed in `cue_core::model::ULTRA`, never
  `main`; about 670 MB in all. A file is written as `<name>.part`; a part already there is resumed with an HTTP range
  request (a server that ignores the range sends the whole file again). The finished part is checked against its size
  and SHA-256 and only then renamed, so a file under its final name has always passed the check, and a folder with
  every file at its size is ready (`download::ready`). A part larger than the file, or one that fails the hash, is
  deleted and starts anew on the next try. The progress (bytes of the whole model) reaches the windows once a percent.
  The request carries no key and nothing of the user's. The installer does not carry the model.
- **States** (`ModelState`, `cue://model`): `missing`; `downloading` with `done` and `total` bytes; `loading`;
  `ready` with its `total`; `failed` with the bytes already kept for the next try (a download that stopped, or files
  that did not load); `unsupported`. At start (`ModelKeeper::start`, before the first event): a processor without AVX2
  is `unsupported`, said once in the log and to every recognition; a ready folder loads; anything else downloads.
  `Fetched` loads it (`Recognizer::load` on a blocking thread, the time in the log), and the loaded model stays in
  memory, so a dictation never waits for a load once CUE has been running a few seconds. A load that fails checks every
  file's hash again (`download::verify`), deletes the damaged ones and only then reports `failed`, so Retry downloads
  just those. Retry, the windows' (`model_retry`) or a dictation's, acts only on `failed` or `missing`, and goes on from
  the bytes already there.
- **"Model is ready".** When the model becomes ready after a download in this run, the Drop shows the `modelReady`
  card, but only at rest with no other card: a dictation that waited for the model shows its text instead.
- The log gets the state's changes, the bytes and seconds of a download, the load time, how many files a check
  deleted, and an error's text; never dictation text.

At its first start this version deletes the Soniox key of earlier versions from Credential Manager
(`keys::forget_soniox`, asked at every start, a no-op after the first); the log says when one was deleted.

### Refine (cleanup)

- **Where it goes** (`runtime::route`, docs/specs/2026-11-08-no-keys-design.md). Every route first gets the text with
  the no-AI layer done (`basics::tidy`: the dictionary and learned spellings, no "эм"/"ээ", tidy spaces), and that is
  what goes in whenever no AI cleans it.
  - `Where::Off`, `Local` with no model chosen, or `Agent` with no agent set yet: `Route::Off`, the no-AI layer's text
    at once (`RefineOutcome::NoAi`, a normal "Done"; the log names the reason). A translation shows `needAiTranslate`.
  - `Where::Local`: the model in the slot (`local::refine`), or the raw text at once while none is ready, or a wait of
    at most `LOAD_WAIT` for the model loading again after a rest.
  - `Where::Agent`: the agent in `refine.agent` through `runtime::agents::Agents`. Its budget (`agent::refine::budget`:
    10 s + 1 s per 100 words for Codex and OpenCode alike (OpenCode took 5–8 s for a 165-word cleanup, past the
    cleanup's own 4 s), a translation a second more, plus 5 s on the
    agent's first request after CUE starts or after it rested: OpenCode's server let go, or 15 min without a request
    to Codex) counts from the moment it is asked, so finding or starting OpenCode's server (`OpenCodeSlot::get_or_start`,
    off the core's thread) comes out of it. The model is `refine.models[agent]`, else Codex's default (GPT-6 Luna), else
    OpenCode's "Быстрая · авто" (`opencode::fast_auto` over the models its server lists, picked once per server) or its
    own default; OpenCode's model goes in the request's JSON body only. Grok is not routed yet.
  - **Marks** (`AgentMarks`, `MARK_FOR` = 5 min): Codex is asked `codex login status` (no text) before its first run and
    every 5 minutes after, since a Codex never signed in retries about 15 s and would read as a timeout; "Not logged in"
    marks it signed out. An expired sign-in, which that command does not tell, is marked from the run's own error
    (`cli::signed_out`), and a missing program marks the agent missing. While a mark holds, a dictation goes in at once
    with `RawAgent(SignedOut | Missing)`, without the agent.
  - **Esc** drops the request: a running Codex is ended with its whole process tree at once (`process::run_until`'s
    cancel flag, set when the request's future is dropped), not at its timeout. The user's Retry of a translation the
    mark answered clears that agent's signed-out or missing mark first, so it asks the agent again.
  - **Working folder** (`agent::empty_cwd`): each agent runs, says its version and its sign-in in its own empty folder
    in `%LOCALAPPDATA%\CUE\agent`; when a stray file is there, in the first empty (or new) `<agent>-1`, `<agent>-2`, …
    beside it, reused while it stays empty. Nothing in them is deleted; one warning names the agent.
  - **Fallback** (`refine.fallback`, `agents::or_local`): "paste as said" leaves the no-AI layer's text and the Drop's
    word. "Почистить: <local model>" cleans (or translates) with the local model only when it is in its slot at that
    moment; a resting one is never woken or waited for. Cleaned in place of an agent that did not answer, nothing is
    shown (`Refined`); in place of one that is signed out or missing, the text goes in and the Drop still says so
    (`RefineOutcome::Covered`). With this fallback the local model is kept loaded beside the agent (`llm_active`).
  - The Drop's words after the paste, without Retry, name the agent (`cue://cleanup-name`): `agentSignIn`
    "<Agent>: войдите заново", `agentMissing` "<Agent> не найден", `agentSilent` "<Agent> не ответил".
  - At startup, and when the main window moves the cleanup to `Agent`, with no agent set: `agent::discover` off the
    core's thread, and the first found in `ORDER`
    (OpenCode, Codex) is written to settings.json; none found, the cleanup moves to this computer with nothing chosen.
    OpenCode is found as `opencode` on `PATH` or in its usual folders, else as OpenCode Desktop's own CLI
    (`%APPDATA%\ai.opencode.desktop\cli\<version>-<arch>\opencode-cli.exe`, the highest version; nothing else there is
    run), else by a server it announces in `~\.local\state\opencode\service.json` that accepts a connection (then with
    no program: CUE uses that server and never starts one).
  - **OpenCode's requests** (`agent::opencode`): to the server the user runs (OpenCode Desktop's, announced with its
    password), else to `opencode-cli serve` CUE starts in `%LOCALAPPDATA%\CUE\agent\opencode` only
    (`opencode::server_cwd`): OpenCode keeps a server's folder as one of the user's projects, so never a spare beside
    it; something in that folder and no server is started (`Failed("cwd")`, one warning). On OpenCode 2.2.5 a cleanup
    is one stateless `POST /api/experimental/generate/text` (`{prompt, model}`, no tools, no session) and names no
    folder: 2.2.5 keeps every folder a request names as one of the user's projects. Its answer is the JSON CUE asks
    for, the same in a code fence, or plain text, fenced or not (no schema holds it), each through the same check
    (`refine::held_to_schema`); a plain-text cleanup whose first line has a colon after words the speaker did not say
    ("Вот исправленный текст:") is refused (`refine::opens_with_lead_in`), and so is a translation whose first line
    has a reply word or a word naming a translation before a colon ("Here is the translation:", "Вот перевод:";
    `refine::translation_opens_with_lead_in`, judged by those words since a translation's are never the speaker's: a
    translated "Plan for tomorrow:" passes). A rate limit or a used-up free allowance
    (`ProviderRequestError`, `reason: "rate_limit"`) is `AgentFailure::RateLimited`: the agent did not answer
    (`RawAgent(Silent)`, "не ответил"; the local fallback cleans as for any silence), the log's reason "rate limit".
- The cloud request described below (the Messages API, OpenRouter, the OpenAI-compatible services) is no longer reached
  by a dictation; it stays in `cue_engines` until the keys go. Right after the second press the runtime asks
  `cue-focus` for the focused app and up to `refine.context_chars` characters before the caret, in parallel with
  transcription. Password fields are never read; the app name comes from the executable's file description, never the
  window title.
- The request: the system prompt as a cached block, a user message with `<app>`, `<before_cursor>` and `<transcript>`
  tags (closing tags inside the data are defused), effort `low`, thinking disabled where the model allows it, and a
  JSON schema that forces `{"text": …}`. Timeout: 3 s plus 1 s per 100 words. A timeout, a network failure or an
  overloaded service (HTTP 500, 502, 503, 504, 529) is asked once more with twice the time; the raw text is pasted
  only when that request fails too. Key, limit and balance refusals are not asked again.
- The user's dictionary (`dictionary.terms`) goes in a second system block after the unchanged prompt: a short rule
  (write a listed name exactly as listed when the speech clearly means it, even if the transcript spells it
  differently, splits it or mixes alphabets; add none the user did not say and change no other word because of the
  list) and the terms, one per line. The cache mark sits on the last system block, so the cached prefix covers the
  prompt and the list together; editing the list costs one cache write. Entries are trimmed, and empty, multi-line,
  overlong (over 60 characters) and repeated ones are dropped, at most 200 kept. With no usable term the request is
  byte for byte the one without a dictionary. Recognition never gets the terms: the model takes no vocabulary.
- The cleanup's style block is the text of `%APPDATA%\CUE\instructions\instructions.md` as it stands
  (`claude::system_blocks`, `Job::Rewrite`), kept by `instructions::Instructions`. CUE writes the file when missing:
  the standard text in the interface language (`cue_core::instructions::standard`: `refine::BOLD_PROMPT` in English,
  `BOLD_PROMPT_RU` in Russian), or the old custom style's text once. The runtime asks for the text at every cleanup and
  warm-up, and the Instructions page while it is shown (`instructions_get`, every 1.5 s); the file is looked at most
  every 300 ms, read again when its modified time or length changed (the version used until then is kept as
  `instructions.prev.md`), cut at 2 000 characters, and an empty, deleted or non-text file means the standard text. The
  protocol (output format, the dictionary and habits blocks, the translation) stays in code. `AGENTS.md` beside the
  file is CUE's own, rewritten when its first line `<!-- CUE <version> -->` is older than the app. The folder holds only
  these files, so an AI opened in it sees nothing else of CUE's; an earlier build kept them beside settings.json, and
  they move into it once at startup without losing anything: a rename that fails is a copy (the original stays, and
  the next start, finding the same text in both, keeps one); with two different `instructions.md` the newer stays in
  use and the other is kept as `instructions.old-<date>.md` (copied first, then removed; a taken name gets `-2`, never
  overwritten), an old previous copy likewise as `instructions.prev.old-<date>.md`; one that can be neither moved nor
  copied is read where it is, and no standard text is written in its place. CUE's old `AGENTS.md` goes.
- When the app is unknown (neither the query at the start of recording nor the one after the second press named it),
  the request carries `<app>unknown app</app>`.
- The answer is used only when it is a complete `{"text": …}` object: a refusal, `max_tokens`, a timeout, a network
  error or a bad response all mean the raw transcript is pasted with the normal "Done". Explicit Markdown the speaker
  did not dictate is stripped, and stray commas and semicolons at the end of list items are removed.
- **Sanity check** on what would be pasted: lengths are compared in characters without numbers (digits and number
  words), and only for transcripts of at least four words with two or more that are not numbers; the ratio must stay
  within `min_ratio`…`max_ratio` (0.35–1.25). At least `min_overlap` (70 %) of the result's non-number words must
  appear in the transcript; words of `dictionary.terms` count as appearing, so a name fixed in a short phrase
  ("открой гитхаб" → "Открой GitHub.") is not taken for an invention. An empty result always fails. A failed check pastes the raw transcript.
- The user's AI signed out, missing or silent pastes the no-AI layer's text (or the local fallback's) and then shows the
  agent's word without Retry (see "Where it goes").

### Translation

- **Direction** (`translate::output_language`). The speech language is `translate.speech` when fixed, else the one
  the text's alphabet names (`speech::speech_language`, see One language a dictation). Speech that is not in
  `translate.target` goes to the target; speech in the target goes to the home language; unknown speech goes to the
  target; with no home language, speech in the target is only cleaned up. The home language
  (`translate::resolve_home`) is decided at startup, and again when the main window changes the target:
  `translate.home`, else the Windows display language, else Russian, then English, skipping any equal to the target.
  Settings example: `"translate": { "target": "en", "home": "ru", "speech": "auto" }` sends Russian speech to English
  and English speech to Russian. Speech outside the pair counts as the pair's language of its alphabet: German speech
  reads as English there and goes to Russian, unless `translate.speech` says `de`.
- **Request** (`claude::Job::Translate(code)`, `request_body_for`). The refine request as is, plus one more system block
  after the cache mark (`translate::instruction`): clean up the transcript by the rules above, then write the whole
  result in the target language (its English name), which replaces the rules to keep the spoken language and never
  translate; keep the meaning, the tone and the formatting rules (plain text, the list rules, one line in terminals);
  dictionary terms, product names and code stay as written. The cached prefix is the one plain dictation uses, so a
  translation never writes a cache of its own, and a request without a translation is byte for byte what it was.
  `max_tokens` doubles (another alphabet may take more tokens), up to 16 000. Timeout: 4 s plus 1 s per 100 words,
  a second more than a cleanup.
- **Check** (`translation_ok`), instead of the sanity check, on the text after Markdown stripping and list tidying.
  The result has letters or digits. From 20 letters of speech, its length is 0.35–2.8 times the speech's, counted in
  letters, each weighted by its script (a Han character counts as 3 Latin letters, a Hangul one as 2, a kana as 1.5),
  and in digits, 2.5 letters each on both sides: a translation often writes the speech's number words as digits
  ("sixty thousand" as "60 000"). A result of digits alone ("сто двадцать пять" → "125") is checked by length however
  short the speech, and by nothing else: no untranslated word can be pasted. When the speech is in another script than
  the target language, the result must be written in the target script. Latin is the script of names, product names
  and code in every language, so the speech's script is its main non-Latin one when it has any. The result is split
  into words (runs of letters of one script); words of `dictionary.terms` never count, nor does a word in the target
  script that the speech already has (names and code kept as said, compared ignoring case), and into a non-Latin
  target no Latin word counts. Of the counted words at least one is in the target script, and so is at least 80 % of
  their weighted letters. So an echo of the speech fails, code-heavy speech included ("запусти npm install lodash" →
  "Запусти npm install lodash."), while "Run npm install lodash." passes on "Run". A pair in one script (English into
  German) is checked by length only, since there the script cannot tell a translation from a cleanup; Serbian, written
  in both Cyrillic and Latin, is checked by length only too. The word overlap of the sanity check does not apply: a
  translation shares no words with the speech.
- **Known limits of the check.** A translation made only of dictionary terms fails ("Sberbank" → "Сбербанк" with that
  term listed). Into a non-Latin target, Latin text shows nothing either way, so the English original followed by its
  translation passes, and so does English with one word in the target script. A phrase of names around one word of
  speech ("кубернетес и докер" → "Kubernetes и Docker.") passes into English unless the names are listed. In a
  same-script pair the untranslated speech passes, and English speech with a word in the target script (a Cyrillic
  name, translated into Russian) counts as such a pair. A digit weighs 2.5 letters, the low end of what a spoken
  number takes, so a long number alone, said in many words and written in few digits, still falls under the length
  bound ("one thousand nine hundred ninety nine" → "1999" fails); inside a sentence the other words carry it. The
  check does not read meaning: a self-correction can come back turned around and pass ("Отправь отчёт Олегу, точнее,
  Игорю" came back as "…to Igor, I mean Oleg" in every bench run).
  A translation made only of target-script words the speech already has fails ("сделай git push" → "Git push."), and so
  does a large round number said in one word and written in many digits ("миллиард" → "1 000 000 000"): both safely,
  with Retry. A short question answered with a bare number ("какой год" → "2026") passes.
- **Failure.** A network error, a timeout, a refusal, `max_tokens`, a response off the schema or a failed check pastes
  nothing: the flow shows "Couldn't translate" with Retry, which translates the saved transcript again. The user's AI
  signed out, missing or silent shows the agent's word with Retry instead; no AI shows "Перевод: выберите AI" without
  Retry. The log gets one line with the reason and the outcome, and the target code when a translation starts; never
  the text.

### Cache warm-up

The cleanup and "Ask" have none: neither goes to the Messages API, and an agent keeps its own caches. On this computer the local model evaluates the cleanup's system part while the person
speaks (`local::warm`).

### Paste

The same path serves dictation, `Ctrl + Win + V` and the main window's Paste again (which carries the window it goes
to, as an "Ask" paste does, see Main window):

1. One short focus query: is there a field, which app, the last few characters before the caret.
2. Trim the text's edges, then add a join space only when the caret sits right after a word.
3. In a terminal every line break becomes a space, so pasting never presses Enter. One list in `platform-win`
   (`terminal.rs`) names the terminals for every check: Windows Terminal, PowerShell, Command Prompt, conhost, mintty
   and Git Bash, WSL, WezTerm, Alacritty, Warp, Tabby, Hyper, Termius, WindTerm, MobaXterm, PuTTY, KiTTY, ConEmu and
   Cmder, Fluent Terminal, by the name Windows shows or the exe (`terminal_app`), and by window class or exe right
   before an input (`terminal_window`; for a Store app's frame, the exe of the app in it). If the query was late, the
   last known app or the app at recording start decides.
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

## Corrections

CUE notices when the user corrects a word it pasted and offers the word for the dictionary (`learn.enabled`, on by
default; off, no field is read). The path from a paste to a suggestion:

1. `insert` keeps what is pasted (`watch_next`, only for a dictation or a translation, never an "Ask" answer, and only
   while `learn.enabled`). When `InsertDone` reports the paste as `Pasted`, the runtime starts `fieldwatch::spawn` on
   the thread `cue-field`, with a stop flag (`field_stop`) that a new dictation (`OpenMic`), a newer watch or "Learn
   from me" turned off sets.
2. The thread asks UI Automation for the focused element once, right after the paste, and keeps it: not a password
   field, not a window of CUE's own, with a text pattern or else a value pattern. It reads that element only (focus
   moving to another window or to CUE changes nothing) every 400 ms (`READ_EVERY_MS`), at most 60 s (`WATCH_MS`), and a
   field of more than 20,000 characters is cut there.
3. `Watch` (in `cue-core`, over plain strings) finds the pasted text in the first reading: the text before it and after
   it are the anchors, and a paste not found within 2 s means no correction. The watch is over 2.5 s after the field's
   last change (`SETTLE_MS`), when the field empties (it was sent: the last reading counts), when the element can no
   longer be read, or at the stop flag. The corrected span is the text between the anchors in the last reading; with
   the anchors gone (a cleared and retyped field), or nothing changed, there is none.
4. The thread ends with `CoreEvent::Corrected { pasted, span, stop }`. `corrections::candidates` aligns the pasted text
   and the span word by word (the longest common subsequence of the words, compared without case and outer punctuation)
   and returns the replaced runs of one to three words as `Replacement { heard, word }`. Of a run with more words
   written than heard, only the leading ones that each look like a name or a term belong to the word; the rest is text
   typed on after it ("позвони Марго и скажи" for "позвони марине" offers "Марго"). A run is a candidate when the new
   text differs from the old by more than case and punctuation, is not in `dictionary.terms` in any case, passes
   `refine::is_term` with the heard text too, and looks like a name or a term: a capital inside it, a digit, letters of
   the other alphabet than the text around it, or a capital first letter that does not start a sentence. Deletions,
   insertions and ordinary lowercase words are not candidates. Texts of more than 400 words are not looked at.
5. The first candidate goes to `Offers`, which holds one suggestion at a time (a waiting or a missed one is replaced, an
   open or just added one stays). It opens once the Drop is at rest and can be clicked (`show_offer` over `at_rest`:
   idle, no tray card, no answer, not paused, not hidden over a full-screen app, which the placement thread tells the
   core with `CoreEvent::IdleHidden` as it tells the page), and the runtime sends the Drop `cue://suggest`
   (`{ word, missed }` or nothing). Paused or over a full-screen app, the page hides an open capsule as it hides the
   idle bar.
6. The Drop's page grows the capsule from the word and reports `suggestAdd`, `suggestUndo`, `suggestDismiss` (its 8 s
   ran out) and `suggestDone` (the 6 s after adding ran out) through `drop_action`; an action out of turn changes
   nothing. The times are timers in `drop.js` that the mouse over the Drop pauses; the rings only draw them, and with
   reduced motion they stand still. `suggestAdd` becomes `SettingChange::DictLearn`, applied and written like the main
   window's own changes, and the main window gets the settings on `cue://settings` (it draws the Dictionary again,
   nothing else); `suggestUndo` becomes `DictRemove`. The dismissed suggestion stays as `missed`, a dot next to the idle
   bar, until the next dictation opens the microphone; hovering the Drop opens the capsule without a ring.

`dictionary.learned` keeps each added word with how it was heard, newest first; each is also in `dictionary.terms`.
`DictRemove` takes a word out of both. The cleanup's dictionary block writes a learned word with how it was heard
("Marisol — sounds like «Марисоль»"), when that is a usable term, and the other words as before.

`fieldwatch`'s `FieldReader` (one method, `read`, giving the field's text or `None` once it can no longer be read) and
`Pace` (the clock and the wait) are what `run` is written over; the UI Automation field is only the Windows
`FieldReader`, so another system plugs in its own reader and its own `spawn`, and the loop, `Watch`, the candidates and
the suggestion stay as they are. The loop and `Watch` are tested with scripted readings, without UI Automation.

The field's text lives in memory only, on `cue-field` and in the event that ends the watch. It is never written to disk
or to the log, which gets "correction found" with the span's length and the number of candidates, or "correction:
none"; the `Debug` of `Replacement`, `Step` and `SuggestView` prints lengths.

## Habits

When the same correction of a dictation is made three times in an app, CUE keeps it as a habit, says so once, and asks
the cleanup to write that app's dictations that way (`learn.enabled` covers it: off, nothing below runs). Habits take
the correction watch's end: the path from there to a card, and on to a result:

1. `CoreEvent::Corrected { pasted, span, stop, app }` carries the app the watch started for, as the history names it
   (`stored_app`: the executable's file description). `on_corrected` offers the dictionary candidate as before and then
   `count_habits`; a dictation whose app is not known, and a translation, are not counted.
2. `habits::correction` reads the pair into `Found` forms (`Form`: `EndMark`, `FirstLetter`, `Replacement`, `Digits`,
   `Dash`, each with its detail) over `corrections::runs`, the alignment `candidates` also uses, split out so that a
   run the dictionary cuts is still whole for habits. A correction shares no form with a paste that has no word in
   common, one end mark swapped for another is neither an end mark nor a first letter, and a replacement longer than
   `MAX_EXAMPLE_CHARS` (40) is no form. The correction's example is the changed words with their neighbours, cut to
   40 characters on each side. A form is counted by its hash under this installation's secret
   (`Form::hash(&Secret)`: HMAC-SHA-256 of its kind and detail, written out over `sha2`, the first 8 bytes; a habit's
   id, `habits::id_of`, is FNV-1a over its forms' hashes and its app, none for every app, so the same forms make one
   habit per app). The secret (`habits::Secret`, 32 random bytes) is kept in Credential Manager
   (`keys::habit_secret`, the entry `habits-secret`); `Runtime::run` sets it before the first event (`key_habits`).
   A missing or malformed one is made anew and goes through `Habits::rekey`: `seen` and `declined` go, the ids are
   made again. Under the secret kept, `Habits::reid` makes again any id made otherwise (a settings.json restored or
   brought over; a set counted under the old id follows it), and the file is written when one changed. A store that cannot be read is left alone (`keys::secret_from` neither makes nor writes): the session
   has no secret. It never reaches settings.json (`#[serde(skip)]`) or a log (`Secret(..)`), and without one nothing
   is counted or declined (`Habits::forget` only removes the habit).
3. `Habits::count` keeps `seen`, a count by app and hash with the day it last grew (30 days, `MAX_SEEN` 300), once per
   form per dictation; a hash in `declined` is skipped. A form already in a habit of the app, or of every app, grows
   that habit's `n`. A habit of another app whose forms all came together in the correction counts once for this app,
   under its id (a form already counted on a habit of this app or of every app is in no set); when that set reaches
   `LEARN_AT` (3) the habit widens (`Habits::widen`): its app is cleared, the rule loses its "In <app>" start
   (`habits::unscoped`), it takes the id of every app, the counts add up, habits of the same forms in other apps fold
   into it (their counts added), its set's and its forms' counts go in every app (none follows the new id), and
   `send_settings` redraws the section without a card. `Counted::widened` pairs each id that went with the new one:
   `follow` moves the open card and the waiting ones to it, and `habits.ts` keeps the row of an app's habit gone with
   the same example for the habit of every app that took its place, so the row changes in place. A habit's form alone in another app counts for that app as any form does. The other forms that reach three
   together become one `Draft`, with the example and `n`.
4. `ask_rule` numbers the draft as found (`next_notice`), keeps it in `drafts` and, when the cleanup runs on this
   computer with its model in the slot, asks that model (`local::rule`). On an agent route the rule is always CUE's
   template: the 5 s wait would expire on most agent runs, and no agent is run for a rule; with no AI, the template
   too.
   `habits::rule_messages` builds it: instructions that name the interface language, and a user message that holds the
   kind(s), the example and the app, nothing else. The answer is a `{rule, short}` object (`read_rule` accepts a rule of
   at most `MAX_RULE_CHARS` (80) on one line, and a short name of at most `MAX_SHORT_CHARS`). On an agent route, with no
   AI, with the local model not in its slot, or on an error, an answer that cannot be used, or no answer within
   `RULE_WAIT_MS` (5 s), `habits::template` writes the rule and the short name from the kinds; a scoped habit's rule starts with the app. The request is not a
   usage row, and its log line says why it fell back, never the answer.
5. `CoreEvent::HabitRule` reaches `on_habit_rule`, which drops the draft if "Learn from me" went off meanwhile, else
   `Habits::learn` (the habit goes first in `list`, past `MAX_HABITS` the oldest go; its id is `id_of` its forms and
   app),
   saves the settings, logs "habit learned" with the kinds, `n` and whether the cleanup wrote the rule, and sends the
   main window `cue://settings`. Its card is queued in `habit_cards`. Two drafts of the same forms in one app (the
   second reached three while the first waited for its rule) make one habit: when the second's rule comes, `learn`
   finds the first's id and adds the second's count to it, with no second card (the runtime reads `Learned.merged`,
   also in the log line; `Learned.was` keeps that habit as it was); a draft whose forms' habit widened meanwhile adds
   to that habit of every app the same way. A habit of another app is never added to: the draft makes its own app's
   habit, with its own id. `learn` (`adopt` keeps only its `habit`) also takes out the habits holding an opposite of the draft's forms
   (`Form::opposite`: an end mark removed and added, lower case and a capital, a replacement and its reverse) in its app
   or in every app, not declined; when one of them held everywhere, the new habit does too and the opposites of every
   app go with it. They come back in `Learned.replaced`, kept on the card (`HabitCard`); one whose own card still
   waits is swapped by `carry` for what that card would bring back (`was`, then its `replaced`), and that card goes,
   so the new card takes back both changes. A habit with an opposite habit in some app never widens
   (`Habits::opposed`). A merged draft that also replaced habits still gets a card.
6. `show_notice` opens what waits when the Drop is at rest (`at_rest`, as for a dictionary suggestion), one notice at a
   time: `first_notice` compares the suggestion's number (`offer_found`) with the first card's, so the one found first
   goes first, and a card waits while the suggestion's capsule is open or shows "undo" (a missed suggestion's dot does
   not hold it back). A habit is "found" when its forms reach three, though its card can open only once its rule came.
   `show_habit_card` sends the rule to the Drop on `cue://habit-rule`, then the plain tray kind `newHabit`, or
   `habitChanged` for a card with replaced habits, on `cue://tray`; the page draws the card with the rule, closes it
   after 7 s, and pauses at the mouse like the others. `trayAction` on it goes to `undo_habit` (`HabitCard::undo`): a
   card that learned a habit calls `Habits::revert(id, replaced, true)`, so the habit goes, its hashes join `declined`,
   and the replaced habits come back as they were; a merged card learned no habit, so it calls `take_back` (the habit
   its draft added to comes back as `was`, with the replaced ones) and declines nothing. The Undo wins: a habit learned or widened since that
   goes back on one brought back, in its app or where either holds everywhere, is removed (not declined) and returned,
   and its card, if one waits, is dropped. `hold_tray` keeps the Drop busy for the done label ("Removed", or "Restored"
   on `habitChanged`) until the card has folded (`TRAY_DONE_MS`, then `TrayDone`), so nothing opens over it. A card
   whose habit was forgotten from the main window meanwhile is skipped. "Learn from me" turned off drops the drafts and
   the cards waiting, and reverts a change waiting for its card without declining (`HabitCard::take_back`:
   `Habits::revert(id, replaced, false)`, the newest first; a merged habit comes back as `was`); a card already open
   stays until it closes. A habit widened while its card is open: the card keeps the rule it opened with (the one with
   the app), while the section shows the new one. The drafts and the cards waiting live in memory only. Quitting CUE
   loses a draft, and the counts that went into it are not given back, so that correction is counted from zero again; it
   loses a card not yet shown too, and its habit stays in the list unannounced.

The other `SettingChange`s are `HabitOn { id, on }` and `HabitsForget`, which empties `seen`, `list` and `declined`
together (a form at two would otherwise become a habit at the next correction), each answered with the settings.
`SettingsView.habits` lists the habits for the section (id, rule, example, the app's name, `n`, on); `habits.ts` draws
the section, with the line for no cleanup connected (the chosen connection is a cloud one with no saved key, checked
when the section is drawn, when the connection changes and when the window gets the focus back).

The request side. Where the cleanup request is built (`Runtime::refine`), `habits_for` allows habits only for a
dictation with "Learn from me" on and a Clean or Rewrite job: a translation and "Ask" take none. `Habits::for_app` takes
the habits that are on and belong to the dictation's app or to every app (an unknown app gets the latter),
`RefineContext::with_habits` carries their rules and their replacements' new words (`said`), and `refine::habits_prompt`
writes the rules as a block of one rule per line under a short intro, a rule over `MAX_RULE_CHARS` skipped. The Messages
API (`claude::system_blocks`) and the other connections put the block after the cache mark, behind the dictionary block
and before the style's block: it changes with the app, so the cached prefix stays the one every dictation reads. With no
habit the request is the one without the block, byte for byte. The answer's sanity check (`claude::settle` →
`decide_with` → `sanity_ok_with`) counts the `said` words as said, as it does the dictionary's terms, for that answer
only: a short phrase whose greeting a habit replaced ("здравствуйте анна" → "Добрый день, Анна!", 1 word of 3 said) is
otherwise rejected and the raw text pasted. Nothing goes into the dictionary, and a translation's check takes none.

The runtime keeps the habits sent, with the text as heard, for each dictation (`ByDictation`, the last four by id), so a
later dictation's cleanup never takes an earlier one's place. When a refined result comes back, `habits::marks` runs
each sent habit's applied check on its text and on the text as heard (`Habit::applied`: every form shows in it: the end
mark is absent or present as the form says, the first letter has the case, the replacement's words are there and the old
ones are not, a digit is there, a dash stands and no spaced hyphen), and the ones the cleanup brought (they show in its
text and not in the text as heard: the recognizer may have written the digits, or no full stop, itself) become
`Entry.habits`, a list of `{ id, short }` on the history entry. A raw fallback marks none. The History's detail lists
them under how the entry was said, as chips; a habit forgotten later keeps its short name on the entry, so its chip
shows as it was.

Text on disk and in logs: `habits.seen` holds the app's name, a hash, a number and a day, no text; the habit holds its
rule, its short name and the example (`before`, `after`, at most 40 characters each), which is the only text of a
correction ever written. The log gets "habit forms counted" and "habit learned" with kinds and numbers, and `leak-scan`
looks for the examples' `before` and `after`, the replacements' `from` and `to` and a short name the cleanup wrote (not
the rule, nor a short name the template writes for the habit's forms: CUE's own literals). A replacement's words are
words of its example, so `forms` holds no text the example does not. The `Debug` of `Form`, `Found`, `Correction`,
`Seen`, `Habit`, `Draft`, `RuleAsk`, `RuleText`, `HabitMark` and `RefineContext` prints lengths and counts, never text.

## Ask

"Ask" (`Ctrl + Win + Alt`) is a dictation whose text becomes a question to the cleanup's AI: the user's agent with
the cleanup's model, or the model on this computer (docs/specs/2026-11-08-no-keys-design.md, "Ask and translation").
The flow decides when to ask, paste and close (see Dictation flow); `runtime` and `runtime::ask` keep what those
effects need, `cue_engines::agent::ask` asks the route, and `cue_engines::ask` holds the steps, the end and the
deadlines of a question.

### Context

- **At the chord** (or when Alt picks "Ask" while the chord is held), while the focus and the selection are where the
  user left them, the runtime asks `cue-focus` for the field with its selection (`query_with_selection`, 500 ms): the
  app, up to `refine.context_chars` characters before the caret, and the selection. A text-pattern field gives its
  first selected range (`GetSelection`): its text, its lines on screen (`GetBoundingRectangles`, physical px) and
  whether it is read-only (`IsReadOnly` of the range; a mixed or unknown value reads as editable). A classic Win32 field
  gives its selection through `EM_GETSEL` and `WM_GETTEXT`, with no rectangles; `ES_READONLY` is read-only. Password
  fields, and fields that will not say whether they are one, are never read. A selection of more than 8,000 characters
  (`MAX_SELECTION_CHARS`) keeps 4,000 from its start and 4,000 from its end with a note between them, from two short
  reads, one at each end, so a whole document selected costs no more; `SelectionRead::Text` says it was `clipped`, and
  such a selection is never replaced. The answer is `SelectionRead`: `Empty`, `Text`, `Password` or `Unknown` (the
  field has no way to report a selection); its `Debug` shows lengths only. Chromium gives
  rectangles too; `CHROMIUM_RECTS` in `focus.rs` turns them off should a browser page or an Electron app place them
  wrongly.
- **The `Ctrl + C` fallback**, at the end of the recording (the chord's keys are up only then): when the field said
  `Unknown`, there may be a field (not the desktop or File Explorer's list), the app named at the chord is not a
  terminal (`terminal_app`) nor an editor that copies its current line with nothing selected (`COPIES_LINE`, empty for
  now), and Windows does not upload copied text to the cloud clipboard by itself: `clipboard::uploads_to_cloud`, read
  once per chord from `HKCU\Software\Microsoft\Clipboard` (`EnableCloudClipboard` on, and `CloudClipboardAutomaticUpload`
  not 0, a missing value counting as automatic), since the app's copy carries none of CUE's marks; the selection then
  reads as none, and the log gets the state only. It runs on `cue-insert` (`InsertJob::CopySelection`), alongside the
  transcription, and only into the window read at the chord: right before `Ctrl + C` that window must still be in front
  and must not be a terminal by its class or executable (`terminal::terminal_window`). The clipboard is saved,
  `Ctrl + C` sent, and for 0.4 s CUE waits for the
  app's copy: a write owned by the window's process, by the app in it when the window is a Store app's frame
  (`WindowRef::app_process`), or by no window (`the_apps_copy`), checked again with the clipboard open. Any other
  process's write is never taken for the selection, and a copy whose text is the user's clipboard text again counts as
  nothing copied. The user's clipboard is then put back with the paste markers. A copy that comes 0.4–2.4 s after
  `Ctrl + C` is still taken back, on `cue-late-copy`, unless there was any input since (`GetLastInputInfo`), the
  clipboard was written again, or a newer clipboard operation of CUE's began (one process-wide lock, `Operations`,
  covers the late guard's check and restore and every paste, left text and copy). The app's copy reaches the Win+V
  history before CUE can mark it, so `forget_in_history` looks among the three newest history items for exactly that
  text, stamped no earlier than just before `Ctrl + C`, and deletes it if Windows allows (`HistoryForget`: removed, not
  found, refused; logged as a kind). That copy carries none of CUE's marks, so with clipboard sync on and its upload
  set to manual, the user may still send it to other devices from Win+V until it is removed there. The request task
  waits for the copy (2 s at most), never the core; a copied text is a selection that can be changed (clipped like a
  read one beyond 8,000 characters). `COPY_FALLBACK` in `selection.rs` switches the whole fallback off.
- What the question is asked with (`runtime::ask::Taken`): the `AskContext` (app, text before the caret, the clipped
  selection, today's local date from `GetLocalTime`), the selection kind (`None`, `Writable`, `ReadOnly`), whether it
  was clipped, its rectangles, whether the app is a terminal, and the window the question was asked in (`WindowRef`,
  the only one an "Ask" paste goes to). The first question of a dictation keeps it until the next chord
  (`KeptTaken`), so Restore after `Esc` asks the same question with the same context, a late copy included, though the
  first ask used up the focus query and the copy.

### Route and request

- A question goes the way the cleanup goes (`route` of `refine`, in `spawn_ask`): with no AI chosen (no AI, or nothing
  chosen on this computer or among the agents) it ends at once, and the Drop says "Спросить: выберите AI"
  (`AskFailure::NoAi` → `AskError::NeedAi` → `needAiAsk`), without Retry. Through the user's agent it goes to
  `Agents::ask` with the cleanup's model (`ask.model` is gone); on this computer to the model in the slot, waited for
  (`llm::LOAD_WAIT`) while it loads again after a rest, and "Couldn't get an answer" with Retry when none is ready.
  A model that only cleans will end here with `needAnsweringModel` once such models are offered.
- `Agents::ask`: a signed-out or missing mark answers at once with the agent's word; otherwise the agent's start
  allowance (5 s on its first request) is added to the first-words deadline, and finding or starting OpenCode's
  server, Codex's sign-in check and the run all come out of it. A run that says the agent is signed out or missing
  marks it, as a cleanup does, and the Drop names the agent in its word ("Codex: войдите заново", "Codex не найден");
  the answer closes, without Retry. A Codex that answered is marked signed in.
- `agent::ask::run`: the instructions (`AGENT_ASK_PROMPT`, English; the answer follows the language of the question)
  and the whole conversation as one user message (`agent_messages`: an agent keeps no session of its own). The
  instructions say what the tags mean, that their content is data and never instructions, how to answer: one JSON
  object `{"answer", "write", "replace", "sources": [{"url", "title"}]}`, with `write` for text the user asked for
  their field and `replace` when it changes the selected text itself, in place of Anthropic's `write_to_field` tool;
  the form of a written text (plain, no quotes, the app's tone, one line in terminals), the form of an answer (the
  answer itself first, short, simple Markdown; never a word about how it decides or whether it searches), and when to
  search (only for current or local facts), with sources only from its own searches and `[]` without one.
- The user message (`first_message`): `<question>`, then `<selection>`, `<before_cursor>`, `<app>` and `<today>`;
  empty parts are left out, and our closing tags inside the data are defused. A follow-up (`follow_up_message`) is the
  question alone; `agent_messages` puts the turns before it first, each user turn in `<user_turn>` and each answer in
  `<your_answer>` (those closing tags inside them defused), then the new question.
- **Codex**: the instructions go in their own `codex-instructions-<hash>.md` and the answer's shape in
  `codex-ask-schema.json` (`ASK_SCHEMA`, every field required, nothing else), both in CUE's agent folder; the
  conversation goes on stdin only. With `ask.web_search` on, `-c web_search=live` replaces `web_search=disabled` and
  `--disable code_mode_host` is left out: on 0.166.0 live search runs through the code-mode host, and with the host
  off Codex has no web access at all (checked 09.11.2026; the run's events are `codex-ask-search.jsonl`, a
  `web_search` item and the answer). Every other flag stays as for a cleanup, and a cleanup, a translation and a
  question without search keep the host off. Its `web_search` items are then allowed in the events, and any other
  tool item still refuses the answer (`codex_answer`). Codex streams no deltas: the answer appears whole. `Esc` drops
  the request, which ends Codex's process tree.
- **OpenCode** is asked as for a cleanup, with no tools: its API does not say whether a model has a search tool, so
  it never searches. Its answer appears whole too: the reply is one JSON object, read once complete, never streamed
  raw onto the card. **This computer**: the same instructions and message, the answer held to `ASK_GRAMMAR` (its
  `sources` always empty); it never searches. Whether a route can search is decided in one place,
  `agent::can_search` (Codex only), for both the question's deadline and its run.
- With `ask.web_search` on, a question answered without it gets `Step::NoSearch`: first when the route cannot search
  (OpenCode, this computer), before the answer when Codex did not search. The answer then says "без поиска" / "no web
  search" under it once it is shown whole (`TurnView::no_search`); a turn that ends as a written text says nothing.
  With web search off, nothing is said.
- The deadlines (`Limits::SPEC`): through an agent the first words, which are the whole answer, within 15 s, 20 s with
  web search, plus the start allowance; on this computer the whole answer's 30 s, since nothing shows before it is all
  written.

### Answer

- `finish` reads the reply: a code fence around it is taken off; a reply that is no JSON object at all is an answer as
  it stands; a broken object, or an empty answer, is "Couldn't get an answer" with Retry. A write (`write: true`) is
  `Step::WriteStart` (its first words) and `AskEnd::Write { text, replace }`; an answer is `TextStart`, `Text` and a
  `Cite` for each source (without a title, its site). Sources count only after a search the route ran (one named
  without a search is made up) and only web pages (`openable`).
- Failures (`AskFailure`): no AI chosen; the agent missing or signed out (the Drop's word, no Retry); a timeout; any
  other failure in a word (`exit`, `no answer`, `answer`, `empty`, `local model`). All but the first two show "Couldn't
  get an answer" with Retry, under the words already shown. One log line each: the kind, never a text. `usage.csv`
  gets the cleanup's model in `refine_model` and the searches Codex ran in `web_searches`; an agent's tokens are its
  own.

### Conversation and the answer

- `runtime::ask::Conversation` lives from the first question until `CloseAnswer`, and is then forgotten. It keeps the
  answered exchanges as `{"role", "text"}` turns (`AskTurn`: the user message as sent, the answer or the text it
  wrote; a turn kept in Anthropic's content blocks by an earlier version still reads, its text blocks joined), the
  question in flight, the sources (numbered in the order a URL is first cited, across the conversation) and
  `AnswerView`, the answer as the windows draw it. A first question is asked with its context; once an exchange has
  succeeded, a follow-up goes alone, and the whole conversation is sent with it as text. A refusal and an empty turn
  never join the conversation, and a failure keeps its question for Retry.
- The request runs in a task of its own (`ask_task`, with a generation, as transcription and refine do): the steps and
  the end come back as events of that generation, so a dropped or replaced request's never reach the conversation. The
  view goes to the windows (`cue://answer`) at once for a new turn, an end, an opening or a follow-up row, and at most
  every 50 ms while words stream (`ViewGate`); the flow hears `AskOpened` once per request (`ShownOnce`).
- A turn's text is its text blocks (`Segment`s) joined as they came: a citation splits a block, and those parts run on.
  Words before the turn's first search only say that it is going to search (a model may write such a sentence,
  though the instructions ask for none): they go when the search starts, and the search line takes their place. Words before a later search and words after it are two blocks with nothing between
  them, so a block after a search (`after_search`) starts a paragraph, in the paste (`TurnView::text`) as on screen
  (`markedText`).
- Where it opens (`open_answer`): in the card when the card window exists, the selection has a rectangle with a size,
  and `cardplace::card_place` finds room for it on the selection's monitor; otherwise in the tray from the Drop. The log
  gets the place only.
- A written text takes the dictation's form in code (`field_text`: Markdown the user did not give taken off, list
  separators tidied), whatever the instruction achieved. What the user gave is the question and the context the model
  was shown, the selection and the text before the caret: a sign there (a README's heading, a `# comment`, a list that
  ends its items with ";") is content, and its rule is off, so a replace keeps what the user did not ask to change. Code
  is never touched: a ``` block goes in as written (one that is the whole text loses only its fence lines), and inline
  code keeps its backticks; the code is set aside while the rules run over the rest. Where it goes (`write_target`):
  about read-only text, it is shown in the answer (in a terminal too); a `replace` in a terminal, or over a clipped
  selection (its middle would be erased), is shown; a `replace` over a writable selection goes over it; an `insert`
  goes to the caret in a terminal (a line break there is Enter, so never below), below a writable selection, or to the
  caret without one. A text shown instead of written is an answer to copy. A write that would be pasted is shown too
  when the window the question was asked in is not in front any more, or is a terminal for a text with line breaks
  (`clipboard::may_paste`, the same window check as before the fallback's `Ctrl + C`, ruling 6b): a write lands
  seconds after the question, and the user may have moved on.
- The buttons (`answer_actions`) follow the selection: read-only → Copy (in a terminal too); a terminal → Insert,
  Copy; writable → Insert below, Replace, Copy (no Replace over a clipped selection); none → Insert, Copy. A refused
  turn has none. They paste or copy the last turn's plain text (`plain_answer`: the Markdown taken off; the lines of a
  ``` block as they are, without the fence lines, and inline code without its backticks) or the written text. A pasted
  answer, by a button or as a write, becomes the last result (`last\result.txt`), so `Ctrl + Win + V` pastes it again.
- Every "Ask" paste, a write or a button's, carries the window read at the chord (`InsertJob::Paste { target }`):
  right before the Right arrow and the Ctrl+V, `cue-insert` checks that it is still in front and, for a text that
  starts with a line break or spans lines, that it is not a terminal (`may_paste`). Otherwise nothing is pasted: the
  text stays in the clipboard and the flow hears "no field" (the "Nowhere to paste" card); a chord window that was not
  known pastes nowhere either. Insert pastes the way a dictation does. Replace pastes over the selection, which is
  still selected, with the selection's own leading and trailing whitespace in place of the text's (`replace_edges`): no
  join space, and a paragraph selected with its line break keeps it. Insert below (`InsertJob::Paste { collapse: true }`)
  waits for the modifiers to be up (2 s at most), presses Right to collapse the selection to its end, and pastes a line
  break and the text in one paste (an Enter would send a chat message). A selection that cannot be collapsed is never
  pasted over: the text stays in the clipboard, and the flow hears "no field". Copy leaves the text in the clipboard
  the normal way.
- A click on a source chip sends only its number (`answer_source`); the core opens the URL only when it is a web page
  (`openable`: `http` or `https` with a host, no spaces, control characters or quotes, 2,048 characters at most), with
  `ShellExecuteW` on a short thread of its own.

## The Drop and tray cards

The Drop window is transparent, borderless, always on top, absent from the taskbar and Alt+Tab, and never takes focus:
`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` are enforced by a window subclass, because the windowing library rebuilds the
extended style on every flag change, and the subclass adds `SWP_NOACTIVATE` to every move, show and z-order change.
The window is larger than the capsule; the UI reports the capsule and card rectangles (`drop_hit_rects`) and
`cue-hit-test` lets the mouse through everywhere else. The "Ask" card has a window of its own built the same way (see
The "Ask" card below).

### Places

The Drop sits at one of four places on the work area of the monitor with the foreground window (`drop.position`):
`bottom` (the default) and `top` are centered on that edge, `left` and `right` halfway up it; in each the Drop's outer
edge is 12 px from the work area's edge. The window is always centered on the Drop's center (the center of its
circle), and the page draws the Drop at the window's center. A new place therefore moves only the window, never the Drop
inside it; the rest of the layout changes with CSS transitions. The window reaches far enough every way for the widest
Drop, the tray cards and the "Ask" answer tray (420 × up to 410 logical px, with its spring overshoot and shadow)
growing toward the screen center, so its size depends on the place (`drop_window::window_size`): 640 × 860 logical px
at the bottom and the top, 900 × 540 at the sides. A square big enough for both would lie mostly on the old monitor
after a move to a monitor of another scale, so it would keep the old scale and never arrive there (see below). The size
changes at the end of a drag, around the Drop's center, so the Drop stays where it was let go; no drag happens while the
answer tray is open. The part past the edge of the work area (over the taskbar, or off the screen) is transparent and
lets the mouse through like the rest.

At the bottom and the top the Drop is centered and grows both ways along the edge, and tray cards open above it (at
the bottom) or below it (at the top). At the sides its outer end is pinned at the edge and it grows toward the center;
the idle strip stands upright against the edge, and on hover and in every other state it is the usual capsule or
circle. Tray cards open beside it, toward the center, centered on it.

After the window is created and placed, `cue-placement` is the only CUE code that moves it, always with
`SetWindowPos(SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)`; Windows itself still resizes it, and may move it, when its scale
changes. At rest the thread checks every 0.5 s that the window is centered on its place's anchor on the foreground
window's monitor, runs the z-order guard, and hides the idle strip over full-screen apps
(`SHQueryUserNotificationState`, told to the page and to the core) and while paused. The window is always centered on
the anchor at its current size, never at the size it would have at the target monitor's scale. The anchor lies a few
dozen px inside the target monitor, so most of a window centered on it is on that monitor whatever the two scales;
Windows takes a window's scale from the monitor it shares the most area with, so it gives the window the target's scale
and size, and the next check centers it again. Placed at the predicted size, most of the window could stay on the old
monitor, whose scale it would then keep, so it would never arrive. While the window is hidden the thread only waits for
the next check.

**Dragging.** Only the idle Drop can be dragged, once hover has opened it. The page watches a press of the primary mouse
button on it: a move of more than 4 px with the button down starts a drag (`drop_drag` `start`); a press that moves less
stays an ordinary click. No tray card is open during a drag: none can be open when it starts, and one that arrives ends
the drag first, as do a dictation, a pause and full screen. While dragging, the page captures the pointer and reports
the whole window as its hit rectangle, so no click falls through while the window catches up with the cursor.
`cue-hit-test` accepts such a whole-window rectangle only while `cue-placement` is dragging or snapping the window, so a
page that misses the end of a drag (busy or hung) cannot leave the whole window catching clicks. `cue-placement` moves
the window itself every 8 ms so the Drop's center keeps its distance from the cursor (`GetCursorPos`); it ends the drag
on `drop_drag` `end` or as soon as the primary button is physically up (`GetAsyncKeyState`, which also covers a release
the page never saw and the secure desktop), and a hidden window ends it too. On release it picks the place whose center
is nearest to the Drop's center on the monitor under it, saves it through the core (`settings.json`), sends
`cue://place` (also when the place did not change: that answers the page and ends its dragging state), and snaps the
window to that place on the foreground window's monitor: a short spring of 250 ms (ease-out-back, past the place by
3.7 % of the way for a short snap and never by more than 4 px, then exactly on it), or at once when Windows animation
effects are off (`SPI_GETCLIENTAREAANIMATION`, the setting WebView2 reports as `prefers-reduced-motion`). The drag and
snap transitions sit behind a small trait for the system calls, so they are tested without a window.

The core drives these moves instead of starting the system move loop (`WM_NCLBUTTONDOWN` with `HTCAPTION`): that loop
is modal on the window thread, can bring up Snap Assist, resizes on DPI changes on its own terms and leaves no room for
the snap. A move from another thread never activates the window, the mouse press does not either
(`WS_EX_NOACTIVATE`), and the app the user is typing in keeps the focus.

### Z-order guard

`WS_EX_TOPMOST` is set once, at creation, and Windows has been seen to put an ordinary (not always-on-top) window of
another app above the Drop and above other always-on-top windows: the Drop was drawn but hidden. Every 0.5 s at rest
`cue-placement` walks the windows above the Drop (`GW_HWNDPREV`), skipping hidden ones and noting for each whether it is
always on top and whether DWM cloaks it (closed Start and Search panels, windows on other virtual desktops). The
ordinary, uncloaked windows above the Drop that overlap the Drop window cover it (`covering_windows`). When some do, the
Drop is put back on top with `SetWindowPos(HWND_TOPMOST, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE)`, and the next check
tells whether that helped (`ZGuard`, a pure state machine). Uncovered, it logs that the Drop is on top again. Still
covered by the same windows (a window in a band above every always-on-top window), it logs that and raises the Drop
again only every 30 s, or at once when the set of covering windows changes. The same windows back on top within 2 s of a
raise that helped came back by themselves (one of them keeps putting itself forward, like a maximized window seen above
the Drop): the same occurrence, held off the same way and raised again 30 s after the raise that helped, so such a
window costs one raise every 30 s, not one a second. Later than that, the user brought them forward: a new occurrence,
answered at once. Each kind of message is logged once per occurrence. A raise whose request fails is logged as such and
is not taken as "cannot be raised above": the next check raises again, and after two failures in a row only every 30 s.
So the guard never raises twice a second for nothing, and does not keep putting the Drop above other always-on-top apps
when that cannot help; always-on-top windows above the Drop are never answered at all. Window titles are never logged.
`zorder_probe` (an example in `cue-platform-win`) prints what the guard sees.

### Events and sizes

Core → window events: `cue://state` (phase, error cause, retry, paused, mode), `cue://level`, `cue://tray` (card kind
or none), `cue://shake`, `cue://idle-hidden`, `cue://place`, `cue://answer` (the whole `AnswerView` of the open "Ask"
answer, or none when it closes: sent at once for a new turn, an end or an opening, at most every 50 ms while words
stream) and `cue://card-place` (for the card: which side of the selection, where the arrow is, how tall the card may
grow), `cue://mic-names` (for the Drop alone: the two names of the "Another microphone" card, sent just before
it), and `cue://model` (to every window: the speech model's `ModelState` whenever it changes; the main and welcome
windows draw it, see The speech model), and `cue://suggest` (for the Drop alone: the word of the suggestion and whether
it was missed, or none; see Corrections). Window → core commands: `drop_action` (`cancel`, `finish`, `undo`, `retry`,
`dismiss`, `trayAction`, `trayClose`, the answer's `answerInsert`, `answerBelow`, `answerReplace`, `answerCopy`,
`answerRetry`, `answerClose`, and the suggestion's `suggestAdd`, `suggestUndo`, `suggestDismiss`, `suggestDone`),
`answer_source` (the number of a source chip clicked), `drop_hit_rects`, `drop_drag` (`start`, `end`), `drop_ready`,
after which the core resends the current state, card, suggestion, `idle-hidden`, place and open answer, and, from the
card window, `card_hit_rects` and `card_ready`, after which the core resends the state, the card's place and the open
answer. The place also comes in an initialization script, so the first frame is already laid out for it.

The windows get the text of an "Ask" question and its answer, the card's quote of the selection (`quote_of`, 40
characters at most), and the titles and sites of its sources, to draw them; never a source's address. A click on a
source sends only its number, and the core opens the page with `ShellExecuteW` (see Ask). No dictation text reaches the
Drop or the card; the history's text goes to the main window alone (see Main window).

One size: the Drop is 36 px high in every state (`--h` in `drop.css`; `DROP_SIZE` in `drop_window.rs` places it, and a
test keeps them equal). The circle states are 36 × 36. Everything inside is the original 40 px drawing scaled by
`--h / 40`: the mark 0.6 h (the same while recording), buttons 0.7 h, small icons 0.5 h, keys and label text 0.9 of
the drawing, paddings and gaps alike. The idle strip is 36 × 7 (upright at the sides), so starting a dictation grows it
into a circle of the same diameter. Tray cards keep their own sizes.

Phases: `idle` (a thin strip; on hover the mark and the hotkey), `start`, `listen` (voice bars and no timer; cancel
and done buttons on hover), `think` (three dots), `done` (a check, gone after 1.5 s), `empty`, `cancel` (Restore with
a 3.6 s ring), `error` (amber "!", the cause on hover, Retry when available; the causes are `model` "Model did not
download" and `translate` "Couldn't translate" with the network icon, `cpu` "Processor not supported", `stt`
"Couldn't recognize", the agent's words `agentSignIn` "<Agent>: войдите заново", `agentMissing` "<Agent> не найден" and
`agentSilent` "<Agent> не ответил" (the agent's name comes on `cue://cleanup-name` just before; "AI" when none came),
`needAiTranslate` "Перевод: выберите AI" and `needAiAsk` "Спросить: выберите AI"; `needTranslatingModel` and
`needAnsweringModel` are kept for models that only clean), `answer` (an "Ask" answer is open and
nothing records: the Drop rests as in `idle`). The frame loop sleeps when nothing moves; with reduced motion turned on
in Windows, states change instantly.

The mode shows while recording: in a translation the mark of `start` takes the translation shape and `listen` adds
the arrows icon next to the voice bars, and "Ask" has an icon of its own; a mode change re-enters the phase, so the icon
changes at once. The error of a failed translation reads "Couldn't translate" with the network cause icon and Retry.
The idle Drop shows no mode, and its hint stays `Ctrl + Win`.

Tray cards grow out of the Drop, one at a time; a new card replaces the old one, a new recording and an "Ask" answer
that opens close it, and hovering pauses the countdown ring:

| Card | Button | Closes |
|---|---|---|
| Nowhere to paste | Copy → Copied | after 9 s |
| Microphone is silent | Settings (opens `ms-settings:privacy-microphone`) | manually |
| Press and release | Got it | after 7 s |
| One minute left | — | after 5 s |
| Another microphone (the pinned one is not there; both names) | — | after 6 s |
| Model is ready (the speech model finished downloading in the background) | — | after 5 s |

A card button arrives as `trayAction`; the core acts on the last card it sent.

The interface is in English, or in Russian when the Windows display language is Russian; `ui.language` in
`settings.json` (`en` or `ru`) overrides Windows. The core makes that choice at startup, and again when the main window
changes `ui.language`; so the tray menu, the Drop, the card and the main window always match. On a change (`Ui::relang`,
on the main thread) the tray menu takes the new labels, and the Drop and the card load their pages again with
`?lang=` (the Drop with `?place=` too, so a Drop dragged since startup lays out its first frame for its place), which
the pages prefer to the values of their initialization scripts; once loaded, each sends `drop_ready` or `card_ready`
and gets its state back (see Events and sizes). The main window draws itself in the language `settings_get` reports.

**The answer tray.** An "Ask" answer without a card opens in a second tray of the same kind (`.ansA`, 420 px wide, up to
410 high), with the Drop's material, growing toward the screen center from any of the four places. It holds the header
(the mark, the question, the cross), the turns (a follow-up's question, the thinking line, the answer revealed a few
words at a time, its source chips), the row of a follow-up being recorded ("Listening for a follow-up…") or one that
could not be made out (with Retry), and the buttons. A request that fails after its first words keeps them, shown
whole, with the error and Retry under them. The thinking line names only real steps ("Searching the web",
"Reading sources", "Writing the answer"); before the first one it is shimmering placeholder lines, and the answer opens
only with a real step. The buttons are disabled while the answer is still coming or still being revealed (Retry
excepted), and all of them and the cross are disabled outside the `answer` phase. While any tray is open the Drop steps
aside. `answer.ts` is one view for the tray and the card: the core sends the whole answer every time, and the view keeps
what is on screen and reveals only what is new.

**Safe Markdown** (`markdown.ts`): paragraphs, `- ` and `1. ` lists, bold, italic, inline code and fenced code;
headings come out as bold lines and links as their text. No HTML: every piece of the model's text reaches the page
through `textContent` or a text node, never as markup. Source numbers become chips, marked in the text with
private-use characters, and the model's own such characters are taken out first, so no answer can fake one.

### The "Ask" card

The card at the selection has its own window (`card_window`, the page `card.html`), created hidden at startup with the
Drop's non-activating subclass, always on top, out of the taskbar, click-through outside the card through its own
`cue-hit-test` and `card_hit_rects`. It is shown and hidden only through Tauri, never with a bare `ShowWindow`: on any
change of its flags (the hit test switches click-through about 60 times a second) the windowing library hides a window
it believes hidden. Without the window (it could not be created) every answer opens in the tray.

- **Where** (`cardplace::card_place`, pure): the card is 390 logical px wide and up to 300 high, with a 6 px arrow 4 px
  from the selection, kept 8 px inside the work area of the selection's monitor. The arrow points into the selection's
  first line (at most 140 px in, at its middle when the line is shorter) and keeps 18 px from the card's rounded
  corners; the card goes under the selection when the tallest card fits there or there is more room below than above,
  and above it otherwise. With less than 120 px of room, or a selection off the work area, the answer goes to the tray.
  The window is the card's size plus 24 px of transparent room for its shadow on every side.
- **Showing** (`cue-card`): the hidden window moves to its place; on a monitor of another scale Windows gives it that
  scale and its size there a moment later, so the thread waits for that size (300 ms at most) and moves it again, so
  the arrow lands exactly; then it raises the window and shows it. The page folds the card away in 0.25 s before the
  window hides; a new answer in that time keeps the window.
- **The page** (`cardPage.ts`) draws the same answer view in the system material, with the arrow, and reports the card's
  rectangle for click-through whenever it changes. Its frame loop runs on for 500 ms after the last change, longer than
  the card's 450 ms pop-in, so the last rectangle it reports is the card at its full size, also for an answer that shows
  whole at once.

## Main window

An ordinary window with the history and the settings (`main_window`, label `main`, the page `window.html`). It opens
from the tray icon (Open CUE, or a double click); there is one at a time: opening it again restores it, shows it and
brings it forward, and an open while it is still being created does nothing more. It is created when opened and
destroyed when closed, so no WebView stays in memory and the page reads the history and the settings again each time.
It is built on `cue-main-window`, never on the main thread (see Threads). Windows draws its frame, in the Windows theme
(`theme(None)`; the page follows `prefers-color-scheme`); it is 700 × 760 logical px, at least 700 × 480, centered,
and unlike the Drop it takes the focus. Closing it never quits CUE: an exit request without a code is prevented, and
Quit exits with one. When it is destroyed, its history channel is dropped and a microphone preview it started stops.

### Commands and permissions

The main and welcome windows' pages talk to the core through the app's own commands (`commands.rs`), and every one
starts with `guard` and its own name. The main window is answered for every command but the welcome window's own
(`welcome_step`, `welcome_close`, `open_link`); the welcome window only for what its steps need (`WELCOME_COMMANDS`:
`settings_get`, `settings_set` with a microphone change alone, `model_get`, `model_retry`, `key_state`, `key_set`,
`key_check`, `mic_list`, `mic_preview`, `mic_access`, and its own); any other window gets `notMain` and a log line with
its label and the command. The Drop's and the card's commands (`drop_action`, `answer_source`, `drop_ready`,
`drop_drag`, `drop_hit_rects`, `card_ready`, `card_hit_rects`) answer those windows alone in the same way (`ipc::guard`)
and do nothing for another. The Drop and the card draw what a model wrote, so no page of theirs may read the history,
change a setting or touch a key. Errors are short codes (`core`, `notFound`, `badValue`, `badName`, `emptyKey`, `store`,
`autostart`, `failed`), never text. The main and welcome windows have capabilities of their own
(`capabilities/main.json`, `welcome.json`) with `core:event:allow-listen` alone, for `cue://mic-level` and `cue://model`
(and the welcome window's `cue://mic-failed`, and the main window's `cue://state`, which holds no text); the Drop's and
the card's `core:default` (`default.json`) does not cover them. The commands:

- `settings_get`, `settings_set` (one `SettingChange`, `{kind, value}`), answered by the core within 3 s with
  `SettingsView` (the history's period and its recordings', each `null` while unknown; the microphone; the
  translation target; the interface language and the language it resolves to; the 60 languages; whether the file is
  written; the dictionary's words). The Dictionary section sends `dictAdd` (the word trimmed and put first; refused
  when `refine::is_term` would not take it, when it is there in any case, or when the list holds `refine::MAX_TERMS`)
  and `dictRemove` (the word as written); a log shows only a word's length;
- `autostart_get`, `autostart_set` (the autostart plugin's Run key, changed only when it differs; the tray menu's
  check follows within 5 s);
- `history_subscribe`, `history_page`, `history_search`, `history_copy`, `history_paste`, `history_clear`,
  `history_audio` (see Recordings);
- `stats_get`: Today's numbers, `cue_core::stats::compute` over usage.csv — words by local day for the last 30 days,
  today's words and the minutes they save at 40 words a minute, words a minute, days in a row, words in all and the
  round number reached. Counted are `dictate` and `translate` rows with `inserted` or `no_field`; a row without `words`
  is estimated from its audio at the speed measured on the rows that have them (150 words a minute without any), and
  one without `day` takes its UTC date moved by the local offset now. The window asks on start, a second after each
  new history entry, and when it comes back to the front;
- `model_get` (the speech model's `ModelState`, answered by the core within 3 s) and `model_retry` (the download goes
  on from where it stopped, then the model loads; the new state comes as `cue://model`);
- `key_state`, `key_set`, `key_check` (Anthropic only: any other name is `badName`);
- `mic_list`, `mic_preview`, `mic_access`;
- the welcome window's `welcome_step`, `welcome_close`, `open_link` (see Welcome window).

### History

`cue_core::history` keeps one file per local day in `%LOCALAPPDATA%\CUE\history`, `YYYY-MM-DD.jsonl`, one line of
JSON per entry (`id`, `ts`, `mode`: `dictation`, `translation` or `ask`, `app`, `text`, `question` for "Ask", and
`raw` for a dictation or a translation: the text as it was recognized, before cleanup or translation, taken at the top
of `refine()` and with each attempt's end, so a retry keeps the one heard anew; its `Debug` shows lengths alone),
appended in one write, so a day is never rewritten. `ts` is the local time to the millisecond; the id is `ts-n`, `n`
counting the run's entries. A line that does not read as an entry is skipped, and only files named for a real date
count, so nothing else in the folder is read or deleted. There is no database: a search reads the files.

- **Written** only by the core (`HistoryLog`), never in the way of the result: a dictation or a translation when its
  text becomes the last result (pasted, or left in the clipboard with "Nowhere to paste"), with the app it went to
  (empty when not known); an "Ask" turn that ends complete, answered or written: its question and the answer as a
  paste puts it (`Conversation::paste_text`, without the Markdown), or the text the model wrote. Not written:
  `Ctrl + Win + V`, an answer pasted with its button, Copy and Paste again from the window, a refusal, a failure, a
  cancelled or empty dictation. A disk error is one log line.
- **Read** by the commands on blocking threads: the first page is today and yesterday (`recent`), Show earlier the next
  7 days that have entries (`older`); a search finds the entries whose text, question or `raw` holds the query in any
  case (ё read as е), newest first, 200 at most. Each entry goes to the page with `hasAudio`, whether its recording
  plays (one listing of the recordings folder per command). New entries reach the page over a `tauri::ipc::Channel`
  the page registers with `history_subscribe` before its first page (one at a time; a new one replaces it, and the
  window's end or a failed send drops it): only the webview that made a channel gets its messages, and no event
  carries history text. The page merges what comes before its first page and drops an entry it already shows (by id).
- **Drawn** by `history.ts`: three or more entries of a day in a row from one known app, each at most 10 minutes after
  the one before by the time in its id, are a stack (`stackRuns`), keyed by its oldest entry so a new one on top keeps
  it open; search results are never stacked. An entry with `raw`, a question, a recording, or one that came over the
  channel opens (a click, or `Enter`/`Space` on the focused row, `aria-expanded`): a dictation's words as heard
  against its text (`said.ts`: Myers' comparison on words without case or punctuation, at most 1000 words dropped and
  added, else shown unmarked), a translation's speech, an "Ask" question, and the player. A search marks its hit
  there too, and an entry it found only in `raw` opens by itself and closes with the search.
- **Retention** (`history.keep`): `forever`, `year` (366 days back from today), `month` (31, the default), `week` (7)
  or `off`. Older days are deleted at startup, on the first entry of a new local day and when the period changes in the
  window (the page then reads its first page again); `off` writes nothing and deletes every day, and a day after today
  (the clock was moved back) is kept. While the period is unknown (`settings::Period::Unknown`: settings.json was not
  read, or `history.keep` holds a value CUE does not know) nothing is written or deleted, until the window sets a
  period; the window shows it as not set. Clear history (a second press within 3 s) sends `CoreEvent::HistoryClear`,
  and the core deletes every day and every recording.
- **Copy and Paste again** look the entry up by id (`history::find`) and send its text to the core
  (`CoreEvent::HistoryText`). Copy leaves it in the clipboard. Paste again takes the window right under the main window
  in z-order (`focus::window_below`: visible, not minimized or cloaked, not a tool window, not CUE's, not the desktop),
  asks Windows to bring it forward and waits for that (`bring_forward`, 500 ms), then pastes there the way
  `Ctrl + Win + V` does, with the window check right before Ctrl+V; with no such window, one that does not come
  forward, or another window in front by then, the text stays in the clipboard. The reply is `pasted` or `clipboard`,
  and the page shows the check on Paste again or on Copy. Neither changes the last result or adds an entry. Paste again
  works while CUE is paused too: it is a click in the window, and the pause is about the chord.

### Recordings

Each entry's sound is kept in `history\audio` as Ogg Opus (`cue_platform_win::opus`: 16 kHz mono, VOIP, 16 kbit/s,
20 ms frames, about 1.6 KB a second; the pages and their end trim per RFC 7845, vendor `CUE, libopus 1.3.1`), one file
per entry named by its id with `-` for `:` (`history::audio_file`; no other name is ever made from a page). Only the
core writes and deletes there (`runtime::recordings`).

- **Period** (`history.audio`: `day`, `week` by default, `month`, `off`): the shorter of it and the history's own
  (`HistorySettings::audio_period`), `off` while the history is off; none is recorded while either is unknown. What
  deletes them (`audio_limit`, a cut-off in days back from today, like the days') is that period, or while the
  recordings' own is unknown the history's (a year: 366 days; forever: none).
- **Taken** on the core thread as a snapshot, `<name>.wav.tmp` in the folder: a hard link to the WAV (a copy where no
  link can be made), so the WAV may be replaced or deleted at once. A dictation's or a translation's is `last\audio.wav`
  once its text was committed and its entry written; an "Ask" question's is `current.wav` as the question goes out
  (`Runtime::ask`), since a follow-up may start recording before the answer ends: under an id of its own that no entry
  gets (the startup sweep knows its name), kept while its request may be asked again, and given to the turn's entry
  when it ends (`encode_as`), or deleted when no entry is written. Only an attempt's own recording counts
  (`Runtime::wav_of`): an attempt whose WAV could not be made takes none, and its commit deletes what an earlier attempt
  left in `current.wav` rather than taking it. `open_mic` deletes `current.wav` before creating it, since a truncating
  create would change a snapshot linked to it.
- **Encoded** on `cue-recordings`, one at a time, into `<name>.ogg.tmp` next to the recording, then renamed; the
  snapshot is deleted whatever came of it. Clear history, `off` and the history turned off move a generation on and
  take the folder away at once on the core thread: renamed to `audio.cleared-<n>` and deleted on the worker (deleted in
  place when a file in it is open, which Windows refuses to rename), so an encoding of an earlier generation is not
  started, or is dropped right before its rename, which then finds no folder; the folder is never made again by one.
  Prunes go to the same queue after the encodings: at startup, on the first entry of a new day and when a period
  changes. At startup alone, before anything is recorded, CUE's own half-written `.wav.tmp`/`.ogg.tmp` and the folders
  left set aside are deleted.
- **Played** in the main window only: `history_audio(id)` checks the id's grammar before any file access (`badValue`),
  then answers the bytes of the file (`tauri::ipc::Response`, an `ArrayBuffer` in the page) when it is a file of the
  folder itself (not a link), 32 MB at most, and not past the cut-off even before a prune deletes it; else `notFound`.
  The page asks on every opening, and once more 2 s later for an entry that came over the channel (its recording may
  still be encoding); it makes a `Blob` of type `audio/ogg; codecs=opus` and an object URL, which the CSP allows
  (`media-src 'self' blob:` in `tauri.conf.json`), and gives the URL back when the entry closes, its row is no longer
  drawn, a new recordings period reloads the list, or the page goes. The length comes from the last Ogg page (all ones
  or over 3 h: the player's own). One plays at a time, only on a press, and every one pauses when `cue://state` says a
  recording starts (`start`, `listen`).
- **Logged** as lengths, sizes, counts and error codes; never a path, a file name, an id or the sound.

### Settings tray

The gear opens a tray with six pages: Speech recognition, Cleanup, Microphone, Languages, History and General. Every
change goes to the
core at once (`settings_set` → `CoreEvent::Settings`), which applies it to its copy (`Settings::apply`), writes the
whole file and answers with the settings as they are now (see Settings). After a `core` error the page reads them
again.

- **Speech recognition**: the model's block (`window/model.ts`, the welcome window shows it too), drawn anew on every
  `cue://model`: "Parakeet Ultra", its state in a few words (Ready, Loading, Downloading N %, Didn't download, Not
  supported, Not downloaded; also the row's value on the first page), "Speech is recognized on this computer.", and,
  while it downloads or once it failed, a thin bar and "N of M MB" with the numbers in Bebas; Retry (`model_retry`)
  once it failed. There is no choice of engine. The page reads the state with `model_get` when the window opens.
- **Cleanup** (`window/cleanup.ts`, docs/specs/2026-11-08-no-keys-design.md, "Cleanup page"): on top Без ИИ · Этот
  компьютер · Ваш AI (`refine.where`), under it the part's list, and a line on where the text goes. Этот компьютер lists
  the local models (`llms_get`, `cue://llms`): the recommended one under "Для ваших языков" (the interface's language
  and the translation's), the others and the found ones ("CUE не проверял", `ModelsView.found`) under "Другие", the
  Hugging Face search last; a model downloads only when pressed, and pressed again on its way it stops. Ваш AI lists
  OpenCode then Codex as `agents_get` finds them, each time the tray or the page opens: the core clears the agents'
  marks, looks for each (OpenCode also by the server it announces), asks Codex `codex login status`, and lists
  OpenCode's models when it cleans, through a server already running (`OpenCodeSlot::running`: the one in use or the
  user's own; none is started for the page, and without one the drop-down offers "Быстрая · авто" alone); the page
  says "Проверяем…" meanwhile. A sign-in a run found failed (`Mark::Expired`) still shows as "войдите заново" though
  `login status` says "Logged in"; one only `login status` refused shows signed in once it says so. A found agent is chosen by its row; one not installed
  has its install page (`agent_install_link`, by id). The chosen agent's model is a drop-down (OpenCode's hundreds with
  a filter, "Быстрая · авто" first), then the fallback, "Почистить: <model>" offered only when the local model's file
  is here. A move to Ваш AI with no agent set takes the first found; with none it stays there, nothing chosen. The
  first page's value is what cleans now, the local download in percent, or in amber an agent to sign in to again or
  not found.
- **Your instructions** (`window/instructions.ts`, docs/specs/2026-11-08-no-keys-design.md, "Instructions page"): the
  Cleanup page's last row before its footer (not on Без ИИ), its line "Стандартные · можно поправить через AI",
  "Изменены вами", or in amber with "!" a broken file, opens a page one level under Cleanup. On top whose they are
  (CUE's standard, the user's own, with "Вернуть стандартные" → `instructions_reset`; a broken file in amber); the
  file's text in a box (`instructions_get`), saved when the box or the page is left, never while typing
  (`instructions_set`, at most 20 000 characters); "Изменить с AI" and its menu: "Открыть в <agent>" (the agent chosen
  for the cleanup when found, else the first found; `instructions_open`, `terminal.rs`: Windows Terminal's `wt -w new
  -d <folder>` when it is here, else the agent in a console window of its own; Codex `-C <folder> <task>`, OpenCode
  `--prompt <task> <folder>`; an argument a console or Windows Terminal would misread is never sent, the agent then
  opens without the task and the core puts it in the clipboard) and "Скопировать задание" (`instructions_copy`: the
  task with its folder, into the clipboard by the insert thread, as the history's Copy; a failure in amber); the path
  and AGENTS.md at the foot. A folder no terminal command line can carry (`InstructionsView.open` false) offers the copy
  alone. While the page is shown it asks `instructions_get` every 1.5 s, which reads a file saved since; each reading
  goes to the main window alone as `cue://instructions` ("Только что"). The box takes the new text unless it holds an
  edit not saved; when the file changed under such an edit, an amber "!" line asks "Взять из файла" or "Оставить моё".
  A save carries the text its edit started from (`base`), and the core refuses it when the file holds another now
  (`changed`), so an AI's save is never written over unasked; "Оставить моё" saves with no base. An edit refused as the
  page is left comes back with the choice. Each reading also starts the before and after: the core cleans the last
  dictation of the history (`history::last_dictation`, its text as heard, tidied as a dictation is) once more on the
  route the cleanup takes now, off its thread (`start_sample`; a newer reading replaces it; `InstructionsSampled`
  brings the result back to the core by its generation): through a loaded local model, or an agent through
  `Agents::sample` (never a marked one, never an OpenCode server started for it: `OpenCodeSlot::running` only). The
  event's `check` says where it stands: `checking` ("Проверяем на последней диктовке…"), `later` when none can come now
  (no dictation yet, a model not loaded, a marked agent, no OpenCode server, no answer: "Было и стало появятся после
  следующей диктовки.", made due again when the next dictation is in the history), `shown` with `sample` (the app,
  before, after), `off` with no AI. A reading a dictation took (`Instructions::text`) stays due (`sample_due`) for the
  page's next look. A dictation starting takes back a sample on the local model. Nothing of it goes to the history or
  usage.csv; the log says only that it was shown.
- **Microphone**: As in Windows or one of the devices `mic_list` names (with the one Windows uses by default), saved
  as `audio.device` and used from the next recording on (see Recording). A pinned one that is not connected shows as
  such, by the same match as `mic::pick`. While the page is shown, `mic_preview` sends the level of the chosen
  microphone (`micpreview`): it opens the device in shared mode, as a recording does, so a dictation meanwhile records
  as usual, and `cue-mic-preview` sends the window only the level, at most ~30 times a second, as `cue://mic-level`;
  the samples are dropped there. One preview at a time; it stops when the Microphone page is left or the tray closes,
  when the window's page unloads or loads again, and when the window is destroyed. It belongs to the window that
  started it (`micpreview::start(app, owner, device)`): its level and a failure go to that window alone
  (`emit_to`; the page listens on its own webview window), only that window's "off" or end stops it, and a preview
  the other window starts replaces it, sending the window it was taken from a level of 0. The Microphone page starts
  its preview again when the main window has the focus again.
- **Translation**: the target language, any of the 60 CUE knows; the core decides the home language again.
- **History**: how long the history is kept (`historyKeep`, see Retention; also the History row's value on the first
  page), how long recordings are kept (`historyAudio`: A day, A week, A month, Don't keep), each not set while
  unknown, and Clear history. A shorter period deletes the older ones at once; the page stops what plays and reads the
  list again.
- **General**: Start at sign-in and the interface language (Auto, English, Русский), which switches every window at
  once (see The Drop and tray cards).

## Welcome window

An ordinary window that walks through Hello, Key, Model, Microphone, First dictation and Done (`welcome_window`, label
`welcome`, the page `welcome.html`). It opens at startup when the key of the cleanup's cloud connection is missing from
Credential Manager (a local server needs none; a
store that cannot be read is no missing key: a log line says so), and with `--welcome`,
matched exactly among the program's arguments (`args_os`). The check runs in `setup` right after the tray and before
the hook starts, so the core holds the hotkey before any chord comes. As with the main window there is one at a time
(its own `CREATING`, with `main_window::decide`), created when opened, destroyed when closed and built on
`cue-welcome-window`. Windows draws its frame in the Windows theme with the close button alone; it is 560 × 430
logical px, centered, not resizable, and it takes the focus, so its first dictation is pasted into its own field.

- **The hold.** `Hold::new` sends `Input::WelcomeGate(true)` before the creating thread starts; its `Drop` sends
  `WelcomeGate(false)` and stops the welcome window's microphone preview. It lives in the window's `Destroyed`
  handler, taken once, so the window's end lets the hotkey go, and so does a failed `build()` or a thread that did not
  start. In between the page moves the gate with its steps (`welcome_step`): held from its load, let go on First
  dictation, held again on any other step. A chord the hold ignores brings the window forward (`bring_forward`, which
  never creates one; see Hotkey chord).
- **Key**: the settings tray's key row (`window/keys.ts`), read again on each entry, with a link to the Anthropic
  Console, the price line ("Recognition is free, on this computer; Anthropic bills only the cleanup.") and the privacy
  line ("Sound stays on this computer; text goes to Anthropic for the cleanup; the history stays on this computer
  only."). Next waits until the key is saved, has not failed its check and is not waiting for one, and no typed key
  waits in the field; leaving the step empties the field.
- **Model**: the settings tray's model block (`window/model.ts`), drawn anew on every `cue://model` (the page asks
  `model_get` once at its load). The model has been downloading since CUE started (see The speech model), so the step
  shows its progress, Retry when it failed, and moves on by itself when the model becomes ready while the step is
  shown. Next is always there: the Microphone step can be done meanwhile, and a first dictation made before the model
  is ready waits for it in Thinking like any other.
- **Microphone**: the settings tray's microphone rows (`window/mic.ts`), with the level of a preview of this window's
  own from entering the step to leaving it. `mic_access` reads whether Windows denies the microphone: `Value` of
  `CapabilityAccessManager\ConsentStore\microphone` and of its `NonPackaged` key, under HKLM and HKCU, where one
  `Deny` is enough (group policy is not read). A refusal, a `cue://mic-failed` or no microphone at all shows a hint
  with a button to the Windows privacy settings; when the window has the focus again the step reads both again with
  a new preview.
- **First dictation**: a text field kept in focus takes the paste of an ordinary dictation; the first text in it
  leads to Done 1.5 s later, with the number of words. Until then the field says it waits and the main button is not
  shown (its place kept).
- **Links** go by name (`open_link`: `anthropic`, `micPrivacy`, exact), each to a fixed address, opened with
  `ShellExecuteW` on `cue-open-link`, so no address of the page reaches the shell. `welcome_close` closes the window.
- The page takes its language from `settings_get` (English or Russian, `welcome/text.ts`) and changes no setting but
  the microphone. Its headings use Montserrat ExtraBold, its keycaps Bebas Neue.

## Tray icon

A monochrome caret mark drawn in code, light or dark to match the taskbar and sized for its DPI (both rechecked every
5 s). It shows a red lamp while recording and turns pale while paused. A double click of the left button opens the main
window, a single one does nothing, and the menu opens on the right button (a menu on the left click would take the
second click of a double click). The menu:

- **Open CUE**: opens the main window.
- **Pause for an hour** (checkmark): a recording in progress is discarded, hotkeys and the idle strip are off, and the
  pause ends by itself after an hour.
- **Start at sign-in** (checkmark): the Windows `Run` entry, re-read every 5 s in case it was changed elsewhere (the
  main window too).
- **Open log**: opens the log folder.
- **Quit**.

Only one CUE runs (`tauri_plugin_single_instance`): a second launch hands its arguments to the one running and
exits, and only `--welcome` does anything there, opening the welcome window or bringing it forward. An installed CUE
and a build of the repository are one app to Windows (the identifier `app.cue.dictation`): they share that single
instance, the data folders, the keys, the WebView profile and the Run value, so while one runs, launching the other
reaches the one running.

## Settings

`%APPDATA%\CUE\settings.json` is read once at startup; after that the core owns its copy, and the main and welcome
windows change it only through the core (`CoreEvent::Settings`, one `SettingChange` at a time, answered over a `oneshot`
with the settings as they are then). Missing fields take defaults, unknown fields are ignored, a UTF-8 BOM is accepted,
and any engine name reads as `parakeet` (`stt.engine` has that one value; the fields of earlier engines,
`soniox_model`, `soniox_async_model`, `language_hints` and the Soniox prices, are read past and never written again,
so the next write drops them). If the file cannot be read, CUE runs on defaults and never writes it during that
session: the window's changes hold until CUE quits, and the history's period is unknown (see Main window). Writes (the
hold-hint counter, the Drop's place, the main window's changes) always write the whole file from the core's copy, so
they never undo each other, and are atomic: a temp file in the same folder, then a rename. A change from the window
takes effect at once: a new translation target decides the home language again, a new interface language relabels the
tray menu and reloads the Drop and the card, a new period deletes the days and recordings it no longer keeps (all
recordings when they are turned off), and a new microphone is
opened by the next recording. The value already set, or one the settings refuse (a microphone name of more than 200
characters, a translation code CUE does not know, a period that is not one of the five), changes and writes nothing.

Default prices are not written to the file, so price fixes in code reach existing installs; once any price is edited,
the whole block is written. `ui.language` (`auto`, `en`, `ru`, in any case and with spaces trimmed; any other value
reads as `auto`) is likewise written only when it is not `auto`, `drop.position` (`bottom`, `top`, `left`, `right`, in
any case and with spaces trimmed; any other value, or a `drop` that is not an object, reads as `bottom`) only when it
is not `bottom`, and `dictionary` only when its list is not empty. `dictionary.terms` is kept as written; a value that
is not a list reads as an empty list and items that are not strings are skipped, so a typo there never makes the file
unreadable. `dictionary.learned` (`{ word, heard }` items, newest first) reads the same way and is written only when it
is not empty; `learn.enabled` is turned off only by `false` and written only then.
The `translate` fields (`target`, `home`, `speech`) go through `translate::known`: trimmed, lowercased, the first
subtag of a locale name (`ru-RU`, `zh-Hans-CN`), the aliases `nb`/`nn` → `no`, `fil` → `tl`, `iw` → `he`, `in` → `id`;
anything else (`auto`, an unknown code, a number) reads as nothing, that is the default, and a `translate` that is not
an object reads as the default section. A `target` of `en` reads as nothing too, so the default is never written
back, and the block is written only when one of its fields is set. The `ask` fields read the same way (`AskSettings`):
`enabled` and `web_search` are turned off only by `false` (anything else reads as on), `model` (of earlier versions)
is read past and never written, `max_searches` a whole number from 1 to 10 (kept, though no agent takes a limit); any other value, and the default itself, reads as nothing, an `ask`
that is not an object reads as the default section, and the block is written only when a field differs from its
default, so the file never becomes unreadable through it. Model names are settings, so a new model version needs no
code change. `audio.device` is a trimmed string; a blank value or one that is not a string reads as nothing (the
Windows microphone), and the block is written only with a device. `history.keep` (`settings::Period`) reads `forever`,
`year`, `month`, `week` and `off`, in any case and with spaces trimmed; a missing value is `month`, which is not
written; any other value (a typo, a later version's name, `null`, a number) leaves the period unknown and is written
back exactly as it was, so no save turns it into a period the user did not choose; a `history` that is not an object
reads as the default. `history.audio` reads `day`, `week`, `month` and `off` the same way (`Period<AudioKeep>`); a
missing value is `week`, which is not written; any other value, `forever` and `year` among them, is unknown and
written back as it was.

## Usage accounting

An attempt is a recording and its transcription; Retry starts a second attempt of the same dictation, and each gets
its own row in `%LOCALAPPDATA%\CUE\usage.csv`:
`ts, audio_ms, stt_engine, stt_path, refine_model, input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
latency_ms, outcome, mode, web_searches, words, day` — words and time, no money: CUE pays for nothing and holds no
price. A new row has `stt_engine` `parakeet` and `stt_path` `local`. Outcomes are `inserted`, `no_field`, `cancelled`,
`empty`, `failed`, `discarded` (written only when audio ran past 380 ms) and `answered` (an "Ask" answer shown in full, a
refusal too: nothing pasted). `mode` is `dictate`, `translate` or `ask` as the dictation's refine or question started
(for an attempt that ends before that, the mode it was recording in: the attempt follows a `ModeToggle` or `ModeChange`
the flow accepts). The Retry of a failed translation re-translates the saved text: a new row with no audio.
`refine_model` is `RefineSettings::usage_model` (`local:<id>`, `agent:<agent>:<model>`, `off`), for "Ask" too: it
goes through the cleanup's AI. `words` is how many words the attempt wrote (`usage::count_words` of the text of a dictation or translation; 0
for a question or one that wrote nothing) and `day` its local date.
Rows of earlier versions are 13, 16 or 18 columns wide and carry the dollars they were priced at (`stt_usd`,
`refine_usd`, `search_usd`), Soniox's paths and cache warm-up rows (`warmup`); they are read with the dollars passed
over (`usage::fields`). A file an earlier version started, its header not this version's, is migrated in place at
the first row this version writes (`usage::migrate`): every row is read by its own columns and written under the
15-column header without its dollars (a row from before Today gets an empty `words` and `day`, read as not known, as
before), the original is kept byte for byte as `usage-<day>.csv` (`-2`, `-3`… when taken), and the new file replaces
`usage.csv`, so Today and `cue-cli usage` show the same numbers. If any step fails, `usage.csv` stays as it was and
rows go after its old ones, which the readers read at any width. Latency runs from the second press to Ctrl+V (from Retry or Restore when used); the target for a
~30-word phrase is 1.5 s or less. Rows that fail to write, for example while the file is open in Excel, wait in memory
and go out before the next one. `cue-cli usage` sums a UTC month: dictations are `inserted` plus `no_field` rows (an
`answered` row is not one), the audio, average latency over `inserted` rows only, a separate average for recordings of
20 s or less, and the rows of each `mode` value, in the order they first appear.

An `ask` row carries the cleanup's AI in `refine_model` and the web searches Codex ran in `web_searches` (an agent's
tokens are its own, so its token columns stay 0); what a question used reaches the attempt at its end, or as a route
reports it (`Step::Spent`). Its latency runs from the second press to the first words for an `answered` row (the target for the first words of
an answer without a search is 2.5 s), and to Ctrl+V for a written text. A follow-up is an attempt of its own, and so is
the Retry of a failed question (no audio). An answer still printing when a follow-up starts waits aside (`answering`)
and gets its own row when it ends. Pasting a complete answer with its button adds no row.

## Installer and release build

`scripts/build-release.ps1` builds the release program and the installer into `target\dist` (`CARGO_TARGET_DIR`), never
into `target\release`. Paths of the build computer are cut out of the program: `--remap-path-prefix` writes the user
profile as `~` and the repository as `.`, passed in `CARGO_ENCODED_RUSTFLAGS` (cargo reads it before `RUSTFLAGS`); the
program refers to its PDB by the file name alone (rustc's default). Vite builds no source maps, and the script stops if
`app\dist` holds one. The shell's `CARGO_TARGET_DIR`, `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` are put back afterwards.
`-ScanBuild` builds the same installer without compression (a config file merged over `tauri.conf.json`) and moves it to
`target\dist\scan`; an installer already in `bundle\nsis` is moved aside for that build and put back. Both builds write
the program to `target\dist\release\cue.exe`, so the one there is the last build's: the shipped installer is built
last, and the program scanned is the one it holds.

The installer is NSIS, for the current user, in English and Russian, with no license page and no WebView2 download
(`webviewInstallMode: skip`: Windows 11 has the runtime). Its script, `app/src-tauri/nsis/installer.nsi`, is the
template of Tauri CLI 2.12.4 (pinned in `app/package.json`) with one line changed: the default folder is
`%LOCALAPPDATA%\Programs\CUE`, since Tauri's `%LOCALAPPDATA%\CUE` is CUE's data folder. `LICENSE`, `NOTICE` and both
OFL files go to the install folder's root. The publisher, `seymurglv`, names the registry key that keeps the install
folder, `HKCU\Software\seymurglv\CUE`; the installer and the uninstaller carry CUE's icon. `hooks.nsh` handles Start
at sign-in, the `Run` value `CUE`:

- an install with no install folder in that key (a first one) writes it, `"<folder>\cue.exe"`, and deletes the
  "turned off" mark Windows may keep under `Explorer\StartupApproved\Run`; a later install keeps the user's choice,
  and an update (`/UPDATE`) leaves the value alone;
- an uninstall reads the value before the running-app check, which may stop it; one that went through, with no
  `/UPDATE` and the data box clear, notes in that key whether the value was there (`StartAtSignIn`). An upgrade by
  hand runs the old uninstaller first (the installer's default choice), which deletes the value; the next install
  puts it back from the note, and deletes the note.

The uninstaller removes the program and, unless updating, its shortcuts and the `Run` value; its "Delete the application
data" box removes the WebView data (`%LOCALAPPDATA%\app.cue.dictation`) and the registry key, never
`%LOCALAPPDATA%\CUE` or `%APPDATA%\CUE`. The running-app check of both looks only at `<folder>\cue.exe`.

`cue-cli leak-scan <file or folder>...` looks through what is to leave the computer for anything private and prints
the file, the byte offset and the kind of each finding, never the finding. It reads what to look for into memory: the
saved keys, the history texts, questions and texts as heard of the last 31 days, the dictionary terms, the microphone
names (the pinned one and every one Windows lists), the Windows user name, computer name and profile path (also with
`/`, `\\` or URL-encoded), the git name and email, and the private folders that the repository's `.git/info/exclude`
names. Patterns add any user profile path, email addresses, `sk-ant-` / `sk-or-` keys, and recordings anywhere in a
file (an installer stores its members as they are): an Ogg page whose first packet is `OpusHead`, and `RIFF`, `WAVE`
and a format chunk that reads as one (a program's table of those names is none). Text files also get a check for long
random-looking strings, joined across `-` and `_`, and across `+` and `/` from 32 characters on, with hashes of known
kinds and Windows GUIDs in braces passed over; text is UTF-8, UTF-16LE (a closing NUL unit allowed) or mostly
printable ASCII. Text is looked for as UTF-8 and UTF-16LE. The public name is a finding only in a path or an address,
and not as the publisher's registry key (`Software\<name>`, then `\CUE` or a character no path holds). The exit code
is 0, 1 for findings, and 2 when something to look for or a given path cannot be read, or a given path holds no file.
Its panic hook prints the location alone. The installer script Tauri renders for a build
(`target\dist\release\nsis\x64\installer.nsi`) names build paths on its compile-time lines (`File`, `!include`, the
icon and path `!define`s); makensis stores none of them.

## Logging and privacy rules

- Logs go to `%LOCALAPPDATA%\CUE\logs\cue.log.YYYY-MM-DD` through a non-blocking writer at `info` level; old files are
  pruned at startup and daily (7 days, 10 MB).
- Logged: events, per-stage timings (`stt_ms`, `refine_ms`, `insert_ms`, `latency_ms`), outcomes and error kinds;
  at startup, how many dictionary terms are used and skipped, and the translation pair (two language codes); the
  target code of each translation; the names of the API keys an earlier version saved that were deleted at startup,
  or could not be (never a value); for "Ask", the kind and length of the
  selection read, whether the `Ctrl + C` fallback ran and what became of its Win+V copy (a kind), the number of turns
  of a request, where the answer opened (tray or card), how a request ended and its number of web searches; for the
  main window, its opening and closing, a command refused to another window (its label and the command), a setting
  change as codes and states (the periods' codes, whether a microphone is pinned, the interface language), how many old
  days of history and recordings were deleted, a recording kept (its length and size), dropped, or not kept (the
  error, no path), the length of an id a recording was asked for by when it is no entry id, where Paste again went
  (`pasted` or `clipboard`), a microphone preview started or stopped (the window, whether a microphone is pinned), and
  that a pinned microphone was not there; for the welcome window, at startup whether `--welcome` asked for it, its
  opening, closing and coming
  forward, each step's hold of the hotkey, whether Windows allows the microphone, and the name of a link that did not
  open; for the speech model, its state changes, the bytes and seconds of a download, the load time, the files a
  check deleted, each recognition's audio length, time and whether the other alphabet was masked out; for the
  correction watch, whether a correction was found, the length of
  its span and the number of candidates; for the instructions, that they were saved from the page (and how many
  characters), moved into their folder, copied as a task, or that an agent opened for them (the agent, the terminal,
  whether with the task), and that a before and after was shown. Never logged: dictation text, text before the caret,
  translations, dictionary terms and the words learned, the text of a field read after a paste, questions, answers,
  selected text, written texts, history entries, the instructions' text and the task, microphone names, the addresses
  and titles of sources, where the selection is on screen, audio, API keys, response bodies. `Effect` and `Input` are
  never logged with `{:?}`, since their `Debug` prints text.
- WebSocket handshake tracing is switched off by a fixed filter, since it would include the `Authorization` header.
- The panic hook is installed first and records only the thread and code location, never the message, which could
  contain dictated text. Debug output of focus results, of the selection read and copied, of the "Ask" context, of
  microphone frames and events, of history entries (their `raw` too) and their views, and of the microphone setting
  and its changes shows only lengths and counts; a recording's snapshot has no `Debug` at all.
- CUE holds no API key: the nine earlier versions saved in Credential Manager (service `CUE`: anthropic,
  openrouter, openai, gemini, deepseek, mistral, groq, xai, and soniox) are deleted at startup, off the core thread
  (`keys::forget_api_keys`); the habits' secret is the one entry kept there. No window, command or `cue-cli` reads or
  asks for a key.
- The history's text and its recordings go to the main window alone: its commands read the files, and new entries come
  over its own channel (see Main window). The microphone names of the "Another microphone" card go to the Drop alone,
  and the main and welcome windows get the device list.
- Sound never leaves this computer: recognition runs on it, Retry included, and only text goes to Anthropic. The
  model's download from Hugging Face carries no key and nothing of the user's. The last dictation (`last\`), the
  history (`history\`, with its recordings in `history\audio`) and bench recordings stay on this computer only.

## Bench

`cue-bench run` reads recordings saved by the app (`%LOCALAPPDATA%\CUE\bench\recordings`, a WAV plus a JSON with the
app name) and compares the speech models on this computer: every folder in `%LOCALAPPDATA%\CUE\models\` that
`Recognizer::load` accepts is one engine, named by its folder, the app's `parakeet-ultra-int8` first and the rest by
name (another export with the same file names, such as NVIDIA's own v3 from `istupakov/parakeet-tdt-0.6b-v3-onnx`,
goes next to it). A folder that does not load (a partial download, another export, a processor ONNX Runtime cannot
run on) is skipped with a console line, and with no model at all the bench says where to put one. Each model is
loaded once before the run and recognizes every whole recording in turn, on a blocking thread, the way the app does
(the pieces and the alphabet mask included); its latency is the model's own recognition time, its cost $0, and the
console line says when the other alphabet was masked out. When recognition starts, the connection to Anthropic is
warmed up as the app does on the second press. The text of `parakeet-ultra-int8` (or, if it failed or is not
there, of the first model that succeeded) is refined with the Anthropic model picked for the cleanup and every Claude model in the price list. It uses
the app's own code paths and `dictionary.terms`, so the request, sanity check and timeouts match. One warm-up call per
model is excluded from the report but counted in spend. A `--max-usd` budget stops new paid calls; `Ctrl+C` finishes
the step under way (a recognition cannot be interrupted, and a refine request under way is already paid for) and
leaves that recording out; a second one exits at once. `results.json` and `report.html` are rewritten after each
recording in `%LOCALAPPDATA%\CUE\bench\runs\<timestamp>`. The report is a single HTML page with blind A/B pairs for
recognition and for refine, scores, latency and cost per dictation, and saves the answers as `choices.json`; runs from
before the local models keep their cloud prices. The bench decides between Ultra and v3 on real recordings before a
release: if Ultra comes out worse, the pinned files in `cue_core::model` change to v3 and nothing else does.

`refine_compare` (an example in `cue-engines`) works on text instead of recordings: a phrase file
(`{id, app, raw, before_cursor?, to?}`) goes through OpenRouter models and the direct Haiku path with the app's prompt,
context and checks, sequentially, under a `--max-usd` budget. A phrase with `to` is translated, on the direct path
only (`claude::translate`, the translation check, the 4 s timeout); OpenRouter models skip it, and the skip is listed
in `out.json`. `out.json` holds every call (the outcome, `to`, and the text that would be pasted, or the raw transcript
when the outcome is not `refined`) and per-model medians, p90 and cost; the console gets only ids and numbers. A
translation that fails its check counts as `raw_fallback`, where the app pastes nothing and says "Couldn't translate";
on the direct path the model's own answer is not kept.

`ask_compare` (an example in `cue-engines`) is the bench of "Ask": a phrase file
(`{id, question, app, selection?, before_cursor?, then?, expect, mode?}`) is asked the way the app asks it
(`first_message` with today's date, `ask::run` with its deadlines and continuations, web search of 2 searches a request
unless `--no-search`, thinking as `--thinking adaptive|off` says, the app's default otherwise), one call at a time over
`--runs` rounds, under a `--max-usd` budget. A phrase's `then` is a follow-up asked after the first answer, and the
follow-up is what is scored. `out.json` holds every call (the decision, the mode of a write, its text or the answer's
Markdown, the first words, the total time, the searches, whether text came before the first search, the cost) and a
summary: right decisions and modes, the median
first words of warm calls with and without a search (the very first call, with an empty cache, is left out), and the
cost. `out.json` is written before the first call, so a path that cannot be written stops the run before anything is
paid, and again after every call, so a run cut short keeps its results; a `--model` without a default price is refused,
since the budget would count its tokens as $0. The console gets only ids, kinds and numbers. The phrase set and the
scoring script (format and content checks) live outside the repository. The run of 2026-10-29 (Haiku 5.5, 30 phrases, 5
runs) decided 99 % right, with the first words of warm calls in 1.7 s without a search and 5.8 s with one, for $0.19.
