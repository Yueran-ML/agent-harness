/**
 * The keys of the settings of a clip's transform and crop, and of the style of a title, as the
 * Inspector of the Edit page sets them (DaVinci Resolve Reference Manual 20, Ch. 53): a setting
 * with no keys is one number; once it has a key, a value given at another frame puts a key there,
 * and the button beside it puts a key at the playhead or takes away the one there.
 *
 * A setting is named by its path in the clip, as the engine's catalog names it: `transform.x`,
 * `crop.left`, `title.size`. The keys are at the frames that the keys of the clip's grade count
 * (`keyClockOf`): the pictures of its media, or its own frames from its first, as those of a title
 * are. Each function here gives the operations to send, and none where nothing would change.
 *
 * The settings of `title.` are of a title only (`settingsOf`): the engine refuses them on any other
 * clip, so nothing here sends one for it, and such a clip is read as having none.
 */
import { firstShowing, keyClockOf, keyRangeOf, keyRunsOf } from '../color/keyFrames';
import type { Bounds } from '../color/corrector';
import type { Messages } from '../../messages/en';
import { NO_CROP, titleSetting, type Clip, type ClipKeyed, type Keyframe, type Media, type Op, type Param, type Timeline } from '../engine';
import { spliceOf } from '../keyframes/curveGeometry';
import { valueAt } from '../keyframes/value';

const b = (neutral: number, low: number, high: number): Bounds => ({ neutral, low, high });

/**
 * The bounds of the settings of a clip that take keys, as the engine's catalog has them
 * (`engine/crates/model/tests/data/catalog.json`, `clip`; the tests hold the two together). Those
 * of the place and of the turn are what the editors show, 16 pictures either way and ten turns:
 * the model takes any finite number there, as it does for the place of the anchor of a title.
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
  'title.size': b(72, 1, 4096),
  'title.x': b(0.5, -16, 16),
  'title.y': b(0.5, -16, 16),
};

/**
 * The settings of a clip that take keys, in the order of the Inspector: those of its Video tab,
 * and after them those of the Title tab, which only a title has (`settingsOf`).
 */
export const CLIP_SETTINGS: readonly ClipKeyed[] = [
  'transform.scale', 'transform.x', 'transform.y', 'transform.rotation', 'transform.opacity',
  'crop.left', 'crop.right', 'crop.top', 'crop.bottom', 'crop.softness',
  'title.size', 'title.x', 'title.y',
];
/** The settings that every clip of picture has: those of `CLIP_SETTINGS` that are not of the style of a title. */
const PICTURE_SETTINGS: readonly ClipKeyed[] = CLIP_SETTINGS.filter(path => titleSetting(path) === null);

/** The settings of a clip that take keys, in the order of the Inspector: those of the style of a title for a title only. */
export function settingsOf(clip: Clip): readonly ClipKeyed[] {
  return clip.source.kind === 'title' ? CLIP_SETTINGS : PICTURE_SETTINGS;
}

/**
 * The name of each setting that takes keys, as the Inspector writes it. Those of a title are named
 * apart from those of the transform, since a keyframe track and the Curve Editor list the place of
 * the text beside the place of the picture.
 */
export function settingNames(labels: Messages['label']['studio']['inspector']): Record<ClipKeyed, string> {
  return {
    'transform.scale': labels.zoom, 'transform.x': labels.positionX, 'transform.y': labels.positionY, 'transform.rotation': labels.rotation,
    'transform.opacity': labels.opacity, 'crop.left': labels.cropLeft, 'crop.right': labels.cropRight, 'crop.top': labels.cropTop,
    'crop.bottom': labels.cropBottom, 'crop.softness': labels.softness,
    'title.size': labels.titleSize, 'title.x': labels.titleX, 'title.y': labels.titleY,
  };
}

/**
 * Whether any setting of a clip's transform or crop, or of the style of a title, has keys. The
 * timeline asks it of every clip that it draws, so it reads the settings where they are and makes
 * nothing.
 */
