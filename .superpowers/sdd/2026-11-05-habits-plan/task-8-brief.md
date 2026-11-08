### Task 8: the main window's Habits section (section A)

**Files:**
- Modify: `app/src-tauri/src/ipc.rs` (`SettingsView`, line 38; `of`, line 114; `Debug`, line 140)
- Create: `app/ui/src/window/habits.ts`
- Modify: `app/ui/src/window/{api,page,settings,text,icons}.ts`, `app/ui/src/design/window.css`
- Test: `app/ui/test/windowFixture.ts`, `app/ui/test/window-habits.test.ts` (new), `app/ui/test/window-settings.test.ts`

**Interfaces:**
- Consumes: `SettingChange::{Learn, HabitOn, HabitForget, HabitsForget}`, `cue://settings`.
- Produces: `SettingsView.habits: Vec<HabitView>` (`{ id, rule, before, after, app, n, on }`, `app` null for
  everywhere); TS `HabitView`, `SettingChange` gains `{ kind: 'habitOn'; value: { id: string; on: boolean } }`,
  `{ kind: 'habitForget'; value: string }`, `{ kind: 'habitsForget' }`; `HabitsView` with `mount(sec)`, `render(animate?)`,
  `recheck()`; the "Learn from me" switch (`[data-sw="learn"]`) in the Habits heading, gone from the General page.

- [ ] **Step 1: `SettingsView.habits`** (ipc.rs). Test first: in `settings_view_json_names` add `"habits"` to `expected`
  (after `"dictionary"`) and `("habits", json!([]))` to `defaults`; and

```rust
    /// The habits as the section draws them; a log shows only their count.
    #[test]
    fn settings_view_has_the_habits() {
        let mut s = Settings::default();
        let d = cue_core::habits::Draft { forms: vec![cue_core::habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let id = s.habits.adopt(d, None, "en").id;
        let view = serde_json::to_value(SettingsView::of(&s, Lang::En, true)).unwrap();
        let expected = json!([{ "id": id, "rule": "In Word: a dash between words, not a hyphen", "before": "a - b", "after": "a — b", "app": "Word", "n": 3, "on": true }]);
        assert_eq!(view["habits"], expected);
        assert!(!format!("{:?}", SettingsView::of(&s, Lang::En, true)).contains("a — b"));
    }
```

  Run `cargo test -p cue ipc` and see it fail. Then in `SettingsView` after `learn`:

```rust
    /// The habits, the newest first, as the main window's Habits section draws them (docs/specs/2026-11-04-habits.md).
    pub habits: Vec<HabitView>,
```

```rust
/// A habit as the Habits section draws it: its rule, its example, the app it holds in (`None`: everywhere), how many
/// times it was corrected, and whether it is on.
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct HabitView {
    pub id: String,
    pub rule: String,
    pub before: String,
    pub after: String,
    pub app: Option<String>,
    pub n: u32,
    pub on: bool,
}
```

  In `of`: `habits: s.habits.list.iter().map(|h| HabitView { id: h.id.clone(), rule: h.rule.clone(), before: h.before.clone(), after: h.after.clone(), app: h.app.clone(), n: h.n, on: h.on }).collect(),`;
  in its `Debug`: `.field("habits", &self.habits.len())`. Run the tests: pass.

- [ ] **Step 2: the window's failing tests.** `windowFixture.ts`: `view()` gains `habits: [],`; add

```ts
/** A habit as the core lists it. */
export function habit(id: string, over: Partial<api.HabitView> = {}): api.HabitView {
  return { id, rule: 'In Telegram — lower case and no full stop', before: 'Send the file.', after: 'send the file', app: 'Telegram', n: 3, on: true, ...over };
}
```

  and in `stub()`'s `settingsSet`, before the generic `now = { ...now, [c.kind]: c.value };`:

