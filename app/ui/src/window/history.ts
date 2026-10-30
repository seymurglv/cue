// The History section (spec 3 §2): results by day — today and yesterday, earlier days on request — with search, copy,
// paste again, the period the history keeps and clearing it. New entries come over the window's channel and appear at
// the top of their day; one already shown (the channel and the first page can both bring it) is not shown twice.
// Text goes in as text: an entry may hold anything a model wrote.
import * as api from './api';
import { KEEPS, type DayView, type EntryView, type Keep } from './api';
import { icon, type IconName } from './icons';
import { flash, h, iconButton, logFailed, type Ctx } from './page';
import { CUE } from '../designParts';
import { dayLabel, daysBefore } from './text';

/** A search runs this long after the last key. */
export const SEARCH_PAUSE_MS = 150;
/** "Clear history" waits this long for its second press. */
export const CLEAR_ARMED_MS = 3000;

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const MODE_ICON: Record<api.Mode, IconName> = { dictation: 'micOn', translation: 'tr', ask: 'ask' };

/** The local date an entry came: its id starts with its time. */
function entryDate(e: EntryView, today: string): string {
  const d = e.id.slice(0, 10);
  return DATE.test(d) ? d : today;
}

/** Newest first: ids start with the entry's time. */
const newestFirst = (a: EntryView, b: EntryView) => (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);

/** `text` into `parent`, the first place that holds `query` (in any case) marked. */
function appendMarked(parent: HTMLElement, text: string, query: string): void {
  const q = query.toLowerCase();
  const low = text.toLowerCase();
  const i = q && low.length === text.length ? low.indexOf(q) : -1;
  if (i < 0) {
    parent.append(text);
    return;
  }
  parent.append(text.slice(0, i), h('mark', '', text.slice(i, i + q.length)), text.slice(i + q.length));
}

export class HistoryView {
  /** The ordinary list: days newest first, as far back as shown. */
  private days: DayView[] = [];
  private more = false;
  private readonly ids = new Set<string>();
  private loaded = false;
  /** Entries that came before the first page. */
  private pending: EntryView[] = [];
  /** The search box's text, and what it found while a search is shown (null — the ordinary list). */
  private query = '';
  private searchOpen = false;
  private results: DayView[] | null = null;
  private searchSeq = 0;
  private searchTimer: ReturnType<typeof setTimeout> | null = null;
  private clearTimer: ReturnType<typeof setTimeout> | null = null;
  private sec: HTMLElement | null = null;
  private list!: HTMLElement;
  private emptyEl!: HTMLElement;
  private earlierBtn!: HTMLButtonElement;
  private keepDd!: HTMLButtonElement;
  private bsr!: HTMLElement;
  private input!: HTMLInputElement;
  private clearLbl!: HTMLElement;

  constructor(private readonly ctx: Ctx) {}

  /** Draws the section into `sec` from what it holds (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    const t = this.ctx.t;
    this.sec = sec;
    sec.textContent = '';
    const h3 = h('h3', '', t.history);
    const find = iconButton('ibt sm', 'search', t.search);
    find.addEventListener('click', () => this.toggleSearch());
    h3.append(find);

    this.bsr = h('div', 'bsr');
    this.bsr.classList.toggle('open', this.searchOpen);
    const label = h('label', 'srch');
    label.innerHTML = icon('search');
    this.input = h('input');
    this.input.placeholder = t.searchHint;
    this.input.setAttribute('aria-label', t.searchHint);
    this.input.spellcheck = false;
    this.input.value = this.query;
    this.input.addEventListener('input', () => this.typed());
    label.append(this.input);
    const inner = h('div');
    inner.append(label);
    this.bsr.append(inner);

    this.list = h('div', 'bhist');
    this.list.addEventListener('click', (e) => this.onRowClick(e));
    this.emptyEl = h('p', 'hempty');

    const more = h('div', 'bmore');
    this.earlierBtn = h('button', '', t.earlier);
    this.earlierBtn.type = 'button';
    this.earlierBtn.addEventListener('click', () => void this.earlier());
    this.keepDd = h('button', 'dd');
    this.keepDd.type = 'button';
    this.keepDd.dataset.dd = 'hkeep';
    this.keepDd.append(h('span', 'ddv'));
    this.keepDd.insertAdjacentHTML('beforeend', icon('chev'));
    this.keepDd.addEventListener('click', () => this.keepMenu());
    more.append(this.earlierBtn, h('span', 'sp'), h('span', '', t.keep), this.keepDd);

    const clearRow = h('div', 'bmore');
    const clear = h('button');
    clear.type = 'button';
    clear.dataset.hclear = '';
    this.clearLbl = h('span', 'lbl', t.clear);
    clear.append(this.clearLbl);
    clear.addEventListener('click', () => this.clearPressed());
    clearRow.append(clear);
    if (this.clearTimer) {
      clearTimeout(this.clearTimer);
      this.clearTimer = null;
    }

    sec.append(h3, this.bsr, this.list, this.emptyEl, more, clearRow);
    this.settingsChanged();
    this.render();
  }


  /** The first page; entries that came over the channel meanwhile go on top of it. */
  async load(): Promise<void> {
    let page: api.PageView = { days: [], more: false };
    try {
      page = await api.historyPage(null);
    } catch (e) {
      logFailed('reading the history', e);
    }
    this.days = [];
    this.ids.clear();
    this.merge(page.days);
    this.more = page.more;
    this.loaded = true;
    const pending = this.pending;
    this.pending = [];
    for (const e of pending) this.merge([{ date: entryDate(e, this.ctx.today()), entries: [e] }]);
    this.render();
    if (this.results !== null) this.search();
  }

