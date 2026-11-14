// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Timeline } from '../src/studio/timeline/Timeline';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import type { EngineBridge } from '../src/bridge';
import type { AudioParam, AudioKey } from '../src/studio/engine';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let n = 0; n < 12; n++) await Promise.resolve(); });
function mount(gain: AudioParam = 0, locked = false, fields = {}) {
  const t = timeline([]);
  t.tracks = [{ id: 'a1', name: 'A1', kind: 'audio', enabled: true, locked, clips: [clip('a1c', 10, 60, { gain_db: gain, ...fields })], transitions: [] }];
  let revision = 17;
  const call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const bridge = { call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
  const props = { timeline: t, media: [media], bridge, session: 1, revision: 17, frame: 20, playing: false, onSeek: vi.fn() };
  return { ...render(<Timeline {...props} />), props, call };
}
const updates = (call: ReturnType<typeof vi.fn>) => call.mock.calls.filter(([method]) => method === 'transaction.apply');
const pointer = (x: number, y: number, extra = {}) => ({ button: 0, pointerId: 1, clientX: x, clientY: y, ...extra });

it('shows one volume overlay and applies a real gesture before release, then closes the same undo step', async () => {
  const { call } = mount();
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30));
  fireEvent.pointerMove(line, pointer(40, 20)); await settle();
  expect(updates(call)).toHaveLength(1);
  const first = updates(call)[0][1];
  expect(first.revision).toBe(17);
  expect(first.gesture.id).toBeTruthy();
  expect(first.ops[0]).toMatchObject({ op: 'set_clip', id: 'a1c' });
  expect(first.ops[0].gain_db).toBeGreaterThan(0);
  fireEvent.pointerUp(line, pointer(40, 20)); await settle();
  expect(updates(call).at(-1)![1].gesture).toEqual({ id: first.gesture.id, end: true });
  expect(updates(call).every(([, args]) => args.revision === 17)).toBe(true);
  expect(updates(call)[0][2]).toEqual({ expectedSession: 1 });
});

it('keeps the final dragged value when released while the first preview is in flight', async () => {
  const { call } = mount();
  let resolve!: (value: unknown) => void;
  call.mockImplementationOnce(() => new Promise(done => { resolve = done; }) as never);
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20));
  const preview = updates(call)[0][1].ops;
  fireEvent.pointerUp(line, pointer(40, 20));
  await act(async () => resolve({ ok: true, result: { revision: 18, done: [] } })); await settle();
  expect(updates(call).at(-1)![1].ops).toEqual(preview);
});

it('never restores over another edit or crosses project sessions', async () => {
  const { call, props, rerender } = mount();
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20)); await settle();
  rerender(<Timeline {...props} revision={19} />);
  fireEvent.keyDown(line, { key: 'Escape' }); await settle();
  expect(updates(call)).toHaveLength(1);
  rerender(<Timeline {...props} revision={19} session={2} />);
  fireEvent.pointerMove(line, pointer(40, 10)); fireEvent.pointerUp(line, pointer(40, 10)); await settle();
  expect(updates(call)).toHaveLength(1);
});

it('restores on pointercancel and unmount using the captured session', async () => {
  for (const cancel of ['pointer', 'unmount']) {
    const view = mount(-9);
    const line = screen.getByTestId('volume-line-a1c');
    fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20)); await settle();
    if (cancel === 'pointer') fireEvent.pointerCancel(line, pointer(40, 20)); else view.unmount();
    await settle();
    expect(updates(view.call).at(-1)![1]).toMatchObject({ revision: 17, gesture: { end: true }, ops: [{ gain_db: -9 }] });
    expect(updates(view.call).at(-1)![2]).toEqual({ expectedSession: 1 });
    view.unmount();
  }
});

