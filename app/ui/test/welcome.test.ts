import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, agents, click, core, feeds, settle, stub, text, view } from './windowFixture';
import type { ModelsView } from '../src/window/api';
import { mountWelcome, TYPED_TO_DONE_MS, type WelcomePage } from '../src/welcome/page';
import { STEPS, wordCount, type Step } from '../src/welcome/steps';
import { doneTitle, en, ru } from '../src/welcome/text';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WelcomePage | null = null;

/** The welcome page, built; motion reduced unless asked otherwise, so nothing slides. */
async function mount(reduced = true): Promise<WelcomePage> {
  document.body.innerHTML = '<div id="w"></div>';
  const p = await mountWelcome(document.getElementById('w')!, { reduced: () => reduced });
  await settle();
  return p;
}

/** The page of the step shown (with motion, pages on their way out stay in the window). */
const pg = () => $$('.pgv').filter((p) => !p.inert).at(-1)!;
const step = () => pg().dataset.step;
const heading = () => text(pg().querySelector('.ohd'));
const primary = () => $<HTMLButtonElement>('[data-pri]')!;
const back = () => $<HTMLButtonElement>('[data-back]')!;
const next = async () => {
  click(primary());
  await settle();
};
/** Next until `s` is the step shown. */
const reach = async (s: Step) => {
  while (step() !== s) await next();
};
const steps = () => core.welcomeStep.mock.calls.map(([dictation]) => dictation);
const previews = () => core.micPreview.mock.calls.map(([on, device]) => (on ? `on:${device ?? 'windows'}` : 'off'));
const field = () => pg().querySelector<HTMLTextAreaElement>('textarea[data-field]')!;
const dictate = (value: string) => {
  field().value = value;
  field().dispatchEvent(new Event('input', { bubbles: true }));
};
/** A mouse press on `e`; true — the page cancelled it, so the focus stays where it is. */
const press = (e: Element) => !e.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
/** The window has the focus again. */
const regain = () => window.dispatchEvent(new Event('focus'));
/** Megabytes as the model's row writes them (decimal), and the model's size (668 MB). */
const MB = 1_000_000;
const TOTAL = 667_729_022;
/** The model's row on the Model step. */
const mrow = () => pg().querySelector<HTMLElement>('.mrow')!;

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub();
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('The welcome window', () => {
  it('goes through its steps, and Back goes back', async () => {
    page = await mount();
    const dots = () => $$('.dots i').map((d) => d.className);
    expect($$('.dots i').map((d) => d.title)).toEqual(['Hello', 'Who cleans the text', 'Model', 'Microphone', 'First dictation', 'Done']);
    expect(step()).toBe('hello');
    expect(heading()).toBe('Speak — CUE writes');
    expect(dots()).toEqual(['on', '', '', '', '', '']);
    expect(back().hidden).toBe(true);
    expect(text(primary())).toBe('Start');
    await next();
    expect(step()).toBe('cleanup');
    expect(dots()).toEqual(['ok', 'on', '', '', '', '']);
    expect(back().hidden).toBe(false);
    click(back());
    await settle();
    expect(step()).toBe('hello');
    expect(back().hidden).toBe(true);
    await next();
    expect(step()).toBe('cleanup');
    expect(text(primary())).toBe('Next');
    await next();
    expect(step()).toBe('model');
    expect(heading()).toBe('Model');
    expect(text(pg().querySelector('.op'))).toBe('CUE recognizes speech on this computer. The model downloads once — go on meanwhile.');
    expect(text(mrow().querySelector('.stx b'))).toBe('Parakeet Ultra');
    expect(text(mrow().querySelector('.stx small.ok'))).toBe('Ready · 668 MB on this computer');
    expect(dots()).toEqual(['ok', 'ok', 'on', '', '', '']);
    expect(text(primary())).toBe('Next');
    expect(primary().disabled).toBe(false);
    await next();
    expect(step()).toBe('mic');
    expect(heading()).toBe('Microphone');
    expect(text(primary())).toBe('It works');
    await next();
    expect(step()).toBe('try');
    expect(heading()).toBe('First dictation');
    // No main button to press here: it keeps its place and its label, unseen, so Back does not move.
    expect(primary().classList.contains('wait')).toBe(true);
    expect(primary().disabled).toBe(true);
    expect(text(primary())).toBe('It works');
    expect(back().hidden).toBe(false);
    // Only one page at a time with reduced motion.
    expect($$('.pgv')).toHaveLength(1);
    click(back());
    await settle();
    expect(step()).toBe('mic');
    expect(dots()).toEqual(['ok', 'ok', 'ok', 'on', '', '']);
    expect(primary().classList.contains('wait')).toBe(false);
    expect(primary().disabled).toBe(false);
  });

  it('asks for no key, wherever the cleanup runs', async () => {
    for (const refineWhere of ['agent', 'local', 'off'] as const) {
      stub(view({ refineWhere }));
      page?.dispose();
      page = await mount();
      expect($$('.dots i').map((d) => d.title)).toEqual(['Hello', 'Who cleans the text', 'Model', 'Microphone', 'First dictation', 'Done']);
      for (let i = 0; i < 4; i++) {
        expect($$('input[data-key], [data-check], [data-link="anthropic"]')).toHaveLength(0);
        await next();
      }
      expect(step()).toBe('try');
    }
  });

  it('First dictation waits in its field, not on a button', async () => {
    page = await mount(false);
    await reach('try');
    expect(step()).toBe('try');
    // The field says it waits, and its border breathes.
    expect(field().placeholder).toBe('Waiting — press the keys');
    expect(field().classList.contains('wait')).toBe(true);
    expect(primary().classList.contains('wait')).toBe(true);
    // The dictation came: the field stops breathing, and Done follows as before, with its button.
    dictate('Hello, this is my first dictation.');
    expect(field().classList.contains('wait')).toBe(false);
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(step()).toBe('done');
    expect(primary().classList.contains('wait')).toBe(false);
    expect(primary().disabled).toBe(false);
    expect(text(primary())).toBe('Done');
    // With reduced motion the field stands still.
    page.dispose();
    page = await mount(true);
    await reach('try');
    expect(field().placeholder).toBe('Waiting — press the keys');
    expect(field().classList.contains('wait')).toBe(false);
  });

  it('holds the hotkey on load', async () => {
    page = await mount();
    expect(steps()).toEqual([false]);
    // Before anything else: a reload starts from a held key and a closed microphone.
    expect(core.welcomeStep.mock.invocationCallOrder[0]).toBeLessThan(core.settingsGet.mock.invocationCallOrder[0]);
    expect(previews()).toEqual(['off']);
  });

  it('the Model step shows the download as it goes, in place: the percent, a bar and the bytes', async () => {
    core.modelGet.mockResolvedValue({ state: 'downloading', done: 100 * MB, total: TOTAL });
    page = await mount();
    await reach('model');
    expect(step()).toBe('model');
    expect(pg().querySelector('.sgrp.orows > .mrow')).not.toBeNull();
    const row = mrow();
    expect(row.dataset.model).toBe('downloading');
    expect(text(row.querySelector('.stx b'))).toBe('Parakeet Ultra');
    expect(text(row.querySelector('.mpct'))).toBe('14 %');
    expect(text(row.querySelector('.mprog small'))).toBe('Downloading: 100 of 668 MB');
    feeds.model({ state: 'downloading', done: 412 * MB + 5, total: TOTAL });
    // The same row, changed where it is.
    expect(mrow()).toBe(row);
    expect(text(row.querySelector('.mpct'))).toBe('61 %');
    expect(row.querySelector<HTMLElement>('.mbar i')!.style.transform).toMatch(/^scaleX\(0\.61\d*\)$/);
    expect(text(row.querySelector('.mdone'))).toBe('412');
    expect(text(row.querySelector('.mprog small'))).toBe('Downloading: 412 of 668 MB');
    expect(row.querySelector('[data-model-retry]')).toBeNull();
    // The step does not wait for the download: Next goes on meanwhile.
    expect(text(primary())).toBe('Next');
    expect(primary().disabled).toBe(false);
    await next();
    expect(step()).toBe('mic');
  });

  it('Retry on a failed download asks the core to download the model again', async () => {
    page = await mount();
    await reach('model');
    feeds.model({ state: 'failed', done: 200 * MB, total: TOTAL });
    const row = mrow();
    expect(row.dataset.model).toBe('failed');
    const small = row.querySelector('.stx small')!;
    expect(small.classList.contains('warn')).toBe(true);
    expect(text(small)).toBe('Did not download · 200 of 668 MB');
    const retry = row.querySelector<HTMLButtonElement>('[data-model-retry]')!;
    expect(text(retry.querySelector('.lbl'))).toBe('Retry');
    expect(core.modelRetry).not.toHaveBeenCalled();
    click(retry);
    await settle();
    expect(core.modelRetry).toHaveBeenCalledTimes(1);
    // Retry stays on the step, and Next is still there to press.
    expect(step()).toBe('model');
    expect(primary().disabled).toBe(false);
  });

  it('the model ready while the Model step is shown moves on to Microphone by itself', async () => {
    core.modelGet.mockResolvedValue({ state: 'downloading', done: 10 * MB, total: TOTAL });
    page = await mount();
    expect(core.onModel).toHaveBeenCalledTimes(1);
    expect(core.modelGet).toHaveBeenCalledTimes(1);
    await reach('model');
    expect(step()).toBe('model');
    feeds.model({ state: 'loading' });
    await settle();
    expect(step()).toBe('model');
    expect(mrow().dataset.model).toBe('loading');
    expect(text(mrow().querySelector('.stx small'))).toBe('Loading');
    feeds.model({ state: 'ready', total: TOTAL });
    await settle();
    expect(step()).toBe('mic');
    expect(heading()).toBe('Microphone');
    // Back to the Model step: it shows the model ready and stays.
    click(back());
    await settle();
    expect(step()).toBe('model');
    expect(text(mrow().querySelector('.stx small.ok'))).toBe('Ready · 668 MB on this computer');
    feeds.model({ state: 'ready', total: TOTAL });
    await settle();
    expect(step()).toBe('model');
  });

  it('the model ready on another step moves nothing', async () => {
    core.modelGet.mockResolvedValue({ state: 'downloading', done: 10 * MB, total: TOTAL });
    page = await mount();
    expect(step()).toBe('hello');
    feeds.model({ state: 'ready', total: TOTAL });
    await settle();
    expect(step()).toBe('hello');
    // Its step then shows it ready, and waits for Next.
    await reach('model');
    expect(step()).toBe('model');
    expect(mrow().dataset.model).toBe('ready');
    await next();
    expect(step()).toBe('mic');
  });

  it('the microphone preview runs only on its step', async () => {
    page = await mount();
    core.micPreview.mockClear();
    await reach('mic');
    expect(previews()).toEqual(['on:windows']);
    const lit = () => $$('[data-meter] i.on').length;
    feeds.level(0.5);
    expect(lit()).toBe(8);
    await next();
    expect(previews()).toEqual(['on:windows', 'off']);
    // Levels off the step light nothing.
    feeds.level(1);
    click(back());
    await settle();
    expect(lit()).toBe(0);
    expect(previews()).toEqual(['on:windows', 'off', 'on:windows']);
    click(back());
    await settle();
    expect(previews()).toEqual(['on:windows', 'off', 'on:windows', 'off']);
    await next();
    window.dispatchEvent(new Event('pagehide'));
    expect(previews()).toEqual(['on:windows', 'off', 'on:windows', 'off', 'on:windows', 'off']);
  });

  it('the preview follows a new microphone', async () => {
    core.micList.mockResolvedValue({ names: ['Laptop Microphone', 'USB Microphone'], default: 'Laptop Microphone' });
    page = await mount();
    await reach('mic');
    click($('[data-dd="mic"]'));
    expect($$('.menu [data-i] .mlb').map((e) => e.textContent)).toEqual([
      'As in Windows (Laptop Microphone)',
      'Laptop Microphone',
      'USB Microphone',
    ]);
    click($$('.menu [data-i]')[2]);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'micDevice', value: 'USB Microphone' });
    expect(previews().at(-1)).toBe('on:USB Microphone');
    expect(text($('[data-dd="mic"] .ddv'))).toBe('USB Microphone');
    // Under the Microphone heading the row has no label of its own; the drop-down keeps it as its name.
    expect(heading()).toBe('Microphone');
    const row = $('[data-dd="mic"]')!.closest('.srow')!;
    expect(row.querySelector('b')).toBeNull();
    expect($('[data-dd="mic"]')!.getAttribute('aria-label')).toBe('Microphone');
  });

  it('a denied, failed or missing microphone shows the hint with the Windows settings', async () => {
    core.micAccess.mockResolvedValue('denied');
    page = await mount();
    await reach('mic');
    const hint = () => pg().querySelector<HTMLElement>('.ohint')!;
    expect(hint().hidden).toBe(false);
    expect(text(hint().querySelector('span'))).toBe('Windows is blocking CUE from using the microphone');
    expect(text(primary())).toBe('Next');
    click(hint().querySelector('[data-link="micPrivacy"]'));
    expect(core.openLink).toHaveBeenCalledWith('micPrivacy');
    // Allowed, but the preview fails.
    core.micAccess.mockResolvedValue('allowed');
    click(back());
    await settle();
    await next();
    expect(hint().hidden).toBe(true);
    expect(text(primary())).toBe('It works');
    feeds.failed();
    expect(hint().hidden).toBe(false);
    // No microphone at all.
    core.micList.mockResolvedValue({ names: [], default: null });
    click(back());
    await settle();
    await next();
    expect(hint().hidden).toBe(false);
    // The step can still be passed.
    await next();
    expect(step()).toBe('try');
  });

  it('looks at the microphone again when the window has the focus back', async () => {
    core.micAccess.mockResolvedValue('denied');
    page = await mount();
    await reach('mic');
    const hint = () => pg().querySelector<HTMLElement>('.ohint')!;
    feeds.failed();
    expect(hint().hidden).toBe(false);
    expect(text(primary())).toBe('Next');
    // Allowed in the Windows settings meanwhile, and a microphone plugged in.
    core.micAccess.mockResolvedValue('allowed');
    core.micList.mockResolvedValue({ names: ['Laptop Microphone', 'USB Microphone'], default: 'Laptop Microphone' });
    core.micPreview.mockClear();
    core.micAccess.mockClear();
    click($('[data-dd="mic"]'));
    expect($('.menu')).not.toBeNull();
    regain();
    await settle();
    expect(previews()).toEqual(['on:windows']);
    expect(core.micAccess).toHaveBeenCalledTimes(1);
    expect(hint().hidden).toBe(true);
    expect(text(primary())).toBe('It works');
    // The rows are drawn again, so a menu over the old ones is closed.
    expect($('.menu')).toBeNull();
    click($('[data-dd="mic"]'));
    expect($$('.menu [data-i] .mlb').map((e) => e.textContent)).toContain('USB Microphone');
    // The listeners of the load still serve: no new ones, a level lights the meter, a new failure shows the hint.
    expect(core.onMicLevel).toHaveBeenCalledTimes(1);
    expect(core.onMicFailed).toHaveBeenCalledTimes(1);
    feeds.level(0.5);
    expect($$('[data-meter] i.on')).toHaveLength(8);
    feeds.failed();
    expect(hint().hidden).toBe(false);
    // On another step the window's focus opens nothing.
    await next();
    expect(step()).toBe('try');
    core.micPreview.mockClear();
    core.micAccess.mockClear();
    regain();
    await settle();
    expect(core.micPreview).not.toHaveBeenCalled();
    expect(core.micAccess).not.toHaveBeenCalled();
  });

  it('First dictation keeps its field in focus, so the dictation is pasted into it', async () => {
    page = await mount();
    await reach('mic');
    // Before the step a press is the browser's as usual.
    expect(press(pg())).toBe(false);
    await next();
    expect(step()).toBe('try');
    expect(document.activeElement).toBe(field());
    // The page, the keys, the bar under the waiting button, that button: the focus stays in the field.
    for (const e of [pg(), pg().querySelector('.keys')!, $('.ofoot')!, primary()]) expect(press(e)).toBe(true);
    // The field itself and Back take the press.
    expect(press(field())).toBe(false);
    expect(press(back())).toBe(false);
    // The window or its page shown again: the field takes the focus back.
    field().blur();
    expect(document.activeElement).toBe(document.body);
    regain();
    expect(document.activeElement).toBe(field());
    field().blur();
    document.dispatchEvent(new Event('visibilitychange'));
    expect(document.activeElement).toBe(field());
    // Back still works, and off the step a press is as usual again.
    click(back());
    await settle();
    expect(step()).toBe('mic');
    expect(press(pg())).toBe(false);
    // Entering the step again focuses the field.
    await next();
    expect(document.activeElement).toBe(field());
  });

  it('lets the hotkey go on First dictation and holds it again when leaving', async () => {
    page = await mount();
    await reach('mic');
    expect(step()).toBe('mic');
    expect(steps()).toEqual([false]);
    await next();
    expect(step()).toBe('try');
    expect(steps()).toEqual([false, true]);
    click(back());
    await settle();
    expect(steps()).toEqual([false, true, false]);
    await next();
    expect(steps()).toEqual([false, true, false, true]);
    dictate('Hello, this is my first dictation.');
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(step()).toBe('done');
    expect(steps()).toEqual([false, true, false, true, false]);
  });

  it('text in the field leads to Done with its words', async () => {
    page = await mount();
    await reach('try');
    expect(document.activeElement).toBe(field());
    const keys = pg().querySelector('.keys')!;
    expect(keys.classList.contains('wait')).toBe(true);
    dictate('Hello, this is my first dictation.');
    expect(keys.classList.contains('down')).toBe(true);
    expect(keys.classList.contains('wait')).toBe(false);
    vi.advanceTimersByTime(TYPED_TO_DONE_MS - 1);
    expect(step()).toBe('try');
    // More text meanwhile: counted, and no second timer.
    dictate('Hello, this is my first dictation. Again.');
    vi.advanceTimersByTime(1);
    await settle();
    expect(step()).toBe('done');
    expect(heading()).toBe('First 7 words!');
    expect(back().hidden).toBe(true);
    expect(text(primary())).toBe('Done');
    expect($$('.otips .otl').map(text)).toEqual([
      'Translation',
      'Ask',
      'CUE lives next to the clock: double-click its icon for the window, right-click for the menu',
    ]);
    expect($$('.otips .keys').map((k) => k.getAttribute('aria-label'))).toEqual(['Ctrl + Win + Shift', 'Ctrl + Win + Alt']);
  });

  it('Back during the wait for Done stays on the steps', async () => {
    page = await mount();
    await reach('try');
    dictate('Hello');
    click(back());
    await settle();
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(step()).toBe('mic');
  });

  it('Done closes the window', async () => {
    page = await mount();
    await reach('try');
    dictate('Hello');
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(core.welcomeClose).not.toHaveBeenCalled();
    await next();
    expect(core.welcomeClose).toHaveBeenCalledTimes(1);
  });

  it('bursts on Done, and not with reduced motion', async () => {
    // jsdom has no Web Animations: a stand-in whose animations never end (pages sliding out stay, marked inert).
    const animate = () => ({ onfinish: null }) as unknown as Animation;
    Object.defineProperty(HTMLElement.prototype, 'animate', { value: animate, configurable: true });
    try {
      for (const reduced of [false, true]) {
        stub();
            page?.dispose();
        page = await mount(reduced);
        expect($$('.pgv')).toHaveLength(1);
        await reach('try');
        dictate('Hello');
        vi.advanceTimersByTime(TYPED_TO_DONE_MS);
        await settle();
        expect(step()).toBe('done');
        // The steps slid with motion, and went at once without it.
        expect($$('.pgv')).toHaveLength(reduced ? 1 : 6);
        expect($$('.bp')).toHaveLength(0);
        vi.advanceTimersByTime(420);
        expect($$('.bp')).toHaveLength(reduced ? 0 : 30);
      }
    } finally {
      delete (HTMLElement.prototype as any).animate;
    }
  });

  it('speaks Russian when the core does', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    expect(document.documentElement.lang).toBe('ru');
    expect(heading()).toBe('Говорите — CUE напишет');
    expect(text(primary())).toBe('Начать');
    await next();
    expect(heading()).toBe('Кто чистит текст');
    expect($$('[data-seg="where"] button').map(text)).toEqual(['Без ИИ', 'Этот компьютер', 'Ваш AI']);
    expect(text(primary())).toBe('Дальше');
    await next();
    expect(heading()).toBe('Модель');
    expect(text(mrow().querySelector('.stx small.ok'))).toBe('Готово · 668 МБ на этом компьютере');
    expect(text(primary())).toBe('Дальше');
    await next();
    expect(text($('[data-dd="mic"] .ddv'))).toBe('Как в Windows');
    expect($('[data-dd="mic"]')!.getAttribute('aria-label')).toBe('Микрофон');
    await next();
    expect(field().placeholder).toBe('Жду диктовку — нажмите клавиши');
    dictate('Привет, это моя первая диктовка.');
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(heading()).toBe('Первые 5 слов!');
    expect(text(primary())).toBe('Готово');
  });

  it('counts words and names them', () => {
    expect(wordCount('Hello, this is my first dictation.')).toBe(6);
    expect(wordCount("  It's a well-known fact — 42 times.\n")).toBe(6);
    expect(wordCount(' … — ')).toBe(0);
    expect(doneTitle(en, 0, 'en')).toBe('All set!');
    expect(doneTitle(en, 1, 'en')).toBe('First word!');
    expect(doneTitle(en, 6, 'en')).toBe('First 6 words!');
    expect(['Первое слово!', 'Первые 2 слова!', 'Первые 5 слов!', 'Первые 21 слово!'])
      .toEqual([1, 2, 5, 21].map((n) => doneTitle(ru, n, 'ru')));
  });
});

