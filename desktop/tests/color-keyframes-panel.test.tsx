// @vitest-environment jsdom
/**
 * The Keyframe Editor of the Color page (`src/studio/color/KeyframesPanel`, DaVinci Resolve
 * Reference Manual 20, Ch. 148): the tracks and the marks that it draws, a diamond for a dynamic
 * keyframe and a circle for a static one; its tool bar, its mode and its keys, each command the
 * one transaction that `nodeKeys` works out; the menus of a mark and of a track; the dialog of
 * Change Dynamic Attributes; a mark dragged as one gesture; and the playhead through the clock of
 * the clip. Auto-Keyframe, where the palettes commit their edits (`useNodeEdit`): the keyframe and
 * the change in one transaction. And the page: the keys from anywhere on it, and the badge of a
 * node that has keyframes.
 */
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { useCallback, useMemo } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Corrector, type Grade, type GradeNode, type Interp, type Keyframe, type Target } from '../src/studio/engine';
import { fullCorrector } from '../src/studio/color/corrector';
import { keyClockOf } from '../src/studio/color/keyFrames';
import { KeyframesPanel } from '../src/studio/color/KeyframesPanel';
import { LANES_WIDTH, easePath, frameAtX, keyRange, rowMiddle, xOfFrame, type KeySpan } from '../src/studio/color/keyTracks';
import {
  addKeyframe, addKeyframes, changeKeyframes, deleteAllKeyframes, deleteKeyframes, easeKeyframes, moveKeyframe, paramsWith, settingsOf, withEaseAt,
  withKeyframe,
} from '../src/studio/color/nodeKeys';
import { useGradeEditing } from '../src/studio/color/useGradeEditing';
import { useKeyframeEditing } from '../src/studio/color/useKeyframes';
import { useNodeEdit, type EditedNode } from '../src/studio/color/useNodeEdit';
import { lutGrade, mount, projectOf, sent as sentOps, settle as settlePage } from './colorEngine';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // The parts of the page lie at the top left; the lanes of the editor, an SVG, are not laid out, and are as wide as they are until measured.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const RATE = { num: 25, den: 1 };
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const corrector = (id: string): GradeNode => ({ id, kind: 'corrector', label: '', enabled: true, locked: false, params: fullCorrector({}) as Corrector });
/** 01 has a dynamic keyframe at 20 and a static one at 60; 02 a dynamic one at 40; 03 is a mixer, which takes none. */
const n1 = withKeyframe(withKeyframe(corrector('n1'), 20, 'dynamic'), 60, 'static');
const n2 = withKeyframe(corrector('n2'), 40, 'dynamic');
const n3: GradeNode = { id: 'n3', kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 2 } };
const GRADE: Grade = { nodes: [n1, n2, n3], links: emptyGrade().links };
/** c1 from frame 10 for 100 frames, showing its media from its first picture: the picture `n` at frame `10 + n`. */
const SPAN: KeySpan = { first: 10, last: 109, clock: keyClockOf(clip('c1', 10, 100), RATE, media) };
const RANGE = keyRange(SPAN);
/** How many settings of a corrector take keys. */
const PLAIN = settingsOf(corrector('n1')).length;

type Call = ReturnType<typeof vi.fn>;
interface Given { grade?: Grade | null; current?: string | null; span?: KeySpan | null; frame?: number | null; revision?: number }

/** The editor with the commands of the page and the transactions of the Color page, as the page holds them. */
function Harness({ call, onSeek, onCurrent, onFailure, grade = GRADE, current = 'n2', span = SPAN, frame = 30, revision = 17 }: Given & {
  call: Call; onSeek: (frame: number, scrubbing: boolean) => void; onCurrent: (node: string) => void; onFailure: (failure: unknown) => void;
}) {
  const bridge = useMemo(() => ({ call }) as unknown as Pick<EngineBridge, 'call'>, [call]);
  const editing = useGradeEditing({ bridge, revision, session: 1 });
  const seek = useCallback((to: number) => onSeek(to, false), [onSeek]);
  const keys = useKeyframeEditing({ grade, target: TARGET, gradeKey: 'clip:c1', current, span, frame, editing, onSeek: seek, reset: '1/t1' });
  return <div className="studio">
    <KeyframesPanel bridge={bridge} session={1} revision={revision} grade={grade} target={TARGET} current={current} onCurrent={onCurrent} span={span}
      frame={frame} keys={keys} editing={editing} onSeek={onSeek} onFailure={onFailure} hint="There is no clip at the playhead." />
  </div>;
}

