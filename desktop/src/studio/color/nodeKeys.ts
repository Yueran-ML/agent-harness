/**
 * The keyframes of the nodes of a grade, as the Keyframe Editor of the Color page keeps them
 * (DaVinci Resolve Reference Manual 20, Ch. 148, pp. 3380–3394): which settings of a node take
 * keys, at which frames a node has a keyframe and of which kind, the operations that each command
 * of the editor sends, the two eases of Change Dynamic Attributes, and what Auto-Keyframe does to
 * a change. Pure, so that the editor, the palettes and the tests work from the same rules.
 *
 * The engine keeps keys setting by setting (`Param`: one number, or keys). The Keyframe Editor has
 * one track for each node, so a keyframe of a node at a frame is here a key at that frame on every
 * setting of the node that takes keys (the paths of the engine's catalog), each of the value that
 * the setting has there: the picture is the same with the keyframe as without it. A node is taken
 * to have a keyframe wherever one of its settings has a key, so that keys set elsewhere (by the
 * agent, on one setting) are seen and can be moved and deleted.
 *
 * Dynamic and static (pp. 3384–3385): the kind of a keyframe says how the settings go from it to
 * the next one, changing all the way (dynamic, a diamond) or staying and jumping at the next
 * (static, a circle); which is what `interp` says of a key, so dynamic is `linear` (or a Bezier
 * stretch, with Dynamic Attributes) and static is `hold`.
 *
 * The frames are those that the keys of the grade count (`keyClockOf`), not frames of a timeline.
 */
import type { Key } from '../../app/keys';
import type { Corrector, CurveKnot, Grade, GradeNode, Interp, Keyframe, NodeKind, Op, Param, Target } from '../engine';
import { deleteKeys, insertKey, spliceOf } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';
import { getAt, shownPoints, shownValue, withValue } from './corrector';
import { isLocked, type Worked } from './nodeEdits';

// --- The settings that take keys.

const each = (heads: readonly string[], tails: readonly string[]): string[] => heads.flatMap(head => tails.map(tail => `${head}.${tail}`));
const RGB = ['r', 'g', 'b'] as const;
const RANGE = ['low', 'high', 'low_soft', 'high_soft'] as const;

/**
 * The settings of a node that take keys, by its kind: the paths of the engine's catalog
 * (`engine/crates/model/tests/data/catalog.json`, `corrector` and `key_mixer`; the tests hold the
 * two together), in its order. A star is every place of a list (a window, a point); a number
 * written out is itself (the two numbers of a point of a curve, its `in` and its `out`). A
 * Parallel Mixer and a Layer Mixer have no setting that takes keys.
 */
export const NODE_KEYED: Readonly<Record<NodeKind, readonly string[]>> = {
  corrector: [
    ...each(['primaries.lift', 'primaries.gamma', 'primaries.gain'], ['y', 'r', 'g', 'b']),
    ...each(['primaries.offset'], RGB),
    ...each(['primaries'], ['contrast', 'pivot', 'saturation', 'hue', 'lum_mix', 'temperature', 'tint', 'color_boost', 'midtone_detail', 'shadows', 'highlights']),
    ...each(['log.shadow', 'log.midtone', 'log.highlight'], RGB),
    'log.low_range', 'log.high_range',
    ...each(['curves.y', 'curves.r', 'curves.g', 'curves.b'], ['intensity', 'points.*.0', 'points.*.1']),
    ...each(['hsl.hue_vs_hue', 'hsl.hue_vs_sat', 'hsl.hue_vs_lum', 'hsl.lum_vs_sat', 'hsl.sat_vs_sat', 'hsl.sat_vs_lum'], ['*.0', '*.1']),
    ...each(['soft_clip.low', 'soft_clip.high', 'soft_clip.low_soft', 'soft_clip.high_soft'], RGB),
    ...each(['key.input', 'key.output', 'key.qualifier'], ['gain', 'offset']),
    'key.input_blur.radius', 'key.input_blur.ratio',
    ...each(['blur.radius', 'blur.ratio', 'blur.scaling'], RGB),
    'blur.coring_softness', 'blur.level', 'blur.mix',
    ...each(['shot_match.gamma', 'shot_match.gain', 'shot_match.offset'], RGB),
    'shot_match.contrast', 'shot_match.pivot',
    ...each(['qualifier.hue'], ['center', 'width', 'soft', 'symmetry']),
    ...each(['qualifier.sat', 'qualifier.lum', 'qualifier.red', 'qualifier.green', 'qualifier.blue'], RANGE),
    ...each(['qualifier.finesse'], ['pre_filter', 'clean_black', 'black_clip', 'clean_white', 'white_clip', 'blur_radius', 'in_out_ratio', 'morph_radius']),
    'windows.*.opacity',
    ...each(['windows.*.transform'], ['size', 'aspect', 'pan', 'tilt', 'rotate']),
    ...each(['windows.*.shape'], ['softness', 'softness.*', 'inside', 'outside', 'points.*.x', 'points.*.y']),
    ...each(['windows.*.shape.points.*.position', 'windows.*.shape.points.*.in', 'windows.*.shape.points.*.out'], ['x', 'y']),
  ],
  key_mixer: ['inputs.*.gain', 'inputs.*.offset', 'output.gain', 'output.offset'],
  parallel_mixer: [],
  layer_mixer: [],
};