```ts
    if (c.kind === 'habitOn') {
      now = { ...now, habits: now.habits.map((x) => (x.id === c.value.id ? { ...x, on: c.value.on } : x)) };
      return now;
    }
    if (c.kind === 'habitForget') {
      now = { ...now, habits: now.habits.filter((x) => x.id !== c.value) };
      return now;
    }
    if (c.kind === 'habitsForget') {
      now = { ...now, habits: [] };
      return now;
    }
```

  `entry()` gains `habits: []` in its defaults (Task 9 uses it). New `test/window-habits.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, habit, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const sec = () => $('.bsec[data-scr="habits"]')!;
const rows = () => $$('.habl .hw[data-w]');
const ids = () => rows().map((r) => r.dataset.w);
const DIGITS = habit('h2', { rule: 'Numbers in digits', before: 'twelve percent', after: '12 %', app: null, n: 7 });

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub(view({ habits: [habit('h1'), DIGITS] }));
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('Habits', () => {
  it('a section after the Dictionary, with its header link, count, hint and rows', async () => {
    page = await mount();
    expect($$('.bsec').map((s) => s.dataset.scr)).toEqual(['today', 'history', 'dictionary', 'habits']);
    expect(text($$('.lk button').at(-1)!)).toBe('Habits');
    expect(text(sec().querySelector('h3 em'))).toBe('2');
    expect(text(sec().querySelector('.dhint'))).toBe(
      'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
    );
    expect(ids()).toEqual(['h1', 'h2']);
    const [first, second] = rows();
    expect(text(first.querySelector('.hrule'))).toBe('In Telegram — lower case and no full stop');
    expect(text(first.querySelector('.hex s'))).toBe('Send the file.');
    expect(text(first.querySelector('.hex ins'))).toBe('send the file');
    expect(text(first.querySelector('.hmeta'))).toBe('· in Telegram · 3 times');
    expect(text(second.querySelector('.hmeta'))).toBe('· everywhere · 7 times');
    expect(second.querySelector('.hmeta svg')).not.toBeNull();
    expect(first.querySelector('.sw')!.getAttribute('aria-checked')).toBe('true');
    expect(first.querySelector('.hdel')!.getAttribute('title')).toBe('Forget');
    expect(first.querySelector('.hdel')!.getAttribute('aria-label')).toBe('Forget: In Telegram — lower case and no full stop');
  });

  it('Russian: where and how many times', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru', habits: [habit('h1'), habit('h2', { app: null, n: 5 })] }));
    page = await mount();
    expect(text($('.hw[data-w="h1"] .hmeta'))).toBe('· в Telegram · 3 раза');
    expect(text($('.hw[data-w="h2"] .hmeta'))).toBe('· везде · 5 раз');
    expect(text(sec().querySelector('h3'))).toContain('Привычки');
  });

  it('a switch turns a habit off in place, muted', async () => {
    page = await mount();
    const row = rows()[0];
    click(row.querySelector('.sw'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitOn', value: { id: 'h1', on: false } });
    expect(rows()[0]).toBe(row);
    expect(row.querySelector('.hb')!.classList.contains('off')).toBe(true);
    expect(row.querySelector('.sw')!.getAttribute('aria-checked')).toBe('false');
  });

  it('"Forget" on a row folds it away and forgets the habit', async () => {
    page = await mount();
    click(rows()[0].querySelector('.hdel'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitForget', value: 'h1' });
    expect(ids()).toEqual(['h2']);
    expect(text(sec().querySelector('h3 em'))).toBe('1');
  });

  it('"Forget all" asks for a second press, then forgets every habit and shows the empty line', async () => {
    page = await mount();
    const b = $<HTMLButtonElement>('[data-hforget]')!;
    click(b);
    await settle();
    expect(text(b)).toBe('Press again');
    expect(core.settingsSet).not.toHaveBeenCalled();
    click(b);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitsForget' });
    expect(rows()).toEqual([]);
    expect($<HTMLElement>('.hempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
  });

  it('the first press of "Forget all" wears off after 3 s', async () => {
    page = await mount();
    const b = $('[data-hforget]')!;
    click(b);
    await vi.advanceTimersByTimeAsync(3000);
    expect(text(b)).toBe('Forget all');
    click(b);
    expect(core.settingsSet).not.toHaveBeenCalled();
  });

  it('an empty list says what will come, without "Forget all"', async () => {
    stub(view());
    page = await mount();
    expect(text($('.hempty'))).toBe('Nothing yet. CUE will notice when you correct the same thing a few times.');
    expect($<HTMLElement>('.hempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
    expect(text(sec().querySelector('h3 em'))).toBe('');
  });

  it('"Learn from me" sits in the heading; off, the rows are muted', async () => {
    page = await mount();
    const sw = $<HTMLButtonElement>('.bsec[data-scr="habits"] h3 [data-sw="learn"]')!;
    expect(sw.getAttribute('role')).toBe('switch');
    expect(sw.getAttribute('aria-checked')).toBe('true');
    click(sw);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'learn', value: false });
    expect(sw.getAttribute('aria-checked')).toBe('false');
    expect($('.habl')!.classList.contains('paused')).toBe(true);
  });

  it('settings the core sends: a new habit grows in with the highlight, one undone from the card goes', async () => {
    page = await mount();
    feeds.settings(view({ habits: [habit('h3', { rule: 'No full stop' }), habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h3', 'h1', 'h2']);
    expect(rows()[0].querySelector('.hb')!.classList.contains('new')).toBe(true);
    expect(rows()[1].querySelector('.hb')!.classList.contains('new')).toBe(false);
    feeds.settings(view({ habits: [habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h1', 'h2']);
  });

  it('with no cleanup connected, a line says habits need it', async () => {
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(false);
    expect(text($('.hnocl'))).toBe('Habits work when the cleanup is on.');
  });

  it('a key saved: no such line', async () => {
    core.keyState.mockResolvedValue({ saved: true, mask: 'sk-…789' });
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
  });

  it('a local server needs no key: no such line', async () => {
    stub(view({ refineConnection: 'ollama', refineModel: 'gemma4:12b', refineAddress: 'http://localhost:11434/v1' }));
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
  });
});
```

  In `window-settings.test.ts` replace `"Learn from me" switch sends
  the learn change` (line 787) with:

```ts
  it('the General page has "Start at sign-in" only: "Learn from me" lives in the Habits heading', async () => {
    page = await mount();
    await open();
    await sub('general');
    expect($('.btray [data-sw="learn"]')).toBeNull();
    expect($('.btray [data-sw="autostart"]')).not.toBeNull();
  });
```

  and in "settings pushed by the core leave the tray as it is…" (line 800) rename it "…: a draft stays, the Habits
  switch follows" and replace its last six lines (from `key('Escape');`) with:

```ts
    feeds.settings(view({ refineStyle: 'custom', learn: false }));
    expect($('.bsec[data-scr="habits"] [data-sw="learn"]')!.getAttribute('aria-checked')).toBe('false');
    expect($('.btray textarea.cta')).toBe(field);
```

  Run `npx vitest run test/window-habits.test.ts test/window-settings.test.ts` and see them fail.

- [ ] **Step 3: `api.ts`.** After `SettingsView`'s `learn`: `/** The habits, the newest first (docs/specs/2026-11-04-habits.md). */ habits: HabitView[];`
  and above `SettingsView`:

```ts
/** A habit as the Habits section draws it: its rule, its example, the app it holds in (null — everywhere), how many
 *  times it was corrected, and whether it is on. */
export interface HabitView {
  id: string;
  rule: string;
  before: string;
  after: string;
  app: string | null;
  n: number;
  on: boolean;
}
```

  `SettingChange` gains `| { kind: 'habitOn'; value: { id: string; on: boolean } } | { kind: 'habitForget'; value: string } | { kind: 'habitsForget' }`.

- [ ] **Step 4: `text.ts`.** In `WinText`, after `dictRemoveWord`, with doc comments; remove `learnHint` (interface, en,
  ru: the General row is gone); keep `learn`:

```ts
  /** The Habits section (docs/specs/2026-11-04-habits.md): its heading (also the header's link), the hint, the line
   *  when no cleanup is connected, the empty list, "Forget all" and its second press, a row's forget button (its name
   *  for a screen reader: {NAME} — the rule), its switch's tooltips, where it holds ({NAME} — the app), how many times,
   *  and the History's chips for a screen reader. */
  habits: string;
  habitsHint: string;
  habitsNoCleanup: string;
  habitsEmpty: string;
  habitsForgetAll: string;
  habitsForgetAgain: string;
  habitForget: string;
  habitForgetName: string;
  habitOff: string;
  habitOn: string;
  habitEverywhere: string;
  habitIn: string;
  habitTimes: Forms;
  habitsApplied: string;
```

  `en`:

```ts
  habits: 'Habits',
  habitsHint: 'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
  habitsNoCleanup: 'Habits work when the cleanup is on.',
  habitsEmpty: 'Nothing yet. CUE will notice when you correct the same thing a few times.',
  habitsForgetAll: 'Forget all',
  habitsForgetAgain: 'Press again',
  habitForget: 'Forget',
  habitForgetName: 'Forget: {NAME}',
  habitOff: 'Turn off',
  habitOn: 'Turn on',
  habitEverywhere: 'everywhere',
  habitIn: 'in {NAME}',
  habitTimes: ['time', 'times'],
  habitsApplied: 'Habits applied',
```

  `ru`:

```ts
  habits: 'Привычки',
  habitsHint: 'Как вы правите текст после диктовки. Когда правка повторяется, CUE запоминает её и дальше пишет так сам. Хранится только на этом компьютере.',
  habitsNoCleanup: 'Привычки работают, когда включена очистка.',
  habitsEmpty: 'Пока ничего. CUE заметит, если вы несколько раз поправите одно и то же.',
  habitsForgetAll: 'Забыть всё',
  habitsForgetAgain: 'Нажмите ещё раз',
  habitForget: 'Забыть',
  habitForgetName: 'Забыть: {NAME}',
  habitOff: 'Выключить',
  habitOn: 'Включить',
  habitEverywhere: 'везде',
  habitIn: 'в {NAME}',
  habitTimes: ['раз', 'раза', 'раз'],
  habitsApplied: 'Применённые привычки',
```

