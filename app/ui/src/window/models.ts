// The settings tray's Model page (docs/specs/2026-11-02-model-picker.md), under Speech recognition: the list of models
// — the short list and the found models downloaded here — and, while a search is typed, the repositories Hugging Face
// found. A row of the list chooses its model: the core downloads it and loads it beside the one in use, and the check
// moves once it is ready; while it downloads, its bar and numbers change in place. A downloaded model not in use can
// be deleted. A found repository CUE cannot run is pale, with the reason.
// The search is secondary, so it is quiet until needed: the list's last row, a magnifier and "Search Hugging Face".
// Pressed (or reached with Tab), the row becomes the field where it stands: the words stay in place as the field's
// hint, a line grows under it from the magnifier, and a cross clears what was typed. Esc clears, then closes; leaving
// an empty field closes it. Typing never loses the field: the list and the search's group are drawn again around it.
// The same page lists the cleanup's local models on the Cleanup page's "Этот компьютер" (docs/specs/2026-11-08-no-keys-
// design.md, "Cleanup page"): a `ModelSource` says which list it is, its calls, its texts and how its rows read — there
// in two groups, the recommended model for the user's languages, then the others with the search as their last row; a
// model found on Hugging Face says CUE has not checked it; a row pressed while its model downloads stops it. A local
// model can be deleted in use too (it is let go first).
import * as api from './api';
import type { EntryState, ModelEntry, ModelsView } from './api';
import { icon } from './icons';
import { bar, bytesLine, percent } from './model';
import { h, logFailed, wrap, type Ctx } from './page';
import { formatNumber, plural, type WinText } from './text';

/** A row's quiet line is made of parts: words, or a marked word ("Recommended", "Not checked by CUE"). */
export type Part = string | HTMLElement;

/** `parts` into `el`, joined by " · "; false when there is none. */
export function joined(el: HTMLElement, parts: Part[]): boolean {
  const kept = parts.filter((p) => p !== '');
  kept.forEach((p, i) => el.append(...(i ? [' · ', p] : [p])));
  return kept.length > 0;
}

/** The search runs this long after the last key, once the field holds this many letters. */
export const SEARCH_MS = 300;
const SEARCH_FROM = 2;
/** Megabytes and gigabytes as the design writes them: decimal. */
const MB = 1_000_000;
const GB = 1_000_000_000;

/** A repository the search found, for either list: `extra` follows its size (a GGUF's quantization). */
export interface FoundRow {
  repo: string;
  downloads: number;
  size: number | null;
  reason: string | null;
  extra?: string | null;
}

/** The texts a list is drawn with: the Model page's, with its own reasons and empty search, and the line of a download
 *  ({DONE}, {TOTAL} — MB; the Model page's own when none). */
export type ModelTexts = Omit<WinText['models'], 'reasons'> & { reasons: Record<string, string>; downloading?: string };

/** Which list a `ModelsPage` draws: its calls, its texts, the quiet line of a row and how a size reads. */
export interface ModelSource {
  /** The list's name in a log line. */
  kind: string;
  texts(): ModelTexts;
  search(q: string): Promise<FoundRow[]>;
  choose(choice: { id: string } | { repo: string }): Promise<void>;
  remove(id: string): Promise<boolean>;
  /** The quiet line under a model's name, without its size (languages, "Recommended", "Not checked by CUE"). */
  about(e: ModelEntry, view: ModelsView | null): Part[];
  size(bytes: number): string;
  /** The model in use can be deleted: the core lets it go first. */
  deleteInUse?: boolean;
  /** The list in two groups under these headings: the first holds the models `first` picks (left out when it has
   *  none), the second the rest, the search its last row. One group, under the list's own heading, without it. */
  groups?(view: ModelsView | null): { heads: [Part, Part]; first: (e: ModelEntry) => boolean };
  /** A mark after a found repository's line (CUE has not checked it). */
  foundMark?(): Part;
}

/** "25 languages", "English only", a language's name for one; nothing when the repository names none. */
function languages(ctx: Ctx, codes: string[]): string {
  const t = ctx.t.models;
  if (codes.length === 1 && codes[0] === 'en') return t.englishOnly;
  if (codes.length === 1) {
    try {
      const name = new Intl.DisplayNames([ctx.lang], { type: 'language' }).of(codes[0]);
      if (name) return name.charAt(0).toLocaleUpperCase(ctx.lang) + name.slice(1);
    } catch {
      // No such data: the code.
    }
    return codes[0];
  }
  return codes.length ? `${codes.length} ${plural(codes.length, t.languages)}` : '';
}

