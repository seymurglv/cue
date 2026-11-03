import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { DRAG_THRESHOLD, DropController, PLACES, pickPlace, type UiState, type UiAction, type Rect } from '../src/controller';
import { trayHtml } from '../src/trays';
import type { Answer } from '../src/answer';
import { DICTS, KEY_NAME, TRAYS, en, pickLang, ru, type TrayKind } from '../src/strings';

const base: UiState = { phase: 'idle', error: null, retry: false, paused: false, mode: 'dictate' };
let sent: UiAction[];
let rectsSent: Rect[][];
let wakes: number;
let ctl: DropController;
const on = () => [...document.querySelectorAll('.drop .pt.on')].map((e) => e.className);

/** Visible Drop parts by their names in drop.js: bx, mk, mi, chip, why, lab, act, undo, retry, kc, ok. */
const BY_TITLE: Record<string, string> = { [en.drop.cancel]: 'bx', [en.drop.restore]: 'undo', [en.drop.retry]: 'retry', [en.drop.done]: 'ok' };
const shown = () =>
  [...document.querySelectorAll('.drop .pt.on')]
    .map((e) => BY_TITLE[e.getAttribute('title') ?? ''] ?? (e.classList[0] === 'mkw' ? 'mk' : e.classList[0]))
    .sort();
const hover = (inside: boolean) => document.querySelector('.drop')!.dispatchEvent(new MouseEvent(inside ? 'mouseenter' : 'mouseleave'));
const CUE = () => (window as any).CUE;
const KINDS: TrayKind[] = ['noField', 'micSilent', 'holdHint', 'lastMinute', 'micMissing', 'modelReady', 'modelSwitched', 'newHabit', 'habitChanged'];

beforeEach(() => {
  document.body.innerHTML = '<div id="stage"></div>';
  document.body.className = '';
  sent = [];
  rectsSent = [];
  wakes = 0;
  ctl = new DropController(document.getElementById('stage')!, {
    send: (a) => sent.push(a),
    rects: (r) => rectsSent.push(r),
    wake: () => wakes++,
  });
});

describe('Dictionaries and language', () => {
  /** Every path to a leaf ("tray.noField.title") with its value. */
  const leaves = (o: unknown, at = ''): [string, unknown][] =>
    o !== null && typeof o === 'object'
      ? Object.entries(o).flatMap(([k, v]) => leaves(v, at ? `${at}.${k}` : k))
      : [[at, o]];

  it('English and Russian have exactly the same keys', () => {
    expect(leaves(ru).map(([k]) => k).sort()).toEqual(leaves(en).map(([k]) => k).sort());
  });

  it('every text in both dictionaries is a non-empty string', () => {
    for (const lang of ['en', 'ru'] as const) {
      for (const [k, v] of leaves(DICTS[lang])) {
        expect(typeof v, `${lang}.${k}`).toBe('string');
        expect((v as string).trim(), `${lang}.${k}`).not.toBe('');
      }
    }
  });

  it('a button label exactly where the card has a button; "{KEY}" only in the hint', () => {
    for (const lang of ['en', 'ru'] as const) {
      for (const kind of KINDS) {
        const text = DICTS[lang].tray[kind];
        expect(text.act != null, `${lang}.${kind}`).toBe(TRAYS[kind].act != null);
        expect(text.body.includes('{KEY}'), `${lang}.${kind}`).toBe(kind === 'holdHint');
      }
    }
  });

  it('English is the approved text', () => {
    expect(en.tray.noField).toEqual({
      title: 'Nowhere to paste',
      body: "The cursor isn't in a text field. Your text is ready — copy it and paste it yourself.",
      act: 'Copy', done: 'Copied',
    });
    expect(en.tray.micSilent.title).toBe('Microphone is silent');
    expect(en.tray.holdHint.act).toBe('Got it');
    expect(en.tray.lastMinute.title).toBe('One minute left');
    expect(en.error).toEqual({
      model: 'Model did not download', cpu: 'Processor not supported', stt: "Couldn't recognize",
      claudeKey: 'Claude key problem', translate: "Couldn't translate",
    });
    expect(en.close).toBe('Close');
    expect(en.drop).toEqual({
      cancel: 'Cancel · Esc', cancelName: 'Cancel', restore: 'Restore', retry: 'Retry', done: 'Done', cancelled: 'Cancelled', ask: 'Ask',
      learnAdd: 'Add to dictionary', learnUndo: 'Undo',
    });
  });

  it('language from the core: "ru" is Russian, anything else is English', () => {
    expect(pickLang('ru')).toBe('ru');
    expect(pickLang('en')).toBe('en');
    for (const v of [undefined, null, '', 'RU', 'ru-RU', 'uk', 'de', 1, {}]) expect(pickLang(v), String(v)).toBe('en');
  });

  it('the controller is English by default', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    hover(true);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Model did not download');
    expect(document.querySelector('.drop .ib.soft')!.getAttribute('title')).toBe('Cancel · Esc');
    expect(document.querySelector('.drop .ib.soft')!.getAttribute('aria-label')).toBe('Cancel');
  });

  it('answer texts are the approved ones', () => {
    expect([ru.answer.searching, ru.answer.reading, ru.answer.writing]).toEqual(['Ищу в интернете', 'Читаю источники', 'Пишу ответ']);
    expect([ru.answer.below, ru.answer.replace, ru.answer.listening]).toEqual(['Вставить ниже', 'Заменить', 'Слушаю уточнение…']);
    expect([ru.answer.refused, ru.answer.missed]).toEqual(['Не могу ответить на это', 'Не расслышал уточнение']);
    for (const lang of ['en', 'ru'] as const) expect(DICTS[lang].answer.about).toContain('{Q}');
  });
});

describe('The Drop follows the core', () => {
  it('recording: the mark only — no timer, also on hover', () => {
    ctl.apply({ ...base, phase: 'listen' });
    ctl.frame(performance.now());
    expect(document.querySelector('.drop')!.getAttribute('data-s')).toBe('listen');
    expect(on().some((c) => c.includes('mkw'))).toBe(true);
    expect(document.querySelector('.drop .t')).toBeNull();
    expect(document.querySelector('.drop')!.textContent).not.toMatch(/\d:\d\d/);
    hover(true);
    ctl.frame(performance.now());
    expect(document.querySelector('.drop')!.textContent).not.toMatch(/\d:\d\d/);
  });

  it('Claude key error — no "retry" and no cause icon', () => {
    ctl.apply({ ...base, phase: 'error', error: 'claudeKey', retry: false });
    const visible = on().join(' ');
    expect(visible).not.toContain('why');
    expect(document.querySelector('.drop .ib.solid[title="Retry"]')!.classList.contains('on')).toBe(false);
  });

  it('network error — "retry" sends retry, the cross sends dismiss', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    (document.querySelector('.drop .ib.solid[title="Retry"]') as HTMLButtonElement).click();
    (document.querySelector('.drop .ib.soft') as HTMLButtonElement).click();
    expect(sent).toEqual(['retry', 'dismiss']);
  });

  it('cancelled — "restore" sends undo; recording — the cross sends cancel, the check sends finish', () => {
    ctl.apply({ ...base, phase: 'cancel' });
    (document.querySelector('.drop .ib.solid[title="Restore"]') as HTMLButtonElement).click();
    ctl.apply({ ...base, phase: 'listen' });
    (document.querySelector('.drop .ib.soft') as HTMLButtonElement).click();
    (document.querySelector('.drop .ib.solid[title="Done"]') as HTMLButtonElement).click();
    expect(sent).toEqual(['undo', 'cancel', 'finish']);
  });

  it('empty text — the mark only', () => {
    ctl.apply({ ...base, phase: 'empty' });
    expect(on().length).toBe(1);
    expect(shown()).toEqual(['mk']);
  });
});