it('uses source samples for Alt-add on a reversed speed clip, and isolates Delete from clip commands', async () => {
  const { call } = mount(0, false, { speed: { num: 2, den: 1 }, reverse: true });
  fireEvent.click(screen.getByRole('button', { name: 'Snapping' }));
  const hit = screen.getByTestId('volume-line-a1c').querySelector('.audio-gain-hit')!;
  fireEvent.pointerDown(hit, pointer(10.5, 30, { altKey: true })); await settle();
  expect(updates(call)[0][1].ops[0]).toMatchObject({ edit: 'clip_gain_keys', action: 'add', at: { ticks: 220320, scale: 48000 } });
  fireEvent.keyDown(hit, { key: 'Delete' }); await settle();
  expect(updates(call).every(([, args]) => args.ops.every((op: { edit?: string }) => op.edit !== 'ripple_delete'))).toBe(true);
});

const gainKeys = (): { keys: AudioKey[] } => ({ keys: [0, 1, 2].map((at, i) => ({ at: { ticks: at * 48000, scale: 48000 }, value: -12 + i * 6, interp: 'linear' })) });
it('keeps a held gesture alive beyond the engine idle limit for continuation and Escape, and stops afterwards', async () => {
  vi.useFakeTimers();
  for (const cancel of [false, true]) {
    const view = mount(-6);
    let revision = 17, last = Date.now(), id: string | null = null, steps = 0, gain = -6;
    view.call.mockImplementation(async (method, args?: any) => {
      if (method !== 'transaction.apply') return { ok: true, result: {} } as never;
      if (Date.now() - last >= 10000) id = null;
      if (args.revision !== revision && id !== args.gesture.id) return { ok: false, error: { code: -32001, message: 'expired', data: { reason: 'stale_revision' } } } as never;
      if (id !== args.gesture.id) steps++;
      id = args.gesture.id; revision++; last = Date.now(); gain = args.ops[0].gain_db;
      if (args.gesture.end) { id = null; if (gain === -6) steps--; }
      return { ok: true, result: { revision, done: [] } } as never;
    });
    const line = screen.getByTestId('volume-line-a1c');
    fireEvent.pointerDown(line, pointer(40, 40));
    await act(async () => { await vi.advanceTimersByTimeAsync(12000); });
    expect(updates(view.call)).toHaveLength(0);
    fireEvent.pointerMove(line, pointer(40, 30)); await settle();
    const initial = updates(view.call)[0][1];
    await act(async () => { await vi.advanceTimersByTimeAsync(12000); });
    if (cancel) fireEvent.keyDown(line, { key: 'Escape' });
    else { fireEvent.pointerMove(line, pointer(40, 20)); await settle(); fireEvent.pointerUp(line, pointer(40, 20)); }
    await settle();
    expect(screen.queryByRole('alert')).toBeNull();
    expect(steps).toBe(cancel ? 0 : 1);
    expect(gain).toBe(cancel ? -6 : updates(view.call).at(-1)![1].ops[0].gain_db);
    expect(updates(view.call).every(([, args, context]) => args.revision === 17 && args.gesture.id === initial.gesture.id && context.expectedSession === 1)).toBe(true);
    const count = updates(view.call).length;
    await act(async () => { await vi.advanceTimersByTimeAsync(20000); });
    expect(updates(view.call)).toHaveLength(count);
    view.unmount();
  }
});

it('serializes keep-alives behind active work and never replaces cancellation or writes after an external revision', async () => {
  vi.useFakeTimers();
  for (const external of [false, true]) {
    const view = mount(-6);
    const line = screen.getByTestId('volume-line-a1c');
    fireEvent.pointerDown(line, pointer(40, 40)); fireEvent.pointerMove(line, pointer(40, 30)); await settle();
    let resolve!: (value: unknown) => void;
    view.call.mockImplementationOnce(() => new Promise(done => { resolve = done; }) as never);
    await act(async () => { await vi.advanceTimersByTimeAsync(5000); });
    expect(updates(view.call)).toHaveLength(2);
    fireEvent.pointerMove(line, pointer(40, 20));
    if (external) view.rerender(<Timeline {...view.props} revision={20} />);
    fireEvent.keyDown(line, { key: 'Escape' });
    await act(async () => { await vi.advanceTimersByTimeAsync(20000); });
    expect(updates(view.call)).toHaveLength(2);
    await act(async () => resolve({ ok: true, result: { revision: 19, done: [] } })); await settle();
    if (external) expect(updates(view.call)).toHaveLength(2);
    else expect(updates(view.call).at(-1)![1]).toMatchObject({ revision: 17, gesture: { end: true }, ops: [{ gain_db: -6 }] });
    const count = updates(view.call).length;
    await act(async () => { await vi.advanceTimersByTimeAsync(20000); });
    expect(updates(view.call)).toHaveLength(count);
    view.unmount();
  }
});

