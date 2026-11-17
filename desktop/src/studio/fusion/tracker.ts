/**
 * The Tracker of a composition on the Fusion page (DaVinci Resolve Reference Manual 20, Chapter 82,
 * pp. 1724–1759 and 2773–2784; PROTOCOL.md, "The Tracker" and "Following a Tracker"): its patterns,
 * each a place on the picture with its path, and what the centres of other nodes follow of it.
 *
 * None of the settings of a Tracker takes keys. Its patterns are a list that is set whole
 * (`set_fusion_node` with `{"trackers": [...]}`): every change of one pattern sends the list with
 * the others as the objects that they are, their paths with them, so that a path that the tracking
 * wrote is never changed by what the page does to another pattern. A path is the result of the
 * tracking: the page clears one, and changes none.
 *
 * The paths count the frames of the keys of the clip (`keyFrameOf`), and are read as the engine
 * reads them (`Pattern::place_at` of `crates/model/src/fusion/tracker.rs`): on the line between
 * the two frames of a path about a frame, and where the first or the last frame of the path has it
 * before and after them. Pure.
 */
import type { Composition, FusionFollow, FusionNode, FusionTracker, FusionTrackerPattern, Op } from '../engine';
import { followersOf, followOf, followOps, MOST_TRACKERS, newPattern, type FusionAt } from './edits';
import { numbered } from './settings';

/** A frame of a path, of the keys of the clip, and where the middle of the pattern was found there: `x` and `y`, `y` up. */
export type PathSample = [frame: number, place: [x: number, y: number]];

/** The frames of a path and their places, in the order of the frames: a path is written as an object of the frames, as text. */
export function pathSamples(path: FusionTrackerPattern['path']): PathSample[] {
  return Object.entries(path ?? {})
    .map(([frame, place]): PathSample => [Number(frame), place])
    .filter(([frame, place]) => Number.isFinite(frame) && Array.isArray(place) && place.length === 2)
    .sort((a, b) => a[0] - b[0]);
}

/**
 * Where the middle of a pattern is at a frame of the keys, which may lie between frames: where its
 * path has it; on the line between the two frames of its path about the frame; where the first
 * frame of the path has it before that one, and the last after it. Null where it has no path, and at
 * a frame that is no number.
 */
export function placeOnPath(path: FusionTrackerPattern['path'], frame: number): [x: number, y: number] | null {
  if (Number.isNaN(frame)) return null;
  const samples = pathSamples(path);
  if (samples.length === 0) return null;
  const [firstFrame, first] = samples[0];
  const [lastFrame, last] = samples[samples.length - 1];
  if (frame <= firstFrame) return [first[0], first[1]];
  if (frame >= lastFrame) return [last[0], last[1]];
  // The frame is after the first of the path and before the last: one is at it or before it, and one after it.
  const next = samples.findIndex(([at]) => at > frame);
  const [fromFrame, from] = samples[next - 1];
  const [toFrame, to] = samples[next];
  const share = (frame - fromFrame) / (toFrame - fromFrame);
  return [from[0] + (to[0] - from[0]) * share, from[1] + (to[1] - from[1]) * share];
}

/** The most places of a path that are drawn on the picture: a longer path is thinned to as many (`thinned`). */
export const MOST_PATH_POINTS = 256;

/**
 * At most `most` of `items`, spread evenly along them, the first and the last always among them, in
 * their order; all of them where they are no more.
 */
export function thinned<T>(items: readonly T[], most: number): T[] {
  if (items.length <= most) return [...items];
  if (most <= 1) return items.length === 0 ? [] : [items[0]];
  const step = (items.length - 1) / (most - 1);
  return Array.from({ length: most }, (_, index) => items[Math.round(index * step)]);
}

/** The patterns of a node that is a Tracker; null of a node of another kind. */
const patternsOf = (node: FusionNode): FusionTrackerPattern[] | null => (node.kind === 'tracker' ? node.params.trackers : null);
/** Whether a Tracker has a pattern at `index`. */
const hasPattern = (list: readonly FusionTrackerPattern[], index: number): boolean => Number.isInteger(index) && index >= 0 && index < list.length;
/** The list of patterns of a node set whole, with what else of the node is set in the same patch. */
const setPatterns = (at: FusionAt, node: Pick<FusionNode, 'id'>, trackers: FusionTrackerPattern[], more: Record<string, unknown> = {}): Op => ({
  op: 'set_fusion_node', ...at, id: node.id, params: { ...more, trackers },
});

/** The list with the pattern at `index` in the place of the one there: the others are the objects that they are. */
const replaced = (list: readonly FusionTrackerPattern[], index: number, pattern: FusionTrackerPattern) => list.map((each, place) => (place === index ? pattern : each));