/** A setting of a node: its path in the node's `params`, as `set_keys` names it, and what it is. */
export interface Setting { path: string; param: Param }

const isParam = (value: unknown): value is Param => typeof value === 'number'
  || (typeof value === 'object' && value !== null && !Array.isArray(value) && Array.isArray((value as { keys?: unknown }).keys));

/** The settings that a path of the catalog names in what holds them: one, or one for each place of a list; none where the path leads nowhere. */
function found(holder: unknown, parts: readonly string[], depth: number, path: string, into: Setting[]): void {
  if (depth === parts.length) {
    if (isParam(holder)) into.push({ path, param: holder });
    return;
  }
  const part = parts[depth];
  const under = (name: string) => (path === '' ? name : `${path}.${name}`);
  if (Array.isArray(holder)) {
    if (part === '*') holder.forEach((item: unknown, at) => found(item, parts, depth + 1, under(String(at)), into));
    else if (/^\d+$/.test(part) && Number(part) < holder.length) found(holder[Number(part)], parts, depth + 1, under(part), into);
    return;
  }
  if (part === '*' || typeof holder !== 'object' || holder === null || !(part in holder)) return;
  found((holder as Record<string, unknown>)[part], parts, depth + 1, under(part), into);
}

/**
 * The settings of the params of a node of a kind that take keys, in the order of the catalog, the
 * places of a list in their order. What the params do not hold is left out: a qualifier or a shot
 * match that the node has not, the softness of a shape that has another.
 */
export function settingsIn(kind: NodeKind, params: unknown): Setting[] {
  const into: Setting[] = [];
  for (const pattern of NODE_KEYED[kind]) found(params, pattern.split('.'), 0, '', into);
  return into;
}

/** The settings of a node that take keys: see `settingsIn`. */
export const settingsOf = (node: GradeNode): Setting[] => settingsIn(node.kind, node.params);

/** What holds settings with one of them put at its path: every object on the way is copied, and nothing beside it. */
function put(holder: unknown, parts: readonly string[], depth: number, value: Param): unknown {
  if (depth === parts.length) return value;
  const part = parts[depth];
  if (Array.isArray(holder)) return holder.map((item: unknown, at) => (String(at) === part ? put(item, parts, depth + 1, value) : item));
  if (typeof holder !== 'object' || holder === null) return holder;
  return { ...holder, [part]: put((holder as Record<string, unknown>)[part], parts, depth + 1, value) };
}

/** The params of a node with these settings in place of what was at their paths. */
export function paramsWith(params: unknown, settings: readonly Setting[]): unknown {
  let made = params;
  for (const { path, param } of settings) made = put(made, path.split('.'), 0, param);
  return made;
}

// --- Keyframes: where a node has them, and of which kind.

/** How the settings go from a keyframe to the next (pp. 3384–3385). */
export type KeyKind = 'dynamic' | 'static';
/** The kind that a key is of: one that holds is static, every other dynamic. */
export const kindOf = (interp: Interp): KeyKind => (interp === 'hold' ? 'static' : 'dynamic');
const interpOf = (kind: KeyKind): Interp => (kind === 'static' ? 'hold' : 'linear');

/**
 * A keyframe of a track: its frame and its kind. Where the keys at the frame are not all of one
 * kind (some settings hold and others do not, as keys set elsewhere can be), it is shown as
 * dynamic, and `mixed` says so.
 */
