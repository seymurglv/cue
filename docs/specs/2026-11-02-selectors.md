# One look for every control: capsules

The settings tray, the main window and the welcome window drew their controls six ways: drop-downs as raised white
plates with a shadow, the key field as a Windows field with a line under it, the custom instructions as a soft fill,
segments with inner corners rounder than their outer one, buttons 26 px high beside 28 px fields, chevrons in two
tones. The main window's own fields (the history's search, the Dictionary's "+ Word") were already capsules with a
hairline ring. Chosen by the owner from three variants on a live render (variant B, "capsules", 02.11.2026): A was one
soft fill at radius 8, C the Windows look at radius 5.

## The rule

- **One height:** every control is 28 px: drop-downs, fields, buttons, segments, menu items.
- **One shape:** a capsule (radius 14 at 28 px). A part inside another is as far in as its radius is smaller: a segment's
  thumb and buttons sit 2 px in at radius 12; a menu's items 4 px in at radius 10 inside a menu of 14. Taller shapes
  keep the capsule's corner: the connection card and the custom instructions 14, the welcome window's text field 16.
- **Two surfaces, by role.** What holds a value or text has no fill and a hairline ring (`--faint`): fields
  (`.kin`, `.cta`, `.ofield`), drop-downs (`.dd`), the connection card (`.pk`); under the pointer the ring darkens to
  30 %. What acts is filled: buttons (`.btn2`, 7 %; the welcome window's main button, the text color), segments
  (6 %, the pressed one on a thumb).
- **Focus:** a field, a drop-down and the card show the ring darkened inside (1.5 px, 45 %), never a line under it
  or the system's ring. Buttons and rows keep the soft plate.
- **Chevrons:** one tone, `--muted`, on drop-downs, the card, the first page's rows and the model row.

## Not changed

Colors and type stay as they are. "Clear history" keeps `--warn` as its text color (2.27:1 on its fill in the light
theme); whether to darken it is still open.
