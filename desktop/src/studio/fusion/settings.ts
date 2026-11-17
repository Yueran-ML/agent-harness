/**
 * The settings of the nodes of a composition that take keys, and the operations that set them, as
 * the Inspector and the Spline Editor of the Fusion page do (DaVinci Resolve Reference Manual 20,
 * Ch. 70, pp. 1432–1457): a setting with no keys is one number; once it has a key, a value given at
 * another frame puts a key there, and the button beside it puts a key at the playhead or takes away
 * the one there.
 *
 * The settings, their defaults and their ranges are those of the engine's catalog of the settings
 * that take keys (`engine/crates/model/tests/data/catalog.json`, `fusion_merge`, `fusion_transform`,
 * `fusion_background`, `fusion_rectangle`, `fusion_ellipse`, `fusion_polygon`, `fusion_blur`,
 * `fusion_mosaic` and `fusion_delta_keyer`; the tests hold the two together). A setting is named by its path in the `params` of its node, and in an
 * operation by the id of the node, a dot, and that path (`n4.center.x`). The two numbers of a point
 * of a Polygon are settings each, named by the place of the point in the list of points, from 0
 * (`points.2.x`); the catalog writes them once, for every point (`points.*.x`). The keys are at the
 * frames that the other keys of the clip count (`keyFrameOf`). Each function here gives the
 * operations to send, and none where nothing would change. Pure.
 */
import { getAt, patchAt, patchOf, type Bounds } from '../color/corrector';
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import type { Clip, FusionKind, FusionNode, Keyframe, Media, Op, Param, Timeline } from '../engine';
import { frameOfKey, keyAt, mediaOf } from '../inspector/clipKeys';
import { spliceOf } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';
import type { FusionAt } from './edits';

/**
 * What a setting counts in, as the catalog names it: a plain number, a share of the width or of the
 * height of the picture, degrees counter-clockwise, pixels of a picture 1080 high (a Blur Size, and
 * the lengths of a Delta Keyer), or pixels of the picture of the composition (an offset of a Mosaic).
 */
export type FusionUnit = 'number' | 'widths' | 'heights' | 'degrees_counterclockwise' | 'pixels1080' | 'pixels';
/** A setting of a node that takes keys: its path in the node's `params`, its default and its range, and its unit. */
export interface FusionSetting { path: string; bounds: Bounds; unit: FusionUnit }

const s = (path: string, neutral: number, low: number, high: number, unit: FusionUnit = 'number'): FusionSetting => ({ path, bounds: { neutral, low, high }, unit });
/** Where a picture lies, how large it is and how far it is turned: of the foreground of a Merge, and of the picture of a Transform. */
const PLACE: readonly FusionSetting[] = [
  s('center.x', 0.5, -15.5, 16.5, 'widths'), s('center.y', 0.5, -15.5, 16.5, 'heights'), s('size', 1, 0, 16), s('angle', 0, -3600, 3600, 'degrees_counterclockwise'),
];

/** The controls that every mask has: its level, and how soft its edge and how wide its border are, in widths of the picture. */
const MASK: readonly FusionSetting[] = [s('level', 1, 0, 1), s('soft_edge', 0, 0, 1, 'widths'), s('border_width', 0, -1, 1, 'widths')];
/** Where the middle of a mask lies. */
const MASK_CENTER: readonly FusionSetting[] = [s('center.x', 0.5, -10, 10, 'widths'), s('center.y', 0.5, -10, 10, 'heights')];
/** How far a mask is turned about its middle. */
const MASK_ANGLE = s('angle', 0, -36000, 36000, 'degrees_counterclockwise');
/** The two numbers of every point of a Polygon, as the catalog writes them: the `*` is the place of the point. */
const POINT_X = 'points.*.x';
const POINT_Y = 'points.*.y';
/** The most that a channel of a colour of a Delta Keyer may be: the greatest number of a half float, so that a colour picked from a picture of high dynamic range is kept. */
const MOST_COLOR = 65504;
/** The three channels of a colour of a Delta Keyer, each a setting: of its screen colour, and of its Replace Color. */
const keyerColor = (name: string, r: number, g: number, b: number): FusionSetting[] => [
  s(`${name}.r`, r, 0, MOST_COLOR), s(`${name}.g`, g, 0, MOST_COLOR), s(`${name}.b`, b, 0, MOST_COLOR),
];

