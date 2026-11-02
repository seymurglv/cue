import { AnswerView, type Answer, type AnswerCommand } from './answer';
import { CUE } from './designParts';
import { DICTS, TRAYS, type Dict, type ErrorCause, type Lang, type TrayKind } from './strings';
import { trayHtml, type MicNames } from './trays';

export type Phase = 'idle' | 'start' | 'listen' | 'think' | 'done' | 'empty' | 'cancel' | 'error' | 'answer';
/** What the dictation does with the speech. The core keeps it: the chord picks it (Shift — translate, Alt — ask),
 *  and Shift switches dictation and translation while recording; the Drop only shows it. */
export type Mode = 'dictate' | 'translate' | 'ask';
export interface UiState { phase: Phase; error: ErrorCause | null; retry: boolean; paused: boolean; mode: Mode }
export type UiAction = 'cancel' | 'finish' | 'undo' | 'retry' | 'dismiss' | 'trayAction' | 'trayClose' | AnswerCommand;
export interface Rect { x: number; y: number; w: number; h: number }
/** Where the Drop sits on the screen; the layout follows it (drop.css, [data-place]). */
export type Place = 'bottom' | 'top' | 'left' | 'right';
export const PLACES: readonly Place[] = ['bottom', 'top', 'left', 'right'];
export type DragPhase = 'start' | 'end';

/** The place the core sent; anything else (missing, unknown) is the default, bottom. */
export function pickPlace(v: unknown): Place {
  return PLACES.find((p) => p === v) ?? 'bottom';
}

export interface DropIo {
  send(a: UiAction): void;
  rects(r: Rect[]): void;
  /** The frame loop went to sleep (frame returned false) and something on screen changes again — restart it. */
  wake?(): void;
  /** The Drop is dragged ('start': the core moves the window after the cursor) or let go ('end': the core snaps it to
   *  the nearest place). The core answers every drag, ended or refused, with the place (see place()). */
  drag?(phase: DragPhase): void;
  /** A source chip of the answer was clicked: the core opens its page. */
  source?(n: number): void;
}

/** How long "Copied" stays before the tray card folds back (as in the original design). */
const DONE_HOLD_MS = 1100;
/** The answer tray folds back into the Drop in 430 ms (drop.js): its words stay until then. */
const ANSWER_FOLD_MS = 450;
/** Quiet longer than every transition in drop.css/tray.css (≤ 550 ms plus delays), the mark's shake (460 ms), hiding the
 *  tray card (430 ms) and labels fading in (420 ms + 16 ms per letter). */
const QUIET_AFTER_MS = 1000;
const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';
/** A press on the Drop becomes a drag once the pointer has moved more than this (CSS px) with the button down;
 *  a press that moves less is an ordinary click. */
export const DRAG_THRESHOLD = 4;

export class DropController {
  private state: UiState = { phase: 'idle', error: null, retry: false, paused: false, mode: 'dictate' };
  private voice: any;
  private drop: any;
  private trayBox: any;
  private dropEl: HTMLElement;
  private trayEl: HTMLElement;
  private answerBox: any;
  private answerEl: HTMLElement;
  private view: AnswerView;
  /** Answer tray generation: changes when it opens and when it closes, so an old fold timer leaves a newer answer alone. */
  private answerGen = 0;
  /** A card answer is open: the chord's shake is the card's (its own window). */
  private cardOpen = false;
  private lastRects = '';
  private fullscreen = false;
  private t: Dict;
  /** Tray card generation: changes with every new card and every close. Old timers and buttons check against it. */
  private trayGen = 0;
  /** Time left until the card closes by itself, ms; 0 — no countdown. Runs only while the mouse is not over the card. */
  private ttlLeft = 0;
  private ttlFrom = 0;
  private ttlTimer: ReturnType<typeof setTimeout> | null = null;
  private trayHovered = false;
  /** The names the missing microphone's card shows: the core sends them just before it. */
  private mic: MicNames | null = null;
  private model: string | null = null;
  /** Quiet mode: false — the frame loop is asleep. calmSince — the frame since which nothing on screen can change. */
  private awake = true;
  private calmSince: number | null = null;
  /** The primary button went down on the open idle Drop: where, for the drag threshold. */
  private press: { id: number; x: number; y: number } | null = null;
  /** The Drop is being dragged: the core moves the window, the pointer is captured. */
  private dragging = false;
  private captured: number | null = null;