it('isolates destructive, clipboard and navigation keys inside the interpolation menu', async () => {
  for (const key of ['Delete', 'Backspace', 'x', 'c', 'v', '[', ']']) {
    const view = mount(gainKeys());
    const line = screen.getByTestId('volume-line-a1c');
    const point = line.querySelector('[data-audio-key="0"]')!;
    fireEvent.pointerDown(point, pointer(0, 40)); fireEvent.pointerUp(point, pointer(0, 40));
    fireEvent.keyDown(line, { key: 'c', ctrlKey: true });
    fireEvent.contextMenu(point, { clientX: 10, clientY: 40 });
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Linear' }), { key, ctrlKey: ['x', 'c', 'v'].includes(key) }); await settle();
    expect(updates(view.call)).toHaveLength(0);
    expect(view.props.onSeek).not.toHaveBeenCalled();
    expect(point.getAttribute('data-selected')).toBe('true');
    view.unmount();
  }
});

it.each([false, true])('keeps range endpoints within the model source bounds with a different media sample rate (reverse=%s)', async reverse => {
  const view = mount(-6, false, { length: 61, reverse, source: { kind: 'media', media: 'm1', in: { ticks: 0, scale: 48000 } } });
  const t = { ...view.props.timeline, rate: { num: 24, den: 1 } };
  const props = { ...view.props, timeline: t, media: [{ ...media, facts: { ...media.facts, audio: { ...media.facts.audio!, rate: 44100 } } }] };
  view.rerender(<Timeline {...props} frame={10} />);
  fireEvent.keyDown(screen.getByRole('region', { name: 'Timeline' }), { key: 'i' });
  view.rerender(<Timeline {...props} frame={71} />);
  fireEvent.keyDown(screen.getByRole('region', { name: 'Timeline' }), { key: 'o' });
  const hit = screen.getByTestId('volume-line-a1c').querySelector('.audio-gain-hit')!;
  fireEvent.pointerDown(hit, pointer(40, 40)); fireEvent.pointerMove(hit, pointer(40, 30)); await settle();
  const range = updates(view.call)[0][1].ops[1];
  expect(range.edit).toBe('clip_gain_range');
  expect(BigInt(range.to.ticks) * 48000n <= 122000n * BigInt(range.to.scale)).toBe(true);
  expect(range.to).toEqual({ ticks: 112087, scale: 44100 });
});
it('collapses a selected group on a plain click but preserves that group when dragged', async () => {
  for (const dragging of [false, true]) {
    const view = mount(gainKeys());
    const line = screen.getByTestId('volume-line-a1c');
    const first = line.querySelector('[data-audio-key="0"]')!, second = line.querySelector('[data-audio-key="1"]')!;
    fireEvent.pointerDown(first, pointer(0, 40)); fireEvent.pointerUp(first, pointer(0, 40));
    fireEvent.pointerDown(second, pointer(100, 40, { ctrlKey: true })); fireEvent.pointerUp(second, pointer(100, 40));
    fireEvent.pointerDown(first, pointer(0, 40));
    expect(line.querySelectorAll('[data-selected="true"]')).toHaveLength(2);
    if (dragging) { fireEvent.pointerMove(first, pointer(0, 30)); await settle(); }
    fireEvent.pointerUp(first, pointer(0, dragging ? 30 : 40)); await settle();
    expect(line.querySelectorAll('[data-selected="true"]')).toHaveLength(dragging ? 2 : 1);
    if (dragging) {
      const changed = updates(view.call)[0][1].ops[0].gain_db.keys;
      expect(changed[0].value).toBeGreaterThan(-12); expect(changed[1].value).toBeGreaterThan(-6); expect(changed[2].value).toBe(0);
    } else {
      fireEvent.keyDown(line, { key: 'Delete' }); await settle();
      expect(updates(view.call)[0][1].ops).toEqual([{ edit: 'clip_gain_keys', timeline: 't1', clip: 'a1c', action: 'remove', at: { ticks: 0, scale: 48000 } }]);
    }
    view.unmount(); await settle();
  }
});

