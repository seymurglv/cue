// The welcome window's page (spec 4a §1): one step at a time — Hello, Keys, Microphone, First dictation, Done — with
// the step dots, Back and one main button at the bottom; a new step slides in from the side. Ported from the approved
// design (stage 6, the welcome window only): Windows draws the frame, so the page draws none. The page draws nothing
// until the core has told it the settings (the language is SettingsView.lang); the theme follows Windows.
// The core holds the hotkey from the window's creation: the page holds it again on load (a reload starts from a known
// state), lets it go on First dictation and holds it again when leaving that step. The microphone is open only while
// the Microphone step is shown. A key typed into the Keys step goes to the core once and leaves the page.
import '../design/cue.css';
import '../design/window.css';
import './welcome.css';
import '../design/cue.js';
import '../design/drop.js';
import * as api from '../window/api';
import type { KeyName, LinkTarget, MicAccess, ModelState, SettingChange, SettingsView } from '../window/api';
import { CUE } from '../designParts';
import { KeyRows } from '../window/keys';
import { MicRows } from '../window/mic';
import { modelRow, stepModelRow } from '../window/model';
import { Menus, codeOf, h, logFailed, relRect, type MenuItem } from '../window/page';
import { TEXTS, pickLang, type Lang, type WinText } from '../window/text';
import { STEPS, keysReady, stepPage, stepsFor, wordCount, type Step } from './steps';
import { WELCOME_TEXTS, type WelcomeText } from './text';

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)';
const DARK = '(prefers-color-scheme: dark)';
/** The Hello mark holds each pose this long (the design's). */
export const HELLO_POSE_MS = 1500;
/** From the first text in the First dictation field to Done (the design's). */
export const TYPED_TO_DONE_MS = 1500;
/** The Done mark thinks this long before its check (the design's). */
const DONE_THINK_MS = 420;
const HELLO_POSES = ['idle', 'listen', 'think', 'done'];

export interface WelcomeOptions {
  /** Reduced motion; by default the system setting. */
  reduced?: () => boolean;
}

/** The design's burst on Done: lines and dots of the mark flying out of (`cx`, `cy`). */
function burst(host: HTMLElement, cx: number, cy: number, n = 30, dist = 90): void {
  const colors = ['var(--rec)', 'var(--ask)', 'var(--ok)', 'var(--warn)'];
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2 + Math.random() * 0.25;
    const d = dist * 0.55 + Math.random() * dist * 0.6;
    const line = i % 3 !== 0;
    const p = h('i', line ? 'bp' : 'bp dot');
    Object.assign(p.style, { left: `${cx}px`, top: `${cy}px`, background: line ? 'currentColor' : colors[i % 4] });
    host.append(p);
    const at = (y: number, s: number) => `translate(-50%,-50%) rotate(${a}rad) translateY(${y}px) scaleY(${s})`;
    p.animate(
      [
        { transform: at(-8, 0.3), opacity: 1 },
        { transform: at(-d, 1), opacity: 1, offset: 0.55 },
        { transform: at(-d - 12, 0.2), opacity: 0 },
      ],
      { duration: 1000 + Math.random() * 400, easing: 'cubic-bezier(.2,.8,.3,1)' },
    ).onfinish = () => p.remove();
  }
}

export class WelcomePage {
  /** The key and microphone rows' texts (the settings tray's). */
  t: WinText = TEXTS.en;
  text: WelcomeText = WELCOME_TEXTS.en;
  lang: Lang = 'en';
  settings: SettingsView | null = null;
  /** The speech model's state, followed from `cue://model`. */
  model: ModelState | null = null;
  readonly keys: KeyRows;
  readonly mic: MicRows;
  readonly reduced: () => boolean;
  /** The steps for the settings (`stepsFor`). */
  private steps: readonly Step[] = STEPS;
  /** The step shown, by its place in `steps`. */
  at = 0;
  /** The words of the first dictation. */
  words = 0;
  private readonly menus: Menus;
  private body!: HTMLElement;
  private dots!: HTMLElement;
  private back!: HTMLButtonElement;
  private pri!: HTMLButtonElement;
  /** The step's page shown (pages sliding out are on their way out). */
  private current: HTMLElement | null = null;
  /** What leaving the step undoes: its timers, its live marks. */
  private undo: (() => void)[] = [];
  private marks: any[] = [];
  private looping = false;
  private readonly voice = new (CUE().Voice)();
  /** Windows' microphone access, and whether this step's preview failed. */
  private access: MicAccess | null = null;
  private failed = false;
  private typed = false;
  private readonly off = new AbortController();

