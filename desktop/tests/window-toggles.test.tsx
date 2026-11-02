// @vitest-environment jsdom
/**
 * The Invert and Mask buttons of each row of the Window list (RM20 p. 3189; Mask on adds, off subtracts, p. 3196 and
 * the project's decision): each a switch of the whole window, never keyed, sent through the node's edit queue as the
 * whole Windows array resolved by window id against its confirmed predecessor, so one undo step each.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { circularWindow, useWindows, WindowPalette } from '../src/studio/color/WindowPalette';
import { Palettes } from '../src/studio/color/Palettes';
import { likeABrowser } from './mounted';
import { fullCorrector } from '../src/studio/color/corrector';
import { mergePatch, type PowerWindow } from '../src/studio/engine';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let n = 0; n < 20; n++) await Promise.resolve(); });

function mount({ locked = false, deferred = true } = {}) {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  const windows: PowerWindow[] = [circularWindow('w1'), { ...circularWindow('w2'), label: 'Face', shape: { kind: 'gradient', softness: 0 } }];
  windows[0].transform.size = { keys: [{ frame: 0, value: 20, interp: 'linear' }, { frame: 10, value: 80, interp: 'hold' }] };
  const sent: { revision: number; ops: { op: string; params: { windows: PowerWindow[] } }[]; gesture?: unknown }[] = [];
  const response = (revision: number) => ({ ok: true as const, result: { revision, done: [], next_id: 1, names: {}, clamped: [] } });
  const replies: ((value: ReturnType<typeof response>) => void)[] = [];
  const call = vi.fn((_method: string, params: unknown) => {
    sent.push(params as typeof sent[number]);
    return deferred ? new Promise<ReturnType<typeof response>>(resolve => replies.push(resolve)) : Promise.resolve(response(11));
  });
  let revision = 10;
  let node = { id: 'n1', kind: 'corrector' as const, enabled: true, locked, label: '', params: fullCorrector({ windows }) };
  const target = { level: 'clip' as const, timeline: 't1', clip: 'c1' };
  let controls: ReturnType<typeof useWindows>;
  function Harness() {
    controls = useWindows({ session: 1, revision, timeline: 't1', target, clip: 'c1', node, realNode: node, complete: true,
      editing: { run: vi.fn(), step: vi.fn(), problem: null, setProblem: vi.fn() }, tool: 'window', frame: 5, clock: 'source' });
    return <Palettes merged={false} left="primaries" center="window" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}}
      edited={{ bridge: { call }, session: 1, revision, target, node, frame: 5, timelineFrame: 5, playing: false, onFailure: vi.fn() }}
      windows={edit => <WindowPalette edit={edit} controls={controls} capturing={false} busy={() => false} status="idle" />} />;
  }
  const view = render(<Harness />);
  return { sent, original: windows, controls: () => controls!,
    answer: async (at: number) => { replies.shift()!(response(at)); await settle(); },
    copy: async (at: number, params: unknown) => { revision = at; node = { ...node, params: fullCorrector(mergePatch(node.params, params)) }; view.rerender(<Harness />); await settle(); },
  };
}
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const pressed = (name: string) => button(name).getAttribute('aria-pressed');

it('shows an Invert and a Mask button on each row, Invert off and Mask on (adding) by default', () => {
  mount();
  for (const name of ['w1', 'Face']) {
    expect(pressed(`Invert ${name}`)).toBe('false');
    expect(pressed(`Mask ${name}`)).toBe('true');
  }
  // The row's own button still chooses the window; the switches are beside it.
  expect(screen.getAllByRole('option').map(each => each.textContent)).toEqual(['w1 · Circular', 'Face · Gradient']);
});

it('Invert sends the whole Windows list with that window inverted, keeping the others and every key', async () => {
  const h = mount();
  fireEvent.click(button('Invert Face'));
  expect(h.sent).toHaveLength(1);
  expect(h.sent[0].gesture).toBeUndefined();
  expect(h.sent[0].ops).toEqual([{ op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c1' }, id: 'n1',
    params: { windows: [h.original[0], { ...h.original[1], invert: true }] } }]);
  // Shown at once, as the engine is asked.
  expect(pressed('Invert Face')).toBe('true');
  expect(pressed('Invert w1')).toBe('false');
  await h.answer(11); await h.copy(11, h.sent[0].ops[0].params);
  expect(pressed('Invert Face')).toBe('true');
  fireEvent.click(button('Invert Face'));
  expect(h.sent[1].ops[0].params.windows[1].invert).toBe(false);
});

it('Mask off subtracts the window; a second switch queued behind it lands on its confirmed predecessor', async () => {
  const h = mount();
  fireEvent.click(button('Mask w1'));
  fireEvent.click(button('Invert Face'));
  expect(pressed('Mask w1')).toBe('false');
  expect(h.sent).toHaveLength(1);
  expect(h.sent[0].ops[0].params.windows).toEqual([{ ...h.original[0], combine: 'subtract' }, h.original[1]]);
  await h.answer(11);
  expect(h.sent).toHaveLength(1);
  await h.copy(11, h.sent[0].ops[0].params);
  expect(h.sent).toHaveLength(2);
  expect(h.sent[1].revision).toBe(11);
  // Not keyed: the keyed Size of w1 is kept as it was, and Mask stays a plain switch.
  expect(h.sent[1].ops[0].params.windows).toEqual([{ ...h.original[0], combine: 'subtract' }, { ...h.original[1], invert: true }]);
  fireEvent.click(button('Mask w1'));
  await h.answer(12); await h.copy(12, h.sent[1].ops[0].params);
  expect(h.sent[2].ops[0].params.windows[0].combine).toBe('add');
});

it('switching a row neither chooses that window nor changes the window being edited', () => {
  const h = mount();
  expect(h.controls().selected?.id).toBe('w1');
  fireEvent.click(button('Invert Face'));
  expect(h.controls().selected?.id).toBe('w1');
  expect(screen.getByRole('option', { name: 'w1 · Circular' }).getAttribute('aria-selected')).toBe('true');
});

it('a locked node cannot be switched', () => {
  const h = mount({ locked: true });
  expect(button('Invert w1').disabled).toBe(true);
  expect(button('Mask w1').disabled).toBe(true);
  fireEvent.click(button('Mask w1'));
  expect(h.sent).toHaveLength(0);
});

it('names Invert and Mask in each language, as DaVinci does', async () => {
  const { ja } = await import('../src/messages/ja');
  const { zhCN } = await import('../src/messages/zh-CN');
  const { en } = await import('../src/messages/en');
  const of = (words: typeof en) => words.label.studio.colorPage.windowControls;
  expect([of(en).invert, of(en).mask]).toEqual(['Invert', 'Mask']);
  expect([of(ja).invert, of(ja).mask]).toEqual(['反転', 'マスク']);
  expect([of(zhCN).invert, of(zhCN).mask]).toEqual(['反转', '蒙版']);
});
