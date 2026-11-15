/**
 * The geometry of the Keyframe Editor of the Color page (DaVinci Resolve Reference Manual 20,
 * Ch. 148, pp. 3381–3386): the frames of keys that a grade is shown over and the frame of the
 * timeline at which each is shown, where a frame and a track are on the screen, the marks of the
 * tracks, which mark a pointer is on, and the curve that the dialog of Change Dynamic Attributes
 * draws. Pure.
 *
 * The ruler counts the frames of the keys (`keyClockOf`: the pictures of the clip's media, or the
 * clip's own frames from its first; the frames of the timeline for its grade), as the Curve Editor
 * of the Edit page does, so a mark is at the frame that its keys are at. A clip that plays backward
 * has its playhead go from right to left.
 */
import { ticksOf } from '../keyframes/curveGeometry';
import { firstShowing, keyRangeOf, type KeyClock, type KeyRun } from './keyFrames';
import { easeShare, type KeyKind, type KeyTrack } from './nodeKeys';

/**
 * Where a grade is shown: the frames of the timeline from `first` to `last`, both shown, and the
 * clock of its keys there. A clip's grade and its group's are shown over the clip, with its clock;
 * the timeline's over the whole timeline, whose frames its keys count. The clock goes one way along
 * the span, unless it is that of a clip with speed points: then `runs` are the stretches of the span
 * along each of which it goes one way (`keyRunsOf`).
 */
export interface KeySpan { first: number; last: number; clock: KeyClock; runs?: readonly KeyRun[] }

/** The clock of the keys of a timeline's grade: its own frames. */
export const TIMELINE_CLOCK: KeyClock = frame => frame;

/** The frame of the keys at a frame of the timeline, which is kept within the span; a playhead between two frames is in the earlier. */
export function keyFrameAt(span: KeySpan, frame: number): number {
  return span.clock(Math.min(span.last, Math.max(span.first, Math.floor(frame))));
}

/** The frames of keys that a span shows, least and greatest. */
export function keyRange(span: KeySpan): [number, number] {
  if (span.runs) return keyRangeOf(span.clock, span.runs);
  const ends = [span.clock(span.first), span.clock(span.last)];
  return [Math.min(...ends), Math.max(...ends)];
}

/**
 * The frame of the timeline at which a span shows a frame of keys: the first that has come to it,
 * through speed and direction (as `frameOfKey` finds it for a clip). The clock goes one way along
 * the span, so it is found by halving. A frame before those that are shown is at the first frame
 * (the last, where the clip plays backward), and one after them at the last (the first). Along the
 * speed points of a clip it is the first frame of the first run that comes by it (`firstShowing`).
 */
export function timelineFrameOf(span: KeySpan, keyFrame: number): number {
  if (span.last <= span.first) return span.first;
  if (span.runs) {
    // A clip with speed points: the first frame that shows the frame of keys, one out of what is shown held to the nearest that is.
    const [low, high] = keyRangeOf(span.clock, span.runs);
    return firstShowing(span.clock, span.runs, Math.min(Math.max(keyFrame, low), high)) ?? span.first;
  }
  const rising = span.clock(span.last) >= span.clock(span.first);
  let from = span.first;
  let to = span.last;
  while (from < to) {
    const middle = Math.floor((from + to) / 2);
    const reached = rising ? span.clock(middle) >= keyFrame : span.clock(middle) <= keyFrame;
    if (reached) to = middle;
    else from = middle + 1;
  }
  return from;
}

/** The frames of the timeline at which a span shows frames of keys, in the order of the timeline, each once; those that it does not show are left out. */
export function timesOf(span: KeySpan, frames: readonly number[]): number[] {
  const [low, high] = keyRange(span);
  const times = frames.filter(frame => frame >= low && frame <= high).map(frame => timelineFrameOf(span, frame));
  return [...new Set(times)].sort((a, b) => a - b);
}

/** The time before or after `from` among times in their order, or null where there is none. */
export function stepTime(times: readonly number[], from: number, direction: -1 | 1): number | null {
  if (direction > 0) return times.find(each => each > from) ?? null;
  for (let at = times.length - 1; at >= 0; at -= 1) if (times[at] < from) return times[at];
  return null;
}

// --- The lanes.