it.each([false, true])('navigates every subframe key in both directions, including shared display frames (reverse=%s)', reverse => {
  const positions = [.2, .4, .6, 2];
  const param = { keys: positions.map(at => ({ at: { ticks: Math.round((reverse ? 60 - at : at) / 25 * 48000), scale: 48000 }, value: 0, interp: 'linear' as const })).sort((a, b) => a.at.ticks - b.at.ticks) };
  const { props, rerender } = mount(param, false, { reverse });
  rerender(<Timeline {...props} frame={10} />);
  const line = screen.getByTestId('volume-line-a1c');
  const selectedAt = () => line.querySelector('[data-selected="true"]')?.getAttribute('data-audio-key');
  const order = reverse ? [3, 2, 1, 0] : [0, 1, 2, 3];
  const visited: (string | null | undefined)[] = [];
  for (let i = 0; i < positions.length; i++) {
    fireEvent.keyDown(line, { key: ']' });
    expect(props.onSeek).toHaveBeenLastCalledWith(Math.round(10 + positions[i]), false);
    visited.push(selectedAt());
    rerender(<Timeline {...props} frame={Math.round(10 + positions[i])} />);
  }
  expect(visited).toEqual(order.map(String));
  for (let i = positions.length - 2; i >= 0; i--) {
    fireEvent.keyDown(line, { key: '[' });
    expect(props.onSeek).toHaveBeenLastCalledWith(Math.round(10 + positions[i]), false);
    expect(selectedAt()).toBe(String(order[i]));
    rerender(<Timeline {...props} frame={Math.round(10 + positions[i])} />);
  }
  // A manual seek to another frame resets navigation to that frame.
  rerender(<Timeline {...props} frame={11} />);
  fireEvent.keyDown(line, { key: ']' }); expect(selectedAt()).toBe(String(order[3]));
  // A pointer selection starts a fresh navigation sequence from the displayed playhead.
  const point = line.querySelector(`[data-audio-key="${order[0]}"]`)!;
  fireEvent.pointerDown(point, pointer(0, 40)); fireEvent.pointerUp(point, pointer(0, 40));
  fireEvent.keyDown(line, { key: ']' }); expect(selectedAt()).toBe(String(order[3]));
  // Same clip ID in a new project must not retain the old logical key position.
  rerender(<Timeline {...props} session={2} frame={10} />);
  fireEvent.keyDown(line, { key: ']' }); expect(selectedAt()).toBe(String(order[0]));
  rerender(<Timeline {...props} session={3} frame={10} />);
  fireEvent.keyDown(line, { key: ']' }); expect(selectedAt()).toBe(String(order[0]));
  const edited = { ...props.timeline, tracks: props.timeline.tracks.map(track => track.kind === 'subtitle' ? track : { ...track, clips: track.clips.map(clip => ({ ...clip, gain_db: { keys: param.keys.map(key => ({ ...key, value: -3 })) } })) }) };
  rerender(<Timeline {...props} timeline={edited} session={3} frame={10} revision={18} />);
  fireEvent.keyDown(line, { key: ']' }); expect(selectedAt()).toBe(String(order[0]));
});

