// Every Drop state side by side, at each of the four places, to check against the design by eye (dev server only).
// Language: ?lang=ru, English otherwise. Each cell stands for a piece of the screen: its border is the work area's edge,
// and the Drop sits where it would on screen (12 px in from the edge). The last section draws the real Drop window
// (dashed) around the widest Drop and the largest tray card at each place, to check that nothing is cut off.
import './design/cue.css';
import './design/drop.css';
import './design/tray.css';
import './app.css';
import './design/cue.js';
import './design/drop.js';
import { trayHtml } from './trays';
import { DICTS, pickLang, type TrayKind } from './strings';
import type { Place } from './controller';

const CUE = (window as any).CUE;
const lang = pickLang(new URLSearchParams(location.search).get('lang'));
const t = DICTS[lang];
document.documentElement.lang = lang;
document.body.style.overflow = 'auto';
document.body.style.background = '#c6d9f7'; // app.css makes body transparent and overrides the style in preview.html
const grid = document.getElementById('grid')!;
/** The Drop window's size in logical px at each place (`window_size` in app/src-tauri/src/drop_window.rs). */
const WINDOW: Record<Place, { w: number; h: number }> = {
  bottom: { w: 640, h: 860 },
  top: { w: 640, h: 860 },
  left: { w: 900, h: 540 },
  right: { w: 900, h: 540 },
};
/** From the work area's edge to the Drop's center: the 12 px gap plus half the Drop. */
const INSET = 'calc(12px + var(--h) / 2)';
/** The Drop's center inside a cell whose border is the work area's edge. */
const ANCHOR: Record<Place, [string, string]> = {
  bottom: ['50%', `calc(100% - ${INSET})`],
  top: ['50%', INSET],
  left: [INSET, '50%'],
  right: [`calc(100% - ${INSET})`, '50%'],
};
// The app always shows icons in dictation mode; the "text" cells show the design's text labels (chip, label, button) for a size check.
const ALL_STATES: [string, any][] = [
  ['idle', {}], ['idle · hover', { hov: true }], ['start', {}], ['listen', {}], ['listen · hover', { hov: true }],
  ['think', {}], ['done', {}], ['empty', {}], ['cancel', {}], ['cancel · hover', { hov: true }],
  ['error · net', { err: 'net' }], ['error · net · hover', { err: 'net', hov: true }],
  ['error · key', { err: 'key' }], ['error · key · hover', { err: 'key', hov: true }],
  ['listen · ask · text', { mode: 'ask', lbl: 'text' }], ['cancel · text', { lbl: 'text' }], ['error · net · text', { err: 'net', lbl: 'text' }],
];
const PLACE_STATES = ALL_STATES.filter(([name]) => ['idle', 'idle · hover', 'start', 'listen · hover', 'think', 'cancel · hover', 'error · net · hover'].includes(name));
const drops: any[] = [];

function api(fx: any) {
  return {
    mode: () => 'dictate', err: () => fx.err ?? 'net', errLabel: () => (fx.err === 'key' ? t.error.stt : t.error.model),
    retry: () => fx.err !== 'key', lbl: () => fx.lbl ?? 'icons', os: () => 'win', text: t.drop, voice: new CUE.Voice(),
    onEsc() {}, onPress() {}, onAction() {},
  };
}

function section(title: string): HTMLElement {
  const h = document.createElement('h2');
  h.textContent = title;
  const row = document.createElement('div');
  row.className = 'row';
  grid.append(h, row);
  return row;
}

function cell(row: HTMLElement, place: Place, label: string, cls = ''): HTMLElement {
  const c = document.createElement('div');
  c.className = `cell ${place} ${cls}`.trim();
  c.dataset.place = place;
  c.style.setProperty('--ax', ANCHOR[place][0]);
  c.style.setProperty('--ay', ANCHOR[place][1]);
  c.innerHTML = `<span>${label}</span>`;
  row.append(c);
  return c;
}

function drop(c: HTMLElement, name: string, fx: any) {
  const a = api(fx);
  const d = new CUE.Drop(c, a, { state: name.split(' ')[0], ...fx });
  drops.push({ drop: d, voice: a.voice });
  return d;
}

/** A still copy of a tray card: no close timers, the ring on the close button full, the core not called. */
function tray(c: HTMLElement, kind: TrayKind) {
  const d = drop(c, 'idle', {});
  new CUE.Tray(c, d, { width: 300 }).open(trayHtml(kind, t));
}

const row36 = section('36 px — every state (bottom)');
for (const [name, fx] of ALL_STATES) drop(cell(row36, 'bottom', name), name, fx);
for (const place of ['bottom', 'top', 'left', 'right'] as Place[]) {
  const row = section(`${place}`);
  for (const [name, fx] of PLACE_STATES) drop(cell(row, place, name), name, fx);
  for (const kind of ['noField', 'micSilent'] as TrayKind[]) tray(cell(row, place, `tray card · ${kind}`, 'tcell'), kind);
}
// The real window around the Drop: centered on the Drop's center; the solid line is the work area's edge.
const winRow = section('the window at each place (dashed) and the work area edge (solid)');
for (const place of ['bottom', 'top', 'left', 'right'] as Place[]) {
  for (const what of ['error · net · hover', 'micSilent'] as const) {
    const c = document.createElement('div');
    c.className = `win ${place}`;
    c.dataset.place = place;
    c.style.width = `${WINDOW[place].w}px`;
    c.style.height = `${WINDOW[place].h}px`;
    c.innerHTML = `<span>${place} · ${what}</span><i class="edge"></i>`;
    winRow.append(c);
    if (what === 'micSilent') tray(c, 'micSilent');
    else drop(c, what, { err: 'net', hov: true });
  }
}
(function loop(now: number) {
  for (const d of drops) { d.voice.step(now, true); d.drop.frame(now); }
  requestAnimationFrame(loop);
})(performance.now());
