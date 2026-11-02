import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, YESTERDAY, click, core, entry, feeds, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import type { Stats } from '../src/window/api';
import { MILESTONE_KEY, REFRESH_MS } from '../src/window/today';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

/** 30 days ending today: day i has i × 100 words, today 2 340. */
function stats(over: Partial<Stats> = {}): Stats {
  const days = Array.from({ length: 30 }, (_, i) => {
    const d = new Date(2026, 9, 31 - (29 - i));
    const date = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
    return { date, words: i === 29 ? 2340 : i * 100 };
  });
  return { todayWords: 2340, savedMinutes: 59, days, wpm: 140, streak: 6, total: 48210, milestone: null, ...over };
}

let page: WindowPage | null = null;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
  core.statsGet.mockResolvedValue(stats());
  localStorage.clear();
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The header', () => {
  it('opens with the dictation mark: a wave of three bars and the red lamp, before the word CUE', async () => {
    page = await mount();
    const logo = $('.bh .logo')!;
    expect(text(logo)).toBe('CUE');
    const svg = logo.querySelector('svg')!;
    expect(svg.getAttribute('aria-hidden')).toBe('true');
    expect(svg.querySelectorAll('.wv line')).toHaveLength(3);
    expect(svg.querySelectorAll('circle.lamp')).toHaveLength(1);
    expect(svg.querySelector('.m-l, .m-dot')).toBeNull();
    expect(logo.firstChild).toBe(svg);
  });
});

describe('Today', () => {
  it('says the day in one sentence, 30 bars and a quiet line', async () => {
    page = await mount();
    const lede = $('.lede')!;
    expect(text(lede)).toBe("Today — 2,340 words.That's 59 minutes without the keyboard.");
    expect($$('.lede .num').map((n) => n.textContent)).toEqual(['2,340', '59']);
    const bars = $$('.m30 i');
    expect(bars).toHaveLength(30);
    expect(bars[29].classList.contains('now')).toBe(true);
    // The highest day (2 800) is full height; today's 2 340 against it.
    expect(bars[28].style.height).toBe('42px');
    expect(bars[29].style.height).toBe('35px');
    expect(bars[0].style.height).toBe('2px');
    expect(text($('.bstat'))).toBe('140 words a minute·6 days in a row·48,210 words in all');
  });

  it('hides the parts it has nothing for', async () => {
    core.statsGet.mockResolvedValue(stats({ wpm: null, streak: 0, total: 0, todayWords: 1, savedMinutes: 0 }));
    page = await mount();
    expect(text($('.lede'))).toBe("Today — 1 word.That's 0 minutes without the keyboard.");
    expect($('.bstat')!.hidden).toBe(true);
  });

  it('writes Russian numbers and word forms', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    core.statsGet.mockResolvedValue(stats({ todayWords: 21, savedMinutes: 2, wpm: 132, streak: 22, total: 2340 }));
    page = await mount();
    expect(text($('.lede'))).toBe('Сегодня — 21 слово.Это 2 минуты без клавиатуры.');
    expect(text($('.bstat'))).toMatch(/^132 слова в минуту·22 дня подряд·2\s340 слов за всё время$/);
  });

  it('names a bar\'s day and words over it', async () => {
    page = await mount();
    $$('.m30 i')[29].dispatchEvent(new MouseEvent('pointerover', { bubbles: true }));
    const tip = $('.tip')!;
    expect(tip.classList.contains('on')).toBe(true);
    expect(tip.querySelector('b')!.textContent).toBe('2,340');
    expect(text(tip)).toBe('2,340Sat, October 31');
    $('.m30')!.dispatchEvent(new MouseEvent('pointerleave'));
    expect(tip.classList.contains('on')).toBe(false);
  });

  it('marks a new milestone once and keeps its pill for the day', async () => {
    core.statsGet.mockResolvedValue(stats({ total: 1200, milestone: 1000 }));
    page = await mount();
    expect(text($('.ms'))).toBe('1,000 words — about 4 pages of a book');
    expect(JSON.parse(localStorage.getItem(MILESTONE_KEY)!)).toEqual({ n: 1000, date: TODAY });
    page.dispose();
    page = await mount();
    expect($('.ms')).not.toBeNull();
    localStorage.setItem(MILESTONE_KEY, JSON.stringify({ n: 1000, date: YESTERDAY }));
    page.dispose();
    page = await mount();
    expect($('.ms')).toBeNull();
  });

  it('asks again a moment after a new entry', async () => {
    page = await mount();
    expect(core.statsGet).toHaveBeenCalledTimes(1);
    core.statsGet.mockResolvedValue(stats({ todayWords: 2400 }));
    feeds.entry(entry(`${TODAY}T12:30:00.000-1`, 'New.'));
    feeds.entry(entry(`${TODAY}T12:30:01.000-1`, 'Newer.'));
    await vi.advanceTimersByTimeAsync(REFRESH_MS);
    await settle();
    expect(core.statsGet).toHaveBeenCalledTimes(2);
    expect($('.lede .num')!.textContent).toBe('2,400');
  });
});

describe('The header', () => {
  it('links to the sections and scrolls to them', async () => {
    const scrollTo = vi.fn();
    HTMLElement.prototype.scrollTo = scrollTo as never;
    page = await mount();
    const links = $$('.bh .lk button');
    expect(links.map((l) => l.textContent)).toEqual(['Today', 'History', 'Dictionary']);
    click(links[1]);
    expect(scrollTo).toHaveBeenCalledTimes(1);
    click(links[0]);
    expect(scrollTo).toHaveBeenLastCalledWith({ top: 0, behavior: 'auto' });
    expect(links.filter((l) => l.getAttribute('aria-current') === 'true')).toHaveLength(1);
  });
});