describe('Which Drop parts are visible in each state', () => {
  it('idle: nothing without hover; on hover the mark and the keys', () => {
    ctl.apply(base);
    expect(document.querySelector('.drop')!.getAttribute('data-s')).toBe('idle');
    expect(shown()).toEqual([]);
    hover(true);
    expect(shown()).toEqual(['kc', 'mk']);
    hover(false);
    expect(shown()).toEqual([]);
  });

  it('idle hint: two keys — "Ctrl" and the Windows logo, "Ctrl + Win" for screen readers', () => {
    ctl.apply(base);
    hover(true);
    const hint = document.querySelector('.drop .kc > span')!;
    expect(hint.getAttribute('role')).toBe('img');
    expect(hint.getAttribute('aria-label')).toBe(KEY_NAME);
    const keys = [...hint.children];
    expect(keys.map((k) => k.className)).toEqual(['key', 'key w']);
    expect(keys[0].textContent).toBe('Ctrl');
    const win = keys[1].querySelector('svg')!;
    expect(win.getAttribute('aria-hidden')).toBe('true');
    expect(win.querySelectorAll('rect').length).toBe(4);
    expect(keys[1].textContent).toBe('');
  });

  it('idle hint on mac: one "fn" key', () => {
    document.body.innerHTML = '';
    const api = { mode: () => 'dictate', err: () => 'net', lbl: () => 'icons', os: () => 'mac', text: en.drop, voice: new (CUE().Voice)(), onEsc() {}, onPress() {}, onAction() {} };
    const drop = new (CUE().Drop)(document.body, api);
    drop.p.dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop.parts.kc.classList.contains('on')).toBe(true);
    const hint = drop.parts.kc.firstElementChild!;
    expect(hint.getAttribute('aria-label')).toBe('fn');
    expect([...hint.children].map((k) => [k.className, k.textContent])).toEqual([['key', 'fn']]);
  });

  it('thinking: three dots centered on the 24 × 24 mark grid (center 12, like the circle)', () => {
    const g = document.createElementNS('http://www.w3.org/2000/svg', 'g');
    new (CUE().Morph)(g, { state: 'think' });
    const xs = [...g.querySelectorAll('.m-l')].slice(0, 3).map((l) => Number(l.getAttribute('x1')));
    expect(xs).toEqual([7.5, 12, 16.5]);
    expect((Math.min(...xs) + Math.max(...xs)) / 2).toBe(12);
  });

  it.each(['start', 'think', 'done', 'empty'] as const)('%s — the mark only, on hover too', (phase) => {
    ctl.apply({ ...base, phase });
    expect(document.querySelector('.drop')!.getAttribute('data-s')).toBe(phase);
    expect(shown()).toEqual(['mk']);
    hover(true);
    expect(shown()).toEqual(['mk']);
  });

  it('recording: the mark; on hover also the cross and the check', () => {
    ctl.apply({ ...base, phase: 'listen' });
    expect(shown()).toEqual(['mk']);
    hover(true);
    expect(shown()).toEqual(['bx', 'mk', 'ok']);
    hover(false);
    expect(shown()).toEqual(['mk']);
  });

  it('cancelled: mark and "restore"; on hover the "Cancelled" label', () => {
    ctl.apply({ ...base, phase: 'cancel' });
    expect(shown()).toEqual(['mk', 'undo']);
    hover(true);
    expect(shown()).toEqual(['lab', 'mk', 'undo']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Cancelled');
  });

  it('network error: mark, cause icon, "retry"; on hover the cross and "No connection"', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Model did not download');
  });

  it('a failed recognition: no cause icon, with "retry"', () => {
    ctl.apply({ ...base, phase: 'error', error: 'stt', retry: true });
    expect(shown()).toEqual(['mk', 'retry']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk', 'retry']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe("Couldn't recognize");
  });

  it('an unsupported processor: no cause icon and no "retry"', () => {
    ctl.apply({ ...base, phase: 'error', error: 'cpu', retry: false });
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Processor not supported');
  });

  it('a failed translation shows its own label and retry, with the network icon', () => {
    ctl.apply({ ...base, phase: 'error', error: 'translate', retry: true, mode: 'translate' });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.net;
    expect(document.querySelector('.drop .why')!.innerHTML).toBe(icon.innerHTML);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe("Couldn't translate");
    (document.querySelector('.drop .ib.solid[title="Retry"]') as HTMLButtonElement).click();
    expect(sent).toEqual(['retry']);
  });

  it('Claude key error: the mark only; on hover the cross and the cause in words', () => {
    ctl.apply({ ...base, phase: 'error', error: 'claudeKey', retry: false });
    expect(shown()).toEqual(['mk']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Claude key problem');
  });

  it('error → error with another cause (network → Claude key) redraws the icon, "retry" and the label', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Model did not download');
    ctl.apply({ ...base, phase: 'error', error: 'claudeKey', retry: false });
    expect(shown()).toEqual(['mk']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Claude key problem');
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Model did not download');
  });
});

describe('Modes', () => {
  it('shows the translate icon while recording a translation', () => {
    ctl.apply({ ...base, phase: 'listen', mode: 'translate' });
    expect(shown()).toContain('mi');
    // Compared as parsed markup: the DOM writes the icon's self-closing <path/> back out as <path></path>.
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.tr;
    expect(document.querySelector('.drop .mi')!.innerHTML).toBe(icon.innerHTML);
  });

  it('switches the icon when the mode changes during the recording', () => {
    ctl.apply({ ...base, phase: 'listen' });
    expect(shown()).not.toContain('mi');
    ctl.apply({ ...base, phase: 'listen', mode: 'translate' });
    expect(shown()).toContain('mi');
  });

  it('a mode picked during start-up reshapes the mark at once', () => {
    const mark = () => (ctl as any).drop.morph.state;
    ctl.apply({ ...base, phase: 'start' });
    expect(mark()).toBe('start');
    ctl.apply({ ...base, phase: 'start', mode: 'translate' });
    expect(mark()).toBe('translate');
  });

  it('a state without a mode is a dictation', () => {
    const { mode: _, ...old } = base;
    ctl.apply({ ...(old as UiState), phase: 'listen' });
    expect(shown()).not.toContain('mi');
  });
});

/** A length in px from a computed value. jsdom cascades drop.css and inherits custom properties, but leaves var() and
 *  calc() as written: this resolves var() through the element's custom properties and does the arithmetic. */
function resolvePx(el: Element, value: string, depth = 0): number {
  if (depth > 8) throw new Error(`var() nested too deep in ${value}`);
  const cs = getComputedStyle(el);
  const flat = value.replace(/var\((--[\w-]+)\)/g, (_, name: string) => {
    const v = cs.getPropertyValue(name).trim();
    if (!v) throw new Error(`${name} is not set on ${el.className}`);
    return `(${resolvePx(el, v, depth + 1)}px)`;
  });
  return evalCalc(flat);
}

/** + - * / and parentheses over px lengths and plain numbers (calc( counts as a parenthesis). */
function evalCalc(src: string): number {
  const toks = src.replace(/calc\(/g, '(').match(/\d*\.?\d+(?:px)?|[-+*/()]/g) ?? [];
  let i = 0;
  const atom = (): number => {
    const t = toks[i++];
    if (t === '(') {
      const v = sum();
      if (toks[i++] !== ')') throw new Error(`unbalanced: ${src}`);
      return v;
    }
    if (t === '-') return -atom();
    const n = parseFloat(t ?? '');
    if (Number.isNaN(n)) throw new Error(`cannot read ${src}`);
    return n;
  };
  const product = (): number => {
    let v = atom();
    while (toks[i] === '*' || toks[i] === '/') v = toks[i++] === '*' ? v * atom() : v / atom();
    return v;
  };
  const sum = (): number => {
    let v = product();
    while (toks[i] === '+' || toks[i] === '-') v = toks[i++] === '+' ? v + product() : v - product();
    return v;
  };
  const v = sum();
  if (i !== toks.length) throw new Error(`cannot read ${src}`);
  return v;
}

/** The resolved px value of a property, e.g. size(drop(), 'height'). */
const size = (el: Element, prop: string) => resolvePx(el, getComputedStyle(el).getPropertyValue(prop));

describe('One size: every Drop size comes from --h (with drop.css)', () => {
  const css = readFileSync(join(__dirname, '../src/design/drop.css'), 'utf8');
  let sheet: HTMLStyleElement;
  beforeEach(() => {
    sheet = document.createElement('style');
    sheet.textContent = css;
    document.head.append(sheet);
  });
  afterEach(() => sheet.remove());
  const drop = () => document.querySelector('.drop') as HTMLElement;
  const part = (sel: string) => document.querySelector(`.drop ${sel}`) as HTMLElement;
  const H = 36;
  /** The mark part's box (what the layout sees): its svg, as large as the circle, with its negative margins. */
  const markBox = (axis: 'x' | 'y' = 'x') => {
    const svg = part('.mkw svg');
    return axis === 'x'
      ? size(svg, 'width') + size(svg, 'margin-left') + size(svg, 'margin-right')
      : size(svg, 'height') + size(svg, 'margin-top') + size(svg, 'margin-bottom');
  };
  /** The width of a Drop that shows only the mark: the mark, its margins and the Drop's padding. */
  const circleWidth = () => {
    const mark = part('.mkw');
    return markBox() + size(mark, 'margin-left') + size(mark, 'margin-right') + size(drop(), 'padding-left') + size(drop(), 'padding-right');
  };

  it('--h is 36 px', () => {
    expect(size(drop(), '--h')).toBe(H);
  });

  it('the same height in every state: idle on hover, start, recording, thinking, done, empty, cancelled, error', () => {
    const heights: Record<string, number> = {};
    for (const phase of ['start', 'listen', 'think', 'done', 'empty', 'cancel', 'error'] as const) {
      ctl.apply({ ...base, phase, error: phase === 'error' ? 'model' : null });
      heights[phase] = size(drop(), 'height');
    }
    ctl.apply(base);
    hover(true);
    heights['idle · hover'] = size(drop(), 'height');
    expect(Object.values(heights).every((h) => h === H), JSON.stringify(heights)).toBe(true);
  });

  it.each(['start', 'listen', 'think', 'done', 'empty'] as const)('%s is a circle of --h with the 0.6 h mark', (phase) => {
    ctl.apply({ ...base, phase });
    expect(markBox('x')).toBeCloseTo(0.6 * H, 6);
    expect(markBox('y')).toBeCloseTo(0.6 * H, 6);
    expect(circleWidth()).toBeCloseTo(H, 6);
    expect(size(drop(), 'min-width')).toBe(H);
  });

  it("the mark's svg is the circle's own box, and its viewBox keeps the 24 grid on the 0.6 h part", () => {
    ctl.apply({ ...base, phase: 'listen' });
    const svg = part('.mkw svg');
    // As large as the circle, starting at the Drop's own edge: padding, the part's margin and the svg's margin add to 0.
    expect([size(svg, 'width'), size(svg, 'height')]).toEqual([H, H]);
    expect(size(drop(), 'padding-left') + size(part('.mkw'), 'margin-left') + size(svg, 'margin-left')).toBeCloseTo(0, 6);
    // Grid x = 0 and x = 24 land on the part's two edges.
    const [vx, vy, vw, vh] = svg.getAttribute('viewBox')!.split(' ').map(Number);
    expect(vw).toBe(vh);
    expect(vx).toBe(vy);
    const unit = size(svg, 'width') / vw;
    const at = (x: number) => size(svg, 'margin-left') + (x - vx) * unit;
    expect(at(0)).toBeCloseTo(0, 6);
    expect(at(24)).toBeCloseTo(markBox(), 6);
  });

  it('derived sizes: buttons 0.7 h, small icons 0.5 h, keys and label text 0.9 of the 40 px drawing', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    hover(true);
    expect(size(part('.ib'), 'width')).toBeCloseTo(0.7 * H, 6);
    expect(size(part('.ib'), 'height')).toBeCloseTo(0.7 * H, 6);
    expect(size(part('.why'), 'width')).toBeCloseTo(0.5 * H, 6);
    expect(size(part('.ib > svg:not(.ring)'), 'width')).toBeCloseTo(16 * 0.9, 6);
    expect(size(part('.lab'), 'font-size')).toBeCloseTo(13.5 * 0.9, 6);
    expect(size(part('.lab'), 'line-height')).toBeCloseTo(24 * 0.9, 6);
    expect(size(part('.act'), 'height')).toBeCloseTo(28 * 0.9, 6);
    expect(size(part('.pt.on'), 'margin-left')).toBeCloseTo(4 * 0.9, 6);
    ctl.apply(base);
    hover(true);
    const key = part('.kc .key');
    expect(size(key, 'height')).toBeCloseTo(18 * 0.9, 6);
    // The keys' system type is 11 in the 40 px drawing.
    expect(size(key, 'font-size')).toBeCloseTo(11 * 0.9, 6);
  });

  /** The four visible gaps of the idle capsule on hover, CSS px: capsule edge to the bar's left stroke edge, the lamp
   *  dot to the first key, between the keys, the last key to the capsule edge. Where the bar and the dot are painted is
   *  read from the idle mark itself (cue.js), not from drop.css. */
  const idleGaps = () => {
    const mark = part('.mkw svg');
    const line = mark.querySelector('line')!;
    const dot = mark.querySelector('circle')!;
    const n = (el: Element, a: string) => parseFloat(el.getAttribute(a)!);
    const box = markBox();
    const barL = ((n(line, 'x1') - n(line, 'stroke-width') / 2) / 24) * box;
    const dotR = ((n(dot, 'cx') + n(dot, 'r')) / 24) * box;
    const mk = part('.mkw');
    const kc = part('.kc');
    return {
      left: size(drop(), 'padding-left') + size(mk, 'margin-left') + barL,
      dotToKey: box - dotR + size(mk, 'margin-right') + size(kc, 'margin-left'),
      keys: size(part('.kc > span'), 'gap'),
      right: size(kc, 'margin-right') + size(drop(), 'padding-right'),
    };
  };

  it('idle on hover: 0.3 h at both ends to what is painted, 0.2 h from the lamp to the keys, at every place and size', () => {
    ctl.apply(base);
    hover(true);
    // The idle mark the margins are measured from: the first bar at x 7.5 with a 2.2 stroke, the lamp at 21 with r 1.9.
    const line = part('.mkw svg line');
    expect([line.getAttribute('x1'), line.getAttribute('stroke-width')]).toEqual(['7.500', '2.2']);
    const dot = part('.mkw svg circle');
    expect([dot.getAttribute('cx'), dot.getAttribute('r')]).toEqual(['21.000', '1.900']);
    const check = (h: number, at: string) => {
      const g = idleGaps();
      expect(g.left, `${at} left`).toBeCloseTo(0.3 * h, 6);
      expect(g.right, `${at} right`).toBeCloseTo(0.3 * h, 6);
      expect(g.dotToKey, `${at} lamp to key`).toBeCloseTo(0.2 * h, 6);
      expect(g.keys, `${at} keys`).toBeCloseTo(0.1 * h, 6);
    };
    for (const place of ['bottom', 'top', 'left', 'right'] as const) {
      ctl.place(place);
      check(H, place);
    }
    // 36 px: no padding on the left, 7.2 on the right; the mark's margins 5.04 and 0, the keys' 6.21 and 3.6.
    ctl.place('bottom');
    expect(size(drop(), 'padding-left')).toBe(0);
    expect(size(drop(), 'padding-right')).toBeCloseTo(7.2, 6);
    expect([size(part('.mkw'), 'margin-left'), size(part('.mkw'), 'margin-right')].map((v) => +v.toFixed(4))).toEqual([5.04, 0]);
    expect([size(part('.kc'), 'margin-left'), size(part('.kc'), 'margin-right')].map((v) => +v.toFixed(4))).toEqual([6.21, 3.6]);
    const big = document.createElement('style');
    big.textContent = ':root{--h:48px}';
    document.head.append(big);
    try {
      check(48, '48 px');
    } finally {
      big.remove();
    }
    // Not on hover, and in other states, the parts keep the usual 4u margins.
    ctl.apply({ ...base, phase: 'listen' });
    expect(size(part('.mkw'), 'margin-left')).toBeCloseTo(3.6, 6);
  });

  it('idle strip: as long as the Drop is high, a fifth of it thick; upright at the side places', () => {
    ctl.apply(base);
    for (const place of ['bottom', 'top'] as const) {
      ctl.place(place);
      expect([size(drop(), 'min-width'), size(drop(), 'height')], place).toEqual([H, H / 5]);
    }
    for (const place of ['left', 'right'] as const) {
      ctl.place(place);
      expect([size(drop(), 'min-width'), size(drop(), 'height')], place).toEqual([H / 5, H]);
      expect([size(drop(), 'padding-left'), size(drop(), 'padding-right')], place).toEqual([0, 0]);
      // On hover it is the usual capsule, the same as at the bottom.
      hover(true);
      expect([size(drop(), 'min-width'), size(drop(), 'height')], `${place} · hover`).toEqual([H, H]);
      hover(false);
    }
  });

  it('a new --h resizes everything with it', () => {
    const big = document.createElement('style');
    big.textContent = ':root{--h:48px}';
    document.head.append(big);
    try {
      ctl.apply({ ...base, phase: 'listen' });
      expect(size(drop(), 'height')).toBe(48);
      expect(markBox()).toBeCloseTo(28.8, 6);
      expect(circleWidth()).toBeCloseTo(48, 6);
    } finally {
      big.remove();
    }
  });

  it('the size and place transitions: height, min-width and padding together, the place over .3 s', () => {
    const t = getComputedStyle(drop()).transition;
    for (const prop of ['height .45s var(--e-spring)', 'min-width .45s var(--e-spring)', 'padding .45s var(--e-spring)']) {
      expect(t).toContain(prop);
    }
    const pv = getComputedStyle(document.querySelector('.pv')!).transition;
    for (const prop of ['left', 'top', 'transform']) expect(pv).toContain(`${prop} .3s`);
  });

  it('at rest the strip breathes; on hover it stops and the keys are soft plates in the system type', () => {
    ctl.apply(base);
    const anim = () => getComputedStyle(drop()).getPropertyValue('animation');
    expect(anim()).toContain('idle-breathe 3.2s');
    hover(true);
    expect(anim()).not.toContain('idle-breathe');
    for (const key of document.querySelectorAll('.drop .kc .key')) {
      const k = getComputedStyle(key);
      expect(k.boxShadow).toBe('none');
      expect(k.backgroundColor).toBe('rgba(255, 255, 255, 0.12)');
      expect(k.fontFamily).not.toContain('Bebas');
    }
  });
});

describe('The mark opens to its own size, the same in every state', () => {
  it("max-width is the mark's natural width (measured once) plus the 2 px every part gets", () => {
    const proto = HTMLElement.prototype as any;
    const bcr = proto.getBoundingClientRect;
    proto.getBoundingClientRect = function (this: HTMLElement) {
      const w = this.classList.contains('mkw') ? 21.6 : 0;
      return { x: 0, y: 0, width: w, height: w, left: 0, top: 0, right: w, bottom: w, toJSON() {} };
    };
    // The mark's svg is as large as the circle and overflows its part: scrollWidth sees more than the part's width.
    const sw = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollWidth');
    Object.defineProperty(HTMLElement.prototype, 'scrollWidth', {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains('mkw') ? 29 : 0;
      },
    });
    try {
      document.body.innerHTML = '<div id="stage"></div>';
      const c = new DropController(document.getElementById('stage')!, { send() {}, rects() {} });
      const mark = () => document.querySelector('.drop .mkw') as HTMLElement;
      for (const phase of ['start', 'listen', 'think', 'done', 'cancel', 'error', 'empty'] as const) {
        c.apply({ ...base, phase, error: phase === 'error' ? 'model' : null });
        expect(mark().style.maxWidth, phase).toBe('23.6px');
      }
    } finally {
      proto.getBoundingClientRect = bcr;
      delete (HTMLElement.prototype as any).scrollWidth;
      if (sw) Object.defineProperty(Element.prototype, 'scrollWidth', sw);
    }
  });
});

