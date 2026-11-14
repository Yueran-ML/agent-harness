// @vitest-environment jsdom
/**
 * A timeline of a thousand clips, six video tracks and four of sound whose waveforms are ready.
 *
 * What is drawn again is counted, not timed: as the playhead moves, no track, clip, tick of the
 * ruler or button of the bar is drawn again; as the view scrolls within the half view that the
 * window moves by, no track or clip is, and no waveform is worked out again; where it crosses into
 * the next, the waveform tiles already drawn are kept and only those that come in are worked out;
 * a zoom does not draw the bar again. How long the drawing takes is timed apart, without these
 * counters, in timeline.perf.test.tsx.
 */
import { memo, type FunctionComponent } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Timeline } from '../src/studio/timeline/Timeline';
import { TILE } from '../src/studio/timeline/Waveform';
import { media } from './timeline-fixture';
import { bigTimeline, readyBridge } from './timeline-perf-fixture';
import { likeABrowser } from './mounted';

const drawn = vi.hoisted(() => ({ rows: 0, clips: 0, ticks: 0, toolbar: 0, columns: 0 }));
/** A component of a module, drawn as it is (a memo component stays one) and counted under `name` each time it is drawn. */
async function counted(original: () => Promise<Record<string, unknown>>, exported: string, name: keyof typeof drawn) {
  const real = await original();
  const component = real[exported] as FunctionComponent<object> & { type?: FunctionComponent<object> };
  const inner = component.type ?? component;
  const drawing = (props: object) => { drawn[name]++; return inner(props); };
  return { ...real, [exported]: component.type ? memo(drawing) : drawing };
}
vi.mock('../src/studio/timeline/TrackRow', original => counted(original, 'TrackRow', 'rows'));
vi.mock('../src/studio/timeline/ClipView', original => counted(original, 'ClipView', 'clips'));
vi.mock('../src/studio/timeline/RulerTicks', original => counted(original, 'RulerTicks', 'ticks'));
vi.mock('../src/studio/timeline/Toolbar', original => counted(original, 'Toolbar', 'toolbar'));
// Each column of a waveform asks for its peaks, a channel at a time: counted, the work of drawing waveforms.
vi.mock('../src/studio/timeline/waveforms', async original => {
  const real = await original<typeof import('../src/studio/timeline/waveforms')>();
  const extent = real.PeakData.prototype.extent;
  real.PeakData.prototype.extent = function counting(this: InstanceType<typeof real.PeakData>, ...given: Parameters<typeof extent>) { drawn.columns++; return extent.apply(this, given); };
  return real;
});

const WIDTH = 956;
let pending: FrameRequestCallback | null = null;
beforeEach(() => {
  likeABrowser();
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(WIDTH + 156);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { pending = callback; return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => { pending = null; });
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

async function mount() {
  const props = { timeline: bigTimeline(), media: [media], bridge: readyBridge(), session: 1, revision: 0, frame: 0, playing: false, onSeek: vi.fn() };
  const tree = (frame = 0, playing = false) => <Timeline {...props} frame={frame} playing={playing} />;
  const view = render(tree());
  // The waveforms come from the engine at once; they are drawn once they have come.
  await act(async () => { for (let at = 0; at < 5; at++) await new Promise(resolve => setTimeout(resolve, 0)); });
  const scroll = document.querySelector<HTMLElement>('.timeline-scroll')!;
  const root = document.querySelector<HTMLElement>('.timeline')!;
  const scrollTo = (left: number) => {
    scroll.scrollLeft = left;
    fireEvent.scroll(scroll);
    act(() => { const frame = pending; pending = null; frame?.(0); });
  };
  const reset = () => { for (const name of Object.keys(drawn) as (keyof typeof drawn)[]) drawn[name] = 0; };
  return { view, tree, scrollTo, root, reset };
}

describe('a thousand-clip timeline', () => {
  it('draws its window, with waveforms, and nothing of it again as the playhead moves', async () => {
    const { view, tree, reset } = await mount();
    // Two views of 956 px at 4 px a frame: some clips of each track, not a thousand.
    expect(document.querySelectorAll('[data-clip]').length).toBeLessThanOrEqual(50);
    expect(document.querySelectorAll('[data-waveform][data-tile]').length).toBeGreaterThan(0);
    const first = document.querySelector('[data-clip]')!;
    reset();
    for (let frame = 1; frame <= 30; frame++) view.rerender(tree(frame, true));
    expect(drawn).toEqual({ rows: 0, clips: 0, ticks: 0, toolbar: 0, columns: 0 });
    expect(document.querySelector('[data-clip]')).toBe(first);
    // The playhead moved all the same.
    expect(document.querySelector<HTMLElement>('.timeline-ruler')!.getAttribute('aria-valuenow')).toBe('30');
  });

  it('draws nothing of the tracks again for a scroll within half a view, and keeps the waveform tiles it has drawn when it crosses one', async () => {
    const { scrollTo, root, reset } = await mount();
    reset();
    for (const left of [60, 150, 333, 470]) scrollTo(left);
    expect(drawn).toEqual({ rows: 0, clips: 0, ticks: 0, toolbar: 0, columns: 0 });
    // Into the next half view: the window moves on, and the ruler with it, but not the bar.
    const tiles = new Map([...document.querySelectorAll<SVGElement>('[data-waveform][data-tile]')].map(tile => [`${tile.closest('[data-clip]')!.getAttribute('data-clip')}:${tile.dataset.tile}`, tile]));
    scrollTo(500);
    expect(drawn.rows).toBe(10);
    expect(drawn.ticks).toBe(1);
    expect(drawn.toolbar).toBe(0);
    // A tile that stays in the window is the same element, not worked out again; only new ones are.
    const after = [...document.querySelectorAll<SVGElement>('[data-waveform][data-tile]')];
    const kept = after.filter(tile => tiles.get(`${tile.closest('[data-clip]')!.getAttribute('data-clip')}:${tile.dataset.tile}`) === tile);
    const added = after.length - kept.length;
    expect(kept.length).toBeGreaterThan(0);
    // Each new tile has TILE / 2 columns of two channels, TILE peaks asked for: no more than that is worked out.
    expect(drawn.columns).toBeLessThanOrEqual(added * TILE);
    // A zoom draws the ruler and the tracks again, and not the bar.
    reset();
    fireEvent.keyDown(root, { key: '=', ctrlKey: true });
    expect(drawn.toolbar).toBe(0);
    expect(drawn.ticks).toBe(1);
  });

});
