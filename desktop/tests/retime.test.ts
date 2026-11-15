/**
 * The course of a clip through its source (`src/studio/retime`), against the numbers of the engine.
 *
 * The numbers expected here are those of the tests of the engine itself, copied by hand, each
 * with the test that it is from: `retime.rs` and `resolve.rs` of `engine/crates/model/src`. The
 * file of cases that the engine writes for speed points (`tests/data/retime.json`) is held in
 * `retime-golden.test.ts`. The cases of one speed are held against what the desktop counted
 * before there were speed points (`ticksOfScaled`), and against `key-clocks.json`.
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Clip, Media, Rate, Retime } from '../src/studio/engine';
import { keyClockOf, ticksOfScaled } from '../src/studio/color/keyFrames';
import {
  backwardAt, beginsLater, clipCourse, constantRetime, courseOf, frameCurveIn, insideMedia, movedPoint, pathTick, percentOf, percentToLast, raisedPoint,
  retimeCourse, samplesAt, soundPieces, speedOfPercent,
} from '../src/studio/retime';
import { clip } from './timeline-fixture';

const PAL: Rate = { num: 25, den: 1 };
const NTSC: Rate = { num: 30000, den: 1001 };
const SCALE = 15360;

/** Speed points, each a frame and the `num` and `den` of its speed, as the tests of the engine write them. */
const retime = (...points: [number, number, number][]): Retime => ({ points: points.map(([frame, num, den]) => ({ frame, speed: { num, den } })) });
const course = (points: Retime, length: number) => courseOf(points, length)!;
/** What each frame of a clip of `length` frames shows, in frames of its source after where it begins: at 25 frames a second, in ticks of 1/25 of a second. */
const shows = (points: Retime, length: number) => Array.from({ length }, (_, k) => Number(course(points, length).offset(k, PAL, 25)));
const pair = (both: [bigint, bigint]) => both.map(Number);

