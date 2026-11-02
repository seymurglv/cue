# Bold cleanup

Parakeet already writes punctuation and capitals, so the careful cleanup often has nothing left to fix in a short
phrase, and it returns the text as heard. The owner wanted a cleanup that also reads like written text. CUE now has
two styles, chosen on the settings tray's General page («Чистка текста» / "Cleanup").

## Styles

- **Careful** (default, `refine.style: "careful"`): as before. It fixes slips, fillers, recognition errors,
  punctuation and grammar, and keeps the speaker's words, tone and meaning.
- **Bold** (`refine.style: "bold"`): the careful rules plus `refine::BOLD_PROMPT`. It also turns spoken constructions
  into written ones ("текст, который я отправил, он был отправлен" → "…, был отправлен", "нету" → "нет"), drops
  real words that only fill the speech ("в принципе", "собственно", "basically"), and rewords clumsy or repeated
  phrasing. It still keeps the meaning, facts, requests, person and tone. Names, terms and the English words a Russian
  speaker uses stay as said, and nothing is added.

Any value of `refine.style` other than `"bold"` reads as careful. A translation is unchanged by the style.

## Request

The bold block is a system block after the cache mark (as the translation block is), so both styles read one cached
prefix and switching styles writes no new cache. The block itself is not cached: a bold dictation cost about $0.0018
against $0.0011 careful on Sonnet 5.5 over 20 of the owner's dictations.

## The answer check

`sanity_ok` is the same for both styles, except that in bold a result word counts as said when its first 4 letters
match a said word's ("инструмента" for "инструмент", "отправлено" for "отправлен"): bold changes endings. An answer
instead of a cleanup is still rejected.

## Measured

`refine_compare --style careful|bold` (the direct path is the app's model, Sonnet 5.5), 20 recent dictations, one run
each: careful left 9 of 20 as heard (one of them a fallback to the raw text after the check); bold changed 18 of 20
with no fallback, in about the same time (median 1.26 s for both).