export interface NodeKeyframe { frame: number; kind: KeyKind; mixed: boolean }

const keysIn = (param: Param): readonly Keyframe[] => (typeof param === 'number' ? [] : param.keys);
const placeAt = (param: Param, frame: number): number => keysIn(param).findIndex(key => key.frame === frame);
const copy = (key: Keyframe): Keyframe => ({ ...key });

/** The keyframes of settings, in the order of their frames: one wherever a setting has a key. */
function keyframesIn(settings: readonly Setting[]): NodeKeyframe[] {
  const kinds = new Map<number, { dynamic: boolean; held: boolean }>();
  for (const { param } of settings) {
    for (const key of keysIn(param)) {
      const at = kinds.get(key.frame) ?? { dynamic: false, held: false };
      if (key.interp === 'hold') at.held = true;
      else at.dynamic = true;
      kinds.set(key.frame, at);
    }
  }
  return [...kinds].sort(([a], [b]) => a - b)
    .map(([frame, { dynamic, held }]): NodeKeyframe => ({ frame, kind: dynamic ? 'dynamic' : 'static', mixed: dynamic && held }));
}

/** The keyframes of a node, in the order of their frames. */
export const keyframesOf = (node: GradeNode): NodeKeyframe[] => keyframesIn(settingsOf(node));

const keyedParams = new WeakMap<object, boolean>();

/**
 * Whether a node has a keyframe: the badge of the node editor (p. 3386). What it finds for the
 * params of a node is kept, as the node editor asks it of every node whenever it is drawn.
 */
export function hasKeyframes(node: GradeNode): boolean {
  if (typeof node.params !== 'object' || node.params === null) return false;
  const known = keyedParams.get(node.params);
  if (known !== undefined) return known;
  const keyed = settingsOf(node).some(({ param }) => keysIn(param).length > 0);
  keyedParams.set(node.params, keyed);
  return keyed;
}

/** A track of the Keyframe Editor: the master track (`node` null), or a node's. */
export interface KeyTrack { node: string | null; keyframes: NodeKeyframe[] }

/** The keyframes of every track together, as the master track shows them (p. 3382): one wherever a track has one. */
export function masterKeyframes(tracks: readonly (readonly NodeKeyframe[])[]): NodeKeyframe[] {
  const kinds = new Map<number, { dynamic: boolean; held: boolean; mixed: boolean }>();
  for (const track of tracks) {
    for (const { frame, kind, mixed } of track) {
      const at = kinds.get(frame) ?? { dynamic: false, held: false, mixed: false };
      if (kind === 'static') at.held = true;
      else at.dynamic = true;
      at.mixed ||= mixed;
      kinds.set(frame, at);
    }
  }
  return [...kinds].sort(([a], [b]) => a - b)
    .map(([frame, { dynamic, held, mixed }]): NodeKeyframe => ({ frame, kind: dynamic ? 'dynamic' : 'static', mixed: mixed || (dynamic && held) }));
}

/** The tracks of a grade: the master track, then one for each node in the order of the node editor's numbers. */
export function tracksOf(grade: Grade): KeyTrack[] {
  const nodes = grade.nodes.map((node): KeyTrack => ({ node: node.id, keyframes: keyframesOf(node) }));
  return [{ node: null, keyframes: masterKeyframes(nodes.map(track => track.keyframes)) }, ...nodes];
}

// --- One setting: a key put, taken away, changed or moved.

/**
 * A setting with a key of a kind at a frame. Where it has none there, one is put on its curve, of
 * the value that it has there (`insertKey`: a Bezier stretch is cut in two, so its shape stays);
 * a static key holds, and a dynamic one put on a stretch that holds goes on in a line. Where it has
 * one, that key is made of the kind, and is left as it is where it is of it already (a Bezier
 * stretch stays dynamic).
 */
export function keyedAt(param: Param, frame: number, kind: KeyKind): Param {
  const at = placeAt(param, frame);
  if (at >= 0 && typeof param !== 'number') {
    if (kindOf(param.keys[at].interp) === kind) return param;
    return { keys: param.keys.map((key, place): Keyframe => (place === at ? { ...key, interp: interpOf(kind) } : copy(key))) };
  }
  const made = insertKey(param, frame);
  if (!made) return param;
  const key = made.keys[made.index];
  made.keys[made.index] = { ...key, interp: kind === 'static' ? 'hold' : key.interp === 'hold' ? 'linear' : key.interp };
  return { keys: made.keys };
}

