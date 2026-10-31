import { describe, expect, it } from 'vitest';
import { STACK_GAP_MS, STACK_MIN, stackRuns } from '../src/window/history';
import { appendDiff, appendHeard, clock, diffWords, findHit, oggDuration, wordKey } from '../src/window/said';
import { TODAY, entry, oggFile } from './windowFixture';

const at = (hms: string, app: string, text = hms) => entry(`${TODAY}T${hms}.000-1`, text, { app });
/** The runs as their entries' times. */
const runs = (es: ReturnType<typeof at>[]) => stackRuns(es).map((r) => r.map((e) => e.id.slice(11, 19)).join(' '));

describe('Stacks', () => {
  it('a run of three or more from one app is a stack; two are not', () => {
    expect(STACK_MIN).toBe(3);
    expect(runs([at('11:58:00', 'Telegram'), at('11:56:00', 'Telegram')])).toEqual(['11:58:00', '11:56:00']);
    expect(runs([at('11:58:00', 'Telegram'), at('11:56:00', 'Telegram'), at('11:53:00', 'Telegram')])).toEqual([
      '11:58:00 11:56:00 11:53:00',
    ]);
  });

  it('each entry at most 10 minutes after the one before it, by the time in its id', () => {
    expect(STACK_GAP_MS).toBe(600_000);
    // Exactly ten minutes apart: still one stack.
    expect(runs([at('12:20:00', 'Mail'), at('12:10:00', 'Mail'), at('12:00:00', 'Mail')])).toEqual([
      '12:20:00 12:10:00 12:00:00',
    ]);
    // A second more breaks it — also where the minutes alone (10:10 after 10:00) would still say ten.
    expect(runs([at('10:20:00', 'Mail'), at('10:10:59', 'Mail'), at('10:00:00', 'Mail')])).toEqual([
      '10:20:00',
      '10:10:59',
      '10:00:00',
    ]);
    // The gap is between neighbours, not from the newest: a long run of close entries stays one stack.
    const long = ['11:40:00', '11:31:00', '11:22:00', '11:13:00', '11:04:00'].map((t) => at(t, 'Notion'));
    expect(stackRuns(long)).toHaveLength(1);
  });

  it('another app breaks a stack, and an entry whose app is not known never stacks', () => {
    expect(
      runs([
        at('11:59:00', 'Telegram'),
        at('11:58:00', 'Telegram'),
        at('11:57:00', 'Chrome'),
        at('11:56:00', 'Telegram'),
        at('11:55:00', 'Telegram'),
        at('11:54:00', 'Telegram'),
      ]),
    ).toEqual(['11:59:00', '11:58:00', '11:57:00', '11:56:00 11:55:00 11:54:00']);
    expect(runs([at('11:59:00', ''), at('11:58:30', ''), at('11:58:00', ''), at('11:57:00', '')])).toEqual([
      '11:59:00',
      '11:58:30',
      '11:58:00',
      '11:57:00',
    ]);
    // An id without a time stands alone.
    expect(stackRuns([at('11:59:00', 'X'), entry('nonsense', 'n', { app: 'X' }), at('11:58:00', 'X')])).toHaveLength(3);
  });
});

/** What appendDiff draws, words dropped as [-…-] and added as {+…+}. */
function diff(heard: string, written: string, query = ''): string {
  const box = document.createElement('div');
  appendDiff(box, heard, written, query ? findHit(heard, query) : null);
  const walk = (n: Node): string =>
    [...n.childNodes]
      .map((c) => {
        if (c.nodeType === Node.TEXT_NODE) return c.textContent;
        const inner = walk(c);
        return { S: `[-${inner}-]`, INS: `{+${inner}+}`, MARK: `<${inner}>` }[c.nodeName] ?? inner;
      })
      .join('');
  return walk(box);
}

