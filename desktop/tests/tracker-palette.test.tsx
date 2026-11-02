// @vitest-environment jsdom
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { TrackerPalette } from '../src/studio/color/TrackerPalette';
import { GRAPH_COLUMNS, graph, selected, trackedRange, trackerKey, tracked, withCrosshair, withoutCrosshair } from '../src/studio/color/tracker';
import { fullCorrector } from '../src/studio/color/corrector';
import type { EditedNode } from '../src/studio/color/useNodeEdit';
import type { EngineEvent, PowerWindow, TrackData } from '../src/studio/engine';
import { en } from '../src/messages/en';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
const byTest = (id: string) => document.querySelector<HTMLElement>(`[data-test="${id}"]`);
const said = en.label.studio.colorPage.trackerPalette;
const hints = en.hint.studio.colorPage.tracker;

function data(): TrackData {
  return {
    anchor: [127.5, 95.5], frame_size: [256, 192], components: { pan: true, tilt: true, zoom: true, rotate: true, perspective: false },
    curves: { pan: { 10: 0, 11: 2, 12: 4 }, tilt: { 10: 0, 11: -1, 12: -2 }, zoom: { 10: 1, 11: 1, 12: 1 }, rotate: { 10: 0, 11: 0.01, 12: 0.02 },
      aspect: { 10: 1, 11: 1, 12: 1 }, shear: { 10: 0, 11: 0, 12: 0 }, yaw: { 10: 0, 11: 0, 12: 0 }, pitch: { 10: 0, 11: 0, 12: 0 } },
    confidence: { 10: 1, 11: 0.9, 12: 0.9 }, points: [], keys: {},
  };
}

function windowOf(id: string, tracking?: PowerWindow['tracking']): PowerWindow {
  return { id, label: '', enabled: true, invert: false, combine: 'add', opacity: 100,
    transform: { size: 50, aspect: 50, pan: 50, tilt: 50, rotate: 0 }, shape: { kind: 'circular', softness: 1 }, ...(tracking ? { tracking } : {}) };
}

function mount(windows: PowerWindow[], chosen: string | null = 'w1') {
  const listeners: ((event: EngineEvent) => void)[] = [];
  const call = vi.fn(async (method: string, _params?: Record<string, unknown>) => {
    if (method === 'track.start') return { ok: true, result: { job: 'track-1', first: 0, last: 99, frame_size: [256, 192], direction: 'forward' } };
    if (method === 'track.edit') return { ok: true, result: { revision: 11 } };
    return { ok: true, result: {} };
  });
  const bridge = { call, onEvent: (listener: (event: EngineEvent) => void) => { listeners.push(listener); return () => {}; } };
  const onSeek = vi.fn();
  const edited: EditedNode = { bridge: { call } as never, session: 1, revision: 10, target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', clock: 'source', timelineFrame: 40, playing: false,
    node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ windows }) }, frame: 11, onFailure: vi.fn() };
  render(<TrackerPalette edited={edited} bridge={bridge as never} window={chosen} onSeek={onSeek} />);
  const tell = (method: string, params: unknown) => act(() => { for (const each of listeners) each({ kind: 'notification', session: 1, method, params }); });
  const sent = (method: string) => call.mock.calls.filter(([m]) => m === method).map(([, p]) => p as Record<string, any>);
  return { call, onSeek, tell, sent };
}

