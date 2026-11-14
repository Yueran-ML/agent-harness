/**
 * The geometry of a curve editor, and what it does to keys (DaVinci Resolve Reference Manual 20,
 * Ch. 53, pp. 1116–1119): where a frame and a value are on the screen, the curve of a setting as
 * a path, where its keys and their handles are, what a pointer is on, and the keys that an edit
 * leaves. Pure: every edit gives new keys and changes none of those it was given.
 *
 * The horizontal axis counts the frames of the keys, which are what `Keyframe.frame` counts (the
 * pictures of a clip's media, or the clip's own frames from its first; `keyClockOf`), not the frames
 * of a timeline. The vertical axis is the value as the engine keeps it.
 *
 * A curve is drawn from the control points that `valueAt` reads (`./value`), and the two axes are
 * straight scales, so the curve drawn is the curve that the engine evaluates.
 */
import type { Interp, Keyframe, Param } from '../engine';
import { EASE_SHARE, eased, valueAt, type Ease } from './value';

/** The least and the most that a setting takes. */
export interface Range { low: number; high: number }
/** The rectangle of the graph, in the pixels of what it is drawn in. */
export interface Rect { left: number; top: number; width: number; height: number }
/** What a graph shows: the frames of keys from its left edge to its right, the values from its bottom to its top. */
export interface View { from: number; to: number; low: number; high: number }
export interface Point { x: number; y: number }
/** A frame of keys and a value there. */
export type At = [frame: number, value: number];
type Four = [number, number, number, number];
type Handle = [number, number];

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));
const copy = (key: Keyframe): Keyframe => ({ ...key });

// --- The view.

/** Where a frame of keys is, from the left. */
export const xOf = (view: View, rect: Rect, frame: number) => rect.left + ((frame - view.from) / (view.to - view.from)) * rect.width;
/** The frame of keys at a place from the left, which may lie between two frames. */
export const frameAt = (view: View, rect: Rect, x: number) => view.from + ((x - rect.left) / rect.width) * (view.to - view.from);
/** Where a value is, from the top: the greatest value is at the top. */
export const yOf = (view: View, rect: Rect, value: number) => rect.top + ((view.high - value) / (view.high - view.low)) * rect.height;
/** The value at a place from the top. */
export const valueAtY = (view: View, rect: Rect, y: number) => view.high - ((y - rect.top) / rect.height) * (view.high - view.low);

/** How much of what is fitted is left free at each side. */
export const FIT_PADDING = 0.1;
/** The frames shown at each side of keys that are all at one frame. */
export const FLAT_FRAMES = 12;
/** The fewest and the most frames that a view shows, and the least and the greatest height of its values. */
const FRAMES = { low: 1, high: 1e7 };
const VALUES = { low: 1e-9, high: 1e9 };

/** The least and the greatest of numbers, opened where they are one number or none, and padded where they are not. */
function opened(numbers: readonly number[], empty: [number, number], flat: (only: number) => number, padding: number): [number, number] {
  if (numbers.length === 0) return empty;
  const low = Math.min(...numbers);
  const high = Math.max(...numbers);
  if (high === low) return [low - flat(low), low + flat(low)];
  const pad = (high - low) * padding;
  return [low - pad, high + pad];
}

/**
 * The view that shows settings whole: every key, and every handle of a Bezier stretch (a curve
 * lies within its control points), with `padding` of the extent free at each side. `frames` are
 * shown too (the frames that a clip shows, say). Keys at one frame have `FLAT_FRAMES` at each side;
 * one value has half of itself (of 1, for a value within 1 of 0) above and below.
 */
export function fitView(params: readonly Param[], frames: readonly number[] = [], padding = FIT_PADDING): View {
  const times = [...frames];
  const values: number[] = [];
  for (const param of params) {
    if (typeof param === 'number') { values.push(param); continue; }
    for (const key of param.keys) { times.push(key.frame); values.push(key.value); }
    for (const stretch of stretchesOf(param)) if (stretch.kind === 'bezier') values.push(stretch.points[1][1], stretch.points[2][1]);
  }
  const [from, to] = opened(times, [0, 2 * FLAT_FRAMES], () => FLAT_FRAMES, padding);
  const [low, high] = opened(values, [-1, 1], only => Math.max(Math.abs(only), 1) / 2, padding);
  return { from, to, low, high };
}

