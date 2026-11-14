/**
 * What the keyframe track of a clip works out apart from the page (`src/studio/timeline/keyTrack`):
 * which settings of a clip have keys, where their keys are on the timeline through the clip's speed
 * and direction, the keys chosen moved together, taken away, copied and pasted, and the room that a
 * track makes for the keyframe tracks open on it.
 */
import { describe, expect, it } from 'vitest';
import type { Clip, ClipKeyed, Interp, Keyframe } from '../src/studio/engine';
import { hasKeys, keyTimes } from '../src/studio/inspector/clipKeys';
import { copyKeys, moveKeys } from '../src/studio/keyframes/curveGeometry';
import {
  chosenKeys, copyChosen, deleteChosen, keyId, keyMarks, keyRows, keyTrackHeight, keyTrackHeights, keyedSettings, moveChosen, openTracks, pasteCopied,
} from '../src/studio/timeline/keyTrack';
import { clip, media, timeline } from './timeline-fixture';

const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const WHOLE = { left: 0, right: 0, top: 0, bottom: 0, softness: 0 };
const ANY = { low: -Infinity, high: Infinity };
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const rate = { num: 25, den: 1 };
const X: ClipKeyed = 'transform.x';
const SCALE: ClipKeyed = 'transform.scale';
const LEFT: ClipKeyed = 'crop.left';
/** c1 from frame 10 for 60 frames, showing its media from its first picture: the picture `n` at frame `10 + n`. */
const keyed = (keys: Keyframe[], fields: Partial<Clip> = {}) => clip('c1', 10, 60, { transform: { ...NONE, x: { keys } }, ...fields });
/** Keys of the place at the pictures 0, 40 and 80, of the zoom at 40, and of the left of the crop at 20. */
const several = clip('c1', 10, 60, {
  transform: { ...NONE, x: { keys: [key(0, 0), key(40, 1), key(80, 0)] }, scale: { keys: [key(40, 2)] } },
  crop: { ...WHOLE, left: { keys: [key(20, 0.5)] } },
});
const ids = (...each: [ClipKeyed, number][]) => new Set(each.map(([path, frame]) => keyId(path, frame)));

describe('the settings of a clip that have keys', () => {
  it('are known without anything made, and listed in the order of the Inspector', () => {
    expect(hasKeys(clip('c1'))).toBe(false);
    expect(hasKeys(clip('c1', 0, 100, { crop: WHOLE }))).toBe(false);
    expect(hasKeys(keyed([key(0, 0)]))).toBe(true);
    expect(hasKeys(clip('c1', 0, 100, { crop: { ...WHOLE, softness: { keys: [key(0, 0)] } } }))).toBe(true);
    expect(keyedSettings(clip('c1'))).toEqual([]);
    // The zoom comes before the place in the Video tab, and the crop after the transform.
    expect(keyedSettings(several)).toEqual([SCALE, X, LEFT]);
    expect(keyId(X, 40)).toBe('transform.x@40');
  });
});