describe('the Tracker palette\'s model (pp. 3231–3238)', () => {
  it('maps the keys of the Color › Tracker menu', () => {
    const key = (k: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean }> = {}) =>
      trackerKey({ key: k, ctrl: false, alt: false, shift: false, meta: false, repeat: false, composing: false, ...mods });
    expect(key('t', { ctrl: true })).toBe('forward');
    expect(key('t', { alt: true })).toBe('reverse');
    expect(key('t', { ctrl: true, alt: true })).toBe('stop');
    expect(key('ArrowRight', { alt: true })).toBe('one_forward');
    expect(key('ArrowLeft', { alt: true })).toBe('one_reverse');
    expect(key('t')).toBeNull();
    expect(key('ArrowRight')).toBeNull();
    expect(key('t', { ctrl: true, shift: true })).toBeNull();
  });

  it('draws each curve about no motion, scaled by its own largest change, and leaves still ones out', () => {
    const drawn = graph(data(), 20);
    expect(drawn.from).toBe(10);
    expect(drawn.to).toBe(20);
    expect(drawn.lines.map(line => line.component)).toEqual(['pan', 'tilt', 'rotate']);
    expect(drawn.lines[0].points).toEqual([[10, -0], [11, -0.5], [12, -1]]);
    expect(drawn.lines[1].points[2]).toEqual([12, 1]);
    expect(tracked({ data: data() })).toBe(true);
    expect(tracked(undefined)).toBe(false);
  });

  it('selects the curves that a box crosses, over whole frames', () => {
    const lines = graph(data(), 12).lines;
    expect(selected(lines, { from: 10.6, to: 12.4, top: -1.1, bottom: -0.4 })).toEqual({ from: 11, to: 12, components: ['pan', 'rotate'] });
    expect(selected(lines, { from: 11.2, to: 11.8, top: -2, bottom: 2 })).toBeNull();
  });

  it('measures and draws a track of 200 000 frames without spreading them into a call', () => {
    const pan: Record<string, number> = {};
    const tilt: Record<string, number> = {};
    for (let f = 0; f < 200_000; f++) { pan[f] = Math.sin(f / 500) * 40; tilt[f] = f % 7; }
    const long: TrackData = { ...data(), curves: { pan, tilt }, confidence: {} };
    expect(trackedRange(long)).toEqual([0, 199_999]);
    const drawn = graph(long, 5);
    expect([drawn.from, drawn.to]).toEqual([0, 199_999]);
    expect(drawn.lines.map(line => line.component)).toEqual(['pan', 'tilt']);
    // Drawn by its least and most in each column of the graph: what a line of pixels shows.
    for (const line of drawn.lines) expect(line.points.length).toBeLessThanOrEqual(4 * GRAPH_COLUMNS + 2);
    const ys = drawn.lines[0].points.map(([, y]) => y);
    expect(Math.min(...ys)).toBeCloseTo(-1, 3);
    expect(Math.max(...ys)).toBeCloseTo(1, 3);
    expect(drawn.lines[0].points[0][0]).toBe(0);
    expect(drawn.lines[0].points.at(-1)![0]).toBe(199_999);
  });

  it('keeps every sample of a short track', () => {
    expect(graph(data(), 12).lines[0].points).toEqual([[10, -0], [11, -0.5], [12, -1]]);
  });

  it('adds a crosshair at the centre of the frame and deletes the selected one', () => {
    expect(withCrosshair([])).toEqual([[0.5, 0.5]]);
    expect(withoutCrosshair([[0.1, 0.1], [0.5, 0.5], [0.9, 0.9]], 1)).toEqual([[0.1, 0.1], [0.9, 0.9]]);
  });
});

