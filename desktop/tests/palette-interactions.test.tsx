// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Palettes } from '../src/studio/color/Palettes';
import { fullCorrector } from '../src/studio/color/corrector';
import { HOLD_MS, type EditedNode } from '../src/studio/color/useNodeEdit';
import { makeDefaultNode } from '../src/studio/color/nodeEdits';
import { mergePatch, type Corrector } from '../src/studio/engine';
import { likeABrowser } from './mounted';

beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 200, bottom: 200, width: 200, height: 200, toJSON() {} });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
const field = (path = 'primaries.saturation') => document.querySelector<HTMLInputElement>(`[data-field="${path}"] input`)!;
const pointer = (element: Element, kind: 'pointerDown' | 'pointerMove' | 'pointerUp', x = 0, y = 0) => fireEvent[kind](element, { pointerId: 1, button: 0, clientX: x, clientY: y });
const STATS = { frame: 0, percentiles: { red: { 1: 0.1, 50: 0.5, 99: 0.9 }, green: { 1: 0.05, 50: 0.45, 99: 0.85 }, blue: { 1: 0, 50: 0.4, 99: 0.8 }, luma: { 1: 0.05, 50: 0.45, 99: 0.85 } } };
function mount(extra: Partial<EditedNode> = {}) {
  const pending: { method: string; resolve: (value: unknown) => void }[] = [];
  const call = vi.fn((method: string, _params?: Record<string, unknown>, _context?: unknown) => new Promise<any>(resolve => pending.push({ method, resolve })));
  let context: EditedNode = { bridge: { call }, session: 1, revision: 10, target: { level: 'pre_clip', group: 'g1' },
    clip: 'c1', clock: 'source-clock', timelineFrame: 0, playing: false,
    node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({}) }, frame: 100, onFailure: vi.fn(), ...extra };
  const tree = () => <Palettes merged={false} left="primaries" center="curves" last="left" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}} edited={context} />;
  const view = render(tree());
  return { call, pending, context: () => context,
    update: (change: Partial<EditedNode>) => { context = { ...context, ...change }; view.rerender(tree()); },
    sync: (revision: number, params: unknown, before = context.before) => {
      context = { ...context, revision, before, node: { ...context.node, params: mergePatch(context.node.params, params) as Corrector } as EditedNode['node'] };
      view.rerender(tree());
    },
    writes: () => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params!),
    answer: async (at = 0, result: unknown = STATS, fail = false) => { await act(async () => pending.splice(at, 1)[0].resolve(fail ? { ok: false, error: { code: -1, message: 'old failure', data: null } } : { ok: true, result })); },
  };
}

