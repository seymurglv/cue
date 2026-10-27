// Interface texts in English (default) and Russian. The core picks the language once from the Windows UI language
// and sets it in the Drop window before any script runs (window.__CUE_LANG__); the tray menu uses the same value.
export type Lang = 'en' | 'ru';
export type TrayKind = 'noField' | 'micSilent' | 'holdHint' | 'lastMinute';
export type ErrorCause = 'net' | 'sonioxKey' | 'claudeKey';

export const KEY_NAME = 'Ctrl + Win';

/** Look and timing of a tray card: the same in every language. */
export interface TrayLook {
  icon: 'copy' | 'mic' | 'key' | 'timer';
  tone: '' | 'warn' | 'ask' | 'ok';
  /** Icon of the action button; null — the card has no button. */
  act: 'copy' | 'gear' | 'ok' | null;
  /** Closes by itself after this many ms; 0 — stays until closed. */
  ttl: number;
}

export const TRAYS: Record<TrayKind, TrayLook> = {
  noField: { icon: 'copy', tone: '', act: 'copy', ttl: 9000 },
  micSilent: { icon: 'mic', tone: 'warn', act: 'gear', ttl: 0 },
  holdHint: { icon: 'key', tone: 'ask', act: 'ok', ttl: 7000 },
  lastMinute: { icon: 'timer', tone: 'warn', act: null, ttl: 5000 },
};

export interface TrayText {
  title: string;
  body: string; // trusted HTML: may contain <b>; {KEY} becomes <kbd>KEY_NAME</kbd>
  /** Button label: present exactly when TRAYS[kind].act is not null. */
  act?: string;
  /** Label after the press, then the card closes; without it the card closes at once. */
  done?: string;
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
}

export interface Dict {
  tray: Record<TrayKind, TrayText>;
  error: Record<ErrorCause, string>;
  /** Close button of a tray card: tooltip and accessible name. */
  close: string;
  drop: DropText;
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
  },
  error: {
    net: 'No connection',
    sonioxKey: 'Soniox key problem',
    claudeKey: 'Claude key problem',
  },
  close: 'Close',
  drop: {
    cancel: 'Cancel · Esc',
    cancelName: 'Cancel',
    restore: 'Restore',
    retry: 'Retry',
    done: 'Done',
    cancelled: 'Cancelled',
    ask: 'Ask',
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
  },
  error: {
    net: 'Нет сети',
    sonioxKey: 'Проблема с ключом Soniox',
    claudeKey: 'Проблема с ключом Claude',
  },
  close: 'Закрыть',
  drop: {
    cancel: 'Отменить · Esc',
    cancelName: 'Отменить',
    restore: 'Вернуть',
    retry: 'Повторить',
    done: 'Готово',
    cancelled: 'Отменено',
    ask: 'Спросить',
  },
};

export const DICTS: Record<Lang, Dict> = { en, ru };

/** The language the core set; anything else (missing, unknown) means English. */
export function pickLang(v: unknown): Lang {
  return v === 'ru' ? 'ru' : 'en';
}
