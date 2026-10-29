// The "Ask" answer (spec 2 §7): the header (the mark, the question, the card's quote, the close button), the turns (a
// follow-up's question, the thinking line, the answer revealed word by word, its sources), the listening row of a
// follow-up, and the buttons. One view for the tray from the Drop and the card at the selection. The core sends the whole
// answer every time (cue://answer); the view keeps what is already on screen and reveals only what is new.
import { CUE } from './designParts';
import { CITE_CLOSE, CITE_OPEN, parseMarkdown, renderBlocks } from './markdown';
import { KEY_NAME, type Dict } from './strings';

export type AnswerPlace = 'tray' | 'card';
export type ThinkStep = 'searching' | 'reading' | 'fetching' | 'writing';
export type TurnState = 'pending' | 'streaming' | 'done' | 'refused' | 'failed';
export type AnswerAction = 'insert' | 'below' | 'replace' | 'copy';
/** The core's answer (cue_core::ask::AnswerView: the same names). */
export interface Segment { text: string; cites: number[] }
export interface Turn { question: string; step: ThinkStep | null; segments: Segment[]; state: TurnState }
export interface Source { n: number; site: string; title: string }
export interface Answer {
  place: AnswerPlace;
  quote: string | null;
  turns: Turn[];
  sources: Source[];
  listening: boolean;
  missed: boolean;
  actions: AnswerAction[];
}
/** The answer's buttons; `retry` is the Drop's Retry of a follow-up that could not be transcribed (decision 7). */
export type AnswerCommand = 'answerInsert' | 'answerBelow' | 'answerReplace' | 'answerCopy' | 'answerRetry' | 'answerClose' | 'retry';

export interface AnswerIo {
  act(a: AnswerCommand): void;
  /** A source chip was clicked: the core opens its page (the view never has the URL). */
  source(n: number): void;
  /** The content grew or shrank: the tray follows its height. */
  resized?(): void;
  wake?(): void;
}

/** One reveal step every this many ms, one to three words each (spec 2 §7). */
export const REVEAL_EVERY_MS = 60;
/** "Copied" stays this long before the button says "Copy" again (the design). */
export const COPIED_MS = 1600;

/** The marks' spring, as the Drop's own (drop.js): with reduced motion stiff, so a mark takes each pose at once. */
const SPRING = { k: 0.17, d: 0.7 };
const STIFF = { k: 20, d: 0.05 };

const COMMAND: Record<AnswerAction, AnswerCommand> = {
  insert: 'answerInsert',
  below: 'answerBelow',
  replace: 'answerReplace',
  copy: 'answerCopy',
};

/** The answer's icons (the approved design, 04-answer). */
function icons(): Record<AnswerAction | 'globe' | 'retry' | 'ok' | 'x', string> {
  const { SV, IC } = CUE();
  return {
    insert: SV('<rect x="4.5" y="4" width="11" height="13.5" rx="2"/><path d="M7.5 4V3h5v1M7.5 9h5M7.5 12h3.5"/>'),
    below: SV('<path d="M4 4.5h12M4 8.5h12"/><path d="M10 11.5v5M7.5 14l2.5 2.5 2.5-2.5"/>'),
    replace: SV('<path d="M4 6.5h9.5M11 4l2.5 2.5L11 9M16 13.5H6.5M9 11l-2.5 2.5L9 16"/>'),
    copy: IC.copy,
    globe: SV('<circle cx="10" cy="10" r="7"/><path d="M3 10h14M10 3c2 2.2 2.8 4.5 2.8 7s-.8 4.8-2.8 7c-2-2.2-2.8-4.5-2.8-7S8 5.2 10 3z"/>'),
    retry: IC.retry,
    ok: IC.ok,
    x: IC.x,
  };
}

/** A turn's text with its citation marks: each segment, then the numbers of the sources it cites. The model's own
 *  private-use marks are taken out first, so no text can fake a citation. */
export function markedText(segments: Segment[]): string {
  const clean = (s: string) => s.replace(/[]/g, '');
  return segments.map((s) => clean(s.text) + s.cites.map((n) => `${CITE_OPEN}${n}${CITE_CLOSE}`).join('')).join('');
}

