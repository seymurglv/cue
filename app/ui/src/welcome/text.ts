// The welcome window's texts in English and Russian, by the core's language (SettingsView.lang). The key and
// microphone rows bring their own texts (window/text.ts), as in the settings tray.
import { fill, type Lang } from '../window/text';
import type { Step } from './steps';

export interface WelcomeText {
  /** The steps' names: the dots' tooltips, and the headings of Keys, Microphone and First dictation. */
  steps: Record<Step, string>;
  back: string;
  start: string;
  next: string;
  finish: string;
  helloTitle: string;
  helloText: string;
  keysText: string;
  /** Before the two links to the consoles. */
  keysWhere: string;
  price: string;
  privacy: string;
  micText: string;
  /** The main button while the microphone shows no trouble. */
  micWorks: string;
  /** Windows denies the microphone, it failed to open, or there is none. */
  micBlocked: string;
  micSettings: string;
  tryText: string;
  trySay: string;
  tryPhrase: string;
  tryField: string;
  tryWaiting: string;
  /** Done's heading: no word counted, one word, and more ({NAME} — the count) by the count's plural form. */
  ready: string;
  firstWord: string;
  firstWords: Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
  doneText: string;
  doneTranslate: string;
  doneAsk: string;
  doneWhere: string;
}

export const en: WelcomeText = {
  steps: { hello: 'Hello', keys: 'Keys', mic: 'Microphone', try: 'First dictation', done: 'Done' },
  back: 'Back',
  start: 'Start',
  next: 'Next',
  finish: 'Done',
  helloTitle: 'Speak — CUE writes',
  helloText: 'Press the keys and speak. The text appears where the caret is: in an email, a chat, a document.',
  keysText: 'CUE works with your own Soniox and Anthropic accounts. Paste a key from each and press Check.',
  keysWhere: 'Where to get them:',
  price: 'Roughly $4–5 a month for an hour of speech a day.',
  privacy: 'Sound goes to Soniox, text to Anthropic; the history stays on this computer only.',
  micText: 'CUE listens only while it records — while the red lamp is on.',
  micWorks: 'It works',
  micBlocked: 'Windows keeps the microphone from CUE',
  micSettings: 'Open Windows settings',
  tryText: 'Press the keys, say the phrase, and press them again.',
  trySay: 'Say:',
  tryPhrase: '“Hello, this is my first dictation”',
  tryField: 'The text will appear here',
  tryWaiting: 'Waiting for dictation',
  ready: 'All set!',
  firstWord: 'First word!',
  firstWords: { other: 'First {NAME} words!' },
  doneText: 'Keep going: emails, chats, notes. Two more modes, and where to find CUE:',
  doneTranslate: 'Translation',
  doneAsk: 'Ask',
  doneWhere: 'CUE lives by the clock: double-click its icon for the window, right-click for the menu',
};

export const ru: WelcomeText = {
  steps: { hello: 'Привет', keys: 'Ключи', mic: 'Микрофон', try: 'Первая диктовка', done: 'Готово' },
  back: 'Назад',
  start: 'Начать',
  next: 'Дальше',
  finish: 'Готово',
  helloTitle: 'Говорите — CUE напишет',
  helloText: 'Нажмите клавиши и говорите. Текст появится там, где стоит курсор, — в почте, чате, документе.',
  keysText: 'CUE работает с вашими аккаунтами Soniox и Anthropic. Вставьте ключ каждого и нажмите «Проверить».',
  keysWhere: 'Где их взять:',
  price: 'Примерно $4–5 в месяц за час речи в день.',
  privacy: 'Звук уходит в Soniox, текст — в Anthropic; история хранится только на этом компьютере.',
  micText: 'CUE слушает только во время записи — пока горит красная лампа.',
  micWorks: 'Работает',
  micBlocked: 'Windows не даёт CUE микрофон',
  micSettings: 'Открыть настройки Windows',
  tryText: 'Нажмите клавиши, скажите фразу и нажмите ещё раз.',
  trySay: 'Скажите:',
  tryPhrase: '«Привет, это моя первая диктовка»',
  tryField: 'Текст появится здесь',
  tryWaiting: 'Ждём диктовку',
  ready: 'Всё готово!',
  firstWord: 'Первое слово!',
  firstWords: { one: 'Первые {NAME} слово!', few: 'Первые {NAME} слова!', many: 'Первые {NAME} слов!', other: 'Первые {NAME} слова!' },
  doneText: 'Так и дальше: письма, чаты, заметки. Ещё два режима и где найти CUE:',
  doneTranslate: 'Перевод',
  doneAsk: 'Спросить',
  doneWhere: 'CUE живёт у часов: двойной щелчок по значку — окно, правая кнопка — меню',
};

export const WELCOME_TEXTS: Record<Lang, WelcomeText> = { en, ru };

/** Done's heading for `n` words. */
export function doneTitle(t: WelcomeText, n: number, lang: Lang): string {
  if (n <= 0) return t.ready;
  if (n === 1) return t.firstWord;
  return fill(t.firstWords[new Intl.PluralRules(lang).select(n)] ?? t.firstWords.other, String(n));
}
