/**
 * The value of a setting with keys at a frame, and the eases of a key, as the engine finds them
 * (`Param::value_at` of engine/crates/model/src/grade.rs and engine/crates/model/src/grade/ease.rs;
 * PROTOCOL.md, "Grades"). A curve editor draws a curve where the engine reads it, so the sums are
 * done here in the order in which the engine does them, and come out the same to the last bit: the
 * engine writes its values out (`engine/crates/model/tests/param_values.rs`), and the tests of this
 * module read them. Pure.
 */
import type { Interp, Keyframe, Param } from '../engine';

type Four = [number, number, number, number];
type Handle = [number, number];

/** The point of a cubic Bezier curve of these control points at `s` of its way. */
function along(p: Four, s: number): number {
  const r = 1 - s;
  return r * r * r * p[0] + 3 * r * r * s * p[1] + 3 * r * s * s * p[2] + s * s * s * p[3];
}

/**
 * The value at `share` of the time of a cubic Bezier curve whose control points are at times `x`
 * and values `y`: the time is found by halving 64 times, as the engine finds it.
 */
function bezier(share: number, x: Four, y: Four): number {
  let low = 0;
  let high = 1;
  for (let step = 0; step < 64; step += 1) {
    const middle = (low + high) / 2;
    if (along(x, middle) < share) low = middle;
    else high = middle;
  }
  return along(y, (low + high) / 2);
}

/**
 * The value of a setting at a frame, which may lie between two frames. Before the first key it is
 * the first's and after the last the last's; a setting of keys without one is 0, as the engine
 * reads it, though the engine takes no such setting.
 */
export function valueAt(param: Param, frame: number): number {
  if (typeof param === 'number') return param;
  const keys = param.keys;
  if (keys.length === 0) return 0;
  const first = keys[0];
  const last = keys[keys.length - 1];
  if (frame <= first.frame) return first.value;
  if (frame >= last.frame) return last.value;
  // The last key at or before the frame; there is one after it.
  let low = 0;
  let high = keys.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (keys[middle].frame <= frame) low = middle + 1;
    else high = middle;
  }
  const from = keys[low - 1];
  const to = keys[low];
  const share = (frame - from.frame) / (to.frame - from.frame);
  const interp = from.interp;
  if (interp === 'linear') return from.value + (to.value - from.value) * share;
  if (interp === 'hold') return from.value;
  const { out, in: into } = interp.bezier;
  return bezier(share, [0, out[0], 1 - into[0], 1], [from.value, from.value + out[1], to.value + into[1], to.value]);
}

/** How a key is come to and left (Reference Manual 20, pp. 1105 and 1117). */
export type Ease = 'linear' | 'in' | 'out' | 'in_out';
/** Every way, in the order of the menu. */
export const EASES: readonly Ease[] = ['linear', 'in', 'out', 'in_out'];

/** The share of the frames between two keys that the handle of an ease reaches over. */
export const EASE_SHARE = 1 / 3;
/** The handle of an ease: flat at its key. */
const FLAT: Handle = [EASE_SHARE, 0];

/** The handles of a straight stretch that rises by `rise`: the one that leaves, the one that comes. */
function straight(rise: number): [Handle, Handle] {
  return [[EASE_SHARE, rise * EASE_SHARE], [EASE_SHARE, -rise * EASE_SHARE]];
}

const same = (a: Handle, b: Handle) => a[0] === b[0] && a[1] === b[1];

/** The two handles of the stretch from a key to the next, as they are or as a line has them. */
function handles(from: Keyframe, to: Keyframe): [Handle, Handle] | undefined {
  if (from.interp === 'hold') return undefined;
  if (from.interp === 'linear') return straight(to.value - from.value);
  return [from.interp.bezier.out, from.interp.bezier.in];
}

/** A stretch of these handles: linear where they are those of a line. */
function stretch(out: Handle, into: Handle, rise: number): Interp {
  const line = straight(rise);
  return same(out, line[0]) && same(into, line[1]) ? 'linear' : { bezier: { out: [out[0], out[1]], in: [into[0], into[1]] } };
}

/**
 * The keys with the key at `index` come to and left in a way. The stretches on its two sides are
 * changed, each in its handle at this key; a stretch that holds is left alone, and so is everything
 * where there is no such key. The keys given are not changed.
 */
export function eased(keys: readonly Keyframe[], index: number, ease: Ease): Keyframe[] {
  const after = keys.map(key => ({ ...key }));
  if (index < 0 || index >= keys.length) return after;
  if (index > 0) {
    const before = keys[index - 1];
    const here = keys[index];
    const rise = here.value - before.value;
    const held = handles(before, here);
    if (held) after[index - 1].interp = stretch(held[0], ease === 'in' || ease === 'in_out' ? FLAT : straight(rise)[1], rise);
  }
  if (index + 1 < keys.length) {
    const here = keys[index];
    const next = keys[index + 1];
    const rise = next.value - here.value;
    const held = handles(here, next);
    if (held) after[index].interp = stretch(ease === 'out' || ease === 'in_out' ? FLAT : straight(rise)[0], held[1], rise);
  }
  return after;
}

/**
 * The way that the key at `index` is come to and left, if it is one of the four: none where a
 * handle at the key was pulled to somewhere else, or where there is no such key. A side that has
 * no stretch, or one that holds, agrees with every way.
 */
export function easeOf(keys: readonly Keyframe[], index: number): Ease | undefined {
  if (index < 0 || index >= keys.length) return undefined;
  const here = keys[index];
  // Whether a side is eased (true), straight (false), of no say (null), or pulled (undefined).
  const side = (handle: Handle | undefined, line: Handle): boolean | null | undefined => {
    if (!handle) return null;
    if (same(handle, FLAT) && !same(line, FLAT)) return true;
    return same(handle, line) ? false : undefined;
  };
  const before = index > 0 ? keys[index - 1] : undefined;
  const next = index + 1 < keys.length ? keys[index + 1] : undefined;
  const coming = before ? side(handles(before, here)?.[1], straight(here.value - before.value)[1]) : null;
  const leaving = next ? side(handles(here, next)?.[0], straight(next.value - here.value)[0]) : null;
  if (coming === undefined || leaving === undefined) return undefined;
  if (coming) return leaving ? 'in_out' : 'in';
  return leaving ? 'out' : 'linear';
}
