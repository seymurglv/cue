// Interface texts in English (default) and Russian. The core picks the language once from the Windows UI language
// and sets it in the Drop window before any script runs (window.__CUE_LANG__); the tray menu uses the same value.
export type Lang = 'en' | 'ru';
export type TrayKind = 'noField' | 'micSilent' | 'holdHint' | 'lastMinute' | 'micMissing' | 'modelReady' | 'modelSwitched' | 'newHabit' | 'habitChanged';
export type ErrorCause =
  | 'model' | 'cpu' | 'stt'
  | 'agentSignIn' | 'agentMissing' | 'agentSilent'
  | 'needAiTranslate' | 'needAiAsk' | 'needTranslatingModel' | 'needAnsweringModel'
  | 'translate';

export const KEY_NAME = 'Ctrl + Win';

/** Look and timing of a tray card: the same in every language. */
export interface TrayLook {
  icon: 'copy' | 'mic' | 'key' | 'timer' | 'model' | 'habit';
  tone: '' | 'warn' | 'ask' | 'ok';
  /** Icon of the action button; null — the card has no button. */
  act: 'copy' | 'gear' | 'ok' | 'undo' | null;
  /** Closes by itself after this many ms; 0 — stays until closed. */
  ttl: number;
}

export const TRAYS: Record<TrayKind, TrayLook> = {
  noField: { icon: 'copy', tone: '', act: 'copy', ttl: 9000 },
  micSilent: { icon: 'mic', tone: 'warn', act: 'gear', ttl: 0 },
  holdHint: { icon: 'key', tone: 'ask', act: 'ok', ttl: 7000 },
  lastMinute: { icon: 'timer', tone: 'warn', act: null, ttl: 5000 },
  micMissing: { icon: 'mic', tone: 'warn', act: null, ttl: 6000 },
  modelReady: { icon: 'model', tone: 'ok', act: null, ttl: 5000 },
  modelSwitched: { icon: 'model', tone: 'ok', act: null, ttl: 5000 },
  newHabit: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },
  habitChanged: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },
};

export interface TrayText {
  title: string;
  /** Trusted HTML: may contain <b>; {KEY} becomes <kbd>KEY_NAME</kbd>; {WANTED} and {USED} become the device names,
   *  {NAME} the speech model's, {RULE} the habit's rule, escaped. */
  body: string;
  /** Button label: present exactly when TRAYS[kind].act is not null. */
  act?: string;
  /** Label after the press, then the card closes; without it the card closes at once. */
  done?: string;
  /** In place of {USED} when Windows gave the microphone no name, of {NAME} when the model's name did not come. */
  unnamed?: string;
  /** The body alone, when {RULE} came empty (the card with no rule to say). */
  bare?: string;
}

/** Labels of the Drop's own buttons and parts (drop.js gets them through its api). */
export interface DropText {
  /** Cancel button: tooltip and accessible name. */
  cancel: string;
  cancelName: string;
  restore: string;
  retry: string;
  done: string;
  cancelled: string;
  ask: string;
  /** The dictionary suggestion's buttons: tooltip and accessible name. */
  learnAdd: string;
  learnUndo: string;
}

/** Texts of an "Ask" answer (spec 2 §7). `about` puts in the selection's start at {Q}. */
export interface AnswerText {
  insert: string;
  below: string;
  replace: string;
  copy: string;
  copied: string;
  retry: string;
  followUp: string;
  about: string;
  searching: string;
  reading: string;
  writing: string;
  listening: string;
  /** The follow-up could not be transcribed (decision 7). */
  missed: string;
  refused: string;
  failed: string;
  /** Under an answer given without web search while it is on (the AI cannot search, or did not). */
  noSearch: string;
}

export interface Dict {
  tray: Record<TrayKind, TrayText>;
  /** The Drop's errors; the agent's words hold {NAME}, the agent (`agentUnnamed` when none was heard). */
  error: Record<ErrorCause, string>;
  agentUnnamed: string;
  /** Close button of a tray card: tooltip and accessible name. */
  close: string;
  drop: DropText;
  answer: AnswerText;
}

