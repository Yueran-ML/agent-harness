// @vitest-environment jsdom
/**
 * The Retime curves of a clip in the Curve Editor of the Edit page (`src/studio/keyframes/ClipCurves`
 * and the lines of `src/studio/keyframes/CurveEditor`; DaVinci Resolve Reference Manual 20,
 * pp. 1078–1081): which clips have them, the points of the Retime Frame curve, what a drag of one up,
 * down and sideways sends, snapped and held to what a segment and the media allow, a speed point
 * added and cleared, a drag as one gesture and taken back, the steps of the Retime Speed curve, the
 * buttons that are off for them, the frames that the graph is over, and a clip on a locked track.
 *
 * The clip is at 25 frames a second and its media counts a tick a frame, so a place of the media
 * reads as a frame.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type Interp, type Keyframe, type Media, type Param, type Retime, type Timeline as TimelineModel } from '../src/studio/engine';
import { ClipCurves, RETIME_FRAME, RETIME_SPEED } from '../src/studio/keyframes/ClipCurves';
import { GRAPH_SIZE, plotRect } from '../src/studio/keyframes/CurveEditor';
import { curvePath, fitView, keyPoint, type View } from '../src/studio/keyframes/curveGeometry';
import { beginsLater, courseOf } from '../src/studio/retime';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); });

const rate = { num: 25, den: 1 };
/** The graph is not laid out in the DOM of the tests: it is as large as it is until it is measured. */
const rect = plotRect(GRAPH_SIZE);
const X = 'transform.x';
const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const retime = (...points: [number, number, number][]): Retime => ({ points: points.map(([frame, num, den]) => ({ frame, speed: { num, den } })) });
/** 20 frames as the source is, a picture held for 10, and 30 frames back at half the speed. */
const POINTS = retime([0, 1, 1], [20, 0, 1], [30, -1, 2]);
/** Where the clip begins in its media: 100 frames into it. */
const BEGINS = { kind: 'media' as const, media: 'm1', in: { ticks: 100, scale: 25 } };
/** c1 from frame 10 for 60 frames, along `POINTS`: it rises from frame 100 of its media to 120, stays, and falls to 105. */
const pointed = (fields: Partial<Clip> = {}) => clip('c1', 10, 60, { retime: POINTS, source: BEGINS, ...fields });
/** The control points of its Retime Frame curve, and the steps of its Retime Speed curve. */
const FRAME_KEYS = [key(0, 100), key(20, 120), key(30, 120), key(60, 105)];
const SPEED_KEYS = [key(0, 100, 'hold'), key(20, 0, 'hold'), key(30, -50, 'hold')];
const timelineOf = (clips: Clip[], locked = false): TimelineModel => ({
  id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});
type Call = ReturnType<typeof vi.fn>;
function wire(clips: Clip[], given: { locked?: boolean; media?: Media[] } = {}) {
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const onSeek = vi.fn();
  const props = {
    bridge: { call } as unknown as Pick<EngineBridge, 'call'>, session: 1, revision: 17, timeline: timelineOf(clips, given.locked), media: given.media ?? [media],
    clips: ['c1'], frame: 30, onSeek,
  };
  const view = render(<ClipCurves {...props} />);
  return { ...view, props, call, onSeek };
}
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
/** What a move of a control point of the Retime Frame curve sends: the speed points whole, and the end of the path that stays in the media. */
const setPoints = (points: Retime, keep: 'start' | 'end') => ({ edit: 'set_speed_points', timeline: 't1', clip: 'c1', retime: points, keep });