/** A number of a pattern that is set in the Inspector: where it was put and how large it is, how large its search area is, and its offset. */
export type PatternField = 'pattern.x' | 'pattern.y' | 'pattern.width' | 'pattern.height' | 'search.width' | 'search.height' | 'offset.x' | 'offset.y';
/** The ranges of the numbers of a pattern and their values in a new one (PROTOCOL.md, "The Tracker"). */
export const PATTERN_FIELDS: Record<PatternField, { neutral: number; low: number; high: number }> = {
  'pattern.x': { neutral: 0.5, low: -15.5, high: 16.5 }, 'pattern.y': { neutral: 0.5, low: -15.5, high: 16.5 },
  'pattern.width': { neutral: 0.05, low: 0, high: 1 }, 'pattern.height': { neutral: 0.05, low: 0, high: 1 },
  'search.width': { neutral: 0.125, low: 0, high: 1 }, 'search.height': { neutral: 0.125, low: 0, high: 1 },
  'offset.x': { neutral: 0, low: -16, high: 16 }, 'offset.y': { neutral: 0, low: -16, high: 16 },
};
/** The range of a place of a Tracker: of a pattern, of its pivot and of its path, as a centre's. */
export const TRACKER_PLACE = PATTERN_FIELDS['pattern.x'];

/** A number of a pattern. */
export function patternValue(pattern: FusionTrackerPattern, field: PatternField): number {
  switch (field) {
    case 'pattern.x': return pattern.pattern.x;
    case 'pattern.y': return pattern.pattern.y;
    case 'pattern.width': return pattern.pattern.width;
    case 'pattern.height': return pattern.pattern.height;
    case 'search.width': return pattern.search.width;
    case 'search.height': return pattern.search.height;
    case 'offset.x': return pattern.offset.x;
    default: return pattern.offset.y;
  }
}

/** A pattern with a number of it changed: the rest of it, its path too, is the same object. */
function withValue(pattern: FusionTrackerPattern, field: PatternField, value: number): FusionTrackerPattern {
  switch (field) {
    case 'pattern.x': return { ...pattern, pattern: { ...pattern.pattern, x: value } };
    case 'pattern.y': return { ...pattern, pattern: { ...pattern.pattern, y: value } };
    case 'pattern.width': return { ...pattern, pattern: { ...pattern.pattern, width: value } };
    case 'pattern.height': return { ...pattern, pattern: { ...pattern.pattern, height: value } };
    case 'search.width': return { ...pattern, search: { ...pattern.search, width: value } };
    case 'search.height': return { ...pattern, search: { ...pattern.search, height: value } };
    case 'offset.x': return { ...pattern, offset: { ...pattern.offset, x: value } };
    default: return { ...pattern, offset: { ...pattern.offset, y: value } };
  }
}

/**
 * Add Tracker: a pattern as the engine makes one, in the middle of the picture, after the others.
 * Nothing where the Tracker has as many as it may have (`MOST_TRACKERS`), and for a node that is no Tracker.
 */
export function addPattern(at: FusionAt, node: FusionNode): Op[] {
  const list = patternsOf(node);
  if (list === null || list.length >= MOST_TRACKERS) return [];
  return [setPatterns(at, node, [...list, newPattern()])];
}

/**
 * Take the pattern `index` away. The patterns after it come one place nearer the start of the list,
 * by which they are named: so the pattern that the pivot names (`pivot_tracker`) is named by its new
 * place where it came after the one taken away, and so is the pattern that a centre follows, in the
 * same transaction; a centre that followed the pattern taken away lets go of the Tracker, as it
 * would follow another pattern otherwise. Nothing where the Tracker has no such pattern.
 */
export function removePattern(at: FusionAt, composition: Composition, node: FusionNode, index: number): Op[] {
  if (node.kind !== 'tracker') return [];
  const list = node.params.trackers;
  if (!hasPattern(list, index)) return [];
  const pivot = node.params.pivot_tracker;
  const more = pivot > index ? { pivot_tracker: pivot - 1 } : {};
  const followers = followersOf(composition, [node.id]).flatMap((follower): Op[] => {
    const follow = followOf(follower);
    if (follow === null || follow.output !== 'tracker' || follow.tracker < index) return [];
    return followOps(at, follower, follow.tracker === index ? null : { ...follow, tracker: follow.tracker - 1 });
  });
  return [setPatterns(at, node, list.filter((_, place) => place !== index), more), ...followers];
}

/** Rename a pattern; an empty name is no name, and the page names it by its place. Nothing where it has that name. */
export function renamePattern(at: FusionAt, node: FusionNode, index: number, name: string): Op[] {
  const list = patternsOf(node);
  if (list === null || !hasPattern(list, index) || list[index].name === name) return [];
  return [setPatterns(at, node, replaced(list, index, { ...list[index], name }))];
}

/** Turn a pattern on or off: one that is off is left out of the motion. Nothing where it is so already. */
export function enablePattern(at: FusionAt, node: FusionNode, index: number, enabled: boolean): Op[] {
  const list = patternsOf(node);
  if (list === null || !hasPattern(list, index) || list[index].enabled === enabled) return [];
  return [setPatterns(at, node, replaced(list, index, { ...list[index], enabled }))];
}

