/**
 * The geometry of the curve editor and what it does to keys (`src/studio/keyframes/curveGeometry`):
 * the view, the path of a setting, which is the curve that `valueAt` reads, where its keys and
 * handles are and what a pointer is on, and every edit as the keys that it leaves; the one splice
 * that makes keys into others, the operations that the Edit page sends for it, the frame of the
 * timeline of a frame of keys, and the bounds of a clip's settings, which are the engine's.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { CLIP_KEYED, type Interp, type Keyframe, type Param } from '../src/studio/engine';
import { CLIP_BOUNDS, frameOfKey, keyFrameOf, keyOps, keyTimes } from '../src/studio/inspector/clipKeys';
import {
  FLAT_FRAMES, HIT_PX, copyKeys, curveDistance, curvePath, deleteKeys, dragHandle, fitView, frameAt, handlePoints, handlesOf, hitCurve, hitHandle, hitKey,
  insertKey, interpIs, keyPoint, keysIn, moveKeys, panned, pasteKeys, pointOn, selectedIn, setInterp, spliceOf, stretchesOf, ticksOf, valueAtY, xOf, yOf, zoomed,
  type Rect, type Splice, type View,
} from '../src/studio/keyframes/curveGeometry';
import { EASE_SHARE, easeOf, eased, valueAt } from '../src/studio/keyframes/value';
import { clip, media } from './timeline-fixture';

const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const bezier = (out: [number, number], into: [number, number]): Interp => ({ bezier: { out, in: into } });
const clone = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
/** Frames 0 to 100 over 400 pixels from 50, values -1 to 1 over 200 pixels from 20: a frame is 4 pixels, and a value of 1 is 100. */
const rect: Rect = { left: 50, top: 20, width: 400, height: 200 };
const view: View = { from: 0, to: 100, low: -1, high: 1 };
const wide = { low: -100, high: 100 };
/** A level run in, a line, a step, a Bezier stretch, and a level run out. */
const mixed: Keyframe[] = [key(20, 0), key(40, 1, 'hold'), key(60, -1, bezier([0.5, 0], [0.25, 0.4])), key(80, 0)];
/** The keys with a splice applied, as the engine applies `splice_keys`. */
const spliced = (keys: readonly Keyframe[], splice: Splice | null) => (splice ? [...keys.slice(0, splice.at), ...splice.keys, ...keys.slice(splice.at + splice.remove)] : [...keys]);
/** What the engine takes of keys: frames that rise, handles whose shares are from 0 to 1. */
const taken = (keys: readonly Keyframe[]) => keys.length > 0 && keys.every((each, index) => Number.isInteger(each.frame) && Number.isFinite(each.value)
  && (index === 0 || keys[index - 1].frame < each.frame)
  && (typeof each.interp === 'string' || [each.interp.bezier.out, each.interp.bezier.in].every(handle => handle[0] >= 0 && handle[0] <= 1 && Number.isFinite(handle[1]))));

describe('the view of a curve editor', () => {
  it('puts frames from left to right and values from the bottom to the top, and reads them back', () => {
    expect([0, 50, 100].map(frame => xOf(view, rect, frame))).toEqual([50, 250, 450]);
    expect([1, 0, -1].map(value => yOf(view, rect, value))).toEqual([20, 120, 220]);
    expect(frameAt(view, rect, 250)).toBe(50);
    expect(valueAtY(view, rect, 70)).toBeCloseTo(0.5, 12);
    for (const frame of [-20, 0, 33.3, 100, 140]) expect(frameAt(view, rect, xOf(view, rect, frame))).toBeCloseTo(frame, 9);
    for (const value of [-3, -1, 0.123, 1, 2]) expect(valueAtY(view, rect, yOf(view, rect, value))).toBeCloseTo(value, 9);
  });

  it('fits keys with a tenth of their extent free at each side', () => {
    const fit = fitView([{ keys: [key(10, 0), key(50, 2)] }]);
    expect(fit.from).toBeCloseTo(6, 9);
    expect(fit.to).toBeCloseTo(54, 9);
    expect(fit.low).toBeCloseTo(-0.2, 9);
    expect(fit.high).toBeCloseTo(2.2, 9);
    // With no padding, the keys are at the edges; several settings are fitted together.
    expect(fitView([{ keys: [key(10, 0), key(50, 2)] }, { keys: [key(-5, 7)] }], [], 0)).toEqual({ from: -5, to: 50, low: 0, high: 7 });
  });

  it('opens a view of one key, of one value and of nothing', () => {
    expect(fitView([{ keys: [key(30, 4)] }])).toEqual({ from: 30 - FLAT_FRAMES, to: 30 + FLAT_FRAMES, low: 2, high: 6 });
    // A value within 1 of 0 has half of 1 above and below; a setting of one number has no frames of its own.
    expect(fitView([0.5])).toEqual({ from: 0, to: 2 * FLAT_FRAMES, low: 0, high: 1 });
    expect(fitView([])).toEqual({ from: 0, to: 2 * FLAT_FRAMES, low: -1, high: 1 });
    // The frames given are shown too: those that a clip shows.
    const shown = fitView([3], [0, 59], 0);
    expect([shown.from, shown.to]).toEqual([0, 59]);
    expect([shown.low, shown.high]).toEqual([1.5, 4.5]);
  });

  it('fits the handles of a Bezier stretch, which the curve lies within', () => {
    const over: Param = { keys: [key(0, 0, bezier([0.3, 0], [0.3, 0.5])), key(10, 1)] };
    const fit = fitView([over], [], 0);
    expect([fit.low, fit.high]).toEqual([0, 1.5]);
    // The curve goes above its last key, and stays within the view.
    const top = Math.max(...Array.from({ length: 101 }, (_, n) => valueAt(over, n / 10)));
    expect(top).toBeGreaterThan(1);
    expect(top).toBeLessThanOrEqual(1.5);
  });

  it('zooms about a point, which stays where it is, and pans', () => {
    const base: View = { from: 0, to: 100, low: 0, high: 10 };
    expect(zoomed(base, 2, { frame: 25 })).toEqual({ from: 12.5, to: 62.5, low: 0, high: 10 });
    expect(zoomed(base, 0.5, { value: 10 })).toEqual({ from: 0, to: 100, low: -10, high: 10 });
    const both = zoomed(base, 4, { frame: 80, value: 2.5 });
    expect(xOf(both, rect, 80)).toBeCloseTo(xOf(base, rect, 80), 9);
    expect(yOf(both, rect, 2.5)).toBeCloseTo(yOf(base, rect, 2.5), 9);
    expect(both.to - both.from).toBeCloseTo(25, 9);
    expect(both.high - both.low).toBeCloseTo(2.5, 9);
    // No nearer than one frame, and a factor that is none changes nothing.
    const nearest = zoomed(base, 1e12, { frame: 25 });
    expect(nearest.to - nearest.from).toBeCloseTo(1, 9);
    expect(xOf(nearest, rect, 25)).toBeCloseTo(xOf(base, rect, 25), 6);
    for (const factor of [0, -2, NaN, Infinity]) expect(zoomed(base, factor, { frame: 25, value: 5 })).toEqual(base);
    expect(panned(base, 10, -2)).toEqual({ from: 10, to: 110, low: -2, high: 8 });
    expect(panned(base, -5)).toEqual({ from: -5, to: 95, low: 0, high: 10 });
  });

  it('marks an axis at steps of 1, 2 or 5, and frames at whole ones', () => {
    expect(ticksOf(0, 100, 5, true)).toEqual([0, 20, 40, 60, 80, 100]);
    expect(ticksOf(-1, 1, 4)).toEqual([-1, -0.5, 0, 0.5, 1]);
    expect(ticksOf(0, 3, 10, true)).toEqual([0, 1, 2, 3]);
    expect(ticksOf(-0.3, 2, 10, true)).toEqual([0, 1, 2]);
    expect(ticksOf(7, 43, 4)).toEqual([10, 20, 30, 40]);
    expect(ticksOf(5, 5, 4)).toEqual([]);
    expect(ticksOf(5, 1, 4)).toEqual([]);
    expect(ticksOf(0, 10, 0)).toEqual([]);
  });
});