  /** The list again from the first page: the period changed, so older days may be gone. */
  reload(): Promise<void> {
    return this.load();
  }

  /** A new entry from the core: on top of its day (today), unless it is shown already or a search is shown. */
  add(e: EntryView): void {
    if (!this.loaded) {
      this.pending.push(e);
      return;
    }
    const date = entryDate(e, this.ctx.today());
    if (!this.merge([{ date, entries: [e] }]) || this.results !== null || !this.sec) return;
    let group = this.list.querySelector<HTMLElement>(`.hgrp[data-day="${date}"]`);
    if (!group) {
      group = this.group({ date, entries: [] });
      const after = [...this.list.children].find((g) => ((g as HTMLElement).dataset.day ?? '') < date);
      this.list.insertBefore(group, after ?? null);
    }
    const row = this.row(e, '');
    row.classList.add('new');
    group.querySelector('.hl')!.prepend(row);
    this.emptyEl.hidden = true;
  }

  /** The period shown next to "Keep": unset when the core does not know it. */
  settingsChanged(): void {
    const keep = this.ctx.settings?.historyKeep ?? null;
    this.keepDd.querySelector('.ddv')!.textContent = keep ? this.ctx.t.keeps[keep] : this.ctx.t.keepUnset;
  }

  /** Days into the ordinary list (merged by date, an entry shown once); whether anything was new. */
  private merge(days: DayView[]): boolean {
    let added = false;
    for (const d of days) {
      if (!DATE.test(d.date)) continue;
      const fresh = d.entries.filter((e) => !this.ids.has(e.id));
      if (!fresh.length) continue;
      for (const e of fresh) this.ids.add(e.id);
      added = true;
      const day = this.days.find((x) => x.date === d.date);
      if (day) {
        day.entries = [...day.entries, ...fresh].sort(newestFirst);
      } else {
        const at = this.days.findIndex((x) => x.date < d.date);
        const copy = { date: d.date, entries: [...fresh].sort(newestFirst) };
        if (at < 0) this.days.push(copy);
        else this.days.splice(at, 0, copy);
      }
    }
    return added;
  }

  private render(): void {
    if (!this.sec) return;
    const days = this.results ?? this.days;
    const q = this.results !== null ? this.query.trim() : '';
    this.list.textContent = '';
    for (const d of days) this.list.append(this.group(d, q));
    this.earlierBtn.hidden = this.results !== null || !this.more;
    this.emptyEl.hidden = days.length > 0 || (this.results === null && this.more);
    this.emptyEl.textContent = this.results !== null ? this.ctx.t.nothing : this.ctx.t.empty;
  }

  private group(d: DayView, q = ''): HTMLElement {
    const g = h('div', 'hgrp');
    g.dataset.day = d.date;
    g.append(h('div', 'hday', dayLabel(d.date, this.ctx.today(), this.ctx.t, this.ctx.lang)));
    const hl = h('div', 'hl');
    for (const e of d.entries) hl.append(this.row(e, q));
    g.append(hl);
    return g;
  }

  /** time · mode · [app] [question] text, in one line; the whole of it in the tooltip; copy and paste again on hover. */
  private row(e: EntryView, q: string): HTMLElement {
    const t = this.ctx.t;
    const r = h('div', 'hr');
    r.dataset.id = e.id;
    const ha = h('span', 'ha');
    const badge = h('span', e.mode === 'ask' ? 'aski' : 'hmi');
    const known = Object.hasOwn(MODE_ICON, e.mode);
    badge.innerHTML = icon(known ? MODE_ICON[e.mode] : 'micOn');
    badge.title = known ? t.modes[e.mode] : '';
    ha.append(badge);
    const hx = h('div', 'hx');
    const htx = h('div', 'htx');
    if (e.app) htx.append(h('span', 'happ', e.app));
    if (e.question) {
      const hq = h('span', 'hq');
      appendMarked(hq, e.question, q);
      htx.append(hq);
    }
    appendMarked(htx, e.text, q);
    htx.title = e.question ? `${e.question}\n\n${e.text}` : e.text;
    hx.append(htx);
    const act = h('span', 'hact');
    const copy = iconButton('ibt sm', 'copy', t.copy);
    copy.dataset.copy = '';
    const paste = iconButton('ibt sm', 'reins', t.pasteAgain);
    paste.dataset.paste = '';
    act.append(copy, paste);
    r.append(h('span', 'ht', e.time), ha, hx, act);
    return r;
  }

