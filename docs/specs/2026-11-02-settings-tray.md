# The settings tray after local speech recognition

Soniox goes (2026-11-02-local-speech.md), so the Keys page would keep one row, and Translation and General would keep
one or two. The tray is regrouped by meaning instead: five rows on the first page, as many as today, each page about
one thing. Chosen by the owner from three variants (variant C, 02.11.2026). The same day the owner found it still
unclear where things are, and the cleanup got a page of its own: its style and the Anthropic key it needs sit together,
so "Speech recognition" is only about the model on this computer and "General" only about starting.

Everything is built from the window's existing parts: `.trw` rows on the first page, `.srow` rows on the pages, `.dd`,
`.sw`, `.segc`, `.kin`, `.btn2`, and the `.tft` look for the note. Page motion does not change: the heading line stays,
rows slide, heights ease without overshoot.

## The first page

Six rows, in this order (icon · title · value · chevron):

| # | Icon | Title (RU / EN) | Value |
|---|---|---|---|
| 1 | `wave`, new | Распознавание / Speech recognition | the model's state, see below |
| 2 | `sparkle`, new | Чистка текста / Cleanup | the style: «Бережная» / "Careful", «Смелая» / "Bold"; «Нет ключа Anthropic» / "No Anthropic key" in `--warn` once the keys are read and it is not saved |
| 3 | `micOn` | Микрофон / Microphone | as today |
| 4 | `globe` | Языки / Languages | `{interface language} · на {target}`: «Русский · на английский» / "English · into Russian" |
| 5 | `clock` | История / History | as today |
| 6 | `half` | Общие / General | start at sign-in on: «Запуск при входе» / "Starts at sign-in"; off: «Вручную» / "Manually" |

The Keys row and its `apiKey` icon go, and the Translation row goes into Languages.

The new icons, on the window's 20 grid with stroke 1.8 (`SV` in icons.ts):

```
wave: SV('<path d="M3.5 8.5v3M6.7 6v8M10 3.5v13M13.3 6.5v7M16.5 8.5v3"/>')
sparkle: SV('<path d="M10 3.2c.6 3.6 2.8 5.8 6.4 6.4-3.6.6-5.8 2.8-6.4 6.4-.6-3.6-2.8-5.8-6.4-6.4 3.6-.6 5.8-2.8 6.4-6.4z"/>')
```

The value of "Speech recognition", the first that holds:

| State | RU | EN | Look |
|---|---|---|---|
| Downloading | «61 %» | "61 %" | the number in Bebas 15, text color, tabular; a line under the row |
| Did not download | «Не скачалась» | "Did not download" | `--warn` |
| Ready | «Готово» | "Ready" | muted, as other values |

The line under the row while downloading: absolute, left 30 px, right 26 px, bottom 3 px, height 2 px, radius 1 px;
the track is the text color at 12 %, the fill is the text color, `scaleX(p)` from the left with a 0.5 s `--e-out`
transition.

## Speech recognition

Title «Распознавание» / "Speech recognition". One row, then a note.

**Parakeet Ultra** (the name is not translated). The row's look by state:

- **Ready.** Under the name, in `.ok` (green): «Готово · 690 МБ на этом компьютере» / "Ready · 690 MB on this
  computer". Nothing on the right: there is no choice of engine.
- **Downloading.** On the right «61 %» / "61 %" in Bebas 15, text color. Under the row, across its whole width (the row
  wraps, a child with `flex-basis: 100%`): a 2 px bar like the one above (6 px above it), then a line 11 px muted:
  «Качается: 412 из 690 МБ» / "Downloading: 412 of 690 MB". The two numbers are in Bebas 13 in the text color.
- **Did not download.** Under the name, all in `.warn` (amber): «Не скачалась · 412 из 690 МБ» / "Did not download ·
  412 of 690 MB", the numbers in Bebas. On the right a `.btn2` with the `retry` icon: «Повторить» / "Retry". It
  continues from where the download stopped, and the row becomes Downloading.

While the page is open, a download in progress changes only the bar's scale, the percent and the first number of MB,
in place; the row is not drawn again.

**The note** under the group, in the `.tft` look (a `--line` border on top, 11 px muted, line height 1.4):
«Звук остаётся на этом компьютере.» / "Sound stays on this computer."

## Cleanup

Title «Чистка текста» / "Cleanup". One group of two rows, then a note.

1. «Стиль» / "Style": the segmented control «Бережная | Смелая» / "Careful | Bold", hint «Смелая ещё и переписывает
   разговорные обороты» / "Bold also turns spoken phrasing into written" (2026-11-02-bold-refine.md).
2. **Anthropic.** Today's key row from keys.ts: the `.kin` field and «Проверить» / "Check", the key's state under the
   name («Работает» / "Works" and the others).

Since 2026-11-02-refine-connections.md the group also has «Уровень» / "Level" after the style, the key row's name is
the selector of the connection («Anthropic ⌄» / «OpenRouter ⌄»), and the key row and the note follow the chosen one.

**The note**, as on Speech recognition: «Текст уходит в Anthropic для чистки; история — только на этом компьютере.» /
"Text goes to Anthropic for the cleanup; the history stays on this computer only."

## Languages

Title «Языки» / "Languages". Two rows:

1. «Язык интерфейса» / "Interface language": the segmented control, as on General today.
2. «Переводить на» / "Translate into": the drop-down, as on Translation today.

## General

Title «Общие» / "General". One row, «Запускать при входе» / "Start at sign-in" with its switch. «Учиться у меня» /
"Learn from me" (2026-11-02-corrections.md) comes in later as the second row.

## Not changed

Microphone and History pages; the tray's size, place and motion; Esc goes back, then closes.