/** One axis of a view made `factor` times as large about a place on it, its length kept within `limits`. */
function scaled(low: number, high: number, factor: number, about: number, limits: Range): [number, number] {
  const length = high - low;
  const next = clamp(length / factor, limits.low, limits.high);
  const share = (about - low) / length;
  return [about - share * next, about + (1 - share) * next];
}

/**
 * A view zoomed about a point: what is at `about.frame` and at `about.value` stays where it is, and
 * the rest comes `factor` times nearer (above 1) or goes farther (below 1). An axis of which `about`
 * says nothing is left as it is. A factor that is no number above 0 changes nothing.
 */
export function zoomed(view: View, factor: number, about: { frame?: number; value?: number }): View {
  if (!(factor > 0) || !Number.isFinite(factor)) return { ...view };
  const [from, to] = about.frame === undefined ? [view.from, view.to] : scaled(view.from, view.to, factor, about.frame, FRAMES);
  const [low, high] = about.value === undefined ? [view.low, view.high] : scaled(view.low, view.high, factor, about.value, VALUES);
  return { from, to, low, high };
}

/** A view moved by frames and by a value: what was at a frame is `frames` to the left of where it was. */
export function panned(view: View, frames: number, value = 0): View {
  return { from: view.from + frames, to: view.to + frames, low: view.low + value, high: view.high + value };
}

/**
 * The marks of an axis from `from` to `to`: at most about `most` of them, at steps of 1, 2 or 5
 * times a power of ten; `whole` keeps them on whole numbers, as frames are.
 */
export function ticksOf(from: number, to: number, most: number, whole = false): number[] {
  const length = to - from;
  if (!(length > 0) || !Number.isFinite(length) || !(most >= 1)) return [];
  const rough = length / most;
  const power = 10 ** Math.floor(Math.log10(rough));
  let step = [1, 2, 5, 10].map(times => times * power).find(each => each >= rough) ?? 10 * power;
  if (whole) step = Math.max(1, Math.round(step));
  const ticks: number[] = [];
  for (let n = Math.ceil(from / step); n * step <= to && ticks.length <= most + 2; n += 1) ticks.push(n * step + 0);
  return ticks;
}

// --- The curve.

/** The stretch from a key to the next: how it goes, and its four control points. */
export interface Stretch {
  /** The place of the key that it leaves. */
  index: number;
  kind: 'linear' | 'hold' | 'bezier';
  /**
   * The key, the handle that leaves it, the handle that comes to the next key, and the next key.
   * Those of a straight stretch are a third of the way along it; a stretch that holds keeps its
   * value to the frame of the next key.
   */
  points: [At, At, At, At];
}

/** The stretch from `from` to `to`, which is the key after it. */
export function stretchOf(from: Keyframe, to: Keyframe, index = 0): Stretch {
  const gap = to.frame - from.frame;
  const first: At = [from.frame, from.value];
  const last: At = [to.frame, to.value];
  const interp = from.interp;
  if (interp === 'hold') return { index, kind: 'hold', points: [first, first, [to.frame, from.value], last] };
  if (interp === 'linear') {
    const rise = to.value - from.value;
    return { index, kind: 'linear', points: [first, [from.frame + gap / 3, from.value + rise / 3], [to.frame - gap / 3, to.value - rise / 3], last] };
  }
  const { out, in: into } = interp.bezier;
  // As `valueAt` has them: times at 0, out[0], 1 - in[0] and 1 of the gap; values from the two keys.
  return { index, kind: 'bezier', points: [first, [from.frame + out[0] * gap, from.value + out[1]], [from.frame + (1 - into[0]) * gap, to.value + into[1]], last] };
}

/** The stretches of a setting, in the order of its keys: one fewer than its keys, and none of a number. */
export function stretchesOf(param: Param): Stretch[] {
  if (typeof param === 'number') return [];
  return param.keys.slice(0, -1).map((key, index) => stretchOf(key, param.keys[index + 1], index));
}

