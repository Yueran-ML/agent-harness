// @vitest-environment jsdom
/**
 * The Curve Editor (`src/studio/keyframes/CurveEditor`): the curves and keys that it draws, what a
 * pointer, the tool bar and the keyboard do to them, each as what it says the settings are to be,
 * and the keys that it keeps from the page; and its place on the Edit page
 * (`src/studio/keyframes/ClipCurves`): the operations that it sends for the clip chosen, a drag as
 * one gesture, the playhead through the frames of the keys, and the buttons that open it.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type Interp, type Keyframe, type Param, type Timeline as TimelineModel } from '../src/studio/engine';
import { Inspector } from '../src/studio/inspector/Inspector';
import { ClipCurves } from '../src/studio/keyframes/ClipCurves';
import { CurveEditor, GRAPH_SIZE, OWN_VALUE_GAP_PX, plotRect, type Curve, type CurveChange, type CurveClipboard } from '../src/studio/keyframes/CurveEditor';
import { copyKeys, curvePath, fitView, insertKey, moveKeys, pasteKeys, setInterp, dragHandle, xOf, yOf } from '../src/studio/keyframes/curveGeometry';
import { Timeline } from '../src/studio/timeline/Timeline';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); });

const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const bounds = { low: -16, high: 16 };
const X = 'transform.x';
const Y = 'transform.y';
const three = [key(0, 0), key(40, 1), key(80, 0)];
const curveOf = (id: string, label: string, param: Param): Curve => ({ id, label, color: id === X ? '#e5534b' : '#57ab5a', param, bounds });
/** The graph is not laid out in the DOM of the tests: it is as large as it is until it is measured. */
const rect = plotRect(GRAPH_SIZE);

function mount(curves: Curve[], given: { playhead?: number | null; disabled?: boolean } = {}) {
  const onPreview = vi.fn<(changes: CurveChange[]) => void>();
  const onCommit = vi.fn<(changes: CurveChange[]) => void>();
  const onCancel = vi.fn();
  const onSeek = vi.fn();
  const clipboard: CurveClipboard = { keys: [] };
  const props = { playhead: 20 as number | null, clipboard, onPreview, onCommit, onCancel, onSeek, ...given };
  const view = render(<CurveEditor curves={curves} {...props} />);
  return { ...view, props, onPreview, onCommit, onCancel, onSeek, clipboard };
}
const editor = () => document.querySelector<HTMLElement>('[data-test="curve-editor"]')!;
const graph = () => document.querySelector<SVGSVGElement>('.curve-editor-graph')!;
const keyOf = (curve: string, index: number) => document.querySelector<SVGPathElement>(`.curve-key[data-curve="${curve}"][data-key="${index}"]`)!;
/** Where something is drawn, to the nearest pixel: a pointer there is within reach of it. */
const placeOf = (element: Element) => {
  const [, x, y] = /translate\(([-\d.e]+) ([-\d.e]+)\)/.exec(element.getAttribute('transform')!)!;
  return { clientX: Math.round(Number(x)), clientY: Math.round(Number(y)) };
};
const at = (curve: string, index: number) => placeOf(keyOf(curve, index));
const pointer = (place: { clientX: number; clientY: number }, extra = {}) => ({ button: 0, pointerId: 1, ...place, ...extra });
const click = (place: { clientX: number; clientY: number }, extra = {}) => {
  fireEvent.pointerDown(graph(), pointer(place, extra));
  fireEvent.pointerUp(graph(), pointer(place, extra));
};
const chosen = (curve = X) => [...document.querySelectorAll(`.curve-key[data-curve="${curve}"][data-selected="true"]`)].map(each => Number(each.getAttribute('data-key')));
const lineOf = (curve: string) => document.querySelector<SVGPathElement>(`.curve-line[data-curve="${curve}"]`);

