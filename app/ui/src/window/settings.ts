// The settings tray (spec 3 §3, regrouped in docs/specs/2026-11-02-settings-tray.md): it grows out of the gear (the gear
// turns into its close button) and folds back into it. Its first page lists Speech recognition (the model on this
// computer, window/model.ts), Cleanup (the style, the connection and its key or address, the model), Microphone, Languages (the
// interface's and the translation's), History (how long the history and its recordings are kept, and clearing it) and
// General with their values; one of them at a time slides in, the chevron goes back; Esc goes back, then closes. Every
// change goes to the core at once.
// Going from page to page, the heading line stays: its title changes letter by letter and the chevron grows in front
// of it; only the rows below slide. The height eases to the new page without overshooting, and nothing scrolls while
// the pages pass (a scroll bar showing for a moment would narrow the rows). A page whose values come later (the
// microphones, new settings) is filled in where it is, even while it is still sliding in.
// Speech recognition's model row opens the Model page (window/models.ts), one level deeper: back goes to Speech
// recognition. Cleanup holds the key row of keys.ts, the Microphone page the microphone rows of mic.ts (the welcome window
// shows them too); the microphone's level is listened to only while that page is shown.
import * as api from './api';
import {
  AUDIO_KEEPS,
  KEEPS,
  REFINE_STYLES,
  type AudioKeep,
  type Keep,
  type KeyName,
  type RefineStyle,
  type UiLanguage,
} from './api';
import { CUE } from '../designParts';
import { icon, type IconName } from './icons';
import { KEY_TITLE, KeyRows } from './keys';
import { MicRows } from './mic';
import { bar, modelRow, modelValue, stepModelRow } from './model';
import { ModelsPage } from './models';
import { dropDown, h, iconButton, logFailed, relRect, srow, wrap, type Ctx } from './page';
import { languageName } from './text';

export { matchMic } from './mic';

