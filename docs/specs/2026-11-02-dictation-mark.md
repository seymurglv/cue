# The dictation mark in the main window's header

The main window's header used the Drop's live mark in its idle pose before the word CUE: a blinking upright bar (a
caret) and the red lamp. The owner wanted a mark that says "dictation" at a glance and keeps the brand's pulsing red
dot. Chosen from three variants on the real window (a microphone, a sound wave, a microphone with two arcs of speech):
the sound wave.

## The mark

On the window's 20 grid, stroke 1.8, round caps, the text color (`page.ts`, `MARK`):

- five voice bars at x 2.5 / 5.7 / 9 / 12.3 / 15.5, heights 6 / 11 / 14 / 10 / 6, centered on y 10;
- the lamp: a circle r 1.7 at (18.2, 3.2), `--rec`, top right of the bars as on the Drop.

## Motion

- The bars roll as a slow wave: each scales between 45 % and 100 % of its height around its middle, 2.4 s a cycle,
  each 0.5 s after the one before it.
- The lamp pulses on its own: 1.8 s, from 80 % size at 55 % opacity to 110 % at full.
- Both are CSS animations (`window.css`), with no frame loop in the page. With reduced motion both stand still.

## Everywhere else the bar was

The owner asked for the mark wherever the blinking bar stood:

- **The Drop's idle mark** (`cue.js`, the `idle` pose, also the "Ask" card's resting cursor): on its 24 grid, five
  bars at x 4.5 / 8 / 11.5 / 15 / 18.5, 7 / 13 / 16 / 12 / 7 high on y 12, a 2.2 stroke; the lamp at (21.6, 4.2),
  r 1.9. Live, the bars roll as the same wave and the lamp fades between 55 % and 100 % opacity; its place and size
  stay, so the hover capsule's even margins (`drop.css`, `--bar-l`, `--dot-r`) hold. The mark has five lines now; the
  other poses park the fifth.
- **The start of a dictation** (`start`): the wave gathers to the middle as five short bars on x 6.4 … 17.6 (3.5 / 5 /
  6.5 / 5 / 3.5 high) that flicker while the lamp grows red at their top right (21.2, 4.2), the spot ask and translate
  park their hidden lamp at. Listening keeps the outer and middle bars as its three voice bars (lines 0, 2, 4); the
  other two fade where they stand. No tall bar shows up on the way, from rest through start into listening.
- **The Drop at rest**: the strip breathes, its opacity between 40 % and 72 % over 3.2 s; hovered or with reduced
  motion it holds still. Chosen from four variants (as is, breathing, a shine, a row of bars).
- **The key hint** on hover: soft plates, a 12 % white fill with no outline, the system type at 11 (in the 40 px
  drawing) instead of Bebas. Chosen from four variants (as is, soft plates, a quiet caption, one capsule).
- **The tray icon** (`trayicon.rs`): the same five bars and lamp, still; monochrome at rest, the lamp red and larger
  while recording, faded on pause. The frame is sized by the large lamp so the mark does not shift.
- **The app icon** (`icons/app-icon.svg` and every size made from it): the graphite plate with the five bars and the
  red lamp.
