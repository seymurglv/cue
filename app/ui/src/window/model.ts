// The speech model as the settings tray and the welcome window show it (docs/specs/2026-11-02-settings-tray.md): on the
// tray's first page a value ("61 %" in Bebas with a thin line under the row, "Did not download" in amber, "Ready"), and
// on its page the model's row: green "Ready · 668 MB on this computer"; "61 %" with a bar and "Downloading: 412 of 668
// MB"; amber "Did not download · 412 of 668 MB" with Retry. While it downloads, the bar, the percent and the bytes
// change in place. The core downloads and loads it; a page only follows `cue://model`.
import * as api from './api';
import type { ModelState } from './api';
import { icon } from './icons';
import { h, logFailed, type Ctx } from './page';

/** Megabytes as the design writes them: decimal, rounded. */
const MB = 1_000_000;

export function percent(done: number, total: number): number {
  return total > 0 ? Math.min(100, Math.floor((done * 100) / total)) : 0;
}

const mb = (bytes: number) => String(Math.round(bytes / MB));

/** The value on the first page and its look: `warn` — amber; `progress` — the share for the line under the row. */
export function modelValue(
  t: Ctx['t'],
  s: ModelState | null,
  keyMissing: boolean,
): { text: string; warn: boolean; progress: number | null } {
  const v = t.modelValue;
  if (!s) return { text: '', warn: false, progress: null };
  switch (s.state) {
    case 'downloading':
      return { text: v.downloading.replace('{N}', String(percent(s.done, s.total))), warn: false, progress: s.done / (s.total || 1) };
    case 'failed':
      return { text: v.failed, warn: true, progress: null };
    case 'unsupported':
      return { text: v.unsupported, warn: true, progress: null };
    case 'ready':
      return keyMissing ? { text: v.noKey, warn: true, progress: null } : { text: v.ready, warn: false, progress: null };
    default:
      return { text: v[s.state], warn: false, progress: null };
  }
}

/** A line with {DONE} and {TOTAL} as megabytes in Bebas (`.num`); the rest as text. */
function bytesLine(template: string, done: number, total: number): DocumentFragment {
  const out = document.createDocumentFragment();
  for (const part of template.split(/(\{DONE\}|\{TOTAL\})/)) {
    if (part === '{DONE}') out.append(h('b', 'num mdone', mb(done)));
    else if (part === '{TOTAL}') out.append(h('b', 'num', mb(total)));
    else if (part) out.append(part);
  }
  return out;
}

/** A 2 px bar filled to `share` from the left. */
export function bar(cls: string, share: number): HTMLElement {
  const b = h('span', cls);
  const fill = h('i');
  fill.style.transform = `scaleX(${Math.max(0, Math.min(1, share))})`;
  b.append(fill);
  return b;
}

/** The model's row on its page, by state. */
export function modelRow(t: Ctx['t'], s: ModelState | null): HTMLElement {
  const row = h('div', 'srow mrow');
  row.dataset.model = s?.state ?? 'unknown';
  const stx = h('div', 'stx');
  const small = h('small');
  stx.append(h('b', '', t.modelName), small);
  const sc = h('div', 'sc');
  row.append(stx, sc);
  const line = t.modelLine;
  if (!s) {
    small.hidden = true;
    return row;
  }
  switch (s.state) {
    case 'ready':
      small.className = 'ok';
      small.append(bytesLine(line.ready, 0, s.total));
      break;
    case 'downloading': {
      small.hidden = true;
      sc.append(h('span', 'num mpct', `${percent(s.done, s.total)} %`));
      const prog = h('div', 'mprog');
      const note = h('small');
      note.append(bytesLine(line.downloading, s.done, s.total));
      prog.append(bar('mbar', s.done / (s.total || 1)), note);
      row.append(prog);
      break;
    }
    case 'failed': {
      small.className = 'warn';
      small.append(bytesLine(line.failed, s.done, s.total));
      const retry = h('button', 'btn2');
      retry.type = 'button';
      retry.dataset.modelRetry = '';
      retry.innerHTML = icon('retry');
      retry.append(h('span', 'lbl', t.modelRetry));
      retry.addEventListener('click', () => void api.modelRetry().catch((e) => logFailed('retrying the speech model', e)));
      sc.append(retry);
      break;
    }
    case 'unsupported':
      small.className = 'warn';
      small.textContent = line.unsupported;
      break;
    default:
      small.textContent = line[s.state];
  }
  return row;
}

/** A download in progress shown again in `row` without drawing it anew: the bar, the percent and the first number.
 *  False when `row` does not show a download (then the caller draws the row again). */
export function stepModelRow(row: Element | null | undefined, s: ModelState): boolean {
  if (!row || s.state !== 'downloading' || (row as HTMLElement).dataset.model !== 'downloading') return false;
  const fill = row.querySelector<HTMLElement>('.mbar i');
  const pct = row.querySelector('.mpct');
  const done = row.querySelector('.mdone');
  if (!fill || !pct || !done) return false;
  fill.style.transform = `scaleX(${Math.min(1, s.done / (s.total || 1))})`;
  pct.textContent = `${percent(s.done, s.total)} %`;
  done.textContent = mb(s.done);
  return true;
}
