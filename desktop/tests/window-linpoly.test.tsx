// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { circularWindow, topologyText, useWindows, windowShape, WindowPalette } from '../src/studio/color/WindowPalette';
import { useViewerInput } from '../src/studio/color/useViewerInput';
import { Palettes } from '../src/studio/color/Palettes';
import type { Comparison } from '../src/studio/color/useComparison';
import { fullCorrector } from '../src/studio/color/corrector';
import { newCorrectorNode } from '../src/studio/color/nodeEdits';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import type { EngineEvent, GradeNode, PowerWindow, ViewPointer } from '../src/studio/engine';
import { likeABrowser } from './mounted';
import { settle } from './colorEngine';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const words = en.label.studio.colorPage.windowControls;

const linear: PowerWindow = { ...circularWindow('w1'), shape: windowShape('linear') };
const polygon: PowerWindow = { ...circularWindow('w2'), shape: windowShape('polygon') };

it('creates Linear and Polygon windows with the model defaults through the same creation gate', () => {
  const node = (windows: PowerWindow[]) => ({ ...newCorrectorNode('n1'), params: { ...newCorrectorNode('n1').params, windows } } as GradeNode);
  const run = vi.fn();
  const hook = renderHook(() => useWindows({ session: 1, revision: 10, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', node: node([]), realNode: null, complete: true, editing: { run, step: vi.fn(), problem: null, setProblem: vi.fn() },
    tool: 'window', frame: 8, clock: 'source' }));
  act(() => hook.result.current.create(() => false, 'linear'));
  const rectangle = [{ x: -0.25, y: -0.25 }, { x: 0.25, y: -0.25 }, { x: 0.25, y: 0.25 }, { x: -0.25, y: 0.25 }];
  expect((run.mock.calls[0][0][0].params.windows as PowerWindow[])[0].shape).toEqual({ kind: 'linear', points: rectangle, softness: [1, 1, 1, 1] });
  expect(windowShape('polygon')).toEqual({ kind: 'polygon', points: rectangle, inside: 0, outside: 0 });
});

function mountPalette(windows: PowerWindow[], notice: Parameters<typeof WindowPalette>[0]['notice'] = null) {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  const sent: { ops: { params: { windows: PowerWindow[] } }[] }[] = [];
  const call = vi.fn((_method: string, params: unknown) => {
    sent.push(params as typeof sent[number]);
    return Promise.resolve({ ok: true as const, result: { revision: 11, done: [], next_id: 1, names: {}, clamped: [] } });
  });
  const node = { id: 'n1', kind: 'corrector' as const, enabled: true, label: '', params: fullCorrector({ windows }) };
  const target = { level: 'clip' as const, timeline: 't1', clip: 'c1' };
  let controls: ReturnType<typeof useWindows>;
  function Harness() {
    controls = useWindows({ session: 1, revision: 10, timeline: 't1', target, clip: 'c1', node, realNode: node, complete: true,
      editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame: 5, clock: 'source' });
    return <Palettes merged={false} left="primaries" center="window" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}}
      edited={{ bridge: { call }, session: 1, revision: 10, target, node, frame: 5, timelineFrame: 5, playing: false, onFailure: vi.fn() }}
      windows={edit => <WindowPalette edit={edit} controls={controls} capturing={false} busy={() => false} status="idle" notice={notice} />} />;
  }
  render(<Harness />);
  return { sent, controls: () => controls! };
}

const commit = (name: string, value: string) => {
  const input = screen.getByRole('textbox', { name });
  fireEvent.doubleClick(input); fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' });
};

it('edits one Linear side softness as a whole-array edit keeping its other sides and keys', async () => {
  const keyed: PowerWindow = { ...linear, shape: { ...windowShape('linear'), softness: [1, 2, { keys: [{ frame: 0, value: 3, interp: 'linear' }, { frame: 9, value: 9, interp: 'hold' }] }, 4] } as PowerWindow['shape'] };
  const h = mountPalette([keyed, polygon]);
  expect((screen.getByRole('textbox', { name: 'Soft 3' }) as HTMLInputElement).value).toBe('3.00');
  expect(screen.queryByRole('textbox', { name: 'Inside Softness' })).toBeNull();
  commit('Soft 3', '40'); await settle();
  const sent = h.sent[0].ops[0].params.windows;
  expect(sent[0].shape).toEqual({ ...keyed.shape, softness: [1, 2, { keys: [{ frame: 0, value: 40, interp: 'linear' }, { frame: 9, value: 9, interp: 'hold' }] }, 4] });
  expect(sent[1]).toEqual(polygon);
});

