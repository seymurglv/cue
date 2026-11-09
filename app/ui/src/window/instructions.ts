// The settings tray's Instructions page (docs/specs/2026-11-08-no-keys-design.md, "Instructions page"; the approved
// mockup's "Ваши инструкции"), one level under Cleanup: the cleanup's style rules, one file the user edits here or with
// an AI.
// - On top, whose they are: CUE's standard or the user's own ("Restore standard"), and in amber with "!" a file that is
//   empty or unreadable (the standard ones then work).
// - The file's text in a box, saved when the box is left (and when the page is left), never while typing: every save
//   is read again by the core and may clean the last dictation once more.
// - "Edit with AI" opens a small menu under it: "Open in <agent>" (the agent chosen for the cleanup, else the first one
//   found) starts it in a terminal in the instructions' folder with CUE's task; "Copy the task" puts it in the clipboard
//   for any AI. A line under it says what happened.
// - When the file is read again (`cue://instructions`, the core's answer to a save or to the page's look while shown),
//   the page takes the new text — unless the user is typing in the box — says "Just now", and once the last dictation
//   is cleaned again with them, shows it before and after.
// - Under it all, the file's path and AGENTS.md.
// The page looks at the file every 1.5 s while it is shown (`instructions_get`): a file an AI saved is read then.
// The Cleanup page's "Your instructions" row takes its line from here (`line`).
import * as api from './api';
import type { AgentView, InstructionsNews, InstructionsSample, InstructionsView } from './api';
import { h, logFailed, type Ctx } from './page';
import { fill } from './text';

/** How often the page looks at the file while it is shown. */
export const LOOK_MS = 1500;

/** The agents "Open in" may start, in the order every list shows them. */
const AGENTS: readonly string[] = ['opencode', 'codex'];

/** The Cleanup row's line: its text, and amber for a broken file. */
export interface InstructionsLine {
  text: string;
  warn: boolean;
}

/** What the line under the menu says: an agent opened (with the task or with it copied), the task copied, or a
 *  failure in amber. */
type Done = { kind: 'opened' | 'openedCopied' | 'notOpened'; name: string } | { kind: 'copied' | 'notSaved' };

export class InstructionsPage {
  /** The instructions as the core last said (null — not heard yet). */
  private view: InstructionsView | null = null;
  private sample: InstructionsSample | null = null;
  /** The core read the file again while the page was shown: "Just now". */
  private fresh = false;
  private menuOpen = false;
  private done: Done | null = null;
  /** The text the box was last filled with or saved as: a box that differs holds an edit not saved yet. */
  private saved = '';
  private box: HTMLTextAreaElement | null = null;
  private root: HTMLElement | null = null;
  /** The page's parts, each filled in its own place. */
  private slots: Record<'head' | 'sample' | 'acts' | 'menu' | 'done' | 'foot', HTMLElement> | null = null;
  /** The parts that came since the last paint: they ease in. */
  private readonly entering = new Set<'sample' | 'menu' | 'done'>();
  private timer: ReturnType<typeof setInterval> | null = null;

  /** `resized` — the page's height changed; `changed` — the Cleanup row's line may have; `agents` — the agents as the
   *  Cleanup page last heard them. */
  constructor(
    private readonly ctx: Ctx,
    private readonly resized: () => void,
    private readonly changed: () => void,
    private readonly agents: () => AgentView[] | null,
  ) {}

  /** The Cleanup row's line: standard, the user's own, or the file broken (amber); empty until the core said. */
  line(): InstructionsLine {
    const l = this.ctx.t.cl.insLine;
    const v = this.view;
    if (!v) return { text: '', warn: false };
    if (v.broken) return { text: l.broken, warn: true };
    return { text: v.edited ? l.edited : l.standard, warn: false };
  }

  /** Asks the core for the instructions (the Cleanup page opened, or this one). */
  refresh(): void {
    api.instructionsGet().then(
      (v) => this.take(v, true),
      (e) => logFailed('reading the instructions', e),
    );
  }

  /** The page is shown: it looks at the file now and every `LOOK_MS`. */
  shown(): void {
    this.fresh = false;
    this.menuOpen = false;
    this.done = null;
    this.refresh();
    this.stopLooking();
    this.timer = setInterval(() => this.refresh(), LOOK_MS);
  }

  /** The page is left (or the tray closed, or the window goes): an edit not saved yet is saved, the looks stop. */
  left(): void {
    this.save();
    this.stopLooking();
    this.root = this.box = this.slots = null;
    this.menuOpen = false;
  }

