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

/** The Drop's circle on the mark's grid: the grid is 0.6 of the Drop's height, so the circle's radius is 20 around (12, 12). */
const RIM_R = 20;
/** How far inside the circle's rim the lamp's edge is; null when it is not drawn. */
function rimGap(g: SVGGElement) {
  const dot = g.querySelector('circle')!;
  if (num(dot, 'opacity') <= 0 || num(dot, 'r') <= 0) return null;
  return RIM_R - Math.hypot(num(dot, 'cx') - 12, num(dot, 'cy') - 12) - num(dot, 'r');
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

/** The mark drawn straight in `state` (no spring), as the static pages draw it; `rim` as the Drop makes it. */
function still(state: string, lvl = 0, rim = false) {
  const g = group();
  new (CUE().Morph)(g, { state, static: true, lvl, rim });
  return g;
}

/** The live mark after `ms` of frames in `state` at a constant voice level: what the Drop shows. */
function live(state: string, lvl = 0, ms = 3000, rim = false) {
  const g = group();
  const m = new (CUE().Morph)(g, { state, rim });
  for (let t = 0; t <= ms; t += 16) m.tick(t, lvl, [0.5, 0.5, 0.5, 0.5]);
  return g;
}

describe('The idle mark', () => {
  it('is the dictation wave: the three voice bars on y = 12 and the lamp at their top right, inside the grid', () => {
    for (const g of [still('idle'), live('idle')]) {
      const all = [...g.querySelectorAll('line')];
      const bars = [all[0], all[2], all[4]];
      expect(bars.map((l) => num(l, 'x1'))).toEqual([7.5, 12, 16.5]);
      expect([all[1], all[3]].map((l) => num(l, 'opacity'))).toEqual([0, 0]);
      for (const l of bars) expect((num(l, 'y1') + num(l, 'y2')) / 2).toBeCloseTo(12, 2);
      const b = paintedBox(g);
      expect(b.left).toBeGreaterThanOrEqual(0.5);
      expect(b.right).toBeLessThanOrEqual(23.5);
      expect(dotBox(g)!.left).toBeGreaterThan(linesBox(g).right - 1);
    }
    // Drawn still, the bars stand in the voice's proportions, 16 at the middle.
    const all = [...still('idle').querySelectorAll('line')];
    expect([all[0], all[2], all[4]].map((l) => +(num(l, 'y2') - num(l, 'y1')).toFixed(2))).toEqual([11.52, 16, 12.8]);
    const dot = still('idle').querySelector('circle')!;
    expect([num(dot, 'cx'), num(dot, 'cy'), num(dot, 'r'), num(dot, 'opacity')]).toEqual([21, 4.5, 1.9, 1]);
  });

  it('from rest through start into listen the bars and the lamp stay where they are: only the heights change', () => {
    const at = (state: string) => {
      const g = still(state);
      const all = [...g.querySelectorAll('line')];
      const dot = g.querySelector('circle')!;
      return { x: all.map((l) => num(l, 'x1')), o: all.map((l) => num(l, 'opacity')), lamp: [num(dot, 'cx'), num(dot, 'cy')] };
    };
    expect(at('start')).toEqual(at('idle'));
    expect(at('listen')).toEqual(at('idle'));
  });

  it('breathes: the bars roll as a wave and the lamp fades in and out, while its place and size stay', () => {
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'idle' });
    const heights = new Set<string>();
    const lamp = new Set<string>();
    for (let t = 0; t <= 2400; t += 16) {
      m.tick(t);
      const l = g.querySelectorAll('line');
      heights.add([...l].map((x) => (num(x, 'y2') - num(x, 'y1')).toFixed(1)).join());
      const dot = g.querySelector('circle')!;
      lamp.add(num(dot, 'opacity').toFixed(2));
      expect([num(dot, 'cx'), num(dot, 'r')]).toEqual([21, 1.9]);
    }
    expect(heights.size).toBeGreaterThan(20);
    expect(lamp.size).toBeGreaterThan(10);
  });
});

