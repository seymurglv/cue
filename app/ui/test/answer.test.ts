import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { AnswerView, COPIED_MS, REVEAL_EVERY_MS, markedText, type Answer, type AnswerCommand, type Turn } from '../src/answer';
import { CITE_CLOSE, CITE_OPEN } from '../src/markdown';
import { en, ru } from '../src/strings';

let root: HTMLElement;
let acts: AnswerCommand[];
let opened: number[];
let reduced = false;
let view: AnswerView;

const turn = (over: Partial<Turn> = {}): Turn => ({ question: 'What is HTTP 418?', step: null, segments: [], state: 'pending', ...over });
const done = (text: string, over: Partial<Turn> = {}) => turn({ state: 'done', segments: [{ text, cites: [] }], ...over });
const answer = (over: Partial<Answer> = {}): Answer => ({
  place: 'tray', quote: null, turns: [turn()], sources: [], listening: false, missed: false, actions: ['insert', 'copy'], ...over,
});
const words = () => [...root.querySelectorAll<HTMLElement>('.aa .w')];
const shown = () => words().filter((w) => w.classList.contains('on')).map((w) => w.textContent);
const buttons = () => [...root.querySelectorAll<HTMLButtonElement>('.aft button')];
/** Frames until nothing is left to reveal. */
const revealAll = () => {
  let t = 10_000;
  do {
    view.frame(t);
    t += REVEAL_EVERY_MS;
  } while (view.revealing());
  view.frame(t);
};
const make = (lang = en) => new AnswerView(root, { act: (a) => acts.push(a), source: (n) => opened.push(n) }, lang, () => reduced);
/** The answer's scroll area as a browser lays it out: 300 px of view over the content (20 px a shown word, 30 the sources,
 *  24 a row). A browser fires "scroll" only after the frame, when more may be in: settle() stands for that. */
const scroller = () => {
  const scr = root.querySelector<HTMLElement>('.scr')!;
  let top = 0;
  let extra = 0;
  const content = () =>
    words().filter((w) => w.classList.contains('on')).length * 20 +
    root.querySelectorAll('.src').length * 30 +
    root.querySelectorAll('.lsn, .uq').length * 24 +
    extra;
  const max = () => Math.max(0, content() - 300);
  Object.defineProperty(scr, 'clientHeight', { configurable: true, get: () => 300 });
  Object.defineProperty(scr, 'scrollHeight', { configurable: true, get: () => Math.max(300, content()) });
  Object.defineProperty(scr, 'scrollTop', {
    configurable: true,
    get: () => top,
    set: (v: number) => {
      top = Math.min(Math.max(0, v), max());
    },
  });
  const settle = () => scr.dispatchEvent(new Event('scroll'));
  return {
    top: () => top,
    atEnd: () => top >= max(),
    grow: (px: number) => {
      extra += px;
    },
    settle,
    /** The user scrolls to `v`. */
    user: (v: number) => {
      scr.scrollTop = v;
      settle();
    },
  };
};
const many = Array.from({ length: 40 }, (_, i) => `w${i}`).join(' ');

beforeEach(() => {
  document.body.innerHTML = '<div id="in"></div>';
  root = document.getElementById('in')!;
  acts = [];
  opened = [];
  reduced = false;
  view = make();
});
afterEach(() => vi.useRealTimers());

