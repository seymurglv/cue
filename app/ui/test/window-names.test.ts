// What a screen reader hears in the main window: every button says what it acts on, and every container that holds
// controls has a name or stays out of the way.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, YESTERDAY, click, core, entry, mount, settle, stub, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import { excerpt } from '../src/window/text';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

const LONG = 'A long dictation that goes on well past forty characters before it ends.';
const morning = entry(`${TODAY}T09:15:02.120-1`, 'Morning standup notes.', { app: 'Notepad' });
const long = entry(`${TODAY}T08:02:40.000-1`, LONG);
const evening = entry(`${YESTERDAY}T18:47:00.000-1`, 'Retro moves to Friday.');

let page: WindowPage | null = null;
const row = (id: string) => $$('.hr').find((r) => r.dataset.id === id)!;
const label = (e: Element | null) => e?.getAttribute('aria-label');
const role = (e: Element | null) => e?.getAttribute('role');

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('An excerpt of an entry', () => {
  it('is its first forty characters on one line, cut with an ellipsis', () => {
    expect(excerpt('Short one.')).toBe('Short one.');
    expect(excerpt('  two\n\nlines\there  ')).toBe('two lines here');
    expect(excerpt(LONG)).toBe('A long dictation that goes on well past…');
    expect(excerpt('x'.repeat(40))).toBe('x'.repeat(40));
    // Characters, not UTF-16 units: an emoji is not cut in half.
    expect(excerpt('😀'.repeat(41))).toBe(`${'😀'.repeat(40)}…`);
  });
});