/** What one turn has on screen. */
interface Item {
  el: HTMLElement;
  /** The follow-up's question row (none for the first turn: the header shows it) and the question it shows. */
  uq: HTMLElement | null;
  question: string;
  body: HTMLElement;
  /** The marked text it was last drawn from ('' — the thinking line or a note). */
  text: string;
  words: HTMLElement[];
  shown: number;
  state: TurnState;
  cites: number[];
  chips: boolean;
}

export class AnswerView {
  private model: Answer | null = null;
  private parts: { aq: HTMLElement; x: HTMLButtonElement; scr: HTMLElement; th: HTMLElement; aft: HTMLElement } | null = null;
  private items: Item[] = [];
  private mark: any = null;
  private ear: { row: HTMLElement; morph: any } | null = null;
  private miss: HTMLElement | null = null;
  private lvl = 0;
  private nextAt = 0;
  private tick = 0;
  /** The scroll follows the end until the user scrolls up. */
  private stick = true;
  /** Where toEnd last put the scroll: its own scroll event is not the user's. */
  private autoTop = 0;
  private footKey = '';
  private copyGen = 0;
  /** The core takes the buttons and the cross now (see ready). */
  private live = true;

  constructor(
    private root: HTMLElement,
    private io: AnswerIo,
    private t: Dict,
    private reduced: () => boolean = () => false,
  ) {}

  /** The core's answer; null — it closed, and the view empties. */
  update(a: Answer | null) {
    this.model = a;
    if (!a) {
      this.root.replaceChildren();
      this.parts = null;
      this.items = [];
      this.mark = null;
      this.ear = null;
      this.miss = null;
      this.footKey = '';
      this.stick = true;
      return;
    }
    if (!this.parts) this.build(a.place);
    this.head(a);
    this.syncTurns(a);
    this.syncListening(a.listening);
    this.syncMissed(a.missed);
    this.foot(a);
    // A reduced-motion change reaches the marks with the next update.
    for (const m of [this.mark, this.ear?.morph]) if (m) Object.assign(m, this.reduced() ? STIFF : SPRING);
    this.toEnd();
    this.io.resized?.();
    this.io.wake?.();
  }

  /** Whether the core takes the answer's buttons and its cross: only in its answer phase. While a follow-up is recorded,
   *  transcribed or missed the answer stays open, but the core ignores them (flow.rs), so they wait. The missed row's Retry
   *  is the Drop's own retry, which the core takes then, and never waits. */
  ready(on: boolean) {
    this.live = on;
    this.enable();
  }

  /** The voice level while a follow-up is recorded. */
  level(v: number) {
    this.lvl = Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
  }

  /** The last request is still running: the buttons wait, the mark spins. */
  busy(): boolean {
    const last = this.model?.turns[this.model.turns.length - 1];
    return !!last && (last.state === 'pending' || last.state === 'streaming');
  }

  /** Words of the last turn are still to be revealed. */
  revealing(): boolean {
    const it = this.items[this.items.length - 1];
    return !!it && it.shown < it.words.length;
  }

  /** Every frame: words, then the sources and the buttons once all is shown. false — nothing here moves. */
  frame(now: number): boolean {
    if (!this.model) return false;
    const it = this.items[this.items.length - 1];
    if (it && it.shown < it.words.length && now >= this.nextAt) {
      const k = this.reduced() ? it.words.length : 1 + (this.tick++ % 3);
      for (let j = 0; j < k && it.shown < it.words.length; j++) it.words[it.shown++].classList.add('on');
      this.nextAt = now + REVEAL_EVERY_MS;
      this.toEnd();
      this.io.resized?.();
    }
    for (const item of this.items) this.chips(item);
    this.enable();
    const busy = this.busy();
    if (this.mark) {
      this.mark.set(busy ? 'ask' : 'idle', now, 'ask');
      // At rest the mark is the lit cursor, held at the start of its blink: the frame loop sleeps then (quiet mode), and a
      // blink stopped half-way would leave the cursor dim.
      this.mark.tick(busy ? now : this.mark.t0);
    }
    this.ear?.morph.tick(now, this.lvl, [0.5, 0.5, 0.5, 0.5]);
    return busy || this.revealing() || this.model.listening;
  }

