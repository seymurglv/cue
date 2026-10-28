import { describe, expect, it } from 'vitest';
import { CUE } from '../src/designParts';

/** The joint box of what the mark paints, on its 24 grid: every visible line with half its stroke (the caps are round,
 *  so that holds at the ends of slanted lines too) and the lamp dot. */
function paintedBox(g: SVGGElement) {
  const num = (el: Element, a: string) => parseFloat(el.getAttribute(a) ?? '0');
  const xs: number[] = [];
  const ys: number[] = [];
  for (const l of g.querySelectorAll('line')) {
    if (num(l, 'opacity') <= 0) continue;
    const half = num(l, 'stroke-width') / 2;
    for (const [x, y] of [[num(l, 'x1'), num(l, 'y1')], [num(l, 'x2'), num(l, 'y2')]]) {
      xs.push(x - half, x + half);
      ys.push(y - half, y + half);
    }
  }
  const dot = g.querySelector('circle')!;
  if (num(dot, 'opacity') > 0 && num(dot, 'r') > 0) {
    const [cx, cy, r] = [num(dot, 'cx'), num(dot, 'cy'), num(dot, 'r')];
    xs.push(cx - r, cx + r);
    ys.push(cy - r, cy + r);
  }
  return { left: Math.min(...xs), right: Math.max(...xs), top: Math.min(...ys), bottom: Math.max(...ys), cx: (Math.min(...xs) + Math.max(...xs)) / 2 };
}

function group() {
  const ns = 'http://www.w3.org/2000/svg';
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  const g = document.createElementNS(ns, 'g') as SVGGElement;
  svg.append(g);
  document.body.append(svg);
  return g;
}

/** The mark drawn straight in `state` (no spring), as the static pages draw it. */
function still(state: string, lvl = 0) {
  const g = group();
  new (CUE().Morph)(g, { state, static: true, lvl });
  return g;
}

/** The live mark after `ms` of frames in `state` at a constant voice level: what the Drop shows. */
function live(state: string, lvl = 0, ms = 3000) {
  const g = group();
  const m = new (CUE().Morph)(g, { state });
  for (let t = 0; t <= ms; t += 16) m.tick(t, lvl, [0.5, 0.5, 0.5, 0.5]);
  return g;
}

describe('The mark in the circle', () => {
  it('start: the bar and the lamp are centered on x = 12, live (the full lamp) exactly, static within 0.3', () => {
    expect(paintedBox(live('start')).cx).toBeCloseTo(12, 2);
    expect(Math.abs(paintedBox(still('start')).cx - 12)).toBeLessThanOrEqual(0.3);
    // Vertically it already was: the bar's round caps reach 3.9 and 20.1.
    const b = paintedBox(still('start'));
    expect((b.top + b.bottom) / 2).toBeCloseTo(12, 6);
  });

  it.each([0, 0.5, 1])('listen at level %s: the bars and the lamp are centered on x = 12 within 0.3', (lvl) => {
    for (const g of [still('listen', lvl), live('listen', lvl)]) {
      expect(Math.abs(paintedBox(g).cx - 12)).toBeLessThanOrEqual(0.3);
    }
    // At the middle level exactly.
    if (lvl === 0.5) expect(paintedBox(still('listen', lvl)).cx).toBeCloseTo(12, 6);
  });

  it('listen: the bars grow from y = 12 at every level, the lamp stays at their top right', () => {
    for (const lvl of [0, 0.3, 0.5, 1]) {
      const g = live('listen', lvl);
      const lines = [...g.querySelectorAll('line')].slice(0, 3);
      for (const l of lines) {
        const mid = (parseFloat(l.getAttribute('y1')!) + parseFloat(l.getAttribute('y2')!)) / 2;
        expect(mid, `level ${lvl}`).toBeCloseTo(12, 2);
      }
      const dot = g.querySelector('circle')!;
      expect(parseFloat(dot.getAttribute('cy')!)).toBeCloseTo(4.5, 2);
      expect(parseFloat(dot.getAttribute('cx')!)).toBeGreaterThan(parseFloat(lines[2].getAttribute('x1')!));
    }
  });

  it.each(['think', 'done', 'cancel', 'error', 'ask', 'translate'])('%s is centered on x = 12 within 0.3', (state) => {
    expect(Math.abs(paintedBox(still(state)).cx - 12)).toBeLessThanOrEqual(0.3);
  });
});
