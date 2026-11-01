// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useNodeEdit, type EditedNode, type Edits } from '../src/studio/color/useNodeEdit';
import { circularWindow, useWindows, WindowPalette } from '../src/studio/color/WindowPalette';
import { Palettes } from '../src/studio/color/Palettes';
import { likeABrowser } from './mounted';
import { fullCorrector } from '../src/studio/color/corrector';
import { mergePatch, type PowerWindow } from '../src/studio/engine';
import type { EditScope } from '../src/studio/color/windowEdits';

afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let n = 0; n < 20; n++) await Promise.resolve(); });
const change = (id: string, field: string, value: number) => [['windows', { window: { id, field, value } }]] as unknown as Edits;
const scoped = (scope: EditScope, id: string, value: number) => [['windows', { window: { id, field: 'opacity', value }, scope }]] as const;
function mount() {
  const windows = [circularWindow('w1'), circularWindow('w2')];
  windows[0].transform.size = { keys: [{ frame: 0, value: 20, interp: { bezier: { in: [1, 2], out: [3, 4] } } }, { frame: 10, value: 80, interp: 'hold' }] };
  const sent: { revision: number; ops: { params: { windows: PowerWindow[] } }[]; gesture?: { id: string; end?: boolean } }[] = [];
  const replies: ((value: unknown) => void)[] = [];
  const bridge = { call: vi.fn((_method: string, params: unknown) => { sent.push(params as typeof sent[number]); return new Promise(resolve => replies.push(resolve)); }) };
  let context: EditedNode = { bridge: bridge as EditedNode['bridge'], session: 1, revision: 10, target: { level: 'clip', timeline: 't1', clip: 'c1' },
    node: { id: 'n1', kind: 'corrector', enabled: true, label: '', params: fullCorrector({ windows }) }, frame: 5, timelineFrame: 5, playing: true, onFailure: vi.fn() };
  const hook = renderHook(at => useNodeEdit(at), { initialProps: context });
  return { hook, sent, original: windows,
    answer: async (revision: number) => { replies.shift()!({ ok: true, result: { revision, done: [], next_id: 1, names: {}, clamped: [] } }); await settle(); },
    copy: async (revision: number, params: unknown) => { context = { ...context, revision, node: { ...context.node, kind: 'corrector', params: fullCorrector(mergePatch(context.node.params, params)) } }; hook.rerender(context); await settle(); },
    frame: (frame: number) => { context = { ...context, frame, timelineFrame: frame }; hook.rerender(context); },
  };
}

it('resolves queued Window fields against their confirmed predecessor without overwriting another window', async () => {
  const h = mount();
  act(() => { h.hook.result.current.commit(change('w1', 'transform.size', 35)); h.hook.result.current.commit(change('w2', 'opacity', 60)); });
  expect(h.sent[0].ops[0].params.windows).toBeInstanceOf(Array);
  await h.answer(11);
  expect(h.sent).toHaveLength(1);
  await h.copy(11, h.sent[0].ops[0].params);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].revision).toBe(11);
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ ...((h.original[0].transform.size as { keys: unknown[] }).keys[0] as object), value: 35 }, (h.original[0].transform.size as { keys: unknown[] }).keys[1]] });
  expect(h.sent[1].ops[0].params.windows[1].opacity).toBe(60);
});

it('uses the actual Window field key to retire a drag and reject a retained old callback', async () => {
  const h = mount(); const old = h.hook.result.current;
  act(() => old.preview(change('w1', 'transform.size', 30)));
  h.frame(11);
  await h.answer(11);
  act(() => { old.preview(change('w1', 'transform.size', 45)); old.commit(change('w1', 'transform.size', 46)); });
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].gesture?.end).toBe(true);
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ ...((h.original[0].transform.size as { keys: unknown[] }).keys[0] as object), value: 30 }, (h.original[0].transform.size as { keys: unknown[] }).keys[1]] });
});