/** A setting without its key at a frame (`deleteKeys`); with its last key taken it is the number that the key had. */
export function withoutKeyAt(param: Param, frame: number): Param {
  const at = placeAt(param, frame);
  return at < 0 ? param : deleteKeys(param, [at], frame);
}

/** A setting with its key at a frame made of a kind (p. 3386); one without a key there is left as it is. */
export function withKindAt(param: Param, frame: number, kind: KeyKind): Param {
  return placeAt(param, frame) < 0 ? param : keyedAt(param, frame, kind);
}

/** A setting with its key at `from` put at `to`. Whoever asks keeps `to` between the keys beside it (`moveRange`). */
export function withKeyMoved(param: Param, from: number, to: number): Param {
  const at = placeAt(param, from);
  if (at < 0 || typeof param === 'number' || from === to) return param;
  return { keys: param.keys.map((key, place): Keyframe => (place === at ? { ...key, frame: to } : copy(key))) };
}

// --- Change Dynamic Attributes (pp. 3391–3392).

/**
 * The two eases of a dynamic keyframe, Start and End, each from 0 to `EASE_MOST`; 0 is none, and
 * with both at 0 the settings change in a line.
 *
 * They are the two handles of the Bezier stretch from the keyframe to the next: Start is the
 * handle that leaves this keyframe (`out`) and End the one that comes to the next (`in`). A handle
 * is level (its `dv` is 0, so it is the same for every setting of the node and stays right where a
 * value is changed afterwards), and reaches over `amount / EASE_MOST * EASE_REACH` of the frames
 * between the two keyframes. A level handle of no length eases nothing at its end: with the other
 * of no length too, the four control points lie on the line and the stretch is written `linear`.
 * With both at 2 the stretch is the Ease In and Out of the Edit page, whose handles reach over a
 * third.
 *
 * The scale (0 to 4) and the reach (two thirds, at which the middle of a stretch eased at both ends
 * goes three times as fast as a line) are ours: the values of the manual's dialog could not be
 * read, and are to be set against DaVinci Resolve.
 */
export const EASE_MOST = 4;
export const EASE_REACH = 2 / 3;
/** How much an ease changes at a step of its slider. */
export const EASE_STEP = 0.5;

const kept = (amount: number): number => (Number.isFinite(amount) ? Math.min(EASE_MOST, Math.max(0, amount)) : 0);
/** The share of the frames of a stretch that the handle of an ease reaches over. */
export const easeShare = (amount: number): number => (kept(amount) / EASE_MOST) * EASE_REACH;
/** The ease of a handle that reaches over a share of its stretch, to a thousandth. */
export const easeAmount = (share: number): number => kept(Math.round((share / EASE_REACH) * EASE_MOST * 1000) / 1000);

/** How a stretch with these eases goes: `linear` with none, else a Bezier stretch of two level handles. */
export function easeInterp(start: number, end: number): Interp {
  const out = easeShare(start);
  const into = easeShare(end);
  return out === 0 && into === 0 ? 'linear' : { bezier: { out: [out, 0], in: [into, 0] } };
}

/** The eases of a stretch, by how far its handles reach; null of one that holds. */
export function easesOf(interp: Interp): { start: number; end: number } | null {
  if (interp === 'hold') return null;
  if (interp === 'linear') return { start: 0, end: 0 };
  return { start: easeAmount(interp.bezier.out[0]), end: easeAmount(interp.bezier.in[0]) };
}

/** A setting with the stretch that leaves its key at a frame eased: left as it is where there is no such stretch, or it holds. */
export function withEaseAt(param: Param, frame: number, start: number, end: number): Param {
  const at = placeAt(param, frame);
  if (at < 0 || typeof param === 'number' || at + 1 >= param.keys.length || param.keys[at].interp === 'hold') return param;
  const interp = easeInterp(start, end);
  return { keys: param.keys.map((key, place): Keyframe => (place === at ? { ...key, interp } : copy(key))) };
}

/**
 * The eases of the keyframe of a node at a frame, as its dialog shows them: those of the first of
 * its settings (in the order of the catalog) whose key there leaves a stretch that does not hold.
 * Null where there is none: a static keyframe, or the last keyframe of the node.
 */
