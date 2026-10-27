export {};
// jsdom has no animation frame loop and no ResizeObserver — plug in simple stand-ins.
(globalThis as any).requestAnimationFrame = (cb: FrameRequestCallback) => setTimeout(() => cb(performance.now()), 0) as unknown as number;
(globalThis as any).ResizeObserver = class { observe() {} disconnect() {} };
// The design scripts only put CUE into window and call nothing on load — so imports being hoisted above the stubs does no harm.
// await import() is not an option here: tsc does not treat them as modules (TS2306), but accepts a side-effect import, as in main.ts.
import '../src/design/cue.js';
import '../src/design/drop.js';