describe('Places', () => {
  const css = readFileSync(join(__dirname, '../src/design/drop.css'), 'utf8');
  let sheet: HTMLStyleElement;
  beforeEach(() => {
    sheet = document.createElement('style');
    sheet.textContent = css;
    document.head.append(sheet);
  });
  afterEach(() => sheet.remove());
  const host = () => document.getElementById('stage')!;
  const flat = (v: string) => v.replace(/\s+/g, '');

  it('bottom by default; the core sets the place; anything else is bottom', () => {
    expect(host().dataset.place).toBe('bottom');
    for (const place of ['top', 'left', 'right', 'bottom'] as const) {
      ctl.place(place);
      expect(host().dataset.place).toBe(place);
    }
    for (const v of ['LEFT', 'middle', '', null, undefined, 3, {}]) {
      ctl.place('left');
      ctl.place(v);
      expect(host().dataset.place, String(v)).toBe('bottom');
    }
    expect(PLACES).toEqual(['bottom', 'top', 'left', 'right']);
    expect(pickPlace('right')).toBe('right');
  });

  it('the Drop and the tray card are pinned on the edge side and grow toward the center', () => {
    const pv = () => getComputedStyle(document.querySelector('.pv')!);
    const tray = () => getComputedStyle(document.querySelector('.tray')!);
    const inner = () => getComputedStyle(document.querySelector('.tray .in')!);
    const expected: Record<string, { left: string; top: string; transform: string; in: Record<string, string> }> = {
      // Bottom: the Drop's bottom on the anchor line, centered; the card's content sits at its bottom.
      bottom: { left: 'var(--ax,50%)', top: 'calc(var(--ay,50%)+var(--h)/2)', transform: 'translate(-50%,-100%)', in: { bottom: '0px', left: '50%' } },
      top: { left: 'var(--ax,50%)', top: 'calc(var(--ay,50%)-var(--h)/2)', transform: 'translate(-50%,0)', in: { top: '0px', left: '50%' } },
      // Sides: the outer end pinned half a Drop from the center, vertically centered; the card beside it, centered on it.
      left: { left: 'calc(var(--ax,50%)-var(--h)/2)', top: 'var(--ay,50%)', transform: 'translate(0,-50%)', in: { left: '0px', top: '50%' } },
      right: { left: 'calc(var(--ax,50%)+var(--h)/2)', top: 'var(--ay,50%)', transform: 'translate(-100%,-50%)', in: { right: '0px', top: '50%' } },
    };
    for (const [place, want] of Object.entries(expected)) {
      ctl.place(place);
      for (const s of [pv(), tray()]) {
        expect([flat(s.left), flat(s.top), flat(s.transform)], place).toEqual([want.left, want.top, want.transform]);
      }
      const zero = (v: string) => (v === '0' ? '0px' : v);
      for (const [prop, v] of Object.entries(want.in)) expect(zero(inner().getPropertyValue(prop)), `${place} .in ${prop}`).toBe(v);
    }
  });

  it('the answer tray is pinned like the tray cards at every place', () => {
    ctl.answer({ place: 'tray', quote: null, turns: [], sources: [], listening: false, missed: false, actions: ['insert', 'copy'] });
    const ans = () => getComputedStyle(document.querySelector('.tray.ansA')!);
    const card = () => getComputedStyle(document.querySelector('.tray:not(.ansA)')!);
    for (const place of PLACES) {
      ctl.place(place);
      expect([flat(ans().left), flat(ans().top), flat(ans().transform)], place).toEqual([flat(card().left), flat(card().top), flat(card().transform)]);
    }
  });
});