/** A cubic Bezier curve of these control points at `s` of its way, summed as `valueAt` sums it. */
function along(p: Four, s: number): number {
  const r = 1 - s;
  return r * r * r * p[0] + 3 * r * r * s * p[1] + 3 * r * s * s * p[2] + s * s * s * p[3];
}

/** The frame and the value of a Bezier stretch at `s` of its way, from 0 to 1. */
export function pointOn(points: Stretch['points'], s: number): At {
  return [along([points[0][0], points[1][0], points[2][0], points[3][0]], s), along([points[0][1], points[1][1], points[2][1], points[3][1]], s)];
}

/** A number of a path: two places are finer than a screen shows. */
const n = (value: number) => String(Number(value.toFixed(2)) + 0);

/**
 * The curve of a setting between two frames of keys, as the `d` of an SVG path. Before the first
 * key it is level at the first's value and after the last at the last's, as the engine reads it; a
 * straight stretch is a line (`L`), a Bezier stretch one cubic curve of its own control points (`C`),
 * and a stretch that holds goes level to the frame of the next key and steps there (`H`, `V`). A
 * setting without keys is level.
 *
 * A stretch that lies wholly outside the frames is left out; one that crosses an end of them is
 * drawn whole, and what draws the path clips it.
 */
export function curvePath(param: Param, view: View, rect: Rect, from = view.from, to = view.to): string {
  const x = (frame: number) => n(xOf(view, rect, frame));
  const y = (value: number) => n(yOf(view, rect, value));
  if (typeof param === 'number' || param.keys.length === 0) {
    const level = typeof param === 'number' ? param : 0;
    return `M${x(from)},${y(level)} L${x(to)},${y(level)}`;
  }
  const keys = param.keys;
  const first = keys[0];
  const last = keys[keys.length - 1];
  const parts: string[] = [];
  // Whether the pen is where the next stretch begins.
  let pen = false;
  if (from < first.frame) { parts.push(`M${x(from)},${y(first.value)}`, `L${x(first.frame)},${y(first.value)}`); pen = true; }
  for (const stretch of stretchesOf(param)) {
    const [a, b, c, d] = stretch.points;
    if (d[0] < from || a[0] > to) { pen = false; continue; }
    if (!pen) parts.push(`M${x(a[0])},${y(a[1])}`);
    if (stretch.kind === 'linear') parts.push(`L${x(d[0])},${y(d[1])}`);
    else if (stretch.kind === 'hold') parts.push(`H${x(d[0])}`, `V${y(d[1])}`);
    else parts.push(`C${x(b[0])},${y(b[1])} ${x(c[0])},${y(c[1])} ${x(d[0])},${y(d[1])}`);
    pen = true;
  }
  if (to > last.frame) {
    if (!pen) parts.push(`M${x(last.frame)},${y(last.value)}`);
    parts.push(`L${x(to)},${y(last.value)}`);
  }
  if (parts.length === 0) parts.push(`M${x(first.frame)},${y(first.value)}`);
  return parts.join(' ');
}

/** Where a key is on the screen. */
export function keyPoint(key: Keyframe, view: View, rect: Rect): Point {
  return { x: xOf(view, rect, key.frame), y: yOf(view, rect, key.value) };
}

/** Which handle of a key: the one of the stretch that comes to it, or of the one that leaves it. */
export type Side = 'in' | 'out';

/**
 * The two Bezier handles of a key, as frames and values: `in` on the stretch that comes to it and
 * `out` on the one that leaves it, each null where there is no such stretch or it is not Bezier (a
 * straight stretch and one that holds show no handles).
 */
export function handlesOf(keys: readonly Keyframe[], index: number): { in: At | null; out: At | null } {
  if (index < 0 || index >= keys.length) return { in: null, out: null };
  const before = index > 0 ? stretchOf(keys[index - 1], keys[index]) : null;
  const after = index + 1 < keys.length ? stretchOf(keys[index], keys[index + 1]) : null;
  return { in: before?.kind === 'bezier' ? before.points[2] : null, out: after?.kind === 'bezier' ? after.points[1] : null };
}

