// The main window's texts in English and Russian. The language is the core's (SettingsView.lang): the window has no
// language set before its script runs, so the page asks for the settings before it draws anything.
import type { AudioKeep, Keep, KeyCheck, Mode, UiLanguage } from './api';

export type Lang = 'en' | 'ru';

/** A key's state as its row shows it: the result of a check, saved but not checked, being checked. */
export type KeyState = KeyCheck | 'saved' | 'checking';

/** A word's forms by count: English one and many; Russian one (1, 21), few (2–4, 22–24) and many. */
export type Forms = readonly [string, string] | readonly [string, string, string];

export interface WinText {
  history: string;
  /** Today: the header's link, and the sentence and the line of the section ({N} is a number, {W} its word). */
  todayNav: string;
  todayWords: string;
  todaySaved: string;
  todayWpm: string;
  todayStreak: string;
  todayTotal: string;
  /** The milestone pill: {N} words, {P} pages of a book. */
  milestone: string;
  words: Forms;
  minutes: Forms;
  days: Forms;
  pages: Forms;
  search: string;
  searchHint: string;
  /** The search's own button once it is open: it closes it. */
  searchClose: string;
  today: string;
  yesterday: string;
  earlier: string;
  /** Over the days before today, each folded into one line. */
  earlierDays: string;
  /** Under today's latest entries: the rest of today ({N} entries). */
  moreToday: string;
  /** A folded day's line for a screen reader ({DAY}, {N}). */
  dayEntries: string;
  /** The tray's History page: how long the history is kept (also the History row's value on the tray's first page). */
  keepHistory: string;
  keepHistoryHint: string;
  keeps: Record<Keep, string>;
  /** The period is unknown: nothing is chosen. */
  keepUnset: string;
  /** Next to the periods: a shorter period deletes older days at once. */
  keepNote: string;
  /** The History page's last row: its label, the line under it, its button and the button waiting for a second press. */
  clear: string;
  clearHint: string;
  clearButton: string;
  clearAgain: string;
  empty: string;
  nothing: string;
  copy: string;
  pasteAgain: string;
  modes: Record<Mode, string>;
  /** A stack's chip, "Telegram 5": its name for a screen reader and its tooltip ({APP}, {N}). */
  stack: string;
  /** Over an opened entry: the text as it was heard (a dictation, its cleanup marked), the speech before it was
   *  translated, the question as it was heard. */
  saidAs: Record<Mode, string>;
  /** After that heading: how it was said was not kept (an entry from before CUE kept it); CUE wrote the dictation just
   *  as it was heard. */
  notKept: string;
  unchanged: string;
  play: string;
  pause: string;
  settings: string;
  close: string;
  back: string;
  /** settings.json was not read at startup. */
  unsaved: string;
  keys: string;
  mic: string;
  translate: string;
  general: string;
  keyHint: string;
  check: string;
  keyStates: Record<KeyState, string>;
  /** The keys row with none saved, and with one missing ({NAME}). */
  noKeys: string;
  noKey: string;
  micLabel: string;
  windowsMic: string;
  /** In the menu: the Windows microphone and its name. */
  windowsMicNamed: string;
  notConnected: string;
  /** Under the microphone row when the pinned one is not there. */
  notConnectedHint: string;
  micTest: string;
  micTestHint: string;
  translateTo: string;
  autostart: string;
  uiLanguage: string;
  uiLanguages: Record<UiLanguage, string>;
  /** The tray's History page: how long recordings are kept, never longer than the history. */
  keepAudio: string;
  keepAudioHint: string;
  audioKeeps: Record<AudioKeep, string>;
  audioNote: string;
}