describe('the curve of a setting', () => {
  it('has a stretch between each two keys, with the control points that the engine reads', () => {
    expect(stretchesOf(0.5)).toEqual([]);
    expect(stretchesOf({ keys: [key(3, 1)] })).toEqual([]);
    const stretches = stretchesOf({ keys: mixed });
    expect(stretches.map(each => [each.index, each.kind])).toEqual([[0, 'linear'], [1, 'hold'], [2, 'bezier']]);
    // The handle that leaves is half of the 20 frames on, level; the one that comes is a quarter back, 0.4 above.
    expect(stretches[2].points).toEqual([[60, -1], [70, -1], [75, 0.4], [80, 0]]);
    expect(stretches[1].points).toEqual([[40, 1], [40, 1], [60, 1], [60, -1]]);
    const [first, second, third, last] = stretches[0].points;
    expect(first).toEqual([20, 0]);
    expect(last).toEqual([40, 1]);
    expect(second[0]).toBeCloseTo(20 + 20 / 3, 9);
    expect(second[1]).toBeCloseTo(1 / 3, 9);
    expect(third[0]).toBeCloseTo(40 - 20 / 3, 9);
    expect(third[1]).toBeCloseTo(2 / 3, 9);
  });

  it('is, at every point of every stretch, what the engine reads at that frame', () => {
    const settings: Param[] = [
      { keys: mixed },
      { keys: [key(0, 0, bezier([0.4, 0.3], [0.25, -0.2])), key(40, 1), key(60, 0)] },
      // A handle that goes above the keys, and one that reaches the whole of its stretch.
      { keys: [key(-10, 2, bezier([0.3, 0], [0.3, 5])), key(10, 3, bezier([1, -4], [0.3, 4])), key(31, -7)] },
      ...(['linear', 'in', 'out', 'in_out'] as const).map(ease => ({ keys: eased([key(0, 0), key(24, 1), key(50, -0.5)], 1, ease) })),
    ];
    for (const param of settings) {
      for (const stretch of stretchesOf(param)) {
        if (stretch.kind === 'hold') continue;
        for (let n = 0; n <= 64; n += 1) {
          const [frame, value] = pointOn(stretch.points, n / 64);
          expect(valueAt(param, frame)).toBeCloseTo(value, 9);
        }
      }
    }
    // A stretch that holds keeps its value up to the frame of the next key, where it steps.
    expect(valueAt({ keys: mixed }, 59.999)).toBe(1);
    expect(valueAt({ keys: mixed }, 60)).toBe(-1);
  });

  it('is drawn as a path of lines, a step and a cubic curve, level before the first key and after the last', () => {
    expect(curvePath({ keys: mixed }, view, rect)).toBe('M50,120 L130,120 L210,20 H290 V220 C330,220 350,80 370,120 L450,120');
    expect(curvePath(0.5, view, rect)).toBe('M50,70 L450,70');
    expect(curvePath({ keys: [key(50, 0)] }, view, rect)).toBe('M50,120 L250,120 L450,120');
    expect(curvePath({ keys: [] }, view, rect)).toBe('M50,120 L450,120');
  });

  it('leaves out the stretches that lie outside the frames asked for', () => {
    // Between 45 and 55 only the step is there; it is drawn whole.
    expect(curvePath({ keys: mixed }, view, rect, 45, 55)).toBe('M210,20 H290 V220');
    // From 70 on: the Bezier stretch whole, and the level run to the end.
    expect(curvePath({ keys: mixed }, view, rect, 70, 90)).toBe('M290,220 C330,220 350,80 370,120 L410,120');
    // Up to 30: the run in and the line whole.
    expect(curvePath({ keys: mixed }, view, rect, 10, 30)).toBe('M90,120 L130,120 L210,20');
    expect(curvePath({ keys: [key(50, 0)] }, view, rect, 50, 50)).toBe('M250,120');
  });

  it('draws the cubic curve of the path where the engine evaluates it', () => {
    const param: Param = { keys: mixed };
    const drawn = /C([-\d.]+),([-\d.]+) ([-\d.]+),([-\d.]+) ([-\d.]+),([-\d.]+)/.exec(curvePath(param, view, rect))!;
    const [bx, by, cx, cy, dx, dy] = drawn.slice(1).map(Number);
    // The curve begins where the step ended: at the key of frame 60.
    const [ax, ay] = [xOf(view, rect, 60), yOf(view, rect, -1)];
    for (let n = 0; n <= 40; n += 1) {
      const s = n / 40;
      const r = 1 - s;
      const x = r * r * r * ax + 3 * r * r * s * bx + 3 * r * s * s * cx + s * s * s * dx;
      const y = r * r * r * ay + 3 * r * r * s * by + 3 * r * s * s * cy + s * s * s * dy;
      expect(valueAt(param, frameAt(view, rect, x))).toBeCloseTo(valueAtY(view, rect, y), 6);
    }
  });

  it('places a key and its two handles on the screen', () => {
    const near = (point: { x: number; y: number } | null, x: number, y: number) => {
      expect(point).not.toBeNull();
      expect(point!.x).toBeCloseTo(x, 9);
      expect(point!.y).toBeCloseTo(y, 9);
    };
    expect(keyPoint(key(50, 0), view, rect)).toEqual({ x: 250, y: 120 });
    near(keyPoint(mixed[1], view, rect), 210, 20);
    // A line and a step show no handles; a Bezier stretch shows one at each of its keys.
    expect(handlesOf(mixed, 0)).toEqual({ in: null, out: null });
    expect(handlesOf(mixed, 1)).toEqual({ in: null, out: null });
    expect(handlesOf(mixed, 2)).toEqual({ in: null, out: [70, -1] });
    expect(handlesOf(mixed, 3)).toEqual({ in: [75, 0.4], out: null });
    expect(handlesOf(mixed, 4)).toEqual({ in: null, out: null });
    expect(handlesOf(mixed, -1)).toEqual({ in: null, out: null });
    expect(handlePoints(mixed, 2, view, rect).in).toBeNull();
    near(handlePoints(mixed, 2, view, rect).out, 330, 220);
    near(handlePoints(mixed, 3, view, rect).in, 350, 80);
    expect(handlePoints(mixed, 3, view, rect).out).toBeNull();
  });
});