const editor = () => document.querySelector<HTMLElement>('[data-test="curve-editor"]')!;
const graph = () => document.querySelector<SVGSVGElement>('.curve-editor-graph')!;
const keyOf = (curve: string, index: number) => document.querySelector<SVGPathElement>(`.curve-key[data-curve="${curve}"][data-key="${index}"]`)!;
const lineOf = (curve: string) => document.querySelector<SVGPathElement>(`.curve-line[data-curve="${curve}"]`);
const framesOf = (curve: string) => [...document.querySelectorAll(`.curve-key[data-curve="${curve}"]`)].map(each => Number(each.getAttribute('data-frame')));
const chosen = (curve: string) => [...document.querySelectorAll(`.curve-key[data-curve="${curve}"][data-selected="true"]`)].map(each => Number(each.getAttribute('data-key')));
const boxes = () => (screen.getAllByRole('checkbox') as HTMLInputElement[]).map(box => [box.getAttribute('aria-label'), box.checked]);
const tip = () => document.querySelector('.curve-editor-tip')?.textContent ?? null;
const playhead = () => document.querySelector('.curve-editor-playhead')?.getAttribute('data-frame') ?? null;
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
/** Press a point, drag it by so many pixels, and keep it there. */
const drag = (curve: string, index: number, dx: number, dy: number) => {
  const from = at(curve, index);
  fireEvent.pointerDown(graph(), pointer(from));
  fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + dx, clientY: from.clientY + dy }));
  return from;
};
const release = () => fireEvent.pointerUp(graph(), pointer({ clientX: 0, clientY: 0 }));
/**
 * The view of a curve as the editor fits it: the frames of the clip with those of the curves that
 * are drawn with it, and the values of the curve itself. In it 40 pixels up are 5.05 frames of the
 * source on the Retime Frame curve of `pointed`, and 40 pixels along are 5 frames of the clip.
 */
const viewOf = (param: Param, drawn: Param[] = [{ keys: FRAME_KEYS }, { keys: SPEED_KEYS }], length = 60): View => {
  const frames = fitView(drawn, [0, length]);
  const values = fitView([param]);
  return { from: frames.from, to: frames.to, low: values.low, high: values.high };
};
/** Where the editor draws a point of the Retime Frame curve of `pointed`. */
const drawnAt = (frame: number, value: number) => {
  const point = keyPoint(key(frame, value), viewOf({ keys: FRAME_KEYS }), rect);
  return `translate(${point.x} ${point.y})`;
};

