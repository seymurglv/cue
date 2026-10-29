# How CUE works

CUE is a Tauri 2 app: a Rust core that runs in the background and makes every decision, and a small TypeScript UI
(the Drop with its tray cards, and the "Ask" card at the selection) rendered by WebView2. The windows only draw the
state the core sends and report button clicks back. The dictation logic is a pure state machine, so it is tested
without windows, a microphone, a network or a real clock.

## Crates

- **`cue-core`** (`app/crates/core`): no Windows APIs, no network, no clock (time is passed in).
  - `flow`: the dictation state machine `Flow`, one for every mode. `handle(input, now_ms)` returns a list of
    `Effect`s.
  - `ui`: `UiState`, `UiPhase`, `Mode`, `ErrorCause`, `TrayKind`, `UiAction`, shared with TypeScript as JSON.
  - `refine`: the system prompt, the dictionary block, the user message, timeout and token budget, the sanity check,
    Markdown stripping, list tidying, and the join space before a paste.
  - `translate`: the languages Soniox recognizes, the speech-language tally, the translation direction, the
    instruction block, the timeout and the translation check.
  - `ask`: the "Ask" instruction and user message, the selection limit, where a written text goes, the buttons of an
    answer, its sources, the plain text for pasting, and `AnswerView`, the answer as the windows draw it.
  - `audio`: resampling to 16 kHz mono, voice level, the "microphone is silent" detector.
  - `settings`, `usage` (CSV rows, costs, monthly summary), `paths`.
- **`cue-engines`** (`app/crates/engines`): `stt` (session facade, ~100 ms chunks, batch fallback), `soniox`
  (WebSocket stream) and `soniox::batch` (REST, with cleanup of uploads), `claude` (refine and translation requests
  and responses, the cache warm-up), `ask` (the streaming "Ask" request, its event parser and decision, `run` and its
  cache warm-up `warm`), `keys` (Windows Credential Manager), `check` (free key probes), `error`. `openrouter` holds
  the server-sent-events reader "Ask" uses too, and otherwise serves only the `refine_compare` example.
- **`cue-platform-win`** (`app/crates/platform-win`): `hook` and `chord` (keyboard, and the mouse while a Shift that
  switched the mode is held), `input` (`SendInput`, modifier state), `clipboard` (paste, and the `Ctrl + C` fallback
  of "Ask"), `focus` (UI Automation: the field, the text before the caret, the selection), `selection` (when to copy
  the selection, where it is on screen), `cardplace` (where the "Ask" card goes), `mic` (cpal), `wav`, `dropwin`
  (window styles, moves, the z-order guard), `placement` (the four places, the nearest place, monitors), `power`.
- **`cue`** (`app/src-tauri`): startup, the `runtime` actor (with `runtime::ask`, the conversation of an open answer),
  `ipc`, `drop_window`, `card_window`, `tray` and `trayicon`, `logging`.
- **`app/ui`**: `main.ts` (events, commands, frame loop), `controller.ts` (`DropController`), `trays.ts`,
  `answer.ts` (the "Ask" answer, one view for the tray and the card), `markdown.ts` (its safe Markdown), `card.ts` and
  `cardPage.ts` (the card window), `strings.ts`, and `design/` (Drop, tray card, answer and animation code, styles, the
  Bebas Neue font).
- **`cue-cli`** (`app/crates/cli`) and **`cue-bench`** (`bench`): developer tools.

## Threads

