// The main window's page (spec 3 §1–3): the header — the dictation mark and CUE on the left, links to the sections and the
// gear on the right — the Today and History sections, and the settings tray that grows out of the gear. A link scrolls
// to its section; the link of the section under the header is the darker one. Ported from the approved design (stage 5).
// The page draws nothing until the core has told it the settings: the language is SettingsView.lang (the window has
// no language set before its script runs). It follows new history entries over its own channel, opened before the
// first page is asked for.
// User text — history text, app and device names — goes into the page as text (textContent, title), never as HTML.
// The helpers and the menu here serve the welcome window's page too.
import '../design/cue.css';
import '../design/window.css';
import '../design/cue.js';
import '../design/drop.js';
import * as api from './api';
import type { SettingChange, SettingsView } from './api';
import { HistoryView } from './history';
import { icon } from './icons';
import { SettingsTray } from './settings';
import { TodayView } from './today';
import { DictionaryView } from './dictionary';
import { TEXTS, isoDate, pickLang, type Lang, type WinText } from './text';

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';
const DARK = '(prefers-color-scheme: dark)';
/** How long a button shows its "done" check (as in the design). */
const FLASH_MS = 1400;
/** The dictation mark before the word CUE: the Drop's three voice bars, in its proportions, that roll as a slow wave,
 *  and the red lamp pulsing at their top right (window.css moves both, and stills them with reduced motion). The
 *  window's 20 grid, stroke 1.8. */
const MARK =
  '<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true">' +
  '<g class="wv"><line x1="3.2" y1="5" x2="3.2" y2="15"/><line x1="8.2" y1="3" x2="8.2" y2="17"/>' +
  '<line x1="13.2" y1="4.4" x2="13.2" y2="15.6"/></g>' +
  '<circle class="lamp" cx="18.2" cy="3.2" r="1.7"/></svg>';

/** One choice of a menu: its value, its label (text), a muted note after it, whether it is the current one; and for a
 *  richer list a second line under the label (`sub`), a quiet chip after it (`badge`), a lamp before it (`lamp`: on or
 *  off), and the group it is listed under (a heading wherever the group changes). */
export interface MenuItem {
  value: string;
  label: string;
  note?: string;
  on: boolean;
  sub?: string;
  badge?: string;
  lamp?: boolean;
  group?: string;
}

/** Which edge of its button a menu lines up with. */
export type MenuAlign = 'start' | 'end';

/** What the History section and the tray take from the page. */
export interface Ctx {
  readonly app: HTMLElement;
  readonly t: WinText;
  readonly lang: Lang;
  /** null — the core did not answer at startup. */
  readonly settings: SettingsView | null;
  reduced(): boolean;
  /** The local date now, `YYYY-MM-DD`. */
  today(): string;
  /** A menu under `button` (a second press on the same button closes it); `pick` gets the value chosen. `align`: the
   *  menu's right edge on the button's (`end`, by default) or its left edge (`start`, for a button that starts a row);
   *  `wide` — the menu as wide as the button. */
  menu(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string, align?: MenuAlign, wide?: boolean): void;
  closeMenu(): void;
  /** A setting through the core; the page takes the settings it answers with (null — refused or no answer). */
  change(c: SettingChange): Promise<SettingsView | null>;
}

export interface PageOptions {
  /** Reduced motion; by default the system setting. */
  reduced?: () => boolean;
  /** The time now; by default the clock. */
  now?: () => Date;
}

/** A failed command's code for the log: a short code from the core, anything else as "failed" — a call's data never
 *  reaches the log. */
export function codeOf(e: unknown): string {
  return typeof e === 'string' && /^[A-Za-z]{1,32}$/.test(e) ? e : 'failed';
}

export function logFailed(what: string, e: unknown): void {
  console.error(`CUE: ${what} failed:`, codeOf(e));
}

/** An element with a class and, optionally, its text. */
export function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls = '', text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** A box that only lays things out: a screen reader passes over it to what it holds. */
export function wrap(cls = ''): HTMLDivElement {
  const d = h('div', cls);
  d.setAttribute('role', 'none');
  return d;
}

