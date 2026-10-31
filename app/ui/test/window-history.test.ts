import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, YESTERDAY, click, core, entry, feeds, key, mount, oggFile, settle, stub, text, type, view } from './windowFixture';
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

/** The object URLs the page made for recordings, and those it gave back. */
let urls: { made: string[]; types: string[]; revoked: string[] } = { made: [], types: [], revoked: [] };

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
  core.historyPage.mockResolvedValue(firstPage());
  // jsdom has no object URLs and plays no sound: stand-ins that say what was asked.
  urls = { made: [], types: [], revoked: [] };
  URL.createObjectURL = (b: Blob | MediaSource) => {
    urls.types.push((b as Blob).type);
    urls.made.push(`blob:cue/${urls.made.length + 1}`);
    return urls.made[urls.made.length - 1];
  };
  URL.revokeObjectURL = (u: string) => void urls.revoked.push(u);
  vi.spyOn(HTMLMediaElement.prototype, 'play').mockImplementation(function (this: HTMLMediaElement) {
    this.dispatchEvent(new Event('play'));
    return Promise.resolve();
  });
  vi.spyOn(HTMLMediaElement.prototype, 'pause').mockImplementation(function (this: HTMLMediaElement) {
    this.dispatchEvent(new Event('pause'));
  });
  vi.spyOn(HTMLMediaElement.prototype, 'load').mockImplementation(() => {});
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
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'onerror');
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
    // The search lives on the heading's line: the magnifier opens it, and turns into its close button.
    const find = $('.bsec>h3 .hsb .hsbt')!;
    expect(find.getAttribute('aria-label')).toBe('Search');
    expect(find.getAttribute('aria-expanded')).toBe('false');
    expect(find.querySelectorAll('svg')).toHaveLength(2);
    click(find);
    expect($('.hsb')!.classList.contains('open')).toBe(true);
    expect(find.getAttribute('aria-label')).toBe('Close search');
    expect(find.getAttribute('aria-expanded')).toBe('true');
    const input = $<HTMLInputElement>('.hsf input')!;
    expect(document.activeElement).toBe(input);
    expect(input.placeholder).toBe('Search the history');
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
    click($('.hsbt'));
    expect($('.hsb')!.classList.contains('open')).toBe(false);
    expect(find.getAttribute('aria-label')).toBe('Search');
    expect(input.value).toBe('');
    expect(rows()).toHaveLength(3);
  });

  it('search marks ё and е alike', async () => {
    const report = entry(`${TODAY}T11:00:00.000-1`, 'Отчёт готов, отчет отправлен.');
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [report] }]);
    page = await mount();
    click($('.hsbt'));
    const input = $<HTMLInputElement>('.hsf input')!;
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
    click($('.hsbt'));
    const input = $<HTMLInputElement>('.hsf input')!;
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

/** Telegram entries of today at these times (newest first), each with its time as its text. */
const tg = (...times: string[]) => times.map((t) => entry(`${TODAY}T${t}.000-1`, `At ${t}.`, { app: 'Telegram' }));
const burst = tg('11:58:00', '11:56:00', '11:53:00', '11:50:00', '11:48:00');
const pageOf = (...entries: ReturnType<typeof entry>[]) => ({ days: [{ date: TODAY, entries }], more: false });
const rowOf = (id: string) => rows().find((r) => r.dataset.id === id)!;
/** A click on an entry's text: the row's own click. */
const openRow = (id: string) => click(rowOf(id).querySelector('.htx'));