describe('real controls keep the interaction that was started', () => {
  it.each(['down', 'draft Enter', 'draft blur', 'curve'] as const)('invalidates %s before its first preview on an external revision', async kind => {
    const edit = mount();
    const element = kind === 'curve' ? document.querySelector<HTMLElement>('[data-test="curve-editor"]')! : field();
    if (kind.startsWith('draft')) { fireEvent.doubleClick(element); fireEvent.change(element, { target: { value: '80' } }); }
    else pointer(element, 'pointerDown', 0, 200);
    const before = edit.writes().length;
    edit.update({ revision: 11 });
    if (kind === 'draft Enter') fireEvent.keyDown(element, { key: 'Enter' });
    else if (kind === 'draft blur') fireEvent.blur(element);
    else { pointer(element, 'pointerMove', 40, 150); pointer(element, 'pointerUp', 40, 150); }
    await settle();
    expect(edit.writes()).toHaveLength(before);
    expect(field().value).toBe('50.00');
  });

  it.each(['clip', 'clock', 'session', 'seek'] as const)('invalidates an unsent draft on a changed %s and keeps the palette mode', async change => {
    const edit = mount();
    fireEvent.click(document.querySelector('[data-mode="bars"]')!);
    const old = field(); fireEvent.doubleClick(old); fireEvent.change(old, { target: { value: '75' } });
    edit.update(change === 'seek' ? { frame: 101, timelineFrame: 1 } : change === 'clip' ? { clip: 'c2' } : change === 'session' ? { session: 2 } : { clock: 'slip' });
    fireEvent.blur(old); await settle();
    expect(edit.writes()).toHaveLength(0);
    expect(document.querySelector('[data-mode="bars"][role="radio"]')!.getAttribute('aria-checked')).toBe('true');
  });

  it('keeps a static control during playback and invalidates only the parameter whose edit key changes', async () => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { saturation: { keys: [{ frame: 0, value: 30 }, { frame: 10, value: 70 }] } } }) } });
    const keyed = field(); const staticControl = field('primaries.hue');
    pointer(keyed, 'pointerDown'); pointer(staticControl, 'pointerDown');
    edit.update({ frame: 11, timelineFrame: 11 });
    pointer(keyed, 'pointerMove', 40); pointer(keyed, 'pointerUp', 40);
    expect(edit.writes()).toHaveLength(0);
    expect(field('primaries.hue')).toBe(staticControl);
    pointer(staticControl, 'pointerMove', 40); pointer(staticControl, 'pointerUp', 40);
    expect(edit.writes()).toHaveLength(1);
    expect(edit.writes()[0]).toMatchObject({ ops: [{ params: { primaries: { hue: 60 } } }] });
  });

  it.each(['wheels', 'bars'].flatMap(mode => ['drag', 'draft Enter', 'draft blur'].map(kind => [mode, kind])))
  ('keeps a static %s field %s when another channel in the same wheel crosses a key', async (mode, kind) => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { lift: { r: { keys: [{ frame: 0, value: 0 }, { frame: 10, value: 0.2 }] } } } }) } });
    if (mode === 'bars') fireEvent.click(screen.getByRole('radio', { name: 'Color Bars' }));
    const green = field('primaries.lift.g');
    if (kind === 'drag') { pointer(green, 'pointerDown'); pointer(green, 'pointerMove', 20); }
    else { fireEvent.doubleClick(green); fireEvent.change(green, { target: { value: '0.15' } }); }
    edit.update({ frame: 11, timelineFrame: 11 });
    expect(field('primaries.lift.g')).toBe(green);
    if (kind === 'drag') {
      pointer(green, 'pointerMove', 40); pointer(green, 'pointerUp', 40);
      await edit.answer(0, { revision: 11 });
      expect(edit.writes()).toHaveLength(2);
      expect(edit.writes()[1]).toMatchObject({ revision: 10, gesture: { ...(edit.writes()[0].gesture as object), end: true } });
      const params = (edit.writes()[1].ops as { params: unknown }[])[0].params;
      await edit.answer(0, { revision: 12 }); edit.sync(12, params);
      expect(screen.getByRole<HTMLButtonElement>('button', { name: 'Auto Color' }).disabled).toBe(false);
    } else {
      expect(green.value).toBe('0.15');
      if (kind === 'draft Enter') fireEvent.keyDown(green, { key: 'Enter' }); else fireEvent.blur(green);
      expect(edit.writes()).toHaveLength(1);
      expect(edit.writes()[0]).toMatchObject({ ops: [{ params: { primaries: { lift: { g: 0.15 } } } }] });
    }
  });

  it.each(['RGB ring', 'Ctrl ring', 'master wheel'] as const)('uses the %s actual paths from pointer-down across a Y key boundary', async kind => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { lift: { y: { keys: [{ frame: 0, value: 0 }, { frame: 10, value: 0.2 }] } } } }) } });
    const selector = `[data-wheel="lift"] [data-test="${kind === 'master wheel' ? 'master-wheel' : 'ring'}"]`;
    const control = document.querySelector(selector)!;
    fireEvent.pointerDown(control, { pointerId: 1, button: 0, clientX: 100, clientY: 100, ctrlKey: kind === 'Ctrl ring' });
    edit.update({ frame: 11, timelineFrame: 11 });
    // Ctrl is latched at down in the existing ring. Shift can still change during a drag.
    fireEvent.pointerMove(control, { pointerId: 1, clientX: 140, clientY: 60, ctrlKey: kind !== 'Ctrl ring', shiftKey: true });
    fireEvent.pointerUp(control, { pointerId: 1, clientX: 140, clientY: 60, ctrlKey: kind !== 'Ctrl ring', shiftKey: true });
    if (kind === 'RGB ring') {
      expect(edit.writes()).toHaveLength(1);
      const lift = ((edit.writes()[0].ops as { params: Corrector }[])[0].params).primaries.lift;
      expect(Object.keys(lift).sort()).toEqual(['b', 'g', 'r']);
    } else expect(edit.writes()).toHaveLength(0);
  });

  it('keeps a Ctrl ring in master mode after Ctrl is released during its move', () => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true });
    const ring = document.querySelector('[data-wheel="lift"] [data-test="ring"]')!;
    fireEvent.pointerDown(ring, { pointerId: 1, button: 0, clientX: 100, clientY: 100, ctrlKey: true });
    edit.update({ frame: 6, timelineFrame: 6 });
    fireEvent.pointerMove(ring, { pointerId: 1, clientX: 140, clientY: 60, ctrlKey: false, shiftKey: true });
    expect(edit.writes()).toHaveLength(1);
    const lift = ((edit.writes()[0].ops as { params: Corrector }[])[0].params).primaries.lift;
    expect(Object.keys(lift).sort()).toEqual(['b', 'g', 'r', 'y']);
    expect(new Set(Object.values(lift)).size).toBe(1);
  });

  it.each(['ring', 'master-wheel'] as const)('does not revive a %s down when playback crosses its key and returns without pointer events', kind => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { lift: { r: { keys: [{ frame: 0, value: 0 }, { frame: 10, value: 0.2 }] } } } }) } });
    const control = document.querySelector(`[data-wheel="lift"] [data-test="${kind}"]`)!;
    pointer(control, 'pointerDown', 100, 100);
    edit.update({ frame: 11, timelineFrame: 11 });
    edit.update({ frame: 5, timelineFrame: 5 });
    pointer(control, 'pointerMove', 140, 100); pointer(control, 'pointerUp', 140, 100);
    expect(edit.writes()).toHaveLength(0);
    // A fresh down on the same control is an independent interaction and is still accepted.
    pointer(control, 'pointerDown', 100, 100); pointer(control, 'pointerMove', 140, 100);
    expect(edit.writes()).toHaveLength(1);
  });

  it('discards a no-op Shift ring click across a key while keeping a fresh Shift click valid', async () => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { lift: { r: { keys: [{ frame: 0, value: 0 }, { frame: 10, value: 0.2 }] } } } }) } });
    const ring = document.querySelector('[data-wheel="lift"] [data-test="ring"]')!;
    fireEvent.pointerDown(ring, { pointerId: 1, button: 0, clientX: 100, clientY: 100, shiftKey: true });
    expect(edit.writes()).toHaveLength(0);
    edit.update({ frame: 11, timelineFrame: 11 });
    fireEvent.pointerUp(ring, { pointerId: 1, clientX: 100, clientY: 100, shiftKey: true });
    expect(edit.writes()).toHaveLength(0);
    fireEvent.pointerDown(ring, { pointerId: 1, button: 0, clientX: 140, clientY: 100, shiftKey: true });
    fireEvent.pointerUp(ring, { pointerId: 1, clientX: 140, clientY: 100, shiftKey: true });
    expect(edit.writes()).toHaveLength(1);
    await edit.answer(0, { revision: 11 });
    expect(edit.writes()).toHaveLength(2);
    expect(edit.writes()[1].gesture).toMatchObject({ ...(edit.writes()[0].gesture as object), end: true });
  });

  it.each([['lift', 'wheels', 0], ['offset', 'bars', 25]] as const)
  ('clears the %s %s master start after an invalidated no-op vertical move', (wheel, mode, neutral) => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: {
        [wheel]: { r: { keys: [{ frame: 0, value: neutral }, { frame: 10, value: neutral + 0.2 }] } },
      } }) } });
    if (mode === 'bars') fireEvent.click(screen.getByRole('radio', { name: 'Color Bars' }));
    const master = document.querySelector(`[data-wheel="${wheel}"] [data-test="master-wheel"]`)!;
    pointer(master, 'pointerDown', 100, 100); pointer(master, 'pointerMove', 100, 140);
    expect(edit.writes()).toHaveLength(0);
    edit.update({ frame: 11, timelineFrame: 11 });
    pointer(master, 'pointerDown', 100, 100); pointer(master, 'pointerMove', 140, 100); pointer(master, 'pointerUp', 140, 100);
    expect(edit.writes()).toHaveLength(1);
    const params = ((edit.writes()[0].ops as { params: Corrector }[])[0].params).primaries[wheel];
    expect(params.r).toMatchObject({ keys: [{ frame: 0, value: neutral }, { frame: 10, value: expect.any(Number) }] });
    expect((params.r as { keys: { value: number }[] }).keys[1].value - (params.g as number)).toBeCloseTo(0.2);
  });

  it.each([false, true])('establishes Escape restore state after a slow preceding ordinary edit (virtual=%s)', async virtual => {
    vi.useFakeTimers();
    const target = { level: 'clip', timeline: 't1', clip: 'c1' } as const;
    const edit = mount({ target, before: virtual ? [makeDefaultNode(target)] : undefined });
    fireEvent.doubleClick(field()); fireEvent.change(field(), { target: { value: '60' } }); fireEvent.keyDown(field(), { key: 'Enter' });
    await act(async () => vi.advanceTimersByTime(HOLD_MS + 1));
    expect(field().value).toBe('50.00');
    pointer(field(), 'pointerDown'); pointer(field(), 'pointerMove', 80);
    expect(field().value).toBe('70.00');
    expect(edit.writes()).toHaveLength(1);
    expect(edit.writes()[0].ops).toHaveLength(virtual ? 2 : 1);
    await edit.answer(0, { revision: 11 });
    // The preceding transaction made the virtual node, if there was one. It owns that creation.
    edit.update({ before: undefined }); edit.sync(11, { primaries: { saturation: 60 } });
    expect(edit.writes()[1]).toMatchObject({ revision: 11, ops: [{ op: 'set_node', params: { primaries: { saturation: 70 } } }] });
    expect(edit.writes()[1].ops).toHaveLength(1);
    fireEvent.keyDown(window, { key: 'Escape' });
    await edit.answer(0, { revision: 12 });
    expect(edit.writes()).toHaveLength(3);
    expect(edit.writes()[2]).toMatchObject({ revision: 11, gesture: { ...(edit.writes()[1].gesture as object), end: true },
      ops: [{ op: 'set_node', params: { primaries: { saturation: 60 } } }] });
    expect(edit.writes()[2].ops).toHaveLength(1);
  });
});