export function hasKeys(clip: Clip): boolean {
  // A clip that a client built may say nothing of its crop (or, in a test, of its transform): it has no keys there.
  const transform: Clip['transform'] | undefined = clip.transform;
  const crop = clip.crop;
  if (transform && (keyed(transform.x) || keyed(transform.y) || keyed(transform.scale) || keyed(transform.rotation) || keyed(transform.opacity))) return true;
  if (crop && (keyed(crop.left) || keyed(crop.right) || keyed(crop.top) || keyed(crop.bottom) || keyed(crop.softness))) return true;
  const source = clip.source;
  return source.kind === 'title' && (keyed(source.style.size) || keyed(source.style.x) || keyed(source.style.y));
}

/** Whether a setting is keys: a number is not, nor one that is left out. */
function keyed(param: Param | undefined): boolean {
  return typeof param === 'object' && param !== null;
}

/** What holds the setting of a path, `transform` or `crop`, and its name there. */
const parts = (path: ClipKeyed) => path.split('.') as ['transform' | 'crop', string];

/**
 * The setting of a clip at a path: a number, or keys. A clip that says nothing of its crop has
 * none. A clip that is no title has no setting of the style of one: it is read as the number that
 * a new title has there, which has no keys, so that nothing takes the clip for keyed.
 */
export function paramOf(clip: Clip, path: ClipKeyed): Param {
  const styled = titleSetting(path);
  if (styled) return clip.source.kind === 'title' ? clip.source.style[styled] : CLIP_BOUNDS[path].neutral;
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

/** Whether a clip has the setting of a path: every clip those of the transform and of the crop, a title alone those of its style. */
export const hasSetting = (clip: Clip, path: ClipKeyed): boolean => titleSetting(path) === null || clip.source.kind === 'title';

/**
 * Give a setting a value at a frame of its keys. A setting with no keys is set with what holds it
 * whole, the transform, the crop or the source of a title with its style, as `set_clip` takes it;
 * one with keys has the key at the frame changed, or a key put there. A clip that has no such
 * setting is sent nothing.
 */
export function putValue(timeline: string, clip: Clip, path: ClipKeyed, value: number, keyFrame: number): Op[] {
  if (!hasSetting(clip, path)) return [];
  const param = paramOf(clip, path);
  if (typeof param === 'number') {
    if (param === value) return [];
    const styled = titleSetting(path);
    // The other settings of the style go along as they are, keys and all.
    if (styled) return clip.source.kind === 'title' ? [{ op: 'set_clip', timeline, id: clip.id, source: { ...clip.source, style: { ...clip.source.style, [styled]: value } } }] : [];
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
  if (!hasSetting(clip, path) || keyAt(param, keyFrame)) return [];
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
  if (!hasSetting(clip, path)) return [];
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
 *
 * A clip with speed points may show a picture at several frames, where it freezes or turns back
 * over it: this is the first of them (`firstShowing`). A picture below those that it shows is at
 * the first frame that shows its lowest, and one above them at the first that shows its highest.
 */
export function frameOfKey(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], keyFrame: number): number {
  if (clip.length <= 0) return clip.start;
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const runs = keyRunsOf(clip);
  if (runs) {
    const [low, high] = keyRangeOf(clock, runs);
    return firstShowing(clock, runs, Math.min(Math.max(keyFrame, low), high)) ?? clip.start;
  }
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
 * does not show are left out. A clip with speed points shows a key where it first comes by its
 * picture (`firstShowing`): once, though it may hold the picture or come back to it.
 */
export function keyTimes(clip: Clip, timeline: Pick<Timeline, 'rate'>, media: readonly Media[], path: ClipKeyed): number[] {
  const param = paramOf(clip, path);
  if (typeof param === 'number' || clip.length <= 0) return [];
  const clock = keyClockOf(clip, timeline.rate, mediaOf(clip, media));
  const runs = keyRunsOf(clip);
  if (runs) {
    const found = param.keys.flatMap((key) => { const at = firstShowing(clock, runs, key.frame); return at === null ? [] : [at]; });
    return [...new Set(found)].sort((a, b) => a - b);
  }
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
