/**
 * The settings of the nodes of a composition that take keys, and the operations that set them, as
 * the Inspector and the Spline Editor of the Fusion page do (DaVinci Resolve Reference Manual 20,
 * Ch. 70, pp. 1432–1457): a setting with no keys is one number; once it has a key, a value given at
 * another frame puts a key there, and the button beside it puts a key at the playhead or takes away
 * the one there.
 *
 * The settings, their defaults and their ranges are those of the engine's catalog of the settings
 * that take keys (`engine/crates/model/tests/data/catalog.json`, `fusion_merge`, `fusion_transform`
 * and `fusion_background`; the tests hold the two together). A setting is named by its path in the
 * `params` of its node, and in an operation by the id of the node, a dot, and that path
 * (`n4.center.x`). The keys are at the frames that the other keys of the clip count
 * (`keyFrameOf`). Each function here gives the operations to send, and none where nothing would
 * change. Pure.
 */
import { getAt, patchAt, type Bounds } from '../color/corrector';
import { keyClockOf, shownKeyRange } from '../color/keyFrames';
import type { Clip, FusionNode, Keyframe, Media, Op, Param, Timeline } from '../engine';
import { frameOfKey, keyAt, mediaOf } from '../inspector/clipKeys';
import { spliceOf } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';
import type { FusionAt } from './edits';

/** What a setting counts in, as the catalog names it: a plain number, a share of the width or of the height of the picture, or degrees counter-clockwise. */
export type FusionUnit = 'number' | 'widths' | 'heights' | 'degrees_counterclockwise';
/** A setting of a node that takes keys: its path in the node's `params`, its default and its range, and its unit. */
export interface FusionSetting { path: string; bounds: Bounds; unit: FusionUnit }

const s = (path: string, neutral: number, low: number, high: number, unit: FusionUnit = 'number'): FusionSetting => ({ path, bounds: { neutral, low, high }, unit });
/** Where a picture lies, how large it is and how far it is turned: of the foreground of a Merge, and of the picture of a Transform. */
const PLACE: readonly FusionSetting[] = [
  s('center.x', 0.5, -15.5, 16.5, 'widths'), s('center.y', 0.5, -15.5, 16.5, 'heights'), s('size', 1, 0, 16), s('angle', 0, -3600, 3600, 'degrees_counterclockwise'),
];

/** The settings of each kind of node that take keys, in the order of the catalog. A MediaIn and a MediaOut have none. */
export const FUSION_SETTINGS: Record<'merge' | 'transform' | 'background', readonly FusionSetting[]> = {
  merge: [s('subtractive_additive', 1, 0, 1), s('alpha_gain', 1, 0, 1), s('burn_in', 0, 0, 1), s('blend', 1, 0, 1), ...PLACE],
  transform: PLACE,
  background: [s('color.r', 0, 0, 1), s('color.g', 0, 0, 1), s('color.b', 0, 0, 1), s('color.a', 1, 0, 1)],
};

const NONE: readonly FusionSetting[] = [];

/** The settings of a node that take keys. */
export function settingsOf(node: Pick<FusionNode, 'kind'>): readonly FusionSetting[] {
  return node.kind === 'merge' || node.kind === 'transform' || node.kind === 'background' ? FUSION_SETTINGS[node.kind] : NONE;
}

/** The setting of a node at a path of its `params`, or undefined where its kind has none there. */
export function settingOf(node: Pick<FusionNode, 'kind'>, path: string): FusionSetting | undefined {
  return settingsOf(node).find(setting => setting.path === path);
}

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

/** A setting of a node set to one number, by a patch of the node's `params`. */
const setNumber = (at: FusionAt, node: Pick<FusionNode, 'id'>, setting: FusionSetting, value: number): Op =>
  ({ op: 'set_fusion_node', ...at, id: node.id, params: patchAt(setting.path, value) });

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
