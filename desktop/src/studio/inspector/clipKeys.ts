/**
 * The keys of the settings of a clip's transform and crop, as the Inspector of the Edit page sets
 * them (DaVinci Resolve Reference Manual 20, Ch. 53): a setting with no keys is one number; once it
 * has a key, a value given at another frame puts a key there, and the button beside it puts a key at
 * the playhead or takes away the one there.
 *
 * A setting is named by its path in the clip, as the engine's catalog names it: `transform.x`,
 * `crop.left`. The keys are at the frames that the keys of the clip's grade count (`keyClockOf`):
 * the pictures of its media, or its own frames from its first. Each function here gives the
 * operations to send, and none where nothing would change.
 */
import { keyClockOf } from '../color/keyFrames';
import type { Bounds } from '../color/corrector';
import type { Messages } from '../../messages/en';
import { NO_CROP, type Clip, type ClipKeyed, type Keyframe, type Media, type Op, type Param, type Timeline } from '../engine';
import { spliceOf } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';

const b = (neutral: number, low: number, high: number): Bounds => ({ neutral, low, high });

/**
 * The bounds of the settings of a clip that take keys, as the engine's catalog has them
 * (`engine/crates/model/tests/data/catalog.json`, `clip`; the tests hold the two together). Those
 * of the place and of the turn are what the editors show, 16 pictures either way and ten turns:
 * the model takes any finite number there.
 */
export const CLIP_BOUNDS: Record<ClipKeyed, Bounds> = {
  'transform.x': b(0, -16, 16),
  'transform.y': b(0, -16, 16),
  'transform.scale': b(1, 0.01, 16),
  'transform.rotation': b(0, -3600, 3600),
  'transform.opacity': b(1, 0, 1),
  'crop.left': b(0, 0, 1),
  'crop.right': b(0, 0, 1),
  'crop.top': b(0, 0, 1),
  'crop.bottom': b(0, 0, 1),
  'crop.softness': b(0, 0, 1),
};

/** The settings of a clip that take keys, in the order of the Video tab of the Inspector. */
export const CLIP_SETTINGS: readonly ClipKeyed[] = [
  'transform.scale', 'transform.x', 'transform.y', 'transform.rotation', 'transform.opacity',
  'crop.left', 'crop.right', 'crop.top', 'crop.bottom', 'crop.softness',
];

/** The name of each setting that takes keys, as the Video tab of the Inspector writes it. */
export function settingNames(labels: Messages['label']['studio']['inspector']): Record<ClipKeyed, string> {
  return {
    'transform.scale': labels.zoom, 'transform.x': labels.positionX, 'transform.y': labels.positionY, 'transform.rotation': labels.rotation,
    'transform.opacity': labels.opacity, 'crop.left': labels.cropLeft, 'crop.right': labels.cropRight, 'crop.top': labels.cropTop,
    'crop.bottom': labels.cropBottom, 'crop.softness': labels.softness,
  };
}

/**
 * Whether any setting of a clip's transform or crop has keys. The timeline asks it of every clip
 * that it draws, so it reads the ten settings where they are and makes nothing.
 */
export function hasKeys(clip: Clip): boolean {
  // A clip that a client built may say nothing of its crop (or, in a test, of its transform): it has no keys there.
  const transform: Clip['transform'] | undefined = clip.transform;
  const crop = clip.crop;
  if (transform && (keyed(transform.x) || keyed(transform.y) || keyed(transform.scale) || keyed(transform.rotation) || keyed(transform.opacity))) return true;
  return crop ? keyed(crop.left) || keyed(crop.right) || keyed(crop.top) || keyed(crop.bottom) || keyed(crop.softness) : false;
}

/** Whether a setting is keys: a number is not, nor one that is left out. */
function keyed(param: Param | undefined): boolean {
  return typeof param === 'object' && param !== null;
}

/** What holds the setting of a path, `transform` or `crop`, and its name there. */
const parts = (path: ClipKeyed) => path.split('.') as ['transform' | 'crop', string];

/** The setting of a clip at a path: a number, or keys. A clip that says nothing of its crop has none. */
export function paramOf(clip: Clip, path: ClipKeyed): Param {
  const [of, name] = parts(path);
  const holder = of === 'transform' ? clip.transform : clip.crop ?? NO_CROP;
  return (holder as unknown as Record<string, Param>)[name];
}

/** The media that a clip is cut from, where it is of media that the project has. */
export function mediaOf(clip: Clip, media: readonly Media[]): Media | undefined {
  const source = clip.source;
  return source.kind === 'media' ? media.find(item => item.id === source.media) : undefined;
}

/**
 * A frame of the timeline kept within a clip: before it, its first; after it, its last. A playhead
 * between two frames (the pages of sound keep it so) is in the earlier one, the frame that is shown.
 */
const within = (clip: Clip, frame: number) => Math.min(Math.max(Math.floor(frame), clip.start), clip.start + Math.max(clip.length, 1) - 1);

/** The frame that the keys of a clip count at a frame of the timeline, which is kept within the clip. */
export function keyFrameOf(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], frame: number): number {
  return keyClockOf(clip, timeline.rate, mediaOf(clip, media))(within(clip, frame));
}

/** What a setting of a clip is at a frame of its keys. */
export function settingAt(clip: Clip, path: ClipKeyed, keyFrame: number): number {
  return valueAt(paramOf(clip, path), keyFrame);
}

/** The key of a setting at a frame of its keys and its place among them, or null. */
export function keyAt(param: Param, keyFrame: number): { key: Keyframe; at: number } | null {
  if (typeof param === 'number') return null;
  const at = param.keys.findIndex(key => key.frame === keyFrame);
  return at < 0 ? null : { key: param.keys[at], at };
}

