import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Answer, Turn } from '../src/answer';
import { CARD_FOLD_MS, CardPage } from '../src/cardPage';
import type { Rect, UiState } from '../src/controller';

const base: UiState = { phase: 'idle', error: null, retry: false, paused: false, mode: 'ask' };
const turn = (over: Partial<Turn> = {}): Turn => ({ question: 'Explain this', step: null, segments: [], state: 'pending', ...over });
const card = (over: Partial<Answer> = {}): Answer => ({
  place: 'card',
  quote: 'Can you send the report by Friday?',
  turns: [turn()],
  sources: [],
  listening: false,
  missed: false,
  actions: ['below', 'replace', 'copy'],
  ...over,
});
const done = () =>
  card({
    turns: [turn({ state: 'done', segments: [{ text: 'Short answer.', cites: [1] }] })],
    sources: [{ n: 1, site: 'rfc-editor.org', title: 'RFC 2324' }],
  });

let acts: string[];
let sources: number[];
let rects: Rect[][];
let page: CardPage;
const el = () => document.querySelector('.ansC') as HTMLElement;

beforeEach(() => {
  vi.useFakeTimers();
  document.body.innerHTML = '<div id="card"></div>';
  acts = [];
  sources = [];
  rects = [];
  page = new CardPage(
    document.getElementById('card')!,
    { act: (a) => acts.push(a), source: (n) => sources.push(n), rects: (r) => rects.push(r), wake: () => {} },
    'en',
  );
  el().getBoundingClientRect = () => ({ x: 24, y: 30, width: 390, height: 180, left: 24, top: 30, right: 414, bottom: 210, toJSON() {} }) as DOMRect;
});

afterEach(() => {
  vi.useRealTimers();
});

describe('The card at the selection', () => {
  it('shows only an answer meant for it', () => {
    expect(el().hidden).toBe(true);
    page.answer(card({ place: 'tray' }));
    expect(el().hidden).toBe(true);
    page.answer(card());
    expect(el().hidden).toBe(false);
    expect(el().querySelector('.aq')!.textContent).toContain('Explain this');
  });

  it('stays asleep while an answer in the tray streams', () => {
    let wakes = 0;
    page = new CardPage(document.getElementById('card')!, { act: () => {}, source: () => {}, rects: () => {}, wake: () => wakes++ }, 'en');
    page.answer(card({ place: 'tray' }));
    page.answer(card({ place: 'tray', turns: [turn({ state: 'streaming', segments: [{ text: 'More', cites: [] }] })] }));
    page.answer(null);
    expect(wakes).toBe(0);
  });

  it('points its arrow and limits its height where the core placed it', () => {
    page.place({ below: false, arrowX: 72, maxHeight: 180 });
    expect(el().classList.contains('above')).toBe(true);
    expect(el().style.getPropertyValue('--ax')).toBe('72px');
    expect(el().style.getPropertyValue('--max-h')).toBe('180px');
    page.place({ below: true, arrowX: 60, maxHeight: 300 });
    expect(el().classList.contains('above')).toBe(false);
  });

  it('takes the mouse only over the card while it is open', () => {
    page.frame(0);
    expect(rects.at(-1)).toEqual([]);
    page.answer(card());
    page.frame(16);
    expect(rects.at(-1)).toEqual([{ x: 24, y: 30, w: 390, h: 180 }]);
    page.answer(null);
    page.frame(32);
    expect(rects.at(-1)).toEqual([]);
  });

  it('keeps reporting its rectangle while it pops in, even for an answer that shows whole at once', () => {
    // A refusal that came before anything showed: nothing moves in the view, but the card still grows to its size.
    let r = { x: 27, y: 24, width: 371, height: 171 };
    el().getBoundingClientRect = () => ({ ...r, left: r.x, top: r.y, right: r.x + r.width, bottom: r.y + r.height, toJSON() {} }) as DOMRect;
    page.answer(card({ turns: [turn({ state: 'refused' })], actions: [] }));
    expect(page.frame(0)).toBe(true);
    r = { x: 24, y: 30, width: 390, height: 180 };
    expect(page.frame(460)).toBe(true);
    expect(rects.at(-1)).toEqual([{ x: 24, y: 30, w: 390, h: 180 }]);
    expect(page.frame(1_000)).toBe(false);
  });

  it('folds away and then empties; a new answer meanwhile keeps it', () => {
    page.answer(card());
    page.answer(null);
    expect(el().classList.contains('out')).toBe(true);
    vi.advanceTimersByTime(CARD_FOLD_MS);
    expect(el().hidden).toBe(true);
    expect(el().querySelector('.in')!.childElementCount).toBe(0);
    page.answer(card());
    page.answer(null);
    page.answer(card());
    vi.advanceTimersByTime(CARD_FOLD_MS);
    expect([el().hidden, el().classList.contains('out')]).toEqual([false, false]);
    expect(el().querySelector('.in')!.childElementCount).toBeGreaterThan(0);
  });

  it('a new answer while the last one folds starts afresh: its words are revealed anew', () => {
    page.answer(done());
    for (let t = 0; t < 2_000; t += 60) page.frame(t);
    expect(el().querySelectorAll('.aa .w.on').length).toBeGreaterThan(0);
    page.answer(null);
    page.answer(card({ turns: [turn({ question: 'And this?', state: 'done', segments: [{ text: 'Another one.', cites: [] }] })] }));
    expect(el().querySelectorAll('.aa .w.on')).toHaveLength(0);
    expect(el().querySelector('.aq')!.textContent).toContain('And this?');
  });

  it('its buttons and source chips reach the core', () => {
    page.apply({ ...base, phase: 'answer' });
    page.answer(done());
    for (let t = 0; t < 2_000; t += 60) page.frame(t);
    (el().querySelector('[data-cmd="answerBelow"]') as HTMLButtonElement).click();
    (el().querySelector('.sc') as HTMLElement).click();
    expect(acts).toEqual(['answerBelow']);
    expect(sources).toEqual([1]);
  });

  it('its buttons and cross wait outside the core answer phase, as in the tray', () => {
    page.answer(done());
    for (let t = 0; t < 2_000; t += 60) page.frame(t);
    const all = () => [...el().querySelectorAll<HTMLButtonElement>('[data-x], .aft button')];
    expect(all().every((b) => b.disabled)).toBe(true);
    for (const b of all()) b.click();
    expect(acts).toEqual([]);
    page.apply({ ...base, phase: 'answer' });
    expect(all().some((b) => b.disabled)).toBe(false);
    page.apply({ ...base, phase: 'listen' });
    expect(all().every((b) => b.disabled)).toBe(true);
    page.apply({ ...base, phase: 'answer' });
    (el().querySelector('[data-x]') as HTMLButtonElement).click();
    expect(acts).toEqual(['answerClose']);
  });

  it('a chord while the answer comes shakes its head', () => {
    page.shake();
    page.answer(card());
    page.shake();
    expect(el().querySelector('.ah')!.classList.contains('shake')).toBe(true);
  });
});
