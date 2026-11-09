// The settings tray's Cleanup page (docs/specs/2026-11-08-no-keys-design.md, "Cleanup page (Чистка текста)"; the
// owner's variant A of the mockup): on top the three parts, Без ИИ · Этот компьютер · Ваш AI, and under them the list of
// the part chosen, then one line on where the text goes.
// - Без ИИ: one row on what CUE does without a model.
// - Этот компьютер: the local models (models.ts), the recommended one for the user's languages first, the others with
//   the Hugging Face search as their last row. Nothing downloads until a model is pressed; pressed again while it
//   downloads, it stops.
// - Ваш AI: OpenCode, then Codex, as found on this computer (`agents_get`, looked at again each time the tray or this
//   page opens; meanwhile the rows say "Checking…" and the page works). A found agent is chosen by its row; one not
//   installed is pale, with its install page's button. Under the chosen one: its model (OpenCode's hundreds with a
//   filter, "Быстрая · авто" first; remembered per agent) and what is pasted when the AI does not answer.
// The first page's Cleanup value comes from here too (`value`): what cleans now, the local model's download in percent,
// and in amber an agent to sign in to again or not found.
// The page is drawn again where it stands when a setting or the agents change (`paint`); only a move to another part
// draws it anew, its list easing in.
import * as api from './api';
import type { AgentView, Where } from './api';
import { icon } from './icons';
import { percent } from './model';
import { ModelsPage, joined, llmSource } from './models';
import { dropDown, h, logFailed, segment, srow, type Ctx, type MenuItem } from './page';
import { fill } from './text';

const WHERE: readonly Where[] = ['off', 'local', 'agent'];
/** The agents CUE offers, in the order every list shows them, by the names they have before the core has said. */
const AGENTS: readonly { id: string; name: string }[] = [
  { id: 'opencode', name: 'OpenCode' },
  { id: 'codex', name: 'Codex' },
];
/** The agent whose row says "Recommended" and whose model defaults to "Быстрая · авто". */
const OPENCODE = 'opencode';
/** A model list longer than this gets a field over it. */
const FILTER_FROM = 12;
/** The local model's states on its way or stuck: the first page's value follows them. */
const BUSY = ['preparing', 'downloading', 'loading', 'failed', 'noSpace', 'notRunnable'] as const;

/** The first page's Cleanup value: its text, in amber (`warn`), and the share for the line under the row while the
 *  local model downloads (`progress`; its text is then the percent, in Bebas). */
export interface CleanupValue {
  text: string;
  warn: boolean;
  progress: number | null;
}

export class CleanupPage {
  private readonly llms: ModelsPage;
  /** The agents as `agents_get` last said (null — not heard yet). */
  private agents: AgentView[] | null = null;
  /** An `agents_get` is on its way. */
  private looking = false;
  /** The newest `agents_get`: an answer to an older one is dropped. */
  private seq = 0;
  /** The part the page shown was drawn for (null — not shown); its boxes, filled again in place. */
  private drawn: Where | null = null;
  private agentBox: HTMLElement | null = null;
  private modelBox: HTMLElement | null = null;
  private foot: HTMLElement | null = null;

  /** `resized` — the page's height changed (the tray follows it); `valueChanged` — the first page's value may have. */
  constructor(
    private readonly ctx: Ctx,
    private readonly resized: () => void,
    private readonly valueChanged: () => void,
  ) {
    this.llms = new ModelsPage(ctx, resized, llmSource(ctx));
  }

  private get where(): Where {
    return this.ctx.settings?.refineWhere ?? 'agent';
  }

  /** The cleanup's local list from the core: drawn where it is shown; the fallback, the note and the value follow. */
  setLlms(view: api.ModelsView): void {
    this.llms.set(view);
    this.paint();
    this.valueChanged();
  }

