### Task 9: the main window — green book, hint, the switch

**Files:**
- Modify: `app/ui/src/window/api.ts`, `page.ts`, `dictionary.ts`, `settings.ts`, `text.ts`, `app/ui/src/design/window.css`
- Test: `app/ui/test/windowFixture.ts`, `app/ui/test/window-dictionary.test.ts`, `app/ui/test/window-settings.test.ts`

**Interfaces:**
- Consumes: `SettingsView.learned: { word: string; heard: string }[]`, `SettingsView.learn: boolean`, `cue://settings`.
- Produces: `api.onSettings(cb)`; `SettingChange` gains `{ kind: 'learn'; value: boolean }`.

- [ ] **Step 1: the failing tests.** In `windowFixture.ts`'s `view()` add `learned: []` and `learn: true`; in its
  `stub()`, simulate `dictRemove` removing from `learned` too. In `window-dictionary.test.ts`:

```ts
  it('a word from a correction has the green book and says how it was heard', async () => {
    const w = await mountWith({ dictionary: ['Marisol', 'Soniox'], learned: [{ word: 'Marisol', heard: 'Марисоль' }] });
    const chip = w.querySelector('.dch[data-word="Marisol"]') as HTMLElement;
    expect(chip.querySelector('.g svg')).not.toBeNull();
    expect(chip.title).toBe('sounds like «Марисоль»');
    expect((w.querySelector('.dch[data-word="Soniox"]') as HTMLElement).querySelector('.g')).toBeNull();
  });

  it('the hint tells what the green book means', async () => {
    const w = await mountWith({});
    const hint = w.querySelector('.dhint') as HTMLElement;
    expect(hint.textContent).toBe('Words CUE must write right.  — from your corrections: CUE noticed the change and offered the word.');
    expect(hint.querySelector('svg')).not.toBeNull();
  });

  it('settings pushed by the core redraw the dictionary', async () => {
    const w = await mountWith({});
    pushSettings({ dictionary: ['Marisol'], learned: [{ word: 'Marisol', heard: 'Марисоль' }] });
    expect(w.querySelector('.dch[data-word="Marisol"] .g')).not.toBeNull();
  });
```

(`mountWith` and `pushSettings` stand for the file's own helpers: use the existing mount helper with a view override;
for `pushSettings`, make the fixture's api stub keep the `onSettings` callback and expose a function that calls it.
Check the exact text the hint test expects against what `h()` and `icon()` produce and adjust the spaces in the
expectation only.) In `window-settings.test.ts`, next to "autostart switch calls autostart_set":

```ts
  it('"Learn from me" switch sends the learn change', async () => {
    const tray = await openGeneral();
    const sw = tray.querySelector('[data-sw="learn"]') as HTMLButtonElement;
    expect(sw.getAttribute('aria-checked')).toBe('true');
    sw.click();
    await flush();
    expect(calls).toContainEqual(['settings_set', { change: { kind: 'learn', value: false } }]);
    expect(sw.getAttribute('aria-checked')).toBe('false');
  });
```

(again with the file's own helpers for opening the General page, the recorded calls and flushing promises).

- [ ] **Step 2: run and see them fail** — `npx vitest run test/window-dictionary.test.ts test/window-settings.test.ts`.

- [ ] **Step 3: implement.**
  - `api.ts`: in `SettingsView` add `/** Words added from a correction, with how they were heard. */ learned: { word: string; heard: string }[];`
    and `/** "Learn from me". */ learn: boolean;`; in `SettingChange` add `| { kind: 'learn'; value: boolean }`; add

```ts
/** Settings the core changed by itself (a word added from the Drop's suggestion). */
export async function onSettings(cb: (v: SettingsView) => void): Promise<void> {
  await getCurrentWebviewWindow().listen<SettingsView>('cue://settings', (e) => cb(e.payload));
}
```

  - `page.ts`: next to `api.onModel(...)`: `api.onSettings((v) => this.took(v)).catch((e) => logFailed('following the settings', e));`
    (`took` is the method that already takes a view after `settingsSet`; if it is private to another object, call
    the same thing `change()` calls with the view).
  - `text.ts`: `dictHintLearned: string` (doc: "The hint's second half, after the green book.") — en
    `'— from your corrections: CUE noticed the change and offered the word.'`, ru
    `'— из ваших правок: CUE заметил замену и предложил слово.'`; `dictHeard: string` (doc: "A learned chip's tooltip;
    {HEARD} is how it was heard.") — en `'sounds like «{HEARD}»'`, ru `'звучит как «{HEARD}»'`; `learn` — en
    `'Learn from me'`, ru `'Учиться у меня'`; `learnHint` — en `'CUE notices your corrections and offers words for the dictionary. Everything stays on this computer.'`,
    ru `'CUE замечает ваши правки и предлагает слова для словаря. Всё остаётся на этом компьютере.'`.
  - `dictionary.ts` `mount`: the hint becomes

```ts
    const hint = h('p', 'dhint', t.dictHint + ' ');
    hint.insertAdjacentHTML('beforeend', icon('book'));
    hint.append(' ' + t.dictHintLearned);
```

    and in `render()`, before `chip.append(w)`:

```ts
        const learned = this.ctx.settings?.learned?.find((l) => l.word === w);
        if (learned) {
          const g = h('i', 'g');
          g.innerHTML = icon('book');
          chip.append(g);
          chip.title = fill(t.dictHeard, learned.heard);
        }
```

    (check `fill`'s placeholder convention in text.ts — `{NAME}` style — and use `{HEARD}` the same way; if `fill`
    takes one value positionally, it fills `{HEARD}`.)
  - `window.css` next to `.dhint`: `.dhint svg{display:inline-block;width:13px;height:13px;vertical-align:-2px;color:var(--ok)}`
    (it exists: check, keep one) and `.dch .g{display:grid;color:var(--ok)}.dch .g svg{width:13px;height:13px}`.
  - `settings.ts` `generalRows()`: add a second switch built like `autostart`, `sw.dataset.sw = 'learn'`,
    `aria-checked` from `this.ctx.settings?.learn ?? true`, label `t.learn`, row `srow(t.learn, t.learnHint, sw).row`.
    In `onClick`, next to the autostart branch:

```ts
    else if (t.closest('[data-sw="learn"]')) this.ctx.change({ kind: 'learn', value: !(this.ctx.settings?.learn ?? true) });
```

    and in `settingsChanged()` (called after every change) set the switch's `aria-checked` from the new view.

- [ ] **Step 4: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`: green.

---

