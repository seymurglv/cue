import { CUE } from './designParts';
import { DICTS, TRAYS, type Dict, type ErrorCause, type Lang, type TrayKind } from './strings';
import { trayHtml } from './trays';

export type Phase = 'idle' | 'start' | 'listen' | 'think' | 'done' | 'empty' | 'cancel' | 'error';
export interface UiState { phase: Phase; error: ErrorCause | null; retry: boolean; paused: boolean }
export type UiAction = 'cancel' | 'finish' | 'undo' | 'retry' | 'dismiss' | 'trayAction' | 'trayClose';
export interface Rect { x: number; y: number; w: number; h: number }

export interface DropIo {
  send(a: UiAction): void;
  rects(r: Rect[]): void;
  /** The frame loop went to sleep (frame returned false) and something on screen changes again — restart it. */
  wake?(): void;
}

/** How long "Copied" stays before the tray card folds back (as in the original design). */
const DONE_HOLD_MS = 1100;
/** Quiet longer than every transition in drop.css/tray.css (≤ 550 ms plus delays), the mark's shake (460 ms), hiding the
 *  tray card (430 ms) and labels fading in (420 ms + 16 ms per letter). */
const QUIET_AFTER_MS = 1000;
const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';

export class DropController {
  private state: UiState = { phase: 'idle', error: null, retry: false, paused: false };
  private voice: any;
  private drop: any;
  private trayBox: any;
  private dropEl: HTMLElement;
  private trayEl: HTMLElement;
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
  /** Quiet mode: false — the frame loop is asleep. calmSince — the frame since which nothing on screen can change. */
  private awake = true;
  private calmSince: number | null = null;

  constructor(private host: HTMLElement, private io: DropIo, lang: Lang = 'en') {
    this.t = DICTS[lang];
    this.voice = new (CUE().Voice)();
    // Without a level from the core Voice would draw made-up syllables; the bars show only what the core sent.
    this.voice.ext = 0;
    const api = {
      mode: () => 'dictate',
      err: () => (this.state.error === 'net' ? 'net' : 'key'),
      errLabel: () => this.t.error[this.state.error ?? 'net'],
      text: this.t.drop,
      retry: () => this.state.retry,
      lbl: () => 'icons',
      os: () => 'win',
      voice: this.voice,
      onEsc: () => this.io.send(this.state.phase === 'error' ? 'dismiss' : 'cancel'),
      onPress: () => this.io.send('finish'),
      onAction: () => this.io.send(this.state.phase === 'cancel' ? 'undo' : 'retry'),
    };
    this.drop = new (CUE().Drop)(host, api);
    this.trayBox = new (CUE().Tray)(host, this.drop, { width: 300 });
    this.dropEl = this.drop.p;
    this.trayEl = this.trayBox.box;
    const wake = () => this.wake();
    this.dropEl.addEventListener('mouseenter', wake);
    this.dropEl.addEventListener('mouseleave', wake);
    // Hovering the tray card pauses the countdown (CSS pauses the ring on the close button itself, .tray:hover).
    this.trayEl.addEventListener('mouseenter', () => { this.trayHovered = true; this.pauseTtl(); this.wake(); });
    this.trayEl.addEventListener('mouseleave', () => { this.trayHovered = false; this.runTtl(); this.wake(); });
    // drop.js changes the mark's spring with this setting — frames are needed for the mark to settle the new way.
    window.matchMedia?.(REDUCED_MOTION)?.addEventListener?.('change', wake);
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
    this.state = s;
    // A new recording does not drift to the last level of the previous one, and the level does not linger after recording.
    if (s.phase !== prev && (s.phase === 'listen' || prev === 'listen')) this.resetVoice();
    if (s.phase !== prev || s.phase === 'error') this.drop.enter(s.phase);
    else this.drop.layout();
    this.syncIdle();
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

  level(v: number) {
    this.voice.ext = Number.isFinite(v) ? Math.min(1, Math.max(0, v)) : 0;
    this.wake();
  }

  /** One notice at a time: a new one replaces the content and starts its own countdown; null — the core removes the card. */
  tray(kind: TrayKind | null) {
    // Markup first, before touching the open card: if it throws, the old card keeps its timer and close button.
    const html = kind == null ? null : trayHtml(kind, this.t);
    this.stopTtl();
    const gen = ++this.trayGen;
    this.wake();
    if (kind == null || html == null) {
      this.trayBox.close();
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

  shake() {
    const d = this.dropEl;
    d.classList.remove('shake');
    void d.offsetWidth;
    d.classList.add('shake');
    this.wake();
  }

  idleHidden(b: boolean) {
    this.fullscreen = b;
    this.syncIdle();
    this.wake();
  }

  /** Every frame: voice and mark; on a change — the rectangles for click-through.
   *  false — nothing on screen can change any more (quiet mode): the loop may sleep until wake. */
  frame(now: number): boolean {
    this.voice.step(now, this.state.phase === 'listen');
    this.drop.frame(now);
    const rects: Rect[] = [];
    const idleHidden = this.state.phase === 'idle' && (this.fullscreen || this.state.paused);
    if (!idleHidden && !this.dropEl.classList.contains('away')) pushRect(rects, this.dropEl);
    if (!this.trayEl.hidden) pushRect(rects, this.trayEl);
    const key = JSON.stringify(rects);
    if (key !== this.lastRects) {
      this.lastRects = key;
      this.io.rects(rects);
    }
    // The latest rectangles were sent above — click-through stays correct while asleep.
    const calm = (this.state.phase === 'idle' || this.state.phase === 'error')
      && !this.drop.hov && !this.trayBox.isOpen && this.trayEl.hidden;
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
function pushRect(out: Rect[], el: HTMLElement) {
  const r = el.getBoundingClientRect();
  if (!(r.width > 0 && r.height > 0)) return;
  const x = Math.floor(r.x), y = Math.floor(r.y);
  out.push({ x, y, w: Math.ceil(r.x + r.width) - x, h: Math.ceil(r.y + r.height) - y });
}