describe('Dragging the Drop', () => {
  let drags: string[];
  beforeEach(() => {
    drags = [];
    document.body.innerHTML = '<div id="stage"></div>';
    document.body.className = '';
    sent = [];
    rectsSent = [];
    ctl = new DropController(document.getElementById('stage')!, {
      send: (a) => sent.push(a),
      rects: (r) => rectsSent.push(r),
      drag: (p) => drags.push(p),
    });
  });
  const dropEl = () => document.querySelector('.drop') as HTMLElement;
  const ptr = (type: string, x: number, y: number, o: PointerEventInit = {}) =>
    dropEl().dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 1, pointerType: 'mouse', isPrimary: true, button: 0, clientX: x, clientY: y, ...o }));
  const stub = (el: Element, x: number, y: number, width: number, height: number) => {
    (el as any).getBoundingClientRect = () => ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON() {} });
  };
  /** Idle with the mouse over it: the capsule is open and can be picked up. */
  const ready = () => {
    ctl.apply(base);
    hover(true);
  };

  it('a press without movement is a click: no drag', () => {
    ready();
    ptr('pointerdown', 100, 50);
    ptr('pointerup', 100, 50);
    dropEl().click();
    expect(drags).toEqual([]);
    expect(sent).toEqual([]);
  });

  it(`moving up to ${DRAG_THRESHOLD} px is still a click; beyond it the drag starts once`, () => {
    ready();
    ptr('pointerdown', 100, 50);
    ptr('pointermove', 104, 50);
    ptr('pointermove', 100, 46);
    expect(drags).toEqual([]);
    ptr('pointermove', 103, 53); // 4.24 px
    expect(drags).toEqual(['start']);
    ptr('pointermove', 300, 200);
    ptr('pointermove', 10, 10);
    expect(drags).toEqual(['start']);
    expect(document.body.classList.contains('dragging')).toBe(true);
    ptr('pointerup', 10, 10);
    expect(drags).toEqual(['start', 'end']);
    expect(document.body.classList.contains('dragging')).toBe(false);
    // A later move without a press does nothing.
    ptr('pointermove', 400, 400);
    expect(drags).toEqual(['start', 'end']);
  });

  it('only the open idle Drop, only with the primary mouse button', () => {
    const tryDrag = (o: PointerEventInit = {}) => {
      ptr('pointerdown', 100, 50, o);
      ptr('pointermove', 140, 50, o);
      ptr('pointerup', 140, 50, o);
    };
    ctl.apply(base);
    tryDrag(); // the strip without hover: nothing to grab
    hover(true);
    tryDrag({ button: 2 });
    tryDrag({ pointerType: 'touch' });
    tryDrag({ pointerType: 'pen' });
    tryDrag({ isPrimary: false });
    for (const phase of ['start', 'listen', 'think', 'done', 'empty', 'cancel', 'error'] as const) {
      ctl.apply({ ...base, phase, error: phase === 'error' ? 'model' : null });
      tryDrag();
    }
    ctl.apply({ ...base, paused: true });
    tryDrag();
    ctl.apply(base);
    ctl.idleHidden(true);
    tryDrag();
    ctl.idleHidden(false);
    ctl.tray('lastMinute');
    tryDrag();
    expect(drags).toEqual([]);
  });

  it("the core's place ends the drag without another end; the button going up later does nothing", () => {
    ready();
    ptr('pointerdown', 100, 50);
    ptr('pointermove', 120, 50);
    ctl.place('left');
    expect(drags).toEqual(['start']);
    expect(document.body.classList.contains('dragging')).toBe(false);
    ptr('pointerup', 120, 50);
    expect(drags).toEqual(['start']);
  });

  it('a dictation, a pause, full screen or a tray card while dragging ends the drag first', () => {
    const cases: [string, () => void][] = [
      ['listen', () => ctl.apply({ ...base, phase: 'start' })],
      ['pause', () => ctl.apply({ ...base, paused: true })],
      ['full screen', () => ctl.idleHidden(true)],
      ['tray card', () => ctl.tray('noField')],
    ];
    for (const [name, interrupt] of cases) {
      ctl.apply(base);
      ctl.idleHidden(false);
      ctl.tray(null);
      (document.querySelector('.tray') as HTMLElement).hidden = true;
      hover(true);
      drags = [];
      ptr('pointerdown', 100, 50);
      ptr('pointermove', 120, 50);
      interrupt();
      expect(drags, name).toEqual(['start', 'end']);
      ptr('pointerup', 120, 50);
      expect(drags, name).toEqual(['start', 'end']);
    }
  });

  it('while dragged the whole window catches the mouse; then the capsule again', () => {
    ready();
    stub(dropEl(), 302, 182, 110, 36);
    ctl.frame(0);
    expect(rectsSent.at(-1)).toEqual([{ x: 302, y: 182, w: 110, h: 36 }]);
    ptr('pointerdown', 330, 200);
    ptr('pointermove', 340, 200);
    ctl.frame(16);
    expect(rectsSent.at(-1)).toEqual([{ x: 0, y: 0, w: window.innerWidth, h: window.innerHeight }]);
    ptr('pointerup', 340, 200);
    ctl.frame(32);
    expect(rectsSent.at(-1)).toEqual([{ x: 302, y: 182, w: 110, h: 36 }]);
  });

  it('the Drop keeps its open look while dragged, and folds when the mouse leaves after', () => {
    ready();
    ptr('pointerdown', 100, 50);
    ptr('pointermove', 120, 50);
    hover(false);
    expect(dropEl().classList.contains('hov')).toBe(true);
    ptr('pointerup', 120, 50);
    hover(false);
    expect(dropEl().classList.contains('hov')).toBe(false);
  });

  it('the frame loop does not sleep while the button is down or the Drop is dragged', () => {
    ready();
    ptr('pointerdown', 100, 50);
    hover(false); // the strip folds under a still-held button: still pressed
    let t = 0;
    for (; t < 3000; t += 16) expect(ctl.frame(t)).toBe(true);
    ptr('pointerup', 100, 50);
    for (; t < 6000 && ctl.frame(t); t += 16);
    expect(t).toBeLessThan(6000);
  });
});

describe('The place at startup', () => {
  it('is set before the Drop is built, so its first layout is already for that place', () => {
    const Drop = CUE().Drop;
    const atBuild: (string | undefined)[] = [];
    CUE().Drop = function (host: HTMLElement, api: unknown) {
      atBuild.push(host.dataset.place);
      return new Drop(host, api);
    };
    try {
      const cases: [unknown, string][] = [['left', 'left'], ['right', 'right'], ['top', 'top'], ['bottom', 'bottom'], [undefined, 'bottom'], ['middle', 'bottom']];
      for (const [given, want] of cases) {
        document.body.innerHTML = '<div id="stage"></div>';
        new DropController(document.getElementById('stage')!, { send() {}, rects() {} }, 'en', given);
        expect(atBuild.at(-1), String(given)).toBe(want);
        expect(document.getElementById('stage')!.dataset.place, String(given)).toBe(want);
      }
      // Without a place: bottom, as before.
      document.body.innerHTML = '<div id="stage"></div>';
      new DropController(document.getElementById('stage')!, { send() {}, rects() {} });
      expect(atBuild.at(-1)).toBe('bottom');
    } finally {
      CUE().Drop = Drop;
    }
  });
});

