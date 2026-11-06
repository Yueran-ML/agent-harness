// @vitest-environment jsdom
/**
 * Meter readings come up to 30 times a second while the transport plays. They reach the meters
 * that show them; the Fairlight page around them (here counted by its toolbar) is not drawn again.
 */
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Meters, Timeline } from '../src/studio/engine';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import type { MonitorControls } from '../src/studio/useMonitor';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const drawn = vi.hoisted(() => ({ shell: 0 }));
vi.mock('../src/studio/fairlight/Toolbar', async original => {
  const real = await original<typeof import('../src/studio/fairlight/Toolbar')>();
  return { ...real, Toolbar: (props: Parameters<typeof real.Toolbar>[0]) => { drawn.shell++; return real.Toolbar(props); } };
});

const song: Timeline = { ...timeline(), tracks: [{
  id: 'a1', name: 'Audio 1', kind: 'audio' as const, enabled: true, locked: false, format: 'stereo' as const,
  clips: [clip('sound1')], transitions: [],
}] as Timeline['tracks'] };
const loudness = { running: true, standard: 'ebu_r128', target: -23, momentary: -25, short_term: -24, momentary_max: -21,
  short_term_max: -22, integrated: -23, range: 3, true_peak: -1.4, sample_peak: -2, seconds: 12, verdict: 'within' as const };
let bar = -30;
const reading = (): Meters => {
  const channel = { bar, peak: bar + 2, bar_position: (bar + 60) / 60, peak_position: (bar + 62) / 60 };
  return { timeline: 't1', playing: true, rate: 30, scale: 'iec', lost: 0, control_room: channel,
    strips: [{ owner: { track: 'a1' }, pre: [], post: [channel, channel] }], loudness: { ...loudness, momentary: bar } };
};
const controls = (): MonitorControls => ({
  monitor: 'timeline', rate: { num: 25, den: 1 }, frame: 0, last: 199, loaded: true, held: false,
  state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: true, speed: 1, last: 199, exact: true,
    sound: 'off', source: null, proxies: true, late: 0 },
  context: () => ({ playing: true, speed: 1, fps: 25, last: 199 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn(),
} as unknown as MonitorControls);
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('draws the meters again at each reading and not the page that owns them', async () => {
  const bridge = {
    call: vi.fn<EngineBridge['call']>(async (method: string) => ({ ok: true as const,
      result: method === 'audio.meters' ? reading() : method === 'automation.parameters' ? { parameters: [] } : {} })),
    onEvent: () => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}),
  };
  const pool = [media], timelines = [song], monitor = controls();
  render(<FairlightPage bridge={bridge} native={false} session={1} revision={7} pool={null} media={pool} timelines={timelines}
    selected={song} onSelect={() => {}} timelineMonitor={monitor} noticeOf={() => null} onRetry={() => {}} onProblem={() => {}} />);
  await settle();
  await act(async () => { await vi.advanceTimersByTimeAsync(200); });
  const strip = () => screen.getByRole('meter', { name: 'Audio 1 1' }).getAttribute('aria-valuenow');
  const room = () => screen.getByRole('meter', { name: 'Control Room 1' }).getAttribute('aria-valuenow');
  const shell = drawn.shell;
  for (const next of [-20, -12, -6]) {
    bar = next;
    await act(async () => { await vi.advanceTimersByTimeAsync(40); });
    expect(strip()).toBe(String(next));
    expect(room()).toBe(String(next));
  }
  expect(drawn.shell).toBe(shell);
});
