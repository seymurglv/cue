// The main window's commands (app/src-tauri/src/commands.rs) and its two feeds: new history entries over its own
// channel, and a microphone's level while the settings show it. The welcome window calls those its steps need — the
// settings (it changes the microphone alone), the keys, the microphone — and its own (welcomeStep, welcomeClose,
// openLink); any other answers it notMain. An error is a short code (notMain, core, notFound, badValue, badName,
// emptyKey, store, autostart, failed), never text.
import { Channel, invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

export type Mode = 'dictation' | 'translation' | 'ask';
/** One history entry: the time of day `HH:MM`, the app the result went to (empty when not known), the text, and for
 *  "Ask" the question (`text` is then the answer or the text the model wrote). `id` starts with the entry's local time,
 *  `YYYY-MM-DDTHH:MM:SS.mmm`. `raw` — the text as it was heard, before CUE cleaned it up (for a translation, the speech
 *  before it was translated); null for "Ask" (its question is that) and for entries of earlier versions. `hasAudio` —
 *  its recording is there to play (`historyAudio`); a new entry comes with false while its recording is encoded, so
 *  ask when the entry opens. */
export interface EntryView {
  id: string;
  time: string;
  mode: Mode;
  app: string;
  text: string;
  question: string | null;
  raw: string | null;
  hasAudio: boolean;
}
/** One day, `YYYY-MM-DD`, entries newest first. */
export interface DayView {
  date: string;
  entries: EntryView[];
}
/** Days newest first; `more` — an earlier day has entries too. */
export interface PageView {
  days: DayView[];
  more: boolean;
}

export type Keep = 'forever' | 'year' | 'month' | 'week' | 'off';
export const KEEPS: readonly Keep[] = ['forever', 'year', 'month', 'week', 'off'];
/** How long recordings are kept: a day keeps today's; never longer than the history itself. */
export type AudioKeep = 'day' | 'week' | 'month' | 'off';
export const AUDIO_KEEPS: readonly AudioKeep[] = ['day', 'week', 'month', 'off'];
export type UiLanguage = 'auto' | 'en' | 'ru';
/** The settings as the core holds them. `historyKeep` is null while the period is unknown (nothing is written to the
 *  history or deleted from it until the user picks one); `historyAudio` — how long recordings are kept, as the user
 *  chose it, null while unknown (none is recorded until the user picks one); `micDevice` null — the Windows microphone;
 *  `lang` — the interface language now; `languages` — the translation targets, code and English name; `saved` —
 *  changes reach settings.json (it was read at startup). */
export interface SettingsView {
  historyKeep: Keep | null;
  historyAudio: AudioKeep | null;
  micDevice: string | null;
  translateTarget: string;
  uiLanguage: UiLanguage;
  lang: string;
  languages: [string, string][];
  saved: boolean;
  /** The dictionary's words, the newest first. */
  dictionary: string[];
}
export type SettingChange =
  | { kind: 'historyKeep'; value: Keep }
  | { kind: 'historyAudio'; value: AudioKeep }
  | { kind: 'micDevice'; value: string | null }
  | { kind: 'translateTarget'; value: string }
  | { kind: 'uiLanguage'; value: UiLanguage }
  | { kind: 'dictAdd'; value: string }
  | { kind: 'dictRemove'; value: string };

export type KeyName = 'anthropic';
/** Whether a key is saved and its mask; never the key. */
export interface KeyView {
  saved: boolean;
  mask: string;
}
export type KeyCheck = 'works' | 'badKey' | 'unchecked' | 'missing' | 'store';
/** The microphones as Windows names them, and the one it records from by default. */
export interface MicList {
  names: string[];
  default: string | null;
}
export type PasteAgain = 'pasted' | 'clipboard';
/** Whether Windows lets CUE use the microphone. */
export type MicAccess = 'allowed' | 'denied';
/** The welcome window's links, by name. */
export type LinkTarget = 'anthropic' | 'micPrivacy';

export const settingsGet = () => invoke<SettingsView>('settings_get');
export const settingsSet = (change: SettingChange) => invoke<SettingsView>('settings_set', { change });
export const autostartGet = () => invoke<boolean>('autostart_get');
export const autostartSet = (on: boolean) => invoke<boolean>('autostart_set', { on });

/** New entries go to `onEntry`. Call it before the first page: an entry that comes meanwhile is in both (same id). */
export function historySubscribe(onEntry: (e: EntryView) => void): Promise<void> {
  const channel = new Channel<EntryView>(onEntry);
  return invoke<void>('history_subscribe', { channel });
}
/** One local day of Today's bars. */
export interface DayWords {
  date: string;
  words: number;
}
/** Today's numbers (`stats_get`): words today and the minutes they saved, the last 30 days oldest first, words a
 *  minute (null without audio), days in a row, words in all, and the largest round number reached (null below 1 000). */
export interface Stats {
  todayWords: number;
  savedMinutes: number;
  days: DayWords[];
  wpm: number | null;
  streak: number;
  total: number;
  milestone: number | null;
}
export const statsGet = () => invoke<Stats>('stats_get');
/** null — today and yesterday; a date — the days with entries before it. */
export const historyPage = (before: string | null) => invoke<PageView>('history_page', { before });
export const historySearch = (query: string) => invoke<DayView[]>('history_search', { query });
export const historyCopy = (id: string) => invoke<void>('history_copy', { id });
export const historyPaste = (id: string) => invoke<PasteAgain>('history_paste', { id });
export const historyClear = () => invoke<void>('history_clear');
/** The recording of the entry `id`, the bytes of an Ogg Opus file (play it as a `Blob` of type
 *  `audio/ogg; codecs=opus`); null when there is none to play — never made, still being encoded, cleared, or past its
 *  period. Any other failure throws its code. */
export async function historyAudio(id: string): Promise<ArrayBuffer | null> {
  try {
    return await invoke<ArrayBuffer>('history_audio', { id });
  } catch (e) {
    if (e === 'notFound') return null;
    throw e;
  }
}

export const keyState = (name: KeyName) => invoke<KeyView>('key_state', { name });
/** The value goes to Windows Credential Manager; the answer is its mask. */
export const keySet = (name: KeyName, value: string) => invoke<KeyView>('key_set', { name, value });
export const keyCheck = (name: KeyName) => invoke<KeyCheck>('key_check', { name });

export const micList = () => invoke<MicList>('mic_list');
/** `device` null — the Windows microphone. The preview is this window's: its level comes here, and only this window's
 *  "off" or close ends it. */
export const micPreview = (on: boolean, device: string | null = null) => invoke<void>('mic_preview', { on, device });
/** Whether Windows lets CUE use the microphone (its privacy settings), read without opening one. */
export const micAccess = () => invoke<MicAccess>('mic_access');
/** A microphone's level, 0..1, while this window's preview runs. */
export async function onMicLevel(cb: (level: number) => void): Promise<void> {
  await getCurrentWebviewWindow().listen<number>('cue://mic-level', (e) => cb(e.payload));
}
/** This window's preview failed: the microphone did not open (Windows may keep apps off it), or it stopped. A level of
 *  0 comes just before. */
export async function onMicFailed(cb: () => void): Promise<void> {
  await getCurrentWebviewWindow().listen<null>('cue://mic-failed', () => cb());
}

/** The speech model on this computer (`cue://model`): downloading with its bytes, loading, ready, failed (`done` bytes
 *  are kept for Retry), or a processor it cannot run on. */
export type ModelState =
  | { state: 'missing' }
  | { state: 'downloading'; done: number; total: number }
  | { state: 'loading' }
  | { state: 'ready'; total: number }
  | { state: 'failed'; done: number; total: number }
  | { state: 'unsupported' };
export const modelGet = () => invoke<ModelState>('model_get');
/** Retry the model's download from where it stopped; the new state comes as `cue://model`. */
export const modelRetry = () => invoke<void>('model_retry');
export async function onModel(cb: (s: ModelState) => void): Promise<void> {
  await getCurrentWebviewWindow().listen<ModelState>('cue://model', (e) => cb(e.payload));
}

/** What the core is doing, as every window hears it: `start` and `listen` — a recording is on. */
export type CorePhase = 'idle' | 'start' | 'listen' | 'think' | 'done' | 'empty' | 'cancel' | 'error' | 'answer';
/** The core's state (`cue://state`, the one the Drop draws): its phase, and whether CUE is paused. No text in it. */
export interface CoreState {
  phase: CorePhase;
  paused: boolean;
}
export async function onCoreState(cb: (s: CoreState) => void): Promise<void> {
  await getCurrentWebviewWindow().listen<CoreState>('cue://state', (e) => cb(e.payload));
}

/** The welcome window's step: on the first dictation's (`dictation`) the hotkey works; on any other — and from the
 *  window's creation until this says otherwise — it starts nothing. Call it on every step change and on load. */
export const welcomeStep = (dictation: boolean) => invoke<void>('welcome_step', { dictation });
/** "Done": the welcome window closes. */
export const welcomeClose = () => invoke<void>('welcome_close');
/** Opens where to get a key, or the Windows microphone settings; CUE holds the addresses. */
export const openLink = (target: LinkTarget) => invoke<void>('open_link', { target });