export const en: WinText = {
  history: 'History',
  todayNav: 'Today',
  todayWords: 'Today — {N} {W}.',
  todaySaved: "That's {N} {W} without the keyboard.",
  todayWpm: '{N} {W} a minute',
  todayStreak: '{N} {W} in a row',
  todayTotal: '{N} {W} in all',
  milestone: '{N} {W} — about {P} {PW} of a book',
  words: ['word', 'words'],
  minutes: ['minute', 'minutes'],
  days: ['day', 'days'],
  pages: ['page', 'pages'],
  search: 'Search',
  searchHint: 'Search the history',
  searchClose: 'Close search',
  today: 'Today',
  yesterday: 'Yesterday',
  earlier: 'Show earlier',
  earlierDays: 'Earlier',
  moreToday: '{N} more today',
  dayEntries: '{DAY}, {N} entries',
  keepHistory: 'Keep history',
  keepHistoryHint: 'Older entries go by themselves',
  keeps: { forever: 'Forever', year: 'A year', month: 'A month', week: 'A week', off: "Don't keep" },
  keepUnset: 'Not set',
  keepNote: 'A shorter period deletes older days at once.',
  clear: 'Clear history',
  clearHint: 'Takes a second press',
  clearButton: 'Clear',
  clearAgain: 'Press again',
  empty: 'Nothing yet. What CUE writes for you will show here.',
  nothing: 'Nothing found',
  copy: 'Copy',
  pasteAgain: 'Paste again',
  modes: { dictation: 'Dictation', translation: 'Translation', ask: 'Ask' },
  stack: '{APP}, {N} entries',
  saidAs: { dictation: 'How you said it', translation: 'What you said', ask: 'Your question' },
  notKept: 'not kept',
  unchanged: 'CUE changed nothing',
  play: 'Play',
  pause: 'Pause',
  settings: 'Settings',
  close: 'Close · Esc',
  back: 'Back',
  unsaved: "settings.json wasn't read — changes last until CUE quits",
  keys: 'Keys',
  mic: 'Microphone',
  translate: 'Translation',
  general: 'General',
  keyHint: 'Paste the key',
  check: 'Check',
  keyStates: {
    works: 'Works',
    badKey: 'Key error',
    unchecked: "Couldn't check",
    missing: 'No key',
    store: "Couldn't check",
    saved: 'Saved',
    checking: 'Checking…',
  },
  noKeys: 'No keys',
  noKey: 'No {NAME} key',
  micLabel: 'Microphone',
  windowsMic: 'As in Windows',
  windowsMicNamed: 'As in Windows ({NAME})',
  notConnected: 'not connected',
  notConnectedHint: 'Not connected — recording from the Windows microphone',
  micTest: 'Test',
  micTestHint: 'Say something',
  translateTo: 'Translate into',
  autostart: 'Start at sign-in',
  uiLanguage: 'Interface language',
  uiLanguages: { auto: 'Auto', en: 'English', ru: 'Русский' },
  keepAudio: 'Keep recordings',
  keepAudioHint: 'Never longer than the history',
  audioKeeps: { day: 'A day', week: 'A week', month: 'A month', off: "Don't keep" },
  audioNote: 'A shorter period deletes older recordings at once.',
};

export const ru: WinText = {
  history: 'История',
  todayNav: 'Сегодня',
  todayWords: 'Сегодня — {N} {W}.',
  todaySaved: 'Это {N} {W} без клавиатуры.',
  todayWpm: '{N} {W} в минуту',
  todayStreak: '{N} {W} подряд',
  todayTotal: '{N} {W} за всё время',
  milestone: '{N} {W} — около {P} {PW} книги',
  words: ['слово', 'слова', 'слов'],
  minutes: ['минута', 'минуты', 'минут'],
  days: ['день', 'дня', 'дней'],
  pages: ['страница', 'страницы', 'страниц'],
  search: 'Поиск',
  searchHint: 'Найти в истории',
  searchClose: 'Закрыть поиск',
  today: 'Сегодня',
  yesterday: 'Вчера',
  earlier: 'Показать раньше',
  earlierDays: 'Раньше',
  moreToday: 'Ещё за сегодня: {N}',
  dayEntries: '{DAY}, записей: {N}',
  keepHistory: 'Хранить историю',
  keepHistoryHint: 'Старое удаляется само',
  keeps: { forever: 'Всегда', year: 'Год', month: 'Месяц', week: 'Неделю', off: 'Не хранить' },
  keepUnset: 'Не задано',
  keepNote: 'Более короткий срок сразу удаляет старые дни.',
  clear: 'Очистить историю',
  clearHint: 'Со второго нажатия',
  clearButton: 'Очистить',
  clearAgain: 'Нажмите ещё раз',
  empty: 'Пока пусто. Здесь появится всё, что CUE напишет за вас.',
  nothing: 'Ничего не найдено',
  copy: 'Копировать',
  pasteAgain: 'Вставить снова',
  modes: { dictation: 'Диктовка', translation: 'Перевод', ask: 'Спросить' },
  stack: '{APP}, записей: {N}',
  saidAs: { dictation: 'Как вы сказали', translation: 'Что вы сказали', ask: 'Ваш вопрос' },
  notKept: 'не сохранилось',
  unchanged: 'CUE ничего не менял',
  play: 'Слушать',
  pause: 'Пауза',
  settings: 'Настройки',
  close: 'Закрыть · Esc',
  back: 'Назад',
  unsaved: 'settings.json не прочитан — изменения действуют до выхода',
  keys: 'Ключи',
  mic: 'Микрофон',
  translate: 'Перевод',
  general: 'Общие',
  keyHint: 'Вставьте ключ',
  check: 'Проверить',
  keyStates: {
    works: 'Работает',
    badKey: 'Ошибка ключа',
    unchecked: 'Не удалось проверить',
    missing: 'Нет ключа',
    store: 'Не удалось проверить',
    saved: 'Сохранён',
    checking: 'Проверяю…',
  },
  noKeys: 'Нет ключей',
  noKey: 'Нет ключа {NAME}',
  micLabel: 'Микрофон',
  windowsMic: 'Как в Windows',
  windowsMicNamed: 'Как в Windows ({NAME})',
  notConnected: 'не подключён',
  notConnectedHint: 'Не подключён — пишу с микрофона Windows',
  micTest: 'Проверка',
  micTestHint: 'Скажите что-нибудь',
  translateTo: 'Переводить на',
  autostart: 'Запускать при входе',
  uiLanguage: 'Язык интерфейса',
  uiLanguages: { auto: 'Авто', en: 'English', ru: 'Русский' },
  keepAudio: 'Хранить записи',
  keepAudioHint: 'Не дольше самой истории',
  audioKeeps: { day: 'День', week: 'Неделю', month: 'Месяц', off: 'Не хранить' },
  audioNote: 'Более короткий срок сразу удаляет старые записи.',
};

