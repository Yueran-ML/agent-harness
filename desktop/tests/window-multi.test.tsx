// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { circularWindow, topologyText, useWindows, windowShape, WindowPalette } from '../src/studio/color/WindowPalette';
import { useViewerInput } from '../src/studio/color/useViewerInput';
import { Palettes } from '../src/studio/color/Palettes';
import type { Comparison } from '../src/studio/color/useComparison';
import { fullCorrector } from '../src/studio/color/corrector';
import { en } from '../src/messages/en';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import type { EngineEvent, PowerWindow, ViewPointer, WindowState } from '../src/studio/engine';
import { likeABrowser } from './mounted';
import { settle } from './colorEngine';

// T23 step 5: a Curve's Shift-selected points (RM20 p. 3195). The engine owns the selection;
// the desktop only learns how many points are selected, sends Esc and Backspace, and explains.

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

const state = (extra: Partial<WindowState> = {}): WindowState => ({ binding: 10, capture: 1, sequence: 12, phase: 'arming', reason: null,
  selected: 'w1', event: 20, revision: 1, keys: { clock: 'source', frame: 10 }, selected_points: [], ...extra });
const point = (kind: ViewPointer['kind'], event: number): ViewPointer => ({ monitor: 'timeline', instance: 1, event,
  presentation: { serial: 1, tag: 1, map: { surface: [256, 144], picture: [128, 72], dest: [0, 0, 256, 144] } }, snapshot: { revision: 1, timeline: 't1', frame: 10, canvas: [128, 72], ready: true,
    editable: true, comparison: 'off', window: { binding: 10, selected: 'w1' } },
  kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x: 10, y: 20,
  inside: true, window: { x: 10, y: 20 }, shift: true, ctrl: false, alt: false });

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
  const onDeleted = vi.fn();
  const props: Parameters<typeof useViewerInput>[0] = {
    bridge: { call, onEvent: listener => { listeners.add(listener); return () => { listeners.delete(listener); }; } },
    session: 1, native: true, loaded: true, timeline: 't1', tool: 'window', intention: 0,
    selection: { monitor: 'timeline', revision: 1, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' }, clip: 'c1', node: 'n1',
      selected: 'w1', visible: true, outline: true, tool: 'window' },
    comparison: { splitting: false, pointerCursor: 'arrow', handlePointer: vi.fn(), resetPointer: vi.fn() } as unknown as Comparison,
    viewer: { current: element }, onMenu: vi.fn(), onWindowState: vi.fn(), onIdleEscape, onDeleted };
  const hook = renderHook(at => useViewerInput(at), { initialProps: props });
  const notify = (method: string, params: unknown) => act(() => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params })));
  const answer = async (kind: string, result: unknown, failure?: { code: number; message: string; data: unknown }) => {
    const index = held.findIndex(each => each.params.kind === kind);
    expect(index).toBeGreaterThanOrEqual(0);
    await act(async () => held.splice(index, 1)[0].resolve(failure ? { ok: false, error: failure } : { ok: true, result }));
  };
  const inputs = () => call.mock.calls.filter(([method]) => method === 'window.input').map(([, params]) => params);
  /** A Shift band from a native down to its up, whose end selects `points`. */
  const band = async (points: number[], capture = 1) => {
    h0.notify('view.pointer', point('down', 20 + capture * 2)); await settle();
    await h0.answer('down', state({ capture, event: 20 + capture * 2, sequence: 12 + capture * 3 })); await settle();
    h0.notify('view.pointer', point('up', 21 + capture * 2)); await settle();
    await h0.answer('up', state({ capture, phase: 'ended', event: 21 + capture * 2, sequence: 13 + capture * 3, selected_points: points })); await settle();
  };
  const h0 = { element, outside, onIdleEscape, onDeleted, notify, answer, inputs, hook, band };
  return h0;
}

