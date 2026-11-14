// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useNodeEdit, type EditedNode } from '../src/studio/color/useNodeEdit';
import { fullCorrector } from '../src/studio/color/corrector';
import { mergePatch, type Corrector } from '../src/studio/engine';

afterEach(() => { cleanup(); vi.useRealTimers(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
const saturation = (value: number) => [['primaries.saturation', value]] as const;
type Sent = { revision: number; ops: { op: string; grade: unknown; id: string; params: unknown }[]; gesture?: { id: string; end?: boolean } };
function mount(extra: Partial<EditedNode> = {}) {
  const pending: ((value: unknown) => void)[] = [];
  const call = vi.fn((_method: string, _params?: Record<string, unknown>, _context?: unknown) => new Promise<any>(resolve => pending.push(resolve)));
  let context: EditedNode = { bridge: { call }, session: 1, revision: 10,
    target: { level: 'clip', timeline: 't1', clip: 'c1' }, clip: 'c1', clock: 'source-clock', timelineFrame: 0, playing: false,
    node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({}) }, frame: 0, onFailure: vi.fn(), ...extra };
  const hook = renderHook(({ at }) => useNodeEdit(at), { initialProps: { at: context } });
  const update = (change: Partial<EditedNode>) => { context = { ...context, ...change }; hook.rerender({ at: context }); };
  return { hook, call, pending, update, context: () => context,
    sent: () => call.mock.calls.map(([, params]) => params as unknown as Sent),
    answer: async (revision: number, error?: unknown) => { await act(async () => pending.shift()!(error ? { ok: false, error } : { ok: true, result: { revision } })); },
    sync: (revision: number, patch: unknown = {}) => update({ revision, node: { ...context.node, params: mergePatch(context.node.params, patch) as Corrector } as EditedNode['node'] }),
  };
}

describe('palette operation ownership', () => {
  const changes: [string, Partial<EditedNode>][] = [
    ['node', { node: { id: 'n2', kind: 'corrector', label: '', enabled: true, params: fullCorrector({}) } }],
    ['session', { session: 2 }], ['group clip', { clip: 'c2' }], ['source clock', { clock: 'slipped-clock' }],
    ['paused seek', { frame: 4, timelineFrame: 4 }],
  ];
  it.each(changes)('drops queued ordinary edits on a changed %s and ignores the old promise', async (_name, change) => {
    const edit = mount({ target: { level: 'pre_clip', group: 'g1' } });
    const old = edit.hook.result.current;
    act(() => { old.commit(saturation(60)); old.commit([['primaries.hue', 70]]); });
    edit.update(change);
    act(() => edit.hook.result.current.commit(saturation(80)));
    await edit.answer(11);
    await settle();
    expect(edit.sent()).toHaveLength(2);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(80);
    expect(edit.call.mock.calls[0][2]).toEqual({ expectedSession: 1 });
  });

  it.each(changes)('closes only the last successful old-gesture value on changed %s', async (_name, change) => {
    const edit = mount({ target: { level: 'pre_clip', group: 'g1' } });
    const old = edit.hook.result.current;
    act(() => { old.preview(saturation(60)); old.preview(saturation(90)); });
    edit.update(change);
    await edit.answer(11);
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1]).toMatchObject({ revision: 10, gesture: { ...edit.sent()[0].gesture, end: true },
      ops: [{ grade: { level: 'pre_clip', group: 'g1' }, id: 'n1', params: { primaries: { saturation: 60 } } }] });
    act(() => { old.preview(saturation(95)); old.commit(saturation(99)); old.cancel(); });
    await edit.answer(12);
    expect(edit.sent()).toHaveLength(2);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(50);
  });

  it.each(['before response', 'after response'] as const)('waits for its own copy %s, preserving earlier ordinary fields', async order => {
    const edit = mount();
    act(() => { edit.hook.result.current.commit([['primaries.lift.r', 0.2]]); edit.hook.result.current.commit([['primaries.lift.g', 0.3]]); });
    if (order === 'before response') edit.sync(11, { primaries: { lift: { r: 0.2 } } });
    await edit.answer(11);
    if (order === 'after response') {
      vi.useFakeTimers(); await act(async () => vi.advanceTimersByTime(5000));
      expect(edit.sent()).toHaveLength(1);
      edit.sync(11, { primaries: { lift: { r: 0.2 } } });
    }
    await settle();
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1].revision).toBe(11);
    // Applying only the second field keeps the first field in the node.
    expect(mergePatch(edit.context().node.params, edit.sent()[1].ops[0].params)).toMatchObject({ primaries: { lift: { r: 0.2, g: 0.3 } } });
  });

  it('keeps one gesture base through early, late and coalesced own notifications', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(60)));
    edit.sync(11, { primaries: { saturation: 60 } });
    await edit.answer(11);
    act(() => edit.hook.result.current.preview(saturation(70)));
    await edit.answer(12);
    act(() => edit.hook.result.current.preview(saturation(80)));
    edit.sync(13, { primaries: { saturation: 80 } });
    await edit.answer(13);
    act(() => edit.hook.result.current.commit(saturation(90)));
    expect(edit.sent().map(each => each.revision)).toEqual([10, 10, 10, 10]);
    expect(new Set(edit.sent().map(each => each.gesture!.id)).size).toBe(1);
  });

  it.each(['ordinary', 'gesture'] as const)('discards pending %s continuation when the copy exceeds its response', async kind => {
    const edit = mount();
    const old = edit.hook.result.current;
    act(() => { old[kind === 'ordinary' ? 'commit' : 'preview'](saturation(60)); old[kind === 'ordinary' ? 'commit' : 'preview'](saturation(80)); });
    edit.sync(12, { primaries: { saturation: 30 } });
    await edit.answer(11);
    act(() => { old.preview(saturation(90)); old.commit(saturation(95)); old.cancel(); });
    expect(edit.sent()).toHaveLength(1);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(30);
  });

  it('does not promote a gesture base after idle expiry, nor restore after its refusal', async () => {
    vi.useFakeTimers();
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(60)));
    await edit.answer(11); edit.sync(11, { primaries: { saturation: 60 } });
    await act(async () => vi.advanceTimersByTime(10_001));
    act(() => edit.hook.result.current.preview(saturation(70)));
    expect(edit.sent()[1].revision).toBe(10);
    await edit.answer(11, { code: -32001, message: 'stale', data: { reason: 'stale_revision' } });
    act(() => { edit.hook.result.current.cancel(); });
    expect(edit.sent()).toHaveLength(2);
  });

  it('invalidates an acknowledged drag when another transaction, undo/redo or gesture advances the copy', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(60))); await edit.answer(11);
    edit.sync(11, { primaries: { saturation: 60 } });
    const old = edit.hook.result.current;
    edit.sync(12, { primaries: { saturation: 35 } });
    act(() => { old.preview(saturation(90)); old.commit(saturation(95)); old.cancel(); });
    expect(edit.sent()).toHaveLength(1);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(35);
  });

  it('drops a waiting gesture if the preceding ordinary edit is refused', async () => {
    const edit = mount();
    act(() => { edit.hook.result.current.commit(saturation(60)); edit.hook.result.current.preview([['primaries.hue', 70]]); });
    await edit.answer(10, { code: -32001, message: 'locked node', data: { reason: 'locked' } });
    expect(edit.sent()).toHaveLength(1);
    expect(edit.hook.result.current.value('primaries.hue')).toBe(50);
  });

  it('does not report an old failure or clear the new owner values', async () => {
    const edit = mount();
    const oldFailure = edit.context().onFailure;
    act(() => edit.hook.result.current.commit(saturation(60)));
    const newFailure = vi.fn(); edit.update({ clip: 'c2', onFailure: newFailure });
    act(() => edit.hook.result.current.commit(saturation(80)));
    await edit.answer(10, { code: -1, message: 'old failure', data: null });
    expect(oldFailure).not.toHaveBeenCalled(); expect(newFailure).not.toHaveBeenCalled();
    expect(edit.sent()).toHaveLength(2); expect(edit.hook.result.current.value('primaries.saturation')).toBe(80);
  });

  it.each([0, 15, 40])('preserves complete keyed Params and restores their handles on Escape at %s', async frame => {
    const param = { keys: [{ frame: 10, value: 30, interp: 'bezier', incoming: [1, 2], outgoing: [3, 4] }, { frame: 20, value: 70, interp: 'linear' }] };
    const edit = mount({ frame, node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { saturation: param } }) } });
    act(() => edit.hook.result.current.preview(saturation(60))); await edit.answer(11);
    edit.sync(11, edit.sent()[0].ops[0].params);
    act(() => edit.hook.result.current.cancel());
    expect(edit.sent()[1]).toMatchObject({ revision: 10, ops: [{ params: { primaries: { saturation: param } } }] });
  });

  it('stops queued keyed work at the next key during playback, but preserves static playback edits', async () => {
    const edit = mount({ frame: 5, timelineFrame: 5, playing: true,
      node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({ primaries: { saturation: { keys: [{ frame: 0, value: 30 }, { frame: 10, value: 70 }] } } }) } });
    act(() => { edit.hook.result.current.preview(saturation(40)); edit.hook.result.current.preview(saturation(50)); });
    edit.update({ frame: 11, timelineFrame: 11 }); await edit.answer(11);
    expect(edit.sent()[1].ops[0].params).toMatchObject({ primaries: { saturation: { keys: [{ frame: 0, value: 40 }, { frame: 10, value: 70 }] } } });
    await edit.answer(12); edit.sync(12, edit.sent()[1].ops[0].params);
    act(() => edit.hook.result.current.preview([['primaries.hue', 60]]));
    edit.update({ frame: 12, timelineFrame: 12 }); await edit.answer(13);
    act(() => edit.hook.result.current.commit([['primaries.hue', 65]]));
    expect(edit.sent().at(-1)).toMatchObject({ revision: 12, gesture: { end: true }, ops: [{ params: { primaries: { hue: 65 } } }] });
  });

  it.each([false, true])('never sends or reports from an old unmounted completion (failure=%s)', async failure => {
    const edit = mount();
    act(() => { edit.hook.result.current.preview(saturation(60)); edit.hook.result.current.commit(saturation(70)); });
    edit.hook.unmount();
    await edit.answer(11, failure ? { code: -1, message: 'gone', data: null } : undefined);
    expect(edit.sent()).toHaveLength(1);
    expect(edit.context().onFailure).not.toHaveBeenCalled();
  });
});