function wire(given: Given = {}) {
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [], next_id: 10, names: {}, clamped: [] } : {} }));
  const props = { call, onSeek: vi.fn(), onCurrent: vi.fn(), onFailure: vi.fn(), ...given };
  const view = render(<Harness {...props} />);
  return { ...view, props, call, onSeek: props.onSeek, onCurrent: props.onCurrent };
}
/** The transactions sent, each as it was called: its method, its params and its context. */
const sent = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply');
const applied = (call: Call) => sent(call).map(([, params]) => params as { revision: number; ops: unknown[]; gesture?: { id: string; end?: boolean } });
const panel = () => document.querySelector<HTMLElement>('[data-test="keyframes-panel"]')!;
const lanes = () => document.querySelector<SVGSVGElement>('[data-test="kf-lanes"]')!;
const marksOn = (row: number) => [...document.querySelectorAll(`.color-kf-mark[data-row="${row}"]`)]
  .map(each => [each.tagName.toLowerCase(), Number(each.getAttribute('data-frame')), each.getAttribute('data-kind')]);
const markAt = (row: number, frame: number) => document.querySelector<SVGElement>(`.color-kf-mark[data-row="${row}"][data-frame="${frame}"]`)!;
const xOf = (frame: number, range: readonly [number, number] = RANGE) => xOfFrame(range, LANES_WIDTH, frame);
/** Where a mark is drawn, to the nearest pixel: a pointer there is within reach of it. */
const at = (row: number, frame: number) => ({ clientX: Math.round(xOf(frame)), clientY: rowMiddle(row) });
const pointer = (place: { clientX: number; clientY: number }, extra = {}) => ({ button: 0, pointerId: 1, ...place, ...extra });
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement;
const press = (value: string, fields = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
const head = (node: string, part: string) => document.querySelector<HTMLButtonElement>(`.color-kf-head[data-track="${node}"] [data-test="${part}"]`)!;
const items = () => screen.getAllByRole('menuitem').map(each => [each.getAttribute('aria-label'), (each as HTMLButtonElement).disabled]);

describe('the Keyframe Editor', () => {
  it('draws a master track and one for each node, a diamond for a dynamic keyframe and a circle for a static one', () => {
    const { rerender, props } = wire();
    expect(screen.getByRole('toolbar', { name: 'Keyframe tools' })).toBeDefined();
    expect([...document.querySelectorAll('.color-kf-name')].map(each => each.textContent)).toEqual(['Master', '01 Corrector', '02 Corrector', '03 Parallel Mixer']);
    expect(document.querySelector('.color-kf-head[data-current]')!.getAttribute('data-track')).toBe('n2');
    // The master track shows every keyframe of every track.
    expect(marksOn(0)).toEqual([['path', 20, 'dynamic'], ['path', 40, 'dynamic'], ['circle', 60, 'static']]);
    expect(marksOn(1)).toEqual([['path', 20, 'dynamic'], ['circle', 60, 'static']]);
    expect(marksOn(2)).toEqual([['path', 40, 'dynamic']]);
    expect(marksOn(3)).toEqual([]);
    // Each mark at its frame along the lanes, in the middle of its track.
    const placed = (row: number, frame: number) => `translate(${Number(xOf(frame).toFixed(2)) + 0} ${rowMiddle(row)})`;
    expect(markAt(1, 20).getAttribute('transform')).toBe(placed(1, 20));
    expect(markAt(0, 60).getAttribute('transform')).toBe(placed(0, 60));
    expect(markAt(2, 40).getAttribute('data-node')).toBe('n2');
    expect(markAt(0, 40).hasAttribute('data-node')).toBe(false);
    // The playhead is at frame 30 of the timeline: the picture 20 of the clip.
    const playhead = () => document.querySelector('[data-test="kf-playhead"]')!;
    expect(playhead().getAttribute('data-frame')).toBe('20');
    expect(document.querySelector('[data-test="kf-frame"]')!.textContent).toBe('20');
    // As the timeline plays, the playhead goes on and the marks stay the elements that they were.
    const mark = markAt(1, 20);
    rerender(<Harness {...props} frame={31} />);
    expect(playhead().getAttribute('data-frame')).toBe('21');
    expect(markAt(1, 20)).toBe(mark);
  });

  it('says so where there is no grade to show', () => {
    wire({ grade: null });
    expect(document.querySelector('[data-test="no-keyframes"]')!.textContent).toBe('There is no clip at the playhead.');
    expect(document.querySelector('[data-test="kf-lanes"]')).toBeNull();
  });

  it('adds and deletes keyframes at the playhead from its tool bar: on every track in the All mode, on the current node in Color', async () => {
    const { call, rerender, props } = wire({ frame: 45 });
    // Frame 45 of the timeline is the picture 35, where no track has a keyframe.
    expect(button('Delete Keyframe').disabled).toBe(true);
    fireEvent.click(button('Add Dynamic Keyframe'));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: addKeyframes(TARGET, GRADE.nodes, 35, 'dynamic') }]);
    expect(applied(call)[0].ops).toHaveLength(2 * PLAIN);
    // The Color mode: the current node alone.
    const mode = screen.getByRole('combobox', { name: 'Keyframe mode' }) as HTMLSelectElement;
    expect([mode.value, [...mode.options].map(each => each.textContent)]).toEqual(['all', ['All', 'Color']]);
    fireEvent.change(mode, { target: { value: 'color' } });
    fireEvent.click(button('Add Static Keyframe'));
    await settle();
    expect(applied(call)[1]).toEqual({ revision: 17, ops: addKeyframes(TARGET, [n2], 35, 'static') });
    // Delete Keyframe is for the keyframe at the playhead, on the tracks of the mode.
    rerender(<Harness {...props} frame={50} />);
    expect(button('Delete Keyframe').disabled).toBe(false);
    fireEvent.click(button('Delete Keyframe'));
    await settle();
    expect(applied(call)[2]).toEqual({ revision: 17, ops: deleteKeyframes(TARGET, [n2], 40) });
    fireEvent.click(button('Delete All Keyframes'));
    await settle();
    expect(applied(call)[3]).toEqual({ revision: 17, ops: deleteAllKeyframes(TARGET, [n2], 40) });
    // Back in the All mode, at the picture 20: the keyframe of 01, which the current node has not.
    rerender(<Harness {...props} frame={30} />);
    expect(button('Delete Keyframe').disabled).toBe(true);
    fireEvent.change(mode, { target: { value: 'all' } });
    expect(button('Delete Keyframe').disabled).toBe(false);
    fireEvent.click(button('Delete Keyframe'));
    await settle();
    expect(applied(call)[4]).toEqual({ revision: 17, ops: deleteKeyframes(TARGET, GRADE.nodes, 20) });
    expect(applied(call)).toHaveLength(5);
  });

  it('takes Ctrl+[, Ctrl+] and Alt+] from the page, and goes to the keyframe before and after with [ and ]', async () => {
    const { call, onSeek, rerender, props } = wire({ frame: 45 });
    press('[', { code: 'BracketLeft', ctrlKey: true });
    await settle();
    press(']', { code: 'BracketRight', ctrlKey: true });
    await settle();
    expect(applied(call).map(each => each.ops)).toEqual([addKeyframes(TARGET, GRADE.nodes, 35, 'dynamic'), addKeyframes(TARGET, GRADE.nodes, 35, 'static')]);
    rerender(<Harness {...props} frame={30} />);
    press(']', { code: 'BracketRight', altKey: true });
    await settle();
    expect(applied(call)[2].ops).toEqual(deleteKeyframes(TARGET, GRADE.nodes, 20));
    // The keyframes of the three tracks are shown at frames 30, 50 and 70 of the timeline.
    press(']', { code: 'BracketRight' });
    expect(onSeek.mock.calls).toEqual([[50, false]]);
    press('[', { code: 'BracketLeft' });
    expect(onSeek).toHaveBeenCalledTimes(1);
    rerender(<Harness {...props} frame={55} />);
    press('[', { code: 'BracketLeft' });
    expect(onSeek.mock.calls.at(-1)).toEqual([50, false]);
    fireEvent.click(button('Next Keyframe'));
    expect(onSeek.mock.calls.at(-1)).toEqual([70, false]);
    // In the Color mode, the keyframes of the current node alone.
    fireEvent.change(screen.getByRole('combobox', { name: 'Keyframe mode' }), { target: { value: 'color' } });
    fireEvent.click(button('Previous Keyframe'));
    expect(onSeek.mock.calls.at(-1)).toEqual([50, false]);
    const before = onSeek.mock.calls.length;
    fireEvent.click(button('Next Keyframe'));
    expect(onSeek).toHaveBeenCalledTimes(before);
    // A key that goes into a field is the field's.
    fireEvent.keyDown(screen.getByRole('combobox', { name: 'Keyframe mode' }), { key: '[', code: 'BracketLeft', ctrlKey: true });
    await settle();
    expect(applied(call)).toHaveLength(3);
  });

  it('changes a keyframe to static or dynamic and deletes it from the menu of its mark, and keys a track from the menu of the track', async () => {
    const { call } = wire();
    fireEvent.contextMenu(lanes(), at(1, 20));
    expect(screen.getByRole('menu', { name: 'Keyframe menu' })).toBeDefined();
    expect(items()).toEqual([['Change to Static Keyframe', false], ['Change Dynamic Attributes', false], ['Delete Keyframe', false]]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change to Static Keyframe' }));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: changeKeyframes(TARGET, [n1], 20, 'static') }]);
    expect(screen.queryByRole('menu')).toBeNull();
    // A static keyframe, the last of its node: it can be made dynamic, and has no stretch to ease.
    fireEvent.contextMenu(lanes(), at(1, 60));
    expect(items()).toEqual([['Change to Dynamic Keyframe', false], ['Change Dynamic Attributes', true], ['Delete Keyframe', false]]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change to Dynamic Keyframe' }));
    await settle();
    expect(applied(call)[1].ops).toEqual(changeKeyframes(TARGET, [n1], 60, 'dynamic'));
    // A mark of the master track stands for the keyframes of every node at its frame.
    fireEvent.contextMenu(lanes(), at(0, 40));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete Keyframe' }));
    await settle();
    expect(applied(call)[2].ops).toEqual(deleteKeyframes(TARGET, GRADE.nodes, 40));
    expect(applied(call)[2].ops).toEqual(deleteKeyframes(TARGET, [n2], 40));
    // A track where no mark is: a keyframe at the playhead (the picture 20) on that track, or all of its keyframes away.
    fireEvent.contextMenu(lanes(), at(2, 80));
    expect(screen.getByRole('menu', { name: 'Keyframe track menu' })).toBeDefined();
    expect(screen.getAllByRole('menuitem').map(each => [each.getAttribute('aria-label'), each.getAttribute('aria-keyshortcuts')]))
      .toEqual([['Add Dynamic Keyframe', 'Ctrl+['], ['Add Static Keyframe', 'Ctrl+]'], ['Delete All Keyframes', null]]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Static Keyframe' }));
    await settle();
    expect(applied(call)[3].ops).toEqual(addKeyframes(TARGET, [n2], 20, 'static'));
    fireEvent.contextMenu(lanes(), at(2, 80));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delete All Keyframes' }));
    await settle();
    expect(applied(call)[4].ops).toEqual(deleteAllKeyframes(TARGET, [n2], 20));
    // A track without keyframes has none to delete; the ruler has no menu.
    fireEvent.contextMenu(lanes(), at(3, 80));
    expect(items()[2]).toEqual(['Delete All Keyframes', true]);
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    fireEvent.contextMenu(lanes(), { clientX: 200, clientY: 6 });
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('sets the two eases of a dynamic keyframe in Change Dynamic Attributes, with the curve that they make', async () => {
    const { call, unmount } = wire();
    fireEvent.contextMenu(lanes(), at(1, 20));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Dynamic Attributes' }));
    const dialog = screen.getByRole('dialog', { name: 'Change Dynamic Attributes' });
    const start = dialog.querySelector<HTMLInputElement>('[data-test="kf-ease-start"]')!;
    const end = dialog.querySelector<HTMLInputElement>('[data-test="kf-ease-end"]')!;
    const curve = () => dialog.querySelector('.color-kf-ease-path')!.getAttribute('d');
    const box = { width: 160, height: 90 };
    // No ease at first: a line.
    expect([start.value, end.value, curve()]).toEqual(['0', '0', easePath(0, 0, box)]);
    fireEvent.change(start, { target: { value: '2' } });
    fireEvent.change(end, { target: { value: '1' } });
    expect([start.value, end.value, curve()]).toEqual(['2', '1', easePath(2, 1, box)]);
    fireEvent.click(dialog.querySelector('[data-test="kf-ease-apply"]')!);
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: easeKeyframes(TARGET, [n1], 20, 2, 1) }]);
    expect(screen.queryByRole('dialog')).toBeNull();
    unmount();
    // A keyframe that has eases shows them; Cancel changes nothing.
    const eased: GradeNode = { ...n1, params: paramsWith(n1.params, settingsOf(n1).map(each => ({ path: each.path, param: withEaseAt(each.param, 20, 3, 1.5) }))) } as GradeNode;
    const again = wire({ grade: { ...GRADE, nodes: [eased, n2, n3] } });
    fireEvent.contextMenu(lanes(), at(1, 20));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Dynamic Attributes' }));
    const shown = screen.getByRole('dialog', { name: 'Change Dynamic Attributes' });
    expect([...shown.querySelectorAll<HTMLInputElement>('input[type="range"]')].map(each => each.value)).toEqual(['3', '1.5']);
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await settle();
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(applied(again.call)).toEqual([]);
  });

  it('makes a drag of a mark one gesture: the settings whole at each move, no further than the next keyframe, and the end as it is let go', async () => {
    const { call } = wire();
    const from = at(1, 20);
    fireEvent.pointerDown(lanes(), pointer(from));
    // A pointer that has hardly moved drags nothing.
    fireEvent.pointerMove(lanes(), pointer({ clientX: from.clientX + 2, clientY: from.clientY }));
    await settle();
    expect(sent(call)).toHaveLength(0);
    fireEvent.pointerMove(lanes(), pointer({ clientX: from.clientX + 60, clientY: from.clientY }));
    await settle();
    const to = frameAtX(RANGE, LANES_WIDTH, from.clientX + 60);
    expect(to).toBeGreaterThan(20);
    expect(sent(call)).toHaveLength(1);
    const [, first, context] = sent(call)[0];
    expect(context).toEqual({ expectedSession: 1 });
    expect(first).toEqual({ revision: 17, ops: moveKeyframe(TARGET, n1, 20, to), gesture: { id: expect.any(String) } });
    expect(first.ops).toHaveLength(PLAIN);
    // The keyframe is drawn where the drag has it, on its track and on the master track.
    expect(marksOn(1)).toEqual([['path', to, 'dynamic'], ['circle', 60, 'static']]);
    expect(marksOn(0).map(each => each[1])).toEqual([to, 40, 60]);
    // It does not pass the keyframe after it.
    fireEvent.pointerMove(lanes(), pointer({ clientX: from.clientX + 400, clientY: from.clientY }));
    await settle();
    expect(sent(call)).toHaveLength(2);
    expect(applied(call)[1]).toEqual({ revision: 17, ops: moveKeyframe(TARGET, n1, 20, 59), gesture: { id: first.gesture.id } });
    fireEvent.pointerUp(lanes(), pointer({ clientX: from.clientX + 400, clientY: from.clientY }));
    await settle();
    expect(sent(call)).toHaveLength(3);
    expect(applied(call)[2]).toEqual({ revision: 17, ops: moveKeyframe(TARGET, n1, 20, 59), gesture: { id: first.gesture.id, end: true } });
  });

  it('puts the keyframe back where a drag is taken back, drags a mark of the master track for its nodes, and leaves a locked node alone', async () => {
    const { call, unmount } = wire();
    const from = at(0, 40);
    fireEvent.pointerDown(lanes(), pointer(from));
    fireEvent.pointerMove(lanes(), pointer({ clientX: from.clientX - 50, clientY: from.clientY }));
    await settle();
    const to = frameAtX(RANGE, LANES_WIDTH, from.clientX - 50);
    // The master track's mark at 40 is the keyframe of 02 alone.
    expect(applied(call)[0].ops).toEqual(moveKeyframe(TARGET, n2, 40, to));
    fireEvent.keyDown(panel(), { key: 'Escape' });
    await settle();
    expect(sent(call)).toHaveLength(2);
    expect(applied(call)[1]).toEqual({ revision: 17, ops: moveKeyframe(TARGET, n2, 40, 40), gesture: { id: applied(call)[0].gesture!.id, end: true } });
    expect(marksOn(2)).toEqual([['path', 40, 'dynamic']]);
    fireEvent.pointerUp(lanes(), pointer(from));
    await settle();
    expect(sent(call)).toHaveLength(2);
    unmount();
    const locked = wire({ grade: { ...GRADE, nodes: [{ ...n1, locked: true }, n2, n3] } });
    const mark = at(1, 20);
    fireEvent.pointerDown(lanes(), pointer(mark));
    fireEvent.pointerMove(lanes(), pointer({ clientX: mark.clientX + 60, clientY: mark.clientY }));
    fireEvent.pointerUp(lanes(), pointer({ clientX: mark.clientX + 60, clientY: mark.clientY }));
    await settle();
    expect(sent(locked.call)).toHaveLength(0);
    expect(marksOn(1)).toEqual([['path', 20, 'dynamic'], ['circle', 60, 'static']]);
  });

  it('moves the playhead with a press on the ruler and a drag along it, and to a mark that is clicked, which is chosen', () => {
    const { onSeek } = wire();
    const there = { clientX: Math.round(xOf(50)), clientY: 6 };
    fireEvent.pointerDown(lanes(), pointer(there));
    fireEvent.pointerMove(lanes(), pointer({ clientX: Math.round(xOf(70)), clientY: 40 }));
    fireEvent.pointerUp(lanes(), pointer({ clientX: Math.round(xOf(70)), clientY: 40 }));
    // The picture 50 is shown at frame 60 of the timeline, and the picture 70 at frame 80.
    expect(onSeek.mock.calls).toEqual([[60, true], [80, true], [80, false]]);
    expect(markAt(1, 60).hasAttribute('data-selected')).toBe(false);
    fireEvent.pointerDown(lanes(), pointer(at(1, 60)));
    fireEvent.pointerUp(lanes(), pointer(at(1, 60)));
    expect(onSeek.mock.calls.at(-1)).toEqual([70, false]);
    expect(markAt(1, 60).getAttribute('data-selected')).toBe('true');
    expect(markAt(0, 60).hasAttribute('data-selected')).toBe(false);
    // A press where no mark is lets go of it.
    fireEvent.pointerDown(lanes(), pointer(at(3, 80)));
    fireEvent.pointerUp(lanes(), pointer(at(3, 80)));
    expect(markAt(1, 60).hasAttribute('data-selected')).toBe(false);
  });

  it('has, in the head of a track, the name of its node, which makes it current, and its switches', async () => {
    const { call, onCurrent } = wire();
    fireEvent.click(head('n1', 'kf-track-name'));
    expect(onCurrent.mock.calls).toEqual([['n1']]);
    expect(head('n2', 'kf-track-name').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(head('n1', 'kf-lock'));
    await settle();
    fireEvent.click(head('n2', 'kf-enable'));
    await settle();
    expect(applied(call).map(each => each.ops)).toEqual([
      [{ op: 'set_node', grade: TARGET, id: 'n1', locked: true }],
      [{ op: 'set_node', grade: TARGET, id: 'n2', enabled: false }],
    ]);
    // Auto-Keyframe is of the track, and is kept by the page; a mixer has nothing to key.
    expect([head('n1', 'kf-auto').getAttribute('aria-pressed'), head('n2', 'kf-auto').getAttribute('aria-pressed')]).toEqual(['false', 'false']);
    fireEvent.click(head('n1', 'kf-auto'));
    expect([head('n1', 'kf-auto').getAttribute('aria-pressed'), head('n2', 'kf-auto').getAttribute('aria-pressed')]).toEqual(['true', 'false']);
    fireEvent.click(head('n1', 'kf-auto'));
    expect(head('n1', 'kf-auto').getAttribute('aria-pressed')).toBe('false');
    expect(head('n3', 'kf-auto').disabled).toBe(true);
    expect(applied(call)).toHaveLength(2);
  });

  it('counts the frames of a clip that plays backward by its clock: a keyframe is put at the picture shown', async () => {
    const backward: KeySpan = { first: 10, last: 69, clock: keyClockOf(clip('c1', 10, 60, { reverse: true }), RATE, media) };
    const { call, onSeek } = wire({ span: backward, frame: 20 });
    // Frame 20 of the timeline shows the picture 49; the clip shows the pictures 59 down to 0.
    expect(document.querySelector('[data-test="kf-playhead"]')!.getAttribute('data-frame')).toBe('49');
    expect(marksOn(0)).toEqual([['path', 20, 'dynamic'], ['path', 40, 'dynamic']]);
    expect(markAt(1, 20).getAttribute('transform')).toBe(`translate(${Number(xOf(20, [0, 59]).toFixed(2)) + 0} ${rowMiddle(1)})`);
    fireEvent.click(button('Add Dynamic Keyframe'));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: addKeyframes(TARGET, GRADE.nodes, 49, 'dynamic') }]);
    // The next keyframe along the timeline is the one at the picture 40, shown at frame 29.
    press(']', { code: 'BracketRight' });
    expect(onSeek.mock.calls).toEqual([[29, false]]);
  });
});

