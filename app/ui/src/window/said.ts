// "How you said it" under an opened history entry: the text as it was heard — for a dictation, word by word against the
// text CUE wrote (words it dropped struck through, words it added or changed in green) — and the entry's recording.
// Words are matched without their case and punctuation. Text goes in as text; the search's place in it is marked.
import { icon } from './icons';
import { h, logFailed } from './page';
import type { WinText } from './text';

/** The most words the comparison drops and adds; a pair further apart than that is shown as heard, unmarked. */
export const DIFF_EDITS = 1000;

/** A word of a text and where it stands in it. */
interface Word {
  start: number;
  end: number;
  key: string;
}

/** A word as the comparison sees it: lowercase, ё read as е, without punctuation ("Hello," and "hello" are one). */
export const wordKey = (w: string) => w.toLowerCase().replace(/ё/g, 'е').replace(/[^\p{L}\p{N}]+/gu, '');

function words(s: string): Word[] {
  const out: Word[] = [];
  for (const m of s.matchAll(/\S+/g)) {
    const at = m.index ?? 0;
    out.push({ start: at, end: at + m[0].length, key: wordKey(m[0]) });
  }
  return out;
}

/** One step from `a` to `b`: a word in both (`a[i]`, `b[j]`), dropped from `a`, or added from `b`. */
export type Step = { k: 'same'; i: number; j: number } | { k: 'del'; i: number } | { k: 'ins'; j: number };

/** The steps from `a` to `b` with the fewest words dropped and added (Myers' O((N+M)·D) comparison, so a long dictation
 *  with a few changes costs little); within a change, what is dropped comes before what is added. null — more than
 *  `maxEdits` words would be dropped and added: too different to mark. */
