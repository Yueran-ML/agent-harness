// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { en } from '../src/messages/en';
import { circularWindow, useWindows } from '../src/studio/color/WindowPalette';
import { presetItems, presetProblem, WindowPresets, type PresetBridge, type PresetPlace } from '../src/studio/color/WindowPresets';
import { newCorrectorNode } from '../src/studio/color/nodeEdits';
import { EngineError, type GradeNode, type PowerWindow, type WindowPreset } from '../src/studio/engine';
import { likeABrowser } from './mounted';

beforeAll(likeABrowser);
afterEach(cleanup);

const words = en.label.studio.colorPage.windowPresets;
const said = en.hint.studio.colorPage.windowPresets;
const target = { level: 'clip', timeline: 't1', clip: 'c1' } as const;
const face: WindowPreset = { id: 'wp1', name: 'Face', windows: [circularWindow('w4'), circularWindow('w5')] };
const pair: WindowPreset = { id: 'wp2', name: 'Pair', windows: [circularWindow('w1')] };

/** A bridge whose library holds `presets`; `refuse` answers a method with a refusal of the engine. */
function engine(presets: WindowPreset[], refuse: Record<string, { code: number; message: string; data: unknown }> = {}) {
  const listeners = new Set<(event: any) => void>();
  const call = vi.fn(async (method: string, _params?: Record<string, unknown>) => {
    if (refuse[method]) return { ok: false, error: refuse[method] };
    if (method === 'window_presets.list') return { ok: true, result: { presets } };
    if (method === 'window_presets.apply') return { ok: true, result: { revision: 11, names: {} } };
    return { ok: true, result: {} };
  });
  const bridge = { call, onEvent: (listener: (event: any) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; } } as unknown as PresetBridge;
  const tell = (event: unknown) => act(() => { for (const each of listeners) each(event); });
  const sent = (method: string) => call.mock.calls.filter(([name]) => name === method).map(([, params]) => params);
  return { bridge, call, tell, sent };
}

function place(bridge: PresetPlace['bridge'], changes: Partial<PresetPlace> = {}): PresetPlace {
  return { bridge, session: 1, target, node: 'n3', frame: 42, revision: 10, enabled: true, windows: 2, ...changes };
}

const item = (id: string) => screen.getByRole('menuitem', { name: (_, element) => element.getAttribute('data-item') === id });
async function openMenu() {
  fireEvent.click(screen.getByRole('button', { name: words.options }));
  return screen.findByRole('menu', { name: words.options });
}