describe('what a pointer is on', () => {
  it('finds the nearest key within reach', () => {
    expect(hitKey(mixed, view, rect, { x: 212, y: 23 })).toBe(1);
    expect(hitKey(mixed, view, rect, { x: 130, y: 120 + HIT_PX })).toBe(0);
    expect(hitKey(mixed, view, rect, { x: 130, y: 121 + HIT_PX })).toBe(-1);
    expect(hitKey(mixed, view, rect, { x: 250, y: 120 })).toBe(-1);
    expect(hitKey([], view, rect, { x: 250, y: 120 })).toBe(-1);
    // Of two keys within reach, the nearer.
    const close = [key(50, 0), key(52, 0)];
    expect(hitKey(close, view, rect, { x: 253, y: 120 })).toBe(0);
    expect(hitKey(close, view, rect, { x: 256, y: 120 })).toBe(1);
    expect(hitKey(close, view, rect, { x: 290, y: 120 }, 40)).toBe(1);
  });

  it('finds a handle of the keys chosen, and of no other', () => {
    expect(hitHandle(mixed, [2, 3], view, rect, { x: 331, y: 219 })).toEqual({ index: 2, side: 'out' });
    expect(hitHandle(mixed, [2, 3], view, rect, { x: 351, y: 82 })).toEqual({ index: 3, side: 'in' });
    expect(hitHandle(mixed, [3], view, rect, { x: 331, y: 219 })).toBeNull();
    expect(hitHandle(mixed, [0, 1], view, rect, { x: 331, y: 219 })).toBeNull();
    expect(hitHandle(mixed, [2, 3], view, rect, { x: 300, y: 150 })).toBeNull();
    expect(hitHandle(mixed, [], view, rect, { x: 331, y: 219 })).toBeNull();
  });

  it('finds a curve along its lines, its step, its bend and its level runs', () => {
    const param: Param = { keys: mixed };
    // The middle of the line from (130, 120) to (210, 20), and a place 20 pixels beneath it.
    expect(hitCurve(param, view, rect, { x: 170, y: 70 })).toBe(true);
    expect(curveDistance(param, view, rect, { x: 170, y: 70 })).toBeLessThan(0.01);
    expect(hitCurve(param, view, rect, { x: 170, y: 90 })).toBe(false);
    // The step at frame 60 goes straight down: halfway down it is on the curve.
    expect(hitCurve(param, view, rect, { x: 290, y: 120 })).toBe(true);
    expect(hitCurve(param, view, rect, { x: 270, y: 120 })).toBe(false);
    // The runs before the first key and after the last.
    expect(hitCurve(param, view, rect, { x: 80, y: 122 })).toBe(true);
    expect(hitCurve(param, view, rect, { x: 80, y: 100 })).toBe(false);
    expect(hitCurve(param, view, rect, { x: 430, y: 118 })).toBe(true);
    // A point of the Bezier stretch, found from its control points.
    const [frame, value] = pointOn(stretchesOf(param)[2].points, 0.4);
    const on = { x: xOf(view, rect, frame), y: yOf(view, rect, value) };
    expect(curveDistance(param, view, rect, on)).toBeLessThan(0.5);
    expect(hitCurve(param, view, rect, { x: on.x, y: on.y - 30 })).toBe(false);
    // A setting of one number is level.
    expect(hitCurve(0.5, view, rect, { x: 300, y: 72 })).toBe(true);
    expect(hitCurve(0.5, view, rect, { x: 300, y: 90 })).toBe(false);
  });

  it('finds the keys in a box, whichever corners it was drawn from, and those at frames', () => {
    expect(keysIn(mixed, view, rect, { x: 120, y: 10 }, { x: 300, y: 230 })).toEqual([0, 1, 2]);
    expect(keysIn(mixed, view, rect, { x: 300, y: 230 }, { x: 120, y: 10 })).toEqual([0, 1, 2]);
    expect(keysIn(mixed, view, rect, { x: 120, y: 100 }, { x: 300, y: 140 })).toEqual([0]);
    expect(keysIn(mixed, view, rect, { x: 0, y: 0 }, { x: 10, y: 10 })).toEqual([]);
    expect(selectedIn(mixed, [40, 80, 99])).toEqual([1, 3]);
    expect(selectedIn(mixed, [])).toEqual([]);
  });
});

