/**
 * The motion path of a clip in the timeline viewer of the Edit page (DaVinci Resolve Reference Manual
 * 20, pp. 1106–1108): the line through the places of the anchor of a clip whose position has keys, a
 * small dot at each frame so that their spacing shows the speed, and a larger point at each key, which
 * is dragged on the picture to move that key.
 *
 * The anchor is where the engine puts the middle of the picture of the clip
 * (`geometry::placement` of engine/crates/compose): the middle of the timeline moved by `transform.x`
 * of its width and `transform.y` of its height. The zoom and the turn of the clip are about that
 * middle and do not move it. The places are in pixels of the timeline from its top left corner, as the
 * engine tells the pointer on the viewer (`view.pointer`) and draws an overlay (`view.overlay`).
 *
 * The position at a frame of the timeline is the one that the engine draws there: the two settings at
 * the frame that the keys of the clip count (`keyClockOf`: the picture of its media that it shows,
 * through its speed, its direction and its speed points, or its own frames for a title, a colour or a
 * still). A clip that shows a picture twice is at the same place twice, and its path goes back over
 * itself.
 *
 * This engine eases each of the two settings in time, by the Bezier of its own stretch; it has no
 * Bezier in the plane. A point made Smooth is come to and left slowly on both axes (Ease In and Out),
 * and the path between two points bends only where the two settings go differently.
 *
 * Pure: the hook `useMotionPath` sends what these work out.
 */
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import type { Clip, Keyframe, Media, OverlayPoint, Param, Timeline, ViewOverlay } from '../engine';
import { CLIP_BOUNDS, keyTimes, mediaOf, paramOf } from '../inspector/clipKeys';
import { insertKey, interpIs, moveKeys, setInterp } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';

/** The two settings of the position of a clip: how far its picture is moved, as shares of the width and of the height of the timeline. */
export const POSITION_X = 'transform.x';
export const POSITION_Y = 'transform.y';
export type PositionPath = typeof POSITION_X | typeof POSITION_Y;
export const POSITION: readonly PositionPath[] = [POSITION_X, POSITION_Y];

/**
 * The most frames of a clip that the path has a dot at. A longer clip has one every few frames (and
 * at the frames of its keys), evenly, so that their spacing still shows the speed.
 */
export const MOST_TICKS = 4096;
/** The most points of keys that a path has: the engine draws no more than 32768 points in all. */
export const MOST_KEYS = 4096;
/** How near a point the pointer must be to press it, in pixels of the screen. */
export const HIT_PX = 8;

/** A point of the path: a frame of the keys at which `transform.x` or `transform.y` has a key, where the anchor is then, and which of the two has a key there. */
export interface PathKey { frame: number; at: OverlayPoint; x: boolean; y: boolean }
/** The motion path of a clip, in pixels of its timeline. */
export interface MotionPath {
  canvas: [width: number, height: number];
  /** The places of the anchor at the frames of the clip, in their order: the points of the line, and its dots. A place held over several frames is there once. */
  ticks: OverlayPoint[];
  /** Its points of keys, in the order of their frames. */
  keys: PathKey[];
}

type Sized = Pick<Timeline, 'width' | 'height'>;

/** Whether the position of a clip has keys: only then has it a motion path. */
export function hasMotion(clip: Clip): boolean {
  // A clip that a client built may say nothing of its transform: it is not moved, and has no path.
  const transform: Clip['transform'] | undefined = clip.transform;
  return transform !== undefined && (typeof transform.x === 'object' || typeof transform.y === 'object');
}

/** Where the anchor of a clip moved by `x` and `y` is, in pixels of the timeline. */
export function placeOf(timeline: Sized, x: number, y: number): OverlayPoint {
  return [timeline.width / 2 + x * timeline.width, timeline.height / 2 + y * timeline.height];
}

