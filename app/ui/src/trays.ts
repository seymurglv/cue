import { CUE } from './designParts';
import { KEY_NAME, TRAYS, type Dict, type TrayKind } from './strings';

/** Tray card content: header — icon, title, close button with the time ring; one thought; button on the right.
 *  data-t on the button label is where the label change starts matching common letters ("Copy" → "Copied"). */
export function trayHtml(kind: TrayKind, t: Dict): string {
  const look = TRAYS[kind];
  const text = t.tray[kind];
  const IC = CUE().IC;
  const body = text.body.replace('{KEY}', `<kbd>${KEY_NAME}</kbd>`);
  const x = `<button class="xb ${look.ttl ? '' : 'nottl'}" style="--ttl:${look.ttl}ms" title="${t.close}" aria-label="${t.close}">${CUE().RING}${IC.x}</button>`;
  const act = look.act && text.act
    ? `<div class="nft"><button class="ab" data-act><span class="ai">${IC[look.act]}</span><span class="lbl" data-t="${text.act}">${text.act}</span></button></div>`
    : '';
  return `<div class="nhd"><span class="nic tone-${look.tone}">${IC[look.icon]}</span><b>${text.title}</b><span class="sp"></span>${x}</div><div class="nbd">${body}</div>${act}`;
}