  /** place: the core's place at startup (any value reads as in place()); set before the Drop is built, so the first
   *  frame is already laid out for it and nothing slides into place. */
  constructor(private host: HTMLElement, private io: DropIo, lang: Lang = 'en', place: unknown = 'bottom') {
    this.t = DICTS[lang];
    this.voice = new (CUE().Voice)();
    // Without a level from the core Voice would draw made-up syllables; the bars show only what the core sent.
    this.voice.ext = 0;
    const api = {
      mode: () => this.state.mode ?? 'dictate',
      // The cause icon: a failed translation shows the network one; the key problems show none.
      // The network icon where the network is the cause: the model's download, a translation.
      err: () => (this.state.error === 'model' || this.state.error === 'translate' ? 'net' : 'key'),
      errLabel: () => this.t.error[this.state.error ?? 'stt'],
      text: this.t.drop,
      retry: () => this.state.retry,
      lbl: () => 'icons',
      os: () => 'win',
      voice: this.voice,
      onEsc: () => this.io.send(this.state.phase === 'error' ? 'dismiss' : 'cancel'),
      onPress: () => this.io.send('finish'),
      onAction: () => this.io.send(this.state.phase === 'cancel' ? 'undo' : 'retry'),
    };
    this.host.dataset.place = pickPlace(place);
    this.drop = new (CUE().Drop)(host, api);
    this.trayBox = new (CUE().Tray)(host, this.drop, { width: 300 });
    this.dropEl = this.drop.p;
    this.trayEl = this.trayBox.box;
    this.answerBox = new (CUE().Tray)(host, this.drop, { width: 420, cls: 'ansA' });
    this.answerEl = this.answerBox.box;
    this.view = new AnswerView(
      this.answerBox.in,
      {
        act: (a) => this.io.send(a),
        source: (n) => this.io.source?.(n),
        resized: () => {
          if (this.answerBox.isOpen) this.answerBox.fit();
        },
        wake: () => this.wake(),
      },
      this.t,
      () => !!window.matchMedia?.(REDUCED_MOTION)?.matches,
    );
    this.view.ready(this.state.phase === 'answer');
    const wake = () => this.wake();
    this.dropEl.addEventListener('mouseenter', wake);
    this.dropEl.addEventListener('mouseleave', wake);
    // Hovering the tray card pauses the countdown (CSS pauses the ring on the close button itself, .tray:hover).
    this.trayEl.addEventListener('mouseenter', () => { this.trayHovered = true; this.pauseTtl(); this.wake(); });
    this.trayEl.addEventListener('mouseleave', () => { this.trayHovered = false; this.runTtl(); this.wake(); });
    // drop.js changes the mark's spring with this setting — frames are needed for the mark to settle the new way.
    window.matchMedia?.(REDUCED_MOTION)?.addEventListener?.('change', wake);
    // Dragging: only the open idle Drop (under the mouse), with the primary mouse button.
    this.dropEl.addEventListener('pointerdown', (e) => this.onPointerDown(e));
    this.dropEl.addEventListener('pointermove', (e) => this.onPointerMove(e));
    for (const type of ['pointerup', 'pointercancel', 'lostpointercapture'] as const) {
      this.dropEl.addEventListener(type, (e) => this.onPointerUp(e));
    }
    // While dragged the Drop keeps its open look even if the window lags a frame behind the cursor. This listener runs
    // before the one in drop.js: capture listeners run first at the target.
    this.dropEl.addEventListener('mouseleave', (e) => { if (this.dragging) e.stopImmediatePropagation(); }, true);
  }