describe('moving keys', () => {
  const four = [key(0, 0), key(10, 1), key(20, 0.5), key(30, 0)];

  it('moves a key by whole frames and by a value, and changes none of the keys given', () => {
    const before = clone(four);
    expect(moveKeys(four, [1], 3.4, 0.25, wide)).toEqual([key(0, 0), key(13, 1.25), key(20, 0.5), key(30, 0)]);
    expect(moveKeys(four, [1], 3.5, 0, wide)[1].frame).toBe(14);
    expect(moveKeys(four, [1], 0.4, 0, wide)[1].frame).toBe(10);
    expect(moveKeys(four, [1], -0.4, 0, wide)).toEqual(four);
    expect(moveKeys(four, [1], -3.6, -2, wide)[1]).toEqual(key(6, -1));
    expect(four).toEqual(before);
    // The keys given back are new ones.
    expect(moveKeys(four, [], 5, 5, wide)).toEqual(four);
    expect(moveKeys(four, [], 5, 5, wide)[0]).not.toBe(four[0]);
  });

  it('keeps a key strictly between the keys beside it that are not moved', () => {
    expect(moveKeys(four, [1], 50, 0, wide).map(each => each.frame)).toEqual([0, 19, 20, 30]);
    expect(moveKeys(four, [1], -50, 0, wide).map(each => each.frame)).toEqual([0, 1, 20, 30]);
    // Keys chosen together move together, as far as the first that is stopped.
    expect(moveKeys(four, [1, 2], 50, 0, wide).map(each => each.frame)).toEqual([0, 19, 29, 30]);
    expect(moveKeys(four, [1, 2], -50, 0, wide).map(each => each.frame)).toEqual([0, 1, 11, 30]);
    expect(moveKeys(four, [0, 2], 50, 0, wide).map(each => each.frame)).toEqual([9, 10, 29, 30]);
    expect(moveKeys(four, [0, 2], -50, 0, wide).map(each => each.frame)).toEqual([-9, 10, 11, 30]);
    // The ends are held by nothing, and neither are all the keys together.
    expect(moveKeys(four, [3], 1000, 0, wide).map(each => each.frame)).toEqual([0, 10, 20, 1030]);
    expect(moveKeys(four, [0], -1000, 0, wide).map(each => each.frame)).toEqual([-1000, 10, 20, 30]);
    expect(moveKeys(four, [0, 1, 2, 3], -1000, 0, wide).map(each => each.frame)).toEqual([-1000, -990, -980, -970]);
    // Keys a frame apart have nowhere to go.
    expect(moveKeys([key(0, 0), key(1, 1), key(2, 0)], [1], 5, 0, wide).map(each => each.frame)).toEqual([0, 1, 2]);
    for (const by of [-50, -3, 0, 7, 50]) expect(taken(moveKeys(four, [1, 2], by, 0, wide))).toBe(true);
  });

  it('keeps each value within the bounds of the setting', () => {
    const unit = { low: 0, high: 1 };
    expect(moveKeys(four, [1, 2], 0, 0.25, unit).map(each => each.value)).toEqual([0, 1, 0.75, 0]);
    expect(moveKeys(four, [1, 2, 3], 0, -0.75, unit).map(each => each.value)).toEqual([0, 0.25, 0, 0]);
    expect(moveKeys(four, [0], 0, 99, unit)[0].value).toBe(1);
  });

  it('passes over places that are no keys, and a move that is no number', () => {
    expect(moveKeys(four, [-1, 9, 1.5], 5, 1, wide)).toEqual(four);
    expect(moveKeys(four, [1], NaN, NaN, wide)).toEqual(four);
    expect(moveKeys(four, [1, 1, 1], 2, 0, wide).map(each => each.frame)).toEqual([0, 12, 20, 30]);
  });

  it('keeps an ease an ease where its key is moved in value', () => {
    const smooth = eased([key(0, 0), key(10, 1), key(20, 0)], 1, 'in_out');
    expect(easeOf(smooth, 1)).toBe('in_out');
    const moved = moveKeys(smooth, [1], 2, 0.5, wide);
    expect(moved[1]).toMatchObject({ frame: 12, value: 1.5 });
    expect(easeOf(moved, 1)).toBe('in_out');
    // The handle of the stretch that lay on its line lies on the line as it is now.
    expect(moved[0].interp).toEqual(bezier([EASE_SHARE, 1.5 * EASE_SHARE], [EASE_SHARE, 0]));
    expect(moved[1].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, 1.5 * EASE_SHARE]));
    // So do the eases of the keys beside one that is moved.
    const into = eased([key(0, 0), key(10, 1), key(20, 0)], 2, 'in');
    expect(easeOf(moveKeys(into, [1], 0, 3, wide), 2)).toBe('in');
    expect(easeOf(moveKeys(eased([key(0, 0), key(10, 1), key(20, 0)], 0, 'out'), [1], 0, 3, wide), 0)).toBe('out');
  });

  it('leaves a handle that was pulled, a line and a step as they were', () => {
    const pulled = [key(0, 0, bezier([0.2, 0.7], [0.6, -0.1])), key(10, 1, 'hold'), key(20, 0), key(30, 4)];
    const moved = moveKeys(pulled, [1, 2], 3, 2, wide);
    expect(moved.map(each => each.interp)).toEqual([bezier([0.2, 0.7], [0.6, -0.1]), 'hold', 'linear', 'linear']);
    expect(moved.map(each => [each.frame, each.value])).toEqual([[0, 0], [13, 3], [23, 2], [30, 4]]);
  });
});

describe('the interpolation of keys', () => {
  const three = [key(0, 0), key(10, 1), key(20, 0)];

  it('eases the keys chosen as the engine eases one', () => {
    for (const ease of ['linear', 'in', 'out', 'in_out'] as const) {
      expect(setInterp(three, [1], ease)).toEqual(eased(three, 1, ease));
      expect(easeOf(setInterp(three, [1], ease), 1)).toBe(ease);
    }
    // Every key eased in and out: each stretch is flat at both its ends.
    const all = setInterp(three, [0, 1, 2], 'in_out');
    const flat = bezier([EASE_SHARE, 0], [EASE_SHARE, 0]);
    expect(all.map(each => each.interp)).toEqual([flat, flat, 'linear']);
    expect(setInterp(three, [2, 0, 1], 'in_out')).toEqual(all);
    // And back to lines.
    expect(setInterp(all, [0, 1, 2], 'linear')).toEqual(three);
    expect(three.every(each => each.interp === 'linear')).toBe(true);
    expect(setInterp(three, [], 'in')).toEqual(three);
    expect(setInterp(three, [7, -1], 'hold')).toEqual(three);
  });

  it('holds the keys chosen, and lets go of one that is then eased', () => {
    expect(setInterp(three, [0, 2], 'hold').map(each => each.interp)).toEqual(['hold', 'linear', 'hold']);
    const held = [key(0, 0, 'hold'), key(10, 1), key(20, 0)];
    expect(setInterp(held, [0], 'linear')).toEqual(three);
    expect(setInterp(held, [0], 'out')[0].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, -EASE_SHARE]));
    // The stretch that comes from a key that holds and is not chosen stays held.
    const after = setInterp(held, [1], 'in_out');
    expect(after[0].interp).toBe('hold');
    expect(after[1].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, EASE_SHARE]));
  });

  it('tells which button of the tool bar is lit', () => {
    expect((['linear', 'in', 'out', 'in_out', 'hold'] as const).map(kind => interpIs(three, [1], kind))).toEqual([true, false, false, false, false]);
    const smooth = setInterp(three, [1], 'in');
    expect((['linear', 'in', 'out', 'in_out', 'hold'] as const).map(kind => interpIs(smooth, [1], kind))).toEqual([false, true, false, false, false]);
    expect(interpIs(setInterp(three, [0, 1], 'hold'), [0, 1], 'hold')).toBe(true);
    expect(interpIs(setInterp(three, [0], 'hold'), [0, 1], 'hold')).toBe(false);
    expect(interpIs(setInterp(three, [1], 'hold'), [1], 'linear')).toBe(false);
    expect(interpIs(three, [], 'linear')).toBe(false);
  });
});

