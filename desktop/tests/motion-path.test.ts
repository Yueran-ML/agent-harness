/**
 * The motion path of a clip (`src/studio/motionPath/motionPath`; DaVinci Resolve Reference Manual 20,
 * pp. 1106–1108): where its points are for keys that go on a line, that hold and that are eased, for a
 * clip with speed points that shows a picture twice, and for a title, whose keys count its own
 * frames; what the engine is asked to draw; which point a press finds; what a drag of a point sets,
 * with the key that the other setting gets; and Linear and Smooth.
 *
 * The timeline is 1920 by 1080 at 25 frames a second, and the media counts a tick a frame, so a place
 * of the media reads as a frame. The anchor of a clip that is not moved is at (960, 540).
 */
import { describe, expect, it } from 'vitest';
import { keyClockOf } from '../src/studio/color/keyFrames';
import { emptyGrade, type Clip, type Interp, type Keyframe, type Param, type Retime, type Timeline } from '../src/studio/engine';
import { EASE_SHARE, valueAt } from '../src/studio/keyframes/value';
import {
  MOST_TICKS, anchorAt, anchorAtPlayhead, hasMotion, hitKey, interpolated, motionPathOf, movedPoint, overlayOf, placeOf, pointInterp,
} from '../src/studio/motionPath/motionPath';
import { clip, media } from './timeline-fixture';

const timeline: Timeline = {
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(), tracks: [],
};
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const keys = (...given: Keyframe[]): Param => ({ keys: given });
const moved = (x: Param, y: Param) => ({ x, y, scale: 1, rotation: 0, opacity: 1 });
/** c1 from frame 10 of the timeline, cut from its media 50 frames in: at frame 10 it shows picture 50. */
const cut = (length: number, x: Param, y: Param, fields: Partial<Clip> = {}) =>
  clip('c1', 10, length, { source: { kind: 'media', media: 'm1', in: { ticks: 50, scale: 25 } }, transform: moved(x, y), ...fields });
/** Five frames that go from the middle to the right edge, a quarter of the height below the middle. */
const ACROSS = cut(5, keys(key(50, 0), key(54, 0.5)), 0.25);
const ACROSS_TICKS = [[960, 810], [1200, 810], [1440, 810], [1680, 810], [1920, 810]];