  /** The agents looked at again (the tray or this page opened, the cleanup moved to "Ваш AI", another agent chosen). */
  refresh(): void {
    const seq = ++this.seq;
    this.looking = true;
    api.agentsGet().then(
      (views) => {
        if (seq !== this.seq) return;
        this.looking = false;
        this.agents = views;
        this.paint();
        this.valueChanged();
      },
      (e) => {
        logFailed('looking at the agents', e);
        if (seq !== this.seq) return;
        this.looking = false;
        this.paint();
      },
    );
  }

  /** The page is left: a search waiting for its keys stops, and nothing is drawn in place until it is shown again. */
  left(): void {
    this.llms.stop();
    this.drawn = null;
    this.agentBox = this.modelBox = this.foot = null;
  }

  /** The page's rows: the three parts, the part's list, the note. A move to another part eases the list in. */
  rows(): HTMLElement[] {
    const t = this.ctx.t.cl;
    const where = this.where;
    const top = h('div', 'segrow');
    top.append(segment('where', WHERE, t.where, where));
    const body: HTMLElement[] = [];
    this.agentBox = this.modelBox = null;
    if (where === 'off') {
      const g = h('div', 'sgrp');
      g.append(srow(t.offTitle, t.offLine).row);
      body.push(g);
    } else if (where === 'local') {
      body.push(...this.llms.rows());
    } else {
      this.agentBox = h('div', 'sgrp');
      this.agentBox.setAttribute('role', 'group');
      this.agentBox.setAttribute('aria-label', t.found);
      this.modelBox = h('div', 'sgrp');
      body.push(this.agentBox, this.modelBox);
    }
    this.foot = h('div', 'tft');
    const moved = this.drawn !== null && this.drawn !== where;
    this.drawn = where;
    this.paint();
    if (moved && !this.ctx.reduced()) {
      for (const b of body) {
        b.animate?.([{ opacity: 0, transform: 'translateY(4px)' }, { opacity: 1, transform: 'none' }], {
          duration: 260,
          easing: 'cubic-bezier(.22,1,.36,1)',
          fill: 'backwards',
        });
      }
    }
    return [top, ...body, this.foot];
  }

  /** The page shown takes the settings and the agents where it stands; false when it shows another part than the
   *  settings say (the caller draws it anew). */
  paint(): boolean {
    if (this.drawn === null) return true;
    if (this.drawn !== this.where) return false;
    this.agentBox?.replaceChildren(...this.agentRows());
    this.modelBox?.replaceChildren(...this.modelRows());
    if (this.modelBox) this.modelBox.hidden = !this.modelBox.childElementCount;
    if (this.foot) this.foot.textContent = this.note();
    this.resized();
    return true;
  }

  /** A part pressed: the thumb slides there at once, the core is told, and the agents are looked at for "Ваш AI". */
  choose(where: Where): void {
    void this.ctx.change({ kind: 'refineWhere', value: where }).then((v) => {
      if (v && where === 'agent') this.refresh();
    });
  }

  /* ---------- Ваш AI ---------- */

  private agent(id: string): AgentView | undefined {
    return this.agents?.find((a) => a.id === id);
  }

