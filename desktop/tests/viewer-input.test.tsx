// @vitest-environment jsdom
import { act, cleanup, fireEvent, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useViewerInput } from '../src/studio/color/useViewerInput';
import type { Comparison } from '../src/studio/color/useComparison';
import type { EngineEvent, ViewPointer, WindowState } from '../src/studio/engine';
import { settle } from './colorEngine';

afterEach(cleanup);
const state = (extra: Partial<WindowState> = {}): WindowState => ({ binding: 10, capture: 1, sequence: 12, phase: 'arming', reason: null,
  selected: 'w1', event: 20, revision: 1, keys: { pan: 0, tilt: 3, clock: 'source', frame: 10 }, selected_points: [], ...extra });
const point = (kind: ViewPointer['kind'], event = 20, extra: Partial<ViewPointer> = {}): ViewPointer => ({ monitor: 'timeline', instance: 1, event,
  presentation: { serial: 1, tag: 1, map: { surface: [256, 144], picture: [128, 72], dest: [0, 0, 256, 144] } }, snapshot: { revision: 1, timeline: 't1', frame: 10, canvas: [1920, 1080], ready: true,
    editable: true, comparison: 'wipe', window: { binding: 10, selected: 'w1' } },
  kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x: 300, y: 200,
  inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false, ...extra });

function mount({ holdBind = false, holdPrepare = false, holdNavigation = false, holdQuery = false } = {}) {
  const listeners = new Set<(event: EngineEvent) => void>();
  const held: { method: string; params: any; resolve: (answer: any) => void }[] = [];
  let nextBinding = 10;
  const call = vi.fn((method: string, params: any, _context?: unknown) => {
    const binding = method === 'window.bind' ? nextBinding++ : null;
    if (method === 'view.viewport') {
      if ((params.operation === 'prepare' && holdPrepare) || (params.operation && params.operation !== 'prepare' && holdNavigation) || (!params.operation && holdQuery)) {
        return new Promise<any>(resolve => held.push({ method, params, resolve }));
      }
      return Promise.resolve({ ok: true, result: { monitor: 'timeline', mode: 'fit', instance: 1, geometry: { surface: [256, 144], picture: [128, 72] } } });
    }
    if (method === 'window.bind' && holdBind) return new Promise<any>(resolve => held.push({ method, params, resolve }));
    if (method === 'window.input' || method === 'window.unbind') return new Promise<any>(resolve => held.push({ method, params, resolve }));
    return Promise.resolve({ ok: true, result: binding === null ? {} : { binding, sequence: (binding - 9) * 10 } });
  });
  const element = document.createElement('section'); element.tabIndex = -1; document.body.append(element);
  const onState = vi.fn(), menu = vi.fn(), handlePointer = vi.fn(), resetPointer = vi.fn();
  let props: Parameters<typeof useViewerInput>[0] = {
    bridge: { call, onEvent: listener => { listeners.add(listener); return () => { listeners.delete(listener); }; } },
    session: 1, native: true, loaded: true, timeline: 't1', tool: 'window', intention: 0,
    selection: { monitor: 'timeline', revision: 1, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' }, clip: 'c1', node: 'n1',
      selected: 'w1', visible: true, outline: true, tool: 'window' },
    comparison: { splitting: false, pointerCursor: 'resize_ew', handlePointer, resetPointer } as unknown as Comparison,
    viewer: { current: element }, onMenu: menu, onWindowState: onState,
  };
  const hook = renderHook(at => useViewerInput(at), { initialProps: props });
  const notify = (method: string, params: unknown, session = 1) => act(() => listeners.forEach(listener => listener({ kind: 'notification', session, method, params })));
  const update = (change: Partial<typeof props>) => { props = { ...props, ...change }; hook.rerender(props); };
  const answer = async (kind: string, result: unknown) => {
    const index = held.findIndex(each => each.params.kind === kind || each.method === kind);
    expect(index).toBeGreaterThanOrEqual(0);
    await act(async () => held.splice(index, 1)[0].resolve({ ok: true, result }));
  };
  return { hook, call, held, notify, update, answer, onState, menu, element, handlePointer, props: () => props,
    inputs: () => call.mock.calls.filter(([method]) => method === 'window.input').map(([, params]) => params) };
}