it('Escape restores the confirmed predecessor whole Windows after the optimistic display expires', async () => {
  vi.useFakeTimers();
  const h = mount();
  act(() => h.hook.result.current.commit(change('w2', 'opacity', 60)));
  await act(async () => vi.advanceTimersByTime(1600));
  act(() => h.hook.result.current.preview(change('w1', 'transform.size', 35)));
  await h.answer(11); await h.copy(11, h.sent[0].ops[0].params);
  await h.answer(12);
  act(() => h.hook.result.current.cancel());
  expect(h.sent.at(-1)?.ops[0].params.windows).toEqual([{ ...h.original[0] }, { ...h.original[1], opacity: 60 }]);
});

it('ends only the successful old scope and waits for its confirmed copy before sending a new window', async () => {
  const h = mount(); const first = { live: true }; const second = { live: true };
  act(() => { h.hook.result.current.preview(scoped(first, 'w1', 60)); h.hook.result.current.preview(scoped(first, 'w1', 75)); });
  act(() => { h.hook.result.current.retireScope(first); h.hook.result.current.preview(scoped(second, 'w2', 80)); });
  await h.answer(11);
  expect(h.sent[1].gesture?.end).toBe(true);
  expect(h.sent[1].ops[0].params.windows[0].opacity).toBe(60);
  await h.answer(12);
  expect(h.sent).toHaveLength(2);
  await h.copy(12, h.sent[1].ops[0].params);
  expect(h.sent[2].revision).toBe(12);
  expect(h.sent[2].gesture?.id).not.toBe(h.sent[0].gesture?.id);
  expect(h.sent[2].ops[0].params.windows.map(w => w.opacity)).toEqual([60, 80]);
  act(() => h.hook.result.current.commit(scoped(first, 'w1', 95)));
  expect(h.sent).toHaveLength(3);
});

it('an unrelated Window key crossing keeps an opacity drag alive', async () => {
  const h = mount(); const scope = { live: true };
  act(() => h.hook.result.current.preview(scoped(scope, 'w2', 60)));
  h.frame(11); await h.answer(11);
  act(() => h.hook.result.current.commit(scoped(scope, 'w2', 65)));
  expect(h.sent[1].gesture?.id).toBe(h.sent[0].gesture?.id);
  expect(h.sent[1].ops[0].params.windows[1].opacity).toBe(65);
});

it('cancelling a different numeric control cannot restore the active field gesture', async () => {
  const h = mount(); const first = { live: true }; const second = { live: true };
  act(() => h.hook.result.current.preview(scoped(second, 'w2', 60)));
  await h.answer(11);
  act(() => (h.hook.result.current.cancel as (scope: EditScope) => void)(first));
  expect(h.sent).toHaveLength(1);
});

function mountPalette(complete = true, deferred = false) {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  const windows = [circularWindow('w1'), { ...circularWindow('w2'), shape: { kind: 'gradient' as const, softness: 0 } }];
  windows[0].transform.size = { keys: [{ frame: 0, value: 20, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] };
  const sent: { revision: number; ops: { params: { windows: PowerWindow[] } }[]; gesture?: { id: string; end?: boolean } }[] = [];
  const response = (revision: number) => ({ ok: true as const, result: { revision, done: [], next_id: 1, names: {}, clamped: [] } });
  const replies: ((value: ReturnType<typeof response>) => void)[] = [];
  const call = vi.fn((_method: string, params: unknown) => {
    sent.push(params as typeof sent[number]);
    return deferred ? new Promise<ReturnType<typeof response>>(resolve => replies.push(resolve)) : Promise.resolve(response(11));
  });
  let frame = 5; let revision = 10;
  let node = { id: 'n1', kind: 'corrector' as const, enabled: true, label: '', params: fullCorrector({ windows }) };
  const target = { level: 'clip' as const, timeline: 't1', clip: 'c1' };
  let controls: ReturnType<typeof useWindows>;
  function Harness() {
    controls = useWindows({ session: 1, revision, timeline: 't1', target, clip: 'c1', node, realNode: node, complete,
      editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame, clock: 'source' });
    return <Palettes merged={false} left="primaries" center="window" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}}
      edited={{ bridge: { call }, session: 1, revision, target, node, frame, timelineFrame: frame, playing: true, onFailure: vi.fn() }}
      windows={edit => <WindowPalette edit={edit} controls={controls} capturing={false} busy={() => false} status="idle" />} />;
  }
  const view = render(<Harness />);
  return { sent, controls: () => controls!, frame: (next: number) => { frame = next; view.rerender(<Harness />); },
    answer: async (at: number) => { replies.shift()!(response(at)); await settle(); },
    copy: async (at: number, params: unknown) => { revision = at; node = { ...node, params: fullCorrector(mergePatch(node.params, params)) }; view.rerender(<Harness />); await settle(); },
  };
}

