/**
 * The on-screen controls of a node of a composition, in the viewer of the Fusion page (DaVinci
 * Resolve Reference Manual 20, Ch. 70, "Using Viewers", and Ch. 108 for the masks): the outline of a
 * mask and the handles that are dragged on the picture to set where it lies and how large it is.
 *
 * The viewer is a window of the engine's, over which the page can draw nothing. So the page works
 * the controls out here, in the pixels of the frame that the timeline monitor draws, and has the
 * engine draw them (`view.overlay`) and tell what the pointer does on the picture (`view.pointer`),
 * in the same pixels. Where the picture of the composition lies in that frame is told by the Fusion
 * view (`viewer.fusion`, PROTOCOL.md, "The Fusion view"): `picture`, its top left corner and its
 * width and height. A place of a composition, a share `(u, v)` of the width and of the height of its
 * picture from its lower left corner, `v` up, is at `x + u · width` and `y + (1 − v) · height`.
 *
 * The shapes are those of the engine (`engine/crates/compose/src/mask.rs`, "Places"): a length in
 * widths of the picture is so many times `width`, one in heights so many times `height`, and an
 * Angle turns a shape counter-clockwise as it is seen.
 *
 * - A Rectangle: `width` in widths and `height` in heights, about its `center`, turned by `angle`.
 *   Its outline is drawn with straight corners, whatever its Corner Radius.
 * - An Ellipse: `width` and `height` both in widths, so that alike they are a circle. Its outline is
 *   a line through `ELLIPSE_STEPS` places of it.
 * - A Polygon: each point from its `center` in widths both ways, `x` to the right and `y` up,
 *   multiplied by `size` and turned by `angle` about the centre.
 * - A Merge and a Transform: `center` alone, where the foreground of the one and the picture of the
 *   other lie.
 * - A Tracker (PROTOCOL.md, "The Tracker"): of each pattern that is on, the box of the pattern and
 *   the box that it is searched for in, about where the pattern is at the playhead (on its path,
 *   where it has one, else where it was put), its path as a line through its frames, and a handle
 *   at its middle. A pattern that is off is not drawn: the view draws every line alike.
 *
 * Each handle is a setting, or two: the centre is `center.x` and `center.y`; the handle in the
 * middle of the right side of a Rectangle or an Ellipse is its `width`, and the one in the middle of
 * its upper side its `height` (right and upper before it is turned); a point of a Polygon is its `x`
 * and its `y`; the handle of a pattern of a Tracker is where it was put, `pattern.x` and `pattern.y`.
 * A drag moves a handle by as much as the pointer moves, along the axes of the shape as it is
 * turned, so that a press a little off a handle does not make it jump.
 *
 * A centre that follows a Tracker (PROTOCOL.md, "Following a Tracker") is drawn where it follows the
 * Tracker to, and its handle is not dragged: its own `x` and `y` are kept and not read.
 *
 * Pure: the hook `useFusionOnscreen` sends what these work out.
 */
import type { Bounds } from '../color/corrector';
import type { FusionKind, FusionNode, FusionPicture, OverlayPoint, ViewOverlay, ViewPointer } from '../engine';
import { valueAt } from '../keyframes/value';
import { HIT_PX } from '../motionPath/motionPath';
import { paramOf, pointPath, settingOf, type FusionSetting } from './settings';
import { MOST_PATH_POINTS, pathSamples, placeOnPath, thinned, TRACKER_PLACE } from './tracker';

/** The kinds of node that have controls on the picture. */
export type ControlKind = Extract<FusionKind, 'rectangle' | 'ellipse' | 'polygon' | 'merge' | 'transform' | 'tracker'>;
const CONTROL_KINDS: readonly FusionKind[] = ['rectangle', 'ellipse', 'polygon', 'merge', 'transform', 'tracker'];
/** Whether a kind of node has controls on the picture. */
export const hasControls = (kind: FusionKind): kind is ControlKind => CONTROL_KINDS.includes(kind);

/** How many straight strokes the outline of an Ellipse is drawn with: a multiple of four, so that the ends of its two axes are places of it. */
export const ELLIPSE_STEPS = 64;

/** What a handle sets: the centre, the width or the height of a Rectangle or an Ellipse, a point of a Polygon, or the place of a pattern of a Tracker. */
export type HandleKind = 'center' | 'width' | 'height' | 'point' | 'pattern';
/**
 * A handle: what it sets, of a point of a Polygon or of a pattern of a Tracker which one (from 0), and where it is, in pixels of
 * the frame drawn. One that is `fixed` is drawn and not taken hold of: the centre of a node that follows a Tracker.
 */
