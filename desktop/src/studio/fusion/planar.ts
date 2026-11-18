/**
 * The Planar Tracker of a composition on the Fusion page (DaVinci Resolve Reference Manual 20,
 * Chapter 83, pp. 1760–1766 and 2749–2758; PROTOCOL.md, "The Planar Tracker" and "The region of a
 * Planar Tracker"): its region, a polygon of the plane at its reference frame; the corners of its
 * Corner Pin; and its path, the homography that takes the plane from the reference frame to each
 * frame that was tracked.
 *
 * None of its settings takes keys. Its region and its corners are set whole (`set_fusion_node` with
 * `{"region": [...]}` and `{"corners": {...}}`); its path is the result of the tracking, which the
 * page clears and changes no other way: a frame set to `null` is taken away. The engine does not
 * make the corners of the region: while the Corner Pin is not what the node does, a region that the
 * page draws sets the corners to the corners of the box about it, so that the Corner Pin begins on
 * the region once it is chosen; with the Corner Pin chosen, the corners are the user's, and a region
 * drawn leaves them as they are.
 *
 * The path is read as the engine reads it (`PlanarTracker::homography` and `motion_at` of
 * `crates/model/src/fusion/planar.rs`): exactly at its frames; between two of them the homography
 * that puts the corners of the box about the region on the lines between where the two put them;
 * before its first frame and after its last as there; and the motion of the plane at a frame is the
 * homography there after the inverse of the one at the reference frame. A homography acts on places
 * as a centre counts them, from the lower left corner of the picture, `y` up. Pure.
 */
import type { FusionHomography, FusionNode, FusionPlanarCorners, FusionPlanarTracker, FusionTrackerPlace, Op } from '../engine';
import type { FusionAt } from './edits';

/** The fewest places of a region: those of a triangle. The page takes none away below them. */
export const LEAST_REGION_POINTS = 3;
/** The most places of a region (PROTOCOL.md, "The Planar Tracker"). */
export const MOST_REGION_POINTS = 64;
/** The range of a place of a region and of a corner, as of a centre; and the middle of the picture. */
export const PLANAR_PLACE = { neutral: 0.5, low: -15.5, high: 16.5 } as const;

/** The four corners of the Corner Pin, in the order that its controls are listed in: upper left, upper right, lower right, lower left. */
export const CORNERS = ['tl', 'tr', 'br', 'bl'] as const;
export type Corner = (typeof CORNERS)[number];

/** A 3×3 matrix, row by row. */
export type Matrix3 = [[number, number, number], [number, number, number], [number, number, number]];
/** The matrix that moves nothing. */
export const IDENTITY: Matrix3 = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];

/** The matrix of a homography of a path, its last number 1. */
export const matrixOf = (h: FusionHomography): Matrix3 => [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1]];

/** Whether two matrices are the same, number for number, as the engine compares them. */
const same = (a: Matrix3, b: Matrix3): boolean => a.every((row, i) => row.every((value, j) => value === b[i][j]));

/** The product `a · b`: `b` first, then `a`. */
export function multiply(a: Matrix3, b: Matrix3): Matrix3 {
  const at = (i: number, j: number) => a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
  return [[at(0, 0), at(0, 1), at(0, 2)], [at(1, 0), at(1, 1), at(1, 2)], [at(2, 0), at(2, 1), at(2, 2)]];
}

/** A matrix divided by its last number, so that it is 1; null where that is 0 or a number is not finite. */
export function normalized(m: Matrix3): Matrix3 | null {
  const corner = m[2][2];
  if (!Number.isFinite(corner) || Math.abs(corner) < 1e-12) return null;
  const out = m.map(row => row.map(value => value / corner)) as Matrix3;
  return out.every(row => row.every(Number.isFinite)) ? out : null;
}

/**
 * The inverse of a matrix; null where it has none (it flattens the plane) or a number is not finite. It is not made to end in 1:
 * a homography is the same at any scale, and its inverse may end in 0.
 */