describe('The mark in the circle', () => {
  it('start: the three voice bars at rest centered on x = 12 and y = 12, the lamp at their right side', () => {
    for (const g of [still('start'), live('start')]) {
      const bars = linesBox(g);
      expect(Math.abs(bars.cx - 12)).toBeLessThanOrEqual(0.05);
      expect(Math.abs(bars.cy - 12)).toBeLessThanOrEqual(0.05);
      expect(dotBox(g)!.left).toBeGreaterThan(bars.right);
      const all = [...g.querySelectorAll('line')];
      expect([all[0], all[2], all[4]].map((l) => num(l, 'x1'))).toEqual([7.5, 12, 16.5]);
      expect([all[1], all[3]].map((l) => num(l, 'opacity'))).toEqual([0, 0]);
    }
    const dot = still('start').querySelector('circle')!;
    expect([num(dot, 'cx'), num(dot, 'cy')]).toEqual([21, 4.5]);
  });

  it('from start into listen nothing moves while the microphone is silent: the bars wait for the voice', () => {
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'start' });
    let t = 0;
    for (; t < 1200; t += 16) m.tick(t);
    const attrs = () => [...g.querySelectorAll('line, circle')].map((e) => [...e.attributes].map((a) => `${a.name}=${a.value}`).join()).join('|');
    const before = attrs();
    m.set('listen', t);
    for (const end = t + 1200; t < end; t += 16) {
      m.tick(t, 0);
      expect(attrs(), `${t} ms`).toBe(before);
    }
  });

  it('from idle into start no tall bar shows up: the wave only gathers to the middle', () => {
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'idle' });
    let t = 0;
    for (; t < 1000; t += 16) m.tick(t);
    m.set('start', t);
    for (const end = t + 1200; t < end; t += 16) {
      m.tick(t);
      for (const l of g.querySelectorAll('line')) {
        if (num(l, 'opacity') <= 0) continue;
        expect(num(l, 'y2') - num(l, 'y1'), `${t} ms`).toBeLessThanOrEqual(16.5);
      }
    }
    // Settled, every bar is short.
    for (const l of g.querySelectorAll('line')) expect(num(l, 'y2') - num(l, 'y1')).toBeLessThanOrEqual(7);
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
      // The voice bars are lines 0, 2 and 4 (start's too); the other two fade between them.
      const all = [...g.querySelectorAll('line')];
      const lines = [all[0], all[2], all[4]];
      for (const l of lines) expect((num(l, 'y1') + num(l, 'y2')) / 2, `level ${lvl}`).toBeCloseTo(12, 2);
      expect([all[1], all[3]].map((l) => num(l, 'opacity'))).toEqual([0, 0]);
      expect(lines.map((l) => num(l, 'x1'))).toEqual([7.5, 12, 16.5]);
      const dot = g.querySelector('circle')!;
      // 9 to the right of the middle bar, 7.5 above the bars' center line.
      expect(num(dot, 'cx')).toBeCloseTo(21, 2);
      expect(num(dot, 'cy')).toBeCloseTo(4.5, 2);
    }
  });

  it('drawn anywhere but the Drop, the lamp stays inside the 24 grid, at its largest too', () => {
    // At the loudest level, drawn straight: r 2.5 at x 21, so 0.5 from the grid's right edge, 2 from its top.
    const d = dotBox(still('listen', 1))!;
    expect([d.right, d.top]).toEqual([23.5, 2]);
  });

  /** Drives the live mark the way the Drop does: `from` until it has settled, then (after `startMs` in start, when
   *  given) listen, the voice level starting at 0 (the controller resets it) and smoothed by Voice, here jumping between
   *  silence and the loudest level again and again, with the bars' jitter at its highest. The farthest the mark's
   *  paint reaches. */
  function intoListen(from: string, startMs: number | null, rim = false) {
    const g = group();
    const m = new (CUE().Morph)(g, { state: from, rim });
    const voice = new (CUE().Voice)();
    let worst = { left: 24, top: 24, right: 0, bottom: 0, rimGap: 99 };
    // In the Drop the lamp is measured against the circle's rim, the lines against the grid.
    const track = () => {
      const b = rim ? linesBox(g) : paintedBox(g);
      const gap = rim ? rimGap(g) : null;
      worst = {
        left: Math.min(worst.left, b.left), top: Math.min(worst.top, b.top), right: Math.max(worst.right, b.right), bottom: Math.max(worst.bottom, b.bottom),
        rimGap: gap == null ? worst.rimGap : Math.min(worst.rimGap, gap),
      };
    };
    let t = 0;
    for (; t < 1000; t += 16) m.tick(t, 0, voice.jit);
    if (startMs != null) {
      m.set('start', t);
      for (const end = t + startMs; t < end; t += 16) {
        m.tick(t, 0, voice.jit);
        track();
      }
    }
    m.set('listen', t);
    for (let i = 0; i < 20; i++) {
      voice.ext = i % 2 ? 0 : 1;
      for (const end = t + 300; t <= end; t += 16) {
        voice.step(t, true);
        m.tick(t, voice.lvl, [1, 1, 1, 1]);
        track();
      }
    }
    return worst;
  }

  // Into listen: from start (a dictation), from ask and translate (their start poses), and from every other pose
  // through a start of any length, short ones included (a new dictation right after the last one ended). Everything
  // stays 0.5 inside the grid: the shapes by their own geometry, the lamp also by being drawn no closer to the edge. In
  // the Drop the lamp, near the circle's rim, stays RIM_GAP inside the circle instead.
  const ways: [string, number | null][] = [
    ['start', null], ['ask', null], ['translate', null],
    ...['idle', 'think', 'done', 'cancel', 'error', 'ask', 'translate'].flatMap((s) => [0, 50, 150, 400].map((ms): [string, number] => [s, ms])),
  ];
  it.each(ways)('from %s (through start for %s ms) the spring keeps everything inside the grid', (from, startMs) => {
    for (const rim of [false, true]) {
      const worst = intoListen(from, startMs, rim);
      // The attributes are written to 3 decimals.
      const eps = 1e-3;
      const at = rim ? 'in the Drop' : 'as drawn';
      expect(worst.left, at).toBeGreaterThanOrEqual(0.5 - eps);
      expect(worst.top, at).toBeGreaterThanOrEqual(0.5 - eps);
      expect(worst.right, at).toBeLessThanOrEqual(23.5 + eps);
      expect(worst.bottom, at).toBeLessThanOrEqual(23.5 + eps);
      if (rim) expect(worst.rimGap, at).toBeGreaterThanOrEqual(CUE().RIM_GAP - 2 * eps);
    }
  });

  it('ask and translate park their hidden lamp at the listen lamp\'s spot, so no clamp is needed on their way in', () => {
    for (const state of ['ask', 'translate']) {
      const dot = still(state).querySelector('circle')!;
      expect([num(dot, 'cx'), num(dot, 'cy'), num(dot, 'opacity')], state).toEqual([21, 4.5, 0]);
    }
  });

  it('the lamp is drawn where its pose puts it in every still pose: the edge line is for overshoots only', () => {
    for (const state of ['idle', 'start', 'listen', 'think', 'done', 'cancel', 'error', 'ask', 'translate']) {
      for (const lvl of [0, 1]) {
        const g = group();
        const m = new (CUE().Morph)(g, { state, static: true, lvl });
        const dot = g.querySelector('circle')!;
        expect([num(dot, 'cx'), num(dot, 'cy')], state).toEqual([+m.x.dx.toFixed(3), +m.x.dy.toFixed(3)]);
      }
    }
  });

  it.each(['think', 'done', 'cancel', 'error', 'ask', 'translate'])('%s is centered on x = 12 within 0.3', (state) => {
    expect(Math.abs(paintedBox(still(state)).cx - 12)).toBeLessThanOrEqual(0.3);
  });
});

