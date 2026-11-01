// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { circularWindow, useWindows } from '../src/studio/color/WindowPalette';
import { newCorrectorNode } from '../src/studio/color/nodeEdits';
import type { Applied, GradeNode, PowerWindow } from '../src/studio/engine';

afterEach(cleanup);
const node = (windows: PowerWindow[], id = 'n1'): GradeNode => ({ ...newCorrectorNode(id), params: { ...newCorrectorNode(id).params, windows } } as GradeNode);
const circle = circularWindow('w2');
const other: PowerWindow = { ...circularWindow('w1'), shape: { kind: 'gradient', softness: { keys: [{ frame: 9, value: 7, interp: 'linear' }] } } };
function mount(virtual = false) {
  const run = vi.fn();
  let props: Parameters<typeof useWindows>[0] = { session: 1, revision: 10, timeline: 't1', target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', node: node(virtual ? [] : [other, circle]), realNode: virtual ? null : node([other, circle]), complete: true,
    editing: { run, step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame: 8, clock: 'source' };
  const hook = renderHook(at => useWindows(at), { initialProps: props });
  const update = (changes: Partial<typeof props>) => { props = { ...props, ...changes }; hook.rerender(props); };
  const ack = (revision = 11) => act(() => run.mock.calls[0][1]({ revision } as Applied));
  const copy = (windows: PowerWindow[], revision = 11) => update({ revision, node: node(windows), realNode: node(windows) });
  return { hook, run, update, ack, copy, props: () => props, create: () => act(() => hook.result.current.create(() => false)) };
}

describe('Window creation and selection intent', () => {
  it('creates a Gradient through the same acknowledgment and real-copy gate', () => {
    const h = mount(true);
    act(() => (h.hook.result.current.create as (busy: () => boolean, kind: string) => void)(() => false, 'gradient'));
    const created = h.run.mock.calls[0][0][0].params.windows as PowerWindow[];
    expect(created[0].shape.kind).toBe('gradient');
    h.ack();
    expect(h.hook.result.current.selection).toBeNull();
    h.copy(created);
    expect(h.hook.result.current.selected?.shape.kind).toBe('gradient');
  });

  it('keeps the creation gate when its new row is chosen before acknowledgment', () => {
    const h = mount(); h.create();
    const created = h.run.mock.calls[0][0][0].params.windows as PowerWindow[];
    h.copy(created);
    act(() => h.hook.result.current.choose('w3'));
    expect(h.hook.result.current.pending).toBe(true);
    expect(h.hook.result.current.selection?.selected).not.toBe('w3');
    h.ack();
    expect(h.hook.result.current.selection?.selected).toBe('w3');
  });

  it.each(['copy first', 'reply first'] as const)('keeps confirmation separate from changed row and OSC intent: %s', order => {
    const h = mount(); h.create();
    const created = h.run.mock.calls[0][0][0].params.windows as PowerWindow[];
    act(() => h.hook.result.current.choose('w2'));
    h.update({ tool: 'none' }); h.update({ tool: 'window' });
    if (order === 'copy first') h.copy(created); else h.ack();
    act(() => h.hook.result.current.choose('w3'));
    expect(h.hook.result.current.pending).toBe(true);
    expect(h.hook.result.current.selection?.selected).toBe('w2');
    if (order === 'copy first') h.ack(); else h.copy(created);
    expect(h.hook.result.current.pending).toBe(false);
    expect(h.hook.result.current.selection?.selected).toBe('w2');
    act(() => h.hook.result.current.choose('w3'));
    expect(h.hook.result.current.selection?.selected).toBe('w3');
  });

  it('retains an unconfirmed group window gate while another clip shows the same group', () => {
    const h = mount();
    h.update({ target: { level: 'pre_clip', group: 'g1' } }); h.create();
    h.copy(h.run.mock.calls[0][0][0].params.windows);
    h.update({ clip: 'c2' });
    act(() => h.hook.result.current.choose('w3'));
    expect(h.hook.result.current.pending).toBe(true);
    expect(h.hook.result.current.selection?.selected).toBe('w2');
    h.ack();
    expect(h.hook.result.current.selection?.selected).toBe('w2');
  });

  it('does not default to a late first window after OSC intent changes', () => {
    const h = mount(true); h.create();
    h.update({ tool: 'none' }); h.update({ tool: 'window' });
    h.copy([circularWindow('w1')]);
    act(() => h.hook.result.current.choose('w1'));
    expect(h.hook.result.current.selection).toBeNull();
    h.ack();
    expect(h.hook.result.current.pending).toBe(false);
    expect(h.hook.result.current.selection).toBeNull();
    act(() => h.hook.result.current.choose('w1'));
    expect(h.hook.result.current.selection?.selected).toBe('w1');
  });

  it('keeps an empty first-window selection after visiting another target with an existing selected row', () => {
    const h = mount(true); h.create(); h.copy([circularWindow('w1')]);
    const original = h.props();
    h.update({ node: node([circle], 'n9'), realNode: node([circle], 'n9') });
    expect(h.hook.result.current.selected?.id).toBe('w2');
    h.update(original);
    expect(h.hook.result.current.pending).toBe(true);
    expect(h.hook.result.current.selection).toBeNull();
    h.ack();
    expect(h.hook.result.current.pending).toBe(false);
    expect(h.hook.result.current.selected).toBeNull();
    expect(h.hook.result.current.selection).toBeNull();
    act(() => h.hook.result.current.choose('w1'));
    expect(h.hook.result.current.selection?.selected).toBe('w1');
  });

  it('retains the original gate through a target round trip before its late acknowledgment', () => {
    const h = mount(); h.create();
    h.copy(h.run.mock.calls[0][0][0].params.windows);
    const original = h.props();
    h.update({ node: node([circle], 'n9'), realNode: node([circle], 'n9') });
    h.update(original);
    act(() => h.hook.result.current.choose('w3'));
    expect(h.hook.result.current.pending).toBe(true);
    expect(h.hook.result.current.selection?.selected).toBe('w2');
    h.ack();
    expect(h.hook.result.current.selection?.selected).toBe('w2');
  });

  it('allows creation in a disconnected grade while deferring only its native binding', () => {
    const h = mount(); h.update({ complete: false });
    expect(h.hook.result.current.enabled).toBe(true);
    h.create();
    expect(h.run).toHaveBeenCalledTimes(1);
    h.copy(h.run.mock.calls[0][0][0].params.windows); h.ack();
    expect(h.hook.result.current.selected?.id).toBe('w3');
    expect(h.hook.result.current.selection).toBeNull();
    h.update({ complete: true });
    expect(h.hook.result.current.selection?.selected).toBe('w3');
  });

  it.each(['copy first', 'reply first'])('selects a new local id only after acknowledgment and a real copy: %s', order => {
    const h = mount();
    act(() => { h.hook.result.current.create(() => false); h.hook.result.current.create(() => false); });
    expect(h.run).toHaveBeenCalledTimes(1);
    const windows = h.run.mock.calls[0][0][0].params.windows as PowerWindow[];
    expect(windows).toEqual([other, circle, circularWindow('w3')]);
    if (order === 'copy first') h.copy(windows); else h.ack();
    expect(h.hook.result.current.selected?.id).toBe('w2');
    expect(h.hook.result.current.pending).toBe(true);
    if (order === 'copy first') h.ack(); else h.copy(windows, 13);
    expect(h.hook.result.current.selected?.id).toBe('w3');
    expect(h.hook.result.current.pending).toBe(false);
  });

  it('does not bind an optimistic virtual default node, including an early copy before its acknowledgment', () => {
    const h = mount(true); h.create();
    expect(h.hook.result.current.selection).toBeNull();
    expect(h.run.mock.calls[0][0]).toHaveLength(1); // run/onDefaultNode supplies add_serial, never a second manual add.
    h.copy([circularWindow('w1')]);
    expect(h.hook.result.current.selection).toBeNull();
    h.ack();
    expect(h.hook.result.current.selection?.selected).toBe('w1');
  });

  it.each(['tool', 'node', 'clip', 'session', 'row'] as const)('does not let a late creation steal the new %s intent', change => {
    const h = mount(); h.create();
    const created = h.run.mock.calls[0][0][0].params.windows as PowerWindow[];
    if (change === 'tool') h.update({ tool: 'none' });
    if (change === 'node') h.update({ node: node([circle], 'n9'), realNode: node([circle], 'n9') });
    if (change === 'clip') h.update({ clip: 'c2', target: { level: 'clip', timeline: 't1', clip: 'c2' } });
    if (change === 'session') h.update({ session: 2 });
    if (change === 'row') act(() => h.hook.result.current.choose('w2'));
    h.ack();
    if (change === 'tool' || change === 'row') h.copy(created);
    if (change === 'node' || change === 'clip') h.update({ revision: 11 });
    expect(h.hook.result.current.selected?.id).toBe('w2');
    expect(h.hook.result.current.pending).toBe(false);
  });

  it('clears a creation superseded by undo/delete, and releases a selected window deleted externally', () => {
    const h = mount(); h.create(); h.ack(); h.copy([other, circle], 12);
    expect(h.hook.result.current.selected?.id).toBe('w2'); expect(h.hook.result.current.pending).toBe(false);
    h.copy([other, circularWindow('w4')], 13);
    expect(h.hook.result.current.selection).toBeNull();
  });

  it('does not create through an unfinished capture and keeps server-selected rows out of user bind intention', () => {
    const h = mount(); act(() => h.hook.result.current.create(() => true)); expect(h.run).not.toHaveBeenCalled();
    h.copy([other, circle, circularWindow('w4')]);
    const before = h.hook.result.current.intention;
    act(() => h.hook.result.current.onState({ binding: 1, capture: 1, sequence: 2, phase: 'armed', reason: null, selected: 'w4', event: 2,
      revision: 11, keys: { pan: 0, tilt: 5, clock: 'source', frame: 8 } }));
    expect(h.hook.result.current.selected?.id).toBe('w4'); expect(h.hook.result.current.intention).toBe(before);
    expect(h.hook.result.current.keys).toEqual({ pan: 0, tilt: 5, clock: 'source', frame: 8 });
    act(() => h.hook.result.current.choose('w2')); expect(h.hook.result.current.intention).toBe(before + 1);
  });
});