  constructor(readonly app: HTMLElement, opts: WelcomeOptions = {}) {
    this.reduced = opts.reduced ?? (() => !!window.matchMedia?.(REDUCED_MOTION)?.matches);
    app.classList.add('app', 'ow');
    this.menus = new Menus(app);
    this.keys = new KeyRows(this, () => this.shown('keys'), () => this.paintPrimary());
    this.mic = new MicRows(this, () => this.shown('mic'), () => this.redrawMic());
    const signal = this.off.signal;
    const dark = window.matchMedia?.(DARK);
    const theme = () => (document.documentElement.dataset.theme = dark?.matches ? 'dark' : 'light');
    theme();
    dark?.addEventListener?.('change', theme, { signal });
    app.addEventListener('click', (e) => this.onClick(e), { signal });
    app.addEventListener('input', (e) => {
      const t = e.target as Element;
      if (t.matches('textarea[data-field]')) this.onTyped();
      // A key typed and not checked yet holds Next (Next would drop it).
      else if (t.matches('input[data-key]')) this.paintPrimary();
    }, { signal });
    app.addEventListener('keydown', (e) => {
      const input = (e.target as Element).closest<HTMLInputElement>('input[data-key]');
      if (input && e.key === 'Enter') {
        e.preventDefault();
        void this.keys.check(input.dataset.key as KeyName);
      }
    }, { signal });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && this.menus.isOpen) {
        this.menus.close();
        e.preventDefault();
      }
    }, { signal });
    const within = (e: Event, sel: string) => e.composedPath().some((n) => n instanceof Element && n.matches(sel));
    document.addEventListener('pointerdown', (e) => {
      if (this.menus.isOpen && !within(e, '.menu, [data-dd]')) this.menus.close();
    }, { signal });
    window.addEventListener('pagehide', () => this.mic.stopPreview(), { signal });
    // First dictation is pasted where the focus is: on that step a press anywhere but on the field, an enabled button
    // or a link leaves the focus in the field (a disabled button lets the press through to the bar, welcome.css).
    document.addEventListener('mousedown', (e) => {
      const t = e.target instanceof Element ? e.target : null;
      if (this.step === 'try' && !t?.closest('textarea[data-field], button:not(:disabled), a[href]')) e.preventDefault();
    }, { signal });
    window.addEventListener('focus', () => this.regained(), { signal });
    document.addEventListener('visibilitychange', () => {
      if (!document.hidden) this.focusField();
    }, { signal });
  }

  get step(): Step {
    return this.steps[this.at];
  }

  /** The hotkey held and no microphone open, as at the window's creation; then the settings, then the first step. */
  async start(): Promise<void> {
    api.welcomeStep(false).catch((e) => logFailed('holding the hotkey', e));
    api.micPreview(false).catch((e) => logFailed('stopping the microphone level', e));
    const view = await api.settingsGet().catch((e) => {
      logFailed('reading the settings', e);
      return null;
    });
    this.settings = view;
    this.steps = stepsFor(view);
    this.lang = pickLang(view?.lang);
    this.t = TEXTS[this.lang];
    this.text = WELCOME_TEXTS[this.lang];
    document.documentElement.lang = this.lang;
    this.build();
    // The model downloads while the steps go on: its step redraws its block, and moves on by itself once it is ready.
    api.onModel((m) => this.followModel(m)).catch((e) => logFailed('following the speech model', e));
    api.modelGet().then((m) => this.followModel(m), (e) => logFailed('reading the speech model', e));
    // For the page's whole life: the window may listen but not stop listening. Only the Microphone step uses them.
    api.onMicLevel((level) => {
      if (this.step === 'mic') this.mic.level(level);
    }).catch((e) => logFailed('following the microphone level', e));
    api.onMicFailed(() => {
      if (this.step !== 'mic') return;
      this.failed = true;
      this.paintHint();
    }).catch((e) => logFailed('following the microphone', e));
  }

  /** Listeners off (the page is gone); the microphone preview ends. */
  dispose(): void {
    this.off.abort();
    this.leave();
    this.mic.stopPreview();
  }

  menu(button: HTMLElement, items: MenuItem[], pick: (value: string) => void, note?: string): void {
    this.menus.show(button, items, pick, note);
  }

  /** The microphone through the core (the only setting this window changes). */
  async change(c: SettingChange): Promise<SettingsView | null> {
    try {
      this.settings = await api.settingsSet(c);
      // A new preview starts for the new choice.
      this.failed = false;
      this.redrawMic();
      return this.settings;
    } catch (e) {
      logFailed(`setting ${c.kind}`, e);
      // No answer in time: the core may have applied it after the wait — show what it holds now.
      if (codeOf(e) === 'core') {
        await api.settingsGet().then((v) => {
          this.settings = v;
          this.redrawMic();
        }, (err) => logFailed('reading the settings', err));
      }
      return null;
    }
  }

  private build(): void {
    this.app.textContent = '';
    this.body = h('div', 'ob');
    const foot = h('div', 'ofoot');
    this.dots = h('span', 'dots');
    for (const s of this.steps) {
      const dot = h('i');
      dot.title = this.text.steps[s];
      this.dots.append(dot);
    }
    this.back = h('button', 'olink', this.text.back);
    this.back.type = 'button';
    this.back.dataset.back = '';
    this.pri = h('button', 'obtn');
    this.pri.type = 'button';
    this.pri.dataset.pri = '';
    this.pri.append(h('span', 'lbl'));
    foot.append(this.dots, h('span', 'sp'), this.back, this.pri);
    this.app.append(this.body, foot);
    this.enter(0);
  }

  /** The page of `step` while it is the one shown; null otherwise. */
  private shown(step: Step): HTMLElement | null {
    return this.step === step ? this.current : null;
  }

  private go(to: number): void {
    if (to < 0 || to >= this.steps.length || to === this.at) return;
    const dir = to > this.at ? 1 : -1;
    this.leave();
    this.at = to;
    this.enter(dir);
  }

  /** The step shown ends: its timers and marks stop, a key typed and not saved is dropped, the microphone closes, the
   *  hotkey is held again. */
  private leave(): void {
    for (const f of this.undo.splice(0)) f();
    this.menus.close();
    const pg = this.current;
    if (this.step === 'keys') pg?.querySelectorAll<HTMLInputElement>('input[data-key]').forEach((i) => (i.value = ''));
    if (this.step === 'mic') this.mic.stopPreview();
    if (this.step === 'try') api.welcomeStep(false).catch((e) => logFailed('holding the hotkey', e));
  }

  /** The step at `at` starts, its page sliding in from the side of `dir` (1 — forward, -1 — back, 0 — at once). */
  private enter(dir: number): void {
    const step = this.step;
    if (step === 'mic') this.failed = false;
    if (step === 'try') this.typed = false;
    this.show(stepPage(step, this), dir);
    this.paintFoot();
    const pg = this.current!;
    if (step === 'hello') this.hello(pg);
    else if (step === 'keys') void this.keys.readAll();
    else if (step === 'mic') this.lookAtMic();
    else if (step === 'try') {
      api.welcomeStep(true).catch((e) => logFailed('letting the hotkey go', e));
      // The field's border breathes while it waits; with reduced motion it stands still.
      if (!this.reduced()) pg.querySelector('textarea[data-field]')?.classList.add('wait');
      this.focusField();
    } else if (step === 'done') this.done(pg);
  }

  /** The Microphone step's preview starts, and Windows' access and its microphones are read. */
  private lookAtMic(): void {
    this.mic.startPreview();
    void this.mic.read();
    void this.readAccess();
    this.paintHint();
  }

  /** The window has the focus again. On Microphone the user may have come back from the Windows settings: the step
   *  looks again with a new preview (a failed one stays failed). On First dictation the field takes the focus back. */
  private regained(): void {
    if (this.step === 'mic') {
      this.menus.close();
      this.failed = false;
      this.lookAtMic();
    }
    this.focusField();
  }

  /** First dictation's field has the focus, so the dictation is pasted into it. */
  private focusField(): void {
    this.shown('try')?.querySelector<HTMLTextAreaElement>('textarea[data-field]')?.focus();
  }

  /** `pg` in place of the step's page shown, as in the design: the new one in from 46 px (480 ms), the old one out by
   *  40 px (260 ms); at once with reduced motion. */
  private show(pg: HTMLElement, dir: number): void {
    const old = this.current;
    this.body.append(pg);
    this.current = pg;
    if (!old) return;
    old.inert = true;
    if (dir === 0 || this.reduced() || typeof pg.animate !== 'function') {
      old.remove();
      return;
    }
    old.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${-dir * 40}px)` }], {
      duration: 260,
      easing: 'ease-in',
      fill: 'forwards',
    }).onfinish = () => old.remove();
    pg.animate([{ opacity: 0, transform: `translateX(${dir * 46}px)` }, { opacity: 1, transform: 'none' }], {
      duration: 480,
      easing: 'cubic-bezier(.22,1,.36,1)',
    });
  }

  /** The dots (the current one drawn out, the passed ones darker), Back (not on the first step or on Done), the main
   *  button. */
  private paintFoot(): void {
    this.dots.querySelectorAll('i').forEach((d, i) => (d.className = i === this.at ? 'on' : i < this.at ? 'ok' : ''));
    this.back.hidden = this.at === 0 || this.step === 'done';
    this.paintPrimary();
  }

  /** The main button's label (changing by its common letters) and whether it can be pressed now. First dictation waits
   *  for the voice, not a press (its field says so): the button is not shown there, and keeps its place and its label,
   *  so Back does not move. */
  private paintPrimary(): void {
    const x = this.text;
    const label = { hello: x.start, keys: x.next, model: x.next, mic: this.blocked() ? x.next : x.micWorks, try: null, done: x.finish }[
      this.step
    ];
    const lbl = this.pri.querySelector<HTMLElement>('.lbl')!;
    if (label !== null && lbl.dataset.t !== label) CUE().setLabel(lbl, label);
    this.pri.classList.toggle('wait', this.step === 'try');
    this.pri.disabled = this.step === 'try' || (this.step === 'keys' && !keysReady(this.keys));
  }

  private onClick(e: Event): void {
    const t = e.target as Element;
    let b: HTMLElement | null;
    if (t.closest('[data-pri]')) this.primary();
    else if (t.closest('[data-back]')) {
      if (this.at > 0 && this.step !== 'done') this.go(this.at - 1);
    } else if ((b = t.closest<HTMLElement>('[data-check]'))) void this.keys.check(b.dataset.check as KeyName);
    else if ((b = t.closest<HTMLElement>('[data-dd="mic"]'))) this.mic.menu(b);
    else if ((b = t.closest<HTMLElement>('[data-link]'))) {
      api.openLink(b.dataset.link as LinkTarget).catch((err) => logFailed('opening a link', err));
    }
  }

  private primary(): void {
    if (this.pri.disabled) return;
    if (this.step === 'done') api.welcomeClose().catch((e) => logFailed('closing the window', e));
    else this.go(this.at + 1);
  }

  /** A timer of the step shown. */
  private later(f: () => void, ms: number): void {
    const t = setTimeout(f, ms);
    this.undo.push(() => clearTimeout(t));
  }

  /** A live mark on the page's big mark, ticked every frame until the step ends; with reduced motion its spring has
   *  almost no inertia, as the Drop's. */
  private mark(pg: HTMLElement, state: string): any {
    const o = this.reduced() ? { state, k: 20, d: 0.05 } : { state };
    const m = new (CUE().Morph)(pg.querySelector('.bigmk g'), o);
    this.marks.push(m);
    this.undo.push(() => (this.marks = this.marks.filter((x) => x !== m)));
    if (!this.looping) {
      this.looping = true;
      const frame = (now: number) => {
        if (!this.marks.length || this.off.signal.aborted) {
          this.looping = false;
          return;
        }
        this.voice.step(now, true);
        for (const x of this.marks) x.tick(now, this.voice.lvl, this.voice.jit);
        requestAnimationFrame(frame);
      };
      requestAnimationFrame(frame);
    }
    return m;
  }

  /** Hello: the mark goes idle → listen → think → done, a pose every 1.5 s. */
  private hello(pg: HTMLElement): void {
    const m = this.mark(pg, HELLO_POSES[0]);
    let k = 0;
    const t = setInterval(() => {
      k = (k + 1) % HELLO_POSES.length;
      m.set(HELLO_POSES[k], performance.now());
    }, HELLO_POSE_MS);
    this.undo.push(() => clearInterval(t));
  }

  /** Done: the mark thinks, then shows its check with the burst (none with reduced motion). */
  private done(pg: HTMLElement): void {
    const m = this.mark(pg, 'think');
    this.later(() => {
      m.set('done', performance.now());
      const mk = pg.querySelector('.bigmk');
      if (this.reduced() || !mk || typeof this.app.animate !== 'function') return;
      const r = relRect(this.app, mk);
      burst(this.app, r.x + r.w / 2, r.y + r.h / 2);
    }, DONE_THINK_MS);
  }

  /** The first text in the First dictation field: the keys show pressed, and Done follows with its words. */
  private onTyped(): void {
    const pg = this.shown('try');
    if (!pg || this.typed) return;
    this.typed = true;
    const keys = pg.querySelector('.keys');
    keys?.classList.remove('wait');
    keys?.classList.add('down');
    pg.querySelector('textarea[data-field]')?.classList.remove('wait');
    this.later(() => {
      this.words = wordCount(pg.querySelector<HTMLTextAreaElement>('textarea[data-field]')?.value ?? '');
      this.go(this.at + 1);
    }, TYPED_TO_DONE_MS);
  }

  private async readAccess(): Promise<void> {
    try {
      this.access = await api.micAccess();
    } catch (e) {
      logFailed('reading the microphone access', e);
      return;
    }
    this.paintHint();
  }

  /** Windows denies CUE the microphone, the preview failed, or Windows has none. */
  private blocked(): boolean {
    return this.access === 'denied' || this.failed || (this.mic.mics !== null && this.mic.names().length === 0);
  }

  private paintHint(): void {
    const hint = this.shown('mic')?.querySelector<HTMLElement>('.ohint');
    if (!hint) return;
    hint.hidden = !this.blocked();
    this.paintPrimary();
  }

  /** The model's new state: its step's block drawn again, and the step left by itself once the model is ready. */
  private followModel(m: ModelState): void {
    const was = this.model?.state;
    this.model = m;
    const row = this.shown('model')?.querySelector('.mrow');
    if (!row) return;
    if (!stepModelRow(row, m)) row.replaceWith(modelRow(this.t, m));
    if (m.state === 'ready' && was !== 'ready') this.go(this.at + 1);
  }

  /** The microphone rows again: the list came, or the choice changed. */
  private redrawMic(): void {
    const rows = this.shown('mic')?.querySelector('.orows');
    if (!rows) return;
    rows.replaceChildren(...this.mic.rows());
    this.paintHint();
  }
}

/** Builds the page in `root` once the core has told it the settings. */
export async function mountWelcome(root: HTMLElement, opts: WelcomeOptions = {}): Promise<WelcomePage> {
  const page = new WelcomePage(root, opts);
  await page.start();
  return page;
}

const root = document.getElementById('cue-welcome');
if (root) void mountWelcome(root);
