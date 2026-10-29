// @vitest-environment jsdom
import { Profiler } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { Timeline } from '../src/studio/timeline/Timeline';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });
describe('a thousand-clip timeline', () => {
  it('renders only its window, keeps clips untouched during playback, and renders scroll and zoom frames below 16 ms', () => {
    likeABrowser();
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
    let pending: FrameRequestCallback | null = null;
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { pending = callback; return 1; });
    vi.stubGlobal('cancelAnimationFrame', () => { pending = null; });
    const t = timeline([]);
    t.tracks = Array.from({ length: 10 }, (_, row) => ({
      id: `v${row}`, kind: 'video', name: `V${row}`, enabled: true, locked: false, transitions: [],
      clips: Array.from({ length: 100 }, (_, i) => clip(`c${row}_${i}`, i * 200, 180)),
    }));
    const bridge = { call: vi.fn(), onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
    const props = { timeline: t, media: [media], bridge, session: 1, frame: 0, playing: false, onSeek: vi.fn() };
    const durations: number[] = [];
    const tree = (frame = 0) => <Profiler id="timeline" onRender={(_id, phase, actual) => { if (phase !== 'mount') durations.push(actual); }}><Timeline {...props} frame={frame} /></Profiler>;
    const view = render(tree());
    expect(document.querySelectorAll('[data-clip]').length).toBeLessThanOrEqual(30);
    const first = document.querySelector('[data-clip]')!;
    const changes = new MutationObserver(() => {});
    changes.observe(first, { attributes: true, childList: true, characterData: true, subtree: true });
    for (let frame = 1; frame <= 20; frame++) view.rerender(tree(frame));
    expect(changes.takeRecords()).toEqual([]);
    expect(document.querySelector('[data-clip]')).toBe(first);
    changes.disconnect();
    const scroll = document.querySelector<HTMLElement>('.timeline-scroll')!;
    const root = document.querySelector<HTMLElement>('.timeline')!;
    durations.length = 0;
    for (let step = 1; step <= 30; step++) {
      scroll.scrollLeft = step * 250;
      fireEvent.scroll(scroll);
      act(() => { const frame = pending; pending = null; frame?.(0); });
      if (step % 5 === 0) fireEvent.keyDown(root, { key: step % 10 === 0 ? '-' : '=', ctrlKey: true });
      expect(document.querySelectorAll('[data-clip]').length).toBeLessThanOrEqual(40);
    }
    expect(durations.length).toBeGreaterThanOrEqual(30);
    expect(Math.max(...durations)).toBeLessThan(16);
  });
});
