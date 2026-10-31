// The main window's page, and the welcome window's, over a stand-in core: each test file mocks '../src/window/api'
// (every command a vi.fn) and sets the answers here. The clock is fake: today is Saturday 31 October 2026.
import { vi } from 'vitest';
import * as api from '../src/window/api';
import { mountWindow, type WindowPage } from '../src/window/page';

export const NOW = new Date(2026, 9, 31, 12, 0);
export const TODAY = '2026-10-31';
export const YESTERDAY = '2026-10-30';

/** The 60 languages a translation can go to, as the core lists them. */
export const LANGUAGES: [string, string][] = [
  ['af', 'Afrikaans'], ['sq', 'Albanian'], ['ar', 'Arabic'], ['az', 'Azerbaijani'], ['eu', 'Basque'], ['be', 'Belarusian'],
  ['bn', 'Bengali'], ['bs', 'Bosnian'], ['bg', 'Bulgarian'], ['ca', 'Catalan'], ['zh', 'Chinese'], ['hr', 'Croatian'],
  ['cs', 'Czech'], ['da', 'Danish'], ['nl', 'Dutch'], ['en', 'English'], ['et', 'Estonian'], ['fi', 'Finnish'],
  ['fr', 'French'], ['gl', 'Galician'], ['de', 'German'], ['el', 'Greek'], ['gu', 'Gujarati'], ['he', 'Hebrew'],
  ['hi', 'Hindi'], ['hu', 'Hungarian'], ['id', 'Indonesian'], ['it', 'Italian'], ['ja', 'Japanese'], ['kn', 'Kannada'],
  ['kk', 'Kazakh'], ['ko', 'Korean'], ['lv', 'Latvian'], ['lt', 'Lithuanian'], ['mk', 'Macedonian'], ['ms', 'Malay'],
  ['ml', 'Malayalam'], ['mr', 'Marathi'], ['no', 'Norwegian'], ['fa', 'Persian'], ['pl', 'Polish'], ['pt', 'Portuguese'],
  ['pa', 'Punjabi'], ['ro', 'Romanian'], ['ru', 'Russian'], ['sr', 'Serbian'], ['sk', 'Slovak'], ['sl', 'Slovenian'],
  ['es', 'Spanish'], ['sw', 'Swahili'], ['sv', 'Swedish'], ['tl', 'Tagalog'], ['ta', 'Tamil'], ['te', 'Telugu'],
  ['th', 'Thai'], ['tr', 'Turkish'], ['uk', 'Ukrainian'], ['ur', 'Urdu'], ['vi', 'Vietnamese'], ['cy', 'Welsh'],
];

export function view(over: Partial<api.SettingsView> = {}): api.SettingsView {
  return {
    historyKeep: 'month',
    historyAudio: 'week',
    micDevice: null,
    translateTarget: 'en',
    uiLanguage: 'auto',
    lang: 'en',
    languages: LANGUAGES,
    saved: true,
    ...over,
  };
}

export function entry(id: string, text: string, over: Partial<api.EntryView> = {}): api.EntryView {
  const time = id.slice(11, 16);
  return { id, time, mode: 'dictation', app: '', text, question: null, raw: null, hasAudio: false, ...over };
}

export const core = vi.mocked(api);

/** What the page subscribed with: new history entries, the microphone's level, a failed microphone. */
export const feeds: { entry: (e: api.EntryView) => void; level: (l: number) => void; failed: () => void } = {
  entry: () => {},
  level: () => {},
  failed: () => {},
};

/** Answers of a core that holds `settings` and changes them as asked. */
export function stub(settings: api.SettingsView = view()): void {
  vi.resetAllMocks();
  let now = settings;
  core.settingsGet.mockImplementation(async () => now);
  core.settingsSet.mockImplementation(async (c) => {
    now = { ...now, [c.kind]: c.value };
    if (c.kind === 'uiLanguage') now.lang = c.value === 'ru' ? 'ru' : 'en';
    return now;
  });
  core.autostartGet.mockResolvedValue(false);
  core.autostartSet.mockImplementation(async (on) => on);
  core.historySubscribe.mockImplementation(async (cb) => {
    feeds.entry = cb;
  });
  core.historyPage.mockResolvedValue({ days: [], more: false });
  core.historySearch.mockResolvedValue([]);
  core.historyCopy.mockResolvedValue(undefined);
  core.historyPaste.mockResolvedValue('pasted');
  core.historyClear.mockResolvedValue(undefined);
  core.keyState.mockResolvedValue({ saved: false, mask: '' });
  core.keySet.mockImplementation(async (_name, value) => ({ saved: true, mask: `${value.slice(0, 3)}…${value.slice(-3)}` }));
  core.keyCheck.mockResolvedValue('works');
  core.micList.mockResolvedValue({ names: ['Laptop Microphone'], default: 'Laptop Microphone' });
  core.micPreview.mockResolvedValue(undefined);
  core.onMicLevel.mockImplementation(async (cb) => {
    feeds.level = cb;
  });
  core.micAccess.mockResolvedValue('allowed');
  core.onMicFailed.mockImplementation(async (cb) => {
    feeds.failed = cb;
  });
  core.welcomeStep.mockResolvedValue(undefined);
  core.welcomeClose.mockResolvedValue(undefined);
  core.openLink.mockResolvedValue(undefined);
}

/** Promises and zero-delay timers run out. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 5; i++) {
    await vi.advanceTimersByTimeAsync(0);
    for (let j = 0; j < 10; j++) await Promise.resolve();
  }
}

/** The page, built and with its first page of history; motion reduced, so nothing slides. */
export async function mount(): Promise<WindowPage> {
  document.body.innerHTML = '<div id="w"></div>';
  const page = await mountWindow(document.getElementById('w')!, { reduced: () => true });
  await settle();
  return page;
}

export const $ = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel);
export const $$ = <T extends Element = HTMLElement>(sel: string) => [...document.querySelectorAll<T>(sel)];
/** Text as the user reads it: CUE.setLabel writes spaces as no-break spaces. */
export const text = (e: Element | null) => (e?.textContent ?? '').replace(/ /g, ' ');

export function click(e: Element | null): void {
  if (!e) throw new Error('nothing to click');
  e.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }));
  e.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

export function key(k: string): void {
  document.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
}

export function type(input: HTMLInputElement, value: string): void {
  input.value = value;
  input.dispatchEvent(new Event('input', { bubbles: true }));
}
