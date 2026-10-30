// The main window's page (spec 3 §1–3): the header — the live mark and CUE on the left, the gear on the right — the
// History section, and the settings tray that grows out of the gear. Ported from the approved design (stage 5).
// The page draws nothing until the core has told it the settings: the language is SettingsView.lang (the window has
// no language set before its script runs). It follows new history entries over its own channel, opened before the
// first page is asked for.
// User text — history text, app and device names — goes into the page as text (textContent, title), never as HTML.
import '../design/cue.css';
import '../design/window.css';
import '../design/cue.js';
import '../design/drop.js';
import * as api from './api';
import type { SettingChange, SettingsView } from './api';
import { CUE } from '../designParts';
import { HistoryView } from './history';
import { icon } from './icons';
import { SettingsTray } from './settings';
import { TEXTS, isoDate, pickLang, type Lang, type WinText } from './text';

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';
const DARK = '(prefers-color-scheme: dark)';
/** How long a button shows its "done" check (as in the design). */
const FLASH_MS = 1400;

/** One choice of a menu: its value, its label (text), a muted note after it, whether it is the current one. */
export interface MenuItem {
  value: string;
  label: string;
  note?: string;
  on: boolean;
}

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
  /** A menu under `button` (a second press on the same button closes it); `pick` gets the value chosen. */
  menu(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string): void;
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

