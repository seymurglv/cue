import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, settle, stub, text, view } from './windowFixture';
import { mountWelcome, TYPED_TO_DONE_MS, type WelcomePage } from '../src/welcome/page';
import { keysReady, wordCount } from '../src/welcome/steps';
import { doneTitle, en, ru } from '../src/welcome/text';
import { KeyRows } from '../src/window/keys';
import type { KeyName } from '../src/window/api';

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
const steps = () => core.welcomeStep.mock.calls.map(([dictation]) => dictation);
const previews = () => core.micPreview.mock.calls.map(([on, device]) => (on ? `on:${device ?? 'windows'}` : 'off'));
const saved = () => core.keyState.mockResolvedValue({ saved: true, mask: 'sk-…789' });
const field = () => pg().querySelector<HTMLTextAreaElement>('textarea[data-field]')!;
const dictate = (value: string) => {
  field().value = value;
  field().dispatchEvent(new Event('input', { bubbles: true }));
};
/** A key pasted into its field, not checked. */
const paste = (input: HTMLInputElement, value: string) => {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
};
/** A mouse press on `e`; true — the page cancelled it, so the focus stays where it is. */
const press = (e: Element) => !e.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
/** The window has the focus again. */
const regain = () => window.dispatchEvent(new Event('focus'));

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
    saved();
    page = await mount();
    const dots = () => $$('.dots i').map((d) => d.className);
    expect($$('.dots i').map((d) => d.title)).toEqual(['Hello', 'Keys', 'Microphone', 'First dictation', 'Done']);
    expect(step()).toBe('hello');
    expect(heading()).toBe('Speak — CUE writes');
    expect(dots()).toEqual(['on', '', '', '', '']);
    expect(back().hidden).toBe(true);
    expect(text(primary())).toBe('Start');
    await next();
    expect(step()).toBe('keys');
    expect(heading()).toBe('Keys');
    expect(dots()).toEqual(['ok', 'on', '', '', '']);
    expect(back().hidden).toBe(false);
    expect(text(primary())).toBe('Next');
    click(back());
    await settle();
    expect(step()).toBe('hello');
    expect(back().hidden).toBe(true);
    await next();
    await next();
    expect(step()).toBe('mic');
    expect(heading()).toBe('Microphone');
    expect(text(primary())).toBe('It works');
    await next();
    expect(step()).toBe('try');
    expect(heading()).toBe('First dictation');
    expect(text(primary())).toBe('Waiting for dictation');
    expect(primary().disabled).toBe(true);
    // Only one page at a time with reduced motion.
    expect($$('.pgv')).toHaveLength(1);
    click(back());
    await settle();
    expect(step()).toBe('mic');
    expect(dots()).toEqual(['ok', 'ok', 'on', '', '']);
  });

  it('holds the hotkey on load', async () => {
    page = await mount();
    expect(steps()).toEqual([false]);
    // Before anything else: a reload starts from a held key and a closed microphone.
    expect(core.welcomeStep.mock.invocationCallOrder[0]).toBeLessThan(core.settingsGet.mock.invocationCallOrder[0]);
    expect(previews()).toEqual(['off']);
  });

  it('Next on Keys stays off without both keys and on a key error', async () => {
    page = await mount();
    await next();
    expect(text($('small[data-state="soniox"]'))).toBe('No key');
    expect(primary().disabled).toBe(true);
    // A click on the disabled button goes nowhere.
    click(primary());
    await settle();
    expect(step()).toBe('keys');
    const typeKey = async (name: KeyName, value: string) => {
      $<HTMLInputElement>(`input[data-key="${name}"]`)!.value = value;
      click($(`[data-check="${name}"]`));
      await settle();
    };
    await typeKey('soniox', 'soniox-key-1234');
    expect(text($('small[data-state="soniox"]'))).toBe('Works');
    expect(primary().disabled).toBe(true);
    core.keyCheck.mockResolvedValueOnce('badKey');
    await typeKey('anthropic', 'sk-ant-wrong-5678');
    expect(text($('small[data-state="anthropic"]'))).toBe('Key error');
    expect(primary().disabled).toBe(true);
    // While a check runs, its result is not known yet.
    let answer: (v: 'works') => void = () => {};
    core.keyCheck.mockImplementationOnce(() => new Promise((r) => (answer = r)));
    click($('[data-check="anthropic"]'));
    await settle();
    expect(text($('small[data-state="anthropic"]'))).toBe('Checking…');
    expect(primary().disabled).toBe(true);
    answer('works');
    await settle();
    expect(primary().disabled).toBe(false);
    await next();
    expect(step()).toBe('mic');
  });

  it('a key pasted over saved ones holds Next until it is checked', async () => {
    saved();
    page = await mount();
    await next();
    expect(primary().disabled).toBe(false);
    const input = $<HTMLInputElement>('input[data-key="anthropic"]')!;
    paste(input, 'sk-ant-newer-2468');
    expect(primary().disabled).toBe(true);
    click(primary());
    await settle();
    expect(step()).toBe('keys');
    expect(core.keySet).not.toHaveBeenCalled();
    // Taken out again (spaces alone are no key): Next as before.
    paste(input, '  ');
    expect(primary().disabled).toBe(false);
    paste(input, 'sk-ant-newer-2468');
    click($('[data-check="anthropic"]'));
    await settle();
    expect(core.keySet).toHaveBeenCalledWith('anthropic', 'sk-ant-newer-2468');
    expect(input.value).toBe('');
    expect(text($('small[data-state="anthropic"]'))).toBe('Works');
    expect(primary().disabled).toBe(false);
    await next();
    expect(step()).toBe('mic');
  });

  it('a key never stays in the page after it is saved, only its mask', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    page = await mount();
    await next();
    const input = $<HTMLInputElement>('input[data-key="soniox"]')!;
    expect(input.type).toBe('password');
    expect(input.getAttribute('autocomplete')).toBe('off');
    input.value = 'soniox-secret-4321';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    // Emptied at once, before the core answers.
    expect(input.value).toBe('');
    await settle();
    expect(core.keySet).toHaveBeenCalledWith('soniox', 'soniox-secret-4321');
    expect(input.placeholder).toBe('son…321');
    expect(document.documentElement.outerHTML).not.toContain('secret');
    // A refused save: nothing in the page or the log either.
    core.keySet.mockRejectedValueOnce('store');
    const other = $<HTMLInputElement>('input[data-key="anthropic"]')!;
    other.value = 'sk-ant-hidden-8765';
    click($('[data-check="anthropic"]'));
    await settle();
    expect(other.value).toBe('');
    expect(document.documentElement.outerHTML).not.toContain('hidden-8765');
    expect(JSON.stringify(errors.mock.calls)).not.toContain('hidden-8765');
    // A key typed and not checked leaves with its step.
    other.value = 'sk-ant-typed-1111';
    click(back());
    await settle();
    expect(other.value).toBe('');
    expect(core.keySet).not.toHaveBeenCalledWith('anthropic', 'sk-ant-typed-1111');
  });

  it('the links open the consoles, and the step says what it costs and where things go', async () => {
    page = await mount();
    await next();
    click($('[data-link="soniox"]'));
    click($('[data-link="anthropic"]'));
    expect(core.openLink.mock.calls).toEqual([['soniox'], ['anthropic']]);
    expect($$('.onote').map(text)).toEqual([
      'Roughly $4–5 a month for an hour of speech a day.',
      'Sound goes to Soniox, text to Anthropic; the history stays on this computer only.',
    ]);
  });

  it('the microphone preview runs only on its step', async () => {
    saved();
    page = await mount();
    core.micPreview.mockClear();
    await next();
    await next();
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
    saved();
    core.micList.mockResolvedValue({ names: ['Laptop Microphone', 'USB Microphone'], default: 'Laptop Microphone' });
    page = await mount();
    await next();
    await next();
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
  });

  it('a denied, failed or missing microphone shows the hint with the Windows settings', async () => {
    saved();
    core.micAccess.mockResolvedValue('denied');
    page = await mount();
    await next();
    await next();
    const hint = () => pg().querySelector<HTMLElement>('.ohint')!;
    expect(hint().hidden).toBe(false);
    expect(text(hint().querySelector('span'))).toBe('Windows keeps the microphone from CUE');
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
    saved();
    core.micAccess.mockResolvedValue('denied');
    page = await mount();
    await next();
    await next();
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
    saved();
    page = await mount();
    await next();
    await next();
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
    saved();
    page = await mount();
    await next();
    await next();
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
    saved();
    page = await mount();
    for (let i = 0; i < 3; i++) await next();
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
      'CUE lives by the clock: double-click its icon for the window, right-click for the menu',
    ]);
    expect($$('.otips .keys').map((k) => k.getAttribute('aria-label'))).toEqual(['Ctrl + Win + Shift', 'Ctrl + Win + Alt']);
  });

  it('Back during the wait for Done stays on the steps', async () => {
    saved();
    page = await mount();
    for (let i = 0; i < 3; i++) await next();
    dictate('Hello');
    click(back());
    await settle();
    vi.advanceTimersByTime(TYPED_TO_DONE_MS);
    await settle();
    expect(step()).toBe('mic');
  });

  it('Done closes the window', async () => {
    saved();
    page = await mount();
    for (let i = 0; i < 3; i++) await next();
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
        saved();
        page?.dispose();
        page = await mount(reduced);
        expect($$('.pgv')).toHaveLength(1);
        for (let i = 0; i < 3; i++) await next();
        dictate('Hello');
        vi.advanceTimersByTime(TYPED_TO_DONE_MS);
        await settle();
        expect(step()).toBe('done');
        // The steps slid with motion, and went at once without it.
        expect($$('.pgv')).toHaveLength(reduced ? 1 : 5);
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
    saved();
    page = await mount();
    expect(document.documentElement.lang).toBe('ru');
    expect(heading()).toBe('Говорите — CUE напишет');
    expect(text(primary())).toBe('Начать');
    await next();
    expect(heading()).toBe('Ключи');
    expect(text($('[data-check="soniox"]'))).toBe('Проверить');
    expect($$('.onote').map(text)).toEqual([
      'Примерно $4–5 в месяц за час речи в день.',
      'Звук уходит в Soniox, текст — в Anthropic; история хранится только на этом компьютере.',
    ]);
    await next();
    expect(text($('[data-dd="mic"] .ddv'))).toBe('Как в Windows');
    await next();
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

  it('keys are ready when both are saved and neither is refused', () => {
    const rows = new KeyRows({ t: {} as any }, () => null, () => {});
    expect(keysReady(rows)).toBe(false);
    rows.read = true;
    rows.keys.soniox.saved = true;
    expect(keysReady(rows)).toBe(false);
    rows.keys.anthropic.saved = true;
    expect(keysReady(rows)).toBe(true);
    for (const state of ['unchecked', 'store', 'works'] as const) {
      rows.keys.anthropic.state = state;
      expect(keysReady(rows)).toBe(true);
    }
    for (const state of ['badKey', 'checking', 'missing'] as const) {
      rows.keys.anthropic.state = state;
      expect(keysReady(rows)).toBe(false);
    }
    // A key typed and not checked yet.
    const box = document.createElement('div');
    box.innerHTML = '<input data-key="soniox">';
    const typed = new KeyRows({ t: {} as any }, () => box, () => {});
    typed.read = true;
    typed.keys.soniox.saved = typed.keys.anthropic.saved = true;
    expect(keysReady(typed)).toBe(true);
    box.querySelector('input')!.value = 'soniox-typed-1357';
    expect(keysReady(typed)).toBe(false);
  });
});