- **Main thread** (Tauri event loop): windows, the tray menu, tray icon redraws.
- **`cue-core`**: the core actor, `Runtime::run`. All events arrive as `CoreEvent`s on one unbounded channel:
  hotkeys, Drop actions, microphone frames, engine results, timers, sleep and lock. It runs on its own thread
  (`block_on`) because it makes short blocking Windows calls (UI Automation up to 150 ms, Credential Manager).
  Network calls and timers are tokio tasks that send their results back as events. A panic while handling one event
  is caught and logged, and the core keeps running.
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
- **`cue-card`**: moves, shows and hides the "Ask" card window.
- **`cue-mic`** (cpal) with `cue-mic-events` forwarding frames; `cue-mic-stop` closes a microphone off the core thread.
- **`cue-power`**: a hidden top-level window for `WM_POWERBROADCAST` and session lock notifications.
- **`cue-hit-test`** (one for the Drop and one for the card, ~60 Hz while the window is shown), **`cue-placement`**
  (every 0.5 s; every 8 ms while the Drop is dragged or snapping into place), **`cue-tray-watch`** (every 5 s),
  **`cue-log-prune`** (daily), **`cue-open-source`** (a short thread for each click on a source).

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
`FinishStt` or `TranscribeSaved`. A task that crashes still reports: transcription as a network error (so Retry is
offered), refine as the raw text, a question as a failure with Retry, paste as "no field".

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
shows the `translate` error (`claudeKey` for a key, balance or limit problem) with Retry. That Retry emits
`RefineAgain` with the saved transcript: the text is translated again, the audio is not transcribed again.

