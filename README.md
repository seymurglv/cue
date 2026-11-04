# CUE

CUE is a voice-dictation app for Windows 11. Press `Ctrl + Win`, speak, press the keys again (or hold them while you
speak), and the text appears at the caret in whatever app you are typing in: fillers and false starts removed,
punctuation added, your own words kept, in Russian or English. Speech is recognized on your computer by a speech model
CUE downloads once (Parakeet Ultra), and the text is tidied up by a language model: by default Claude, from Anthropic,
with your own key or one of seven other cloud connections, or, free, by a small model CUE downloads and runs on
your computer. Add
`Shift` and the text comes out translated; add `Alt` instead and ask a question out loud, and the answer appears next
to your work. CUE runs quietly in the background: a small indicator at the bottom of the screen (the Drop) shows what
it is doing, and an icon next to the clock opens the main window, with the history of everything CUE wrote and the
settings.

## Install

Download the installer, `CUE_<version>_x64-setup.exe`, from the latest release:
[github.com/seymurglv/cue/releases/latest](https://github.com/seymurglv/cue/releases/latest).

1. Run the installer, `CUE_<version>_x64-setup.exe`. CUE is not code-signed, so Windows shows "Windows protected your
   PC": click **More info**, then **Run anyway**. Windows asks this again for every new version of CUE. If Smart App
   Control is on (Windows Security → App & browser control), it blocks unsigned apps like CUE, and CUE will not run.
2. The installer, in English or Russian, needs no administrator rights and asks for no license agreement. It installs
   CUE for you alone, into `%LOCALAPPDATA%\Programs\CUE`, adds it to the Start menu and downloads nothing: the
   WebView2 runtime that CUE's windows use comes with Windows 11. The speech model is not in the installer: CUE
   downloads it on its first start (see below).
3. Its last page starts CUE and offers a desktop shortcut: both boxes are ticked, so untick **Create desktop
   shortcut** if you do not want one. A first install also turns on **Start at sign-in**; installing CUE again, or a
   newer version, keeps your choice. Ticking **Delete the application data** in any uninstall, including the one an
   upgrade runs first, makes the next install a first one. You can turn Start at sign-in off in the menu of the icon
   next to the clock, or in the main window's General settings.

On its first start, and on every start while the key of the cleanup's connection is missing (Anthropic's, unless you
chose another in the settings), CUE opens a welcome window: Hello, Key, Model, Microphone, First dictation and Done.
Its Key step asks for the Anthropic key, which "Ask" always uses; the key of another connection goes into the main
window's **Cleanup** settings. With the cleanup on this computer no key is needed: the welcome window has no Key step
and does not open by itself. While it is open, `Ctrl + Win` works only at the First dictation step; at the others
it brings the window forward. If you close it before the key is saved, it comes back at the next start. To open it
again at any time, even while CUE is running, press `Win + R` and run `"%LOCALAPPDATA%\Programs\CUE\cue.exe" --welcome`.

At the same first start CUE downloads its speech model, Parakeet Ultra, about 670 MB, from Hugging Face into
`%LOCALAPPDATA%\CUE\models`, in the background. The Model step shows its progress and moves on by itself when it is
ready; you can go on to the other steps meanwhile. A download that stops (no network, the computer asleep) goes on
from where it stopped when you press **Retry** (in the welcome window, the main window's **Speech recognition**
settings, or the Drop). After that, recognition needs no network. Another speech model can be chosen later in the
**Speech recognition** settings (see Features). The model needs a processor with AVX2 (2013 and later); on one without
it, the Drop says "Processor not supported".

## Keys

CUE uses your own account at the services it talks to, which bill you directly for what you use:

- **Anthropic** cleans the text up and translates it, by default with Claude Haiku 5.5, and answers your questions
  with Claude whatever the cleanup goes through. Sign in at the [Anthropic Console](https://console.anthropic.com),
  set up billing and create an API key.
- Instead of Anthropic, the main window's **Cleanup** settings can send the cleanup and translation through another
  connection, each with a recommended model, chosen until you pick another. In the cloud, each with a key of its own:
  OpenRouter, OpenAI, Google Gemini, DeepSeek, Mistral, Groq and xAI (their models are listed in
  [API keys](#api-keys)).
- Or, with no key and no account, **Runs on: This computer** in the Cleanup settings: CUE downloads a small language
  model from Hugging Face and runs it on the processor, free, the text never leaving this computer. Three are on its
  list: **Qwen3 4B Instruct** (recommended, 2.5 GB), **Gemma 4 E2B** (3.1 GB) and **Qwen3 1.7B** (1.1 GB, the fastest);
  others can be found with **Search Hugging Face** (one GGUF file of up to 6 GB). A processor is slower than the cloud:
  on the computer CUE is built on (8 performance cores), Qwen3 4B takes 3–8 seconds for a short dictation and about
  half a minute for each minute of speech; a laptop takes two or three times longer. The model reads CUE's
  instructions while you speak and runs on the processor's performance cores, leaving the rest to other programs. A
  long dictation is cleaned in pieces of about 80 words, ending where sentences end, each with the text before it as
  context, so there is no limit on its length; Esc while it is cleaned cancels it. Qwen3 1.7B often leaves the text as
  recognized, and only Qwen3 4B translates. Until the model has downloaded and loaded, dictations are pasted as
  recognized. The model is in memory only while the cleanup runs on this computer: with **Cloud** none is loaded.
  Downloaded models are listed on the Cleanup page either way and can be deleted there, the one in use too (until
  another is chosen, dictations are then pasted as recognized). Ollama and LM Studio are no longer used.

Speech recognition runs on your computer and costs nothing. Earlier versions of CUE also used Soniox, with a key of
its own: this version does not, and deletes that key from Windows Credential Manager at its first start.

Paste the Anthropic key into the welcome window, or any cloud connection's key into the main window's **Cleanup**
settings, and press **Check**: it saves the key and sends a free test request. Keys are kept in Windows Credential
Manager, each under its connection's id; CUE's windows show only a mask of them.

Dictation and translation cost about $0.0001 a dictation with the default cleanup model, Claude Haiku 5.5: under $1 a
month for an hour of speech a day in dictations of about 13 seconds, cache warm-ups included. Claude Sonnet 5.5 costs
about 15 to 20 times as much. All of it is the cleanup. Other services bill at their own prices, which the Cleanup
settings' list of models shows per million tokens; a model on this computer costs nothing. Questions asked with
`Ctrl + Win + Alt` cost more, and each web search one of them runs adds $0.01. CUE writes down what every dictation and
question cost, on your computer only (`usage.csv`, see [Costs](#costs)).

## Privacy

- **Sound stays on this computer.** Each dictation, translation and question is recognized here, by the speech model;
  no recording is sent anywhere, **Retry** included.
- **The cleanup's connection** receives the text: what the model recognized, the app name and up to 500 characters
  before the caret. That is Anthropic by default, or the service you chose in the Cleanup settings; with the cleanup
  on this computer the text stays here, and so do translations and habits' rules.
- **Anthropic** receives your questions to "Ask", with the text you selected, whatever the cleanup goes through.
  Nothing is read from password fields.
- **Your corrections stay on this computer.** With **Learn from me** on (it is by default), CUE reads the field it
  pasted into for up to a minute after each dictation or translation, to notice a word you corrected. The text stays in
  memory: nothing of it is written to disk, to the log or sent anywhere. A word goes into the dictionary only when you
  press **+** on the Drop; from then on the cleanup's connection gets it with the dictionary, with how it was heard.
  When you correct the same thing a few times, CUE keeps it as a habit: the words around the correction (at most 40
  characters before and after it) are the only text of a correction it writes to disk. What it counts on the way is
  kept by a hash made with a random secret of this computer, which Windows Credential Manager keeps beside the keys
  (never in `settings.json`), so a copy of the file does not tell which words were counted. The cleanup's connection
  receives the words around the correction once, with the kind of the correction and the app's name, to word the
  habit's rule; it already received that text when you dictated it. From then on the rule goes with every cleanup
  request of a dictation in that app, or in every app once the habit holds everywhere. Password fields, CUE's own
  windows and "Ask" answers are never read.
- The microphone is open only while CUE records, and while a window shows its level for you to check it.
- Your history, with the recordings of what you said for the period you choose, settings, logs and the copy of your
  last dictation stay on this computer. CUE has no telemetry: it connects only to the cleanup's connection, to
  Anthropic for "Ask", and to Hugging Face to download speech models and the cleanup's local models (a plain download
  of fixed files: nothing of yours goes with it) and to search them when you type in a model list's search. The installer sends nothing.

Each service has its own policy on how long it keeps what it receives. Every detail is in
[Privacy in detail](#privacy-in-detail).

## Uninstall

Quit CUE from the menu of its icon next to the clock, then open Windows **Settings → Apps → Installed apps**, find
**CUE** and choose **Uninstall**. This removes the program, its Start menu entry, its desktop shortcut and Start at
sign-in. Your history, settings, speech models and keys stay, so installing CUE again picks up where you left off. The
uninstaller's **Delete the application data** box removes only the cache of CUE's windows (the last folder below) and
the installer's registry key (the last line). To remove everything:

- the folder `%LOCALAPPDATA%\CUE`: the history and its recordings, the last dictation, the logs, the cost records and
  the speech models (`models`, about 670 MB each);
- the folder `%APPDATA%\CUE`: the settings;
- the folder `%LOCALAPPDATA%\app.cue.dictation`: the cache of CUE's windows;
- the keys: in **Control Panel → Credential Manager → Windows Credentials**, remove under Generic Credentials each
  one named after a connection: `anthropic.CUE`, and those of the others you saved a key for, `openrouter.CUE`,
  `openai.CUE`, `gemini.CUE`, `deepseek.CUE`, `mistral.CUE`, `groq.CUE` and `xai.CUE` (the `soniox.CUE` of earlier
  versions is already gone: CUE deletes it at its first start), and `habits-secret.CUE`, the secret the habits' counts
  are hashed with;
- the registry key `HKEY_CURRENT_USER\Software\seymurglv`, named after CUE's publisher, where the installer remembers
  the folder CUE was installed in and whether Start at sign-in was on: in Registry Editor.

To open a folder, paste its path into the address bar of File Explorer.

---

The rest of this file describes CUE in detail: every feature and setting, what is sent where, and how to build CUE
from source.

## Features

- **Three chords.** `Ctrl + Win` (left or right keys, any order) starts dictation; `Ctrl + Win + Shift` (any order too)
  starts a translated one; `Ctrl + Win + Alt` asks a question (see "Ask" below). Tap and release: CUE records until you
  press the chord again (with or without `Shift`; the finishing chord does not change the mode). Hold for 0.6 s or
  longer: recording ends when you let go. While CUE records, a press of `Shift` (left or right; on its own after a tap,
  or added while the starting chord is still held) switches between dictation and translation (a question, to a
  translation and back), and the Drop changes its icon at once: a second press takes it back. A `Shift` pressed
  together with other keys (a capital letter, a shortcut) or with a mouse click (`Shift` + click extends a selection)
  does not switch the mode; letting go of `Shift` changes nothing, and outside a recording `Shift` does nothing.
  `Alt` picks "Ask" only together with `Ctrl + Win` held, pressed before them or while they are still down; after a
  tap it changes nothing. `Ctrl + Win` with `Shift` and `Alt` together is the Office key: CUE leaves it to Windows, and
  `Ctrl + Win + Alt` too when "Ask" is off (`ask.enabled`).
- **Ask** (`Ctrl + Win + Alt`). Ask out loud and the answer appears in words, in the language you asked in:
  - With nothing selected it opens in a panel that grows out of the Drop, toward the middle of the screen. With text
    selected it opens in a card next to the selection, its arrow pointing at the text; when CUE cannot tell where the
    selection is on screen, or there is no room for the card, it opens in the panel instead.
  - The answer opens with its first real step: a web search starting, its sources coming in, or the first words. Until
    then the Drop shows that it is thinking. Answers from a web search number their sources, and a chip under the
    answer opens each one in your browser.
  - The buttons depend on what was selected: **Insert below**, **Replace** and **Copy** for text you can edit (no
    **Replace** for a selection of more than 8,000 characters, which reaches the model as its start and its end); only
    **Copy** for text you cannot (a received message, a web page); **Insert** and **Copy** with nothing selected. They
    wait until the answer has finished printing, and paste only into the window you asked in: with another window in
    front, the text stays in the clipboard and the "Nowhere to paste" card offers Copy. An answer pasted with a button
    becomes the last result, so `Ctrl + Win + V` pastes it again.
  - `Ctrl + Win` while the answer is open records a follow-up in the same conversation, even while the answer is
    still printing (the follow-up goes once it has finished). A follow-up CUE could not make out shows **Retry**, and
    the answer stays. `Esc` or the cross closes the answer, and the conversation is forgotten.
  - "Write…", "fix…", "translate…", "reply…": the text goes straight into the field, as a dictation would. It replaces
    the selection when you asked to change it, and otherwise goes at the caret, or on a new line below the selection
    (in a terminal always at the caret, on one line). Code and Markdown already in the selection or before the caret
    are kept as written. A text written about something you cannot edit, a change of a terminal's text or of a
    selection of more than 8,000 characters, a text that comes while a follow-up is on its way or while CUE is paused,
    and one that comes once the window you asked in is no longer in front are shown in the answer instead, with its
    buttons (only **Copy** for text you cannot edit).
  - A question about the present ("the latest version of…", "the exchange rate today") makes the model search the web
    first (`ask.web_search`); facts that do not change, explanations and writing need no search.
- **Translation** (`Ctrl + Win + Shift`). Speech that is not in your target language (`translate.target`, English
  by default) is translated into it; speech in the target language goes to your home language (`translate.home`, by
  default the Windows display language). The speech language is told by its alphabet: the language of the pair
  written in Cyrillic or in Latin letters, whichever your sentence is in, so "Открой Telegram" counts as Russian
  (`translate.speech` fixes it instead). The cleanup of dictation applies too (fillers, lists, numbers, one line in
  terminals), and names from `dictionary.terms`, product names and code are kept as written; a self-correction,
  though, can come back turned around (see Known limitations). If the translation fails (no network, a timeout, or an
  answer that does not look like a translation), nothing is pasted: the Drop shows "Couldn't translate", and **Retry**
  translates the saved text again.
- **Text at the caret in any app**, pasted through the clipboard. Your previous clipboard content comes back about
  half a second later, and dictated text is kept out of Windows clipboard history and the cloud clipboard.
- **Speech recognized on this computer** by Parakeet Ultra, Moondream's post-trained version of NVIDIA's Parakeet TDT
  0.6B v3, which writes punctuation and capital letters itself. The model runs on half the processor's threads (2 to
  8). The text comes once you finish, but a long dictation is recognized in pieces while you speak: once 15 s are
  waiting, a piece is cut at the quietest moment between 10 and 15 s and recognized in the background, so only the
  last seconds are left when you finish, and the cleanup follows. While recording, the Drop shows your voice level,
  never text. A dictation made before the model has finished downloading or loading is kept: the Drop thinks until the
  model is ready, and the text is pasted then. When the model becomes ready after a download, a quiet card says so:
  "Model is ready".
- **A choice of speech models** on the Model page (Settings → Speech recognition, the model's row): Parakeet Ultra
  (the default), NVIDIA's own Parakeet TDT 0.6B v3 (25 languages, as Ultra) and v2 (English only), or a Parakeet TDT
  model found on Hugging Face from the page's search, when it is exported to int8 ONNX the way CUE runs it (a model
  that is not is shown pale, with the reason). A found model is pinned at the repository's current commit when you
  choose it, and every model's files are checked by SHA-256. A model not yet here is downloaded first, if the drive
  has room for it and 10 % more; the model in use keeps recognizing until the new one has loaded, then CUE switches to
  it without a restart. Downloaded models stay, so going back is instant, and one not in use can be deleted.
- **Russian or English, one language a dictation.** Names and terms in the other alphabet stay as you said them
  ("запусти cargo test в Visual Studio Code", "call Марисоль"); when the model writes part of a Russian sentence in
  English (or the other way round), CUE recognizes it again in the dictation's own alphabet.
- **Cleanup:** fillers, stutters and abandoned starts are removed, "Wednesday, no, Thursday" becomes "Thursday",
  punctuation and paragraphs are added, lists go on separate lines, numbers are written as digits. The Default style,
  CUE's own, also makes the text read as written rather than spoken: colloquial forms take their standard spelling
  ("нету" → "нет"), words that only fill the speech go ("в принципе", "basically"), and a clumsy or repeated phrase is
  reworded, while the meaning, every fact and request, your person and tone stay, names and terms stay as said, and
  nothing is added. The Custom style puts your own instructions (up to 2,000 characters) in place of CUE's: they win
  where they differ from its rules, and blank ones clean up as Default. The style does not change a translation. The
  tone adapts to the target app; in terminals the result always stays on one line, so nothing runs by itself.
- **Safe fallbacks:** if cleanup is slow, refused or looks wrong, or there is no network, the raw transcript is pasted
  instead. If recognition fails, the recording is kept and **Retry** recognizes it again; if the model did not
  download, **Retry** downloads the rest of it first.
- **`Esc`** cancels; **Restore** (for 3.6 s) brings the dictation back, and asks again a question cancelled before its
  answer showed, with the same selection. **`Ctrl + Win + V`** pastes the last result again, a translation included.
- **Nowhere to paste?** When the focus is clearly not a text field (desktop, File Explorer's file list), the text
  stays in the clipboard and a card offers to copy it.
- **Recording limit:** at 8:00 a card says one minute is left, at 9:00 recording ends on its own and the text is pasted
  as usual. The Drop shows no timer while recording, only the voice bars.
- **The Drop stays where you put it.** Hover over the thin strip to open it, then drag it: when you let go it snaps
  to the nearest of four places: the bottom (the default) or the top edge, centered, or the left or the right edge,
  halfway up. The place is remembered (`drop.position` in `settings.json`). A plain click without moving does what it
  always did, and dragging never takes the focus from the app you are typing in. The Drop can be dragged only while
  idle, and it always stays on top of ordinary windows.
- **The main window** opens from the tray icon: **Open CUE** in its menu, or a double click. It is one ordinary window
  in the Windows theme; opening it again brings it forward, and closing it leaves CUE running. Links at the top,
  **Today**, **History** and **Dictionary**, scroll to their sections.
  - **Today:** how many words you dictated today and how many minutes of typing that saved (at 40 words a minute), a
    bar for each of the last 30 days (hover one for its date and words), your words a minute, how many days in a row
    you have dictated, and your words in all. Counted are dictations and translations that wrote text. When the total
    passes a round number (1,000, 10,000, 100,000 words…), a pill says so once, for that day. Rows of `usage.csv`
    written before CUE 0.1.2 kept no word count: their words are estimated from the length of their audio.
  - **Dictionary:** the names and terms CUE must write right, as chips, the newest first (`dictionary.terms`, see
    Settings). **+ Word** opens a field: type a word and press `Enter` to add it (a repeat in any case, an empty one or
    one longer than 60 characters is refused); the cross on a chip, shown on hover, removes it. The cleanup uses the
    change from the next dictation on. A word added from a suggestion (see "Words from your corrections") has a green
    book, and its hint says how CUE heard it.
  - **History:** every result of CUE, newest first, with its time, mode and app: dictations, translations, "Ask"
    answers (the question and the answer) and texts "Ask" wrote into a field; a translation and "Ask" show their mode
    as a small icon, a plain dictation none. The list stays short: today shows its latest 7 rows and the rest under
    **N more today**; every earlier day is one line under **Earlier** (its name, its two most used apps and how many
    entries it has) that opens in place, and older days come with **Show earlier**. Three or more entries in a row from one app, each within 10 minutes of the one before, make
    a stack: its newest entry on top, the app and the count on a chip that opens it. A click on an entry (or `Enter`)
    shows how you said it: a dictation as it was recognized, the words CUE removed struck out and the ones it added or
    changed in green; a translation's speech; an "Ask" question; and its recording, with play and pause, while it is
    kept. Every entry opens to its whole text: one from before CUE kept how it was said says *not kept*, and a
    dictation CUE wrote just as it was heard says so. A recording plays only when you press it, and
    pauses when CUE starts recording. The search, out of the magnifier, looks through the texts, the questions and how
    they were said; an entry found only in how it was said opens by itself. Hover over a row for **Copy** and **Paste
    again**: the text goes into the window right under the main window (usually the one you were in before opening
    it), or into the clipboard when there is none. `Ctrl + Win + V`, a paste with an answer's button, a refusal and a
    failure add no entry, and **Paste again** does not change the last result.
  - **Settings** (the gear): a tray whose first page lists six pages, each with its value. **Speech recognition**
    (the model in use and its state: Ready, Downloading with its percent, its megabytes and a bar, or Did not download
    with **Retry**, which goes on from where the download stopped; the model's row opens the Model page, see "A choice
    of speech models"), **Cleanup** (the style, Default or Custom with a field for your instructions; **Runs on**, This
    computer or Cloud; on this computer, the list of local models as the Model page lists speech models, the
    recommended one marked, with **Search Hugging Face**; in the cloud, the connection, a card that opens the list of
    eight, each with a lamp that is green once its key is saved, its key with **Check**, which saves it and sends a
    free test request, the window showing only its mask, and the model, each with its price per million tokens, the
    recommended one marked; its value on the first page is This computer or the connection's name, in amber "Model not
    downloaded", the download's percent, or "No *name* key" when the chosen cloud connection has none), **Microphone** (As in Windows, or a device of
    your own, with a live level), **Languages** (the interface language, and the target language of
    `Ctrl + Win + Shift`, any of the 60), **History** (**Keep history**: Forever, A year, A month (the default), A week
    or Don't keep, older days deleted on their own; **Keep recordings**: A day, A week (the default), A month or Don't
    keep, never longer than the history itself; a shorter period deletes the older ones at once; **Clear history**
    needs a second press), **General** (Start at sign-in). A change takes effect at once, without a restart, and is
    written to `settings.json`.
- **Words from your corrections** (**Learn from me** in the Habits section's heading, on by default). After CUE pastes a
  dictation or a translation, it reads the field for up to a minute, to see whether you changed a word. When you write
  a name or a term in place of what CUE heard ("Марисоль" → "Marisol"), a couple of seconds after you stop typing the
  Drop grows into a capsule with the word and **+**; a ring around **+** runs out in 8 s, and hovering pauses it. **+**
  adds the word to the dictionary with how it was heard, and turns into **Undo**, which takes it out again (6 s). A
  suggestion not pressed in time folds into a green dot next to the Drop, which stays until the next dictation starts;
  hovering the Drop opens the capsule again. The cleanup then gets the word with how it was heard ("Marisol — sounds
  like «Марисоль»"). Turned off, no field is read. Only fields that give their text to Windows can be read: where one
  does not (Telegram, some fields inside web pages), nothing is offered.
- **Habits** (the main window's Habits section, after the Dictionary). The same watch notices how you correct the text
  of a dictation in an app: the end mark added or taken off, the first letter's case, a word written another way
  ("здравствуйте" → "добрый день"), numbers as digits, a dash in place of a hyphen. When the same correction has been
  made three times in one app, CUE keeps it as a habit and says so once: a "New habit" card grows out of the Drop with
  the rule in a sentence ("In Telegram — lower case and no full stop") and **Undo**, which forgets the habit and never
  learns that correction again. The card closes by itself after 7 s, and hovering it pauses that; it waits until the
  Drop is at rest, and never opens together with a dictionary suggestion (the one found first goes first). From then on
  the habit goes into the cleanup of that app's dictations, to write the text your way (translations and "Ask" take
  none). When all of a habit's corrections come together in one dictation three times in another app, the habit holds in
  every app: its row says so, and no card opens. One of them alone there counts for that app, and can become that app's
  own habit. The same correction learned in two apps makes two habits, one in each; once it holds in every app, they
  are one. When you go back on a habit in its app three times (the full stop it takes off added again, the capital
  back, the old words back), the habit changes: the old one goes, the new one is learned, and a "Habit changed" card
  says the new rule; its **Undo** brings the old habit back as it was and never learns the new one (when the
  correction only added to a habit already kept, **Undo** brings that habit back as it was, too, and forgets
  nothing). A habit of every app
  changes the same way, from three times in any one app, and the change holds everywhere. Two opposite habits never live
  together, so a habit with an opposite in some app does not spread to every app. The section lists the habits, the
  newest first, each with its rule, the example, the app, how many times it was corrected and a switch; **Forget** shows
  on hover, and **Forget all** (a second press confirms it) forgets them all and what was counted. With **Learn from
  me**, whose switch sits in the section's heading, turned off, the rows are muted and nothing is learned or applied;
  with no cleanup connected (the chosen connection is a cloud one with no key), a line says that habits work when the
  cleanup is on. In the History, an entry lists a habit whose effect the cleanup brought (its text shows it, and what
  you said did not) under how you said it, as a chip with the habit's short name ("без точки"); a habit forgotten later
  keeps its chip as it was.
- **A pinned microphone.** A microphone chosen in the settings stays CUE's even when Windows changes its default (a
  headset plugged in, say). When it is not connected, CUE records from the Windows microphone, and a card says so once:
  "*name* not found — recording from *name*".
- **Tray icon:** a double click opens the main window, and the right button opens the menu: Open CUE, Pause for an
  hour, Start at sign-in, Open log, Quit. A single left click does nothing.
- **Interface language:** English, or Russian when the Windows display language is Russian; the main window's Languages
  settings (or `ui.language` in `settings.json`) can fix it to either, and the tray menu, the Drop and the main window
  switch at once.
- **Usage accounting:** every dictation's cost is recorded locally, with its mode; `cue-cli usage` shows a monthly
  summary.
- **Comparison bench** (a developer tool): runs your saved recordings through every speech model in CUE's models
  folder and through several Claude models, and builds a blind A/B report. A second one (`ask_compare`) asks a set of
  questions the way "Ask" does and measures its decisions, speed and cost.

Not in this version: changing the hotkeys, text appearing while you speak, speech models other than Parakeet TDT in
int8 ONNX, recognition on the graphics card, a vocabulary for speech recognition itself (the dictionary guides only
the cleanup, see Settings), "Ask" through a service other than Anthropic, macOS.

## Requirements for building

- Windows 11 (x64).
- Rust stable with the MSVC toolchain, 1.90 or newer (edition 2024):
  `rustup default stable-x86_64-pc-windows-msvc`.
- Visual Studio Build Tools with the "Desktop development with C++" workload.
- Microsoft Edge WebView2 Runtime (included with Windows 11).
- Node.js 24 or newer, with npm (`"node": ">=24"` in `app/package.json`).
- A processor with AVX2, for the speech model.
- An Anthropic API key (for "Ask" and the benches, and for the cleanup by default), or another connection's key or a
  local server for the cleanup alone. The tests need none.

The first build downloads ONNX Runtime's prebuilt library, which the `ort` crate links into the program, into
`%LOCALAPPDATA%\ort.pyke.io`, once.

## API keys

Keys live in Windows Credential Manager under the service name `CUE`, each under its connection's id (`anthropic`,
`openrouter`, `openai`, `gemini`, `deepseek`, `mistral`, `groq`, `xai`; Credential Manager shows them as
`anthropic.CUE` and so on). Beside them, `habits-secret` (`habits-secret.CUE`) keeps the random secret the habits'
counts are hashed with (see [Privacy in detail](#privacy-in-detail)); CUE makes it itself. They are never written to
files, never printed in full and never logged. Enter a key once
in the main window (Settings → Cleanup: choose the connection, paste the key and press **Check**), the Anthropic key
also in the welcome window, or with the command-line tool (input is hidden):

```powershell
cargo run -p cue-cli -- keys set anthropic   # or openrouter, openai, gemini, deepseek, mistral, groq, xai
cargo run -p cue-cli -- keys show            # masked values, e.g. sk-ant…f3a9
cargo run -p cue-cli -- keys check           # a free test request per key
```

The test request is Anthropic's and OpenRouter's own check, and for the other services a request for their list of
models (10 s): it works, the key is refused, or it could not be checked. A local server is checked the same way without
a key, within 3 s. The cleanup's connections and their models (the first of each is the recommended one, the default
until another is picked; prices in USD per million input / output tokens, as CUE counts them):

| Id | Connection | Models |
|---|---|---|
| `anthropic` | Anthropic (the default) | Claude Haiku 5.5 `claude-haiku-5-5` $0.10 / $0.50; Claude Sonnet 5.5 `claude-sonnet-5-5` $2 / $10 |
| `openrouter` | OpenRouter | Claude Haiku 5.5 `anthropic/claude-haiku-5.5` $0.10 / $0.50; Claude Sonnet 5.5 `anthropic/claude-sonnet-5.5` $2 / $10; GPT-6 Luna `openai/gpt-6-luna` $0.10 / $0.50 |
| `openai` | OpenAI | GPT-6 Luna `gpt-6-luna` $0.10 / $0.50; GPT-6.1 Terra `gpt-6.1-terra` $1 / $5 |
| `gemini` | Google Gemini | Gemini 3.1 Flash-Lite `gemini-3.1-flash-lite` $0.25 / $1.50; Gemini 3.5 Flash-Lite `gemini-3.5-flash-lite` $0.30 / $2.50 |
| `deepseek` | DeepSeek | DeepSeek Flash `deepseek-flash` $0.30 / $1.20 |
| `mistral` | Mistral | Mistral Small 4 `mistral-small-2603` $0.15 / $0.60; Ministral 8B `ministral-8b-2512` $0.15 / $0.15 |
| `groq` | Groq | gpt-oss 120B `openai/gpt-oss-120b` $0.15 / $0.60; gpt-oss 20B `openai/gpt-oss-20b` $0.075 / $0.30 |
| `xai` | xAI | Grok 4.3 `grok-4.3` $1.25 / $2.50 |

Anthropic gets the Messages API request it always had: Claude Haiku 5.5 at effort `medium` with thinking off, Claude
Sonnet 5.5 at effort `low`. Every other connection gets one OpenAI-compatible Chat Completions request, with the key as
a bearer token (none for a local server), reasoning turned down as far as the service allows, and an answer held to
the same `{"text": …}` form. A missing, refused or unfunded key, a limit, no network, a timeout or an unusable answer
pastes the raw text, as before. `refine_compare` (see Repository layout) still compares cleanup models through
OpenRouter's key, and the direct path through Anthropic's.

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
menu to launch it with Windows. Only one CUE runs at a time: a second launch hands its `--welcome`, if any, to the
one running and exits. This build and an installed CUE are one app to Windows (one identifier), so they share that
single instance, the data folders and keys, the cache of the windows and the Start at sign-in value. While this build
runs, the installer's **Run CUE** and an installed `cue.exe --welcome` reach it instead, without a word: to try an
installed CUE, quit this one first, and check which program runs with `(Get-Process cue).Path`.

`DirectML.dll` goes beside the program: ONNX Runtime, linked into it, is built with DirectML and loads the DLL at
start, though CUE runs the model on the processor. Cargo copies it next to `cue.exe` with the build, and the installer
carries it there too.

The installer is built into `target\dist`, so `target\release` is left alone. Before an installer leaves this
computer, build it twice, the uncompressed one for the scan first, and look through both and the program for anything
private:

```powershell
scripts\build-release.ps1 -ScanBuild
scripts\build-release.ps1
cargo run -p cue-cli -- leak-scan target\dist\release\cue.exe target\dist\scan app\dist
```

The script prints the installer's path, `target\dist\release\bundle\nsis\CUE_<version>_x64-setup.exe`. It cuts this
computer's paths out of the program (rustc writes the user profile as `~` and the repository as `.`, by flags passed
in `CARGO_ENCODED_RUSTFLAGS`) and puts the shell's `CARGO_TARGET_DIR`, `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` back
afterwards. The first installer build downloads NSIS and Tauri's plugin for it from Tauri's releases on GitHub into
`%LOCALAPPDATA%\tauri`, once. The installer script, `app/src-tauri/nsis/installer.nsi`, is the template of Tauri CLI
2.12.4 with one line changed, the default folder (Tauri's own is `%LOCALAPPDATA%\CUE`, CUE's data folder), and
`hooks.nsh` next to it turns on Start at sign-in at the first install. The CLI is pinned to that version in
`app/package.json`: after updating it, compare its template with this one again.

`-ScanBuild` makes the same installer without compression, in `target\dist\scan`, so that the scan reads the files
inside it as they are. Both builds write the installer to the same place: an installer already there is moved aside
for the scan build and put back, so either order keeps both. The program, `target\dist\release\cue.exe`, is the last
build's: build the shipped installer last, as above, and the program scanned is the one it holds. The programs inside
the two installers differ only in the time stamps of the build and the id of its debug-symbol file. The shipped
installer is compressed, so the scan cannot read inside it, and a few bytes of its compressed data can look like an
email address by chance: before it leaves this computer, unpack it by hand and scan what it holds.

`leak-scan` looks for paths into a user profile (also URL-encoded), the Windows user name and computer name, email
addresses and API keys, for exact copies of the keys saved in Credential Manager, the history of the last 31 days, the
dictionary terms, the habits' examples (`before` and `after`), the words of their replacements and the short names the
cleanup wrote (not the rules, nor a short name the template wrote: those are mostly CUE's own words), the microphone
names, the git name and email and the private folders that `.git/info/exclude` names (its lines ending with `/`), for
recordings (Ogg Opus and WAV, also stored inside an installer), and, in text files, for long random-looking strings. It
prints the file, the offset and the kind of each finding, never the finding itself. It exits with 1 when it found
anything, and with 2 when it could not read what to look for (a key, the settings, the history, the microphone list,
git) or a given path, or a given path holds no file. The installer script that Tauri writes for the build
(`target\dist\release\nsis\x64\installer.nsi`) names files of this computer in its `File`, `!include`, icon and path
`!define` lines: makensis reads them and stores none of those paths, which the scan of the uncompressed installer
confirms.

Tests and lints:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cd app; npm test; npm run build
```

The tests need no microphone, network, API keys or speech model (network code, the model's download included, is
tested against a local fake server). Three ignored tests in `cue-engines` check the real model: run them with
`cargo test -p cue-engines -- --ignored` and `CUE_MODEL_DIR` set to a model folder (one of them also takes a
recording in `CUE_TEST_WAV`, and the log-mel one the export's reference preprocessor `nemo128.onnx` in `CUE_NEMO128`).

## Settings

`%APPDATA%\CUE\settings.json`. CUE reads it at startup, so restart CUE after editing it by hand. A change made in the
main window takes effect at once and is written to the file right away. A missing file or missing fields mean
defaults; unknown fields are ignored. If the file cannot be read or parsed, CUE runs on defaults for that session and
does not overwrite it: changes made in the main window then last until CUE quits (the window says so), and the
history is neither written nor cleaned up until a period is chosen in the window. The defaults:

```json
{
  "stt": { "engine": "parakeet", "model": "parakeet-ultra-int8" },
  "refine": {
    "connection": "anthropic",
    "style": "default",
    "context_chars": 500,
    "min_ratio": 0.35,
    "max_ratio": 1.25,
    "min_overlap": 0.7
  },
  "hold_hints_left": 2,
  "bench": { "save_recordings": false }
}
```

- `stt.engine` has a single value, `parakeet`, the speech model on this computer; any other name reads as it. The
  fields of earlier versions (`soniox_model`, `soniox_async_model`, `language_hints`, and the Soniox prices under
  `pricing`) are ignored and dropped the next time CUE writes the file.
- `stt.model`: the speech model's id, the name of its folder under `models`: `parakeet-ultra-int8` (the default),
  `parakeet-v3-int8`, `parakeet-v2-int8`, or `hf--<owner>--<repository>` (lower case) for a model found on Hugging
  Face, whose folder keeps its `manifest.json`. The Model page writes it once the new model has loaded. An id CUE
  cannot find reads as Parakeet Ultra.
- `refine`: the cleanup and translation.
  - `connection` (default `anthropic`): one of the ids in [API keys](#api-keys); case and surrounding spaces do not
    matter, and any other value reads as `anthropic`.
  - `models`: the model picked for a connection, by connection id, kept only where it is not the recommended one.
    Example: `"models": { "anthropic": "claude-sonnet-5-5" }`. A model CUE does not list for that connection reads as
    the recommended one.
  - `runs` (default `cloud`): `"cloud"`, through `connection`, or `"local"`, a model on this computer. A `connection`
    of `ollama` or `lmstudio` (earlier versions) reads as `local`; their `addresses` are read past and not written
    again.
  - `local`: the local model's id (`llm--<owner>--<repository>--<quantization>`), written once it has loaded; absent,
    the recommended one, Qwen3 4B Instruct.
  - `style`: `"default"`, CUE's own cleanup, or `"custom"`, your instructions in `custom`. Case and surrounding spaces
    do not matter; any other value, the `"careful"` and `"bold"` of earlier versions included, reads as `"default"`.
  - `custom`: the instructions of the Custom style, kept only when not empty; the main window takes up to 2,000
    characters and trims them. Blank instructions clean up as Default.
  - `context_chars`: how much text before the caret is sent as context, for cleanup and for "Ask" alike (0 turns it
    off). `min_ratio`, `max_ratio`, `min_overlap`: the sanity check (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)).

  The `model` and `level` of earlier versions are ignored and dropped the next time CUE writes the file, so a file that
  named Sonnet cleans up with the default, Claude Haiku 5.5, until another model is picked.
- `hold_hints_left`: how many more times the "Press and release" card appears after a held chord.
- `bench.save_recordings`: copy each successful dictation's audio and app name for the bench.
- `ui.language`: the language of the tray menu, the Drop, the "Ask" card and the main window. `"auto"` (the default)
  follows the Windows display language, `"en"` and `"ru"` fix it (case and surrounding spaces do not matter), any
  other value means `"auto"`. The value is kept in the file only when it is `"en"` or `"ru"`. The main window's
  Languages settings set it too. Example: `"ui": { "language": "ru" }`.
- `audio.device`: the microphone CUE records from, by the name Windows shows for it, chosen in the main window's
  settings. Without it (the default) CUE records from the microphone Windows uses by default; with it, CUE keeps that
  device when Windows changes its default, and when the device is not connected records from the Windows microphone
  and says so on a card, once. Surrounding spaces are trimmed; a blank value or one that is not a string means the
  Windows microphone. The value is kept in the file only when set. Example:
  `"audio": { "device": "Microphone (USB Audio Device)" }`.
- `history.keep`: how long the history keeps its days: `"forever"`, `"year"`, `"month"` (the default), `"week"` or
  `"off"` (nothing is written, and the days already kept are deleted). Case and surrounding spaces do not matter. Any
  other value leaves the period unknown: CUE then neither writes nor deletes history until a period is chosen in the
  main window, and keeps the value in the file as it was. The value is kept in the file only when it is not
  `"month"`. Example: `"history": { "keep": "forever" }`.
- `history.audio`: how long the recordings of the history's entries are kept: `"day"` (today's), `"week"` (the
  default), `"month"` or `"off"` (none is made, and those kept are deleted). They never outlive their entries: the
  shorter of this and `history.keep` holds, and none is made while the history is off. Any other value leaves the
  period unknown: no recording is made until one is chosen in the main window, and only the history's own period
  deletes them. The value is kept in the file only when it is not `"week"`. Example: `"history": { "audio": "day" }`.
- `drop.position`: where the Drop sits: `"bottom"` (the default; centered, 12 px above the taskbar), `"top"`
  (centered, 12 px below the top of the screen), `"left"` or `"right"` (halfway up, 12 px from the edge). Dragging the
  Drop sets it. Case and surrounding spaces do not matter; any other value means `"bottom"`. The value is kept in the
  file only when it is not `"bottom"`. Example: `"drop": { "position": "right" }`.
- `dictionary.terms`: names and terms you often say, written as they should appear; the main window's Dictionary edits
  the same list. Example:
  `"dictionary": { "terms": ["Parakeet", "Claude Haiku", "GitHub"] }`. When the transcript clearly means one of them
  but spells it differently, splits it or mixes alphabets (`Пара kit`, `гитхаб`, `git hub`), the cleanup writes it as
  listed. The list affects only the cleanup, whichever connection it goes through, where it rides along with the
  system prompt (cached, with Anthropic) and costs almost nothing; the speech model takes no vocabulary. Entries are
  trimmed; empty ones, ones with a line break, ones longer than 60 characters and exact repeats (case-sensitive) are
  skipped, and only the first 200 are used. A value that is not a list reads as an empty list. The block is kept in
  the file only when the list is not empty.
- `dictionary.learned`: the words added from the Drop's suggestions, newest first, each with how it was heard:
  `"dictionary": { "terms": ["Marisol"], "learned": [{ "word": "Marisol", "heard": "Марисоль" }] }`. Each is also in
  `terms`, which the cleanup reads as before; it gets a learned word with how it was heard ("Marisol — sounds like
  «Марисоль»"). Removing a word from the main window's Dictionary removes it from both lists. Anything in `learned` that
  is not a word with its `heard` is skipped, and the list is kept in the file only when it is not empty.
- `learn.enabled` (default `true`): `false` turns "Learn from me" off, and CUE then reads no field after a paste. Only
  `false` turns it off; any other value reads as on, and the default is not written to the file. The switch in the
  Habits section's heading sets it too. `false` also stops habits from being counted, learned and applied. Example:
  `"learn": { "enabled": false }`.
- `habits`: what "Learn from me" noticed in the way you correct dictations. Each list is kept in the file only when it
  is not empty, and anything in it that does not read is skipped.
  - `seen`: how many times a correction was made in an app, for the habits to come: `{ app, form, n, last }`, the
    form by its hash (made with this computer's secret) and no text. A count not seen for 30 days is dropped, and at
    most 300 are kept; a habit's whole set corrected together in another app is counted as one entry, under the
    habit's id.
  - `list`: the habits, newest first, at most 50: `{ id, forms, rule, short, before, after, app, n, on }`. `forms` are
    the kinds of correction with their details, `rule` the sentence the cleanup follows, `short` its short name,
    `before` and `after` the example (at most 40 characters on each side), `app` the app it holds in (none: everywhere),
    `n` how many times it was corrected and `on` its switch. The example is the only text of a correction that CUE
    writes to disk: a replacement's form holds the words it replaced and the ones written instead, which are words of
    its example (`{ "kind": "replacement", "from": "здравствуйте", "to": "добрый день" }`).
  - `declined`: the hashes of corrections never to learn again (a habit that was undone or forgotten). **Forget all**
    empties it.
- `translate`: the language pair of `Ctrl + Win + Shift`, as the ISO 639-1 codes of the 60 languages CUE knows
  (`en`, `ru`, `de`, `zh`, …). Case and surrounding spaces do not matter, and a locale name reads as its language
  (`"ru-RU"` is `ru`).
  - `target` (default `en`): speech in any other language is translated into it. The main window's Languages
    settings set it too.
  - `home`: speech in the target language is translated into it. By default the Windows display language, else
    Russian (English when the target is Russian); one equal to the target is skipped. With no home language, speech in
    the target language is only cleaned up.
  - `speech`: `"auto"` (the default) tells the speech language by the recognized text's alphabet: the language of the
    pair written in that alphabet, the target first. A code fixes the speech language, and with it the direction.

  An unknown code reads as the default, and defaults are not written to the file. Example for a Russian speaker who
  writes to German colleagues: `"translate": { "target": "de", "home": "ru" }`. Russian speech is pasted in German,
  German speech in Russian; speech in Latin letters counts as German there, English included.
- `ask`: the "Ask" mode.
  - `enabled` (default `true`): `false` turns "Ask" off, and `Ctrl + Win + Alt` then goes to Windows as is.
  - `web_search` (default `true`): `false` answers without searching the web.
  - `model` (default `claude-haiku-5-5`): the Claude model that answers; keep it in the `pricing.claude` list, or its
    cost is counted as 0. A model name that does not exist makes every question fail (the log says to check
    `settings.json`).
  - `max_searches` (default `2`): web searches a request may run, a whole number from 1 to 10. A question that pauses
    after a search is continued and may search again, so it runs at most `2 × max_searches − 1` searches (3 by
    default).

  Only `false` turns a switch off; any other wrong value (an empty or non-string model, `0`, `11` or `"3"` searches, a
  section that is not an object) reads as the default, and defaults are not written to the file. Example:
  `"ask": { "model": "claude-sonnet-5-5", "max_searches": 3 }`.
- `pricing` (not written unless you change it): a `claude` list of
  `{model, input_per_mtok, output_per_mtok, cache_read_per_mtok, cache_write_per_mtok}` in USD. Defaults: Haiku 5.5
  $0.10 / $0.50, Sonnet 5.5 $2 / $10, Opus 5.5 $4 / $20 per million input / output tokens, and every model of the
  cleanup's connections at the price in [API keys](#api-keys), under the id `usage.csv` gives it (OpenRouter's as it
  is, `anthropic/claude-haiku-5.5`; another service's after its connection, `openai:gpt-6-luna`), its cache reads at a
  tenth of its input price. A cache write costs 1.25 × the input price. A model on this computer has no price.
  `web_search_usd` ($0.01) prices one web search of "Ask". Recognition on this computer has no price.

Start at sign-in is not a setting: it is the Windows `Run` registry entry, toggled from the tray menu or the main
window's General settings.

## Local data

Everything else lives in `%LOCALAPPDATA%\CUE`:

| Path | What |
|---|---|
| `last\audio.wav`, `last\result.txt` | The last successful dictation (16 kHz mono WAV and the final text), for `Ctrl + Win + V` and Copy. Replaced by the next one, and by an "Ask" result: a text written into a field keeps its question's recording as `audio.wav`; an answer pasted with its button has no audio, so the old `audio.wav` is deleted. |
| `last\current.wav` | Audio of the dictation in progress; after a failure it waits there for Retry. Deleted when a dictation is cancelled or comes out empty, and when a question has been answered. |
| `history\YYYY-MM-DD.jsonl` | The history the main window shows: one file per local day, one line of JSON per result (its time, mode, app and text, for "Ask" the question, and for a dictation or a translation the text as it was recognized, `raw`). Days older than `history.keep` are deleted on their own, and **Clear history** deletes them all. |
| `history\audio\` | The recordings of the history's entries, one Ogg Opus file per entry (speech at 16 kbit/s, about 6 MB an hour), named by the entry's id. Older than `history.audio` (and never older than the history) they are deleted on their own; **Clear history**, Don't keep and the history turned off delete them all. A clear first renames the folder `audio.cleared-<n>`, then deletes it. |
| `logs\cue.log.YYYY-MM-DD` | Daily log: events, per-stage timings, error kinds. No dictation text, no questions, answers or selected text, no history, no microphone names, no audio, no keys. Kept 7 days and 10 MB at most. **Open log** in the tray menu opens this folder. |
| `usage.csv` | One row per attempt: time (UTC), audio length, recognition engine and path, cost, model, tokens, latency, outcome, mode, web searches, how many words it wrote and its local date. A cache warm-up gets a row of its own. No text. |
| `models\parakeet-ultra-int8\` | The default speech model, about 670 MB: `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx` and `vocab.txt`, downloaded once from a fixed commit of the Hugging Face repository `Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx`. Each file comes as `<name>.part`, resumes from where it stopped, and is renamed only once its SHA-256 matches the one in CUE's code; a file that does not match is deleted and downloaded again. |
| `models\<id>\` | Every other model chosen on the Model page, the same three files downloaded the same way: `parakeet-v3-int8` and `parakeet-v2-int8` (`istupakov/parakeet-tdt-0.6b-v3-onnx` and `…-v2-onnx`, pinned in CUE's code like Ultra), and `hf--<owner>--<repository>` for a model found on Hugging Face, with its `manifest.json`: the repository, the commit it was pinned at and each file's size and SHA-256. A folder without a readable manifest is not CUE's: it is never shown or deleted. **Delete** on the Model page removes a model's folder. |
| `bench\recordings\` | Recordings for the bench, only when `bench.save_recordings` is `true`. |
| `bench\runs\<timestamp>\` | Bench runs: `results.json` and `report.html`. |

WebView2 keeps the browser profile of CUE's windows (cache and storage) separately, in
`%LOCALAPPDATA%\app.cue.dictation\EBWebView` (`app.cue.dictation` is the app identifier in `tauri.conf.json`).

## Costs

```powershell
cargo run -p cue-cli -- usage            # current month (UTC)
cargo run -p cue-cli -- usage 2026-10
```

The summary shows the number of dictations, minutes of audio, transcription cost (0 for recognition on this computer;
rows written by earlier versions keep the Soniox cost they were written with) and refine cost, the cost per dictation
and per hour of audio, the average latency from the second key press to the paste (overall and for recordings of
20 s or less), web search when there was some, and the rows and dollars of each mode. Prices come from
`settings.json`. A Retry is a second attempt, whose cleanup is paid again, and gets its own row; the Retry of a
failed translation translates the saved text again, so its row has no audio.

New rows have `stt_engine` `parakeet`, `stt_path` `local` and `stt_usd` 0; the columns stay as they were.
`refine_model` names the cleanup's model as the price list does: Anthropic's and OpenRouter's ids as they are
(`claude-haiku-5-5`, `anthropic/claude-haiku-5.5`), another service's after its connection (`openai:gpt-6-luna`), and
a model on this computer after `local:` (`local:llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m`, at $0). The
columns `mode`, `web_searches` and `search_usd` came with the modes, and `words` and `day` with Today. `mode` is
`dictate`, `translate`, `ask` or `warmup`; `words` is how many words a dictation or translation wrote (0 for anything
else) and `day` the local date.
- An `ask` row is one question: its audio, the tokens of `ask.model` (in the `refine_model` and token columns) and its
  web searches at $0.01 each (`web_searches`, `search_usd`). An answer you close while it is still printing is paid
  for as far as it came, its web searches included, and so is a question cancelled with `Esc`. Each follow-up gets a row of its own, and so does
  the Retry of a failed question (without audio). Pasting an answer with its button adds no row: the answer was counted
  when it was complete. A shown answer has the outcome `answered`, and its latency runs from the second key press to
  the first words; a text written into the field is `inserted` (or `no_field`), with the latency to the paste.
- A `warmup` row is a cache warm-up (see Privacy in detail), of the cleanup (only when it goes through Anthropic) or of
  "Ask" (then with `ask.model`): paid tokens, no audio, outcome `discarded`; it counts in the cost, not as a dictation.

Rows written before these columns existed read as dictation, and the header of an existing file is not rewritten: it
keeps the 13 old names while new rows have 16 fields.

`usage.csv` is comma-separated. In Excel with a locale whose list separator is not a comma (Russian, for example), do not
open the file directly; use **Data → From Text/CSV** and choose the comma delimiter.

## Comparison bench

The bench compares speech models and cleanup models on your own speech and shows how the sanity check behaves on
it. It does not tune the thresholds.

1. Set `"bench": { "save_recordings": true }` in `settings.json`, restart CUE and dictate as usual.
2. Put the speech models to compare in `%LOCALAPPDATA%\CUE\models\`, one folder each: `parakeet-ultra-int8`, the
   default model, is there once CUE has downloaded it, and so is every model downloaded on the Model page (NVIDIA's own
   v3 and v2, models found on Hugging Face); another export with the same file names (`encoder-model.int8.onnx`,
   `decoder_joint-model.int8.onnx`, `vocab.txt`) can go next to them in a folder of its own.
3. Run it (needs the Anthropic key):

   ```powershell
   cargo run -p cue-bench -- run [--limit N] [--max-usd X]
   ```

   Every folder that loads is one speech model, named by its folder; a folder that does not load (a partial download,
   another export) is skipped with a console line. Each model is loaded once and recognizes every recording in turn,
   as the app does; its time is the model's own recognition time, its cost is $0, and its console line says when the
   other alphabet was masked out (see One language a dictation in docs/ARCHITECTURE.md). The text of
   `parakeet-ultra-int8` (or of the first model that succeeded, without it) is then cleaned through Anthropic by the
   Claude model picked for the Anthropic connection (`refine.models`, Claude Haiku 5.5 by default) and every Claude
   model in the `pricing.claude` list, with the same request (including `dictionary.terms`), sanity check and timeout
   as the app. `--max-usd` (default $1) stops new paid calls once reached. The first `Ctrl+C` finishes the step
   under way and stops; a second one exits at once. Results are saved after every recording.
4. Open `report.html` from the run folder. It shows blind A/B pairs for recognition and for refine, then the scores,
   latency and cost per dictation. **Save choices** downloads your answers as `choices.json`.

`cargo run -p cue-bench -- report <run folder>` rebuilds `report.html` from `results.json`.

### The "Ask" bench

`ask_compare` (an example in `cue-engines`; needs the Anthropic key, and it is paid) asks a file of questions the way
the app asks them: the same instruction, tools, web search (2 searches a request), deadlines and continuations, with
today's date. It writes every call and a summary to `out.json`, before the first call and again after every call, so a
run cut short keeps what it paid for; the console gets only ids and numbers.

```powershell
cargo run -p cue-engines --example ask_compare -- phrases.json out.json [--runs 5] [--max-usd 1.00]
```

`phrases.json` is an array of questions with their context:

```json
[
  {"id": "q-capital", "question": "столица австралии", "app": "Telegram Desktop", "expect": "answer"},
  {"id": "w-punct", "question": "расставь знаки препинания", "app": "Telegram Desktop",
   "selection": "привет как дела", "expect": "write", "mode": "replace"},
  {"id": "f-tcp", "question": "чем tcp отличается от udp", "then": "а что быстрее", "app": "Google Chrome",
   "expect": "answer"}
]
```

`selection` and `before_cursor` are optional context; `then` is a follow-up asked after the first answer, and the
follow-up is what is scored; `expect` is `answer` or `write`, and `mode` (`replace` or `insert`, for a write only) is
the mode the write should take. Each run asks every question once, one at a time; the very first call (an empty prompt
cache) is left out of the medians. The summary counts the right decisions and modes, the median time to the first
words with and without a search, and the cost; the run stops once more than `--max-usd` is spent. `--model` asks
another model (`claude-haiku-5-5` by default; one without a default price is refused, since the budget could not count
it), `--no-search` asks without web search, and `--thinking adaptive` sends the request with the model's adaptive
thinking, to compare with the app's default (`--thinking off`). An answer's text and first words are the ones the app
shows: the words the model wrote before its first web search go when it starts. Each call in `out.json` also says
whether the model wrote such words (`text_before_search`) and the error codes of searches that failed (`search_errors`).

## Privacy in detail

- **Speech recognition** runs on this computer: no sound leaves it, while you record, on Retry or ever. The network
  is needed only to download a model once, from Hugging Face (`huggingface.co`, files of a fixed commit; the request
  carries no key and nothing of yours), and for the cleanup and "Ask". The Model page's search sends Hugging Face the
  words typed into it (from two letters on), and choosing a model it found asks Hugging Face for that repository's
  list of files; nothing else of yours goes with either.
- **The cleanup's connection** (Anthropic by default, or the one chosen in the Cleanup settings) receives the
  transcript, the app name, up to `refine.context_chars` (500) characters of text before the caret, your
  `dictionary.terms` (for a word added from a correction, also how it was heard, `dictionary.learned`), the rules of the
  habits that are on for the dictation's app (see Habits below) and, with the Custom style, your instructions. Nothing
  is read from password fields, and the window title is never sent. A translation goes in the same kind of request to
  the same connection, with the target language added. Through OpenRouter, CUE first asks for providers that do not
  store requests for training, and only when none of them can take the request, for any provider. With the cleanup on
  this computer none of this is sent: the local model gets the same request, and the text stays here.
- **Anthropic, when the cleanup goes through it**: while you record, CUE also sends an empty `HEAD` request without a
  key to open the connection early, so cleanup starts faster. When a dictation starts four minutes or more after the
  last cleanup, CUE sends a cache warm-up: the cleanup request with your dictionary (as the cleanup gets it) and a
  one-character placeholder instead of a transcript, without the app name or any text before the caret, so the first
  cleanup after a pause reads the prompt from Anthropic's cache. Over the other connections there is neither.
- **"Ask"** always goes to Anthropic, whatever the cleanup's connection. It sends the question (as it was
  recognized), the selected text (up to 8,000 characters: a longer selection goes as its start and its end), up to
  `refine.context_chars` (500) characters before the caret, the app name and today's local date; a follow-up sends
  only its question, with the conversation so far. With web search on, Anthropic searches the web for what the model
  asks it to. Nothing is read from password fields. When the "Ask" chord starts, and on a follow-up, four minutes or
  more after the last "Ask" request or warm-up, CUE sends a cache warm-up of the "Ask" instruction with a
  one-character placeholder, nothing of yours. The conversation lives only while the answer is open.
- **The `Ctrl + C` fallback of "Ask".** When the app does not tell CUE what is selected, CUE presses `Ctrl + C` in that
  window once the recording ends (never in a terminal CUE knows, nor in another window than the one you asked in), reads
  the selection from the clipboard and puts your clipboard back as it was. It never does while Windows syncs copied
  text to your other devices by itself (Settings → System → Clipboard: clipboard history across your devices, with
  "Automatically sync text that I copy"): the app's copy would be uploaded at once, so the selection then reads as
  none. CUE reads that setting from the registry at each "Ask" chord and logs only whether it is on. Only a copy made by that app counts (or one
  with no owner, as some apps copy), and a copy that is the same text as your clipboard already held counts as nothing
  selected. The app's copy reaches the Windows clipboard history (Win+V) before CUE can mark it, so CUE then tries to
  remove it from there; in the live check on 2026-10-29 Windows refused, so the copy may stay in Win+V (see Known
  limitations). CUE's own writes (each paste, and your clipboard put back) carry the marks that keep them out of the
  history and the cloud clipboard, but the app's copy carries none: with clipboard sync set to manual, it can be sent
  to your other devices from Win+V by hand for as long as it stays there.
- **Learn from me.** While it is on, after a dictation or a translation was pasted with `Ctrl + V` into a field that
  gives its text to Windows (UI Automation), CUE reads that one field every 0.4 s, for 60 s at most. It stops when the
  text has not changed for 2.5 s after a change, when the field empties (the message was sent), when the field can no
  longer be read or when you start a dictation. Password fields, CUE's own windows and "Ask" answers are not read, and
  neither is any other field than the one pasted into. The field's text lives in memory while CUE watches and is then
  dropped: it is not written to a file, not sent anywhere, and not logged. The log gets whether a correction was found
  and numbers (characters, candidates). A word reaches `settings.json` only when you press **+**; **Undo**, or removing
  it from the Dictionary, takes it out again with how it was heard.
- **Habits.** A dictation's correction is also read for habits, in memory, and counted by hash: `settings.json` keeps
  the app's name, the hash of the kind of correction, a number and a day, no text. The hash is made with a random secret
  of this installation (32 bytes, an HMAC), kept in Windows Credential Manager under `habits-secret` beside the keys,
  never in `settings.json` or the log. When it is missing or malformed, CUE makes a new one at start and drops the
  counts and the declined corrections (their hashes no longer match); the habits keep working. When Credential Manager
  cannot be read, the secret kept there is left alone and nothing is counted until the next start. When a correction
  reaches three in an app, the habit keeps in `settings.json` its rule, its short name and an example: the words around
  the correction, at most 40 characters on each side, which is the only text of a correction written to disk. The
  cleanup's connection receives that example once to word the rule, in a short request with the kind of the correction,
  the example and the app's name, nothing else; it is not written to `usage.csv`. Through OpenRouter it goes only to
  providers that do not store requests for training, and when none can take it, the rule comes from a template. It is
  not sent at all without a cleanup connection that can be asked (a cloud one with no key): the rule then comes from the
  template too, as it does when the answer does not come in 5 s. The rules of the habits that are on go with every
  cleanup request of a dictation in their app, and those of every app with all, as one line each; translations and "Ask"
  get none. The log gets "habit forms counted" and "habit learned" with the kinds and numbers, no text. **Learn from
  me** turned off stops all of it: no field is read, nothing is counted or learned, and no habit goes into a request.
  **Forget** and **Forget all**, or **Undo** on the card, remove a habit with its example.
- **Inside CUE:** the Drop and the card windows get the text of the question and the answer, the card's short quote of
  the selection (40 characters at most), and the titles and sites of the sources, to draw them; never the addresses. A
  click on a source asks the core to open it, and the core opens only `http` and `https` pages, in the default browser.
  The history's text goes to the main window alone. The welcome window cannot read the history, and of the settings
  it changes only the microphone; the Drop and the card cannot read the history, the settings or the keys. A key
  pasted in the main window or the welcome window goes straight to Credential Manager, and the window gets back only
  its mask.
- The microphone is open only while CUE is recording, and while the main window's Microphone settings or the welcome
  window's Microphone step show its level (only the level reaches that window; that audio is never kept).
- **Stays on this computer:** every sound CUE records, the history with its recordings, the last dictation's audio and
  text, recordings saved for the bench, the speech models, settings, logs and usage rows. Logs and `usage.csv` never
  contain dictation text, questions, answers, selected text, sources, the history, microphone names, your Custom
  instructions or a local server's address (only their length, or whether the server answered), nor any sound:
  a recording's line has its length and size alone; a crash records only the thread and code location, not the panic
  message. A recording reaches the main window alone, as the bytes of its file, and plays there from memory.
- Check the data-retention policy of each service you use for the text it receives.

## Known limitations

- Windows does not let a normal app intercept keys in, or paste into, windows running as administrator; CUE does not
  react there. The lock screen and UAC prompts are out of reach too.
- Clipboard restore covers text, HTML, RTF, images (PNG and DIB) and file lists; rarer formats are not restored.
- The terminal rules (one line when pasting, no `Ctrl + C`, no line break from "Ask") recognize Windows Terminal,
  PowerShell, Command Prompt, conhost, mintty and Git Bash, WSL, WezTerm, Alacritty, Warp, Tabby, Hyper, Termius,
  WindTerm, MobaXterm, PuTTY, KiTTY, ConEmu and Cmder, and Fluent Terminal, by app name, executable or window class; a
  terminal outside that list is treated as an ordinary app, and the VS Code integrated terminal looks like the editor.
- A translation is checked by length and by script, not by meaning. Between two languages written in the same script
  (English and German) only the length is checked, so untranslated text can pass there. English speech with a word in
  the target script (a Cyrillic name, translated into Russian) counts as such a pair too and gets the length check
  only. A short question answered with a bare number can be pasted; a translation made only of words the speech
  already has ("сделай git push" → "Git push."), or of a large number said in one word, fails and offers Retry. Other
  limits are in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).
- The speech language of a translation is told by its alphabet alone, so speech in a language outside the pair counts
  as the pair's language written in the same letters: with the default pair (English and Russian), German speech
  counts as English and is translated into Russian. `translate.speech` fixes the speech language.
- Text appears only after you finish. A long dictation is recognized in pieces while you speak, but nothing of it is
  shown until it is pasted.
- Parakeet v2 recognizes no Russian: with it, every dictation comes out in Latin letters and reads as English.
- The recommended models of the connections were chosen from the services' documentation, not by comparing their
  cleanup on real dictations.
- "Ask" goes through Anthropic alone, so it needs the Anthropic key even when the cleanup goes through another
  connection, and the welcome window's Key step asks for that key whichever connection is chosen.
- A dictation is Russian or English. When both alphabets hold whole sentences (at least two of each language's little
  words: "the", "to", «и», «что»…), the one with fewer letters is taken for a slip of the model, and the dictation is
  recognized again without it, so a sentence said in the other language on purpose comes out spelled in the main
  one's letters.
- The speech model needs a processor with AVX2; on one without it, CUE cannot recognize speech ("Processor not
  supported").
- A translation can turn a self-correction around: "Отправь отчёт Олегу, точнее, Игорю" ("to Oleg, I mean Igor")
  came back as "…to Igor, I mean Oleg" in every bench run. The check cannot see it, so read a translated correction
  before sending it.
- "Ask" cannot tell where the selection is on screen in classic Win32 fields, nor in apps that do not report it; the
  answer then opens in the panel by the Drop. Chromium apps report it: checked in a Brave address bar, not yet in page
  text or in Electron apps, where a misplaced card would be the sign to switch it off (`CHROMIUM_RECTS` in
  `focus.rs`).
- The `Ctrl + C` fallback's copy may stay in the Win+V history. CUE tries to remove it, but in the live check on
  2026-10-29 Windows refused, so the copy can stay there.
- While Windows syncs copied text to your other devices automatically, there is no `Ctrl + C` fallback, so "Ask" sees
  no selection in apps that do not report it themselves. Setting clipboard sync to manual (Settings → System →
  Clipboard) brings the fallback back; its copy carries none of CUE's marks, so until CUE removes it from Win+V it can
  be synced from there by hand. CUE reads the setting the way Settings stores it (`EnableCloudClipboard` and
  `CloudClipboardAutomaticUpload` under `HKCU\Software\Microsoft\Clipboard`); a policy that turns sync off for the
  whole computer is not read, and with the user setting still on, the fallback stays off.
- A selection of more than 8,000 characters reaches "Ask" as its first and last 4,000: it is never replaced (no
  **Replace** button, and a change of it is shown with **Copy**), and an answer about it sees only those parts.
- Thinking is off for "Ask" (chosen after the bench: 98 % right decisions, 93 % format and checks, the first words in
  1.2 s without a web search and 1.3 s with one, no timeouts). Before a web search the model often writes a short
  sentence, such as "let me check": it can show for a moment, and goes when the search starts. Now and then it
  answers a question about something current (the latest version of a program, say) from what it knows, without
  searching: 1 run in 10 on the bench, and once in the live check. Such an answer can be out of date.
- Editors that copy the current line on `Ctrl + C` with nothing selected (VS Code does by default) could hand that line
  to "Ask" as the selection when they do not report the selection themselves. None is listed yet (`COPIES_LINE` in
  `selection.rs`): VS Code was not available to check.
- A text that "Ask" writes into the field cannot be undone from CUE; `Esc` before it is written cancels the question,
  and **Restore** asks it again.
- **Insert below** moves the caret with the Right arrow, which ends the selection at its end in most editors; where
  Right behaves differently with a selection, the text lands at the caret instead. When the Right arrow cannot be sent,
  the text stays in the clipboard and the "Nowhere to paste" card offers Copy.
- When web search fails or is not available to your key, the model answers from what it knows, which may be out of
  date. A key without web search is asked again without it, at once.
- The `Ctrl + C` fallback stays out of the terminals listed above, by app name, executable or window class. In a
  terminal outside that list that does not report its selection, a question asked there would press `Ctrl + C`, which
  stops the program running in it.
- Reading a selection through the `Ctrl + C` fallback in a Store (UWP) app was not checked live.
- "Learn from me" reads a field only when Windows gives its text: it works in Claude and in a Brave address bar, and
  Telegram and fields inside web pages may not give theirs, so nothing is offered there. A correction is noticed only
  for a word written in place of another (not a deleted or an added one) that looks like a name or a term.

## Repository layout

```
Cargo.toml                Cargo workspace
LICENSE, NOTICE           Apache License 2.0, and the copyright notice
app/
  package.json            front-end tooling: Vite, TypeScript, Vitest, Tauri CLI
  crates/core/            cue-core: dictation state machine, settings, refine, translation and "Ask" rules, the
                          cleanup's connections and their models, the speech rules (one language a dictation, long
                          recordings cut), the speech models (the short list, a found model's manifest, whether a
                          repository runs), audio helpers, usage, the history files
  crates/engines/         cue-engines: speech recognition on this computer (Parakeet: log-mel, TDT decoding, ONNX
                          Runtime), the models' download and Hugging Face's search, refine and translation through
                          Anthropic, OpenRouter and the OpenAI-compatible services, "Ask", API keys and checks
  crates/platform-win/    cue-platform-win: keyboard hook (and a mouse hook while a Shift that switched the mode is
                          held), clipboard paste and copy, focus and selection via UI Automation, microphone (a
                          pinned one by its name), Drop window styles and placement, card placement, sleep and lock
                          events, the Ogg Opus encoding of the history's recordings
  crates/cli/             cue-cli: API keys, monthly usage and the leak scan of a build
  src-tauri/              the cue app: runtime, Drop, card, main and welcome windows, the main and welcome windows'
                          commands, tray icon, logging; nsis/ holds the installer's script and hooks
  ui/                     the Drop, its tray cards, the "Ask" answer and card, the main window (history and
                          settings) and the welcome window: TypeScript + Vite, no framework
bench/                    cue-bench: comparison bench
docs/ARCHITECTURE.md      how CUE works inside
scripts/build-release.ps1 the installer build
```

`app/crates/platform-win/examples` and `app/crates/engines/examples` hold small probes for checking the keyboard
hook, microphone, focus, paste, sleep events, the Drop's z-order (`zorder_probe`, run while CUE is running), refine
and its cache warm-up on a real machine (`cargo run -p <crate> --example <name>`). `refine_compare` (in
`cue-engines`) runs a phrase file of your own through cleanup models with the app's prompt and checks; a phrase with
`"to": "en"` is translated on the app's own path instead (see the example's header). `ask_compare` is the "Ask" bench
(see above); `ask_probe` checks web search and the cache warm-up of "Ask", and `selection_probe` (in `cue-platform-win`)
prints what CUE reads of the selection in an app and what the `Ctrl + C` fallback does there, never the text.

## License

CUE is licensed under the Apache License, Version 2.0: see [LICENSE](LICENSE) and [NOTICE](NOTICE).

The Bebas Neue and Montserrat ExtraBold fonts in `app/ui/src/design/fonts` are distributed under the SIL Open Font
License 1.1 (see `OFL.txt` and `OFL-Montserrat.txt`).

The program includes the Opus audio codec, libopus 1.3.1 in its Rust translation `unsafe-libopus` (BSD-3-Clause), and
`arrayref` (BSD-2-Clause), which it uses, and ONNX Runtime (MIT, by Microsoft), linked in through the `ort` crate
(MIT or Apache-2.0, by pyke.io). Its log-mel features and TDT decoding are ported from
[parakeet-rs](https://github.com/altunenes/parakeet-rs) (MIT or Apache-2.0, by Enes Altun). Their licenses are in
[NOTICE](NOTICE). The installer also carries `DirectML.dll`, Microsoft DirectX Machine Learning 1.15.4, unchanged, as
it comes with ONNX Runtime; it is not covered by CUE's license but by Microsoft's own license terms for DirectML, named
in [NOTICE](NOTICE).

The speech models CUE downloads are not part of the program or the installer. The default is
[Parakeet Ultra](https://huggingface.co/moondream/parakeet-ultra) by Moondream, a post-trained version of
[NVIDIA Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), in the int8 ONNX export
[Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx](https://huggingface.co/Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx); all
three are licensed under [CC-BY-4.0](https://creativecommons.org/licenses/by/4.0/). See [NOTICE](NOTICE). The other
two models of the Model page's list, downloaded only when chosen, are NVIDIA's Parakeet TDT 0.6B v3 and v2 in the
int8 ONNX exports [istupakov/parakeet-tdt-0.6b-v3-onnx](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx)
and [istupakov/parakeet-tdt-0.6b-v2-onnx](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v2-onnx), also under
CC-BY-4.0; a model found on Hugging Face comes under its repository's own license.

`app/src-tauri/nsis/installer.nsi` is the installer template of the Tauri CLI (Apache-2.0 OR MIT, by the Tauri
contributors) with the default install folder changed, as its comment on that line says.