/* ---------- the step "Who cleans the text" ---------- */

const GB = 1_000_000_000;
// Today's short list (core/llm.rs): Qwen3 4B Instruct recommended, then Gemma 4 E2B and Qwen3 1.7B.
const QWEN = 'llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m';
const GEMMA = 'llm--unsloth--gemma-4-e2b-it-gguf--q4_k_m';
const SMALL = 'llm--unsloth--qwen3-1.7b-gguf--q4_k_m';

/** The local list as the core sends it: nothing downloaded, Qwen3 4B recommended. */
function llms(): ModelsView {
  const entries = [
    { id: QWEN, name: 'Qwen3 4B Instruct', repo: 'unsloth/Qwen3-4B-Instruct-2507-GGUF', languages: [], size: 2.5 * GB, state: 'missing' },
    { id: GEMMA, name: 'Gemma 4 E2B', repo: 'unsloth/gemma-4-E2B-it-GGUF', languages: [], size: 3.1 * GB, state: 'missing' },
    { id: SMALL, name: 'Qwen3 1.7B', repo: 'unsloth/Qwen3-1.7B-GGUF', languages: [], size: 1.1 * GB, state: 'missing' },
  ];
  return { current: '', entries, recommended: QWEN, found: [] } as ModelsView;
}

/** OpenCode (found as OpenCode Desktop) and Codex signed in. */
const both = () =>
  agents({
    opencode: { found: true, models: [{ id: 'anthropic/claude-haiku-5-5', name: 'Claude Haiku 5.5' }], model: 'anthropic/claude-haiku-5-5' },
    codex: { found: true, signedIn: true },
  });