/** Where the two handles of a key are on the screen: see `handlesOf`. */
export function handlePoints(keys: readonly Keyframe[], index: number, view: View, rect: Rect): { in: Point | null; out: Point | null } {
  const handles = handlesOf(keys, index);
  const point = (at: At | null): Point | null => (at ? { x: xOf(view, rect, at[0]), y: yOf(view, rect, at[1]) } : null);
  return { in: point(handles.in), out: point(handles.out) };
}

// --- What a pointer is on.

/** How many pixels from a key, a handle or a curve a pointer takes it. */
export const HIT_PX = 7;

/** The key nearest a point within `reach` pixels of it, by its place among the keys, or -1. */
export function hitKey(keys: readonly Keyframe[], view: View, rect: Rect, at: Point, reach = HIT_PX): number {
  let found = -1;
  let best = reach;
  keys.forEach((key, index) => {
    const point = keyPoint(key, view, rect);
    const distance = Math.hypot(point.x - at.x, point.y - at.y);
    if (distance <= best) { best = distance; found = index; }
  });
  return found;
}

/** The handle nearest a point within `reach` pixels of it, among the handles of the keys at `indices`, or null. */
export function hitHandle(keys: readonly Keyframe[], indices: readonly number[], view: View, rect: Rect, at: Point, reach = HIT_PX): { index: number; side: Side } | null {
  let found: { index: number; side: Side } | null = null;
  let best = reach;
  for (const index of indices) {
    const points = handlePoints(keys, index, view, rect);
    for (const side of ['in', 'out'] as const) {
      const point = points[side];
      if (!point) continue;
      const distance = Math.hypot(point.x - at.x, point.y - at.y);
      if (distance <= best) { best = distance; found = { index, side }; }
    }
  }
  return found;
}

/** How far a point is from the line between two others. */
function fromSegment(at: Point, a: Point, b: Point): number {
  const dx = b.x - a.x;
  const dy = b.y - a.y;
  const length = dx * dx + dy * dy;
  const share = length === 0 ? 0 : clamp(((at.x - a.x) * dx + (at.y - a.y) * dy) / length, 0, 1);
  return Math.hypot(at.x - (a.x + share * dx), at.y - (a.y + share * dy));
}

/**
 * How many pixels a point is from the curve of a setting, as far as the curve is within `reach`
 * pixels of it to the left and to the right: the curve is read at each pixel there, where `valueAt`
 * reads it, and the step of a stretch that holds is the line between two of them.
 */
export function curveDistance(param: Param, view: View, rect: Rect, at: Point, reach = HIT_PX): number {
  let best = Infinity;
  let before: Point | null = null;
  const steps = Math.max(1, Math.ceil(reach));
  for (let step = -steps; step <= steps; step += 1) {
    const x = at.x + step;
    const point = { x, y: yOf(view, rect, valueAt(param, frameAt(view, rect, x))) };
    if (before) best = Math.min(best, fromSegment(at, before, point));
    before = point;
  }
  return best;
}

/** Whether a point is on the curve of a setting, within `reach` pixels. */
export function hitCurve(param: Param, view: View, rect: Rect, at: Point, reach = HIT_PX): boolean {
  return curveDistance(param, view, rect, at, reach) <= reach;
}

/** The keys within the box between two points, by their places. */
export function keysIn(keys: readonly Keyframe[], view: View, rect: Rect, a: Point, b: Point): number[] {
  const left = Math.min(a.x, b.x);
  const right = Math.max(a.x, b.x);
  const top = Math.min(a.y, b.y);
  const bottom = Math.max(a.y, b.y);
  return keys.flatMap((key, index) => {
    const point = keyPoint(key, view, rect);
    return point.x >= left && point.x <= right && point.y >= top && point.y <= bottom ? [index] : [];
  });
}

/** The places of the keys that are at some of these frames, in the order of the keys. */
export function selectedIn(keys: readonly Keyframe[], frames: readonly number[]): number[] {
  return keys.flatMap((key, index) => (frames.includes(key.frame) ? [index] : []));
}

