// The History section (spec 3 §2): results by day — today and yesterday, earlier days on request — with search, copy
// and paste again; how long it is kept, and clearing it, are on the settings tray's History page. New entries come over
// the window's channel and appear at the top of their day; one already shown (the channel and the first page can both
// bring it) is not shown twice.
// A run of entries from one app, each a few minutes after the one before, is one stack: its newest entry on top, a chip
// "Telegram 5" and two sheet edges under it; the chip and the edges open it in place. A click on an entry (not on its
// buttons), or Enter or Space on it, opens it: its whole text, how it was said and its recording (said.ts). Every entry
// opens, so each can be checked: one kept before CUE kept how it was said says so, and one CUE left as it was heard
// says that. The search grows out of the
// magnifier on the "History" line; what it finds is never stacked, and an entry it found only in how it was said opens
// by itself, the place marked. Stacks and entries opened stay open when the list is drawn again. Every recording
// pauses when CUE starts recording: what plays aloud would go into it.
// Text goes in as text: an entry may hold anything a model wrote.
import * as api from './api';
import type { DayView, EntryView } from './api';
import { icon, type IconName } from './icons';
import { flash, h, iconButton, logFailed, type Ctx } from './page';
import { Player, appendDiff, appendHeard, findHit } from './said';
import { dayLabel, daysBefore } from './text';

/** A search runs this long after the last key. */
export const SEARCH_PAUSE_MS = 150;
/** Entries of one app make a stack from this many on, */
export const STACK_MIN = 3;
/** each at most this long after the one before it (by the time in their ids). */
export const STACK_GAP_MS = 10 * 60 * 1000;
/** How long an entry takes to close (window.css: .hdet, .htx). */
const SHUT_MS = 460;
/** A new entry opened before its recording was encoded asks for it once more this much later. */
export const AUDIO_AGAIN_MS = 2000;

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const ID_TIME = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{3})/;
/** The badge of a mode; a plain dictation (most entries) has none, and neither has a mode the window does not know. */
const MODE_ICON: Partial<Record<api.Mode, IconName>> = { translation: 'tr', ask: 'ask' };

/** The local date an entry came: its id starts with its time. */
function entryDate(e: EntryView, today: string): string {
  const d = e.id.slice(0, 10);
  return DATE.test(d) ? d : today;
}

/** An entry's local time from its id, in ms on a clock of its own (only the gaps between entries count); null — its id
 *  has no time. */
function idTime(id: string): number | null {
  const m = ID_TIME.exec(id);
  if (!m) return null;
  const [y, mo, d, hh, mi, ss, ms] = m.slice(1).map(Number);
  return Date.UTC(y, mo - 1, d, hh, mi, ss, ms);
}

/** A day's entries (newest first) as they are drawn: a run from one app, each entry at most STACK_GAP_MS after the one
 *  before it, is a stack from STACK_MIN entries on; any other entry stands alone, and so does one whose app is not
 *  known. */
export function stackRuns(entries: readonly EntryView[]): EntryView[][] {
  const out: EntryView[][] = [];
  let run: EntryView[] = [];
  const flush = () => {
    if (run.length >= STACK_MIN) out.push(run);
    else for (const e of run) out.push([e]);
    run = [];
  };
  for (const e of entries) {
    const prev = run[run.length - 1];
    const t = idTime(e.id);
    const pt = prev ? idTime(prev.id) : null;
    if (!prev || !e.app || e.app !== prev.app || t === null || pt === null || pt < t || pt - t > STACK_GAP_MS) flush();
    run.push(e);
  }
  flush();
  return out;
}

/** Newest first: ids start with the entry's time. */
const newestFirst = (a: EntryView, b: EntryView) => (a.id < b.id ? 1 : a.id > b.id ? -1 : 0);

/** Text as the search compares it (as the core does): lowercase, with ё read as е. Both are one UTF-16 unit, so a
 *  place in the folded text is the same place in the text. */
const fold = (s: string) => s.toLowerCase().replace(/ё/g, 'е');

/** `text` into `parent`, the first place that holds `query` (in any case, ё and е alike) marked. */
function appendMarked(parent: HTMLElement, text: string, query: string): void {
  const q = fold(query);
  const low = fold(text);
  const i = q && low.length === text.length ? low.indexOf(q) : -1;
  if (i < 0) {
    parent.append(text);
    return;
  }
  parent.append(text.slice(0, i), h('mark', '', text.slice(i, i + q.length)), text.slice(i + q.length));
}

