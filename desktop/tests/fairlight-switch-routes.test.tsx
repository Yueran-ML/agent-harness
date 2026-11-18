// @vitest-environment jsdom
/**
 * Solo, Mute and the fader wherever else they are shown (Index, VCA strip, Inspector) make the
 * mixer strip's edits: Exclusive Solo and Track Groups reach them, a playing lane is not written
 * under, and while automation is written the lane takes the value.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { AudioAutomationAdapter } from '../src/studio/audio/automationAdapter';
import type { ClipTrack, Timeline, Track } from '../src/studio/engine';
import { IndexTracks } from '../src/studio/fairlight/IndexTracks';
import { Inspector } from '../src/studio/fairlight/Inspector';
import { VcaStrip } from '../src/studio/fairlight/VcaStrip';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const sound = (id: string, fields: Partial<Track> = {}) => ({ id, name: `Voice ${id}`, kind: 'audio' as const, enabled: true,
  locked: false, clips: [clip(`clip-${id}`)], transitions: [], ...fields }) as Track;
const song = (fields: Partial<Timeline> = {}, a1: Partial<Track> = {}): Timeline => ({ ...timeline(),
  tracks: [sound('a1', a1), sound('a2', { solo: true }), sound('a3')],
  track_groups: [{ id: 'g1', name: 'Dialogue', members: ['a1', 'a2'], linked: ['fader', 'mute', 'solo'], active: true }],
  vcas: [{ id: 'V1', name: 'VCA 1' }, { id: 'V2', name: 'VCA 2', solo: true }],
  buses: [{ id: 'B1', name: 'Main 1', color: '#fff', format: 'stereo', solo: true }],
  ...fields });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.restoreAllMocks(); });

it('makes the Index Solo an edit that follows groups and Exclusive Solo, and its Mute a mute of the group', () => {
  const edit = vi.fn();
  render(<IndexTracks timeline={song()} hidden={[]} onHidden={() => {}} onSelect={() => {}} onEdit={edit} />);
  fireEvent.click(screen.getByRole('button', { name: 'Solo Voice a1' }), { detail: 0 });
  expect(edit).toHaveBeenLastCalledWith([{ edit: 'set_track_audio', timeline: 't1', track: 'a1', patch: { solo: true }, follow_groups: true }]);
  fireEvent.click(screen.getByRole('button', { name: 'Mute Voice a1' }), { detail: 0 });
  expect(edit).toHaveBeenLastCalledWith(['a1', 'a2'].map(track => ({ edit: 'set_track_audio', timeline: 't1', track,
    patch: { mute: true }, follow_groups: false })));
});

it('takes the other solos off when a VCA is soloed under Exclusive Solo, and only then', async () => {
  const call = vi.fn(async () => ({ ok: true as const, result: { revision: 8, done: [] } }));
  const context = (t: Timeline, revision = 7) => ({ bridge: { call } as never, session: 1, revision, timeline: t });
  const exclusive = song({ audio: { exclusive_solo: true } as Timeline['audio'] });
  const view = render(<VcaStrip context={context(exclusive)} vca={exclusive.vcas![0]} seconds={0} selected={false} onSelect={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Solo VCA 1' })); await settle();
  const ops = () => (call.mock.calls.at(-1) as unknown as [string, { ops: unknown[] }])[1].ops;
  expect(ops()).toEqual([
    { op: 'set_track_audio', timeline: 't1', track: 'a2', patch: { solo: false } },
    { op: 'set_bus', timeline: 't1', id: 'B1', patch: { solo: false } },
    { op: 'set_vca', timeline: 't1', id: 'V2', patch: { solo: false } },
    { op: 'set_vca', timeline: 't1', id: 'V1', patch: { solo: true } },
  ]);
  const shared = song();
  view.rerender(<VcaStrip context={context(shared, 8)} vca={shared.vcas![0]} seconds={0} selected={false} onSelect={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Solo VCA 1' })); await settle();
  expect(ops()).toEqual([{ op: 'set_vca', timeline: 't1', id: 'V1', patch: { solo: true } }]);
});

it('moves the group from the Inspector fader, keeps off a playing lane, and writes the lane while recording', () => {
  const edit = vi.fn(), bridge = { call: vi.fn(async () => ({ ok: true as const, result: {} })), onEvent: () => () => {} };
  const strip = (fader_db: number) => ({ strip: { fader_db } as NonNullable<ClipTrack['strip']> });
  const commit = (value: string) => {
    const box = document.querySelector<HTMLInputElement>('[data-field="track-fader"] input.inspector-number')!;
    fireEvent.change(box, { target: { value } }); fireEvent.keyDown(box, { key: 'Enter' });
  };
  const view = render(<Inspector bridge={bridge} session={1} revision={7} timeline={song({}, strip(-6))} media={[media]} clips={[]}
    track="a1" onEdit={edit} />);
  commit('-3');
  // As the mixer's fader commits it: the group's levels as they were, then the move that the group follows.
  expect(edit.mock.lastCall![0].at(-1)).toEqual({ edit: 'set_track_audio', timeline: 't1', track: 'a1',
    patch: { strip: { fader_db: -3 } }, follow_groups: true });
  // A lane that plays: the fader shows it and is not written under it.
  const lanes = { fader_db: { points: [{ at: { ticks: 0, scale: 48000 }, value: -12 }] } } as ClipTrack['lanes'];
  const automated = song({}, { ...strip(-6), lanes });
  view.rerender(<Inspector bridge={bridge} session={1} revision={8} timeline={automated} media={[media]} clips={[]} track="a1" onEdit={edit} />);
  expect(document.querySelector<HTMLInputElement>('[data-field="track-fader"] input.inspector-number')!.disabled).toBe(true);
  // Writing automation: the value goes to the lane, not to the base fader.
  const instant = vi.fn(() => true);
  const recording = { enabled: true, recording: true, trim: false, preview: false, start: () => true, change: () => {}, end: () => {},
    active: () => false, instant, show: () => {}, live: null } satisfies AudioAutomationAdapter;
  edit.mockClear();
  view.rerender(<Inspector bridge={bridge} session={1} revision={8} timeline={automated} media={[media]} clips={[]} track="a1"
    onEdit={edit} automation={recording} />);
  commit('-9');
  expect(instant).toHaveBeenCalledWith({ of: { track: 'a1' }, path: 'fader_db' }, -9);
  expect(edit).not.toHaveBeenCalled();
});