describe('the motion path of a clip', () => {
  it('is of a clip whose position has keys, and of no other', () => {
    expect(hasMotion(ACROSS)).toBe(true);
    expect(hasMotion(cut(5, 0.25, keys(key(50, 0))))).toBe(true);
    expect(hasMotion(cut(5, 0.25, 0.5))).toBe(false);
    expect(motionPathOf(cut(5, 0.25, 0.5), timeline, [media])).toBeNull();
    // Keys of its zoom or of its turn move no anchor.
    expect(motionPathOf(clip('c1', 10, 5, { transform: { x: 0, y: 0, scale: keys(key(0, 1), key(4, 2)), rotation: keys(key(0, 0), key(4, 90)), opacity: 1 } }), timeline, [media])).toBeNull();
    expect(motionPathOf(cut(0, keys(key(50, 0)), 0), timeline, [media])).toBeNull();
  });

  it('puts the anchor at the middle of the timeline moved by shares of its width and its height', () => {
    expect(placeOf(timeline, 0, 0)).toEqual([960, 540]);
    expect(placeOf(timeline, 0.5, -0.5)).toEqual([1920, 0]);
    expect(placeOf(timeline, -0.25, 0.125)).toEqual([480, 675]);
    // The keys of a clip of media count the pictures of its media: picture 52 is the third frame of the clip.
    expect(anchorAt(ACROSS, timeline, 52)).toEqual([1440, 810]);
  });

  it('has a place at each frame along keys that go on a line, and a point at each key', () => {
    expect(motionPathOf(ACROSS, timeline, [media])).toEqual({
      canvas: [1920, 1080],
      ticks: ACROSS_TICKS,
      keys: [{ frame: 50, at: [960, 810], x: true, y: false }, { frame: 54, at: [1920, 810], x: true, y: false }],
    });
  });

  it('makes one point of a key of each setting at one frame, and a point of a key that one setting alone has', () => {
    const both = cut(5, keys(key(50, 0), key(52, 0.25), key(54, 0.5)), keys(key(50, 0), key(54, 0.5)));
    const path = motionPathOf(both, timeline, [media])!;
    expect(path.keys).toEqual([
      { frame: 50, at: [960, 540], x: true, y: true },
      { frame: 52, at: [1440, 810], x: true, y: false },
      { frame: 54, at: [1920, 1080], x: true, y: true },
    ]);
    expect(path.ticks).toEqual([[960, 540], [1200, 675], [1440, 810], [1680, 945], [1920, 1080]]);
  });

  it('leaves out the points of keys that the clip does not show, which still shape the line', () => {
    const trimmed = cut(5, keys(key(40, -1.25), key(50, 0), key(54, 0.5), key(70, 2.5)), 0.25);
    const path = motionPathOf(trimmed, timeline, [media])!;
    expect(path.keys.map(each => each.frame)).toEqual([50, 54]);
    expect(path.ticks).toEqual(ACROSS_TICKS);
    // Cut two pictures later, it shows the key at 54 alone, and goes on towards the one at 70 that it does not reach.
    const later = motionPathOf({ ...trimmed, source: { kind: 'media', media: 'm1', in: { ticks: 52, scale: 25 } } }, timeline, [media])!;
    expect(later.keys.map(each => each.frame)).toEqual([54]);
    expect(later.ticks).toEqual([[1440, 810], [1680, 810], [1920, 810], [2160, 810], [2400, 810]]);
  });

  it('stays at one place while a key holds, which is there once', () => {
    const held = cut(5, keys(key(50, 0, 'hold'), key(52, 0.5), key(54, 0.5)), 0);
    expect(motionPathOf(held, timeline, [media])).toEqual({
      canvas: [1920, 1080],
      ticks: [[960, 540], [1920, 540]],
      keys: [{ frame: 50, at: [960, 540], x: true, y: false }, { frame: 52, at: [1920, 540], x: true, y: false }, { frame: 54, at: [1920, 540], x: true, y: false }],
    });
  });

  it('spaces its places as an eased stretch goes: close where it is slow, far where it is fast', () => {
    const eased: Interp = { bezier: { out: [EASE_SHARE, 0], in: [EASE_SHARE, 0] } };
    const x = keys(key(50, 0, eased), key(58, 0.5));
    const path = motionPathOf(cut(9, x, 0), timeline, [media])!;
    // Each place is the setting as the engine reads it at that picture (`valueAt`, held to the engine's values).
    expect(path.ticks).toEqual([50, 51, 52, 53, 54, 55, 56, 57, 58].map(frame => [960 + 1920 * valueAt(x, frame), 540]));
    const steps = path.ticks.slice(1).map((at, index) => at[0] - path.ticks[index][0]);
    expect(steps).toHaveLength(8);
    // Slow away from the first key, fastest half way, slow into the last; and the same going in as coming out.
    expect(steps[0]).toBeLessThan(steps[1]);
    expect(steps[1]).toBeLessThan(steps[2]);
    expect(steps[2]).toBeLessThan(steps[3]);
    expect(steps[4]).toBeGreaterThan(steps[5]);
    expect(steps[6]).toBeGreaterThan(steps[7]);
    expect(steps[0]).toBeCloseTo(steps[7], 6);
    expect(path.ticks[4][0]).toBeCloseTo(1440, 6);
    expect(path.ticks[0]).toEqual([960, 540]);
    expect(path.ticks[8]).toEqual([1920, 540]);
  });

  it('goes back over itself where a clip with speed points shows a picture again', () => {
    const points: Retime = { points: [{ frame: 0, speed: { num: 1, den: 1 } }, { frame: 20, speed: { num: 0, den: 1 } }, { frame: 30, speed: { num: -1, den: 2 } }] };
    // 20 frames as the source is from picture 100, that picture held for 10, and 30 frames back at half the speed.
    const x = keys(key(100, 0), key(120, 0.5));
    const turning = clip('c1', 10, 60, { retime: points, source: { kind: 'media', media: 'm1', in: { ticks: 100, scale: 25 } }, transform: moved(x, 0) });
    const clock = keyClockOf(turning, timeline.rate, media);
    const path = motionPathOf(turning, timeline, [media])!;
    // A place at each frame of the timeline, at the picture that the clip shows there (48 pixels a picture); a picture shown on two frames running is there once.
    const shown = Array.from({ length: 60 }, (_, index) => clock(10 + index));
    const placeAt = (frame: number) => 960 + 1920 * valueAt(x, frame);
    const places = shown.map(placeAt);
    expect(path.ticks).toEqual(places.filter((place, index) => index === 0 || place !== places[index - 1]).map(place => [place, 540]));
    expect(path.ticks.length).toBeLessThan(60);
    expect(path.ticks[0]).toEqual([960, 540]);
    // It reaches the place of the picture that it holds, and comes back from it over the places that it passed.
    const furthest = Math.max(...path.ticks.map(at => at[0]));
    const turn = path.ticks.findIndex(at => at[0] === furthest);
    expect(furthest).toBe(placeAt(Math.max(...shown)));
    expect(furthest).toBeCloseTo(960 + 48 * (Math.max(...shown) - 100), 6);
    expect(turn).toBeGreaterThan(0);
    expect(turn).toBeLessThan(path.ticks.length - 1);
    const back = path.ticks.slice(turn + 1).map(at => at[0]);
    const out = path.ticks.slice(0, turn).map(at => at[0]);
    expect(back.every(place => out.includes(place))).toBe(true);
    expect(back.every((place, index) => index === 0 || place < back[index - 1])).toBe(true);
    // The points of its keys are those of the pictures that it shows, each once.
    expect(path.keys.map(each => each.frame)).toEqual([100, 120].filter(frame => frame >= Math.min(...shown) && frame <= Math.max(...shown)));
  });

  it('counts the frames of a title from its first, as its keys do', () => {
    const title = clip('c1', 100, 3, {
      source: { kind: 'title', text: 'Chapter one', style: { font: 'Arial', size: 72, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } },
      transform: moved(keys(key(0, 0), key(2, 0.5)), keys(key(0, -0.25), key(2, 0.25))),
    });
    expect(motionPathOf(title, timeline, [media])).toEqual({
      canvas: [1920, 1080],
      ticks: [[960, 270], [1440, 540], [1920, 810]],
      keys: [{ frame: 0, at: [960, 270], x: true, y: true }, { frame: 2, at: [1920, 810], x: true, y: true }],
    });
    // The place of the text of a title is another setting, of its style: it moves no anchor of the clip.
    expect(anchorAt(title, timeline, 1)).toEqual([1440, 540]);
  });

  it('has a place every few frames of a clip of more frames than it has dots for, and one at each key', () => {
    const length = MOST_TICKS * 2 + 1;
    const long = clip('c1', 0, length, { transform: moved(keys(key(0, 0), key(4, 0.25), key(length - 1, 0.5)), 0) });
    const path = motionPathOf(long, timeline, [media])!;
    // Every third frame (0 to 8190), the last frame, and the frame of the key that lies between two of them.
    expect(path.ticks).toHaveLength(2731 + 1 + 1);
    expect(path.ticks.length).toBeLessThanOrEqual(MOST_TICKS + 3);
    expect(path.ticks.slice(0, 3)).toEqual([[960, 540], [1320, 540], [1440, 540]]);
    expect(path.ticks[path.ticks.length - 1]).toEqual([1920, 540]);
    expect(path.keys.map(each => each.frame)).toEqual([0, 4, length - 1]);
  });

  it('marks the place at the playhead while the playhead is on the clip', () => {
    expect(anchorAtPlayhead(ACROSS, timeline, [media], 11)).toEqual([1200, 810]);
    // A playhead between two frames is in the earlier one, the frame that is shown.
    expect(anchorAtPlayhead(ACROSS, timeline, [media], 11.7)).toEqual([1200, 810]);
    expect(anchorAtPlayhead(ACROSS, timeline, [media], 14)).toEqual([1920, 810]);
    expect(anchorAtPlayhead(ACROSS, timeline, [media], 9)).toBeNull();
    expect(anchorAtPlayhead(ACROSS, timeline, [media], 15)).toBeNull();
  });
});