describe('the course of a clip with speed points', () => {
  // retime.rs, `a_frame_on_a_speed_point_is_of_the_segment_that_begins_there`.
  it('puts a frame on a speed point in the segment that begins there', () => {
    const mapping = retime([0, 1, 1], [4, 2, 1], [8, 1, 2]);
    const read = course(mapping, 12);
    expect([3, 4, 7, 8, 11].map(k => read.speedAt(k))).toEqual([{ num: 1, den: 1 }, { num: 2, den: 1 }, { num: 2, den: 1 }, { num: 1, den: 2 }, { num: 1, den: 2 }]);
    // Before the clip is of its first segment, and after it of its last.
    expect([read.speedAt(-1), read.speedAt(12)]).toEqual([{ num: 1, den: 1 }, { num: 1, den: 2 }]);
    expect([read.segmentOf(4).from, read.segmentOf(4).to]).toEqual([4, 8]);
    expect([read.segments.length, read.length, read.unit]).toEqual([3, 12, 2]);
    expect(shows(mapping, 12)).toEqual([0, 1, 2, 3, 4, 6, 8, 10, 12, 12, 13, 13]);
    // Places are counted in halves of a frame, as the slowest speed needs.
    expect([read.place(4), read.place(9)]).toEqual([8n, 25n]);
    // A frame that a transition shows goes on as the segment at that end does.
    expect([read.offset(-2, PAL, 25), read.offset(13, PAL, 25)]).toEqual([-2n, 14n]);
    expect(pair(read.shown(0, 0, PAL, 25))).toEqual([0, 13]);
    expect(pair(read.shown(2, 3, PAL, 25))).toEqual([-2, 15]);
    expect(read.span(PAL, 25)).toBe(14n);
    // retime.rs, `what_a_clip_shows_ends_at_the_highest_place_of_its_path`.
    expect(read.reach(0, 0, PAL, 25)).toBe(14n);
  });

  // retime.rs, `a_freeze_holds_a_picture_and_the_clip_plays_on_from_it`.
  it('holds one picture all through a freeze, and plays on from it', () => {
    const frozen = retime([0, 1, 1], [4, 0, 1], [7, 1, 1]);
    expect(shows(frozen, 13)).toEqual([0, 1, 2, 3, 4, 4, 4, 4, 5, 6, 7, 8, 9]);
    expect(course(frozen, 13).span(PAL, 25)).toBe(10n);
    // Backward, a frame shows the place where it ends.
    expect(shows(retime([0, -1, 1]), 10)).toEqual([9, 8, 7, 6, 5, 4, 3, 2, 1, 0]);
    expect(shows(retime([0, -1, 1], [5, 0, 1], [8, -1, 1]), 13)).toEqual([9, 8, 7, 6, 5, 5, 5, 5, 4, 3, 2, 1, 0]);
    expect(shows(retime([0, 1, 1], [4, 0, 1], [6, 2, 1]), 10)).toEqual([0, 1, 2, 3, 4, 4, 4, 6, 8, 10]);
    expect(shows(retime([0, -1, 1], [5, 1, 1]), 10)).toEqual([4, 3, 2, 1, 0, 0, 1, 2, 3, 4]);
    expect(shows(retime([0, -1, 1], [5, 0, 1], [7, 1, 1]), 12)).toEqual([4, 3, 2, 1, 0, 0, 0, 0, 1, 2, 3, 4]);
    // A clip that is a freeze frame from end to end shows where it begins.
    expect(shows(retime([0, 0, 1]), 5)).toEqual([0, 0, 0, 0, 0]);
    expect(course(retime([0, 0, 1]), 5).span(PAL, SCALE)).toBe(0n);
  });

  // retime.rs, `a_segment_turned_backward_goes_back_over_what_was_shown`.
  it('goes back over what was shown where a segment plays backward, and begins at the lowest place of its path', () => {
    const turned = retime([0, 1, 1], [6, -1, 1]);
    expect(shows(turned, 10)).toEqual([0, 1, 2, 3, 4, 5, 5, 4, 3, 2]);
    expect(course(turned, 10).span(PAL, 25)).toBe(6n);
    expect(pair(course(turned, 10).shown(0, 0, PAL, 25))).toEqual([0, 5]);
    expect(shows(retime([0, 1, 1], [4, -2, 1], [6, 1, 1]), 10)).toEqual([0, 1, 2, 3, 2, 0, 0, 1, 2, 3]);
    // Further back than it came: its first frame is above where it begins in its media.
    const deep = retime([0, 1, 1], [2, -1, 1]);
    expect(shows(deep, 10)).toEqual([6, 7, 7, 6, 5, 4, 3, 2, 1, 0]);
    expect(course(deep, 10).span(PAL, 25)).toBe(8n);
    expect(pair(course(deep, 10).shown(0, 0, PAL, 25))).toEqual([0, 7]);
  });

  // retime.rs, `a_speed_point_added_where_the_speed_does_not_change_moves_no_frame`.
  it('brings a place to ticks once, so a speed point where the speed does not change moves no frame', () => {
    const read = course(retime([0, 1, 1], [1, 1, 1], [2, 1, 1], [3, 1, 1]), 10);
    expect([0, 1, 2, 3, 4].map(k => read.offset(k, NTSC, 1000))).toEqual([0n, 33n, 66n, 100n, 133n]);
    const mixed = retime([0, 1, 3], [7, -2, 7], [20, 0, 1], [26, 5, 2], [40, -1, 1]);
    const before = Array.from({ length: 66 }, (_, k) => course(mixed, 60).offset(k - 3, NTSC, 1000));
    for (const at of [1, 6, 13, 22, 39, 59]) {
      const points = [...mixed.points];
      const index = points.findIndex(point => point.frame > at);
      points.splice(index < 0 ? points.length : index, 0, { frame: at, speed: points[(index < 0 ? points.length : index) - 1].speed });
      expect(Array.from({ length: 66 }, (_, k) => course({ points }, 60).offset(k - 3, NTSC, 1000)), `a speed point at ${at}`).toEqual(before);
    }
  });

  // retime.rs, `a_place_is_found_in_the_first_frame_that_shows_it`.
  it('finds a place in the first frame that shows it', () => {
    const find = (mapping: Retime, length: number, tick: number) => course(mapping, length).frameOf(tick, PAL, 25);
    // Twice as fast: every other place is shown, and one between is of the frame before it.
    const fast = retime([0, 2, 1]);
    for (let tick = 0; tick < 20; tick++) expect(find(fast, 10, tick), `${tick}`).toBe(Math.floor(tick / 2));
    expect([find(fast, 10, 20), find(fast, 10, -1)]).toEqual([null, null]);
    // Half as fast: each place is shown twice, and found in the first of the two frames.
    const slow = retime([0, 1, 2]);
    for (let tick = 0; tick < 5; tick++) expect(find(slow, 10, tick), `${tick}`).toBe(2 * tick);
    expect(find(slow, 10, 5)).toBeNull();
    const backward = retime([0, -1, 1]);
    for (let tick = 0; tick < 10; tick++) expect(find(backward, 10, tick), `${tick}`).toBe(9 - tick);
    expect(find(backward, 10, 10)).toBeNull();
    // A freeze: the first frame that holds the picture.
    const frozen = retime([0, 1, 1], [4, 0, 1], [7, 1, 1]);
    expect([3, 4, 5, 9, 10].map(tick => find(frozen, 13, tick))).toEqual([3, 4, 8, 12, null]);
    const held = retime([0, 1, 1], [5, 0, 1]);
    expect([find(held, 8, 5), find(held, 8, 6)]).toEqual([5, null]);
    // Forward and back over it: the way up comes first.
    const turned = retime([0, 1, 1], [6, -1, 1]);
    expect([3, 5, 6].map(tick => find(turned, 10, tick))).toEqual([3, 5, null]);
    // Back first and then forward: the way down comes first.
    const returning = retime([0, -1, 1], [5, 1, 1]);
    expect([find(returning, 10, 4), find(returning, 10, 0)]).toEqual([0, 4]);
  });

  // retime.rs, `a_frame_and_the_place_that_it_shows_lead_back_to_each_other`.
  it('leads from a frame to its place and back to no later frame', () => {
    const mappings: [Retime, number][] = [
      [retime([0, 137, 250]), 40], [retime([0, -3, 1]), 40], [retime([0, 2, 1], [9, 0, 1], [15, 1, 3]), 40],
      [retime([0, -1, 2], [11, -5, 3], [30, 0, 1]), 40], [retime([0, 1, 3], [7, -2, 7], [20, 0, 1], [26, 5, 2], [40, -1, 1]), 60],
    ];
    for (const [mapping, length] of mappings) {
      for (const [rate, scale] of [[PAL, 25], [PAL, SCALE], [NTSC, 1000]] as const) {
        const read = course(mapping, length);
        for (let k = 0; k < length; k++) {
          const tick = read.offset(k, rate, scale);
          const found = read.frameOf(tick, rate, scale);
          expect(found, `frame ${k}`).not.toBeNull();
          expect(found!).toBeLessThanOrEqual(k);
          expect(read.offset(found!, rate, scale) <= tick).toBe(true);
        }
        const span = read.span(rate, scale);
        expect(read.frameOf(-1n, rate, scale)).toBeNull();
        expect(read.frameOf(span + 1n, rate, scale)).toBeNull();
      }
    }
  });

  // retime.rs, `both_curves_are_read_from_the_speed_points`.
  it('gives the Retime Speed and the Retime Frame curve from the speed points', () => {
    const read = course(retime([0, 1, 1], [4, 2, 1], [8, 1, 2]), 12);
    expect(read.speedCurve()).toEqual([{ from: 0, to: 4, percent: 100 }, { from: 4, to: 8, percent: 200 }, { from: 8, to: 12, percent: 50 }]);
    expect(read.frameCurve()).toEqual([{ frame: 0, source: 0 }, { frame: 4, source: 4 }, { frame: 8, source: 12 }, { frame: 12, source: 14 }]);
    // Backward the speed is below 0, and the left control point is the higher.
    const backward = course(retime([0, -1, 1]), 10);
    expect(backward.speedCurve()).toEqual([{ from: 0, to: 10, percent: -100 }]);
    expect(backward.frameCurve()).toEqual([{ frame: 0, source: 10 }, { frame: 10, source: 0 }]);
    // A freeze is no speed, and level.
    const frozen = course(retime([0, 1, 1], [4, 0, 1], [7, 3, 4]), 11);
    expect(frozen.speedCurve()).toEqual([{ from: 0, to: 4, percent: 100 }, { from: 4, to: 7, percent: 0 }, { from: 7, to: 11, percent: 75 }]);
    expect(frozen.frameCurve()).toEqual([{ frame: 0, source: 0 }, { frame: 4, source: 4 }, { frame: 7, source: 4 }, { frame: 11, source: 7 }]);
    // retime.rs, `the_sound_of_a_segment_is_stretched_by_one_over_its_speed`.
    expect([percentOf({ num: -4, den: 1 }), percentOf({ num: 1, den: 8 }), percentOf({ num: 0, den: 3 })]).toEqual([-400, 12.5, 0]);
  });

  // The rules of `Retime::check`, as its doc says them.
  it('is of no clip where the speed points are not those of a clip of that length', () => {
    expect(courseOf({ points: [] }, 10)).toBeNull();
    expect(courseOf(retime([1, 1, 1]), 10)).toBeNull();
    expect(courseOf(retime([0, 1, 1], [4, 2, 1], [4, 1, 1]), 10)).toBeNull();
    expect(courseOf(retime([0, 1, 1], [10, 2, 1]), 10)).toBeNull();
    expect(courseOf(retime([0, 1, 101]), 10)).toBeNull();
    expect(courseOf(retime([0, 101, 1]), 10)).toBeNull();
    expect(courseOf(retime([0, 1, 0]), 10)).toBeNull();
    // The least number that both denominators divide is more than 32 bits hold.
    expect(courseOf(retime([0, 65537, 65537], [5, 65539, 65539]), 10)).toBeNull();
    expect(courseOf(retime([0, 1, 100], [5, 100, 1], [6, 0, 7]), 10)).not.toBeNull();
    // A clip is read at its speed where its speed points are not its own, and one without them has no course of them.
    expect(retimeCourse({ length: 10, speed: { num: 1, den: 1 }, reverse: false, retime: retime([0, 1, 1], [10, 2, 1]) })).toBeNull();
    expect(retimeCourse({ length: 10, speed: { num: 2, den: 1 }, reverse: true })).toBeNull();
  });
});