export interface Handle { kind: HandleKind; index?: number; at: OverlayPoint; fixed?: true }
/**
 * The controls of a node: the lines of its outline, each through its places in order, and its handles. Of two handles at one place,
 * the later is the one pressed. `marks`, of a Tracker, are where its patterns are on their paths at the playhead, drawn over the
 * handles in the colour of the playhead.
 */
export interface Controls { lines: OverlayPoint[][]; handles: Handle[]; marks?: OverlayPoint[] }
/** A setting and the value that a drag gives it. */
export interface Moved { setting: FusionSetting; value: number }
/**
 * What else the controls of a node are drawn with: where a centre that follows a Tracker is (`follows`, the place that the Tracker
 * publishes at the frame, or null where the page does not work it out, and the centre is drawn at its own place then), and the
 * pattern of a Tracker that a drag is taking (`placing`), drawn where the drag has put it.
 */
export interface ControlOptions {
  follows?: { place: [x: number, y: number] | null } | null;
  placing?: { index: number; place: [x: number, y: number] } | null;
}

/** The name of a handle among those of its node: what is dragged is known by it though the node is drawn anew. */
export const handleName = (handle: Pick<Handle, 'kind' | 'index'>): string => (handle.kind === 'point' || handle.kind === 'pattern' ? `${handle.kind}:${handle.index ?? 0}` : handle.kind);

/** A number of pixels as it is sent: to a thousandth of a pixel, and never the zero that is written with a minus. */
const pixel = (value: number): number => Math.round(value * 1000) / 1000 + 0;
/** A value of a setting as a drag sends it: to a millionth, as a number typed is. */
const tidy = (value: number): number => Number(value.toFixed(6)) + 0;
const within = (bounds: Bounds, value: number): number => Math.min(bounds.high, Math.max(bounds.low, value));

/** Where a place of a composition is in the frame drawn: `(u, v)`, shares of the width and of the height of its picture from its lower left corner, `v` up. */
export function toPixels(picture: FusionPicture, u: number, v: number): OverlayPoint {
  return [picture.x + u * picture.width, picture.y + (1 - v) * picture.height];
}

/** The place of a composition that a point of the frame drawn is at: the inverse of `toPixels`. */
export function fromPixels(picture: FusionPicture, x: number, y: number): [u: number, v: number] {
  return [(x - picture.x) / picture.width, 1 - (y - picture.y) / picture.height];
}

/**
 * The cosine and the sine of an Angle in degrees. A whole number of quarter turns has them exact, so
 * that a shape turned by 90° lies on whole pixels where it would unturned.
 */
export function turnOf(degrees: number): [cosine: number, sine: number] {
  const quarters = degrees / 90;
  if (Number.isInteger(quarters)) {
    const exact: [number, number][] = [[1, 0], [0, 1], [-1, 0], [0, -1]];
    return exact[((quarters % 4) + 4) % 4];
  }
  const radians = (degrees * Math.PI) / 180;
  return [Math.cos(radians), Math.sin(radians)];
}

/**
 * A setting of a node at a frame of its keys, kept within its range as the engine keeps it when it
 * draws; 0 where the node has no such setting.
 */
function read(node: FusionNode, path: string, keyFrame: number): number {
  const setting = settingOf(node, path);
  if (!setting) return 0;
  const value = valueAt(paramOf(node, setting), keyFrame);
  return Number.isFinite(value) ? within(setting.bounds, value) : setting.bounds.neutral;
}

/** A place for a setting of a node held within the range of that setting: where a centre that follows a Tracker is drawn. */
function held(node: FusionNode, path: string, value: number): number {
  const setting = settingOf(node, path);
  return setting && Number.isFinite(value) ? within(setting.bounds, value) : value;
}

/** How many points a Polygon has; none for another node. */
const pointsOf = (node: FusionNode): number => (node.kind === 'polygon' && Array.isArray(node.params.points) ? node.params.points.length : 0);

/** The length, in pixels of the frame drawn, that the `height` of a Rectangle or an Ellipse counts: the height of the picture for the one, its width for the other. */
const heightUnit = (node: FusionNode, picture: FusionPicture): number => (node.kind === 'rectangle' ? picture.height : picture.width);

