// The settings tray (spec 3 §3, regrouped in docs/specs/2026-11-02-settings-tray.md): it grows out of the gear (the gear
// turns into its close button) and folds back into it. Its first page lists Speech recognition (the model on this
// computer, window/model.ts, and the Anthropic key), Microphone, Languages (the interface's and the translation's),
// History (how long the history and its recordings are kept, and clearing it) and General with their values; one of them at a time slides in, the chevron goes back; Esc goes
// back, then closes. Every change goes to the core at once.
// Going from page to page, the heading line stays: its title changes letter by letter and the chevron grows in front
// of it; only the rows below slide. The height eases to the new page without overshooting, and nothing scrolls while
// the pages pass (a scroll bar showing for a moment would narrow the rows). A page whose values come later (the
// microphones, new settings) is filled in where it is, even while it is still sliding in.
// Speech recognition holds the key row of keys.ts, the Microphone page the microphone rows of mic.ts (the welcome window
// shows them too); the microphone's level is listened to only while that page is shown.
import * as api from './api';
import { AUDIO_KEEPS, KEEPS, type AudioKeep, type Keep, type KeyName, type UiLanguage } from './api';
import { CUE } from '../designParts';
import { icon, type IconName } from './icons';
import { KeyRows } from './keys';
import { MicRows } from './mic';
import { bar, modelRow, modelValue, stepModelRow } from './model';
import { dropDown, h, iconButton, logFailed, relRect, srow, type Ctx } from './page';
import { languageName } from './text';

export { matchMic } from './mic';

