import { CUE } from './designParts';
import { KEY_NAME, TRAYS, type Dict, type TrayKind } from './strings';

/** The pinned microphone that is not there and the one recording instead, as Windows names them (cue://mic-names). */
export interface MicNames {
  wanted: string;
  used: string;
}

/** Text for the trusted HTML of a card body: a device name may hold anything. */
function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** Tray card content: header — icon, title, close button with the time ring; one thought; button on the right.
 *  data-t on the button label is where the label change starts matching common letters ("Copy" → "Copied").
 *  names fill the missing microphone's card, rule the habit's ({RULE}), escaped; replaced by functions, so a "$&" in a name stays as it is. */
export function trayHtml(kind: TrayKind, t: Dict, names?: MicNames | null, model?: string | null, rule?: string | null): string {
  const look = TRAYS[kind];
  const text = t.tray[kind];
  const IC = CUE().IC;
  const used = names?.used || text.unnamed || '';
  const rawRule = (rule ?? '').trim();
  // No rule: the sentence alone. A rule that ends with its own stop gets no second one.
  const form = !rawRule && text.bare != null ? text.bare : /[.!?…]$/.test(rawRule) ? text.body.replace('{RULE}.', '{RULE}') : text.body;
  const body = form
    .replace('{KEY}', `<kbd>${KEY_NAME}</kbd>`)
    .replace('{WANTED}', () => escapeHtml(names?.wanted ?? ''))
    .replace('{USED}', () => escapeHtml(used))
    .replace('{NAME}', () => escapeHtml(model || text.unnamed || ''))
    .replace('{RULE}', () => escapeHtml(rawRule));
  const x = `<button class="xb ${look.ttl ? '' : 'nottl'}" style="--ttl:${look.ttl}ms" title="${t.close}" aria-label="${t.close}">${CUE().RING}${IC.x}</button>`;
  const act = look.act && text.act
    ? `<div class="nft"><button class="ab" data-act><span class="ai">${IC[look.act]}</span><span class="lbl" data-t="${text.act}">${text.act}</span></button></div>`
    : '';
  return `<div class="nhd"><span class="nic tone-${look.tone}">${IC[look.icon]}</span><b>${text.title}</b><span class="sp"></span>${x}</div><div class="nbd">${body}</div>${act}`;
}
