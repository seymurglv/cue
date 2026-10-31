// The microphone rows of the settings tray's Microphone page and of the welcome window's Microphone step: the
// microphone CUE records from, chosen among those Windows has, and its level. The level comes from a preview the core
// runs for this window, on only while the rows are shown. Device names go into the page as text.
import * as api from './api';
import type { MicList } from './api';
import { dropDown, h, logFailed, srow, type Ctx, type MenuItem } from './page';
import { fill } from './text';

/** The level meter's segments. */
export const METER_SEGMENTS = 16;

/** Which of `names` the core records from for the pinned `wanted`, by the core's own rule (`mic::pick` in
 *  platform-win): the same name once both are trimmed, else the first that differs only in case; -1 — not there. */
export function matchMic(names: readonly string[], wanted: string): number {
  const w = wanted.trim();
  if (!w) return -1;
  const exact = names.findIndex((n) => n.trim() === w);
  if (exact >= 0) return exact;
  const lw = w.toLowerCase();
  return names.findIndex((n) => n.trim().toLowerCase() === lw);
}

export class MicRows {
  /** The microphones Windows has (null — not listed yet). */
  mics: MicList | null = null;
  private previewOn = false;

  /** `box` — where the rows are shown now (null — nowhere); `listed` — the list came, the rows are to be drawn again. */
  constructor(
    private readonly ctx: Pick<Ctx, 't' | 'settings' | 'menu' | 'change'>,
    private readonly box: () => ParentNode | null,
    private readonly listed: () => void,
  ) {}

  /** The microphone chosen, as the settings name it ("As in Windows" when none is pinned); empty — not known. */
  label(): string {
    const s = this.ctx.settings;
    return s ? (s.micDevice ?? this.ctx.t.windowsMic) : '';
  }

  /** The microphones Windows has, each name once. */
  names(): string[] {
    const seen = new Set<string>();
    const names: string[] = [];
    for (const n of this.mics?.names ?? []) {
      if (seen.has(n.trim())) continue;
      seen.add(n.trim());
      names.push(n);
    }
    return names;
  }

  rows(): HTMLElement[] {
    const t = this.ctx.t;
    const at = this.pinnedAt();
    const shown = at >= 0 ? this.names()[at] : this.label();
    // No label: the heading over the rows already says "Microphone". The drop-down fills the row and keeps the label
    // as its name; its value is its description (and the tooltip of a long device name).
    const dd = dropDown('mic', shown);
    dd.setAttribute('aria-label', t.micLabel);
    dd.title = shown;
    const hint = h('small', 'warn', t.notConnectedHint);
    hint.hidden = !this.pinnedMissing();
    const stx = h('div', 'stx');
    stx.append(dd, hint);
    const choose = h('div', 'srow');
    choose.append(stx);
    const meter = h('span', 'meter');
    meter.dataset.meter = '';
    for (let i = 0; i < METER_SEGMENTS; i++) meter.append(h('i'));
    return [choose, srow(t.micTest, t.micTestHint, meter).row];
  }

  /** The level on the meter shown. */
  level(level: number): void {
    const n = Math.round(Math.min(1, Math.max(0, Number(level) || 0)) * METER_SEGMENTS);
    this.box()?.querySelectorAll('[data-meter] i').forEach((i, k) => i.classList.toggle('on', k < n));
  }

  startPreview(): void {
    this.previewOn = true;
    api.micPreview(true, this.ctx.settings?.micDevice ?? null).catch((e) => logFailed('showing the microphone level', e));
  }

  stopPreview(): void {
    if (!this.previewOn) return;
    this.previewOn = false;
    api.micPreview(false).catch((e) => logFailed('stopping the microphone level', e));
  }

  async read(): Promise<void> {
    try {
      this.mics = await api.micList();
    } catch (e) {
      logFailed('listing the microphones', e);
      return;
    }
    this.listed();
  }

  menu(b: HTMLElement): void {
    const t = this.ctx.t;
    const pinned = this.ctx.settings?.micDevice ?? null;
    const at = this.pinnedAt();
    const windows = this.mics?.default ? fill(t.windowsMicNamed, this.mics.default) : t.windowsMic;
    const items: MenuItem[] = [
      { value: '', label: windows, on: pinned === null },
      ...this.names().map((n, i) => ({ value: n, label: n, on: i === at })),
    ];
    if (pinned && this.pinnedMissing()) items.push({ value: pinned, label: pinned, note: t.notConnected, on: true });
    this.ctx.menu(b, items, (v) => void this.choose(v || null));
  }

  /** The listed microphone the core records from for the pinned one (`matchMic`); -1 — none, or none pinned. */
  private pinnedAt(): number {
    const pinned = this.ctx.settings?.micDevice ?? null;
    return pinned ? matchMic(this.names(), pinned) : -1;
  }

  /** A microphone is pinned, the list is known, and it is not in it. */
  private pinnedMissing(): boolean {
    return !!this.ctx.settings?.micDevice && !!this.mics && this.pinnedAt() < 0;
  }

  private async choose(device: string | null): Promise<void> {
    const view = await this.ctx.change({ kind: 'micDevice', value: device });
    // The level follows the new choice.
    if (view && this.previewOn) {
      api.micPreview(true, view.micDevice).catch((e) => logFailed('showing the microphone level', e));
    }
  }
}
