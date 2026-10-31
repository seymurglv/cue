// The main window's command wrappers over a stand-in `invoke`: what they send and what they make of the answer.
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { historyAudio } from '../src/window/api';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: class {} }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: vi.fn() }));

const ID = '2026-10-31T07:30:00.123-4';

describe('historyAudio', () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it('gives the bytes of the recording, asked for by the entry id', async () => {
    const bytes = new Uint8Array([0x4f, 0x67, 0x67, 0x53]).buffer;
    vi.mocked(invoke).mockResolvedValueOnce(bytes);
    expect(await historyAudio(ID)).toBe(bytes);
    expect(invoke).toHaveBeenCalledWith('history_audio', { id: ID });
  });

  it('gives null when there is no recording to play', async () => {
    vi.mocked(invoke).mockRejectedValueOnce('notFound');
    expect(await historyAudio(ID)).toBeNull();
  });

  it('throws any other code', async () => {
    for (const code of ['notMain', 'badValue', 'failed']) {
      vi.mocked(invoke).mockRejectedValueOnce(code);
      await expect(historyAudio(ID)).rejects.toBe(code);
    }
  });
});
