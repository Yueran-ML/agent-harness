import type { Key } from '../../app/keys';
import type { TrackComponent, TrackComponents, TrackData, TrackDirection, WindowTracking } from '../engine';

/**
 * The Tracker palette's window mode (Reference Manual 20, chapter 140, pp. 3229–3255; PROTOCOL.md,
 * "Window tracking"): what it draws and what its buttons and keys ask of the engine.
 */

/** The curves of the tracker graph, in the order of its boxes, each colour-coded as its label (p. 3236). */
export const GRAPH_CURVES: readonly { component: TrackComponent; box: keyof TrackComponents; color: string }[] = [
  { component: 'pan', box: 'pan', color: '#e0605a' },
  { component: 'tilt', box: 'tilt', color: '#6cc46c' },
  { component: 'zoom', box: 'zoom', color: '#5a9ae0' },
  { component: 'rotate', box: 'rotate', color: '#e0c050' },
  { component: 'aspect', box: 'perspective', color: '#c070d0' },
  { component: 'shear', box: 'perspective', color: '#c070d0' },
  { component: 'yaw', box: 'perspective', color: '#a070e0' },
  { component: 'pitch', box: 'perspective', color: '#a070e0' },
];

export const IDENTITY: Record<TrackComponent, number> = { pan: 0, tilt: 0, zoom: 1, rotate: 0, aspect: 1, shear: 0, yaw: 0, pitch: 0 };

/** Pan, Tilt, Zoom and Rotate on, Perspective 3D off: the core's default (the manual does not say). */
export const DEFAULT_COMPONENTS: TrackComponents = { pan: true, tilt: true, zoom: true, rotate: true, perspective: false };

/** The samples of a curve, by frame, in order. */
export function samples(data: TrackData | undefined, component: TrackComponent): [number, number][] {
  const curve = data?.curves[component];
  if (!curve) return [];
  return Object.entries(curve).map(([frame, value]) => [Number(frame), value] as [number, number])
    .filter(([frame, value]) => Number.isFinite(frame) && Number.isFinite(value)).sort((a, b) => a[0] - b[0]);
}

/** Whether a window has been tracked: a curve has a sample (the badge of p. 3231). */
export function tracked(tracking: WindowTracking | undefined): boolean {
  return GRAPH_CURVES.some(({ component }) => samples(tracking?.data, component).length > 0);
}

/** The first and last frames of curves' samples, each in order; null where none has any. */
function rangeOf(curves: readonly (readonly [number, number][])[]): [number, number] | null {
  let first = Infinity;
  let last = -Infinity;
  for (const points of curves) {
    if (points.length === 0) continue;
    first = Math.min(first, points[0][0]);
    last = Math.max(last, points[points.length - 1][0]);
  }
  return first === Infinity ? null : [first, last];
}

/** The first and last tracked frames. Loops, not spreads: a long clip has more samples than a call takes arguments. */
export function trackedRange(data: TrackData | undefined): [number, number] | null {
  return rangeOf(GRAPH_CURVES.map(({ component }) => samples(data, component)));
}

export interface GraphLine { component: TrackComponent; color: string; points: [number, number][] }
export interface Graph { from: number; to: number; lines: GraphLine[] }

/** The columns the graph is drawn in: its width in the palette's own units. */
export const GRAPH_COLUMNS = 400;

/**
 * A line of more points than the graph has columns, drawn by the least and the most of each column, in the order of
 * their frames, and its first and last points: what the column of pixels shows, at a cost that does not grow with the
 * length of the clip. A shorter line keeps every point.
 */
function thinned(points: [number, number][], from: number, to: number): [number, number][] {
  if (points.length <= 2 * GRAPH_COLUMNS) return points;
  const width = (to - from) / GRAPH_COLUMNS;
  const out: [number, number][] = [points[0]];
  let column = -1;
  let low = points[0];
  let high = points[0];
  const flush = () => {
    if (column < 0) return;
    const [a, b] = low[0] <= high[0] ? [low, high] : [high, low];
    if (out[out.length - 1] !== a) out.push(a);
    if (b !== a) out.push(b);
  };
  for (const point of points) {
    const at = Math.min(GRAPH_COLUMNS - 1, Math.max(0, Math.floor((point[0] - from) / width)));
    if (at !== column) { flush(); column = at; low = point; high = point; continue; }
    if (point[1] < low[1]) low = point;
    if (point[1] > high[1]) high = point;
  }
  flush();
  const end = points[points.length - 1];
  if (out[out.length - 1] !== end) out.push(end);
  return out;
}