export function inverse(m: Matrix3): Matrix3 | null {
  const [[a, b, c], [d, e, f], [g, h, i]] = m;
  const A = e * i - f * h;
  const B = -(d * i - f * g);
  const C = d * h - e * g;
  const det = a * A + b * B + c * C;
  if (!Number.isFinite(det) || Math.abs(det) < 1e-12) return null;
  const adjugate: Matrix3 = [
    [A, -(b * i - c * h), b * f - c * e],
    [B, a * i - c * g, -(a * f - c * d)],
    [C, -(a * h - b * g), a * e - b * d],
  ];
  const out = adjugate.map(row => row.map(value => value / det)) as Matrix3;
  return out.every(row => row.every(Number.isFinite)) ? out : null;
}

/** Where a matrix puts a place: `[x, y, 1]` through it, divided by its third number; null where that is 0 or the place is not finite. */
export function applyTo(m: Matrix3, [x, y]: readonly [number, number]): [number, number] | null {
  const w = m[2][0] * x + m[2][1] * y + m[2][2];
  if (!Number.isFinite(w) || Math.abs(w) < 1e-12) return null;
  const out: [number, number] = [(m[0][0] * x + m[0][1] * y + m[0][2]) / w, (m[1][0] * x + m[1][1] * y + m[1][2]) / w];
  return Number.isFinite(out[0]) && Number.isFinite(out[1]) ? out : null;
}

/**
 * The homography that puts the four places `from` on the four places `to`, its last number 1: the
 * eight numbers that solve the eight equations of the four, by Gaussian elimination. Null where the
 * places make no quadrilateral (three on a line, two at one place).
 */
export function fitHomography(from: readonly (readonly [number, number])[], to: readonly (readonly [number, number])[]): Matrix3 | null {
  if (from.length !== 4 || to.length !== 4) return null;
  // Each pair gives two rows: h11 x + h12 y + h13 − h31 x x' − h32 y x' = x', and the same of y'.
  const rows: number[][] = [];
  for (let at = 0; at < 4; at++) {
    const [x, y] = from[at];
    const [u, v] = to[at];
    rows.push([x, y, 1, 0, 0, 0, -x * u, -y * u, u]);
    rows.push([0, 0, 0, x, y, 1, -x * v, -y * v, v]);
  }
  for (let column = 0; column < 8; column++) {
    let pivot = column;
    for (let row = column + 1; row < 8; row++) if (Math.abs(rows[row][column]) > Math.abs(rows[pivot][column])) pivot = row;
    if (!(Math.abs(rows[pivot][column]) > 1e-12)) return null;
    [rows[column], rows[pivot]] = [rows[pivot], rows[column]];
    for (let row = 0; row < 8; row++) {
      if (row === column) continue;
      const factor = rows[row][column] / rows[column][column];
      if (factor === 0) continue;
      for (let k = column; k < 9; k++) rows[row][k] -= factor * rows[column][k];
    }
  }
  const h = rows.map((row, index) => row[8] / row[index]);
  const out: Matrix3 = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1]];
  return out.every(row => row.every(Number.isFinite)) ? out : null;
}

/** A frame of a path, of the keys of the clip, and its homography. */
export type PathFrame = [frame: number, homography: FusionHomography];

/** The frames of a path and their homographies, in the order of the frames: a path is written as an object of the frames, as text. */
export function pathFrames(path: FusionPlanarTracker['path'] | null | undefined): PathFrame[] {
  return Object.entries(path ?? {})
    .map(([frame, homography]): PathFrame => [Number(frame), homography])
    .filter(([frame, homography]) => Number.isFinite(frame) && Array.isArray(homography) && homography.length === 8)
    .sort((a, b) => a[0] - b[0]);
}

/** The least width and height, in shares of the picture, of the box about a region for the path to be read between its frames by it. */
const SMALLEST = 1e-6;
/** The corners of the picture, from its lower left one round to its upper left one. */
const PICTURE: [number, number][] = [[0, 0], [1, 0], [1, 1], [0, 1]];

/** The corners of the box about a region, from its lower left one round; those of the picture where the box is less than `SMALLEST` across or up. */
function boundingBox(region: readonly FusionTrackerPlace[]): [number, number][] {
  const xs = region.map(place => place.x);
  const ys = region.map(place => place.y);
  const [left, right, bottom, top] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  if (!(right - left > SMALLEST && top - bottom > SMALLEST)) return PICTURE;
  return [[left, bottom], [right, bottom], [right, top], [left, top]];
}