// --- The edits.

/** The handle of an ease: flat at its key (as `./value` has it). */
const FLAT: Handle = [EASE_SHARE, 0];
/** The handles of a straight stretch that rises by `rise` (as `./value` has them). */
const straight = (rise: number): [Handle, Handle] => [[EASE_SHARE, rise * EASE_SHARE], [EASE_SHARE, -rise * EASE_SHARE]];
const same = (a: Handle, b: Handle) => a[0] === b[0] && a[1] === b[1];

/** The places among `count` keys that `selected` names, each once. */
function chosenOf(selected: readonly number[], count: number): Set<number> {
  return new Set(selected.filter(index => Number.isInteger(index) && index >= 0 && index < count));
}

/**
 * The keys `after` a change of values, with the handles that lay on the straight line of their
 * stretch `before` laid on the line as it is now. An ease is a Bezier stretch with one handle flat
 * and the other on the line (`eased`): moved so, it stays that ease. A flat handle and one that was
 * pulled elsewhere keep what they were. Both lists hold the same keys in the same order.
 */
function carried(before: readonly Keyframe[], after: Keyframe[]): Keyframe[] {
  return after.map((key, index) => {
    const interp = before[index].interp;
    if (interp === 'linear' || interp === 'hold' || index + 1 >= after.length) return key;
    const was = straight(before[index + 1].value - before[index].value);
    const now = straight(after[index + 1].value - key.value);
    const kept = (handle: Handle, line: Handle, fresh: Handle): Handle => (same(handle, line) && !same(line, FLAT) ? fresh : [handle[0], handle[1]]);
    return { ...key, interp: { bezier: { out: kept(interp.bezier.out, was[0], now[0]), in: kept(interp.bezier.in, was[1], now[1]) } } };
  });
}

/**
 * The keys with those at `selected` moved by frames and by a value (Reference Manual 20, p. 1118).
 * They move together in time, by whole frames, and no further than leaves each strictly between the
 * keys beside it that are not moved, so the order of the keys stays; each value is kept within
 * `bounds` by itself. A stretch that was eased stays so (`carried`).
 */
export function moveKeys(keys: readonly Keyframe[], selected: readonly number[], frames: number, value: number, bounds: Range): Keyframe[] {
  const chosen = chosenOf(selected, keys.length);
  if (chosen.size === 0) return keys.map(copy);
  let low = -Infinity;
  let high = Infinity;
  for (const index of chosen) {
    if (index > 0 && !chosen.has(index - 1)) low = Math.max(low, keys[index - 1].frame + 1 - keys[index].frame);
    if (index + 1 < keys.length && !chosen.has(index + 1)) high = Math.min(high, keys[index + 1].frame - 1 - keys[index].frame);
  }
  low = Math.ceil(low);
  high = Math.floor(high);
  const asked = Number.isFinite(frames) ? Math.round(frames) : 0;
  // `+ 0`: a move of -0 frames is one of none.
  const shift = low > high ? 0 : clamp(asked, low, high) + 0;
  const by = Number.isFinite(value) ? value : 0;
  const moved = keys.map((key, index) => (chosen.has(index)
    ? { ...key, frame: key.frame + shift, value: clamp(key.value + by, bounds.low, bounds.high) }
    : copy(key)));
  return carried(keys, moved);
}

/** How the keys chosen are made to go: one of the four eases, or held. */
export type KeyEase = Ease | 'hold';
/** Every way, in the order of the tool bar. */
export const KEY_EASES: readonly KeyEase[] = ['linear', 'in', 'out', 'in_out', 'hold'];

/**
 * The keys with those at `selected` come to and left in a way (Reference Manual 20, pp. 1118–1119).
 * `hold` makes each of them keep its value to the next key. An ease (`eased`) first lets go of a
 * key chosen that holds, whose stretch is then straight, and so can be eased; the stretch that comes
 * to it from a key that holds and is not chosen is left held.
 */