/** Where the anchor of a clip is at a frame of its keys. */
export function anchorAt(clip: Clip, timeline: Sized, keyFrame: number): OverlayPoint {
  return placeOf(timeline, valueAt(paramOf(clip, POSITION_X), keyFrame), valueAt(paramOf(clip, POSITION_Y), keyFrame));
}

const framesOf = (param: Param): number[] => (typeof param === 'number' ? [] : param.keys.map(key => key.frame));

/**
 * The motion path of a clip on a timeline; null for a clip whose position has no keys, or that lasts
 * no frame.
 *
 * The points of keys are those of the frames of keys that the clip shows (`shownKeyRange`): a key
 * trimmed out of the clip still shapes the path, and has no point on it.
 */
export function motionPathOf(clip: Clip, timeline: Pick<Timeline, 'width' | 'height' | 'rate'>, media: readonly Media[]): MotionPath | null {
  if (!hasMotion(clip) || clip.length <= 0) return null;
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const last = clip.start + clip.length - 1;
  const step = Math.max(1, Math.ceil(clip.length / MOST_TICKS));
  const frames = new Set<number>();
  for (let frame = clip.start; frame <= last; frame += step) frames.add(frame);
  frames.add(last);
  // Where frames are passed over, the line still goes through the points of the keys.
  if (step > 1) for (const path of POSITION) for (const frame of keyTimes(clip, timeline, media, path)) frames.add(frame);
  const ticks: OverlayPoint[] = [];
  for (const frame of [...frames].sort((a, b) => a - b)) {
    const at = anchorAt(clip, timeline, clock(frame));
    const before = ticks[ticks.length - 1];
    if (!before || before[0] !== at[0] || before[1] !== at[1]) ticks.push(at);
  }
  const x = new Set(framesOf(paramOf(clip, POSITION_X)));
  const y = new Set(framesOf(paramOf(clip, POSITION_Y)));
  const [low, high] = shownKeyRange(clip, clock);
  const keys = [...new Set([...x, ...y])].filter(frame => frame >= low && frame <= high).sort((a, b) => a - b).slice(0, MOST_KEYS)
    .map((frame): PathKey => ({ frame, at: anchorAt(clip, timeline, frame), x: x.has(frame), y: y.has(frame) }));
  return { canvas: [timeline.width, timeline.height], ticks, keys };
}

/** Where the anchor of a clip is at a frame of the timeline; null where the clip is not at that frame. */
export function anchorAtPlayhead(clip: Clip, timeline: Pick<Timeline, 'width' | 'height' | 'rate'>, media: readonly Media[], frame: number): OverlayPoint | null {
  const shown = Math.floor(frame);
  if (shown < clip.start || shown >= clip.start + clip.length) return null;
  return anchorAt(clip, timeline, keyClockOf(clip, timeline.rate, mediaOf(clip, media))(shown));
}

/**
 * What the engine is to draw for a path (`view.overlay`): the line, its dots, the points of the keys
 * over them, the one that is dragged (`active`, its place among the keys) in its own colour, and the
 * place at the playhead (`current`) on top. A kind that has no dot is left out.
 */
export function overlayOf(path: MotionPath, current: OverlayPoint | null = null, active: number | null = null): ViewOverlay {
  const dragged = active !== null && active >= 0 && active < path.keys.length ? path.keys[active] : null;
  const dots: ViewOverlay['dots'] = [
    { kind: 'tick', at: path.ticks },
    { kind: 'key', at: path.keys.filter(key => key !== dragged).map(key => key.at) },
    { kind: 'active', at: dragged ? [dragged.at] : [] },
    { kind: 'current', at: current ? [current] : [] },
  ];
  return { canvas: path.canvas, lines: path.ticks.length > 1 ? [path.ticks] : [], dots: dots.filter(each => each.at.length > 0) };
}

/**
 * The point of keys of a path that is pressed at a place of the picture: the nearest within `reach`
 * pixels of the timeline, the later of two as near (it is drawn over the other); -1 where none is.
 */