/** A small icon button with its tooltip and accessible name. */
export function iconButton(cls: string, name: Parameters<typeof icon>[0], label: string): HTMLButtonElement {
  const b = h('button', cls);
  b.type = 'button';
  b.innerHTML = icon(name);
  b.title = label;
  b.setAttribute('aria-label', label);
  return b;
}

/** A settings row: its label and an optional line under it on the left, its controls on the right. */
export function srow(label: string, desc: string | null, ...ctrls: HTMLElement[]): { row: HTMLElement; small: HTMLElement } {
  const row = h('div', 'srow');
  const stx = h('div', 'stx');
  const small = h('small', '', desc ?? '');
  small.hidden = !desc;
  stx.append(h('b', '', label), small);
  const sc = h('div', 'sc');
  sc.append(...ctrls);
  row.append(stx, sc);
  return { row, small };
}

export function dropDown(kind: string, value: string): HTMLButtonElement {
  const b = h('button', 'dd');
  b.type = 'button';
  b.dataset.dd = kind;
  b.append(h('span', 'ddv', value));
  b.insertAdjacentHTML('beforeend', icon('chev'));
  return b;
}

/** The button shows a check for a moment (the design's flash). */
export function flash(b: HTMLElement): void {
  if (b.dataset.flash) return;
  const html = b.innerHTML;
  b.dataset.flash = '1';
  b.innerHTML = icon('ok');
  b.classList.add('done');
  setTimeout(() => {
    b.innerHTML = html;
    b.classList.remove('done');
    delete b.dataset.flash;
  }, FLASH_MS);
}

/** The rectangle of `el` in the window's own coordinates. */
export function relRect(app: HTMLElement, el: Element): { x: number; y: number; w: number; h: number } {
  const ar = app.getBoundingClientRect();
  const r = el.getBoundingClientRect();
  return { x: r.left - ar.left, y: r.top - ar.top, w: r.width, h: r.height };
}

/** The window's one open menu: under its button, inside the window's box `app`. */
export class Menus {
  private open: { m: HTMLElement; b: HTMLElement } | null = null;

  constructor(private readonly app: HTMLElement) {}

  get isOpen(): boolean {
    return this.open !== null;
  }

  /** A menu under `button` (a second press on the same button closes it); `pick` gets the value chosen. */
  show(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string, align: MenuAlign = 'end', wide = false): void {
    if (this.open?.b === button) {
      this.close();
      return;
    }
    this.close();
    if (!items.length) return;
    const m = h('div', 'menu');
    m.setAttribute('role', 'menu');
    let current: HTMLElement | null = null;
    let group: string | undefined;
    items.forEach((it, i) => {
      if (it.group !== undefined && it.group !== group) m.append(h('div', 'mg', it.group));
      group = it.group;
      const b = h('button');
      b.type = 'button';
      b.dataset.i = String(i);
      b.setAttribute('role', 'menuitemradio');
      b.setAttribute('aria-checked', String(it.on));
      if (it.lamp !== undefined) b.append(h('span', it.lamp ? 'mlamp on' : 'mlamp'));
      if (it.sub === undefined) b.append(h('span', 'mlb', it.label));
      else {
        const two = h('span', 'mt');
        two.append(h('span', 'mlb', it.label), h('span', 'msub', it.sub));
        b.append(two);
        m.classList.add('two');
      }
      if (it.badge) b.append(h('span', 'mbadge', it.badge));
      if (it.note) b.append(h('span', 'mnc', it.note));
      if (it.on) {
        b.insertAdjacentHTML('beforeend', icon('ok'));
        current = b;
      }
      m.append(b);
    });
    if (note) m.append(h('div', 'mnote', note));
    m.addEventListener('click', (e) => {
      const b = (e.target as Element).closest<HTMLElement>('[data-i]');
      if (!b) return;
      const it = items[Number(b.dataset.i)];
      this.close();
      if (it) pick(it.value);
    });
    if (wide) {
      m.classList.add('wide');
      m.style.width = `${button.offsetWidth}px`;
    }
    this.app.append(m);
    this.open = { m, b: button };
    this.place(m, button, current, align);
  }

