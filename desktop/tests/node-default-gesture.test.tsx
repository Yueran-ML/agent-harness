// @vitest-environment jsdom
/**
 * A palette control dragged on the default node 01 of a grade without nodes (T3d): the gesture's
 * first transaction makes the node (`add_serial` after the source) and sets it, the others set it
 * by the id it was made with, and the last ends the gesture, so the engine keeps the drag as one
 * step to undo; Escape takes the node away again. An edit made at once makes it in its transaction.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useNodeEdit, type EditedNode } from '../src/studio/color/useNodeEdit';
import { makeDefaultNode, newCorrectorNode } from '../src/studio/color/nodeEdits';
import type { Target } from '../src/studio/engine';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c3' };
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
type Sent = { revision: number; ops: Record<string, unknown>[]; gesture?: { id: string; end?: boolean } };

/**
 * The hook over the default node n10 of a grade without nodes, with an engine that answers at once
 * and keeps its history as the engine does (PROTOCOL.md, "Gestures"): transactions of one gesture
 * are one step, ended by the one that says `end`; a transaction without a gesture is a step alone.
 */
function mount() {
  const steps: Sent[][] = [];
  let open: string | null = null;
  let revision = 4;
  const bridge = {
    call: vi.fn(async (method: string, sent?: Record<string, unknown>) => {
      if (method !== 'transaction.apply') return { ok: true, result: {} };
      const each = sent as unknown as Sent;
      if (each.gesture && each.gesture.id === open) steps.at(-1)!.push(each);
      else steps.push([each]);
      open = each.gesture && !each.gesture.end ? each.gesture.id : null;
      revision += 1;
      return { ok: true, result: { revision, done: [], next_id: 11, names: {}, clamped: [] } };
    }),
  };
  const edited: EditedNode = {
    bridge: bridge as unknown as EditedNode['bridge'], revision: 4, target: TARGET, node: newCorrectorNode('n10'), frame: 0, onFailure: vi.fn(),
    before: [makeDefaultNode(TARGET)],
  };
  const hook = renderHook(({ at }) => useNodeEdit(at), { initialProps: { at: edited } });
  return { hook, steps, sent: () => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, each]) => each as unknown as Sent) };
}

const saturation = (value: number) => [['primaries.saturation', value]] as const;
const setTo = (value: number) => ({ op: 'set_node', grade: TARGET, id: 'n10', params: { primaries: { saturation: value } } });

describe('a drag of a palette control on the default node 01', () => {
  it('makes the node in the first transaction of its gesture only, and is one step to undo', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(55))); await settle();
    act(() => edit.hook.result.current.preview(saturation(58))); await settle();
    act(() => edit.hook.result.current.preview(saturation(61))); await settle();
    act(() => edit.hook.result.current.commit(saturation(62))); await settle();
    const sent = edit.sent();
    expect(sent.map(each => each.ops)).toEqual([
      [{ edit: 'add_serial', grade: TARGET, node: 'source' }, setTo(55)],
      [setTo(58)], [setTo(61)], [setTo(62)],
    ]);
    expect(new Set(sent.map(each => each.gesture!.id)).size).toBe(1);
    expect(sent.at(-1)!.gesture!.end).toBe(true);
    // One step to undo: the node made and set.
    expect(edit.steps).toHaveLength(1);
    expect(edit.steps[0]).toHaveLength(4);
  });

  it('takes the node away again where it is cancelled, so the drag leaves the grade as it was', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(70))); await settle();
    act(() => edit.hook.result.current.cancel()); await settle();
    expect(edit.sent().map(each => each.ops)).toEqual([
      [{ edit: 'add_serial', grade: TARGET, node: 'source' }, setTo(70)],
      [{ op: 'remove_node', grade: TARGET, id: 'n10' }],
    ]);
    expect(edit.steps).toHaveLength(1);
  });

  it('makes the node in the transaction of an edit made at once', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.commit(saturation(40))); await settle();
    expect(edit.sent()).toEqual([{ revision: 4, ops: [{ edit: 'add_serial', grade: TARGET, node: 'source' }, setTo(40)] }]);
    expect(edit.steps).toHaveLength(1);
  });
});