const parts = () => pg().querySelectorAll<HTMLElement>('[data-seg="where"] button');
const pressedPart = () => [...parts()].find((b) => b.getAttribute('aria-pressed') === 'true')?.dataset.v;
const agentRow = (id: string) => pg().querySelector<HTMLElement>(`[data-agent="${id}"]`)!;
const changes = () => core.settingsSet.mock.calls.map(([c]) => c);
const note = () => text(pg().querySelector('.tft'));

describe('The welcome step "Who cleans the text"', () => {
  it('steps_are_hello_cleanup_model_mic_try_done', async () => {
    expect(STEPS).toEqual(['hello', 'cleanup', 'model', 'mic', 'try', 'done']);
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    // The agents are looked at on the step, not before.
    expect(core.agentsGet).not.toHaveBeenCalled();
    await next();
    expect(step()).toBe('cleanup');
    expect(heading()).toBe('Who cleans the text');
    // The Cleanup page itself: its three parts, its lists, its note.
    expect([...parts()].map(text)).toEqual(['No AI', 'This computer', 'Your AI']);
    expect(core.agentsGet).toHaveBeenCalledTimes(1);
    expect(pg().querySelector('.tft')).not.toBeNull();
    // Its instructions are the main window's: no row for them here, and they are never asked for.
    expect(pg().querySelector('[data-sub="instructions"]')).toBeNull();
    expect(core.instructionsGet).not.toHaveBeenCalled();
    expect(text(primary())).toBe('Next');
    expect(primary().disabled).toBe(false);
    await next();
    expect(step()).toBe('model');
    for (const s of ['mic', 'try'] as const) {
      await reach(s);
      expect(step()).toBe(s);
    }
  });

  it('default_is_opencode_when_found', async () => {
    core.agentsGet.mockResolvedValue(both());
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await next();
    expect(changes()).toEqual([{ kind: 'refineAgent', value: 'opencode' }]);
    expect(pressedPart()).toBe('agent');
    expect(agentRow('opencode').getAttribute('aria-current')).toBe('true');
    expect(agentRow('codex').getAttribute('aria-current')).toBeNull();
    expect(note()).toBe('Text goes wherever OpenCode is set up to send it: your provider, your key or subscription. CUE holds no keys.');
    // Nothing downloads, and leaving and coming back chooses nothing again.
    click(back());
    await settle();
    await next();
    expect(changes()).toHaveLength(1);
    expect(core.llmChoose).not.toHaveBeenCalled();
  });

  it('default_is_opencode_when_found, also over this computer with nothing chosen', async () => {
    stub(view({ refineWhere: 'local' }));
    core.agentsGet.mockResolvedValue(both());
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await next();
    expect(changes()).toEqual([
      { kind: 'refineAgent', value: 'opencode' },
      { kind: 'refineWhere', value: 'agent' },
    ]);
    expect(pressedPart()).toBe('agent');
    expect(agentRow('opencode').getAttribute('aria-current')).toBe('true');
  });

  it('default_is_first_found_agent', async () => {
    core.agentsGet.mockResolvedValue(agents({ codex: { found: true, signedIn: true } }));
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await next();
    expect(changes()).toEqual([{ kind: 'refineAgent', value: 'codex' }]);
    expect(pressedPart()).toBe('agent');
    expect(agentRow('codex').getAttribute('aria-current')).toBe('true');
    // OpenCode, not installed, keeps its place first: pale, recommended, with its install button.
    expect(agentRow('opencode').classList.contains('off')).toBe(true);
    expect(text(agentRow('opencode').querySelector('small'))).toContain('Recommended');
    click(agentRow('opencode').querySelector('button'));
    expect(core.agentInstallLink).toHaveBeenCalledWith('opencode');
  });

  it('no_agent_highlights_recommended_and_downloads_nothing', async () => {
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await next();
    // None found: This computer, with nothing chosen there.
    expect(changes()).toEqual([{ kind: 'refineWhere', value: 'local' }]);
    expect(pressedPart()).toBe('local');
    const rows = [...pg().querySelectorAll<HTMLElement>('.mlist [data-mid]')];
    expect(rows.map((r) => r.dataset.mid)).toEqual([QWEN, GEMMA, SMALL]);
    // The recommended model first, under "For your languages", marked so.
    expect(text(pg().querySelector('.mlist .mgh'))).toContain('For your languages');
    expect(text(rows[0].querySelector('small'))).toBe('Recommended');
    expect(rows.every((r) => r.getAttribute('aria-current') === null && r.dataset.state === 'missing')).toBe(true);
    expect(note()).toBe('Until a model is chosen, text goes in without AI.');
    expect(core.llmChoose).not.toHaveBeenCalled();
    // A press starts its download.
    click(rows[0]);
    await settle();
    expect(core.llmChoose).toHaveBeenCalledWith({ id: QWEN });
  });

  it('keeps a choice already made', async () => {
    for (const s of [view({ refineWhere: 'off' }), view({ refineWhere: 'agent', refineAgent: 'codex' }), view({ refineWhere: 'local', refineLocal: GEMMA })]) {
      stub(s);
      core.agentsGet.mockResolvedValue(both());
      core.llmsGet.mockResolvedValue(llms());
      page?.dispose();
      page = await mount();
      await next();
      expect(changes()).toEqual([]);
      expect(pressedPart()).toBe(s.refineWhere);
    }
    expect(agentRow('codex')).toBeNull();
  });

  it('the parts and the agents are chosen as on the Cleanup page', async () => {
    core.agentsGet.mockResolvedValue(both());
    core.llmsGet.mockResolvedValue(llms());
    page = await mount();
    await next();
    click(agentRow('codex'));
    await settle();
    expect(changes().at(-1)).toEqual({ kind: 'refineAgent', value: 'codex' });
    expect(agentRow('codex').getAttribute('aria-current')).toBe('true');
    expect(note()).toBe('Text goes to ChatGPT through Codex, under your sign-in. CUE holds no keys.');
    click([...parts()].find((b) => b.dataset.v === 'off')!);
    await settle();
    expect(changes().at(-1)).toEqual({ kind: 'refineWhere', value: 'off' });
    expect(pressedPart()).toBe('off');
    expect(text(pg().querySelector('.srow b'))).toBe('What CUE does without AI');
    expect(pg().querySelector('[data-agent]')).toBeNull();
    // The step keeps the choice when left and entered again.
    await next();
    click(back());
    await settle();
    expect(pressedPart()).toBe('off');
    expect(changes().at(-1)).toEqual({ kind: 'refineWhere', value: 'off' });
  });

  it('speaks Russian on its step', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    core.agentsGet.mockResolvedValue(both());
    page = await mount();
    await next();
    expect(heading()).toBe('Кто чистит текст');
    expect($$('.dots i')[1].title).toBe('Кто чистит текст');
    expect(text(agentRow('opencode').querySelector('small'))).toContain('Рекомендуем');
  });
});