**"Ask" in the flow.** A question is recorded and transcribed like a dictation; then, instead of `Refine`, the flow
emits `Ask { id, question, follow_up }` and stays in Thinking (the Drop's three dots) until the answer has something to
show. The runtime reports back with four inputs: `AskOpened` (the first real step: a search began, its results came,
or the first words), `AskAnswered` (the answer is complete; a refusal, or a written text shown instead of written,
too), `AskWrite { how }` (the model wrote text for the field and the runtime holds it; `PasteHow` is `AtCaret`,
`Replace` or `Below`) and `AskFailed { error }` (`AskError::Key` or `Failed`). An answer that opens or ends moves the
flow to the `Answer` phase: the Drop rests (it looks idle), the answer shows in the tray or the card, `Esc` stays
swallowed, and a chord records a follow-up. A write goes to `Inserting` at once with `AskPaste { id, how }` and ends
the conversation (`CloseAnswer`); the answer's buttons (Insert, Insert below, Replace) do the same for an answer that
is complete on screen, and Retry after a failure emits `AskAgain { id }`. `AbortAsk { id }` drops the request in
flight. A `Key` failure closes the answer and shows the Drop's "Claude key problem" without Retry; any other failure
keeps the answer open with the error and Retry, and the question is kept.

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
  under the user's hands, and CUE never injects input while paused: the runtime asks `Flow::shows_write` and shows such
  a text in the answer instead. A pause closes an open answer (with a follow-up under way and the request in flight),
  since `Esc` does not reach CUE then.
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
- Language identification is on for every dictation, stream and batch alike: the mode can still change after the
  stream config is sent, and Soniox prices it into the hourly rate. Each token carries its language, and the speech
  language is the one of the most words among the final tokens (`translate::LanguageTally`): a word starts at the
  first letter after whitespace and belongs to the language of the token it starts in, so a word split over several
  tokens counts once and a long foreign name does not outweigh a short phrase ("Открой Telegram" is Russian).
  Punctuation starts no word; in scripts written without spaces (Chinese, Japanese, Thai, Lao, Khmer, Myanmar) every
  character counts as a word; tokens without a known language are skipped; a tie goes to the language heard first.
- The language hints are `stt.language_hints`, except for a translation with `translate.speech` fixed: then that code
  is the only hint, on the stream, the batch fallback and Retry. The stream takes the mode the dictation started in.
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
  JSON schema that forces `{"text": …}`. Timeout: 3 s plus 1 s per 100 words.
- The user's dictionary (`dictionary.terms`) goes in a second system block after the unchanged prompt: a short rule
  (write a listed name exactly as listed when the speech clearly means it, even if the transcript spells it
  differently, splits it or mixes alphabets; add none the user did not say and change no other word because of the
  list) and the terms, one per line. The cache mark sits on the last system block, so the cached prefix covers the
  prompt and the list together; editing the list costs one cache write. Entries are trimmed, and empty, multi-line,
  overlong (over 60 characters) and repeated ones are dropped, at most 200 kept. With no usable term the request is
  byte for byte the one without a dictionary. Soniox never gets the terms: it bills context tokens on every dictation.
- When the app is unknown (neither the query at the start of recording nor the one after the second press named it),
  the request carries `<app>unknown app</app>`; Soniox gets no app context in that case.
- The answer is used only when it is a complete `{"text": …}` object: a refusal, `max_tokens`, a timeout, a network
  error or a bad response all mean the raw transcript is pasted with the normal "Done". Explicit Markdown the speaker
  did not dictate is stripped, and stray commas and semicolons at the end of list items are removed.
- **Sanity check** on what would be pasted: lengths are compared in characters without numbers (digits and number
  words), and only for transcripts of at least four words with two or more that are not numbers; the ratio must stay
  within `min_ratio`…`max_ratio` (0.35–1.25). At least `min_overlap` (70 %) of the result's non-number words must
  appear in the transcript; words of `dictionary.terms` count as appearing, so a name fixed in a short phrase
  ("открой гитхаб" → "Открой GitHub.") is not taken for an invention. An empty result always fails. A failed check pastes the raw transcript.
- A key, balance or limit problem at Anthropic pastes the raw transcript and then shows "Claude key problem" without
  Retry.

### Translation

- **Direction** (`translate::output_language`). The speech language is `translate.speech` when fixed, else the one
  Soniox heard most (see Transcription). Speech that is not in `translate.target` goes to the target; speech in the
  target goes to the home language; unknown speech goes to the target; with no home language, speech in the target is
  only cleaned up. The home language (`translate::resolve_home`) is decided once at startup: `translate.home`, else
  the Windows display language, else the first language hint, skipping any equal to the target. Settings example:
  `"translate": { "target": "en", "home": "ru", "speech": "auto" }` sends Russian (or German) speech to English and
  English speech to Russian.
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
  nothing: the flow shows "Couldn't translate" with Retry, which translates the saved transcript again. No Anthropic
  key, or a key, balance or limit problem, shows "Claude key problem" with Retry instead. The log gets one line with the
  reason and the outcome, and the target code when a translation starts; never the text.

### Cache warm-up

When a dictation starts (on `StartStt`, in any mode but "Ask") and no refine or warm-up has started in the last
4 minutes (`CACHE_WARM_AFTER_MS`: the 5-minute cache lifetime counts from the start of a request, and the recording
takes time), the runtime sends a warm-up (`claude::warm`): the refine request with the same system blocks and
dictionary, a one-character transcript, no app, no text before the caret and `max_tokens: 1` (`warm_body`; the API
refuses `max_tokens: 0` together with the output format, and the format must stay for the prefix to match).
Dictation and translation share the prefix, so one warm-up serves both. A model whose thinking cannot be turned off
(Opus 5.5) gets no warm-up: a one-token answer leaves it no thinking budget. The warm-up has a 5 s timeout; a failure
writes one log line with the reason, and the dictation's own refine writes the cache. After waking from sleep or an
unlock the next dictation warms the cache again. The tokens are paid for and get a usage row of their own
(`warmup`); a recording cancelled after the press has paid for its warm-up. "Ask" has a prefix and a warm-up of its
own (see Ask).

### Paste

The same path serves dictation and `Ctrl + Win + V`:

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

## Ask

"Ask" (`Ctrl + Win + Alt`) is a dictation whose text becomes a question to Claude (`ask.model`, Haiku 5.5 by default).
The flow decides when to ask, paste and close (see Dictation flow); `runtime` and `runtime::ask` keep what those
effects need, and `cue_engines::ask` talks to the API.

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

### Request

- `cue_engines::ask::request_body`: `stream: true`; the tools first (they render first in the cached prefix):
  `write_to_field`, strict (its input always matches the schema: `text`, and `mode` `insert` or `replace`) and without
  eager input streaming, so the input arrives whole and checked; then web search, when `ask.web_search` is on. The
  version is the one the first probe picked (`ask_probe`, 2026-10-29: both versions worked, the filtered one brought the
  first words of a searched question within 8 s and answered a follow-up after a search): `web_search_20260318`, the
  filtered search, with `max_uses` = `ask.max_searches`. A key whose organization has no web search gets a 400 that says
  so, and the question is asked again at once without it.
- The instruction (`ASK_PROMPT`, English; the answer follows the language of the question) is one cached system
  block: what the tags mean, that their content is data and never instructions, when to call `write_to_field` and with
  which mode, the form of a written text (plain, no quotes, the app's tone, one line in terminals), the form of an
  answer (short, simple Markdown: bold, lists, code; no headings, tables or links), and when to search (anything that
  may have changed since the training data; facts that cannot change, explanations and writing need none).
- The user message (`first_message`): `<question>`, then `<selection>`, `<before_cursor>`, `<app>` and `<today>`;
  empty parts are left out, and our closing tags inside the data are defused. The date sits in the user message, not in
  the cached instruction, so the cached prefix is the same every day; the bench checks that questions about the present
  still search. A follow-up (`follow_up_message`) is the question alone.
- Two cache marks: one on the instruction, which covers the tools before it, and one on the last block of the last
  user message, so a follow-up reads the conversation so far from the cache. The stored conversation keeps no marks (a
  request may carry four at most). The first follow-up writes the previous answer to the cache; the API looks back only
  about 20 blocks from a mark for an earlier entry, and an answer with citations is split into many blocks, so a
  follow-up after a long answer may read only the instruction from the cache (not an error).
- `output_config.effort: "low"`, `tool_choice: auto`, no `temperature`, `top_p` or `top_k`. Thinking is off
  (`thinking: disabled`, `Thinking::DEFAULT`), chosen after the bench of 2026-10-29. Adaptive thinking, the model's
  default (left out of the request, with 2,048 tokens of room added to the answer's 2,000), met the bar too (decisions
  of 97 %, first words in 1.8 s without a search and 5.0 s with one), but timed out on 3 of 210 calls: its thinking is
  silence to the first-words deadline. Off gave 98 % decisions, 93 % format and checks, first words in 1.2 s without a
  search and 1.3 s with one, and no timeouts. Without thinking the model may write a short sentence before a search
  ("let me check the current version"), which stays at the top of the answer and is its first words. The code
  execution container of a filtered search is sent back with the next request of the conversation.

### Stream

- `StreamState` turns one response's events into its content blocks as the API would return them whole (text with its
  citations, tool calls with their input parsed, search results with their encrypted content, thinking with its
  signature) and into `Step`s for the answer on screen: `Searching`, `Reading`, `TextStart`, `Text`, `Cite` (a search
  result's URL and title; without a title, its site), `WriteStart`, and `Spent` (what the question has cost so far,
  sent as the counts change). Events and deltas it does not know are skipped. The first words (`Step::is_first_words`)
  are text that is not blank, or the start of a write.
- The decision (`decide`): a refusal (`stop_reason: refusal`) wins over anything the stream already had; a
  `write_to_field` call counts only complete and valid (not cut at `max_tokens`, a mode from the schema, some text);
  otherwise any text is an answer; nothing to show is a `Malformed` failure.
- `run`: a paused turn (`pause_turn`) is continued with its blocks and the same tools, at most 3 times and only while
  the question's searches are fewer than `max_uses`, so a question runs at most `2 × max_searches − 1` searches. A
  write cut at `max_tokens` (a long email) is asked again once with 4,000 more tokens of room, under deadlines of its
  own from when the retry is sent. The deadlines: the first words within 8 s, or within 20 s once a search began; the
  whole answer, continuations included, within 30 s. The rest of an answered response's body is read in the background
  (5 s at most), so the connection can be reused.
- Failures (`AskFailure`, one log line with the kind): no key; 401, 402 or 403, or a 400 about the credit balance (a
  key problem: `AskError::Key`, the Drop's "Claude key problem" without Retry); 404, the model in `ask.model` does not
  exist (`NoModel`, the log says to check `settings.json`); a timeout; the network; another HTTP status (429 and 5xx
  too: a rate limit passes, so Retry may help, unlike a key problem); an `error` event in the stream (its type); a
  stream against the protocol. Everything but a key problem shows "Couldn't get an answer" with Retry, under the words
  already shown, which stay. A failure keeps what the question spent, the response that failed part-way included: the
  API reports a response's searches only at its end, so until then each search the stream started counts
  (`StreamState::usage`), and a question closed or cut after it searched still pays for the search; once the end
  reports the count, that one is used (it leaves out a search the API deferred and never ran).

### Conversation and the answer

- `runtime::ask::Conversation` lives from the first question until `CloseAnswer`, and is then forgotten. It keeps the
  answered exchanges, the question in flight, the container, the sources (numbered in the order a URL is first cited,
  across the conversation) and `AnswerView`, the answer as the windows draw it. A first question is asked with its
  context; once an exchange has succeeded, a follow-up goes alone. The model's blocks go back as they came, except
  a search the API never ran (it defers a search when the model also calls a client tool, and a follow-up that leaves
  it unanswered is refused). A follow-up after a written text that was shown answers that call first, with a
  `tool_result` saying the text was shown instead of written. A refusal and an empty turn never join the conversation,
  and a failure keeps its question for Retry.
- The request runs in a task of its own (`ask_task`, with a generation, as transcription and refine do): the steps and
  the end come back as events of that generation, so a dropped or replaced request's never reach the conversation. The
  view goes to the windows (`cue://answer`) at once for a new turn, an end, an opening or a follow-up row, and at most
  every 50 ms while words stream (`ViewGate`); the flow hears `AskOpened` once per request (`ShownOnce`).
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

### Cache warm-up of "Ask"

At the "Ask" chord (and when Alt picks "Ask"), and at a follow-up's chord, when no "Ask" request or warm-up started in
the last 4 minutes, the runtime sends `ask::warm`: the request's prefix (the same tools, instruction, thinking, effort
and tool choice) with a one-character placeholder, no stream, `max_tokens: 0` (the probe found it accepted), and the
cache mark on the instruction only. It has a 5 s timeout; a failure writes one log line, and the question's own request
writes the cache. Its tokens get a `warmup` usage row priced with `ask.model`. The refine warm-up is not sent for
"Ask", and after sleep both warm again on the next chord.

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
(`SHQueryUserNotificationState`) and while paused. The window is always centered on the anchor at its current size,
never at the size it would have at the target monitor's scale. The anchor lies a few dozen px inside the target monitor,
so most of a window centered on it is on that monitor whatever the two scales; Windows takes a window's scale from the
monitor it shares the most area with, so it gives the window the target's scale and size, and the next check centers it
again. Placed at the predicted size, most of the window could stay on the old monitor, whose scale it would then keep,
so it would never arrive. While the window is hidden the thread only waits for the next check.

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
grow). Window → core commands: `drop_action` (`cancel`, `finish`, `undo`, `retry`, `dismiss`, `trayAction`,
`trayClose`, and the answer's `answerInsert`, `answerBelow`, `answerReplace`, `answerCopy`, `answerRetry`,
`answerClose`), `answer_source` (the number of a source chip clicked), `drop_hit_rects`, `drop_drag` (`start`, `end`),
`drop_ready`, after which the core resends the current state, card, `idle-hidden`, place and open answer, and, from the
card window, `card_hit_rects` and `card_ready`, after which the core resends the state, the card's place and the open
answer. The place also comes in an initialization script, so the first frame is already laid out for it.