describe('Stacks', () => {
  it('a burst from one app is one stack; its chip and its sheets open it in place', async () => {
    core.historyPage.mockResolvedValue(pageOf(...burst, morning));
    page = await mount();
    const stacks = $$('.hstk');
    expect(stacks).toHaveLength(1);
    const s = stacks[0];
    expect(s.classList.contains('open')).toBe(false);
    // The newest on top, the chip "Telegram 5", two sheet edges under it; the other four hidden in it.
    const top = s.querySelector<HTMLElement>(':scope > .hr.htop')!;
    expect(top.dataset.id).toBe(burst[0].id);
    const chip = top.querySelector<HTMLElement>('.htx .hchip')!;
    expect(chip.tagName).toBe('BUTTON');
    expect(chip.textContent).toBe('Telegram5');
    expect(chip.querySelector('.hcnt')!.textContent).toBe('5');
    expect(chip.getAttribute('aria-label')).toBe('Telegram, 5 entries');
    expect(chip.getAttribute('aria-expanded')).toBe('false');
    expect(s.querySelectorAll('.hedge i')).toHaveLength(2);
    expect([...s.querySelectorAll<HTMLElement>('.hrest .hr')].map((r) => r.dataset.id)).toEqual(burst.slice(1).map((e) => e.id));
    // Anything else stands alone.
    expect(rowOf(morning.id).closest('.hstk')).toBeNull();
    click(chip);
    expect(s.classList.contains('open')).toBe(true);
    expect(chip.getAttribute('aria-expanded')).toBe('true');
    // Opening a stack opens no entry.
    expect($$('.hr.open')).toHaveLength(0);
    click(chip);
    expect(s.classList.contains('open')).toBe(false);
    click(s.querySelector('.hedge i'));
    expect(s.classList.contains('open')).toBe(true);
    // Copy and paste again on every entry, a hidden one too.
    const inner = rowOf(burst[3].id);
    expect(inner.querySelectorAll('[data-copy], [data-paste]')).toHaveLength(2);
    click(inner.querySelector('[data-copy]'));
    await settle();
    expect(core.historyCopy).toHaveBeenCalledWith(burst[3].id);
    expect(s.classList.contains('open')).toBe(true);
  });

  it('a run of entries whose app is not known is never stacked', async () => {
    const unknown = ['11:59:00', '11:58:30', '11:58:00'].map((t) => entry(`${TODAY}T${t}.000-1`, t));
    core.historyPage.mockResolvedValue(pageOf(...unknown));
    page = await mount();
    expect($$('.hstk')).toHaveLength(0);
    expect(rows()).toHaveLength(3);
  });

  it('a new entry joins its stack, which stays open, or starts one', async () => {
    core.historyPage.mockResolvedValue(pageOf(...burst, morning));
    page = await mount();
    click($('.hchip'));
    const [six] = tg('11:59:30');
    feeds.entry(six);
    let s = $('.hstk')!;
    expect(s.classList.contains('open')).toBe(true);
    expect(s.querySelector('.hcnt')!.textContent).toBe('6');
    const top = s.querySelector<HTMLElement>(':scope > .hr')!;
    expect(top.dataset.id).toBe(six.id);
    expect(top.classList.contains('new')).toBe(true);
    // Two from another app stand alone; the third makes them a stack of their own, closed.
    const note = (t: string) => entry(`${TODAY}T${t}.000-1`, `Note ${t}.`, { app: 'Notepad' });
    feeds.entry(note('11:59:40'));
    feeds.entry(note('11:59:45'));
    expect($$('.hstk')).toHaveLength(1);
    feeds.entry(note('11:59:50'));
    const stacks = $$('.hstk');
    expect(stacks).toHaveLength(2);
    s = stacks[0];
    expect(s.querySelector('.hcnt')!.textContent).toBe('3');
    expect(s.classList.contains('open')).toBe(false);
    expect(s.querySelector(':scope > .hr')!.classList.contains('new')).toBe(true);
    expect(stacks[1].classList.contains('open')).toBe(true);
  });

  it('an opened entry is not folded away when its run becomes a stack', async () => {
    const [a, b] = tg('11:56:00', '11:53:00').map((e) => ({ ...e, raw: `${e.text} as heard` }));
    core.historyPage.mockResolvedValue(pageOf(a, b));
    page = await mount();
    openRow(b.id);
    expect(rowOf(b.id).classList.contains('open')).toBe(true);
    feeds.entry(tg('11:58:00')[0]);
    const s = $('.hstk')!;
    expect(s.classList.contains('open')).toBe(true);
    expect(rowOf(b.id).classList.contains('open')).toBe(true);
    expect(rowOf(b.id).closest('.hrest')).not.toBeNull();
    // Closing the stack closes what was opened in it.
    click($('.hchip'));
    expect(rowOf(b.id).classList.contains('open')).toBe(false);
  });

  it('what a search finds is never stacked', async () => {
    core.historyPage.mockResolvedValue(pageOf(...burst));
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: burst }]);
    page = await mount();
    expect($$('.hstk')).toHaveLength(1);
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'at');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect($$('.hstk, .hchip, .hedge')).toHaveLength(0);
    expect(rows()).toHaveLength(5);
    expect(rows().every((r) => r.querySelector('mark')!.textContent === 'At')).toBe(true);
  });
});