describe('Voice — only from the core', () => {
  const frames = (n: number) => {
    const t0 = performance.now();
    for (let i = 1; i <= n; i++) ctl.frame(t0 + i * 16);
  };
  const voice = () => (ctl as any).voice as { lvl: number; ext: number | null };

  it('recording without a level from the core — the bars stay at zero', () => {
    ctl.apply({ ...base, phase: 'listen' });
    frames(60);
    expect(voice().lvl).toBe(0);
  });

  it('a level from the core moves the bars; outside [0, 1] or not a number — clamped', () => {
    ctl.apply({ ...base, phase: 'listen' });
    ctl.level(0.8);
    frames(30);
    expect(voice().lvl).toBeGreaterThan(0.5);
    ctl.level(5);
    expect(voice().ext).toBe(1);
    ctl.level(-2);
    expect(voice().ext).toBe(0);
    ctl.level(NaN);
    expect(voice().ext).toBe(0);
    ctl.level(Infinity);
    expect(voice().ext).toBe(0);
  });

  it('a new recording does not drift to the last level of the previous one', () => {
    ctl.apply({ ...base, phase: 'listen' });
    ctl.level(0.9);
    frames(30);
    expect(voice().lvl).toBeGreaterThan(0.5);
    ctl.apply({ ...base, phase: 'think' }); // leaving the recording resets the level
    expect(voice().ext).toBe(0);
    expect(voice().lvl).toBe(0);
    ctl.level(0.9); // a late level event after the recording
    ctl.apply({ ...base, phase: 'listen' }); // entering a recording starts from zero again
    expect(voice().ext).toBe(0);
    expect(voice().lvl).toBe(0);
    frames(30);
    expect(voice().lvl).toBe(0);
  });
});

describe('Idle, pause, full screen, shake', () => {
  it('the bar is hidden while paused and over a full-screen app', () => {
    ctl.apply({ ...base, paused: true });
    expect(document.body.classList.contains('idle-hidden')).toBe(true);
    ctl.idleHidden(true);
    ctl.apply({ ...base, paused: false });
    expect(document.body.classList.contains('idle-hidden')).toBe(true);
    ctl.idleHidden(false);
    expect(document.body.classList.contains('idle-hidden')).toBe(false);
  });

  it('shake', () => {
    ctl.shake();
    expect(document.querySelector('.drop')!.classList.contains('shake')).toBe(true);
  });
});

describe('Rectangles for click-through', () => {
  const stub = (el: Element, x: number, y: number, width: number, height: number) => {
    (el as any).getBoundingClientRect = () => ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON() {} });
  };

  it('sent only on a change; a hidden bar does not catch the mouse, a recording does', () => {
    stub(document.querySelector('.drop')!, 162, 233, 36, 7);
    ctl.frame(0);
    ctl.frame(16);
    expect(rectsSent).toEqual([[{ x: 162, y: 233, w: 36, h: 7 }]]);
    ctl.idleHidden(true);
    ctl.frame(32);
    expect(rectsSent.at(-1)).toEqual([]);
    ctl.apply({ ...base, phase: 'listen' });
    ctl.frame(48);
    expect(rectsSent.at(-1)).toEqual([{ x: 162, y: 233, w: 36, h: 7 }]);
    expect(rectsSent.length).toBe(3);
  });

  it('rounded outwards to whole px; fractional jitter sends no new message', () => {
    const d = document.querySelector('.drop')!;
    stub(d, 142.4, 200.6, 75.2, 39.1);
    ctl.frame(0);
    expect(rectsSent).toEqual([[{ x: 142, y: 200, w: 76, h: 40 }]]); // right edge 217.6 → 218, bottom 239.7 → 240
    stub(d, 142.45, 200.9, 75.1, 38.95);
    ctl.frame(16);
    expect(rectsSent.length).toBe(1);
  });

  it('while a tray card is open — its rectangle instead of the Drop', () => {
    stub(document.querySelector('.drop')!, 162, 233, 36, 7);
    ctl.tray('lastMinute');
    stub(document.querySelector('.tray')!, 30, 140, 300, 100);
    ctl.frame(0);
    expect(rectsSent.at(-1)).toEqual([{ x: 30, y: 140, w: 300, h: 100 }]);
  });
});