  /** OpenCode then Codex: found, with its line (OpenCode "Recommended"), the one in use checked; a sign-in to redo in
   *  amber; not installed pale, with its install button; while they are looked at, "Checking…". */
  private agentRows(): HTMLElement[] {
    const t = this.ctx.t.cl;
    const s = this.ctx.settings;
    const rows: HTMLElement[] = [h('div', 'mgh', t.found)];
    for (const { id, name } of AGENTS) {
      const a = this.agent(id);
      const r = h('div', 'srow mdl');
      r.dataset.agent = id;
      const stx = h('div', 'stx');
      const small = h('small');
      stx.append(h('b', '', a?.name ?? name), small);
      const sc = h('div', 'sc');
      r.append(stx, sc);
      rows.push(r);
      const rec = () => (id === OPENCODE ? h('span', 'rc', t.recommended) : '');
      if (!a) {
        small.textContent = this.looking ? t.checking : '';
        small.hidden = !this.looking;
        continue;
      }
      if (!a.found) {
        r.classList.add('off');
        r.setAttribute('aria-disabled', 'true');
        joined(small, [rec(), id === OPENCODE ? t.openCodeMissing : t.notInstalled]);
        const b = h('button', 'btn2 lnk', id === OPENCODE ? t.install : t.howInstall);
        b.type = 'button';
        b.addEventListener('click', (e) => {
          e.stopPropagation();
          api.agentInstallLink(id).catch((err) => logFailed('opening the install page', err));
        });
        sc.append(b);
        continue;
      }
      r.tabIndex = 0;
      r.setAttribute('role', 'button');
      if (a.signedIn === false) {
        small.className = 'warn';
        small.textContent = t.signIn[id] ?? fill(t.value.signIn, a.name);
      } else {
        joined(small, [rec(), t.lines[id] ?? '']);
      }
      if (s?.refineWhere === 'agent' && s.refineAgent === id) {
        r.setAttribute('aria-current', 'true');
        const ok = h('span', 'mok');
        ok.innerHTML = icon('ok');
        sc.append(ok);
      } else {
        const pick = () => this.pickAgent(a);
        r.addEventListener('click', pick);
        r.addEventListener('keydown', (e) => {
          if (e.key !== 'Enter' && e.key !== ' ') return;
          e.preventDefault();
          pick();
        });
      }
    }
    return rows;
  }

  /** Another agent cleans: OpenCode's models are listed once it is the one. */
  private pickAgent(a: AgentView): void {
    void this.ctx.change({ kind: 'refineAgent', value: a.id }).then((v) => {
      if (v && a.id === OPENCODE && !a.models.length) this.refresh();
    });
  }

  /** Under the chosen agent: its model (once the agents are known and it is found), and the fallback. */
  private modelRows(): HTMLElement[] {
    const t = this.ctx.t.cl;
    const s = this.ctx.settings;
    const id = s?.refineAgent;
    if (!s || s.refineWhere !== 'agent' || !id) return [];
    const rows: HTMLElement[] = [];
    const a = this.agent(id);
    if (a?.found) {
      const pick = s.refineAgentModel[id] ?? '';
      const dd = dropDown('amodel', this.modelName(a, pick));
      dd.addEventListener('click', () => this.modelMenu(dd, a, pick));
      rows.push(srow(t.model, id === OPENCODE && !pick ? fill(t.autoHint, a.name) : null, dd).row);
    }
    const local = this.localModel();
    const on = s.refineFallback === 'local' && local ? 'local' : 'raw';
    const label = (v: string) => (v === 'local' && local ? fill(t.local, local.name) : t.raw);
    const fb = dropDown('fallback', label(on));
    fb.addEventListener('click', () => {
      const items: MenuItem[] = [{ value: 'raw', label: t.raw, on: on === 'raw' }];
      if (local) items.push({ value: 'local', label: label('local'), on: on === 'local' });
      this.ctx.menu(fb, items, (v) => void this.ctx.change({ kind: 'refineFallback', value: v === 'local' ? 'local' : 'raw' }));
    });
    rows.push(srow(t.fallback, local ? t.fallbackHint : t.fallbackNone, fb).row);
    return rows;
  }

  /** The model's name as the row shows it: the one picked, else the agent's default ("Быстрая · авто" for OpenCode). */
  private modelName(a: AgentView, pick: string): string {
    const named = (id: string) => a.models.find((m) => m.id === id)?.name ?? id;
    if (pick) return named(pick);
    return a.id === OPENCODE ? this.ctx.t.cl.auto : named(a.model);
  }

