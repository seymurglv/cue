// Every Drop state side by side, to check against the original CUE design by eye. Language: ?lang=ru, English otherwise.
import './design/cue.css';
import './design/drop.css';
import './design/tray.css';
import './app.css';
import './design/cue.js';
import './design/drop.js';
import { trayHtml } from './trays';
import { DICTS, pickLang, type TrayKind } from './strings';

const CUE = (window as any).CUE;
const lang = pickLang(new URLSearchParams(location.search).get('lang'));
const t = DICTS[lang];
document.documentElement.lang = lang;
document.body.style.overflow = 'auto';
document.body.style.background = '#c6d9f7'; // app.css makes body transparent and overrides the style in preview.html
const grid = document.getElementById('grid')!;
// The app always shows icons in dictation mode; the "text" cells show the design's text labels (chip, label, button) for a size check.
const states: [string, any][] = [
  ['idle', {}], ['idle · hover', { hov: true }], ['start', {}], ['listen', {}], ['listen · hover', { hov: true }],
  ['think', {}], ['done', {}], ['empty', {}], ['cancel', {}], ['cancel · hover', { hov: true }],
  ['error · net', { err: 'net' }], ['error · net · hover', { err: 'net', hov: true }],
  ['error · key', { err: 'key' }], ['error · key · hover', { err: 'key', hov: true }],
  ['listen · ask · text', { mode: 'ask', lbl: 'text' }], ['cancel · text', { lbl: 'text' }], ['error · net · text', { err: 'net', lbl: 'text' }],
];
const drops: any[] = [];
for (const [name, fx] of states) {
  const cell = document.createElement('div');
  cell.className = 'cell';
  cell.style.setProperty('--anchor', '24px');
  cell.innerHTML = `<span>${name}</span>`;
  grid.append(cell);
  const voice = new CUE.Voice();
  const api = {
    mode: () => 'dictate', err: () => fx.err ?? 'net', errLabel: () => (fx.err === 'key' ? t.error.sonioxKey : t.error.net),
    retry: () => fx.err !== 'key', lbl: () => fx.lbl ?? 'icons', os: () => 'win', text: t.drop, voice, onEsc() {}, onPress() {}, onAction() {},
  };
  drops.push({ drop: new CUE.Drop(cell, api, { state: name.split(' ')[0], ...fx }), voice });
}
// Below the states every tray card is open: the same trayHtml markup and the same CUE.Tray as in the Drop window.
// A still copy: no close timers, the ring on the close button is full (as on the design's static sheets), the core is not called.
for (const kind of ['noField', 'micSilent', 'holdHint', 'lastMinute'] as TrayKind[]) {
  const cell = document.createElement('div');
  cell.className = 'cell tcell';
  cell.style.setProperty('--anchor', '24px');
  cell.innerHTML = `<span>tray card · ${kind}</span>`;
  grid.append(cell);
  const api = { mode: () => 'dictate', err: () => 'net', lbl: () => 'icons', os: () => 'win', text: t.drop, voice: new CUE.Voice(), onEsc() {}, onPress() {}, onAction() {} };
  const drop = new CUE.Drop(cell, api, { state: 'idle' });
  new CUE.Tray(cell, drop, { width: 300 }).open(trayHtml(kind, t));
}
(function loop(now: number) {
  for (const d of drops) { d.voice.step(now, true); d.drop.frame(now); }
  requestAnimationFrame(loop);
})(performance.now());