  /** Any event: the quiet period starts over; a sleeping frame loop is started. */
  private wake() {
    this.calmSince = null;
    if (this.awake) return;
    this.awake = true;
    this.io.wake?.();
  }

  apply(s: UiState) {
    const prev = this.state.phase;
    const prevMode = this.state.mode ?? 'dictate';
    this.state = s;
    // Dragging is for the idle Drop only: a dictation or a pause ends it, and the core snaps the Drop to a place.
    if (s.phase !== 'idle' || s.paused) this.endDrag(true);
    // A new recording does not drift to the last level of the previous one, and the level does not linger after recording.
    if (s.phase !== prev && (s.phase === 'listen' || prev === 'listen')) this.resetVoice();
    // A new mode during start-up or recording re-enters the phase: drop.js picks the mark's shape and icon by mode there.
    // An open answer rests the Drop: it looks idle (and steps aside while the answer is in the tray).
    if (s.phase !== prev || s.phase === 'error' || (s.mode ?? 'dictate') !== prevMode) this.drop.enter(s.phase === 'answer' ? 'idle' : s.phase);
    else this.drop.layout();
    // The core takes the answer's buttons only in its answer phase; during a follow-up they wait.
    this.view.ready(s.phase === 'answer');
    this.syncIdle();
    this.wake();
  }

  /** The core's place for the Drop; the layout follows (drop.css). It is also the core's answer to every drag, so a
   *  drag the page still thinks is going on is over. */
  place(p: unknown) {
    this.host.dataset.place = pickPlace(p);
    this.endDrag(false);
    this.wake();
  }

  private resetVoice() {
    this.voice.ext = 0;
    this.voice.tgt = 0;
    this.voice.lvl = 0;
  }

  /** The idle bar is hidden while paused and over full-screen apps; it does not catch the mouse then. */
  private syncIdle() {
    document.body.classList.toggle('idle-hidden', this.fullscreen || this.state.paused);
  }

  /** The Drop can be picked up: idle, shown (not paused, not over a full-screen app), open under the mouse, no tray card. */
  private draggable(): boolean {
    return this.state.phase === 'idle' && !this.state.paused && !this.fullscreen && this.drop.hov
      && this.trayEl.hidden && !this.trayBox.isOpen && !this.answerBox.isOpen;
  }

  private onPointerDown(e: PointerEvent) {
    this.press = null;
    if (e.button !== 0 || e.pointerType !== 'mouse' || !e.isPrimary || this.dragging || !this.draggable()) return;
    this.press = { id: e.pointerId, x: e.clientX, y: e.clientY };
    this.wake();
  }

