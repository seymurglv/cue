// The settings tray (spec 3 §3): it grows out of the gear (the gear turns into its close button) and folds back into
// it. Its first page lists Keys, Microphone, Translation and General with their values; one of them at a time slides
// in, the chevron goes back; Esc goes back, then closes. Every change goes to the core at once.
// The Keys page holds the key rows of keys.ts, the Microphone page the microphone rows of mic.ts (the welcome window
// shows them too); the microphone's level is listened to only while that page is shown.
import * as api from './api';
import type { KeyName, UiLanguage } from './api';
import { icon, type IconName } from './icons';
import { KEYS, KEY_TITLE, KeyRows } from './keys';
import { MicRows } from './mic';
import { dropDown, h, iconButton, logFailed, relRect, srow, type Ctx } from './page';
import { fill, languageName } from './text';

export { matchMic } from './mic';

type Sub = 'keys' | 'mic' | 'translate' | 'general';
const SUBS: readonly Sub[] = ['keys', 'mic', 'translate', 'general'];
const SUB_ICON: Record<Sub, IconName> = { keys: 'apiKey', mic: 'micOn', translate: 'globe', general: 'half' };
const UI_LANGUAGES: readonly UiLanguage[] = ['auto', 'en', 'ru'];
/** The tray's width (the design's). */
const WIDTH = 340;
/** How long the tray takes to fold back before it is hidden. */
const FOLD_MS = 470;

/** The design's segment thumb: under the pressed button, without sliding the first time. */
function syncSeg(sg: HTMLElement): void {
  const b = sg.querySelector<HTMLElement>('button[aria-pressed=true]');
  const th = sg.querySelector<HTMLElement>('.thb');
  if (!b || !th) return;
  const first = !sg.dataset.ready;
  if (first) th.style.transition = 'none';
  th.style.width = `${b.offsetWidth}px`;
  th.style.transform = `translateX(${b.offsetLeft - 2}px)`;
  if (first) {
    void th.offsetWidth;
    th.style.transition = '';
    sg.dataset.ready = '1';
  }
}

export class SettingsTray {
  isOpen = false;
  sub: Sub | null = null;
  private el: HTMLElement | null = null;
  private bin!: HTMLElement;
  private gear!: HTMLElement;
  /** The page shown (pages sliding out are on their way out). */
  private current: HTMLElement | null = null;
  private hideTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly keys: KeyRows;
  private readonly mic: MicRows;
  private autostart: boolean | null = null;

  constructor(private readonly ctx: Ctx) {
    this.keys = new KeyRows(ctx, () => this.current, () => {
      const em = this.current?.querySelector('em[data-val="keys"]');
      if (em) em.textContent = this.value('keys');
      this.fit();
    });
    this.mic = new MicRows(ctx, () => this.current, () => {
      if (this.sub === 'mic') this.show(this.page('mic'), 0);
    });
  }

  /** Takes the page's tray box and gear; an open tray stays open on its page (the page was built again). */
  mount(el: HTMLElement, gear: HTMLElement): void {
    this.el = el;
    this.bin = el.querySelector('.bin')!;
    this.gear = gear;
    this.current = null;
    el.addEventListener('click', (e) => this.onClick(e));
    el.addEventListener('keydown', (e) => {
      const input = (e.target as Element).closest<HTMLInputElement>('input[data-key]');
      if (input && e.key === 'Enter') {
        e.preventDefault();
        void this.keys.check(input.dataset.key as KeyName);
      }
    });
    if (!this.isOpen) return;
    el.hidden = false;
    el.classList.add('open');
    gear.classList.add('hid');
    this.place();
    this.show(this.page(this.sub), 0);
  }

  open(): void {
    const el = this.el;
    if (this.isOpen || !el) return;
    this.isOpen = true;
    this.sub = null;
    if (this.hideTimer) clearTimeout(this.hideTimer);
    el.style.transition = 'none';
    Object.assign(el.style, this.gearBox());
    el.hidden = false;
    el.classList.remove('open');
    this.bin.textContent = '';
    this.current = null;
    void el.offsetWidth;
    el.style.transition = '';
    this.place();
    this.show(this.page(null), 1);
    el.classList.add('open');
    this.gear.classList.add('hid');
    void this.keys.readAll();
  }

  close(): void {
    const el = this.el;
    if (!this.isOpen || !el) return;
    this.isOpen = false;
    if (this.sub === 'mic') this.stopPreview();
    this.sub = null;
    this.ctx.closeMenu();
    el.classList.remove('open');
    Object.assign(el.style, this.gearBox());
    this.gear.classList.remove('hid');
    this.hideTimer = setTimeout(() => {
      if (!this.isOpen) el.hidden = true;
    }, this.ctx.reduced() ? 0 : FOLD_MS);
  }