/** Set a number of a pattern, kept within its range. Nothing where it is that already. */
export function setPatternValue(at: FusionAt, node: FusionNode, index: number, field: PatternField, value: number): Op[] {
  const list = patternsOf(node);
  if (list === null || !hasPattern(list, index) || !Number.isFinite(value)) return [];
  const { low, high } = PATTERN_FIELDS[field];
  const kept = Math.min(high, Math.max(low, value));
  if (patternValue(list[index], field) === kept) return [];
  return [setPatterns(at, node, replaced(list, index, withValue(list[index], field, kept)))];
}

/**
 * Put a pattern at a place, where a drag on the picture takes it (`pattern.x` and `pattern.y`): the
 * place where it was put, and not its path, which is the result of the tracking. One operation,
 * whatever it was before, as each move of a drag sends; null where the Tracker has no such pattern.
 */
export function placedPattern(at: FusionAt, node: FusionNode, index: number, x: number, y: number): Op | null {
  const list = patternsOf(node);
  if (list === null || !hasPattern(list, index)) return null;
  const pattern = list[index];
  return setPatterns(at, node, replaced(list, index, { ...pattern, pattern: { ...pattern.pattern, x, y } }));
}

/** The patterns of a Tracker put back as they were: what a drag that is taken back sends. */
export function restoredPatterns(at: FusionAt, node: FusionNode): Op | null {
  const list = patternsOf(node);
  return list === null ? null : setPatterns(at, node, list);
}

/** Clear Path: the path of a pattern taken away, so that it is not tracked until it is tracked again. Nothing where it has none. */
export function clearPath(at: FusionAt, node: FusionNode, index: number): Op[] {
  const list = patternsOf(node);
  if (list === null || !hasPattern(list, index) || pathSamples(list[index].path).length === 0) return [];
  return [setPatterns(at, node, replaced(list, index, { ...list[index], path: {} }))];
}

/** The settings of a Tracker besides its patterns, each of which takes no keys. */
export type TrackerSettings = Partial<Omit<FusionTracker, 'trackers'>>;

/** Set settings of a Tracker by one patch: those that are another value than they are. Nothing where none is. */
export function setTracker(at: FusionAt, node: FusionNode, patch: TrackerSettings): Op[] {
  if (node.kind !== 'tracker') return [];
  const params = node.params as unknown as Record<string, unknown>;
  const changed = Object.entries(patch).filter(([name, value]) => value !== undefined && JSON.stringify(params[name]) !== JSON.stringify(value));
  return changed.length === 0 ? [] : [{ op: 'set_fusion_node', ...at, id: node.id, params: Object.fromEntries(changed) }];
}

/** The name of a pattern as the page shows it: the name that it was given, or else `unnamed` with its place in the list, from 1. */
export const patternName = (pattern: Pick<FusionTrackerPattern, 'name'> | undefined, index: number, unnamed: string): string => (pattern?.name ? pattern.name : numbered(unnamed, index));

/**
 * What a centre may follow, of the Trackers of a composition in their order (Connect To): of each, where each of its patterns is,
 * and its Steady and its Unsteady Position.
 */
export function followChoices(composition: Composition): FusionFollow[] {
  return composition.nodes.flatMap((node): FusionFollow[] => (node.kind !== 'tracker' ? [] : [
    ...node.params.trackers.map((_, tracker): FusionFollow => ({ node: node.id, output: 'tracker', tracker })),
    { node: node.id, output: 'steady_position', tracker: 0 },
    { node: node.id, output: 'unsteady_position', tracker: 0 },
  ]));
}

/** The words that name what a centre follows. */
export interface FollowWords { unnamed: string; steady: string; unsteady: string }

/** What a centre follows, as the page names it: the name of the Tracker, and the name of the pattern or of the position (`Tracker 1 › Eye`). */
export function followName(composition: Composition, names: ReadonlyMap<string, string>, follow: FusionFollow, words: FollowWords): string {
  const tracker = composition.nodes.find(each => each.id === follow.node);
  const what = follow.output === 'steady_position' ? words.steady : follow.output === 'unsteady_position' ? words.unsteady
    : patternName(tracker?.kind === 'tracker' ? tracker.params.trackers[follow.tracker] : undefined, follow.tracker, words.unnamed);
  return `${names.get(follow.node) ?? follow.node} › ${what}`;
}

/**
 * Where a centre that follows the pattern of a Tracker is at a frame of the keys: where the pattern
 * is on its path there, and its offset (the Offset Position). Null where nothing is published there
 * (the Tracker is not there or is passed through, the pattern is not there or has no path), and of
 * the Steady and the Unsteady Position, which are of the motion of all the patterns and are not
 * worked out on the page.
 */
export function followedPlace(composition: Composition, follow: FusionFollow, keyFrame: number): [x: number, y: number] | null {
  if (follow.output !== 'tracker') return null;
  const tracker = composition.nodes.find(each => each.id === follow.node);
  if (!tracker || tracker.kind !== 'tracker' || tracker.pass_through) return null;
  const pattern = tracker.params.trackers[follow.tracker];
  if (!pattern) return null;
  const place = placeOnPath(pattern.path, keyFrame);
  return place === null ? null : [place[0] + pattern.offset.x, place[1] + pattern.offset.y];
}
