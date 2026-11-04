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

const usd = (id: string, name: string, input: number, output: number): api.ModelView => ({ id, name, usd: [input, output] });

/** The cleanup's cloud connections as the core lists them (`cue_core::connections`). */
export const CONNECTIONS: api.ConnectionView[] = [
  { id: 'anthropic', name: 'Anthropic', models: [usd('claude-haiku-5-5', 'Claude Haiku 5.5', 0.1, 0.5), usd('claude-sonnet-5-5', 'Claude Sonnet 5.5', 2, 10)] },
  { id: 'openai', name: 'OpenAI', models: [usd('gpt-6-luna', 'GPT-6 Luna', 0.1, 0.5), usd('gpt-6.1-terra', 'GPT-6.1 Terra', 1, 5)] },
  { id: 'gemini', name: 'Google Gemini', models: [usd('gemini-3.1-flash-lite', 'Gemini 3.1 Flash-Lite', 0.25, 1.5)] },
  { id: 'openrouter', name: 'OpenRouter', models: [usd('anthropic/claude-haiku-5.5', 'Claude Haiku 5.5', 0.1, 0.5)] },
  { id: 'deepseek', name: 'DeepSeek', models: [usd('deepseek-flash', 'DeepSeek Flash', 0.3, 1.2)] },
  { id: 'mistral', name: 'Mistral', models: [usd('mistral-small-2603', 'Mistral Small 4', 0.15, 0.6)] },
  { id: 'groq', name: 'Groq', models: [usd('openai/gpt-oss-120b', 'gpt-oss 120B', 0.15, 0.6)] },
  { id: 'xai', name: 'xAI', models: [usd('grok-4.3', 'Grok 4.3', 1.25, 2.5)] },
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
    dictionary: [],
    refineStyle: 'default',
    refineCustom: '',
    refineConnection: 'anthropic',
    refineModel: 'claude-haiku-5-5',
    refineRuns: 'cloud',
    refineLocal: '',
    connections: CONNECTIONS,
    learned: [],
    learn: true,
    habits: [],
    ...over,
  };
}

/** A habit as the core lists it. */
export function habit(id: string, over: Partial<api.HabitView> = {}): api.HabitView {
  return { id, rule: 'In Telegram — lower case and no full stop', before: 'Send the file.', after: 'send the file', app: 'Telegram', n: 3, on: true, ...over };
}

export function entry(id: string, text: string, over: Partial<api.EntryView> = {}): api.EntryView {
  const time = id.slice(11, 16);
  return { id, time, mode: 'dictation', app: '', text, question: null, raw: null, hasAudio: false, habits: [], ...over };
}

/** The 25 languages of Parakeet v3 and Ultra. */
const EUROPEAN = ['en', 'es', 'fr', 'de', 'bg', 'hr', 'cs', 'da', 'nl', 'et', 'fi', 'el', 'hu', 'it', 'lv', 'lt', 'mt', 'pl', 'pt', 'ro', 'sk', 'sl', 'sv', 'ru', 'uk'];

/** The Model page's list as the core sends it: the short list, Ultra in use, the others not downloaded. */
export function modelsView(over: Partial<Record<string, Partial<api.ModelEntry>>> = {}, current = 'parakeet-ultra-int8'): api.ModelsView {
  const entries: api.ModelEntry[] = [
    { id: 'parakeet-ultra-int8', name: 'Parakeet Ultra', repo: 'Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx', languages: EUROPEAN, size: 667_729_022, state: 'inUse' },
    { id: 'parakeet-v3-int8', name: 'Parakeet v3', repo: 'istupakov/parakeet-tdt-0.6b-v3-onnx', languages: EUROPEAN, size: 670_479_942, state: 'missing' },
    { id: 'parakeet-v2-int8', name: 'Parakeet v2', repo: 'istupakov/parakeet-tdt-0.6b-v2-onnx', languages: ['en'], size: 661_191_684, state: 'missing' },
  ];
  const changed = entries.map((e) => ({ ...e, ...over[e.id] }) as api.ModelEntry);
  for (const [id, e] of Object.entries(over)) if (!entries.some((x) => x.id === id)) changed.push(e as api.ModelEntry);
  return { current, entries: changed };
}

export const core = vi.mocked(api);

/** What the page subscribed with: new history entries, the microphone's level, a failed microphone, the core's state. */
export const feeds: {
  entry: (e: api.EntryView) => void;
  level: (l: number) => void;
  failed: () => void;
  state: (s: api.CoreState) => void;
  model: (m: api.ModelState) => void;
  models: (m: api.ModelsView) => void;
  llms: (m: api.ModelsView) => void;
  settings: (v: api.SettingsView) => void;
} = {
  entry: () => {},
  level: () => {},
  failed: () => {},
  state: () => {},
  model: () => {},
  models: () => {},
  llms: () => {},
  settings: () => {},
};