  close(): void {
    this.open?.m.remove();
    this.open = null;
  }

  /** Under the button, its right edge on the button's (or, `start`, its left edge, the items' text over the button's);
   *  above it when there is no room below; scrolled to the current choice when it is taller than the room. */
  private place(m: HTMLElement, b: HTMLElement, current: HTMLElement | null, align: MenuAlign): void {
    const p = relRect(this.app, b);
    const H = this.app.offsetHeight;
    const below = H - (p.y + p.h + 4) - 8;
    const above = p.y - 4 - 8;
    const tall = m.offsetHeight;
    const down = tall <= below || below >= above;
    m.style.maxHeight = `${Math.max(120, down ? below : above)}px`;
    const x = align === 'start' ? p.x - 4 : p.x + p.w - m.offsetWidth;
    m.style.left = `${Math.max(8, Math.min(x, this.app.offsetWidth - m.offsetWidth - 8))}px`;
    if (down) {
      m.style.top = `${p.y + p.h + 4}px`;
    } else {
      m.style.top = `${p.y - Math.min(tall, above) - 4}px`;
      m.style.transformOrigin = '50% 100%';
    }
    if (current && m.scrollHeight > m.clientHeight) {
      m.scrollTop = Math.max(0, current.offsetTop - (m.clientHeight - current.offsetHeight) / 2);
    }
  }
}

export class WindowPage implements Ctx {
  t: WinText = TEXTS.en;
  lang: Lang = 'en';
  settings: SettingsView | null = null;
  readonly history: HistoryView;
  readonly numbers: TodayView;
  readonly dict: DictionaryView;
  readonly tray: SettingsTray;
  readonly reduced: () => boolean;
  private readonly now: () => Date;
  private readonly menus: Menus;
  private readonly off = new AbortController();

  constructor(readonly app: HTMLElement, opts: PageOptions = {}) {
    this.reduced = opts.reduced ?? (() => !!window.matchMedia?.(REDUCED_MOTION)?.matches);
    this.now = opts.now ?? (() => new Date());
    app.classList.add('app');
    app.setAttribute('role', 'none');
    this.menus = new Menus(app);
    this.history = new HistoryView(this);
    this.numbers = new TodayView(this);
    this.dict = new DictionaryView(this);
    this.tray = new SettingsTray(this, () => this.history.clear());
    const signal = this.off.signal;
    const dark = window.matchMedia?.(DARK);
    const theme = () => (document.documentElement.dataset.theme = dark?.matches ? 'dark' : 'light');
    theme();
    dark?.addEventListener?.('change', theme, { signal });
    document.addEventListener('keydown', (e) => this.onKey(e), { signal });
    // Outside a menu or the tray: by the event's path as it was dispatched — a click inside may already have replaced
    // the element it hit (a tray page, a menu that closed).
    const within = (e: Event, sel: string) => e.composedPath().some((n) => n instanceof Element && n.matches(sel));
    document.addEventListener('pointerdown', (e) => {
      if (this.menus.isOpen && !within(e, '.menu, [data-dd]')) this.closeMenu();
    }, { signal });
    document.addEventListener('click', (e) => {
      if (this.tray.isOpen && !within(e, '.btray, .menu, .gear')) this.tray.close();
    }, { signal });
    window.addEventListener('resize', () => this.tray.place(), { signal });
    // Back to the window, maybe on another day: "Today" and "Yesterday" follow the date, and the microphone level
    // comes back if the welcome window took the preview meanwhile.
    window.addEventListener('focus', () => {
      this.history.followDate();
      void this.numbers.load();
      this.tray.regained();
    }, { signal });
    // The microphone stays open only while its settings page is shown; recordings stop and give back their URLs.
    window.addEventListener('pagehide', () => {
      this.tray.stopPreview();
      this.history.release();
    }, { signal });
  }