/**
 * The homography of the path of a Planar Tracker at a frame of its keys, which may lie between frames: at a frame of the path the
 * homography written there; between two frames the homography that puts the corners of the box about the region on the lines
 * between where those of the two frames put them, as far along as the frame is between them (that of the frame before where it
 * cannot be made); before the first frame that of the first, after the last that of the last; and none where the path has no
 * frame, or the frame is no number.
 */
export function homographyAt(params: Pick<FusionPlanarTracker, 'path' | 'region'>, frame: number): Matrix3 {
  if (Number.isNaN(frame)) return IDENTITY;
  const frames = pathFrames(params.path);
  if (frames.length === 0) return IDENTITY;
  const [first, start] = frames[0];
  const [last, end] = frames[frames.length - 1];
  if (frame <= first) return matrixOf(start);
  if (frame >= last) return matrixOf(end);
  const whole = Math.floor(frame);
  // The frame is after the first of the path and before the last: one is at it or before it, and one after it.
  const after = frames.findIndex(([at]) => at > whole);
  const [beforeFrame, from] = frames[after - 1];
  const [afterFrame, to] = frames[after];
  const share = (frame - beforeFrame) / (afterFrame - beforeFrame);
  if (share <= 0) return matrixOf(from);
  const [earlier, later] = [matrixOf(from), matrixOf(to)];
  const corners = boundingBox(Array.isArray(params.region) ? params.region : []);
  const on: [number, number][] = [];
  for (const corner of corners) {
    const a = applyTo(earlier, corner);
    const b = applyTo(later, corner);
    if (a === null || b === null) return earlier;
    on.push([a[0] + (b[0] - a[0]) * share, a[1] + (b[1] - a[1]) * share]);
  }
  return fitHomography(corners, on) ?? earlier;
}

/**
 * The motion of the plane of a Planar Tracker from its reference frame to a frame of its keys: the homography of the path there
 * after the inverse of the one at the reference frame, so that nothing moves at the reference frame whatever the path has there;
 * the identity where the two are one and where the path has no frame. Null where the homography at the reference frame cannot be
 * undone: the node is its input then.
 */
export function motionAt(params: Pick<FusionPlanarTracker, 'path' | 'region' | 'reference'>, frame: number): Matrix3 | null {
  const rest = homographyAt(params, params.reference);
  const there = homographyAt(params, frame);
  if (same(there, rest)) return IDENTITY;
  if (same(rest, IDENTITY)) return there;
  const back = inverse(rest);
  return back === null ? null : normalized(multiply(there, back));
}