export function easeOf(node: GradeNode, frame: number): { start: number; end: number } | null {
  for (const { param } of settingsOf(node)) {
    const at = placeAt(param, frame);
    if (at < 0 || typeof param === 'number' || at + 1 >= param.keys.length) continue;
    const eases = easesOf(param.keys[at].interp);
    if (eases) return eases;
  }
  return null;
}

// --- The operations of one node.

/** The two operations that change the keys of a setting of a node. */
export type KeyOp = Extract<Op, { op: 'set_keys' | 'splice_keys' }>;

/**
 * Make a setting what it is to be: one `splice_keys` of the keys that differ, between those that
 * the two begin with alike and those that they end with alike; `set_keys` where it is to be a
 * number, since a setting keeps one key at least; and nothing where it is that already.
 */
export function settingOps(grade: Target, id: string, path: string, before: Param, after: Param): KeyOp[] {
  if (typeof after === 'number') return before === after ? [] : [{ op: 'set_keys', grade, id, path, param: after }];
  if (after.keys.length === 0) return [];
  const splice = spliceOf(keysIn(before), after.keys);
  return splice ? [{ op: 'splice_keys', grade, id, path, ...splice }] : [];
}

const changed = (grade: Target, node: GradeNode, change: (param: Param) => Param): KeyOp[] =>
  settingsOf(node).flatMap(({ path, param }) => settingOps(grade, node.id, path, param, change(param)));

/** A keyframe of a kind put on a node at a frame: a key on each of its settings (`keyedAt`). */
export const addKeyframe = (grade: Target, node: GradeNode, frame: number, kind: KeyKind): KeyOp[] => changed(grade, node, param => keyedAt(param, frame, kind));

/** The keyframe of a node at a frame taken away: every key there; a setting left without one is a number again. */
export const removeKeyframe = (grade: Target, node: GradeNode, frame: number): KeyOp[] => changed(grade, node, param => withoutKeyAt(param, frame));

/**
 * Every keyframe of a node taken away (Delete All Keyframes, p. 3392): each setting is the number
 * that it is at `at`, a frame of the keys (the playhead), so the picture shown stays as it is.
 */
export const removeAllKeyframes = (grade: Target, node: GradeNode, at: number): KeyOp[] => changed(grade, node, param => (typeof param === 'number' ? param : valueAt(param, at)));

/** The keyframe of a node at a frame made dynamic or static (p. 3386). */
export const setKeyframeKind = (grade: Target, node: GradeNode, frame: number, kind: KeyKind): KeyOp[] => changed(grade, node, param => withKindAt(param, frame, kind));

/** The keyframe of a node at a frame given its two eases (Change Dynamic Attributes, pp. 3391–3392). */
export const easeKeyframe = (grade: Target, node: GradeNode, frame: number, start: number, end: number): KeyOp[] =>
  changed(grade, node, param => withEaseAt(param, frame, start, end));

/**
 * The frames that the keyframe of a node at a frame can be moved to, first and last: strictly
 * between the keyframes of the node beside it, so that no key of any setting passes another. Null
 * where the node has no keyframe there.
 */
export function moveRange(node: GradeNode, frame: number): [number, number] | null {
  const frames = keyframesOf(node).map(each => each.frame);
  const at = frames.indexOf(frame);
  if (at < 0) return null;
  return [at > 0 ? frames[at - 1] + 1 : -Infinity, at + 1 < frames.length ? frames[at + 1] - 1 : Infinity];
}

/**
 * The keyframe of a node at `from` put at `to`, as a drag sends it: each setting that has a key
 * there set whole (`set_keys`), which is the same whatever went before it. With `to` at `from`, it
 * is what puts the settings back as they are.
 */
export function moveKeyframe(grade: Target, node: GradeNode, from: number, to: number): KeyOp[] {
  return settingsOf(node).flatMap(({ path, param }): KeyOp[] => (placeAt(param, from) < 0 ? []
    : [{ op: 'set_keys', grade, id: node.id, path, param: withKeyMoved(param, from, to) }]));
}

/** A node as it is with a keyframe of a kind at a frame: what `addKeyframe` makes of it. */
export function withKeyframe(node: GradeNode, frame: number, kind: KeyKind): GradeNode {
  const settings = settingsOf(node).map(({ path, param }): Setting => ({ path, param: keyedAt(param, frame, kind) }));
  return { ...node, params: paramsWith(node.params, settings) } as GradeNode;
}