it('deselects the selected Curve points with the first idle Esc and only then leaves the draft', async () => {
  const h = mountInput(); await settle();
  await h.band([0, 1]);
  expect(h.hook.result.current.points).toBe(2);
  const first = fireEvent.keyDown(h.element, { key: 'Escape' });
  expect(first).toBe(false);
  expect(h.inputs().at(-1)).toEqual({ binding: 10, kind: 'clear_points' });
  expect(h.onIdleEscape).not.toHaveBeenCalled();
  await h.answer('clear_points', { binding: 10, phase: 'cleared', cleared: 2 }); await settle();
  expect(h.hook.result.current.points).toBe(0);
  expect(h.onIdleEscape).not.toHaveBeenCalled();
  // With none selected, Esc leaves the draft as before, without asking the engine.
  fireEvent.keyDown(h.element, { key: 'Escape' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(1);
  expect(h.inputs().filter(each => each.kind === 'clear_points')).toHaveLength(1);
  // If the engine had already dropped them (an undo changed the anchors), that Esc still leaves the draft.
  await h.band([2], 2);
  fireEvent.keyDown(h.element, { key: 'Escape' });
  await h.answer('clear_points', { binding: 10, phase: 'cleared', cleared: 0 }); await settle();
  expect(h.onIdleEscape).toHaveBeenCalledTimes(2);
  // Esc typed outside the viewer does nothing to them.
  await h.band([1, 2], 3);
  fireEvent.keyDown(h.outside, { key: 'Escape' });
  expect(h.inputs().filter(each => each.kind === 'clear_points')).toHaveLength(2);
});

it('deletes the selected points with Backspace in the viewer and explains a refused delete', async () => {
  const h = mountInput(); await settle();
  // Nothing selected: Backspace is left alone.
  expect(fireEvent.keyDown(h.element, { key: 'Backspace' })).toBe(true);
  await h.band([0, 1]);
  fireEvent.keyDown(h.outside, { key: 'Backspace' });
  fireEvent.keyDown(h.element, { key: 'Backspace', ctrlKey: true });
  expect(h.inputs().filter(each => each.kind === 'delete_points')).toHaveLength(0);
  expect(fireEvent.keyDown(h.element, { key: 'Backspace' })).toBe(false);
  expect(h.inputs().at(-1)).toEqual({ binding: 10, kind: 'delete_points' });
  const deleted = { binding: 10, phase: 'deleted', selected: 'w1', points: [0, 1], revision: 2 };
  await h.answer('delete_points', deleted); await settle();
  expect(h.onDeleted).toHaveBeenCalledWith(deleted);
  expect(h.hook.result.current.points).toBe(0);
  // Below a closed Curve's two points nothing is deleted; the palette says why, and the selection stays.
  await h.band([0, 1, 2], 2);
  fireEvent.keyDown(h.element, { key: 'Backspace' });
  await h.answer('delete_points', null, { code: -32001, message: 'refused',
    data: { reason: 'delete', topology: { why: 'minimum', least: 2, shape: 'curve' } } }); await settle();
  expect(h.hook.result.current.notice).toEqual({ action: 'deleteSelected', topology: { why: 'minimum', least: 2, shape: 'curve' } });
  expect(topologyText(en, h.hook.result.current.notice!)).toContain('selected points were not deleted');
  expect(h.hook.result.current.points).toBe(3);
  // A middle-click refusal still names the single point.
  expect(topologyText(en, { action: 'delete', topology: { why: 'minimum', least: 2, shape: 'curve' } })).toContain('the point was not deleted');
});

it('drops the count when the engine says a project change dropped the selected points', async () => {
  const h = mountInput(); await settle();
  await h.band([0, 1]);
  expect(h.hook.result.current.points).toBe(2);
  // Another binding's note, or one not newer than the last state, changes nothing.
  h.notify('window.points', { binding: 9, sequence: 30, revision: 2, selected_points: [] }); await settle();
  h.notify('window.points', { binding: 10, sequence: 16, revision: 2, selected_points: [] }); await settle();
  expect(h.hook.result.current.points).toBe(2);
  // An undo that changed the anchor count: the next Esc leaves the draft without asking the engine.
  h.notify('window.points', { binding: 10, sequence: 17, revision: 2, selected_points: [] }); await settle();
  expect(h.hook.result.current.points).toBe(0);
  fireEvent.keyDown(h.element, { key: 'Escape' });
  expect(h.onIdleEscape).toHaveBeenCalledTimes(1);
  expect(h.inputs().filter(each => each.kind === 'clear_points')).toHaveLength(0);
  // Later states still count, from the newer sequence.
  await h.band([2, 3], 2);
  expect(h.hook.result.current.points).toBe(2);
});

const anchor = (x: number, y: number) => ({ position: { x, y }, in: { x: 0, y: 0 }, out: { x: 0, y: 0 } });
const curve: PowerWindow = { ...circularWindow('w1'), shape: { kind: 'curve', points: [anchor(-0.2, 0), anchor(0.2, 0), anchor(0, 0.2)], closed: true, inside: 0, outside: 0 } };
const polygon: PowerWindow = { ...circularWindow('w2'), shape: windowShape('polygon') };

function mountPalette(windows: PowerWindow[], points: number) {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  const call = vi.fn(() => Promise.resolve({ ok: true as const, result: { revision: 11, done: [], next_id: 1, names: {}, clamped: [] } }));
  const real = { id: 'n1', kind: 'corrector' as const, enabled: true, label: '', params: fullCorrector({ windows }) };
  const target = { level: 'clip' as const, timeline: 't1', clip: 'c1' };
  function Harness() {
    const controls = useWindows({ session: 1, revision: 10, timeline: 't1', target, clip: 'c1', node: real, realNode: real, complete: true,
      editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame: 5, clock: 'source' });
    return <Palettes merged={false} left="primaries" center="window" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}}
      edited={{ bridge: { call }, session: 1, revision: 10, target, node: real, frame: 5, timelineFrame: 5, playing: false, onFailure: vi.fn() }}
      windows={edit => <WindowPalette edit={edit} controls={controls} capturing={false} busy={() => false} status="idle" notice={null} points={points} />} />;
  }
  return render(<Harness />);
}

it('says how many Curve points are selected and what they take, only for a selected Curve', () => {
  mountPalette([curve], 2);
  const said = document.querySelector('[data-test="window-points"]')!.textContent!;
  expect(said).toContain('Selected points: 2');
  expect(said).toContain('Backspace deletes them; Esc deselects them.');
  cleanup();
  mountPalette([curve], 0);
  expect(document.querySelector('[data-test="window-points"]')).toBeNull();
  cleanup();
  mountPalette([polygon], 2);
  expect(document.querySelector('[data-test="window-points"]')).toBeNull();
});

it('says the selected points in each language', () => {
  for (const locale of [ja, zhCN]) {
    const label = locale.label.studio.colorPage.windowControls.selectedPoints;
    expect(label).not.toBe(en.label.studio.colorPage.windowControls.selectedPoints); expect(label).toContain('{count}');
    const hint = locale.hint.studio.colorPage.windowControls;
    expect(hint.selectedPoints).not.toBe(en.hint.studio.colorPage.windowControls.selectedPoints);
    expect(hint.selectedPoints).toContain('Backspace'); expect(hint.selectedPoints).toContain('Esc');
    expect(hint.topology.minimumCurvePoints).toContain('{least}');
    expect(hint.topology.minimumCurvePoints).not.toBe(hint.topology.minimumCurve);
  }
});