describe('Auto-Keyframe of a track', () => {
  type Sent = { revision: number; ops: Record<string, unknown>[]; gesture?: { id: string; end?: boolean } };
  const saturation = (value: number) => [['primaries.saturation', value]] as const;

  /** The palettes' editing of a node at frame 30 of its keys, with an engine whose answers wait until they are let go. */
  function mountEdit(given: Partial<EditedNode> = {}, node: GradeNode = corrector('n1')) {
    let revision = 10;
    const waiting: { answer: () => void }[] = [];
    const bridge = {
      call: vi.fn((method: string) => new Promise<unknown>((resolve) => {
        if (method !== 'transaction.apply') { resolve({ ok: true, result: {} }); return; }
        waiting.push({ answer: () => { revision += 1; resolve({ ok: true, result: { revision, done: [], next_id: 9, names: {}, clamped: [] } }); } });
      })),
    };
    const edited: EditedNode = { bridge: bridge as unknown as EditedNode['bridge'], revision: 10, target: TARGET, node, frame: 30, onFailure: vi.fn(), ...given };
    const hook = renderHook(({ at: edit }) => useNodeEdit(edit), { initialProps: { at: edited } });
    return {
      hook, node,
      sent: () => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(call => (call as unknown[])[1] as Sent),
      /** Answer the transaction that waits longest. */
      answer: async () => { waiting.shift()!.answer(); await settle(); },
    };
  }
  const setNode = (params: unknown) => ({ op: 'set_node', grade: TARGET, id: 'n1', params });

  it('puts a dynamic keyframe of the node before a change at a frame without one, in the transaction of the change', () => {
    const edit = mountEdit({ autoKey: true });
    act(() => edit.hook.result.current.commit(saturation(60)));
    expect(edit.sent()).toHaveLength(1);
    const [only] = edit.sent();
    expect(only.gesture).toBeUndefined();
    expect(only.revision).toBe(10);
    // One keyframe, a key on each setting, then the change, which goes into its key.
    expect(only.ops).toHaveLength(PLAIN + 1);
    expect(only.ops).toEqual([...addKeyframe(TARGET, edit.node, 30, 'dynamic'), setNode({ primaries: { saturation: { keys: [key(30, 60)] } } })]);
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(60);
    expect(edit.hook.result.current.keyed('primaries.saturation')).toBe(true);
  });

  it('changes nothing of today where it is off, where the timeline plays, and where the node has a keyframe at the frame', () => {
    const off = mountEdit();
    act(() => off.hook.result.current.commit(saturation(60)));
    expect(off.sent().map(each => each.ops)).toEqual([[setNode({ primaries: { saturation: 60 } })]]);
    const playing = mountEdit({ autoKey: true, playing: true });
    act(() => playing.hook.result.current.commit(saturation(60)));
    expect(playing.sent().map(each => each.ops)).toEqual([[setNode({ primaries: { saturation: 60 } })]]);
    // The node has a keyframe at 30: the change goes into it, and no other is put.
    const there = mountEdit({ autoKey: true }, withKeyframe(corrector('n1'), 30, 'dynamic'));
    act(() => there.hook.result.current.commit(saturation(60)));
    expect(there.sent().map(each => each.ops)).toEqual([[setNode({ primaries: { saturation: { keys: [key(30, 60)] } } })]]);
    // A value given back as it was shown changes nothing, and puts no keyframe.
    const same = mountEdit({ autoKey: true });
    act(() => same.hook.result.current.commit(saturation(50)));
    expect(same.sent()).toHaveLength(0);
  });

  it('leaves a setting that was given back as it was shown at what it is at the frame, between its keys', () => {
    const animated: GradeNode = { ...corrector('n1'), params: fullCorrector({ primaries: { saturation: { keys: [key(0, 50), key(100, 100)] } } }) as Corrector } as GradeNode;
    const edit = mountEdit({ autoKey: true, frame: 50 }, animated);
    // The control of the saturation shows 50, the key at its left, and gives it back with the change of the contrast.
    expect(edit.hook.result.current.value('primaries.saturation')).toBe(50);
    act(() => edit.hook.result.current.commit([['primaries.saturation', 50], ['primaries.contrast', 1.2]]));
    expect(edit.sent()).toHaveLength(1);
    expect(edit.sent()[0].ops).toEqual([...addKeyframe(TARGET, animated, 50, 'dynamic'), setNode({ primaries: {
      saturation: { keys: [key(0, 50), key(50, 75), key(100, 100)] }, contrast: { keys: [key(50, 1.2)] },
    } })]);
  });

  it('puts the keyframe with the first transaction of a drag, which stays one gesture, and takes it back with the drag', async () => {
    const edit = mountEdit({ autoKey: true });
    act(() => edit.hook.result.current.preview(saturation(60)));
    expect(edit.sent()).toHaveLength(1);
    const [first] = edit.sent();
    expect(first.ops).toEqual([...addKeyframe(TARGET, edit.node, 30, 'dynamic'), setNode({ primaries: { saturation: { keys: [key(30, 60)] } } })]);
    expect(first.gesture).toEqual({ id: expect.any(String) });
    await edit.answer();
    // The moves after the first change the key alone.
    act(() => edit.hook.result.current.preview(saturation(70)));
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1].ops).toEqual([setNode({ primaries: { saturation: { keys: [key(30, 70)] } } })]);
    expect(edit.sent()[1].gesture).toEqual({ id: first.gesture!.id });
    act(() => edit.hook.result.current.commit(saturation(80)));
    await edit.answer();
    expect(edit.sent()).toHaveLength(3);
    expect(edit.sent()[2].ops).toEqual([setNode({ primaries: { saturation: { keys: [key(30, 80)] } } })]);
    expect(edit.sent()[2].gesture).toEqual({ id: first.gesture!.id, end: true });
    // Escape: the settings are put back whole, so the keyframe goes with the change.
    const back = mountEdit({ autoKey: true });
    act(() => back.hook.result.current.preview(saturation(60)));
    await back.answer();
    act(() => back.hook.result.current.cancel());
    await settle();
    const last = back.sent().at(-1)!;
    expect(last.gesture!.end).toBe(true);
    expect(last.ops).toEqual([setNode(fullCorrector({}))]);
    expect(back.hook.result.current.value('primaries.saturation')).toBe(50);
  });
});

