// The settings tray (spec 3 §3): it grows out of the gear (the gear turns into its close button) and folds back into
// it. Its first page lists Keys, Microphone, Translation and General with their values; one of them at a time slides
// in, the chevron goes back; Esc goes back, then closes. Every change goes to the core at once.
// A key is typed into a password field, sent once to be saved and the field cleared: its value is not kept here and
// never logged. The microphone page listens to the chosen microphone's level while it is shown, and only then.
import * as api from './api';
import type { KeyName, MicList, UiLanguage } from './api';
import { icon, type IconName } from './icons';
import { codeOf, h, iconButton, logFailed, relRect, type Ctx, type MenuItem } from './page';
import { fill, languageName, type KeyState } from './text';

type Sub = 'keys' | 'mic' | 'translate' | 'general';
const SUBS: readonly Sub[] = ['keys', 'mic', 'translate', 'general'];
const SUB_ICON: Record<Sub, IconName> = { keys: 'apiKey', mic: 'micOn', translate: 'globe', general: 'half' };
const KEYS: readonly KeyName[] = ['soniox', 'anthropic'];
const KEY_TITLE: Record<KeyName, string> = { soniox: 'Soniox', anthropic: 'Anthropic' };
const UI_LANGUAGES: readonly UiLanguage[] = ['auto', 'en', 'ru'];
/** The level meter's segments. */
export const METER_SEGMENTS = 16;
/** The tray's width (the design's). */
const WIDTH = 340;
/** How long the tray takes to fold back before it is hidden. */
const FOLD_MS = 470;

interface KeyRow {
  saved: boolean;
  mask: string;
  state: KeyState | null;
}

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

/** Which of `names` the core records from for the pinned `wanted`, by the core's own rule (`mic::pick` in
 *  platform-win): the same name once both are trimmed, else the first that differs only in case; -1 — not there. */
export function matchMic(names: readonly string[], wanted: string): number {
  const w = wanted.trim();
  if (!w) return -1;
  const exact = names.findIndex((n) => n.trim() === w);
  if (exact >= 0) return exact;
  const lw = w.toLowerCase();
  return names.findIndex((n) => n.trim().toLowerCase() === lw);
}