/**
 * The settings of each kind of node that take keys, in the order of the catalog. A MediaIn, a
 * MediaOut and a Tracker have none. Those of a Polygon end with the two numbers of a point, written once for
 * every point as the catalog writes them; `settingsOf` gives them for each point of a node.
 */
export const FUSION_SETTINGS: Record<Exclude<FusionKind, 'media_in' | 'media_out'>, readonly FusionSetting[]> = {
  merge: [s('subtractive_additive', 1, 0, 1), s('alpha_gain', 1, 0, 1), s('burn_in', 0, 0, 1), s('blend', 1, 0, 1), ...PLACE],
  transform: PLACE,
  background: [s('color.r', 0, 0, 1), s('color.g', 0, 0, 1), s('color.b', 0, 0, 1), s('color.a', 1, 0, 1)],
  rectangle: [...MASK, ...MASK_CENTER, s('width', 0.5, 0, 10, 'widths'), s('height', 0.5, 0, 10, 'heights'), s('corner_radius', 0, 0, 1), MASK_ANGLE],
  // Both the width and the height of an Ellipse are in widths of the picture, so that alike they are a circle.
  ellipse: [...MASK, ...MASK_CENTER, s('width', 0.5, 0, 10, 'widths'), s('height', 0.5, 0, 10, 'widths'), MASK_ANGLE],
  polygon: [...MASK, ...MASK_CENTER, s('size', 1, 0, 10), MASK_ANGLE, s(POINT_X, 0, -16, 16, 'widths'), s(POINT_Y, 0, -16, 16, 'widths')],
  // The Blur Size is counted as the filters of the Color page are, in pixels of a picture 1080 high.
  blur: [s('size.x', 5, 0, 500, 'pixels1080'), s('size.y', 5, 0, 500, 'pixels1080'), s('blend', 1, 0, 1)],
  mosaic: [s('frequency', 32, 1, 16384), s('aspect', 1, 0.0625, 16), s('offset.x', 0, -16384, 16384, 'pixels'), s('offset.y', 0, -16384, 16384, 'pixels')],
  // The sixteen numbers of a Delta Keyer, in the order of its `params`: its Pre-Blur, its Erode/Dilate and its Matte Blur are lengths, counted as a Blur Size is.
  delta_keyer: [
    ...keyerColor('screen', 0, 1, 0), s('gain', 1, 0, 10), s('balance', 0.5, 0, 1), s('pre_blur', 0, 0, 100, 'pixels1080'),
    s('low', 0, 0, 1), s('high', 1, 0, 1), s('erode_dilate', 0, -100, 100, 'pixels1080'), s('matte_blur', 0, 0, 100, 'pixels1080'),
    s('clean_foreground', 0, 0, 1), s('clean_background', 0, 0, 1), ...keyerColor('replace_color', 0.5, 0.5, 0.5), s('spill_suppression', 1, 0, 1),
  ],
  // A Tracker has no entry in the catalog: none of its settings takes keys, and the paths of its patterns are what moves it.
  tracker: [],
};

/**
 * The sentences for the two refusals of a Delta Keyer that are of no one number (PROTOCOL.md, "The
 * Delta Keyer"): a screen colour that has no channel above its other two, and a lower threshold
 * above the upper one. Each is named as `refusalOf` takes a sentence for a refusal of one thing: by
 * the reason, a colon, and what the engine says the refusal is of.
 */
export const keyerRefusals = (said: { keyerScreen: string; keyerThreshold: string }): Record<string, string> => ({
  'out_of_range:delta_keyer.screen': said.keyerScreen, 'out_of_range:delta_keyer.low': said.keyerThreshold,
});

const NONE: readonly FusionSetting[] = [];

/** Whether a setting is a number of a point of a Polygon, whose path names the place of the point: `points.2.x`. */
export const isPointSetting = (setting: Pick<FusionSetting, 'path'>): boolean => setting.path.startsWith('points.');

/** The path of a number of a point of a Polygon: of the point `index`, from 0, its `x` or its `y`. */
export const pointPath = (index: number, axis: 'x' | 'y'): string => `points.${index}.${axis}`;

/** The settings of a Polygon of so many points, kept so that the same list is given each time. */
const OF_POLYGON = new Map<number, readonly FusionSetting[]>();
function polygonSettings(points: number): readonly FusionSetting[] {
  const known = OF_POLYGON.get(points);
  if (known) return known;
  const plain = FUSION_SETTINGS.polygon.filter(setting => !isPointSetting(setting));
  const ofPoint = FUSION_SETTINGS.polygon.filter(isPointSetting);
  const made: FusionSetting[] = [...plain];
  for (let index = 0; index < points; index++) for (const setting of ofPoint) made.push({ ...setting, path: setting.path.replace('*', String(index)) });
  OF_POLYGON.set(points, made);
  return made;
}