- [ ] **Step 5: `icons.ts`.** In `OWN` after `pause`, the trash of design stage 5 on the same grid:
  `trash: SV('<path d="M4 5.8h12M8 5.8V4h4v1.8M5.6 5.8l.7 10.4h7.4l.7-10.4"/>'),`; `IconName` gains `| 'habit'` (the
  shared set's); the header comment adds "the trash of a habit's row".

- [ ] **Step 6: `habits.ts`** (new):

```ts
// The Habits section (docs/specs/2026-11-04-habits.md; design stage 5, variant A): how the user corrects text after a
// dictation, learned when a correction repeats. The heading with the count and the "Learn from me" switch; the hint;
// one row per habit, the newest first — the rule, its example struck → green, where it holds and how many times, a
// switch, and "Forget" on hover; "Forget all" at the bottom on a second press. With "Learn from me" off the rows are
// muted; with no cleanup connected a line says habits need it. The core keeps the habits (`habits.list` in
// settings.json) and changes them (`habitOn`, `habitForget`, `habitsForget`); the rows follow its answers and what it
// sends by itself: a new one grows in with the highlight, a gone one folds away, one switched off is muted in place.
// Text goes into the page as text.
import { CUE } from '../designParts';
import * as api from './api';
import type { HabitView } from './api';
import { icon } from './icons';
import { h, iconButton, logFailed, type Ctx } from './page';
import { CLEAR_ARMED_MS } from './settings';
import { fill, plural } from './text';

/** How long a row takes to grow in or fold away (window.css: .hw.anim). */
const ROW_MS = 450;

export class HabitsView {
  private sec: HTMLElement | null = null;
  private count!: HTMLElement;
  private learn!: HTMLButtonElement;
  private list!: HTMLElement;
  private empty!: HTMLElement;
  private noCleanup!: HTMLElement;
  private more!: HTMLElement;
  private forgetAll!: HTMLButtonElement;
  private armed: ReturnType<typeof setTimeout> | null = null;
  /** The habits drawn: one not among them comes in with the highlight. */
  private drawn = new Set<string>();
  /** The connection the no-cleanup line was checked for. */
  private checked: string | null = null;

  constructor(private readonly ctx: Ctx) {}

  private get habits(): HabitView[] {
    return this.ctx.settings?.habits ?? [];
  }

  private get learning(): boolean {
    return this.ctx.settings?.learn ?? true;
  }

  /** Draws the section into `sec` (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    const t = this.ctx.t;
    this.sec = sec;
    sec.textContent = '';
    this.disarm();
    const h3 = h('h3', '', t.habits);
    this.count = h('em', 'lbl');
    const right = h('span', 'h3r', t.learn);
    this.learn = h('button', 'sw');
    this.learn.type = 'button';
    this.learn.dataset.sw = 'learn';
    this.learn.setAttribute('role', 'switch');
    this.learn.setAttribute('aria-label', t.learn);
    this.learn.addEventListener('click', () => void this.ctx.change({ kind: 'learn', value: !this.learning }));
    right.append(this.learn);
    h3.append(this.count, right);
    this.noCleanup = h('p', 'hnocl', t.habitsNoCleanup);
    this.noCleanup.hidden = true;
    this.list = h('div', 'habl');
    this.list.setAttribute('role', 'list');
    this.list.addEventListener('click', (e) => this.onClick(e));
    this.empty = h('p', 'hempty', t.habitsEmpty);
    this.more = h('div', 'bmore');
    this.forgetAll = h('button');
    this.forgetAll.type = 'button';
    this.forgetAll.dataset.hforget = '';
    this.forgetAll.append(h('span', 'lbl', t.habitsForgetAll));
    this.forgetAll.addEventListener('click', () => this.forgetAllPressed());
    this.more.append(this.forgetAll);
    sec.append(h3, h('p', 'dhint', t.habitsHint), this.noCleanup, this.list, this.empty, this.more);
    this.drawn.clear();
    this.checked = null;
    this.render(false);
  }

  /** The habits as the core holds them now: rows kept are changed in place, new ones grow in, gone ones fold away. */
  render(animate = true): void {
    if (!this.sec) return;
    const list = this.habits;
    const n = list.length ? String(list.length) : '';
    if (animate) CUE().setLabel(this.count, n);
    else {
      this.count.textContent = n;
      this.count.dataset.t = n;
    }
    this.learn.setAttribute('aria-checked', String(this.learning));
    this.list.classList.toggle('paused', !this.learning);
    const rows = new Map([...this.list.querySelectorAll<HTMLElement>('.hw[data-w]')].map((w) => [w.dataset.w!, w]));
    const keep = new Set(list.map((x) => x.id));
    for (const [id, w] of rows) if (!keep.has(id)) this.fold(w, animate);
    const fresh: HTMLElement[] = [];
    let prev: HTMLElement | null = null;
    for (const x of list) {
      let w = rows.get(x.id);
      if (w) this.update(w, x);
      else {
        w = this.row(x);
        if (this.drawn.size || animate) fresh.push(w);
      }
      if (prev) {
        if (prev.nextElementSibling !== w) prev.after(w);
      } else if (this.list.firstElementChild !== w) this.list.prepend(w);
      prev = w;
    }
    for (const w of fresh) this.grow(w);
    this.drawn = keep;
    this.empty.hidden = list.length > 0;
    this.more.hidden = list.length === 0;
    const conn = this.ctx.settings?.refineConnection ?? null;
    if (conn !== this.checked) {
      this.checked = conn;
      void this.recheck();
    }
  }

  /** The line "Habits work when the cleanup is on": shown while the cleanup's connection is a cloud one with no key
   *  saved (a local server needs none). Asked again when the window gets the focus back. */
  async recheck(): Promise<void> {
    if (!this.sec) return;
    const s = this.ctx.settings;
    const c = s?.connections.find((x) => x.id === s.refineConnection);
    if (!c || c.local) {
      this.noCleanup.hidden = true;
      return;
    }
    try {
      this.noCleanup.hidden = (await api.keyState(c.id as api.KeyName)).saved;
    } catch (e) {
      logFailed('reading the cleanup key', e);
      this.noCleanup.hidden = true;
    }
  }

  private row(x: HabitView): HTMLElement {
    const t = this.ctx.t;
    const w = h('div', 'hw');
    w.dataset.w = x.id;
    const b = h('div', 'hb');
    b.setAttribute('role', 'listitem');
    const bx = h('div', 'hbx');
    bx.append(h('b', 'hrule'), h('div', 'hex'));
    const del = iconButton('ibt sm hdel', 'trash', t.habitForget);
    del.dataset.forget = x.id;
    const sw = h('button', 'sw');
    sw.type = 'button';
    sw.dataset.hsw = x.id;
    sw.setAttribute('role', 'switch');
    b.append(bx, del, sw);
    w.append(b);
    this.update(w, x);
    return w;
  }

  /** A row says what the habit is now: its rule, example, where, how many times, on or off. */
  private update(w: HTMLElement, x: HabitView): void {
    const t = this.ctx.t;
    const b = w.firstElementChild as HTMLElement;
    b.classList.toggle('off', !x.on);
    b.querySelector('.hrule')!.textContent = x.rule;
    const ex = b.querySelector('.hex')!;
    ex.textContent = '';
    const where = h('span', 'hmeta', '· ');
    if (x.app) where.append(fill(t.habitIn, x.app));
    else {
      where.insertAdjacentHTML('beforeend', icon('globe'));
      where.append(t.habitEverywhere);
    }
    where.append(` · ${x.n}\u00a0${plural(x.n, t.habitTimes)}`);
    ex.append(h('s', '', x.before), ' → ', h('ins', '', x.after), where);
    b.querySelector('.hdel')!.setAttribute('aria-label', fill(t.habitForgetName, x.rule));
    const sw = b.querySelector<HTMLElement>('.sw')!;
    sw.setAttribute('aria-checked', String(x.on));
    sw.setAttribute('aria-label', x.rule);
    sw.title = x.on ? t.habitOff : t.habitOn;
  }

  private onClick(e: Event): void {
    const el = e.target as Element;
    const sw = el.closest<HTMLElement>('[data-hsw]');
    if (sw) {
      const id = sw.dataset.hsw!;
      const x = this.habits.find((y) => y.id === id);
      if (x) void this.ctx.change({ kind: 'habitOn', value: { id, on: !x.on } });
      return;
    }
    const del = el.closest<HTMLElement>('[data-forget]');
    if (del) void this.forget(del.dataset.forget!);
  }

  /** The row folds away, then the core forgets the habit; a refusal draws it back. */
  private async forget(id: string): Promise<void> {
    const w = this.list.querySelector<HTMLElement>(`.hw[data-w="${id}"]`);
    if (w) this.fold(w, true);
    await new Promise((r) => setTimeout(r, this.ctx.reduced() ? 0 : ROW_MS));
    if (!(await this.ctx.change({ kind: 'habitForget', value: id }))) this.render(false);
  }

  /** "Forget all" works on a second press within CLEAR_ARMED_MS, as "Clear history" does. */
  private forgetAllPressed(): void {
    const t = this.ctx.t;
    const l = this.forgetAll.querySelector<HTMLElement>('.lbl')!;
    if (!this.armed) {
      this.armed = setTimeout(() => {
        this.armed = null;
        this.forgetAll.classList.remove('armed');
        if (l.isConnected) CUE().setLabel(l, t.habitsForgetAll);
      }, CLEAR_ARMED_MS);
      this.forgetAll.classList.add('armed');
      CUE().setLabel(l, t.habitsForgetAgain);
      return;
    }
    this.disarm();
    this.forgetAll.classList.remove('armed');
    CUE().setLabel(l, t.habitsForgetAll);
    void this.ctx.change({ kind: 'habitsForget' });
  }

  private disarm(): void {
    if (this.armed) clearTimeout(this.armed);
    this.armed = null;
  }

  /** A new row grows from nothing with the highlight of a new dictionary chip. */
  private grow(w: HTMLElement): void {
    (w.firstElementChild as HTMLElement).classList.add('new');
    if (this.ctx.reduced()) return;
    w.classList.add('anim', 'shut');
    void w.offsetHeight;
    w.classList.remove('shut');
    w.style.height = `${w.scrollHeight}px`;
    setTimeout(() => {
      w.classList.remove('anim');
      w.style.height = '';
    }, ROW_MS);
  }

  /** A gone row folds away (at once with reduced motion or when nothing moves). */
  private fold(w: HTMLElement, animate: boolean): void {
    delete w.dataset.w;
    if (!animate || this.ctx.reduced()) {
      w.remove();
      return;
    }
    w.style.height = `${w.offsetHeight}px`;
    w.classList.add('anim');
    void w.offsetHeight;
    w.classList.add('shut');
    setTimeout(() => w.remove(), ROW_MS);
  }
}
```

- [ ] **Step 7: `page.ts`.** Import `HabitsView`; `readonly habits: HabitsView;` with the others and
  `this.habits = new HabitsView(this);` after `this.dict = …`. In `build()`: `const habitsSec = section('habits', t.habits);`,
  `this.sections = [todaySec, sec, dictSec, habitsSec];`, the links gain `['habits', t.habits]`,
  `col.append(todaySec, sec, dictSec, habitsSec);`, and after `this.dict.mount(dictSec);` `this.habits.mount(habitsSec);`.
  In `took()`'s else branch, after `this.dict.render();`: `this.habits.render();`. `pushed()` becomes:

```ts
  /** Settings the core sent by itself (a word learned or taken back from the Drop, a habit learned, widened or undone
   *  from its card): the Dictionary and the Habits are drawn again. The tray is not redrawn: a draft in it (the
   *  instructions are saved when their field loses focus) stays as typed. */
  private pushed(view: SettingsView): void {
    this.settings = view;
    this.dict.render();
    this.habits.render();
  }
```

  In the window's `focus` listener (constructor): `void this.habits.recheck();`. The header comment's first sentence
  gains "the Dictionary and Habits sections".

- [ ] **Step 8: `settings.ts`.** `generalRows()` (line 803) returns `[srow(t.autostart, null, sw).row]` (the `learn`
  switch goes); the `[data-sw="learn"]` branch of `onClick` (line 465) goes; `learnChanged()` (line 298) goes.

- [ ] **Step 9: `window.css`**, after the Dictionary's rules (`.dch .g`), from the approved variant A:

```css
/* The Habits section (docs/specs/2026-11-04-habits.md, design stage 5, variant A): the switch at the heading's right,
   a row per habit — the rule, its example struck → green, where and how many times, "Forget" on hover and a switch. Off
   (a habit, or "Learn from me"), the words take --muted: never opacity, so they keep their contrast. */
.bsec>h3 .h3r{order:6;display:flex;align-items:center;gap:8px;font-size:12px;font-weight:400;color:var(--muted)}
.bsec>h3 em.lbl{display:inline-flex}
.hnocl{margin:-4px 0 10px;font-size:12px;color:var(--warn-ink)}
.habl{display:flex;flex-direction:column;margin:0 -8px}
.hw.anim{overflow:hidden;transition:height .45s var(--e-sm),opacity .3s}
.hw.shut{height:0!important;opacity:0}
.hb{position:relative;display:grid;grid-template-columns:minmax(0,1fr) auto auto;column-gap:10px;align-items:center;padding:8px;border-radius:8px;transition:background .2s}
.hb:hover{background:color-mix(in srgb,var(--text) 4%,transparent)}
.hb.new{animation:rowIn .6s var(--e-spring),habHl 2.8s var(--e-out)}
@keyframes habHl{0%,30%{background:color-mix(in srgb,var(--ok) 14%,transparent)}}
.hbx{min-width:0}
.hrule{display:block;font-size:13px;font-weight:500;line-height:20px;color:var(--text);transition:color .25s}
.hex{font-size:12px;line-height:18px;color:var(--muted)}
.hex s{text-decoration-thickness:1px}
.hex ins{-webkit-box-decoration-break:clone;box-decoration-break:clone;text-decoration:none;color:var(--text);background:color-mix(in srgb,var(--ok) 15%,transparent);border-radius:3px;padding:0 3px;transition:background .25s,color .25s}
.hmeta{margin-left:6px;white-space:nowrap}
.app .hmeta svg{display:inline-block;width:12px;height:12px;vertical-align:-2px;margin-right:3px}
.hdel{opacity:0;transition:opacity .2s}
.hb:hover .hdel,.hdel:focus-visible{opacity:1}
.hb.off .hrule,.habl.paused .hrule{color:var(--muted)}
.hb.off ins,.habl.paused ins{background:color-mix(in srgb,var(--text) 6%,transparent);color:var(--muted)}
.hempty{margin:0;font-size:12px;color:var(--muted);animation:msIn .3s var(--e-out) both}
.bmore button.armed{color:var(--warn-ink)}
@media (prefers-reduced-motion:reduce){.hb.new{animation:none}.hw.anim{transition:none}.hempty{animation:none}}
```

  Add `.hdel` to the `:focus-visible` outline list at line 429 only if `.ibt` is not already in it.

- [ ] **Step 10: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`, `cargo test -p cue`: green.

---


## Coordinator rulings for this task (binding, override the text above)
- Use a new class for the Habits empty line (e.g. `.hbempty`), not `.hempty` (taken by the History).
- Update the existing tests that the moved switch and the new section change: window-settings.test.ts (~336, 'Learn from me' no longer on the General page), window-today.test.ts (~128, four header links), window-history.test.ts (~93-94, scope the `.bmore` query to the History section).