  today(): string {
    return isoDate(this.now());
  }

  /** The sections the header links to, in page order. */
  private sections: HTMLElement[] = [];
  private links: HTMLButtonElement[] = [];
  private scroller: HTMLElement | null = null;

  /** The settings and the history channel first (both before anything is drawn or asked for), then the first page. */
  async start(): Promise<void> {
    // A preview an earlier page of this window left running (it was reloaded) ends: the microphone is open only while
    // its settings page is shown.
    api.micPreview(false).catch((e) => logFailed('stopping the microphone level', e));
    const [view] = await Promise.all([
      api.settingsGet().catch((e) => {
        logFailed('reading the settings', e);
        return null;
      }),
      api.historySubscribe((entry) => {
        this.history.add(entry);
        this.numbers.refreshSoon();
      }).catch((e) => logFailed('following the history', e)),
    ]);
    this.settings = view;
    this.setLang(view?.lang);
    this.build();
    api.onMicLevel((level) => this.tray.level(level)).catch((e) => logFailed('following the microphone level', e));
    // The speech model's state: asked once, then followed.
    api.onModel((m) => this.tray.setModel(m)).catch((e) => logFailed('following the speech model', e));
    api.modelGet().then((m) => this.tray.setModel(m), (e) => logFailed('reading the speech model', e));
    // The Model page's list and the model in use, the same way.
    api.onModels((v) => this.tray.setModels(v)).catch((e) => logFailed('following the speech models', e));
    api.modelsGet().then((v) => this.tray.setModels(v), (e) => logFailed('reading the speech models', e));
    // A recording CUE starts must not take in a recording of the history playing aloud.
    api.onCoreState((s) => {
      if (s.phase === 'start' || s.phase === 'listen') this.history.recordingStarts();
    }).catch((e) => logFailed('following the recording state', e));
    await Promise.all([this.history.load(), this.numbers.load()]);
  }

  /** Listeners off (the page is gone); the microphone preview ends, recordings stop. */
  dispose(): void {
    this.off.abort();
    this.numbers.dispose();
    this.tray.stopPreview();
    this.history.release();
  }

  async change(c: SettingChange): Promise<SettingsView | null> {
    try {
      const view = await api.settingsSet(c);
      this.took(view);
      return view;
    } catch (e) {
      logFailed(`setting ${c.kind}`, e);
      // No answer in time: the core may have applied it after the wait — show what it holds now.
      if (codeOf(e) === 'core') {
        await api.settingsGet().then((v) => this.took(v), (err) => logFailed('reading the settings', err));
      }
      return null;
    }
  }

  menu(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string, align?: MenuAlign, wide?: boolean): void {
    this.menus.show(button, items, pick, note, align, wide);
  }

  closeMenu(): void {
    this.menus.close();
  }

  private setLang(v: unknown): void {
    this.lang = pickLang(v);
    this.t = TEXTS[this.lang];
    document.documentElement.lang = this.lang;
  }

  /** New settings from the core: the language may have changed (the whole page in it), a period too (the history
   *  shows what is left of it; a recording playing stops, and the entries say anew which have one). */
  private took(view: SettingsView): void {
    const keepChanged = this.settings?.historyKeep !== view.historyKeep;
    const audioChanged = this.settings?.historyAudio !== view.historyAudio;
    const relang = pickLang(view.lang) !== this.lang;
    this.settings = view;
    if (relang) {
      this.setLang(view.lang);
      this.closeMenu();
      this.build();
    } else {
      this.tray.settingsChanged();
      this.dict.render();
    }
    if (audioChanged) this.history.release();
    if (keepChanged || audioChanged) void this.history.reload();
  }