export const TEXTS: Record<Lang, WinText> = { en, ru };

/** The core's language; anything else means English. */
export function pickLang(v: unknown): Lang {
  return v === 'ru' ? 'ru' : 'en';
}

/** `{NAME}` replaced by a function, so a "$&" in a device name stays as it is. */
export function fill(template: string, name: string): string {
  return template.replace('{NAME}', () => name);
}

/** The local date `YYYY-MM-DD` of `d`. */
export function isoDate(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** The local date `days` before `date` (`YYYY-MM-DD`). */
export function daysBefore(date: string, days: number): string {
  const [y, m, d] = date.split('-').map(Number);
  return isoDate(new Date(y, m - 1, d - days));
}

const upperFirst = (s: string, lang: Lang) => s.charAt(0).toLocaleUpperCase(lang) + s.slice(1);

/** A day's heading: "Today", "Yesterday", then the weekday with the day and the month (the year too when it is not
 *  this one), in the window's language. */
export function dayLabel(date: string, today: string, t: WinText, lang: Lang): string {
  if (date === today) return t.today;
  if (date === daysBefore(today, 1)) return t.yesterday;
  const [y, m, d] = date.split('-').map(Number);
  if (!y || !m || !d) return date;
  const opts: Intl.DateTimeFormatOptions = { weekday: 'long', day: 'numeric', month: 'long' };
  if (String(y) !== today.slice(0, 4)) opts.year = 'numeric';
  return upperFirst(new Intl.DateTimeFormat(lang, opts).format(new Date(y, m - 1, d)), lang);
}

/** A translation language's name in the window's language, or its English name when the system has none. */
export function languageName(code: string, english: string, lang: Lang): string {
  try {
    const name = new Intl.DisplayNames([lang], { type: 'language', fallback: 'none' }).of(code);
    if (name && name !== code) return upperFirst(name, lang);
  } catch {
    // No such data: the English name.
  }
  return english;
}

/** The form of a word for `n`: English singular for 1; Russian by the last digits (21 слово, 22 слова, 25 слов, 11 слов). */
export function plural(n: number, forms: Forms): string {
  if (forms.length === 2) return n === 1 ? forms[0] : forms[1];
  const d = Math.abs(n) % 100;
  const u = d % 10;
  if (u === 1 && d !== 11) return forms[0];
  if (u >= 2 && u <= 4 && (d < 12 || d > 14)) return forms[1];
  return forms[2];
}

/** A number as the language writes it: "2,340", "2 340". */
export function formatNumber(n: number, lang: Lang): string {
  return new Intl.NumberFormat(lang === 'ru' ? 'ru-RU' : 'en-US').format(n);
}
