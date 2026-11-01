// Today, the main window's first section (design stage 5): today's words and the minutes they saved in one sentence,
// the last 30 days as bars (a tooltip names the day and its words), then words a minute, days in a row and words in
// all. When the total passes a round number the window has not marked yet, a pill says so and lines burst from the
// number once; the pill stays for the rest of that day. The numbers come from the core (`stats_get`): on start, a
// moment after each new history entry (its usage row is written just after it), and when the date changes.
import * as api from './api';
import type { Stats } from './api';
import { icon } from './icons';
import { h, logFailed, type Ctx } from './page';
import { formatNumber, plural, type Forms } from './text';

/** A new entry's usage row is written right after it: the numbers are asked for this much later. */
export const REFRESH_MS = 1000;
/** The highest bar, px; a day with words is at least 2 px high. */
const BARS_PX = 42;
/** Words a page of a book holds, for the milestone. */
const PAGE_WORDS = 250;
/** Where the window keeps the milestone it marked and the day it did: `{"n":100000,"date":"2026-11-01"}`. */
export const MILESTONE_KEY = 'cue.milestone';

const ZERO: Stats = { todayWords: 0, savedMinutes: 0, days: [], wpm: null, streak: 0, total: 0, milestone: null };

/** The milestone the window marked, or null (none, or storage unavailable). */
function marked(): { n: number; date: string } | null {
  try {
    const v = JSON.parse(localStorage.getItem(MILESTONE_KEY) ?? 'null');
    return v && typeof v.n === 'number' && typeof v.date === 'string' ? v : null;
  } catch {
    return null;
  }
}

function mark(n: number, date: string): void {
  try {
    localStorage.setItem(MILESTONE_KEY, JSON.stringify({ n, date }));
  } catch {
    // Storage off: the pill shows again next time, which is harmless.
  }
}

export class TodayView {
  private stats: Stats = ZERO;
  private sec: HTMLElement | null = null;
  private tip: HTMLElement | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private seq = 0;

  constructor(private readonly ctx: Ctx) {}

  /** Draws the section into `sec` from what it holds (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    this.sec = sec;
    this.draw(false);
  }

  /** The numbers from the core, drawn again. */
  async load(): Promise<void> {
    const seq = ++this.seq;
    let s: Stats;
    try {
      s = await api.statsGet();
    } catch (e) {
      logFailed('reading the numbers', e);
      return;
    }
    if (seq !== this.seq) return;
    this.stats = s;
    this.draw(true);
  }