describe('Names in English', () => {
  beforeEach(() => {
    stub(view({ dictionary: ['Soniox', 'GitHub'] }));
    core.historyPage.mockResolvedValue({
      days: [
        { date: TODAY, entries: [morning, long] },
        { date: YESTERDAY, entries: [evening] },
      ],
      more: false,
    });
  });

  it('copy and paste again name the entry they act on; their tooltips stay short', async () => {
    page = await mount();
    const copy = row(morning.id).querySelector('[data-copy]');
    const paste = row(morning.id).querySelector('[data-paste]');
    expect(label(copy)).toBe('Copy: Morning standup notes.');
    expect(label(paste)).toBe('Paste again: Morning standup notes.');
    expect(copy!.getAttribute('title')).toBe('Copy');
    expect(paste!.getAttribute('title')).toBe('Paste again');
    expect(label(row(long.id).querySelector('[data-copy]'))).toBe('Copy: A long dictation that goes on well past…');
    expect(label(row(long.id).querySelector('[data-paste]'))).toBe('Paste again: A long dictation that goes on well past…');
  });

  it('the name stays while the button shows its check', async () => {
    page = await mount();
    const copy = row(morning.id).querySelector<HTMLElement>('[data-copy]')!;
    click(copy);
    await settle();
    expect(copy.classList.contains('done')).toBe(true);
    expect(label(copy)).toBe('Copy: Morning standup notes.');
  });

  it("a dictionary word's delete button names the word", async () => {
    page = await mount();
    const xs = $$('.dch .dx');
    expect(xs.map(label)).toEqual(['Delete the word Soniox', 'Delete the word GitHub']);
    expect(xs[0].getAttribute('title')).toBe('Remove');
  });

  it('the dictionary is a list of its words', async () => {
    page = await mount();
    expect(role($('.dflow'))).toBe('list');
    expect($$('.dch').map(role)).toEqual(['listitem', 'listitem']);
  });

  it('the sections, the history list, its days and the settings are named', async () => {
    page = await mount();
    expect(label($('[data-scr=today]'))).toBe('Today');
    expect(label($('[data-scr=history]'))).toBe('History');
    expect(label($('[data-scr=dictionary]'))).toBe('Dictionary');
    expect(role($('.bhist'))).toBe('group');
    expect(label($('.bhist'))).toBe('History entries');
    expect($$('.hgrp').map(role)).toEqual(['group', 'group']);
    expect($$('.hgrp').map(label)).toEqual(['Today', 'Yesterday']);
    expect(label($('nav.lk'))).toBe('Sections');
    // The scroller holds the whole page and stays in a screen reader's tree as it scrolls: it is named.
    expect(role($('.bsc'))).toBe('main');
    expect(label($('.bsc'))).toBe('Main window');
    expect(role($('.btray'))).toBe('region');
    expect(label($('.btray'))).toBe('Settings');
  });

  it('an entry is named by its time and its words', async () => {
    page = await mount();
    expect(role(row(morning.id))).toBe('group');
    expect(label(row(morning.id))).toBe('09:15, Morning standup notes.');
    expect(label(row(long.id))).toBe('08:02, A long dictation that goes on well past…');
  });

  it('wrappers that only lay things out stay out of the way', async () => {
    page = await mount();
    for (const sel of ['#w', 'header.bh', '.bcol', '.hsb', '.btray .bin', '.hgrp > .hl']) {
      expect(role($(sel)), sel).toBe('none');
    }
  });

  it("each of the settings tray's pages is named by its title; its frames stay out of the way", async () => {
    page = await mount();
    click($('.gear'));
    await settle();
    const shown = () => $$('.tpb > div').filter((p) => !p.hidden).at(-1)!;
    expect(role(shown())).toBe('group');
    expect(label(shown())).toBe('Settings');
    click($('[data-sub=mic]'));
    await vi.advanceTimersByTimeAsync(1000);
    expect(label(shown())).toBe('Microphone');
    for (const sel of ['.tpg', '.tph', '.tpb', '.sgrp']) expect(role($(sel)), sel).toBe('none');
  });

  it('a stack and its hidden part stay out of the way too', async () => {
    const burst = [0, 1, 2].map((i) =>
      entry(`${TODAY}T10:0${5 - i}:00.000-1`, `Telegram note ${i}.`, { app: 'Telegram' }),
    );
    core.historyPage.mockResolvedValue({ days: [{ date: TODAY, entries: burst }], more: false });
    page = await mount();
    expect(role($('.hstk'))).toBe('none');
    expect(role($('.hrest'))).toBe('none');
    expect(role($('.hrest > div'))).toBe('none');
  });

  it('a folded day and the rest of today stay out of the way', async () => {
    const many = Array.from({ length: 9 }, (_, i) => entry(`${TODAY}T0${9 - i}:00:00.000-1`, `Note ${i}.`));
    core.historyPage.mockResolvedValue({
      days: [
        { date: TODAY, entries: many },
        { date: YESTERDAY, entries: [evening] },
      ],
      more: false,
    });
    page = await mount();
    for (const f of $$('.hfold')) {
      expect(role(f)).toBe('none');
      expect(role(f.firstElementChild)).toBe('none');
    }
  });
});

describe('Names in Russian', () => {
  beforeEach(() => {
    stub(view({ lang: 'ru', uiLanguage: 'ru', dictionary: ['Soniox'] }));
    core.historyPage.mockResolvedValue({ days: [{ date: TODAY, entries: [morning, long] }], more: false });
  });

  it('every new name comes in Russian', async () => {
    page = await mount();
    expect(label(row(morning.id).querySelector('[data-copy]'))).toBe('Копировать: Morning standup notes.');
    expect(label(row(morning.id).querySelector('[data-paste]'))).toBe('Вставить снова: Morning standup notes.');
    expect(label($('.dch .dx'))).toBe('Удалить слово Soniox');
    expect(label($('[data-scr=today]'))).toBe('Сегодня');
    expect(label($('[data-scr=history]'))).toBe('История');
    expect(label($('[data-scr=dictionary]'))).toBe('Словарь');
    expect(label($('.bhist'))).toBe('Записи истории');
    expect(label($('.hgrp'))).toBe('Сегодня');
    expect(label($('nav.lk'))).toBe('Разделы');
    expect(label($('.bsc'))).toBe('Главное окно');
    expect(label($('.btray'))).toBe('Настройки');
  });

  it('a word with "$&" in it is named as it is written', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru', dictionary: ['A$&B'] }));
    page = await mount();
    expect(label($('.dch .dx'))).toBe('Удалить слово A$&B');
  });
});