describe("The Drop's mark: the bars centered on the circle, the lamp a status light near its rim", () => {
  /** One grid unit in CSS px in the 36 px Drop (the grid is 0.6 of its height), and the owner's tolerance: 0.25 px. */
  const PX = (36 * 0.6) / 24;
  const TOL = 0.25 / PX;
  /** Where the visible lines' ink sits on average: each stroke a capsule weighted by its area. */
  function ink(g: SVGGElement) {
    let w = 0;
    let x = 0;
    let y = 0;
    for (const l of g.querySelectorAll('line')) {
      if (num(l, 'opacity') <= 0) continue;
      const sw = num(l, 'stroke-width');
      const len = Math.hypot(num(l, 'x2') - num(l, 'x1'), num(l, 'y2') - num(l, 'y1'));
      const a = len * sw + Math.PI * (sw / 2) ** 2;
      w += a;
      x += a * (num(l, 'x1') + num(l, 'x2')) / 2;
      y += a * (num(l, 'y1') + num(l, 'y2')) / 2;
    }
    return { x: x / w, y: y / w };
  }
  /** The shortest gap between the lamp's edge and a visible line's painted edge. */
  function barGap(g: SVGGElement) {
    const dot = g.querySelector('circle')!;
    const [cx, cy, r] = [num(dot, 'cx'), num(dot, 'cy'), num(dot, 'r')];
    let gap = Infinity;
    for (const l of g.querySelectorAll('line')) {
      if (num(l, 'opacity') <= 0) continue;
      const [x1, y1, x2, y2] = ['x1', 'y1', 'x2', 'y2'].map((a) => num(l, a));
      const len2 = (x2 - x1) ** 2 + (y2 - y1) ** 2;
      const t = len2 ? Math.min(1, Math.max(0, ((cx - x1) * (x2 - x1) + (cy - y1) * (y2 - y1)) / len2)) : 0;
      gap = Math.min(gap, Math.hypot(cx - (x1 + t * (x2 - x1)), cy - (y1 + t * (y2 - y1))) - num(l, 'stroke-width') / 2 - r);
    }
    return gap;
  }
  /** The bars as the Drop shows them: drawn still, and live after 3 s at a constant level with even jitter. */
  const poses = (rim = true) => [
    ['idle', still('idle', 0, rim)], ['idle live', live('idle', 0, 3000, rim)],
    ...[0, 0.5, 1].flatMap((lvl) => [
      [`start at ${lvl}`, still('start', lvl, rim)], [`listen at ${lvl}`, still('listen', lvl, rim)],
      [`listen live at ${lvl}`, live('listen', lvl, 3000, rim)],
    ]),
    ['think live', live('think', 0, 3000, rim)],
  ] as [string, SVGGElement][];

  it('the bars are centered on the circle within 0.25 px, by their box and by their ink, in every pose they show in', () => {
    for (const [at, g] of poses()) {
      const b = linesBox(g);
      expect(Math.abs(b.cx - 12), `${at}: box x`).toBeLessThanOrEqual(TOL);
      if (!at.startsWith('think')) expect(Math.abs(b.cy - 12), `${at}: box y`).toBeLessThanOrEqual(TOL);
      // The idle wave rolls the bars' heights, so its ink moves with the wave; drawn still it is centered.
      if (at !== 'idle live') expect(Math.abs(ink(g).x - 12), `${at}: ink x`).toBeLessThanOrEqual(TOL);
    }
    // The bars stand where every other mark draws them: on 7.5 / 12 / 16.5.
    const all = [...still('listen', 1, true).querySelectorAll('line')];
    expect([all[0], all[2], all[4]].map((l) => num(l, 'x1'))).toEqual([7.5, 12, 16.5]);
  });

  it('the lamp sits up and right at 45 degrees, wholly inside the circle, at least RIM_GAP from its rim and clear of the bars', () => {
    expect(CUE().RIM_LAMP).toEqual({ x: 22, y: 2 });
    expect(CUE().RIM_GAP).toBe(3);
    for (const [at, g] of poses()) {
      if (at.startsWith('think')) continue;
      const dot = g.querySelector('circle')!;
      expect([num(dot, 'cx'), num(dot, 'cy')], at).toEqual([22, 2]);
      expect(rimGap(g)!, `${at}: rim`).toBeGreaterThanOrEqual(CUE().RIM_GAP);
      expect(barGap(g), `${at}: bars`).toBeGreaterThanOrEqual(3);
    }
    // The loudest voice with the tallest jitter: r 2.5, 3.36 inside the rim (3 px) and 3.1 from the bars (2.8 px).
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'listen', rim: true });
    for (let t = 0; t <= 3000; t += 16) m.tick(t, 1, [1, 1, 1, 1]);
    expect(num(g.querySelector('circle')!, 'r')).toBeCloseTo(2.5, 3);
    expect(rimGap(g)!).toBeCloseTo(20 - 10 * Math.SQRT2 - 2.5, 2);
    expect(barGap(g)).toBeGreaterThanOrEqual(3);
  });

  it('every pose that hides the lamp parks it at its spot, so the light goes out where it is', () => {
    for (const state of ['think', 'done', 'cancel', 'ask', 'translate']) {
      const dot = still(state, 0, true).querySelector('circle')!;
      expect([num(dot, 'cx'), num(dot, 'cy'), num(dot, 'r'), num(dot, 'opacity')], state).toEqual([22, 2, 0, 0]);
    }
    // Elsewhere the mark is drawn as before.
    expect([num(still('listen').querySelector('circle')!, 'cx'), num(still('think').querySelector('circle')!, 'cx')]).toEqual([21, 12]);
  });

  it('every other pose is centered as it is drawn anywhere', () => {
    for (const state of ['done', 'cancel', 'error', 'ask', 'translate']) {
      expect(Math.abs(paintedBox(still(state, 0, true)).cx - 12), state).toBeLessThanOrEqual(0.3);
    }
  });

  it('from rest through start, listen and think into done the bars never move sideways, frame by frame', () => {
    const g = group();
    const m = new (CUE().Morph)(g, { state: 'idle', rim: true });
    const voice = new (CUE().Voice)();
    const bars = () => [...g.querySelectorAll('line')].filter((_, i) => i % 2 === 0).map((l) => [num(l, 'x1'), num(l, 'x2')]);
    let t = 0;
    let worst = 0;
    let lamp = { x: 0, y: 0 };
    const run = (state: string | null, ms: number, lvl: () => number) => {
      if (state) m.set(state, t);
      for (const end = t + ms; t < end; t += 16) {
        voice.step(t, state === 'listen');
        m.tick(t, lvl(), voice.jit);
        bars().forEach(([x1, x2], i) => (worst = Math.max(worst, Math.abs(x1 - [7.5, 12, 16.5][i]), Math.abs(x2 - [7.5, 12, 16.5][i]))));
        const dot = g.querySelector('circle')!;
        if (num(dot, 'opacity') > 0) lamp = { x: Math.max(lamp.x, Math.abs(num(dot, 'cx') - 22)), y: Math.max(lamp.y, Math.abs(num(dot, 'cy') - 2)) };
      }
    };
    run(null, 1000, () => 0);
    run('start', 300, () => 0);
    for (let i = 0; i < 6; i++) {
      voice.ext = i % 2 ? 0.1 : 1;
      run(i ? null : 'listen', 400, () => voice.lvl);
    }
    run('think', 1500, () => 0);
    expect(worst).toBe(0);
    // While it shows, the lamp too stays where it is: it only grows, shrinks and goes out.
    expect(lamp).toEqual({ x: 0, y: 0 });
    // Into done the dots draw the check, which comes to rest centered.
    m.set('done', t);
    for (const end = t + 1500; t < end; t += 16) m.tick(t, 0, voice.jit);
    expect(Math.abs(linesBox(g).cx - 12)).toBeLessThanOrEqual(0.3);
  });
});