describe('dragging a Bezier handle', () => {
  const smooth = [key(0, 0, bezier([EASE_SHARE, 0], [EASE_SHARE, 0])), key(30, 1)];

  it('puts the handle where it is dragged: its share of the frames and its change of value', () => {
    const before = clone(smooth);
    expect(dragHandle(smooth, 0, 'out', 15, 0.4)[0].interp).toEqual(bezier([0.5, 0.4], [EASE_SHARE, 0]));
    const into = dragHandle(smooth, 1, 'in', 21, 0.8)[0].interp as { bezier: { out: [number, number]; in: [number, number] } };
    expect(into.bezier.out).toEqual([EASE_SHARE, 0]);
    expect(into.bezier.in[0]).toBeCloseTo(0.3, 12);
    expect(into.bezier.in[1]).toBeCloseTo(-0.2, 12);
    expect(smooth).toEqual(before);
    // The handle is where the pointer was: the control point of the stretch is at that frame and value.
    expect(handlesOf(dragHandle(smooth, 0, 'out', 15, 0.4), 0).out).toEqual([15, 0.4]);
    const back = handlesOf(dragHandle(smooth, 1, 'in', 21, 0.8), 1).in!;
    expect(back[0]).toBeCloseTo(21, 9);
    expect(back[1]).toBeCloseTo(0.8, 9);
  });

  it('keeps the share from 0 to 1, and the value within bounds where they are given', () => {
    expect(dragHandle(smooth, 0, 'out', -10, 0.4)[0].interp).toEqual(bezier([0, 0.4], [EASE_SHARE, 0]));
    expect(dragHandle(smooth, 0, 'out', 100, 0.4)[0].interp).toEqual(bezier([1, 0.4], [EASE_SHARE, 0]));
    expect(dragHandle(smooth, 1, 'in', 100, 1)[0].interp).toEqual(bezier([EASE_SHARE, 0], [0, 0]));
    expect(dragHandle(smooth, 1, 'in', -100, 1)[0].interp).toEqual(bezier([EASE_SHARE, 0], [1, 0]));
    // Without bounds a handle may go above its keys; with them the curve stays in what the setting takes.
    expect(dragHandle(smooth, 0, 'out', 15, 1.5)[0].interp).toEqual(bezier([0.5, 1.5], [EASE_SHARE, 0]));
    const kept = dragHandle(smooth, 0, 'out', 15, 1.5, { low: 0, high: 1 });
    expect(kept[0].interp).toEqual(bezier([0.5, 1], [EASE_SHARE, 0]));
    expect(dragHandle(smooth, 1, 'in', 15, -3, { low: 0, high: 1 })[0].interp).toEqual(bezier([EASE_SHARE, 0], [0.5, -1]));
    for (let frame = 0; frame <= 30; frame += 0.5) {
      const value = valueAt({ keys: kept }, frame);
      expect(value).toBeGreaterThanOrEqual(0);
      expect(value).toBeLessThanOrEqual(1);
    }
    expect(taken(kept)).toBe(true);
  });

  it('makes a line a Bezier stretch whose other handle stays on the line, and leaves a step alone', () => {
    const line = [key(0, 0), key(30, 3), key(60, 3, 'hold'), key(90, 0)];
    expect(dragHandle(line, 0, 'out', 10, 0)[0].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, -3 * EASE_SHARE]));
    expect(dragHandle(line, 1, 'in', 20, 3)[0].interp).toEqual(bezier([EASE_SHARE, 3 * EASE_SHARE], [EASE_SHARE, 0]));
    expect(dragHandle(line, 2, 'out', 70, 9)).toEqual(line);
    expect(dragHandle(line, 3, 'in', 70, 9)).toEqual(line);
    // A side with no stretch, a key that is none, and a place that is no number.
    expect(dragHandle(line, 0, 'in', -5, 9)).toEqual(line);
    expect(dragHandle(line, 3, 'out', 95, 9)).toEqual(line);
    expect(dragHandle(line, 9, 'out', 95, 9)).toEqual(line);
    expect(dragHandle(line, 0, 'out', NaN, 9)).toEqual(line);
  });
});