type Sub = 'model' | 'mic' | 'languages' | 'history' | 'general';
const SUBS: readonly Sub[] = ['model', 'mic', 'languages', 'history', 'general'];
const SUB_ICON: Record<Sub, IconName> = {
  model: 'wave',
  mic: 'micOn',
  languages: 'globe',
  history: 'clock',
  general: 'half',
};
const UI_LANGUAGES: readonly UiLanguage[] = ['auto', 'en', 'ru'];
/** The tray's width (the design's). */
const WIDTH = 340;
/** How long the tray takes to fold back before it is hidden. */
const FOLD_MS = 470;
/** How long pages take to pass (window.css: .btray.swap — the height; the slides in `show` are shorter). */
const SWAP_MS = 480;
/** "Clear history" waits this long for its second press. */
export const CLEAR_ARMED_MS = 3000;

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
  /** The tray's content: the heading line (it stays from page to page) over the pages' rows. */
  private frame: HTMLElement | null = null;
  private head!: HTMLElement;
  private heading!: HTMLElement;
  private pages!: HTMLElement;
  /** The rows of the page shown (pages sliding out are on their way out). */
  private current: HTMLElement | null = null;
  private hideTimer: ReturnType<typeof setTimeout> | null = null;
  private swapTimer: ReturnType<typeof setTimeout> | null = null;
  private readonly keys: KeyRows;
  private readonly mic: MicRows;
  private autostart: boolean | null = null;
  /** The speech model's state, as `cue://model` last said (null — not heard yet). */
  private model: api.ModelState | null = null;
  /** "Clear history" was pressed once and waits for the second press. */
  private clearTimer: ReturnType<typeof setTimeout> | null = null;

  /** `clearHistory` — the history's own clearing (the list empties with it). */
  constructor(
    private readonly ctx: Ctx,
    private readonly clearHistory: () => Promise<void>,
  ) {
    // The key's state shows in Speech recognition's value ("No Anthropic key") and on its page.
    this.keys = new KeyRows(ctx, () => this.current, () => {
      this.paintModelValue();
      this.fit();
    });
    this.mic = new MicRows(ctx, () => this.current, () => {
      if (this.sub === 'mic') this.redraw();
    });
  }

  /** The speech model's new state: its value on the first page; on its page a download goes on in place, any other
   *  change draws the model's row again. */
  setModel(state: api.ModelState): void {
    this.model = state;
    this.paintModelValue();
    if (this.sub !== 'model') return;
    const row = this.current?.querySelector('.mrow');
    if (row && !stepModelRow(row, state)) row.replaceWith(modelRow(this.ctx.t, state));
    this.fit();
  }

  /** The first page's Speech recognition value: its text, amber or Bebas, and the line under the row while the model
   *  downloads. */
  private paintModelValue(page: ParentNode | null = this.current): void {
    const row = page?.querySelector<HTMLElement>('[data-sub="model"]');
    if (!row) return;
    const v = modelValue(this.ctx.t, this.model, this.keys.read && !this.keys.keys.anthropic.saved);
    const em = row.querySelector('em')!;
    em.textContent = v.text;
    em.classList.toggle('warn', v.warn);
    em.classList.toggle('num', v.progress !== null);
    const old = row.querySelector('.tline');
    if (v.progress === null) old?.remove();
    else if (old) old.querySelector<HTMLElement>('i')!.style.transform = `scaleX(${Math.min(1, v.progress)})`;
    else row.append(bar('tline', v.progress));
  }

  /** Takes the page's tray box and gear; an open tray stays open on its page (the page was built again). */
  mount(el: HTMLElement, gear: HTMLElement): void {
    this.el = el;
    this.bin = el.querySelector('.bin')!;
    this.gear = gear;
    this.frame = null;
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
    this.show(this.sub, 0);
  }

  open(): void {
    const el = this.el;
    if (this.isOpen || !el) return;
    this.isOpen = true;
    this.sub = null;
    if (this.hideTimer) clearTimeout(this.hideTimer);
    this.swapped();
    el.style.transition = 'none';
    Object.assign(el.style, this.gearBox());
    el.hidden = false;
    el.classList.remove('open');
    this.bin.textContent = '';
    this.frame = null;
    this.current = null;
    void el.offsetWidth;
    el.style.transition = '';
    this.place();
    this.show(null, 0);
    el.classList.add('open');
    this.gear.classList.add('hid');
    void this.keys.readAll();
    // General's value says whether CUE starts at sign-in.
    void this.readAutostart();
  }

  close(): void {
    const el = this.el;
    if (!this.isOpen || !el) return;
    this.isOpen = false;
    if (this.sub === 'mic') this.stopPreview();
    this.sub = null;
    this.ctx.closeMenu();
    this.swapped();
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
    if (this.isOpen) this.redraw();
  }

  /** The microphone's level on the meter (only the microphone page has one). */
  level(level: number): void {
    if (this.sub === 'mic') this.mic.level(level);
  }

  stopPreview(): void {
    this.mic.stopPreview();
  }

  /** The window has the focus again: the Microphone page starts its preview anew. The welcome window's Microphone step
   *  may have taken it meanwhile (one preview at a time), and the meter then fell to 0 and would stay there. */
  regained(): void {
    if (this.isOpen && this.sub === 'mic') this.mic.startPreview();
  }

  private fit(): void {
    const frame = this.frame;
    if (!frame || !this.el) return;
    const top = parseFloat(this.el.style.top) || 0;
    const maxH = Math.max(0, this.ctx.app.offsetHeight - top - 14);
    this.bin.style.maxHeight = `${maxH}px`;
    this.el.style.height = `${Math.min(frame.offsetHeight, maxH)}px`;
  }

  private gearBox(): Partial<CSSStyleDeclaration> {
    const g = relRect(this.ctx.app, this.gear);
    return { left: `${g.x}px`, top: `${g.y}px`, width: `${g.w}px`, height: `${g.h}px`, borderRadius: `${g.h / 2}px` };
  }

  /** The heading line — back, the title, close — and the place for the pages' rows under it, drawn anew. */
  private build(): void {
    const t = this.ctx.t;
    this.frame = h('div', 'tpg');
    this.head = h('div', 'tph');
    const back = iconButton('ibt back', 'chev', t.back);
    back.dataset.tback = '';
    this.heading = h('b', 'lbl');
    const x = iconButton('ibt', 'x', t.close);
    x.dataset.tclose = '';
    this.head.append(back, this.heading, x);
    this.pages = h('div', 'tpb');
    this.frame.append(this.head, this.pages);
    this.bin.textContent = '';
    this.bin.append(this.frame);
  }

  /** The heading line for `sub`: the chevron only under the first page (out of reach there), the title changed letter by
   *  letter when `morph`, else at once. */
  private retitle(sub: Sub | null, morph: boolean): void {
    const name = sub ? this.title(sub) : this.ctx.t.settings;
    this.head.classList.toggle('root', !sub);
    const back = this.head.querySelector<HTMLElement>('[data-tback]')!;
    back.tabIndex = sub ? 0 : -1;
    back.setAttribute('aria-hidden', String(!sub));
    if ((this.heading.dataset.t ?? '') === name) return;
    if (morph) {
      CUE().setLabel(this.heading, name);
    } else {
      this.heading.dataset.t = name;
      this.heading.textContent = name;
    }
  }

  /** The page `sub` in place of the one shown: its rows sliding in from the side of `dir` (1 — deeper, -1 — back), at
   *  once with 0, under the heading line that stays. */
  private show(sub: Sub | null, dir: number): void {
    if (!this.frame) this.build();
    const pg = this.page(sub);
    const olds = [...this.pages.children] as HTMLElement[];
    const slide = dir !== 0 && !this.ctx.reduced() && typeof pg.animate === 'function';
    this.retitle(sub, slide);
    if (slide && olds.length) this.swapping();
    this.pages.append(pg);
    this.bin.scrollTop = 0;
    this.current = pg;
    for (const old of olds) {
      Object.assign(old.style, { position: 'absolute', left: '0', right: '0', top: '0' });
      if (!slide) {
        old.remove();
        continue;
      }
      // The rows going fade at once and are nearly gone when the new ones start: the two pages never read over each other.
      old.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${-dir * 24}px)` }], {
        duration: 160,
        easing: 'cubic-bezier(.33,1,.68,1)',
        fill: 'forwards',
      }).onfinish = () => old.remove();
    }
    if (slide && olds.length) {
      pg.animate([{ opacity: 0, transform: `translateX(${dir * 32}px)` }, { opacity: 1, transform: 'none' }], {
        duration: 400,
        delay: 110,
        easing: 'cubic-bezier(.22,1,.36,1)',
        fill: 'backwards',
      });
    }
    pg.querySelectorAll<HTMLElement>('.segc').forEach(syncSeg);
    this.fit();
  }

  /** The page shown drawn again where it is, with its new values: one still sliding in keeps sliding. */
  private redraw(): void {
    const pg = this.current;
    if (!pg) return;
    pg.replaceChildren(...this.page(this.sub).childNodes);
    pg.querySelectorAll<HTMLElement>('.segc').forEach(syncSeg);
    this.fit();
  }

  /** Pages pass: for `SWAP_MS` the tray's height eases without overshooting and its content does not scroll. */
  private swapping(): void {
    if (!this.el) return;
    if (this.swapTimer) clearTimeout(this.swapTimer);
    this.el.classList.add('swap');
    this.swapTimer = setTimeout(() => this.swapped(), SWAP_MS);
  }

  private swapped(): void {
    if (this.swapTimer) clearTimeout(this.swapTimer);
    this.swapTimer = null;
    this.el?.classList.remove('swap');
  }

  private go(sub: Sub): void {
    this.sub = sub;
    this.show(sub, 1);
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
    this.show(null, -1);
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
    else if ((b = t.closest<HTMLElement>('[data-dd="hkeep"]'))) this.keepMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="haudio"]'))) this.audioMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-hclear]'))) this.clearPressed(b);
    else if (t.closest('[data-sw="autostart"]')) void this.toggleAutostart();
    else if ((b = t.closest<HTMLElement>('.segc button')) && b.getAttribute('aria-pressed') !== 'true') {
      void this.ctx.change({ kind: 'uiLanguage', value: b.dataset.v as UiLanguage });
    }
  }

  /* ---------- pages ---------- */

  private page(sub: Sub | null): HTMLElement {
    const t = this.ctx.t;
    // A page drawn anew (another page, the tray opened again, new settings) never takes a first press of "Clear history"
    // with it: its new button would clear on one press.
    this.disarm();
    const pg = h('div');
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
      this.paintModelValue(pg);
      if (this.ctx.settings && !this.ctx.settings.saved) pg.append(h('div', 'tft', t.unsaved));
      return pg;
    }
    const g = h('div', 'sgrp');
    pg.append(g);
    if (sub === 'model') {
      g.append(modelRow(t, this.model), this.keys.row('anthropic'));
      pg.append(h('div', 'tft', t.modelNote));
    } else if (sub === 'mic') g.append(...this.mic.rows());
    else if (sub === 'languages') g.append(...this.languageRows());
    else if (sub === 'history') g.append(...this.historyRows());
    else g.append(...this.generalRows());
    return pg;
  }

  private title(sub: Sub): string {
    const t = this.ctx.t;
    return { model: t.model, mic: t.mic, languages: t.languages, history: t.history, general: t.general }[sub];
  }

  /** A section's value on the first page. */
  private value(sub: Sub): string {
    const t = this.ctx.t;
    const s = this.ctx.settings;
    switch (sub) {
      case 'model':
        // Drawn with its look by `paintModelValue`.
        return '';
      case 'mic':
        return this.mic.label();
      case 'languages':
        if (!s) return '';
        return t.languagesValue.replace('{UI}', t.uiLanguages[s.uiLanguage] ?? '').replace('{TO}', () => {
          const name = this.targetName();
          return this.ctx.lang === 'ru' ? name.toLocaleLowerCase('ru') : name;
        });
      case 'history':
        return s ? this.keepLabel() : '';
      case 'general':
        return this.autostart === null ? '' : this.autostart ? t.generalOn : t.generalOff;
    }
  }

  /** The translation's language as the interface names it. */
  private targetName(): string {
    const s = this.ctx.settings;
    if (!s) return '';
    const english = s.languages.find(([c]) => c === s.translateTarget)?.[1] ?? s.translateTarget;
    return languageName(s.translateTarget, english, this.ctx.lang);
  }

  /* ---------- languages ---------- */

  /** The interface's language, then the one a translation goes into. */
  private languageRows(): HTMLElement[] {
    const t = this.ctx.t;
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
    return [srow(t.uiLanguage, null, seg).row, srow(t.translateTo, null, dropDown('target', this.targetName())).row];
  }

  /* ---------- history ---------- */

  /** How long the history is kept, how long its recordings are, and clearing it. */
  private historyRows(): HTMLElement[] {
    const t = this.ctx.t;
    const clear = h('button', 'btn2 ck warn');
    clear.type = 'button';
    clear.dataset.hclear = '';
    clear.append(h('span', 'lbl', t.clearButton));
    return [
      srow(t.keepHistory, t.keepHistoryHint, dropDown('hkeep', this.keepLabel())).row,
      srow(t.keepAudio, t.keepAudioHint, dropDown('haudio', this.audioLabel())).row,
      srow(t.clear, t.clearHint, clear).row,
    ];
  }

  /** How long the history is kept; unset while the core does not know it. */
  private keepLabel(): string {
    const v = this.ctx.settings?.historyKeep ?? null;
    return v ? this.ctx.t.keeps[v] ?? '' : this.ctx.t.keepUnset;
  }

  private keepMenu(b: HTMLElement): void {
    const t = this.ctx.t;
    const cur = this.ctx.settings?.historyKeep ?? null;
    this.ctx.menu(
      b,
      KEEPS.map((k) => ({ value: k, label: t.keeps[k], on: k === cur })),
      (v) => void this.ctx.change({ kind: 'historyKeep', value: v as Keep }),
      t.keepNote,
    );
  }

  /** "Clear history" works on the second press within 3 s, as "Forget all" in the design. */
  private clearPressed(b: HTMLElement): void {
    const t = this.ctx.t;
    const l = b.querySelector<HTMLElement>('.lbl')!;
    if (!this.clearTimer) {
      this.clearTimer = setTimeout(() => {
        this.clearTimer = null;
        if (l.isConnected) CUE().setLabel(l, t.clearButton);
      }, CLEAR_ARMED_MS);
      CUE().setLabel(l, t.clearAgain);
      return;
    }
    this.disarm();
    CUE().setLabel(l, t.clearButton);
    void this.clearHistory();
  }

  private disarm(): void {
    if (this.clearTimer) clearTimeout(this.clearTimer);
    this.clearTimer = null;
  }

  /** How long recordings are kept; unset while the core does not know it. */
  private audioLabel(): string {
    const v = this.ctx.settings?.historyAudio ?? null;
    return v ? this.ctx.t.audioKeeps[v] ?? '' : this.ctx.t.keepUnset;
  }

  private audioMenu(b: HTMLElement): void {
    const t = this.ctx.t;
    const cur = this.ctx.settings?.historyAudio ?? null;
    this.ctx.menu(
      b,
      AUDIO_KEEPS.map((k) => ({ value: k, label: t.audioKeeps[k], on: k === cur })),
      (v) => void this.ctx.change({ kind: 'historyAudio', value: v as AudioKeep }),
      t.audioNote,
    );
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
    return [srow(t.autostart, null, sw).row];
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
    const em = this.current?.querySelector('em[data-val="general"]');
    if (em) em.textContent = this.value('general');
    const sw = this.current?.querySelector<HTMLButtonElement>('[data-sw="autostart"]');
    if (!sw) return;
    sw.setAttribute('aria-checked', String(!!this.autostart));
    sw.disabled = this.autostart === null;
  }
}