/** A settings row: its label and an optional line under it on the left, its controls on the right. */
function srow(label: string, desc: string | null, ...ctrls: HTMLElement[]): { row: HTMLElement; small: HTMLElement } {
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

function dropDown(kind: string, value: string): HTMLButtonElement {
  const b = h('button', 'dd');
  b.type = 'button';
  b.dataset.dd = kind;
  b.append(h('span', 'ddv', value));
  b.insertAdjacentHTML('beforeend', icon('chev'));
  return b;
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
  private keys: Record<KeyName, KeyRow> = {
    soniox: { saved: false, mask: '', state: null },
    anthropic: { saved: false, mask: '', state: null },
  };
  private keysRead = false;
  private mics: MicList | null = null;
  private autostart: boolean | null = null;
  private previewOn = false;

  constructor(private readonly ctx: Ctx) {}

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
        void this.check(input.dataset.key as KeyName);
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
    void this.readKeys();
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
    if (this.sub !== 'mic') return;
    const n = Math.round(Math.min(1, Math.max(0, Number(level) || 0)) * METER_SEGMENTS);
    this.current?.querySelectorAll('[data-meter] i').forEach((i, k) => i.classList.toggle('on', k < n));
  }

  stopPreview(): void {
    if (!this.previewOn) return;
    this.previewOn = false;
    api.micPreview(false).catch((e) => logFailed('stopping the microphone level', e));
  }

  private startPreview(): void {
    this.previewOn = true;
    api.micPreview(true, this.ctx.settings?.micDevice ?? null).catch((e) => logFailed('showing the microphone level', e));
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
      this.startPreview();
      void this.readMics();
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
    else if ((b = t.closest<HTMLElement>('[data-check]'))) void this.check(b.dataset.check as KeyName);
    else if ((b = t.closest<HTMLElement>('[data-dd="mic"]'))) this.micMenu(b);
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
    if (sub === 'keys') g.append(...KEYS.map((k) => this.keyRow(k)));
    else if (sub === 'mic') g.append(...this.micRows());
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
        if (!this.keysRead) return '';
        const missing = KEYS.filter((k) => !this.keys[k].saved);
        if (missing.length === KEYS.length) return t.noKeys;
        if (missing.length) return fill(t.noKey, KEY_TITLE[missing[0]]);
        return KEYS.map((k) => KEY_TITLE[k]).join(', ');
      }
      case 'mic':
        return s ? (s.micDevice ?? t.windowsMic) : '';
      case 'translate': {
        if (!s) return '';
        const english = s.languages.find(([c]) => c === s.translateTarget)?.[1] ?? s.translateTarget;
        return languageName(s.translateTarget, english, this.ctx.lang);
      }
      case 'general':
        return s ? t.uiLanguages[s.uiLanguage] ?? '' : '';
    }
  }

  /* ---------- keys ---------- */

  private keyRow(name: KeyName): HTMLElement {
    const t = this.ctx.t;
    const input = h('input', 'kin');
    input.type = 'password';
    input.setAttribute('autocomplete', 'off');
    input.spellcheck = false;
    input.dataset.key = name;
    input.setAttribute('aria-label', KEY_TITLE[name]);
    const btn = h('button', 'btn2 ck', t.check);
    btn.type = 'button';
    btn.dataset.check = name;
    const { row, small } = srow(KEY_TITLE[name], null, input, btn);
    small.dataset.state = name;
    row.dataset.key = name;
    this.paintKey(name, input, btn, small);
    return row;
  }

  /** A key row's hint (the mask, or "paste the key") and state. */
  private paintKey(name: KeyName, input: HTMLInputElement, btn: HTMLButtonElement, small: HTMLElement): void {
    const t = this.ctx.t;
    const k = this.keys[name];
    input.placeholder = k.saved && k.mask ? k.mask : t.keyHint;
    btn.disabled = k.state === 'checking';
    const state = k.state ?? (this.keysRead ? (k.saved ? 'saved' : 'missing') : null);
    small.textContent = state ? t.keyStates[state] : '';
    small.hidden = !state;
    small.classList.toggle('ok', state === 'works');
    small.classList.toggle('warn', state === 'badKey');
  }

  /** The key rows and the first page's value show what is known now. */
  private refreshKeys(): void {
    const pg = this.current;
    if (!pg) return;
    const em = pg.querySelector('em[data-val="keys"]');
    if (em) em.textContent = this.value('keys');
    for (const name of KEYS) {
      const row = pg.querySelector(`.srow[data-key="${name}"]`);
      if (!row) continue;
      this.paintKey(name, row.querySelector('input')!, row.querySelector('button')!, row.querySelector('small')!);
    }
    this.fit();
  }

  private async readKeys(): Promise<void> {
    await Promise.all(
      KEYS.map((name) =>
        api.keyState(name).then(
          (v) => Object.assign(this.keys[name], { saved: v.saved, mask: v.mask }),
          (e) => logFailed(`reading the ${name} key`, e),
        ),
      ),
    );
    this.keysRead = true;
    this.refreshKeys();
  }

  /** The field's key, if one was typed, is saved (the field is emptied at once), then the saved key is checked with a
   *  free request. */
  private async check(name: KeyName): Promise<void> {
    const k = this.keys[name];
    if (k.state === 'checking') return;
    k.state = 'checking';
    this.refreshKeys();
    try {
      if (await this.saveTyped(name)) k.state = await api.keyCheck(name);
    } catch (e) {
      logFailed(`checking the ${name} key`, e);
      k.state = 'unchecked';
    }
    this.refreshKeys();
  }

  /** Sends the typed key to be saved and clears the field; false — it was refused. Nothing typed: true. */
  private async saveTyped(name: KeyName): Promise<boolean> {
    const input = this.current?.querySelector<HTMLInputElement>(`input[data-key="${name}"]`);
    const typed = input?.value ?? '';
    if (input) input.value = '';
    if (!typed.trim()) return true;
    const k = this.keys[name];
    try {
      const v = await api.keySet(name, typed);
      Object.assign(k, { saved: v.saved, mask: v.mask });
      return true;
    } catch (e) {
      logFailed(`saving the ${name} key`, e);
      k.state = codeOf(e) === 'emptyKey' ? 'missing' : 'store';
      return false;
    }
  }

  /* ---------- microphone ---------- */

  /** The microphones Windows has, each name once. */
  private micNames(): string[] {
    const seen = new Set<string>();
    const names: string[] = [];
    for (const n of this.mics?.names ?? []) {
      if (seen.has(n.trim())) continue;
      seen.add(n.trim());
      names.push(n);
    }
    return names;
  }

  /** The listed microphone the core records from for the pinned one (`matchMic`); -1 — none, or none pinned. */
  private pinnedAt(): number {
    const pinned = this.ctx.settings?.micDevice ?? null;
    return pinned ? matchMic(this.micNames(), pinned) : -1;
  }

  /** A microphone is pinned, the list is known, and it is not in it. */
  private pinnedMissing(): boolean {
    return !!this.ctx.settings?.micDevice && !!this.mics && this.pinnedAt() < 0;
  }

  private micRows(): HTMLElement[] {
    const t = this.ctx.t;
    const at = this.pinnedAt();
    const shown = at >= 0 ? this.micNames()[at] : this.value('mic');
    const choose = srow(t.micLabel, this.pinnedMissing() ? t.notConnectedHint : null, dropDown('mic', shown));
    choose.small.classList.add('warn');
    const meter = h('span', 'meter');
    meter.dataset.meter = '';
    for (let i = 0; i < METER_SEGMENTS; i++) meter.append(h('i'));
    return [choose.row, srow(t.micTest, t.micTestHint, meter).row];
  }

  private async readMics(): Promise<void> {
    try {
      this.mics = await api.micList();
    } catch (e) {
      logFailed('listing the microphones', e);
      return;
    }
    if (this.sub === 'mic') this.show(this.page('mic'), 0);
  }

  private micMenu(b: HTMLElement): void {
    const t = this.ctx.t;
    const pinned = this.ctx.settings?.micDevice ?? null;
    const at = this.pinnedAt();
    const windows = this.mics?.default ? fill(t.windowsMicNamed, this.mics.default) : t.windowsMic;
    const items: MenuItem[] = [
      { value: '', label: windows, on: pinned === null },
      ...this.micNames().map((n, i) => ({ value: n, label: n, on: i === at })),
    ];
    if (pinned && this.pinnedMissing()) items.push({ value: pinned, label: pinned, note: t.notConnected, on: true });
    this.ctx.menu(b, items, (v) => void this.chooseMic(v || null));
  }

  private async chooseMic(device: string | null): Promise<void> {
    const view = await this.ctx.change({ kind: 'micDevice', value: device });
    // The level follows the new choice.
    if (view && this.sub === 'mic' && this.previewOn) {
      api.micPreview(true, view.micDevice).catch((e) => logFailed('showing the microphone level', e));
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
