// @vitest-environment jsdom
/**
 * A palette control dragged is sent as it goes (T13e): each preview a transaction of one gesture,
 * at most one waiting for its answer and the newest edits in place of those not sent; the commit
 * the last, which ends it; Escape sends back what was there; a refusal stops the drag; and the
 * page's copy takes in the changes of a gesture told together, from their `base`.
 */
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useNodeEdit, type EditedNode } from '../src/studio/color/useNodeEdit';
import { fullCorrector } from '../src/studio/color/corrector';
import { EngineError, ProjectCopy, mergePatch, viewPointerOf, type Corrector, type EngineEvent, type GradeNode, type Target } from '../src/studio/engine';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });

type Sent = { revision: number; ops: { op: string; params: Record<string, unknown> }[]; gesture?: { id: string; end?: boolean } };

/** The hook over a corrector, with an engine whose answers wait until they are let go. */
function mount() {
  const node: GradeNode = { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({}) as Corrector };
  let revision = 10;
  const waiting: { sent: Sent; answer: (failure?: unknown) => void }[] = [];
  const bridge = {
    call: vi.fn((method: string, sent?: Record<string, unknown>) => new Promise<unknown>((resolve) => {
      if (method !== 'transaction.apply') { resolve({ ok: true, result: {} }); return; }
      waiting.push({
        sent: sent as unknown as Sent,
        answer: (failure) => {
          if (failure) { resolve({ ok: false, error: failure }); return; }
          revision += 1;
          resolve({ ok: true, result: { revision, done: [], next_id: 9, names: {}, clamped: [] } });
        },
      });
    })),
  };
  const onFailure = vi.fn();
  const edited: EditedNode = { bridge: bridge as unknown as EditedNode['bridge'], revision: 10, target: TARGET, node, frame: 0, onFailure };
  const hook = renderHook(({ at }) => useNodeEdit(at), { initialProps: { at: edited } });
  return {
    hook, onFailure, waiting,
    /** The copy takes in what the engine did. */
    sync: async () => { hook.rerender({ at: { ...edited, revision } }); await settle(); },
    sent: () => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, each]) => each as unknown as Sent),
    /** Answer the transaction that waits longest. */
    answer: async (failure?: unknown) => { waiting.shift()!.answer(failure); await settle(); },
  };
}

const saturation = (value: number) => [['primaries.saturation', value]] as const;

describe('a drag of a palette control', () => {
  it('is sent as it goes, one at a time, the newest in place of those not sent, and ended by the commit', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(60)));
    // Shown at once, and sent at once, as the first of a gesture, bound to the copy's revision.
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(60);
    expect(edit.sent()).toHaveLength(1);
    const [first] = edit.sent();
    expect(first.revision).toBe(10);
    expect(first.ops).toEqual([{ op: 'set_node', grade: TARGET, id: 'n1', params: { primaries: { saturation: 60 } } }]);
    expect(first.gesture).toEqual({ id: expect.stringMatching(/^palette-\d+$/) });
    // While it waits, three more previews: only the newest is sent, once it is answered.
    act(() => { for (const value of [61, 62, 63]) edit.hook.result.current.preview(saturation(value)); });
    expect(edit.sent()).toHaveLength(1);
    await edit.answer();
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1].ops[0].params).toEqual({ primaries: { saturation: 63 } });
    expect(edit.sent()[1].gesture).toEqual({ id: first.gesture!.id });
    // Let go while that waits: the last is sent once it is answered, and ends the gesture.
    act(() => edit.hook.result.current.commit(saturation(64)));
    expect(edit.sent()).toHaveLength(2);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(64);
    await edit.answer();
    expect(edit.sent()).toHaveLength(3);
    expect(edit.sent()[2].ops[0].params).toEqual({ primaries: { saturation: 64 } });
    expect(edit.sent()[2].gesture).toEqual({ id: first.gesture!.id, end: true });
    await edit.answer();
    // The next drag is another gesture, sent once the copy has taken in the one before.
    act(() => edit.hook.result.current.preview(saturation(10)));
    await settle();
    expect(edit.sent()).toHaveLength(3);
    await edit.sync();
    await settle();
    const next = edit.sent().at(-1)!;
    expect(next.gesture!.id).not.toBe(first.gesture!.id);
    expect(next.gesture!.end).toBeUndefined();
  });

  it('sends the last even where it changes nothing more, and an edit made at once as a step alone', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(70)));
    await edit.answer();
    act(() => edit.hook.result.current.commit(saturation(70)));
    await settle();
    expect(edit.sent().map(sent => sent.gesture?.end)).toEqual([undefined, true]);
    await edit.answer();
    await edit.sync();
    // A reset or a click: no gesture.
    act(() => edit.hook.result.current.commit(saturation(30)));
    await settle();
    expect(edit.sent()).toHaveLength(3);
    expect(edit.sent()[2].gesture).toBeUndefined();
  });

  it('puts back what was there with Escape, as the last of the gesture, which then leaves no step', async () => {
    const edit = mount();
    act(() => {
      edit.hook.result.current.preview(saturation(80));
    });
    await edit.answer();
    act(() => edit.hook.result.current.preview([['primaries.saturation', 90], ['primaries.hue', 60]]));
    await edit.answer();
    act(() => edit.hook.result.current.cancel());
    await settle();
    const last = edit.sent().at(-1)!;
    expect(last.gesture!.end).toBe(true);
    expect(last.ops[0].params).toEqual({ primaries: { saturation: 50, hue: 50 } });
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(50);
    // Escape before anything was sent sends nothing.
    const edit2 = mount();
    act(() => edit2.hook.result.current.cancel());
    await settle();
    expect(edit2.sent()).toHaveLength(0);
  });

  it('stops sending once the engine refuses one, and says why once', async () => {
    const edit = mount();
    act(() => edit.hook.result.current.preview(saturation(60)));
    act(() => edit.hook.result.current.preview(saturation(61)));
    await edit.answer({ code: -32001, message: 'the project changed since this request was prepared', data: { reason: 'stale_revision' } });
    expect(edit.onFailure).toHaveBeenCalledOnce();
    expect(edit.onFailure.mock.calls[0][0]).toBeInstanceOf(EngineError);
    act(() => edit.hook.result.current.preview(saturation(62)));
    act(() => edit.hook.result.current.commit(saturation(63)));
    await settle();
    expect(edit.sent()).toHaveLength(1);
    // The next drag begins anew.
    act(() => edit.hook.result.current.preview(saturation(20)));
    await settle();
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1].gesture!.id).not.toBe(edit.sent()[0].gesture!.id);
    // A drag that changes nothing sends nothing, and its end is an edit made at once: none.
    const still = mount();
    act(() => still.hook.result.current.preview(saturation(50)));
    act(() => still.hook.result.current.commit(saturation(50)));
    await settle();
    expect(still.sent()).toHaveLength(0);
  });
});

