// @vitest-environment jsdom
import { act, cleanup, fireEvent, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useViewerInput } from '../src/studio/color/useViewerInput';
import type { Comparison } from '../src/studio/color/useComparison';
import type { EngineEvent, ViewPointer, WindowState } from '../src/studio/engine';
import { settle } from './colorEngine';

afterEach(cleanup);
const state = (extra: Partial<WindowState> = {}): WindowState => ({ binding: 10, capture: 1, sequence: 12, phase: 'arming', reason: null,
  selected: 'w1', event: 20, revision: 1, keys: { pan: 0, tilt: 3, clock: 'source', frame: 10 }, ...extra });
const point = (kind: ViewPointer['kind'], event = 20, extra: Partial<ViewPointer> = {}): ViewPointer => ({ monitor: 'timeline', instance: 1, event,
  presentation: { serial: 1, tag: 1, map: null }, snapshot: { revision: 1, timeline: 't1', frame: 10, canvas: [1920, 1080], ready: true,
    editable: true, comparison: 'wipe', window: { binding: 10, selected: 'w1' } },
  kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x: 300, y: 200,
  inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false, ...extra });

function mount({ holdBind = false } = {}) {
  const listeners = new Set<(event: EngineEvent) => void>();
  const held: { method: string; params: any; resolve: (answer: any) => void }[] = [];
  let nextBinding = 10;
  const call = vi.fn((method: string, params: any, _context?: unknown) => {
    const binding = method === 'window.bind' ? nextBinding++ : null;
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
    expect(h.call.mock.calls.at(-1)).toEqual(['view.pointer', { monitor: 'timeline', on: false }, { expectedSession: 1 }]);
  });

  it('suppresses Window under Split and dispatches tiles, then restores the chosen Window tool', async () => {
    const h = mount(); await settle();
    h.update({ comparison: { ...h.props().comparison, splitting: true, pointerCursor: 'arrow' } }); await settle();
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    h.notify('view.pointer', point('down', 20, { snapshot: { ...point('down').snapshot!, comparison: 'split', window: null } }));
    expect(h.handlePointer).toHaveBeenCalledTimes(1); expect(h.inputs()).toEqual([]);
    h.update({ comparison: { ...h.props().comparison, splitting: false } }); await settle();
    expect(h.call.mock.calls.slice(-2).map(([method]) => method)).toEqual(['view.pointer', 'window.bind']);
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
    const h = mount();
    let bound!: (answer: any) => void;
    h.call.mockImplementationOnce(() => new Promise(resolve => { bound = resolve; }));
    await settle(); h.update({ tool: 'image_wipe' }); await settle(); h.update({ tool: 'none' });
    await act(async () => bound({ ok: true, result: { binding: 10, sequence: 10 } })); await settle();
    expect(h.call.mock.calls.at(-1)![0]).toBe('window.unbind');
    await h.answer('window.unbind', { binding: 10, phase: 'ended' }); await settle();
    expect(h.call.mock.calls.some(([method]) => method === 'view.pointer')).toBe(false);
    h.notify('view.pointer', point('down', 30, { button: 'right' }));
    expect(h.menu).not.toHaveBeenCalled();
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