/**
 * The tracker graph: each curve drawn about its value of no motion and scaled by the largest
 * change of its own, so that a pan of pixels and a turn of radians are both seen (the vertical
 * slider of p. 3237 scales them all by `scale`). `y` is from -1 (top) to 1 (bottom) before `scale`.
 * A long curve is thinned to the columns of the graph.
 */
export function graph(data: TrackData | undefined, frame: number, scale = 1): Graph {
  const curves = GRAPH_CURVES.map(each => ({ ...each, points: samples(data, each.component) }));
  const range = rangeOf(curves.map(each => each.points));
  const from = Math.min(range?.[0] ?? frame, frame);
  const to = Math.max(range?.[1] ?? frame, frame, from + 1);
  const lines: GraphLine[] = [];
  for (const { component, color, points } of curves) {
    if (points.length === 0) continue;
    const identity = IDENTITY[component];
    let most = 0;
    for (const [, value] of points) most = Math.max(most, Math.abs(value - identity));
    if (!(most > 1e-12)) continue;
    const drawn = points.map(([f, value]) => [f, -((value - identity) / most) * scale] as [number, number]);
    lines.push({ component, color, points: thinned(drawn, from, to) });
  }
  return { from, to, lines };
}

/** A box drawn over the graph, in frames and in its `y`. */
export interface Selection { from: number; to: number; top: number; bottom: number }

/** The curves that pass through a box drawn over the graph, and the frames it spans (p. 3238). */
export function selected(lines: readonly GraphLine[], box: Selection): { from: number; to: number; components: TrackComponent[] } | null {
  const [from, to] = [Math.ceil(Math.min(box.from, box.to)), Math.floor(Math.max(box.from, box.to))];
  const [top, bottom] = [Math.min(box.top, box.bottom), Math.max(box.top, box.bottom)];
  if (to < from) return null;
  const components = lines.filter(line => line.points.some(([f, y]) => f >= from && f <= to && y >= top && y <= bottom))
    .map(line => line.component);
  return components.length === 0 ? null : { from, to, components };
}

/**
 * The keys of the Color › Tracker menu (p. 3231): Track Forward Ctrl+T, Track Reverse Alt+T,
 * Track Stop Ctrl+Alt+T, One Frame Forward Alt+→ and One Frame Reverse Alt+← (Command and
 * Option on a Mac are Ctrl and Alt here).
 */
export function trackerKey(key: Key): TrackDirection | 'stop' | null {
  if (key.shift || key.meta || key.repeat) return null;
  const t = key.code === 'KeyT' || key.key.toLowerCase() === 't';
  if (t && key.ctrl && key.alt) return 'stop';
  if (t && key.ctrl) return 'forward';
  if (t && key.alt) return 'reverse';
  if (key.alt && !key.ctrl && key.key === 'ArrowRight') return 'one_forward';
  if (key.alt && !key.ctrl && key.key === 'ArrowLeft') return 'one_reverse';
  return null;
}

/** Add Tracker: a crosshair at the centre of the frame (p. 3238). */
export function withCrosshair(crosshairs: readonly [number, number][]): [number, number][] {
  return [...crosshairs, [0.5, 0.5]];
}

/** Delete Tracker: the selected crosshair taken away. */
export function withoutCrosshair(crosshairs: readonly [number, number][], index: number): [number, number][] {
  return crosshairs.filter((_, i) => i !== index);
}

/** The keyframe of Frame mode at `frame`, if there is one. */
export function keyAt(data: TrackData | undefined, frame: number): boolean {
  return data !== undefined && Object.prototype.hasOwnProperty.call(data.keys, String(frame));
}