  private onRowClick(ev: Event): void {
    const b = (ev.target as Element).closest<HTMLElement>('[data-copy],[data-paste]');
    const id = b?.closest<HTMLElement>('.hr')?.dataset.id;
    if (!b || !id) return;
    const copyBtn = b.parentElement!.querySelector<HTMLElement>('[data-copy]')!;
    if (b.dataset.copy !== undefined) {
      api.historyCopy(id).then(() => flash(copyBtn), (e) => logFailed('copying', e));
    } else {
      // Pasted: the check on "Paste again"; nowhere to paste — the text is in the clipboard: the check on "Copy".
      api.historyPaste(id).then((to) => flash(to === 'pasted' ? b : copyBtn), (e) => logFailed('pasting again', e));
    }
  }

  private toggleSearch(): void {
    this.searchOpen = !this.searchOpen;
    this.bsr.classList.toggle('open', this.searchOpen);
    if (this.searchOpen) {
      this.input.focus({ preventScroll: true });
    } else {
      this.input.value = '';
      this.typed();
    }
  }

  /** A search a moment after the last key; an empty box is the ordinary list at once. */
  private typed(): void {
    this.query = this.input.value;
    if (this.searchTimer) clearTimeout(this.searchTimer);
    this.searchTimer = null;
    if (!this.query.trim()) {
      this.searchSeq++;
      if (this.results !== null) {
        this.results = null;
        this.render();
      }
      return;
    }
    this.searchTimer = setTimeout(() => {
      this.searchTimer = null;
      this.search();
    }, SEARCH_PAUSE_MS);
  }

  private search(): void {
    const seq = ++this.searchSeq;
    const q = this.query.trim();
    if (!q) return;
    api.historySearch(q).then(
      (days) => {
        if (seq !== this.searchSeq) return;
        this.results = days.filter((d) => DATE.test(d.date));
        this.render();
      },
      (e) => logFailed('searching the history', e),
    );
  }

  /** "Show earlier": the days before the last one shown (before yesterday when today and yesterday are empty). */
  private async earlier(): Promise<void> {
    const last = this.days.length ? this.days[this.days.length - 1].date : null;
    const yesterday = daysBefore(this.ctx.today(), 1);
    const before = last && last < yesterday ? last : yesterday;
    let page: api.PageView;
    try {
      page = await api.historyPage(before);
    } catch (e) {
      logFailed('reading the history', e);
      return;
    }
    const shown = new Set(this.days.map((d) => d.date));
    this.merge(page.days);
    this.more = page.more;
    if (this.results !== null) {
      this.earlierBtn.hidden = true;
      return;
    }
    this.render();
    if (this.ctx.reduced()) return;
    for (const g of this.list.querySelectorAll<HTMLElement>('.hgrp')) {
      if (shown.has(g.dataset.day ?? '') || typeof g.animate !== 'function') continue;
      g.animate([{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], {
        duration: 400,
        easing: 'cubic-bezier(.22,1,.36,1)',
      });
    }
  }

  private keepMenu(): void {
    const t = this.ctx.t;
    const cur = this.ctx.settings?.historyKeep ?? null;
    this.ctx.menu(
      this.keepDd,
      KEEPS.map((k) => ({ value: k, label: t.keeps[k], on: k === cur })),
      (v) => void this.ctx.change({ kind: 'historyKeep', value: v as Keep }),
      t.keepNote,
    );
  }

  /** "Clear history" works on the second press within 3 s, as "Forget all" in the design. */
  private clearPressed(): void {
    const t = this.ctx.t;
    const l = this.clearLbl;
    if (!this.clearTimer) {
      this.clearTimer = setTimeout(() => {
        this.clearTimer = null;
        if (l.isConnected) CUE().setLabel(l, t.clear);
      }, CLEAR_ARMED_MS);
      CUE().setLabel(l, t.clearAgain);
      return;
    }
    clearTimeout(this.clearTimer);
    this.clearTimer = null;
    CUE().setLabel(l, t.clear);
    void this.clearAll();
  }

  private async clearAll(): Promise<void> {
    try {
      await api.historyClear();
    } catch (e) {
      logFailed('clearing the history', e);
      return;
    }
    this.days = [];
    this.more = false;
    this.ids.clear();
    if (this.results !== null) this.results = [];
    this.render();
  }
}