it('real Window numbers show the left key and edit even when only native binding is incomplete', async () => {
  const h = mountPalette(false);
  const input = screen.getByRole('textbox', { name: 'Size' }) as HTMLInputElement;
  expect(input.value).toBe('20.00'); expect(input.disabled).toBe(false);
  expect(h.controls().selection).toBeNull();
  fireEvent.doubleClick(input); fireEvent.change(input, { target: { value: '35' } }); fireEvent.keyDown(input, { key: 'Enter' });
  await settle();
  expect(h.sent[0].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ frame: 0, value: 35, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] });
  expect(h.sent[0].ops[0].params.windows[1].shape).toEqual({ kind: 'gradient', softness: 0 });
});

it.each(['selection', 'key'] as const)('real numeric down cannot revive after a %s round trip', kind => {
  const h = mountPalette();
  const old = screen.getByRole('textbox', { name: 'Size' });
  fireEvent.pointerDown(old, { button: 0, clientX: 0, clientY: 0, pointerId: 1 });
  if (kind === 'selection') {
    fireEvent.click(screen.getByRole('option', { name: /w2/ }));
    fireEvent.click(screen.getByRole('option', { name: /w1/ }));
  } else { h.frame(11); h.frame(5); }
  expect(screen.getByRole('textbox', { name: 'Size' })).not.toBe(old);
  fireEvent.pointerMove(old, { clientX: 20, clientY: 0, pointerId: 1 });
  fireEvent.pointerUp(old, { clientX: 20, clientY: 0, pointerId: 1 });
  expect(h.sent).toHaveLength(0);
});


it('reports only the native captured control fields and their own left keys', () => {
  const h = mountPalette();
  act(() => h.controls().onState({ binding: 1, capture: 1, sequence: 2, phase: 'armed', reason: null, selected: 'w1', event: 2, revision: 10,
    keys: { size: 0, clock: 'source', frame: 5 } }));
  const hints = document.querySelector('[data-test="window-keys"]')!;
  expect(hints.textContent).toContain('Size:');
  expect(hints.textContent).toContain('0');
  expect(hints.textContent).not.toContain('Pan:');
  expect(hints.textContent).not.toContain('undefined');
});


it.each(['commit', 'preview'] as const)('new-key numeric %s waits for the previous key gesture and its confirmed copy', async mode => {
  const h = mountPalette(true, true);
  const old = screen.getByRole('textbox', { name: 'Size' });
  fireEvent.pointerDown(old, { button: 0, clientX: 0, pointerId: 1 });
  fireEvent.pointerMove(old, { clientX: 40, pointerId: 1 });
  fireEvent.pointerMove(old, { clientX: 80, pointerId: 1 });
  expect(h.sent).toHaveLength(1);
  h.frame(11);
  const next = screen.getByRole('textbox', { name: 'Size' });
  if (mode === 'commit') {
    fireEvent.doubleClick(next); fireEvent.change(next, { target: { value: '90' } }); fireEvent.keyDown(next, { key: 'Enter' });
  } else {
    fireEvent.pointerDown(next, { button: 0, clientX: 0, pointerId: 2 });
    fireEvent.pointerMove(next, { clientX: 40, pointerId: 2 });
  }
  expect(h.sent).toHaveLength(1);
  await h.answer(11);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].gesture).toEqual({ ...h.sent[0].gesture, end: true });
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ frame: 0, value: 30, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] });
  await h.copy(11, h.sent[0].ops[0].params);
  await h.answer(12);
  expect(h.sent).toHaveLength(2);
  await h.copy(12, h.sent[1].ops[0].params);
  expect(h.sent).toHaveLength(3);
  expect(h.sent[2].revision).toBe(12);
  expect(h.sent[2].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ frame: 0, value: 30, interp: 'linear' }, { frame: 10, value: 90, interp: 'hold' }] });
  expect(h.sent[2].ops[0].params.windows[1]).toEqual(h.sent[1].ops[0].params.windows[1]);
  if (mode === 'preview') {
    expect(h.sent[2].gesture?.id).not.toBe(h.sent[0].gesture?.id);
    await h.answer(13);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(h.sent[3].ops[0].params.windows).toEqual(h.sent[1].ops[0].params.windows);
  }
});