/** The measures of the lanes, in pixels: the height of the ruler and of a track, and what is kept free at the left and the right of the frames. */
export const LANE = { ruler: 20, row: 22, pad: 10 } as const;
/** The width of the lanes until they are measured, and where they are not laid out. */
export const LANES_WIDTH = 480;

type Range = readonly [number, number];
export interface Point { x: number; y: number }

const reach = (width: number) => Math.max(1, width - 2 * LANE.pad);
const extent = (range: Range) => Math.max(1, range[1] - range[0]);

/** Where a frame of keys is, from the left of lanes of a width that show a range of frames. */
export const xOfFrame = (range: Range, width: number, frame: number): number => LANE.pad + ((frame - range[0]) / extent(range)) * reach(width);

/** The frame of keys at a place from the left: a whole one, kept within the range. */
export function frameAtX(range: Range, width: number, x: number): number {
  const frame = Math.round(range[0] + ((x - LANE.pad) / reach(width)) * extent(range)) + 0;
  return Math.min(range[1], Math.max(range[0], frame));
}

/** Where the track at a row begins, from the top: the master track is row 0, beneath the ruler. */
export const rowTop = (row: number): number => LANE.ruler + row * LANE.row;
/** The middle of the track at a row, where its marks are. */
export const rowMiddle = (row: number): number => rowTop(row) + LANE.row / 2;
/** The height of lanes of so many tracks. */
export const lanesHeight = (rows: number): number => rowTop(Math.max(rows, 1));

/** The row of the track at a place from the top, or null: in the ruler, or beneath the last track. */
export function rowAt(y: number, rows: number): number | null {
  if (y < LANE.ruler) return null;
  const row = Math.floor((y - LANE.ruler) / LANE.row);
  return row < rows ? row : null;
}

/** A keyframe as a track shows it: a diamond where it is dynamic, a circle where it is static (pp. 3384–3385). */
export interface Mark { row: number; node: string | null; frame: number; kind: KeyKind; mixed: boolean; x: number; y: number }

/** The marks of tracks in lanes of a width that show a range of frames: those of frames outside the range are left out. */
export function marksOf(tracks: readonly KeyTrack[], range: Range, width: number): Mark[] {
  return tracks.flatMap((track, row) => track.keyframes
    .filter(each => each.frame >= range[0] && each.frame <= range[1])
    .map((each): Mark => ({ row, node: track.node, frame: each.frame, kind: each.kind, mixed: each.mixed, x: xOfFrame(range, width, each.frame), y: rowMiddle(row) })));
}

/** How many pixels from a mark a pointer takes it. */
export const MARK_REACH = 7;
/** A dynamic keyframe, about where it is. */
export const DIAMOND = 'M0,-5 L5,0 L0,5 L-5,0 Z';
/** The radius of a static keyframe. */
export const MARK_RADIUS = 4;

/** The mark nearest a point within `within` pixels of it, on the track that the point is in; or null. */
export function hitMark(marks: readonly Mark[], at: Point, within = MARK_REACH): Mark | null {
  let hit: Mark | null = null;
  let best = within;
  for (const mark of marks) {
    if (Math.abs(mark.y - at.y) > LANE.row / 2) continue;
    const distance = Math.abs(mark.x - at.x);
    if (distance <= best) { best = distance; hit = mark; }
  }
  return hit;
}

/** The frames that the ruler writes, for lanes of a width: whole ones, about one for each 70 pixels. */
export function rulerTicks(range: Range, width: number): number[] {
  if (range[1] <= range[0]) return [range[0]];
  return ticksOf(range[0], range[1], Math.max(1, Math.floor(reach(width) / 70)), true);
}

/**
 * The curve of a dynamic keyframe with two eases, as the `d` of an SVG path in a box: from the
 * keyframe at the lower left to the next at the upper right, its two handles level and as long as
 * `easeShare` says, which are the handles that the keys get. With no ease it is the line.
 */
export function easePath(start: number, end: number, box: { width: number; height: number }): string {
  const n = (value: number) => String(Number(value.toFixed(2)) + 0);
  const { width, height } = box;
  return `M0,${n(height)} C${n(easeShare(start) * width)},${n(height)} ${n((1 - easeShare(end)) * width)},0 ${n(width)},0`;
}