export const en: Dict = {
  tray: {
    noField: {
      title: 'Nowhere to paste',
      body: "The cursor isn't in a text field. Your text is ready — copy it and paste it yourself.",
      act: 'Copy', done: 'Copied',
    },
    micSilent: {
      title: 'Microphone is silent',
      body: "No sound is coming from the microphone. Check that it's on and that <b>microphone access</b> is allowed.",
      act: 'Settings',
    },
    holdHint: {
      title: 'Press and release',
      body: 'To speak, press {KEY} once — no need to hold it.',
      act: 'Got it',
    },
    lastMinute: {
      title: 'One minute left',
      body: 'At 9:00 the recording stops on its own and the text is pasted as usual.',
    },
    micMissing: {
      title: 'Another microphone',
      body: '<b>{WANTED}</b> not found — recording from <b>{USED}</b>',
      unnamed: 'the Windows microphone',
    },
    modelReady: {
      title: 'Model is ready',
      body: 'Speech is recognized on this computer.',
    },
    modelSwitched: {
      title: 'Switched model',
      body: '<b>{NAME}</b>',
      unnamed: 'The model you chose',
    },
    newHabit: {
      title: 'New habit',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Removed',
      bare: 'CUE writes it this way from now on.',
    },
    habitChanged: {
      title: 'Habit changed',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Restored',
      bare: 'CUE writes it this way from now on.',
    },
  },
  error: {
    model: 'Model did not download',
    cpu: 'Processor not supported',
    stt: "Couldn't recognize",
    agentSignIn: '{NAME}: sign in again',
    agentMissing: '{NAME} not found',
    agentSilent: "{NAME} didn't answer",
    needAiTranslate: 'Translate: choose an AI',
    needAiAsk: 'Ask: choose an AI',
    needTranslatingModel: 'Translate: needs a model that translates',
    needAnsweringModel: 'Ask: needs a model that answers',
    translate: "Couldn't translate",
  },
  agentUnnamed: 'AI',
  close: 'Close',
  drop: {
    cancel: 'Cancel · Esc',
    cancelName: 'Cancel',
    restore: 'Restore',
    retry: 'Retry',
    done: 'Done',
    cancelled: 'Cancelled',
    ask: 'Ask',
    learnAdd: 'Add to dictionary',
    learnUndo: 'Undo',
  },
  answer: {
    insert: 'Insert',
    below: 'Insert below',
    replace: 'Replace',
    copy: 'Copy',
    copied: 'Copied',
    retry: 'Retry',
    followUp: 'follow up',
    about: 'about the selection: “{Q}”',
    searching: 'Searching the web',
    reading: 'Reading sources',
    writing: 'Writing the answer',
    listening: 'Listening for a follow-up…',
    missed: "Couldn't make out the follow-up",
    refused: "Can't answer that",
    failed: "Couldn't get an answer",
    noSearch: 'no web search',
  },
};

export const ru: Dict = {
  tray: {
    noField: {
      title: 'Некуда вставить',
      body: 'Курсор не стоит в поле ввода. Текст готов — скопируйте и вставьте сами.',
      act: 'Скопировать', done: 'Скопировано',
    },
    micSilent: {
      title: 'Микрофон молчит',
      body: 'От микрофона не приходит звук. Проверьте, включён ли он и разрешён ли <b>доступ к микрофону</b>.',
      act: 'Настройки',
    },
    holdHint: {
      title: 'Нажмите и отпустите',
      body: 'Чтобы говорить, нажмите {KEY} один раз — держать не нужно.',
      act: 'Понятно',
    },
    lastMinute: {
      title: 'Осталась минута',
      body: 'На 9:00 запись завершится сама, текст вставится как обычно.',
    },
    micMissing: {
      title: 'Другой микрофон',
      body: '<b>{WANTED}</b> не найден — пишу с <b>{USED}</b>',
      unnamed: 'микрофона Windows',
    },
    modelReady: {
      title: 'Модель готова',
      body: 'Речь распознаётся на этом компьютере.',
    },
    modelSwitched: {
      title: 'Модель переключена',
      body: '<b>{NAME}</b>',
      unnamed: 'Выбранная модель',
    },
    newHabit: {
      title: 'Новая привычка',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Убрано',
      bare: 'Дальше CUE пишет так сам.',
    },
    habitChanged: {
      title: 'Привычка изменилась',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Возвращено',
      bare: 'Дальше CUE пишет так сам.',
    },
  },
  error: {
    model: 'Модель не скачалась',
    cpu: 'Процессор не подходит',
    stt: 'Не распознал',
    agentSignIn: '{NAME}: войдите заново',
    agentMissing: '{NAME} не найден',
    agentSilent: '{NAME} не ответил',
    needAiTranslate: 'Перевод: выберите AI',
    needAiAsk: 'Спросить: выберите AI',
    needTranslatingModel: 'Перевод: нужна модель, которая переводит',
    needAnsweringModel: 'Спросить: нужна модель, которая отвечает',
    translate: 'Не удалось перевести',
  },
  agentUnnamed: 'AI',
  close: 'Закрыть',
  drop: {
    cancel: 'Отменить · Esc',
    cancelName: 'Отменить',
    restore: 'Вернуть',
    retry: 'Повторить',
    done: 'Готово',
    cancelled: 'Отменено',
    ask: 'Спросить',
    learnAdd: 'Добавить в словарь',
    learnUndo: 'Отменить',
  },
  answer: {
    insert: 'Вставить',
    below: 'Вставить ниже',
    replace: 'Заменить',
    copy: 'Копировать',
    copied: 'Скопировано',
    retry: 'Повторить',
    followUp: 'уточнить',
    about: 'по выделенному: «{Q}»',
    searching: 'Ищу в интернете',
    reading: 'Читаю источники',
    writing: 'Пишу ответ',
    listening: 'Слушаю уточнение…',
    missed: 'Не расслышал уточнение',
    refused: 'Не могу ответить на это',
    failed: 'Не удалось получить ответ',
    noSearch: 'без поиска',
  },
};

export const DICTS: Record<Lang, Dict> = { en, ru };

/** The language the core set; anything else (missing, unknown) means English. */
export function pickLang(v: unknown): Lang {
  return v === 'ru' ? 'ru' : 'en';
}