/** Whether a search for `q` found `e` only in how it was said: its row shows nothing that matches. */
function foundOnlyAsSaid(e: EntryView, q: string): boolean {
  const shows = findHit(e.text, q) !== null || (!!e.question && findHit(e.question, q) !== null);
  return !!e.raw && !shows && findHit(e.raw, q) !== null;
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
  /** The local date the day headings were drawn for ("Today", "Yesterday"). */
  private drawnFor = '';
  /** Stacks opened, by their oldest entry (a new entry on top keeps a stack open). */
  private readonly openStacks = new Set<string>();
  /** Entries opened to show how they were said. */
  private readonly opened = new Set<string>();
  /** Those of them the search opened (found only in how they were said): they close with it. */
  private readonly found = new Set<string>();
  /** Entries that came over the channel: their recording may still be on its way, so it is asked for anyway. */
  private readonly live = new Set<string>();
  /** The recordings of the entries opened, by entry; one plays at a time. */
  private readonly players = new Map<string, Player>();
  /** Rows closing: their text stays whole until it has shrunk to one line. */
  private readonly shutting = new Map<HTMLElement, ReturnType<typeof setTimeout>>();
  private sec: HTMLElement | null = null;
  private list!: HTMLElement;
  private emptyEl!: HTMLElement;
  private earlierBtn!: HTMLButtonElement;
  private bsr!: HTMLElement;
  private find!: HTMLButtonElement;
  private input!: HTMLInputElement;

  constructor(private readonly ctx: Ctx) {}

  /** Draws the section into `sec` from what it holds (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    const t = this.ctx.t;
    this.release();
    this.sec = sec;
    sec.textContent = '';
    // The search lives on the heading's line: the hairline after "History" and the magnifier at its end. Opening, the
    // magnifier's own background stretches left over the line into the field (one eased motion; nothing under it
    // moves), and the magnifier turns into a cross that closes it.
    const h3 = h('h3', '', t.history);
    this.bsr = h('div', 'hsb');
    this.bsr.classList.toggle('open', this.searchOpen);
    const field = h('span', 'hsf');
    this.input = h('input');
    this.input.placeholder = t.searchHint;
    this.input.setAttribute('aria-label', t.searchHint);
    this.input.spellcheck = false;
    this.input.value = this.query;
    this.input.addEventListener('input', () => this.typed());
    field.append(this.input);
    this.find = iconButton('ibt sm hsbt', 'search', t.search);
    this.find.insertAdjacentHTML('beforeend', icon('x'));
    this.find.addEventListener('click', () => this.toggleSearch());
    this.labelFind();
    this.bsr.append(field, this.find);
    h3.append(this.bsr);

    this.list = h('div', 'bhist');
    this.list.addEventListener('click', (e) => this.onListClick(e));
    this.list.addEventListener('keydown', (e) => this.onListKey(e));
    this.emptyEl = h('p', 'hempty');

    const more = h('div', 'bmore');
    this.earlierBtn = h('button', '', t.earlier);
    this.earlierBtn.type = 'button';
    this.earlierBtn.addEventListener('click', () => void this.earlier());
    more.append(this.earlierBtn);

    sec.append(h3, this.list, this.emptyEl, more);
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

  /** A new entry from the core: on top of its day (today) — in its stack, or starting one — unless it is shown already
   *  or a search is shown. */
  add(e: EntryView): void {
    this.live.add(e.id);
    if (!this.loaded) {
      this.pending.push(e);
      return;
    }
    const today = this.ctx.today();
    const date = entryDate(e, today);
    if (!this.merge([{ date, entries: [e] }]) || this.results !== null || !this.sec) return;
    const group = this.list.querySelector<HTMLElement>(`.hgrp[data-day="${date}"]`);
    const day = this.days.find((d) => d.date === date);
    if (!group || !day || today !== this.drawnFor) {
      // A day of its own, or the date changed since the list was drawn (the window stayed open past midnight): every
      // heading is drawn again for today.
      this.render();
    } else {
      // The day is drawn again: the entry may join a stack, or make one.
      const focused = this.focusedRow();
      group.replaceWith(this.group(day));
      this.refocus(focused);
      this.releaseDropped();
      this.emptyEl.hidden = true;
    }
    this.rowOf(e.id)?.classList.add('new');
  }

  /** The day headings follow the date: the list is drawn again when the date changed since it was drawn. */
  followDate(): void {
    if (this.sec && this.ctx.today() !== this.drawnFor) this.render();
  }

  /** Esc: the search closes; false — it is not open. */
  esc(): boolean {
    if (!this.searchOpen) return false;
    this.toggleSearch();
    return true;
  }

  /** Every recording stops and gives back its object URL (the page is going, or drawn anew, or the recordings' period
   *  changed). */
  release(): void {
    for (const p of this.players.values()) p.release();
    this.players.clear();
  }

  /** CUE starts recording: every recording pauses, so none plays into it. */
  recordingStarts(): void {
    for (const p of this.players.values()) p.pause();
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
    this.drawnFor = this.ctx.today();
    const days = this.results ?? this.days;
    const q = this.results !== null ? this.query.trim() : '';
    const focused = this.focusedRow();
    this.list.textContent = '';
    for (const d of days) this.list.append(this.group(d, q));
    this.refocus(focused);
    this.releaseDropped();
    this.earlierBtn.hidden = this.results !== null || !this.more;
    this.emptyEl.hidden = days.length > 0 || (this.results === null && this.more);
    this.emptyEl.textContent = this.results !== null ? this.ctx.t.nothing : this.ctx.t.empty;
  }

  /** The entry whose row has the keyboard's focus, if one has. */
  private focusedRow(): string | undefined {
    const a = document.activeElement;
    return a instanceof HTMLElement && a.classList.contains('hr') && this.list.contains(a) ? a.dataset.id : undefined;
  }

  /** The list drawn again: the entry that had the focus has it again in its new row. */
  private refocus(id: string | undefined): void {
    if (id) this.rowOf(id)?.focus({ preventScroll: true });
  }

  /** Recordings whose entries are no longer drawn stop and give back their object URLs. */
  private releaseDropped(): void {
    for (const [id, p] of this.players) {
      if (this.list.contains(p.el)) continue;
      p.release();
      this.players.delete(id);
    }
  }

  /** A day: what a search found as it is; the ordinary list with its stacks. */
  private group(d: DayView, q = ''): HTMLElement {
    const g = h('div', 'hgrp');
    g.dataset.day = d.date;
    g.append(h('div', 'hday', dayLabel(d.date, this.ctx.today(), this.ctx.t, this.ctx.lang)));
    const hl = h('div', 'hl');
    if (this.results !== null) {
      for (const e of d.entries) hl.append(this.row(e, q));
    } else {
      for (const run of stackRuns(d.entries)) hl.append(run.length > 1 ? this.stack(run) : this.row(run[0], ''));
    }
    g.append(hl);
    return g;
  }

  /** The newest entry on top, the chip with the app and the count, two sheet edges under it; the rest hidden until it
   *  opens. A stack that holds an opened entry is drawn open. */
  private stack(run: EntryView[]): HTMLElement {
    const key = run[run.length - 1].id;
    const rest = run.slice(1);
    if (rest.some((e) => this.opened.has(e.id))) this.openStacks.add(key);
    const open = this.openStacks.has(key);
    const s = h('div', 'hstk');
    s.dataset.stack = key;
    s.classList.toggle('open', open);
    const top = this.row(run[0], '', run.length);
    top.classList.add('htop');
    top.querySelector('.hchip')?.setAttribute('aria-expanded', String(open));
    // The sheets open it too (by mouse; the chip is the button for the keyboard and screen readers).
    const edge = h('span', 'hedge');
    edge.setAttribute('aria-hidden', 'true');
    edge.append(h('i'), h('i'));
    const hidden = h('div', 'hrest');
    const inner = h('div');
    for (const e of rest) inner.append(this.row(e, ''));
    hidden.append(inner);
    s.append(top, edge, hidden);
    return s;
  }

  /** time · mode · [app] [question] text, in one line (a dictation's mode is an empty place, so the texts line up);
   *  the whole of it in the tooltip; copy and paste again on hover.
   *  `count` — the top of a stack of so many: its app is the stack's chip. Every entry opens. */
  private row(e: EntryView, q: string, count = 0): HTMLElement {
    const t = this.ctx.t;
    const r = h('div', 'hr');
    r.dataset.id = e.id;
    const ha = h('span', 'ha');
    const mi = Object.hasOwn(MODE_ICON, e.mode) ? MODE_ICON[e.mode] : undefined;
    if (mi) {
      const badge = h('span', e.mode === 'ask' ? 'aski' : 'hmi');
      badge.innerHTML = icon(mi);
      badge.title = t.modes[e.mode];
      ha.append(badge);
    }
    const hx = h('div', 'hx');
    const htx = h('div', 'htx');
    if (e.app && count) {
      const chip = h('button', 'hchip');
      chip.type = 'button';
      const label = t.stack.replace('{N}', String(count)).replace('{APP}', () => e.app);
      chip.title = label;
      chip.setAttribute('aria-label', label);
      chip.append(h('span', 'happ', e.app), h('span', 'hcnt', String(count)));
      htx.append(chip);
    } else if (e.app) {
      htx.append(h('span', 'happ', e.app));
    }
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
    // The keyboard and screen readers reach it too: Enter or Space opens it (`onListKey`).
    const open = this.opened.has(e.id);
    r.classList.add('hmore');
    r.tabIndex = 0;
    r.setAttribute('aria-expanded', String(open));
    if (open) {
      r.classList.add('open');
      hx.append(this.details(e, q));
    }
    return r;
  }

  /** How it was said — a dictation against its text, a translation's speech, the question — and the recording. Not
   *  kept (an entry from before CUE kept it): the heading says so; a dictation CUE left as it was heard says that. */
  private details(e: EntryView, q: string): HTMLElement {
    const t = this.ctx.t;
    const det = h('div', 'hdet');
    const inner = h('div');
    const heard = e.mode === 'ask' ? e.question : e.raw;
    const lab = h('div', 'hlab', t.saidAs[e.mode] ?? '');
    inner.append(lab);
    if (heard) {
      const box = h('div', 'hraw');
      const hit = q ? findHit(heard, q) : null;
      if (e.mode === 'dictation') appendDiff(box, heard, e.text, hit);
      else appendHeard(box, heard, hit);
      if (e.mode === 'dictation' && heard.trim() === e.text.trim()) lab.append(h('span', 'hnote', t.unchanged));
      inner.append(box);
    } else {
      lab.append(h('span', 'hnote', t.notKept));
    }
    inner.append(this.player(e).el);
    det.append(inner);
    return det;
  }

  /** The entry's player: the one it has (it keeps playing when the list is drawn again), or a new one that asks the
   *  core for the recording — shown at once when the entry has one, else once it comes; none to play — no player. A new
   *  entry's recording may still be encoded (a few seconds at most): it is asked for once more `AUDIO_AGAIN_MS` later. */
  private player(e: EntryView): Player {
    const have = this.players.get(e.id);
    if (have) return have;
    const p = new Player(this.ctx.t, (starting) => {
      for (const other of this.players.values()) if (other !== starting) other.pause();
    });
    p.el.hidden = !e.hasAudio;
    this.players.set(e.id, p);
    const mine = () => this.players.get(e.id) === p;
    const gone = () => {
      if (!mine()) return;
      this.players.delete(e.id);
      p.release();
      p.el.remove();
    };
    const ask = (first: boolean) =>
      api.historyAudio(e.id).then(
        (bytes) => {
          if (!mine()) return;
          if (bytes && bytes.byteLength) {
            p.load(bytes);
            p.el.hidden = false;
          } else if (first && this.live.has(e.id)) {
            setTimeout(() => void (mine() && ask(false)), AUDIO_AGAIN_MS);
          } else {
            gone();
          }
        },
        (err) => {
          logFailed('reading a recording', err);
          gone();
        },
      );
    void ask(true);
    return p;
  }

  private rowOf(id: string): HTMLElement | undefined {
    return [...this.list.querySelectorAll<HTMLElement>('.hr')].find((r) => r.dataset.id === id);
  }

  private entryOf(id: string): EntryView | undefined {
    for (const d of this.results ?? this.days) {
      const e = d.entries.find((x) => x.id === id);
      if (e) return e;
    }
    return undefined;
  }

  private onListClick(ev: Event): void {
    const t = ev.target as Element;
    const b = t.closest<HTMLElement>('[data-copy],[data-paste]');
    if (b) {
      this.copyOrPaste(b);
      return;
    }
    const s = t.closest<HTMLElement>('.hchip, .hedge')?.closest<HTMLElement>('.hstk');
    if (s) {
      this.toggleStack(s);
      return;
    }
    // Inside how it was said (its text may be selected, its player pressed) or on a button: not the row's.
    if (t.closest('.hdet, button')) return;
    const r = t.closest<HTMLElement>('.hr.hmore');
    // A drag that selected the entry's text is not a click on it.
    if (!r || (window.getSelection?.()?.toString() ?? '') !== '') return;
    this.toggleEntry(r);
  }

  /** Enter or Space on an entry that opens (on the row itself: its buttons and its player take the keys as buttons). */
  private onListKey(ev: KeyboardEvent): void {
    if (ev.key !== 'Enter' && ev.key !== ' ') return;
    const r = ev.target;
    if (!(r instanceof HTMLElement) || !r.matches('.hr.hmore')) return;
    ev.preventDefault();
    this.toggleEntry(r);
  }

  private copyOrPaste(b: HTMLElement): void {
    const id = b.closest<HTMLElement>('.hr')?.dataset.id;
    if (!id) return;
    const copyBtn = b.parentElement!.querySelector<HTMLElement>('[data-copy]')!;
    if (b.dataset.copy !== undefined) {
      api.historyCopy(id).then(() => flash(copyBtn), (e) => logFailed('copying', e));
    } else {
      // Pasted: the check on "Paste again"; nowhere to paste — the text is in the clipboard: the check on "Copy".
      api.historyPaste(id).then((to) => flash(to === 'pasted' ? b : copyBtn), (e) => logFailed('pasting again', e));
    }
  }

  /** A stack opens or closes in place; closing it closes the entries opened in it. */
  private toggleStack(s: HTMLElement): void {
    const key = s.dataset.stack ?? '';
    const on = !this.openStacks.has(key);
    if (on) {
      this.openStacks.add(key);
    } else {
      this.openStacks.delete(key);
      for (const r of s.querySelectorAll<HTMLElement>('.hrest .hr.open')) this.closeEntry(r);
    }
    s.classList.toggle('open', on);
    s.querySelector('.hchip')?.setAttribute('aria-expanded', String(on));
  }

  /** An entry opens to how it was said (its height grows from 0), or closes again. */
  private toggleEntry(r: HTMLElement): void {
    const id = r.dataset.id ?? '';
    if (this.opened.has(id)) {
      this.closeEntry(r);
      return;
    }
    const e = this.entryOf(id);
    if (!e) return;
    this.opened.add(id);
    r.querySelector('.hdet')?.remove();
    const q = this.results !== null ? this.query.trim() : '';
    const det = this.details(e, q);
    r.querySelector('.hx')!.append(det);
    // Drawn closed first, so it grows.
    void det.offsetHeight;
    this.unshut(r);
    r.classList.add('open');
    r.setAttribute('aria-expanded', 'true');
  }

  /** Closed: its recording stops and gives back its object URL; how it was said folds away. */
  private closeEntry(r: HTMLElement): void {
    const id = r.dataset.id ?? '';
    this.opened.delete(id);
    this.found.delete(id);
    this.unshut(r);
    r.classList.add('shut');
    r.classList.remove('open');
    r.setAttribute('aria-expanded', 'false');
    this.shutting.set(r, setTimeout(() => this.unshut(r), this.ctx.reduced() ? 0 : SHUT_MS));
    const p = this.players.get(id);
    if (p) {
      this.players.delete(id);
      p.release();
    }
  }

  /** The row's text is one line again (it has closed, or opens anew). */
  private unshut(r: HTMLElement): void {
    const timer = this.shutting.get(r);
    if (timer === undefined) return;
    clearTimeout(timer);
    this.shutting.delete(r);
    r.classList.remove('shut');
  }

  private labelFind(): void {
    const label = this.searchOpen ? this.ctx.t.searchClose : this.ctx.t.search;
    this.find.title = label;
    this.find.setAttribute('aria-label', label);
    this.find.setAttribute('aria-expanded', String(this.searchOpen));
  }

  private toggleSearch(): void {
    this.searchOpen = !this.searchOpen;
    this.bsr.classList.toggle('open', this.searchOpen);
    this.labelFind();
    if (this.searchOpen) {
      this.input.focus({ preventScroll: true });
    } else {
      if (document.activeElement === this.input) this.find.focus({ preventScroll: true });
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
        this.closeFound();
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
        this.openFound(q);
        this.render();
      },
      (e) => logFailed('searching the history', e),
    );
  }

  /** What the search found only in how it was said opens by itself (its place is marked there); what the last search
   *  opened so closes first. */
  private openFound(q: string): void {
    this.closeFound();
    for (const d of this.results ?? []) {
      for (const e of d.entries) {
        if (this.opened.has(e.id) || !foundOnlyAsSaid(e, q)) continue;
        this.opened.add(e.id);
        this.found.add(e.id);
      }
    }
  }

  /** The entries the search opened close again. */
  private closeFound(): void {
    for (const id of this.found) this.opened.delete(id);
    this.found.clear();
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

  /** Clear history (the tray's History page asks twice): the core deletes every day, and the list is empty. */
  async clear(): Promise<void> {
    try {
      await api.historyClear();
    } catch (e) {
      logFailed('clearing the history', e);
      return;
    }
    this.days = [];
    this.more = false;
    this.ids.clear();
    this.opened.clear();
    this.found.clear();
    this.openStacks.clear();
    this.live.clear();
    if (this.results !== null) this.results = [];
    this.render();
  }
}
