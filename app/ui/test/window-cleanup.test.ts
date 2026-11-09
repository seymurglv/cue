import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, agents, click, core, feeds, mount, settle, stub, text, type, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import type { AgentView, LlmFound, ModelsView } from '../src/window/api';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const GB = 1_000_000_000;
const MB = 1_000_000;
// Today's short list (core/llm.rs): Qwen3 4B Instruct recommended, then Gemma 4 E2B and Qwen3 1.7B; one found model.
const QWEN = 'llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m';
const GEMMA = 'llm--unsloth--gemma-4-e2b-it-gguf--q4_k_m';
const SMALL = 'llm--unsloth--qwen3-1.7b-gguf--q4_k_m';
const TLITE = 'llm--t-tech--t-lite-it-2.1-gguf--q4_k_m';

/** The local list: nothing downloaded; `over` changes an entry, `current` is the one in use. */
function llms(over: Partial<Record<string, object>> = {}, current = ''): ModelsView {
  const entries = [
    { id: QWEN, name: 'Qwen3 4B Instruct', repo: 'unsloth/Qwen3-4B-Instruct-2507-GGUF', languages: [], size: 2.5 * GB, state: 'missing' },
    { id: GEMMA, name: 'Gemma 4 E2B', repo: 'unsloth/gemma-4-E2B-it-GGUF', languages: [], size: 3.1 * GB, state: 'missing' },
    { id: SMALL, name: 'Qwen3 1.7B', repo: 'unsloth/Qwen3-1.7B-GGUF', languages: [], size: 1.1 * GB, state: 'missing' },
  ].map((e) => ({ ...e, ...over[e.id] }));
  const found = over[TLITE] ? [{ id: TLITE, name: 'T-lite-it-2.1', repo: 't-tech/T-lite-it-2.1-GGUF', languages: [], size: 5 * GB, state: 'downloaded', ...over[TLITE] }] : [];
  return { current, entries: [...entries, ...found], recommended: QWEN, found: found.map((f) => f.id) } as ModelsView;
}

/** OpenCode's own models: two named, then many more, as an install with ~390 lists them. */
function openCodeModels(): AgentView['models'] {
  const many = Array.from({ length: 388 }, (_, i) => ({ id: `provider${i % 20}/model-${i}`, name: `Model ${i}` }));
  return [{ id: 'anthropic/claude-haiku-5-5', name: 'Claude Haiku 5.5' }, { id: 'openai/gpt-6', name: 'GPT-6' }, ...many];
}