describe('putting a key on a curve', () => {
  it('gives a setting of one number its first key', () => {
    expect(insertKey(0.3, 20.4)).toEqual({ keys: [key(20, 0.3)], index: 0 });
  });

  it('puts a linear key on a line, which stays the line', () => {
    const line: Param = { keys: [key(0, 0), key(40, 1)] };
    expect(insertKey(line, 10)).toEqual({ keys: [key(0, 0), key(10, 0.25), key(40, 1)], index: 1 });
    expect(insertKey(line, 9.6)!.keys[1].frame).toBe(10);
    // Where there is a key already there is nothing to put.
    expect(insertKey(line, 0)).toBeNull();
    expect(insertKey(line, 40.2)).toBeNull();
    expect(line).toEqual({ keys: [key(0, 0), key(40, 1)] });
  });

  it('puts a key that holds on a step', () => {
    const step: Param = { keys: [key(0, 2, 'hold'), key(40, 5)] };
    const made = insertKey(step, 10)!;
    expect(made).toEqual({ keys: [key(0, 2, 'hold'), key(10, 2, 'hold'), key(40, 5)], index: 1 });
    for (const frame of [0, 5, 10, 39.5, 40, 50]) expect(valueAt({ keys: made.keys }, frame)).toBe(valueAt(step, frame));
  });

  it('puts a level key before the first key and after the last', () => {
    const param: Param = { keys: [key(10, 2), key(20, 5, bezier([0.5, 3], [0.5, 3]))] };
    expect(insertKey(param, 0)).toEqual({ keys: [key(0, 2), key(10, 2), key(20, 5, bezier([0.5, 3], [0.5, 3]))], index: 0 });
    // The key that was last gets a stretch: its handles would bend what was level, so it is linear.
    const after = insertKey(param, 30)!;
    expect(after).toEqual({ keys: [key(10, 2), key(20, 5), key(30, 5)], index: 2 });
    for (const frame of [-5, 10, 15, 20, 25, 30, 99]) expect(valueAt({ keys: after.keys }, frame)).toBe(valueAt(param, frame));
    // One that held goes on holding, which is level too.
    expect(insertKey({ keys: [key(10, 2, 'hold')] }, 30)!.keys).toEqual([key(10, 2, 'hold'), key(30, 2)]);
    expect(insertKey({ keys: [] }, 5)).toEqual({ keys: [key(5, 0)], index: 0 });
  });

  it('cuts a Bezier stretch in two halves that are the curve that was there', () => {
    const curves: { keys: Keyframe[] }[] = [
      { keys: [key(0, 0, bezier([0.4, 0.3], [0.25, -0.2])), key(40, 1), key(60, 0)] },
      { keys: setInterp([key(0, 0), key(40, 1), key(60, 0)], [0, 1], 'in_out') },
      { keys: [key(-20, 5, bezier([0.1, -6], [0.6, 2])), key(20, -3)] },
    ];
    for (const param of curves) {
      const before = clone(param);
      const [first, second] = param.keys;
      for (const frame of [first.frame + 1, first.frame + 13, second.frame - 7, second.frame - 1]) {
        const made = insertKey(param, frame)!;
        expect(made.index).toBe(1);
        expect(made.keys).toHaveLength(param.keys.length + 1);
        expect(made.keys[1].frame).toBe(frame);
        expect(made.keys[1].value).toBe(valueAt(param, frame));
        expect(taken(made.keys)).toBe(true);
        // What follows the stretch is as it was.
        expect(made.keys.slice(2)).toEqual(param.keys.slice(1));
        for (let at = first.frame - 2; at <= second.frame + 2; at += 0.25) {
          expect(valueAt({ keys: made.keys }, at)).toBeCloseTo(valueAt(param, at), 8);
        }
      }
      expect(param).toEqual(before);
    }
  });

  it('keeps the handles of a time that runs evenly a third of each half', () => {
    // Handles a third along: time runs evenly, and so it does in each half.
    const even: Param = { keys: [key(0, 0, bezier([EASE_SHARE, 0.9], [EASE_SHARE, 0.2])), key(30, 1)] };
    const made = insertKey(even, 12)!;
    for (const each of made.keys.slice(0, 2)) {
      const interp = each.interp as { bezier: { out: [number, number]; in: [number, number] } };
      expect(interp.bezier.out[0]).toBeCloseTo(EASE_SHARE, 9);
      expect(interp.bezier.in[0]).toBeCloseTo(EASE_SHARE, 9);
    }
  });

  it('gives keys that the engine takes where a handle of a half would reach past the new key', () => {
    // Both handles reach the whole stretch: the time stands still in the middle.
    const far: Param = { keys: [key(0, 0, bezier([1, 0], [1, 0])), key(40, 1)] };
    for (const frame of [1, 10, 20, 30, 39]) {
      const made = insertKey(far, frame)!;
      expect(taken(made.keys)).toBe(true);
      expect(made.keys[1].value).toBe(valueAt(far, frame));
      // The ends of the two halves are where they were.
      for (const at of [0, frame, 40]) expect(valueAt({ keys: made.keys }, at)).toBeCloseTo(valueAt(far, at), 12);
    }
  });
});

describe('taking keys away', () => {
  const three = [key(0, 0), key(10, 1), key(20, 3)];

  it('takes the keys chosen, and gives a setting with none chosen as it is', () => {
    const param: Param = { keys: three };
    expect(deleteKeys(param, [1])).toEqual({ keys: [key(0, 0), key(20, 3)] });
    expect(deleteKeys(param, [0, 2])).toEqual({ keys: [key(10, 1)] });
    expect(deleteKeys(param, [2])).toEqual({ keys: [key(0, 0), key(10, 1)] });
    expect(deleteKeys(param, [])).toBe(param);
    expect(deleteKeys(param, [5, -1])).toBe(param);
    expect(deleteKeys(0.4, [0])).toBe(0.4);
    expect(param).toEqual({ keys: [key(0, 0), key(10, 1), key(20, 3)] });
  });

  it('leaves the number that a setting had where its last key is taken', () => {
    expect(deleteKeys({ keys: [key(40, 0.75)] }, [0])).toBe(0.75);
    expect(deleteKeys({ keys: [key(40, 0.75)] }, [0], 3)).toBe(0.75);
    // All of several: what the setting is at the frame given, or at its first key.
    expect(deleteKeys({ keys: three }, [0, 1, 2], 5)).toBe(0.5);
    expect(deleteKeys({ keys: three }, [0, 1, 2], 99)).toBe(3);
    expect(deleteKeys({ keys: three }, [2, 1, 0])).toBe(0);
  });

  it('keeps the ease of a key that stays, and a step', () => {
    // The last key is eased into; with the middle key gone, the first goes to it eased in still.
    const eased2 = setInterp(three, [2], 'in');
    const left = deleteKeys({ keys: eased2 }, [1]) as { keys: Keyframe[] };
    expect(left.keys.map(each => each.frame)).toEqual([0, 20]);
    expect(left.keys[0].interp).toEqual(bezier([EASE_SHARE, 3 * EASE_SHARE], [EASE_SHARE, 0]));
    expect(easeOf(left.keys, 1)).toBe('in');
    // The first key is eased out of.
    const out = deleteKeys({ keys: setInterp(three, [0], 'out') }, [1]) as { keys: Keyframe[] };
    expect(out.keys[0].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, -3 * EASE_SHARE]));
    expect(easeOf(out.keys, 0)).toBe('out');
    // Both: eased out of the first and into the last.
    const both = deleteKeys({ keys: setInterp(three, [0, 2], 'in_out') }, [1]) as { keys: Keyframe[] };
    expect(both.keys[0].interp).toEqual(bezier([EASE_SHARE, 0], [EASE_SHARE, 0]));
    // A key that holds holds to the next that stays; lines stay a line.
    expect(deleteKeys({ keys: [key(0, 0, 'hold'), key(10, 1), key(20, 3)] }, [1])).toEqual({ keys: [key(0, 0, 'hold'), key(20, 3)] });
    expect(deleteKeys({ keys: [key(0, 0), key(10, 1, 'hold'), key(20, 3)] }, [1])).toEqual({ keys: [key(0, 0), key(20, 3)] });
    // The key that becomes the last keeps how it went on, which says nothing now.
    expect(deleteKeys({ keys: eased2 }, [2])).toEqual({ keys: eased2.slice(0, 2) });
    expect(taken(left.keys) && taken(out.keys) && taken(both.keys)).toBe(true);
  });
});