it('moves segment endpoints together; key Shift locks the first direction while line Shift fine tunes', async () => {
  const first = mount(gainKeys());
  const hit = screen.getByTestId('volume-line-a1c').querySelector('.audio-gain-hit')!;
  fireEvent.pointerDown(hit, pointer(40, 40)); fireEvent.pointerMove(hit, pointer(40, 30)); await settle();
  const moved = updates(first.call)[0][1].ops[0].gain_db.keys;
  expect(moved[1].value - moved[0].value).toBeCloseTo(6);
  expect(moved[2].value).toBe(0);
  first.unmount(); await settle();
  const second = mount(gainKeys());
  const key = screen.getByTestId('volume-line-a1c').querySelector('[data-audio-key="1"]')!;
  fireEvent.pointerDown(key, pointer(100, 40, { shiftKey: true }));
  fireEvent.pointerMove(key, pointer(110, 41, { shiftKey: true }));
  fireEvent.pointerMove(key, pointer(600, 10, { shiftKey: true })); await settle();
  const params = updates(second.call).at(-1)![1].ops[0].gain_db.keys;
  expect(params[1].value).toBe(-6);
  expect(params[1].at.ticks).toBeLessThan(params[2].at.ticks);
});

it('drags a fade to 1.5 timeline frames even for a sped reverse clip and resets its curve', async () => {
  const { call } = mount(0, false, { speed: { num: 2, den: 1 }, reverse: true });
  const handle = screen.getByTestId('fade-in-a1c');
  fireEvent.pointerDown(handle, pointer(0, 4)); fireEvent.pointerMove(handle, pointer(6, 4)); await settle();
  expect(updates(call)[0][1].ops[0].audio_fade_in.length).toEqual({ ticks: 2880, scale: 48000 });
  fireEvent.pointerUp(handle, pointer(6, 4)); await settle();
  expect(updates(call).at(-1)![1].gesture.end).toBe(true);
});

it('rebuilds range preview from the starting curve, so repeated moves never compound delta', async () => {
  const { call, props, rerender } = mount(-6);
  const root = screen.getByRole('region', { name: 'Timeline' });
  fireEvent.keyDown(root, { key: 'i' });
  rerender(<Timeline {...props} frame={40} />); fireEvent.keyDown(root, { key: 'o' });
  const hit = screen.getByTestId('volume-line-a1c').querySelector('.audio-gain-hit')!;
  fireEvent.pointerDown(hit, pointer(60, 40)); fireEvent.pointerMove(hit, pointer(60, 30)); await settle();
  fireEvent.pointerMove(hit, pointer(60, 20)); await settle();
  expect(updates(call)).toHaveLength(2);
  for (const [, args] of updates(call)) {
    expect(args.ops[0]).toMatchObject({ op: 'set_clip', gain_db: -6 });
    expect(args.ops[1]).toMatchObject({ edit: 'clip_gain_range' });
  }
  expect(updates(call)[1][1].ops[1].delta_db).toBeGreaterThan(updates(call)[0][1].ops[1].delta_db);
});

it('allows a new project with a lower revision after finishing an old gesture', async () => {
  const { call, props, rerender } = mount();
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20)); await settle();
  fireEvent.pointerUp(line, pointer(40, 20)); await settle();
  rerender(<Timeline {...props} session={2} revision={1} />);
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20)); await settle();
  expect(updates(call).at(-1)![1].revision).toBe(1);
  expect(updates(call).at(-1)![2]).toEqual({ expectedSession: 2 });
});

it('drops queued previews and Escape restore when an external notification overtakes the in-flight reply', async () => {
  const { call, props, rerender } = mount();
  let resolve!: (value: unknown) => void;
  call.mockImplementationOnce(() => new Promise(done => { resolve = done; }) as never);
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20));
  fireEvent.pointerMove(line, pointer(40, 10));
  rerender(<Timeline {...props} revision={19} />);
  fireEvent.keyDown(line, { key: 'Escape' });
  await act(async () => resolve({ ok: true, result: { revision: 18, done: [] } })); await settle();
  expect(updates(call)).toHaveLength(1);
});