describe('what the engine is asked to draw for a motion path', () => {
  const path = motionPathOf(ACROSS, timeline, [media])!;

  it('is the line through its places, a dot at each, and the points of its keys over them', () => {
    expect(overlayOf(path)).toEqual({
      canvas: [1920, 1080],
      lines: [ACROSS_TICKS],
      dots: [{ kind: 'tick', at: ACROSS_TICKS }, { kind: 'key', at: [[960, 810], [1920, 810]] }],
    });
  });

  it('has the place at the playhead on top, and the point that is dragged in its own kind', () => {
    expect(overlayOf(path, [1200, 810], 1).dots).toEqual([
      { kind: 'tick', at: ACROSS_TICKS },
      { kind: 'key', at: [[960, 810]] },
      { kind: 'active', at: [[1920, 810]] },
      { kind: 'current', at: [[1200, 810]] },
    ]);
    // A point that is not one of the path is no point that is dragged.
    expect(overlayOf(path, null, 7).dots.map(each => each.kind)).toEqual(['tick', 'key']);
  });

  it('has no line through one place', () => {
    const still = motionPathOf(cut(5, keys(key(50, 0.25)), 0), timeline, [media])!;
    expect(overlayOf(still)).toEqual({ canvas: [1920, 1080], lines: [], dots: [{ kind: 'tick', at: [[1440, 540]] }, { kind: 'key', at: [[1440, 540]] }] });
  });
});

