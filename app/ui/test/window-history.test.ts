import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, YESTERDAY, click, core, entry, feeds, mount, settle, stub, text, type, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

const morning = entry(`${TODAY}T09:15:02.120-1`, 'Morning standup notes.', { app: 'Notepad' });
const answer = entry(`${TODAY}T08:02:40.000-1`, 'Swarm is simpler.', { mode: 'ask', question: 'Swarm or Kubernetes?' });
const evening = entry(`${YESTERDAY}T18:47:00.000-1`, 'Retro moves to Friday.', { mode: 'translation', app: 'Mail' });
const firstPage = (more = false) => ({
  days: [
    { date: TODAY, entries: [morning, answer] },
    { date: YESTERDAY, entries: [evening] },
  ],
  more,
});
const rows = () => $$('.hr');
const rowTexts = () => rows().map((r) => r.querySelector('.htx')!.textContent);

let page: WindowPage | null = null;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
  core.historyPage.mockResolvedValue(firstPage());
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The history', () => {
  it('groups entries under Today and Yesterday', async () => {
    page = await mount();
    // The channel is open before the first page is asked for.
    expect(core.historySubscribe.mock.invocationCallOrder[0]).toBeLessThan(core.historyPage.mock.invocationCallOrder[0]);
    expect(core.historyPage).toHaveBeenCalledWith(null);
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Today', 'Yesterday']);
    const today = $$('.hgrp')[0];
    expect([...today.querySelectorAll('.ht')].map((t) => t.textContent)).toEqual(['09:15', '08:02']);
    // App and question muted before the text; the mode as a badge.
    const first = rows()[0];
    expect(first.querySelector('.happ')!.textContent).toBe('Notepad');
    expect(first.querySelector('.hmi')).not.toBeNull();
    const ask = rows()[1];
    expect(ask.querySelector('.aski')).not.toBeNull();
    expect(ask.querySelector('.hq')!.textContent).toBe('Swarm or Kubernetes?');
    expect(ask.querySelector('.htx')!.getAttribute('title')).toBe('Swarm or Kubernetes?\n\nSwarm is simpler.');
    expect(first.querySelector('.htx')!.getAttribute('title')).toBe('Morning standup notes.');
    // Nothing earlier: no "Show earlier", no empty note.
    expect(text($('.bmore button'))).toBe('Show earlier');
    expect($('.bmore button')!.hidden).toBe(true);
    expect($('.hempty')!.hidden).toBe(true);
  });

  it('says so when there is nothing yet', async () => {
    core.historyPage.mockResolvedValue({ days: [], more: false });
    page = await mount();
    expect(rows()).toHaveLength(0);
    expect($('.hempty')!.hidden).toBe(false);
  });

  it('escapes entry text', async () => {
    const evil = '<img src=x onerror="window.pwned=1">';
    core.historyPage.mockResolvedValue({
      days: [
        {
          date: TODAY,
          entries: [entry(`${TODAY}T10:00:00.000-1`, `${evil} & more`, { mode: 'ask', app: '<b>App</b>', question: '<i>q</i>' })],
        },
      ],
      more: false,
    });
    core.historySearch.mockResolvedValue([
      { date: TODAY, entries: [entry(`${TODAY}T10:00:00.000-1`, `${evil} & more`, { app: '<b>App</b>' })] },
    ]);
    page = await mount();
    const htx = $('.hr .htx')!;
    expect(htx.querySelector('img, b, i')).toBeNull();
    expect(htx.textContent).toBe(`<b>App</b><i>q</i>${evil} & more`);
    expect(htx.getAttribute('title')).toBe(`<i>q</i>\n\n${evil} & more`);
    // Found text is marked, still as text.
    click($('h3 .ibt'));
    type($<HTMLInputElement>('.srch input')!, 'onerror');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    const found = $('.hr .htx')!;
    expect(found.querySelector('img, b')).toBeNull();
    expect(found.querySelector('mark')!.textContent).toBe('onerror');
    expect(found.textContent).toBe(`<b>App</b>${evil} & more`);
    expect((window as any).pwned).toBeUndefined();
  });

  it('search waits for a pause and clearing restores the list', async () => {
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [morning] }]);
    page = await mount();
    expect(rows()).toHaveLength(3);
    click($('h3 .ibt'));
    expect($('.bsr')!.classList.contains('open')).toBe(true);
    const input = $<HTMLInputElement>('.srch input')!;
    type(input, 'stand');
    await vi.advanceTimersByTimeAsync(100);
    type(input, 'standup');
    await vi.advanceTimersByTimeAsync(100);
    expect(core.historySearch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60);
    await settle();
    expect(core.historySearch).toHaveBeenCalledTimes(1);
    expect(core.historySearch).toHaveBeenCalledWith('standup');
    expect(rowTexts()).toEqual(['NotepadMorning standup notes.']);
    expect($('.hr mark')!.textContent).toBe('standup');
    expect($('.bmore button')!.hidden).toBe(true);
    // Nothing found says so.
    core.historySearch.mockResolvedValue([]);
    type(input, 'zebra');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect(rows()).toHaveLength(0);
    expect(text($('.hempty'))).toBe('Nothing found');
    // Cleared: the ordinary list at once, no search.
    type(input, '');
    expect(rows()).toHaveLength(3);
    expect($('.hr mark')).toBeNull();
    expect($('.hempty')!.hidden).toBe(true);
    await vi.advanceTimersByTimeAsync(500);
    expect(core.historySearch).toHaveBeenCalledTimes(2);
    // Closing the search from its icon clears it too.
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [morning] }]);
    type(input, 'standup');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect(rows()).toHaveLength(1);
    click($('h3 .ibt'));
    expect($('.bsr')!.classList.contains('open')).toBe(false);
    expect(input.value).toBe('');
    expect(rows()).toHaveLength(3);
  });

  it('search marks ё and е alike', async () => {
    const report = entry(`${TODAY}T11:00:00.000-1`, 'Отчёт готов, отчет отправлен.');
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [report] }]);
    page = await mount();
    click($('h3 .ibt'));
    const input = $<HTMLInputElement>('.srch input')!;
    type(input, 'отчет');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect($('.hr mark')!.textContent).toBe('Отчёт');
    expect($('.hr .htx')!.textContent).toBe('Отчёт готов, отчет отправлен.');
    type(input, 'ОТЧЁТ ОТПР');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect($('.hr mark')!.textContent).toBe('отчет отпр');
  });

  it('show earlier appends the next page and hides without more', async () => {
    core.historyPage.mockResolvedValueOnce(firstPage(true));
    const older = entry('2026-10-24T21:12:00.000-1', 'An idea for an article.');
    core.historyPage.mockResolvedValueOnce({ days: [{ date: '2026-10-24', entries: [older] }], more: false });
    page = await mount();
    const earlier = $$('.bmore button')[0];
    expect(earlier.hidden).toBe(false);
    click(earlier);
    await settle();
    expect(core.historyPage).toHaveBeenLastCalledWith(YESTERDAY);
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Today', 'Yesterday', 'Saturday, October 24']);
    expect(rowTexts().at(-1)).toBe('An idea for an article.');
    expect(earlier.hidden).toBe(true);
  });

  it('asks for days before yesterday when today and yesterday are empty', async () => {
    core.historyPage.mockResolvedValueOnce({ days: [], more: true });
    core.historyPage.mockResolvedValueOnce({ days: [{ date: '2026-10-20', entries: [morning] }], more: true });
    page = await mount();
    expect($('.hempty')!.hidden).toBe(true);
    click($$('.bmore button')[0]);
    await settle();
    expect(core.historyPage).toHaveBeenLastCalledWith(YESTERDAY);
    core.historyPage.mockResolvedValueOnce({ days: [], more: false });
    click($$('.bmore button')[0]);
    await settle();
    expect(core.historyPage).toHaveBeenLastCalledWith('2026-10-20');
  });

  it('clear needs a second press within 3 s', async () => {
    page = await mount();
    const clear = $('[data-hclear]')!;
    expect(text(clear)).toBe('Clear history');
    click(clear);
    expect(text(clear)).toBe('Clear history? Press again');
    expect(core.historyClear).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3000);
    expect(text(clear)).toBe('Clear history');
    // A first press again, then a second one in time.
    click(clear);
    await vi.advanceTimersByTimeAsync(2000);
    click(clear);
    await settle();
    expect(core.historyClear).toHaveBeenCalledTimes(1);
    expect(text(clear)).toBe('Clear history');
    expect(rows()).toHaveLength(0);
    expect($('.hempty')!.hidden).toBe(false);
  });

  it('copy and paste again call their commands', async () => {
    page = await mount();
    const row = rows()[0];
    const copy = row.querySelector<HTMLElement>('[data-copy]')!;
    const paste = row.querySelector<HTMLElement>('[data-paste]')!;
    expect(copy.getAttribute('aria-label')).toBe('Copy');
    expect(paste.getAttribute('aria-label')).toBe('Paste again');
    click(copy);
    await settle();
    expect(core.historyCopy).toHaveBeenCalledWith(morning.id);
    expect(copy.classList.contains('done')).toBe(true);
    await vi.advanceTimersByTimeAsync(1400);
    expect(copy.classList.contains('done')).toBe(false);
    // Pasted: the check on "Paste again".
    click(paste);
    await settle();
    expect(core.historyPaste).toHaveBeenCalledWith(morning.id);
    expect(paste.classList.contains('done')).toBe(true);
    expect(copy.classList.contains('done')).toBe(false);
    await vi.advanceTimersByTimeAsync(1400);
    // Nowhere to paste: the text is in the clipboard — the same mark as "Copy".
    core.historyPaste.mockResolvedValue('clipboard');
    click(paste);
    await settle();
    expect(core.historyPaste).toHaveBeenCalledTimes(2);
    expect(copy.classList.contains('done')).toBe(true);
    expect(paste.classList.contains('done')).toBe(false);
  });

  it('a new entry appears at the top of Today', async () => {
    page = await mount();
    const fresh = entry(`${TODAY}T12:31:00.000-1`, 'Call moves to tomorrow.');
    feeds.entry(fresh);
    const today = $$('.hgrp')[0];
    const top = today.querySelector('.hr')!;
    expect(top.querySelector('.htx')!.textContent).toBe('Call moves to tomorrow.');
    expect(top.classList.contains('new')).toBe(true);
    // The same entry again (the channel and a page can both bring it) is not shown twice.
    feeds.entry(fresh);
    expect(today.querySelectorAll('.hr')).toHaveLength(3);
  });

  it('day headings follow the date past midnight', async () => {
    page = await mount();
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Today', 'Yesterday']);
    // The window stays open past midnight: the first entry of the new day redraws every heading.
    vi.setSystemTime(new Date(2026, 10, 1, 0, 5));
    feeds.entry(entry('2026-11-01T00:04:00.000-1', 'Just after midnight.'));
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Today', 'Yesterday', 'Friday, October 30']);
    const top = rows()[0];
    expect(top.querySelector('.htx')!.textContent).toBe('Just after midnight.');
    expect(top.classList.contains('new')).toBe(true);
    expect($$('.hr.new')).toHaveLength(1);
  });

  it('day headings follow the date when the window comes back', async () => {
    page = await mount();
    vi.setSystemTime(new Date(2026, 10, 1, 9, 0));
    window.dispatchEvent(new Event('focus'));
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Yesterday', 'Friday, October 30']);
    // The same day again: nothing is drawn anew.
    const first = rows()[0];
    window.dispatchEvent(new Event('focus'));
    expect(rows()[0]).toBe(first);
  });

  it('a new entry makes Today when there was none', async () => {
    core.historyPage.mockResolvedValue({ days: [{ date: YESTERDAY, entries: [evening] }], more: false });
    page = await mount();
    feeds.entry(entry(`${TODAY}T07:00:00.000-1`, 'First of the day.'));
    expect($$('.hgrp .hday').map((d) => d.textContent)).toEqual(['Today', 'Yesterday']);
    expect(rowTexts()[0]).toBe('First of the day.');
  });

  it('an entry that comes with the first page is shown once', async () => {
    let answerPage: (p: unknown) => void = () => {};
    core.historyPage.mockImplementationOnce(() => new Promise((r) => (answerPage = r as never)));
    document.body.innerHTML = '<div id="w"></div>';
    const { mountWindow } = await import('../src/window/page');
    const started = mountWindow(document.getElementById('w')!, { reduced: () => true });
    await settle();
    const fresh = entry(`${TODAY}T12:31:00.000-1`, 'Came while the page loaded.');
    feeds.entry(fresh);
    answerPage({ days: [{ date: TODAY, entries: [fresh, morning] }], more: false });
    page = await started;
    await settle();
    expect(rowTexts()).toEqual(['Came while the page loaded.', 'NotepadMorning standup notes.']);
  });

  it('a new entry waits while a search is shown', async () => {
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [morning] }]);
    page = await mount();
    click($('h3 .ibt'));
    const input = $<HTMLInputElement>('.srch input')!;
    type(input, 'standup');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    feeds.entry(entry(`${TODAY}T12:31:00.000-1`, 'Typed during a search.'));
    expect(rows()).toHaveLength(1);
    type(input, '');
    expect(rowTexts()[0]).toBe('Typed during a search.');
    expect(rows()).toHaveLength(4);
  });

  it('keep menu sends historyKeep', async () => {
    page = await mount();
    const dd = $('[data-dd="hkeep"]')!;
    expect(text(dd)).toBe('A month');
    click(dd);
    const items = $$('.menu [data-i]');
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['Forever', 'A year', 'A month', 'A week', "Don't keep"]);
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true', 'false', 'false']);
    expect(text($('.menu .mnote'))).toBe('A shorter period deletes older days at once.');
    click(items[3]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'historyKeep', value: 'week' });
    expect($('.menu')).toBeNull();
    expect(text(dd)).toBe('A week');
    // Older days may be gone: the list is read again.
    expect(core.historyPage).toHaveBeenCalledTimes(2);
  });

  it('an unknown period shows unset, with nothing chosen', async () => {
    stub(view({ historyKeep: null }));
    core.historyPage.mockResolvedValue(firstPage());
    page = await mount();
    const dd = $('[data-dd="hkeep"]')!;
    expect(text(dd)).toBe('Not set');
    click(dd);
    expect($$('.menu [data-i]').every((b) => b.getAttribute('aria-checked') === 'false')).toBe(true);
    // A second press on it closes the menu.
    click(dd);
    expect($('.menu')).toBeNull();
  });
});
