// The card window: draws the "Ask" answer at the selection and reports the card's rectangle for click-through.
import './design/cue.css';
import './app.css';
import './design/answer.css';
import './design/cue.js';
import './design/drop.js';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Answer } from './answer';
import { CardPage, type CardPlace } from './cardPage';
import type { UiState } from './controller';
import { pickLang } from './strings';

/** A core command without unhandled rejections. The log gets only the command name and the error, never the call data. */
const call = (cmd: string, args?: InvokeArgs) =>
  invoke(cmd, args).catch((e: unknown) => console.error(`CUE: command ${cmd} failed:`, e));

/** Frame loop, as in the Drop window: runs while the page asks for more, the page wakes it. */
let running = false;
let lastFrameError = '';
function loop(now: number) {
  let more = true;
  try {
    more = page.frame(now);
  } catch (e) {
    const msg = e instanceof Error ? `${e.name}: ${e.message}` : 'unknown error';
    if (msg !== lastFrameError) console.error('CUE: card frame not drawn:', msg);
    lastFrameError = msg;
  }
  if (more) requestAnimationFrame(loop);
  else running = false;
}
function wake() {
  if (running) return;
  running = true;
  requestAnimationFrame(loop);
}

const lang = pickLang((window as any).__CUE_LANG__);
document.documentElement.lang = lang;
const page = new CardPage(
  document.getElementById('card')!,
  {
    act: (action) => void call('drop_action', { action }),
    source: (n) => void call('answer_source', { n }),
    rects: (rects) => void call('card_hit_rects', { rects }),
    wake,
  },
  lang,
  () => !!window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches,
);
wake();

/** Subscribe first, then card_ready: otherwise the state, the place or the answer could come before the subscriptions. */
async function start() {
  const subs = await Promise.allSettled([
    listen<UiState>('cue://state', (e) => page.apply(e.payload)),
    listen<Answer | null>('cue://answer', (e) => page.answer(e.payload)),
    listen<CardPlace>('cue://card-place', (e) => page.place(e.payload)),
    listen<number>('cue://level', (e) => page.level(e.payload)),
    listen('cue://shake', () => page.shake()),
  ]);
  for (const s of subs) if (s.status === 'rejected') console.error('CUE: subscribing to a core event failed:', s.reason);
  await call('card_ready');
}
void start();