export function hitKey(path: MotionPath, at: OverlayPoint, reach: number): number {
  let found = -1;
  let nearest = Infinity;
  path.keys.forEach((key, index) => {
    const distance = Math.hypot(key.at[0] - at[0], key.at[1] - at[1]);
    if (distance <= reach && distance <= nearest) { found = index; nearest = distance; }
  });
  return found;
}

/**
 * A setting of the position with its key at a frame moved by `delta`, kept within what the setting
 * takes, the eases of the stretches beside it kept (`moveKeys`). Not moved, it is as it was.
 *
 * Where it has keys and none at the frame, a key is put there first, of the value that it has there,
 * its curve kept (`insertKey`), as a value given in the Inspector at a frame without a key puts one.
 * Where it has no keys at all it is one value on every frame, and one key would be so too: it gets a
 * key at each frame of `paired`, the frames of the keys of the other setting, so that the point that
 * is dragged moves and the others stay.
 */
function axisMoved(param: Param, path: PositionPath, frame: number, delta: number, paired: readonly number[]): Param {
  if (delta === 0) return param;
  const had: Keyframe[] = typeof param === 'number' ? paired.map((at): Keyframe => ({ frame: at, value: param, interp: 'linear' })) : param.keys;
  const index = had.findIndex(key => key.frame === frame);
  const made = index < 0 ? insertKey({ keys: had }, frame) : { keys: had, index };
  if (!made) return param;
  return { keys: moveKeys(made.keys, [made.index], 0, delta, CLIP_BOUNDS[path]) };
}

/**
 * The two settings of the position of a clip with the point of keys at `frame` moved by `dx` of the
 * width of the timeline and `dy` of its height, worked out from the clip as it is given (as it was
 * when the drag began): what each move of a drag sets whole.
 */
export function movedPoint(clip: Clip, frame: number, dx: number, dy: number): Record<PositionPath, Param> {
  const x = paramOf(clip, POSITION_X);
  const y = paramOf(clip, POSITION_Y);
  return {
    'transform.x': axisMoved(x, POSITION_X, frame, dx, framesOf(y)),
    'transform.y': axisMoved(y, POSITION_Y, frame, dy, framesOf(x)),
  };
}

/** How a point of the path is come to and left: on straight lines, or slowly (Ease In and Out on both settings). */
export type PointInterp = 'linear' | 'smooth';

/**
 * A setting with its key at a frame made Linear or Smooth, by the Curve Editor's `setInterp`: Smooth
 * is its Ease In and Out, the two handles at the key flat a third of the way to the keys beside it;
 * Linear lays them on the lines to those keys. A key that held is let go first. Null where the
 * setting has no key at the frame.
 */
export function interpolated(param: Param, frame: number, kind: PointInterp): Param | null {
  if (typeof param === 'number') return null;
  const index = param.keys.findIndex(key => key.frame === frame);
  return index < 0 ? null : { keys: setInterp(param.keys, [index], kind === 'smooth' ? 'in_out' : 'linear') };
}

/**
 * How the point of keys of a clip at a frame goes: Linear or Smooth where each of the two settings
 * that has a key there goes so, null where they differ or go another way (one eased on one side, a
 * handle pulled in the Curve Editor, a key that holds). A point between keys of one value, where the
 * two are the same curve, is Linear.
 */
export function pointInterp(clip: Clip, frame: number): PointInterp | null {
  const placed = POSITION.flatMap((path) => {
    const param = paramOf(clip, path);
    const index = typeof param === 'number' ? -1 : param.keys.findIndex(key => key.frame === frame);
    return typeof param === 'number' || index < 0 ? [] : [{ keys: param.keys, index }];
  });
  if (placed.length === 0) return null;
  if (placed.every(({ keys, index }) => interpIs(keys, [index], 'linear'))) return 'linear';
  return placed.every(({ keys, index }) => interpIs(keys, [index], 'in_out')) ? 'smooth' : null;
}
