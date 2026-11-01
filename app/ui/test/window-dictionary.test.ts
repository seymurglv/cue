import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, key, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import { MAX_WORD_CHARS, refusedWord } from '../src/window/dictionary';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const words = () => $$('.dflow .dch').map((c) => c.dataset.word);
const input = () => $<HTMLInputElement>('.dni')!;

/** Types `word` into the open field and presses Enter. */
async function enter(word: string): Promise<void> {
  const i = input();
  i.value = word;
  i.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
  await settle();
}

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub(view({ dictionary: ['Soniox', 'GitHub'] }));
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('The dictionary', () => {
  it('shows the words as chips with their count', async () => {
    page = await mount();
    expect(text($('[data-scr=dictionary] h3'))).toBe('Dictionary2');
    expect(words()).toEqual(['Soniox', 'GitHub']);
    expect($('.dch .dx')!.getAttribute('aria-label')).toBe('Remove');
  });

  it('adds a word first, with a glint, and keeps the field for the next', async () => {
    page = await mount();
    click($('.dnb'));
    expect($('.dnew')!.classList.contains('open')).toBe(true);
    expect(document.activeElement).toBe(input());
    await enter('  Kubernetes ');
    expect(core.settingsSet).toHaveBeenLastCalledWith({ kind: 'dictAdd', value: 'Kubernetes' });
    expect(words()).toEqual(['Kubernetes', 'Soniox', 'GitHub']);
    expect($('.dch')!.classList.contains('new')).toBe(true);
    expect(input().value).toBe('');
    expect($('.dnew')!.classList.contains('open')).toBe(true);
    expect(text($('[data-scr=dictionary] h3 em'))).toBe('3');
  });

  it('shakes on a repeat or a word it cannot take, and asks nothing', async () => {
    page = await mount();
    click($('.dnb'));
    for (const bad of ['soniox', 'x'.repeat(MAX_WORD_CHARS + 1)]) {
      await enter(bad);
      expect(input().classList.contains('bad')).toBe(true);
    }
    expect(core.settingsSet).not.toHaveBeenCalled();
    expect(words()).toEqual(['Soniox', 'GitHub']);
  });

  it('removes a word: the chip folds away', async () => {
    page = await mount();
    click($$('.dch')[0].querySelector('.dx'));
    expect($('.dch')!.classList.contains('out')).toBe(true);
    await settle();
    expect(core.settingsSet).toHaveBeenLastCalledWith({ kind: 'dictRemove', value: 'Soniox' });
    expect(words()).toEqual(['GitHub']);
  });

  it('closes the field with Esc and leaves the history search alone', async () => {
    page = await mount();
    click($('.dnb'));
    input().value = 'half';
    input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect($('.dnew')!.classList.contains('open')).toBe(false);
    expect(input().value).toBe('');
    click($('.dnb'));
    key('Escape');
    expect($('.dnew')!.classList.contains('open')).toBe(false);
  });

  it('an empty dictionary shows only the hint and the button', async () => {
    stub(view({ dictionary: [] }));
    page = await mount();
    expect($('.dflow')!.hidden).toBe(true);
    expect(text($('[data-scr=dictionary] h3'))).toBe('Dictionary');
    expect($('.dnb')).not.toBeNull();
  });

  it('refuses what the core would refuse', () => {
    expect(refusedWord('  ', [])).toBe(true);
    expect(refusedWord('a\nb', [])).toBe(true);
    expect(refusedWord('GITHUB', ['GitHub'])).toBe(true);
    expect(refusedWord('я'.repeat(MAX_WORD_CHARS), [])).toBe(false);
    expect(refusedWord(' Kubernetes ', ['GitHub'])).toBe(false);
  });
});