The windows get the text of an "Ask" question and its answer, the card's quote of the selection (`quote_of`, 40
characters at most), and the titles and sites of its sources, to draw them; never a source's address. A click on a
source sends only its number, and the core opens the page with `ShellExecuteW` (see Ask). No dictation text crosses this
boundary.

One size: the Drop is 36 px high in every state (`--h` in `drop.css`; `DROP_SIZE` in `drop_window.rs` places it, and a
test keeps them equal). The circle states are 36 × 36. Everything inside is the original 40 px drawing scaled by
`--h / 40`: the mark 0.6 h (the same while recording), buttons 0.7 h, small icons 0.5 h, keys and label text 0.9 of
the drawing, paddings and gaps alike. The idle strip is 36 × 7 (upright at the sides), so starting a dictation grows it
into a circle of the same diameter. Tray cards keep their own sizes.

Phases: `idle` (a thin strip; on hover the mark and the hotkey), `start`, `listen` (voice bars and no timer; cancel
and done buttons on hover), `think` (three dots), `done` (a check, gone after 1.5 s), `empty`, `cancel` (Restore with
a 3.6 s ring), `error` (amber "!", the cause on hover, Retry when available), `answer` (an "Ask" answer is open and
nothing records: the Drop rests as in `idle`). The frame loop sleeps when nothing moves; with reduced motion turned on
in Windows, states change instantly.

