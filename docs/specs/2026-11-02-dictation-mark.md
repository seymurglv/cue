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

The Drop keeps its own live mark. Only the main window's header changes.
