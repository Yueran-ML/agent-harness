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
import type { EngineEvent, GradeNode, PowerWindow, ViewPointer, WindowState } from '../src/studio/engine';
import { likeABrowser } from './mounted';
import { settle } from './colorEngine';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const words = en.label.studio.colorPage.windowControls;

const anchor = (x: number, y: number) => ({ position: { x, y }, in: { x: 0, y: 0 }, out: { x: 0, y: 0 } });
const draft: PowerWindow = { ...circularWindow('w1'), shape: { kind: 'curve', points: [anchor(-0.2, 0), anchor(0.2, 0)], closed: false, inside: 0, outside: 0 } };
const closed: PowerWindow = { ...circularWindow('w2'), shape: { kind: 'curve', points: [anchor(-0.2, 0), anchor(0.2, 0), anchor(0, 0.2)], closed: true, inside: 0, outside: 0 } };
const polygon: PowerWindow = { ...circularWindow('w3'), shape: windowShape('polygon') };
const node = (windows: PowerWindow[]) => ({ ...newCorrectorNode('n1'), params: { ...newCorrectorNode('n1').params, windows } } as GradeNode);

function windowsHook(windows: PowerWindow[], run = vi.fn()) {
  const real = node(windows);
  return renderHook(() => useWindows({ session: 1, revision: 10, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', node: real, realNode: real, complete: true, editing: { run, step: vi.fn(), problem: null, setProblem: vi.fn() },
    tool: 'window', frame: 8, clock: 'source' }));
}

it('creates a Curve as an empty open draft through the same creation gate, drawn by clicks in the viewer', () => {
  const run = vi.fn();
  const hook = renderHook(() => useWindows({ session: 1, revision: 10, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', node: node([]), realNode: null, complete: true, editing: { run, step: vi.fn(), problem: null, setProblem: vi.fn() },
    tool: 'window', frame: 8, clock: 'source' }));
  act(() => hook.result.current.create(() => false, 'curve'));
  expect((run.mock.calls[0][0][0].params.windows as PowerWindow[])[0].shape).toEqual({ kind: 'curve', points: [], closed: false, inside: 0, outside: 0 });
  // A second create waits for the first one's acknowledgment.
  act(() => hook.result.current.create(() => false, 'curve'));
  expect(run).toHaveBeenCalledTimes(1);
});

it('leaves an open Curve with Esc keeping its draft and binding, and picks it again to continue', () => {
  const hook = windowsHook([draft, polygon]);
  act(() => hook.result.current.choose('w1'));
  const before = hook.result.current.intention;
  let left = false;
  act(() => { left = hook.result.current.leaveDraft(); });
  expect(left).toBe(true);
  expect(hook.result.current.selected).toBeNull();
  // The binding stays so a click on a window selects it again; nothing was edited.
  expect(hook.result.current.selection).toMatchObject({ selected: null, node: 'n1', tool: 'window' });
  expect(hook.result.current.intention).toBe(before + 1);
  expect(hook.result.current.windows).toEqual([draft, polygon]);
  act(() => hook.result.current.onState({ binding: 1, capture: 1, sequence: 2, phase: 'armed', reason: null, selected: 'w1', event: 2, revision: 10,
    keys: { pan: null, tilt: null, clock: 'source', frame: 8 }, selected_points: [] }));
  expect(hook.result.current.selection?.selected).toBe('w1');
  // Nothing to leave: no window selected now, a closed Curve, or another shape.
  act(() => { left = hook.result.current.leaveDraft(); });
  act(() => { left = hook.result.current.leaveDraft(); });
  expect(left).toBe(false);
  const other = windowsHook([closed, polygon]);
  act(() => other.result.current.choose('w2'));
  act(() => { left = other.result.current.leaveDraft(); });
  expect(left).toBe(false); expect(other.result.current.selection?.selected).toBe('w2');
  act(() => other.result.current.choose('w3'));
  act(() => { left = other.result.current.leaveDraft(); });
  expect(left).toBe(false); expect(other.result.current.selection?.selected).toBe('w3');
});