/** A node as it is with its keyframe at `from` put at `to`: what a drag shows before the engine tells of it. */
export function withKeyframeMoved(node: GradeNode, from: number, to: number): GradeNode {
  const settings = settingsOf(node).map(({ path, param }): Setting => ({ path, param: withKeyMoved(param, from, to) }));
  return { ...node, params: paramsWith(node.params, settings) } as GradeNode;
}

// --- The commands of the editor, each one transaction.

/**
 * Which tracks a keyframe is put on or taken from (pp. 3383–3384): `all`, every node's; `color`,
 * the current node's alone. (Sizing, the third mode of DaVinci Resolve, is not here: this studio
 * has no Sizing track yet.)
 */
export type KeyScope = 'all' | 'color';
export const KEY_SCOPES: readonly KeyScope[] = ['all', 'color'];

/** Whose tracks a command is for: those of the mode (the tool bar and the keys), every node's (the master track), or one node's. */
export type KeyTracks = 'scope' | 'master' | { node: string };

/** The nodes whose tracks a command is for. */
export function nodesFor(grade: Grade, tracks: KeyTracks, scope: KeyScope, current: string | null): GradeNode[] {
  if (tracks === 'master' || (tracks === 'scope' && scope === 'all')) return [...grade.nodes];
  const id = tracks === 'scope' ? current : tracks.node;
  return grade.nodes.filter(node => node.id === id);
}

/**
 * The operations of a command on several nodes, as one transaction. A locked node keeps its
 * settings (p. 3382) and is passed over; where the command would change locked nodes alone, it is
 * refused as the engine would refuse it. Nothing to do is null.
 */
function onNodes(nodes: readonly GradeNode[], ops: (node: GradeNode) => KeyOp[]): Worked {
  const all: KeyOp[] = [];
  let locked = false;
  for (const node of nodes) {
    const made = ops(node);
    if (made.length === 0) continue;
    if (isLocked(node)) locked = true;
    else all.push(...made);
  }
  return all.length > 0 ? all : locked ? { refused: 'node_locked' } : null;
}

/** Add Dynamic Keyframe (Ctrl+[) or Add Static Keyframe (Ctrl+]) at a frame (pp. 3384–3385). */
export const addKeyframes = (grade: Target, nodes: readonly GradeNode[], frame: number, kind: KeyKind): Worked => onNodes(nodes, node => addKeyframe(grade, node, frame, kind));
/** Delete Keyframe (Alt+]) at a frame (p. 3392). */
export const deleteKeyframes = (grade: Target, nodes: readonly GradeNode[], frame: number): Worked => onNodes(nodes, node => removeKeyframe(grade, node, frame));
/** Delete All Keyframes (p. 3392); `at` is the frame of the keys at the playhead. */
export const deleteAllKeyframes = (grade: Target, nodes: readonly GradeNode[], at: number): Worked => onNodes(nodes, node => removeAllKeyframes(grade, node, at));
/** Change to Dynamic Keyframe, or to Static (p. 3386). */
export const changeKeyframes = (grade: Target, nodes: readonly GradeNode[], frame: number, kind: KeyKind): Worked => onNodes(nodes, node => setKeyframeKind(grade, node, frame, kind));
/** Change Dynamic Attributes (pp. 3391–3392). */
export const easeKeyframes = (grade: Target, nodes: readonly GradeNode[], frame: number, start: number, end: number): Worked =>
  onNodes(nodes, node => easeKeyframe(grade, node, frame, start, end));

/** The frames at which any of these nodes has a keyframe, in their order, each once. */
export function keyframeFrames(nodes: readonly GradeNode[]): number[] {
  return masterKeyframes(nodes.map(keyframesOf)).map(each => each.frame);
}

/** What a key asks of the Keyframe Editor. */
export type KeyframeCommand = { kind: 'add'; key: KeyKind } | { kind: 'delete' } | { kind: 'step'; direction: -1 | 1 };

/**
 * The keys of keyframes on the Color page, as DaVinci Resolve has them on Windows: Ctrl+[ adds a
 * dynamic keyframe and Ctrl+] a static one (pp. 3384–3385), Alt+] deletes the keyframe at the
 * playhead (p. 3392), and [ and ] go to the keyframe before and after (p. 1114). A bracket is
 * known by where it lies, as Alt changes what it types on some layouts.
 */