describe('Tray card', () => {
  it('"Nowhere to paste": the button changes its label by common letters and sends trayAction', () => {
    ctl.tray('noField');
    expect(document.querySelector('.tray')!.textContent).toContain('Nowhere to paste');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect(sent).toEqual(['trayAction']);
    expect(document.querySelector('.tray .lbl')!.getAttribute('data-t')).toBe('Copied');
    // "Cop" stays, only the new tail "ied" fades in.
    const letters = [...document.querySelectorAll('.tray .lbl span')];
    expect(letters.map((s) => s.textContent).join('')).toBe('Copied');
    expect(letters.filter((s) => s.classList.contains('ch-in')).map((s) => s.textContent).join('')).toBe('ied');
  });

  it('the cross closes the card and tells the core', () => {
    ctl.tray('micSilent');
    (document.querySelector('.tray .xb') as HTMLButtonElement).click();
    expect(sent).toEqual(['trayClose']);
  });

  it('grows out of the Drop\'s own shape and folds back into it: the circle, a wider capsule, the upright strip', () => {
    vi.useFakeTimers();
    try {
      const d = document.querySelector('.drop') as HTMLElement;
      const box = document.querySelector('.tray') as HTMLElement;
      const size = (w: number, h: number) => {
        Object.defineProperty(d, 'offsetWidth', { configurable: true, value: w });
        Object.defineProperty(d, 'offsetHeight', { configurable: true, value: h });
      };
      for (const [w, h] of [[36, 36], [116, 36], [7, 36]]) {
        size(w, h);
        ctl.tray('lastMinute');
        expect([box.style.width, box.style.height, box.style.borderRadius], `open ${w}×${h}`).toEqual([`${w}px`, `${h}px`, `${h / 2}px`]);
        ctl.tray(null);
        expect([box.style.width, box.style.height, box.style.borderRadius], `close ${w}×${h}`).toEqual([`${w}px`, `${h}px`, `${h / 2}px`]);
        vi.advanceTimersByTime(500);
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it('the hint puts in the keys', () => {
    ctl.tray('holdHint');
    expect(document.querySelector('.tray kbd')!.textContent).toBe('Ctrl + Win');
  });

  it('micMissing card escapes device names', () => {
    const wanted = '<img src=x onerror="alert(1)"> Headset & Co';
    const used = 'Mic $& $1 $` <b>(Realtek)</b>';
    ctl.micNames({ wanted, used });
    ctl.tray('micMissing');
    const body = document.querySelector('.tray .nbd')!;
    expect(body.querySelector('img')).toBeNull();
    expect([...body.querySelectorAll('b')].map((b) => b.textContent)).toEqual([wanted, used]);
    expect(body.textContent).toBe(`${wanted} not found — recording from ${used}`);
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe(en.tray.micMissing.title);
    expect(document.querySelector('.tray [data-act]')).toBeNull();

    // In Russian, and a name Windows did not give: the Windows microphone.
    const box = document.createElement('div');
    box.innerHTML = trayHtml('micMissing', ru, { wanted: 'Гарнитура "Jabra"', used: '' });
    expect(box.querySelector('.nbd')!.textContent).toBe(`Гарнитура "Jabra" не найден — пишу с ${ru.tray.micMissing.unnamed}`);
  });

  it('modelReady card: the model icon, green, closes by itself in 5 s, no button', () => {
    expect(TRAYS.modelReady).toEqual({ icon: 'model', tone: 'ok', act: null, ttl: 5000 });
    ctl.tray('modelReady');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Model is ready');
    expect(document.querySelector('.tray .nic svg')).not.toBeNull();
    expect(document.querySelector('.tray [data-act]')).toBeNull();
  });

  it('modelSwitched card: "Model is ready" with the name of the model chosen, escaped; green, 5 s, no button', () => {
    expect(TRAYS.modelSwitched).toEqual({ icon: 'model', tone: 'ok', act: null, ttl: 5000 });
    ctl.modelName('Parakeet <v3> & Co');
    ctl.tray('modelSwitched');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Model is ready');
    const body = document.querySelector('.tray .nbd')!;
    expect(body.querySelector('b')!.textContent).toBe('Parakeet <v3> & Co');
    expect(document.querySelector('.tray [data-act]')).toBeNull();
    // In Russian, and no name heard (the Drop was reloaded meanwhile).
    const box = document.createElement('div');
    box.innerHTML = trayHtml('modelSwitched', ru, null, null);
    expect(box.querySelector('.nhd b')!.textContent).toBe('Модель готова');
    expect(box.querySelector('.nbd')!.textContent).toBe(ru.tray.modelSwitched.unnamed);
  });

  it('micMissing card: microphone icon, amber, closes by itself in 6 s, no button', () => {
    expect(TRAYS.micMissing).toEqual({ icon: 'mic', tone: 'warn', act: null, ttl: 6000 });
    expect(en.tray.micMissing.body).toBe('<b>{WANTED}</b> not found — recording from <b>{USED}</b>');
    expect(ru.tray.micMissing.body).toBe('<b>{WANTED}</b> не найден — пишу с <b>{USED}</b>');
  });

  it('markup: "Close" from the dictionary, the ring only on cards that close by themselves, a button where there is an action', () => {
    const box = document.createElement('div');
    box.innerHTML = trayHtml('noField', en);
    const xb = box.querySelector('.xb')!;
    expect(xb.getAttribute('title')).toBe(en.close);
    expect(xb.getAttribute('aria-label')).toBe(en.close);
    expect(xb.classList.contains('nottl')).toBe(false);
    expect(xb.getAttribute('style')).toBe(`--ttl:${TRAYS.noField.ttl}ms`);
    box.innerHTML = trayHtml('micSilent', en);
    expect(box.querySelector('.xb')!.classList.contains('nottl')).toBe(true);
    box.innerHTML = trayHtml('lastMinute', en);
    expect(box.querySelector('[data-act]')).toBeNull();
  });

  it('every card in every language: title, body and button from that language', () => {
    for (const lang of ['en', 'ru'] as const) {
      const t = DICTS[lang];
      for (const kind of KINDS) {
        const box = document.createElement('div');
        box.innerHTML = trayHtml(kind, t, { wanted: 'Headset', used: 'Microphone' }, 'Parakeet v3', 'In Telegram');
        const body = t.tray[kind].body
          .replace('{KEY}', `<kbd>${KEY_NAME}</kbd>`)
          .replace('{WANTED}', 'Headset')
          .replace('{USED}', 'Microphone')
          .replace('{NAME}', 'Parakeet v3')
          .replace('{RULE}', 'In Telegram');
        expect(box.querySelector('.nhd b')!.textContent, `${lang}.${kind}`).toBe(t.tray[kind].title);
        expect(box.querySelector('.xb')!.getAttribute('aria-label'), `${lang}.${kind}`).toBe(t.close);
        expect(box.querySelector('.nbd')!.innerHTML, `${lang}.${kind}`).toBe(body);
        expect(box.querySelector('.lbl')?.textContent, `${lang}.${kind}`).toBe(t.tray[kind].act);
      }
    }
  });
});

describe('Russian interface', () => {
  beforeEach(() => {
    document.body.innerHTML = '<div id="stage"></div>';
    ctl = new DropController(document.getElementById('stage')!, { send: (a) => sent.push(a), rects() {}, wake() {} }, 'ru');
  });

  it('Drop buttons and labels in Russian', () => {
    const title = (sel: string) => document.querySelector(sel)!.getAttribute('title');
    expect(title('.drop .ib.soft')).toBe('Отменить · Esc');
    expect(document.querySelector('.drop .ib.soft')!.getAttribute('aria-label')).toBe('Отменить');
    expect([...document.querySelectorAll('.drop .ib.solid:not(.sbt)')].map((e) => e.getAttribute('title'))).toEqual(['Вернуть', 'Повторить', 'Готово']);
    ctl.apply({ ...base, phase: 'cancel' });
    hover(true);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Отменено');
    ctl.apply({ ...base, phase: 'error', error: 'stt', retry: true });
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Не распознал');
    ctl.apply({ ...base, phase: 'error', error: 'translate', retry: true, mode: 'translate' });
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Не удалось перевести');
  });

  it('no-field card in Russian: the copy label changes by common letters, only the new tail fades in', () => {
    ctl.tray('noField');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Некуда вставить');
    expect(document.querySelector('.tray .xb')!.getAttribute('title')).toBe('Закрыть');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect(sent).toEqual(['trayAction']);
    const letters = [...document.querySelectorAll('.tray .lbl span')];
    expect(letters.map((s) => s.textContent).join('')).toBe('Скопировано');
    expect(letters.filter((s) => s.classList.contains('ch-in')).map((s) => s.textContent).join('')).toBe('но');
  });
  it('the habit card in Russian: «Отменить» becomes «Убрано»', () => {
    ctl.habitRule('В Telegram — с маленькой буквы и без точки');
    ctl.tray('newHabit');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Новая привычка');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с маленькой буквы и без точки. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Убрано');
  });
  it('the changed habit card in Russian: «Отменить» becomes «Возвращено»', () => {
    ctl.habitRule('В Telegram — с точкой в конце');
    ctl.tray('habitChanged');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Привычка изменилась');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с точкой в конце. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Возвращено');
  });
});

describe('Tray card closes on a timer', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const title = () => box().querySelector('.nhd b')!.textContent;
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('closes after its time and tells the core', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(TRAYS.lastMinute.ttl - 1);
    expect(isOpen()).toBe(true);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
    vi.advanceTimersByTime(500);
    expect(box().hidden).toBe(true);
  });

  it('hover pauses the countdown, leaving resumes it from what is left', () => {
    ctl.tray('noField');
    vi.advanceTimersByTime(3000);
    box().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(60_000);
    expect(isOpen()).toBe(true);
    expect(sent).toEqual([]);
    box().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(TRAYS.noField.ttl - 3000 - 1);
    expect(isOpen()).toBe(true);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('a new card replaces the content and starts the countdown over — the old timer does not close it', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(4000);
    ctl.tray('holdHint');
    vi.advanceTimersByTime(2000); // the old one would have fired at 5000
    expect(isOpen()).toBe(true);
    expect(title()).toBe('Press and release');
    ctl.tray('holdHint'); // the same kind again — also from the start
    vi.advanceTimersByTime(TRAYS.holdHint.ttl - 1);
    expect(isOpen()).toBe(true);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('the close after "Copied" does not close a new card of the same kind', () => {
    ctl.tray('noField');
    vi.advanceTimersByTime(20);
    act().click();
    vi.advanceTimersByTime(500);
    ctl.tray('noField');
    vi.advanceTimersByTime(1000); // 1500 ms after the press
    expect(isOpen()).toBe(true);
    expect(act().disabled).toBe(false);
    expect(box().querySelector('.lbl')!.textContent).toBe('Copy');
    expect(sent).toEqual(['trayAction']);
  });

  it('without a time ("Microphone is silent") it does not close by itself', () => {
    ctl.tray('micSilent');
    vi.advanceTimersByTime(10 * 60_000);
    expect(isOpen()).toBe(true);
    expect(sent).toEqual([]);
  });

  it('pressing the button cancels the countdown: closes after "Copied", without trayClose', () => {
    ctl.tray('noField');
    vi.advanceTimersByTime(TRAYS.noField.ttl - 500);
    act().click();
    vi.advanceTimersByTime(1000); // the countdown would have ended in the middle of "Copied"
    expect(isOpen()).toBe(true);
    expect(box().querySelector('.lbl')!.getAttribute('data-t')).toBe('Copied');
    vi.advanceTimersByTime(100);
    expect(isOpen()).toBe(false);
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual(['trayAction']);
  });

  it('a button without a result label ("Got it") closes at once, the countdown no longer fires', () => {
    ctl.tray('holdHint');
    vi.advanceTimersByTime(20);
    act().click();
    expect(isOpen()).toBe(false);
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual(['trayAction']);
  });

  it('the core removes the card — the countdown is cancelled, no trayClose', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(1000);
    ctl.tray(null);
    expect(isOpen()).toBe(false);
    vi.advanceTimersByTime(60_000);
    expect(box().hidden).toBe(true);
    expect(sent).toEqual([]);
  });

  it('the cross while folding does not send trayClose a second time', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(20);
    const xb = box().querySelector('.xb') as HTMLButtonElement;
    xb.click();
    xb.click();
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual(['trayClose']);
  });
});

describe('New habit card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — lower case and no full stop';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the rule under "New habit", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    expect(box().querySelector('.nhd b')!.textContent).toBe('New habit');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s; hovering pauses it', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    box().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(30_000);
    expect(isOpen()).toBe(true);
    box().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Removed", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    act().click();
    expect(sent).toEqual(['trayAction']);
    const letters = [...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('');
    expect(letters).toBe('Removed');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });

  it('without a rule the card says only the sentence', () => {
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd')!.textContent).toBe('CUE writes it this way from now on.');
  });

  it('a second card does not repeat the old rule', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd')!.textContent).toBe('CUE writes it this way from now on.');
    ctl.habitRule(RULE);
    ctl.tray(null);
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd')!.textContent).toBe('CUE writes it this way from now on.');
  });

  it('a rule that ends with its own stop gets no extra full stop', () => {
    ctl.habitRule('Is it a question?');
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd')!.textContent).toBe('Is it a question? CUE writes it this way from now on.');
  });

  it('the rule goes in as text', () => {
    ctl.habitRule('<b>x</b>');
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd b')).toBeNull();
    expect(box().querySelector('.nbd')!.textContent).toContain('<b>x</b>');
  });
});

describe('Habit changed card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — a full stop at the end';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the new rule under "Habit changed", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    expect(box().querySelector('.nhd b')!.textContent).toBe('Habit changed');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Restored", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    act().click();
    expect(sent).toEqual(['trayAction']);
    expect([...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('')).toBe('Restored');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });
});

