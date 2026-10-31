// The welcome window's steps in order, and what each one shows (spec 4a §1; the approved design's welcome window, stage
// 6). The page (page.ts) moves between them and does what they do; Keys and Microphone show the settings tray's own
// rows (window/keys.ts, window/mic.ts). Texts go in as text.
import { CUE } from '../designParts';
import type { LinkTarget } from '../window/api';
import { icon } from '../window/icons';
import { KEYS, type KeyRows } from '../window/keys';
import type { MicRows } from '../window/mic';
import { h } from '../window/page';
import type { Lang } from '../window/text';
import { doneTitle, type WelcomeText } from './text';

export type Step = 'hello' | 'keys' | 'mic' | 'try' | 'done';
export const STEPS: readonly Step[] = ['hello', 'keys', 'mic', 'try', 'done'];

/** The Windows key's logo, as on the Drop's key hint. */
const WIN_LOGO =
  '<svg viewBox="0 0 10 10" fill="currentColor" aria-hidden="true"><rect x="0" y="0" width="4.4" height="4.4"/><rect x="5.6" y="0" width="4.4" height="4.4"/><rect x="0" y="5.6" width="4.4" height="4.4"/><rect x="5.6" y="5.6" width="4.4" height="4.4"/></svg>';

/** "Next" on Keys: both keys saved, neither refused by its check, being checked or found missing by it, and no key typed
 *  and not checked yet (Next would drop it; Check saves it). */
export function keysReady(rows: KeyRows): boolean {
  return !rows.typed() && KEYS.every((k) => {
    const state = rows.state(k);
    return rows.keys[k].saved && state !== 'badKey' && state !== 'checking' && state !== 'missing';
  });
}

/** The words of `text`: runs of letters and digits, with an apostrophe or a hyphen inside a word kept in it. */
export function wordCount(text: string): number {
  return text.match(/[\p{L}\p{N}]+(?:['’-][\p{L}\p{N}]+)*/gu)?.length ?? 0;
}

/** What a step's page takes from the welcome page. */
export interface StepParts {
  readonly text: WelcomeText;
  readonly lang: Lang;
  readonly keys: KeyRows;
  readonly mic: MicRows;
  /** The words of the first dictation. */
  readonly words: number;
}

/** The keys of the dictation chord, then `more` (Shift, Alt): Bebas keycaps, the Windows key as its logo. */
function keycaps(cls: string, ...more: string[]): HTMLElement {
  const keys = h('span', `keys ${cls}`);
  keys.setAttribute('role', 'img');
  keys.setAttribute('aria-label', ['Ctrl', 'Win', ...more].join(' + '));
  const win = h('span', 'kk w');
  win.innerHTML = WIN_LOGO;
  keys.append(h('span', 'kk', 'Ctrl'), h('i', '', '+'), win);
  for (const k of more) keys.append(h('i', '', '+'), h('span', 'kk', k));
  return keys;
}

/** The big live mark (the page brings it to life). */
function bigMark(): HTMLElement {
  const m = h('span', 'bigmk');
  m.innerHTML = '<svg viewBox="0 0 24 24" aria-hidden="true"><g></g></svg>';
  return m;
}

function link(target: LinkTarget, label: string, cls = 'oa'): HTMLButtonElement {
  const b = h('button', cls, label);
  b.type = 'button';
  b.dataset.link = target;
  return b;
}

/** A tip on Done: its icon (`ask` — in the Ask color), its text, its keys. */
function tip(svg: string, label: string, keys?: HTMLElement, ask = false): HTMLElement {
  const row = h('div');
  const ic = h('span', ask ? 'ic ask' : 'ic');
  ic.innerHTML = svg;
  row.append(ic, h('span', 'otl', label));
  if (keys) row.append(keys);
  return row;
}

/** A step's page, as it is when the step starts. */
export function stepPage(step: Step, p: StepParts): HTMLElement {
  const x = p.text;
  const pg = h('div', 'pgv');
  pg.dataset.step = step;
  switch (step) {
    case 'hello':
      pg.append(bigMark(), h('div', 'oname', 'CUE'), h('div', 'ohd', x.helloTitle), h('div', 'op', x.helloText));
      break;
    case 'keys': {
      const rows = h('div', 'sgrp orows');
      rows.append(...KEYS.map((k) => p.keys.row(k)));
      const links = h('div', 'olinks');
      links.append(`${x.keysWhere} `, link('soniox', 'Soniox Console'), ' · ', link('anthropic', 'Anthropic Console'));
      pg.append(h('div', 'ohd', x.steps.keys), h('div', 'op', x.keysText), rows, links, h('div', 'onote', x.price));
      pg.append(h('div', 'onote', x.privacy));
      break;
    }
    case 'mic': {
      const ill = h('span', 'oill');
      ill.innerHTML = icon('micOn');
      const rows = h('div', 'sgrp orows');
      rows.append(...p.mic.rows());
      const hint = h('div', 'ohint');
      hint.hidden = true;
      hint.append(h('span', '', x.micBlocked), link('micPrivacy', x.micSettings, 'btn2'));
      pg.append(ill, h('div', 'ohd', x.steps.mic), h('div', 'op', x.micText), rows, hint);
      break;
    }
    case 'try': {
      const keys = h('div', 'tryk');
      keys.append(keycaps('big wait'));
      const phrase = h('span', 'phr', `${x.trySay} `);
      phrase.append(h('b', '', x.tryPhrase));
      const field = h('textarea', 'ofield');
      field.dataset.field = '';
      field.placeholder = x.tryField;
      field.spellcheck = false;
      field.setAttribute('aria-label', x.steps.try);
      const box = h('div', 'tryf');
      box.append(phrase, field);
      pg.append(h('div', 'ohd', x.steps.try), h('div', 'op', x.tryText), keys, box);
      break;
    }
    case 'done': {
      const tips = h('div', 'otips');
      const where = CUE().SV('<path d="M7 4h2.5M7 16h2.5M8.3 4v12"/><path d="M12.5 8.5h4M12.5 11.5h3"/>');
      tips.append(
        tip(icon('tr'), x.doneTranslate, keycaps('sm', 'Shift')),
        tip(icon('ask'), x.doneAsk, keycaps('sm', 'Alt'), true),
        tip(where, x.doneWhere),
      );
      pg.append(bigMark(), h('div', 'ohd', doneTitle(x, p.words, p.lang)), h('div', 'op', x.doneText), tips);
      break;
    }
  }
  return pg;
}