describe('the keyframes of the Color page', () => {
  const C1: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
  const graph = () => document.querySelector<SVGElement>('[data-test="grade-graph"]')!;
  const badged = () => [...graph().querySelectorAll<SVGElement>('[data-node]')].filter(each => each.querySelector('[data-test="node-keyed"]')).map(each => each.dataset.node);

  it('take Ctrl+[ whichever panel is shown, badge the nodes that have keyframes, and show them in the Keyframes panel', async () => {
    const { bridge } = await mount(projectOf(), { page: 'Color' });
    expect(badged()).toEqual([]);
    // The playhead is at the first frame of the first clip, whose grade has two correctors and a mixer; the Scopes panel is shown.
    press('[', { code: 'BracketLeft', ctrlKey: true });
    await settlePage();
    expect(sentOps(bridge).at(-1)).toEqual(addKeyframes(C1, lutGrade().nodes, 0, 'dynamic'));
    expect(badged()).toEqual(['n1', 'n2']);
    fireEvent.click(screen.getByRole('tab', { name: 'Keyframes' }));
    await settlePage();
    expect(document.querySelector('[data-test="keyframes-panel"]')).not.toBeNull();
    expect([0, 1, 2, 3].map(marksOn)).toEqual([[['path', 0, 'dynamic']], [['path', 0, 'dynamic']], [['path', 0, 'dynamic']], []]);
    // Alt+] takes the keyframe away again, and the badges with it.
    press(']', { code: 'BracketRight', altKey: true });
    await settlePage();
    expect(sentOps(bridge).at(-1)).toEqual(deleteKeyframes(C1, lutGrade().nodes.map(node => withKeyframe(node, 0, 'dynamic')), 0));
    expect(badged()).toEqual([]);
    expect(marksOn(0)).toEqual([]);
  });
});