describe('copying and pasting keys', () => {
  const four = [key(0, 0), key(10, 1, 'hold'), key(25, 0.5, bezier([0.2, 0.1], [0.3, 0])), key(30, 0)];
  const unit = { low: 0, high: 1 };

  it('copies the keys chosen by how far each is after the first', () => {
    expect(copyKeys(four, [2, 1])).toEqual([{ offset: 0, value: 1, interp: 'hold' }, { offset: 15, value: 0.5, interp: bezier([0.2, 0.1], [0.3, 0]) }]);
    expect(copyKeys(four, [])).toEqual([]);
    expect(copyKeys(four, [8])).toEqual([]);
  });

  it('pastes them from a frame on, as far apart as they were', () => {
    const copied = copyKeys(four, [0, 1]);
    expect(pasteKeys(0.5, copied, 100.4, unit)).toEqual({ keys: [key(100, 0), key(110, 1, 'hold')], frames: [100, 110] });
    // Among keys: in the order of the frames, the others left where they are.
    const made = pasteKeys({ keys: four }, copied, 12, unit)!;
    expect(made.frames).toEqual([12, 22]);
    expect(made.keys.map(each => each.frame)).toEqual([0, 10, 12, 22, 25, 30]);
    expect(made.keys[2]).toEqual(key(12, 0));
    expect(made.keys[3]).toEqual(key(22, 1, 'hold'));
    expect(taken(made.keys)).toBe(true);
    expect(pasteKeys({ keys: four }, [], 12, unit)).toBeNull();
    expect(pasteKeys({ keys: four }, copied, NaN, unit)).toBeNull();
  });

  it('replaces a key at a frame that it pastes to, and keeps values within the bounds', () => {
    const copied = [{ offset: 0, value: 7, interp: 'linear' as const }, { offset: 15, value: -2, interp: 'linear' as const }];
    const made = pasteKeys({ keys: four }, copied, 10, unit)!;
    expect(made.keys.map(each => [each.frame, each.value])).toEqual([[0, 0], [10, 1], [25, 0], [30, 0]]);
    // The first replaces a key and goes as it was copied; the last goes on as the key that it replaces did.
    expect(made.keys[1].interp).toBe('linear');
    expect(made.keys[2].interp).toEqual(bezier([0.2, 0.1], [0.3, 0]));
  });

  it('makes the last of them linear where it was copied with the handles of a stretch that was not', () => {
    const copied = copyKeys(four, [1, 2]);
    const made = pasteKeys(0, copied, 50, unit)!;
    expect(made.keys).toEqual([key(50, 1, 'hold'), key(65, 0.5)]);
    // Copied with its stretch, a Bezier key goes as it was.
    const whole = pasteKeys(0, copyKeys(four, [2, 3]), 50, unit)!;
    expect(whole.keys).toEqual([key(50, 0.5, bezier([0.2, 0.1], [0.3, 0])), key(55, 0)]);
  });
});

describe('the splice that makes keys into others', () => {
  const four = [key(0, 0), key(10, 1), key(20, 0.5), key(30, 0)];

  it('is none where they are the same', () => {
    expect(spliceOf(four, clone(four))).toBeNull();
    expect(spliceOf([], [])).toBeNull();
    const curved = [key(0, 0, bezier([0.2, 0.1], [0.3, 0])), key(9, 1)];
    expect(spliceOf(curved, clone(curved))).toBeNull();
  });

  it('is what lies between the keys that they begin and end with alike', () => {
    expect(spliceOf(four, [key(0, 0), key(13, 1), key(20, 0.5), key(30, 0)])).toEqual({ at: 1, remove: 1, keys: [key(13, 1)] });
    expect(spliceOf(four, [key(0, 0), key(10, 1), key(15, 2), key(20, 0.5), key(30, 0)])).toEqual({ at: 2, remove: 0, keys: [key(15, 2)] });
    expect(spliceOf(four, [key(0, 0), key(30, 0)])).toEqual({ at: 1, remove: 2, keys: [] });
    expect(spliceOf(four, [key(10, 1), key(20, 0.5), key(30, 0)])).toEqual({ at: 0, remove: 1, keys: [] });
    expect(spliceOf(four, [...four, key(40, 2)])).toEqual({ at: 4, remove: 0, keys: [key(40, 2)] });
    expect(spliceOf([], four)).toEqual({ at: 0, remove: 0, keys: four });
    expect(spliceOf(four, [key(5, 5)])).toEqual({ at: 0, remove: 4, keys: [key(5, 5)] });
    // A handle that differs is a key that differs.
    const before = [key(0, 0, bezier([0.2, 0.1], [0.3, 0])), key(9, 1)];
    const after = [key(0, 0, bezier([0.2, 0.1], [0.3, 0.5])), key(9, 1)];
    expect(spliceOf(before, after)).toEqual({ at: 0, remove: 1, keys: [after[0]] });
    expect(spliceOf([key(0, 0, 'hold'), key(9, 1)], [key(0, 0), key(9, 1)])).toEqual({ at: 0, remove: 1, keys: [key(0, 0)] });
    // Keys that repeat are taken from the front first: one splice still makes the one into the other.
    const same = [key(0, 0), key(1, 0)];
    expect(spliceOf(same, [key(0, 0)])).toEqual({ at: 1, remove: 1, keys: [] });
  });

  it('makes the keys of every edit out of the keys before it', () => {
    const eased4 = setInterp(four, [1, 2], 'in_out');
    const edits: Keyframe[][] = [
      moveKeys(four, [1, 2], 4, 0.25, { low: -9, high: 9 }),
      moveKeys(eased4, [2], -3, 1, { low: -9, high: 9 }),
      eased4,
      setInterp(four, [3], 'hold'),
      dragHandle(eased4, 1, 'out', 14, 2),
      insertKey({ keys: four }, 15)!.keys,
      insertKey({ keys: eased4 }, 15)!.keys,
      insertKey({ keys: four }, -5)!.keys,
      insertKey({ keys: four }, 99)!.keys,
      (deleteKeys({ keys: eased4 }, [1, 2]) as { keys: Keyframe[] }).keys,
      (deleteKeys({ keys: four }, [0]) as { keys: Keyframe[] }).keys,
      pasteKeys({ keys: four }, copyKeys(four, [0, 1]), 12, { low: -9, high: 9 })!.keys,
    ];
    for (const after of edits) {
      const base = after === edits[1] || after === edits[4] || after === edits[6] || after === edits[9] ? eased4 : four;
      const splice = spliceOf(base, after);
      expect(splice).not.toBeNull();
      expect(spliced(base, splice)).toEqual(after);
      expect(taken(after)).toBe(true);
    }
  });
});