describe('the Tracker palette', () => {
  it('asks for a window when none is selected', () => {
    mount([windowOf('w1')], null);
    expect(byTest('tracker-no-window')?.textContent).toBe(hints.noWindow);
  });

  it('draws the tracker graph with the playhead', () => {
    mount([windowOf('w1', { mode: 'clip', data: data() })]);
    const curves = [...document.querySelectorAll('[data-test="tracker-graph"] polyline')].map(line => line.getAttribute('data-curve'));
    expect(curves).toEqual(['pan', 'tilt', 'rotate']);
    expect(byTest('tracker-playhead')).not.toBeNull();
  });

  it('tracks forward from the playhead with the boxes set before, shows each frame reached and how it ended', async () => {
    const { sent, tell, onSeek } = mount([windowOf('w1')]);
    fireEvent.click(byTest('tracker-box-perspective')!);
    fireEvent.click(byTest('tracker-forward')!);
    await settle();
    expect(sent('track.start')).toEqual([{ grade: { level: 'clip', timeline: 't1', clip: 'c1' }, node: 'n1', window: 'w1', revision: 10, frame: 11,
      direction: 'forward', tracker: 'cloud', components: { pan: true, tilt: true, zoom: true, rotate: true, perspective: true } }]);
    expect((byTest('tracker-forward') as HTMLButtonElement).disabled).toBe(true);
    expect((byTest('tracker-stop') as HTMLButtonElement).disabled).toBe(false);
    await tell('track.progress', { job: 'track-1', frame: 20, timeline_frame: 49, done: 9, total: 88, lost: 0, revision: 12 });
    expect(onSeek).toHaveBeenCalledWith(49);
    expect(byTest('tracker-status')?.textContent).toBe(hints.tracking(20, 9, 88));
    // Another job's news is not this one's.
    await tell('track.state', { job: 'track-9', phase: 'done', reason: null, frames: 1, lost: 0, frame: 1, revision: 1, tracked: null });
    expect((byTest('tracker-stop') as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(byTest('tracker-stop')!);
    await settle();
    expect(sent('track.stop')).toEqual([{ job: 'track-1' }]);
    await tell('track.state', { job: 'track-1', phase: 'stopped', reason: 'stop', frames: 9, lost: 0, frame: 20, revision: 12, tracked: [11, 20] });
    expect(byTest('tracker-status')?.textContent).toBe(hints.stopped);
    expect((byTest('tracker-forward') as HTMLButtonElement).disabled).toBe(false);
  });

  it('takes the keys of the Color › Tracker menu while it is open', async () => {
    const { sent, tell } = mount([windowOf('w1')]);
    fireEvent.keyDown(document.body, { key: 't', code: 'KeyT', altKey: true });
    await settle();
    expect(sent('track.start').map(p => p.direction)).toEqual(['reverse']);
    fireEvent.keyDown(document.body, { key: 't', code: 'KeyT', ctrlKey: true, altKey: true });
    await settle();
    expect(sent('track.stop')).toEqual([{ job: 'track-1' }]);
    await tell('track.state', { job: 'track-1', phase: 'done', reason: null, frames: 11, lost: 0, frame: 0, revision: 12, tracked: [0, 11] });
    expect(byTest('tracker-status')?.textContent).toBe(hints.done(11));
    fireEvent.keyDown(document.body, { key: 'ArrowRight', altKey: true });
    await settle();
    expect(sent('track.start').map(p => p.direction)).toEqual(['reverse', 'one_forward']);
  });

  it('says why a track was refused', async () => {
    const { call } = mount([windowOf('w1')]);
    call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32001, message: 'no', data: { reason: 'untrackable' } } }) as never);
    fireEvent.click(byTest('tracker-forward')!);
    await settle();
    expect(byTest('tracker-status')?.textContent).toBe(hints.failed.untrackable);
  });

  it('sets Clip or Frame, clears what a box selects, keys Frame mode and resets, each one edit', async () => {
    const { sent } = mount([windowOf('w1', { mode: 'frame', data: data() })]);
    fireEvent.click(byTest('tracker-mode-clip')!);
    await settle();
    expect(sent('track.edit')[0]).toEqual({ grade: { level: 'clip', timeline: 't1', clip: 'c1' }, node: 'n1', window: 'w1', revision: 10, edit: { kind: 'mode', mode: 'clip' } });
    // A box from frame 10.5 to the end over the top of the graph: Pan's rise and Rotate's.
    const svg = byTest('tracker-graph')!;
    svg.getBoundingClientRect = () => ({ left: 0, top: 0, width: 100, height: 120, right: 100, bottom: 120, x: 0, y: 0, toJSON() {} }) as DOMRect;
    fireEvent.pointerDown(svg, { pointerId: 1, clientX: 50, clientY: 0 });
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: 100, clientY: 45 });
    fireEvent.pointerUp(svg, { pointerId: 1, clientX: 100, clientY: 45 });
    expect(byTest('tracker-selection')).not.toBeNull();
    const options = byTest('tracker-options') as HTMLSelectElement;
    fireEvent.change(options, { target: { value: 'clear' } });
    await settle();
    expect(sent('track.edit')[1].edit).toEqual({ kind: 'clear', from: 11, to: 12, components: ['pan', 'rotate'] });
    fireEvent.change(options, { target: { value: 'add_key' } });
    fireEvent.change(options, { target: { value: 'reset' } });
    await settle();
    expect(sent('track.edit').slice(2).map(p => p.edit)).toEqual([{ kind: 'add_key', frame: 11 }, { kind: 'reset' }]);
  });

  it('adds and deletes the point tracker\'s crosshairs, and places one by number', async () => {
    const { sent } = mount([windowOf('w1', { crosshairs: [[0.25, 0.25]], data: data() })]);
    fireEvent.change(byTest('tracker-kind')!, { target: { value: 'point' } });
    fireEvent.click(byTest('tracker-add-point')!);
    await settle();
    expect(sent('track.edit')[0].edit).toEqual({ kind: 'crosshairs', points: [[0.25, 0.25], [0.5, 0.5]] });
    fireEvent.change(byTest('tracker-point-x')!, { target: { value: '30' } });
    fireEvent.click(byTest('tracker-delete-point')!);
    await settle();
    expect(sent('track.edit').slice(1).map(p => p.edit)).toEqual([
      { kind: 'crosshairs', points: [[0.3, 0.25]] },
      { kind: 'crosshairs', points: [] },
    ]);
  });

  it('copies one window\'s track and pastes it to another of the node', async () => {
    const windows = [windowOf('w1', { data: data() }), windowOf('w2')];
    const first = mount(windows, 'w1');
    fireEvent.change(byTest('tracker-options')!, { target: { value: 'copy' } });
    expect(byTest('tracker-status')?.textContent).toBe(hints.copied);
    cleanup();
    const second = mount(windows, 'w2');
    fireEvent.change(byTest('tracker-options')!, { target: { value: 'paste' } });
    await settle();
    expect(second.sent('track.edit').map(p => [p.window, p.edit])).toEqual([['w2', { kind: 'paste', from: 'w1' }]]);
    expect(first.sent('track.edit')).toEqual([]);
  });
});
