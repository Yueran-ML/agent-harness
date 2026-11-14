/**
 * The keys of the settings of a clip's transform, as the Inspector of the Edit page sets them
 * (DaVinci Resolve Reference Manual 20, Ch. 53): a setting with no keys is one number; once it has a
 * key, a value given at another frame puts a key there, and the button beside it puts a key at the
 * playhead or takes away the one there.
 *
 * The keys are at the frames that the keys of the clip's grade count (`keyClockOf`): the pictures of
 * its media, or its own frames from its first. Each function here gives the operations to send, and
 * none where nothing would change.
 */
import { keyClockOf } from '../color/keyFrames';
import type { Clip, Keyframe, Media, Op, Param, Timeline, Transform } from '../engine';
import { valueAt } from '../keyframes/value';

export type TransformSetting = keyof Transform;

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

/** What a setting of a clip's transform is at a frame of its keys. */
export function settingAt(clip: Clip, name: TransformSetting, keyFrame: number): number {
  return valueAt(clip.transform[name], keyFrame);
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

const path = (name: TransformSetting) => `transform.${name}`;

/**
 * Give a setting a value at a frame of its keys. A setting with no keys is set with the transform
 * whole, as `set_clip` takes it; one with keys has the key at the frame changed, or a key put there.
 */
export function putValue(timeline: string, clip: Clip, name: TransformSetting, value: number, keyFrame: number): Op[] {
  const param = clip.transform[name];
  if (typeof param === 'number') {
    return param === value ? [] : [{ op: 'set_clip', timeline, id: clip.id, transform: { ...clip.transform, [name]: value } }];
  }
  const found = keyAt(param, keyFrame);
  if (found) {
    return found.key.value === value ? [] : [{ op: 'splice_clip_keys', timeline, id: clip.id, path: path(name), at: found.at, remove: 1, keys: [{ ...found.key, value }] }];
  }
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path: path(name), at: placeOf(param.keys, keyFrame), remove: 0, keys: [{ frame: keyFrame, value, interp: 'linear' }] }];
}

/** Put a key at a frame of the keys of a setting, of the value that it has there; none where there is one. */
export function addKey(timeline: string, clip: Clip, name: TransformSetting, keyFrame: number): Op[] {
  const param = clip.transform[name];
  if (keyAt(param, keyFrame)) return [];
  const keys = typeof param === 'number' ? [] : param.keys;
  const key: Keyframe = { frame: keyFrame, value: valueAt(param, keyFrame), interp: 'linear' };
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path: path(name), at: placeOf(keys, keyFrame), remove: 0, keys: [key] }];
}

/** Take away the key at a frame of the keys of a setting; its last key leaves its value, as one number. */
export function removeKey(timeline: string, clip: Clip, name: TransformSetting, keyFrame: number): Op[] {
  const param = clip.transform[name];
  const found = keyAt(param, keyFrame);
  if (!found || typeof param === 'number') return [];
  if (param.keys.length === 1) return [{ op: 'set_clip_keys', timeline, id: clip.id, path: path(name), param: found.key.value }];
  return [{ op: 'splice_clip_keys', timeline, id: clip.id, path: path(name), at: found.at, remove: 1, keys: [] }];
}

/**
 * The frames of the timeline at which a clip shows the pictures that the keys of a setting are at,
 * in the order of the timeline: for each key the first frame that shows its picture, or the first
 * that shows a later one where the clip plays too fast to show it. Keys of pictures that the clip
 * does not show are left out.
 */
export function keyTimes(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], name: TransformSetting): number[] {
  const param = clip.transform[name];
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
