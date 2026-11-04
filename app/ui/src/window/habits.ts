// The Habits section (docs/specs/2026-11-04-habits.md; design stage 5, variant A): how the user corrects text after a
// dictation, learned when a correction repeats. The heading with the count and the "Learn from me" switch; the hint;
// one row per habit, the newest first — the rule, its example struck → green, where it holds and how many times, a
// switch, and "Forget" on hover; "Forget all" at the bottom on a second press. With "Learn from me" off the rows are
// muted; with no cleanup connected a line says habits need it. The core keeps the habits (`habits.list` in
// settings.json) and changes them (`habitOn`, `habitForget`, `habitsForget`); the rows follow its answers and what it
// sends by itself: a new one grows in with the highlight, a gone one folds away, one switched off is muted in place,
// and one that widened (it takes the id of everywhere) changes in place.
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
  /** Where the cleanup ran and its connection when the no-cleanup line was last checked (`runs:connection`). */
  private checked: string | null = null;
  /** The latest check of the cleanup's key: an earlier one's answer, coming later, is dropped. */
  private keyAsk = 0;
  /** The habits whose rows fold away while the core is asked to forget them: another press does nothing. */
  private readonly forgetting = new Set<string>();
  /** The habit each row shows now. */
  private readonly shown = new WeakMap<HTMLElement, HabitView>();

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
    this.empty = h('p', 'hbempty', t.habitsEmpty);
    this.more = h('div', 'bmore');
    this.forgetAll = h('button');
    this.forgetAll.type = 'button';
    this.forgetAll.dataset.hforget = '';
    this.forgetAll.append(h('span', 'lbl', t.habitsForgetAll));
    this.forgetAll.addEventListener('click', () => this.forgetAllPressed());
    this.more.append(this.forgetAll);
    sec.append(h3, h('p', 'dhint', t.habitsHint), this.noCleanup, this.list, this.empty, this.more);
    this.checked = null;
    this.render(false);
  }

  /** The habits as the core holds them now: rows kept are changed in place, new ones grow in, gone ones fold away
   *  (`animate` false: the section is drawn anew, nothing moves). */
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
    // A habit that widened has the id of everywhere now: the row of an app's habit gone with the same example is its
    // row, changed in place.
    for (const x of list) {
      if (x.app || rows.has(x.id)) continue;
      const was = [...rows].find(([id, w]) => {
        const y = this.shown.get(w);
        return !keep.has(id) && !!y?.app && y.before === x.before && y.after === x.after;
      });
      if (was) {
        rows.delete(was[0]);
        rows.set(x.id, was[1]);
      }
    }
    for (const [id, w] of rows) if (!keep.has(id)) this.fold(w, animate);
    const fresh: HTMLElement[] = [];
    let prev: HTMLElement | null = null;
    for (const x of list) {
      let w = rows.get(x.id);
      if (w) this.update(w, x);
      else {
        w = this.row(x);
        if (animate) fresh.push(w);
      }
      if (prev) {
        if (prev.nextElementSibling !== w) prev.after(w);
      } else if (this.list.firstElementChild !== w) this.list.prepend(w);
      prev = w;
    }
    for (const w of fresh) this.grow(w);
    this.empty.hidden = list.length > 0;
    this.more.hidden = list.length === 0;
    // Where the cleanup runs and its connection: a change of either asks the key again.
    const s = this.ctx.settings;
    const conn = s ? `${s.refineRuns}:${s.refineConnection}` : null;
    if (conn !== this.checked) {
      this.checked = conn;
      void this.recheck();
    }
  }

  /** The line "Habits work when the cleanup is on": shown while the cleanup runs in the cloud with no key saved for
   *  its connection (on this computer it needs none). Asked again when the window gets the focus back and when a key
   *  changes. */
  async recheck(): Promise<void> {
    if (!this.sec) return;
    const ask = ++this.keyAsk;
    const s = this.ctx.settings;
    const c = s?.connections.find((x) => x.id === s.refineConnection);
    if (!c || s?.refineRuns === 'local') {
      this.noCleanup.hidden = true;
      return;
    }
    let saved = true;
    try {
      saved = (await api.keyState(c.id as api.KeyName)).saved;
    } catch (e) {
      logFailed('reading the cleanup key', e);
    }
    if (ask === this.keyAsk) this.noCleanup.hidden = saved;
  }

  private row(x: HabitView): HTMLElement {
    const t = this.ctx.t;
    // The wrapper only grows and folds: a screen reader goes through it to the list item.
    const w = h('div', 'hw');
    w.setAttribute('role', 'none');
    const b = h('div', 'hb');
    b.setAttribute('role', 'listitem');
    const bx = h('div', 'hbx');
    bx.append(h('b', 'hrule'), h('div', 'hex'));
    const del = iconButton('ibt sm hdel', 'trash', t.habitForget);
    const sw = h('button', 'sw');
    sw.type = 'button';
    sw.setAttribute('role', 'switch');
    b.append(bx, del, sw);
    w.append(b);
    this.update(w, x);
    return w;
  }

  /** A row says what the habit is now: its id, rule, example, where, how many times, on or off. */
  private update(w: HTMLElement, x: HabitView): void {
    const t = this.ctx.t;
    this.shown.set(w, x);
    w.dataset.w = x.id;
    const b = w.firstElementChild as HTMLElement;
    b.querySelector<HTMLElement>('.hdel')!.dataset.forget = x.id;
    b.querySelector<HTMLElement>('.sw')!.dataset.hsw = x.id;
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

  /** The row folds away, then the core forgets the habit; a refusal draws it back. Once, however often it is pressed. */
  private async forget(id: string): Promise<void> {
    if (this.forgetting.has(id)) return;
    this.forgetting.add(id);
    try {
      const w = [...this.list.querySelectorAll<HTMLElement>('.hw[data-w]')].find((r) => r.dataset.w === id);
      if (w) this.fold(w, true);
      await new Promise((r) => setTimeout(r, this.ctx.reduced() ? 0 : ROW_MS));
      if (!(await this.ctx.change({ kind: 'habitForget', value: id }))) this.render(false);
    } finally {
      this.forgetting.delete(id);
    }
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

  /** A new row grows from nothing with the highlight of a new dictionary chip (at once with reduced motion). */
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
