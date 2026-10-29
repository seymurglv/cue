// The "Ask" card at the selection (spec 2 §7): the answer view in the system material, its arrow pointing at the selection.
// The core places the window; the page draws the answer, reports the card's rectangle for click-through, and folds the
// card away when the answer closes.
import { AnswerView, type Answer, type AnswerCommand } from './answer';
import { pushRect, type Rect, type UiState } from './controller';
import { DICTS, type Lang } from './strings';

/** From the core (`cue://card-place`): which side of the selection, where the arrow is, how tall the card may grow. */
export interface CardPlace {
  below: boolean;
  arrowX: number;
  maxHeight: number;
}

export interface CardIo {
  act(a: AnswerCommand): void;
  source(n: number): void;
  rects(r: Rect[]): void;
  wake(): void;
}

/** The card folds away in 250 ms (.ansC.out): its words stay until then. The core hides the window after the same time. */
export const CARD_FOLD_MS = 260;
/** Quiet longer than the card's pop-in (.ansC, 450 ms) before the frame loop sleeps: the last rectangle it reports is the
 *  card at its full size, also for an answer that shows whole at once (a refusal or a failure before anything showed). */
const QUIET_AFTER_MS = 500;

export class CardPage {
  private el: HTMLElement;
  private view: AnswerView;
  private open = false;
  private gen = 0;
  private lastRects = '';
  /** The frame since which nothing in the card moves; null — something changed. */
  private calmSince: number | null = null;

  constructor(root: HTMLElement, private io: CardIo, lang: Lang, reduced: () => boolean = () => false) {
    root.innerHTML = '<div class="ansC card-m" hidden><div class="in"></div></div>';
    this.el = root.firstElementChild as HTMLElement;
    this.view = new AnswerView(
      this.el.querySelector('.in') as HTMLElement,
      { act: (a) => io.act(a), source: (n) => io.source(n), wake: () => this.wake() },
      DICTS[lang],
      reduced,
    );
    // As in the Drop window: until the core says it is in its answer phase, the buttons and the cross wait.
    this.view.ready(false);
  }

  /** The core's state (`cue://state`): the core takes the answer's buttons and its cross only in its answer phase. */
  apply(s: UiState) {
    this.view.ready(s.phase === 'answer');
  }

  place(p: CardPlace) {
    this.el.classList.toggle('above', !p.below);
    this.el.style.setProperty('--ax', `${p.arrowX}px`);
    this.el.style.setProperty('--max-h', `${p.maxHeight}px`);
    this.wake();
  }

  /** The core's answer: a card answer shows here; anything else (the tray, closed) folds the card away. A tray answer with
   *  no card open changes nothing here, so the frame loop stays asleep while it streams. */
  answer(a: Answer | null) {
    if (a?.place === 'card') {
      if (!this.open) {
        this.open = true;
        this.gen++;
        this.el.classList.remove('out');
        this.el.hidden = false;
        // An answer back while the last one folds is a new one: it starts from an empty view, so its words are revealed
        // anew.
        this.view.update(null);
      }
      this.view.update(a);
      this.wake();
    } else if (this.open) {
      this.open = false;
      const gen = ++this.gen;
      this.el.classList.add('out');
      setTimeout(() => {
        if (gen !== this.gen) return;
        this.el.hidden = true;
        this.el.classList.remove('out');
        this.view.update(null);
      }, CARD_FOLD_MS);
      this.wake();
    }
  }

  level(v: number) {
    this.view.level(v);
  }

  /** A chord while the answer still comes (spec 2 §6.2): the card's head shakes. */
  shake() {
    const head = this.open ? (this.el.querySelector('.ah') as HTMLElement | null) : null;
    if (!head) return;
    head.classList.remove('shake');
    void head.offsetWidth;
    head.classList.add('shake');
    this.wake();
  }

  /** Something on screen changes: the quiet period starts over and a sleeping frame loop is started. */
  private wake() {
    this.calmSince = null;
    this.io.wake();
  }

  /** Every frame: the answer's words, then the card's rectangle for click-through when it changed. false — nothing has
   *  moved for a while: the loop may sleep until something wakes it. */
  frame(now: number): boolean {
    const moving = this.view.frame(now);
    const rects: Rect[] = [];
    if (this.open) pushRect(rects, this.el);
    const key = JSON.stringify(rects);
    if (key !== this.lastRects) {
      this.lastRects = key;
      this.io.rects(rects);
    }
    if (moving) {
      this.calmSince = null;
      return true;
    }
    this.calmSince ??= now;
    return now - this.calmSince < QUIET_AFTER_MS;
  }
}