describe('The search', () => {
  it('Esc closes a menu, then the tray, then the search', async () => {
    page = await mount();
    click($('.hsbt'));
    const input = $<HTMLInputElement>('.hsf input')!;
    type(input, 'standup');
    click($('[data-dd="hkeep"]'));
    expect($('.menu')).not.toBeNull();
    key('Escape');
    expect($('.menu')).toBeNull();
    expect($('.hsb')!.classList.contains('open')).toBe(true);
    click($('.gear'));
    expect($('.btray')!.classList.contains('open')).toBe(true);
    key('Escape');
    expect($('.btray')!.classList.contains('open')).toBe(false);
    expect($('.hsb')!.classList.contains('open')).toBe(true);
    key('Escape');
    expect($('.hsb')!.classList.contains('open')).toBe(false);
    expect(input.value).toBe('');
    expect($('.hsbt')!.getAttribute('aria-label')).toBe('Search');
  });
});

describe('How it was said', () => {
  const said = entry(`${TODAY}T10:40:00.000-1`, "So I think we should move the call to Thursday. We're going to ship it.", {
    app: 'Outlook',
    raw: "so um I think we should uh move the call to thursday we're gonna ship it",
  });

  it('a dictation opens to its words as heard, its cleanup marked, and closes again', async () => {
    core.historyPage.mockResolvedValue(pageOf(said, morning));
    page = await mount();
    const r = rowOf(said.id);
    expect(r.classList.contains('hmore')).toBe(true);
    expect(r.querySelector('.hdet')).toBeNull();
    openRow(said.id);
    expect(r.classList.contains('open')).toBe(true);
    expect(text(r.querySelector('.hdet .hlab'))).toBe('How you said it');
    const raw = r.querySelector('.hdet .hraw')!;
    expect([...raw.querySelectorAll('s')].map((x) => x.textContent)).toEqual(['um', 'uh', 'gonna']);
    expect([...raw.querySelectorAll('ins')].map((x) => x.textContent)).toEqual(['going to']);
    expect(raw.textContent).toBe("so um I think we should uh move the call to thursday we're gonna going to ship it");
    expect(core.historyAudio).toHaveBeenCalledWith(said.id);
    // A click inside what opened is not the row's.
    click(raw);
    expect(r.classList.contains('open')).toBe(true);
    openRow(said.id);
    expect(r.classList.contains('open')).toBe(false);
    // Its text stays whole while it shrinks back to one line.
    expect(r.classList.contains('shut')).toBe(true);
    await settle();
    expect(r.classList.contains('shut')).toBe(false);
    // Its buttons are not the row's either.
    click(r.querySelector('[data-copy]'));
    expect(r.classList.contains('open')).toBe(false);
  });

  it('an entry with nothing more to show does not open', async () => {
    page = await mount();
    const r = rowOf(morning.id);
    expect(r.classList.contains('hmore')).toBe(false);
    openRow(morning.id);
    expect(r.classList.contains('open')).toBe(false);
    expect(r.querySelector('.hdet')).toBeNull();
    expect(core.historyAudio).not.toHaveBeenCalled();
  });

  it('a translation shows the speech as it was, an Ask its question', async () => {
    const tr = entry(`${TODAY}T10:00:00.000-1`, 'See you tomorrow at ten.', {
      mode: 'translation',
      app: 'Mail',
      raw: 'Увидимся завтра в десять',
    });
    core.historyPage.mockResolvedValue(pageOf(tr, answer));
    page = await mount();
    openRow(tr.id);
    const t = rowOf(tr.id);
    expect(text(t.querySelector('.hlab'))).toBe('What you said');
    expect(t.querySelector('.hraw')!.textContent).toBe('Увидимся завтра в десять');
    expect(t.querySelector('.hraw s, .hraw ins')).toBeNull();
    openRow(answer.id);
    const a = rowOf(answer.id);
    expect(a.classList.contains('open')).toBe(true);
    expect(text(a.querySelector('.hlab'))).toBe('Your question');
    expect(a.querySelector('.hraw')!.textContent).toBe('Swarm or Kubernetes?');
    expect(a.querySelector('.hraw s, .hraw ins')).toBeNull();
  });

  it('an entry found only in how it was said opens by itself, the place marked, and closes with the search', async () => {
    const found = entry(`${TODAY}T10:00:00.000-1`, 'Could you confirm the delivery location?', {
      app: 'Mail',
      raw: 'could you confirm the delivery address',
    });
    const shown = entry(`${TODAY}T09:00:00.000-1`, 'The address is on the card.', { raw: 'the address is on the card' });
    core.historyPage.mockResolvedValue(pageOf(found, shown));
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [found, shown] }]);
    page = await mount();
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'ADDRESS');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    const r = rowOf(found.id);
    expect(r.querySelector('.htx mark')).toBeNull();
    expect(r.classList.contains('open')).toBe(true);
    expect(r.getAttribute('aria-expanded')).toBe('true');
    expect(r.querySelector('.hraw mark')!.textContent).toBe('address');
    expect(r.querySelector('.hraw s mark')!.textContent).toBe('address');
    // A hit its row shows opens nothing.
    expect(rowOf(shown.id).querySelector('.htx mark')!.textContent).toBe('address');
    expect(rowOf(shown.id).classList.contains('open')).toBe(false);
    // The search closes: so does what it opened.
    key('Escape');
    await settle();
    expect(rowOf(found.id).classList.contains('open')).toBe(false);
    // One the user opened stays open, and a search that finds it again leaves it so.
    openRow(shown.id);
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'card');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    key('Escape');
    await settle();
    expect(rowOf(shown.id).classList.contains('open')).toBe(true);
  });

  it('Enter or Space on an entry opens and closes it; its buttons keep their keys', async () => {
    core.historyPage.mockResolvedValue(pageOf(said, morning));
    page = await mount();
    const r = rowOf(said.id);
    expect(r.tabIndex).toBe(0);
    expect(r.getAttribute('aria-expanded')).toBe('false');
    // An entry with nothing more to show is no stop for the keyboard.
    expect(rowOf(morning.id).hasAttribute('tabindex')).toBe(false);
    expect(rowOf(morning.id).hasAttribute('aria-expanded')).toBe(false);
    const press = (target: Element, k: string) => {
      const ev = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true });
      target.dispatchEvent(ev);
      return ev.defaultPrevented;
    };
    r.focus();
    expect(press(r, 'Enter')).toBe(true);
    expect(r.classList.contains('open')).toBe(true);
    expect(r.getAttribute('aria-expanded')).toBe('true');
    expect(press(r, ' ')).toBe(true);
    expect(r.classList.contains('open')).toBe(false);
    expect(r.getAttribute('aria-expanded')).toBe('false');
    expect(press(r.querySelector('[data-copy]')!, 'Enter')).toBe(false);
    expect(press(r, 'a')).toBe(false);
    expect(r.classList.contains('open')).toBe(false);
    // Drawn again (a new entry), the entry keeps the focus in its new row.
    feeds.entry(entry(`${TODAY}T11:00:00.000-1`, 'Later.'));
    expect(document.activeElement).toBe(rowOf(said.id));
    expect(rowOf(said.id)).not.toBe(r);
  });

  it('an opened entry stays open when the list is drawn again', async () => {
    core.historyPage.mockResolvedValue(pageOf(said, morning));
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [morning] }]);
    page = await mount();
    openRow(said.id);
    feeds.entry(entry(`${TODAY}T11:00:00.000-1`, 'Later.'));
    expect(rowOf(said.id).classList.contains('open')).toBe(true);
    // Away while a search is shown, open again after it.
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'standup');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect(rows()).toHaveLength(1);
    key('Escape');
    expect(rowOf(said.id).classList.contains('open')).toBe(true);
    expect(rowOf(said.id).querySelectorAll('.hraw s')).toHaveLength(3);
  });
});