  private stopLooking(): void {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  /** `cue://instructions`: the file was read again; with `sample`, the last dictation before and after. */
  news(n: InstructionsNews): void {
    if (this.root) this.fresh = true;
    if (n.sample) this.entering.add('sample');
    this.sample = n.sample;
    this.take({ text: n.text, edited: n.edited, broken: n.broken, path: this.view?.path ?? '' });
  }

  /** The instructions as the core says they are: the box takes the text unless an edit is being made in it. */
  private take(v: InstructionsView, quiet = false): void {
    const old = this.view;
    const same = !!old && old.text === v.text && old.edited === v.edited && old.broken === v.broken && old.path === v.path;
    // The page's own look found nothing new: nothing is drawn again.
    if (quiet && same) return;
    const editing = this.typing();
    this.view = { ...v, path: v.path || this.view?.path || '' };
    if (!editing) {
      this.saved = v.text;
      if (this.box) this.box.value = v.text;
    }
    this.changed();
    this.paint();
  }

  /** The user is making an edit: the box has the focus or holds text not saved. */
  private typing(): boolean {
    const b = this.box;
    return !!b && (b.value !== this.saved || document.activeElement === b);
  }

  /** The box's text saved when it differs from what the core holds. */
  private save(): void {
    const b = this.box;
    if (!b || b.value === this.saved) return;
    const text = b.value;
    this.saved = text;
    api.instructionsSet(text).then(
      (v) => {
        this.done = null;
        this.take(v);
      },
      (e) => {
        logFailed('saving the instructions', e);
        this.done = { kind: 'notSaved' };
        this.paint();
      },
    );
  }

  /** The page's rows; the page shown is filled in place from then on (`paint`). */
  rows(): HTMLElement[] {
    const t = this.ctx.t.ins;
    const root = h('div', 'ipg');
    const box = h('textarea', 'pta');
    box.spellcheck = false;
    box.setAttribute('aria-label', t.box);
    box.addEventListener('change', () => this.save());
    box.value = this.saved;
    // Each part has its own place: the box is never moved, so it keeps its focus, its caret and an edit.
    const slot = () => h('div', 'islot');
    this.slots = { head: slot(), sample: slot(), acts: slot(), menu: slot(), done: slot(), foot: slot() };
    const sl = this.slots;
    root.append(sl.head, box, sl.sample, sl.acts, sl.menu, sl.done, sl.foot);
    this.root = root;
    this.box = box;
    this.paint();
    return [root];
  }

  /** The page drawn again where it stands, each part in its place; a part that just came (the menu, the before and
   *  after, the line under the menu) eases in. */
  private paint(): void {
    const sl = this.slots;
    if (!this.root || !sl) return;
    const t = this.ctx.t.ins;
    const v = this.view;
    sl.head.replaceChildren(this.head());
    sl.sample.replaceChildren(...(this.sample ? [this.compare(this.sample)] : []));
    sl.acts.replaceChildren(this.actions());
    sl.menu.replaceChildren(...(this.menuOpen ? [this.menu()] : []));
    const done = this.doneLine();
    sl.done.replaceChildren(...(done ? [done] : []));
    const foot = h('div', 'tft');
    // The long path breaks after a folder's backslash, never inside a name.
    const path = h('span', 'path');
    (v?.path ?? '').split(/(?<=\\)/).forEach((part, i) => path.append(...(i ? [h('wbr'), part] : [part])));
    const [pre, post] = t.foot.split('{PATH}');
    foot.append(pre ?? '', path, post ?? '');
    sl.foot.replaceChildren(foot);
    for (const name of this.entering) {
      const el = sl[name].firstElementChild as HTMLElement | null;
      if (el && !this.ctx.reduced()) {
        el.animate?.([{ opacity: 0, transform: 'translateY(-4px)' }, { opacity: 1, transform: 'none' }], {
          duration: 240,
          easing: 'cubic-bezier(.22,1,.36,1)',
        });
      }
    }
    this.entering.clear();
    this.resized();
  }

  /** Whose the instructions are, and restore for the user's own or a broken file. */
  private head(): HTMLElement {
    const t = this.ctx.t.ins;
    const v = this.view;
    const g = h('div', 'sgrp');
    const row = h('div', 'srow ihead');
    const stx = h('div', 'stx');
    const small = h('small');
    stx.append(h('b', '', v?.edited ? t.edited : t.standard), small);
    if (v?.broken) {
      small.className = 'warn';
      small.append(bang(), t.broken);
    } else {
      small.textContent = this.fresh && v?.edited ? t.justNow : t.standardLine;
    }
    const sc = h('div', 'sc');
    if (v?.edited || v?.broken) {
      const reset = h('button', 'btn2 ck', t.reset);
      reset.type = 'button';
      reset.dataset.ireset = '';
      reset.addEventListener('click', () => this.reset());
      sc.append(reset);
    }
    row.append(stx, sc);
    g.append(row);
    return g;
  }

  private reset(): void {
    this.menuOpen = false;
    this.done = null;
    this.sample = null;
    // The box takes the standard text whatever it held.
    if (this.box) this.box.value = this.saved;
    api.instructionsReset().then(
      (v) => this.take(v),
      (e) => {
        logFailed('restoring the instructions', e);
        this.done = { kind: 'notSaved' };
        this.paint();
      },
    );
  }

  /** The last dictation as it was pasted, struck through, and as the new instructions clean it. */
  private compare(s: InstructionsSample): HTMLElement {
    const t = this.ctx.t.ins;
    const c = h('div', 'cmpr');
    c.setAttribute('role', 'group');
    const was = h('div');
    was.append(h('small', '', s.app ? t.before.replace('{APP}', () => s.app) : t.beforeNoApp), h('s', '', s.before));
    const now = h('div');
    now.append(h('small', '', t.after), h('span', '', s.after));
    c.append(was, now);
    return c;
  }

  /** "Edit with AI", which opens its menu under it. */
  private actions(): HTMLElement {
    const t = this.ctx.t.ins;
    const acts = h('div', 'acts');
    const ai = h('button', 'btn2 pri', t.withAi);
    ai.type = 'button';
    ai.dataset.iai = '';
    ai.setAttribute('aria-expanded', String(this.menuOpen));
    ai.addEventListener('click', () => {
      this.menuOpen = !this.menuOpen;
      if (this.menuOpen) this.entering.add('menu');
      this.done = null;
      this.paint();
    });
    acts.append(ai);
    return acts;
  }

  /** The agent "Open in" starts: the one chosen for the cleanup when it is found here, else the first one found. */
  private agent(): AgentView | null {
    const found = (this.agents() ?? []).filter((a) => a.found && AGENTS.includes(a.id));
    const chosen = this.ctx.settings?.refineAgent;
    return found.find((a) => a.id === chosen) ?? found[0] ?? null;
  }

  private menu(): HTMLElement {
    const t = this.ctx.t.ins;
    const m = h('div', 'pmenu');
    m.setAttribute('role', 'group');
    m.setAttribute('aria-label', t.withAi);
    const item = (title: string, line: string, data: string, act: () => void) => {
      const b = h('button');
      b.type = 'button';
      b.dataset[data] = '';
      b.append(h('b', '', title), h('small', '', line));
      b.addEventListener('click', act);
      m.append(b);
    };
    const a = this.agent();
    if (a) item(fill(t.open, a.name), fill(t.openLine, a.name), 'irun', () => this.open(a));
    item(t.copy, t.copyLine, 'icopy', () => this.copy());
    return m;
  }

  private open(a: AgentView): void {
    this.menuOpen = false;
    this.paint();
    api.instructionsOpen(a.id).then(
      (how) => {
        this.done = { kind: how === 'taskCopied' ? 'openedCopied' : 'opened', name: a.name };
        this.entering.add('done');
        this.paint();
      },
      (e) => {
        logFailed('opening the agent', e);
        this.done = { kind: 'notOpened', name: a.name };
        this.entering.add('done');
        this.paint();
      },
    );
  }

  private copy(): void {
    this.menuOpen = false;
    this.paint();
    api.instructionsCopy().then(
      () => {
        this.done = { kind: 'copied' };
        this.entering.add('done');
        this.paint();
      },
      (e) => logFailed('copying the task', e),
    );
  }

  /** What the last action did; a failure in amber with "!". */
  private doneLine(): HTMLElement | null {
    const d = this.done;
    if (!d) return null;
    const t = this.ctx.t.ins;
    const line = h('div', 'idone');
    line.setAttribute('role', 'status');
    switch (d.kind) {
      case 'opened':
      case 'openedCopied':
        line.textContent = fill(t[d.kind], d.name);
        break;
      case 'copied':
        line.textContent = t.copied;
        break;
      case 'notOpened':
      case 'notSaved':
        line.classList.add('warn');
        line.append(bang(), d.kind === 'notOpened' ? fill(t.notOpened, d.name) : t.notSaved);
        break;
    }
    return line;
  }
}

/** The "!" an error starts with, in amber. */
export function bang(): HTMLElement {
  const b = h('span', 'bang', '!');
  b.setAttribute('aria-hidden', 'true');
  return b;
}