  /** A new entry came: the numbers again a moment later (several entries close together ask once). */
  refreshSoon(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.load();
    }, REFRESH_MS);
  }

  dispose(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  /** `celebrate`: a milestone not marked yet bursts now (not on a redraw for the language). */
  private draw(celebrate: boolean): void {
    const sec = this.sec;
    if (!sec) return;
    const { t, lang } = this.ctx;
    const s = this.stats;
    sec.textContent = '';

    const lede = h('p', 'lede');
    const words = this.phrase(t.todayWords, s.todayWords, t.words);
    words.querySelector('.num')?.setAttribute('data-n', 'today');
    lede.append(words, h('br'), this.phrase(t.todaySaved, s.savedMinutes, t.minutes));

    const bars = h('div', 'm30');
    const max = Math.max(1, ...s.days.map((d) => d.words));
    const today = this.ctx.today();
    for (const d of s.days) {
      const bar = h('i');
      if (d.date === today) bar.classList.add('now');
      bar.style.height = `${d.words ? Math.max(2, Math.round((d.words / max) * BARS_PX)) : 2}px`;
      bar.dataset.date = d.date;
      bar.dataset.words = String(d.words);
      bars.append(bar);
    }
    bars.addEventListener('pointerover', (e) => this.showTip(e.target));
    bars.addEventListener('pointerleave', () => this.hideTip());

    const line = h('div', 'bstat');
    const parts: HTMLElement[] = [];
    if (s.wpm) parts.push(this.phrase(t.todayWpm, s.wpm, t.words));
    if (s.streak) parts.push(this.phrase(t.todayStreak, s.streak, t.days));
    if (s.total) {
      const total = this.phrase(t.todayTotal, s.total, t.words);
      total.querySelector('.num')?.setAttribute('data-n', 'total');
      parts.push(total);
    }
    parts.forEach((p, i) => {
      if (i) line.append(h('span', 'bdot', '·'));
      line.append(p);
    });
    line.hidden = parts.length === 0;

    sec.append(lede, bars, line);

    const m = s.milestone;
    const seen = marked();
    const fresh = m !== null && (!seen || seen.n < m);
    if (fresh && celebrate) mark(m, today);
    if (m !== null && (fresh || (seen?.n === m && seen.date === today))) {
      const pill = h('span', 'ms');
      pill.innerHTML = icon('ask');
      const pages = Math.round(m / PAGE_WORDS);
      const text = t.milestone
        .replace('{N}', '\u0000')
        .replace('{W}', plural(m, t.words))
        .replace('{P}', formatNumber(pages, lang))
        .replace('{PW}', plural(pages, t.pages));
      const [before, after] = text.split('\u0000');
      pill.append(before, h('b', '', formatNumber(m, lang)), after);
      const row = h('div', 'msr');
      row.append(pill);
      sec.append(row);
      if (fresh && celebrate) this.burst(sec.querySelector<HTMLElement>('[data-n=total]'));
    }
  }

  /** A template with {N} as a Bebas number and {W} as the word's form for it. */
  private phrase(template: string, n: number, forms: Forms): HTMLElement {
    const out = h('span');
    const [before, after = ''] = template.replace('{W}', plural(n, forms)).split('{N}');
    out.append(before, h('span', 'num', formatNumber(n, this.ctx.lang)), after);
    return out;
  }

  /** The day under the pointer: its words and its date, over its bar. */
  private showTip(target: EventTarget | null): void {
    if (!(target instanceof HTMLElement) || !target.dataset.date) return;
    const app = this.ctx.app;
    if (!this.tip || !app.contains(this.tip)) {
      this.tip = h('div', 'tip');
      app.append(this.tip);
    }
    const [y, mo, d] = target.dataset.date.split('-').map(Number);
    const date = new Intl.DateTimeFormat(this.ctx.lang === 'ru' ? 'ru-RU' : 'en-US', {
      weekday: 'short',
      day: 'numeric',
      month: 'long',
    }).format(new Date(y, mo - 1, d));
    const words = Number(target.dataset.words);
    this.tip.textContent = '';
    this.tip.append(h('b', '', formatNumber(words, this.ctx.lang)), date);
    const a = app.getBoundingClientRect();
    const b = target.getBoundingClientRect();
    this.tip.style.left = `${b.left - a.left + b.width / 2}px`;
    this.tip.style.top = `${b.top - a.top}px`;
    this.tip.classList.add('on');
  }

  private hideTip(): void {
    this.tip?.classList.remove('on');
  }

  /** Lines fly out of the number and fade, once; with reduced motion only the number pops. */
  private burst(target: HTMLElement | null): void {
    if (!target) return;
    target.classList.remove('pop');
    void target.offsetWidth;
    target.classList.add('pop');
    const app = this.ctx.app;
    if (this.ctx.reduced() || typeof app.animate !== 'function') return;
    const a = app.getBoundingClientRect();
    const b = target.getBoundingClientRect();
    const cx = b.left - a.left + b.width / 2;
    const cy = b.top - a.top + b.height / 2;
    const colors = ['var(--rec)', 'var(--ask)', 'var(--ok)', 'var(--warn)'];
    for (let i = 0; i < 18; i++) {
      const angle = (i / 18) * Math.PI * 2;
      const dist = 34 + (i % 5) * 9;
      const line = i % 3 !== 0;
      const p = h('i', line ? 'bp' : 'bp dot');
      p.style.left = `${cx}px`;
      p.style.top = `${cy}px`;
      p.style.background = line ? 'var(--text)' : colors[i % 4];
      app.append(p);
      p.animate(
        [
          { transform: `translate(-50%,-50%) rotate(${angle}rad) translateY(-6px) scaleY(.3)`, opacity: 1 },
          { transform: `translate(-50%,-50%) rotate(${angle}rad) translateY(${-dist}px) scaleY(1)`, opacity: 1, offset: 0.55 },
          { transform: `translate(-50%,-50%) rotate(${angle}rad) translateY(${-dist - 10}px) scaleY(.2)`, opacity: 0 },
        ],
        { duration: 900 + (i % 4) * 90, easing: 'cubic-bezier(.2,.8,.3,1)' },
      ).onfinish = () => p.remove();
    }
  }
}