describe('The option menu of window presets (RM20 p. 3199)', () => {
  it('lists Save as New Preset, Update and Delete by name, then the presets, and says when there are none', () => {
    const act = { saveNew: vi.fn(), update: vi.fn(), remove: vi.fn(), recall: vi.fn() };
    const empty = presetItems(words, [], { enabled: true, windows: 1 }, act);
    expect(empty.map(each => [each.label, !!each.disabled])).toEqual([
      [words.saveNew, false], [words.update, true], [words.delete, true], [words.none, true]]);
    const listed = presetItems(words, [face, pair], { enabled: true, windows: 0 }, act);
    // Without windows nothing is saved or updated; presets are still recalled and deleted.
    expect(listed.map(each => [each.label, !!each.disabled])).toEqual([
      [words.saveNew, true], [words.update, true], [words.delete, false], ['Face', false], ['Pair', false]]);
    expect(listed[2].items?.map(each => each.label)).toEqual(['Face', 'Pair']);
    listed[2].items?.[1].run?.();
    expect(act.remove).toHaveBeenCalledWith('wp2');
    const locked = presetItems(words, [face], { enabled: false, windows: 2 }, act);
    expect(locked.map(each => !!each.disabled)).toEqual([true, true, false, true]);
  });

  it('says why the engine refused, in the page words', () => {
    const refusal = (data: unknown) => new EngineError({ code: -32001, message: 'raw', data } as never);
    expect(presetProblem(said, refusal({ reason: 'name_taken', name: 'Face' }))).toBe(said.nameTaken.replace('{name}', 'Face'));
    expect(presetProblem(said, refusal({ reason: 'no_windows' }))).toBe(said.noWindows);
    expect(presetProblem(said, refusal({ reason: 'missing', what: 'preset' }))).toBe(said.missing);
    expect(presetProblem(said, refusal({ reason: 'missing', what: 'node' }))).toBe('raw');
  });

  it('names a new preset in a dialog and saves the node at the frame of the playhead; a name taken keeps the dialog', async () => {
    const h = engine([face], { 'window_presets.save': { code: -32001, message: 'taken', data: { reason: 'name_taken', name: 'Face' } } });
    render(<WindowPresets place={place(h.bridge)} />);
    await waitFor(() => expect(h.sent('window_presets.list')).toHaveLength(1));
    await openMenu();
    fireEvent.click(item('window-preset-save'));
    const ok = screen.getByRole('button', { name: words.ok }) as HTMLButtonElement;
    expect(ok.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText(words.name), { target: { value: '  Face ' } });
    fireEvent.click(ok);
    await screen.findByText(said.nameTaken.replace('{name}', 'Face'));
    expect(h.sent('window_presets.save')).toEqual([{ name: 'Face', grade: target, node: 'n3', frame: 42 }]);
    expect(screen.getByLabelText(words.name)).toBeTruthy();
    // Another name is saved, and the dialog goes.
    h.call.mockImplementation(async () => ({ ok: true, result: {} }));
    fireEvent.change(screen.getByLabelText(words.name), { target: { value: 'Wide' } });
    fireEvent.submit(screen.getByLabelText(words.name).closest('form')!);
    await waitFor(() => expect(screen.queryByLabelText(words.name)).toBeNull());
    expect(h.sent('window_presets.save').at(-1)).toEqual({ name: 'Wide', grade: target, node: 'n3', frame: 42 });
  });

  it('recalls a preset onto the node at its revision, and selects its first window', async () => {
    const h = engine([face, pair]);
    const recalled = vi.fn();
    render(<WindowPresets place={place(h.bridge)} onRecalled={recalled} />);
    await waitFor(() => expect(h.sent('window_presets.list')).toHaveLength(1));
    await openMenu();
    await waitFor(() => item('window-preset-wp1'));
    fireEvent.click(item('window-preset-wp1'));
    expect(h.sent('window_presets.apply')).toEqual([{ preset: 'wp1', grade: target, node: 'n3', revision: 10 }]);
    await waitFor(() => expect(recalled).toHaveBeenCalledWith('w4'));
  });

  it('updates and deletes by name from their submenus, with no confirmation, and lists again when the library changes', async () => {
    const h = engine([face, pair]);
    render(<WindowPresets place={place(h.bridge)} />);
    await waitFor(() => expect(h.sent('window_presets.list')).toHaveLength(1));
    await openMenu();
    await waitFor(() => item('window-preset-update'));
    fireEvent.click(item('window-preset-update'));
    await waitFor(() => item('window-preset-update-wp2'));
    fireEvent.click(item('window-preset-update-wp2'));
    expect(h.sent('window_presets.update')).toEqual([{ preset: 'wp2', grade: target, node: 'n3', frame: 42 }]);
    await openMenu();
    fireEvent.click(item('window-preset-delete'));
    await waitFor(() => item('window-preset-delete-wp1'));
    fireEvent.click(item('window-preset-delete-wp1'));
    expect(h.sent('window_presets.delete')).toEqual([{ preset: 'wp1' }]);
    expect(screen.queryByRole('dialog')).toBeNull();
    h.tell({ kind: 'notification', method: 'window_presets.changed', params: {} });
    await waitFor(() => expect(h.sent('window_presets.list')).toHaveLength(2));
  });

  it('has no node to save or recall onto while the palette shows a default node that is not made yet', () => {
    const node = (windows: PowerWindow[]): GradeNode => ({ ...newCorrectorNode('n1'), params: { ...newCorrectorNode('n1').params, windows } } as GradeNode);
    const h = engine([]);
    const props: Parameters<typeof useWindows>[0] = { bridge: h.bridge, session: 1, revision: 10, timeline: 't1', target, clip: 'c1',
      node: node([]), realNode: null, complete: true, editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() },
      tool: 'window', frame: 8, clock: 'source' };
    const hook = renderHook(at => useWindows(at), { initialProps: props });
    expect(hook.result.current.presets.node).toBeNull();
    hook.rerender({ ...props, node: node([circularWindow('w1')]), realNode: node([circularWindow('w1')]) });
    expect(hook.result.current.presets).toMatchObject({ node: 'n1', frame: 8, revision: 10, windows: 1, target });
  });
});