describe('a course of one speed', () => {
  // retime.rs, `a_mapping_of_one_speed_reads_its_media_as_a_clip_of_that_speed_does`.
  it('gives the ticks that a clip of that speed was counted to show', () => {
    const speeds = [{ num: 1, den: 1 }, { num: 2, den: 1 }, { num: 1, den: 2 }, { num: 24000, den: 1001 }, { num: 137, den: 250 }, { num: 1, den: 100 }, { num: 100, den: 1 }];
    const length = 100;
    for (const speed of speeds) {
      for (const reverse of [false, true]) {
        for (const [rate, scale] of [[PAL, SCALE], [PAL, 25], [NTSC, 1000], [NTSC, 90000]] as const) {
          const read = course(constantRetime(speed, reverse), length);
          for (let k = -5; k < length + 5; k++) {
            const shown = reverse ? length - 1 - k : k;
            expect(read.tick(1000, k, rate, scale), `frame ${k} at ${speed.num}/${speed.den}, backward ${reverse}`).toBe(1000n + ticksOfScaled(shown, speed, rate, scale));
          }
          expect(read.span(rate, scale)).toBe(ticksOfScaled(length, speed, rate, scale));
        }
      }
    }
  });

  it('gives the frames of keys that the engine wrote for clips of one speed, given as one speed point', () => {
    interface Case { name: string; clip: Clip; media: Media | null; rate: Rate; frames: number[]; keys: number[] }
    const cases: Case[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/key-clocks.json', import.meta.url), 'utf8'));
    let counted = 0;
    for (const each of cases) {
      if (each.clip.source.kind !== 'media' || !each.media?.facts.video || each.media.facts.video.still) continue;
      const pointed: Clip = { ...each.clip, speed: { num: 1, den: 1 }, reverse: false, retime: constantRetime(each.clip.speed, each.clip.reverse) };
      expect(each.frames.map(keyClockOf(pointed, each.rate, each.media)), each.name).toEqual(each.keys);
      counted += 1;
    }
    expect(counted).toBeGreaterThanOrEqual(6);
  });

  it('is counted in integers, past what a double holds of a product', () => {
    // 2^53 / 3 frames at 7/3 of a timeline of 30000/1001, in thousandths of a second: the product needs more than 53 bits, and the ticks fit in 64.
    const frames = 3_002_399_751_580_330;
    const read = course(retime([0, 7, 3]), frames + 1);
    expect(read.offset(frames, NTSC, 1000)).toBe((BigInt(frames) * 1001n * 1000n * 7n) / (3n * 30000n));
    // A count too large for 64 bits is the most that they hold, and one too far below nought the least.
    const far = course(retime([0, 100, 1]), Number.MAX_SAFE_INTEGER);
    expect(far.offset(Number.MAX_SAFE_INTEGER - 1, { num: 1, den: 1000 }, 4_000_000_000)).toBe((1n << 63n) - 1n);
    expect(far.offset(-(Number.MAX_SAFE_INTEGER - 1), { num: 1, den: 1000 }, 4_000_000_000)).toBe(-(1n << 63n));
  });
});

describe('the sound of a clip with speed points', () => {
  // retime.rs, `the_sound_goes_along_the_path_and_stands_still_in_a_freeze`.
  it('goes along the path, and stands still in a freeze', () => {
    // 4 frames forward, a picture held for 3, and 3 frames back at half the speed: 1920 samples to a frame, from sample 192000 of the media.
    const sounding = course(retime([0, 1, 1], [4, 0, 1], [7, -1, 2]), 10).sounding(0, PAL, 48000);
    const begins = { ticks: 100, scale: 25 };
    expect(sounding.parts()).toEqual([
      { from: -Infinity, to: 7680, speed: { num: 1, den: 1 } }, { from: 7680, to: 13440, speed: { num: 0, den: 1 } }, { from: 13440, to: Infinity, speed: { num: -1, den: 2 } },
    ]);
    expect([-1, 0, 7679, 7680, 13439, 13440, 19199, 19200].map(sample => sounding.partOf(sample))).toEqual([0, 0, 0, 1, 1, 2, 2, 2]);
    const read = (sample: number) => sounding.sourceSample(begins, sample, 48000);
    expect([read(-3), read(0), read(7679)]).toEqual([191_997, 192_000, 199_679]);
    expect([read(7680), read(13439)]).toEqual([199_680, 199_680]);
    // Backward at half the speed a sample reads where it ends, half a sample below the one before it.
    expect([read(13440), read(13441), read(13442)]).toEqual([199_679, 199_679, 199_678]);
    expect([read(19199), read(19200)]).toEqual([196_800, 196_799]);
    // At another rate of the media the places are the same.
    expect([sounding.sourceSample(begins, 0, 44100), sounding.sourceSample(begins, 7680, 44100)]).toEqual([176_400, 183_456]);
    // The time of the source rises, stands still and falls.
    expect(sounding.seconds(0)).toBeCloseTo(0, 12);
    expect(sounding.seconds(3840)).toBeCloseTo(0.08, 12);
    expect(sounding.seconds(7680)).toBeCloseTo(0.16, 12);
    expect(sounding.seconds(13000)).toBeCloseTo(0.16, 12);
    expect(sounding.seconds(13440)).toBeCloseTo(0.16 - 0.5 / 48000, 12);
    expect(sounding.seconds(19199)).toBeCloseTo(0.1, 12);
    // A time of the source is read once on the way up and once on the way down, and at no sample of the freeze.
    expect(sounding.samplesReading(0.08)).toEqual([3840, 21119]);
    expect(sounding.samplesReading(0.12)).toEqual([5760, 17279]);
  });

  // resolve.rs, `a_clip_of_sound_with_speed_points_gives_a_piece_of_each_segment_that_plays`.
  it('is a piece for each segment that plays, and none for a freeze', () => {
    // c7 on frames [50, 100) from 2 seconds into its media: 20 frames forward, a picture held for 10, and 20 back at half the speed.
    const c7 = clip('c7', 50, 50, { source: { kind: 'media', media: 'm1', in: { ticks: 30720, scale: SCALE } }, retime: retime([0, 1, 1], [20, 0, 1], [30, -1, 2]) });
    const spans = (window?: [number, number]) => soundPieces(c7, PAL, 48000, 48000, window).map(piece => [piece.from, piece.to, piece.sourceStart]);
    // With the 5 frames that a crossfade plays of it before its first, as the engine's test has it.
    const whole = soundPieces(c7, PAL, 48000, 48000, [86_400, 192_000]);
    expect(whole.map(piece => [piece.from, piece.to, piece.sourceStart])).toEqual([[86_400, 134_400, 86_400], [153_600, 192_000, 134_399]]);
    expect(whole.map(piece => [piece.speed, piece.reverse])).toEqual([[{ num: 1, den: 1 }, false], [{ num: 1, den: 2 }, true]]);
    expect(whole.map(piece => piece.frames)).toEqual([[0, 20], [30, 50]]);
    // A window of the timeline gives the pieces within it: none in the freeze.
    expect(spans([140_000, 160_000])).toEqual([[153_600, 160_000, 134_399]]);
    expect(spans([153_700, 153_800])).toEqual([[153_700, 153_800, 134_349]]);
    expect(spans([135_000, 150_000])).toEqual([]);
    // On its own frames it sounds from where its first begins.
    expect(spans()).toEqual([[96_000, 134_400, 96_000], [153_600, 192_000, 134_399]]);
    // Back at one speed it is one piece, as it was.
    const plain = clip('c7', 50, 50, { source: c7.source });
    expect(soundPieces(plain, PAL, 48000, 48000, [86_400, 192_000]).map(piece => [piece.from, piece.to, piece.sourceStart, piece.reverse])).toEqual([[86_400, 192_000, 86_400, false]]);
    expect(samplesAt(50, PAL, 48000)).toBe(96_000);
    expect(samplesAt(1001, NTSC, 48000)).toBe(1_603_201);
  });

  it('plays backward where the segment that a frame is in does', () => {
    const pointed = { length: 30, speed: { num: 1, den: 1 }, reverse: false, retime: retime([0, 1, 1], [10, 0, 1], [20, -1, 1]) };
    expect([0, 9, 10, 19, 20, 29].map(frame => backwardAt(pointed, frame))).toEqual([false, false, false, false, true, true]);
    expect(backwardAt({ length: 30, speed: { num: 2, den: 1 }, reverse: true }, 5)).toBe(true);
    expect(clipCourse({ length: 30, speed: { num: 2, den: 1 }, reverse: true })!.speedCurve()).toEqual([{ from: 0, to: 30, percent: -200 }]);
  });
});

describe('the speeds that a drag of a handle makes', () => {
  it('are whole hundredths of the speed of the source, in their lowest terms', () => {
    expect([150, 38, 0, -50, 100, 1, 10_000].map(speedOfPercent)).toEqual([
      { num: 3, den: 2 }, { num: 19, den: 50 }, { num: 0, den: 1 }, { num: -1, den: 2 }, { num: 1, den: 1 }, { num: 1, den: 100 }, { num: 100, den: 1 },
    ]);
  });

  it('make a segment end where its upper handle is let go, showing what it showed', () => {
    const read = course(retime([0, 1, 1], [40, -1, 2]), 100);
    // 40 frames at the speed of the source in 20 frames, in 80, and in 3.
    expect([20, 80, 3, 40].map(frames => percentToLast(read.segments[0], frames, read.unit))).toEqual([200, 50, 1333, 100]);
    // Backward, the speed has no sign: the segment keeps its way. Too fast or too slow, it is held to what a segment may have.
    expect([30, 60, 1, 9000].map(frames => percentToLast(read.segments[1], frames, read.unit))).toEqual([100, 50, 3000, 1]);
    // A freeze shows nothing that could last longer.
    expect(percentToLast(course(retime([0, 0, 1]), 10).segments[0], 5, 1)).toBeNull();
  });

  it('move a speed point with its lower handle, the clip lasting as long and the speeds either side changing', () => {
    const read = course(retime([0, 1, 1], [40, 1, 2], [80, -1, 1]), 100);
    // The first 40 frames of the source in 20 frames, and the next 20 in 60: 200% and 33%.
    expect(movedPoint(read, 1, 20)).toEqual(retime([0, 2, 1], [20, 33, 100], [80, -1, 1]));
    // Into a freeze and out of one, the freeze stays a freeze.
    const frozen = course(retime([0, 1, 1], [20, 0, 1], [30, -1, 2]), 60);
    expect(movedPoint(frozen, 1, 25)).toEqual(retime([0, 4, 5], [25, 0, 1], [30, -1, 2]));
    expect(movedPoint(frozen, 2, 40)).toEqual(retime([0, 1, 1], [20, 0, 1], [40, -3, 4]));
    // Not past the speed point before it nor the end of its segment, and the first speed point is not one to move.
    expect([movedPoint(read, 1, 0), movedPoint(read, 1, 80), movedPoint(read, 0, 5), movedPoint(read, 3, 90)]).toEqual([null, null, null, null]);
    // What it gives is the speed points of a clip of that length.
    expect(courseOf(movedPoint(read, 2, 99)!, 100)).not.toBeNull();
  });
});

describe('where a clip begins in its media once its speed points are set whole', () => {
  it('finds the tick at which the path begins and the one at which it stops', () => {
    // 10 frames up, 20 down and 20 up: the path begins 10 frames above its lowest place and stops 20 above it.
    const turned = course(retime([0, 1, 1], [10, -1, 1], [30, 1, 1]), 50);
    expect([pathTick(turned, 'start', PAL, 25), pathTick(turned, 'end', PAL, 25)]).toEqual([10n, 20n]);
    // Backward from end to end at twice the speed, it begins at its highest place and stops at its lowest.
    const backward = course(retime([0, -2, 1]), 25);
    expect([pathTick(backward, 'start', PAL, 25), pathTick(backward, 'end', PAL, 25)]).toEqual([50n, 0n]);
    // 25 frames at half the speed stop half way through a tick: it is rounded down.
    expect(pathTick(course(retime([0, 1, 2]), 25), 'end', PAL, 25)).toBe(12n);
  });

  // edit/retiming.rs, `speed_points_set_whole_keep_where_the_path_begins_to_the_tick`.
  it('keeps the place where the path begins at its tick, though the lowest place of the path moves by less than one', () => {
    const before = course(retime([0, 1, 1], [10, -1, 1], [30, 1, 1]), 50);
    // The speed point at frame 30 at frame 37: 27 frames down at 74% go 19.98 frames, so the path begins 9.98 frames above its lowest place.
    const after = course(retime([0, 1, 1], [10, -37, 50], [37, 77, 50]), 50);
    expect(beginsLater(before, after, 'start', PAL, 25)).toBe(1n);
    // c5 begins at frame 50 of m1, and then at frame 51: its first 10 frames show the pictures 60 to 69, as they did.
    const first = (read: typeof before, begins: number) => Array.from({ length: 10 }, (_, k) => Number(read.tick(begins, k, PAL, 25)));
    expect(first(after, 51)).toEqual(first(before, 50));
    expect(first(before, 50)).toEqual([60, 61, 62, 63, 64, 65, 66, 67, 68, 69]);
    // The speed points that it had put it back where it began, and the same speed points move it nowhere.
    expect(beginsLater(after, before, 'start', PAL, 25)).toBe(-1n);
    expect(beginsLater(after, after, 'start', PAL, 25)).toBe(0n);
  });

  // edit/retiming.rs, `speed_points_set_whole_may_keep_where_the_path_stops`.
  it('keeps the place where the path stops instead, where it is told to', () => {
    const two = course(retime([0, 1, 1], [20, 1, 1]), 50);
    const slower = course(retime([0, 1, 2], [20, 1, 1]), 50);
    // The first 20 frames go 10 frames of the source and not 20: kept where it stops, the clip begins 10 frames later.
    expect([beginsLater(two, slower, 'end', PAL, 25), beginsLater(two, slower, 'start', PAL, 25)]).toEqual([10n, 0n]);
    // A clip of one speed at half of it shows the last 25 frames of the 50 that it showed.
    expect(beginsLater(course(retime([0, 1, 1]), 50), course(retime([0, 1, 2]), 50), 'end', PAL, 25)).toBe(25n);
    // Turned round, it comes down to where it stopped: it begins there, 50 frames later.
    expect(beginsLater(course(retime([0, 1, 1]), 50), course(retime([0, -1, 1]), 50), 'end', PAL, 25)).toBe(50n);
  });

  it('says whether a clip shows only what its media has', () => {
    // 100 frames forward in media of 250: from its frame 150 they end where the media ends.
    const plain = course(retime([0, 1, 1]), 100);
    expect([0n, 150n, 151n, -1n].map(begins => insideMedia(plain, begins, 250n, PAL, 25))).toEqual([true, true, false, false]);
    // Backward, the same frames of the media are shown.
    const backward = course(retime([0, -1, 1]), 100);
    expect([0n, 150n, 151n, -1n].map(begins => insideMedia(backward, begins, 250n, PAL, 25))).toEqual([true, true, false, false]);
  });
});

describe('the Retime Frame curve of a clip', () => {
  // 20 frames as the source is, a picture held for 10, and 30 frames back at half the speed.
  const frozen = course(retime([0, 1, 1], [20, 0, 1], [30, -1, 2]), 60);

  it('has a control point at each speed point and at the end of the clip, at the frame of the media that the path is at', () => {
    // The clip begins 100 frames into its media: it rises to 120, stays, and falls to 105.
    expect(frameCurveIn(frozen, 100, PAL, 25)).toEqual([{ frame: 0, source: 100 }, { frame: 20, source: 120 }, { frame: 30, source: 120 }, { frame: 60, source: 105 }]);
    // Ticks of 1/15360 of a second are brought to frames of the timeline.
    expect(frameCurveIn(course(retime([0, 2, 1]), 10), 2 * SCALE, PAL, SCALE)).toEqual([{ frame: 0, source: 50 }, { frame: 10, source: 70 }]);
  });

  it('gives the segments either side of a control point that is raised other speeds, the frames of the speed points staying', () => {
    // The second point 5 frames higher: 25 frames of the source in the first 20, at 125%, and 5 back in the 10 of the freeze, at 50%.
    expect(raisedPoint(frozen, 1, 5)).toEqual({ retime: retime([0, 5, 4], [20, -1, 2], [30, -1, 2]), keep: 'start' });
    // The first point 5 frames higher: its segment goes 15 frames in its 20, and the end of the path is what stays in the media.
    expect(raisedPoint(frozen, 0, 5)).toEqual({ retime: retime([0, 3, 4], [20, 0, 1], [30, -1, 2]), keep: 'end' });
    // The end of the clip 6 frames lower: the last segment goes back 21 frames in its 30, at 70%.
    expect(raisedPoint(frozen, 3, -6)).toEqual({ retime: retime([0, 1, 1], [20, 0, 1], [30, -7, 10]), keep: 'start' });
    // Not raised, a speed that is a whole hundredth stays what it is.
    expect(raisedPoint(frozen, 2, 0)).toEqual({ retime: retime([0, 1, 1], [20, 0, 1], [30, -1, 2]), keep: 'start' });
  });

  it('brings the speeds to whole hundredths that a segment may have: a freeze where it is level, and no faster than 10000%', () => {
    // The end of the clip raised to within a tenth of a frame of where the last segment begins: less than half a hundredth, a freeze.
    expect(raisedPoint(frozen, 3, 14.9)!.retime.points[2].speed).toEqual({ num: 0, den: 1 });
    // 13 frames in 30 are 43.33%: 43%.
    expect(raisedPoint(frozen, 3, 28)!.retime.points[2].speed).toEqual({ num: 43, den: 100 });
    // Far up, the segment before the point plays as fast as one may, and the one after it comes back as fast.
    expect(raisedPoint(frozen, 1, 5000)!.retime.points.map(point => point.speed)).toEqual([{ num: 100, den: 1 }, { num: -100, den: 1 }, { num: -1, den: 2 }]);
    // What it gives is the speed points of a clip of that length, and there is no such point outside them.
    expect(courseOf(raisedPoint(frozen, 1, 5000)!.retime, 60)).not.toBeNull();
    expect([raisedPoint(frozen, 4, 1), raisedPoint(frozen, -1, 1), raisedPoint(frozen, 1, Number.NaN)]).toEqual([null, null, null]);
  });
});
