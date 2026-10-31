// The key rows of the settings tray's Keys page and of the welcome window's Keys step: Soniox and Anthropic, each a
// password field, "Check" and the key's state. A key is typed into the field, sent once to be saved and the field
// cleared: its value is not kept here and never logged, and only its mask comes back into the page.
import * as api from './api';
import type { KeyName } from './api';
import { codeOf, h, logFailed, srow, type Ctx } from './page';
import type { KeyState } from './text';

export const KEYS: readonly KeyName[] = ['soniox', 'anthropic'];
export const KEY_TITLE: Record<KeyName, string> = { soniox: 'Soniox', anthropic: 'Anthropic' };

/** What the page knows of a key: saved or not, its mask, the result of its last check (null — none yet). */
export interface KeyInfo {
  saved: boolean;
  mask: string;
  state: KeyState | null;
}

export class KeyRows {
  readonly keys: Record<KeyName, KeyInfo> = {
    soniox: { saved: false, mask: '', state: null },
    anthropic: { saved: false, mask: '', state: null },
  };
  /** The saved keys were read: until then a row shows no state. */
  read = false;

  /** `box` — where the rows are shown now (null — nowhere); `changed` — the rows show something new. */
  constructor(
    private readonly ctx: Pick<Ctx, 't'>,
    private readonly box: () => ParentNode | null,
    private readonly changed: () => void,
  ) {}

  row(name: KeyName): HTMLElement {
    const t = this.ctx.t;
    const input = h('input', 'kin');
    input.type = 'password';
    input.setAttribute('autocomplete', 'off');
    input.spellcheck = false;
    input.dataset.key = name;
    input.setAttribute('aria-label', KEY_TITLE[name]);
    const btn = h('button', 'btn2 ck', t.check);
    btn.type = 'button';
    btn.dataset.check = name;
    const { row, small } = srow(KEY_TITLE[name], null, input, btn);
    small.dataset.state = name;
    row.dataset.key = name;
    this.paint(name, input, btn, small);
    return row;
  }

  /** The state a row shows: the last check's, else saved or not once the keys were read. */
  state(name: KeyName): KeyState | null {
    const k = this.keys[name];
    return k.state ?? (this.read ? (k.saved ? 'saved' : 'missing') : null);
  }

  /** The rows shown show what is known now. */
  refresh(): void {
    const box = this.box();
    for (const name of KEYS) {
      const row = box?.querySelector(`.srow[data-key="${name}"]`);
      if (row) this.paint(name, row.querySelector('input')!, row.querySelector('button')!, row.querySelector('small')!);
    }
    this.changed();
  }

  /** Whether each key is saved, and its mask. */
  async readAll(): Promise<void> {
    await Promise.all(
      KEYS.map((name) =>
        api.keyState(name).then(
          (v) => Object.assign(this.keys[name], { saved: v.saved, mask: v.mask }),
          (e) => logFailed(`reading the ${name} key`, e),
        ),
      ),
    );
    this.read = true;
    this.refresh();
  }

  /** The field's key, if one was typed, is saved (the field is emptied at once), then the saved key is checked with a
   *  free request. */
  async check(name: KeyName): Promise<void> {
    const k = this.keys[name];
    if (k.state === 'checking') return;
    k.state = 'checking';
    this.refresh();
    try {
      if (await this.saveTyped(name)) k.state = await api.keyCheck(name);
    } catch (e) {
      logFailed(`checking the ${name} key`, e);
      k.state = 'unchecked';
    }
    this.refresh();
  }

  /** A row's hint (the mask, or "paste the key") and state. */
  private paint(name: KeyName, input: HTMLInputElement, btn: HTMLButtonElement, small: HTMLElement): void {
    const t = this.ctx.t;
    const k = this.keys[name];
    input.placeholder = k.saved && k.mask ? k.mask : t.keyHint;
    btn.disabled = k.state === 'checking';
    const state = this.state(name);
    small.textContent = state ? t.keyStates[state] : '';
    small.hidden = !state;
    small.classList.toggle('ok', state === 'works');
    small.classList.toggle('warn', state === 'badKey');
  }

  /** Sends the typed key to be saved and clears the field; false — it was refused. Nothing typed: true. */
  private async saveTyped(name: KeyName): Promise<boolean> {
    const input = this.box()?.querySelector<HTMLInputElement>(`input[data-key="${name}"]`);
    const typed = input?.value ?? '';
    if (input) input.value = '';
    if (!typed.trim()) return true;
    const k = this.keys[name];
    try {
      const v = await api.keySet(name, typed);
      Object.assign(k, { saved: v.saved, mask: v.mask });
      return true;
    } catch (e) {
      logFailed(`saving the ${name} key`, e);
      k.state = codeOf(e) === 'emptyKey' ? 'missing' : 'store';
      return false;
    }
  }
}