export function setInterp(keys: readonly Keyframe[], selected: readonly number[], kind: KeyEase): Keyframe[] {
  let after = keys.map(copy);
  const chosen = [...chosenOf(selected, keys.length)].sort((a, b) => a - b);
  if (kind === 'hold') {
    for (const index of chosen) after[index].interp = 'hold';
    return after;
  }
  for (const index of chosen) if (after[index].interp === 'hold') after[index].interp = 'linear';
  for (const index of chosen) after = eased(after, index, kind);
  return after;
}

/** Whether every key at `selected` goes in a way: for the button of the tool bar that is lit. */
export function interpIs(keys: readonly Keyframe[], selected: readonly number[], kind: KeyEase): boolean {
  const chosen = [...chosenOf(selected, keys.length)];
  if (chosen.length === 0) return false;
  if (kind === 'hold') return chosen.every(index => keys[index].interp === 'hold');
  return chosen.every(index => keys[index].interp !== 'hold') && spliceOf(keys, setInterp(keys, chosen, kind)) === null;
}

/**
 * The keys with a Bezier handle of the key at `index` dragged to a frame and a value (Reference
 * Manual 20, p. 1118). The handle keeps to its stretch: its share of the frames between the two keys
 * is kept from 0 to 1, as the engine takes it, and its `dv` is the value it was dragged to less its
 * key's. With `bounds`, the value is kept within them, so the curve (which lies within its control
 * points) never leaves what the setting takes. A straight stretch becomes a Bezier one whose other
 * handle stays on the line; one that holds, or a side with no stretch, is left as it is.
 */
export function dragHandle(keys: readonly Keyframe[], index: number, side: Side, frame: number, value: number, bounds?: Range): Keyframe[] {
  const after = keys.map(copy);
  const from = side === 'out' ? index : index - 1;
  if (!Number.isInteger(index) || from < 0 || from + 1 >= keys.length || !Number.isFinite(frame) || !Number.isFinite(value)) return after;
  const interp = keys[from].interp;
  if (interp === 'hold') return after;
  const first = keys[from];
  const next = keys[from + 1];
  const line = straight(next.value - first.value);
  const out: Handle = interp === 'linear' ? line[0] : [interp.bezier.out[0], interp.bezier.out[1]];
  const into: Handle = interp === 'linear' ? line[1] : [interp.bezier.in[0], interp.bezier.in[1]];
  const key = keys[index];
  const share = clamp((side === 'out' ? frame - key.frame : key.frame - frame) / (next.frame - first.frame), 0, 1);
  const to = bounds ? clamp(value, bounds.low, bounds.high) : value;
  const handle: Handle = [share, to - key.value];
  after[from].interp = { bezier: side === 'out' ? { out: handle, in: into } : { out, in: handle } };
  return after;
}

/** The two halves of a cubic Bezier curve of these control points cut at `s` of its way (de Casteljau). */
function halves(p: Four, s: number): [Four, Four] {
  const mix = (a: number, b: number) => a + (b - a) * s;
  const a = mix(p[0], p[1]);
  const b = mix(p[1], p[2]);
  const c = mix(p[2], p[3]);
  const d = mix(a, b);
  const e = mix(b, c);
  const middle = mix(d, e);
  return [[p[0], a, d, middle], [middle, e, c, p[3]]];
}

/** How far along a curve of these times it is at `share` of its time: halved 64 times, as `valueAt` finds it. */
function wayAt(x: Four, share: number): number {
  let low = 0;
  let high = 1;
  for (let step = 0; step < 64; step += 1) {
    const middle = (low + high) / 2;
    if (along(x, middle) < share) low = middle;
    else high = middle;
  }
  return (low + high) / 2;
}

/**
 * The keys of a setting with a key put on its curve at a frame (rounded to a whole one), of the
 * value that the setting has there, and the place of that key; null where there is a key at that
 * frame already (Reference Manual 20, p. 1117: Option-click on a curve).
 *
 * The curve keeps its shape, as it does where a linear key is put on a line:
 * - on a straight stretch the key is linear, and the two stretches are the line;
 * - on a stretch that holds the key holds too, with the value held;
 * - a Bezier stretch is cut in two at the key (de Casteljau), and each half takes the handles of its
 *   part of the curve, so the two halves are the curve that was there. A handle of a half would have
 *   to reach past the new key only where a handle of the whole reached well past the middle of its
 *   stretch; its share is then kept at 1, and that half differs a little;
 * - before the first key and after the last the setting is level, and the new key is linear with the
 *   value there. The key that was last then gets a stretch: where it was Bezier it becomes linear,
 *   since its handles would bend what was level.
 * A setting that is a number gets its first key, with that number.
 */