The mode shows while recording: in a translation the mark of `start` takes the translation shape and `listen` adds
the arrows icon next to the voice bars, and "Ask" has an icon of its own; a mode change re-enters the phase, so the icon
changes at once. The error of a failed translation reads "Couldn't translate" with the network cause icon and Retry.
The idle Drop shows no mode, and its hint stays `Ctrl + Win`.

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
`settings.json` (`en` or `ru`) overrides Windows. The core makes that choice once at startup, so the tray menu, the Drop
and the card always match.

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

`%APPDATA%\CUE\settings.json` is read once at startup. Missing fields take defaults, unknown fields are ignored, a UTF-8
BOM is accepted, and any engine name reads as `soniox`. If the file cannot be read, CUE runs on defaults and never
writes it during that session. Writes (the hold-hint counter, the Drop's place) always write the whole file from the
core's copy, so they never undo each other, and are atomic: a temp file in the same folder, then a rename. Default
prices are not written to the file, so price fixes in code reach existing installs; once any price is edited, the whole
block is written. `ui.language` (`auto`, `en`, `ru`, in any case and with spaces trimmed; any other value reads as
`auto`) is likewise written only when it is not `auto`, `drop.position` (`bottom`, `top`, `left`, `right`, in any case
and with spaces trimmed; any other value, or a `drop` that is not an object, reads as `bottom`) only when it is not
`bottom`, and `dictionary` only when its list is not empty. `dictionary.terms` is kept as written; a value that is not a
list reads as an empty list and items that are not strings are skipped, so a typo there never makes the file unreadable.
The `translate` fields (`target`, `home`, `speech`) go through `translate::known`: trimmed, lowercased, the first
subtag of a locale name (`ru-RU`, `zh-Hans-CN`), the aliases `nb`/`nn` → `no`, `fil` → `tl`, `iw` → `he`, `in` → `id`;
anything else (`auto`, an unknown code, a number) reads as nothing, that is the default, and a `translate` that is not
an object reads as the default section. A `target` of `en` reads as nothing too, so the default is never written
back, and the block is written only when one of its fields is set. The `ask` fields read the same way (`AskSettings`):
`enabled` and `web_search` are turned off only by `false` (anything else reads as on), `model` is a trimmed non-empty
string, `max_searches` a whole number from 1 to 10; any other value, and the default itself, reads as nothing, an `ask`
that is not an object reads as the default section, and the block is written only when a field differs from its
default, so the file never becomes unreadable through it. Model names are settings, so a new model version needs no
code change.

## Usage accounting

An attempt is a recording and its transcription; Retry starts a second attempt of the same dictation, and each gets
its own row in `%LOCALAPPDATA%\CUE\usage.csv`:
`ts, audio_ms, stt_engine, stt_path, stt_usd, refine_model, input_tokens, cache_read_tokens, cache_write_tokens,
output_tokens, refine_usd, latency_ms, outcome, mode, web_searches, search_usd`. `stt_path` is `stream`,
`stream+batch` or `batch`; outcomes are `inserted`, `no_field`, `cancelled`, `empty`, `failed`, `discarded` (written
only when audio ran past 380 ms) and `answered` (an "Ask" answer shown in full, a refusal too: paid for, nothing
pasted). `mode` is `dictate`, `translate` or `ask` as the dictation's refine or question started (for an attempt that
ends before that, the mode it was recording in: the attempt follows a `ModeToggle` or `ModeChange` the flow accepts),
or `warmup`. The Retry of a failed translation re-translates the saved text: a new row with no audio. A cache warm-up
is a row of its own: its tokens and cost, no audio, latency 0, outcome `discarded`, mode `warmup`, and the model it
warmed (`refine.model`, or `ask.model` for "Ask"). Rows written before the last three columns existed read as
dictation, and the header of an existing file is never rewritten (new files get the 16-column header). Latency runs
from the second press to Ctrl+V (from Retry or Restore when used); the target for a ~30-word phrase is 1.5 s or less.
Rows that fail to write, for example while the file is open in Excel, wait in memory and go out before the next one.
`cue-cli usage` sums a UTC month: dictations are `inserted` plus `no_field` rows (an `answered` row is not one),
average latency counts only `inserted` rows, and a separate average covers recordings of 20 s or less. The cost includes
failed attempts and warm-ups; web search is shown only when there was some, and the rows and dollars of each `mode`
value follow, in the order they first appear.

