// The main window's texts in English and Russian. The language is the core's (SettingsView.lang): the window has no
// language set before its script runs, so the page asks for the settings before it draws anything.
import type { AudioKeep, Keep, Mode, UiLanguage } from './api';

export type Lang = 'en' | 'ru';

/** A word's forms by count: English one and many; Russian one (1, 21), few (2–4, 22–24) and many. */
export type Forms = readonly [string, string] | readonly [string, string, string];

export interface WinText {
  history: string;
  /** The Dictionary section: its heading (also the header's link), the line under it, the add button, the field's
   *  placeholder, a word's remove button. */
  dictionary: string;
  dictHint: string;
  /** The hint's second half, after the green book. */
  dictHintLearned: string;
  /** A learned chip's tooltip; {HEARD} is how it was heard. */
  dictHeard: string;
  /** "Learn from me": the switch in the Habits heading. */
  learn: string;
  dictAdd: string;
  dictNew: string;
  dictRemove: string;
  /** A word's remove button for a screen reader ({NAME} — the word). */
  dictRemoveWord: string;
  /** The Habits section (docs/specs/2026-11-04-habits.md): its heading (also the header's link), the hint, the line
   *  when no cleanup is connected, the empty list, "Forget all" and its second press, a row's forget button (its name
   *  for a screen reader: {NAME} — the rule), its switch's tooltips, where it holds ({NAME} — the app), how many times,
   *  and the History's chips for a screen reader. */
  habits: string;
  habitsHint: string;
  habitsNoCleanup: string;
  habitsEmpty: string;
  habitsForgetAll: string;
  habitsForgetAgain: string;
  habitForget: string;
  habitForgetName: string;
  habitOff: string;
  habitOn: string;
  habitEverywhere: string;
  habitIn: string;
  habitTimes: Forms;
  habitsApplied: string;
  /** The page's containers for a screen reader: the scrolling page, the header's links, the history's list. */
  main: string;
  sections: string;
  entries: string;
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
  /** An entry and its buttons for a screen reader ({TIME}, {TEXT} — the start of the entry, `excerpt`). */
  entryName: string;
  copyEntry: string;
  pasteEntry: string;
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
  mic: string;
  translate: string;
  general: string;
  /** Speech recognition (docs/specs/2026-11-02-settings-tray.md): its title; on the first page its value by the model's
   *  state ({N} — percent); on its page the model's line by state ({DONE}, {TOTAL} —
   *  MB), Retry, and the note on where sound goes. */
  model: string;
  modelName: string;
  modelValue: { ready: string; loading: string; downloading: string; failed: string; unsupported: string; missing: string };
  modelLine: { ready: string; downloading: string; failed: string; loading: string; unsupported: string; missing: string };
  modelRetry: string;
  modelNote: string;
  /** The Model page (docs/specs/2026-11-02-model-picker.md): its title, the search's hint and its clearing cross, the two groups, a row's line
   *  ({N} languages, {SIZE} in MB), its states ({DONE}, {TOTAL} — MB; {NEED}, {FREE} — GB), Delete, a found
   *  repository's line ({N} {W} downloads) and why one cannot run, and what the search says instead of rows. */
  models: {
    title: string;
    hint: string;
    clear: string;
    list: string;
    found: string;
    languages: Forms;
    englishOnly: string;
    mb: string;
    preparing: string;
    loading: string;
    notRunnable: string;
    noSpace: string;
    remove: string;
    downloads: Forms;
    reasons: { architecture: string; noInt8: string; layout: string };
    offline: string;
    none: string;
  };
  /** Where the cleanup runs: the row's name, its hint by the half chosen, the halves. */
  runsOn: string;
  runsHint: { local: string; cloud: string };
  runs: { local: string; cloud: string };
  /** The note under the Cleanup page on this computer. */
  cleanupNoteLocal: string;
  /** The first page's Cleanup value on this computer while its model is not ready ({PCT} — percent). */
  cleanupValue: { notDownloaded: string; downloading: string };
  /** The local models' list: the recommended one's line, a size ({SIZE} — gigabytes), why a found one cannot run, an
   *  empty search. */
  /** The local models: "Recommended", a size, the reasons a found one cannot run, an empty search, and the heading of
   *  the models downloaded here, shown to delete while the cleanup runs in the cloud. */
  llms: { recommended: string; gb: string; downloaded: string; reasons: { severalFiles: string; noChatTemplate: string; tooLarge: string; architecture: string; noQuant: string }; none: string };
  /** Cleanup: its title and the note on where text goes ({NAME} — where it runs). */
  cleanup: string;
  cleanupNote: string;
  /** Languages: its title and its value ({UI} — the interface language, {TO} — the translation's). */
  languages: string;
  languagesValue: string;
  /** General's value: start at sign-in on, off. */
  generalOn: string;
  generalOff: string;
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
  dictionary: 'Dictionary',
  dictHint: 'Words CUE must write right.',
  dictHintLearned: '— from your corrections: CUE noticed the change and offered the word.',
  dictHeard: 'sounds like «{HEARD}»',
  learn: 'Learn from me',
  dictAdd: 'Word',
  dictNew: 'New word, Enter',
  dictRemove: 'Remove',
  dictRemoveWord: 'Delete the word {NAME}',
  habits: 'Habits',
  habitsHint: 'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
  habitsNoCleanup: 'Habits work when the cleanup is on.',
  habitsEmpty: 'Nothing yet. CUE will notice when you correct the same thing a few times.',
  habitsForgetAll: 'Forget all',
  habitsForgetAgain: 'Press again',
  habitForget: 'Forget',
  habitForgetName: 'Forget: {NAME}',
  habitOff: 'Turn off',
  habitOn: 'Turn on',
  habitEverywhere: 'everywhere',
  habitIn: 'in {NAME}',
  habitTimes: ['time', 'times'],
  habitsApplied: 'Habits applied',
  main: 'Main window',
  sections: 'Sections',
  entries: 'History entries',
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
  entryName: '{TIME}, {TEXT}',
  copyEntry: 'Copy: {TEXT}',
  pasteEntry: 'Paste again: {TEXT}',
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
  model: 'Speech recognition',
  modelName: 'Parakeet Ultra',
  modelValue: {
    ready: 'Ready',
    loading: 'Loading',
    downloading: '{N} %',
    failed: 'Did not download',
    unsupported: 'Not supported',
    missing: 'Not downloaded',
  },
  modelLine: {
    ready: 'Ready · {TOTAL} MB on this computer',
    downloading: 'Downloading: {DONE} of {TOTAL} MB',
    failed: 'Did not download · {DONE} of {TOTAL} MB',
    loading: 'Loading',
    unsupported: "This processor can't run it",
    missing: 'Not downloaded',
  },
  modelRetry: 'Retry',
  modelNote: 'Sound stays on this computer.',
  models: {
    title: 'Model',
    hint: 'Search Hugging Face',
    clear: 'Clear',
    list: 'Models',
    found: 'On Hugging Face',
    languages: ['language', 'languages'],
    englishOnly: 'English only',
    mb: '{SIZE} MB',
    preparing: 'Getting ready',
    loading: 'Loading',
    notRunnable: "Doesn't run in CUE",
    noSpace: 'Needs {NEED} GB, {FREE} free',
    remove: 'Delete',
    downloads: ['download', 'downloads'],
    reasons: { architecture: 'Another architecture', noInt8: 'No int8', layout: 'Another file layout' },
    offline: 'No connection to Hugging Face',
    none: 'No Parakeet models for this search',
  },
  runsOn: 'Runs on',
  runsHint: { local: 'Free, private', cloud: 'Your AI' },
  runs: { local: 'This computer', cloud: 'Cloud' },
  cleanupNoteLocal: 'Free. The text never leaves this computer; the model runs on its processor.',
  cleanupValue: { notDownloaded: 'Model not downloaded', downloading: 'Downloading {PCT} %' },
  llms: {
    recommended: 'Recommended · Russian and English',
    gb: '{SIZE} GB',
    downloaded: 'Models on this computer',
    reasons: { severalFiles: 'several files', noChatTemplate: 'no chat template', tooLarge: 'too large', architecture: 'another architecture', noQuant: 'no quantization CUE takes' },
    none: 'No GGUF models for this search',
  },
  cleanup: 'Cleanup',
  cleanupNote: 'Text goes to {NAME} for the cleanup; the history stays on this computer only.',
  languages: 'Languages',
  languagesValue: '{UI} · into {TO}',
  generalOn: 'Starts at sign-in',
  generalOff: 'Manually',
  mic: 'Microphone',
  translate: 'Translation',
  general: 'General',
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
  dictionary: 'Словарь',
  dictHint: 'Слова, которые CUE должен писать правильно.',
  dictHintLearned: '— из ваших правок: CUE заметил замену и предложил слово.',
  dictHeard: 'звучит как «{HEARD}»',
  learn: 'Учиться у меня',
  dictAdd: 'Слово',
  dictNew: 'Новое слово, Enter',
  dictRemove: 'Удалить',
  dictRemoveWord: 'Удалить слово {NAME}',
  habits: 'Привычки',
  habitsHint: 'Как вы правите текст после диктовки. Когда правка повторяется, CUE запоминает её и дальше пишет так сам. Хранится только на этом компьютере.',
  habitsNoCleanup: 'Привычки работают, когда включена очистка.',
  habitsEmpty: 'Пока ничего. CUE заметит, если вы несколько раз поправите одно и то же.',
  habitsForgetAll: 'Забыть всё',
  habitsForgetAgain: 'Нажмите ещё раз',
  habitForget: 'Забыть',
  habitForgetName: 'Забыть: {NAME}',
  habitOff: 'Выключить',
  habitOn: 'Включить',
  habitEverywhere: 'везде',
  habitIn: 'в {NAME}',
  habitTimes: ['раз', 'раза', 'раз'],
  habitsApplied: 'Применённые привычки',
  main: 'Главное окно',
  sections: 'Разделы',
  entries: 'Записи истории',
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
  entryName: '{TIME}, {TEXT}',
  copyEntry: 'Копировать: {TEXT}',
  pasteEntry: 'Вставить снова: {TEXT}',
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
  model: 'Распознавание',
  modelName: 'Parakeet Ultra',
  modelValue: {
    ready: 'Готово',
    loading: 'Загружается',
    downloading: '{N} %',
    failed: 'Не скачалась',
    unsupported: 'Не поддерживается',
    missing: 'Не скачана',
  },
  modelLine: {
    ready: 'Готово · {TOTAL} МБ на этом компьютере',
    downloading: 'Качается: {DONE} из {TOTAL} МБ',
    failed: 'Не скачалась · {DONE} из {TOTAL} МБ',
    loading: 'Загружается',
    unsupported: 'Этот процессор её не запустит',
    missing: 'Не скачана',
  },
  modelRetry: 'Повторить',
  modelNote: 'Звук остаётся на этом компьютере.',
  models: {
    title: 'Модель',
    hint: 'Найти на Hugging Face',
    clear: 'Очистить',
    list: 'Модели',
    found: 'На Hugging Face',
    languages: ['язык', 'языка', 'языков'],
    englishOnly: 'Только английский',
    mb: '{SIZE} МБ',
    preparing: 'Готовится',
    loading: 'Загружается',
    notRunnable: 'Не запускается в CUE',
    noSpace: 'Нужно {NEED} ГБ, свободно {FREE}',
    remove: 'Удалить',
    downloads: ['скачивание', 'скачивания', 'скачиваний'],
    reasons: { architecture: 'Другая архитектура', noInt8: 'Нет int8', layout: 'Другая раскладка файлов' },
    offline: 'Нет связи с Hugging Face',
    none: 'Нет моделей Parakeet по запросу',
  },
  runsOn: 'Где работает',
  runsHint: { local: 'Бесплатно, приватно', cloud: 'Ваш AI' },
  runs: { local: 'Этот компьютер', cloud: 'Облако' },
  cleanupNoteLocal: 'Бесплатно. Текст не покидает этот компьютер; модель работает на его процессоре.',
  cleanupValue: { notDownloaded: 'Модель не скачана', downloading: 'Качается {PCT} %' },
  llms: {
    recommended: 'Рекомендуем · русский и английский',
    gb: '{SIZE} ГБ',
    downloaded: 'Модели на этом компьютере',
    reasons: { severalFiles: 'несколько файлов', noChatTemplate: 'нет шаблона чата', tooLarge: 'слишком большая', architecture: 'другая архитектура', noQuant: 'нет подходящего квантования' },
    none: 'Нет GGUF-моделей по запросу',
  },
  cleanup: 'Чистка текста',
  cleanupNote: 'Текст уходит в {NAME} для чистки; история — только на этом компьютере.',
  languages: 'Языки',
  languagesValue: '{UI} · на {TO}',
  generalOn: 'Запуск при входе',
  generalOff: 'Вручную',
  mic: 'Микрофон',
  translate: 'Перевод',
  general: 'Общие',
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

/** The characters of an entry a screen reader names it by. */
export const EXCERPT_CHARS = 40;

/** The start of `text` on one line: its first EXCERPT_CHARS characters, "…" after them when there is more. */
export function excerpt(text: string): string {
  const chars = [...text.replace(/\s+/g, ' ').trim()];
  return chars.length > EXCERPT_CHARS ? `${chars.slice(0, EXCERPT_CHARS).join('').trimEnd()}…` : chars.join('');
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