/** The speech models: the Model page under Speech recognition. */
export function speechSource(ctx: Ctx): ModelSource {
  return {
    kind: 'speech model',
    texts: () => ctx.t.models,
    search: (q) => api.modelsSearch(q),
    choose: (c) => api.modelChoose(c),
    remove: (id) => api.modelDelete(id),
    about: (e) => [languages(ctx, e.languages)],
    size: (bytes) => ctx.t.models.mb.replace('{SIZE}', String(Math.round(bytes / MB))),
  };
}

/** The user's languages as the heading of the recommended models names them: the interface's, then the one a
 *  translation goes into ("русский, английский"; "English"). The history's languages are a later step (the catalog
 *  by languages); until then these two say who the recommendation is for. */
function userLanguages(ctx: Ctx): string {
  const codes = [...new Set([ctx.lang, ctx.settings?.translateTarget].filter((c): c is string => !!c))];
  const name = (code: string) => {
    try {
      return new Intl.DisplayNames([ctx.lang], { type: 'language' }).of(code) ?? code;
    } catch {
      return code;
    }
  };
  return codes.map(name).join(', ');
}

/** The cleanup's local models: the Cleanup page's "Этот компьютер". */
export function llmSource(ctx: Ctx): ModelSource {
  const cl = () => ctx.t.cl;
  return {
    kind: 'language model',
    texts: () => ({ ...ctx.t.models, reasons: ctx.t.llms.reasons, none: ctx.t.llms.none, downloading: cl().downloading }),
    search: async (q) => (await api.llmsSearch(q)).map((f) => ({ ...f, extra: f.quant })),
    choose: (c) => api.llmChoose(c),
    remove: (id) => api.llmDelete(id),
    deleteInUse: true,
    about: (e, view) => [
      e.id === view?.recommended ? h('span', 'rc', cl().recommended) : '',
      view?.found?.includes(e.id) ? h('span', 'ncheck', cl().notChecked) : '',
    ],
    size: (bytes) => {
      const n = (bytes / GB).toFixed(1);
      return ctx.t.llms.gb.replace('{SIZE}', ctx.lang === 'ru' ? n.replace('.', ',') : n);
    },
    groups: (view) => {
      const forYou = h('span', '', cl().forYou);
      forYou.append(h('span', 'mut', ` · ${userLanguages(ctx)}`));
      return { heads: [forYou, cl().others], first: (e) => e.id === view?.recommended };
    },
    foundMark: () => h('span', 'ncheck', cl().notChecked),
  };
}

/** A switch on its way: choosing the model in use stops it. */
const RUNNING: readonly EntryState['state'][] = ['preparing', 'downloading', 'loading'];

export class ModelsPage {
  private view: ModelsView | null = null;
  private query = '';
  /** What the search found; 'offline' — it failed; null — no search shown. */
  private results: FoundRow[] | 'offline' | null = null;
  /** The newest search: an answer to an older one is dropped. */
  private seq = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private list: HTMLElement | null = null;
  private found: HTMLElement | null = null;

  /** `resized` — the page's height changed (the tray follows it). */
  constructor(
    private readonly ctx: Ctx,
    private readonly resized: () => void,
    private readonly source: ModelSource = speechSource(ctx),
  ) {}

  /** The core's list as it last said (null — not heard yet). */
  listView(): ModelsView | null {
    return this.view;
  }

  private get t(): ModelTexts {
    return this.source.texts();
  }

  /** The model in use's name, once the core has said. */
  currentName(): string | null {
    const v = this.view;
    return v?.entries.find((e) => e.id === v.current)?.name ?? null;
  }

  /** The core's new list: drawn again where it is shown. */
  set(view: ModelsView): void {
    this.view = view;
    this.paintList();
    this.paintFound();
    this.resized();
  }

  /** The page's content: the list, the search's row under it, what the search found. */
  rows(): HTMLElement[] {
    const t = this.t;
    this.list = h('div', 'sgrp mlist');
    this.list.setAttribute('aria-label', t.list);
    this.found = h('div', 'sgrp mfound');
    this.found.setAttribute('aria-label', t.found);
    for (const box of [this.list, this.found]) {
      box.addEventListener('click', (e) => this.pressed(e.target as Element));
      box.addEventListener('keydown', (e) => {
        if ((e.key === 'Enter' || e.key === ' ') && (e.target as Element).matches('[data-mid],[data-repo]')) {
          e.preventDefault();
          this.pressed(e.target as Element);
        }
      });
    }
    this.paintList();
    this.paintFound();
    return [this.list, this.searchRow(), this.found];
  }