describe('the Retime curves of a clip in the Curve Editor', () => {
  it('are listed with the settings and drawn over the frames of the clip: a point at each speed point and at the end of the clip', () => {
    wire([pointed()]);
    // The clip has speed points: its two curves are shown from the start, before the ten settings, none of which has keys.
    expect(boxes().slice(0, 2)).toEqual([['Retime Frame', true], ['Retime Speed', true]]);
    expect(boxes()).toHaveLength(12);
    expect(boxes().slice(2).every(([, checked]) => checked === false)).toBe(true);
    // The Retime Frame curve is the one edited: it rises with the forward segment, is level in the freeze, and falls where the clip goes backward.
    expect(lineOf(RETIME_FRAME)!.getAttribute('data-active')).toBe('true');
    expect(lineOf(RETIME_FRAME)!.getAttribute('d')).toBe(curvePath({ keys: FRAME_KEYS }, viewOf({ keys: FRAME_KEYS }), rect));
    expect(framesOf(RETIME_FRAME)).toEqual([0, 20, 30, 60]);
    expect([0, 1, 2, 3].map(index => keyOf(RETIME_FRAME, index).getAttribute('transform'))).toEqual([drawnAt(0, 100), drawnAt(20, 120), drawnAt(30, 120), drawnAt(60, 105)]);
    // The Retime Speed curve is a step for each segment, with a point where the segment begins.
    expect(lineOf(RETIME_SPEED)!.getAttribute('data-active')).toBe('false');
    expect(lineOf(RETIME_SPEED)!.getAttribute('d')).toBe(curvePath({ keys: SPEED_KEYS }, viewOf({ keys: SPEED_KEYS }), rect));
    expect(framesOf(RETIME_SPEED)).toEqual([0, 20, 30]);
    // The playhead is at frame 30 of the timeline: frame 20 of the clip.
    expect(playhead()).toBe('20');
  });

  it('are those of a clip without speed points too, listed last and hidden until one is asked for', () => {
    wire([clip('c1', 10, 60, { source: BEGINS })]);
    expect(boxes().slice(-2)).toEqual([['Retime Frame', false], ['Retime Speed', false]]);
    expect(lineOf(RETIME_FRAME)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Retime Frame' }));
    // One segment at the speed of the source: a straight line from where the clip begins in its media to 60 frames on.
    const line: Param = { keys: [key(0, 100), key(60, 160)] };
    expect(lineOf(RETIME_FRAME)!.getAttribute('d')).toBe(curvePath(line, viewOf(line, [line]), rect));
    expect(framesOf(RETIME_FRAME)).toEqual([0, 60]);
    expect(lineOf(RETIME_SPEED)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    expect(framesOf(RETIME_SPEED)).toEqual([0]);
  });

  it('are not there for a title, a colour or a still, which have no speed', () => {
    const title = wire([clip('c1', 10, 60, { source: { kind: 'title', text: 'Hello', style: { font: 'Inter', size: 72, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } } })]);
    expect(boxes()).toHaveLength(10);
    expect(screen.queryByRole('button', { name: 'Retime Frame' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Retime Speed' })).toBeNull();
    title.unmount();
    const colour = wire([clip('c1', 10, 60, { source: { kind: 'color', color: '#000000ff' } })]);
    expect(boxes()).toHaveLength(10);
    colour.unmount();
    const still: Media = { ...media, facts: { ...media.facts, video: { ...media.facts.video!, still: true } } };
    wire([clip('c1', 10, 60)], { media: [still] });
    expect(boxes()).toHaveLength(10);
    expect(document.querySelector(`[data-curve="${RETIME_FRAME}"]`)).toBeNull();
  });

  it('are drawn over the frames of the clip, and the settings over those of their keys: the graph shows the ones of the curve that is edited', () => {
    const { onSeek } = wire([pointed({ transform: { ...NONE, x: { keys: [key(100, 0), key(110, 0.25)] } } })]);
    // A setting with keys is the one edited first, over the pictures of the media: the playhead is at the picture 120, which the freeze holds.
    expect(lineOf(X)!.getAttribute('data-active')).toBe('true');
    expect(lineOf(RETIME_FRAME)).toBeNull();
    expect(playhead()).toBe('120');
    fireEvent.click(screen.getByRole('button', { name: 'Retime Frame' }));
    expect(lineOf(RETIME_FRAME)!.getAttribute('data-active')).toBe('true');
    expect(lineOf(RETIME_SPEED)).not.toBeNull();
    expect(lineOf(X)).toBeNull();
    expect(playhead()).toBe('20');
    // A press on the ruler over frame 20 of the clip moves the playhead to frame 30 of the timeline.
    const over = { clientX: at(RETIME_FRAME, 1).clientX, clientY: 6 };
    fireEvent.pointerDown(graph(), pointer(over));
    fireEvent.pointerUp(graph(), pointer(over));
    expect(onSeek.mock.calls).toEqual([[30, false]]);
    // Back on the setting, the graph is over the pictures again.
    fireEvent.click(screen.getByRole('button', { name: 'Position X' }));
    expect(lineOf(X)!.getAttribute('data-active')).toBe('true');
    expect(lineOf(RETIME_FRAME)).toBeNull();
    expect(playhead()).toBe('120');
  });
});

describe('a control point of the Retime Frame curve', () => {
  it('dragged up gives the segments either side other speeds, the frames of the speed points staying: one gesture of the speed points whole', async () => {
    const { call } = wire([pointed()]);
    // 40 px up are 5.05 frames of the source: 25.05 in the first 20 frames of the clip, at 125%, and back to where the freeze stopped at 50%.
    drag(RETIME_FRAME, 1, 0, -40);
    const raised = retime([0, 5, 4], [20, -1, 2], [30, -1, 2]);
    expect(tip()).toBe('Retime Frame 125% · −50%');
    // The graph draws the points where the engine then has them, whatever the timeline is given meanwhile: the second at the frame 125 of the media.
    expect(keyOf(RETIME_FRAME, 1).getAttribute('transform')).toBe(drawnAt(20, 125));
    expect(framesOf(RETIME_FRAME)).toEqual([0, 20, 30, 60]);
    expect(chosen(RETIME_FRAME)).toEqual([1]);
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [setPoints(raised, 'start')], gesture: { id: expect.any(String) } }]);
    release();
    await settle();
    // The last move ends the gesture: the whole drag is one step to undo.
    const sent = applied(call);
    expect(sent).toHaveLength(2);
    expect(sent[1]).toEqual({ revision: 17, ops: [setPoints(raised, 'start')], gesture: { id: sent[0].gesture.id, end: true } });
    expect(tip()).toBeNull();
  });

  it('is held to the speeds that a segment may have, and to what the media has', async () => {
    const { call } = wire([pointed()]);
    const from = drag(RETIME_FRAME, 1, 0, -20000);
    // Far up, the first segment plays as fast as one may, and the next comes back as fast.
    expect(tip()).toBe('Retime Frame 10000% · −10000%');
    await settle();
    expect(applied(call).at(-1).ops).toEqual([setPoints(retime([0, 100, 1], [20, -100, 1], [30, -1, 2]), 'start')]);
    const before = applied(call).length;
    // Far down, the clip would go back over more than its media has before where it begins: nothing is asked of the engine, and the point stays.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX, clientY: from.clientY + 2000 }));
    await settle();
    expect(applied(call)).toHaveLength(before);
    expect(tip()).toBe('Retime Frame 10000% · −10000%');
    // A little down it goes: 2.5 frames of the source lower, 17.5 in the first 20 frames, which the nearest hundredth makes 87%.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX, clientY: from.clientY + 20 }));
    await settle();
    expect(tip()).toBe('Retime Frame 87% · 26%');
    expect(applied(call).at(-1).ops).toEqual([setPoints(retime([0, 87, 100], [20, 13, 50], [30, -1, 2]), 'start')]);
  });

  it('dragged sideways moves the speed point in time, as the lower handle of the Retime Controls does', async () => {
    const { call } = wire([pointed()]);
    // 40 px along are 5 frames: the speed point goes from frame 20 to 25, the 20 frames of the source before it are shown in 25, at 80%, and the freeze stays one.
    const from = drag(RETIME_FRAME, 1, 40, 0);
    expect(tip()).toBe('Retime Frame 80% · 0%');
    expect(framesOf(RETIME_FRAME)).toEqual([0, 25, 30, 60]);
    expect(keyOf(RETIME_FRAME, 1).getAttribute('transform')).toBe(drawnAt(25, 120));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [setPoints(retime([0, 4, 5], [25, 0, 1], [30, -1, 2]), 'start')], gesture: { id: expect.any(String) } }]);
    // It stays strictly before the end of its segment, and strictly after the speed point before it.
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX + 2000, clientY: from.clientY }));
    expect(framesOf(RETIME_FRAME)).toEqual([0, 29, 30, 60]);
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX - 2000, clientY: from.clientY }));
    expect(framesOf(RETIME_FRAME)).toEqual([0, 1, 30, 60]);
    await settle();
    expect(applied(call).at(-1).ops).toEqual([setPoints(retime([0, 20, 1], [1, 0, 1], [30, -1, 2]), 'start')]);
  });

  it('moves up and down only where it is the first point or the end of the clip, and the first takes where the clip begins with it', async () => {
    const first = wire([pointed()]);
    // The first point 40 px up, and as far along: 5.05 frames higher, its segment goes 14.95 frames in its 20, at 75%.
    drag(RETIME_FRAME, 0, 40, -40);
    const raised = retime([0, 3, 4], [20, 0, 1], [30, -1, 2]);
    expect(tip()).toBe('Retime Frame 75%');
    expect(framesOf(RETIME_FRAME)).toEqual([0, 20, 30, 60]);
    await settle();
    // What follows its segment stays where it is in the media: the end of the path is what the engine keeps.
    expect(applied(first.call)).toEqual([{ revision: 17, ops: [setPoints(raised, 'end')], gesture: { id: expect.any(String) } }]);
    // At 75% the segment goes 15 frames: the clip begins 5 frames later in its media, by exactly what the point then shows more, and the other points stay.
    expect(beginsLater(courseOf(POINTS, 60)!, courseOf(raised, 60)!, 'end', rate, 25)).toBe(5n);
    expect([0, 1, 2, 3].map(index => keyOf(RETIME_FRAME, index).getAttribute('transform'))).toEqual([drawnAt(0, 105), drawnAt(20, 120), drawnAt(30, 120), drawnAt(60, 105)]);
    // Taken back, the speed points that the clip had are set with the same end kept, which puts the clip back where it began.
    fireEvent.keyDown(editor(), { key: 'Escape' });
    await settle();
    expect(applied(first.call).at(-1)).toEqual({ revision: 17, ops: [setPoints(POINTS, 'end')], gesture: { id: expect.any(String), end: true } });
    cleanup();
    // The end of the clip 40 px down and as far along: the last segment goes back 20.05 frames in its 30, at 67%, and the path begins where it did.
    const last = wire([pointed()]);
    drag(RETIME_FRAME, 3, 40, 40);
    expect(tip()).toBe('Retime Frame −67%');
    expect(framesOf(RETIME_FRAME)).toEqual([0, 20, 30, 60]);
    await settle();
    expect(applied(last.call)).toEqual([{ revision: 17, ops: [setPoints(retime([0, 1, 1], [20, 0, 1], [30, -67, 100]), 'start')], gesture: { id: expect.any(String) } }]);
  });

  it('is put back where it was by Escape, and the release after it does nothing more', async () => {
    const { call } = wire([pointed()]);
    drag(RETIME_FRAME, 1, 0, -40);
    await settle();
    fireEvent.keyDown(editor(), { key: 'Escape' });
    await settle();
    expect(applied(call)).toHaveLength(2);
    expect(applied(call)[1]).toEqual({ revision: 17, ops: [setPoints(POINTS, 'start')], gesture: { id: applied(call)[0].gesture.id, end: true } });
    // The graph draws the clip as it is again.
    expect(keyOf(RETIME_FRAME, 1).getAttribute('transform')).toBe(drawnAt(20, 120));
    expect(tip()).toBeNull();
    release();
    await settle();
    expect(applied(call)).toHaveLength(2);
  });

  it('is added by a double click on the line, and cleared with Delete where it is a speed point', async () => {
    const added = wire([pointed()]);
    const [first, second] = [at(RETIME_FRAME, 0), at(RETIME_FRAME, 1)];
    // Half way between the first two points is frame 10 of the clip.
    fireEvent.doubleClick(graph(), { clientX: Math.round((first.clientX + second.clientX) / 2), clientY: Math.round((first.clientY + second.clientY) / 2) });
    await settle();
    expect(applied(added.call)).toEqual([{ revision: 17, ops: [{ edit: 'add_speed_point', timeline: 't1', clip: 'c1', at: 10 }] }]);
    cleanup();
    // The two speed points after the first, chosen and deleted: one transaction that clears both.
    const cleared = wire([pointed()]);
    click(at(RETIME_FRAME, 1));
    click(at(RETIME_FRAME, 2), { ctrlKey: true });
    expect(chosen(RETIME_FRAME)).toEqual([1, 2]);
    fireEvent.keyDown(editor(), { key: 'Delete' });
    await settle();
    expect(applied(cleared.call)).toEqual([{ revision: 17, ops: [
      { edit: 'clear_speed_point', timeline: 't1', clip: 'c1', at: 20 }, { edit: 'clear_speed_point', timeline: 't1', clip: 'c1', at: 30 },
    ] }]);
    expect(chosen(RETIME_FRAME)).toEqual([]);
    cleanup();
    // The first point and the end of the clip are no speed points to clear, and a double click on a point adds none.
    const kept = wire([pointed()]);
    click(at(RETIME_FRAME, 0));
    click(at(RETIME_FRAME, 3), { ctrlKey: true });
    fireEvent.keyDown(editor(), { key: 'Delete' });
    fireEvent.doubleClick(graph(), at(RETIME_FRAME, 1));
    await settle();
    expect(applied(kept.call)).toEqual([]);
  });

  it('has no Bezier handles, and the buttons of interpolation, of copying and of pasting are off for it', async () => {
    const { call } = wire([pointed()]);
    click(at(RETIME_FRAME, 1));
    expect(chosen(RETIME_FRAME)).toEqual([1]);
    expect(document.querySelector('.curve-handle')).toBeNull();
    for (const name of ['Linear', 'Ease In', 'Ease Out', 'Ease In and Out', 'Hold']) {
      const button = screen.getByRole('button', { name }) as HTMLButtonElement;
      expect([name, button.disabled, button.getAttribute('aria-pressed')]).toEqual([name, true, 'false']);
    }
    expect(document.querySelector<HTMLButtonElement>('[data-test="curve-copy"]')!.disabled).toBe(true);
    expect(document.querySelector<HTMLButtonElement>('[data-test="curve-paste"]')!.disabled).toBe(true);
    // Neither do the keys of copying, of pasting and of nudging do anything to it.
    for (const pressed of ['c', 'x', 'v', 'ArrowRight']) fireEvent.keyDown(editor(), { key: pressed, ctrlKey: true });
    await settle();
    expect(applied(call)).toEqual([]);
    expect(chosen(RETIME_FRAME)).toEqual([1]);
  });

  it('is shown on a locked track, and nothing changes it', async () => {
    const { call } = wire([pointed()], { locked: true });
    expect(document.querySelector('[data-test="curves-locked"]')).not.toBeNull();
    expect(framesOf(RETIME_FRAME)).toEqual([0, 20, 30, 60]);
    drag(RETIME_FRAME, 1, 40, -40);
    expect(tip()).toBeNull();
    release();
    click(at(RETIME_FRAME, 1));
    expect(chosen(RETIME_FRAME)).toEqual([1]);
    fireEvent.keyDown(editor(), { key: 'Delete' });
    const [first, second] = [at(RETIME_FRAME, 0), at(RETIME_FRAME, 1)];
    fireEvent.doubleClick(graph(), { clientX: Math.round((first.clientX + second.clientX) / 2), clientY: Math.round((first.clientY + second.clientY) / 2) });
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    drag(RETIME_SPEED, 0, 0, -53);
    release();
    await settle();
    expect(applied(call)).toEqual([]);
  });
});