describe('Tray card: hover across a replacement, markup error', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const title = () => box().querySelector('.nhd b')!.textContent;

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('hover carries over a replacement: the new card stays while the mouse is over it, then gets its full time', () => {
    ctl.tray('noField');
    vi.advanceTimersByTime(1000);
    box().dispatchEvent(new MouseEvent('mouseenter'));
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(60_000);
    expect(isOpen()).toBe(true);
    expect(title()).toBe('One minute left');
    expect(sent).toEqual([]);
    box().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(TRAYS.lastMinute.ttl - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('a hidden card does not count as hovered: the next one closes on time even without mouseleave', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(20);
    box().dispatchEvent(new MouseEvent('mouseenter'));
    ctl.tray(null);
    vi.advanceTimersByTime(500); // the card is hidden, no mouseleave came
    expect(box().hidden).toBe(true);
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(TRAYS.lastMinute.ttl);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('the new card markup throws — the old card keeps working: timer and cross are alive', () => {
    ctl.tray('lastMinute');
    vi.advanceTimersByTime(1000);
    expect(() => ctl.tray('bogus' as any)).toThrow();
    expect(isOpen()).toBe(true);
    expect(title()).toBe('One minute left');
    vi.advanceTimersByTime(TRAYS.lastMinute.ttl - 1000);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);

    ctl.tray('micSilent');
    vi.advanceTimersByTime(20);
    expect(() => ctl.tray('bogus' as any)).toThrow();
    (box().querySelector('.xb') as HTMLButtonElement).click();
    expect(sent).toEqual(['trayClose', 'trayClose']);
  });
});

describe('Quiet mode: frames stop when nothing on screen can change', () => {
  /** Frames 16 ms apart while frame() asks to go on; returns the time of the last frame, or -1 if it never went quiet. */
  const runUntilQuiet = (from: number, limit = 5000) => {
    for (let t = from; t <= from + limit; t += 16) if (!ctl.frame(t)) return t;
    return -1;
  };
  const stub = (el: Element, x: number, y: number, width: number, height: number) => {
    (el as any).getBoundingClientRect = () => ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON() {} });
  };

  it('idle: goes quiet after 1 s without events', () => {
    expect(ctl.frame(0)).toBe(true);
    expect(ctl.frame(500)).toBe(true);
    expect(ctl.frame(999)).toBe(true);
    expect(ctl.frame(1000)).toBe(false);
    expect(wakes).toBe(0);
  });

  it('an error goes quiet too (the mark is still after the shake); recording, thinking, done, cancelled do not', () => {
    ctl.apply({ ...base, phase: 'error', error: 'model', retry: true });
    expect(runUntilQuiet(0)).toBe(1008);
    for (const phase of ['listen', 'think', 'done', 'cancel', 'start', 'empty'] as const) {
      ctl.apply({ ...base, phase });
      expect(runUntilQuiet(0)).toBe(-1);
    }
  });

  it('does not go quiet while the mouse is over the Drop; after leaving — in 1 s', () => {
    hover(true);
    expect(runUntilQuiet(0)).toBe(-1);
    hover(false);
    expect(runUntilQuiet(6000)).toBe(7008);
  });

  it('does not go quiet while a tray card is open or folding', () => {
    vi.useFakeTimers();
    try {
      ctl.tray('micSilent');
      expect(runUntilQuiet(0)).toBe(-1);
      ctl.tray(null);
      expect(runUntilQuiet(6000, 300)).toBe(-1); // still folding (430 ms until hidden)
      vi.advanceTimersByTime(500);
      expect((document.querySelector('.tray') as HTMLElement).hidden).toBe(true);
      expect(runUntilQuiet(7000)).toBe(8008);
    } finally {
      vi.useRealTimers();
    }
  });

  it('any event wakes it: state, level, tray card, shake, full screen, mouse on the Drop and on the card', () => {
    const inputs: [string, () => void][] = [
      ['apply', () => ctl.apply(base)],
      ['level', () => ctl.level(0)],
      ['tray', () => ctl.tray(null)],
      ['shake', () => ctl.shake()],
      ['idleHidden', () => ctl.idleHidden(false)],
      ['mouseenter Drop', () => { hover(true); hover(false); }],
      ['mouseenter tray card', () => document.querySelector('.tray')!.dispatchEvent(new MouseEvent('mouseenter'))],
    ];
    let t = 0;
    for (const [name, input] of inputs) {
      document.querySelector('.tray')!.dispatchEvent(new MouseEvent('mouseleave'));
      const quietAt = runUntilQuiet(t);
      expect(quietAt, name).toBeGreaterThan(0);
      const before = wakes;
      input();
      expect(wakes, name).toBe(before + 1);
      expect(ctl.frame(quietAt + 16), name).toBe(true); // the quiet countdown started over
      t = quietAt + 32;
    }
  });

  it('while awake there is nothing to wake — wake is not called on every event', () => {
    ctl.frame(0);
    ctl.apply(base);
    ctl.level(0);
    ctl.shake();
    expect(wakes).toBe(0);
  });

  it('before sleeping the latest rectangles have been sent', () => {
    const d = document.querySelector('.drop')!;
    stub(d, 122.5, 206, 115, 34); // still folding after hover
    ctl.frame(0);
    stub(d, 162, 233, 36, 7); // the transition has ended
    expect(ctl.frame(1000)).toBe(false);
    expect(rectsSent.at(-1)).toEqual([{ x: 162, y: 233, w: 36, h: 7 }]);
  });

  it('a prefers-reduced-motion change wakes it', () => {
    const listeners: (() => void)[] = [];
    const mq = { matches: false, addEventListener: (_: string, f: () => void) => listeners.push(f), removeEventListener() {} };
    (window as any).matchMedia = () => mq;
    try {
      document.body.innerHTML = '<div id="stage"></div>';
      let w = 0;
      const c = new DropController(document.getElementById('stage')!, { send() {}, rects() {}, wake: () => w++ });
      for (let t = 0; c.frame(t); t += 16);
      mq.matches = true;
      for (const f of listeners) f();
      expect(w).toBe(1);
      expect(c.frame(5000)).toBe(true);
    } finally {
      delete (window as any).matchMedia;
    }
  });
});