/**
 * The controls of a node as it is at a frame of its keys, on a picture that lies at `picture` in the
 * frame drawn; null for a node of a kind that has none.
 *
 * - A Rectangle: its outline, from its lower left corner round counter-clockwise and back to it
 *   (lower and left before it is turned), and three handles: the centre, the middle of its right
 *   side (`width`) and the middle of its upper side (`height`).
 * - An Ellipse: its outline, from the end of its axis across round counter-clockwise and back to it,
 *   and the same three handles, at the ends of its two axes.
 * - A Polygon: its outline through its points and back to the first (none with fewer than two
 *   points), and a handle at the centre and at each point. A polygon of no size has all its points
 *   at its centre, and no handle but that one: a point of it cannot be told from where it is.
 * - A Merge and a Transform: the handle of the centre alone.
 * - A Tracker: `trackerControls`.
 *
 * The handle of the centre is first, so that a handle that lies on it (of a shape of no width) is
 * the one pressed, and the shape can be drawn out again. A centre that follows a Tracker is where
 * `options.follows` says, held within the range of the centre as the engine holds it, and its
 * handle is `fixed`; the outline and the other handles of a mask lie about it.
 */
export function controlsOf(node: FusionNode, keyFrame: number, picture: FusionPicture, options: ControlOptions = {}): Controls | null {
  if (!hasControls(node.kind)) return null;
  if (node.kind === 'tracker') return trackerControls(node, keyFrame, picture, options.placing ?? null);
  const follows = options.follows ?? null;
  const own: [number, number] = [read(node, 'center.x', keyFrame), read(node, 'center.y', keyFrame)];
  const followed = follows?.place ?? null;
  const [u, v]: [number, number] = followed === null ? own : [held(node, 'center.x', followed[0]), held(node, 'center.y', followed[1])];
  const centre = toPixels(picture, u, v);
  const middle: Handle = follows === null ? { kind: 'center', at: [pixel(centre[0]), pixel(centre[1])] }
    : { kind: 'center', at: [pixel(centre[0]), pixel(centre[1])], fixed: true };
  if (node.kind === 'merge' || node.kind === 'transform') return { lines: [], handles: [middle] };
  const [cosine, sine] = turnOf(read(node, 'angle', keyFrame));
  /** A place from the centre along the axes of the shape, `x` to the right and `y` up before it is turned, in pixels. */
  const place = (x: number, y: number): OverlayPoint => [pixel(centre[0] + x * cosine - y * sine), pixel(centre[1] - (x * sine + y * cosine))];
  if (node.kind === 'polygon') {
    const scale = read(node, 'size', keyFrame) * picture.width;
    const points = Array.from({ length: pointsOf(node) }, (_, index) => place(scale * read(node, pointPath(index, 'x'), keyFrame), scale * read(node, pointPath(index, 'y'), keyFrame)));
    return {
      lines: points.length >= 2 ? [[...points, points[0]]] : [],
      handles: [middle, ...(scale > 0 ? points.map((at, index): Handle => ({ kind: 'point', index, at })) : [])],
    };
  }
  const halfWidth = (read(node, 'width', keyFrame) * picture.width) / 2;
  const halfHeight = (read(node, 'height', keyFrame) * heightUnit(node, picture)) / 2;
  const outline = node.kind === 'rectangle'
    ? [place(-halfWidth, -halfHeight), place(halfWidth, -halfHeight), place(halfWidth, halfHeight), place(-halfWidth, halfHeight), place(-halfWidth, -halfHeight)]
    : Array.from({ length: ELLIPSE_STEPS + 1 }, (_, step) => {
      const [across, up] = turnOf((step * 360) / ELLIPSE_STEPS);
      return place(halfWidth * across, halfHeight * up);
    });
  return { lines: [outline], handles: [middle, { kind: 'width', at: place(halfWidth, 0) }, { kind: 'height', at: place(0, halfHeight) }] };
}

/**
 * The controls of a Tracker at a frame of its keys (PROTOCOL.md, "The Tracker"): of each pattern
 * that is on, in the order of its list,
 *
 * - the box of the pattern, `pattern.width` of the width of the picture wide and `pattern.height`
 *   of its height high, and the box that it is searched for in, `search.width` by `search.height`,
 *   both about where the pattern is at the frame (on its path where it has one, else where it was
 *   put, `pattern.x` and `pattern.y`; where the drag of `placing` has put it while it is dragged),
 *   each from its lower left corner round counter-clockwise and back to it, as a Rectangle is drawn;
 * - its path, where it has two frames or more: a line through the places of its frames in their
 *   order, thinned to `MOST_PATH_POINTS` (its first and its last always among them);
 * - a handle at its middle, and where it has a path a mark there, which is where the path has it at
 *   the playhead.
 *
 * A pattern that is off is not drawn, and has no handle.
 */
