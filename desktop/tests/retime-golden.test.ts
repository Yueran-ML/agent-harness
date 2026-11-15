/**
 * Where a clip with speed points reads its source (`src/studio/retime`, and the clock of its keys
 * in `src/studio/color/keyFrames`), held to the numbers of the engine itself: the cases and what
 * the engine found for each are written by the engine (`engine/crates/model/tests/retime_golden.rs`,
 * which says what each part of a case is). The tests of `retime.test.ts` hold the same functions to
 * numbers copied by hand from the tests of the engine; these are not copied.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { keyClockOf, shownKeyRange } from '../src/studio/color/keyFrames';
import type { Clip, Media, Rate, Speed, SpeedPoint, Time } from '../src/studio/engine';
import { clipCourse, samplesAt, soundPieces, type Course, type FramePoint, type SpeedStep } from '../src/studio/retime';

/** A count as the engine writes it: a number, or the string of its digits where a double does not hold it exactly. */
type Whole = number | string;
interface Shown { before: number; after: number; first: Whole; last: Whole; end: Whole }
interface Piece { from: Whole; to: Whole; source_start: Whole; speed: Speed; reverse: boolean }
interface Part { from: Whole | null; to: Whole | null; speed: Speed }
interface Trimmed { points: SpeedPoint[]; length: number; later: Whole }
interface Case {
  name: string;
  clip: Clip;
  media: Media;
  rate: Rate;
  sample_rate: number;
  unit: number;
  segments: { from: number; to: number; speed: Speed }[];
  frames: number[];
  ticks: Whole[];
  span: Whole;
  shown: Shown[];
  frame_of: { ticks: Whole[]; frames: (number | null)[] };
  speed_curve: SpeedStep[];
  frame_curve: FramePoint[];
  keys: { frames: number[]; at: number[]; shown: [number, number] | null };
  sound: { parts: Part[]; windows: { from: Whole; to: Whole; pieces: Piece[] }[]; samples: Whole[]; source_samples: Whole[] };
  from_frame: ({ from: number } & Trimmed)[];
  with_length: ({ to: number } & Trimmed)[];
}
const cases: Case[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/retime.json', import.meta.url), 'utf8'));

/** The parts of a case that the desktop is held to. */
const HELD = ['clip', 'frame_curve', 'frame_of', 'frames', 'keys', 'media', 'name', 'rate', 'sample_rate', 'segments', 'shown', 'sound', 'span', 'speed_curve', 'ticks', 'unit'];
/**
 * The parts of a case that the desktop is not held to, since it has no function that counts them:
 * `from_frame` and `with_length` are what the engine makes of the speed points of a clip whose head
 * or tail is moved (`Retime::from_frame`, `Retime::with_length`). The desktop sends the trim and
 * draws what the engine answers; while a drag lasts it only finds how far an end may be moved
 * (`retimedRoom` of `timeline/interact`), from `Course.shown` and `Course.reach`, which are held here.
 */
const NOT_HELD = ['from_frame', 'with_length'];

const big = (count: Whole): bigint => BigInt(count);
const scaleOf = (each: Case): number => each.media.facts.duration.scale;
/** Where the clip of a case begins in its media. */
function beginsOf(each: Case): Time {
  if (each.clip.source.kind !== 'media') throw new Error(`the clip of "${each.name}" is not cut from media`);
  return each.clip.source.in;
}
/** The course of the clip of a case: along its speed points, or that of its one speed. */
function courseOf(each: Case): Course {
  const course = clipCourse(each.clip);
  if (!course) throw new Error(`the clip of "${each.name}" has no course`);
  return course;
}

describe('the cases of speed points that the engine wrote', () => {
  it('are all read, and every part of each is held or named as not held', () => {
    expect(cases.length).toBeGreaterThanOrEqual(20);
    expect(new Set(cases.map(each => each.name)).size).toBe(cases.length);
    for (const each of cases) expect(Object.keys(each).sort(), each.name).toEqual([...HELD, ...NOT_HELD].sort());
  });

  it('are of clips of one speed, of clips with speed points, and of counts that a double does not hold', () => {
    expect(cases.filter(each => !each.clip.retime).length).toBeGreaterThanOrEqual(6);
    expect(cases.filter(each => (each.clip.retime?.points.length ?? 0) >= 12).length).toBeGreaterThanOrEqual(1);
    expect(cases.some(each => each.ticks.some(tick => typeof tick === 'string'))).toBe(true);
  });
});

describe('the course of a clip, against the numbers of the engine', () => {
  it('has the unit and the segments that the engine counts', () => {
    for (const each of cases) {
      const course = courseOf(each);
      expect(course.unit, each.name).toBe(each.unit);
      expect(course.length, each.name).toBe(each.clip.length);
      expect(course.segments.map(({ from, to, speed }) => ({ from, to, speed })), each.name).toEqual(each.segments);
    }
  });

  it('shows at each frame, and at those that a transition reads about the clip, the tick that the engine shows', () => {
    for (const each of cases) {
      const course = courseOf(each);
      const begins = beginsOf(each).ticks;
      expect(each.frames.length, each.name).toBe(each.ticks.length);
      expect(each.frames.map(k => course.tick(begins, k, each.rate, scaleOf(each))), each.name).toEqual(each.ticks.map(big));
    }
  });

  it('shows the span that the engine counts, and with the frames of a transition the same lowest and highest tick and the same end', () => {
    for (const each of cases) {
      const course = courseOf(each);
      expect(course.span(each.rate, scaleOf(each)), each.name).toBe(big(each.span));
      for (const { before, after, first, last, end } of each.shown) {
        const what = `${each.name}, ${before} before and ${after} after`;
        expect(course.shown(before, after, each.rate, scaleOf(each)), what).toEqual([big(first), big(last)]);
        expect(course.reach(before, after, each.rate, scaleOf(each)), what).toBe(big(end));
      }
    }
  });

  it('finds a place in the frame that the engine finds it in, and in none where the engine finds none', () => {
    for (const each of cases) {
      const course = courseOf(each);
      const { ticks, frames } = each.frame_of;
      expect(ticks.length, each.name).toBe(frames.length);
      expect(ticks.map(tick => course.frameOf(big(tick), each.rate, scaleOf(each))), each.name).toEqual(frames);
    }
  });

  it('gives the points of the Retime Speed and the Retime Frame curve, to the last bit', () => {
    for (const each of cases) {
      const course = courseOf(each);
      // The engine writes each as the nearest double; `toEqual` takes two numbers as equal only where they are the same double.
      expect(course.speedCurve(), each.name).toEqual(each.speed_curve);
      expect(course.frameCurve(), each.name).toEqual(each.frame_curve);
    }
  });
});

describe('the keys of a clip, against the numbers of the engine', () => {
  it('are at the frame of the media that the engine reads them at, and the clip shows the frames that the engine says', () => {
    for (const each of cases) {
      const clock = keyClockOf(each.clip, each.rate, each.media);
      expect(each.keys.frames.length, each.name).toBe(each.keys.at.length);
      expect(each.keys.frames.map(clock), each.name).toEqual(each.keys.at);
      expect(shownKeyRange(each.clip, clock), each.name).toEqual(each.keys.shown);
    }
  });
});

describe('the sound of a clip, against the numbers of the engine', () => {
  const mediaRateOf = (each: Case): number => each.media.facts.audio!.rate;

  it('is of the segment that the engine gives each sample of the timeline to', () => {
    // A part that goes on without end has no number there: the engine writes null, and the desktop an infinity.
    const edge = (sample: number): bigint | null => (Number.isFinite(sample) ? BigInt(sample) : null);
    const written = (sample: Whole | null): bigint | null => (sample === null ? null : big(sample));
    for (const each of cases) {
      const sounding = courseOf(each).sounding(each.clip.start, each.rate, each.sample_rate);
      expect(sounding.parts().map(part => ({ from: edge(part.from), to: edge(part.to), speed: part.speed })), each.name)
        .toEqual(each.sound.parts.map(part => ({ from: written(part.from), to: written(part.to), speed: part.speed })));
    }
  });

  it('reads at each sample of the timeline the sample of the media that the engine reads', () => {
    for (const each of cases) {
      const sounding = courseOf(each).sounding(each.clip.start, each.rate, each.sample_rate);
      const { samples, source_samples: read } = each.sound;
      expect(samples.length, each.name).toBe(read.length);
      expect(samples.map(sample => BigInt(sounding.sourceSample(beginsOf(each), Number(sample), mediaRateOf(each)))), each.name).toEqual(read.map(big));
    }
  });

  it('is heard in the pieces that the engine hears, in a window about the clip and in windows that begin within it', () => {
    for (const each of cases) {
      const { start, length } = each.clip;
      const first = samplesAt(start, each.rate, each.sample_rate);
      const end = samplesAt(start + length, each.rate, each.sample_rate);
      expect(each.sound.windows.length, each.name).toBe(3);
      for (const asked of each.sound.windows) {
        // The engine hears the clip on its own samples within the window, no crossfade playing more of it: those are the samples that the desktop is told the clip sounds on.
        const sounds: [number, number] = [Math.max(Number(asked.from), first), Math.min(Number(asked.to), end)];
        const pieces = soundPieces(each.clip, each.rate, each.sample_rate, mediaRateOf(each), sounds);
        expect(
          pieces.map(piece => ({ from: BigInt(piece.from), to: BigInt(piece.to), source_start: BigInt(piece.sourceStart), speed: piece.speed, reverse: piece.reverse })),
          `${each.name}, samples ${asked.from} to ${asked.to}`,
        ).toEqual(asked.pieces.map(piece => ({ from: big(piece.from), to: big(piece.to), source_start: big(piece.source_start), speed: piece.speed, reverse: piece.reverse })));
      }
    }
  });
});