  /** The search as the list's last row, opening into the field where it stands. */
  private searchRow(): HTMLElement {
    const t = this.t;
    const row = wrap('msr');
    const glyph = h('span', 'msg');
    glyph.innerHTML = icon('search');
    const input = h('input');
    input.placeholder = t.hint;
    input.setAttribute('aria-label', t.hint);
    input.spellcheck = false;
    input.value = this.query;
    const clear = h('button', 'msx');
    clear.type = 'button';
    clear.innerHTML = icon('x');
    clear.title = t.clear;
    clear.setAttribute('aria-label', t.clear);
    clear.tabIndex = -1;
    row.append(glyph, h('span', 'msl', t.hint), input, h('i', 'msu'), clear);
    const paint = () => {
      row.classList.toggle('typed', input.value.length > 0);
      if (input.value) row.classList.add('open');
    };
    const empty = () => {
      input.value = '';
      this.typed('');
      paint();
    };
    row.addEventListener('mousedown', (e) => {
      if ((e.target as Element).closest('.msx') || e.target === input) return;
      e.preventDefault();
      input.focus();
    });
    clear.addEventListener('mousedown', (e) => e.preventDefault());
    clear.addEventListener('click', () => {
      empty();
      input.focus();
    });
    input.addEventListener('focus', () => row.classList.add('open'));
    input.addEventListener('blur', () => row.classList.toggle('open', input.value.length > 0));
    input.addEventListener('input', () => {
      this.typed(input.value);
      paint();
    });
    // One step at a time: Esc clears what was typed, then closes the field; only then does the tray go back.
    input.addEventListener('keydown', (e) => {
      if (e.key !== 'Escape') return;
      e.stopPropagation();
      if (input.value) empty();
      else input.blur();
    });
    paint();
    return row;
  }