function trackerControls(node: Extract<FusionNode, { kind: 'tracker' }>, keyFrame: number, picture: FusionPicture, placing: ControlOptions['placing']): Controls {
  const lines: OverlayPoint[][] = [];
  const handles: Handle[] = [];
  const marks: OverlayPoint[] = [];
  const at = (u: number, v: number): OverlayPoint => {
    const [x, y] = toPixels(picture, u, v);
    return [pixel(x), pixel(y)];
  };
  (Array.isArray(node.params.trackers) ? node.params.trackers : []).forEach((pattern, index) => {
    if (!pattern.enabled) return;
    const onPath = placeOnPath(pattern.path, keyFrame);
    const [u, v] = placing?.index === index ? placing.place : onPath ?? [pattern.pattern.x, pattern.pattern.y];
    const [x, y] = toPixels(picture, u, v);
    /** A box about the middle of the pattern, `width` of the width of the picture wide and `height` of its height high. */
    const box = (width: number, height: number): OverlayPoint[] => {
      const across = (width * picture.width) / 2;
      const up = (height * picture.height) / 2;
      const corner = (right: number, top: number): OverlayPoint => [pixel(x + right * across), pixel(y - top * up)];
      return [corner(-1, -1), corner(1, -1), corner(1, 1), corner(-1, 1), corner(-1, -1)];
    };
    lines.push(box(pattern.pattern.width, pattern.pattern.height), box(pattern.search.width, pattern.search.height));
    const samples = pathSamples(pattern.path);
    if (samples.length >= 2) lines.push(thinned(samples, MOST_PATH_POINTS).map(([, place]) => at(place[0], place[1])));
    handles.push({ kind: 'pattern', index, at: [pixel(x), pixel(y)] });
    if (onPath !== null) marks.push(at(onPath[0], onPath[1]));
  });
  return { lines, handles, marks };
}

/**
 * What the engine is to draw for the controls of a node (`view.overlay`), over a frame of `canvas`
 * pixels: the outline, the centre in the colour of the place at the playhead (`current`), the other
 * handles over it as points of keys are drawn (`key`), the places of the patterns of a Tracker on
 * their paths at the playhead over those (`current`, where there are any), and the handle that is
 * being dragged (`active`, its place among the handles) on top in its own colour. A kind that has no
 * dot is left out.
 */
export function overlayOf(controls: Controls, canvas: [width: number, height: number], active: number | null = null): ViewOverlay {
  const held = active !== null && active >= 0 && active < controls.handles.length ? controls.handles[active] : null;
  const dots: ViewOverlay['dots'] = [
    { kind: 'current', at: controls.handles.filter(handle => handle.kind === 'center' && handle !== held).map(handle => handle.at) },
    { kind: 'key', at: controls.handles.filter(handle => handle.kind !== 'center' && handle !== held).map(handle => handle.at) },
    { kind: 'current', at: controls.marks ?? [] },
    { kind: 'active', at: held ? [held.at] : [] },
  ];
  return { canvas: [canvas[0], canvas[1]], lines: controls.lines, dots: dots.filter(each => each.at.length > 0) };
}

/**
 * The handle that is pressed at a place of the frame drawn: the nearest within `reach` pixels of the
 * frame, the later of two as near; -1 where none is. A handle that is `fixed` is never pressed.
 */
export function hitHandle(controls: Controls, at: OverlayPoint, reach: number): number {
  let found = -1;
  let nearest = Infinity;
  controls.handles.forEach((handle, index) => {
    if (handle.fixed) return;
    const distance = Math.hypot(handle.at[0] - at[0], handle.at[1] - at[1]);
    if (distance <= reach && distance <= nearest) { found = index; nearest = distance; }
  });
  return found;
}

/** How far from a handle the pointer may be to press it, in pixels of the frame drawn, as the picture is shown: `HIT_PX` pixels of the screen. */
export function reachOf(pointer: Pick<ViewPointer, 'presentation'>): number {
  const map = pointer.presentation?.map;
  return map && map.dest[2] > 0 ? (HIT_PX * map.picture[0]) / map.dest[2] : HIT_PX;
}

