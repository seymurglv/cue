import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, habit, mount, settle, stub, text, view } from './windowFixture';
import { mountWindow, type WindowPage } from '../src/window/page';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const sec = () => $('.bsec[data-scr="habits"]')!;
const rows = () => $$('.habl .hw[data-w]');
const ids = () => rows().map((r) => r.dataset.w);
const DIGITS = habit('h2', { rule: 'Numbers in digits', before: 'twelve percent', after: '12 %', app: null, n: 7 });

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub(view({ habits: [habit('h1'), DIGITS] }));
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('Habits', () => {
  it('a section after the Dictionary, with its header link, count, hint and rows', async () => {
    page = await mount();
    expect($$('.bsec').map((s) => s.dataset.scr)).toEqual(['today', 'history', 'dictionary', 'habits']);
    expect(text($$('.lk button').at(-1)!)).toBe('Habits');
    expect(text(sec().querySelector('h3 em'))).toBe('2');
    expect(text(sec().querySelector('.dhint'))).toBe(
      'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
    );
    expect(ids()).toEqual(['h1', 'h2']);
    const [first, second] = rows();
    expect(text(first.querySelector('.hrule'))).toBe('In Telegram — lower case and no full stop');
    expect(text(first.querySelector('.hex s'))).toBe('Send the file.');
    expect(text(first.querySelector('.hex ins'))).toBe('send the file');
    expect(text(first.querySelector('.hmeta'))).toBe('· in Telegram · 3 times');
    expect(text(second.querySelector('.hmeta'))).toBe('· everywhere · 7 times');
    expect(second.querySelector('.hmeta svg')).not.toBeNull();
    expect(first.querySelector('.sw')!.getAttribute('aria-checked')).toBe('true');
    expect(first.querySelector('.hdel')!.getAttribute('title')).toBe('Forget');
    expect(first.querySelector('.hdel')!.getAttribute('aria-label')).toBe('Forget: In Telegram — lower case and no full stop');
  });

  it('Russian: where and how many times', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru', habits: [habit('h1'), habit('h2', { app: null, n: 5 })] }));
    page = await mount();
    expect(text($('.hw[data-w="h1"] .hmeta'))).toBe('· в Telegram · 3 раза');
    expect(text($('.hw[data-w="h2"] .hmeta'))).toBe('· везде · 5 раз');
    expect(text(sec().querySelector('h3'))).toContain('Привычки');
  });

  it('a switch turns a habit off in place, muted', async () => {
    page = await mount();
    const row = rows()[0];
    click(row.querySelector('.sw'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitOn', value: { id: 'h1', on: false } });
    expect(rows()[0]).toBe(row);
    expect(row.querySelector('.hb')!.classList.contains('off')).toBe(true);
    expect(row.querySelector('.sw')!.getAttribute('aria-checked')).toBe('false');
  });

  it('"Forget" on a row folds it away and forgets the habit', async () => {
    page = await mount();
    click(rows()[0].querySelector('.hdel'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitForget', value: 'h1' });
    expect(ids()).toEqual(['h2']);
    expect(text(sec().querySelector('h3 em'))).toBe('1');
  });

  it('a second press on the trash while the row folds away forgets the habit once', async () => {
    document.body.innerHTML = '<div id="w"></div>';
    page = await mountWindow(document.getElementById('w')!, { reduced: () => false });
    await settle();
    const del = rows()[0].querySelector('.hdel');
    click(del);
    click(del);
    await vi.advanceTimersByTimeAsync(500);
    await settle();
    expect(core.settingsSet.mock.calls.filter(([c]) => c.kind === 'habitForget')).toEqual([[{ kind: 'habitForget', value: 'h1' }]]);
    expect(ids()).toEqual(['h2']);
  });

  it('"Forget all" asks for a second press, then forgets every habit and shows the empty line', async () => {
    page = await mount();
    const b = $<HTMLButtonElement>('[data-hforget]')!;
    click(b);
    await settle();
    expect(text(b)).toBe('Press again');
    expect(core.settingsSet).not.toHaveBeenCalled();
    click(b);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitsForget' });
    expect(rows()).toEqual([]);
    expect($<HTMLElement>('.hbempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
  });

  it('the first press of "Forget all" wears off after 3 s', async () => {
    page = await mount();
    const b = $('[data-hforget]')!;
    click(b);
    await vi.advanceTimersByTimeAsync(3000);
    expect(text(b)).toBe('Forget all');
    click(b);
    expect(core.settingsSet).not.toHaveBeenCalled();
  });

  it('an empty list says what will come, without "Forget all"', async () => {
    stub(view());
    page = await mount();
    expect(text($('.hbempty'))).toBe('Nothing yet. CUE will notice when you correct the same thing a few times.');
    expect($<HTMLElement>('.hbempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
    expect(text(sec().querySelector('h3 em'))).toBe('');
  });

  it('"Learn from me" sits in the heading; off, the rows are muted', async () => {
    page = await mount();
    const sw = $<HTMLButtonElement>('.bsec[data-scr="habits"] h3 [data-sw="learn"]')!;
    expect(sw.getAttribute('role')).toBe('switch');
    expect(sw.getAttribute('aria-checked')).toBe('true');
    click(sw);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'learn', value: false });
    expect(sw.getAttribute('aria-checked')).toBe('false');
    expect($('.habl')!.classList.contains('paused')).toBe(true);
  });

  it('settings the core sends: a new habit grows in with the highlight, one undone from the card goes', async () => {
    page = await mount();
    feeds.settings(view({ habits: [habit('h3', { rule: 'No full stop' }), habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h3', 'h1', 'h2']);
    expect(rows()[0].querySelector('.hb')!.classList.contains('new')).toBe(true);
    expect(rows()[1].querySelector('.hb')!.classList.contains('new')).toBe(false);
    feeds.settings(view({ habits: [habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h1', 'h2']);
  });

  it('a habit that widens takes the id of everywhere: its row changes in place', async () => {
    page = await mount();
    const row = rows()[0];
    feeds.settings(view({ habits: [habit('h9', { rule: 'Lower case and no full stop', app: null, n: 6 }), DIGITS] }));
    expect(ids()).toEqual(['h9', 'h2']);
    expect(rows()[0]).toBe(row);
    expect(row.querySelector('.hb')!.classList.contains('new')).toBe(false);
    expect(text(row.querySelector('.hrule'))).toBe('Lower case and no full stop');
    expect(text(row.querySelector('.hmeta'))).toBe('· everywhere · 6 times');
    click(row.querySelector('.sw'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitOn', value: { id: 'h9', on: false } });
    // A habit of everywhere with another example is a new one: the gone row folds away.
    feeds.settings(view({ habits: [habit('h8', { before: 'Call me.', after: 'call me', app: null }), DIGITS] }));
    expect(ids()).toEqual(['h8', 'h2']);
    expect(rows()[0]).not.toBe(row);
  });

  it('with the cleanup off (no AI), a line says habits need it', async () => {
    stub(view({ refineWhere: 'off' }));
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(false);
    expect(text($('.hnocl'))).toBe('Habits work when the cleanup is on.');
  });

  it('the user\'s own AI or this computer: no such line, and no key is asked for', async () => {
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
    for (const refineWhere of ['local', 'off', 'agent'] as const) {
      feeds.settings(view({ habits: [habit('h1'), DIGITS], refineWhere }));
      expect($<HTMLElement>('.hnocl')!.hidden).toBe(refineWhere !== 'off');
    }
    window.dispatchEvent(new Event('focus'));
    await settle();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
    expect('keyState' in core).toBe(false);
  });
});