/** Answers of a core that holds `settings` and changes them as asked. */
export function stub(settings: api.SettingsView = view()): void {
  vi.resetAllMocks();
  let now = settings;
  core.settingsGet.mockImplementation(async () => now);
  core.settingsSet.mockImplementation(async (c) => {
    // The dictionary as the core keeps it: a new word first (a repeat in any case refused), a word removed as written.
    if (c.kind === 'dictAdd') {
      const taken = now.dictionary.some((w) => w.toLowerCase() === c.value.trim().toLowerCase());
      if (!taken) now = { ...now, dictionary: [c.value.trim(), ...now.dictionary] };
      return now;
    }
    if (c.kind === 'dictRemove') {
      now = { ...now, dictionary: now.dictionary.filter((w) => w !== c.value), learned: now.learned.filter((l) => l.word !== c.value) };
      return now;
    }
    // The habits as the core keeps them: one switched in place, one forgotten, or all.
    if (c.kind === 'habitOn') {
      now = { ...now, habits: now.habits.map((x) => (x.id === c.value.id ? { ...x, on: c.value.on } : x)) };
      return now;
    }
    if (c.kind === 'habitForget') {
      now = { ...now, habits: now.habits.filter((x) => x.id !== c.value) };
      return now;
    }
    if (c.kind === 'habitsForget') {
      now = { ...now, habits: [] };
      return now;
    }
    now = { ...now, [c.kind]: c.value };
    if (c.kind === 'uiLanguage') now.lang = c.value === 'ru' ? 'ru' : 'en';
    // Another connection: its recommended model, as the core answers.
    if (c.kind === 'refineConnection') {
      const conn = now.connections.find((x) => x.id === c.value)!;
      now = { ...now, refineModel: conn.models[0].id };
    }
    return now;
  });
  core.autostartGet.mockResolvedValue(false);
  core.autostartSet.mockImplementation(async (on) => on);
  core.historySubscribe.mockImplementation(async (cb) => {
    feeds.entry = cb;
  });
  core.historyPage.mockResolvedValue({ days: [], more: false });
  core.statsGet.mockResolvedValue({ todayWords: 0, savedMinutes: 0, days: [], wpm: null, streak: 0, total: 0, milestone: null });
  core.historySearch.mockResolvedValue([]);
  core.historyCopy.mockResolvedValue(undefined);
  core.historyPaste.mockResolvedValue('pasted');
  core.historyClear.mockResolvedValue(undefined);
  core.historyAudio.mockResolvedValue(null);
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
  core.onCoreState.mockImplementation(async (cb) => {
    feeds.state = cb;
  });
  core.modelGet.mockResolvedValue({ state: 'ready', total: 667_729_022 });
  core.modelRetry.mockResolvedValue(undefined);
  core.onModel.mockImplementation(async (cb) => {
    feeds.model = cb;
  });
  core.onSettings.mockImplementation(async (cb) => {
    feeds.settings = cb;
  });
  core.modelsGet.mockResolvedValue(modelsView());
  core.onModels.mockImplementation(async (cb) => {
    feeds.models = cb;
  });
  core.modelsSearch.mockResolvedValue([]);
  core.modelChoose.mockResolvedValue(undefined);
  core.modelDelete.mockResolvedValue(true);
  core.llmsGet.mockResolvedValue({ current: '', entries: [], recommended: null });
  core.onLlms.mockImplementation(async (cb) => {
    feeds.llms = cb;
  });
  core.llmsSearch.mockResolvedValue([]);
  core.llmChoose.mockResolvedValue(undefined);
  core.llmDelete.mockResolvedValue(true);
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

/** A made-up Ogg Opus file laid out as CUE writes one — OpusHead alone on the first page, the end-of-stream page last —
 *  that plays `seconds` (its last position less the pre-skip, at 48 kHz). No sound in it: nothing here decodes it. */
export function oggFile(seconds: number, preSkip = 312): ArrayBuffer {
  const page = (type: number, granule: number, seq: number, packet: number[]) => {
    const p = new Uint8Array(28 + packet.length);
    p.set([0x4f, 0x67, 0x67, 0x53, 0, type]);
    const v = new DataView(p.buffer);
    v.setUint32(6, granule % 2 ** 32, true);
    v.setUint32(10, Math.floor(granule / 2 ** 32), true);
    v.setUint32(14, 7, true);
    v.setUint32(18, seq, true);
    p[26] = 1;
    p[27] = packet.length;
    p.set(packet, 28);
    return p;
  };
  const head = [...'OpusHead'].map((c) => c.charCodeAt(0)).concat([1, 1, preSkip & 255, preSkip >> 8, 0x80, 0x3e, 0, 0, 0, 0, 0]);
  const tags = [...'OpusTags'].map((c) => c.charCodeAt(0)).concat([3, 0, 0, 0, 67, 85, 69, 0, 0, 0, 0]);
  const parts = [page(2, 0, 0, head), page(0, 0, 1, tags), page(0, preSkip + 960, 2, [8, 1, 2]), page(4, preSkip + Math.round(seconds * 48_000), 3, [8, 3, 4])];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out.buffer;
}