describe('a press on a motion path', () => {
  it('finds the nearest point within reach, and the later of two at one place', () => {
    const path = motionPathOf(ACROSS, timeline, [media])!;
    expect(hitKey(path, [965, 813], 8)).toBe(0);
    expect(hitKey(path, [1915, 806], 8)).toBe(1);
    expect(hitKey(path, [970, 810], 8)).toBe(-1);
    expect(hitKey(path, [970, 810], 16)).toBe(0);
    expect(hitKey(path, [1440, 810], 16)).toBe(-1);
    const held = motionPathOf(cut(5, keys(key(50, 0, 'hold'), key(52, 0.5), key(54, 0.5)), 0), timeline, [media])!;
    expect(hitKey(held, [1920, 540], 8)).toBe(2);
  });
});

describe('a point of a motion path that is dragged', () => {
  it('moves the key of each setting at its frame, and no other', () => {
    const both = cut(5, keys(key(50, 0), key(54, 0.5)), keys(key(50, 0.25), key(54, 0.25)));
    expect(movedPoint(both, 54, 0.125, -0.0625)).toEqual({
      'transform.x': keys(key(50, 0), key(54, 0.625)),
      'transform.y': keys(key(50, 0.25), key(54, 0.1875)),
    });
    // Not moved along an axis, its setting is as it was.
    expect(movedPoint(both, 54, 0, 0.5)).toEqual({ 'transform.x': both.transform.x, 'transform.y': keys(key(50, 0.25), key(54, 0.75)) });
    expect(movedPoint(both, 54, 0, 0)).toEqual({ 'transform.x': both.transform.x, 'transform.y': both.transform.y });
  });

  it('keeps each setting within what it takes', () => {
    const both = cut(5, keys(key(50, 0), key(54, 0.5)), keys(key(50, 0.25), key(54, 0.25)));
    expect(movedPoint(both, 54, 100, -100)).toEqual({
      'transform.x': keys(key(50, 0), key(54, 16)),
      'transform.y': keys(key(50, 0.25), key(54, -16)),
    });
  });

  it('puts a key on the setting that has none at the frame, of the value that it has there, and moves it', () => {
    const apart = cut(5, keys(key(50, 0), key(54, 0.5)), keys(key(48, 0), key(56, 0.5)));
    expect(movedPoint(apart, 54, 0, 0.125)).toEqual({
      'transform.x': apart.transform.x,
      'transform.y': keys(key(48, 0), key(54, 0.5), key(56, 0.5)),
    });
    // Moved along the axis that has the key only, the other gets none.
    expect(movedPoint(apart, 54, 0.25, 0)).toEqual({ 'transform.x': keys(key(50, 0), key(54, 0.75)), 'transform.y': apart.transform.y });
  });

  it('gives a setting that is one value a key at each point of the path, so that the point dragged alone moves', () => {
    const level = cut(5, keys(key(50, 0), key(52, 0.25), key(54, 0.5)), 0.25);
    expect(movedPoint(level, 52, 0, 0.125)).toEqual({
      'transform.x': level.transform.x,
      'transform.y': keys(key(50, 0.25), key(52, 0.375), key(54, 0.25)),
    });
    expect(movedPoint(level, 52, 0.125, 0)).toEqual({ 'transform.x': keys(key(50, 0), key(52, 0.375), key(54, 0.5)), 'transform.y': 0.25 });
  });
});

