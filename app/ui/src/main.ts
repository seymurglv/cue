// The Drop window: draws the core's state and reports clicks and the rectangles for click-through back to it.
import './design/cue.css';
import './design/drop.css';
import './design/tray.css';
import './design/answer.css';
import './app.css';
import './design/cue.js';
import './design/drop.js';
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Answer } from './answer';
import { DropController, type DragPhase, type UiState, type UiAction, type Suggest } from './controller';
import { pickLang, type TrayKind } from './strings';
import type { MicNames } from './trays';

/** A core command without unhandled rejections. The log gets only the command name and the error, never the call data. */
const call = (cmd: string, args?: InvokeArgs) =>
  invoke(cmd, args).catch((e: unknown) => console.error(`CUE: command ${cmd} failed:`, e));

/** Frame loop: runs while frame asks for more; sleeps when quiet, the controller wakes it (wake).
 *  A frame error does not stop the loop — otherwise the click-through rectangles would freeze in the old place. */
let running = false;
let lastFrameError = '';
function loop(now: number) {
  let more = true;
  try {
    more = ctl.frame(now);
  } catch (e) {
    const msg = e instanceof Error ? `${e.name}: ${e.message}` : 'unknown error';
    if (msg !== lastFrameError) console.error('CUE: Drop frame not drawn:', msg);
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

// The core sets the language in an initialization script, before this module runs. When the interface language changes
// while CUE runs, the core loads the page again with the language and the place in its address: they win over the
// script's values, which stay those of startup.
const params = new URLSearchParams(location.search);
const lang = pickLang(params.get('lang') ?? (window as any).__CUE_LANG__);
document.documentElement.lang = lang;
// The place comes the same way as the language, before the Drop is built; later changes come as cue://place.
const place: unknown = params.get('place') ?? (window as any).__CUE_PLACE__;
const ctl = new DropController(document.getElementById('stage')!, {
  send: (action: UiAction) => void call('drop_action', { action }),
  rects: (rects) => void call('drop_hit_rects', { rects }),
  wake,
  drag: (phase: DragPhase) => void call('drop_drag', { phase }),
  source: (n: number) => void call('answer_source', { n }),
}, lang, place);
wake();

/** Subscribe to every core event first, then drop_ready: otherwise the first state could arrive before the subscriptions. */
async function start() {
  const subs = await Promise.allSettled([
    listen<UiState>('cue://state', (e) => ctl.apply(e.payload)),
    listen<number>('cue://level', (e) => ctl.level(e.payload)),
    listen<MicNames>('cue://mic-names', (e) => ctl.micNames(e.payload)),
    listen<string>('cue://model-name', (e) => ctl.modelName(e.payload)),
    listen<string>('cue://habit-rule', (e) => ctl.habitRule(e.payload)),
    listen<string>('cue://cleanup-name', (e) => ctl.cleanupName(e.payload)),
    listen<TrayKind | null>('cue://tray', (e) => ctl.tray(e.payload)),
    listen<Suggest | null>('cue://suggest', (e) => ctl.suggest(e.payload)),
    listen('cue://shake', () => ctl.shake()),
    listen<boolean>('cue://idle-hidden', (e) => ctl.idleHidden(e.payload)),
    listen<string>('cue://place', (e) => ctl.place(e.payload)),
    listen<Answer | null>('cue://answer', (e) => ctl.answer(e.payload)),
  ]);
  for (const s of subs) if (s.status === 'rejected') console.error('CUE: subscribing to a core event failed:', s.reason);
  await call('drop_ready');
}
void start();