  private onPointerMove(e: PointerEvent) {
    const p = this.press;
    if (!p || e.pointerId !== p.id || this.dragging) return;
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) <= DRAG_THRESHOLD) return;
    if (!this.draggable()) {
      this.press = null;
      return;
    }
    // Only now, past the threshold: a press without movement stays an ordinary click, untouched.
    this.dragging = true;
    try {
      this.dropEl.setPointerCapture?.(e.pointerId);
      this.captured = e.pointerId;
    } catch {
      // The pointer is already up: the core sees the button up and ends the drag itself.
    }
    document.body.classList.add('dragging');
    this.io.drag?.('start');
    this.wake();
  }

  private onPointerUp(e: PointerEvent) {
    const p = this.press;
    if (!p || e.pointerId !== p.id) return;
    this.press = null;
    this.endDrag(true);
  }

  /** tellCore: the page ends the drag (button up, the Drop left idle, a tray card came); otherwise the core already has. */
  private endDrag(tellCore: boolean) {
    this.press = null;
    if (!this.dragging) return;
    this.dragging = false;
    document.body.classList.remove('dragging');
    const id = this.captured;
    this.captured = null;
    if (id != null) {
      try {
        if (this.dropEl.hasPointerCapture?.(id)) this.dropEl.releasePointerCapture(id);
      } catch {
        // Already released.
      }
    }
    if (tellCore) this.io.drag?.('end');
    this.wake();
  }

  level(v: number) {
    this.voice.ext = Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
    this.view.level(this.voice.ext);
    this.wake();
  }

  /** The pinned microphone that is not there and the one recording instead, for the card that follows. */
  micNames(n: MicNames) {
    this.mic = { wanted: String(n?.wanted ?? ''), used: String(n?.used ?? '') };
  }

  /** The name of the speech model just switched to, for the card that follows. */
  modelName(name: string) {
    this.model = String(name ?? '');
  }

  /** One notice at a time: a new one replaces the content and starts its own countdown; null — the core removes the card. */
  tray(kind: TrayKind | null) {
    // Markup first, before touching the open card: if it throws, the old card keeps its timer and close button.
    const html = kind == null ? null : trayHtml(kind, this.t, this.mic, this.model);
    // No tray card while dragging: the drag ends first.
    if (html != null) this.endDrag(true);
    this.stopTtl();
    const gen = ++this.trayGen;
    this.wake();
    if (kind == null || html == null) {
      this.trayBox.close();
      setTimeout(() => this.wake(), ANSWER_FOLD_MS);
      return;
    }
    // A hidden card cannot be under the mouse; an open one keeps its hover (the new ring stands still too).
    if (this.trayEl.hidden) this.trayHovered = false;
    const root: HTMLElement = this.trayBox.open(html);
    root.querySelector('.xb')?.addEventListener('click', () => {
      if (this.closeTray(gen)) this.io.send('trayClose');
    });
    const act = root.querySelector('[data-act]') as HTMLButtonElement | null;
    act?.addEventListener('click', () => {
      if (gen !== this.trayGen) return;
      this.stopTtl();
      this.io.send('trayAction');
      const done = this.t.tray[kind].done;
      if (done) {
        CUE().setLabel(act.querySelector('.lbl'), done);
        act.querySelector('.ai')!.innerHTML = CUE().IC.ok;
        act.disabled = true;
        setTimeout(() => this.closeTray(gen), DONE_HOLD_MS);
      } else this.closeTray(gen);
    });
    this.ttlLeft = TRAYS[kind].ttl;
    this.runTtl();
  }

  /** Closes the card if it is still the same generation. true — closed now. */
  private closeTray(gen: number): boolean {
    if (gen !== this.trayGen) return false;
    this.stopTtl();
    this.trayGen++;
    this.trayBox.close();
    setTimeout(() => this.wake(), ANSWER_FOLD_MS);
    this.wake();
    return true;
  }

  private runTtl() {
    if (this.ttlLeft <= 0 || this.ttlTimer != null || this.trayHovered) return;
    const gen = this.trayGen;
    this.ttlFrom = performance.now();
    this.ttlTimer = setTimeout(() => {
      this.ttlTimer = null;
      this.ttlLeft = 0;
      if (this.closeTray(gen)) this.io.send('trayClose');
    }, this.ttlLeft);
  }

  private pauseTtl() {
    if (this.ttlTimer == null) return;
    clearTimeout(this.ttlTimer);
    this.ttlTimer = null;
    this.ttlLeft = Math.max(1, this.ttlLeft - (performance.now() - this.ttlFrom));
  }

  private stopTtl() {
    if (this.ttlTimer != null) clearTimeout(this.ttlTimer);
    this.ttlTimer = null;
    this.ttlLeft = 0;
  }

  /** The "Ask" answer from the core (spec 2 §7). A tray answer grows out of the Drop toward the screen center, pinned like
   *  the tray cards (drop.css, [data-place]); a card answer is the card window's; null — the answer closed. */
  answer(a: Answer | null) {
    this.cardOpen = a?.place === 'card';
    if (a?.place === 'tray') {
      if (!this.answerBox.isOpen) {
        this.endDrag(true);
        this.answerGen++;
        // The last answer's words stay until its fold is over (and a little after): a new one starts from an empty view,
        // so its words are revealed anew.
        this.view.update(null);
        this.answerBox.open(null);
      }
      this.view.update(a);
    } else if (this.answerBox.isOpen) {
      const gen = ++this.answerGen;
      this.answerBox.close();
      setTimeout(() => {
        if (gen === this.answerGen) this.view.update(null);
      }, ANSWER_FOLD_MS);
      setTimeout(() => this.wake(), ANSWER_FOLD_MS);
    }
    this.wake();
  }

  /** A chord that only shakes: the answer's header while the answer tray is open (the tray's own transform holds its
   *  place), nothing here while a card answer is open (the card window shakes it), the Drop otherwise. */
  shake() {
    if (this.cardOpen) return;
    const head = this.answerBox.isOpen ? (this.answerBox.in.querySelector('.ah') as HTMLElement | null) : null;
    const d = head ?? this.dropEl;
    d.classList.remove('shake');
    void d.offsetWidth;
    d.classList.add('shake');
    this.wake();
  }

  idleHidden(b: boolean) {
    this.fullscreen = b;
    if (b) this.endDrag(true);
    this.syncIdle();
    this.wake();
  }

  /** Every frame: voice and mark; on a change — the rectangles for click-through.
   *  false — nothing on screen can change any more (quiet mode): the loop may sleep until wake. */
  frame(now: number): boolean {
    const answering = this.view.frame(now);
    this.voice.step(now, this.state.phase === 'listen');
    this.drop.frame(now);
    // Both trays step the Drop aside, and each one's close timer (drop.js, 430 ms) brings it back even while the other is
    // still open: the frame puts it right. A tray still folding counts too, so the Drop comes back only once it is folded.
    this.drop.away(!this.trayEl.hidden || !this.answerEl.hidden);
    const rects: Rect[] = [];
    // In the answer phase the Drop is drawn at rest and hides the same way: its rectangle does not go out.
    const idleHidden = (this.state.phase === 'idle' || this.state.phase === 'answer') && (this.fullscreen || this.state.paused);
    // While dragged the whole window takes the mouse: the window follows the cursor a frame late and must not let a
    // click through to the app below in that frame.
    if (this.dragging) rects.push({ x: 0, y: 0, w: window.innerWidth, h: window.innerHeight });
    else if (!idleHidden && !this.dropEl.classList.contains('away')) pushRect(rects, this.dropEl);
    if (!this.trayEl.hidden) pushRect(rects, this.trayEl);
    if (!this.answerEl.hidden) pushRect(rects, this.answerEl);
    const key = JSON.stringify(rects);
    if (key !== this.lastRects) {
      this.lastRects = key;
      this.io.rects(rects);
    }
    // The latest rectangles were sent above — click-through stays correct while asleep.
    const calm = (this.state.phase === 'idle' || this.state.phase === 'error' || this.state.phase === 'answer') && !answering
      && !this.drop.hov && !this.trayBox.isOpen && this.trayEl.hidden && !this.dragging && this.press == null;
    if (!calm) {
      this.calmSince = null;
      return true;
    }
    if (this.calmSince == null) this.calmSince = now;
    if (now - this.calmSince < QUIET_AFTER_MS) return true;
    this.awake = false;
    return false;
  }
}

/** Element rectangle, rounded outwards to whole px: fractional jitter during transitions sends no new messages. */
export function pushRect(out: Rect[], el: HTMLElement) {
  const r = el.getBoundingClientRect();
  if (!(r.width > 0 && r.height > 0)) return;
  const x = Math.floor(r.x), y = Math.floor(r.y);
  out.push({ x, y, w: Math.ceil(r.x + r.width) - x, h: Math.ceil(r.y + r.height) - y });
}
