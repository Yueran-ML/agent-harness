// @vitest-environment jsdom
import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Inspector } from '../src/studio/inspector/Inspector';
import { Inspector as FairlightInspector } from '../src/studio/fairlight/Inspector';
import { applyOp, type ClipTrack, type Project } from '../src/studio/engine';
import type { EngineBridge } from '../src/bridge';
import { defaultEq } from '../src/studio/audio/processorModel';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 25; i++) await Promise.resolve(); });
function setup(fairlight = false, clips = [clip('sound')], locked = false) {
  const source = { ...media, facts: { ...media.facts, audio: { rate: 48000, channels: 2, codec: 'pcm', streams: [{ index: 1, channels: 2, layout: 'stereo' }, { index: 3, channels: 1, layout: 'mono' }] } } };
  let project: Project = { schema: 4, next_id: 10, bins: [], groups: [], media: [source], timelines: [{ ...timeline([]), tracks: [{ id: 'a', name: 'Voice', kind: 'audio', format: 'stereo', channels: 2, enabled: true, locked, clips, transitions: [] }] }] };
  let revision = 10;
  const before: Project[] = [];
  const requests: { method: string; params: any; context: any }[] = [];
  let publish: (value: { project: Project; revision: number }) => void;
  const bridge = { call: async (method: string, params: any, context: any) => {
    requests.push({ method, params, context });
    if (method === 'audio.eq_response') return { ok: true, result: { revision, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) } };
    if (method === 'transaction.apply') {
      before.push(project);
      for (const op of params.ops) project = applyOp(project, op);
      revision++; publish({ project, revision });
      return { ok: true, result: { revision, done: params.ops } };
    }
    throw new Error(method);
  } } as Pick<EngineBridge, 'call'>;
  function Host({ session = 1, selected = clips.map(c => c.id) }) {
    const [state, set] = useState({ project, revision }); publish = set;
    const props = { bridge, session, revision: state.revision, timeline: state.project.timelines[0], media: state.project.media, clips: selected, transition: null, onSpeed: vi.fn(), frame: 0, onSeek: vi.fn() };
    return fairlight ? <FairlightInspector {...props} track={null} onEdit={() => {}} /> : <Inspector {...props} />;
  }
  const view = render(<Host />);
  return { ...view, requests, writes: () => requests.filter(r => r.method === 'transaction.apply'), model: () => (project.timelines[0].tracks[0] as ClipTrack).clips,
    restoredSnapshot: () => act(() => { project = before.pop()!; publish({ project, revision: ++revision }); }),
    session: () => view.rerender(<Host session={2} />), selection: (ids: string[]) => view.rerender(<Host selected={ids} />) };
}
it.each([false, true])('opens the shared four-band clip EQ in the inspector (Fairlight=%s)', async fairlight => {
  const s = setup(fairlight); fireEvent.click(screen.getByRole('button', { name: 'Equalizer' })); await settle();
  const eq = screen.getByRole('region', { name: 'EQ' });
  expect(within(eq).getAllByRole('group', { name: /^Band [1-4]$/ })).toHaveLength(4);
  expect(within(eq).getByRole('combobox', { name: 'Preset' })).toBeTruthy();
  expect(s.requests[0]).toMatchObject({ method: 'audio.eq_response', params: { timeline: 't1', target: { clip: 'sound' } }, context: { expectedSession: 1 } });
  fireEvent.click(within(eq).getByRole('checkbox', { name: 'Enable' })); await settle();
  expect(s.model()[0].eq).toEqual({ ...defaultEq(true), on: true });
  expect(s.writes()[0].params).toMatchObject({ revision: 10, gesture: { end: true }, ops: [{ op: 'set_clip', id: 'sound' }] });
});
it('configures a native multi-stream channel map with one real transaction and keeps clip gain', async () => {
  const s = setup(false, [clip('sound', 0, 100, { gain_db: -7 })]);
  fireEvent.click(screen.getByRole('tab', { name: 'File' }));
  fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: '1:1' } });
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 2 Source' }), { target: { value: '3:0' } });
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 2 Mute' }));
  fireEvent.change(screen.getByRole('spinbutton', { name: 'Channel 1 Level' }), { target: { value: '-3' } });
  expect(s.writes()).toHaveLength(0);
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  expect(s.writes()).toHaveLength(1);
  expect(s.model()[0].channels).toEqual([{ source: { stream: 1, channel: 1 }, enabled: true, mute: false, level_db: -3 }, { source: { stream: 3, channel: 0 }, enabled: true, mute: true, level_db: 0 }]);
  expect(s.model()[0].gain_db).toBe(-7);
});
it('maps an averaged stereo pair explicitly and restores automatic mixing without changing track format', async () => {
  const s = setup(true); fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: 'mix:1' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  expect(s.model()[0].channels?.[0].source).toEqual([{ stream: 1, channel: 0 }, { stream: 1, channel: 1 }]);
  expect(s.model()[0].channels).toHaveLength(2);
  fireEvent.click(screen.getByRole('button', { name: 'Restore automatic mix' })); await settle();
  expect(s.model()[0].channels).toBeNull();
});
it('keeps a cross-stream mixed source distinct from a whole-stream mono preset', async () => {
  const s = setup(false, [clip('sound', 0, 100, { channels: [{ source: [{ stream: 1, channel: 1 }, { stream: 3, channel: 0 }] }, { source: null }] })]);
  fireEvent.click(screen.getByRole('tab', { name: 'File' }));
  const source = screen.getByRole('combobox', { name: 'Channel 1 Source' }) as HTMLSelectElement;
  expect(source.selectedOptions[0].textContent).toBe('Custom mapping');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 2 Mute' }));
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  expect(s.model()[0].channels?.[0].source).toEqual([{ stream: 1, channel: 1 }, { stream: 3, channel: 0 }]);
});
it('applies multi-clip mapping together while keeping each clip gain and key data', async () => {
  const animated = { keys: [{ at: { ticks: 0, scale: 48000 }, value: -9, interp: 'ease_in' as const }] };
  const s = setup(false, [clip('a', 0, 100, { gain_db: animated }), clip('b', 100, 100, { gain_db: 3 })]);
  fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: '1:0' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  expect(s.writes()).toHaveLength(1); expect(s.writes()[0].params.ops.map((op: any) => op.id)).toEqual(['a', 'b']);
  expect(s.model().map(clip => clip.gain_db)).toEqual([animated, 3]);
  expect(s.model().map(clip => clip.channels?.[0].source)).toEqual([{ stream: 1, channel: 0 }, { stream: 1, channel: 0 }]);
});
it('discards unapplied mappings on session and selection changes', async () => {
  const s = setup(false, [clip('a'), clip('b')]);
  fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: '3:0' } });
  s.session(); await settle(); expect(screen.queryByRole('button', { name: 'Apply channel mapping' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' })); s.selection(['b']); await settle();
  expect(screen.queryByRole('button', { name: 'Apply channel mapping' })).toBeNull(); expect(s.writes()).toHaveLength(0);
});
it('disables EQ and channel writes on locked tracks', async () => {
  const s = setup(false, [clip('sound')], true);
  fireEvent.click(screen.getByRole('button', { name: 'Equalizer' })); await settle();
  expect((screen.getByRole('checkbox', { name: 'Enable' }) as HTMLInputElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('tab', { name: 'File' }));
  expect((screen.getByRole('button', { name: 'Custom mapping' }) as HTMLButtonElement).disabled).toBe(true);
  expect(s.writes()).toHaveLength(0);
});
it('uses one clip EQ gesture and restores the previous curve on Escape', async () => {
  const eq = { ...defaultEq(true), on: true };
  const s = setup(false, [clip('sound', 0, 100, { eq })]); fireEvent.click(screen.getByRole('button', { name: 'Equalizer' })); await settle();
  const handle = screen.getByRole('slider', { name: 'EQ band 2' });
  fireEvent.pointerDown(handle, { button: 0, clientX: 50, clientY: 50 });
  fireEvent.pointerMove(handle, { clientX: 80, clientY: 30 }); await settle();
  expect(s.model()[0].eq?.bands[1].gain_db).toBeGreaterThan(0);
  fireEvent.keyDown(handle, { key: 'Escape' }); await settle();
  expect(s.model()[0].eq).toEqual(eq);
  expect(new Set(s.writes().map(r => r.params.gesture.id)).size).toBe(1);
  expect(s.writes().at(-1)?.params.gesture.end).toBe(true);
});
it('leaves an emptied channel level blank and refuses to apply it', async () => {
  const s = setup(); fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
  fireEvent.change(screen.getByRole('spinbutton', { name: 'Channel 1 Level' }), { target: { value: '' } });
  expect((screen.getByRole('button', { name: 'Apply channel mapping' }) as HTMLButtonElement).disabled).toBe(true);
  expect(errors).not.toHaveBeenCalled(); expect(s.writes()).toHaveLength(0);
});
it('shows the restored channel model after an undo snapshot arrives', async () => {
  const s = setup(); fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: '1:1' } });
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  s.restoredSnapshot(); await settle();
  expect(screen.getByText('Automatic mix')).toBeTruthy();
  expect(screen.queryByRole('button', { name: 'Apply channel mapping' })).toBeNull();
});
it('uses Alt on Mute to keep only the chosen channel unmuted', async () => {
  const s = setup(); fireEvent.click(screen.getByRole('tab', { name: 'File' })); fireEvent.click(screen.getByRole('button', { name: 'Custom mapping' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 2 Mute' }), { altKey: true });
  fireEvent.click(screen.getByRole('button', { name: 'Apply channel mapping' })); await settle();
  expect(s.model()[0].channels?.map(channel => channel.mute)).toEqual([true, false]);
});