/** The corners of the box about a region, as the Corner Pin names them: its upper left, upper right, lower right and lower left. */
export function boxCorners(region: readonly FusionTrackerPlace[]): FusionPlanarCorners {
  const xs = region.map(place => place.x);
  const ys = region.map(place => place.y);
  const [left, right, bottom, top] = region.length === 0 ? [0.25, 0.75, 0.25, 0.75] : [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  return { tl: { x: left, y: top }, tr: { x: right, y: top }, br: { x: right, y: bottom }, bl: { x: left, y: bottom } };
}

/** The settings of a node that is a Planar Tracker; null of a node of another kind. */
const planarOf = (node: FusionNode): FusionPlanarTracker | null => (node.kind === 'planar_tracker' ? node.params : null);

/** The settings of a Planar Tracker that are set by a patch, each of which takes no keys. */
export type PlanarSettings = Partial<Pick<FusionPlanarTracker, 'operation' | 'motion' | 'reference' | 'corners'>>;

/** Set settings of a Planar Tracker by one patch: those that are another value than they are. Nothing where none is. */
export function setPlanar(at: FusionAt, node: FusionNode, patch: PlanarSettings): Op[] {
  const params = planarOf(node) as unknown as Record<string, unknown> | null;
  if (params === null) return [];
  const changed = Object.entries(patch).filter(([name, value]) => value !== undefined && JSON.stringify(params[name]) !== JSON.stringify(value));
  return changed.length === 0 ? [] : [{ op: 'set_fusion_node', ...at, id: node.id, params: Object.fromEntries(changed) }];
}

/**
 * The region of a Planar Tracker set whole, as each move of a drag of one of its points sends it, whatever it was before: with
 * the corners of the box about it, where the Corner Pin is not what the node does (see above). Null for a node of another kind.
 */
export function regionOp(at: FusionAt, node: FusionNode, region: FusionTrackerPlace[]): Op | null {
  const params = planarOf(node);
  if (params === null) return null;
  const more = params.operation === 'corner_pin' ? {} : { corners: boxCorners(region) };
  return { op: 'set_fusion_node', ...at, id: node.id, params: { region, ...more } };
}

/** The region and the corners of a Planar Tracker put back as they were: what a drag of the region or of a corner that is taken back sends. */
export function restoredPlanar(at: FusionAt, node: FusionNode): Op | null {
  const params = planarOf(node);
  return params === null ? null : { op: 'set_fusion_node', ...at, id: node.id, params: { region: params.region, corners: params.corners } };
}

/** The corners of the Corner Pin of a Planar Tracker set whole, as each move of a drag of one of them sends them. Null for a node of another kind. */
export function cornersOp(at: FusionAt, node: FusionNode, corners: FusionPlanarCorners): Op | null {
  return planarOf(node) === null ? null : { op: 'set_fusion_node', ...at, id: node.id, params: { corners } };
}

/** A node that is a Planar Tracker with another region; any other node as it is. */
export function withRegion(node: FusionNode, region: FusionTrackerPlace[]): FusionNode {
  return node.kind === 'planar_tracker' ? { ...node, params: { ...node.params, region } } : node;
}

/**
 * A place put into the region of a Planar Tracker after its place `after` (counted from 0): the region set whole, every place
 * that was there the object that it is. Nothing where the region has as many places as it may have, or no such place, and for a
 * node of another kind.
 */
export function insertRegionPoint(at: FusionAt, node: FusionNode, after: number, place: FusionTrackerPlace): Op[] {
  const params = planarOf(node);
  if (params === null || params.region.length >= MOST_REGION_POINTS || !Number.isInteger(after) || after < 0 || after >= params.region.length) return [];
  const op = regionOp(at, node, [...params.region.slice(0, after + 1), place, ...params.region.slice(after + 1)]);
  return op === null ? [] : [op];
}

/** Take the place `index` (counted from 0) of the region of a Planar Tracker away. Nothing where it would leave fewer than three, or there is no such place. */
export function removeRegionPoint(at: FusionAt, node: FusionNode, index: number): Op[] {
  const params = planarOf(node);
  if (params === null || params.region.length <= LEAST_REGION_POINTS || !Number.isInteger(index) || index < 0 || index >= params.region.length) return [];
  const op = regionOp(at, node, params.region.filter((_, place) => place !== index));
  return op === null ? [] : [op];
}

/**
 * Clear Track: every frame of the path of a Planar Tracker taken away, each given `null` in one patch of the path, so that the
 * region is where it was drawn at every frame again. Nothing where it has no path.
 */
export function clearTrack(at: FusionAt, node: FusionNode): Op[] {
  const params = planarOf(node);
  const frames = params === null ? [] : Object.keys(params.path ?? {});
  if (frames.length === 0) return [];
  return [{ op: 'set_fusion_node', ...at, id: node.id, params: { path: Object.fromEntries(frames.map(frame => [frame, null])) } }];
}

/**
 * The sentences for the refusals of the settings of a Planar Tracker, each named as `refusalOf` takes a sentence for a refusal
 * of one thing: by the reason, a colon, and what the engine says the refusal is of.
 */
export const planarRefusals = (said: { planarRegionRange: string; planarCornersRange: string; planarReferenceRange: string }): Record<string, string> => ({
  'out_of_range:planar_tracker.region': said.planarRegionRange,
  'out_of_range:planar_tracker.corners': said.planarCornersRange,
  'out_of_range:planar_tracker.reference': said.planarReferenceRange,
});