export function insertKey(param: Param, frame: number): { keys: Keyframe[]; index: number } | null {
  const at = Math.round(frame) + 0;
  if (!Number.isFinite(at)) return null;
  if (typeof param === 'number') return { keys: [{ frame: at, value: param, interp: 'linear' }], index: 0 };
  const keys = param.keys.map(copy);
  if (keys.some(key => key.frame === at)) return null;
  const value = valueAt(param, at);
  const found = keys.findIndex(key => key.frame > at);
  if (found === 0 || keys.length === 0) {
    keys.unshift({ frame: at, value, interp: 'linear' });
    return { keys, index: 0 };
  }
  if (found < 0) {
    const last = keys[keys.length - 1];
    if (last.interp !== 'linear' && last.interp !== 'hold') last.interp = 'linear';
    keys.push({ frame: at, value, interp: 'linear' });
    return { keys, index: keys.length - 1 };
  }
  const from = keys[found - 1];
  const to = keys[found];
  let made: Keyframe;
  if (from.interp === 'linear' || from.interp === 'hold') made = { frame: at, value, interp: from.interp };
  else {
    const { out, in: into } = from.interp.bezier;
    const share = (at - from.frame) / (to.frame - from.frame);
    const x: Four = [0, out[0], 1 - into[0], 1];
    const s = wayAt(x, share);
    const [leftX, rightX] = halves(x, s);
    const [leftY, rightY] = halves([from.value, from.value + out[1], to.value + into[1], to.value], s);
    // Each half in the shares of its own gap, which are `share` and `1 - share` of the whole.
    from.interp = { bezier: { out: [clamp(leftX[1] / share, 0, 1), leftY[1] - from.value], in: [clamp((share - leftX[2]) / share, 0, 1), leftY[2] - value] } };
    made = { frame: at, value, interp: { bezier: {
      out: [clamp((rightX[1] - share) / (1 - share), 0, 1), rightY[1] - value],
      in: [clamp((1 - rightX[2]) / (1 - share), 0, 1), rightY[2] - to.value],
    } } };
  }
  keys.splice(found, 0, made);
  return { keys, index: found };
}

/**
 * A setting with the keys at `selected` taken away (Reference Manual 20, p. 1118). Where a key
 * between two that stay is taken, the stretch that is left keeps the handle with which it left the
 * first and the one with which it came to the second (a handle on the line of its stretch is laid on
 * the line of the new one; a side that was straight, or the coming side of a stretch that held, is
 * straight), so an ease at a key that stays is still there; a stretch that left its key held still
 * holds. With its last key taken a setting is the number that it had: the value at `at`, a frame of
 * keys (the playhead, say), or at its first key where none is given, which for one key is that key's
 * value. Nothing chosen gives the setting itself.
 */
export function deleteKeys(param: Param, selected: readonly number[], at?: number): Param {
  if (typeof param === 'number') return param;
  const keys = param.keys;
  const chosen = chosenOf(selected, keys.length);
  if (chosen.size === 0) return param;
  if (chosen.size === keys.length) return valueAt(param, at ?? keys[0].frame);
  const kept: Keyframe[] = [];
  keys.forEach((key, index) => {
    if (chosen.has(index)) return;
    const made = copy(key);
    // The key that stays after this one, where keys between them go.
    let next = index + 1;
    while (next < keys.length && chosen.has(next)) next += 1;
    if (next > index + 1 && next < keys.length && key.interp !== 'hold') {
      const line = straight(keys[next].value - key.value);
      const leaving = key.interp;
      const coming = keys[next - 1].interp;
      const out: Handle = leaving === 'linear' ? line[0]
        : same(leaving.bezier.out, straight(keys[index + 1].value - key.value)[0]) ? line[0] : [leaving.bezier.out[0], leaving.bezier.out[1]];
      const into: Handle = coming === 'linear' || coming === 'hold' ? line[1]
        : same(coming.bezier.in, straight(keys[next].value - keys[next - 1].value)[1]) ? line[1] : [coming.bezier.in[0], coming.bezier.in[1]];
      made.interp = same(out, line[0]) && same(into, line[1]) ? 'linear' : { bezier: { out, in: into } };
    }
    kept.push(made);
  });
  return { keys: kept };
}

