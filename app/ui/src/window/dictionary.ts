// The Dictionary section (design stage 5): the words CUE must write right, as chips, the newest first, with their count
// by the heading. "+ Word" turns into a field; Enter adds the word (it comes in first with a glint) and the field stays
// for the next one; a repeat or a word the dictionary cannot take shakes the field; Esc or leaving the field empty
// closes it. A chip's cross shows on hover and removes the word: the chip folds away. The core keeps the words
// (`dictionary.terms` in settings.json, which refine reads) and checks them again (`dictAdd`, `dictRemove`).
// A word goes into the page as text.
import { icon } from './icons';
import { h, iconButton, type Ctx } from './page';

/** The longest word the dictionary takes, as the core counts it (refine::MAX_TERM_CHARS). */
export const MAX_WORD_CHARS = 60;
/** How long a removed chip takes to fold away (window.css: .dch.out). */
const OUT_MS = 300;

/** What the dictionary would refuse before the core is asked: empty, too long, on more than one line, already there. */
export function refusedWord(word: string, words: readonly string[]): boolean {
  const w = word.trim();
  const lower = w.toLowerCase();
  return (
    !w ||
    [...w].length > MAX_WORD_CHARS ||
    [...w].some((c) => c.charCodeAt(0) < 0x20 || c === '\x7f' || c === '\u2028' || c === '\u2029') ||
    words.some((x) => x.trim().toLowerCase() === lower)
  );
}

export class DictionaryView {
  private sec: HTMLElement | null = null;
  private flow!: HTMLElement;
  private count!: HTMLElement;
  private add!: HTMLElement;
  private input!: HTMLInputElement;
  /** The word just added: its chip comes in with a glint once. */
  private fresh: string | null = null;

  constructor(private readonly ctx: Ctx) {}

  private get words(): string[] {
    return this.ctx.settings?.dictionary ?? [];
  }

  /** Draws the section into `sec` (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    const t = this.ctx.t;
    this.sec = sec;
    sec.textContent = '';
    const h3 = h('h3', '', t.dictionary);
    this.count = h('em');
    h3.append(this.count);
    this.flow = h('div', 'dflow');
    this.flow.addEventListener('click', (e) => {
      const b = (e.target as Element).closest<HTMLElement>('[data-remove]');
      if (b) void this.remove(b.closest<HTMLElement>('.dch')!);
    });
    this.add = h('span', 'dnew');
    const open = h('button', 'dnb');
    open.type = 'button';
    open.innerHTML = icon('plus');
    open.append(h('span', '', t.dictAdd));
    open.addEventListener('click', () => this.openAdd());
    this.input = h('input', 'dni');
    this.input.placeholder = t.dictNew;
    this.input.setAttribute('aria-label', t.dictNew);
    this.input.spellcheck = false;
    this.input.maxLength = MAX_WORD_CHARS * 2;
    this.input.addEventListener('keydown', (e) => this.onKey(e));
    this.input.addEventListener('blur', () => {
      if (!this.input.value.trim()) this.closeAdd();
    });
    this.add.append(open, this.input);
    sec.append(h3, h('p', 'dhint', t.dictHint), this.flow, this.add);
    this.render();
  }

  /** The words as the core holds them now (after any change of the settings). */
  render(): void {
    if (!this.sec) return;
    const t = this.ctx.t;
    this.count.textContent = this.words.length ? String(this.words.length) : '';
    this.flow.textContent = '';
    for (const w of this.words) {
      const chip = h('span', 'dch');
      chip.dataset.word = w;
      chip.append(w);
      const x = iconButton('dx', 'x', t.dictRemove);
      x.dataset.remove = '';
      chip.append(x);
      if (w === this.fresh) chip.classList.add('new');
      this.flow.append(chip);
    }
    this.flow.hidden = this.words.length === 0;
    this.fresh = null;
  }

  /** Esc: the field closes; false — it is not open. */
  esc(): boolean {
    if (!this.add?.classList.contains('open')) return false;
    this.closeAdd();
    return true;
  }

  private openAdd(): void {
    this.add.classList.add('open');
    this.input.focus({ preventScroll: true });
  }

  private closeAdd(): void {
    this.input.value = '';
    this.add.classList.remove('open');
  }

  private onKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this.closeAdd();
      return;
    }
    if (e.key !== 'Enter') return;
    e.preventDefault();
    void this.submit();
  }

  private async submit(): Promise<void> {
    const word = this.input.value.trim();
    if (refusedWord(word, this.words)) {
      if (word) this.shake();
      return;
    }
    const view = await this.ctx.change({ kind: 'dictAdd', value: word });
    if (!view?.dictionary.includes(word)) {
      this.shake();
      return;
    }
    this.fresh = word;
    this.input.value = '';
    this.render();
  }

  /** The chip folds away, then the core removes the word; a refusal draws it back. */
  private async remove(chip: HTMLElement): Promise<void> {
    const word = chip.dataset.word ?? '';
    chip.style.width = `${chip.offsetWidth}px`;
    void chip.offsetWidth;
    chip.classList.add('out');
    await new Promise((r) => setTimeout(r, this.ctx.reduced() ? 0 : OUT_MS));
    // The page draws the words again once the core answers; a refusal leaves them as they were.
    if (!(await this.ctx.change({ kind: 'dictRemove', value: word }))) this.render();
  }

  private shake(): void {
    this.input.classList.remove('bad');
    void this.input.offsetWidth;
    this.input.classList.add('bad');
  }
}