const both = (over: Parameters<typeof agents>[0] = {}) =>
  agents({
    opencode: { found: true, models: openCodeModels(), model: 'anthropic/claude-haiku-5-5', ...over.opencode },
    codex: { found: true, signedIn: true, ...over.codex },
  });

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
const where = () => $$('.btray [data-seg="where"] button');
const pressed = () => where().find((b) => b.getAttribute('aria-pressed') === 'true')?.dataset.v;
const agentRows = () => $$('.btray [data-agent]');
const agentRow = (id: string) => $<HTMLElement>(`.btray [data-agent="${id}"]`)!;
const row = (id: string) => $<HTMLElement>(`.btray [data-mid="${id}"]`)!;
const small = (e: Element | null) => text(e?.querySelector('.stx small') ?? null);
const foot = () => text($('.btray .tft'));
const heads = () => $$('.btray .mlist .mgh').map((g) => text(g));
const listed = () => $$('.btray .mlist [data-mid] .stx b').map((b) => text(b));
const menuItems = () => $$('.menu [data-i]').filter((b) => !b.hidden);

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The Cleanup page: No AI, This computer or Your AI on top', () => {
  it('three_part_control_switches_lists', async () => {
    core.agentsGet.mockResolvedValue(both());
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await cleanup();
    expect(where().map((b) => b.textContent)).toEqual(['No AI', 'This computer', 'Your AI']);
    expect(pressed()).toBe('agent');
    expect(agentRows()).toHaveLength(2);
    expect($('.btray .mlist')).toBeNull();

    click(where()[1]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineWhere', value: 'local' });
    expect(pressed()).toBe('local');
    expect(agentRows()).toHaveLength(0);
    expect(listed()).toEqual(['Qwen3 4B Instruct', 'Gemma 4 E2B', 'Qwen3 1.7B']);
    expect($('.btray .msr')).not.toBeNull();

    click(where()[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineWhere', value: 'off' });
    expect(pressed()).toBe('off');
    expect($('.btray .mlist')).toBeNull();
    const only = $$('.btray .sgrp .srow');
    expect(only).toHaveLength(1);
    expect(text(only[0].querySelector('.stx b'))).toBe('What CUE does without AI');
    expect(small(only[0])).toBe('Words from the dictionary and your corrections, no “um” or “uh”, tidy spaces. Instant.');

    click(where()[2]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineWhere', value: 'agent' });
    expect(agentRows()).toHaveLength(2);
  });

  it('agents_in_order_opencode_first_with_recommended', async () => {
    stub(view({ refineAgent: 'opencode' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    expect(core.agentsGet).toHaveBeenCalled();
    expect(text($('.btray .sgrp .mgh'))).toBe('Found on this computer');
    expect(agentRows().map((r) => r.dataset.agent)).toEqual(['opencode', 'codex']);
    expect(agentRows().map((r) => text(r.querySelector('.stx b')))).toEqual(['OpenCode', 'Codex']);
    expect(small(agentRow('opencode'))).toBe('Recommended · Any provider: your key or subscription · ~2 s');
    expect(agentRow('opencode').querySelector('.rc')).not.toBeNull();
    expect(small(agentRow('codex'))).toBe('Signs in with ChatGPT · ~4 s');
    expect(agentRow('opencode').querySelector('.mok')).not.toBeNull();
    expect(agentRow('opencode').getAttribute('aria-current')).toBe('true');
    expect(agentRow('codex').querySelector('.mok')).toBeNull();
    expect($('.btray [data-agent] .btn2')).toBeNull();
    expect(foot()).toBe('Text goes wherever OpenCode is set up to send it: your provider, your key or subscription. CUE holds no keys.');

    click(agentRow('codex'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineAgent', value: 'codex' });
    expect(agentRow('codex').querySelector('.mok')).not.toBeNull();
    expect(foot()).toBe('Text goes to ChatGPT through Codex, under your sign-in. CUE holds no keys.');
  });

  it('while the agents are looked at, the rows say so quietly and the page works', async () => {
    stub(view({ refineAgent: 'codex' }));
    let answer: (v: AgentView[]) => void = () => {};
    core.agentsGet.mockReturnValue(new Promise((r) => (answer = r)));
    page = await mount();
    await cleanup();
    expect(agentRows().map((r) => small(r))).toEqual(['Checking…', 'Checking…']);
    expect($('.btray [data-dd="amodel"]')).toBeNull();
    expect($('.btray [data-dd="fallback"]')).not.toBeNull();
    answer(both());
    await settle();
    expect(small(agentRow('codex'))).toBe('Signs in with ChatGPT · ~4 s');
  });

  it('missing_opencode_shows_install_button', async () => {
    core.agentsGet.mockResolvedValue(agents());
    page = await mount();
    await cleanup();
    const oc = agentRow('opencode');
    expect(oc.classList.contains('off')).toBe(true);
    expect(small(oc)).toBe('Recommended · Not installed: install it and CUE finds it by itself');
    expect(text(oc.querySelector('.btn2'))).toBe('Install');
    expect(small(agentRow('codex'))).toBe('Not installed');
    expect(text(agentRow('codex').querySelector('.btn2'))).toBe('How to install');
    click(oc.querySelector('.btn2'));
    expect(core.agentInstallLink).toHaveBeenCalledWith('opencode');
    click(agentRow('codex').querySelector('.btn2'));
    expect(core.agentInstallLink).toHaveBeenCalledWith('codex');
    click(oc);
    await settle();
    expect(core.settingsSet).not.toHaveBeenCalled();
    expect($('.btray [data-dd="amodel"]')).toBeNull();
    expect($('.btray [data-dd="fallback"]')).toBeNull();
    expect(foot()).toBe('Until an AI is chosen, text goes in without AI.');
    // Found OpenCode: no button.
    core.agentsGet.mockResolvedValue(both());
    click($('.btray [data-tback]'));
    click($('.btray [data-sub="cleanup"]'));
    await settle();
    expect($('.btray [data-agent="opencode"] .btn2')).toBeNull();
  });

  it('signed_out_value_is_amber_with_sign_in_again', async () => {
    stub(view({ refineAgent: 'codex' }));
    core.agentsGet.mockResolvedValue(both({ codex: { signedIn: false } }));
    page = await mount();
    await open();
    expect(text(value())).toBe('Codex: sign in again');
    expect(value().classList.contains('warn')).toBe(true);
    click($('.btray [data-sub="cleanup"]'));
    await settle();
    const line = agentRow('codex').querySelector('.stx small')!;
    expect(text(line)).toBe('Sign in again: codex login in a terminal');
    expect(line.classList.contains('warn')).toBe(true);
    // The chosen agent is not here: its name in amber, "not found".
    core.agentsGet.mockResolvedValue(agents({ codex: { found: false } }));
    click($('.btray [data-tback]'));
    await settle();
    page.tray.close();
    await open();
    expect(text(value())).toBe('Codex not found');
    expect(value().classList.contains('warn')).toBe(true);
  });

  it('model_dropdown_lists_agent_models_and_remembers_per_agent', async () => {
    stub(view({ refineAgent: 'opencode' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    const dd = () => $<HTMLElement>('.btray [data-dd="amodel"]')!;
    const ddRow = () => dd().closest('.srow')!;
    expect(text(ddRow().querySelector('.stx b'))).toBe('Model');
    expect(small(ddRow())).toBe('The fast one of those in OpenCode');
    expect(text(dd().querySelector('.ddv'))).toBe('Fast · auto');
    click(dd());
    expect(menuItems()).toHaveLength(391);
    expect(text(menuItems()[0].querySelector('.mlb'))).toBe('Fast · auto');
    expect(text(menuItems()[0].querySelector('.msub'))).toBe('Claude Haiku 5.5');
    expect(menuItems()[0].getAttribute('aria-checked')).toBe('true');
    // The list is long: a field over it filters as the user types.
    const filter = $<HTMLInputElement>('.menu .mfl input')!;
    expect(filter.placeholder).toBe('Find a model');
    expect(document.activeElement).toBe(filter);
    type(filter, 'gpt');
    expect(menuItems().map((b) => text(b.querySelector('.mlb')))).toEqual(['GPT-6']);
    type(filter, 'zzz');
    expect(menuItems()).toHaveLength(0);
    expect(text($('.menu .mfnone'))).toBe('No such model');
    type(filter, 'GPT-6');
    click(menuItems()[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineAgentModel', value: { agent: 'opencode', model: 'openai/gpt-6' } });
    expect(text(dd().querySelector('.ddv'))).toBe('GPT-6');
    expect(small(ddRow())).toBe('');
    // Codex keeps its own: its default, not OpenCode's pick.
    click(agentRow('codex'));
    await settle();
    expect(text(dd().querySelector('.ddv'))).toBe('GPT-6 Luna');
    click(dd());
    expect(menuItems().map((b) => text(b.querySelector('.mlb')))).toEqual(['GPT-6 Luna', 'GPT-6 Sol', 'GPT-6 Astra', 'GPT-6.1 Sol', 'GPT-6.1 Terra', 'GPT-5.6 Luna']);
    expect($('.menu .mfl')).toBeNull();
    click(menuItems()[4]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineAgentModel', value: { agent: 'codex', model: 'gpt-6.1-terra' } });
    click(agentRow('opencode'));
    await settle();
    expect(text(dd().querySelector('.ddv'))).toBe('GPT-6');
    // "Fast · auto" goes back to OpenCode's own pick.
    click(dd());
    click(menuItems()[0]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineAgentModel', value: { agent: 'opencode', model: '' } });
  });

  it('Enter in the filter takes the first model left; Esc closes the list', async () => {
    stub(view({ refineAgent: 'opencode' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    // Esc in the field closes the list, and only the list: the tray stays on its page.
    click($('.btray [data-dd="amodel"]'));
    $<HTMLInputElement>('.menu .mfl input')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect($('.menu')).toBeNull();
    expect(core.settingsSet).not.toHaveBeenCalled();
    expect($('.btray [data-seg="where"]')).not.toBeNull();
    click($('.btray [data-dd="amodel"]'));
    const filter = $<HTMLInputElement>('.menu .mfl input')!;
    type(filter, 'haiku');
    filter.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineAgentModel', value: { agent: 'opencode', model: 'anthropic/claude-haiku-5-5' } });
    expect($('.menu')).toBeNull();
  });

  it('a move the core refuses puts the thumb back on the part the settings hold', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    page = await mount();
    await cleanup();
    core.settingsSet.mockRejectedValueOnce('badValue');
    click(where()[1]);
    expect(pressed()).toBe('local');
    await settle();
    expect(pressed()).toBe('agent');
    expect($('.btray .mlist')).toBeNull();
    errors.mockRestore();
  });

  it('the time in an agent line keeps to the word before it', async () => {
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    expect(agentRow('opencode').querySelector('.stx small')!.textContent).toContain('subscription · ~2 s');
    expect(agentRow('codex').querySelector('.stx small')!.textContent).toBe('Signs in with ChatGPT · ~4 s');
  });

  it('fallback_offers_local_only_when_downloaded', async () => {
    // A model chosen on this computer but not downloaded: nothing to clean with yet.
    stub(view({ refineAgent: 'codex', refineLocal: QWEN }));
    core.agentsGet.mockResolvedValue(both());
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await cleanup();
    const fb = () => $<HTMLElement>('.btray [data-dd="fallback"]')!;
    const fbRow = () => fb().closest('.srow')!;
    expect(text(fbRow().querySelector('.stx b'))).toBe("If the AI didn't answer");
    expect(small(fbRow())).toBe('Download a model to clean with it');
    expect(text(fb().querySelector('.ddv'))).toBe('Paste as said');
    click(fb());
    expect(menuItems().map((b) => text(b.querySelector('.mlb')))).toEqual(['Paste as said']);
    click(fb());
    // Its file came: it is offered by name.
    feeds.llms(llms({ [QWEN]: { state: 'downloaded' } }));
    await settle();
    expect(small(fbRow())).toBe('Or clean with the model on this computer');
    click(fb());
    expect(menuItems().map((b) => text(b.querySelector('.mlb')))).toEqual(['Paste as said', 'Clean: Qwen3 4B Instruct']);
    click(menuItems()[1]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'refineFallback', value: 'local' });
    expect(text(fb().querySelector('.ddv'))).toBe('Clean: Qwen3 4B Instruct');
  });

  it('local_groups_and_recommended_tag', async () => {
    stub(view({ refineWhere: 'local', refineLocal: QWEN }));
    core.llmsGet.mockResolvedValue(llms({ [QWEN]: { state: 'inUse' }, [TLITE]: { state: 'downloaded' } }, QWEN));
    page = await mount();
    await cleanup();
    expect(heads()).toEqual(['For your languages · English', 'Others']);
    expect($('.btray .mlist .mgh .mut')).not.toBeNull();
    expect(listed()).toEqual(['Qwen3 4B Instruct', 'Gemma 4 E2B', 'Qwen3 1.7B', 'T-lite-it-2.1']);
    // Recommended first, in its group, with its size; the others with their size on the right.
    expect(small(row(QWEN))).toBe('Recommended · 2.5 GB');
    expect(row(QWEN).querySelector('.rc')).not.toBeNull();
    expect(row(QWEN).querySelector('.mok')).not.toBeNull();
    expect(row(QWEN).querySelector('.mdel')).not.toBeNull();
    expect(text(row(GEMMA).querySelector('.msz'))).toBe('3.1 GB');
    expect(row(GEMMA).querySelector('.stx small')?.hasAttribute('hidden')).toBe(true);
    // The search is the last row of "Others".
    const list = $('.btray .mlist')!;
    expect(list.nextElementSibling?.classList.contains('msr')).toBe(true);
    expect(foot()).toBe('Free. Text never leaves this computer; the model runs on its processor.');
  });

  it('Russian: the languages in the heading are the interface and the translation', async () => {
    stub(view({ refineWhere: 'local', lang: 'ru', uiLanguage: 'ru', translateTarget: 'en' }));
    core.llmsGet.mockResolvedValue(llms({ [QWEN]: { state: 'downloaded' } }));
    page = await mount();
    await cleanup();
    expect(heads()).toEqual(['Для ваших языков · русский, английский', 'Другие']);
    expect(small(row(QWEN))).toBe('Рекомендуем · 2,5 ГБ');
    expect(where().map((b) => b.textContent)).toEqual(['Без ИИ', 'Этот компьютер', 'Ваш AI']);
  });

  it('pressing_missing_model_downloads_with_percent_and_stop', async () => {
    stub(view({ refineWhere: 'local' }));
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await cleanup();
    click(row(QWEN));
    expect(core.llmChoose).toHaveBeenCalledWith({ id: QWEN });
    // A double-click is one press: it never starts and stops the download at once.
    await vi.advanceTimersByTimeAsync(150);
    click(row(QWEN));
    expect(core.llmChoose).toHaveBeenCalledTimes(1);
    feeds.llms(llms({ [QWEN]: { state: 'downloading', done: 42 * MB, total: 100 * MB } }));
    await settle();
    expect(text(row(QWEN).querySelector('.mpct'))).toBe('42 %');
    expect(row(QWEN).querySelector('.mbar')).not.toBeNull();
    expect(text(row(QWEN).querySelector('.mprog small'))).toBe('Downloading: 42 of 100 MB · press to stop');
    const bar = row(QWEN).querySelector('.mbar');
    feeds.llms(llms({ [QWEN]: { state: 'downloading', done: 61 * MB, total: 100 * MB } }));
    expect(row(QWEN).querySelector('.mbar')).toBe(bar);
    expect(text(row(QWEN).querySelector('.mpct'))).toBe('61 %');
    // Pressed again later: the core stops it.
    await vi.advanceTimersByTimeAsync(500);
    click(row(QWEN));
    expect(core.llmChoose).toHaveBeenCalledTimes(2);
    expect(core.llmChoose).toHaveBeenLastCalledWith({ id: QWEN });
    // The first page: the percent in Bebas, the line under the row.
    click($('.btray [data-tback]'));
    await settle();
    expect(text(value())).toBe('61 %');
    expect(value().classList.contains('num')).toBe(true);
    expect($('.btray [data-sub="cleanup"] .tline')).not.toBeNull();
    feeds.llms(llms({ [QWEN]: { state: 'inUse' } }, QWEN));
    expect(text(value())).toBe('Qwen3 4B Instruct');
    expect($('.btray [data-sub="cleanup"] .tline')).toBeNull();
  });

  it('found_model_marked_not_checked', async () => {
    stub(view({ refineWhere: 'local' }));
    core.llmsGet.mockResolvedValue(llms({ [TLITE]: { state: 'downloaded' } }));
    const answer: LlmFound[] = [
      { repo: 'ai-sage/GigaChat3.1-10B-A1.8B-GGUF', downloads: 3000, size: 6.5 * GB, quant: 'Q4_K_M', reason: null },
      { repo: 'big/split-GGUF', downloads: 30, size: null, quant: null, reason: 'severalFiles' },
    ];
    core.llmsSearch.mockResolvedValue(answer);
    page = await mount();
    await cleanup();
    expect(small(row(TLITE))).toBe('Not checked by CUE · 5.0 GB');
    expect(row(TLITE).querySelector('.ncheck')).not.toBeNull();
    expect(row(GEMMA).querySelector('.ncheck')).toBeNull();
    type($<HTMLInputElement>('.btray .msr input')!, 'giga');
    await vi.advanceTimersByTimeAsync(300);
    await settle();
    const found = $$('.btray .mfound [data-repo]');
    expect(small(found[0])).toBe('3,000 downloads · 6.5 GB · Q4_K_M · Not checked by CUE');
    expect(found[0].querySelector('.ncheck')).not.toBeNull();
    expect(found[1].classList.contains('off')).toBe(true);
    expect(small(found[1])).toBe('several files');
  });

  it('footer_per_route_verbatim', async () => {
    const ru = { lang: 'ru', uiLanguage: 'ru' } as const;
    const notes: [Partial<Parameters<typeof view>[0]>, AgentView[], ModelsView, string][] = [
      [{ refineAgent: 'opencode' }, both(), llms(), 'Текст уходит туда, куда настроен OpenCode: ваш провайдер, ваш ключ или подписка. CUE ключей не хранит.'],
      [{ refineAgent: 'codex' }, both(), llms(), 'Текст уходит в ChatGPT через Codex, по вашему входу. CUE не хранит ключи.'],
      [{ refineAgent: null }, agents(), llms(), 'Пока AI не выбран, текст вставляется без ИИ.'],
      [{ refineWhere: 'local', refineLocal: QWEN }, both(), llms({ [QWEN]: { state: 'inUse' } }, QWEN), 'Бесплатно. Текст не покидает этот компьютер, модель работает на процессоре.'],
      [{ refineWhere: 'local' }, both(), llms(), 'Пока модель не выбрана, текст вставляется без ИИ.'],
      [{ refineWhere: 'off' }, both(), llms(), 'Текст вставляется как распознан. CUE сам правит слова из словаря и убирает «эм» и «ээ».'],
    ];
    for (const [over, a, l, note] of notes) {
      stub(view({ ...ru, ...over }));
      core.agentsGet.mockResolvedValue(a);
      core.llmsGet.mockResolvedValue(l);
      page?.dispose();
      page = await mount();
      await cleanup();
      expect(foot(), JSON.stringify(over)).toBe(note);
    }
  });

  it('value_shows_agent_and_model', async () => {
    stub(view({ refineAgent: 'opencode' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await open();
    expect(text(value())).toBe('OpenCode · Fast · auto');
    expect(value().classList.contains('warn')).toBe(false);
    feeds.settings(view({ refineAgent: 'opencode', refineAgentModel: { opencode: 'openai/gpt-6' } }));
    page.tray.settingsChanged();
    expect(text(value())).toBe('OpenCode · GPT-6');
    feeds.settings(view({ refineAgent: 'codex' }));
    page.tray.settingsChanged();
    expect(text(value())).toBe('Codex · GPT-6 Luna');
    feeds.settings(view({ refineWhere: 'off' }));
    page.tray.settingsChanged();
    expect(text(value())).toBe('No AI');
    feeds.settings(view({ refineWhere: 'local', refineLocal: QWEN }));
    feeds.llms(llms({ [QWEN]: { state: 'inUse' } }, QWEN));
    page.tray.settingsChanged();
    expect(text(value())).toBe('Qwen3 4B Instruct');
  });

  it('value_nothing_chosen', async () => {
    page = await mount();
    await open();
    expect(text(value())).toBe('Not chosen');
    expect(value().classList.contains('warn')).toBe(false);
    feeds.settings(view({ refineWhere: 'local' }));
    feeds.llms(llms());
    page.tray.settingsChanged();
    expect(text(value())).toBe('Not chosen');
    // A model chosen that is no longer here: in amber.
    feeds.settings(view({ refineWhere: 'local', refineLocal: QWEN }));
    page.tray.settingsChanged();
    expect(text(value())).toBe('Model not downloaded');
    expect(value().classList.contains('warn')).toBe(true);
  });

  it('a switch that failed never hides the model still cleaning; one on its way shows its step', async () => {
    stub(view({ refineWhere: 'local', refineLocal: GEMMA }));
    page = await mount();
    await open();
    for (const stuck of [{ state: 'failed', done: 1, total: 2 }, { state: 'noSpace', needed: GB, free: 0 }, { state: 'notRunnable' }]) {
      feeds.llms(llms({ [GEMMA]: { state: 'inUse' }, [QWEN]: stuck }, GEMMA));
      expect(text(value()), stuck.state).toBe('Gemma 4 E2B');
      expect(value().classList.contains('warn')).toBe(false);
    }
    feeds.llms(llms({ [GEMMA]: { state: 'inUse' }, [QWEN]: { state: 'downloading', done: 30, total: 100 } }, GEMMA));
    expect(text(value())).toBe('30 %');
    // With no model cleaning, the failure shows.
    feeds.llms(llms({ [QWEN]: { state: 'failed', done: 1, total: 2 } }));
    expect(text(value())).toBe('Did not download');
    expect(value().classList.contains('warn')).toBe(true);
  });

  it('a cleanup moved to Your AI with none chosen follows the agent the core settles on', async () => {
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    expect(foot()).toBe('Until an AI is chosen, text goes in without AI.');
    feeds.settings(view({ refineAgent: 'opencode' }));
    await settle();
    expect(agentRow('opencode').querySelector('.mok')).not.toBeNull();
    expect(foot()).toBe('Text goes wherever OpenCode is set up to send it: your provider, your key or subscription. CUE holds no keys.');
  });
});

describe('The Cleanup page: Your instructions', () => {
  const insRow = () => $<HTMLElement>('.btray [data-sub="instructions"]');

  it('instructions_row_opens_the_page_and_hides_on_no_ai', async () => {
    core.agentsGet.mockResolvedValue(both());
    stub(view({ refineWhere: 'agent', refineAgent: 'codex' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await cleanup();
    // Before the footer, with its line from the core.
    const row = insRow()!;
    expect(text(row.querySelector('.stx b'))).toBe('Your instructions');
    expect(small(row)).toBe('Standard · you can adjust them with AI');
    expect(row.nextElementSibling?.classList.contains('tft') || row.parentElement?.nextElementSibling?.classList.contains('tft')).toBe(true);
    // The user's own: said so.
    core.instructionsGet.mockResolvedValue({ text: 'Keep it short.', edited: true, broken: false, path: 'x', open: true });
    click($('.btray [data-tback]'));
    await settle();
    click($('.btray [data-sub="cleanup"]'));
    await settle();
    expect(small(insRow())).toBe('Changed by you');
    // A broken file: amber.
    core.instructionsGet.mockResolvedValue({ text: 'x', edited: false, broken: true, path: 'x', open: true });
    click($('.btray [data-tback]'));
    await settle();
    click($('.btray [data-sub="cleanup"]'));
    await settle();
    expect(insRow()!.querySelector('.stx small')!.classList.contains('warn')).toBe(true);
    // It opens the Instructions page, and back goes to Cleanup.
    click(insRow());
    await settle();
    expect(text($('.btray .tph .lbl'))).toBe('Your instructions');
    expect($('.btray textarea.pta')).not.toBeNull();
    click($('.btray [data-tback]'));
    await settle();
    expect(text($('.btray .tph .lbl'))).toBe('Cleanup');
    // No AI: no instructions are used, so no row.
    click(where().find((b) => b.dataset.v === 'off')!);
    await settle();
    expect(insRow()).toBeNull();
    // This computer: the row is there.
    click(where().find((b) => b.dataset.v === 'local')!);
    await settle();
    expect(insRow()).not.toBeNull();
  });
});