it('stops on stale refusal without rebasing or restoring', async () => {
  const { call } = mount();
  call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'stale', data: { reason: 'stale_revision', actual: 22 } } } as never);
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 20)); await settle();
  fireEvent.pointerMove(line, pointer(40, 10)); fireEvent.keyDown(line, { key: 'Escape' }); await settle();
  expect(updates(call)).toHaveLength(1);
  expect(screen.getByRole('alert').textContent).toContain('changed');
});

it('lets the engine reject fade conflicts and prevents gestures on known transition edges', async () => {
  const { call, props, rerender } = mount();
  const track = props.timeline.tracks[0];
  if (track.kind === 'subtitle') throw new Error('fixture');
  // Engine snapshots replace edited tracks; do not mutate the previous memoized row in place.
  const timeline = { ...props.timeline, tracks: [{ ...track, transitions: [{ id: 'transition', left: 'other', right: 'a1c', kind: 'crossfade' as const, length: 8, align: 'center' as const }] }, ...props.timeline.tracks.slice(1)] };
  rerender(<Timeline {...props} timeline={timeline} revision={18} />);
  const handle = screen.getByTestId('fade-in-a1c');
  fireEvent.pointerDown(handle, pointer(0, 4)); fireEvent.pointerMove(handle, pointer(6, 4)); fireEvent.pointerUp(handle, pointer(6, 4)); await settle();
  expect(updates(call)).toHaveLength(0);
});

it('copies, cuts, pastes, deletes and changes interpolation only in the keyframe focus', async () => {
  const { call, props, rerender } = mount(gainKeys());
  const line = screen.getByTestId('volume-line-a1c');
  const first = line.querySelector('[data-audio-key="0"]')!, last = line.querySelector('[data-audio-key="2"]')!;
  fireEvent.pointerDown(first, pointer(0, 40)); fireEvent.pointerUp(first, pointer(0, 40));
  fireEvent.pointerDown(last, pointer(200, 40, { shiftKey: true })); fireEvent.pointerUp(last, pointer(200, 40));
  fireEvent.keyDown(line, { key: 'c', ctrlKey: true });
  // The copy starts at this clip's first frame and fits here; moving it 0.4 s later still fits.
  rerender(<Timeline {...props} frame={20} />);
  fireEvent.keyDown(line, { key: 'v', ctrlKey: true }); await settle();
  expect(updates(call)[0][1].ops).toHaveLength(3);
  expect(updates(call)[0][1].ops[0]).toMatchObject({ action: 'add', at: { ticks: 19200, scale: 48000 } });
  // A fresh accepted copy enables the next ordinary edit.
  rerender(<Timeline {...props} revision={18} />);
  fireEvent.contextMenu(first, { clientX: 0, clientY: 40 });
  expect(screen.getAllByRole('menuitem')).toHaveLength(3);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Ease In' })); await settle();
  expect(updates(call).at(-1)![1].ops[0]).toMatchObject({ action: 'set_interp', interp: 'ease_in' });
  rerender(<Timeline {...props} revision={19} />);
  fireEvent.pointerDown(first, pointer(0, 40)); fireEvent.pointerUp(first, pointer(0, 40));
  fireEvent.keyDown(line, { key: 'x', ctrlKey: true }); await settle();
  expect(updates(call).at(-1)![1].ops[0]).toMatchObject({ edit: 'clip_gain_keys', action: 'remove' });
  expect(updates(call).every(([, args]) => args.ops.every((op: { edit?: string }) => !['lift', 'ripple_delete'].includes(op.edit ?? '')))).toBe(true);
});