An `ask` row carries `ask.model` in `refine_model`, the tokens of all the question's requests (continuations, a widened
write, and a request aborted by `Esc` before Restore asked again), and its searches in `web_searches`, priced in
`search_usd` (searches × `pricing.web_search_usd`). What a question has spent reaches the attempt as the stream goes
(`Step::Spent`), so an answer closed or aborted mid-stream, a failed one and one cut by a deadline still write their
cost: the tokens reported so far, and every search the stream started. Its latency runs from the second press to the first words for an `answered` row (the target for the first words of
an answer without a search is 2.5 s), and to Ctrl+V for a written text. A follow-up is an attempt of its own, and so is
the Retry of a failed question (no audio). An answer still printing when a follow-up starts waits aside (`answering`)
and gets its own row when it ends. Pasting a complete answer with its button adds no row.

## Logging and privacy rules

- Logs go to `%LOCALAPPDATA%\CUE\logs\cue.log.YYYY-MM-DD` through a non-blocking writer at `info` level; old files are
  pruned at startup and daily (7 days, 10 MB).
- Logged: events, per-stage timings (`stt_ms`, `refine_ms`, `insert_ms`, `latency_ms`), outcomes and error kinds;
  at startup, how many dictionary terms are used and skipped, and the translation pair (two language codes); the
  target code of each translation; the token counts of each cache warm-up; for "Ask", the kind and length of the
  selection read, whether the `Ctrl + C` fallback ran and what became of its Win+V copy (a kind), the number of turns
  of a request, where the answer opened (tray or card), how a request ended and its number of web searches. Never
  logged: dictation text, text before the caret, translations, dictionary terms, questions, answers, selected text,
  written texts, the addresses and titles of sources, where the selection is on screen, audio, API keys, response
  bodies. `Effect` and `Input` are never logged with `{:?}`, since their `Debug` prints text.