describe('Auto Color request ownership', () => {
  it.each(['clip', 'clock', 'session', 'frame', 'revision', 'local'] as const)('ignores an old result after %s changed', async kind => {
    const edit = mount(); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    if (kind === 'local') { fireEvent.doubleClick(field()); fireEvent.change(field(), { target: { value: '80' } }); fireEvent.keyDown(field(), { key: 'Enter' }); }
    else edit.update(kind === 'clip' ? { clip: 'c2' } : kind === 'clock' ? { clock: 'slip' } : kind === 'session' ? { session: 2 } : kind === 'frame' ? { timelineFrame: 1, frame: 101 } : { revision: 11 });
    const before = edit.writes().length; await edit.answer();
    expect(edit.writes()).toHaveLength(before);
    expect(edit.call.mock.calls[0][2]).toEqual({ expectedSession: 1 });
  });

  it.each(['flat', 'failure'] as const)('does not deliver a stale %s to the new target', async kind => {
    const edit = mount(); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' })); edit.update({ clip: 'c2' });
    await edit.answer(0, { ...STATS, percentiles: { ...STATS.percentiles, blue: { 1: 0.4, 99: 0.4 } } }, kind === 'failure');
    expect(screen.queryByRole('status')).toBeNull(); expect(edit.context().onFailure).not.toHaveBeenCalled();
  });

  it('consumes only the newest statistics request, with timeline rather than source-key frame', async () => {
    const edit = mount(); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' })); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    await edit.answer(1); expect(edit.writes()).toHaveLength(1);
    expect(edit.writes()[0].revision).toBe(10);
    await edit.answer(0); expect(edit.writes()).toHaveLength(1);
  });

  it('does not start statistics while a local edit is pending', async () => {
    const edit = mount(); pointer(field(), 'pointerDown'); pointer(field(), 'pointerMove', 40);
    fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    expect(edit.call.mock.calls.filter(([method]) => method === 'scopes.statistics')).toHaveLength(0);
  });

  it.each(['draft', 'down before preview', 'curve down'] as const)('invalidates statistics when a local %s starts before any transaction', async kind => {
    const edit = mount(); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    if (kind === 'draft') { fireEvent.doubleClick(field()); fireEvent.change(field(), { target: { value: '80' } }); }
    else if (kind === 'curve down') pointer(document.querySelector('[data-test="curve-editor"]')!, 'pointerDown', 0, 200);
    else pointer(field(), 'pointerDown');
    expect(edit.writes()).toHaveLength(0);
    await edit.answer();
    expect(edit.writes()).toHaveLength(0);
  });

  it('does not revive a measurement after playback leaves and returns to its timeline frame', async () => {
    const edit = mount({ playing: true }); fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    edit.update({ timelineFrame: 1, frame: 101 }); edit.update({ timelineFrame: 0, frame: 100 });
    await edit.answer(); expect(edit.writes()).toHaveLength(0);
  });
});
