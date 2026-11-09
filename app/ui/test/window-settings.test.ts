import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, click, core, entry, feeds, key, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import { CLEAR_ARMED_MS, matchMic } from '../src/window/settings';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;

const tray = () => $('.btray')!;
const title = () => text($('.btray .tph b'));
const sub = async (name: string) => {
  click($(`.btray [data-sub="${name}"]`));
  await settle();
};
const open = async () => {
  click($('.gear'));
  await settle();
};
const previews = () => core.micPreview.mock.calls.map(([on, device]) => (on ? `on:${device ?? 'windows'}` : 'off'));
/** Megabytes as the model's row writes them (decimal), and the model's size (668 MB). */
const MB = 1_000_000;
const TOTAL = 667_729_022;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The settings tray', () => {
  it('gear opens the tray and Esc goes back then closes', async () => {
    page = await mount();
    expect(tray().hidden).toBe(true);
    await open();
    expect(tray().hidden).toBe(false);
    expect(tray().classList.contains('open')).toBe(true);
    expect($('.gear')!.classList.contains('hid')).toBe(true);
    expect(title()).toBe('Settings');
    expect($$('.btray .trw').map((r) => r.querySelector('span:not(.si)')!.textContent)).toEqual([
      'Speech recognition',
      'Cleanup',
      'Microphone',
      'Languages',
      'History',
      'General',
    ]);
    expect($$('.btray .trw').map((r) => r.dataset.sub)).toEqual(['model', 'cleanup', 'mic', 'languages', 'history', 'general']);
    // Each row with its value.
    expect($$('.btray .trw em').map((e) => e.textContent)).toEqual([
      'Ready',
      'Cloud',
      'As in Windows',
      'Auto · into English',
      'A month',
      'Manually',
    ]);
    await sub('languages');
    expect(title()).toBe('Languages');
    key('Escape');
    expect(title()).toBe('Settings');
    expect(tray().classList.contains('open')).toBe(true);
    key('Escape');
    expect(tray().classList.contains('open')).toBe(false);
    expect($('.gear')!.classList.contains('hid')).toBe(false);
    await settle();
    expect(tray().hidden).toBe(true);
    // A click outside closes it too; the close button as well.
    await open();
    click($('.bcol'));
    expect(tray().classList.contains('open')).toBe(false);
    await open();
    click($('.btray [data-tclose]'));
    expect(tray().classList.contains('open')).toBe(false);
  });

  it('says quietly that changes last until CUE quits when settings.json was not read', async () => {
    stub(view({ saved: false }));
    page = await mount();
    await open();
    expect(text($('.btray .tft'))).toBe("settings.json wasn't read — changes last until CUE quits");
    page.dispose();
    stub();
    page = await mount();
    await open();
    expect($('.btray .tft')).toBeNull();
  });

  it('the Speech recognition row shows the model state and follows it', async () => {
    page = await mount();
    expect(core.modelGet).toHaveBeenCalledTimes(1);
    expect(core.onModel).toHaveBeenCalledTimes(1);
    await open();
    const em = () => $<HTMLElement>('.btray em[data-val="model"]')!;
    const value = () => text(em());
    expect(value()).toBe('Ready');
    expect(em().classList.contains('warn')).toBe(false);
    feeds.model({ state: 'failed', done: 412 * MB, total: TOTAL });
    expect(value()).toBe('Did not download');
    expect(em().classList.contains('warn')).toBe(true);
    feeds.model({ state: 'loading' });
    expect(value()).toBe('Loading');
    expect(em().classList.contains('warn')).toBe(false);
    feeds.model({ state: 'unsupported' });
    expect(value()).toBe('Not supported');
    expect(em().classList.contains('warn')).toBe(true);
    feeds.model({ state: 'missing' });
    expect(value()).toBe('Not downloaded');
    feeds.model({ state: 'ready', total: TOTAL });
    expect(value()).toBe('Ready');
    // A state heard while the tray is closed shows when it opens.
    key('Escape');
    feeds.model({ state: 'downloading', done: 67 * MB, total: TOTAL });
    await open();
    expect(value()).toBe('10 %');
  });

  it('while the model downloads, its row shows the percent in Bebas and a line under it, changed in place', async () => {
    page = await mount();
    await open();
    const row = $<HTMLElement>('.btray [data-sub="model"]')!;
    const em = row.querySelector('em')!;
    expect(row.querySelector('.tline')).toBeNull();
    expect(em.classList.contains('num')).toBe(false);
    feeds.model({ state: 'downloading', done: 412 * MB, total: TOTAL });
    expect(text(em)).toBe('61 %');
    expect(em.classList.contains('num')).toBe(true);
    expect(em.classList.contains('warn')).toBe(false);
    const line = row.querySelector<HTMLElement>('.tline')!;
    expect(line.querySelector<HTMLElement>('i')!.style.transform).toMatch(/^scaleX\(0\.61\d*\)$/);
    // The next step of the download: the same row, the same line, a new scale and number.
    feeds.model({ state: 'downloading', done: 600 * MB, total: TOTAL });
    expect($('.btray [data-sub="model"]')).toBe(row);
    expect(row.querySelectorAll('.tline')).toHaveLength(1);
    expect(row.querySelector('.tline')).toBe(line);
    expect(line.querySelector<HTMLElement>('i')!.style.transform).toMatch(/^scaleX\(0\.89\d*\)$/);
    expect(text(em)).toBe('89 %');
    // Done: the line goes, the value is a word again.
    feeds.model({ state: 'ready', total: TOTAL });
    expect(row.querySelector('.tline')).toBeNull();
    expect(em.classList.contains('num')).toBe(false);
    expect(text(em)).toBe('Ready');
  });

  it('the Speech recognition page: the model row and the note', async () => {
    core.modelGet.mockResolvedValue({ state: 'downloading', done: 412 * MB, total: TOTAL });
    page = await mount();
    await open();
    await sub('model');
    expect(title()).toBe('Speech recognition');
    const rows = $$('.btray .sgrp > .srow');
    expect(rows.map((r) => (r.classList.contains('mrow') ? 'model' : r.dataset.key))).toEqual(['model']);
    expect(text($('.btray .tft'))).toBe('Sound stays on this computer.');
    const mrow = () => $<HTMLElement>('.btray .mrow')!;
    const row = mrow();
    expect(row.dataset.model).toBe('downloading');
    expect(text(row.querySelector('.stx b'))).toBe('Parakeet Ultra');
    expect(text(row.querySelector('.sc .mpct'))).toBe('61 %');
    expect(text(row.querySelector('.mprog small'))).toBe('Downloading: 412 of 668 MB');
    expect(row.querySelector<HTMLElement>('.mbar i')!.style.transform).toMatch(/^scaleX\(0\.61\d*\)$/);
    expect(row.querySelector('[data-model-retry]')).toBeNull();
    // The download goes on: the row changes where it is.
    feeds.model({ state: 'downloading', done: 470 * MB, total: TOTAL });
    expect(mrow()).toBe(row);
    expect(text(row.querySelector('.mpct'))).toBe('70 %');
    expect(text(row.querySelector('.mdone'))).toBe('470');
    expect(text(row.querySelector('.mprog small'))).toBe('Downloading: 470 of 668 MB');
    // Failed: the row is drawn again, with Retry.
    feeds.model({ state: 'failed', done: 500 * MB, total: TOTAL });
    expect(title()).toBe('Speech recognition');
    expect(mrow()).not.toBe(row);
    expect(mrow().dataset.model).toBe('failed');
    expect(text(mrow().querySelector('.stx small.warn'))).toBe('Did not download · 500 of 668 MB');
    expect(mrow().querySelector('.mpct')).toBeNull();
    const retry = mrow().querySelector<HTMLButtonElement>('[data-model-retry]')!;
    expect(text(retry.querySelector('.lbl'))).toBe('Retry');
    click(retry);
    await settle();
    expect(core.modelRetry).toHaveBeenCalledTimes(1);
    // Ready: green, with its size; no bar, no percent, no Retry.
    feeds.model({ state: 'ready', total: TOTAL });
    expect(text(mrow().querySelector('.stx small.ok'))).toBe('Ready · 668 MB on this computer');
    expect(mrow().querySelector('.mbar')).toBeNull();
    expect(mrow().querySelector('.mpct')).toBeNull();
    expect(mrow().querySelector('[data-model-retry]')).toBeNull();
    // Back on the first page, the row says so too.
    key('Escape');
    expect(text($('.btray em[data-val="model"]'))).toBe('Ready');
  });

  it('Languages shows the interface language and the translation, and holds both controls', async () => {
    stub(view({ uiLanguage: 'en', translateTarget: 'ru' }));
    page = await mount();
    await open();
    const value = () => text($('.btray em[data-val="languages"]'));
    expect(value()).toBe('English · into Russian');
    await sub('languages');
    expect(title()).toBe('Languages');
    const rows = $$('.btray .srow');
    expect(rows.map((r) => text(r.querySelector('.stx b')))).toEqual(['Interface language', 'Translate into']);
    const seg = [...rows[0].querySelectorAll('.segc button')];
    expect(seg.map((b) => b.textContent)).toEqual(['Auto', 'English', 'Русский']);
    expect(seg.map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'true', 'false']);
    expect(text(rows[1].querySelector('[data-dd="target"] .ddv'))).toBe('Russian');
    click($('[data-dd="target"]'));
    click($$('.menu [data-i]').find((b) => b.textContent === 'German')!);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'translateTarget', value: 'de' });
    expect(text($('[data-dd="target"] .ddv'))).toBe('German');
    key('Escape');
    expect(value()).toBe('English · into German');
    // In Russian the target is written in lower case after «на».
    page.dispose();
    stub(view({ lang: 'ru', uiLanguage: 'ru', translateTarget: 'en' }));
    page = await mount();
    await open();
    expect(value()).toBe('Русский · на английский');
    await sub('languages');
    expect(title()).toBe('Языки');
    expect($$('.btray .srow .stx b').map(text)).toEqual(['Язык интерфейса', 'Переводить на']);
  });

  it('General holds only start at sign-in, and its value follows the switch', async () => {
    page = await mount();
    await open();
    const value = () => text($('.btray em[data-val="general"]'));
    expect(value()).toBe('Manually');
    await sub('general');
    expect(title()).toBe('General');
    expect($$('.btray .srow').map((r) => text(r.querySelector('.stx b')))).toEqual(['Start at sign-in']);
    expect($('.btray .segc')).toBeNull();
    click($('[data-sw="autostart"]'));
    await settle();
    expect(core.autostartSet).toHaveBeenCalledWith(true);
    key('Escape');
    expect(value()).toBe('Starts at sign-in');
    // Read again on every opening: changed elsewhere, the value shows it.
    key('Escape');
    core.autostartGet.mockResolvedValue(false);
    await open();
    expect(value()).toBe('Manually');
    // In Russian too.
    page.dispose();
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    core.autostartGet.mockResolvedValue(true);
    page = await mount();
    await open();
    expect(value()).toBe('Запуск при входе');
  });

  it('the heading line stays from page to page, and a page is filled in where it is', async () => {
    let listed: (v: { names: string[]; default: string | null }) => void = () => {};
    core.micList.mockReturnValue(new Promise((r) => (listed = r)));
    page = await mount();
    await open();
    const head = $('.btray .tph')!;
    const back = $('.btray [data-tback]')!;
    // Under the first page the chevron is out of reach.
    expect(head.classList.contains('root')).toBe(true);
    expect(back.tabIndex).toBe(-1);
    expect(back.getAttribute('aria-hidden')).toBe('true');
    click($('.btray [data-sub="mic"]'));
    expect($('.btray .tph')).toBe(head);
    expect(head.classList.contains('root')).toBe(false);
    expect(back.tabIndex).toBe(0);
    expect(title()).toBe('Microphone');
    // The microphones come while the page slides in: it is filled in, not drawn anew.
    const shown = $('.btray .tpb > div:last-child')!;
    listed({ names: ['Laptop Microphone', 'USB Microphone'], default: 'Laptop Microphone' });
    await settle();
    expect($('.btray .tpb > div:last-child')).toBe(shown);
    expect(shown.querySelector('[data-dd="mic"]')).not.toBeNull();
    click(back);
    expect($('.btray .tph')).toBe(head);
    expect(title()).toBe('Settings');
  });

  it('mic page starts the preview and leaving it stops it', async () => {
    page = await mount();
    // A preview a reloaded page left running ends at the start.
    expect(previews()).toEqual(['off']);
    core.micPreview.mockClear();
    await open();
    expect(core.micPreview).not.toHaveBeenCalled();
    await sub('mic');
    expect(previews()).toEqual(['on:windows']);
    click($('.btray [data-tback]'));
    expect(previews()).toEqual(['on:windows', 'off']);
    await sub('mic');
    key('Escape');
    expect(previews()).toEqual(['on:windows', 'off', 'on:windows', 'off']);
    await sub('mic');
    click($('.btray [data-tclose]'));
    expect(previews()).toEqual(['on:windows', 'off', 'on:windows', 'off', 'on:windows', 'off']);
    await open();
    await sub('mic');
    window.dispatchEvent(new Event('pagehide'));
    expect(previews().slice(-2)).toEqual(['on:windows', 'off']);
    // Other pages never open the microphone.
    core.micPreview.mockClear();
    click($('.btray [data-tback]'));
    await sub('model');
    key('Escape');
    await sub('general');
    expect(previews()).toEqual([]);
  });

  it('the mic page starts its preview anew when the window has the focus again', async () => {
    page = await mount();
    await open();
    await sub('mic');
    core.micPreview.mockClear();
    // The welcome window took the preview meanwhile: its meter fell to 0.
    window.dispatchEvent(new Event('focus'));
    expect(previews()).toEqual(['on:windows']);
    // Off the Microphone page, or with the tray closed, the microphone stays closed.
    click($('.btray [data-tback]'));
    core.micPreview.mockClear();
    window.dispatchEvent(new Event('focus'));
    await sub('model');
    window.dispatchEvent(new Event('focus'));
    key('Escape');
    key('Escape');
    window.dispatchEvent(new Event('focus'));
    expect(previews()).toEqual([]);
  });

  it('mic preview follows the pinned microphone and a new choice', async () => {
    stub(view({ micDevice: 'USB Microphone' }));
    core.micList.mockResolvedValue({ names: ['Laptop Microphone', 'USB Microphone'], default: 'Laptop Microphone' });
    page = await mount();
    await open();
    await sub('mic');
    expect(previews().at(-1)).toBe('on:USB Microphone');
    click($('[data-dd="mic"]'));
    click($$('.menu [data-i]')[1]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'micDevice', value: 'Laptop Microphone' });
    expect(previews().at(-1)).toBe('on:Laptop Microphone');
    expect(text($('[data-dd="mic"] .ddv'))).toBe('Laptop Microphone');
    // "As in Windows" unpins.
    click($('[data-dd="mic"]'));
    click($$('.menu [data-i]')[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenLastCalledWith({ kind: 'micDevice', value: null });
    expect(previews().at(-1)).toBe('on:windows');
  });

  it('the microphone row has no label under the Microphone heading: the drop-down fills it and keeps that name', async () => {
    stub(view({ micDevice: 'Old Headset' }));
    page = await mount();
    await open();
    await sub('mic');
    expect(title()).toBe('Microphone');
    const [choose, test] = $$('.btray .srow');
    expect(choose.querySelector('b')).toBeNull();
    expect(choose.querySelector('.sc')).toBeNull();
    const dd = choose.querySelector<HTMLElement>('.stx > [data-dd="mic"]')!;
    expect(dd.getAttribute('aria-label')).toBe('Microphone');
    expect(dd.title).toBe('Old Headset');
    // The hint still shows under it while the pinned one is not connected.
    const hint = choose.querySelector<HTMLElement>('.stx > [data-dd="mic"] + small')!;
    expect(hint.hidden).toBe(false);
    expect(text(hint)).toBe('Not connected — recording from the Windows microphone');
    expect(text(test.querySelector('.stx b'))).toBe('Test');
    // In Russian too.
    page.dispose();
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    await sub('mic');
    expect($('[data-dd="mic"]')!.getAttribute('aria-label')).toBe('Микрофон');
    expect($('[data-dd="mic"] + small')!.hidden).toBe(true);
  });

  it('mic level lights the meter', async () => {
    page = await mount();
    await open();
    const lit = () => $$('[data-meter] i.on').length;
    feeds.level(0.9);
    await sub('mic');
    expect($$('[data-meter] i')).toHaveLength(16);
    expect(lit()).toBe(0);
    feeds.level(0.5);
    expect(lit()).toBe(8);
    feeds.level(1.4);
    expect(lit()).toBe(16);
    feeds.level(0);
    expect(lit()).toBe(0);
  });

  it('missing pinned mic is shown as not connected', async () => {
    stub(view({ micDevice: 'Old Headset' }));
    core.micList.mockResolvedValue({
      names: ['Laptop Microphone', 'USB <img src=x> Mic', 'USB <img src=x> Mic'],
      default: 'Laptop Microphone',
    });
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="mic"]'))).toBe('Old Headset');
    await sub('mic');
    expect(text($('[data-dd="mic"] .ddv'))).toBe('Old Headset');
    expect(text($('.btray .srow small'))).toBe('Not connected — recording from the Windows microphone');
    click($('[data-dd="mic"]'));
    const items = $$('.menu [data-i]');
    // Each name once; the pinned one last, marked; device names as text.
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual([
      'As in Windows (Laptop Microphone)',
      'Laptop Microphone',
      'USB <img src=x> Mic',
      'Old Headset',
    ]);
    expect($('.menu img')).toBeNull();
    expect(text(items[3].querySelector('.mnc'))).toBe('not connected');
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'false', 'false', 'true']);
  });

  it('a pinned mic matches as the core matches it: trimmed, then in any case', async () => {
    expect(matchMic(['USB Mic', 'usb mic'], ' usb mic ')).toBe(1);
    expect(matchMic(['USB Mic ', 'Laptop'], 'usb mic')).toBe(0);
    expect(matchMic(['Laptop'], 'USB Mic')).toBe(-1);
    expect(matchMic(['Laptop'], '  ')).toBe(-1);
    stub(view({ micDevice: 'usb microphone' }));
    core.micList.mockResolvedValue({ names: ['Laptop Microphone', 'USB Microphone'], default: null });
    page = await mount();
    await open();
    await sub('mic');
    expect($('.btray .srow small')!.hidden).toBe(true);
    expect(text($('[data-dd="mic"] .ddv'))).toBe('USB Microphone');
    click($('[data-dd="mic"]'));
    const items = $$('.menu [data-i]');
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['As in Windows', 'Laptop Microphone', 'USB Microphone']);
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true']);
  });

  it('the translation menu on Languages lists 60 languages named in the window language', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="languages"]'))).toBe('Русский · на английский');
    await sub('languages');
    expect(title()).toBe('Языки');
    click($('[data-dd="target"]'));
    const labels = $$('.menu [data-i] .mlb').map((e) => e.textContent!);
    expect(labels).toHaveLength(60);
    expect(labels).toContain('Немецкий');
    expect(labels).toContain('Казахский');
    expect([...labels].sort((a, b) => a.localeCompare(b, 'ru'))).toEqual(labels);
    const on = $$('.menu [data-i]').filter((b) => b.getAttribute('aria-checked') === 'true');
    expect(on.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['Английский']);
    click($$('.menu [data-i]').find((b) => b.textContent === 'Немецкий')!);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'translateTarget', value: 'de' });
    expect(text($('[data-dd="target"] .ddv'))).toBe('Немецкий');
  });

  it('language switch re-renders the page', async () => {
    core.historyPage.mockResolvedValue({ days: [{ date: TODAY, entries: [entry(`${TODAY}T09:15:00.000-1`, 'Kept.')] }], more: false });
    page = await mount();
    expect(text($('.bsec h3'))).toBe('History');
    expect(document.documentElement.lang).toBe('en');
    await open();
    await sub('languages');
    const seg = $$('.segc button');
    expect(seg.map((b) => b.textContent)).toEqual(['Auto', 'English', 'Русский']);
    expect(seg.map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false', 'false']);
    click(seg[2]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'uiLanguage', value: 'ru' });
    expect(document.documentElement.lang).toBe('ru');
    expect(text($('.bsec h3'))).toBe('История');
    expect(text($('.hday'))).toBe('Сегодня');
    expect(text($('.hr .htx'))).toBe('Kept.');
    // The tray stays open on its page, in the new language.
    expect(tray().classList.contains('open')).toBe(true);
    expect(title()).toBe('Языки');
    expect($$('.segc button').map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'true']);
    expect($('.gear')!.getAttribute('aria-label')).toBe('Настройки');
    key('Escape');
    expect(title()).toBe('Настройки');
  });

  it('the History page keeps the history, then its recordings, then clears it', async () => {
    page = await mount();
    await open();
    await sub('history');
    expect(title()).toBe('History');
    const rows = $$('.btray .srow');
    expect(rows.map((r) => text(r.querySelector('.stx b')))).toEqual(['Keep history', 'Keep recordings', 'Clear history']);
    expect(rows.map((r) => text(r.querySelector('.stx small')))).toEqual([
      'Older entries go by themselves',
      'Never longer than the history',
      'Takes a second press',
    ]);
    expect(rows[0].querySelector('[data-dd="hkeep"]')).not.toBeNull();
    expect(rows[1].querySelector('[data-dd="haudio"]')).not.toBeNull();
    expect(text(rows[2].querySelector('[data-hclear]'))).toBe('Clear');
    // In Russian too.
    page.dispose();
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="history"]'))).toBe('Месяц');
    await sub('history');
    expect($$('.btray .srow .stx b').map(text)).toEqual(['Хранить историю', 'Хранить записи', 'Очистить историю']);
    expect($$('.btray .srow .stx small').map(text)).toEqual(['Старое удаляется само', 'Не дольше самой истории', 'Со второго нажатия']);
    expect(text($('[data-hclear]'))).toBe('Очистить');
    click($('[data-hclear]'));
    expect(text($('[data-hclear]'))).toBe('Нажмите ещё раз');
  });

  it('the History page keeps the history for the period chosen, and the History row shows it', async () => {
    page = await mount();
    await open();
    await sub('history');
    const dd = $('[data-dd="hkeep"]')!;
    expect(text(dd.querySelector('.ddv'))).toBe('A month');
    click(dd);
    const items = $$('.menu [data-i]');
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['Forever', 'A year', 'A month', 'A week', "Don't keep"]);
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'false', 'true', 'false', 'false']);
    expect(text($('.menu .mnote'))).toBe('A shorter period deletes older days at once.');
    click(items[3]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'historyKeep', value: 'week' });
    expect($('.menu')).toBeNull();
    expect(text($('[data-dd="hkeep"] .ddv'))).toBe('A week');
    // Older days may be gone: the list is read again.
    expect(core.historyPage).toHaveBeenCalledTimes(2);
    key('Escape');
    expect(text($('.btray em[data-val="history"]'))).toBe('A week');
  });

  it('an unknown history period shows unset, with nothing chosen', async () => {
    stub(view({ historyKeep: null }));
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="history"]'))).toBe('Not set');
    await sub('history');
    const dd = $('[data-dd="hkeep"]')!;
    expect(text(dd.querySelector('.ddv'))).toBe('Not set');
    click(dd);
    expect($$('.menu [data-i]').every((b) => b.getAttribute('aria-checked') === 'false')).toBe(true);
    // A second press on it closes the menu.
    click(dd);
    expect($('.menu')).toBeNull();
    // Esc closes a menu before it goes back.
    click(dd);
    key('Escape');
    expect($('.menu')).toBeNull();
    expect(title()).toBe('History');
  });

  it('clear history needs a second press within 3 s, and empties the list', async () => {
    core.historyPage.mockResolvedValue({ days: [{ date: TODAY, entries: [entry(`${TODAY}T09:15:00.000-1`, 'Kept.')] }], more: false });
    page = await mount();
    expect($$('.hr')).toHaveLength(1);
    await open();
    await sub('history');
    const clear = $('[data-hclear]')!;
    expect(text(clear)).toBe('Clear');
    click(clear);
    expect(text(clear)).toBe('Press again');
    expect(core.historyClear).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(CLEAR_ARMED_MS);
    expect(text(clear)).toBe('Clear');
    // A first press again, then a second one in time.
    click(clear);
    await vi.advanceTimersByTimeAsync(2000);
    click(clear);
    await settle();
    expect(core.historyClear).toHaveBeenCalledTimes(1);
    expect(text(clear)).toBe('Clear');
    expect($$('.hr')).toHaveLength(0);
    expect($('.hempty')!.hidden).toBe(false);
    // The tray stays open on its page.
    expect(tray().classList.contains('open')).toBe(true);
    expect(title()).toBe('History');
  });

  it('a first press of clear history does not outlive its page', async () => {
    page = await mount();
    await open();
    await sub('history');
    click($('[data-hclear]'));
    expect(text($('[data-hclear]'))).toBe('Press again');
    // Back and in again within the 3 s: one press only asks again.
    key('Escape');
    await sub('history');
    expect(text($('[data-hclear]'))).toBe('Clear');
    click($('[data-hclear]'));
    await settle();
    expect(core.historyClear).not.toHaveBeenCalled();
    expect(text($('[data-hclear]'))).toBe('Press again');
    // Closed and opened again: the same.
    key('Escape');
    key('Escape');
    await open();
    await sub('history');
    click($('[data-hclear]'));
    await settle();
    expect(core.historyClear).not.toHaveBeenCalled();
    // New settings draw the page anew: the same.
    click($('[data-dd="haudio"]'));
    click($$('.menu [data-i]')[0]);
    await settle();
    expect(text($('[data-hclear]'))).toBe('Clear');
    click($('[data-hclear]'));
    await settle();
    expect(core.historyClear).not.toHaveBeenCalled();
    // The timer of the press before does not reset the label of the new button.
    await vi.advanceTimersByTimeAsync(CLEAR_ARMED_MS - 500);
    expect(text($('[data-hclear]'))).toBe('Press again');
  });

  it('the History page keeps recordings for the period chosen', async () => {
    page = await mount();
    await open();
    await sub('history');
    const dd = $('[data-dd="haudio"]')!;
    expect(text(dd.querySelector('.ddv'))).toBe('A week');
    click(dd);
    const items = $$('.menu [data-i]');
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['A day', 'A week', 'A month', "Don't keep"]);
    expect(items.map((b) => b.getAttribute('aria-checked'))).toEqual(['false', 'true', 'false', 'false']);
    expect(text($('.menu .mnote'))).toBe('A shorter period deletes older recordings at once.');
    click(items[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'historyAudio', value: 'day' });
    expect($('.menu')).toBeNull();
    expect(text($('[data-dd="haudio"] .ddv'))).toBe('A day');
    // The history is read again: its entries say anew which have a recording to play.
    expect(core.historyPage).toHaveBeenCalledTimes(2);
    click($('[data-dd="haudio"]'));
    click($$('.menu [data-i]')[3]);
    await settle();
    expect(core.settingsSet).toHaveBeenLastCalledWith({ kind: 'historyAudio', value: 'off' });
    expect(text($('[data-dd="haudio"] .ddv'))).toBe("Don't keep");
    // The History row shows how long the history is kept, not its recordings.
    key('Escape');
    expect(text($('.btray em[data-val="history"]'))).toBe('A month');
  });

  it('an unknown recordings period shows unset, with nothing chosen, in Russian too', async () => {
    stub(view({ historyAudio: null, lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    expect($$('.btray .trw span:not(.si)').map((e) => e.textContent)).toContain('История');
    await sub('history');
    expect(text($('[data-dd="haudio"]')!.closest('.srow')!.querySelector('.stx b'))).toBe('Хранить записи');
    const dd = $('[data-dd="haudio"]')!;
    expect(text(dd.querySelector('.ddv'))).toBe('Не задано');
    click(dd);
    const items = $$('.menu [data-i]');
    expect(items.map((b) => b.querySelector('.mlb')!.textContent)).toEqual(['День', 'Неделю', 'Месяц', 'Не хранить']);
    expect(items.every((b) => b.getAttribute('aria-checked') === 'false')).toBe(true);
    click(items[2]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'historyAudio', value: 'month' });
    expect(text($('[data-dd="haudio"] .ddv'))).toBe('Месяц');
  });

  it('autostart switch calls autostart_set', async () => {
    page = await mount();
    await open();
    await sub('general');
    expect(core.autostartGet).toHaveBeenCalled();
    const sw = $<HTMLButtonElement>('[data-sw="autostart"]')!;
    expect(sw.getAttribute('role')).toBe('switch');
    expect(sw.getAttribute('aria-checked')).toBe('false');
    click(sw);
    await settle();
    expect(core.autostartSet).toHaveBeenCalledWith(true);
    expect(sw.getAttribute('aria-checked')).toBe('true');
    click(sw);
    await settle();
    expect(core.autostartSet).toHaveBeenLastCalledWith(false);
    expect(sw.getAttribute('aria-checked')).toBe('false');
  });

  it('the General page has "Start at sign-in" only: "Learn from me" lives in the Habits heading', async () => {
    page = await mount();
    await open();
    await sub('general');
    expect($('.btray [data-sw="learn"]')).toBeNull();
    expect($('.btray [data-sw="autostart"]')).not.toBeNull();
  });

  it('settings pushed by the core leave the tray as it is: the Habits switch follows', async () => {
    page = await mount();
    await open();
    await sub('cleanup');
    const group = $('.btray .sgrp');
    feeds.settings(view({ dictionary: ['Marisol'], learned: [{ word: 'Marisol', heard: 'Марисоль' }] }));
    expect($('.dch[data-word="Marisol"]')).not.toBeNull();
    feeds.settings(view({ learn: false }));
    expect($('.bsec[data-scr="habits"] [data-sw="learn"]')!.getAttribute('aria-checked')).toBe('false');
    expect(title()).toBe('Cleanup');
    expect(group).not.toBeNull();
  });

  it('the Cleanup page: where it runs, and the note', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    const value = () => text($('.btray em[data-val="cleanup"]'));
    expect(value()).toBe('Облако');
    await sub('cleanup');
    expect(title()).toBe('Чистка текста');
    const seg = () => $$('[data-seg="runs"] button');
    expect(seg().map((b) => b.textContent)).toEqual(['Этот компьютер', 'Облако']);
    expect(seg().map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'true']);
    expect(text($('.btray .tft'))).toBe('Текст уходит в Облако для чистки; история — только на этом компьютере.');
  });

  it('reads the settings again when the core did not answer a change in time', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    page = await mount();
    core.settingsSet.mockRejectedValueOnce('core');
    core.settingsGet.mockResolvedValueOnce(view({ translateTarget: 'fr' }));
    await open();
    await sub('languages');
    click($('[data-dd="target"]'));
    click($$('.menu [data-i]').find((b) => b.textContent === 'French')!);
    await settle();
    expect(text($('[data-dd="target"] .ddv'))).toBe('French');
    expect(errors).toHaveBeenCalledWith('CUE: setting translateTarget failed:', 'core');
    errors.mockRestore();
  });
});