describe('the curve editor', () => {
  it('lists the settings, those with keys first and shown, and draws the curve and the keys of each one shown', () => {
    const param: Param = { keys: three };
    mount([curveOf(Y, 'Position Y', 0.5), curveOf(X, 'Position X', param)]);
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes.map(box => [box.getAttribute('aria-label'), box.checked])).toEqual([['Position X', true], ['Position Y', false]]);
    // The one curve shown is fitted to its keys, and is the path of the setting.
    expect(lineOf(X)!.getAttribute('d')).toBe(curvePath(param, fitView([param]), rect));
    expect(lineOf(X)!.getAttribute('data-active')).toBe('true');
    expect(lineOf(Y)).toBeNull();
    expect(document.querySelectorAll(`.curve-key[data-curve="${X}"]`)).toHaveLength(3);
    expect(chosen()).toEqual([]);
    expect(document.querySelector('.curve-editor-playhead')!.getAttribute('data-frame')).toBe('20');
    // A setting of one number is level, once its box is ticked; the curve with keys stays the one edited.
    fireEvent.click(boxes[1]);
    expect(lineOf(Y)!.getAttribute('d')).toBe(curvePath(0.5, { ...fitView([param, 0.5]), ...{ low: fitView([0.5]).low, high: fitView([0.5]).high } }, rect));
    expect(lineOf(Y)!.getAttribute('data-active')).toBe('false');
    fireEvent.click(boxes[0]);
    expect(lineOf(X)).toBeNull();
  });

  it('draws each curve at a scale of its own, and writes the values of a line that is not the one edited at its points', () => {
    // A Retime Frame curve from 0 to 60 and a Retime Speed curve of 48% and then of -54%, over the 100 frames of a clip.
    const own = { id: 'clip', playhead: 0, range: [0, 100] as const, onSeek: vi.fn() };
    const line = { shown: true, drag: () => null };
    const unbounded = { low: -Infinity, high: Infinity };
    const frame: Param = { keys: [key(0, 0), key(100, 60)] };
    const speed: Param = { keys: [key(0, 48, 'hold'), key(50, -54, 'hold')] };
    const FRAME = 'retime.frame';
    const SPEED = 'retime.speed';
    mount([
      { id: FRAME, label: 'Retime Frame', color: '#dcbdfb', param: frame, bounds: unbounded, display: { factor: 1, unit: '', digits: 1 }, clock: own, line },
      { id: SPEED, label: 'Retime Speed', color: '#96d0ff', param: speed, bounds: unbounded, display: { factor: 1, unit: '%', digits: 0 }, clock: own, line },
    ]);
    const span = fitView([frame, speed], [0, 100]);
    const viewOf = (param: Param) => ({ from: span.from, to: span.to, low: fitView([param]).low, high: fitView([param]).high });
    const placed = (element: Element) => { const [, x, y] = /translate\(([-\d.e]+) ([-\d.e]+)\)/.exec(element.getAttribute('transform')!)!; return [Number(x), Number(y)]; };
    const valuesAtLeft = () => [...document.querySelectorAll('.curve-editor-value')].map(each => each.textContent);
    const written = () => [...document.querySelectorAll('.curve-editor-own-value')].map(each => [
      each.parentElement!.getAttribute('data-curve'), each.textContent, Number(each.getAttribute('x')), Number(each.getAttribute('y'))]);
    // The frame curve is the one edited: the values at the left are its own, in frames, and none of them is a speed.
    expect(lineOf(FRAME)!.getAttribute('data-active')).toBe('true');
    expect(valuesAtLeft()).toEqual(expect.arrayContaining(['0.0', '60.0']));
    expect(valuesAtLeft().some(value => value!.endsWith('%'))).toBe(false);
    // The speed is at its own scale, from -64.2% to 58.2% over the 190 pixels of the graph: 48% is drawn 10.2% under
    // its upper edge and -54% as far over its lower edge, each where its own value is.
    const scale = viewOf(speed);
    expect([scale.low, scale.high].map(value => Number(value.toFixed(1)))).toEqual([-64.2, 58.2]);
    const [high, low] = [yOf(scale, rect, 48), yOf(scale, rect, -54)];
    expect([high, low].map(y => Number(y.toFixed(2)))).toEqual([35.83, 194.17]);
    expect(placed(keyOf(SPEED, 0))).toEqual([xOf(scale, rect, 0), high]);
    expect(placed(keyOf(SPEED, 1))).toEqual([xOf(scale, rect, 50), low]);
    expect(lineOf(SPEED)!.getAttribute('d')).toBe(curvePath(speed, scale, rect));
    // The frame curve is at its own, where 0 and 60 are as far from the edges: so the two cannot be read by one row of values,
    // and the speed says its own at each step, in its colour: under the step at the upper edge, over the one at the lower.
    expect(placed(keyOf(FRAME, 1))[1]).toBe(yOf(viewOf(frame), rect, 60));
    expect(placed(keyOf(FRAME, 1))[1]).toBeCloseTo(high, 6);
    expect(written()).toEqual([
      [SPEED, '48%', xOf(scale, rect, 0) + 7, high + 13],
      [SPEED, '-54%', xOf(scale, rect, 50) + 7, low - 6],
    ]);
    expect(document.querySelector('.curve-editor-own-value')!.getAttribute('style')).toMatch(/fill:\s*(#96d0ff|rgb\(150, 208, 255\))/);
    // With the speed the one edited, the values at the left are speeds, and the frame curve says its frames at its points.
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    expect(lineOf(SPEED)!.getAttribute('data-active')).toBe('true');
    expect(valuesAtLeft().every(value => value!.endsWith('%'))).toBe(true);
    expect(written().map(([curve, said]) => [curve, said])).toEqual([[FRAME, '0.0'], [FRAME, '60.0']]);
    // A point nearer than a value is wide to the one written before it is not written over it.
    cleanup();
    const near = xOf(scale, rect, 0) + OWN_VALUE_GAP_PX - 1;
    const steps: Param = { keys: [key(0, 48, 'hold'), key(Math.floor(span.from + ((near - rect.left) / rect.width) * (span.to - span.from)), 20, 'hold'), key(50, -54, 'hold')] };
    mount([
      { id: FRAME, label: 'Retime Frame', color: '#dcbdfb', param: frame, bounds: unbounded, clock: own, line },
      { id: SPEED, label: 'Retime Speed', color: '#96d0ff', param: steps, bounds: unbounded, display: { factor: 1, unit: '%', digits: 0 }, clock: own, line },
    ]);
    expect(written().map(([, said]) => said)).toEqual(['48%', '-54%']);
    // The settings with keys, which are no lines, are drawn as they were: each at its own scale, with nothing written at their keys.
    cleanup();
    mount([curveOf(X, 'Position X', { keys: three }), curveOf(Y, 'Position Y', { keys: [key(0, 300), key(80, 900)] })]);
    expect(document.querySelector('.curve-editor-own-value')).toBeNull();
  });

  it('chooses a key with a click, more with Ctrl and Shift, and those in a box', () => {
    mount([curveOf(X, 'Position X', { keys: three })]);
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    click(at(X, 0), { ctrlKey: true });
    expect(chosen()).toEqual([0, 1]);
    click(at(X, 0), { ctrlKey: true });
    expect(chosen()).toEqual([1]);
    // A click on another key chooses it alone; Shift chooses all from it to the one clicked.
    click(at(X, 0));
    expect(chosen()).toEqual([0]);
    click(at(X, 2), { shiftKey: true });
    expect(chosen()).toEqual([0, 1, 2]);
    // A click where nothing is lets go of them.
    click({ clientX: rect.left + 5, clientY: rect.top + 5 });
    expect(chosen()).toEqual([]);
    // A box around the first two keys.
    const first = at(X, 0);
    const second = at(X, 1);
    fireEvent.pointerDown(graph(), pointer({ clientX: first.clientX - 12, clientY: second.clientY - 12 }));
    fireEvent.pointerMove(graph(), pointer({ clientX: second.clientX + 12, clientY: first.clientY + 12 }));
    expect(document.querySelector('.curve-editor-box')).not.toBeNull();
    expect(chosen()).toEqual([0, 1]);
    fireEvent.pointerUp(graph(), pointer({ clientX: second.clientX + 12, clientY: first.clientY + 12 }));
    expect(document.querySelector('.curve-editor-box')).toBeNull();
    expect(chosen()).toEqual([0, 1]);
  });

  it('moves a key that is dragged: a preview at each move, and one commit of the last as it is let go', () => {
    const param: Param = { keys: three };
    const { onPreview, onCommit, onCancel } = mount([curveOf(X, 'Position X', param)]);
    const view = fitView([param]);
    const from = at(X, 1);
    const moved = (dx: number, dy: number) => moveKeys(three, [1], (dx / rect.width) * (view.to - view.from), (-dy / rect.height) * (view.high - view.low), bounds);
    fireEvent.pointerDown(graph(), pointer(from));
    // Not yet a drag.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 1, clientY: from.clientY + 1 }));
    expect(onPreview).not.toHaveBeenCalled();
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 60, clientY: from.clientY + 19 }));
    expect(onPreview.mock.calls).toEqual([[[{ id: X, param: { keys: moved(60, 19) } }]]]);
    expect(moved(60, 19)[1].frame).toBe(50);
    expect(moved(60, 19)[1].value).toBeLessThan(1);
    // The editor draws the key where the drag has it, whatever it is given meanwhile.
    expect(keyOf(X, 1).getAttribute('data-frame')).toBe('50');
    expect(chosen()).toEqual([1]);
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX - 30, clientY: from.clientY + 19 }));
    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(onPreview.mock.calls[1]).toEqual([[{ id: X, param: { keys: moved(-30, 19) } }]]);
    // A move to where the pointer is says nothing again.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX - 30, clientY: from.clientY + 19 }));
    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(onCommit).not.toHaveBeenCalled();
    fireEvent.pointerUp(graph(), pointer({ clientX: from.clientX - 30, clientY: from.clientY + 19 }));
    expect(onCommit.mock.calls).toEqual([[[{ id: X, param: { keys: moved(-30, 19) } }]]]);
    expect(onCancel).not.toHaveBeenCalled();
    // Let go, it draws what it is given again.
    expect(keyOf(X, 1).getAttribute('data-frame')).toBe('40');
    // A click that moves nothing sends nothing.
    click(at(X, 2));
    expect(onPreview).toHaveBeenCalledTimes(2);
    expect(onCommit).toHaveBeenCalledTimes(1);
  });

  it('moves every key chosen together, keeps a drag with Shift to one direction, and takes a drag back with Escape', () => {
    const param: Param = { keys: three };
    const { onPreview, onCommit, onCancel } = mount([curveOf(X, 'Position X', param)]);
    const view = fitView([param]);
    const frames = (dx: number) => (dx / rect.width) * (view.to - view.from);
    const values = (dy: number) => (-dy / rect.height) * (view.high - view.low);
    click(at(X, 0));
    click(at(X, 1), { ctrlKey: true });
    const from = at(X, 1);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 60, clientY: from.clientY + 19 }));
    expect(onPreview.mock.calls[0]).toEqual([[{ id: X, param: { keys: moveKeys(three, [0, 1], frames(60), values(19), bounds) } }]]);
    expect(chosen()).toEqual([0, 1]);
    // With Shift the drag keeps to the direction that it took: here along the frames.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 90, clientY: from.clientY + 19 }, { shiftKey: true }));
    expect(onPreview.mock.calls[1]).toEqual([[{ id: X, param: { keys: moveKeys(three, [0, 1], frames(90), 0, bounds) } }]]);
    fireEvent.keyDown(editor(), { key: 'Escape' });
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(keyOf(X, 1).getAttribute('data-frame')).toBe('40');
    expect(chosen()).toEqual([0, 1]);
    fireEvent.pointerUp(graph(), pointer(from));
    expect(onCommit).not.toHaveBeenCalled();
    // A press on one of several chosen, let go where it was, chooses that one alone.
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    expect(onCommit).not.toHaveBeenCalled();
  });

  it('shows the handles of a key chosen on a Bezier stretch, and shapes the stretch as one is dragged', () => {
    const smooth = setInterp(three, [1], 'in_out');
    const param: Param = { keys: smooth };
    const { onPreview, onCommit } = mount([curveOf(X, 'Position X', param)]);
    expect(document.querySelectorAll('.curve-handle')).toHaveLength(0);
    click(at(X, 1));
    const handles = [...document.querySelectorAll('.curve-handle')];
    expect(handles.map(each => each.getAttribute('data-side'))).toEqual(['in', 'out']);
    const from = placeOf(handles[1].querySelector('circle')!);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 20, clientY: from.clientY + 30 }));
    expect(onPreview).toHaveBeenCalledTimes(1);
    const keys = (onPreview.mock.calls[0][0][0].param as { keys: Keyframe[] }).keys;
    // Only the stretch that leaves the key changed, in the handle at this key.
    expect(keys[0]).toEqual(smooth[0]);
    expect(keys[2]).toEqual(smooth[2]);
    const before = (smooth[1].interp as { bezier: { out: [number, number]; in: [number, number] } }).bezier;
    const after = (keys[1].interp as { bezier: { out: [number, number]; in: [number, number] } }).bezier;
    expect(after.in).toEqual(before.in);
    expect(after.out[0]).toBeGreaterThan(before.out[0]);
    expect(after.out[1]).toBeLessThan(0);
    const view = fitView([param]);
    const to = { x: from.clientX + 20, y: from.clientY + 30 };
    const frame = view.from + ((to.x - rect.left) / rect.width) * (view.to - view.from);
    const value = view.high - ((to.y - rect.top) / rect.height) * (view.high - view.low);
    expect(keys).toEqual(dragHandle(smooth, 1, 'out', frame, value, bounds));
    fireEvent.pointerUp(graph(), pointer({ clientX: to.x, clientY: to.y }));
    expect(onCommit.mock.calls).toEqual([[[{ id: X, param: { keys } }]]]);
  });

  it('sets the interpolation of the keys chosen from the tool bar', () => {
    const { onCommit } = mount([curveOf(X, 'Position X', { keys: three })]);
    const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
    for (const name of ['Linear', 'Ease In', 'Ease Out', 'Ease In and Out', 'Hold']) expect(button(name).disabled).toBe(true);
    click(at(X, 1));
    expect(button('Ease In').disabled).toBe(false);
    // The keys are lines: Linear is lit.
    expect(['Linear', 'Ease In', 'Hold'].map(name => button(name).getAttribute('aria-pressed'))).toEqual(['true', 'false', 'false']);
    fireEvent.click(button('Ease In'));
    expect(onCommit.mock.calls[0]).toEqual([[{ id: X, param: { keys: setInterp(three, [1], 'in') } }]]);
    fireEvent.click(button('Ease In and Out'));
    expect(onCommit.mock.calls[1]).toEqual([[{ id: X, param: { keys: setInterp(three, [1], 'in_out') } }]]);
    fireEvent.click(button('Ease Out'));
    expect(onCommit.mock.calls[2]).toEqual([[{ id: X, param: { keys: setInterp(three, [1], 'out') } }]]);
    fireEvent.click(button('Hold'));
    expect(onCommit.mock.calls[3]).toEqual([[{ id: X, param: { keys: [key(0, 0), key(40, 1, 'hold'), key(80, 0)] } }]]);
    // What changes nothing is not sent.
    fireEvent.click(button('Linear'));
    expect(onCommit).toHaveBeenCalledTimes(4);
  });

  it('takes the keys chosen away with Delete and Backspace, and leaves a number where none stays', () => {
    const { onCommit } = mount([curveOf(X, 'Position X', { keys: three })]);
    click(at(X, 1));
    click(at(X, 2), { ctrlKey: true });
    fireEvent.keyDown(editor(), { key: 'Delete' });
    expect(onCommit.mock.calls[0]).toEqual([[{ id: X, param: { keys: [key(0, 0)] } }]]);
    expect(chosen()).toEqual([]);
    // Ctrl+A chooses every key; without them the setting is what it was at the playhead, frame 20.
    fireEvent.keyDown(editor(), { key: 'a', ctrlKey: true });
    expect(chosen()).toEqual([0, 1, 2]);
    fireEvent.keyDown(editor(), { key: 'Backspace' });
    expect(onCommit.mock.calls[1]).toEqual([[{ id: X, param: 0.5 }]]);
  });

  it('puts a key on a curve with a double click and with Alt-click, and chooses it', () => {
    const param: Param = { keys: three };
    const { onCommit } = mount([curveOf(X, 'Position X', param)]);
    const [first, second, third] = [at(X, 0), at(X, 1), at(X, 2)];
    // Halfway along the line from the first key to the second: frame 20.
    const middle = { clientX: Math.round((first.clientX + second.clientX) / 2), clientY: Math.round((first.clientY + second.clientY) / 2) };
    fireEvent.doubleClick(graph(), middle);
    expect(onCommit.mock.calls[0]).toEqual([[{ id: X, param: { keys: insertKey(param, 20)!.keys } }]]);
    expect(insertKey(param, 20)!.keys[1]).toEqual(key(20, 0.5));
    // Halfway from the second to the third: frame 60.
    const later = { clientX: Math.round((second.clientX + third.clientX) / 2), clientY: Math.round((second.clientY + third.clientY) / 2) };
    fireEvent.pointerDown(graph(), pointer(later, { altKey: true }));
    fireEvent.pointerUp(graph(), pointer(later, { altKey: true }));
    expect(onCommit.mock.calls[1]).toEqual([[{ id: X, param: { keys: insertKey(param, 60)!.keys } }]]);
    // On a key, and away from every curve, a double click puts none.
    fireEvent.doubleClick(graph(), second);
    fireEvent.doubleClick(graph(), { clientX: middle.clientX, clientY: middle.clientY + 40 });
    expect(onCommit).toHaveBeenCalledTimes(2);
  });

  it('copies the keys chosen and pastes them at the playhead, as far apart as they were', () => {
    const param: Param = { keys: three };
    const { onCommit, clipboard } = mount([curveOf(X, 'Position X', param)]);
    const paste = screen.getByRole('button', { name: 'Paste Keyframes' }) as HTMLButtonElement;
    const copy = screen.getByRole('button', { name: 'Copy Keyframes' }) as HTMLButtonElement;
    expect([copy.disabled, paste.disabled]).toEqual([true, true]);
    click(at(X, 0));
    click(at(X, 1), { ctrlKey: true });
    fireEvent.click(copy);
    expect(clipboard.keys).toEqual(copyKeys(three, [0, 1]));
    expect(paste.disabled).toBe(false);
    fireEvent.click(paste);
    const pasted = pasteKeys(param, copyKeys(three, [0, 1]), 20, bounds)!;
    expect(pasted.frames).toEqual([20, 60]);
    expect(onCommit.mock.calls[0]).toEqual([[{ id: X, param: { keys: pasted.keys } }]]);
    // With the keys: Ctrl+C and Ctrl+V; Ctrl+X copies and takes away.
    clipboard.keys = [];
    click(at(X, 2));
    fireEvent.keyDown(editor(), { key: 'c', ctrlKey: true });
    expect(clipboard.keys).toEqual([{ offset: 0, value: 0, interp: 'linear' }]);
    fireEvent.keyDown(editor(), { key: 'v', ctrlKey: true });
    expect(onCommit.mock.calls[1]).toEqual([[{ id: X, param: { keys: [key(0, 0), key(20, 0), key(40, 1), key(80, 0)] } }]]);
    click(at(X, 1));
    fireEvent.keyDown(editor(), { key: 'x', ctrlKey: true });
    expect(clipboard.keys).toEqual([{ offset: 0, value: 1, interp: 'linear' }]);
    expect(onCommit.mock.calls[2]).toEqual([[{ id: X, param: { keys: [key(0, 0), key(80, 0)] } }]]);
  });

  it('moves the keys chosen by a frame with Ctrl and an arrow', () => {
    const { onCommit } = mount([curveOf(X, 'Position X', { keys: three })]);
    click(at(X, 1));
    fireEvent.keyDown(editor(), { key: 'ArrowRight', ctrlKey: true });
    expect(onCommit.mock.calls[0]).toEqual([[{ id: X, param: { keys: [key(0, 0), key(41, 1), key(80, 0)] } }]]);
    // The editor is given the same keys still: the key is chosen again where it is.
    click(at(X, 1));
    fireEvent.keyDown(editor(), { key: 'ArrowLeft', ctrlKey: true });
    expect(onCommit.mock.calls[1]).toEqual([[{ id: X, param: { keys: [key(0, 0), key(39, 1), key(80, 0)] } }]]);
  });

  it('keeps its keys from the page whether or not anything is chosen, and lets the others go on', () => {
    const { onCommit, onPreview, clipboard } = mount([curveOf(X, 'Position X', { keys: three })]);
    const page = vi.fn();
    window.addEventListener('keydown', page);
    try {
      const owned: [string, object][] = [['Delete', {}], ['Backspace', {}], ['Escape', {}], ['c', { ctrlKey: true }], ['x', { ctrlKey: true }], ['v', { ctrlKey: true }],
        ['a', { metaKey: true }], ['ArrowLeft', { ctrlKey: true }], ['ArrowRight', { ctrlKey: true }]];
      // Nothing is chosen. Select All is tried after the others: it would give them something to do.
      for (const [name, modifiers] of owned.filter(([name]) => name !== 'a')) fireEvent.keyDown(editor(), { key: name, ...modifiers });
      expect(page).not.toHaveBeenCalled();
      expect(onCommit).not.toHaveBeenCalled();
      expect(onPreview).not.toHaveBeenCalled();
      expect(clipboard.keys).toEqual([]);
      fireEvent.keyDown(editor(), { key: 'a', metaKey: true });
      expect(page).not.toHaveBeenCalled();
      // The transport, Undo and the tools of the timeline are not the editor's.
      const others: [string, object][] = [[' ', {}], ['z', { ctrlKey: true }], ['b', {}], ['ArrowLeft', {}], ['c', {}], ['v', { ctrlKey: true, shiftKey: true }]];
      for (const [name, modifiers] of others) fireEvent.keyDown(editor(), { key: name, ...modifiers });
      expect(page).toHaveBeenCalledTimes(6);
      expect(onCommit).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener('keydown', page);
    }
  });

  it('moves the playhead from its ruler, in the frames of the keys', () => {
    const { onSeek } = mount([curveOf(X, 'Position X', { keys: three })]);
    const second = at(X, 1);
    fireEvent.pointerDown(graph(), pointer({ clientX: second.clientX, clientY: 6 }));
    expect(onSeek.mock.calls).toEqual([[40]]);
    fireEvent.pointerMove(graph(), pointer({ clientX: at(X, 2).clientX, clientY: 90 }));
    expect(onSeek.mock.calls).toEqual([[40], [80]]);
    fireEvent.pointerUp(graph(), pointer({ clientX: at(X, 2).clientX, clientY: 90 }));
    expect(chosen()).toEqual([]);
    // No playhead is drawn where there is none.
    cleanup();
    mount([curveOf(X, 'Position X', { keys: three })], { playhead: null });
    expect(document.querySelector('.curve-editor-playhead')).toBeNull();
    expect((screen.getByRole('button', { name: 'Paste Keyframes' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('edits one curve at a time: a click on another, or on its name, makes it the one edited', () => {
    // Each curve is fitted to its own values, so the keys of the two are drawn apart.
    const other = [key(20, 2), key(60, -2)];
    mount([curveOf(X, 'Position X', { keys: three }), curveOf(Y, 'Position Y', { keys: other })]);
    expect([lineOf(X)!.getAttribute('data-active'), lineOf(Y)!.getAttribute('data-active')]).toEqual(['true', 'false']);
    click(at(X, 1));
    expect(chosen(X)).toEqual([1]);
    // A key of the dimmed curve: it is the one edited, and that key alone is chosen.
    click(at(Y, 1));
    expect([lineOf(X)!.getAttribute('data-active'), lineOf(Y)!.getAttribute('data-active')]).toEqual(['false', 'true']);
    expect(chosen(Y)).toEqual([1]);
    expect(chosen(X)).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: 'Position X' }));
    expect(lineOf(X)!.getAttribute('data-active')).toBe('true');
    expect(chosen(Y)).toEqual([]);
  });

  it('keeps a key chosen by its frame as the keys change under it, and lets go of one that is gone', () => {
    const { rerender, props } = mount([curveOf(X, 'Position X', { keys: three })]);
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    // A key came before it: it is the third now.
    rerender(<CurveEditor curves={[curveOf(X, 'Position X', { keys: [key(0, 0), key(20, 0.5), key(40, 1), key(80, 0)] })]} {...props} />);
    expect(chosen()).toEqual([2]);
    rerender(<CurveEditor curves={[curveOf(X, 'Position X', { keys: [key(0, 0), key(80, 0)] })]} {...props} />);
    expect(chosen()).toEqual([]);
    expect((screen.getByRole('button', { name: 'Hold' }) as HTMLButtonElement).disabled).toBe(true);
    // And where the setting has no keys any more, nothing of it is drawn.
    rerender(<CurveEditor curves={[curveOf(X, 'Position X', 0.25)]} {...props} />);
    expect(document.querySelectorAll('.curve-key')).toHaveLength(0);
    expect(lineOf(X)).toBeNull();
  });

  it('changes nothing where it may not, though keys can be chosen and copied', () => {
    const { onCommit, onPreview, clipboard } = mount([curveOf(X, 'Position X', { keys: three })], { disabled: true });
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    expect((screen.getByRole('button', { name: 'Ease In' }) as HTMLButtonElement).disabled).toBe(true);
    const from = at(X, 1);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 60, clientY: from.clientY }));
    fireEvent.pointerUp(graph(), pointer({ clientX: from.clientX + 60, clientY: from.clientY }));
    fireEvent.keyDown(editor(), { key: 'Delete' });
    fireEvent.keyDown(editor(), { key: 'c', ctrlKey: true });
    fireEvent.keyDown(editor(), { key: 'v', ctrlKey: true });
    fireEvent.doubleClick(graph(), { clientX: Math.round((at(X, 0).clientX + from.clientX) / 2), clientY: Math.round((at(X, 0).clientY + from.clientY) / 2) });
    expect(onPreview).not.toHaveBeenCalled();
    expect(onCommit).not.toHaveBeenCalled();
    expect(clipboard.keys).toEqual([{ offset: 0, value: 1, interp: 'linear' }]);
  });
});