const sized = (scope: EditScope, value: number) => [['windows', { window: { id: 'w1', field: 'transform.size', value }, scope }]] as const;

it('a released Window drag keeps its release value when a key crossing retires its scope while the finish is queued', async () => {
  const h = mount(); const scope = { live: true };
  act(() => { h.hook.result.current.preview(sized(scope, 30)); h.hook.result.current.preview(sized(scope, 40)); });
  act(() => h.hook.result.current.commit(sized(scope, 46)));
  expect(h.sent).toHaveLength(1);
  h.frame(11);
  await h.answer(11);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].gesture).toEqual({ id: h.sent[0].gesture?.id, end: true });
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ ...((h.original[0].transform.size as { keys: unknown[] }).keys[0] as object), value: 46 }, (h.original[0].transform.size as { keys: unknown[] }).keys[1]] });
});

it.each(['typed', 'dragged'] as const)('a %s Size commit queued behind a predecessor still lands after the user selects another window', async mode => {
  const h = mountPalette(true, true);
  const opacity = screen.getByRole('textbox', { name: 'Opacity' });
  fireEvent.doubleClick(opacity); fireEvent.change(opacity, { target: { value: '60' } }); fireEvent.keyDown(opacity, { key: 'Enter' });
  expect(h.sent).toHaveLength(1);
  const size = screen.getByRole('textbox', { name: 'Size' });
  if (mode === 'typed') {
    fireEvent.doubleClick(size); fireEvent.change(size, { target: { value: '35' } }); fireEvent.blur(size);
  } else {
    fireEvent.pointerDown(size, { button: 0, clientX: 0, pointerId: 1 });
    fireEvent.pointerMove(size, { clientX: 40, pointerId: 1 });
    fireEvent.pointerUp(size, { clientX: 40, pointerId: 1 });
  }
  expect(h.sent).toHaveLength(1);
  fireEvent.click(screen.getByRole('option', { name: /w2/ }));
  expect(screen.queryByRole('textbox', { name: 'Size' })).not.toBe(size);
  await h.answer(11);
  await h.copy(11, h.sent[0].ops[0].params);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].revision).toBe(11);
  expect(h.sent[1].ops[0].params.windows[0].opacity).toBe(60);
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ frame: 0, value: mode === 'typed' ? 35 : 30, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] });
});

it('a started Size drag released just before selecting another window finishes with its release value', async () => {
  const h = mountPalette(true, true);
  const size = screen.getByRole('textbox', { name: 'Size' });
  fireEvent.pointerDown(size, { button: 0, clientX: 0, pointerId: 1 });
  fireEvent.pointerMove(size, { clientX: 40, pointerId: 1 });
  fireEvent.pointerMove(size, { clientX: 80, pointerId: 1 });
  fireEvent.pointerUp(size, { clientX: 100, pointerId: 1 });
  expect(h.sent).toHaveLength(1);
  fireEvent.click(screen.getByRole('option', { name: /w2/ }));
  await h.answer(11);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].gesture).toEqual({ ...h.sent[0].gesture, end: true });
  expect(h.sent[1].ops[0].params.windows[0].transform.size).toEqual({ keys: [{ frame: 0, value: 40, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] });
});

it('names the Window number fields in each language, as the other palettes do', async () => {
  const { en } = await import('../src/messages/en');
  const { ja } = await import('../src/messages/ja');
  const { zhCN } = await import('../src/messages/zh-CN');
  const english = en.label.studio.colorPage.windowControls.fields;
  for (const words of [ja, zhCN]) {
    for (const [name, text] of Object.entries(words.label.studio.colorPage.windowControls.fields)) {
      expect(text, name).not.toBe(english[name as keyof typeof english]);
    }
  }
});