  /** The agent's models: OpenCode's "Быстрая · авто" first (naming the model it picks), then its own by provider,
   *  with a field over a long list; Codex's as they are. */
  private modelMenu(dd: HTMLElement, a: AgentView, pick: string): void {
    const t = this.ctx.t.cl;
    const current = pick || (a.id === OPENCODE ? '' : a.model);
    const provider = (id: string) => (id.includes('/') ? id.slice(0, id.indexOf('/')) : undefined);
    const models = a.id === OPENCODE ? [...a.models].sort((x, y) => (provider(x.id) ?? '').localeCompare(provider(y.id) ?? '')) : a.models;
    const items: MenuItem[] = models.map((m) => ({ value: m.id, label: m.name, on: m.id === current, group: a.id === OPENCODE ? provider(m.id) : undefined }));
    if (a.id === OPENCODE) {
      const auto = a.models.find((m) => m.id === a.model)?.name;
      items.unshift({ value: '', label: t.auto, sub: auto, on: !pick });
    }
    const filter = items.length > FILTER_FROM ? { hint: t.filter, none: t.noMatch } : undefined;
    const take = (model: string) => void this.ctx.change({ kind: 'refineAgentModel', value: { agent: a.id, model } });
    this.ctx.menu(dd, items, take, undefined, 'end', false, filter);
  }

  /** The local model the fallback cleans with: the one chosen on this computer, when its file is here. */
  private localModel(): api.ModelEntry | undefined {
    const v = this.llms.listView();
    const id = v?.current || this.ctx.settings?.refineLocal;
    return v?.entries.find((e) => e.id === id && (e.state === 'inUse' || e.state === 'downloaded'));
  }

  /* ---------- the note and the first page's value ---------- */

  /** One line on where the text goes, by route; "nothing chosen" says the text goes in without AI. */
  private note(): string {
    const n = this.ctx.t.cl.notes;
    const s = this.ctx.settings;
    switch (this.where) {
      case 'off':
        return n.off;
      case 'local':
        return this.llms.listView()?.current || s?.refineLocal ? n.local : n.noneLocal;
      default:
        if (!s?.refineAgent) return n.noneAgent;
        return s.refineAgent === OPENCODE ? n.opencode : n.codex;
    }
  }

  /** What cleans now, for the first page: Без ИИ; the local model, its download or its trouble; the agent and its
   *  model, or in amber the agent to sign in to again or not found; "Не выбрано". */
  value(): CleanupValue {
    const t = this.ctx.t;
    const plain = (text: string, warn = false): CleanupValue => ({ text, warn, progress: null });
    const s = this.ctx.settings;
    if (!s) return plain('');
    if (s.refineWhere === 'off') return plain(t.cl.value.off);
    if (s.refineWhere === 'local') {
      const v = this.llms.listView();
      if (!v) return plain('');
      const e = v.entries.find((x) => (BUSY as readonly string[]).includes(x.state));
      switch (e?.state) {
        case 'downloading':
          return { text: `${percent(e.done, e.total)} %`, warn: false, progress: e.done / (e.total || 1) };
        case 'preparing':
          return { text: '0 %', warn: false, progress: 0 };
        case 'loading':
          return plain(t.models.loading);
        case 'notRunnable':
          return plain(t.models.notRunnable, true);
        case 'failed':
        case 'noSpace':
          return plain(t.modelValue.failed, true);
      }
      const current = v.entries.find((x) => x.id === v.current);
      if (current) return plain(current.name);
      return s.refineLocal ? plain(t.cl.value.notDownloaded, true) : plain(t.cl.value.none);
    }
    const id = s.refineAgent;
    if (!id) return plain(t.cl.value.none);
    const a = this.agent(id);
    const name = a?.name ?? AGENTS.find((x) => x.id === id)?.name ?? id;
    if (a && !a.found) return plain(fill(t.cl.value.missing, name), true);
    if (a?.signedIn === false) return plain(fill(t.cl.value.signIn, name), true);
    const pick = s.refineAgentModel[id] ?? '';
    const model = a ? this.modelName(a, pick) : pick || (id === OPENCODE ? t.cl.auto : '');
    return plain(model ? `${name} · ${model}` : name);
  }
}
