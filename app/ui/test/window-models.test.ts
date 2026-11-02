import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, key, modelsView, mount, settle, stub, text, type, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import type { Found } from '../src/window/api';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const MB = 1_000_000;
const V3 = 670_479_942;

const title = () => text($('.btray .tph b'));
const open = async () => {
  click($('.gear'));
  await settle();
};
/** The Model page, through Speech recognition. */
const models = async () => {
  await open();
  click($('.btray [data-sub="model"]'));
  await settle();
  click($('.btray .mrow[data-sub="models"]'));
  await settle();
};
const row = (id: string) => $<HTMLElement>(`.btray [data-mid="${id}"]`)!;
const names = () => $$('.btray .mlist [data-mid] .stx b').map((b) => text(b));
const small = (e: Element) => text(e.querySelector('.stx small'));
const search = () => $<HTMLElement>('.btray .msr')!;
const field = () => $<HTMLInputElement>('.btray .msr input')!;
const found = () => $$('.btray .mfound [data-repo]');

/** A search's answer: a runnable model, one on the short list, Whisper, sherpa-onnx's layout. */
const answer: Found[] = [
  { repo: 'someone/parakeet-tdt-0.6b-v3-greek', downloads: 134, size: 668 * MB, reason: null, languages: ['el'] },
  { repo: 'istupakov/parakeet-tdt-0.6b-v3-onnx', downloads: 126_454, size: V3, reason: null, languages: ['en'] },
  { repo: 'onnx-community/whisper-small', downloads: 9_000, size: null, reason: 'architecture', languages: ['en'] },
  { repo: 'csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8', downloads: 1, size: null, reason: 'layout', languages: [] },
];

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The Model page', () => {
  it('opens from the model row of Speech recognition and goes back to it', async () => {
    page = await mount();
    await models();
    expect(title()).toBe('Model');
    expect(names()).toEqual(['Parakeet Ultra', 'Parakeet v3', 'Parakeet v2']);
    expect(small(row('parakeet-ultra-int8'))).toBe('25 languages · 668 MB');
    expect(row('parakeet-ultra-int8').querySelector('.mok')).not.toBeNull();
    expect(small(row('parakeet-v3-int8'))).toBe('25 languages');
    expect(text(row('parakeet-v3-int8').querySelector('.msz'))).toBe('670 MB');
    expect(small(row('parakeet-v2-int8'))).toBe('English only');
    expect(text($('.btray .mlist .mgh'))).toBe('Models');
    expect($('.btray .mfound')!.hidden).toBe(true);
    key('Escape');
    expect(title()).toBe('Speech recognition');
    key('Escape');
    expect(title()).toBe('Settings');
  });

  it('Retry on the model row retries the download and stays; Enter on the row opens the Model page', async () => {
    core.modelGet.mockResolvedValue({ state: 'failed', done: 0, total: 667_729_022 });
    page = await mount();
    await open();
    click($('.btray [data-sub="model"]'));
    await settle();
    click($('.btray .mrow [data-model-retry]'));
    await settle();
    expect(core.modelRetry).toHaveBeenCalledTimes(1);
    expect(title()).toBe('Speech recognition');
    const mrow = $<HTMLElement>('.btray .mrow[data-sub="models"]')!;
    mrow.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle();
    expect(title()).toBe('Model');
  });

  it('a chosen model downloads in its row, and takes the check once it is ready', async () => {
    page = await mount();
    await models();
    click(row('parakeet-v3-int8'));
    await settle();
    expect(core.modelChoose).toHaveBeenCalledWith({ id: 'parakeet-v3-int8' });
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'downloading', done: 412 * MB, total: V3 } }));
    const r = row('parakeet-v3-int8');
    expect(text(r.querySelector('.mpct'))).toBe('61 %');
    expect(text(r.querySelector('.mprog small'))).toBe('Downloading: 412 of 670 MB');
    expect(row('parakeet-ultra-int8').querySelector('.mok')).not.toBeNull();
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'downloading', done: 600 * MB, total: V3 } }));
    expect(row('parakeet-v3-int8')).toBe(r);
    expect(text(r.querySelector('.mpct'))).toBe('89 %');
    expect(text(r.querySelector('.mdone'))).toBe('600');
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'loading' } }));
    expect(small(row('parakeet-v3-int8'))).toBe('Loading');
    feeds.models(
      modelsView({ 'parakeet-v3-int8': { state: 'inUse' }, 'parakeet-ultra-int8': { state: 'downloaded' } }, 'parakeet-v3-int8'),
    );
    expect(row('parakeet-v3-int8').querySelector('.mok')).not.toBeNull();
    expect(row('parakeet-ultra-int8').querySelector('.mok')).toBeNull();
    // Speech recognition names the model in use.
    key('Escape');
    expect(text($('.btray .mrow .stx b'))).toBe('Parakeet v3');
    // Choosing the one in use asks nothing.
    click($('.btray .mrow[data-sub="models"]'));
    await settle();
    click(row('parakeet-v3-int8'));
    await settle();
    expect(core.modelChoose).toHaveBeenCalledTimes(1);
  });

  it('a switch that stopped says why, with Retry where trying again helps', async () => {
    page = await mount();
    await models();
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'failed', done: 412 * MB, total: V3 } }));
    expect(small(row('parakeet-v3-int8'))).toBe('Did not download · 412 of 670 MB');
    click(row('parakeet-v3-int8').querySelector('[data-mretry]'));
    await settle();
    expect(core.modelChoose).toHaveBeenLastCalledWith({ id: 'parakeet-v3-int8' });
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'noSpace', needed: 737 * MB, free: 412 * MB } }));
    expect(small(row('parakeet-v3-int8'))).toBe('Needs 0.7 GB, 0.4 free');
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'notRunnable' } }));
    expect(small(row('parakeet-v3-int8'))).toBe("Doesn't run in CUE");
    expect(row('parakeet-v3-int8').querySelector('[data-mretry]')).toBeNull();
    feeds.models(modelsView({ 'parakeet-v3-int8': { state: 'preparing' } }));
    expect(small(row('parakeet-v3-int8'))).toBe('Getting ready');
  });

  it('the search is the last row of the list, and opens into a field where it stands', async () => {
    page = await mount();
    await models();
    // After the list, before what the search finds; quiet until pressed.
    const after = search().previousElementSibling as HTMLElement;
    expect(after.classList.contains('mlist')).toBe(true);
    expect(text(search().querySelector('.msl'))).toBe('Search Hugging Face');
    expect(search().classList.contains('open')).toBe(false);
    expect(field().placeholder).toBe('Search Hugging Face');
    // Pressed anywhere on the row, the field takes the focus and the row opens.
    search().dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
    expect(document.activeElement).toBe(field());
    expect(search().classList.contains('open')).toBe(true);
    // Typed: a cross clears it, the focus stays.
    type(field(), 'gr');
    expect(search().classList.contains('typed')).toBe(true);
    click(search().querySelector('.msx'));
    expect(field().value).toBe('');
    expect(search().classList.contains('typed')).toBe(false);
    expect(document.activeElement).toBe(field());
    // Esc with text clears it; Esc on an empty field closes the row and stays on the page.
    type(field(), 'greek');
    field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(field().value).toBe('');
    expect(title()).toBe('Model');
    field().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(search().classList.contains('open')).toBe(false);
    expect(title()).toBe('Model');
    // Leaving an empty field closes the row; one with text stays open.
    field().focus();
    expect(search().classList.contains('open')).toBe(true);
    field().blur();
    expect(search().classList.contains('open')).toBe(false);
    field().focus();
    type(field(), 'tdt');
    field().blur();
    expect(search().classList.contains('open')).toBe(true);
  });

  it('a downloaded model can be deleted, the one in use cannot', async () => {
    core.modelsGet.mockResolvedValue(modelsView({ 'parakeet-v3-int8': { state: 'downloaded' } }));
    page = await mount();
    await models();
    expect(row('parakeet-ultra-int8').querySelector('[data-mdel]')).toBeNull();
    const del = row('parakeet-v3-int8').querySelector('[data-mdel]')!;
    expect(text(del)).toBe('Delete');
    click(del);
    await settle();
    expect(core.modelDelete).toHaveBeenCalledWith('parakeet-v3-int8');
    expect(core.modelChoose).not.toHaveBeenCalled();
  });

  it('the search finds repositories on Hugging Face, the ones CUE cannot run pale', async () => {
    core.modelsSearch.mockResolvedValue(answer);
    page = await mount();
    await models();
    type(field(), 'g');
    await vi.advanceTimersByTimeAsync(400);
    expect(core.modelsSearch).not.toHaveBeenCalled();
    type(field(), 'gr');
    await vi.advanceTimersByTimeAsync(200);
    type(field(), 'greek');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(core.modelsSearch).toHaveBeenCalledTimes(1);
    expect(core.modelsSearch).toHaveBeenCalledWith('greek');
    expect($('.btray .mfound')!.hidden).toBe(false);
    expect(text($('.btray .mfound .mgh'))).toBe('On Hugging Face');
    // The one on the short list is not shown twice.
    expect(found().map((r) => r.dataset.repo)).toEqual([
      'someone/parakeet-tdt-0.6b-v3-greek',
      'onnx-community/whisper-small',
      'csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8',
    ]);
    expect(small(found()[0])).toBe('134 downloads · 668 MB');
    expect(found()[0].classList.contains('off')).toBe(false);
    expect(small(found()[1])).toBe('Another architecture');
    expect(found()[1].classList.contains('off')).toBe(true);
    expect(small(found()[2])).toBe('Another file layout');
    click(found()[1]);
    await settle();
    expect(core.modelChoose).not.toHaveBeenCalled();
    click(found()[0]);
    await settle();
    expect(core.modelChoose).toHaveBeenCalledWith({ repo: 'someone/parakeet-tdt-0.6b-v3-greek' });
    // Typed text stays in the field while the list is drawn again.
    feeds.models(modelsView());
    expect(field().value).toBe('greek');
    type(field(), 'x');
    await vi.advanceTimersByTimeAsync(400);
    expect($('.btray .mfound')!.hidden).toBe(true);
  });

  it('no connection, nothing found, and a late answer to an older search', async () => {
    page = await mount();
    await models();
    core.modelsSearch.mockRejectedValueOnce('failed');
    type(field(), 'tdt');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(text($('.btray .mfound .mfnote'))).toBe('No connection to Hugging Face');
    core.modelsSearch.mockResolvedValueOnce([]);
    type(field(), 'zzz');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(text($('.btray .mfound .mfnote'))).toBe('No Parakeet models for this search');
    // An older search answering after a newer one is dropped.
    let late: (f: Found[]) => void = () => {};
    core.modelsSearch.mockImplementationOnce(() => new Promise((r) => (late = r)));
    core.modelsSearch.mockResolvedValueOnce([answer[0]]);
    type(field(), 'old');
    await vi.advanceTimersByTimeAsync(300);
    type(field(), 'new');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    late(answer);
    await settle();
    expect(found().map((r) => r.dataset.repo)).toEqual(['someone/parakeet-tdt-0.6b-v3-greek']);
  });

  it('in Russian', async () => {
    stub(view({ uiLanguage: 'ru', lang: 'ru' }));
    core.modelsGet.mockResolvedValue(modelsView({ 'parakeet-v3-int8': { state: 'noSpace', needed: 737 * MB, free: 412 * MB } }));
    core.modelsSearch.mockResolvedValue(answer);
    page = await mount();
    await models();
    expect(title()).toBe('Модель');
    expect(small(row('parakeet-ultra-int8'))).toBe('25 языков · 668 МБ');
    expect(small(row('parakeet-v2-int8'))).toBe('Только английский');
    expect(small(row('parakeet-v3-int8'))).toBe('Нужно 0,7 ГБ, свободно 0,4');
    expect(field().placeholder).toBe('Найти на Hugging Face');
    type(field(), 'greek');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(text($('.btray .mfound .mgh'))).toBe('На Hugging Face');
    expect(small(found()[0])).toBe('134 скачивания · 668 МБ');
    expect(small(found()[1])).toBe('Другая архитектура');
  });
});

describe('The Model page for a screen reader', () => {
  it('names its list and what the search found; the search row only lays things out', async () => {
    page = await mount();
    await models();
    expect($('.btray .mlist')!.getAttribute('aria-label')).toBe('Models');
    expect($('.btray .mfound')!.getAttribute('aria-label')).toBe('On Hugging Face');
    expect($('.btray .msr')!.getAttribute('role')).toBe('none');
  });
});