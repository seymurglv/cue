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
 *  `YYYY-MM-DDTHH:MM:SS.mmm`. */
export interface EntryView {
  id: string;
  time: string;
  mode: Mode;
  app: string;
  text: string;
  question: string | null;
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
export type UiLanguage = 'auto' | 'en' | 'ru';
/** The settings as the core holds them. `historyKeep` is null while the period is unknown (nothing is written to the
 *  history or deleted from it until the user picks one); `micDevice` null — the Windows microphone; `lang` — the
 *  interface language now; `languages` — the translation targets, code and English name; `saved` — changes reach
 *  settings.json (it was read at startup). */
export interface SettingsView {
  historyKeep: Keep | null;
  micDevice: string | null;
  translateTarget: string;
  uiLanguage: UiLanguage;
  lang: string;
  languages: [string, string][];
  saved: boolean;
}
export type SettingChange =
  | { kind: 'historyKeep'; value: Keep }
  | { kind: 'micDevice'; value: string | null }
  | { kind: 'translateTarget'; value: string }
  | { kind: 'uiLanguage'; value: UiLanguage };

export type KeyName = 'soniox' | 'anthropic';
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
export type LinkTarget = 'soniox' | 'anthropic' | 'micPrivacy';

export const settingsGet = () => invoke<SettingsView>('settings_get');
export const settingsSet = (change: SettingChange) => invoke<SettingsView>('settings_set', { change });
export const autostartGet = () => invoke<boolean>('autostart_get');
export const autostartSet = (on: boolean) => invoke<boolean>('autostart_set', { on });

/** New entries go to `onEntry`. Call it before the first page: an entry that comes meanwhile is in both (same id). */
export function historySubscribe(onEntry: (e: EntryView) => void): Promise<void> {
  const channel = new Channel<EntryView>(onEntry);
  return invoke<void>('history_subscribe', { channel });
}
/** null — today and yesterday; a date — the days with entries before it. */
export const historyPage = (before: string | null) => invoke<PageView>('history_page', { before });
export const historySearch = (query: string) => invoke<DayView[]>('history_search', { query });
export const historyCopy = (id: string) => invoke<void>('history_copy', { id });
export const historyPaste = (id: string) => invoke<PasteAgain>('history_paste', { id });
export const historyClear = () => invoke<void>('history_clear');

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

/** The welcome window's step: on the first dictation's (`dictation`) the hotkey works; on any other — and from the
 *  window's creation until this says otherwise — it starts nothing. Call it on every step change and on load. */
export const welcomeStep = (dictation: boolean) => invoke<void>('welcome_step', { dictation });
/** "Done": the welcome window closes. */
export const welcomeClose = () => invoke<void>('welcome_close');
/** Opens where to get a key, or the Windows microphone settings; CUE holds the addresses. */
export const openLink = (target: LinkTarget) => invoke<void>('open_link', { target });