it('edits Polygon inside and outside softness numerically, with no Soft 1–4', async () => {
  const h = mountPalette([polygon, linear]);
  expect(screen.queryByRole('textbox', { name: 'Soft 1' })).toBeNull();
  commit('Outside Softness', '25'); await settle();
  expect(h.sent[0].ops[0].params.windows[0].shape).toEqual({ ...polygon.shape, outside: 25 });
  expect(h.sent[0].ops[0].params.windows[1]).toEqual(linear);
});

it('names captured points, an inserted point and a Linear side in the key hints', () => {
  const h = mountPalette([linear]);
  act(() => h.controls().onState({ binding: 1, capture: 1, sequence: 2, phase: 'armed', reason: null, selected: 'w1', event: 2, revision: 10,
    keys: { points: [{ point: 0, x: 4, y: null }, { point: 1, x: null, y: null }], clock: 'source', frame: 5 } }));
  const hints = () => document.querySelector('[data-test="window-keys"]')!.textContent!;
  expect(hints()).toContain('Point 1: X Key frame 4 · Y Static');
  expect(hints()).toContain('Point 2: X Static · Y Static');
  expect(hints()).not.toContain('Soft');
  act(() => h.controls().onState({ binding: 1, capture: 2, sequence: 3, phase: 'armed', reason: null, selected: 'w1', event: 3, revision: 10,
    keys: { softness: 7, edge: 2, clock: 'source', frame: 5 } }));
  expect(hints()).toContain('Soft 3: Key frame 7');
  act(() => h.controls().onState({ binding: 1, capture: 3, sequence: 4, phase: 'armed', reason: null, selected: 'w1', event: 4, revision: 10,
    keys: { points: [{ point: 3, x: null, y: 2 }], inserted: 3, clock: 'source', frame: 5 } }));
  expect(hints()).toContain('New point: X Static · Y Key frame 2');
});

it('explains a refused insertion or deletion with its points, field, interval and a next step', () => {
  const jump = { action: 'insert' as const, topology: { why: 'jump' as const, axis: 'y' as const, points: [0, 1], frames: [0, 20] as [number, number], clock: 'source' as const } };
  const text = topologyText(words, jump);
  expect(text).toContain('Y of points 1, 2'); expect(text).toContain('Source clock frames 0–20');
  expect(text).toContain('Linear'); expect(text).toContain('not changed');
  expect(topologyText(words, { action: 'delete', topology: { why: 'minimum', least: 3 } })).toContain('at least 3 points');
  expect(topologyText(words, { action: 'insert', topology: { why: 'maximum', most: 128 } })).toContain('128');
  mountPalette([polygon], jump);
  expect(screen.getByRole('status').textContent).toBe(text);
});

it('names the new Window fields in each language', () => {
  for (const locale of [ja, zhCN]) {
    const local = locale.label.studio.colorPage.windowControls;
    for (const name of ['soft2', 'inside', 'outside'] as const) expect(local.fields[name]).not.toBe(words.fields[name]);
    expect(local.addLinear).not.toBe(words.addLinear); expect(local.topology.jump).not.toBe(words.topology.jump);
    expect(local.topology.jump).toContain('{points}'); expect(local.topology.jump).toContain('{frames}');
  }
});

const point = (kind: ViewPointer['kind'], event: number, extra: Partial<ViewPointer> = {}): ViewPointer => ({ monitor: 'timeline', instance: 1, event,
  presentation: { serial: 1, tag: 1, map: { surface: [256, 144], picture: [128, 72], dest: [0, 0, 256, 144] } }, snapshot: { revision: 1, timeline: 't1', frame: 10, canvas: [128, 72], ready: true,
    editable: true, comparison: 'off', window: { binding: 10, selected: 'w2' } },
  kind, button: kind === 'down' || kind === 'up' ? 'middle' : null, buttons: kind === 'up' ? [] : ['middle'], x: 10, y: 20,
  inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false, ...extra });