/** The settings of a node that take keys: of a Polygon, those of its kind and then the two numbers of each of its points, in their order. */
export function settingsOf(node: FusionNode): readonly FusionSetting[] {
  if (node.kind === 'media_in' || node.kind === 'media_out') return NONE;
  if (node.kind !== 'polygon') return FUSION_SETTINGS[node.kind];
  return polygonSettings(Array.isArray(node.params.points) ? node.params.points.length : 0);
}

/** The setting of a node at a path of its `params`, or undefined where its kind has none there, or the polygon no such point. */
export function settingOf(node: FusionNode, path: string): FusionSetting | undefined {
  return settingsOf(node).find(setting => setting.path === path);
}

/** A name with the number of a point in it, where the catalogue of words writes `{n}`: the points are counted from 1. */
export const numbered = (text: string, index: number): string => text.replace('{n}', String(index + 1));

/** A setting of a node: a number, or keys. One that the node does not say is its default. */
export function paramOf(node: FusionNode, setting: FusionSetting): Param {
  const found = getAt(node.params, setting.path);
  return typeof found === 'number' || (typeof found === 'object' && found !== null) ? found as Param : setting.bounds.neutral;
}

/** A setting as an operation names it: the id of its node, a dot, and its path. */
export const pathOf = (node: Pick<FusionNode, 'id'>, setting: FusionSetting): string => `${node.id}.${setting.path}`;

/** Whether any setting of a node has keys: the node is drawn with a badge then (p. 1459). */
export function hasKeys(node: FusionNode): boolean {
  return settingsOf(node).some(setting => typeof paramOf(node, setting) === 'object');
}

/** Where a key at a frame goes among keys in the order of their frames. */
const placeOf = (keys: readonly Keyframe[], keyFrame: number) => {
  const at = keys.findIndex(key => key.frame > keyFrame);
  return at < 0 ? keys.length : at;
};

/**
 * A setting of a node set to one number: by a patch of the node's `params`; but a number of a point
 * of a Polygon by `set_fusion_keys` with the number, since a patch puts a list in place of a list,
 * whole, and cannot name one point of it.
 */
const setNumber = (at: FusionAt, node: Pick<FusionNode, 'id'>, setting: FusionSetting, value: number): Op => (isPointSetting(setting)
  ? { op: 'set_fusion_keys', ...at, path: pathOf(node, setting), param: value }
  : { op: 'set_fusion_node', ...at, id: node.id, params: patchAt(setting.path, value) });

/**
 * Give a setting a value at a frame of its keys. A setting with no keys is set to the number; one
 * with keys has the key at the frame changed, or a key put there.
 */
export function putValue(at: FusionAt, node: FusionNode, setting: FusionSetting, value: number, keyFrame: number): Op[] {
  const param = paramOf(node, setting);
  if (typeof param === 'number') return param === value ? [] : [setNumber(at, node, setting, value)];
  const path = pathOf(node, setting);
  const found = keyAt(param, keyFrame);
  if (found) return found.key.value === value ? [] : [{ op: 'splice_fusion_keys', ...at, path, at: found.at, remove: 1, keys: [{ ...found.key, value }] }];
  return [{ op: 'splice_fusion_keys', ...at, path, at: placeOf(param.keys, keyFrame), remove: 0, keys: [{ frame: keyFrame, value, interp: 'linear' }] }];
}

/**
 * Give several settings of a node values at a frame of their keys, together, as a colour chosen as
 * one sets its three channels. Those that have no keys are set by **one** patch of the node, so that
 * what the engine checks of them together (of the screen colour of a Delta Keyer, that one channel
 * is above the other two) is checked of the colour as it is to be, and never of one half way
 * between the two; those that have keys take the value as a key at the frame, each by its path.
 * A setting that is at its value already is left out.
 */