  /** The page is left: a search waiting for its keys stops. */
  stop(): void {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  private typed(value: string): void {
    this.query = value;
    this.stop();
    const q = value.trim();
    if (q.length < SEARCH_FROM) {
      this.seq++;
      if (this.results !== null) {
        this.results = null;
        this.paintFound();
        this.resized();
      }
      return;
    }
    this.timer = setTimeout(() => void this.search(q), SEARCH_MS);
  }

  private async search(q: string): Promise<void> {
    this.timer = null;
    const seq = ++this.seq;
    let results: FoundRow[] | 'offline';
    try {
      results = await this.source.search(q);
    } catch (e) {
      logFailed('searching Hugging Face', e);
      results = 'offline';
    }
    if (seq !== this.seq) return;
    this.results = results;
    this.paintFound();
    this.resized();
  }

  private pressed(target: Element): void {
    let b: HTMLElement | null;
    if ((b = target.closest<HTMLElement>('[data-mretry]'))) this.choose({ id: b.closest<HTMLElement>('[data-mid]')!.dataset.mid! });
    else if ((b = target.closest<HTMLElement>('[data-mdel]'))) {
      const id = b.closest<HTMLElement>('[data-mid]')!.dataset.mid!;
      this.source.remove(id).catch((e) => logFailed(`deleting a ${this.source.kind}`, e));
    } else if ((b = target.closest<HTMLElement>('[data-mid]'))) {
      const v = this.view;
      const id = b.dataset.mid!;
      // The model in use is chosen again only to stop a switch; a model on its way, pressed again, stops too (the
      // core's choice of the model it is switching to).
      const switching = v?.entries.some((e) => e.id !== v.current && RUNNING.includes(e.state));
      if (id !== v?.current || switching) this.choose({ id });
    } else if ((b = target.closest<HTMLElement>('[data-repo]:not(.off)'))) this.choose({ repo: b.dataset.repo! });
  }

  private choose(choice: { id: string } | { repo: string }): void {
    this.source.choose(choice).catch((e) => logFailed(`choosing a ${this.source.kind}`, e));
  }

  /* ---------- drawing ---------- */


  private gb(bytes: number): string {
    const n = (bytes / GB).toFixed(1);
    return this.ctx.lang === 'ru' ? n.replace('.', ',') : n;
  }

  private paintList(): void {
    const list = this.list;
    if (!list) return;
    const olds = new Map([...list.querySelectorAll<HTMLElement>('[data-mid]')].map((r) => [r.dataset.mid!, r]));
    const entries = this.view?.entries ?? [];
    const row = (e: ModelEntry) => {
      const old = olds.get(e.id);
      return old && stepRow(old, e) ? old : this.row(e);
    };
    const head = (p: Part) => {
      const g = h('div', 'mgh');
      g.append(p);
      return g;
    };
    const groups = this.source.groups?.(this.view);
    if (!groups) {
      list.replaceChildren(head(this.t.list), ...entries.map(row));
      return;
    }
    const first = entries.filter(groups.first);
    const rest = entries.filter((e) => !groups.first(e));
    list.replaceChildren(...(first.length ? [head(groups.heads[0]), ...first.map(row)] : []), head(groups.heads[1]), ...rest.map(row));
  }

  /** A model of the list, by its state. */
  private row(e: ModelEntry): HTMLElement {
    const t = { ...this.ctx.t, models: this.t };
    const r = h('div', 'srow mdl');
    r.dataset.mid = e.id;
    r.dataset.state = e.state;
    r.tabIndex = 0;
    r.setAttribute('role', 'button');
    const stx = h('div', 'stx');
    const small = h('small');
    stx.append(h('b', '', e.name), small);
    const sc = h('div', 'sc');
    r.append(stx, sc);
    const about = this.source.about(e, this.view);
    const sized = () => joined(small, [...about, this.source.size(e.size)]);
    const warn = (text: string) => {
      small.className = 'warn';
      small.textContent = text;
    };
    const remove = () => {
      const b = h('button', 'mdel', t.models.remove);
      b.type = 'button';
      b.dataset.mdel = '';
      sc.append(b);
    };
    switch (e.state) {
      case 'inUse': {
        sized();
        const ok = h('span', 'mok');
        ok.innerHTML = icon('ok');
        sc.append(ok);
        r.setAttribute('aria-current', 'true');
        if (this.source.deleteInUse) remove();
        break;
      }
      case 'downloaded':
        sized();
        remove();
        break;
      case 'missing':
        small.hidden = !joined(small, about);
        sc.append(h('span', 'msz', this.source.size(e.size)));
        break;
      case 'preparing':
        small.textContent = t.models.preparing;
        break;
      case 'loading':
        small.textContent = t.models.loading;
        break;
      case 'downloading': {
        small.hidden = true;
        sc.append(h('span', 'num mpct', `${percent(e.done, e.total)} %`));
        const prog = h('div', 'mprog');
        const note = h('small');
        note.append(bytesLine(this.t.downloading ?? t.modelLine.downloading, e.done, e.total));
        prog.append(bar('mbar', e.done / (e.total || 1)), note);
        r.append(prog);
        break;
      }
      case 'failed': {
        small.className = 'warn';
        if (e.total > 0) small.append(bytesLine(t.modelLine.failed, e.done, e.total));
        else small.textContent = t.modelValue.failed;
        const retry = h('button', 'btn2');
        retry.type = 'button';
        retry.dataset.mretry = '';
        retry.innerHTML = icon('retry');
        retry.append(h('span', 'lbl', t.modelRetry));
        sc.append(retry);
        break;
      }
      case 'notRunnable':
        warn(t.models.notRunnable);
        remove();
        break;
      case 'noSpace':
        warn(t.models.noSpace.replace('{NEED}', this.gb(e.needed)).replace('{FREE}', this.gb(e.free)));
        break;
    }
    return r;
  }

  private paintFound(): void {
    const box = this.found;
    if (!box) return;
    const t = this.t;
    box.hidden = this.results === null;
    const listed = new Set((this.view?.entries ?? []).map((e) => e.repo.toLowerCase()));
    const head = h('div', 'mgh', t.found);
    if (this.results === null) return box.replaceChildren();
    if (this.results === 'offline') return box.replaceChildren(head, h('div', 'mfnote', t.offline));
    const rows = this.results.filter((f) => !listed.has(f.repo.toLowerCase())).map((f) => this.foundRow(f));
    box.replaceChildren(head, ...(rows.length ? rows : [h('div', 'mfnote', t.none)]));
  }

  /** A repository the search found: pale, with its reason, when CUE cannot run it. */
  private foundRow(f: FoundRow): HTMLElement {
    const t = this.t;
    const r = h('div', 'srow mfd');
    r.dataset.repo = f.repo;
    const small = h('small');
    if (f.reason) small.textContent = t.reasons[f.reason];
    else {
      const downloads = `${formatNumber(f.downloads, this.ctx.lang)} ${plural(f.downloads, t.downloads)}`;
      joined(small, [downloads, f.size ? this.source.size(f.size) : '', f.extra ?? '', this.source.foundMark?.() ?? '']);
    }
    const stx = h('div', 'stx');
    stx.append(h('b', '', f.repo), small);
    r.append(stx);
    if (f.reason) {
      r.classList.add('off');
      r.setAttribute('aria-disabled', 'true');
    } else {
      r.tabIndex = 0;
      r.setAttribute('role', 'button');
    }
    return r;
  }
}

/** A download going on shown again in `row` without drawing it anew: the bar, the percent and the first number. False
 *  when `row` does not show a download of this model (then it is drawn again). */
function stepRow(row: HTMLElement, e: ModelEntry): boolean {
  if (e.state !== 'downloading' || row.dataset.state !== 'downloading') return false;
  const fill = row.querySelector<HTMLElement>('.mbar i');
  const pct = row.querySelector('.mpct');
  const done = row.querySelector('.mdone');
  if (!fill || !pct || !done) return false;
  fill.style.transform = `scaleX(${Math.min(1, e.done / (e.total || 1))})`;
  pct.textContent = `${percent(e.done, e.total)} %`;
  done.textContent = String(Math.round(e.done / MB));
  return true;
}