describe('the frames at which a clip shows keys', () => {
  it('are those of every setting together, each with the settings that have a key there', () => {
    expect(keyMarks(several, { rate }, [media])).toEqual([
      { at: 10, paths: [X] }, { at: 30, paths: [LEFT] }, { at: 50, paths: [SCALE, X] },
    ]);
    const union = [...new Set([SCALE, X, LEFT].flatMap(path => keyTimes(several, { rate }, [media], path)))].sort((a, b) => a - b);
    expect(keyMarks(several, { rate }, [media]).map(mark => mark.at)).toEqual(union);
    expect(keyMarks(clip('c1', 10, 60), { rate }, [media])).toEqual([]);
  });

  it('are found through the speed and the direction of the clip', () => {
    // Twice as fast, the picture 40 comes at frame 30, and 41, which is not shown, with the one after it.
    const fast = keyed([key(0, 0), key(40, 1), key(41, 0)], { length: 30, speed: { num: 2, den: 1 } });
    expect(keyMarks(fast, { rate }, [media]).map(mark => mark.at)).toEqual(keyTimes(fast, { rate }, [media], X));
    expect(keyMarks(fast, { rate }, [media]).map(mark => mark.at)).toEqual([10, 30, 31]);
    // Backward, the first picture is shown last.
    const back = keyed([key(0, 0), key(40, 1)], { reverse: true });
    expect(keyMarks(back, { rate }, [media]).map(mark => mark.at)).toEqual([29, 69]);
  });

  it('give a row for each setting, with every key that the clip shows where it is drawn', () => {
    expect(keyRows(several, { rate }, [media])).toEqual([
      { path: SCALE, keys: [{ path: SCALE, index: 0, frame: 40, at: 50 }] },
      // The key at the picture 80 is of a picture that the clip does not show: it is there, and not drawn.
      { path: X, keys: [{ path: X, index: 0, frame: 0, at: 10 }, { path: X, index: 1, frame: 40, at: 50 }] },
      { path: LEFT, keys: [{ path: LEFT, index: 0, frame: 20, at: 30 }] },
    ]);
    // The keys of each row are where `keyTimes` finds them.
    for (const row of keyRows(several, { rate }, [media])) expect(row.keys.map(each => each.at)).toEqual(keyTimes(several, { rate }, [media], row.path));
    // A drag has the keys of a setting elsewhere for now.
    const live = new Map<ClipKeyed, Keyframe[]>([[X, [key(5, 0), key(40, 1)]]]);
    expect(keyRows(several, { rate }, [media], live)[1].keys.map(each => [each.frame, each.at])).toEqual([[5, 15], [40, 50]]);
    // Twice as fast, two keys may be drawn at one frame; each is its own key still.
    const fast = keyed([key(0, 0), key(41, 1), key(42, 0)], { length: 30, speed: { num: 2, den: 1 } });
    expect(keyRows(fast, { rate }, [media])[0].keys.map(each => [each.index, each.frame, each.at])).toEqual([[0, 0, 10], [1, 41, 31], [2, 42, 31]]);
    const back = keyed([key(0, 0), key(40, 1)], { reverse: true });
    expect(keyRows(back, { rate }, [media])[0].keys.map(each => [each.frame, each.at])).toEqual([[0, 69], [40, 29]]);
  });
});