/** A small icon button with its tooltip and accessible name. */
export function iconButton(cls: string, name: Parameters<typeof icon>[0], label: string): HTMLButtonElement {
  const b = h('button', cls);
  b.type = 'button';
  b.innerHTML = icon(name);
  b.title = label;
  b.setAttribute('aria-label', label);
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

export class WindowPage implements Ctx {
  t: WinText = TEXTS.en;
  lang: Lang = 'en';
  settings: SettingsView | null = null;
  readonly history: HistoryView;
  readonly tray: SettingsTray;
  readonly reduced: () => boolean;
  private readonly now: () => Date;
  private openMenu: { m: HTMLElement; b: HTMLElement } | null = null;
  private logo: any = null;
  private looping = false;
  private readonly off = new AbortController();

  constructor(readonly app: HTMLElement, opts: PageOptions = {}) {
    this.reduced = opts.reduced ?? (() => !!window.matchMedia?.(REDUCED_MOTION)?.matches);
    this.now = opts.now ?? (() => new Date());
    app.classList.add('app');
    this.history = new HistoryView(this);
    this.tray = new SettingsTray(this);
    const signal = this.off.signal;
    const dark = window.matchMedia?.(DARK);
    const theme = () => (document.documentElement.dataset.theme = dark?.matches ? 'dark' : 'light');
    theme();
    dark?.addEventListener?.('change', theme, { signal });
    window.matchMedia?.(REDUCED_MOTION)?.addEventListener?.('change', () => this.animate(), { signal });
    document.addEventListener('keydown', (e) => this.onKey(e), { signal });
    // Outside a menu or the tray: by the event's path as it was dispatched — a click inside may already have replaced
    // the element it hit (a tray page, a menu that closed).
    const within = (e: Event, sel: string) => e.composedPath().some((n) => n instanceof Element && n.matches(sel));
    document.addEventListener('pointerdown', (e) => {
      if (this.openMenu && !within(e, '.menu, [data-dd]')) this.closeMenu();
    }, { signal });
    document.addEventListener('click', (e) => {
      if (this.tray.isOpen && !within(e, '.btray, .menu, .gear')) this.tray.close();
    }, { signal });
    window.addEventListener('resize', () => this.tray.place(), { signal });
    // Back to the window, maybe on another day: "Today" and "Yesterday" follow the date.
    window.addEventListener('focus', () => this.history.followDate(), { signal });
    // The microphone stays open only while its settings page is shown.
    window.addEventListener('pagehide', () => this.tray.stopPreview(), { signal });
  }

  today(): string {
    return isoDate(this.now());
  }

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
      api.historySubscribe((entry) => this.history.add(entry)).catch((e) => logFailed('following the history', e)),
    ]);
    this.settings = view;
    this.setLang(view?.lang);
    this.build();
    api.onMicLevel((level) => this.tray.level(level)).catch((e) => logFailed('following the microphone level', e));
    await this.history.load();
  }

  /** Listeners off (the page is gone); the microphone preview ends. */
  dispose(): void {
    this.off.abort();
    this.tray.stopPreview();
    this.looping = false;
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

  menu(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string): void {
    if (this.openMenu?.b === button) {
      this.closeMenu();
      return;
    }
    this.closeMenu();
    if (!items.length) return;
    const m = h('div', 'menu');
    m.setAttribute('role', 'menu');
    let current: HTMLElement | null = null;
    items.forEach((it, i) => {
      const b = h('button');
      b.type = 'button';
      b.dataset.i = String(i);
      b.setAttribute('role', 'menuitemradio');
      b.setAttribute('aria-checked', String(it.on));
      b.append(h('span', 'mlb', it.label));
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
      this.closeMenu();
      if (it) pick(it.value);
    });
    this.app.append(m);
    this.openMenu = { m, b: button };
    this.placeMenu(m, button, current);
  }

  closeMenu(): void {
    this.openMenu?.m.remove();
    this.openMenu = null;
  }

  /** Under the button, its right edge on the button's; above it when there is no room below; scrolled to the current
   *  choice when it is taller than the room. */
  private placeMenu(m: HTMLElement, b: HTMLElement, current: HTMLElement | null): void {
    const p = relRect(this.app, b);
    const H = this.app.offsetHeight;
    const below = H - (p.y + p.h + 4) - 8;
    const above = p.y - 4 - 8;
    const tall = m.offsetHeight;
    const down = tall <= below || below >= above;
    m.style.maxHeight = `${Math.max(120, down ? below : above)}px`;
    m.style.left = `${Math.max(8, Math.min(p.x + p.w - m.offsetWidth, this.app.offsetWidth - m.offsetWidth - 8))}px`;
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

  private setLang(v: unknown): void {
    this.lang = pickLang(v);
    this.t = TEXTS[this.lang];
    document.documentElement.lang = this.lang;
  }

  /** New settings from the core: the language may have changed (the whole page in it), the period too (the history
   *  shows what is left of it). */
  private took(view: SettingsView): void {
    const keepChanged = this.settings?.historyKeep !== view.historyKeep;
    const relang = pickLang(view.lang) !== this.lang;
    this.settings = view;
    if (relang) {
      this.setLang(view.lang);
      this.closeMenu();
      this.build();
    } else {
      this.history.settingsChanged();
      this.tray.settingsChanged();
    }
    if (keepChanged) void this.history.reload();
  }

  /** The window's content in the current language; the History section and the tray keep their state. */
  private build(): void {
    const t = this.t;
    this.app.textContent = '';
    const bsc = h('div', 'bsc');
    const bh = h('header', 'bh');
    const logo = h('span', 'logo');
    logo.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><g></g></svg>';
    logo.append('CUE');
    const gear = iconButton('ibt gear', 'gear', t.settings);
    gear.addEventListener('click', () => (this.tray.isOpen ? this.tray.close() : this.tray.open()));
    bh.append(logo, gear);
    const col = h('div', 'bcol');
    const sec = h('section', 'bsec');
    sec.dataset.scr = 'history';
    col.append(sec);
    bsc.append(bh, col);
    bsc.addEventListener('scroll', () => {
      bh.classList.toggle('sc', bsc.scrollTop > 4);
      // A menu would stay where its button was.
      if (this.openMenu && !this.tray.isOpen) this.closeMenu();
    });
    const tray = h('div', 'btray');
    tray.hidden = true;
    tray.append(h('div', 'bin'));
    this.app.append(bsc, tray);
    this.logo = new (CUE().Morph)(logo.querySelector('g'), { state: 'idle' });
    this.history.mount(sec);
    this.tray.mount(tray, gear);
    document.fonts?.ready?.then(() => this.tray.refit());
    this.animate();
  }

  /** The mark breathes in its idle pose, frame by frame; with reduced motion it stands still. */
  private animate(): void {
    if (this.looping || this.reduced() || this.off.signal.aborted) return;
    this.looping = true;
    const frame = (now: number) => {
      if (!this.looping || this.reduced()) {
        this.looping = false;
        return;
      }
      this.logo?.tick(now);
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }

  /** Esc: an open menu closes; in the tray, back to its first page, then closed. */
  private onKey(e: KeyboardEvent): void {
    if (e.key !== 'Escape') return;
    if (this.openMenu) {
      this.closeMenu();
      e.preventDefault();
      return;
    }
    if (this.tray.esc()) e.preventDefault();
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