describe('How it was said', () => {
  it('words are matched without their case and punctuation', () => {
    expect(wordKey('Thursday.')).toBe('thursday');
    expect(wordKey('«Ёлка»,')).toBe('елка');
    expect(wordKey('—')).toBe('');
    expect(diff('so um I think we should uh move the call to thursday', 'So I think we should move the call to Thursday.')).toBe(
      'so [-um-] I think we should [-uh-] move the call to thursday',
    );
  });

  it('a changed word is dropped, then what it became is added', () => {
    expect(diff("we're gonna ship it on friday", "We're going to ship it on Friday.")).toBe(
      "we're [-gonna-] {+going to+} ship it on friday",
    );
    expect(diff('call me', 'Call me tomorrow, please.')).toBe('call me {+tomorrow, please.+}');
    expect(diff('um uh', '')).toBe('[-um uh-]');
  });

  it('a word that is only punctuation is neither dropped nor added', () => {
    expect(diff('one - two', 'One — two.')).toBe('one - two');
  });

  it('a search hit in the words as heard is marked, in dropped words too', () => {
    expect(diff('confirm the delivery address uh please', 'Confirm the delivery location, please.', 'address')).toBe(
      'confirm the delivery [-<address> uh-] {+location,+} please',
    );
    const box = document.createElement('div');
    appendHeard(box, 'Встретимся в четверг', findHit('Встретимся в четверг', 'ЧЕТВЁРГ'));
    expect(box.querySelector('mark')!.textContent).toBe('четверг');
  });

  it('the comparison drops and adds the fewest words, and gives up past its limit', () => {
    const a = 'a b c d e f'.split(' ');
    const b = 'a x c d y f'.split(' ');
    const steps = diffWords(a, b)!;
    expect(steps.map((s) => s.k)).toEqual(['same', 'del', 'ins', 'same', 'same', 'del', 'ins', 'same']);
    expect(steps.map((s) => (s.k === 'ins' ? b[s.j] : a[s.i]))).toEqual(['a', 'b', 'x', 'c', 'd', 'e', 'y', 'f']);
    expect(diffWords([], [])).toEqual([]);
    expect(diffWords(['a'], [])).toEqual([{ k: 'del', i: 0 }]);
    expect(diffWords([], ['a', 'b'])!.map((s) => s.k)).toEqual(['ins', 'ins']);
    // Fifty words for fifty others: a hundred dropped and added; past the limit — shown as heard, unmarked.
    const many = Array.from({ length: 50 }, (_, i) => `w${i}`);
    const other = Array.from({ length: 50 }, (_, i) => `v${i}`);
    expect(diffWords(many, other, 99)).toBeNull();
    expect(diffWords(['s', ...many, 'e'], ['s', ...other, 'e'], 100)).toHaveLength(102);
    const box = document.createElement('div');
    appendDiff(box, 'x '.repeat(1200).trim(), 'y '.repeat(1200).trim(), null);
    expect(box.querySelector('s, ins')).toBeNull();
    expect(box.textContent).toBe('x '.repeat(1200).trim());
  });

  it('a long dictation with a few fillers is still compared', () => {
    const words = Array.from({ length: 3000 }, (_, i) => `w${i}`);
    const heard = words.map((w, i) => (i % 500 === 0 ? `um ${w}` : w)).join(' ');
    const out = diff(heard, words.join(' '));
    expect(out.match(/\[-um-\]/g)).toHaveLength(6);
  });
});

describe('The recording', () => {
  it('its length is read from the Ogg pages', () => {
    expect(oggDuration(oggFile(3.2))).toBeCloseTo(3.2, 3);
    expect(oggDuration(oggFile(61, 0))).toBeCloseTo(61, 3);
    expect(oggDuration(new ArrayBuffer(10))).toBeNull();
    expect(oggDuration(new TextEncoder().encode('OggS but not really a file at all, no head here').buffer)).toBeNull();
    // No end-of-stream page: unknown.
    const cut = new Uint8Array(oggFile(2));
    cut[cut.length - 31 + 5] = 0;
    expect(oggDuration(cut.buffer)).toBeNull();
  });

  it('shows as m:ss', () => {
    expect(clock(0.4)).toBe('0:01');
    expect(clock(3.2)).toBe('0:03');
    expect(clock(59.6)).toBe('1:00');
    expect(clock(61)).toBe('1:01');
    expect(clock(754)).toBe('12:34');
  });
});