it('Ctrl toggles key selection; Ctrl+Alt deletes one point; brackets seek in timeline order', async () => {
  const { call, props } = mount(gainKeys(), false, { reverse: true });
  const line = screen.getByTestId('volume-line-a1c');
  const points = line.querySelectorAll('[data-audio-key]');
  fireEvent.pointerDown(points[0], pointer(220, 40)); fireEvent.pointerUp(points[0], pointer(220, 40));
  fireEvent.pointerDown(points[2], pointer(20, 40, { ctrlKey: true })); fireEvent.pointerUp(points[2], pointer(20, 40));
  expect(line.querySelectorAll('[data-selected="true"]')).toHaveLength(2);
  fireEvent.keyDown(line, { key: ']' });
  expect(props.onSeek).toHaveBeenCalledWith(45, false);
  fireEvent.pointerDown(points[1], pointer(100, 40, { ctrlKey: true, altKey: true })); await settle();
  expect(updates(call)[0][1].ops[0]).toMatchObject({ action: 'remove', at: { ticks: 48000, scale: 48000 } });
});

it('clamps fade lengths to the other fade, varies curve/skew and resets to linear', async () => {
  const fields = { audio_fade_in: { length: { ticks: 48000, scale: 48000 }, curve: .2, skew: -.1 }, audio_fade_out: { length: { ticks: 48000, scale: 48000 } } };
  const { call, props, rerender } = mount(0, false, fields);
  const handle = screen.getByTestId('fade-in-a1c');
  fireEvent.pointerDown(handle, pointer(100, 4)); fireEvent.pointerMove(handle, pointer(500, 4)); await settle();
  expect(updates(call)[0][1].ops[0].audio_fade_in.length).toEqual({ ticks: 67200, scale: 48000 });
  fireEvent.pointerUp(handle, pointer(500, 4)); await settle();
  rerender(<Timeline {...props} revision={19} />);
  const shape = screen.getByTestId('fade-curve-in-a1c');
  fireEvent.pointerDown(shape, pointer(50, 40)); fireEvent.pointerMove(shape, pointer(80, 10)); await settle();
  const changed = updates(call).at(-1)![1].ops[0].audio_fade_in;
  expect(changed.curve).toBe(1); expect(changed.skew).toBeCloseTo(.15);
  fireEvent.pointerUp(shape, pointer(80, 10)); await settle();
  rerender(<Timeline {...props} revision={21} />);
  fireEvent.doubleClick(shape); await settle();
  expect(updates(call).at(-1)![1].ops[0].audio_fade_in).toMatchObject({ curve: 0, skew: 0 });
});

it('uses Shift as one tenth of volume travel, keeping plain clip Shift editing separate', async () => {
  const normal = mount(); const a = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(a, pointer(40, 40)); fireEvent.pointerMove(a, pointer(40, 39)); await settle();
  // Cross the threshold first, then return to the desired position.
  fireEvent.pointerMove(a, pointer(40, 36)); fireEvent.pointerMove(a, pointer(40, 39)); await settle();
  const ordinary = updates(normal.call).at(-1)![1].ops[0].gain_db;
  normal.unmount(); await settle();
  const fine = mount(); const b = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(b, pointer(40, 40)); fireEvent.pointerMove(b, pointer(40, 30, { shiftKey: true })); await settle();
  expect(updates(fine.call).at(-1)![1].ops[0].gain_db).toBeCloseTo(ordinary);
});

it('restores the captured value in the gesture on Escape and respects locked tracks', async () => {
  const mounted = mount(-6);
  const line = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(line, pointer(40, 30)); fireEvent.pointerMove(line, pointer(40, 15)); await settle();
  fireEvent.keyDown(line, { key: 'Escape' }); await settle();
  expect(updates(mounted.call).at(-1)![1]).toMatchObject({ revision: 17, gesture: { end: true }, ops: [{ gain_db: -6 }] });
  mounted.unmount();
  const { call } = mount(0, true);
  const locked = screen.getByTestId('volume-line-a1c');
  fireEvent.pointerDown(locked, pointer(40, 30)); fireEvent.pointerMove(locked, pointer(40, 15)); fireEvent.pointerUp(locked, pointer(40, 15)); await settle();
  expect(updates(call)).toHaveLength(0);
});
