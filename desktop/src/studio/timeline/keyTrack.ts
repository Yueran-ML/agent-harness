/**
 * The keyframe track of a clip on the timeline of the Edit page, as DaVinci Resolve shows the keys
 * of a clip beneath it (Reference Manual 20, Ch. 53, pp. 1108–1114): where the keys of the settings
 * of a clip's transform and crop are on the timeline, which of them are chosen, and what an edit of
 * the track leaves of them. Pure: nothing here changes what it is given.
 *
 * A key is at a frame of the keys (`Keyframe.frame`: a picture of the clip's media, or a frame of
 * the clip itself), and is shown at the frame of the timeline at which the clip shows that picture
 * (`keyTimes`, `frameOfKey`), through the clip's speed and direction. A key of a picture that the
 * clip does not show is still there and is not drawn (p. 1114). The arithmetic of the keys is that of
 * the Curve Editor (`../keyframes/curveGeometry`): only what several settings do together is here.
 *
 * A key that is chosen is known by its setting and its frame (`keyId`), so it stays chosen where
 * other keys come and go, and one that is gone is chosen no longer.
 */
import { keyClockOf } from '../color/keyFrames';
import type { Clip, ClipKeyed, Keyframe, Media, Param, Timeline } from '../engine';
import { CLIP_SETTINGS, frameOfKey, hasKeys, keyTimes, mediaOf, paramOf } from '../inspector/clipKeys';
import { copyKeys, deleteKeys, moveKeys, pasteKeys, spliceOf, type CopiedKey, type Range } from '../keyframes/curveGeometry';
import { clipLocations } from './selection';

/** How high a row of a keyframe track is, in pixels. */
export const KEY_ROW = 16;
/** How high the keyframe track of a clip is with so many settings keyed: a row for each and the group row above them, within its border. */
export const keyTrackHeight = (settings: number): number => (settings + 1) * KEY_ROW + 2;
/** How many pixels above the room that a track makes for its keyframe tracks one begins: just under its clip. */
export const KEY_TRACK_RISE = 3;

/** A move in time changes no value, and a paste puts the values that were copied: neither is held to the bounds of an editor. */
const ANY: Range = { low: -Infinity, high: Infinity };

/** The settings of a clip that have keys, in the order of the Video tab of the Inspector. */
export function keyedSettings(clip: Clip): ClipKeyed[] {
  return CLIP_SETTINGS.filter(path => typeof paramOf(clip, path) === 'object');
}

/** A frame of the timeline at which a clip shows keys, and the settings that have a key there. */
export interface KeyMark { at: number; paths: ClipKeyed[] }

/**
 * The frames of the timeline at which a clip shows a key of any of its settings, in the order of
 * the timeline, each with the settings that have a key there: what DaVinci Resolve's group track
 * shows (p. 1112), and the diamonds along the lower edge of the clip.
 */
export function keyMarks(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[]): KeyMark[] {
  const marks = new Map<number, ClipKeyed[]>();
  for (const path of CLIP_SETTINGS) {
    for (const at of keyTimes(clip, timeline, media, path)) {
      const paths = marks.get(at);
      if (paths) paths.push(path);
      else marks.set(at, [path]);
    }
  }
  return [...marks].sort((a, b) => a[0] - b[0]).map(([at, paths]) => ({ at, paths }));
}

/** A key of a setting as a keyframe track shows it: its place among the keys, its frame, and the frame of the timeline that it is drawn at. */
export interface TrackKey { path: ClipKeyed; index: number; frame: number; at: number }
/** The row of one setting in a keyframe track: its keys that the clip shows. */
export interface KeyRow { path: ClipKeyed; keys: TrackKey[] }

/**
 * The rows of the keyframe track of a clip, one for each setting that has keys, in the order of the
 * Inspector. `live` gives the keys of settings as a drag has them for now, in place of the clip's.
 * Two keys may be drawn at one frame of the timeline where the clip plays faster than its media.
 */
export function keyRows(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], live?: ReadonlyMap<ClipKeyed, readonly Keyframe[]> | null): KeyRow[] {
  const settings = keyedSettings(clip);
  if (clip.length <= 0) return settings.map(path => ({ path, keys: [] }));
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const first = clock(clip.start);
  const last = clock(clip.start + clip.length - 1);
  const low = Math.min(first, last);
  const high = Math.max(first, last);
  return settings.map((path) => {
    const param = paramOf(clip, path);
    const keys = live?.get(path) ?? (typeof param === 'number' ? [] : param.keys);
    return {
      path,
      keys: keys.flatMap((key, index) => (key.frame < low || key.frame > high ? []
        : [{ path, index, frame: key.frame, at: frameOfKey(clip, timeline, media, key.frame) }])),
    };
  });
}

/** What a key that is chosen is known by: its setting and its frame. */
export const keyId = (path: ClipKeyed, frame: number): string => `${path}@${frame}`;

/** A setting of which keys are chosen: its keys, and the places of those chosen, in the order of the keys. */
export interface ChosenKeys { path: ClipKeyed; keys: Keyframe[]; indices: number[] }

/** The settings of a clip of which keys are chosen. A key that the clip no longer has is not among them. */
export function chosenKeys(clip: Clip, chosen: ReadonlySet<string>): ChosenKeys[] {
  if (chosen.size === 0) return [];
  return keyedSettings(clip).flatMap((path) => {
    const param = paramOf(clip, path);
    const keys = typeof param === 'number' ? [] : param.keys;
    const indices = keys.flatMap((key, index) => (chosen.has(keyId(path, key.frame)) ? [index] : []));
    return indices.length > 0 ? [{ path, keys: keys.map(key => ({ ...key })), indices }] : [];
  });
}