  private build(place: AnswerPlace) {
    const x = place === 'card' ? 'ibb' : 'xb nottl';
    // Static markup with the dictionary's words only: the model's text never goes through innerHTML.
    this.root.innerHTML =
      `<div class="ah"><span class="am"><svg viewBox="0 0 24 24" width="24" height="24"><g></g></svg></span>` +
      `<div class="aq"></div><button class="${x}" data-x title="${this.t.close}" aria-label="${this.t.close}">${icons().x}</button></div>` +
      `<div class="scr"><div class="th"></div></div><div class="aft"></div>`;
    const q = (s: string) => this.root.querySelector(s) as HTMLElement;
    this.parts = { aq: q('.aq'), x: q('[data-x]') as HTMLButtonElement, scr: q('.scr'), th: q('.th'), aft: q('.aft') };
    this.mark = new (CUE().Morph)(this.root.querySelector('.am g'), { state: 'ask', tone: 'ask' });
    const close = this.parts.x;
    close.addEventListener('click', () => {
      if (!close.disabled) this.io.act('answerClose');
    });
    const scr = this.parts.scr;
    this.stick = true;
    this.autoTop = 0;
    scr.addEventListener('scroll', () => {
      // The view's own scroll to the end fires this too, maybe only once more has come in under it: only a scroll to
      // elsewhere than where toEnd put it, the user's, lets go of the end.
      if (scr.scrollTop + scr.clientHeight >= scr.scrollHeight - 4) this.stick = true;
      else if (Math.abs(scr.scrollTop - this.autoTop) > 1) this.stick = false;
    });
  }

  private head(a: Answer) {
    const aq = this.parts!.aq;
    aq.textContent = a.turns[0]?.question ?? '';
    if (a.quote != null) {
      const small = this.root.ownerDocument.createElement('small');
      // A function, so "$&" and the like in the selection are not replacement patterns.
      const quote = a.quote;
      small.textContent = this.t.answer.about.replace('{Q}', () => quote);
      aq.appendChild(small);
    }
  }

  private syncTurns(a: Answer) {
    const th = this.parts!.th;
    while (this.items.length > a.turns.length) this.items.pop()!.el.remove();
    a.turns.forEach((turn, i) => {
      let it = this.items[i];
      if (!it) {
        // An earlier answer is shown whole once a follow-up starts.
        const prev = this.items[i - 1];
        if (prev) this.showAll(prev);
        it = this.newItem(i, turn.question);
        this.items.push(it);
        th.insertBefore(it.el, this.ear?.row ?? null);
      }
      this.syncItem(it, turn);
    });
  }

  private newItem(i: number, question: string): Item {
    const doc = this.root.ownerDocument;
    const el = doc.createElement('div');
    el.className = 'it';
    let uq: HTMLElement | null = null;
    if (i > 0) {
      uq = doc.createElement('div');
      uq.className = 'uq';
      uq.textContent = question;
      el.appendChild(uq);
    }
    const body = doc.createElement('div');
    el.appendChild(body);
    return { el, uq, question, body, text: '', words: [], shown: 0, state: 'pending', cites: [], chips: false };
  }

  private syncItem(it: Item, turn: Turn) {
    // A new question takes the place of a failed one: the same row, its own words.
    if (turn.question !== it.question) {
      it.question = turn.question;
      if (it.uq) it.uq.textContent = turn.question;
    }
    it.state = turn.state;
    it.cites = [...new Set(turn.segments.flatMap((s) => s.cites))];
    if (turn.state === 'refused') return this.note(it, this.t.answer.refused);
    const text = markedText(turn.segments);
    if (turn.state === 'failed') return this.failed(it, text);
    if (!text.trim()) return this.thinking(it, turn.step);
    if (text !== it.text) this.render(it, text);
  }

  /** A failed turn (spec 2 §8): the words already shown stay, all of them now, with the error under them and Retry below;
   *  with no words, the error alone. */
  private failed(it: Item, text: string) {
    if (!text.trim()) return this.note(it, this.t.answer.failed);
    if (text !== it.text) this.render(it, text);
    this.showAll(it);
    if (it.body.querySelector('.note')) return;
    const n = this.root.ownerDocument.createElement('div');
    n.className = 'note';
    n.textContent = this.t.answer.failed;
    it.body.appendChild(n);
  }