/** Where a key at a frame goes among keys in the order of their frames. */
const placeOf = (keys: readonly Keyframe[], keyFrame: number) => {
  const at = keys.findIndex(key => key.frame > keyFrame);
  return at < 0 ? keys.length : at;
};

/**
 * Give a setting a value at a frame of its keys. A setting with no keys is set with what holds it
 * whole, the transform or the crop, as `set_clip` takes it; one with keys has the key at the frame
 * changed, or a key put there.
 */
export function putValue(timeline: string, clip: Clip, path: ClipKeyed, value: number, keyFrame: number): Op[] {
  const param = paramOf(clip, path);
  if (typeof param === 'number') {
    if (param === value) return [];
    const [of, name] = parts(path);
    return of === 'transform'
      ? [{ op: 'set_clip', timeline, id: clip.id, transform: { ...clip.transform, [name]: value } }]
      : [{ op: 'set_clip', timeline, id: clip.id, crop: { ...(clip.crop ?? NO_CROP), [name]: value } }];
  }
  const found = keyAt(param, keyFrame);
  if (found) {
    return found.key.value === value ? [] : [{ op: 'splice_clip_keys', timeline, id: clip.id, path, at: found.at, remove: 1, keys: [{ ...found.key, value }] }];
  }
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path, at: placeOf(param.keys, keyFrame), remove: 0, keys: [{ frame: keyFrame, value, interp: 'linear' }] }];
}

/** Put a key at a frame of the keys of a setting, of the value that it has there; none where there is one. */
export function addKey(timeline: string, clip: Clip, path: ClipKeyed, keyFrame: number): Op[] {
  const param = paramOf(clip, path);
  if (keyAt(param, keyFrame)) return [];
  const keys = typeof param === 'number' ? [] : param.keys;
  const key: Keyframe = { frame: keyFrame, value: valueAt(param, keyFrame), interp: 'linear' };
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path, at: placeOf(keys, keyFrame), remove: 0, keys: [key] }];
}

/** Take away the key at a frame of the keys of a setting; its last key leaves its value, as one number. */
export function removeKey(timeline: string, clip: Clip, path: ClipKeyed, keyFrame: number): Op[] {
  const param = paramOf(clip, path);
  const found = keyAt(param, keyFrame);
  if (!found || typeof param === 'number') return [];
  if (param.keys.length === 1) return [{ op: 'set_clip_keys', timeline, id: clip.id, path, param: found.key.value }];
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path, at: found.at, remove: 1, keys: [] }];
}

/**
 * Make a setting of a clip what it is to be, a number or keys, as a curve editor leaves it: one
 * `splice_clip_keys` of the keys that differ, between those that the two begin with alike and
 * those that they end with alike; `set_clip_keys` where it is to be a number, since a setting keeps
 * one key at least; and nothing where it is that already. A setting that is a number gets its
 * first keys by a splice at 0, as the engine takes them.
 */
export function keyOps(timeline: string, clip: Clip, path: ClipKeyed, after: Param): Op[] {
  const before = paramOf(clip, path);
  if (typeof after === 'number') {
    return before === after ? [] : [{ op: 'set_clip_keys', timeline, id: clip.id, path, param: after }];
  }
  // The engine takes no setting of keys without one: its number stays.
  if (after.keys.length === 0) return [];
  const splice = spliceOf(typeof before === 'number' ? [] : before.keys, after.keys);
  return splice ? [{ op: 'splice_clip_keys', timeline, id: clip.id, path, ...splice }] : [];
}

/**
 * The frame of the timeline at which a clip shows the picture of a frame of its keys: the first
 * that has come to it, found as `keyTimes` finds the frames of keys, through speed and direction.
 * A picture before those that the clip shows is at its first frame (its last, where it plays
 * backward), and one after them at its last (its first).
 */
export function frameOfKey(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], keyFrame: number): number {
  if (clip.length <= 0) return clip.start;
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const last = clip.start + clip.length - 1;
  const rising = clock(last) >= clock(clip.start);
  // The clock goes one way along the clip: the first frame that has come to the picture.
  let from = clip.start;
  let to = last;
  while (from < to) {
    const middle = Math.floor((from + to) / 2);
    const reached = rising ? clock(middle) >= keyFrame : clock(middle) <= keyFrame;
    if (reached) to = middle;
    else from = middle + 1;
  }
  return from;
}

/**
 * The frames of the timeline at which a clip shows the pictures that the keys of a setting are at,
 * in the order of the timeline: for each key the first frame that shows its picture, or the first
 * that shows a later one where the clip plays too fast to show it. Keys of pictures that the clip
 * does not show are left out.
 */
export function keyTimes(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], path: ClipKeyed): number[] {
  const param = paramOf(clip, path);
  if (typeof param === 'number' || clip.length <= 0) return [];
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const first = clip.start;
  const last = clip.start + clip.length - 1;
  const rising = clock(last) >= clock(first);
  const low = Math.min(clock(first), clock(last));
  const high = Math.max(clock(first), clock(last));
  const times: number[] = [];
  for (const key of param.keys) {
    if (key.frame < low || key.frame > high) continue;
    // The clock goes one way along the clip: the first frame that has come to the key.
    let from = first;
    let to = last;
    while (from < to) {
      const middle = Math.floor((from + to) / 2);
      const reached = rising ? clock(middle) >= key.frame : clock(middle) <= key.frame;
      if (reached) to = middle;
      else from = middle + 1;
    }
    times.push(from);
  }
  return [...new Set(times)].sort((a, b) => a - b);
}