export function keyframeKey(key: Key): KeyframeCommand | null {
  if (key.composing || key.meta || key.shift) return null;
  const left = key.code === 'BracketLeft' || key.key === '[';
  const right = key.code === 'BracketRight' || key.key === ']';
  if (!left && !right) return null;
  if (!key.ctrl && !key.alt) return { kind: 'step', direction: left ? -1 : 1 };
  if (key.repeat) return null;
  if (key.ctrl && !key.alt) return { kind: 'add', key: left ? 'dynamic' : 'static' };
  return !key.ctrl && right ? { kind: 'delete' } : null;
}

// --- Auto-Keyframe (p. 3383).

/** The corrector that a change is made to under Auto-Keyframe: with the keyframe that the change puts first (`keyed`), and as it was shown before it (`shown`). */
export interface AutoKeyed { keyed: Corrector; shown: Corrector }

const autoMade = new WeakMap<Corrector, { frame: number; keyed: Corrector | null }>();

/**
 * The settings of a corrector with a dynamic keyframe at a frame, as Auto-Keyframe puts one before
 * a change at a frame where the node has no keyframe; null where it has one there already (one of
 * its settings has a key at the frame), and the change goes into it. What it gives for a corrector
 * and a frame is kept, as it is asked for many times while a palette is drawn.
 */
export function autoKeyed(params: Corrector, frame: number): Corrector | null {
  const made = autoMade.get(params);
  if (made && made.frame === frame) return made.keyed;
  const settings = settingsIn('corrector', params);
  const keyed = settings.some(({ param }) => placeAt(param, frame) >= 0) ? null
    : paramsWith(params, settings.map(({ path, param }): Setting => ({ path, param: keyedAt(param, frame, 'dynamic') }))) as Corrector;
  autoMade.set(params, { frame, keyed });
  return keyed;
}

/**
 * What a number given to a setting under Auto-Keyframe makes of it. A control gives the number
 * that it shows, which for a setting with keys is that of the key at its left (p. 3384), not the
 * value between two keys; so a number that is what the control showed changes nothing, and the
 * setting keeps the new key with the value that it had at the frame. Another number goes into the
 * key at the frame. `current` is the corrector as the edits before this one have left it.
 */
export function autoValue(path: string, value: number, current: Corrector, auto: AutoKeyed, frame: number): Param {
  const was = getAt(auto.shown, path) as Param | undefined;
  const first = getAt(auto.keyed, path) as Param | undefined;
  if (was !== undefined && first !== undefined && value === shownValue(was, frame)) return first;
  return withValue(getAt(current, path) as Param | undefined, frame, value);
}

/** The paths at which a corrector keeps the points of a curve. */
const CURVE_POINTS = /^(curves\.[yrgb]\.points|hsl\.[a-z_]+)$/;
const isKnots = (value: unknown): value is CurveKnot[] => Array.isArray(value) && value.every(each => Array.isArray(each) && each.length === 2 && isParam(each[0]) && isParam(each[1]));

/**
 * What a value put whole at a path under Auto-Keyframe makes of it. The points of a curve, where
 * the edit left as many as there were, go into the keys at the frame number by number, as
 * `autoValue` puts each; a curve that gained or lost a point, and everything that is no curve (a
 * switch, a LUT), is put as it is given.
 */
export function autoWhole(path: string, whole: unknown, current: Corrector, auto: AutoKeyed, frame: number): unknown {
  if (!CURVE_POINTS.test(path)) return whole;
  const now = getAt(current, path);
  const was = getAt(auto.shown, path);
  const first = getAt(auto.keyed, path);
  if (!isKnots(whole) || !isKnots(now) || !isKnots(was) || !isKnots(first)) return whole;
  if (whole.length !== now.length || whole.length !== was.length || whole.length !== first.length) return whole;
  const next = shownPoints(whole, frame);
  const old = shownPoints(was, frame);
  return now.map((knot, at): CurveKnot => [
    next[at][0] === old[at][0] ? first[at][0] : withValue(knot[0], frame, next[at][0]),
    next[at][1] === old[at][1] ? first[at][1] : withValue(knot[1], frame, next[at][1]),
  ]);
}