  /** Before the words: the real step the model is on, if any (spec 2 §7), over shimmering placeholder lines. */
  private thinking(it: Item, step: ThinkStep | null) {
    let tl = it.body.querySelector<HTMLElement>('.think .tl');
    if (!tl) {
      it.body.innerHTML =
        `<div class="think"><span class="ti">${icons().globe}</span><span class="tl"></span></div>` +
        `<div class="sk"><i></i><i></i><i></i></div>`;
      tl = it.body.querySelector<HTMLElement>('.think .tl')!;
      Object.assign(it, { text: '', words: [], shown: 0, chips: false });
    }
    const label = step ? this.t.answer[step] : '';
    if ((tl.dataset.t ?? '') !== label) CUE().setLabel(tl, label);
    it.body.querySelector<HTMLElement>('.ti')!.hidden = !step;
  }

  private note(it: Item, text: string) {
    if (it.body.querySelector('.note')?.textContent === text) return;
    const n = this.root.ownerDocument.createElement('div');
    n.className = 'note';
    n.textContent = text;
    it.body.replaceChildren(n);
    Object.assign(it, { text: '', words: [], shown: 0, chips: false });
  }

  private render(it: Item, text: string) {
    const { root, words } = renderBlocks(parseMarkdown(text), this.root.ownerDocument);
    const shown = Math.min(it.shown, words.length);
    for (let i = 0; i < shown; i++) words[i].classList.add('on', 'seen');
    const aa = this.root.ownerDocument.createElement('div');
    aa.className = 'aa';
    aa.appendChild(root);
    it.body.replaceChildren(aa);
    Object.assign(it, { text, words, shown, chips: false });
  }

  private showAll(it: Item) {
    for (const w of it.words) w.classList.add('on', 'seen');
    it.shown = it.words.length;
  }

  /** The sources under a turn, once it is complete and shown whole (spec 2 §7): chips one after another (CSS, 80 ms). */
  private chips(it: Item) {
    if (it.chips || it.state !== 'done' || !it.words.length || it.shown < it.words.length) return;
    it.chips = true;
    const doc = this.root.ownerDocument;
    const list = it.cites.map((n) => this.model!.sources.find((s) => s.n === n)).filter((s): s is Source => !!s);
    if (list.length) {
      const src = doc.createElement('div');
      src.className = 'src';
      list.forEach((s, i) => {
        const chip = doc.createElement('button');
        chip.className = 'sc';
        chip.dataset.n = String(s.n);
        chip.style.setProperty('--i', String(i));
        chip.title = s.title;
        const num = doc.createElement('i');
        num.textContent = String(s.n);
        const site = doc.createElement('b');
        site.textContent = s.site;
        chip.append(num, site, doc.createTextNode(` ${s.title}`));
        chip.addEventListener('click', () => this.io.source(s.n));
        chip.addEventListener('mouseenter', () => this.light(it, s.n, true));
        chip.addEventListener('mouseleave', () => this.light(it, s.n, false));
        src.appendChild(chip);
      });
      it.body.appendChild(src);
      // The sources come into view like the words before them.
      this.toEnd();
    }
    for (const c of it.body.querySelectorAll<HTMLElement>('.cite')) {
      const n = Number(c.dataset.n);
      c.addEventListener('mouseenter', () => this.light(it, n, true));
      c.addEventListener('mouseleave', () => this.light(it, n, false));
    }
    this.io.resized?.();
  }

  /** A source's numbers in the text and its chip light up together (spec 2 §7). */
  private light(it: Item, n: number, on: boolean) {
    for (const el of it.body.querySelectorAll(`[data-n="${n}"]`)) el.classList.toggle('hl', on);
  }

  private syncListening(on: boolean) {
    if (on && !this.ear) {
      const doc = this.root.ownerDocument;
      const row = doc.createElement('div');
      row.className = 'lsn';
      row.innerHTML = '<svg viewBox="0 0 24 24" width="20" height="20"><g></g></svg>';
      const label = doc.createElement('span');
      label.textContent = this.t.answer.listening;
      row.appendChild(label);
      const morph = new (CUE().Morph)(row.querySelector('g'), { state: 'listen' });
      morph.tone('askrec');
      this.parts!.th.appendChild(row);
      this.ear = { row, morph };
    } else if (!on && this.ear) {
      this.ear.row.remove();
      this.ear = null;
    }
  }