describe('The "Ask" answer', () => {
  it('citation marks follow their segments; the model cannot fake one', () => {
    expect(markedText([{ text: 'Born 1916', cites: [1, 2] }, { text: '.9', cites: [] }])).toBe(
      'Born 191612.9',
    );
  });

  it('words after a search start a paragraph; blocks a citation splits run on', () => {
    const segments = [
      { text: 'Checking the data.', cites: [] },
      { text: ' The weather in Baku: ', cites: [], after_search: true },
      { text: '+18 °C', cites: [1] },
      { text: ', clear.', cites: [] },
    ];
    expect(markedText(segments)).toBe(`Checking the data.\n\nThe weather in Baku: +18 °C${CITE_OPEN}1${CITE_CLOSE}, clear.`);
    view.update(answer({ turns: [turn({ state: 'done', segments })] }));
    const paragraphs = [...root.querySelectorAll('.aa p')].map((p) => p.textContent);
    expect(paragraphs).toEqual(['Checking the data.', 'The weather in Baku: +18 °C1, clear.']);
  });

  it('opens with the question; the card quotes the selection', () => {
    view.update(answer());
    expect(root.querySelector('.aq')!.textContent).toBe('What is HTTP 418?');
    expect(root.querySelector('.am g')).not.toBeNull();
    view.update(null);
    view = make(ru);
    view.update(answer({ place: 'card', quote: 'Нужно выбрать оркестратор…', actions: ['below', 'replace', 'copy'] }));
    expect(root.querySelector('.aq small')!.textContent).toBe('по выделенному: «Нужно выбрать оркестратор…»');
    expect(root.querySelector('[data-x]')!.className).toBe('ibb');
  });

  it('the quote is shown as selected, "$" patterns included', () => {
    view.update(answer({ place: 'card', quote: "costs $& or $' or $1", actions: ['copy'] }));
    expect(root.querySelector('.aq small')!.textContent).toBe("about the selection: “costs $& or $' or $1”");
  });

  it('at rest the mark is the lit cursor: the frame loop sleeps then, and a blink must not stop half-way', () => {
    view.update(answer({ turns: [done('x')] }));
    // From 10 000 the mark rests; 10 600 to 11 100 is where a blinking cursor would be dim.
    for (let t = 10_000; t <= 11_100; t += 16) view.frame(t);
    const lines = [...root.querySelectorAll('.am .m-l')].slice(0, 3);
    expect(lines.map((l) => Number(l.getAttribute('opacity')))).toEqual([1, 1, 1]);
  });

  it('the thinking line names only real steps; the words replace it', () => {
    view.update(answer());
    const tl = () => root.querySelector<HTMLElement>('.think .tl')!;
    expect(tl().dataset.t ?? '').toBe('');
    expect(root.querySelector('.sk')).not.toBeNull();
    view.update(answer({ turns: [turn({ step: 'searching' })] }));
    expect(tl().dataset.t).toBe('Searching the web');
    view.update(answer({ turns: [turn({ step: 'reading' })] }));
    expect(tl().dataset.t).toBe('Reading sources');
    view.update(answer({ turns: [turn({ state: 'streaming', segments: [{ text: 'An April', cites: [] }] })] }));
    expect(root.querySelector('.think')).toBeNull();
    expect(words().map((w) => w.textContent)).toEqual(['An', 'April']);
  });

  it('reveals one to three words a step; with reduced motion, all at once', () => {
    view.update(answer({ turns: [done('one two three four five six seven')] }));
    expect(shown()).toEqual([]);
    view.frame(0);
    expect(shown()).toHaveLength(1);
    view.frame(30);
    expect(shown()).toHaveLength(1);
    view.frame(60);
    expect(shown()).toHaveLength(3);
    view.frame(120);
    expect(shown()).toHaveLength(6);
    view.frame(180);
    expect(shown()).toHaveLength(7);
    view.update(null);
    reduced = true;
    view.update(answer({ turns: [done('a b c d')] }));
    view.frame(1_000);
    expect(shown()).toEqual(['a', 'b', 'c', 'd']);
  });

  it('more text keeps the words already shown, without fading them in again', () => {
    view.update(answer({ turns: [turn({ state: 'streaming', segments: [{ text: 'one two', cites: [] }] })] }));
    view.frame(0);
    view.frame(60);
    expect(shown()).toEqual(['one', 'two']);
    view.update(answer({ turns: [turn({ state: 'streaming', segments: [{ text: 'one two three', cites: [] }] })] }));
    expect(shown()).toEqual(['one', 'two']);
    expect(root.querySelectorAll('.aa .w.seen')).toHaveLength(2);
  });

  it('the tray: Insert and Copy, waiting for the answer; the follow-up hint', () => {
    view.update(answer({ turns: [turn({ state: 'streaming', segments: [{ text: 'a b', cites: [] }] })] }));
    expect(buttons().map((b) => b.dataset.cmd)).toEqual(['answerInsert', 'answerCopy']);
    expect(buttons().every((b) => b.disabled)).toBe(true);
    view.update(answer({ turns: [done('a b')] }));
    expect(buttons().every((b) => b.disabled), 'still revealing').toBe(true);
    revealAll();
    expect(buttons().some((b) => b.disabled)).toBe(false);
    expect(root.querySelector('.btnA.pri')!.textContent).toBe('Insert');
    expect(root.querySelector('.hint')!.textContent).toBe('Ctrl + Win follow up');
    buttons()[0].click();
    expect(acts).toEqual(['answerInsert']);
  });

  it('a tray answer about a selection offers the selection\'s buttons, with their labels', () => {
    view.update(answer({ actions: ['below', 'replace', 'copy'], turns: [done('x')] }));
    revealAll();
    expect(buttons().map((b) => [b.dataset.cmd, b.className])).toEqual([
      ['answerBelow', 'btnA pri'],
      ['answerReplace', 'btnA sec'],
      ['answerCopy', 'btnA sec'],
    ]);
  });

  it('the card: Insert below, Replace and a copy icon; at read-only text, Copy alone', () => {
    const card = answer({ place: 'card', actions: ['below', 'replace', 'copy'], turns: [done('x')] });
    view.update(card);
    revealAll();
    expect(buttons().map((b) => [b.dataset.cmd, b.className])).toEqual([
      ['answerBelow', 'btnA pri'],
      ['answerReplace', 'btnA sec'],
      ['answerCopy', 'ibb'],
    ]);
    expect(root.querySelector('.hint')!.textContent).toBe('Ctrl + Win');
    view.update({ ...card, actions: ['copy'] });
    expect(buttons().map((b) => b.dataset.cmd)).toEqual(['answerCopy']);
  });

  it('Copy says Copied, then Copy again', () => {
    vi.useFakeTimers();
    view.update(answer({ turns: [done('x')] }));
    revealAll();
    const copy = buttons()[1];
    copy.click();
    expect(acts).toEqual(['answerCopy']);
    expect(copy.querySelector('.lbl')!.getAttribute('data-t')).toBe('Copied');
    vi.advanceTimersByTime(COPIED_MS);
    expect(copy.querySelector('.lbl')!.getAttribute('data-t')).toBe('Copy');
  });

  it('sources: chips once the turn is shown whole, lit together with their numbers; a click asks the core', () => {
    view.update(
      answer({
        sources: [{ n: 1, site: 'blog.rust-lang.org', title: 'Rust Blog' }],
        turns: [turn({ state: 'done', segments: [{ text: 'Rust 1.95', cites: [1] }, { text: '.', cites: [] }] })],
      }),
    );
    expect(root.querySelector('.src')).toBeNull();
    revealAll();
    const chip = root.querySelector<HTMLButtonElement>('.sc')!;
    expect(chip.textContent).toBe('1blog.rust-lang.org Rust Blog');
    const cite = root.querySelector<HTMLElement>('.aa .cite')!;
    expect(cite.textContent).toBe('1');
    chip.dispatchEvent(new MouseEvent('mouseenter'));
    expect(cite.classList.contains('hl')).toBe(true);
    chip.dispatchEvent(new MouseEvent('mouseleave'));
    cite.dispatchEvent(new MouseEvent('mouseenter'));
    expect(chip.classList.contains('hl')).toBe(true);
    chip.click();
    expect(opened).toEqual([1]);
  });

  it('a follow-up shows its question; the earlier answer is shown whole', () => {
    const first = done('a b c d e');
    view.update(answer({ turns: [first] }));
    view.frame(0);
    view.update(answer({ turns: [first, turn({ question: 'And for five servers?' })] }));
    expect(root.querySelectorAll('.it')[0].querySelectorAll('.w.on')).toHaveLength(5);
    expect(root.querySelector('.it .uq')!.textContent).toBe('And for five servers?');
    expect(root.querySelector('.aq')!.textContent).toBe('What is HTTP 418?');
  });

  it('the listening row of a follow-up comes and goes', () => {
    view.update(answer({ listening: true, turns: [done('x')] }));
    expect(root.querySelector('.lsn')!.textContent).toBe('Listening for a follow-up…');
    view.update(answer({ listening: false, turns: [done('x')] }));
    expect(root.querySelector('.lsn')).toBeNull();
  });

  it('a follow-up that could not be transcribed: a row with Retry, the answer stays', () => {
    view.update(answer({ missed: true, turns: [done('x')] }));
    revealAll();
    const row = root.querySelector<HTMLElement>('.lsn.miss')!;
    expect(row.textContent).toContain("Couldn't make out the follow-up");
    row.querySelector<HTMLButtonElement>('button')!.click();
    expect(acts).toEqual(['retry']);
    expect(root.querySelector('.aa')!.textContent).toBe('x');
    view.update(answer({ missed: false, turns: [done('x')] }));
    expect(root.querySelector('.lsn.miss')).toBeNull();
  });

  it('a new question in place of a failed follow-up shows its own words', () => {
    const first = done('a');
    view.update(answer({ turns: [first, turn({ question: 'And for five?', state: 'failed' })] }));
    view.update(answer({ turns: [first, turn({ question: 'And for ten?' })] }));
    expect(root.querySelectorAll('.it')[1].querySelector('.uq')!.textContent).toBe('And for ten?');
  });

  it('a failure after words were shown keeps them, with the error and Retry under them', () => {
    view.update(answer({ turns: [turn({ state: 'streaming', segments: [{ text: 'The first words', cites: [] }] })] }));
    view.update(answer({ turns: [turn({ state: 'failed', segments: [{ text: 'The first words', cites: [] }] })] }));
    expect(shown()).toEqual(['The', 'first', 'words']);
    expect(view.revealing()).toBe(false);
    const body = root.querySelector('.it')!.lastElementChild!;
    expect([...body.children].map((c) => c.className)).toEqual(['aa', 'note']);
    expect(root.querySelector('.note')!.textContent).toBe("Couldn't get an answer");
    expect(buttons().map((b) => b.dataset.cmd)).toEqual(['answerRetry']);
    // Retry starts the turn over.
    view.update(answer({ turns: [turn({ state: 'pending' })] }));
    expect([root.querySelector('.note'), root.querySelector('.aa')]).toEqual([null, null]);
  });

  it('a refusal and a failure; a failure offers Retry', () => {
    // A refused turn comes with no buttons (the core clears them): nothing to paste.
    view.update(answer({ turns: [turn({ state: 'refused' })], actions: [] }));
    expect(root.querySelector('.note')!.textContent).toBe("Can't answer that");
    expect(buttons()).toHaveLength(0);
    view.update(answer({ turns: [turn({ state: 'failed' })] }));
    expect(root.querySelector('.note')!.textContent).toBe("Couldn't get an answer");
    expect(buttons().map((b) => [b.dataset.cmd, b.disabled])).toEqual([['answerRetry', false]]);
    buttons()[0].click();
    expect(acts).toEqual(['answerRetry']);
  });

  it('no markup from the model reaches the page', () => {
    view.update(answer({ turns: [done('<img src=x onerror="alert(1)"> **<b>x</b>**', { question: '<b>q</b>' })] }));
    revealAll();
    expect(root.querySelector('img')).toBeNull();
    expect(root.querySelector('.aa b b')).toBeNull();
    expect(root.querySelector('.aa')!.textContent).toBe('<img src=x onerror="alert(1)"> <b>x</b>');
    expect(root.querySelector('.aq')!.textContent).toBe('<b>q</b>');
  });

  it('outside the core\'s answer phase the buttons and the cross wait; the missed row\'s Retry does not', () => {
    view.update(answer({ missed: true, turns: [done('x')] }));
    revealAll();
    const x = root.querySelector<HTMLButtonElement>('[data-x]')!;
    view.ready(false);
    view.frame(99_999);
    expect([x.disabled, ...buttons().map((b) => b.disabled)]).toEqual([true, true, true]);
    x.click();
    buttons()[0].click();
    const retry = root.querySelector<HTMLButtonElement>('.lsn.miss button')!;
    expect(retry.disabled).toBe(false);
    retry.click();
    expect(acts).toEqual(['retry']);
    view.update(answer({ turns: [turn({ state: 'failed' })] }));
    expect(buttons().map((b) => [b.dataset.cmd, b.disabled])).toEqual([['answerRetry', true]]);
    view.ready(true);
    expect([x.disabled, ...buttons().map((b) => b.disabled)]).toEqual([false, false]);
    buttons()[0].click();
    expect(acts).toEqual(['retry', 'answerRetry']);
  });

  it('the view follows the end: the sources come into view, then a follow-up\'s listening row; with reduced motion too', () => {
    const a = answer({
      sources: [{ n: 1, site: 's.io', title: 'S' }],
      turns: [turn({ state: 'done', segments: [{ text: many, cites: [1] }] })],
    });
    for (const rm of [false, true]) {
      view.update(null);
      reduced = rm;
      view.update(a);
      const s = scroller();
      let t = 10_000;
      do {
        view.frame(t);
        s.settle();
        t += REVEAL_EVERY_MS;
      } while (view.revealing());
      view.frame(t);
      s.settle();
      expect(root.querySelector('.src'), `reduced: ${rm}`).not.toBeNull();
      expect(s.atEnd(), `sources, reduced: ${rm}`).toBe(true);
      view.update({ ...a, listening: true });
      s.settle();
      expect(root.querySelector('.lsn'), `reduced: ${rm}`).not.toBeNull();
      expect(s.atEnd(), `listening, reduced: ${rm}`).toBe(true);
    }
  });

  it('only the user\'s scroll lets go of the end, never the view\'s own', () => {
    const streaming = (text: string) => answer({ turns: [turn({ state: 'streaming', segments: [{ text, cites: [] }] })] });
    view.update(streaming(many));
    const s = scroller();
    revealAll();
    // The event of the view's own scroll comes once more is in: still following.
    s.grow(50);
    s.settle();
    view.update(streaming(`${many} more`));
    expect(s.atEnd()).toBe(true);
    // The user scrolls up: the view stays there.
    s.user(100);
    view.update(streaming(`${many} more words`));
    revealAll();
    expect(s.top()).toBe(100);
    // Back at the end: following again.
    s.user(1e6);
    view.update(streaming(`${many} more words still`));
    revealAll();
    expect(s.atEnd()).toBe(true);
  });

  it('a new answer follows the end again, however the last one was scrolled', () => {
    view.update(answer({ turns: [done(many)] }));
    let s = scroller();
    revealAll();
    s.user(0);
    view.update(null);
    view.update(answer({ turns: [done(many)] }));
    s = scroller();
    revealAll();
    expect(s.atEnd()).toBe(true);
  });

  it('with reduced motion the marks take their pose at once, like the Drop\'s', () => {
    const cursorX = () => Number(root.querySelector('.am .m-l')!.getAttribute('x1'));
    view.update(answer({ turns: [done('x')] }));
    view.frame(0);
    // The idle mark's first voice bar stands at x 4.5.
    expect(cursorX(), 'the soft spring is still on its way from the asterisk').not.toBe(4.5);
    view.update(null);
    reduced = true;
    view.update(answer({ turns: [done('x')] }));
    view.frame(0);
    expect(cursorX()).toBe(4.5);
    view.update(answer({ listening: true, turns: [done('x')] }));
    view.level(1);
    view.frame(16);
    // The voice bars are lines 0, 2 and 4 of the mark.
    const middleBar = root.querySelectorAll('.lsn .m-l')[2];
    expect(Number(middleBar.getAttribute('y1'))).toBeCloseTo(6.1875, 2);
  });

  it('the cross asks the core to close; null empties the view', () => {
    view.update(answer());
    root.querySelector<HTMLButtonElement>('[data-x]')!.click();
    expect(acts).toEqual(['answerClose']);
    view.update(null);
    expect(root.children).toHaveLength(0);
  });
});
