// @vitest-environment jsdom
/**
 * The Fairlight timeline with its automation view open: as the playhead moves, no track row and
 * no automation lane is drawn again; the lanes still take the frame under the playhead when they
 * are used (here, Clear Track Automation with Hold).
 */
import { memo, type FunctionComponent } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const drawn = vi.hoisted(() => ({ rows: 0, lanes: 0 }));
async function counted(original: () => Promise<Record<string, unknown>>, exported: string, name: keyof typeof drawn) {
  const real = await original();
  const component = real[exported] as FunctionComponent<object> & { type?: FunctionComponent<object> };
  const inner = component.type ?? component;
  const drawing = (props: object) => { drawn[name]++; return inner(props); };
  return { ...real, [exported]: component.type ? memo(drawing) : drawing };
}
vi.mock('../src/studio/timeline/TrackRow', original => counted(original, 'TrackRow', 'rows'));
vi.mock('../src/studio/fairlight/AutomationLanes', original => counted(original, 'AutomationLane', 'lanes'));

const song: Timeline = {
  ...timeline(),
  tracks: Array.from({ length: 4 }, (_, i) => ({
    id: `a${i}`, kind: 'audio' as const, name: `Audio ${i}`, enabled: true, locked: false,
    clips: [clip(`c${i}`, 0, 3000)], transitions: [],
  })),
  buses: [{ id: 'B1', name: 'Main 1', format: 'stereo' } as NonNullable<Timeline['buses']>[number]],
};
const pool = [media];
const view = {
  ...defaultFairlightView(), automation: true,
  lanePaths: { 'track:a0': 'fader_db', 'track:a1': 'fader_db', 'track:a2': 'fader_db', 'track:a3': 'fader_db', 'bus:B1': 'fader_db' },
};
const parameters = [{ path: 'fader_db', kind: 'continuous', scale: 'linear', value: 0, min: -100, max: 10 }];
const engine = () => ({
  call: vi.fn<EngineBridge['call']>(async (method: string) => ({ ok: true as const,
    result: method === 'automation.parameters' ? { revision: 3, parameters } : { revision: 4, done: [] } })),
  onEvent: () => () => {},
});
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(400);
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('draws no row and no automation lane again as the playhead moves, and a lane still uses the playhead', async () => {
  const bridge = engine(), update = vi.fn(), onSeek = vi.fn();
  const tree = (frame: number, playing: boolean) => <FairlightTimeline view={view} update={update} bridge={bridge}
    session={1} timeline={song} media={pool} revision={3} frame={frame} playing={playing} onSeek={onSeek} />;
  const mounted = render(tree(0, false)); await settle();
  expect(screen.getAllByLabelText(/^Audio \d Fader$/)).toHaveLength(4);
  expect(screen.getByLabelText('Main 1 Fader')).toBeTruthy();
  drawn.rows = 0; drawn.lanes = 0;
  for (let frame = 1; frame <= 30; frame++) mounted.rerender(tree(frame, true));
  expect(drawn).toEqual({ rows: 0, lanes: 0 });
  // Clear Track Automation, Hold: at the frame where the playhead is now, not where it was drawn.
  fireEvent.contextMenu(document.querySelector('[data-track="a0"] .automation-lane-header')!);
  fireEvent.click(screen.getByRole('button', { name: 'Hold' })); await settle();
  const write = bridge.call.mock.calls.find(([method]) => method === 'transaction.apply')!;
  expect((write[1] as { ops: { at: { ticks: number } }[] }).ops[0]).toMatchObject({
    edit: 'clear_lanes', then: 'hold', at: { ticks: 30 / 25 * 48000, scale: 48000 },
  });
});

it('draws no row again as the playhead moves while the headers show meters and a choice of tracks', async () => {
  const bridge = engine(), update = vi.fn(), onSeek = vi.fn();
  const listeners = new Set<() => void>();
  const meters = { subscribe: (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; }, current: () => null };
  const tree = (frame: number) => <FairlightTimeline view={{ ...view, automation: false }} update={update} bridge={bridge}
    session={1} timeline={song} media={pool} revision={3} frame={frame} playing onSeek={onSeek} meters={meters}
    selectedTracks={['a1', 'a2']} onTracksSelect={() => {}} />;
  const mounted = render(tree(0)); await settle();
  expect(screen.getAllByRole('meter', { name: /^Track meter Audio/ }).length).toBeGreaterThan(0);
  drawn.rows = 0;
  for (let frame = 1; frame <= 30; frame++) { mounted.rerender(tree(frame)); listeners.forEach(listener => listener()); }
  expect(drawn.rows).toBe(0);
});