  /** Esc: back to the first page, then closed; false — the tray is not open. */
  esc(): boolean {
    if (!this.isOpen) return false;
    if (this.sub) this.back();
    else this.close();
    return true;
  }

  /** Next to the gear, as wide as the design draws it (the window may have been resized). */
  place(): void {
    if (!this.isOpen || !this.el) return;
    const g = relRect(this.ctx.app, this.gear);
    Object.assign(this.el.style, { left: `${g.x + g.w + 6 - WIDTH}px`, top: `${g.y - 6}px`, width: `${WIDTH}px`, borderRadius: '14px' });
    this.fit();
  }

  /** The height follows the page, up to the window's bottom. */
  refit(): void {
    if (this.isOpen) {
      this.current?.querySelectorAll<HTMLElement>('.segc').forEach(syncSeg);
      this.fit();
    }
  }

  /** The settings changed: the page shown shows the new values. */
  settingsChanged(): void {
    if (this.isOpen) this.show(this.page(this.sub), 0);
  }

  /** The microphone's level on the meter (only the microphone page has one). */
  level(level: number): void {
    if (this.sub === 'mic') this.mic.level(level);
  }

  stopPreview(): void {
    this.mic.stopPreview();
  }

  private fit(): void {
    const pg = this.current;
    if (!pg || !this.el) return;
    const top = parseFloat(this.el.style.top) || 0;
    const maxH = Math.max(0, this.ctx.app.offsetHeight - top - 14);
    this.bin.style.maxHeight = `${maxH}px`;
    this.el.style.height = `${Math.min(pg.offsetHeight, maxH)}px`;
  }

  private gearBox(): Partial<CSSStyleDeclaration> {
    const g = relRect(this.ctx.app, this.gear);
    return { left: `${g.x}px`, top: `${g.y}px`, width: `${g.w}px`, height: `${g.h}px`, borderRadius: `${g.h / 2}px` };
  }