/** A key that was copied: how many frames it is after the first of those copied with it. */
export interface CopiedKey { offset: number; value: number; interp: Interp }

/** The keys at `selected` as they are copied, in the order of their frames (Reference Manual 20, p. 1118). */
export function copyKeys(keys: readonly Keyframe[], selected: readonly number[]): CopiedKey[] {
  const chosen = [...chosenOf(selected, keys.length)].sort((a, b) => a - b);
  if (chosen.length === 0) return [];
  const first = keys[chosen[0]].frame;
  return chosen.map(index => ({ offset: keys[index].frame - first, value: keys[index].value, interp: keys[index].interp }));
}

/**
 * A setting with copied keys pasted so that the first of them is at a frame (rounded to a whole
 * one) and the others as far after it as they were (Reference Manual 20, p. 1118), and the frames
 * of the keys pasted; null where nothing was copied. A key that is at one of those frames is
 * replaced, and the others stay. Each value is kept within `bounds`. The keys go as they were copied
 * to go, but for the last: the handles of a Bezier stretch that left it belong to a stretch that was
 * not copied, so it is linear (where it replaces a key, it goes on as that key did).
 */
export function pasteKeys(param: Param, copied: readonly CopiedKey[], at: number, bounds: Range): { keys: Keyframe[]; frames: number[] } | null {
  const start = Math.round(at) + 0;
  if (copied.length === 0 || !Number.isFinite(start)) return null;
  const before = typeof param === 'number' ? [] : param.keys;
  const pasted = new Map<number, Keyframe>();
  copied.forEach((each, place) => {
    const frame = start + Math.round(each.offset);
    const there = before.find(key => key.frame === frame);
    const last = place === copied.length - 1;
    const interp: Interp = last && there ? there.interp : last && each.interp !== 'linear' && each.interp !== 'hold' ? 'linear' : each.interp;
    pasted.set(frame, { frame, value: clamp(each.value, bounds.low, bounds.high), interp });
  });
  const keys = [...before.filter(key => !pasted.has(key.frame)).map(copy), ...pasted.values()].sort((a, b) => a.frame - b.frame);
  return { keys, frames: [...pasted.keys()].sort((a, b) => a - b) };
}

const sameInterp = (a: Interp, b: Interp) => (typeof a === 'string' || typeof b === 'string'
  ? a === b
  : same(a.bezier.out, b.bezier.out) && same(a.bezier.in, b.bezier.in));
const sameKey = (a: Keyframe, b: Keyframe) => a.frame === b.frame && a.value === b.value && sameInterp(a.interp, b.interp);

/** What `splice_keys` takes: `keys` in place of the `remove` keys from the key `at` on. */
export interface Splice { at: number; remove: number; keys: Keyframe[] }

/**
 * The one splice that makes the keys `before` into the keys `after`: what lies between the keys
 * that they begin with alike and those that they end with alike. Null where they are the same keys.
 */
export function spliceOf(before: readonly Keyframe[], after: readonly Keyframe[]): Splice | null {
  let head = 0;
  while (head < before.length && head < after.length && sameKey(before[head], after[head])) head += 1;
  let tail = 0;
  while (tail < before.length - head && tail < after.length - head && sameKey(before[before.length - 1 - tail], after[after.length - 1 - tail])) tail += 1;
  const remove = before.length - head - tail;
  const keys = after.slice(head, after.length - tail).map(copy);
  return remove === 0 && keys.length === 0 ? null : { at: head, remove, keys };
}