describe('Linear and Smooth of a point of a motion path', () => {
  const line = [key(50, 0), key(52, 0.25), key(54, 0.5)];
  const smooth = [
    key(50, 0, { bezier: { out: [EASE_SHARE, 0.25 * EASE_SHARE], in: [EASE_SHARE, 0] } }),
    key(52, 0.25, { bezier: { out: [EASE_SHARE, 0], in: [EASE_SHARE, -0.25 * EASE_SHARE] } }),
    key(54, 0.5),
  ];

  it('makes the two handles at the key flat for Smooth, as Ease In and Out of the Curve Editor does, and lays them on the lines for Linear', () => {
    expect(interpolated({ keys: line }, 52, 'smooth')).toEqual({ keys: smooth });
    expect(interpolated({ keys: smooth }, 52, 'linear')).toEqual({ keys: line });
    expect(interpolated({ keys: line }, 52, 'linear')).toEqual({ keys: line });
    // A key that holds is let go.
    expect(interpolated(keys(key(50, 0, 'hold'), key(54, 0.5)), 50, 'linear')).toEqual(keys(key(50, 0), key(54, 0.5)));
    // No key at the frame, or no keys at all: nothing to make.
    expect(interpolated({ keys: line }, 51, 'smooth')).toBeNull();
    expect(interpolated(0.25, 52, 'smooth')).toBeNull();
  });

  it('says how a point goes, by the settings that have a key there', () => {
    expect(pointInterp(cut(5, { keys: line }, 0.25), 52)).toBe('linear');
    expect(pointInterp(cut(5, { keys: smooth }, 0.25), 52)).toBe('smooth');
    expect(pointInterp(cut(5, { keys: smooth }, { keys: smooth }), 52)).toBe('smooth');
    // One setting smooth and the other on lines, a point with no key, and a key that holds, are neither.
    expect(pointInterp(cut(5, { keys: smooth }, { keys: line }), 52)).toBeNull();
    expect(pointInterp(cut(5, { keys: line }, 0.25), 51)).toBeNull();
    expect(pointInterp(cut(5, keys(key(50, 0, 'hold'), key(54, 0.5)), 0.25), 50)).toBeNull();
    // The settings that have no key at the frame have no say.
    expect(pointInterp(cut(5, { keys: smooth }, keys(key(50, 0), key(54, 0.5))), 52)).toBe('smooth');
  });

  it('keeps a point Smooth as it is dragged', () => {
    const before = cut(5, { keys: smooth }, 0.25);
    const after = movedPoint(before, 52, 0.125, 0);
    expect(pointInterp({ ...before, transform: moved(after['transform.x'], after['transform.y']) }, 52)).toBe('smooth');
    expect(valueAt(after['transform.x'], 52)).toBeCloseTo(0.375, 12);
  });
});
