import { describe, expect, it, beforeEach, afterEach, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { DropController, type UiState, type UiAction, type Rect } from '../src/controller';
import { trayHtml } from '../src/trays';
import { DICTS, KEY_NAME, TRAYS, en, pickLang, ru, type TrayKind } from '../src/strings';

const base: UiState = { phase: 'idle', error: null, retry: false, paused: false };
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
const KINDS: TrayKind[] = ['noField', 'micSilent', 'holdHint', 'lastMinute'];

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
    expect(en.error).toEqual({ net: 'No connection', sonioxKey: 'Soniox key problem', claudeKey: 'Claude key problem' });
    expect(en.close).toBe('Close');
    expect(en.drop).toEqual({
      cancel: 'Cancel · Esc', cancelName: 'Cancel', restore: 'Restore', retry: 'Retry', done: 'Done', cancelled: 'Cancelled', ask: 'Ask',
    });
  });

  it('language from the core: "ru" is Russian, anything else is English', () => {
    expect(pickLang('ru')).toBe('ru');
    expect(pickLang('en')).toBe('en');
    for (const v of [undefined, null, '', 'RU', 'ru-RU', 'uk', 'de', 1, {}]) expect(pickLang(v), String(v)).toBe('en');
  });

  it('the controller is English by default', () => {
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
    hover(true);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('No connection');
    expect(document.querySelector('.drop .ib.soft')!.getAttribute('title')).toBe('Cancel · Esc');
    expect(document.querySelector('.drop .ib.soft')!.getAttribute('aria-label')).toBe('Cancel');
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
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
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
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('No connection');
  });

  it('Soniox key error: no cause icon, with "retry"', () => {
    ctl.apply({ ...base, phase: 'error', error: 'sonioxKey', retry: true });
    expect(shown()).toEqual(['mk', 'retry']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk', 'retry']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Soniox key problem');
  });

  it('Claude key error: the mark only; on hover the cross and the cause in words', () => {
    ctl.apply({ ...base, phase: 'error', error: 'claudeKey', retry: false });
    expect(shown()).toEqual(['mk']);
    hover(true);
    expect(shown()).toEqual(['bx', 'lab', 'mk']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Claude key problem');
  });

  it('error → error with another cause (network → Claude key) redraws the icon, "retry" and the label', () => {
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('No connection');
    ctl.apply({ ...base, phase: 'error', error: 'claudeKey', retry: false });
    expect(shown()).toEqual(['mk']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Claude key problem');
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
    expect(shown()).toEqual(['mk', 'retry', 'why']);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('No connection');
  });
});

