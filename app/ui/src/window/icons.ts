// The window's icons: the shared set (CUE.IC from design/drop.js) and the ones only the window uses, drawn as in the
// approved design (stage 5): search, chevron, paste again, microphone, globe, half circle, play and pause. The key of
// the "Keys" row is new: the design's key icon is a keyboard, for its hotkeys section, which the window does not have;
// so are the clock of the "History" row, the open book of the milestone (its pages of a book), the sound wave of
// "Speech recognition", the sparkle of "Cleanup" and the trash of a habit's row.
import { CUE } from '../designParts';

const SV = (d: string) =>
  `<svg viewBox="0 0 20 20" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${d}</svg>`;

const OWN = {
  search: SV('<circle cx="9" cy="9" r="5.3"/><path d="m13 13 3.8 3.8"/>'),
  chev: SV('<path d="M8 5l5 5-5 5"/>'),
  reins: SV('<path d="M15.5 4.5v4.8a2.7 2.7 0 0 1-2.7 2.7H4.8"/><path d="M8 8.6 4.6 12 8 15.4"/>'),
  micOn: SV('<rect x="7.4" y="2.8" width="5.2" height="9" rx="2.6"/><path d="M4.8 9.6a5.2 5.2 0 0 0 10.4 0M10 14.8v2.4"/>'),
  globe: SV('<circle cx="10" cy="10" r="7"/><path d="M3 10h14M10 3c2 2.2 2.8 4.5 2.8 7s-.8 4.8-2.8 7c-2-2.2-2.8-4.5-2.8-7S8 5.2 10 3z"/>'),
  half: SV('<circle cx="10" cy="10" r="6.6"/><path d="M10 3.4v13.2a6.6 6.6 0 0 0 0-13.2z" fill="currentColor" stroke="none"/>'),
  apiKey: SV('<circle cx="6.8" cy="13.2" r="3.4"/><path d="m9.2 10.8 7-7M13.6 6.4l2 2M11.6 8.4l1.6 1.6"/>'),
  clock: SV('<circle cx="10" cy="10" r="7"/><path d="M10 6.2V10l2.6 1.7"/>'),
  pages: SV('<path d="M10 5.6C8.5 4.4 6.5 3.9 3.3 3.9v11.3c3.2 0 5.2.5 6.7 1.7 1.5-1.2 3.5-1.7 6.7-1.7V3.9c-3.2 0-5.2.5-6.7 1.7z"/><path d="M10 5.6v11.3"/>'),
  plus: SV('<path d="M10 4.6v10.8M4.6 10h10.8"/>'),
  wave: SV('<path d="M3.5 8.5v3M6.7 6v8M10 3.5v13M13.3 6.5v7M16.5 8.5v3"/>'),
  sparkle: SV('<path d="M10 3.2c.6 3.6 2.8 5.8 6.4 6.4-3.6.6-5.8 2.8-6.4 6.4-.6-3.6-2.8-5.8-6.4-6.4 3.6-.6 5.8-2.8 6.4-6.4z"/>'),
  play: '<svg viewBox="0 0 20 20" width="16" height="16" aria-hidden="true"><path d="M7 5.2v9.6l7.6-4.8z" fill="currentColor"/></svg>',
  pause: SV('<path d="M7.5 5.5v9M12.5 5.5v9"/>'),
  trash: SV('<path d="M4 5.8h12M8 5.8V4h4v1.8M5.6 5.8l.7 10.4h7.4l.7-10.4"/>'),
};

export type IconName = keyof typeof OWN | 'x' | 'ok' | 'tr' | 'ask' | 'copy' | 'gear' | 'book' | 'model' | 'retry' | 'habit';

let all: Record<string, string> | null = null;

/** Trusted SVG markup of an icon (the design scripts must have run: they fill window.CUE). */
export function icon(name: IconName): string {
  all ??= { ...CUE().IC, ...OWN };
  return all![name];
}
