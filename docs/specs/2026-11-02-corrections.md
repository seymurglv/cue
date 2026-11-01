# Corrections and dictionary suggestions

Parts 3 and 4 of bringing the approved main window (design stage 5) and trays (design stage 3) into the app: CUE notices
when you correct a word it pasted, and quietly offers to add the word you wrote to the dictionary. Habits (part 5)
build on the same watcher later.

A spike on 2026-11-02 confirmed the field can be read after a paste: in Claude (Electron) and in Brave's address bar UI
Automation's TextPattern gave the field's text, every keystroke of a correction and the moment it was sent. Telegram
and fields inside web pages were not checked; where the field cannot be read, nothing is offered.

## What the user sees

1. CUE pastes "Мне нужно позвонить Марисоль." You change "Марисоль" to "Marisol" by hand.
2. A couple of seconds after you stop typing, the Drop at rest grows into a capsule 36 px high: **book · Marisol · +**.
   No sentence, only the word you wrote. A ring around "+" runs out in 8 s; hovering pauses it. The capsule never
   takes the focus: you keep typing.
3. "+" adds the word: the book turns green, "+" turns into "undo" with a 6 s ring. "Undo" takes the word out again.
   When the ring runs out, the capsule folds back into the Drop.
4. Not pressed within 8 s: the capsule folds into a green dot next to the idle bar. The dot waits until the next
   dictation starts (then it goes: the suggestion is missed); hovering the Drop opens the same capsule.
5. In the main window's Dictionary, a word added this way has a green book; hovering the chip says how it was heard
   ("sounds like «Марисоль»"). The hint under the heading gains the design's second half: "📗 — from your corrections:
   CUE noticed the change and offered the word."

Every change grows out of the Drop and folds back into it (design stage 3, tray A), with the family's motion rules.
While the Drop records, thinks, or shows an error or another tray, a suggestion waits until it is at rest again. One
suggestion at a time; when one correction holds several candidates, the first is offered.

Texts are in English and Russian. The Drop's capsule has no text besides the word; its buttons have accessible names
("Add to dictionary", "Undo").

## The switch

`learn.enabled` in settings.json, on by default. Until the Habits section exists, it is a row on the settings tray's
General page: "Learn from me" / «Учиться у меня», with the line "CUE notices your corrections and offers words for the
dictionary. Everything stays on this computer." / «CUE замечает ваши правки и предлагает слова для словаря. Всё остаётся
на этом компьютере.» Off, CUE never reads a field after a paste.

## How a correction is found

**Watcher** (`cue-platform-win`). It starts only after a dictation or a translation was pasted with Ctrl+V into a field
whose text UI Automation gives (TextPattern, else ValuePattern), and only while `learn.enabled`.

- At the paste it keeps the element it pasted into and a snapshot of its text, and finds the pasted text in it: the
  text before it and after it are the anchors.
- It reads that element every 400 ms, for at most 60 s. The correction is over when the text has not changed for
  2.5 s after a change, when the field empties (it was sent: the last snapshot before it counts), when the element can
  no longer be read, or when a new dictation starts.
- The corrected span is the text between the longest unchanged prefix and suffix of the anchors in the last snapshot.
  If the anchors cannot be found (the field was cleared and retyped, another message), there is no correction.
- It never reads another element: when focus moves to the whole page, another window or CUE itself, the kept element is
  still the one read.
- The field's text lives only in memory while watching. It is never written to disk or to the log; the log gets
  "correction found / none" and numbers (characters, seconds, words changed).

**Diff** (`cue-core::corrections`, pure). Aligns the pasted and the corrected text word by word and returns replacements
of one to three words: `{ heard: "Марисоль", word: "Marisol" }`. A replacement is a dictionary candidate when the new
text:

- differs from the old one by more than case and punctuation;
- looks like a name or a term: it starts with a capital letter, holds Latin letters in Cyrillic text (or the
  reverse), holds a digit, or has a capital inside (`GitHub`);
- is not in the dictionary already (any case) and passes `refine::is_term`.

Deletions, inserted words and ordinary lowercase words are not candidates.

## The dictionary

`dictionary.learned` in settings.json: `[{ "word": "Marisol", "heard": "Марисоль" }]`, the words added from a
suggestion, newest first. Each is also in `dictionary.terms`, which refine reads as before. Removing a word from the
dictionary removes it from `learned` too. The refine request's dictionary block adds, for learned words, how they are
heard ("Marisol — sounds like «Марисоль»"), which guides the model better than the word alone.

New setting changes: `dictLearn { word, heard }` (adds to both lists, refused like `dictAdd`) and the existing
`dictRemove`. `SettingsView.dictionary` stays the list of words; `SettingsView.learned` lists the learned ones with
their `heard`, and `SettingsView.learn` is the switch.

## The Drop

A new Drop state, `suggest`, owned by the core's flow like the trays: the core sends the word when the Drop is at rest,
the page grows the capsule (its width from the word), and reports `SuggestAdd`, `SuggestUndo`, `SuggestDismiss` (ring
ran out) and hover. `missed` keeps the dot until the next `StartStt`. The native window grows and shrinks with the
capsule the way it does for trays (`drop_window`).

## Out of scope

Habits (rules learned from repeated corrections), corrections in "Ask" answers, reading Telegram or web page fields
beyond what UI Automation gives, a list of declined suggestions.

## Testing

- Core: the diff (single and multi-word replacements, case-only and punctuation-only changes, deletions, inserts,
  a word already in the dictionary, mixed scripts, digits), `dictLearn` and removal from `learned`, the refine block
  with `heard`.
- Watcher: the corrected span from snapshots (prefix and suffix anchors, a sent field, a cleared field, a field changed
  elsewhere), with snapshots as plain strings so it runs without UI Automation.
- Flow: a suggestion waits for rest, the 8 s and 6 s rings, the dot until the next dictation, undo.
- Drop page and window: the capsule, the dot, the book in the Dictionary, the switch.
- Live: Claude and Brave, both themes.