  /** The follow-up could not be transcribed (decision 7): a row with Retry, which transcribes it again; the answer stays. */
  private syncMissed(on: boolean) {
    if (on && !this.miss) {
      const label = this.root.ownerDocument.createElement('span');
      label.textContent = this.t.answer.missed;
      const row = this.root.ownerDocument.createElement('div');
      row.className = 'lsn miss';
      row.append(label, this.button('btnA sec', 'retry', this.t.answer.retry, 'retry'));
      this.parts!.th.appendChild(row);
      this.miss = row;
    } else if (!on && this.miss) {
      this.miss.remove();
      this.miss = null;
    }
  }

  /** The buttons (spec 2 §7): the tray's Insert and Copy with the follow-up hint; the card's Insert below, Replace and a copy
   *  icon with the key alone; after a failure, Retry. Rebuilt only when the set changes, so "Copied" keeps its moment. */
  private foot(a: Answer) {
    const failed = a.turns[a.turns.length - 1]?.state === 'failed';
    const key = failed ? 'retry' : `${a.place}:${a.actions.join(',')}`;
    if (key !== this.footKey) {
      this.footKey = key;
      const doc = this.root.ownerDocument;
      const aft = this.parts!.aft;
      aft.replaceChildren();
      if (failed) aft.appendChild(this.button('btnA pri', 'retry', this.t.answer.retry, 'answerRetry'));
      else
        a.actions.forEach((act, i) => {
          const iconOnly = act === 'copy' && a.place === 'card';
          const cls = iconOnly ? 'ibb' : i === 0 ? 'btnA pri' : 'btnA sec';
          aft.appendChild(this.button(cls, act, iconOnly ? null : this.t.answer[act], COMMAND[act]));
        });
      const sp = doc.createElement('span');
      sp.className = 'sp';
      const hint = doc.createElement('span');
      hint.className = 'hint';
      const kbd = doc.createElement('kbd');
      kbd.textContent = KEY_NAME;
      hint.appendChild(kbd);
      if (a.place === 'tray') hint.appendChild(doc.createTextNode(` ${this.t.answer.followUp}`));
      aft.append(sp, hint);
    }
    this.enable();
  }

  private button(cls: string, icon: AnswerAction | 'retry', label: string | null, cmd: AnswerCommand): HTMLButtonElement {
    const doc = this.root.ownerDocument;
    const b = doc.createElement('button');
    b.className = cls;
    b.dataset.cmd = cmd;
    const ai = doc.createElement('span');
    ai.className = 'ai';
    ai.innerHTML = icons()[icon];
    b.appendChild(ai);
    let lbl: HTMLElement | null = null;
    if (label != null) {
      lbl = doc.createElement('span');
      lbl.className = 'lbl';
      lbl.dataset.t = label;
      lbl.textContent = label;
      b.appendChild(lbl);
    } else b.title = this.t.answer.copy;
    b.addEventListener('click', () => {
      if (b.disabled) return;
      this.io.act(cmd);
      if (cmd !== 'answerCopy') return;
      const gen = ++this.copyGen;
      if (lbl) CUE().setLabel(lbl, this.t.answer.copied);
      ai.innerHTML = icons().ok;
      setTimeout(() => {
        if (gen !== this.copyGen || !b.isConnected) return;
        if (lbl) CUE().setLabel(lbl, this.t.answer.copy);
        ai.innerHTML = icons().copy;
      }, COPIED_MS);
    });
    return b;
  }

  /** The buttons wait while the answer is still coming or still being revealed (spec 2 §7), Retry aside; all of them and
   *  the cross wait outside the core's answer phase (ready). */
  private enable() {
    if (!this.parts) return;
    const off = this.busy() || this.revealing();
    for (const b of this.parts.aft.querySelectorAll<HTMLButtonElement>('button')) {
      b.disabled = !this.live || (off && b.dataset.cmd !== 'answerRetry');
    }
    this.parts.x.disabled = !this.live;
  }

  private toEnd() {
    const scr = this.parts?.scr;
    if (!scr || !this.stick) return;
    scr.scrollTop = scr.scrollHeight;
    this.autoTop = scr.scrollTop;
  }
}