describe('the copy of the project', () => {
  it('takes in the changes of a gesture told together from their base, and fetches again on a gap', async () => {
    let listener: (event: EngineEvent) => void = () => {};
    const node = { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector({}) };
    const project = {
      version: 3, name: 'p', media: [], bins: [], groups: [], next_id: 5,
      timelines: [{ id: 't1', name: 'T', width: 64, height: 36, rate: { num: 25, den: 1 }, sample_rate: 48000,
        tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [],
          clips: [{ id: 'c1', start: 0, length: 10, source: { kind: 'color', color: '#000000ff' }, enabled: true, speed: { num: 1, den: 1 }, reverse: false,
            link: null, transform: { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 }, gain_db: 0, pan: 0, fade_in: 0, fade_out: 0,
            grade: { nodes: [node], links: [] }, group: null }] }] }],
    };
    const call = vi.fn(async () => ({ ok: true as const, result: { revision: 4, next_id: 5, project } }));
    const copy = new ProjectCopy({ call, onEvent: (heard: (event: EngineEvent) => void) => { listener = heard; return () => {}; } } as never);
    listener({ kind: 'opened', session: 1, project: 'p', owned: true, pid: 1, revision: 4 });
    await settle();
    expect(copy.getSnapshot().revision).toBe(4);
    const set = (value: number) => ({ op: 'set_node', grade: TARGET, id: 'n1', params: { primaries: { saturation: value } } });
    const told = (params: Record<string, unknown>) => listener({ kind: 'notification', session: 1, method: 'project.changed', params });
    told({ revision: 7, base: 4, by: 'desktop', made_by: 'desktop', done: [set(10), set(20), set(30)], next_id: 5 });
    expect(copy.getSnapshot().revision).toBe(7);
    const saturationNow = () => {
      const track = copy.getSnapshot().project!.timelines[0].tracks[0] as unknown as { clips: { grade: { nodes: { params: unknown }[] } }[] };
      return (mergePatch({}, track.clips[0].grade.nodes[0].params) as Corrector).primaries.saturation;
    };
    expect(saturationNow()).toBe(30);
    told({ revision: 8, by: 'desktop', made_by: 'desktop', done: [set(40)], next_id: 5 });
    expect(saturationNow()).toBe(40);
    // One from another base: fetched again.
    expect(call).toHaveBeenCalledTimes(1);
    told({ revision: 12, base: 10, by: 'desktop', made_by: 'desktop', done: [set(50)], next_id: 5 });
    await settle();
    expect(call).toHaveBeenCalledTimes(2);
  });

  it('reads what the pointer did on a view from the events of the engine', () => {
    const pointer = { monitor: 'timeline', kind: 'down', button: 'left', buttons: ['left'], x: 10.5, y: 4.5, inside: true, window: { x: 21, y: 9 }, shift: false, ctrl: false, alt: true };
    expect(viewPointerOf({ kind: 'notification', session: 1, method: 'view.pointer', params: pointer })).toEqual(pointer);
    expect(viewPointerOf({ kind: 'notification', session: 1, method: 'project.changed', params: {} })).toBeNull();
    expect(viewPointerOf({ kind: 'exited', session: 1, expected: true, code: 0, signal: null })).toBeNull();
  });
});