  /** `pg` in place of the page shown: sliding in from the side of `dir` (1 — deeper, -1 — back), at once with 0. */
  private show(pg: HTMLElement, dir: number): void {
    const olds = [...this.bin.children] as HTMLElement[];
    this.bin.append(pg);
    this.bin.scrollTop = 0;
    this.current = pg;
    const slide = dir !== 0 && !this.ctx.reduced() && typeof pg.animate === 'function';
    for (const old of olds) {
      Object.assign(old.style, { position: 'absolute', left: '0', right: '0', top: '0' });
      if (!slide) {
        old.remove();
        continue;
      }
      old.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${-dir * 40}px)` }], {
        duration: 240,
        easing: 'ease-in',
        fill: 'forwards',
      }).onfinish = () => old.remove();
    }
    if (slide && olds.length) {
      pg.animate([{ opacity: 0, transform: `translateX(${dir * 40}px)` }, { opacity: 1, transform: 'none' }], {
        duration: 440,
        easing: 'cubic-bezier(.22,1,.36,1)',
      });
    }
    pg.querySelectorAll<HTMLElement>('.segc').forEach(syncSeg);
    this.fit();
  }

  private go(sub: Sub): void {
    this.sub = sub;
    this.show(this.page(sub), 1);
    if (sub === 'mic') {
      this.mic.startPreview();
      void this.mic.read();
    } else if (sub === 'general') {
      void this.readAutostart();
    }
  }

  private back(): void {
    if (this.sub === 'mic') this.stopPreview();
    this.ctx.closeMenu();
    this.sub = null;
    this.show(this.page(null), -1);
  }

  private onClick(e: Event): void {
    const t = e.target as Element;
    let b: HTMLElement | null;
    if ((b = t.closest<HTMLElement>('[data-sub]'))) this.go(b.dataset.sub as Sub);
    else if (t.closest('[data-tback]')) this.back();
    else if (t.closest('[data-tclose]')) this.close();
    else if ((b = t.closest<HTMLElement>('[data-check]'))) void this.keys.check(b.dataset.check as KeyName);
    else if ((b = t.closest<HTMLElement>('[data-dd="mic"]'))) this.mic.menu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="target"]'))) this.targetMenu(b);
    else if (t.closest('[data-sw="autostart"]')) void this.toggleAutostart();
    else if ((b = t.closest<HTMLElement>('.segc button')) && b.getAttribute('aria-pressed') !== 'true') {
      void this.ctx.change({ kind: 'uiLanguage', value: b.dataset.v as UiLanguage });
    }
  }

  /* ---------- pages ---------- */

  private page(sub: Sub | null): HTMLElement {
    const t = this.ctx.t;
    const pg = h('div', 'tpg');
    const tph = h('div', 'tph');
    if (sub) {
      const back = iconButton('ibt back', 'chev', t.back);
      back.dataset.tback = '';
      tph.append(back);
    }
    const title = h('b', sub ? '' : 'lbl', sub ? this.title(sub) : t.settings);
    const x = iconButton('ibt', 'x', t.close);
    x.dataset.tclose = '';
    tph.append(title, x);
    pg.append(tph);
    if (!sub) {
      for (const s of SUBS) {
        const row = h('button', 'trw');
        row.type = 'button';
        row.dataset.sub = s;
        const si = h('span', 'si');
        si.innerHTML = icon(SUB_ICON[s]);
        const em = h('em', '', this.value(s));
        em.dataset.val = s;
        row.append(si, h('span', '', this.title(s)), em);
        row.insertAdjacentHTML('beforeend', icon('chev'));
        pg.append(row);
      }
      if (this.ctx.settings && !this.ctx.settings.saved) pg.append(h('div', 'tft', t.unsaved));
      return pg;
    }
    const g = h('div', 'sgrp');
    if (sub === 'keys') g.append(...KEYS.map((k) => this.keys.row(k)));
    else if (sub === 'mic') g.append(...this.mic.rows());
    else if (sub === 'translate') g.append(srow(t.translateTo, null, dropDown('target', this.value('translate'))).row);
    else g.append(...this.generalRows());
    pg.append(g);
    return pg;
  }

  private title(sub: Sub): string {
    const t = this.ctx.t;
    return { keys: t.keys, mic: t.mic, translate: t.translate, general: t.general }[sub];
  }

  /** A section's value on the first page. */
  private value(sub: Sub): string {
    const t = this.ctx.t;
    const s = this.ctx.settings;
    switch (sub) {
      case 'keys': {
        if (!this.keys.read) return '';
        const missing = KEYS.filter((k) => !this.keys.keys[k].saved);
        if (missing.length === KEYS.length) return t.noKeys;
        if (missing.length) return fill(t.noKey, KEY_TITLE[missing[0]]);
        return KEYS.map((k) => KEY_TITLE[k]).join(', ');
      }
      case 'mic':
        return this.mic.label();
      case 'translate': {
        if (!s) return '';
        const english = s.languages.find(([c]) => c === s.translateTarget)?.[1] ?? s.translateTarget;
        return languageName(s.translateTarget, english, this.ctx.lang);
      }
      case 'general':
        return s ? t.uiLanguages[s.uiLanguage] ?? '' : '';
    }
  }

  /* ---------- translation ---------- */

  private targetMenu(b: HTMLElement): void {
    const s = this.ctx.settings;
    if (!s) return;
    const lang = this.ctx.lang;
    const items = s.languages
      .map(([code, english]) => ({ value: code, label: languageName(code, english, lang), on: code === s.translateTarget }))
      .sort((a, c) => a.label.localeCompare(c.label, lang));
    this.ctx.menu(b, items, (v) => void this.ctx.change({ kind: 'translateTarget', value: v }));
  }

  /* ---------- general ---------- */

  private generalRows(): HTMLElement[] {
    const t = this.ctx.t;
    const sw = h('button', 'sw');
    sw.type = 'button';
    sw.dataset.sw = 'autostart';
    sw.setAttribute('role', 'switch');
    sw.setAttribute('aria-checked', String(!!this.autostart));
    sw.setAttribute('aria-label', t.autostart);
    sw.disabled = this.autostart === null;
    const seg = h('span', 'segc');
    seg.append(h('span', 'thb'));
    const cur = this.ctx.settings?.uiLanguage;
    for (const l of UI_LANGUAGES) {
      const b = h('button', '', t.uiLanguages[l]);
      b.type = 'button';
      b.dataset.v = l;
      b.setAttribute('aria-pressed', String(l === cur));
      seg.append(b);
    }
    return [srow(t.autostart, null, sw).row, srow(t.uiLanguage, null, seg).row];
  }

  private async readAutostart(): Promise<void> {
    try {
      this.autostart = await api.autostartGet();
    } catch (e) {
      logFailed('reading start at sign-in', e);
      return;
    }
    this.paintAutostart();
  }

  private async toggleAutostart(): Promise<void> {
    if (this.autostart === null) return;
    try {
      this.autostart = await api.autostartSet(!this.autostart);
    } catch (e) {
      logFailed('setting start at sign-in', e);
    }
    this.paintAutostart();
  }

  private paintAutostart(): void {
    const sw = this.current?.querySelector<HTMLButtonElement>('[data-sw="autostart"]');
    if (!sw) return;
    sw.setAttribute('aria-checked', String(!!this.autostart));
    sw.disabled = this.autostart === null;
  }
}