type Sub = 'model' | 'models' | 'cleanup' | 'mic' | 'languages' | 'history' | 'general';
const SUBS: readonly Exclude<Sub, 'models'>[] = ['model', 'cleanup', 'mic', 'languages', 'history', 'general'];
/** The page a page goes back to: the Model page is Speech recognition's; the rest are the first page's. */
const PARENT: Partial<Record<Sub, Sub>> = { models: 'model' };
const SUB_ICON: Record<Exclude<Sub, 'models'>, IconName> = {
  model: 'wave',
  cleanup: 'sparkle',
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

/** The longest custom instructions the core keeps (`settings::MAX_CUSTOM_CHARS`). */
const CUSTOM_CHARS = 2000;

/** Dollars per million tokens as the model list shows them: whole numbers bare, cents always two places ("0.10", "2",
 *  "0.075"). */
function usd(n: number): string {
  if (Number.isInteger(n)) return String(n);
  const places = (String(n).split('.')[1] ?? '').length;
  return n.toFixed(Math.max(2, places));
}

/** A segmented control of `values` named by `labels`, `cur` pressed; `name` tells the click which setting it is. */
function segment<V extends string>(name: string, values: readonly V[], labels: Record<V, string>, cur: V | undefined): HTMLElement {
  const seg = h('span', 'segc');
  seg.dataset.seg = name;
  seg.append(h('span', 'thb'));
  for (const v of values) {
    const b = h('button', '', labels[v]);
    b.type = 'button';
    b.dataset.v = v;
    b.setAttribute('aria-pressed', String(v === cur));
    seg.append(b);
  }
  return seg;
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
  private readonly models: ModelsPage;
  private autostart: boolean | null = null;
  /** The speech model's state, as `cue://model` last said (null — not heard yet). */
  private model: api.ModelState | null = null;
  /** The last check of each local server: it answered, it did not, or it is being asked. */
  private readonly servers: Record<string, boolean | 'checking'> = {};
  /** "Clear history" was pressed once and waits for the second press. */
  private clearTimer: ReturnType<typeof setTimeout> | null = null;

  /** `clearHistory` — the history's own clearing (the list empties with it). */
  constructor(
    private readonly ctx: Ctx,
    private readonly clearHistory: () => Promise<void>,
  ) {
    // The key's state shows in Cleanup's value ("No Anthropic key") and on its page, on the card too.
    this.keys = new KeyRows(ctx, () => this.current, () => {
      this.paintCleanupValue();
      this.paintCard();
      this.fit();
    });
    this.mic = new MicRows(ctx, () => this.current, () => {
      if (this.sub === 'mic') this.redraw();
    });
    this.models = new ModelsPage(ctx, () => this.fit());
  }

  /** The Model page's list and the model in use: the page shown follows it, and Speech recognition names the model. */
  setModels(view: api.ModelsView): void {
    this.models.set(view);
    if (this.sub !== 'model') return;
    const row = this.current?.querySelector('.mrow');
    if (row && !stepModelRow(row, this.model ?? { state: 'missing' })) row.replaceWith(this.modelRow());
    else row?.querySelector('.stx b')?.replaceChildren(this.models.currentName() ?? this.ctx.t.modelName);
  }

  /** Speech recognition's row: the model in use with its state, opening the Model page. */
  private modelRow(): HTMLElement {
    const row = modelRow(this.ctx.t, this.model, this.models.currentName() ?? undefined);
    row.dataset.sub = 'models';
    row.tabIndex = 0;
    row.setAttribute('role', 'button');
    row.querySelector('.sc')!.insertAdjacentHTML('beforeend', icon('chev'));
    return row;
  }

  /** The speech model's new state: its value on the first page; on its page a download goes on in place, any other
   *  change draws the model's row again. */
  setModel(state: api.ModelState): void {
    this.model = state;
    this.paintModelValue();
    if (this.sub !== 'model') return;
    const row = this.current?.querySelector('.mrow');
    if (row && !stepModelRow(row, state)) row.replaceWith(this.modelRow());
    this.fit();
  }

  /** The first page's Speech recognition value: its text, amber or Bebas, and the line under the row while the model
   *  downloads. */
  private paintModelValue(page: ParentNode | null = this.current): void {
    const row = page?.querySelector<HTMLElement>('[data-sub="model"]');
    if (!row) return;
    const v = modelValue(this.ctx.t, this.model);
    const em = row.querySelector('em')!;
    em.textContent = v.text;
    em.classList.toggle('warn', v.warn);
    em.classList.toggle('num', v.progress !== null);
    const old = row.querySelector('.tline');
    if (v.progress === null) old?.remove();
    else if (old) old.querySelector<HTMLElement>('i')!.style.transform = `scaleX(${Math.min(1, v.progress)})`;
    else row.append(bar('tline', v.progress));
  }

  /** The first page's Cleanup value: the style, or amber "No Anthropic key" once the keys are read and it is not saved. */
  private paintCleanupValue(page: ParentNode | null = this.current): void {
    const em = page?.querySelector<HTMLElement>('em[data-val="cleanup"]');
    if (!em) return;
    const key = this.cleanupKey();
    const noKey = !!key && this.keys.read && !this.keys.keys[key].saved;
    em.textContent = noKey && key ? this.ctx.t.cleanupNoKey.replace('{NAME}', KEY_TITLE[key]) : this.value('cleanup');
    em.classList.toggle('warn', noKey);
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
      // The model row is a button of its own: Enter or Space opens the Model page.
      if ((e.key === 'Enter' || e.key === ' ') && (e.target as Element).matches('.mrow[data-sub]')) {
        e.preventDefault();
        this.go('models');
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
    if (this.sub === 'models') this.models.stop();
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
    this.frame = wrap('tpg');
    this.head = wrap('tph');
    const back = iconButton('ibt back', 'chev', t.back);
    back.dataset.tback = '';
    this.heading = h('b', 'lbl');
    const x = iconButton('ibt', 'x', t.close);
    x.dataset.tclose = '';
    this.head.append(back, this.heading, x);
    this.pages = wrap('tpb');
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
    if (this.sub === 'models') this.models.stop();
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
    if (this.sub === 'models') this.models.stop();
    this.ctx.closeMenu();
    const parent = this.sub ? PARENT[this.sub] ?? null : null;
    this.sub = parent;
    this.show(parent, -1);
  }

  private onClick(e: Event): void {
    const t = e.target as Element;
    let b: HTMLElement | null;
    // Retry on the model row is the row's own: it does not open the Model page.
    if ((b = t.closest<HTMLElement>('[data-sub]')) && !t.closest('[data-model-retry]')) this.go(b.dataset.sub as Sub);
    else if (t.closest('[data-tback]')) this.back();
    else if (t.closest('[data-tclose]')) this.close();
    else if ((b = t.closest<HTMLElement>('[data-check]'))) void this.keys.check(b.dataset.check as KeyName);
    else if ((b = t.closest<HTMLElement>('[data-dd="mic"]'))) this.mic.menu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="connection"]'))) this.connectionMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="rmodel"]'))) this.modelMenu(b);
    else if (t.closest('[data-server-check]')) void this.serverCheck();
    else if ((b = t.closest<HTMLElement>('[data-dd="target"]'))) this.targetMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="hkeep"]'))) this.keepMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-dd="haudio"]'))) this.audioMenu(b);
    else if ((b = t.closest<HTMLElement>('[data-hclear]'))) this.clearPressed(b);
    else if (t.closest('[data-sw="autostart"]')) void this.toggleAutostart();
    else if ((b = t.closest<HTMLElement>('.segc button')) && b.getAttribute('aria-pressed') !== 'true') {
      const v = b.dataset.v!;
      const seg = b.closest<HTMLElement>('.segc')!.dataset.seg;
      if (seg === 'refine') void this.ctx.change({ kind: 'refineStyle', value: v as RefineStyle });
      else void this.ctx.change({ kind: 'uiLanguage', value: v as UiLanguage });
    }
  }

  /* ---------- pages ---------- */

  private page(sub: Sub | null): HTMLElement {
    const t = this.ctx.t;
    // A page drawn anew (another page, the tray opened again, new settings) never takes a first press of "Clear history"
    // with it: its new button would clear on one press.
    this.disarm();
    // A screen reader hears the page by its title.
    const pg = h('div');
    pg.setAttribute('role', 'group');
    pg.setAttribute('aria-label', sub ? this.title(sub) : t.settings);
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
      this.paintCleanupValue(pg);
      if (this.ctx.settings && !this.ctx.settings.saved) pg.append(h('div', 'tft', t.unsaved));
      return pg;
    }
    if (sub === 'models') {
      pg.append(...this.models.rows());
      return pg;
    }
    const g = wrap('sgrp');
    pg.append(g);
    if (sub === 'model') {
      g.append(this.modelRow());
      pg.append(h('div', 'tft', t.modelNote));
    } else if (sub === 'cleanup') {
      g.append(...this.cleanupRows());
      const c = this.connection();
      pg.append(h('div', 'tft', c?.local ? t.cleanupNoteLocal : t.cleanupNote.replace('{NAME}', c?.name ?? '')));
    } else if (sub === 'mic') g.append(...this.mic.rows());
    else if (sub === 'languages') g.append(...this.languageRows());
    else if (sub === 'history') g.append(...this.historyRows());
    else g.append(...this.generalRows());
    return pg;
  }

  private title(sub: Sub): string {
    const t = this.ctx.t;
    return { model: t.model, models: t.models.title, cleanup: t.cleanup, mic: t.mic, languages: t.languages, history: t.history, general: t.general }[sub];
  }

  /** A section's value on the first page. */
  private value(sub: Exclude<Sub, 'models'>): string {
    const t = this.ctx.t;
    const s = this.ctx.settings;
    switch (sub) {
      case 'model':
        // Drawn with its look by `paintModelValue`.
        return '';
      case 'cleanup':
        // The key's warning over it is `paintCleanupValue`'s.
        return s ? t.refineStyles[s.refineStyle] ?? '' : '';
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
    const seg = segment('ui', UI_LANGUAGES, t.uiLanguages, this.ctx.settings?.uiLanguage);
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

  /* ---------- cleanup ---------- */

  /** The style (and the custom instructions' field), the connection card, the connection's key or address, its model
   *  (docs/specs/2026-11-02-refine-connections.md). */
  private cleanupRows(): HTMLElement[] {
    const t = this.ctx.t;
    const s = this.ctx.settings;
    const custom = s?.refineStyle === 'custom';
    const style = segment('refine', REFINE_STYLES, t.refineStyles, s?.refineStyle);
    const rows = [srow(t.refineStyle, custom ? t.refineCustomHint : t.refineStyleHint, style).row];
    if (custom) rows.push(this.customRow());
    const c = this.connection();
    if (!c) return rows;
    const card = h('div', 'srow wide');
    card.append(this.card(c));
    rows.push(card, c.local ? this.addressRow(c) : this.keys.row(c.id as KeyName, t.keyRow));
    const model = c.models.find((m) => m.id === s?.refineModel) ?? c.models[0];
    rows.push(srow(t.refineModel, null, dropDown('rmodel', model?.name ?? '')).row);
    return rows;
  }

  /** The custom style's instructions: saved when the field loses focus. */
  private customRow(): HTMLElement {
    const field = h('textarea', 'cta');
    field.placeholder = this.ctx.t.refineCustomPlaceholder;
    field.maxLength = CUSTOM_CHARS;
    field.value = this.ctx.settings?.refineCustom ?? '';
    field.setAttribute('aria-label', this.ctx.t.refineStyles.custom);
    field.addEventListener('change', () => void this.ctx.change({ kind: 'refineCustom', value: field.value }));
    const row = h('div', 'srow wide');
    row.append(field);
    return row;
  }

  /** The connection card: a lamp (ready or not), the name, its state under it; it opens the list of connections. */
  private card(c: api.ConnectionView): HTMLElement {
    const b = h('button', 'pk');
    b.type = 'button';
    b.dataset.dd = 'connection';
    b.setAttribute('aria-label', this.ctx.t.refineConnection);
    const pt = h('span', 'pt');
    pt.append(h('b', '', c.name), h('small'));
    b.append(h('span', 'lamp'), pt);
    b.insertAdjacentHTML('beforeend', icon('chev'));
    this.paintCard(b, c);
    return b;
  }

  /** The card's lamp and state, as they are now (a key saved or checked, a server answered). */
  private paintCard(b: HTMLElement | null | undefined = this.current?.querySelector<HTMLElement>('.pk'), c = this.connection()): void {
    if (!b || !c) return;
    const { text, on } = this.readiness(c);
    b.querySelector('.pt small')!.textContent = text;
    b.querySelector('.lamp')!.classList.toggle('on', on);
  }

  /** A connection's state in words, and whether it is ready: a key saved, or a local server that answered. */
  private readiness(c: api.ConnectionView): { text: string; on: boolean } {
    const t = this.ctx.t;
    if (c.local) {
      const server = this.servers[c.id];
      const text = server === 'checking' ? t.server.checking : server === true ? t.server.running : server === false ? t.server.down : t.server.unknown;
      return { text, on: server === true };
    }
    const key = this.keys.keys[c.id as KeyName];
    return key?.saved ? { text: t.keySavedMask.replace('{MASK}', key.mask), on: true } : { text: t.keyStates.missing, on: false };
  }

  /** A local server's address and "Check". */
  private addressRow(c: api.ConnectionView): HTMLElement {
    const t = this.ctx.t;
    const input = h('input', 'kin');
    input.type = 'text';
    input.spellcheck = false;
    input.dataset.address = c.id;
    input.value = this.ctx.settings?.refineAddress ?? '';
    input.setAttribute('aria-label', t.addressRow);
    const btn = h('button', 'btn2 ck', t.check);
    btn.type = 'button';
    btn.dataset.serverCheck = c.id;
    const { row } = srow(t.addressRow, null, input, btn);
    row.dataset.server = c.id;
    return row;
  }

  /** Asks the local server at the address typed whether it answers; a new address is kept once checked. */
  private async serverCheck(): Promise<void> {
    const c = this.connection();
    const address = this.current?.querySelector<HTMLInputElement>('input[data-address]')?.value.trim() ?? '';
    if (!c?.local || !address || this.servers[c.id] === 'checking') return;
    this.servers[c.id] = 'checking';
    this.paintCard();
    let running = false;
    try {
      running = await api.serverCheck(address);
    } catch (e) {
      logFailed('checking the local server', e);
    }
    this.servers[c.id] = running;
    if (address !== this.ctx.settings?.refineAddress) await this.ctx.change({ kind: 'refineAddress', value: address });
    this.paintCard();
  }

  /** Every connection, cloud first, each with its lamp and state; the one chosen is checked. */
  private connectionMenu(b: HTMLElement): void {
    const t = this.ctx.t;
    const s = this.ctx.settings;
    if (!s) return;
    const items = s.connections.map((c) => {
      const { text, on } = this.readiness(c);
      const group = c.local ? t.connectionGroups.local : t.connectionGroups.cloud;
      return { value: c.id, label: c.name, sub: text, lamp: on, group, on: c.id === s.refineConnection };
    });
    this.ctx.menu(b, items, (v) => void this.ctx.change({ kind: 'refineConnection', value: v }), undefined, 'start', true);
  }

  /** The connection's models, each with its price or size, the recommended one marked. */
  private modelMenu(b: HTMLElement): void {
    const t = this.ctx.t;
    const c = this.connection();
    if (!c) return;
    const cur = c.models.find((m) => m.id === this.ctx.settings?.refineModel) ?? c.models[0];
    const items = c.models.map((m, i) => ({
      value: m.id,
      label: m.name,
      sub: m.usd
        ? t.perMtok.replace('{IN}', usd(m.usd[0])).replace('{OUT}', usd(m.usd[1]))
        : t.onDisk.replace('{GB}', (m.gb ?? 0).toLocaleString(this.ctx.lang)),
      badge: i === 0 ? t.recommended : undefined,
      on: m === cur,
    }));
    this.ctx.menu(b, items, (v) => void this.ctx.change({ kind: 'refineModel', value: v }));
  }

  /** The cleanup's connection as the core lists it. */
  private connection(): api.ConnectionView | undefined {
    const s = this.ctx.settings;
    return s?.connections.find((c) => c.id === s.refineConnection);
  }

  /** The key the cleanup needs: the chosen cloud connection's; none for a local server. */
  private cleanupKey(): KeyName | null {
    const c = this.connection();
    return c && !c.local ? (c.id as KeyName) : null;
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
