// @vitest-environment jsdom
/**
 * The media pool is not drawn again as the playhead moves: the transport tells where it is up to
 * 30 times a second, and a project may hold a thousand media. A row is drawn again only when what
 * it shows changes. How often the rows are drawn is counted by the names they write.
 */
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { ProjectCopy, type EngineAnswer, type EngineEvent, type Media, type TransportState } from '../src/studio/engine';
import { Studio } from '../src/studio/Studio';
import { fileName } from '../src/studio/timecode';
import { likeABrowser } from './mounted';

vi.mock('../src/studio/timecode', async (original) => {
  const real = await original<typeof import('../src/studio/timecode')>();
  return { ...real, fileName: vi.fn(real.fileName) };
});

beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('requestAnimationFrame', () => 0);
  vi.stubGlobal('cancelAnimationFrame', () => {});
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const COUNT = 1000;
const media = (at: number): Media => ({
  id: `m${at}`, path: `C:\\media\\clip-${at}.mp4`,
  identity: { size: 1000, modified_us: 0, digest: '0'.repeat(64) },
  facts: { duration: { ticks: 250, scale: 25 }, video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'h264', bits: 8, rotation: 0 }, audio: null },
});
const state = (frame: number): TransportState => ({
  media: 'm1', frame, playing: true, speed: 1, exact: true, shown: frame, last: 249, sound: 'none', source: 'original', proxies: true,
});

describe('the media pool of a large project', () => {
  it('is not drawn again as the playhead moves, and draws again only the row whose proxy changed', async () => {
    const listeners = new Set<(event: EngineEvent) => void>();
    const tell = (event: EngineEvent) => act(() => { for (const listener of [...listeners]) listener(event); });
    const project = { schema: 1, next_id: COUNT + 1, media: Array.from({ length: COUNT }, (_, at) => media(at + 1)), timelines: [] };
    const bridge = {
      open: async () => {
        const opened = { session: 1, project: 'F:\\film\\.columbina\\studio', owned: true, pid: 1, revision: 0 };
        for (const listener of [...listeners]) listener({ kind: 'opened', ...opened });
        return opened;
      },
      call: async (method: string): Promise<EngineAnswer> => (method === 'project.get'
        ? { ok: true, result: { revision: 0, next_id: project.next_id, project } }
        : { ok: true, result: state(0) }),
      importFiles: async () => ({ ok: true, result: null }),
      viewRect: async () => {},
      viewShow: async () => {},
      close: async () => {},
      onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    } as unknown as EngineBridge;
    render(<Studio cwd={'F:\\film'} engine={{ bridge, copy: new ProjectCopy(bridge), native: true }} language="en" onClose={() => {}} />);
    await act(async () => { for (let at = 0; at < 10; at += 1) await new Promise(resolve => setTimeout(resolve, 0)); });
    const named = vi.mocked(fileName);
    expect(document.querySelectorAll('[data-test="pool-item"]')).toHaveLength(COUNT);
    named.mockClear();
    const began = performance.now();
    for (let frame = 1; frame <= 30; frame += 1) tell({ kind: 'notification', session: 1, method: 'transport.state', params: state(frame) });
    const took = performance.now() - began;
    // Only the name of the media loaded, over the viewer, is written again: once a state.
    expect(named.mock.calls.length).toBeLessThanOrEqual(30);
    expect(document.querySelector('[data-test="timecode"]')!.textContent).toBe('00:00:01:05');
    // A second of playing, told 30 times, takes far less than a second to draw.
    expect(took).toBeLessThan(500);
    named.mockClear();
    tell({ kind: 'notification', session: 1, method: 'media.proxy', params: { media: 'm7', state: 'running', progress: 0.5 } });
    // The row of m7, and the name over the viewer.
    expect(named.mock.calls.map(([path]) => path)).toEqual(expect.arrayContaining(['C:\\media\\clip-7.mp4']));
    expect(named.mock.calls.length).toBeLessThanOrEqual(2);
  });
});