/**
 * The settings that a handle of a node sets, and the value of each once the pointer has gone `dx`
 * pixels of the frame drawn to the right and `dy` down since it pressed the handle: worked out from
 * the node as it is given (as it was when the drag began) at the frame `keyFrame` of its keys, so
 * that each move of a drag is the same whatever went before it. Each value is kept within the range
 * of its setting, and written to a millionth.
 *
 * - The centre goes with the pointer: `dx` of the width of the picture across, `dy` of its height down.
 * - The `width` of a Rectangle or an Ellipse grows by twice what the pointer goes along the axis
 *   across of the shape as it is turned, since the shape stays about its centre; its `height` by
 *   twice what the pointer goes along its axis up. Dragged through the centre, the shape comes out
 *   on the other side.
 * - A point of a Polygon goes with the pointer: its `x` and its `y` are counted from the centre
 *   before the polygon is sized and turned, so the way of the pointer is turned back by the Angle
 *   and divided by the Size, which is the mapping of the engine inverted.
 *
 * Nothing for a handle that the node has not, and for a point of a polygon of no size. A pattern of
 * a Tracker is no setting that takes keys: it is dragged by `draggedPattern`.
 */
export function draggedValues(node: FusionNode, handle: Pick<Handle, 'kind' | 'index'>, keyFrame: number, picture: FusionPicture, dx: number, dy: number): Moved[] {
  if (!hasControls(node.kind) || node.kind === 'tracker' || picture.width <= 0 || picture.height <= 0) return [];
  const moved = (path: string, value: (was: number) => number): Moved[] => {
    const setting = settingOf(node, path);
    return setting ? [{ setting, value: tidy(within(setting.bounds, value(read(node, path, keyFrame)))) }] : [];
  };
  if (handle.kind === 'center') {
    return [...moved('center.x', was => was + dx / picture.width), ...moved('center.y', was => was - dy / picture.height)];
  }
  if (node.kind === 'merge' || node.kind === 'transform') return [];
  const [cosine, sine] = turnOf(read(node, 'angle', keyFrame));
  // How far the pointer has gone along the two axes of the shape as it is turned: across, and up.
  const across = dx * cosine - dy * sine;
  const up = -dx * sine - dy * cosine;
  if (node.kind === 'polygon') {
    const index = handle.index;
    const scale = read(node, 'size', keyFrame) * picture.width;
    if (handle.kind !== 'point' || index === undefined || !(scale > 0)) return [];
    return [...moved(pointPath(index, 'x'), was => was + across / scale), ...moved(pointPath(index, 'y'), was => was + up / scale)];
  }
  if (handle.kind === 'width') return moved('width', was => Math.abs(was * picture.width + 2 * across) / picture.width);
  if (handle.kind === 'height') {
    const unit = heightUnit(node, picture);
    return moved('height', was => Math.abs(was * unit + 2 * up) / unit);
  }
  return [];
}

/**
 * Where the pattern `index` of a Tracker is put once the pointer has gone `dx` pixels of the frame
 * drawn to the right and `dy` down since it pressed its handle: from where the handle was drawn (on
 * the path at the frame `keyFrame` of the keys where the pattern has a path, else where it was put),
 * `dx` of the width of the picture across and `dy` of its height down, each kept within the range of
 * a place and written to a millionth, as a centre is dragged. Worked out from the node as it was when
 * the drag began. This is where the pattern is put (`pattern.x` and `pattern.y`), and not its path,
 * which the tracking writes. Null for a node that is no Tracker, a pattern that it has not, and a
 * picture that has no size.
 */
export function draggedPattern(node: FusionNode, index: number, keyFrame: number, picture: FusionPicture, dx: number, dy: number): [x: number, y: number] | null {
  if (node.kind !== 'tracker' || picture.width <= 0 || picture.height <= 0) return null;
  const pattern = Number.isInteger(index) && index >= 0 ? node.params.trackers[index] : undefined;
  if (!pattern) return null;
  const [u, v] = placeOnPath(pattern.path, keyFrame) ?? [pattern.pattern.x, pattern.pattern.y];
  return [tidy(within(TRACKER_PLACE, u + dx / picture.width)), tidy(within(TRACKER_PLACE, v - dy / picture.height))];
}
