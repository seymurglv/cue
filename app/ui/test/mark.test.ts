import { describe, expect, it } from 'vitest';
import { CUE } from '../src/designParts';

const num = (el: Element, a: string) => parseFloat(el.getAttribute(a) ?? '0');

/** The box of what the visible lines paint, on the mark's 24 grid, with half the stroke (the caps are round, so that
 *  holds at the ends of slanted lines too). */
function linesBox(g: SVGGElement) {
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
  return box(xs, ys);
}

/** The lamp's box; null when it is not drawn. */
function dotBox(g: SVGGElement) {
  const dot = g.querySelector('circle')!;
  if (num(dot, 'opacity') <= 0 || num(dot, 'r') <= 0) return null;
  const [cx, cy, r] = [num(dot, 'cx'), num(dot, 'cy'), num(dot, 'r')];
  return box([cx - r, cx + r], [cy - r, cy + r]);
}

function box(xs: number[], ys: number[]) {
  const [left, right, top, bottom] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  return { left, right, top, bottom, cx: (left + right) / 2, cy: (top + bottom) / 2 };
}

/** Everything the mark paints: the lines and the lamp. */
function paintedBox(g: SVGGElement) {
  const l = linesBox(g);
  const d = dotBox(g);
  return d ? box([l.left, l.right, d.left, d.right], [l.top, l.bottom, d.top, d.bottom]) : l;
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
  it('start: the bar is centered on x = 12 and y = 12, the lamp at its right side', () => {
    for (const g of [still('start'), live('start')]) {
      const bar = linesBox(g);
      expect(Math.abs(bar.cx - 12)).toBeLessThanOrEqual(0.05);
      expect(Math.abs(bar.cy - 12)).toBeLessThanOrEqual(0.05);
      expect(dotBox(g)!.left).toBeGreaterThan(bar.right);
    }
    // The lamp keeps its original offset from the bar: 6.4 to the right, at y = 6.2.
    const dot = still('start').querySelector('circle')!;
    expect([num(dot, 'cx'), num(dot, 'cy')]).toEqual([18.4, 6.2]);
  });

  it.each([0, 0.5, 1])('listen at level %s: the bars are centered on x = 12, the lamp at their right side', (lvl) => {
    for (const g of [still('listen', lvl), live('listen', lvl)]) {
      const bars = linesBox(g);
      expect(Math.abs(bars.cx - 12)).toBeLessThanOrEqual(0.05);
      expect(dotBox(g)!.left).toBeGreaterThan(bars.right);
    }
  });

  it('listen: the bars grow from y = 12 at every level, the lamp keeps its offset at their top right', () => {
    for (const lvl of [0, 0.3, 0.5, 1]) {
      const g = live('listen', lvl);
      const lines = [...g.querySelectorAll('line')].slice(0, 3);
      for (const l of lines) expect((num(l, 'y1') + num(l, 'y2')) / 2, `level ${lvl}`).toBeCloseTo(12, 2);
      expect(lines.map((l) => num(l, 'x1'))).toEqual([7.5, 12, 16.5]);
      const dot = g.querySelector('circle')!;
      // 9 to the right of the middle bar, 7.5 above the bars' center line.
      expect(num(dot, 'cx')).toBeCloseTo(21, 2);
      expect(num(dot, 'cy')).toBeCloseTo(4.5, 2);
    }
  });

  it('the lamp stays inside the 24 grid the Drop clips the mark to, at its largest too', () => {
    // At the loudest level, drawn straight: r 2.5 at x 21, so 0.5 from the grid's right edge, 2 from its top.
    const d = dotBox(still('listen', 1))!;
    expect([d.right, d.top]).toEqual([23.5, 2]);
    // Live, driven the way the Drop drives it: the voice level starts at 0 when recording starts (the controller resets
    // it) and is smoothed by Voice, here jumping between silence and the loudest level again and again, with the bars'
    // jitter at its highest. The spring never takes anything out of the grid.
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'start' });
    const voice = new (CUE().Voice)();
    let worst = { left: 24, top: 24, right: 0, bottom: 0 };
    let t = 0;
    for (; t < 1000; t += 16) m.tick(t, 0, voice.jit);
    m.set('listen', t);
    for (let i = 0; i < 20; i++) {
      voice.ext = i % 2 ? 0 : 1;
      for (const end = t + 300; t <= end; t += 16) {
        voice.step(t, true);
        m.tick(t, voice.lvl, [1, 1, 1, 1]);
        const b = paintedBox(g);
        worst = { left: Math.min(worst.left, b.left), top: Math.min(worst.top, b.top), right: Math.max(worst.right, b.right), bottom: Math.max(worst.bottom, b.bottom) };
      }
    }
    expect(worst.left).toBeGreaterThanOrEqual(0.5);
    expect(worst.top).toBeGreaterThanOrEqual(0.5);
    expect(worst.right).toBeLessThanOrEqual(23.5);
    expect(worst.bottom).toBeLessThanOrEqual(23.5);
  });

  it.each(['think', 'done', 'cancel', 'error', 'ask', 'translate'])('%s is centered on x = 12 within 0.3', (state) => {
    expect(Math.abs(paintedBox(still(state)).cx - 12)).toBeLessThanOrEqual(0.3);
  });
});