describe('the operations that a curve editor sends for a clip', () => {
  const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
  const keyed = (keys: Keyframe[]) => clip('c1', 10, 60, { transform: { ...NONE, x: { keys } } });
  const splice = (at: number, remove: number, keys: Keyframe[], path = 'transform.x') => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path, at, remove, keys });

  it('are one splice of the keys that differ, with their place among the keys', () => {
    const c1 = keyed([key(0, 0), key(10, 1), key(20, 0.5), key(30, 0)]);
    expect(keyOps('t1', c1, 'transform.x', { keys: [key(0, 0), key(13, 1), key(20, 0.5), key(30, 0)] })).toEqual([splice(1, 1, [key(13, 1)])]);
    expect(keyOps('t1', c1, 'transform.x', { keys: [key(0, 0), key(10, 1), key(20, 0.5)] })).toEqual([splice(3, 1, [])]);
    expect(keyOps('t1', c1, 'transform.x', { keys: [key(0, 0), key(10, 1), key(15, 0.75), key(20, 0.5), key(30, 0)] })).toEqual([splice(2, 0, [key(15, 0.75)])]);
    expect(keyOps('t1', c1, 'transform.x', { keys: [key(0, 0), key(10, 1), key(20, 0.5), key(30, 0)] })).toEqual([]);
  });

  it('give a setting of one number its first keys by a splice at 0, and a number by `set_clip_keys`', () => {
    const plain = clip('c1', 10, 60);
    expect(keyOps('t1', plain, 'transform.rotation', { keys: [key(20, 0)] })).toEqual([splice(0, 0, [key(20, 0)], 'transform.rotation')]);
    // A clip that says nothing of its crop has none.
    expect(keyOps('t1', plain, 'crop.left', { keys: [key(5, 0.1), key(9, 0.2)] })).toEqual([splice(0, 0, [key(5, 0.1), key(9, 0.2)], 'crop.left')]);
    expect(keyOps('t1', plain, 'transform.rotation', 0)).toEqual([]);
    expect(keyOps('t1', plain, 'transform.rotation', 45)).toEqual([{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'transform.rotation', param: 45 }]);
    // The last key of a setting cannot be spliced away: the setting is set to its number.
    expect(keyOps('t1', keyed([key(40, 0.75)]), 'transform.x', 0.75)).toEqual([{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', param: 0.75 }]);
    expect(keyOps('t1', keyed([key(40, 0.75)]), 'transform.x', { keys: [] })).toEqual([]);
  });
});

describe('the frame of the timeline of a frame of keys', () => {
  const rate = { num: 25, den: 1 };
  const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };

  it('is the first frame at which the clip shows that picture, and an end of the clip for a picture that it does not show', () => {
    // c1 from frame 10 for 60 frames shows the picture `n` at frame `10 + n`.
    const c1 = clip('c1', 10, 60);
    expect([0, 20, 59].map(at => frameOfKey(c1, { rate }, [media], at))).toEqual([10, 30, 69]);
    expect([-5, 60, 500].map(at => frameOfKey(c1, { rate }, [media], at))).toEqual([10, 69, 69]);
    for (const frame of [10, 25, 69]) expect(frameOfKey(c1, { rate }, [media], keyFrameOf(c1, { rate }, [media], frame))).toBe(frame);
    // Twice as fast, the picture 40 comes at frame 30; 41 is not shown, and is with the one after it.
    const fast = clip('c1', 10, 30, { speed: { num: 2, den: 1 } });
    expect([0, 40, 41].map(at => frameOfKey(fast, { rate }, [media], at))).toEqual([10, 30, 31]);
    // Backward, the first picture is shown last.
    const back = clip('c1', 10, 60, { reverse: true });
    expect([0, 40, 59, 500, -5].map(at => frameOfKey(back, { rate }, [media], at))).toEqual([69, 29, 10, 10, 69]);
    expect(frameOfKey(clip('c1', 10, 0), { rate }, [media], 7)).toBe(10);
  });

  it('is where `keyTimes` finds the keys', () => {
    const keys = [key(0, 0), key(40, 1), key(41, 0)];
    for (const fields of [{}, { length: 30, speed: { num: 2, den: 1 } }, { reverse: true }]) {
      const c1 = clip('c1', 10, 60, { transform: { ...NONE, x: { keys } }, ...fields });
      const times = keys.map(each => frameOfKey(c1, { rate }, [media], each.frame));
      expect([...new Set(times)].sort((a, b) => a - b)).toEqual(keyTimes(c1, { rate }, [media], 'transform.x'));
    }
  });
});

describe('the bounds of the settings of a clip that take keys', () => {
  interface Entry { path: string; default: number; low: number; high: number; unit: string; page: string; palette: string; clock: string }
  const catalog: { clip: Entry[] } = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/catalog.json', import.meta.url), 'utf8'));

  it('are those of the catalog of the engine, for every setting and no other', () => {
    expect(catalog.clip.map(entry => entry.path).sort()).toEqual([...CLIP_KEYED].sort());
    expect(Object.keys(CLIP_BOUNDS).sort()).toEqual([...CLIP_KEYED].sort());
    const kept: Record<string, { neutral: number; low: number; high: number }> = CLIP_BOUNDS;
    for (const entry of catalog.clip) {
      expect(kept[entry.path], entry.path).toEqual({ neutral: entry.default, low: entry.low, high: entry.high });
    }
    expect(catalog.clip.every(entry => entry.page === 'edit' && entry.clock === 'source_frame')).toBe(true);
  });
});
