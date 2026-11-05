# The basics: cleanup without a model (short design)

Date: 2026-11-05. Status: for the owner's approval.

## Why

Without an AI connected (no key, the local model not downloaded, or cleanup off), CUE pastes exactly what Parakeet
recognized: the dictionary, the learned words and the habits reach only a model's prompt. The owner wants a really
optimized product that still does the basics without any model, the way Hex runs fixed steps before its rewrite.

## Goals and limits

- Fix what CUE knows for certain, without a model, in under a millisecond, with no new setting.
- Never guess: a word changes only on an exact match (the owner's pick, option A).
- Not: fuzzy or phonetic matching, transliteration of dictionary words, removing words that can carry meaning ("как бы",
  "типа", "like", "you know").

## Approaches (picked: A)

| | What | Trade-off |
|---|---|---|
| **A. Exact only** | Learned pairs and the case of dictionary words | Never wrong; a hand-added word is fixed only after it was corrected once |
| B. A + transliteration | Also "гитхаб" → "GitHub" for 4+ letter words | Works from day one; can misfire on look-alike words |
| C. Fuzzy | Similar-sounding words | Catches more; replaces ordinary words sometimes |

## What it does, in order

1. **Learned words** (`dictionary.learned`: what recognition heard → the word you wrote): each `heard` found as a whole
   word or phrase, case-insensitively, becomes its `word`; longer phrases first.
2. **Dictionary case** (`dictionary.terms`): a word or phrase equal to a term apart from letter case takes the term's
   spelling ("github" → "GitHub", "claude haiku" → "Claude Haiku").
3. **Fillers**: standalone "э", "ээ", "эээ", "эм", "мм", "ммм", "um", "uh", "erm", "hmm" are removed with one comma next
   to them; "ну" and "well" only as the first word of the dictation, with its comma.
4. **Tidy**: no space before `, . ! ? : ;`, single spaces, no doubled commas left by a removed filler, and a capital
   letter where a removed filler left the text starting with a small one.

Words match on whole-word boundaries, so "эмоция" or "humble" never lose letters. Text that would become empty stays as
recognized.

## Where it runs

- On every dictation, translation and question, right after recognition and before any model: the model gets the
  cleaner text, and the text pasted when no model works (no key, the local model not ready, a cleanup that failed its
  checks, a timeout) is this one instead of the raw one.
- The history keeps the raw text as heard, as today; the output checks compare the model's answer with this text.
- A pure function in `cue_core` (a new module), given the text and the dictionary; tests are table-driven, in Russian
  and English.

## Left to the builder

- The module's name and file layout; the exact filler list beyond the one above, kept conservative.
- When the Cleanup page's redesign lands, its Off line reads "Without AI: dictionary spellings and filler words".
