import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, mount, settle, stub, text, type, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import type { LlmFound, ModelsView } from '../src/window/api';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const GB = 1_000_000_000;
const GEMMA = 'llm--google--gemma-4-e4b-it-gguf--q4_k_m';
const QWEN = 'llm--qwen--qwen3-4b-gguf--q4_k_m';

/** The local list: Gemma recommended, Qwen not downloaded; `over` changes an entry or the one in use. */
function llms(over: Partial<Record<string, object>> = {}, current = GEMMA): ModelsView {
  const entries = [
    { id: GEMMA, name: 'Gemma 4 E4B', repo: 'google/gemma-4-E4B-it-GGUF', languages: [], size: 3.1 * GB, state: 'inUse' },
    { id: QWEN, name: 'Qwen3 4B', repo: 'Qwen/Qwen3-4B-GGUF', languages: [], size: 2.5 * GB, state: 'missing' },
  ].map((e) => ({ ...e, ...over[e.id] }));
  return { current, entries, recommended: GEMMA } as ModelsView;
}

const open = async () => {
  click($('.gear'));
  await settle();
};
const cleanup = async () => {
  await open();
  click($('.btray [data-sub="cleanup"]'));
  await settle();
};
const value = () => $<HTMLElement>('.btray em[data-val="cleanup"]')!;
const runs = () => $$('[data-seg="runs"] button');
const names = () => $$('.btray .mlist [data-mid] .stx b').map((b) => text(b));
const row = (id: string) => $<HTMLElement>(`.btray [data-mid="${id}"]`)!;
const small = (e: Element) => text(e.querySelector('.stx small'));

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
  core.keyState.mockResolvedValue({ saved: true, mask: 'sk-…789' });
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The Cleanup page: on this computer or in the cloud', () => {
  it('the style first, then "Runs on"; the cloud shows the card, the key and the model', async () => {
    page = await mount();
    await cleanup();
    const rows = $$('.btray .sgrp > .srow');
    expect(text(rows[0].querySelector('.stx b'))).toBe('Style');
    expect(text(rows[1].querySelector('.stx b'))).toBe('Runs on');
    expect(text(rows[1].querySelector('.stx small'))).toBe('Your key, fastest');
    expect(runs().map((b) => b.textContent)).toEqual(['This computer', 'Cloud']);
    expect(runs().map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'true']);
    expect($('.btray .pk[data-dd="connection"]')).not.toBeNull();
    expect($('.btray .srow[data-key="anthropic"]')).not.toBeNull();
    expect($('.btray [data-dd="rmodel"]')).not.toBeNull();
    expect($<HTMLElement>('.btray .mlist')?.hidden ?? true).toBe(true);
    expect(text($('.btray .tft'))).toBe('Text goes to Anthropic for the cleanup; the history stays on this computer only.');
  });

  it('the cloud lists only the models downloaded here, to delete', async () => {
    core.llmsGet.mockResolvedValue(llms({ [GEMMA]: { state: 'downloaded' } }, ''));
    page = await mount();
    await cleanup();
    const list = $<HTMLElement>('.btray .mlist')!;
    expect(list.hidden).toBe(false);
    expect(text(list.querySelector('.mgh'))).toBe('Models on this computer');
    expect(names()).toEqual(['Gemma 4 E4B']);
    expect($('.btray .msr')).toBeNull();
    click(row(GEMMA));
    expect(core.llmChoose).not.toHaveBeenCalled();
    click(row(GEMMA).querySelector('.mdel'));
    expect(core.llmDelete).toHaveBeenCalledWith(GEMMA);
  });

  it('on this computer the model in use can be deleted too', async () => {
    stub(view({ refineRuns: 'local' }));
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await cleanup();
    expect(row(GEMMA).querySelector('.mok')).not.toBeNull();
    click(row(GEMMA).querySelector('.mdel'));
    expect(core.llmDelete).toHaveBeenCalledWith(GEMMA);
    expect(core.llmChoose).not.toHaveBeenCalled();
  });

  it('this computer shows the local models, the search and the free note', async () => {
    stub(view({ refineRuns: 'local' }));
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await cleanup();
    expect(runs().map((b) => b.getAttribute('aria-pressed'))).toEqual(['true', 'false']);
    expect(text($$('.btray .sgrp > .srow')[1].querySelector('.stx small'))).toBe('Free, private');
    expect(names()).toEqual(['Gemma 4 E4B', 'Qwen3 4B']);
    expect(small(row(GEMMA))).toBe('Recommended · Russian and English · 3.1 GB');
    expect(row(GEMMA).querySelector('.mok')).not.toBeNull();
    expect(text(row(QWEN).querySelector('.msz'))).toBe('2.5 GB');
    expect($('.btray .msr')).not.toBeNull();
    expect($('.btray .pk')).toBeNull();
    expect($('.btray [data-dd="rmodel"]')).toBeNull();
    expect(text($('.btray .tft'))).toBe('Free. The text never leaves this computer; the model runs on its processor.');
  });

  it('pressing the other half moves the cleanup there', async () => {
    page = await mount();
    await cleanup();
    click(runs()[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineRuns', value: 'local' });
    expect(runs()[0].getAttribute('aria-pressed')).toBe('true');
    expect($('.btray .mlist')).not.toBeNull();
    expect($('.btray .pk')).toBeNull();
  });

  it('a local row is chosen, and a found one; one CUE cannot run is pale with its reason', async () => {
    stub(view({ refineRuns: 'local' }));
    core.llmsGet.mockResolvedValue(llms());
    const answer: LlmFound[] = [
      { repo: 'Qwen/Qwen3-1.7B-GGUF', downloads: 900, size: 1.8 * GB, quant: 'Q8_0', reason: null },
      { repo: 'big/split-GGUF', downloads: 30, size: null, quant: null, reason: 'severalFiles' },
    ];
    core.llmsSearch.mockResolvedValue(answer);
    page = await mount();
    await cleanup();
    click(row(QWEN));
    expect(core.llmChoose).toHaveBeenCalledWith({ id: QWEN });
    expect(core.modelChoose).not.toHaveBeenCalled();
    type($<HTMLInputElement>('.btray .msr input')!, 'qwen');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    expect(core.llmsSearch).toHaveBeenCalledWith('qwen');
    const found = $$('.btray .mfound [data-repo]');
    expect(found.map((r) => text(r.querySelector('.stx b')))).toEqual(['Qwen/Qwen3-1.7B-GGUF', 'big/split-GGUF']);
    expect(small(found[0])).toBe('900 downloads · 1.8 GB · Q8_0');
    expect(found[1].classList.contains('off')).toBe(true);
    expect(small(found[1])).toBe('several files');
    click(found[0]);
    expect(core.llmChoose).toHaveBeenCalledWith({ repo: 'Qwen/Qwen3-1.7B-GGUF' });
    click(found[1]);
    expect(core.llmChoose).toHaveBeenCalledTimes(2);
  });

  it('the first page says where the cleanup runs and what its model is doing', async () => {
    page = await mount();
    await open();
    expect(text(value())).toBe('Anthropic');
    expect(value().classList.contains('warn')).toBe(false);
    feeds.settings(view({ refineRuns: 'local' }));
    feeds.llms(llms({ [GEMMA]: { state: 'missing' } }, ''));
    await settle();
    expect(text(value())).toBe('Model not downloaded');
    expect(value().classList.contains('warn')).toBe(true);
    feeds.llms(llms({ [GEMMA]: { state: 'downloading', done: 42, total: 100 } }, ''));
    expect(text(value())).toBe('Downloading 42 %');
    expect(value().classList.contains('warn')).toBe(true);
    feeds.llms(llms());
    expect(text(value())).toBe('This computer');
    expect(value().classList.contains('warn')).toBe(false);
  });

  it('Russian words', async () => {
    stub(view({ refineRuns: 'local', lang: 'ru', uiLanguage: 'ru' }));
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await open();
    expect(text(value())).toBe('Этот компьютер');
    click($('.btray [data-sub="cleanup"]'));
    await settle();
    expect(runs().map((b) => b.textContent)).toEqual(['Этот компьютер', 'Облако']);
    expect(small(row(GEMMA))).toBe('Рекомендуем · русский и английский · 3,1 ГБ');
    expect(text($('.btray .tft'))).toBe('Бесплатно. Текст не покидает этот компьютер; модель работает на его процессоре.');
  });
});
