// @vitest-environment jsdom
/**
 * How long React takes to draw a timeline of a thousand clips (six video tracks and four of sound
 * whose waveforms are ready) as it scrolls by 250 px at a time and zooms in and out: the 95th
 * percentile of all the draws is held below a frame of 60 Hz (16 ms). One round goes first that is
 * not timed, for the code to be compiled as it is when the page has been used a while.
 *
 * This file runs in the project of performance tests, alone in one process after the others (see
 * vitest.config.ts). What is drawn again, and what not, is counted in timeline-renders.test.tsx.
 */
import { Profiler } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Timeline } from '../src/studio/timeline/Timeline';
import { media } from './timeline-fixture';
import { bigTimeline, readyBridge, spread } from './timeline-perf-fixture';
import { likeABrowser } from './mounted';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

describe('a thousand-clip timeline', () => {
  it('draws a scroll or a zoom in less than a frame of 60 Hz at the 95th percentile', async () => {
    likeABrowser();
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956 + 156);
    let pending: FrameRequestCallback | null = null;
    vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { pending = callback; return 1; });
    vi.stubGlobal('cancelAnimationFrame', () => { pending = null; });
    const times: number[] = [];
    let timing = false;
    render(<Profiler id="timeline" onRender={(_id, phase, actual) => { if (timing && phase !== 'mount') times.push(actual); }}>
      <Timeline timeline={bigTimeline()} media={[media]} bridge={readyBridge()} session={1} revision={0} frame={0} playing={false} onSeek={vi.fn()} />
    </Profiler>);
    // The waveforms come from the engine at once; they are drawn once they have come.
    await act(async () => { for (let at = 0; at < 5; at++) await new Promise(resolve => setTimeout(resolve, 0)); });
    const scroll = document.querySelector<HTMLElement>('.timeline-scroll')!;
    const root = document.querySelector<HTMLElement>('.timeline')!;
    for (let round = 0; round < 4; round++) {
      timing = round > 0;
      for (let step = 1; step <= 60; step++) {
        scroll.scrollLeft = (round * 60 + step) * 250;
        fireEvent.scroll(scroll);
        act(() => { const frame = pending; pending = null; frame?.(0); });
        if (step % 10 === 0) fireEvent.keyDown(root, { key: step % 20 === 0 ? '-' : '=', ctrlKey: true });
        expect(document.querySelectorAll('[data-clip]').length).toBeLessThanOrEqual(60);
      }
    }
    expect(times.length).toBeGreaterThanOrEqual(180);
    const { p50, p95, max } = spread(times);
    if (process.env.COLUMBINA_PERF) console.info(`timeline perf: ${times.length} draws, p50 ${p50.toFixed(2)} ms, p95 ${p95.toFixed(2)} ms, max ${max.toFixed(2)} ms`);
    expect(p95).toBeLessThan(16);
  });
});