function mountInput() {
  const listeners = new Set<(event: EngineEvent) => void>();
  const held: { method: string; params: any; resolve: (answer: any) => void }[] = [];
  const call = vi.fn((method: string, params: any) => {
    if (method === 'view.viewport') return Promise.resolve({ ok: true, result: { monitor: 'timeline', mode: 'fit', instance: 1, geometry: { surface: [256, 144], picture: [128, 72] } } });
    if (method === 'window.input') return new Promise<any>(resolve => held.push({ method, params, resolve }));
    return Promise.resolve({ ok: true, result: method === 'window.bind' ? { binding: 10, sequence: 1 } : {} });
  });
  const element = document.createElement('section'); document.body.append(element);
  const onDeleted = vi.fn();
  const props: Parameters<typeof useViewerInput>[0] = {
    bridge: { call, onEvent: listener => { listeners.add(listener); return () => { listeners.delete(listener); }; } },
    session: 1, native: true, loaded: true, timeline: 't1', tool: 'window', intention: 0,
    selection: { monitor: 'timeline', revision: 1, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' }, clip: 'c1', node: 'n1',
      selected: 'w2', visible: true, outline: true, tool: 'window' },
    comparison: { splitting: false, pointerCursor: 'arrow', handlePointer: vi.fn(), resetPointer: vi.fn() } as unknown as Comparison,
    viewer: { current: element }, onMenu: vi.fn(), onWindowState: vi.fn(), onDeleted };
  const hook = renderHook(at => useViewerInput(at), { initialProps: props });
  const notify = (pointer: ViewPointer) => act(() => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method: 'view.pointer', params: pointer })));
  const inputs = () => call.mock.calls.filter(([method]) => method === 'window.input').map(([, params]) => params);
  return { hook, held, notify, inputs, onDeleted };
}

it('sends an unmoved middle tap to the engine as a delete of its native down, never a moved one', async () => {
  const h = mountInput(); await settle();
  h.notify(point('down', 30)); h.notify(point('up', 31)); await settle();
  expect(h.inputs()).toEqual([{ binding: 10, kind: 'delete', event: 30 }]);
  await act(async () => h.held.shift()!.resolve({ ok: true, result: { binding: 10, phase: 'deleted', selected: 'w2', point: 2, revision: 2 } }));
  expect(h.onDeleted).toHaveBeenCalledWith({ binding: 10, phase: 'deleted', selected: 'w2', point: 2, revision: 2 });
  h.notify(point('down', 32)); h.notify(point('move', 33, { window: { x: 30, y: 20 }, middle_moved: true }));
  h.notify(point('up', 34, { window: { x: 10, y: 20 }, middle_moved: true })); await settle();
  expect(h.inputs()).toHaveLength(1);
  // Returning to the origin before release still pans: the native latch decides.
  h.notify(point('down', 35)); h.notify(point('up', 36, { middle_moved: true })); await settle();
  expect(h.inputs()).toHaveLength(1);
});

it('shows why a delete or an insertion was refused and clears it on the next capture', async () => {
  const h = mountInput(); await settle();
  h.notify(point('down', 30)); h.notify(point('up', 31)); await settle();
  await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'refused',
    data: { reason: 'delete', topology: { why: 'minimum', least: 3 } } } }));
  expect(h.hook.result.current.notice).toEqual({ action: 'delete', topology: { why: 'minimum', least: 3 } });
  h.notify(point('down', 40, { button: 'left', buttons: ['left'] })); await settle();
  expect(h.hook.result.current.notice).toBeNull();
  const topology = { why: 'timing', axis: 'x', points: [2, 3], frames: [5, 9], clock: 'source' };
  await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'refused', data: { reason: 'insert', topology } } }));
  expect(h.hook.result.current.notice).toEqual({ action: 'insert', topology });
  expect(h.hook.result.current.status).toBe('idle');
});