const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
/** c1 from frame 10 for 60 frames, showing its media from its first picture: the picture `n` at frame `10 + n`. */
const keyed = (keys: Keyframe[], fields: Partial<Clip> = {}) => clip('c1', 10, 60, { transform: { ...NONE, x: { keys } }, ...fields });
const timelineOf = (clips: Clip[], locked = false): TimelineModel => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});
type Call = ReturnType<typeof vi.fn>;
function wire(clips: Clip[], given: { chosen?: string[]; frame?: number; locked?: boolean } = {}) {
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const onSeek = vi.fn();
  const props = {
    bridge: { call } as unknown as Pick<EngineBridge, 'call'>, session: 1, revision: 17, timeline: timelineOf(clips, given.locked), media: [media],
    clips: given.chosen ?? ['c1'], frame: given.frame ?? 30, onSeek,
  };
  const view = render(<ClipCurves {...props} />);
  return { ...view, props, call, onSeek };
}
const sent = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply');
const applied = (call: Call) => sent(call).map(([, params]) => params);
const quarter = [key(0, 0), key(40, 0.25), key(80, 0)];
const splice = (at: number, remove: number, keys: Keyframe[]) => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: X, at, remove, keys });

describe('the Curve Editor of the Edit page', () => {
  it('shows the settings of the one clip of picture chosen, with the playhead at the frame of its keys', () => {
    wire([keyed(quarter)]);
    expect(document.querySelector('[data-test="clip-curves"]')!.getAttribute('data-clip')).toBe('c1');
    expect(screen.getByRole('group', { name: 'Curve Editor · Interview' })).toBeDefined();
    // The ten settings of the transform and the crop, the one with keys first and shown; and after them the two Retime curves of a clip of
    // media, hidden while the clip has no speed points (`retime-curves.test.tsx`).
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes.map(box => box.getAttribute('aria-label'))).toEqual(['Position X', 'Zoom', 'Position Y', 'Rotation angle', 'Opacity',
      'Crop left', 'Crop right', 'Crop top', 'Crop bottom', 'Softness', 'Retime Frame', 'Retime Speed']);
    expect(boxes.map(box => box.checked)).toEqual([true, ...Array.from({ length: 11 }, () => false)]);
    expect(document.querySelectorAll(`.curve-key[data-curve="${X}"]`)).toHaveLength(3);
    // The playhead is at frame 30 of the timeline: the picture 20 of the clip.
    expect(document.querySelector('.curve-editor-playhead')!.getAttribute('data-frame')).toBe('20');
  });

  it('says what to choose where no clip of picture, or more than one, is chosen', () => {
    const none = wire([keyed(quarter)], { chosen: [] });
    expect(document.querySelector('[data-test="curves-empty"]')!.textContent).toBe('Select one video clip on the timeline to edit its keyframes.');
    expect(document.querySelector('[data-test="curve-editor"]')).toBeNull();
    none.unmount();
    wire([keyed(quarter), clip('c2', 100, 50)], { chosen: ['c1', 'c2'] });
    expect(document.querySelector('[data-test="curves-empty"]')).not.toBeNull();
    cleanup();
    // A clip that is not there is none; one of picture among them is the one.
    wire([keyed(quarter)], { chosen: ['gone', 'c1'] });
    expect(document.querySelector('[data-test="curve-editor"]')).not.toBeNull();
  });

  it('sends an edit of the tool bar as one splice of the keys that differ, with their place', async () => {
    const { call, rerender, props } = wire([keyed(quarter)]);
    click(at(X, 1));
    fireEvent.click(screen.getByRole('button', { name: 'Ease In' }));
    await settle();
    // Easing into the second key changes the stretch that comes to it, which the first key holds.
    const smooth = setInterp(quarter, [1], 'in');
    expect(applied(call)).toEqual([{ revision: 17, ops: [splice(0, 1, [smooth[0]])] }]);
    expect(smooth[0].interp).not.toBe('linear');
    // The copy takes the edit in; the key is chosen still, and the next edit is worked out from it.
    rerender(<ClipCurves {...props} revision={18} timeline={timelineOf([keyed(smooth)])} />);
    expect(chosen()).toEqual([1]);
    expect(screen.getByRole('button', { name: 'Ease In' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByRole('button', { name: 'Hold' }));
    await settle();
    expect(applied(call)[1]).toEqual({ revision: 18, ops: [splice(1, 1, [key(40, 0.25, 'hold')])] });
  });

  it('sends a key taken away as a splice, and the last key of a setting as its number', async () => {
    const some = wire([keyed(quarter)]);
    click(at(X, 1));
    fireEvent.keyDown(editor(), { key: 'Delete' });
    await settle();
    expect(applied(some.call)).toEqual([{ revision: 17, ops: [splice(1, 1, [])] }]);
    cleanup();
    const one = wire([keyed([key(20, 0.3)])]);
    click(at(X, 0));
    fireEvent.keyDown(editor(), { key: 'Delete' });
    await settle();
    expect(applied(one.call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: X, param: 0.3 }] }]);
  });

  it('sends a key put on a curve and keys pasted as splices where they go among the keys', async () => {
    const { call } = wire([keyed(quarter)]);
    const [first, second] = [at(X, 0), at(X, 1)];
    fireEvent.doubleClick(graph(), { clientX: Math.round((first.clientX + second.clientX) / 2), clientY: Math.round((first.clientY + second.clientY) / 2) });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [splice(1, 0, [key(20, 0.125)])] }]);
    cleanup();
    // The last key copied and pasted at the playhead, the picture 20: it goes between the first two.
    const again = wire([keyed(quarter)]);
    click(at(X, 2));
    fireEvent.click(screen.getByRole('button', { name: 'Copy Keyframes' }));
    fireEvent.click(screen.getByRole('button', { name: 'Paste Keyframes' }));
    await settle();
    expect(applied(again.call)).toEqual([{ revision: 17, ops: [splice(1, 0, [key(20, 0)])] }]);
  });

  it('finds the place of a key among the keys that the timeline has as the edit goes', async () => {
    const { call, rerender, props } = wire([keyed(quarter)]);
    click(at(X, 2));
    // Someone put a key before it meanwhile: the key chosen is the fourth now.
    rerender(<ClipCurves {...props} revision={18} timeline={timelineOf([keyed([key(0, 0), key(10, 0.5), key(40, 0.25), key(80, 0)])])} />);
    fireEvent.keyDown(editor(), { key: 'Delete' });
    await settle();
    expect(applied(call)).toEqual([{ revision: 18, ops: [splice(3, 1, [])] }]);
  });

  it('makes a drag one gesture: the setting whole at each move, and the end as the key is let go', async () => {
    const { call } = wire([keyed(quarter)]);
    const from = at(X, 1);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 40, clientY: from.clientY + 10 }));
    await settle();
    expect(sent(call)).toHaveLength(1);
    const [, first, context] = sent(call)[0];
    expect(context).toEqual({ expectedSession: 1 });
    expect(first.revision).toBe(17);
    expect(first.gesture.id).toBeTruthy();
    expect(first.gesture.end).toBeUndefined();
    expect(first.ops).toHaveLength(1);
    expect(first.ops[0]).toMatchObject({ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: X });
    const keys = first.ops[0].param.keys as Keyframe[];
    expect(keys.map(each => each.frame)).toEqual([0, Number(keyOf(X, 1).getAttribute('data-frame')), 80]);
    expect(keys[1].frame).toBeGreaterThan(40);
    expect(keys[1].value).toBeLessThan(0.25);
    fireEvent.pointerUp(graph(), pointer({ clientX: from.clientX + 40, clientY: from.clientY + 10 }));
    await settle();
    expect(sent(call)).toHaveLength(2);
    const last = sent(call)[1][1];
    expect(last.gesture).toEqual({ id: first.gesture.id, end: true });
    expect(last.ops).toEqual(first.ops);
    expect(last.revision).toBe(17);
  });

  it('puts a setting back as it was where a drag is taken back', async () => {
    const { call } = wire([keyed(quarter)]);
    const from = at(X, 1);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 40, clientY: from.clientY + 10 }));
    await settle();
    fireEvent.keyDown(editor(), { key: 'Escape' });
    await settle();
    expect(sent(call)).toHaveLength(2);
    expect(sent(call)[1][1]).toMatchObject({ revision: 17, gesture: { end: true }, ops: [{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: X, param: { keys: quarter } }] });
    fireEvent.pointerUp(graph(), pointer(from));
    await settle();
    expect(sent(call)).toHaveLength(2);
  });

  it('moves the playhead of the timeline to where the clip shows the picture pressed on the ruler', () => {
    const { onSeek } = wire([keyed(quarter)]);
    fireEvent.pointerDown(graph(), pointer({ clientX: at(X, 1).clientX, clientY: 6 }));
    fireEvent.pointerUp(graph(), pointer({ clientX: at(X, 1).clientX, clientY: 6 }));
    // The picture 40 is shown at frame 50 of the timeline.
    expect(onSeek.mock.calls).toEqual([[50, false]]);
    cleanup();
    // Backward, it is shown at frame 29.
    const back = wire([keyed(quarter, { reverse: true })]);
    fireEvent.pointerDown(graph(), pointer({ clientX: at(X, 1).clientX, clientY: 6 }));
    expect(back.onSeek.mock.calls).toEqual([[29, false]]);
  });

  it('shows a clip on a locked track and changes nothing of it', async () => {
    const { call } = wire([keyed(quarter)], { locked: true });
    expect(document.querySelector('[data-test="curves-locked"]')).not.toBeNull();
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    expect((screen.getByRole('button', { name: 'Ease In' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(editor(), { key: 'Delete' });
    const from = at(X, 1);
    fireEvent.pointerDown(graph(), pointer(from));
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 40, clientY: from.clientY }));
    fireEvent.pointerUp(graph(), pointer({ clientX: from.clientX + 40, clientY: from.clientY }));
    await settle();
    expect(sent(call)).toHaveLength(0);
  });

  it('follows the clip as it changes, and begins anew for another clip', () => {
    const { rerender, props } = wire([keyed(quarter)]);
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    // Undo took the key away: it is chosen no longer, and the others are drawn.
    rerender(<ClipCurves {...props} revision={18} timeline={timelineOf([keyed([key(0, 0), key(80, 0)])])} />);
    expect(document.querySelectorAll(`.curve-key[data-curve="${X}"]`)).toHaveLength(2);
    expect(chosen()).toEqual([]);
    click(at(X, 1));
    expect(chosen()).toEqual([1]);
    // Another clip chosen: nothing of the first is carried over.
    const c2 = clip('c2', 100, 50, { transform: { ...NONE, x: { keys: [key(0, 0), key(80, 0)] } } });
    rerender(<ClipCurves {...props} revision={18} timeline={timelineOf([keyed(quarter), c2])} clips={['c2']} />);
    expect(document.querySelector('[data-test="clip-curves"]')!.getAttribute('data-clip')).toBe('c2');
    expect(chosen()).toEqual([]);
  });

  it('is opened from the Video tab of the inspector and from the tool bar of the timeline', () => {
    const onCurves = vi.fn();
    const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
    const bridge = { call, onEvent: vi.fn(() => () => {}) } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>;
    const model = timelineOf([keyed(quarter)]);
    const shown = render(<Inspector bridge={bridge} session={1} revision={17} timeline={model} media={[media]} clips={['c1']} transition={null}
      onSpeed={vi.fn()} frame={30} onCurves={onCurves} />);
    fireEvent.click(document.querySelector('[data-test="open-curves"]')!);
    expect(onCurves).toHaveBeenCalledTimes(1);
    shown.unmount();
    // Without a Curve Editor on the page there is no button.
    const plain = render(<Inspector bridge={bridge} session={1} revision={17} timeline={model} media={[media]} clips={['c1']} transition={null} onSpeed={vi.fn()} frame={30} />);
    expect(document.querySelector('[data-test="open-curves"]')).toBeNull();
    plain.unmount();
    vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
    const common = { timeline: model, media: [media], bridge, session: 1, revision: 17, frame: 20, playing: false, onSeek: vi.fn() };
    const first = render(<Timeline {...common} curvesOpen onCurves={onCurves} />);
    const toggle = screen.getByRole('button', { name: 'Curve Editor' });
    expect(toggle.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(toggle);
    expect(onCurves).toHaveBeenCalledTimes(2);
    first.unmount();
    render(<Timeline {...common} />);
    expect(screen.queryByRole('button', { name: 'Curve Editor' })).toBeNull();
  });
});
