### Task 7: the card on the Drop (`newHabit`)

**Files:**
- Modify: `app/ui/src/design/drop.js` (`IC`, line 14), `app/ui/src/strings.ts`, `app/ui/src/trays.ts`,
  `app/ui/src/controller.ts`, `app/ui/src/main.ts` (line 65)
- Test: `app/ui/test/controller.test.ts`

**Interfaces:**
- Consumes: `cue://tray` `"newHabit"`, `cue://habit-rule` (a string), `trayAction` / `trayClose` as for the other cards.
- Produces: `TrayKind` gains `'newHabit'`; `TRAYS.newHabit = { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 }`;
  `DropController.habitRule(rule: string)`; `trayHtml(kind, t, names?, model?, rule?)` (`{RULE}` in a body, escaped);
  `CUE.IC.habit`.

- [ ] **Step 1: the failing tests.** In `controller.test.ts` add `'newHabit'` to `KINDS` (line 24), and after the
  "Tray card closes on a timer" block:

```ts
describe('New habit card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — lower case and no full stop';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the rule under "New habit", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    expect(box().querySelector('.nhd b')!.textContent).toBe('New habit');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s; hovering pauses it', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    box().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(30_000);
    expect(isOpen()).toBe(true);
    box().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Removed", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    act().click();
    expect(sent).toEqual(['trayAction']);
    const letters = [...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('');
    expect(letters).toBe('Removed');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });

  it('the rule goes in as text', () => {
    ctl.habitRule('<b>x</b>');
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd b')).toBeNull();
    expect(box().querySelector('.nbd')!.textContent).toContain('<b>x</b>');
  });
});
```

  And in the "Russian interface" block:

```ts
  it('the habit card in Russian: «Отменить» becomes «Убрано»', () => {
    ctl.habitRule('В Telegram — с маленькой буквы и без точки');
    ctl.tray('newHabit');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Новая привычка');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с маленькой буквы и без точки. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Убрано');
  });
```

  Run `npx vitest run test/controller.test.ts` (from `app`) and see them fail.

- [ ] **Step 2: the icon.** In `drop.js`'s `IC`, after `book`, drawn on the 20 grid with the 1.8 stroke (design stage 5's
  habit icon, a pen with a plus):

```js
 habit:SV('<path d="M4 16.2c2.2-.2 3.6-1 4.8-2.2l6.6-6.6a1.6 1.6 0 0 0-2.3-2.3L6.5 11.7C5.3 12.9 4.5 14.3 4.3 16"/><path d="M15.6 12.6v3M14.1 14.1h3"/>'),
```

- [ ] **Step 3: `strings.ts`.** `TrayKind` (line 4) gains `| 'newHabit'`; `TrayLook.icon` gains `| 'habit'`,
  `TrayLook.act` gains `| 'undo'`; `TrayText.body`'s comment adds "{RULE} the habit's rule, escaped"; `TRAYS` gains
  `newHabit: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },`; `en.tray` gains

```ts
    newHabit: {
      title: 'New habit',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Removed',
    },
```

  and `ru.tray`

```ts
    newHabit: {
      title: 'Новая привычка',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Убрано',
    },
```

- [ ] **Step 4: `trays.ts`.** `trayHtml` takes `rule?: string | null` after `model`, and its body chain gains
  `.replace('{RULE}', () => escapeHtml(rule ?? ''))`; its doc comment says "{RULE} the habit's rule".

- [ ] **Step 5: `controller.ts`.** After `private model` (line 76): `private rule: string | null = null;`; after
  `modelName` (line 278):

```ts
  /** The rule of the habit just learned, for the card that follows. */
  habitRule(rule: string) {
    this.rule = String(rule ?? '');
  }
```

  and line 285: `trayHtml(kind, this.t, this.mic, this.model, this.rule)`. The existing `done` path gives "Undo" →
  "Removed" by shared letters and closes the card after `DONE_HOLD_MS`; nothing else changes.

- [ ] **Step 6: `main.ts`.** After the `cue://model-name` listener:
  `listen<string>('cue://habit-rule', (e) => ctl.habitRule(e.payload)),`.

- [ ] **Step 7: run** `npx vitest run` and `npx tsc --noEmit -p tsconfig.json` from `app`: green.

---


## Coordinator rulings for this task (binding, override the text above)
- The existing test that renders every tray card in every language must fill `{RULE}` for `newHabit` (pass a sample rule) — update that test, do not exclude the new kind.

- Task 6 (core) contract: the rule arrives on `cue://habit-rule` just before `cue://tray` = `newHabit`; "Undo" is the card's existing tray action (`trayAction`); after it the core holds the Drop busy 1100 + 430 ms itself, so the page keeps its current behaviour: show the done label ("Removed") for DONE_HOLD_MS, then close the card WITHOUT sending trayClose.
