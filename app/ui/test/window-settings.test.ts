import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, TODAY, click, core, entry, feeds, key, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';
import { matchMic } from '../src/window/settings';

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
      'Keys',
      'Microphone',
      'Translation',
      'General',
    ]);
    // Each row with its value.
    expect($$('.btray .trw em').map((e) => e.textContent)).toEqual(['No keys', 'As in Windows', 'English', 'Auto']);
    await sub('translate');
    expect(title()).toBe('Translation');
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

  it('key field is a password field and is cleared after check', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    page = await mount();
    await open();
    await sub('keys');
    const input = $<HTMLInputElement>('input[data-key="soniox"]')!;
    expect(input.type).toBe('password');
    expect(input.getAttribute('autocomplete')).toBe('off');
    expect(input.placeholder).toBe('Paste the key');
    input.value = 'sk-secret-123';
    click($('[data-check="soniox"]'));
    // Emptied at once, before the core answers.
    expect(input.value).toBe('');
    await settle();
    expect(core.keySet).toHaveBeenCalledWith('soniox', 'sk-secret-123');
    expect(core.keyCheck).toHaveBeenCalledWith('soniox');
    expect(input.value).toBe('');
    expect(input.placeholder).toBe('sk-…123');
    expect(text($('small[data-state="soniox"]'))).toBe('Works');
    // A refused save: the key never reaches the log.
    core.keySet.mockRejectedValueOnce('store');
    input.value = 'sk-other-456';
    click($('[data-check="soniox"]'));
    await settle();
    expect(input.value).toBe('');
    expect(core.keyCheck).toHaveBeenCalledTimes(1);
    expect(text($('small[data-state="soniox"]'))).toBe("Couldn't check");
    const logged = JSON.stringify(errors.mock.calls);
    expect(logged).not.toContain('secret');
    expect(logged).not.toContain('other-456');
    // Enter in the field checks too.
    input.value = 'sk-third-789';
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(input.value).toBe('');
    await settle();
    expect(core.keySet).toHaveBeenLastCalledWith('soniox', 'sk-third-789');
    errors.mockRestore();
  });

  it('key states read works and key error', async () => {
    core.keyState.mockImplementation(async (name) => ({ saved: true, mask: name === 'soniox' ? 'abc…xyz' : 'sk-…789' }));
    core.keyCheck.mockImplementation(async (name) => (name === 'soniox' ? 'works' : 'badKey'));
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="keys"]'))).toBe('Soniox, Anthropic');
    await sub('keys');
    expect(text($('small[data-state="soniox"]'))).toBe('Saved');
    expect($<HTMLInputElement>('input[data-key="anthropic"]')!.placeholder).toBe('sk-…789');
    click($('[data-check="soniox"]'));
    click($('[data-check="anthropic"]'));
    expect(text($('small[data-state="soniox"]'))).toBe('Checking…');
    expect($<HTMLButtonElement>('[data-check="soniox"]')!.disabled).toBe(true);
    await settle();
    // Empty fields: the saved keys are checked, nothing is saved.
    expect(core.keySet).not.toHaveBeenCalled();
    const soniox = $('small[data-state="soniox"]')!;
    const anthropic = $('small[data-state="anthropic"]')!;
    expect(text(soniox)).toBe('Works');
    expect(soniox.classList.contains('ok')).toBe(true);
    expect(text(anthropic)).toBe('Key error');
    expect(anthropic.classList.contains('warn')).toBe(true);
    // The other states.
    core.keyCheck.mockResolvedValueOnce('unchecked').mockResolvedValueOnce('missing');
    click($('[data-check="soniox"]'));
    await settle();
    expect(text(soniox)).toBe("Couldn't check");
    click($('[data-check="soniox"]'));
    await settle();
    expect(text(soniox)).toBe('No key');
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
    await sub('keys');
    key('Escape');
    await sub('general');
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

  it('translation menu lists 60 languages named in the window language', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru' }));
    page = await mount();
    await open();
    expect(text($('.btray em[data-val="translate"]'))).toBe('Английский');
    await sub('translate');
    expect(title()).toBe('Перевод');
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
    await sub('general');
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
    expect(title()).toBe('Общие');
    expect($$('.segc button').map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'true']);
    expect($('.gear')!.getAttribute('aria-label')).toBe('Настройки');
    key('Escape');
    expect(title()).toBe('Настройки');
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

  it('reads the settings again when the core did not answer a change in time', async () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
    page = await mount();
    core.settingsSet.mockRejectedValueOnce('core');
    core.settingsGet.mockResolvedValueOnce(view({ translateTarget: 'fr' }));
    await open();
    await sub('translate');
    click($('[data-dd="target"]'));
    click($$('.menu [data-i]').find((b) => b.textContent === 'French')!);
    await settle();
    expect(text($('[data-dd="target"] .ddv'))).toBe('French');
    expect(errors).toHaveBeenCalledWith('CUE: setting translateTarget failed:', 'core');
    errors.mockRestore();
  });
});