describe('"Ask" answer in the tray', () => {
  const done = (over: Partial<Answer> = {}): Answer => ({
    place: 'tray',
    quote: null,
    turns: [{ question: 'What is HTTP 418?', step: null, segments: [{ text: "I'm a teapot.", cites: [] }], state: 'done' }],
    sources: [],
    listening: false,
    missed: false,
    actions: ['insert', 'copy'],
    ...over,
  });
  const box = () => document.querySelector('.tray.ansA') as HTMLElement;

  it('grows out of the Drop, 420 wide; the Drop steps aside; closing folds it back and empties it', () => {
    vi.useFakeTimers();
    ctl.answer(done());
    expect(box().hidden).toBe(false);
    expect((box().querySelector('.in') as HTMLElement).style.width).toBe('420px');
    expect(document.querySelector('.drop')!.classList.contains('away')).toBe(true);
    expect(box().querySelector('.aq')!.textContent).toBe('What is HTTP 418?');
    ctl.answer(null);
    vi.advanceTimersByTime(1_000);
    expect(box().hidden).toBe(true);
    expect(document.querySelector('.drop')!.classList.contains('away')).toBe(false);
    expect(box().querySelector('.in')!.children).toHaveLength(0);
    vi.useRealTimers();
  });

  it('a card answer belongs to the card window, not the Drop', () => {
    ctl.answer(done({ place: 'card' }));
    expect(box().hidden).toBe(true);
  });

  it('while a card answer is open, a chord shakes the card, not the Drop', () => {
    ctl.answer({ place: 'card', quote: 'q', turns: [], sources: [], listening: false, missed: false, actions: ['below', 'replace', 'copy'] });
    ctl.shake();
    expect(document.querySelector('.drop')!.classList.contains('shake')).toBe(false);
    ctl.answer(null);
    ctl.shake();
    expect(document.querySelector('.drop')!.classList.contains('shake')).toBe(true);
  });

  it('its buttons and source chips reach the core', () => {
    const opened: number[] = [];
    document.body.innerHTML = '<div id="stage"></div>';
    sent = [];
    ctl = new DropController(document.getElementById('stage')!, {
      send: (a) => sent.push(a),
      rects: () => {},
      source: (n) => opened.push(n),
    });
    ctl.apply({ ...base, phase: 'answer', mode: 'ask' });
    ctl.answer(done({ sources: [{ n: 1, site: 'httpwg.org', title: 'RFC 2324' }], turns: [{ question: 'q', step: null, segments: [{ text: 'Teapot', cites: [1] }], state: 'done' }] }));
    for (let t = 0; t < 1_000; t += 60) ctl.frame(t);
    box().querySelector<HTMLButtonElement>('[data-cmd="answerInsert"]')!.click();
    box().querySelector<HTMLButtonElement>('.sc')!.click();
    box().querySelector<HTMLButtonElement>('[data-x]')!.click();
    expect(sent).toEqual(['answerInsert', 'answerClose']);
    expect(opened).toEqual([1]);
  });

  it('while a follow-up is recorded, thought over or missed, its buttons and cross wait: the core takes them only in its answer phase', () => {
    ctl.apply({ ...base, phase: 'answer', mode: 'ask' });
    ctl.answer(done());
    for (let t = 0; t < 1_000; t += 60) ctl.frame(t);
    const all = () => [...box().querySelectorAll<HTMLButtonElement>('[data-x], .aft button')];
    expect(all().map((b) => b.disabled)).toEqual([false, false, false]);
    for (const phase of ['start', 'listen', 'think', 'error'] as const) {
      ctl.apply({ ...base, phase, mode: 'ask' });
      ctl.frame(2_000);
      expect(all().map((b) => b.disabled), phase).toEqual([true, true, true]);
      for (const b of all()) b.click();
      expect(sent, phase).toEqual([]);
    }
    // The missed follow-up's Retry is the Drop's own retry, which the core takes in its error phase.
    ctl.answer(done({ missed: true }));
    box().querySelector<HTMLButtonElement>('.lsn.miss button')!.click();
    expect(sent).toEqual(['retry']);
    ctl.apply({ ...base, phase: 'answer', mode: 'ask' });
    expect(all().map((b) => b.disabled)).toEqual([false, false, false]);
    box().querySelector<HTMLButtonElement>('[data-cmd="answerInsert"]')!.click();
    expect(sent).toEqual(['retry', 'answerInsert']);
  });

  it('the resting Drop of an open answer looks idle; it is not draggable then', () => {
    ctl.answer(done({ place: 'card' }));
    ctl.apply({ ...base, phase: 'answer', mode: 'ask' });
    expect(document.querySelector('.drop')!.getAttribute('data-s')).toBe('idle');
  });

  it('while the answer is open its rectangle catches the mouse; a chord that only shakes moves its header', () => {
    ctl.answer(done());
    const stub = (el: Element, x: number, y: number, w: number, h: number) => {
      (el as any).getBoundingClientRect = () => ({ x, y, width: w, height: h, left: x, top: y, right: x + w, bottom: y + h, toJSON() {} });
    };
    stub(box(), 270, 40, 420, 300);
    ctl.frame(0);
    expect(rectsSent.at(-1)).toEqual([{ x: 270, y: 40, w: 420, h: 300 }]);
    ctl.shake();
    expect(box().querySelector('.ah')!.classList.contains('shake')).toBe(true);
  });

  it('the Drop stays aside while either tray is open', () => {
    vi.useFakeTimers();
    ctl.answer(done());
    ctl.tray('noField');
    ctl.tray(null);
    vi.advanceTimersByTime(500);
    ctl.frame(1_000);
    expect(document.querySelector('.drop')!.classList.contains('away')).toBe(true);
    vi.useRealTimers();
  });

  it('a tray still folding keeps the Drop aside until it has folded into it', () => {
    vi.useFakeTimers();
    try {
      const away = () => document.querySelector('.drop')!.classList.contains('away');
      for (const [open, close] of [[() => ctl.tray('noField'), () => ctl.tray(null)], [() => ctl.answer(done()), () => ctl.answer(null)]]) {
        open();
        ctl.frame(0);
        close();
        ctl.frame(16);
        expect(away()).toBe(true);
        vi.advanceTimersByTime(500);
        ctl.frame(600);
        expect(away()).toBe(false);
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it('an answer back while the last one folds (or just has) starts afresh: its words are revealed anew', () => {
    vi.useFakeTimers();
    try {
      const turnOf = (question: string, text: string) => ({ question, step: null, segments: [{ text, cites: [] }], state: 'done' as const });
      // 200 ms: the tray is still folding; 440 ms: it is hidden, but the old words are not cleared yet (450 ms).
      for (const after of [200, 440]) {
        ctl.answer(done({ turns: [turnOf('First?', 'one two three four five')] }));
        for (let t = 0; t < 1_000; t += 60) ctl.frame(t);
        ctl.answer(null);
        vi.advanceTimersByTime(after);
        ctl.answer(done({ turns: [turnOf('Second?', 'six seven eight')] }));
        expect(box().querySelectorAll('.aa .w.on'), `${after} ms`).toHaveLength(0);
        expect(box().querySelector('.aq')!.textContent).toBe('Second?');
        ctl.answer(null);
        vi.advanceTimersByTime(1_000);
      }
    } finally {
      vi.useRealTimers();
    }
  });

  it('an answer over a full-screen app sends no rectangle for the hidden Drop', () => {
    const d = document.querySelector('.drop') as HTMLElement;
    (d as any).getBoundingClientRect = () => ({ x: 302, y: 182, width: 110, height: 36, left: 302, top: 182, right: 412, bottom: 218, toJSON() {} });
    ctl.frame(0);
    expect(rectsSent.at(-1)).toEqual([{ x: 302, y: 182, w: 110, h: 36 }]);
    ctl.apply({ ...base, phase: 'answer', mode: 'ask' });
    ctl.idleHidden(true);
    ctl.frame(16);
    expect(rectsSent.at(-1)).toEqual([]);
  });

  it('the frame loop sleeps once the answer is shown whole, and wakes for a new one', () => {
    ctl.answer(done());
    let t = 0;
    while (ctl.frame(t) && t < 10_000) t += 16;
    expect(t).toBeLessThan(10_000);
    const before = wakes;
    ctl.answer(done({ listening: true }));
    expect(wakes).toBeGreaterThan(before);
    expect(ctl.frame(t + 16)).toBe(true);
  });
});

describe('Dictionary suggestion', () => {
  const drop = () => document.querySelector('.drop') as HTMLElement;
  const btn = () => drop().querySelector('.sbt') as HTMLButtonElement;
  const dot = () => document.querySelector('.sgdot') as HTMLElement;
  const ring = () => btn().querySelector('.ring circle') as SVGElement;
  const ringEnd = () => ring().dispatchEvent(new Event('animationend', { bubbles: true }));

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('opens the capsule with the word and adds it', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().querySelector('.swd')!.textContent).toContain('Marisol');
    expect(btn().getAttribute('aria-label')).toBe('Add to dictionary');
    btn().click();
    expect(sent).toEqual(['suggestAdd']);
    expect(drop().dataset.s).toBe('added');
    expect(drop().querySelector('.sbk')!.classList.contains('ok')).toBe(true);
    expect(btn().getAttribute('aria-label')).toBe('Undo');
  });

  it('undo takes the word out; the end of its ring closes', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    btn().click();
    expect(sent).toEqual(['suggestAdd', 'suggestUndo']);
    ctl.suggest(null); // the core answers an undo with nothing
    expect(drop().dataset.s).toBe('idle');
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    vi.advanceTimersByTime(6000 - 1);
    expect(sent.at(-1)).toBe('suggestAdd');
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe('suggestDone');
    expect(drop().dataset.s).toBe('idle');
  });

  it('a ring run out tells the core, and the missed one waits as a dot', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    vi.advanceTimersByTime(8000 - 1);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(sent).toEqual(['suggestDismiss']);
    ctl.suggest({ word: 'Marisol', missed: true });
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('hovering a missed one opens the capsule without a ring, leaving folds it back', () => {
    ctl.suggest({ word: 'Marisol', missed: true });
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().classList.contains('sgwait')).toBe(true);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('hovering a missed one while a habit card is open leaves the capsule closed', () => {
    ctl.suggest({ word: 'Marisol', missed: true });
    ctl.habitRule('В Telegram — с маленькой буквы и без точки');
    ctl.tray('newHabit');
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop().dataset.s).toBe('idle');
    expect(drop().classList.contains('sgwait')).toBe(false);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    ctl.tray(null);
    vi.advanceTimersByTime(1000);
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop().dataset.s).toBe('sugg');
  });

  it('nothing folds the capsule away; a dictation drops it locally too', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.suggest(null);
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(false);
    ctl.suggest({ word: 'Marisol', missed: true });
    ctl.apply({ phase: 'start', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(dot().classList.contains('on')).toBe(false);
  });

  it('an idle state while the capsule is open keeps it open', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.apply({ phase: 'idle', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(drop().dataset.s).toBe('sugg');
  });

  it('a hover-opened missed capsule never runs out', () => {
    ctl.suggest({ word: 'Marisol', missed: true });
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual([]);
    expect(drop().dataset.s).toBe('sugg');
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual([]);
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('hover pauses the 8 s, leaving resumes it from what is left', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    vi.advanceTimersByTime(3000);
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual([]);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(5000 - 1);
    expect(sent).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(sent).toEqual(['suggestDismiss']);
  });

  it('the 6 s after "+" waits while the mouse is on the capsule', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    btn().click();
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual(['suggestAdd']);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(6000);
    expect(sent).toEqual(['suggestAdd', 'suggestDone']);
  });

  it('the time is kept without the ring: its end alone does nothing (reduced motion has none)', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ringEnd();
    expect(sent).toEqual([]);
    expect(drop().dataset.s).toBe('sugg');
    btn().click();
    ringEnd();
    expect(sent).toEqual(['suggestAdd']);
    expect(drop().dataset.s).toBe('added');
  });

  it('a dictation stops the time of an open capsule', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.apply({ ...base, phase: 'start' });
    vi.advanceTimersByTime(60_000);
    expect(sent).toEqual([]);
  });

  it('the dot hides with the idle-hidden Drop and while the Drop steps aside', () => {
    const sheet = document.createElement('style');
    sheet.textContent = ['../src/app.css', '../src/design/drop.css'].map((f) => readFileSync(join(__dirname, f), 'utf8')).join(' ');
    document.head.append(sheet);
    try {
      ctl.suggest({ word: 'Marisol', missed: true });
      expect(getComputedStyle(dot()).opacity).not.toBe('0');
      document.body.classList.add('idle-hidden');
      expect(getComputedStyle(dot()).opacity).toBe('0');
      document.body.classList.remove('idle-hidden');
      drop().classList.add('away');
      expect(getComputedStyle(dot()).opacity).toBe('0');
    } finally {
      sheet.remove();
    }
  });

  it('the capsule hides with the idle-hidden Drop: nothing is drawn that cannot be clicked', () => {
    const sheet = document.createElement('style');
    sheet.textContent = readFileSync(join(__dirname, '../src/app.css'), 'utf8');
    document.head.append(sheet);
    try {
      for (const added of [false, true]) {
        ctl.suggest({ word: 'Marisol', missed: false });
        if (added) btn().click();
        expect(drop().dataset.s).toBe(added ? 'added' : 'sugg');
        expect(getComputedStyle(drop()).opacity).not.toBe('0');
        document.body.classList.add('idle-hidden');
        expect(getComputedStyle(drop()).opacity).toBe('0');
        expect(getComputedStyle(drop()).pointerEvents).toBe('none');
        document.body.classList.remove('idle-hidden');
      }
      ctl.apply({ ...base, phase: 'listen' });
      document.body.classList.add('idle-hidden');
      expect(getComputedStyle(drop()).opacity).not.toBe('0');
    } finally {
      sheet.remove();
    }
  });

  it('the word goes in as text', () => {
    ctl.suggest({ word: '<b>x</b>', missed: false });
    expect(drop().querySelector('.swd b')).toBeNull();
  });
});