function mountPalette(windows: PowerWindow[]) {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  const sent: { ops: { params: { windows: PowerWindow[] } }[] }[] = [];
  const call = vi.fn((_method: string, params: unknown) => {
    sent.push(params as typeof sent[number]);
    return Promise.resolve({ ok: true as const, result: { revision: 11, done: [], next_id: 1, names: {}, clamped: [] } });
  });
  const real = { id: 'n1', kind: 'corrector' as const, enabled: true, label: '', params: fullCorrector({ windows }) };
  const target = { level: 'clip' as const, timeline: 't1', clip: 'c1' };
  let controls: ReturnType<typeof useWindows>;
  function Harness() {
    controls = useWindows({ session: 1, revision: 10, timeline: 't1', target, clip: 'c1', node: real, realNode: real, complete: true,
      editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame: 5, clock: 'source' });
    return <Palettes merged={false} left="primaries" center="window" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}}
      edited={{ bridge: { call }, session: 1, revision: 10, target, node: real, frame: 5, timelineFrame: 5, playing: false, onFailure: vi.fn() }}
      windows={edit => <WindowPalette edit={edit} controls={controls} capturing={false} busy={() => false} status="idle" notice={null} />} />;
  }
  render(<Harness />);
  return { sent, controls: () => controls! };
}

const commit = (name: string, value: string) => {
  const input = screen.getByRole('textbox', { name });
  fireEvent.doubleClick(input); fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' });
};

it('lists Curve rows as selectable and edits its inside and outside softness as whole-array edits keeping its points', async () => {
  const h = mountPalette([closed, polygon]);
  expect(screen.getByRole('button', { name: words.addCurve })).toBeTruthy();
  const row = screen.getAllByRole('option')[0] as HTMLButtonElement;
  expect(row.disabled).toBe(false);
  act(() => h.controls().choose('w2'));
  expect(screen.queryByRole('textbox', { name: 'Soft 1' })).toBeNull();
  commit('Inside Softness', '30'); await settle();
  const sent = h.sent[0].ops[0].params.windows;
  expect(sent[0].shape).toEqual({ ...closed.shape, inside: 30 });
  expect(sent[1]).toEqual(polygon);
});

it('names a Curve point, its in and out handles and its inside or outside softness in the key hints', () => {
  const h = mountPalette([closed]);
  const hints = () => document.querySelector('[data-test="window-keys"]')!.textContent!;
  act(() => h.controls().onState({ binding: 1, capture: 1, sequence: 2, phase: 'armed', reason: null, selected: 'w2', event: 2, revision: 10,
    keys: { points: [{ point: 1, out_x: 4, out_y: null, in_x: null, in_y: 6 }], clock: 'source', frame: 5 }, selected_points: [] }));
  expect(hints()).toContain('Point 2 out handle: X Key frame 4 · Y Static');
  expect(hints()).toContain('Point 2 in handle: X Static · Y Key frame 6');
  expect(hints()).not.toContain('Point 2: X');
  act(() => h.controls().onState({ binding: 1, capture: 2, sequence: 3, phase: 'armed', reason: null, selected: 'w2', event: 3, revision: 10,
    keys: { points: [{ point: 3, out_x: null, out_y: null, in_x: null, in_y: null }], inserted: 3, clock: 'source', frame: 5 }, selected_points: [] }));
  expect(hints()).toContain('New point out handle: X Static · Y Static');
  act(() => h.controls().onState({ binding: 1, capture: 3, sequence: 4, phase: 'armed', reason: null, selected: 'w2', event: 4, revision: 10,
    keys: { outside: 2, clock: 'source', frame: 5 }, selected_points: [] }));
  expect(hints()).toContain('Outside Softness: Key frame 2');
});

it('explains a refused Curve deletion with the closed Curve minimum, apart from the Polygon one', () => {
  const curve = topologyText(en, { action: 'delete', topology: { why: 'minimum', least: 2, shape: 'curve' } });
  expect(curve).toContain('closed curve'); expect(curve).toContain('at least 2 points');
  expect(topologyText(en, { action: 'delete', topology: { why: 'minimum', least: 3, shape: 'polygon' } })).toContain('polygon window');
  expect(topologyText(en, { action: 'delete', topology: { why: 'minimum', least: 3 } })).toContain('polygon window');
});

it('names the Curve words in each language', () => {
  for (const locale of [ja, zhCN]) {
    const local = locale.label.studio.colorPage.windowControls;
    for (const name of ['addCurve', 'handleIn', 'handleOut'] as const) expect(local[name]).not.toBe(words[name]);
    const said = locale.hint.studio.colorPage.windowControls.topology.minimumCurve;
    expect(said).not.toBe(en.hint.studio.colorPage.windowControls.topology.minimumCurve); expect(said).toContain('{least}');
  }
});