export function diffWords(a: readonly string[], b: readonly string[], maxEdits = DIFF_EDITS): Step[] | null {
  // The common start and end need no search.
  let s = 0;
  while (s < a.length && s < b.length && a[s] === b[s]) s++;
  let ea = a.length;
  let eb = b.length;
  while (ea > s && eb > s && a[ea - 1] === b[eb - 1]) {
    ea--;
    eb--;
  }
  const n = ea - s;
  const m = eb - s;
  // v[k + off]: how far along `a` the furthest path on diagonal k (x − y) has got. Each round's v (k in −d−1..d+1) is
  // kept for the way back.
  const off = maxEdits + 2;
  const v = new Int32Array(2 * off + 1);
  const trace: Int32Array[] = [];
  let found = -1;
  for (let d = 0; d <= maxEdits && found < 0; d++) {
    trace.push(v.slice(off - d - 1, off + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x = k === -d || (k !== d && v[off + k - 1] < v[off + k + 1]) ? v[off + k + 1] : v[off + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[s + x] === b[s + y]) {
        x++;
        y++;
      }
      v[off + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break;
      }
    }
  }
  if (found < 0) return null;
  // Back from the end: each round is one word dropped (a step along a) or added (along b) after a run of the same.
  const back: Step[] = [];
  let x = n;
  let y = m;
  for (let d = found; d >= 0; d--) {
    const t = trace[d];
    const at = (k: number) => t[k + d + 1];
    const k = x - y;
    const pk = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
    const px = d === 0 ? 0 : at(pk);
    const py = px - pk;
    while (x > px && y > py) {
      x--;
      y--;
      back.push({ k: 'same', i: s + x, j: s + y });
    }
    if (d === 0) break;
    if (x === px) back.push({ k: 'ins', j: s + py });
    else back.push({ k: 'del', i: s + px });
    x = px;
    y = py;
  }
  const out: Step[] = [];
  for (let k = 0; k < s; k++) out.push({ k: 'same', i: k, j: k });
  let dels: Step[] = [];
  let ins: Step[] = [];
  for (const st of back.reverse()) {
    if (st.k === 'del') dels.push(st);
    else if (st.k === 'ins') ins.push(st);
    else {
      out.push(...dels, ...ins, st);
      dels = [];
      ins = [];
    }
  }
  out.push(...dels, ...ins);
  for (let k = 0; k < a.length - ea; k++) out.push({ k: 'same', i: ea + k, j: eb + k });
  return out;
}

/** Where `query` first stands in `text` (any case, ё and е alike), as [start, end); null — not in it. */
export function findHit(text: string, query: string): [number, number] | null {
  const fold = (s: string) => s.toLowerCase().replace(/ё/g, 'е');
  const q = fold(query.trim());
  const low = fold(text);
  if (!q || low.length !== text.length) return null;
  const i = low.indexOf(q);
  return i < 0 ? null : [i, i + q.length];
}

/** `text[start, end)` into `parent`, the part inside `hit` marked. */
function appendPart(parent: HTMLElement, text: string, start: number, end: number, hit: [number, number] | null): void {
  const hs = hit ? Math.max(start, hit[0]) : end;
  const he = hit ? Math.min(end, hit[1]) : end;
  if (hs >= he) {
    parent.append(text.slice(start, end));
    return;
  }
  if (hs > start) parent.append(text.slice(start, hs));
  parent.append(h('mark', '', text.slice(hs, he)));
  if (he < end) parent.append(text.slice(he, end));
}

/** The text as heard, plain (a translation's speech, a question), the search's place marked. */
export function appendHeard(parent: HTMLElement, heard: string, hit: [number, number] | null): void {
  appendPart(parent, heard, 0, heard.length, hit);
}

/** A dictation as heard against the text CUE wrote: its words as heard, those CUE dropped struck through (`s`), the
 *  ones it added or changed to after them (`ins`). A word that is only punctuation is neither dropped nor added. */
export function appendDiff(parent: HTMLElement, heard: string, written: string, hit: [number, number] | null): void {
  const all = words(heard);
  const a = all.filter((x) => x.key);
  const b = words(written).filter((x) => x.key);
  const steps = diffWords(
    a.map((x) => x.key),
    b.map((x) => x.key),
  );
  if (!steps) {
    appendHeard(parent, heard, hit);
    return;
  }
  // The words as drawn: heard ones (kept, dropped, or punctuation of their own) in their order, added ones among them.
  type Item = { k: 'kept' | 'del'; w: Word } | { k: 'ins'; w: Word };
  const items: Item[] = [];
  let next = 0;
  const upTo = (w: Word) => {
    while (next < all.length && all[next].start < w.start) {
      items.push({ k: 'kept', w: all[next] });
      next++;
    }
    next++;
  };
  for (const st of steps) {
    if (st.k === 'ins') {
      items.push({ k: 'ins', w: b[st.j] });
    } else {
      upTo(a[st.i]);
      items.push({ k: st.k === 'same' ? 'kept' : 'del', w: a[st.i] });
    }
  }
  while (next < all.length) items.push({ k: 'kept', w: all[next++] });

  let box: HTMLElement | null = null;
  let boxKind = '';
  items.forEach((it, n) => {
    if (n > 0) (it.k !== 'kept' && it.k === boxKind && box ? box : parent).append(' ');
    if (it.k === 'kept') {
      box = null;
      boxKind = '';
      appendPart(parent, heard, it.w.start, it.w.end, hit);
      return;
    }
    if (it.k !== boxKind || !box) {
      box = h(it.k === 'del' ? 's' : 'ins');
      boxKind = it.k;
      parent.append(box);
    }
    if (it.k === 'del') appendPart(box, heard, it.w.start, it.w.end, hit);
    else box.append(written.slice(it.w.start, it.w.end));
  });
}

/** How long an Ogg Opus recording plays, in seconds: the last page's position less the pre-skip, at 48 kHz (RFC 7845);
 *  null — not one CUE wrote (no OpusHead first, no last page). */
export function oggDuration(bytes: ArrayBuffer): number | null {
  const b = new Uint8Array(bytes);
  const at = (o: number, s: string) => [...s].every((c, k) => b[o + k] === c.charCodeAt(0));
  if (b.length < 47 || !at(0, 'OggS')) return null;
  const head = 27 + b[26];
  if (head + 19 > b.length || !at(head, 'OpusHead')) return null;
  const preSkip = b[head + 10] | (b[head + 11] << 8);
  const view = new DataView(bytes);
  for (let o = b.length - 27; o > 0; o--) {
    // The last page: "OggS", version 0, the end-of-stream flag.
    if (b[o] !== 0x4f || !at(o, 'OggS') || b[o + 4] !== 0 || !(b[o + 5] & 4)) continue;
    const pos = view.getUint32(o + 6, true) + view.getUint32(o + 10, true) * 2 ** 32;
    const s = (pos - preSkip) / 48_000;
    return Number.isFinite(s) && s > 0 ? s : null;
  }
  return null;
}

/** Seconds as the player shows them, m:ss. */
export function clock(seconds: number): string {
  const s = Math.max(1, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

/** The recording's type for the page (WebView2 plays Ogg Opus). */
export const AUDIO_TYPE = 'audio/ogg; codecs=opus';

/** An entry's recording: play/pause, a thin line of how far it played, its length. Shown before its bytes come (the
 *  button waits); the bytes become an object URL, given back by `release`. */
export class Player {
  readonly el: HTMLElement;
  private readonly btn: HTMLButtonElement;
  private readonly bar: HTMLElement;
  private readonly len: HTMLElement;
  private audio: HTMLAudioElement | null = null;
  private url = '';
  private seconds = 0;
  private on = false;

  /** `starts` — it is about to play (another one stops). */
  constructor(
    private readonly t: WinText,
    private readonly starts: (p: Player) => void,
  ) {
    this.el = h('div', 'hplay');
    this.btn = h('button', 'hpb');
    this.btn.type = 'button';
    this.btn.disabled = true;
    this.btn.addEventListener('click', () => this.toggle());
    this.bar = h('span', 'hpr');
    this.bar.append(h('i'));
    this.len = h('span', 'hdur');
    this.el.append(this.btn, this.bar, this.len);
    this.paint();
  }

  get playing(): boolean {
    return this.on;
  }

  /** The recording's bytes (an Ogg Opus file): it can play from now on. */
  load(bytes: ArrayBuffer): void {
    if (this.audio) return;
    this.url = URL.createObjectURL(new Blob([bytes], { type: AUDIO_TYPE }));
    const audio = new Audio();
    audio.preload = 'metadata';
    const stopped = () => {
      this.on = false;
      this.paint();
    };
    audio.addEventListener('pause', stopped);
    audio.addEventListener('error', stopped);
    audio.addEventListener('ended', () => {
      stopped();
      this.progress(0);
    });
    // The line moves in steps of about a quarter of a second, eased between them (window.css).
    audio.addEventListener('timeupdate', () => {
      if (this.seconds) this.progress(audio.currentTime / this.seconds);
    });
    audio.addEventListener('durationchange', () => {
      if (!this.seconds && Number.isFinite(audio.duration) && audio.duration > 0) this.length(audio.duration);
    });
    audio.src = this.url;
    this.audio = audio;
    const s = oggDuration(bytes);
    if (s) this.length(s);
    this.btn.disabled = false;
  }

  toggle(): void {
    const audio = this.audio;
    if (!audio) return;
    if (this.on) {
      audio.pause();
      return;
    }
    this.starts(this);
    this.on = true;
    this.paint();
    Promise.resolve(audio.play()).catch((e) => {
      this.on = false;
      this.paint();
      logFailed('playing a recording', e);
    });
  }

  pause(): void {
    if (this.on) this.audio?.pause();
  }

  /** Stopped, its object URL given back; it plays no more. */
  release(): void {
    const audio = this.audio;
    this.audio = null;
    this.on = false;
    if (audio) {
      audio.pause();
      audio.removeAttribute('src');
    }
    if (this.url) URL.revokeObjectURL(this.url);
    this.url = '';
    this.btn.disabled = true;
    this.paint();
  }

  private length(s: number): void {
    this.seconds = s;
    this.len.textContent = clock(s);
  }

  private paint(): void {
    const label = this.on ? this.t.pause : this.t.play;
    this.btn.innerHTML = icon(this.on ? 'pause' : 'play');
    this.btn.title = label;
    this.btn.setAttribute('aria-label', label);
  }

  private progress(p: number): void {
    (this.bar.firstElementChild as HTMLElement).style.transform = `scaleX(${Math.min(1, Math.max(0, p))})`;
  }
}