describe('the keys chosen in a keyframe track', () => {
  const three = [key(0, 0), key(20, 0.25), key(40, 0)];
  const both = clip('c1', 10, 60, { transform: { ...NONE, x: { keys: three }, scale: { keys: [key(20, 2), key(30, 1)] } } });

  it('are found among the keys of the clip, setting by setting, and not where they are gone', () => {
    expect(chosenKeys(both, ids([X, 20], [SCALE, 30], [X, 99], [LEFT, 20]))).toEqual([
      { path: SCALE, keys: [key(20, 2), key(30, 1)], indices: [1] },
      { path: X, keys: three, indices: [1] },
    ]);
    expect(chosenKeys(both, new Set())).toEqual([]);
  });

  it('move in time by whole frames, no key past one beside it that is not moved', () => {
    const one = chosenKeys(keyed(three), ids([X, 20]));
    expect(moveChosen(one, 5)).toEqual({ shift: 5, settings: [{ path: X, keys: moveKeys(three, [1], 5, 0, ANY) }] });
    expect(moveChosen(one, 5).settings[0].keys).toEqual([key(0, 0), key(25, 0.25), key(40, 0)]);
    expect(moveChosen(one, 4.6).shift).toBe(5);
    // It stops a frame short of the key after it, and of the key before it.
    expect(moveChosen(one, 100).settings[0].keys.map(each => each.frame)).toEqual([0, 39, 40]);
    expect(moveChosen(one, -100).settings[0].keys.map(each => each.frame)).toEqual([0, 1, 40]);
    expect(moveChosen(one, 0)).toEqual({ shift: 0, settings: [{ path: X, keys: three }] });
    // Two keys that are both chosen go together past where one alone would stop.
    const pair = chosenKeys(keyed(three), ids([X, 20], [X, 40]));
    expect(moveChosen(pair, 30).settings[0].keys.map(each => each.frame)).toEqual([0, 50, 70]);
  });

  it('move the keys of several settings by the same frames: as far as all of them have room for', () => {
    // The zoom has a key at 30 that is not chosen: its key at 20 goes to 29 at most, and that of the place with it.
    const together = chosenKeys(both, ids([X, 20], [SCALE, 20]));
    const moved = moveChosen(together, 15);
    expect(moved.shift).toBe(9);
    expect(moved.settings).toEqual([
      { path: SCALE, keys: [key(29, 2), key(30, 1)] },
      { path: X, keys: [key(0, 0), key(29, 0.25), key(40, 0)] },
    ]);
    // With both keys of the zoom chosen, the place is what holds them: its key stops before 40.
    expect(moveChosen(chosenKeys(both, ids([X, 20], [SCALE, 20], [SCALE, 30])), 25).shift).toBe(19);
    expect(moveChosen(together, -30).shift).toBe(-19);
  });

  it('are taken away, and leave the number that a setting had where its last key goes', () => {
    expect(deleteChosen(keyed(three), ids([X, 20]), 20)).toEqual([{ path: X, param: { keys: [key(0, 0), key(40, 0)] } }]);
    expect(deleteChosen(keyed([key(20, 0.3)]), ids([X, 20]), 0)).toEqual([{ path: X, param: 0.3 }]);
    // Every key of a setting: it is what it was at the frame given, the playhead.
    expect(deleteChosen(keyed([key(0, 0), key(40, 1)]), ids([X, 0], [X, 40]), 20)).toEqual([{ path: X, param: 0.5 }]);
    expect(deleteChosen(both, ids([X, 20], [SCALE, 20]), 0)).toEqual([
      { path: SCALE, param: { keys: [key(30, 1)] } },
      { path: X, param: { keys: [key(0, 0), key(40, 0)] } },
    ]);
    expect(deleteChosen(keyed(three), ids([X, 21]), 20)).toEqual([]);
  });

  it('are copied as far apart as they are, across settings, and pasted so with the first at a frame', () => {
    const copied = copyChosen(both, ids([X, 0], [X, 20], [SCALE, 30]));
    expect(copied).toEqual([
      { path: SCALE, lead: 30, keys: copyKeys([key(20, 2), key(30, 1)], [1]) },
      { path: X, lead: 0, keys: copyKeys(three, [0, 1]) },
    ]);
    expect(copyChosen(both, new Set())).toEqual([]);
    // Into a clip without keys: each setting gets its first keys, the zoom 30 frames after the first of the place.
    const plain = clip('c2', 100, 50);
    expect(pasteCopied(plain, copied, 7)).toEqual({
      changes: [{ path: SCALE, param: { keys: [key(37, 1)] } }, { path: X, param: { keys: [key(7, 0), key(27, 0.25)] } }],
      chosen: ['transform.scale@37', 'transform.x@7', 'transform.x@27'],
    });
    // Among keys that are there: a key at a frame pasted on is replaced, the others stay.
    const made = pasteCopied(both, copyChosen(both, ids([X, 20])), 40);
    expect(made).toEqual({ changes: [{ path: X, param: { keys: [key(0, 0), key(20, 0.25), key(40, 0.25)] } }], chosen: ['transform.x@40'] });
    // Pasted where they are, nothing changes; they are chosen all the same.
    expect(pasteCopied(both, copyChosen(both, ids([X, 20])), 20)).toEqual({ changes: [], chosen: ['transform.x@20'] });
  });
});

describe('the keyframe tracks open on a timeline', () => {
  const other = clip('c2', 100, 50);
  const model = timeline([several, other]);

  it('are given room by their track: that of the highest, and none where none is open', () => {
    expect(keyTrackHeight(3)).toBe(66);
    expect(keyTrackHeights(model, ['c1'])).toEqual(new Map([['v1', keyTrackHeight(3)]]));
    const second = clip('c3', 200, 50, { transform: { ...NONE, opacity: { keys: [key(0, 1)] } } });
    expect(keyTrackHeights(timeline([several, other, second]), ['c3', 'c1'])).toEqual(new Map([['v1', keyTrackHeight(3)]]));
    expect(keyTrackHeights(timeline([other, second]), ['c3'])).toEqual(new Map([['v1', keyTrackHeight(1)]]));
    // A clip without keys, one that is gone, no clip and no timeline: the same empty map each time.
    const none = keyTrackHeights(model, []);
    expect(none.size).toBe(0);
    expect(keyTrackHeights(model, ['c2', 'gone'])).toBe(none);
    expect(keyTrackHeights(model, undefined)).toBe(none);
    expect(keyTrackHeights(null, ['c1'])).toBe(none);
  });

  it('stay open while their clips are there and have keys', () => {
    const open = ['c1'];
    expect(openTracks(model, open)).toBe(open);
    expect(openTracks(model, ['c1', 'c2', 'gone'])).toEqual(['c1']);
    expect(openTracks(timeline([clip('c1', 10, 60), other]), open)).toEqual([]);
    expect(openTracks(null, open)).toEqual([]);
    const none: string[] = [];
    expect(openTracks(null, none)).toBe(none);
  });
});