const state = (extra: Partial<WindowState> = {}): WindowState => ({ binding: 10, capture: 1, sequence: 12, phase: 'arming', reason: null,
  selected: 'w1', event: 20, revision: 1, keys: { points: [{ point: 2, out_x: null, out_y: null, in_x: null, in_y: null }], inserted: 2, clock: 'source', frame: 10 }, selected_points: [], ...extra });
const point = (kind: ViewPointer['kind'], event: number): ViewPointer => ({ monitor: 'timeline', instance: 1, event,
  presentation: { serial: 1, tag: 1, map: { surface: [256, 144], picture: [128, 72], dest: [0, 0, 256, 144] } }, snapshot: { revision: 1, timeline: 't1', frame: 10, canvas: [128, 72], ready: true,
    editable: true, comparison: 'off', window: { binding: 10, selected: 'w1' } },
  kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x: 10, y: 20,
  inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false });

function mountInput() {
  const listeners = new Set<(event: EngineEvent) => void>();
  const held: { method: string; params: any; resolve: (answer: any) => void }[] = [];
  const call = vi.fn((method: string, params: any) => {
    if (method === 'view.viewport') return Promise.resolve({ ok: true, result: { monitor: 'timeline', mode: 'fit', instance: 1, geometry: { surface: [256, 144], picture: [128, 72] } } });
    if (method === 'window.input') return new Promise<any>(resolve => held.push({ method, params, resolve }));
    return Promise.resolve({ ok: true, result: method === 'window.bind' ? { binding: 10, sequence: 1 } : {} });
  });
  const element = document.createElement('section'); element.tabIndex = -1; document.body.append(element);
  const outside = document.createElement('input'); document.body.append(outside);
  const onIdleEscape = vi.fn(() => true);
  const props: Parameters<typeof useViewerInput>[0] = {
    bridge: { call, onEvent: listener => { listeners.add(listener); return () => { listeners.delete(listener); }; } },
    session: 1, native: true, loaded: true, timeline: 't1', tool: 'window', intention: 0,
    selection: { monitor: 'timeline', revision: 1, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' }, clip: 'c1', node: 'n1',
      selected: 'w1', visible: true, outline: true, tool: 'window' },
    comparison: { splitting: false, pointerCursor: 'arrow', handlePointer: vi.fn(), resetPointer: vi.fn() } as unknown as Comparison,
    viewer: { current: element }, onMenu: vi.fn(), onWindowState: vi.fn(), onIdleEscape };
  renderHook(at => useViewerInput(at), { initialProps: props });
  const notify = (method: string, params: unknown) => act(() => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params })));
  const answer = async (kind: string, result: unknown) => {
    const index = held.findIndex(each => each.params.kind === kind);
    expect(index).toBeGreaterThanOrEqual(0);
    await act(async () => held.splice(index, 1)[0].resolve({ ok: true, result }));
  };
  const inputs = () => call.mock.calls.filter(([method]) => method === 'window.input').map(([, params]) => params);
  return { element, outside, onIdleEscape, notify, answer, inputs };
}

it('gives an idle Esc in the viewer to the draft, never one typed elsewhere, and Esc during a drawing drag only cancels that drag', async () => {
  const h = mountInput(); await settle();
  const idle = fireEvent.keyDown(h.element, { key: 'Escape' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(1);
  expect(idle).toBe(false);
  fireEvent.keyDown(h.outside, { key: 'Escape' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(1);
  // A drag that draws a point: Esc cancels that capture and the draft is kept by the engine.
  h.notify('view.pointer', point('down', 20)); await settle();
  h.notify('window.state', state());
  fireEvent.keyDown(h.element, { key: 'Escape' });
  await h.answer('down', state()); await settle();
  expect(h.inputs().at(-1)).toEqual({ binding: 10, capture: 1, kind: 'cancel' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(1);
  await h.answer('cancel', state({ phase: 'ended', sequence: 14 })); await settle();
  // Idle again: Esc goes to the draft once more.
  fireEvent.keyDown(h.element, { key: 'Escape' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(2);
});