  /** The window's content in the current language; the History section and the tray keep their state. */
  private build(): void {
    const t = this.t;
    this.app.textContent = '';
    // The scroller stays in a screen reader's tree however it is marked (it scrolls): it is the page's main part.
    const bsc = h('div', 'bsc');
    bsc.setAttribute('role', 'main');
    bsc.setAttribute('aria-label', t.main);
    this.scroller = bsc;
    const bh = h('header', 'bh');
    bh.setAttribute('role', 'none');
    const logo = h('span', 'logo');
    logo.innerHTML = MARK;
    logo.append('CUE');
    const gear = iconButton('ibt gear', 'gear', t.settings);
    gear.addEventListener('click', () => (this.tray.isOpen ? this.tray.close() : this.tray.open()));
    const col = wrap('bcol');
    // Each section is named as its link, so a screen reader can go to it as to a region.
    const section = (scr: string, label: string) => {
      const s = h('section', 'bsec');
      s.dataset.scr = scr;
      s.setAttribute('aria-label', label);
      return s;
    };
    const todaySec = section('today', t.todayNav);
    const sec = section('history', t.history);
    const dictSec = section('dictionary', t.dictionary);
    this.sections = [todaySec, sec, dictSec];
    const nav = h('nav', 'lk');
    nav.setAttribute('aria-label', t.sections);
    this.links = [
      ['today', t.todayNav],
      ['history', t.history],
      ['dictionary', t.dictionary],
    ].map(([go, label]) => {
      const b = h('button', '', label);
      b.type = 'button';
      b.dataset.go = go;
      b.addEventListener('click', () => this.go(bsc, bh, go));
      return b;
    });
    nav.append(...this.links);
    bh.append(logo, nav, gear);
    col.append(todaySec, sec, dictSec);
    bsc.append(bh, col);
    bsc.addEventListener('scroll', () => {
      bh.classList.toggle('sc', bsc.scrollTop > 4);
      this.markCurrent(bh);
      // A menu would stay where its button was.
      if (this.menus.isOpen && !this.tray.isOpen) this.closeMenu();
    });
    const tray = h('div', 'btray');
    tray.hidden = true;
    tray.setAttribute('role', 'region');
    tray.setAttribute('aria-label', t.settings);
    tray.append(wrap('bin'));
    this.app.append(bsc, tray);
    this.numbers.mount(todaySec);
    this.history.mount(sec);
    this.dict.mount(dictSec);
    this.markCurrent(bh);
    this.tray.mount(tray, gear);
    document.fonts?.ready?.then(() => this.tray.refit());
  }

  /** A link's section comes up under the header: smoothly, at once with reduced motion. */
  private go(bsc: HTMLElement, bh: HTMLElement, go: string): void {
    const sec = this.sections.find((s) => s.dataset.scr === go);
    if (!sec) return;
    const top = go === this.sections[0]?.dataset.scr ? 0 : sec.offsetTop - bh.offsetHeight + 4;
    bsc.scrollTo?.({ top, behavior: this.reduced() ? 'auto' : 'smooth' });
  }

  /** The link of the last section whose top has passed under the header is the current one; scrolled to the end, the
   *  last section's (a short one can never reach the header). */
  private markCurrent(bh: HTMLElement): void {
    const line = bh.getBoundingClientRect().bottom + 1;
    let current = this.sections[0]?.dataset.scr;
    for (const s of this.sections) if (s.getBoundingClientRect().top <= line) current = s.dataset.scr;
    const sc = this.scroller;
    if (sc && sc.scrollTop > 4 && sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 2) {
      current = this.sections[this.sections.length - 1]?.dataset.scr;
    }
    for (const l of this.links) l.setAttribute('aria-current', String(l.dataset.go === current));
  }

  /** Esc: an open menu closes; in the tray, back to its first page, then closed; then the field of a new dictionary word
   *  closes, then the history's search. */
  private onKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    if (this.menus.isOpen) {
      this.closeMenu();
      e.preventDefault();
      return;
    }
    if (this.tray.esc() || this.dict.esc() || this.history.esc()) e.preventDefault();
  }
}

/** Builds the page in `root` once the core has told it the settings. */
export async function mountWindow(root: HTMLElement, opts: PageOptions = {}): Promise<WindowPage> {
  const page = new WindowPage(root, opts);
  await page.start();
  return page;
}

const root = document.getElementById('cue-window');
if (root) void mountWindow(root);
