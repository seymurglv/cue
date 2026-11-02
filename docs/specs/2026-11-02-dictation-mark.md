# The dictation mark in the main window's header

The main window's header used the Drop's live mark in its idle pose before the word CUE: a blinking upright bar (a
caret) and the red lamp. The owner wanted a mark that says "dictation" at a glance and keeps the brand's pulsing red
dot. Chosen from three variants on the real window (a microphone, a sound wave, a microphone with two arcs of speech):
the sound wave.

## The mark

On the window's 20 grid, stroke 1.8, round caps, the text color (`page.ts`, `MARK`):

- three voice bars at x 3.2 / 8.2 / 13.2, heights 10 / 14 / 11.2 (the Drop's proportions), centered on y 10;
- the lamp: a circle r 1.7 at (18.2, 3.2), `--rec`, top right of the bars as on the Drop.

## Three bars everywhere

The first mark had five bars at rest and three while recording. The owner wanted one mark everywhere: the same three
bars from rest through start into recording, only their heights changing. Chosen from two waves on the live Drop
(even, 10 / 16 / 10; in the voice's proportions, 11.5 / 16 / 13): the voice's proportions.

## Motion

- The bars roll as a slow wave: each scales between 45 % and 100 % of its height around its middle, 2.4 s a cycle,
  each 0.5 s after the one before it.
- The lamp pulses on its own: 1.8 s, from 80 % size at 55 % opacity to 110 % at full.
- Both are CSS animations (`window.css`), with no frame loop in the page. With reduced motion both stand still.

## Everywhere else the bar was

The owner asked for the mark wherever the blinking bar stood:

- **The Drop's idle mark** (`cue.js`, the `idle` pose, also the "Ask" card's resting cursor and the welcome window's
  large mark): on its 24 grid, the three voice bars at x 7.5 / 12 / 16.5 (lines 0, 2, 4; the other two parked between
  them), 11.52 / 16 / 12.8 high on y 12 (16 times the voice's proportions .72 / 1 / .8), a 2.2 stroke; the lamp at
  (21, 4.5), r 1.9. Live, the bars roll as the same wave and the lamp fades between 55 % and 100 % opacity; its place
  and size stay, so the hover capsule's even margins (`drop.css`, `--bar-l`, `--dot-r`) hold.
- **The start of a dictation** (`start`) is listening before the microphone has sent anything: the wave gathers into
  the three voice bars at rest (x 7.5 / 12 / 16.5, lines 0, 2, 4, 3 high; the other two fade where they stand) while
  the lamp grows red at their top right (21, 4.5), the spot ask and translate park their hidden lamp at. Listening
  then changes nothing: the bars move only with the voice. The first version had start's own five flickering bars and
  a larger lamp; going into listening, two bars faded, the others slid and the lamp shrank, which read as a glitch.
  The owner asked for one motion. Of three on the live Drop (as is, the bars at rest, a faint made-up tremor) the last
  two looked the same to the owner; the bars at rest were taken, as they show only what the microphone sends. Five
  bars in start and listening too, as at rest, were tried on the live Drop; the owner kept three, and then asked for
  three at rest as well (above).
- **The Drop at rest**: the strip breathes, its opacity between 40 % and 72 % over 3.2 s; hovered or with reduced
  motion it holds still. Chosen from four variants (as is, breathing, a shine, a row of bars).
- **The key hint** on hover: soft plates, a 12 % white fill with no outline, the system type at 11 (in the 40 px
  drawing) instead of Bebas. Chosen from four variants (as is, soft plates, a quiet caption, one capsule).
- **The tray icon** (`trayicon.rs`): the same three bars and lamp, still; monochrome at rest, the lamp red and larger
  while recording, faded on pause. The frame is sized by the large lamp so the mark does not shift.
- **The app icon** (`icons/app-icon.svg` and every size made from it): the graphite plate with the three bars and the
  red lamp, what they paint centered on the plate.