describe('The recording', () => {
  const one = entry(`${TODAY}T10:40:00.000-1`, 'First.', { app: 'Mail', raw: 'first', hasAudio: true });
  const two = entry(`${TODAY}T10:20:00.000-1`, 'Second.', { app: 'Notepad', raw: 'second', hasAudio: true });
  const bytes = (id: string) => oggFile(id === one.id ? 3.2 : 61);
  const player = (id: string) => rowOf(id).querySelector<HTMLElement>('.hplay')!;
  const button = (id: string) => player(id).querySelector<HTMLButtonElement>('.hpb')!;

  it('plays one at a time, its URL given back on closing, when its row is gone and when the page goes', async () => {
    core.historyPage.mockResolvedValue(pageOf(one, two));
    core.historyAudio.mockImplementation(async (id) => bytes(id));
    page = await mount();
    openRow(one.id);
    // It has a recording: the player is there at once, waiting for it.
    expect(player(one.id).hidden).toBe(false);
    expect(button(one.id).disabled).toBe(true);
    await settle();
    expect(button(one.id).disabled).toBe(false);
    expect(player(one.id).querySelector('.hdur')!.textContent).toBe('0:03');
    expect(player(one.id).querySelectorAll('.hpr i')).toHaveLength(1);
    expect(urls.types).toEqual(['audio/ogg; codecs=opus']);
    expect(button(one.id).getAttribute('aria-label')).toBe('Play');
    click(button(one.id));
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(1);
    expect(button(one.id).getAttribute('aria-label')).toBe('Pause');
    expect(rowOf(one.id).classList.contains('open')).toBe(true);
    // A second one plays: the first stops.
    openRow(two.id);
    await settle();
    expect(player(two.id).querySelector('.hdur')!.textContent).toBe('1:01');
    click(button(two.id));
    expect(button(one.id).getAttribute('aria-label')).toBe('Play');
    expect(button(two.id).getAttribute('aria-label')).toBe('Pause');
    // Pause, and play again.
    click(button(two.id));
    expect(button(two.id).getAttribute('aria-label')).toBe('Play');
    click(button(two.id));
    expect(button(two.id).getAttribute('aria-label')).toBe('Pause');
    // The list drawn again: the same player, still playing, nothing given back.
    const kept = player(two.id);
    feeds.entry(entry(`${TODAY}T11:00:00.000-1`, 'Later.'));
    expect(player(two.id)).toBe(kept);
    expect(button(two.id).getAttribute('aria-label')).toBe('Pause');
    expect(urls.revoked).toEqual([]);
    expect(core.historyAudio).toHaveBeenCalledTimes(2);
    // Closed: its URL is given back.
    openRow(one.id);
    expect(urls.revoked).toEqual([urls.made[0]]);
    // Its row gone (a search shows other entries): given back too; open again after it, asked for anew.
    core.historySearch.mockResolvedValue([{ date: TODAY, entries: [one] }]);
    click($('.hsbt'));
    type($<HTMLInputElement>('.hsf input')!, 'first');
    await vi.advanceTimersByTimeAsync(200);
    await settle();
    expect(urls.revoked).toEqual(urls.made.slice(0, 2));
    key('Escape');
    await settle();
    expect(core.historyAudio).toHaveBeenCalledTimes(3);
    expect(button(two.id).disabled).toBe(false);
    // The page goes: every URL given back.
    window.dispatchEvent(new Event('pagehide'));
    expect(urls.revoked).toEqual(urls.made);
    expect(button(two.id).disabled).toBe(true);
  });

  it('no recording, no player; a new entry asks for its recording anyway', async () => {
    const old = entry(`${TODAY}T10:00:00.000-1`, 'Older.', { raw: 'older' });
    core.historyPage.mockResolvedValue(pageOf(old));
    page = await mount();
    openRow(old.id);
    // None said yet: nothing shown while it is asked for, and none comes.
    expect(player(old.id).hidden).toBe(true);
    await settle();
    expect(core.historyAudio).toHaveBeenCalledWith(old.id);
    expect(rowOf(old.id).querySelector('.hplay')).toBeNull();
    expect(urls.made).toEqual([]);
    // A new entry comes before its recording is encoded (hasAudio false); opened, it is asked for.
    const fresh = entry(`${TODAY}T11:30:00.000-1`, 'Fresh.', { raw: 'fresh' });
    feeds.entry(fresh);
    core.historyAudio.mockResolvedValue(oggFile(2));
    openRow(fresh.id);
    await settle();
    expect(player(fresh.id).hidden).toBe(false);
    expect(player(fresh.id).querySelector('.hdur')!.textContent).toBe('0:02');
    // A failure to read it shows no player and logs only its code.
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    core.historyAudio.mockRejectedValue('failed');
    const other = entry(`${TODAY}T11:40:00.000-1`, 'Other.', { raw: 'other text heard' });
    feeds.entry(other);
    openRow(other.id);
    await settle();
    expect(rowOf(other.id).querySelector('.hplay')).toBeNull();
    expect(err).toHaveBeenCalledWith('CUE: reading a recording failed:', 'failed');
    expect(JSON.stringify(err.mock.calls)).not.toContain('other text heard');
    err.mockRestore();
  });

  it('a new entry opened while its recording is encoded asks once more a moment later', async () => {
    page = await mount();
    const fresh = entry(`${TODAY}T11:30:00.000-1`, 'Fresh.', { raw: 'fresh' });
    feeds.entry(fresh);
    core.historyAudio.mockResolvedValueOnce(null).mockResolvedValueOnce(oggFile(2));
    openRow(fresh.id);
    await settle();
    expect(core.historyAudio).toHaveBeenCalledTimes(1);
    expect(player(fresh.id).hidden).toBe(true);
    await vi.advanceTimersByTimeAsync(2000);
    await settle();
    expect(core.historyAudio).toHaveBeenCalledTimes(2);
    expect(player(fresh.id).hidden).toBe(false);
    expect(player(fresh.id).querySelector('.hdur')!.textContent).toBe('0:02');
    // None the second time either: no player, and no third ask.
    const other = entry(`${TODAY}T11:40:00.000-1`, 'Other.', { raw: 'other' });
    feeds.entry(other);
    core.historyAudio.mockResolvedValue(null);
    openRow(other.id);
    await settle();
    await vi.advanceTimersByTimeAsync(5000);
    await settle();
    expect(core.historyAudio).toHaveBeenCalledTimes(4);
    expect(rowOf(other.id).querySelector('.hplay')).toBeNull();
    // An entry of the first page is not asked for again.
    const old = entry(`${TODAY}T10:00:00.000-1`, 'Older.', { raw: 'older' });
    core.historyPage.mockResolvedValue(pageOf(old));
    await page.history.reload();
    openRow(old.id);
    await settle();
    await vi.advanceTimersByTimeAsync(5000);
    expect(core.historyAudio).toHaveBeenCalledTimes(5);
  });

  it('every recording pauses when CUE starts recording', async () => {
    core.historyPage.mockResolvedValue(pageOf(one));
    core.historyAudio.mockImplementation(async (id) => bytes(id));
    page = await mount();
    openRow(one.id);
    await settle();
    click(button(one.id));
    expect(button(one.id).getAttribute('aria-label')).toBe('Pause');
    feeds.state({ phase: 'think', paused: false });
    expect(button(one.id).getAttribute('aria-label')).toBe('Pause');
    feeds.state({ phase: 'start', paused: false });
    expect(button(one.id).getAttribute('aria-label')).toBe('Play');
    click(button(one.id));
    feeds.state({ phase: 'listen', paused: false });
    expect(button(one.id).getAttribute('aria-label')).toBe('Play');
  });

  it('a new recordings period stops what plays and reads the list again', async () => {
    core.historyPage.mockResolvedValue(pageOf(one));
    core.historyAudio.mockImplementation(async (id) => bytes(id));
    page = await mount();
    openRow(one.id);
    await settle();
    click(button(one.id));
    // "Don't keep": the recording stops and gives back its URL; the entry says it has none, and its player goes.
    core.historyPage.mockResolvedValue(pageOf({ ...one, hasAudio: false }));
    core.historyAudio.mockResolvedValue(null);
    await page.change({ kind: 'historyAudio', value: 'off' });
    expect(urls.revoked).toEqual(urls.made);
    await settle();
    expect(core.historyPage).toHaveBeenCalledTimes(2);
    expect(rowOf(one.id).classList.contains('open')).toBe(true);
    expect(rowOf(one.id).querySelector('.hplay')).toBeNull();
    // The same period again changes nothing.
    await page.change({ kind: 'historyAudio', value: 'off' });
    await settle();
    expect(core.historyPage).toHaveBeenCalledTimes(2);
  });

  it('a recording given back lets go of its file, and a start cut short by it is no failure', async () => {
    core.historyPage.mockResolvedValue(pageOf(one));
    core.historyAudio.mockImplementation(async (id) => bytes(id));
    page = await mount();
    openRow(one.id);
    await settle();
    const err = vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.mocked(HTMLMediaElement.prototype.play).mockImplementationOnce(() =>
      Promise.reject(new DOMException('The play() request was interrupted', 'AbortError')),
    );
    click(button(one.id));
    openRow(one.id);
    await settle();
    expect(HTMLMediaElement.prototype.load).toHaveBeenCalledTimes(1);
    expect(err).not.toHaveBeenCalled();
    // Any other failure is told, by its code only.
    openRow(one.id);
    await settle();
    vi.mocked(HTMLMediaElement.prototype.play).mockImplementationOnce(() => Promise.reject(new Error('decode')));
    click(button(one.id));
    await settle();
    expect(err).toHaveBeenCalledWith('CUE: playing a recording failed:', 'failed');
    expect(button(one.id).getAttribute('aria-label')).toBe('Play');
    err.mockRestore();
  });

  it('closed before its recording came: nothing is made of it', async () => {
    let answerAudio: (b: ArrayBuffer | null) => void = () => {};
    core.historyPage.mockResolvedValue(pageOf(one));
    core.historyAudio.mockImplementation(() => new Promise((r) => (answerAudio = r)));
    page = await mount();
    openRow(one.id);
    openRow(one.id);
    answerAudio(oggFile(1));
    await settle();
    expect(urls.made).toEqual([]);
  });
});