- WebSocket handshake tracing is switched off by a fixed filter, since it would include the `Authorization` header.
- The panic hook is installed first and records only the thread and code location, never the message, which could
  contain dictated text. Debug output of focus results, of the selection read and copied, of the "Ask" context and of
  microphone frames shows only lengths.
- Keys are read from Credential Manager on use, appear in the CLI only masked, and are cut out of engine error texts.
- Soniox uploads are deleted after use. The last dictation (`last\`) and bench recordings stay on this computer only.

## Bench

`cue-bench run` reads recordings saved by the app (`%LOCALAPPDATA%\CUE\bench\recordings`, a WAV plus a JSON with the
app name), and for each one runs the Soniox stream at real-time pace and the batch model at the same time, then
refines the stream text (or the batch text if the stream failed) with `refine.model` and every model in the price
list. It uses the app's own code paths and `dictionary.terms`, so the request, sanity check and timeouts match. One
warm-up call per model is excluded from the report but counted in spend. A `--max-usd` budget stops new paid calls;
`Ctrl+C` finishes the work already sent, and before exiting the bench waits for Soniox cleanup. `results.json` and
`report.html` are rewritten after each recording in `%LOCALAPPDATA%\CUE\bench\runs\<timestamp>`. The report is a
single HTML page with blind A/B pairs for transcription and for refine, scores, latency and cost per dictation, and
saves the answers as `choices.json`.

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
