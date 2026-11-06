// @vitest-environment jsdom
/**
 * Fairlight's transport and timecode area (Reference Manual 20, pp. 3667–3668, 3675, 3744–3747):
 * Range In and Out with the range's duration, Loop (Ctrl+/), Play In to Out (Alt+/), Fast Forward
 * and Fast Reverse through 8, 24, 60, 150 and 360 times, Go To Timecode (=, keypad digits), and
 * the Razor command.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { TimelineTransportState, Timeline } from '../src/studio/engine';
import { FairlightTransport, nextShuttle } from '../src/studio/fairlight/FairlightTransport';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { commandOf } from '../src/studio/keys';
import { enteredFrame } from '../src/studio/timecode';
import type { MonitorControls } from '../src/studio/useMonitor';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const rate = { num: 25, den: 1 };
const engine = () => ({ call: vi.fn<EngineBridge['call']>().mockResolvedValue({ ok: true, result: {} }), onEvent: () => () => {} });
const controls = (state: Partial<TimelineTransportState> = {}): MonitorControls => ({
  monitor: 'timeline', rate, frame: 100, last: 999, loaded: true, held: false,
  state: { monitor: 'timeline', timeline: 't1', frame: 100, shown: 100, playing: false, speed: 1, last: 999, exact: true,
    sound: 'off', source: null, proxies: true, late: 0, ...state } as TimelineTransportState,
  context: () => ({ playing: false, speed: 1, fps: 25, last: 999 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn(),
});
const calls = (bridge: ReturnType<typeof engine>, method: string) => bridge.call.mock.calls.filter(([name]) => name === method).map(([, params]) => params);
const settle = () => act(async () => { await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

describe('Go To Timecode', () => {
  it('reads digits from the frames up, a dot as two zeros, and + or - as a move from the playhead', () => {
    expect(enteredFrame('1000', 0, rate)).toBe(250);
    expect(enteredFrame('01:00:00:00', 0, rate)).toBe(90000);
    expect(enteredFrame('1.', 0, rate)).toBe(25);
    expect(enteredFrame('+12', 100, rate)).toBe(112);
    expect(enteredFrame('-1.', 100, rate)).toBe(75);
    expect(enteredFrame('-1..', 100, rate)).toBe(0);
    expect(enteredFrame('12a', 100, rate)).toBeNull();
    expect(enteredFrame('123456789', 100, rate)).toBeNull();
  });
});

it('plays fast at once for Shift+L and Shift+J', () => {
  const context = { playing: false, speed: 1, fps: 25, last: 99 };
  expect(commandOf({ key: 'L', shiftKey: true }, context)).toEqual({ method: 'transport.play', params: { speed: 8 } });
  expect(commandOf({ key: 'J', shiftKey: true }, context)).toEqual({ method: 'transport.play', params: { speed: -8 } });
  expect(commandOf({ key: 'l' }, context)).toEqual({ method: 'transport.play', params: { speed: 1 } });
});

it('marks Range In and Out at the playhead and shows them with the duration', () => {
  const onMark = vi.fn();
  const view = (marks: { in: number | null; out: number | null }) => <FairlightTransport bridge={engine()} session={1} monitor={controls()}
    marks={marks} onMark={onMark} onFailure={() => {}} />;
  const mounted = render(view({ in: null, out: null }));
  expect(screen.getByRole('button', { name: 'Playhead timecode' }).textContent).toBe('00:00:04:00');
  expect(screen.getByLabelText('Range Duration').textContent).toBe('--:--:--:--');
  expect((screen.getByRole('button', { name: 'Play In to Out' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Range In' }));
  fireEvent.click(screen.getByRole('button', { name: 'Range Out' }));
  expect(onMark.mock.calls).toEqual([['in'], ['out']]);
  // The Out point is kept as the frame after the range; it is shown as its last frame.
  mounted.rerender(view({ in: 50, out: 101 }));
  expect(screen.getByRole('status', { name: 'Range In' }).textContent).toBe('00:00:02:00');
  expect(screen.getByRole('status', { name: 'Range Out' }).textContent).toBe('00:00:04:00');
  expect(screen.getByLabelText('Range Duration').textContent).toBe('00:00:02:01');
});

it('loops with Ctrl+/ and plays In to Out with Alt+/ or its button', async () => {
  const bridge = engine();
  const view = (loop: boolean) => <FairlightTransport bridge={bridge} session={3} monitor={controls(loop ? { loop: true } : {})}
    marks={{ in: 50, out: 101 }} onMark={() => {}} onFailure={() => {}} />;
  const mounted = render(view(false));
  fireEvent.keyDown(document.body, { key: '/', code: 'Slash', ctrlKey: true }); await settle();
  expect(calls(bridge, 'transport.loop')).toEqual([{ monitor: 'timeline', on: true }]);
  mounted.rerender(view(true));
  expect(screen.getByRole('button', { name: 'Loop' }).getAttribute('aria-pressed')).toBe('true');
  fireEvent.click(screen.getByRole('button', { name: 'Loop' })); await settle();
  expect(calls(bridge, 'transport.loop')[1]).toEqual({ monitor: 'timeline', on: false });
  fireEvent.keyDown(document.body, { key: '/', code: 'Slash', altKey: true }); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Play In to Out' })); await settle();
  expect(calls(bridge, 'transport.play_range')).toEqual([1, 2].map(() => ({ monitor: 'timeline', from: 50, to: 100 })));
  expect(bridge.call.mock.calls[0][2]).toEqual({ expectedSession: 3 });
});

it('goes through the shuttle speeds, past 8 by moving the playhead silently, until another key', () => {
  vi.useFakeTimers();
  expect([8, 24, 60, 150, 360, 360].map((_, i, all) => nextShuttle(i ? all[i - 1] : 1, 1))).toEqual([8, 24, 60, 150, 360, 360]);
  expect(nextShuttle(24, -1)).toBe(-8);
  const playing = controls({ playing: true, speed: 8 });
  const mounted = render(<FairlightTransport bridge={engine()} session={1} monitor={controls()} marks={{ in: null, out: null }} onMark={() => {}} onFailure={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Fast Forward' }));
  mounted.rerender(<FairlightTransport bridge={engine()} session={1} monitor={playing} marks={{ in: null, out: null }} onMark={() => {}} onFailure={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Fast Forward' }));
  expect(playing.run).toHaveBeenCalledWith({ method: 'transport.pause' });
  expect(screen.getByText('24×')).toBeTruthy();
  act(() => { vi.advanceTimersByTime(1000 / 15 * 2 + 1); });
  // 24 times 25 frames a second, 15 moves a second: 40 frames a move.
  expect(vi.mocked(playing.seek).mock.calls).toEqual([[140, false], [180, false]]);
  fireEvent.keyDown(document.body, { key: 'Shift' });
  act(() => { vi.advanceTimersByTime(1000); });
  expect(vi.mocked(playing.seek)).toHaveBeenCalledTimes(2);
  expect(screen.queryByText('24×')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Fast Reverse' }));
  expect(playing.run).toHaveBeenLastCalledWith({ method: 'transport.play', params: { speed: -8 } });
});

it('goes to a timecode typed after = or a keypad digit, and not for one that is not', () => {
  const monitor = controls();
  render(<FairlightTransport bridge={engine()} session={1} monitor={monitor} marks={{ in: null, out: null }} onMark={() => {}} onFailure={() => {}} />);
  fireEvent.keyDown(document.body, { key: '=' });
  const field = screen.getByRole('textbox', { name: 'Go To Timecode' });
  fireEvent.change(field, { target: { value: '+1.' } }); fireEvent.keyDown(field, { key: 'Enter' });
  expect(monitor.seek).toHaveBeenLastCalledWith(125, false);
  fireEvent.keyDown(document.body, { key: '2', code: 'Numpad2' });
  expect((screen.getByRole('textbox', { name: 'Go To Timecode' }) as HTMLInputElement).value).toBe('2');
  fireEvent.keyDown(screen.getByRole('textbox', { name: 'Go To Timecode' }), { key: 'Escape' });
  fireEvent.click(screen.getByRole('button', { name: 'Playhead timecode' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Go To Timecode' }), { target: { value: 'x' } });
  fireEvent.keyDown(screen.getByRole('textbox', { name: 'Go To Timecode' }), { key: 'Enter' });
  expect(monitor.seek).toHaveBeenCalledTimes(1);
});

it('cuts with Razor every unlocked track that a clip crosses at the playhead, in one transaction', async () => {
  const bridge = engine();
  const song: Timeline = { ...timeline(), tracks: [
    { id: 'A1', kind: 'audio', name: 'Audio 1', enabled: true, locked: false, clips: [clip('a', 0, 100)], transitions: [] },
    { id: 'A2', kind: 'audio', name: 'Audio 2', enabled: true, locked: true, clips: [clip('b', 0, 100)], transitions: [] },
    { id: 'A3', kind: 'audio', name: 'Audio 3', enabled: true, locked: false, clips: [clip('c', 60, 100)], transitions: [] },
    { id: 'V1', kind: 'video', name: 'Video 1', enabled: true, locked: false, clips: [clip('d', 0, 100)], transitions: [] },
  ] as Timeline['tracks'] };
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  render(<FairlightTimeline view={defaultFairlightView()} update={() => {}} bridge={bridge} session={1} timeline={song} media={[media]}
    revision={6} frame={50} playing={false} onSeek={() => {}} />);
  await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Razor' })); });
  expect(calls(bridge, 'transaction.apply')).toEqual([{ revision: 6, ops: [{ edit: 'blade', timeline: 't1', at: 50, tracks: ['A1', 'V1'], linked: false }] }]);
});

it('mutes, sets the level of and dims the monitor through audio.monitor, the floor of the slider being −∞', async () => {
  let monitor = { mute: false, level: 0 as number | null, dim: false };
  const bridge = { onEvent: () => () => {}, call: vi.fn<EngineBridge['call']>(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'audio.monitor') monitor = { ...monitor, ...params };
    return { ok: true as const, result: method === 'audio.monitor' ? { ...monitor } : {} };
  }) };
  render(<FairlightTransport bridge={bridge} session={2} monitor={controls()} marks={{ in: null, out: null }} onMark={() => {}} onFailure={() => {}} />);
  await settle();
  expect(calls(bridge as never, 'audio.monitor')).toEqual([{}]);
  fireEvent.click(screen.getByRole('button', { name: 'Monitor mute' })); await settle();
  expect(screen.getByRole('button', { name: 'Monitor mute' }).getAttribute('aria-pressed')).toBe('true');
  const slider = screen.getByRole('slider', { name: 'Monitor level' });
  fireEvent.pointerDown(slider);
  fireEvent.change(slider, { target: { value: '-12' } }); await settle();
  expect(screen.getByRole('status', { name: 'Monitor level' }).textContent).toBe('-12.0 dB');
  fireEvent.change(slider, { target: { value: '-60' } }); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'DIM' })); await settle();
  expect(calls(bridge as never, 'audio.monitor').slice(1)).toEqual([{ mute: true }, { level: -12 }, { level: null }, { dim: true }]);
  expect(slider.getAttribute('data-dim')).toBe('true');
  expect(slider.getAttribute('aria-valuetext')).toBe('−∞ dB');
});