export function putValues(at: FusionAt, node: FusionNode, values: readonly { setting: FusionSetting; value: number }[], keyFrame: number): Op[] {
  const numbers: [string, number][] = [];
  const keyed: Op[] = [];
  for (const { setting, value } of values) {
    const param = paramOf(node, setting);
    if (typeof param !== 'number' || isPointSetting(setting)) keyed.push(...putValue(at, node, setting, value, keyFrame));
    else if (param !== value) numbers.push([setting.path, value]);
  }
  return numbers.length > 0 ? [{ op: 'set_fusion_node', ...at, id: node.id, params: patchOf(numbers) }, ...keyed] : keyed;
}

/** Put a key at a frame of the keys of a setting, of the value that it has there; none where there is one. */
export function addKey(at: FusionAt, node: FusionNode, setting: FusionSetting, keyFrame: number): Op[] {
  const param = paramOf(node, setting);
  if (keyAt(param, keyFrame)) return [];
  const keys = typeof param === 'number' ? [] : param.keys;
  const key: Keyframe = { frame: keyFrame, value: valueAt(param, keyFrame), interp: 'linear' };
  return [{ op: 'splice_fusion_keys', ...at, path: pathOf(node, setting), at: placeOf(keys, keyFrame), remove: 0, keys: [key] }];
}

/** Take away the key at a frame of the keys of a setting; its last key leaves its value, as one number. */
export function removeKey(at: FusionAt, node: FusionNode, setting: FusionSetting, keyFrame: number): Op[] {
  const param = paramOf(node, setting);
  const found = keyAt(param, keyFrame);
  if (!found || typeof param === 'number') return [];
  const path = pathOf(node, setting);
  if (param.keys.length === 1) return [{ op: 'set_fusion_keys', ...at, path, param: found.key.value }];
  return [{ op: 'splice_fusion_keys', ...at, path, at: found.at, remove: 1, keys: [] }];
}

/**
 * Make a setting what it is to be, a number or keys, as a curve editor leaves it: one
 * `splice_fusion_keys` of the keys that differ; `set_fusion_keys` where it is to be a number, since
 * a setting keeps one key at least; and nothing where it is that already.
 */
export function keyOps(at: FusionAt, node: FusionNode, setting: FusionSetting, after: Param): Op[] {
  const before = paramOf(node, setting);
  const path = pathOf(node, setting);
  if (typeof after === 'number') return before === after ? [] : [{ op: 'set_fusion_keys', ...at, path, param: after }];
  // The engine takes no setting of keys without one: its number stays.
  if (after.keys.length === 0) return [];
  const splice = spliceOf(typeof before === 'number' ? [] : before.keys, after.keys);
  return splice ? [{ op: 'splice_fusion_keys', ...at, path, ...splice }] : [];
}

/**
 * A setting as a value at a frame of its keys makes it of what it was (`initial`), set whole: what
 * each move of a drag sends, which is the same whatever went before it. A number is the number;
 * keys are the keys with the one at the frame changed, or with one put there.
 */
export function wholeValue(at: FusionAt, node: Pick<FusionNode, 'id'>, setting: FusionSetting, initial: Param, value: number, keyFrame: number): Op {
  if (typeof initial === 'number') return setNumber(at, node, setting, value);
  const found = keyAt(initial, keyFrame);
  const keys = found ? initial.keys.map((key, index) => (index === found.at ? { ...key, value } : key))
    : [...initial.keys.slice(0, placeOf(initial.keys, keyFrame)), { frame: keyFrame, value, interp: 'linear' as const }, ...initial.keys.slice(placeOf(initial.keys, keyFrame))];
  return { op: 'set_fusion_keys', ...at, path: pathOf(node, setting), param: { keys } };
}

/** A setting put back as it was: what a drag that is taken back sends. */
export function wholeParam(at: FusionAt, node: Pick<FusionNode, 'id'>, setting: FusionSetting, param: Param): Op {
  return typeof param === 'number' ? setNumber(at, node, setting, param) : { op: 'set_fusion_keys', ...at, path: pathOf(node, setting), param };
}

/**
 * The frames of the timeline at which a clip shows the pictures that the keys of a setting are at,
 * in the order of the timeline: for each key the first frame that shows its picture. Keys of
 * pictures that the clip does not show are left out.
 */
export function keyTimesOf(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], param: Param): number[] {
  if (typeof param === 'number' || clip.length <= 0) return [];
  const [low, high] = shownKeyRange(clip, keyClockOf(clip, timeline.rate, mediaOf(clip, media)));
  const times = param.keys.filter(key => key.frame >= low && key.frame <= high).map(key => frameOfKey(clip, timeline, media, key.frame));
  return [...new Set(times)].sort((a, b) => a - b);
}