describe('the sole viewer input owner', () => {
  it('hands a picker of colour the pointer before a window, with the crosshair, and gives the window back after', async () => {
    const h = mount(); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    const onPointer = vi.fn();
    h.update({ picker: { cursor: 'crosshair', onPointer } }); await settle();
    // The window lets the pointer go; the view takes it with the crosshair of the picker.
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'view.pointer').at(-1)![1]).toEqual({ monitor: 'timeline', on: true, cursor: 'crosshair' });
    const down = point('down', 30, { snapshot: { ...point('down').snapshot!, window: null } });
    h.notify('view.pointer', down);
    h.notify('view.pointer', point('move', 31, { button: null, snapshot: { ...point('down').snapshot!, window: null } }));
    h.notify('view.pointer', point('up', 32, { snapshot: { ...point('down').snapshot!, window: null } })); await settle();
    expect(onPointer.mock.calls.map(([event]) => event.kind)).toEqual(['down', 'move', 'up']);
    expect(h.inputs()).toEqual([]);
    expect(h.handlePointer).not.toHaveBeenCalled();
    // The right button still opens the viewer's menu, and a split screen keeps the pointer for its tiles.
    h.notify('view.pointer', point('down', 33, { button: 'right', buttons: ['right'], snapshot: null })); await settle();
    expect(h.menu).toHaveBeenCalledTimes(1);
    expect(onPointer).toHaveBeenCalledTimes(3);
    // Without the picker the window is bound again.
    h.update({ picker: null }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
  });

  it('ignores a fresh wheel reporting inherited middle held without querying or mutating', async () => {
    const h = mount({ holdPrepare: true }); await settle(); h.update({ tool: 'image_wipe' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { button: 'middle', buttons: ['middle'], snapshot: null, presentation: null }));
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    h.notify('view.pointer', point('wheel', 21, { button: null, buttons: ['middle'], delta: 120, snapshot: null })); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation)).toEqual([]);
    expect(h.held).toEqual([]);
  });

  it('retains both wheels accepted before a held middle pan and prepares their cumulative base', async () => {
    const h = mount({ holdNavigation: true, holdPrepare: true }); await settle();
    const navigation = () => h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation).map(([, p]) => p);
    const first = { mode: 'scale', scale: 2.2, center: { x: 0.3, y: 0.7 } };
    const base = { ...first, scale: 2.42 };
    const geometry = { surface: [256, 144], picture: [128, 72] };
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 120 })); await settle();
    h.notify('view.pointer', point('wheel', 21, { buttons: [], button: null, delta: 120 }));
    h.notify('view.pointer', point('down', 22, { button: 'middle', buttons: ['middle'] })); await settle();
    // Only intentions accepted before this Down survive. New navigation during the pan is ignored.
    h.notify('view.pointer', point('wheel', 23, { buttons: ['middle'], button: null, delta: 120 }));
    fireEvent.keyDown(h.element, { key: '+', ctrlKey: true });
    act(() => h.hook.result.current.setZoom(4));
    await h.answer('view.viewport', { monitor: 'timeline', ...first }); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'zoom']);
    await h.answer('view.viewport', { monitor: 'timeline', ...base }); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'zoom', 'prepare']);
    await h.answer('view.viewport', { monitor: 'timeline', instance: 1, geometry, buttons: ['middle'], ...base }); await settle();
    h.notify('view.pointer', point('move', 24, { button: null, buttons: ['middle'], window: { x: 22, y: 20 }, middle_moved: true })); await settle();
    expect(navigation().at(-1)).toEqual({ monitor: 'timeline', instance: 1, operation: 'pan', base, geometry, offset: { x: 12, y: 0 } });
    await h.answer('view.viewport', { monitor: 'timeline', ...base });
    h.notify('view.pointer', point('up', 25, { button: 'middle', buttons: [], window: { x: 22, y: 20 }, middle_moved: true })); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'zoom', 'prepare', 'pan']);
    expect(h.inputs()).toEqual([]);
  });

  it('finishes keyboard and header accepted before a held middle pan in their original order', async () => {
    const h = mount({ holdNavigation: true, holdPrepare: true }); await settle();
    const navigation = () => h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation).map(([, p]) => p);
    const geometry = { surface: [256, 144], picture: [128, 72] };
    const first = { mode: 'scale', scale: 2.2, center: { x: 0.3, y: 0.7 } };
    const second = { ...first, scale: 2.42 }, base = { ...first, scale: 1.5 };
    const prepared = (setting: typeof first) => ({ monitor: 'timeline', instance: 1, geometry, buttons: ['middle'], ...setting });
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 120 })); await settle();
    fireEvent.keyDown(h.element, { key: '+', ctrlKey: true });
    act(() => h.hook.result.current.setZoom(1.5));
    h.notify('view.pointer', point('down', 21, { button: 'middle', buttons: ['middle'] }));
    await h.answer('view.viewport', { monitor: 'timeline', ...first }); await settle();
    await h.answer('view.viewport', prepared(first)); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'prepare', 'zoom']);
    await h.answer('view.viewport', { monitor: 'timeline', ...second }); await settle();
    await h.answer('view.viewport', prepared(second)); await settle();
    expect(navigation().at(-1)).toEqual({ monitor: 'timeline', instance: 1, operation: 'scale', scale: 1.5 });
    await h.answer('view.viewport', { monitor: 'timeline', ...base }); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'prepare', 'zoom', 'prepare', 'scale', 'prepare']);
    await h.answer('view.viewport', prepared(base)); await settle();
    h.notify('view.pointer', point('move', 22, { button: null, buttons: ['middle'], window: { x: 22, y: 20 }, middle_moved: true })); await settle();
    expect(navigation().at(-1)).toEqual({ monitor: 'timeline', instance: 1, operation: 'pan', base, geometry, offset: { x: 12, y: 0 } });
    await h.answer('view.viewport', { monitor: 'timeline', ...base });
    h.notify('view.pointer', point('up', 23, { button: 'middle', buttons: [], window: { x: 22, y: 20 }, middle_moved: true })); await settle();
    expect(navigation().map(p => p.operation)).toEqual(['zoom', 'prepare', 'zoom', 'prepare', 'scale', 'prepare', 'pan']);
    expect(h.inputs()).toEqual([]);
  });

  it.each(['left', 'middle'] as const)('checks inherited %s state once on a navigation intention after release outside the child', async button => {
    const h = mount({ holdPrepare: true }); await settle(); h.update({ tool: 'image_wipe' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { button, buttons: [button], snapshot: null, presentation: null }));
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    const mutation = () => h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation && p.operation !== 'prepare');
    for (const buttons of [[button], undefined, []]) {
      fireEvent.keyDown(h.element, { key: 'Z', code: 'KeyZ', shiftKey: true }); await settle();
      expect(h.held).toHaveLength(1);
      expect(h.held[0].params).toEqual({ monitor: 'timeline', operation: 'prepare', instance: 1 });
      await h.answer('view.viewport', { monitor: 'timeline', mode: 'fit', instance: 1, buttons,
        geometry: { surface: [256, 144], picture: [128, 72] } }); await settle();
      expect(mutation()).toHaveLength(buttons?.length === 0 ? 1 : 0);
    }
    expect(mutation()[0][1]).toEqual({ monitor: 'timeline', instance: 1, operation: 'fit' });
    expect(h.call.mock.calls.filter(([method]) => method === 'view.pointer')).toHaveLength(3);
    expect(h.inputs()).toEqual([]);
  });

  it.each(['event', 'instance', 'session', 'tool'] as const)('drops an inherited-button confirmation after a newer %s', async change => {
    const h = mount({ holdPrepare: true }); await settle(); h.update({ tool: 'image_wipe' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { snapshot: null, presentation: null }));
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    act(() => h.hook.result.current.setZoom(2)); await settle();
    expect(h.held).toHaveLength(1);
    if (change === 'event') h.notify('view.pointer', point('down', 21, { snapshot: null }));
    else if (change === 'instance') h.notify('view.pointer', point('move', 21, { instance: 2, button: null, buttons: [], snapshot: null }));
    else h.update(change === 'session' ? { session: 2 } : { tool: 'none' });
    await h.answer('view.viewport', { monitor: 'timeline', mode: 'fit', instance: 1, buttons: [],
      geometry: { surface: [256, 144], picture: [128, 72] } }); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation && p.operation !== 'prepare')).toHaveLength(0);
    if (change === 'event') {
      act(() => h.hook.result.current.setZoom(2)); await settle();
      expect(h.held).toHaveLength(0); // The new physical Down is still held; the old snapshot did not clear it.
    }
  });

  it.each(['middle', 'left'] as const)('synchronizes %s release before the new pointer owner is acknowledged', async button => {
    const h = mount(); await settle(); h.update({ tool: 'image_wipe' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { button, buttons: [button], snapshot: null })); await settle();
    const original = h.call.getMockImplementation()!;
    h.call.mockImplementation((method, params, context) => method === 'view.pointer' && params.on && params.cursor === 'arrow'
      ? new Promise(resolve => h.held.push({ method, params, resolve })) : original(method, params, context));
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    const mutations = () => h.call.mock.calls.filter(([method, params]) => method === 'view.viewport' && params.operation && params.operation !== 'prepare').map(([, params]) => params);
    fireEvent.keyDown(h.element, { key: 'Z', code: 'KeyZ', shiftKey: true });
    act(() => h.hook.result.current.setZoom(2)); await settle();
    expect(mutations()).toEqual([]);
    expect(h.call.mock.calls.filter(([method]) => method === 'view.pointer').slice(-2).map(([, params]) => params)).toEqual([
      { monitor: 'timeline', on: false }, { monitor: 'timeline', on: true, cursor: 'arrow' },
    ]);
    // Native on-boundary synchronization can overtake the RPC response. It only updates
    // physical state here; the new owner cannot receive an old gesture.
    h.notify('view.pointer', point('up', 21, { button, buttons: [], snapshot: null }));
    await h.answer('view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }); await settle();
    fireEvent.keyDown(h.element, { key: 'Z', code: 'KeyZ', shiftKey: true }); await settle();
    act(() => h.hook.result.current.setZoom(2)); await settle();
    expect(mutations()).toEqual([
      { monitor: 'timeline', instance: 1, operation: 'fit' }, { monitor: 'timeline', instance: 1, operation: 'scale', scale: 2 },
    ]);
    expect(h.inputs()).toEqual([]);
  });

  it.each(['keyboard', 'header'] as const)('keeps queued %s navigation with its known instance behind a delayed wheel', async source => {
    const h = mount({ holdNavigation: true }); await settle();
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 120 })); await settle();
    if (source === 'keyboard') fireEvent.keyDown(h.element, { key: 'Z', code: 'KeyZ', shiftKey: true });
    else act(() => h.hook.result.current.setZoom(1));
    h.notify('view.pointer', point('move', 21, { instance: 2, button: null, buttons: [] }));
    const original = h.call.getMockImplementation()!;
    h.call.mockImplementation((method, params, context) => method === 'view.viewport' && params.operation === 'prepare'
      ? Promise.resolve({ ok: true, result: { monitor: 'timeline', mode: 'fit', instance: 2, geometry: { surface: [256, 144], picture: [128, 72] } } })
      : original(method, params, context));
    await h.answer('view.viewport', { monitor: 'timeline', mode: 'scale', scale: 2.2, center: { x: 0.5, y: 0.5 }, instance: 1 }); await settle();
    const navigation = h.call.mock.calls.filter(([method, params]) => method === 'view.viewport' && params.operation).map(([, params]) => params);
    if (h.held.length) await h.answer('view.viewport', { monitor: 'timeline', mode: 'fit', instance: 2 });
    expect(navigation).toEqual([{ monitor: 'timeline', instance: 1, operation: 'zoom', factor: 1.1, at: { x: 10.5, y: 20.5 } }]);
    expect(h.hook.result.current.viewport).toEqual({ mode: 'fit' });
  });

  it.each([0, 12])('retains a middle gesture started after native LeftUp while that capture is closing (dx=%s)', async dx => {
    const h = mount(); await settle();
    h.notify('view.pointer', point('down', 20));
    h.notify('view.pointer', point('up', 21));
    h.notify('view.pointer', point('down', 22, { button: 'middle', buttons: ['middle'] }));
    h.notify('view.pointer', point('move', 23, { button: null, buttons: ['middle'], window: { x: 10 + dx, y: 20 }, middle_moved: dx !== 0 }));
    h.notify('view.pointer', point('up', 24, { button: 'middle', buttons: [], window: { x: 10 + dx, y: 20 }, middle_moved: dx !== 0 }));
    await settle();
    const navigation = () => h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation).map(([, p]) => p);
    expect(navigation()).toEqual([]);
    await h.answer('down', state()); await settle();
    expect(h.inputs()).toEqual([{ binding: 10, kind: 'down', event: 20 }, { binding: 10, capture: 1, kind: 'up' }]);
    expect(navigation()).toEqual([]);
    await h.answer('up', state({ phase: 'ended', sequence: 14, event: 21 })); await settle();
    expect(navigation()).toEqual([
      { monitor: 'timeline', operation: 'prepare', instance: 1 },
      ...(dx === 0 ? [] : [{ monitor: 'timeline', instance: 1, operation: 'pan', base: { mode: 'fit' },
        geometry: { surface: [256, 144], picture: [128, 72] }, offset: { x: dx, y: 0 } }]),
    ]);
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    expect(h.call.mock.calls.some(([method]) => method === 'view.pointer' || method === 'transaction.apply')).toBe(false);
    expect(h.hook.result.current.capturing).toBe(false);
  });

  it('orders a wheel ahead of middle preparation even when its reply is delayed', async () => {
    const h = mount({ holdNavigation: true, holdPrepare: true }); await settle();
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 120 })); await settle();
    h.notify('view.pointer', point('down', 21, { button: 'middle', buttons: ['middle'] }));
    h.notify('view.pointer', point('up', 22, { button: 'middle', buttons: [], window: { x: 20, y: 25 } })); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'prepare')).toHaveLength(0);
    const base = { mode: 'scale', scale: 2.2, center: { x: 0.3, y: 0.7 } };
    await h.answer('view.viewport', { monitor: 'timeline', ...base }); await settle();
    await h.answer('view.viewport', { monitor: 'timeline', instance: 1, geometry: { surface: [256, 144], picture: [128, 72] }, ...base }); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'pan').at(-1)?.[1]).toMatchObject({ base, offset: { x: 10, y: 5 } });
    await h.answer('view.viewport', { monitor: 'timeline', ...base });
  });

  it('keeps navigation enabled for Image Wipe with no active divider', async () => {
    const h = mount(); await settle(); h.update({ tool: 'image_wipe', comparison: { ...h.props().comparison, pointerCursor: null } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.at(-1)).toEqual(['view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: 1 }]);
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 30, snapshot: null })); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'zoom')).toHaveLength(1);
    expect(h.handlePointer).not.toHaveBeenCalled();
  });

  it('uses only the base navigation pointer while Window controls are hidden', async () => {
    const h = mount(); await settle(); h.update({ selection: { ...h.props().selection!, visible: false } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.at(-1)).toEqual(['view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: 1 }]);
    h.notify('view.pointer', point('down', 20, { snapshot: null }));
    expect(h.inputs()).toEqual([]); expect(h.handlePointer).not.toHaveBeenCalled();
  });

  it('rejects a pan if prepare finds resized geometry after the native down', async () => {
    const h = mount({ holdPrepare: true }); await settle();
    h.notify('view.pointer', point('down', 20, { button: 'middle', buttons: ['middle'] })); await settle();
    h.notify('view.pointer', point('up', 21, { button: 'middle', buttons: [], window: { x: 80, y: 50 } }));
    await h.answer('view.viewport', { monitor: 'timeline', instance: 1, mode: 'fit', geometry: { surface: [300, 144], picture: [128, 72] } }); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'pan')).toHaveLength(0);
  });

  it.each(['up', 'cancel'] as const)('retains the last pan device position when %s arrives before prepare', async terminal => {
    const h = mount({ holdPrepare: true }); await settle();
    h.notify('view.pointer', point('down', 20, { button: 'middle', buttons: ['middle'], window: { x: 10, y: 20 } })); await settle();
    h.notify('view.pointer', point('move', 21, { button: null, buttons: ['middle'], window: { x: 30, y: 45 }, middle_moved: true }));
    h.notify('view.pointer', point(terminal, 22, { button: terminal === 'up' ? 'middle' : null, buttons: [], window: { x: 40, y: 50 }, middle_moved: true }));
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'pan')).toHaveLength(0);
    const base = { mode: 'scale', scale: 2.42, center: { x: 0.2, y: 0.7 } };
    const geometry = { surface: [256, 144], picture: [128, 72] };
    await h.answer('view.viewport', { monitor: 'timeline', instance: 1, geometry, ...base }); await settle();
    const pans = h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'pan').map(([, p]) => p);
    expect(pans).toEqual([
      { monitor: 'timeline', instance: 1, operation: 'pan', base, geometry, offset: { x: 20, y: 25 } },
      { monitor: 'timeline', instance: 1, operation: 'pan', base, geometry, offset: { x: 30, y: 30 } },
    ]);
    expect(h.inputs()).toEqual([]);
  });

  it('keeps a middle gesture exclusive and a zero-displacement click free of viewport mutations', async () => {
    const h = mount(); await settle();
    h.notify('view.pointer', point('down', 20, { button: 'middle', buttons: ['middle'] })); await settle();
    h.notify('view.pointer', point('wheel', 21, { button: null, buttons: ['middle'], delta: 120 }));
    h.notify('view.pointer', point('down', 22, { button: 'left', buttons: ['left', 'middle'] }));
    fireEvent.keyDown(h.element, { key: '+', ctrlKey: true });
    act(() => h.hook.result.current.setZoom(2));
    h.notify('view.pointer', point('up', 23, { button: 'left', buttons: ['middle'] }));
    // The native queue may coalesce an excursion back to its origin. That is not a future delete click.
    h.notify('view.pointer', point('move', 24, { button: null, buttons: ['middle'], middle_moved: true }));
    h.notify('view.pointer', point('up', 25, { button: 'middle', buttons: [], middle_moved: true })); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation && p.operation !== 'prepare')).toHaveLength(0);
    expect(h.inputs()).toEqual([]); expect(h.handlePointer).not.toHaveBeenCalled();
  });

  it('prepares keyboard and header navigation before any native mouse event, respecting focus and percentages', async () => {
    const h = mount(); await settle();
    fireEvent.keyDown(document.body, { key: '+', ctrlKey: true });
    const field = document.createElement('input'); h.element.append(field);
    fireEvent.keyDown(field, { key: '+', ctrlKey: true }); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation)).toHaveLength(0);
    for (const keys of [{ key: '=', ctrlKey: true }, { key: '-', ctrlKey: true }, { key: 'Z', code: 'KeyZ', shiftKey: true, altKey: true },
      { key: 'Z', code: 'KeyZ', shiftKey: true }]) { fireEvent.keyDown(h.element, keys); await settle(); }
    act(() => h.hook.result.current.setZoom(1.5)); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation !== 'prepare' && p.operation).map(([, p]) => p)).toEqual([
      { monitor: 'timeline', instance: 1, operation: 'zoom', factor: 1.1 }, { monitor: 'timeline', instance: 1, operation: 'zoom', factor: 1 / 1.1 },
      { monitor: 'timeline', instance: 1, operation: 'scale', scale: 1 }, { monitor: 'timeline', instance: 1, operation: 'fit' },
      { monitor: 'timeline', instance: 1, operation: 'scale', scale: 1.5 },
    ]);
  });

  it.each(['session', 'timeline', 'tool', 'instance'] as const)('drops a pan preparation after changing %s', async change => {
    const h = mount({ holdPrepare: true }); await settle();
    h.notify('view.pointer', point('down', 20, { button: 'middle', buttons: ['middle'] })); await settle();
    h.notify('view.pointer', point('up', 21, { button: 'middle', buttons: [], window: { x: 50, y: 60 }, middle_moved: true }));
    if (change === 'instance') h.notify('view.pointer', point('move', 22, { instance: 2, buttons: [] }));
    else h.update(change === 'session' ? { session: 2 } : change === 'timeline' ? { timeline: 't2' } : { tool: 'none' });
    await h.answer('view.viewport', { monitor: 'timeline', mode: 'scale', scale: 3, center: { x: 0.2, y: 0.7 }, instance: 1,
      geometry: { surface: [256, 144], picture: [128, 72] } }); await settle();
    expect(h.call.mock.calls.filter(([method, p]) => method === 'view.viewport' && p.operation === 'pan')).toHaveLength(0);
  });

  it('does not let an initial late query overwrite an accepted wheel setting or a retired instance reply', async () => {
    const h = mount({ holdQuery: true, holdNavigation: true }); await settle();
    h.notify('view.pointer', point('wheel', 20, { buttons: [], button: null, delta: 120 })); await settle();
    await act(async () => h.held.find(each => each.params.operation === 'zoom')!.resolve({ ok: true, result: { monitor: 'timeline', mode: 'scale', scale: 2.2, center: { x: 0.4, y: 0.6 } } }));
    await settle();
    await h.answer('view.viewport', { monitor: 'timeline', mode: 'fit' }); await settle();
    expect(h.hook.result.current.viewport).toMatchObject({ mode: 'scale', scale: 2.2 });
    h.notify('view.pointer', point('wheel', 21, { buttons: [], button: null, delta: 120 })); await settle();
    h.notify('view.pointer', point('move', 22, { instance: 2, buttons: [] }));
    await act(async () => h.held.at(-1)!.resolve({ ok: true, result: { monitor: 'timeline', mode: 'scale', scale: 9, center: { x: 0.4, y: 0.6 } } }));
    await settle();
    expect(h.hook.result.current.viewport).toMatchObject({ mode: 'scale', scale: 2.2 });
  });

  it('keeps navigation pointer input with OSC off without dispatching a hidden wipe', async () => {
    const h = mount(); await settle();
    h.update({ tool: 'none' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.at(-1)).toEqual(['view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: 1 }]);
    h.notify('view.pointer', point('down', 21, { snapshot: null }));
    expect(h.handlePointer).not.toHaveBeenCalled();
    h.notify('view.pointer', point('up', 22, { snapshot: null }));
    h.notify('view.pointer', point('wheel' as ViewPointer['kind'], 23, { snapshot: null, buttons: [], button: null, delta: 30 } as Partial<ViewPointer>));
    await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'view.viewport').at(-1)?.[1]).toMatchObject({
      monitor: 'timeline', instance: 1, operation: 'zoom', factor: 1.1 ** 0.25, at: { x: 10.5, y: 20.5 },
    });
  });

  it('waits for the same Window up to finish before wheel navigation and keeps the binding', async () => {
    const h = mount(); await settle();
    h.notify('view.pointer', point('down'));
    h.notify('view.pointer', point('wheel' as ViewPointer['kind'], 21, { delta: 120 } as Partial<ViewPointer>));
    await settle();
    expect(h.call.mock.calls.filter(([method, params]) => method === 'view.viewport' && params.operation)).toHaveLength(0);
    h.notify('view.pointer', point('up', 22));
    h.notify('view.pointer', point('wheel' as ViewPointer['kind'], 23, { buttons: [], button: null, delta: 120 } as Partial<ViewPointer>));
    await settle();
    expect(h.call.mock.calls.filter(([method, params]) => method === 'view.viewport' && params.operation)).toHaveLength(0);
    await h.answer('down', state()); await settle();
    await h.answer('up', state({ phase: 'ended', sequence: 14, event: 22 })); await settle();
    expect(h.call.mock.calls.filter(([method, params]) => method === 'view.viewport' && params.operation).map(([, params]) => params)).toEqual([
      { monitor: 'timeline', instance: 1, operation: 'zoom', factor: 1.1, at: { x: 10.5, y: 20.5 } },
    ]);
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    expect(h.call.mock.calls.some(([method]) => method === 'view.pointer')).toBe(false);
  });

  it('remembers retirement before the pending bind reply and waits for the matching copy before rebinding', async () => {
    const h = mount({ holdBind: true }); await settle();
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 2, reason: 'project_changed' });
    await h.answer('window.bind', { binding: 10, sequence: 10 }); await settle();
    h.notify('view.pointer', point('down'));
    expect(h.inputs()).toEqual([]);
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    h.update({ selection: { ...h.props().selection!, revision: 2 } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind').at(-1)![1]).toMatchObject({ revision: 2 });
  });

  it('retries a stale bind against an already newer copy using the sent revision', async () => {
    const h = mount({ holdBind: true }); await settle();
    h.update({ selection: { ...h.props().selection!, revision: 3 } });
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'stale', data: { reason: 'stale_revision' } } }));
    await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
    expect(h.call.mock.calls.at(-1)![1]).toMatchObject({ revision: 3 });
  });

  it.each([true, false])('renews an idle retired binding once with the current copy (copy first: %s)', async copyFirst => {
    const h = mount(); await settle();
    if (copyFirst) h.update({ selection: { ...h.props().selection!, revision: 5 } });
    h.notify('window.unbound', { binding: 10, sequence: 9, revision: 3, reason: 'project_changed' });
    h.notify('window.unbound', { binding: 9, sequence: 99, revision: 3, reason: 'project_changed' });
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 3, reason: 'project_changed' }, 0);
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 3, reason: 'project_changed' });
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 3, reason: 'project_changed' });
    await settle();
    if (!copyFirst) {
      expect(h.call.mock.calls.filter(([method]) => method === 'window.unbind')).toHaveLength(0);
      h.update({ selection: { ...h.props().selection!, revision: 5 } }); await settle();
    }
    // The server may already have removed the old binding. Its cleanup refusal must not prevent renewal.
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'binding', data: { reason: 'binding' } } }));
    await settle();
    const binds = () => h.call.mock.calls.filter(([method]) => method === 'window.bind');
    expect(binds()).toHaveLength(2); expect(binds().at(-1)![1]).toMatchObject({ revision: 5 });
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 3, reason: 'project_changed' });
    h.notify('window.unbound', { binding: 11, sequence: 19, revision: 3, reason: 'project_changed' });
    h.notify('window.unbound', { binding: 11, sequence: 22, revision: 6, reason: 'project_changed' }, 0);
    h.update({ selection: { ...h.props().selection!, revision: 6 } }); await settle();
    expect(binds()).toHaveLength(2);
    expect(h.call.mock.calls.some(([method]) => method === 'view.pointer')).toBe(false);
  });

  it('matches early retirement candidates to the pending binding and never revives it with a late reply', async () => {
    const h = mount({ holdBind: true }); await settle();
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 2, reason: 'project_changed' });
    // A different binding, even with a later sequence, must not overwrite this candidate.
    h.notify('window.unbound', { binding: 9, sequence: 99, revision: 100, reason: 'project_changed' });
    h.update({ selection: { ...h.props().selection!, revision: 3 } });
    await h.answer('window.bind', { binding: 10, sequence: 10 }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 2, reason: 'project_changed' });
    await h.answer('window.bind', { binding: 11, sequence: 20 }); await settle();
    h.notify('view.pointer', point('down', 21, { snapshot: { ...point('down').snapshot!, revision: 3, window: { binding: 11, selected: 'w1' } } }));
    expect(h.inputs()).toEqual([{ binding: 11, kind: 'down', event: 21 }]);
  });

  it('does not retry a stale bind at the same revision, and renews once a newer copy arrives', async () => {
    const h = mount({ holdBind: true }); await settle();
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'stale', data: { reason: 'stale_revision' } } }));
    h.update({ selection: { ...h.props().selection! } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    h.update({ selection: { ...h.props().selection!, revision: 2 } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
    await h.answer('window.bind', { binding: 11, sequence: 20 });
    h.update({ selection: { ...h.props().selection!, revision: 8 } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
  });

  it('lets a new target supersede an old pending stale bind without restoring the old context', async () => {
    const h = mount({ holdBind: true }); await settle();
    h.update({ selection: { ...h.props().selection!, revision: 3, target: { level: 'clip', timeline: 't1', clip: 'c2' }, clip: 'c2' } });
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'stale', data: { reason: 'stale_revision' } } }));
    await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
    expect(h.call.mock.calls.at(-1)![1]).toMatchObject({ revision: 3, clip: 'c2' });
    await h.answer('window.bind', { binding: 11, sequence: 20 }); await settle();
    expect(h.hook.result.current.status).toBe('idle');
  });

  it('does not restore a retired target after switching OSC off or switching sessions', async () => {
    const h = mount(); await settle();
    h.notify('window.unbound', { binding: 10, sequence: 12, revision: 2, reason: 'project_changed' });
    h.update({ tool: 'none' }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' });
    h.update({ selection: { ...h.props().selection!, revision: 2 } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    h.update({ session: 2, tool: 'window' }); await settle();
    h.notify('window.unbound', { binding: 11, sequence: 22, revision: 3, reason: 'project_changed' }, 1);
    h.update({ selection: { ...h.props().selection!, revision: 3 } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
  });

  it('recovers after capture ended then binding retired, ignoring the older pending down reply', async () => {
    const h = mount(); await settle(); h.notify('view.pointer', point('down'));
    h.notify('window.state', state({ phase: 'ended', sequence: 14 }));
    h.notify('window.unbound', { binding: 10, sequence: 15, revision: 2, reason: 'project_changed' });
    h.update({ selection: { ...h.props().selection!, revision: 2 } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    await h.answer('down', state());
    expect(h.hook.result.current.capturing).toBe(false);
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(2);
    expect(h.inputs()).toHaveLength(1);
  });

  it.each([['up', true], ['cancel', true], ['up', false], ['cancel', false]] as const)(
    'remembers %s before down replies (early state: %s) and does not revive on a late arming reply', async (kind, earlyState) => {
    const h = mount(); await settle();
    h.notify('view.pointer', point('down'));
    expect(document.activeElement).toBe(h.element);
    expect(h.inputs()).toEqual([{ binding: 10, kind: 'down', event: 20 }]);
    if (kind === 'up') h.notify('view.pointer', point('up', 21));
    else fireEvent.keyDown(h.element, { key: 'Escape' });
    expect(h.inputs()).toHaveLength(1);
    // The asynchronous notification can supply the capture before the down reply.
    if (earlyState) h.notify('window.state', state());
    else await h.answer('down', state());
    expect(h.inputs()[1]).toEqual({ binding: 10, capture: 1, kind });
    h.notify('window.state', state({ sequence: 14, phase: 'ended' }));
    if (earlyState) await h.answer('down', state());
    await h.answer(kind, state({ sequence: 14, phase: 'ended' }));
    expect(h.hook.result.current.capturing).toBe(false);
    expect(h.hook.result.current.status).toBe('idle');
    expect(h.inputs()).toHaveLength(2);
  });

  it('echoes native moves beyond divider and picture without coordinates, filtering stale session, instance, binding and sequence', async () => {
    const h = mount(); await settle();
    h.notify('view.pointer', point('down'), 0);
    h.notify('view.pointer', point('down', 20, { snapshot: { ...point('down').snapshot!, window: { binding: 9, selected: 'w1' } } }));
    expect(h.inputs()).toEqual([]);
    h.notify('view.pointer', point('down', 21));
    await h.answer('down', state({ event: 21 }));
    h.notify('window.state', state({ event: 21, sequence: 13, phase: 'armed', selected: 'w2' }));
    h.notify('window.state', state({ event: 21, sequence: 12, phase: 'arming' }));
    h.notify('window.state', state({ event: 21, binding: 9, sequence: 99, phase: 'ended' }));
    h.notify('view.pointer', point('move', 22, { instance: 0 }));
    h.notify('view.pointer', point('move', 23, { x: -100, y: 2000, inside: false, snapshot: null }));
    expect(h.inputs().at(-1)).toEqual({ binding: 10, capture: 1, kind: 'move', event: 23 });
    h.notify('view.pointer', point('move', 22));
    h.notify('view.pointer', point('up', 24, { x: 9999, y: -500, inside: false }));
    expect(h.inputs().at(-1)).toEqual({ binding: 10, capture: 1, kind: 'up' });
    h.notify('window.state', state({ event: 24, sequence: 16, phase: 'ended', selected: 'w2' }));
    await h.answer('move', state({ event: 23, sequence: 14, phase: 'armed' }));
    await h.answer('up', state({ event: 24, sequence: 16, phase: 'ended' }));
    expect(h.onState.mock.calls.at(-1)![0].selected).toBe('w2');
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    expect(h.handlePointer).not.toHaveBeenCalled();
  });

  it('retains one binding through own copy revisions and server selections, but serializes user ownership switches', async () => {
    const h = mount(); await settle();
    h.update({ selection: { ...h.props().selection!, revision: 5, selected: 'w2' } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'window.bind')).toHaveLength(1);
    h.notify('view.pointer', point('down', 20, { button: 'right' }));
    expect(h.menu).toHaveBeenCalledWith({ x: 10, y: 20 }); expect(h.inputs()).toEqual([]);
    h.update({ tool: 'image_wipe' }); await settle();
    expect(h.call.mock.calls.at(-1)![0]).toBe('window.unbind');
    expect(h.call.mock.calls.some(([method]) => method === 'view.pointer')).toBe(false);
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.at(-1)).toEqual(['view.pointer', { monitor: 'timeline', on: true, cursor: 'resize_ew' }, { expectedSession: 1 }]);
    h.notify('view.pointer', point('down', 21, { snapshot: null })); expect(h.handlePointer).toHaveBeenCalledTimes(1);
    h.update({ tool: 'none' }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'view.pointer').slice(-2)).toEqual([
      ['view.pointer', { monitor: 'timeline', on: false }, { expectedSession: 1 }],
      ['view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: 1 }],
    ]);
  });

  it('suppresses Window under Split and dispatches tiles, then restores the chosen Window tool', async () => {
    const h = mount(); await settle();
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { snapshot: { ...point('down').snapshot!, comparison: 'split', window: null } }));
    expect(h.handlePointer).toHaveBeenCalledTimes(1); expect(h.inputs()).toEqual([]);
    h.update({ comparison: { ...h.props().comparison, splitting: false } }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method !== 'view.viewport').slice(-2).map(([method]) => method)).toEqual(['view.pointer', 'window.bind']);
  });

  it('keeps cleanup on the originating session while a replacement waits for the old down', async () => {
    const h = mount(); await settle(); h.notify('view.pointer', point('down'));
    h.update({ session: 2 }); await settle();
    expect(h.hook.result.current.busy()).toBe(true);
    await h.answer('down', state());
    expect(h.inputs().at(-1)).toEqual({ binding: 10, capture: 1, kind: 'up' });
    await h.answer('up', state({ phase: 'ended', sequence: 14 })); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.hook.result.current.busy()).toBe(false);
    expect(h.call.mock.calls.filter(([method]) => method.startsWith('window.')).map(([method, , context]) => [method, context])).toEqual([
      ['window.bind', { expectedSession: 1 }], ['window.input', { expectedSession: 1 }], ['window.input', { expectedSession: 1 }],
      ['window.unbind', { expectedSession: 1 }], ['window.bind', { expectedSession: 2 }],
    ]);
    h.notify('view.pointer', point('down', 30), 1); expect(h.inputs()).toHaveLength(2);
  });

  it('awaits a terminal already sent from an early state before unbinding', async () => {
    const h = mount(); await settle(); h.notify('view.pointer', point('down'));
    h.notify('window.state', state());
    fireEvent.keyDown(h.element, { key: 'Escape' });
    h.update({ tool: 'image_wipe' });
    await h.answer('down', state()); await settle();
    expect(h.call.mock.calls.some(([method]) => method === 'window.unbind')).toBe(false);
    await h.answer('cancel', state({ phase: 'ended', sequence: 14 })); await settle();
    expect(h.call.mock.calls.at(-1)![0]).toBe('window.unbind');
  });

  it('does not begin another capture while a window creation is awaiting confirmation', async () => {
    const h = mount(); await settle(); h.update({ creating: true });
    h.notify('view.pointer', point('down'));
    expect(h.inputs()).toEqual([]);
    h.update({ creating: false }); h.notify('view.pointer', point('down', 21));
    expect(h.inputs()).toEqual([{ binding: 10, kind: 'down', event: 21 }]);
  });

  it('cleans a late bind before starting a new owner, and drops an intermediate wipe owner superseded by Off', async () => {
    const h = mount({ holdBind: true });
    await settle(); h.update({ tool: 'image_wipe' }); await settle(); h.update({ tool: 'none' });
    await h.answer('window.bind', { binding: 10, sequence: 10 }); await settle();
    expect(h.call.mock.calls.at(-1)![0]).toBe('window.unbind');
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.filter(([method]) => method === 'view.pointer')).toEqual([
      ['view.pointer', { monitor: 'timeline', on: true, cursor: 'arrow' }, { expectedSession: 1 }],
    ]);
    h.notify('view.pointer', point('down', 30, { button: 'right', snapshot: null }));
    expect(h.menu).toHaveBeenCalledOnce();
  });

  it('treats a hit refusal as a blank click and reports an unavailable frame in UI state', async () => {
    const h = mount(); await settle(); h.notify('view.pointer', point('down'));
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'hit', data: { reason: 'hit' } } }));
    expect(h.hook.result.current.status).toBe('idle'); expect(h.hook.result.current.busy()).toBe(false);
    h.notify('view.pointer', point('down', 21));
    await act(async () => h.held.shift()!.resolve({ ok: false, error: { code: -32001, message: 'map', data: { reason: 'map' } } }));
    expect(h.hook.result.current.status).toBe('refused');
  });
});