describe('a step of the Retime Speed curve', () => {
  it('dragged up or down gives its segment another speed as it is let go, the way that the segment goes', async () => {
    const faster = wire([pointed()]);
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    expect(lineOf(RETIME_SPEED)!.getAttribute('data-active')).toBe('true');
    // 53 px up are 50 hundredths: the first segment at 150%. A step does not move along.
    drag(RETIME_SPEED, 0, 30, -53);
    expect(tip()).toBe('Retime Speed 150%');
    expect(framesOf(RETIME_SPEED)).toEqual([0, 20, 30]);
    await settle();
    // Nothing is sent while it is dragged: the segment then lasts another length, which is one edit as it is let go.
    expect(applied(faster.call)).toEqual([]);
    release();
    await settle();
    expect(applied(faster.call)).toEqual([{ revision: 17, ops: [{ edit: 'set_segment_speed', timeline: 't1', clip: 'c1', at: 0, speed: { num: 3, den: 2 } }] }]);
    expect(tip()).toBeNull();
    cleanup();
    // The last segment goes backward at 50%: 53 px down, at 100%, backward still, and the speed is sent without its sign.
    const backward = wire([pointed()]);
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    drag(RETIME_SPEED, 2, 0, 53);
    expect(tip()).toBe('Retime Speed −100%');
    release();
    await settle();
    expect(applied(backward.call)).toEqual([{ revision: 17, ops: [{ edit: 'set_segment_speed', timeline: 't1', clip: 'c1', at: 30, speed: { num: 1, den: 1 } }] }]);
  });

  it('is held to the speeds that a segment may have, leaves a freeze as it is, and sends nothing where it is taken back or not moved', async () => {
    const slowest = wire([pointed()]);
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    // Far down, the segment plays as slowly as one may, forward still; far up, as fast.
    const from = drag(RETIME_SPEED, 0, 0, 100000);
    expect(tip()).toBe('Retime Speed 1%');
    fireEvent.pointerMove(graph(), pointer({ clientX: from.clientX, clientY: from.clientY - 100000 }));
    expect(tip()).toBe('Retime Speed 10000%');
    release();
    await settle();
    expect(applied(slowest.call)).toEqual([{ revision: 17, ops: [{ edit: 'set_segment_speed', timeline: 't1', clip: 'c1', at: 0, speed: { num: 100, den: 1 } }] }]);
    cleanup();
    const still = wire([pointed()]);
    fireEvent.click(screen.getByRole('button', { name: 'Retime Speed' }));
    // The freeze has no speed to change: its step does not move.
    drag(RETIME_SPEED, 1, 0, -53);
    expect(tip()).toBeNull();
    release();
    // A drag taken back with Escape, and one that goes along only, leave the segment its speed.
    drag(RETIME_SPEED, 0, 0, -53);
    fireEvent.keyDown(editor(), { key: 'Escape' });
    release();
    drag(RETIME_SPEED, 0, 30, 0);
    release();
    await settle();
    expect(applied(still.call)).toEqual([]);
  });
});