describe('Recording: a 28 px mark in a round Drop (with drop.css)', () => {
  // jsdom has no layout, but it does cascade a style sheet: enough to check the sizes the CSS gives each state.
  const css = readFileSync(join(__dirname, '../src/design/drop.css'), 'utf8');
  let sheet: HTMLStyleElement;
  beforeEach(() => {
    sheet = document.createElement('style');
    sheet.textContent = css;
    document.head.append(sheet);
  });
  afterEach(() => sheet.remove());
  const drop = () => document.querySelector('.drop') as HTMLElement;
  const mark = () => document.querySelector('.drop .mkw') as HTMLElement;
  const px = (v: string) => parseFloat(v);

  it('the mark is 24 px, 28 px while recording, and its max-width follows that size at once', () => {
    const sizes: Record<string, [string, string]> = {};
    for (const phase of ['start', 'listen', 'think', 'done', 'listen', 'cancel', 'listen', 'error', 'empty'] as const) {
      ctl.apply({ ...base, phase, error: phase === 'error' ? 'net' : null });
      sizes[phase] = [getComputedStyle(drop()).getPropertyValue('--mk'), mark().style.maxWidth];
    }
    expect(sizes).toEqual({
      start: ['24px', '26px'], listen: ['28px', '30px'], think: ['24px', '26px'], done: ['24px', '26px'],
      cancel: ['24px', '26px'], error: ['24px', '26px'], empty: ['24px', '26px'],
    });
  });

  it("the mark's max-width is its size in the new state, not its current (still animating) width", () => {
    ctl.apply({ ...base, phase: 'listen' });
    // jsdom measures every element as 0 wide: give the mark the width it has right after recording ends.
    Object.defineProperty(mark(), 'scrollWidth', { configurable: true, get: () => 28 });
    ctl.apply({ ...base, phase: 'think' });
    expect(mark().style.maxWidth).toBe('26px');
    ctl.apply({ ...base, phase: 'listen' });
    expect(mark().style.maxWidth).toBe('30px');
  });

  it('while recording the Drop is a circle: its height is the mark plus padding and margin on both sides', () => {
    ctl.apply({ ...base, phase: 'listen' });
    const d = getComputedStyle(drop()), m = getComputedStyle(mark());
    const width = px(d.getPropertyValue('--mk')) + px(d.paddingLeft) + px(d.paddingRight) + px(m.marginLeft) + px(m.marginRight);
    expect(px(d.height)).toBe(44);
    expect(width).toBe(44);
  });

  it('the other mark-only states stay 40 px circles with the 24 px mark', () => {
    for (const phase of ['start', 'think', 'done', 'empty'] as const) {
      ctl.apply({ ...base, phase });
      const d = getComputedStyle(drop()), m = getComputedStyle(mark());
      const width = px(d.getPropertyValue('--mk')) + px(d.paddingLeft) + px(d.paddingRight) + px(m.marginLeft) + px(m.marginRight);
      expect([px(d.height), width], phase).toEqual([40, 40]);
    }
  });

  it("the mark's size and the Drop's height change with the same curve, so the Drop is round during the change too", () => {
    ctl.apply({ ...base, phase: 'listen' });
    const svg = getComputedStyle(mark().querySelector('svg')!);
    expect([svg.width, svg.height]).toEqual(['var(--mk)', 'var(--mk)']);
    const height = /(?:^|,)\s*height ([^,]+)/;
    const curve = getComputedStyle(drop()).transition.match(height)?.[1];
    expect(curve).toBe('.45s var(--e-spring)');
    expect(svg.transition).toContain(`width ${curve}`);
    expect(svg.transition.match(height)?.[1]).toBe(curve);
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
    stub(document.querySelector('.drop')!, 163, 232, 34, 8);
    ctl.frame(0);
    ctl.frame(16);
    expect(rectsSent).toEqual([[{ x: 163, y: 232, w: 34, h: 8 }]]);
    ctl.idleHidden(true);
    ctl.frame(32);
    expect(rectsSent.at(-1)).toEqual([]);
    ctl.apply({ ...base, phase: 'listen' });
    ctl.frame(48);
    expect(rectsSent.at(-1)).toEqual([{ x: 163, y: 232, w: 34, h: 8 }]);
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
    stub(document.querySelector('.drop')!, 163, 232, 34, 8);
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

  it('grows out of the Drop\'s own shape and folds back into it: the 44 px recording circle too', () => {
    vi.useFakeTimers();
    try {
      const d = document.querySelector('.drop') as HTMLElement;
      const box = document.querySelector('.tray') as HTMLElement;
      const size = (w: number, h: number) => {
        Object.defineProperty(d, 'offsetWidth', { configurable: true, value: w });
        Object.defineProperty(d, 'offsetHeight', { configurable: true, value: h });
      };
      for (const [w, h] of [[44, 44], [40, 40], [116, 44]]) {
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
        box.innerHTML = trayHtml(kind, t);
        expect(box.querySelector('.nhd b')!.textContent, `${lang}.${kind}`).toBe(t.tray[kind].title);
        expect(box.querySelector('.xb')!.getAttribute('aria-label'), `${lang}.${kind}`).toBe(t.close);
        expect(box.querySelector('.nbd')!.innerHTML, `${lang}.${kind}`).toBe(t.tray[kind].body.replace('{KEY}', `<kbd>${KEY_NAME}</kbd>`));
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
    expect([...document.querySelectorAll('.drop .ib.solid')].map((e) => e.getAttribute('title'))).toEqual(['Вернуть', 'Повторить', 'Готово']);
    ctl.apply({ ...base, phase: 'cancel' });
    hover(true);
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Отменено');
    ctl.apply({ ...base, phase: 'error', error: 'sonioxKey', retry: true });
    expect(document.querySelector('.drop .lab')!.getAttribute('data-t')).toBe('Проблема с ключом Soniox');
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
    ctl.apply({ ...base, phase: 'error', error: 'net', retry: true });
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
    stub(d, 163, 232, 34, 8); // the transition has ended
    expect(ctl.frame(1000)).toBe(false);
    expect(rectsSent.at(-1)).toEqual([{ x: 163, y: 232, w: 34, h: 8 }]);
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