/** A setting as an edit of the keyframe track leaves it. */
export interface SettingChange { path: ClipKeyed; param: Param }

/**
 * The keys chosen moved by frames of the keys (p. 1110): all of them by the same whole number, the
 * most that every setting lets its own go by (`moveKeys`: a key stays strictly between the keys
 * beside it that are not moved), so that keys of several settings at one frame stay at one frame
 * (p. 1112). Gives how far they went, and the keys of each setting.
 */
export function moveChosen(settings: readonly ChosenKeys[], frames: number): { shift: number; settings: { path: ClipKeyed; keys: Keyframe[] }[] } {
  let shift = Number.isFinite(frames) ? Math.round(frames) + 0 : 0;
  // Each setting holds the move to what its own keys have room for; a move held by one is within the room of those before it.
  for (const { keys, indices } of settings) {
    const first = indices[0];
    const went = moveKeys(keys, indices, shift, 0, ANY)[first].frame - keys[first].frame;
    if (Math.abs(went) < Math.abs(shift)) shift = went + 0;
  }
  return { shift, settings: settings.map(({ path, keys, indices }) => ({ path, keys: moveKeys(keys, indices, shift, 0, ANY) })) };
}

/**
 * The settings of a clip with the keys chosen taken away (p. 1114). A setting that loses its last
 * key is the number that it had at `at`, a frame of the keys (the playhead), as `deleteKeys` has it.
 */
export function deleteChosen(clip: Clip, chosen: ReadonlySet<string>, at: number): SettingChange[] {
  return chosenKeys(clip, chosen).map(({ path, indices }) => ({ path, param: deleteKeys(paramOf(clip, path), indices, at) }));
}

/** The keys of one setting as they were copied, and how many frames of the keys its first is after the first of all those copied with it. */
export interface CopiedSetting { path: ClipKeyed; lead: number; keys: CopiedKey[] }
/**
 * The keys copied in a keyframe track, kept by the timeline for all the tracks open on it. It is not
 * the clipboard of the Curve Editor, which holds the keys of one curve and says nothing of which.
 */
export interface KeyTrackClipboard { settings: CopiedSetting[] }

/** The keys chosen as they are copied (p. 1114): those of each setting, as far apart as they are, each setting as far after the first key of all as it is. */
export function copyChosen(clip: Clip, chosen: ReadonlySet<string>): CopiedSetting[] {
  const found = chosenKeys(clip, chosen);
  if (found.length === 0) return [];
  const first = Math.min(...found.map(({ keys, indices }) => keys[indices[0]].frame));
  return found.map(({ path, keys, indices }) => ({ path, lead: keys[indices[0]].frame - first, keys: copyKeys(keys, indices) }));
}

/**
 * Copied keys pasted into a clip so that the first of them all is at `at`, a frame of the keys, and
 * the others as far after it as they were (p. 1114): each into the setting that it was copied from,
 * which gets its first keys so where it had none. Gives the settings that change, and the keys
 * pasted, to be chosen.
 */
export function pasteCopied(clip: Clip, copied: readonly CopiedSetting[], at: number): { changes: SettingChange[]; chosen: string[] } {
  const changes: SettingChange[] = [];
  const chosen: string[] = [];
  for (const { path, lead, keys } of copied) {
    const param = paramOf(clip, path);
    const made = pasteKeys(param, keys, at + lead, ANY);
    if (!made) continue;
    chosen.push(...made.frames.map(frame => keyId(path, frame)));
    if (typeof param === 'number' || spliceOf(param.keys, made.keys) !== null) changes.push({ path, param: { keys: made.keys } });
  }
  return { changes, chosen };
}

/** Whether a clip may have a keyframe track open: it is there, on a track of picture, and has keys. */
const tracked = (timeline: Timeline, id: string): Clip | null => {
  const found = clipLocations(timeline).get(id);
  return found && found.track.kind === 'video' && hasKeys(found.clip) ? found.clip : null;
};

const NO_HEIGHTS: ReadonlyMap<string, number> = new Map();

/**
 * The room that each track makes beneath its clips for the keyframe tracks open on it, in pixels,
 * by the id of the track: that of its highest one. The same empty map where none is open, so that
 * a timeline without keyframe tracks is laid out as it always was.
 */
export function keyTrackHeights(timeline: Timeline | null, open: readonly string[] | undefined): ReadonlyMap<string, number> {
  if (!timeline || !open || open.length === 0) return NO_HEIGHTS;
  let heights: Map<string, number> | null = null;
  for (const id of open) {
    const clip = tracked(timeline, id);
    if (!clip) continue;
    const track = clipLocations(timeline).get(id)!.track.id;
    heights ??= new Map();
    heights.set(track, Math.max(heights.get(track) ?? 0, keyTrackHeight(keyedSettings(clip).length)));
  }
  return heights ?? NO_HEIGHTS;
}

/**
 * The clips whose keyframe tracks stay open on a timeline: a clip that is gone, or that has lost
 * its last key, has none. The list itself where all of them stay.
 */
export function openTracks(timeline: Timeline | null, open: readonly string[]): readonly string[] {
  if (open.length === 0) return open;
  const kept = timeline ? open.filter(id => tracked(timeline, id) !== null) : [];
  return kept.length === open.length ? open : kept;
}
