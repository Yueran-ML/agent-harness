/**
 * Where a clip that changes its speed on the way reads its source, as the engine counts it
 * (`engine/crates/model/src/retime.rs`, `span.rs`, `resolve.rs`; DaVinci Resolve's Retime Controls,
 * Reference Manual 20, pp. 1070–1082).
 *
 * A clip with speed points (`Clip.retime`) holds a list of them: each is at a frame of the clip,
 * counted from its first, and gives the speed of the segment that begins there and lasts to the
 * next one or to the end of the clip (above 0 forward, below 0 backward, 0 a freeze). Put one after
 * another, the segments are one path through the source without a jump, and the clip begins in its
 * media (`source.in`) at the lowest place of that path within the clip.
 *
 * Everything that the engine counts in integers is counted in integers here, with `BigInt` where a
 * product of 64-bit numbers is more than a double holds: the places of the path are whole numbers
 * of a unit of a frame (the least that every speed is a whole number of), brought to ticks once,
 * rounded down, and a count that an `i64` does not hold is the most or the least that it holds, as
 * in the engine. A course of one speed gives the ticks that `ticksOfScaled` of `color/keyFrames`
 * gives. What is only drawn (the waveform, the line of the gain) is read from the same path as a
 * line of floating numbers (`Course.along`).
 */
import type { Clip, Rate, Retime, Speed, SpeedPoint, Time } from './engine';

/** The most frames that a clip may last (`ops::LARGEST`): what JavaScript holds exactly. */
export const LARGEST = Number.MAX_SAFE_INTEGER;
/** The most speed points that a clip may have (`retime::MOST_POINTS`). */
export const MOST_POINTS = 1024;

const I64_MAX = (1n << 63n) - 1n;
const I64_MIN = -(1n << 63n);
const U32_MAX = 0xffff_ffffn;

/** A count as an `i64` holds it: one too large is the most that there is, one too far below nought the least. */
const within = (count: bigint): bigint => (count > I64_MAX ? I64_MAX : count < I64_MIN ? I64_MIN : count);

/** `a / b` rounded down, as Rust's `div_euclid` for a `b` above 0. */
export function floorDiv(a: bigint, b: bigint): bigint {
  const q = a / b;
  return a % b !== 0n && (a < 0n) !== (b < 0n) ? q - 1n : q;
}

const gcd = (a: bigint, b: bigint): bigint => {
  let [x, y] = [a < 0n ? -a : a, b < 0n ? -b : b];
  while (y !== 0n) [x, y] = [y, x % y];
  return x;
};
const abs = (n: bigint): bigint => (n < 0n ? -n : n);

/** The speed of a clip of one speed as the speed of a segment: below 0 where it plays backward. */
export const signedSpeed = (speed: Speed, reverse: boolean): Speed => ({ num: reverse ? -speed.num : speed.num, den: speed.den });

/** The speed points of a clip of one speed: one, at frame 0 (`Retime::constant`). */
export const constantRetime = (speed: Speed, reverse: boolean): Retime => ({ points: [{ frame: 0, speed: signedSpeed(speed, reverse) }] });

/**
 * A speed as the Retime Controls show it, in hundredths of the speed of the source: 100 as the
 * source is, 0 in a freeze, below 0 backward (`SignedSpeed::percent`).
 */
export const percentOf = (speed: Speed): number => {
  const percent = 100 * Math.abs(speed.num) / speed.den;
  return speed.num < 0 ? -percent : percent;
};

/** Whether a speed is one that a segment may have: a freeze, or from 1/100 to 100 either way (`SignedSpeed::check`). */
function speedAllowed(speed: Speed): boolean {
  if (!Number.isSafeInteger(speed.num) || !Number.isSafeInteger(speed.den) || speed.den <= 0 || speed.den > 0xffff_ffff) return false;
  const num = BigInt(Math.abs(speed.num));
  const den = BigInt(speed.den);
  return num === 0n || (num * 100n >= den && num <= den * 100n);
}

/** A segment of a clip: the frames from one speed point to the next, at one speed. */
export interface Segment {
  /** Its first frame, where its speed point is. */
  from: number;
  /** The frame after its last: where the next speed point is, or the end of the clip. */
  to: number;
  /** How fast it plays, and which way. */
  speed: Speed;
  /** The place of the path where it begins, in units of a frame, above the lowest place of the path within the clip. */
  at: bigint;
  /** How far the path goes with each of its frames, in units of a frame: below 0, down the source. */
  step: bigint;
}

/** The place of the path at `frame`, as a segment goes: also before the segment and after it. */
const placeIn = (segment: Segment, frame: bigint): bigint => segment.at + segment.step * (frame - BigInt(segment.from));
/** The place that frame `k` shows, as a segment goes: where the frame begins, or, in a segment that goes backward, where it ends. */
const shownIn = (segment: Segment, k: bigint): bigint => placeIn(segment, k + (segment.step < 0n ? 1n : 0n));

/** A place, counted in `unit`s of a frame, in ticks of `scale`, rounded down, on a timeline of `rate` (`retime::ticks_of`). */
function ticksOf(place: bigint, unit: number, rate: Rate, scale: number): bigint {
  const times = BigInt(rate.den) * BigInt(scale);
  const per = BigInt(unit) * BigInt(rate.num);
  if (per === 0n) return place < 0n ? I64_MIN : I64_MAX;
  return within(floorDiv(place * times, per));
}

/** The first of the frames from `from` to the one before `to` that `holds` for, where it holds for every frame after one that it holds for; `to` if it holds for none. */
function first(from: number, to: number, holds: (k: number) => boolean): number {
  let [low, high] = [from, to];
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (holds(middle)) high = middle;
    else low = middle + 1;
  }
  return low;
}

/** A step of the Retime Speed curve (p. 1080): the speed of a segment over its frames, in hundredths of the speed of the source. */
export interface SpeedStep { from: number; to: number; percent: number }
/** A control point of the Retime Frame curve (p. 1079): the place of the source, in frames of the timeline's rate above the lowest place shown, at a frame of the clip. */
export interface FramePoint { frame: number; source: number }

/** A part of the path of a clip as a line to draw by: over the frames `from`–`to` of the clip it goes from `at`, in frames of the source at the rate of the timeline above the lowest place, by `step` a frame. */
export interface PathPart { from: number; to: number; at: number; step: number }

/**
 * The course of a clip through its source: its speed points, checked and counted out for a clip of
 * a length (`Retime::course`). A place of the media is given in ticks of its scale, counted from
 * where the clip begins in its media; the rate is that of the timeline.
 */
export class Course {
  constructor(
    /** The segments, in the order of their frames: at least one. */
    readonly segments: readonly Segment[],
    /** How many units of a place make a frame, and a speed of 1. */
    readonly unit: number,
    /** The highest place of the path within the clip, above the lowest. */
    readonly highest: bigint,
    /** How many frames the clip lasts. */
    readonly length: number,
  ) {}

  /** The segment that frame `k` of the clip is in. A frame before the clip is of the first segment, and one after it of the last. */
  segmentOf(k: number): Segment {
    const after = first(0, this.segments.length, index => this.segments[index].from > k);
    return this.segments[Math.max(0, after - 1)];
  }

  /** The speed of the segment that frame `k` is in. */
  speedAt(k: number): Speed { return this.segmentOf(k).speed; }

  /** The place that frame `k` shows, in units of a frame of the source at the rate of the timeline, above the lowest place that the clip shows. */
  place(k: number): bigint { return shownIn(this.segmentOf(k), BigInt(Math.floor(k))); }

  private shows(segment: Segment, k: number, rate: Rate, scale: number): bigint {
    return ticksOf(shownIn(segment, BigInt(Math.floor(k))), this.unit, rate, scale);
  }

  /** The place in its media that frame `k` shows, in ticks of `scale` after where the clip begins in its media (`Course::offset`). */
  offset(k: number, rate: Rate, scale: number): bigint { return this.shows(this.segmentOf(k), k, rate, scale); }

  /** The place in its media that frame `k` of a clip that begins at `begins` shows, in ticks of `scale` (`Course::tick`). */
  tick(begins: bigint | number, k: number, rate: Rate, scale: number): bigint { return within(BigInt(begins) + this.offset(k, rate, scale)); }

  /** The ticks of `scale` of its media that the clip shows, from the lowest place of its path to the highest (`Course::span`). */
  span(rate: Rate, scale: number): bigint { return ticksOf(this.highest, this.unit, rate, scale); }

  /**
   * The lowest and the highest place that the frames of the clip show, with `before` frames more
   * before its first and `after` more after its last, as a transition shows them (`Course::shown`).
   */
  shown(before: number, after: number, rate: Rate, scale: number): [bigint, bigint] {
    const earliest = -Math.max(before, 0);
    const latest = this.length - 1 + Math.max(after, 0);
    let lowest = I64_MAX;
    let highest = I64_MIN;
    this.segments.forEach((segment, index) => {
      // A segment goes one way: its first frame and its last show its ends.
      const from = index === 0 ? earliest : segment.from;
      const to = index + 1 === this.segments.length ? latest : segment.to - 1;
      for (const frame of [from, to]) {
        const tick = this.shows(segment, frame, rate, scale);
        if (tick < lowest) lowest = tick;
        if (tick > highest) highest = tick;
      }
    });
    return [lowest, highest];
  }

  /** The highest place that the path comes to, with frames more before and after as `shown` has them: where what its frames show ends (`Course::reach`). */
  reach(before: number, after: number, rate: Rate, scale: number): bigint {
    const earliest = -BigInt(Math.max(before, 0));
    const ends = BigInt(this.length) + BigInt(Math.max(after, 0));
    // A segment goes one way: the path is highest in it where it begins or where it ends.
    let highest = placeIn(this.segments[0], earliest);
    for (const [index, segment] of this.segments.entries()) {
      const from = index === 0 ? earliest : BigInt(segment.from);
      const to = index + 1 === this.segments.length ? ends : BigInt(segment.to);
      for (const place of [placeIn(segment, from), placeIn(segment, to)]) if (place > highest) highest = place;
    }
    return ticksOf(highest, this.unit, rate, scale);
  }

  private frameIn(segment: Segment, tick: bigint, rate: Rate, scale: number): number | null {
    const shows = (k: number) => this.shows(segment, k, rate, scale);
    const { from, to } = segment;
    if (segment.step === 0n) return shows(from) === tick ? from : null;
    if (segment.step > 0n) {
      // Its frames show higher and higher places, up to where it stops.
      const stops = ticksOf(placeIn(segment, BigInt(to)), this.unit, rate, scale);
      const above = first(from, to, k => shows(k) > tick);
      if (above === from) return null;
      const found = shows(above - 1);
      // Where it stops, and above, only a place that a frame shows itself is on show.
      if (tick >= stops && found !== tick) return null;
      return first(from, above, k => shows(k) >= found);
    }
    // Its frames show lower and lower places, down from where it begins: there, and above, only a place that a frame shows itself is on show.
    const begins = ticksOf(placeIn(segment, BigInt(from)), this.unit, rate, scale);
    const found = first(from, to, k => shows(k) <= tick);
    return found < to && (tick < begins || shows(found) === tick) ? found : null;
  }

  /**
   * The first frame of the clip in which the place `tick` of its media is on show, in ticks of
   * `scale` after where the clip begins in its media (`Course::frame_of`): the frame that shows the
   * place, or the nearest place below it that its segment shows. Null where the clip does not come
   * by the place.
   */
  frameOf(tick: bigint | number, rate: Rate, scale: number): number | null {
    const wanted = BigInt(tick);
    for (const segment of this.segments) {
      const found = this.frameIn(segment, wanted, rate, scale);
      if (found !== null) return found;
    }
    return null;
  }

  /** The Retime Speed curve (p. 1080): a step for each segment, level at its speed (`Course::speed_curve`). */
  speedCurve(): SpeedStep[] {
    return this.segments.map(segment => ({ from: segment.from, to: segment.to, percent: percentOf(segment.speed) }));
  }

  /** The Retime Frame curve (p. 1079): the place of the source at each speed point and at the end of the clip (`Course::frame_curve`). */
  frameCurve(): FramePoint[] {
    const point = (frame: number, place: bigint): FramePoint => ({ frame, source: Number(place) / this.unit });
    const curve = this.segments.map(segment => point(segment.from, segment.at));
    const last = this.segments[this.segments.length - 1];
    curve.push(point(last.to, placeIn(last, BigInt(last.to))));
    return curve;
  }

  /** The path as lines to draw by, one for each segment: floating numbers, for what is drawn and not counted. */
  parts(): PathPart[] {
    return this.segments.map(segment => ({ from: segment.from, to: segment.to, at: Number(segment.at) / this.unit, step: Number(segment.step) / this.unit }));
  }

  /**
   * The place of the path at `frame` of the clip, which need not be a whole frame, in frames of the
   * source at the rate of the timeline above the lowest place: along the line of the segment that
   * the frame is in. Before the clip the first segment goes on, and after it the last.
   */
  along(frame: number): number {
    const segment = this.segmentOf(frame >= this.length ? this.length - 1 : Math.floor(frame));
    return (Number(segment.at) + Number(segment.step) * (frame - segment.from)) / this.unit;
  }

  /**
   * The frames of the clip, which need not be whole, at which the path is at `place` (as `along`
   * gives it): one for each segment that comes by it, in their order, and none of a freeze. The
   * first segment goes on before the clip and the last one after it, as they do in the engine
   * (`Sounding::samples_reading`).
   */
  framesAt(place: number): number[] {
    const frames: number[] = [];
    this.segments.forEach((segment, index) => {
      if (segment.step === 0n) return;
      const frame = segment.from + (place * this.unit - Number(segment.at)) / Number(segment.step);
      const low = index === 0 ? -Infinity : segment.from;
      const high = index + 1 === this.segments.length ? Infinity : segment.to;
      if (frame >= low && frame < high) frames.push(frame);
    });
    return frames;
  }

  /** How the sound of the clip is read along the path, where the clip begins at frame `start` of a timeline of `rate` frames and `sampleRate` samples a second (`Course::sounding`). */
  sounding(start: number, rate: Rate, sampleRate: number): Sounding {
    // The path is lowest where a segment begins, or else where the clip ends.
    const frame = this.segments.find(segment => segment.at === 0n)?.from ?? this.length;
    const per = BigInt(Math.max(rate.num, 1));
    const spread = (frames: bigint) => frames * BigInt(sampleRate) * BigInt(rate.den);
    const sample = floorDiv(spread(BigInt(start) + BigInt(frame)), per);
    // The first sample that begins at each speed point or after it: the samples from the frame of the lowest place to it, rounded up.
    const starts = this.segments.slice(1).map(segment => within(sample - floorDiv(-spread(BigInt(segment.from) - BigInt(frame)), per)));
    return new Sounding(this.segments, this.unit, rate, sampleRate, frame, within(sample), starts);
  }
}

/** A part of the sound of a clip with speed points: the samples of the timeline that are of one of its segments (`SoundPart`). */
export interface SoundPart {
  /** The first sample of the timeline that is of the segment: `-Infinity` for the first segment, which goes on before the clip. */
  from: number;
  /** The sample after the last that is of the segment: `Infinity` for the last, which goes on after the clip. */
  to: number;
  /** How fast the segment plays, and which way. A freeze has no sound. */
  speed: Speed;
}

/** How the sound of a clip is read along its path (`retime::Sounding`): which segment each sample of the timeline is of, and the place that it reads. */
export class Sounding {
  constructor(
    private readonly segments: readonly Segment[],
    private readonly unit: number,
    private readonly rate: Rate,
    private readonly sampleRate: number,
    /** The frame of the clip at which its path is lowest: the first of them, where there are several. */
    readonly frame: number,
    /** The sample of the timeline that that frame starts at, rounded down. */
    private readonly sample: bigint,
    /** The first sample of each segment but the first. */
    private readonly starts: readonly bigint[],
  ) {}

  /** The parts of the sound, one for each segment of the clip, in their order: those of the freezes too. */
  parts(): SoundPart[] {
    return this.segments.map((segment, index) => ({
      from: index === 0 ? -Infinity : Number(this.starts[index - 1]),
      to: index < this.starts.length ? Number(this.starts[index]) : Infinity,
      speed: segment.speed,
    }));
  }

  /** Which part sample `sample` of the timeline is of, by its place among `parts`. */
  partOf(sample: number): number {
    const wanted = BigInt(Math.floor(sample));
    return first(0, this.starts.length, index => this.starts[index] > wanted);
  }

  /** The place of the path that `sample` reads as `segment` goes, above the lowest, as a count of which `sampleRate * rate.den` make a unit of a frame. */
  private reads(segment: Segment, sample: bigint): bigint {
    const level = segment.step * (BigInt(this.frame) - BigInt(segment.from)) + segment.at;
    // A sample of a segment that goes backward reads the place where the sample ends.
    const along = sample + (segment.step < 0n ? 1n : 0n) - this.sample;
    return level * BigInt(this.sampleRate) * BigInt(this.rate.den) + segment.step * along * BigInt(this.rate.num);
  }

  /** How many of what `reads` counts make a second of the source. */
  private perSecond(): bigint { return BigInt(this.sampleRate) * BigInt(this.unit) * BigInt(Math.max(this.rate.num, 1)); }

  /**
   * The sample of the media of a clip that begins in it at `begins`, at `mediaRate` samples a
   * second, that sample `sample` of the timeline is read from, rounded down (`Sounding::source_sample`).
   */
  sourceSample(begins: Time, sample: number, mediaRate: number): number {
    const segment = this.segments[Math.min(this.partOf(sample), this.segments.length - 1)];
    const along = this.reads(segment, BigInt(Math.floor(sample))) * BigInt(mediaRate);
    // begins * mediaRate / scale + along / per, each part divided on its own, and their remainders added.
    const scale = BigInt(Math.max(begins.scale, 1));
    const per = this.perSecond();
    const into = BigInt(begins.ticks) * BigInt(mediaRate);
    const qIn = floorDiv(into, scale);
    const rIn = into - qIn * scale;
    const qAlong = floorDiv(along, per);
    const rAlong = along - qAlong * per;
    const carry = rIn * per + rAlong * scale >= scale * per ? 1n : 0n;
    return Number(within(qIn + qAlong + carry));
  }

  /** The time of the source that sample `sample` of the timeline reads, in seconds after where the clip begins in its media (`Sounding::seconds`). */
  seconds(sample: number): number {
    const segment = this.segments[Math.min(this.partOf(sample), this.segments.length - 1)];
    return Number(this.reads(segment, BigInt(Math.floor(sample)))) / Number(this.perSecond());
  }

  /** The samples of the timeline that read the time `seconds` of the source: one for each segment that comes by it, and none of a freeze (`Sounding::samples_reading`). */
  samplesReading(seconds: number): number[] {
    const wanted = seconds * Number(this.perSecond());
    const samples: number[] = [];
    this.parts().forEach((part, index) => {
      const segment = this.segments[index];
      if (segment.step === 0n) return;
      const step = Number(segment.step) * Math.max(this.rate.num, 1);
      const at = (wanted - Number(this.reads(segment, 0n))) / step;
      // The last sample that reads the time or what is before it: forward, rounded down.
      const sample = segment.step < 0n ? Math.ceil(at) : Math.floor(at);
      if (sample >= part.from && sample < part.to) samples.push(sample);
    });
    return samples;
  }
}

/**
 * The course of a clip of `length` frames with the speed points `retime`, or null where they are
 * not those of such a clip, as `Retime::check` says: no point or too many, the first not at frame
 * 0, two not in the order of their frames, one at the end of the clip or after it, a speed that
 * plays and is not from 1/100 to 100 either way, or speeds with no unit that 32 bits hold.
 */
export function courseOf(retime: Retime, length: number): Course | null {
  const points = retime.points;
  if (!Number.isSafeInteger(length) || points.length === 0 || points.length > MOST_POINTS || points[0].frame !== 0) return null;
  let unit = 1n;
  for (let index = 0; index < points.length; index += 1) {
    const point = points[index];
    if (!Number.isSafeInteger(point.frame) || (index > 0 && points[index - 1].frame >= point.frame) || !speedAllowed(point.speed)) return null;
    const den = BigInt(point.speed.den);
    unit = unit / gcd(unit, den) * den;
    if (unit > U32_MAX) return null;
  }
  if (points[points.length - 1].frame >= length) return null;
  const segments: Segment[] = [];
  let [at, lowest, highest] = [0n, 0n, 0n];
  points.forEach((point, index) => {
    const to = index + 1 < points.length ? points[index + 1].frame : length;
    const step = BigInt(point.speed.num) * (unit / BigInt(point.speed.den));
    segments.push({ from: point.frame, to, speed: point.speed, at, step });
    at += step * BigInt(to - point.frame);
    if (at < lowest) lowest = at;
    if (at > highest) highest = at;
  });
  for (const segment of segments) segment.at -= lowest;
  return new Course(segments, Number(unit), highest - lowest, length);
}

/** What of a clip its course is counted from. */
export type Retimable = Pick<Clip, 'length' | 'speed' | 'reverse' | 'retime'>;

/** The courses counted out, by the speed points that they are of: a project is never changed in place, so those of a clip stay the same object until they change. */
const courses = new WeakMap<Retime, { length: number; course: Course | null }>();

/**
 * The course of a clip that has speed points, or null for a clip of one speed (and for speed points
 * that are not those of the clip, which no operation leaves so: such a clip is read at its speed,
 * as `span::Reading` reads it). It is counted out once for the speed points and the length of a
 * clip, and kept: what draws a clip asks for it many times.
 */
export function retimeCourse(clip: Retimable): Course | null {
  const retime = clip.retime;
  if (!retime) return null;
  const kept = courses.get(retime);
  if (kept && kept.length === clip.length) return kept.course;
  const course = courseOf(retime, clip.length);
  courses.set(retime, { length: clip.length, course });
  return course;
}

/** The course of any clip: along its speed points, or, for a clip of one speed, that of one speed point of that speed and direction. Null for a clip of no frame. */
export function clipCourse(clip: Retimable): Course | null {
  return courseOf(clip.retime ?? constantRetime(clip.speed, clip.reverse), clip.length);
}

/** Whether a clip plays its source backward at `frame` of its own frames: its direction, or that of the segment of its speed points that the frame is in. */
export function backwardAt(clip: Retimable, frame: number): boolean {
  const course = retimeCourse(clip);
  return course ? course.speedAt(Math.floor(frame)).num < 0 : clip.reverse;
}

/** The sample of a timeline of `rate` and `sampleRate` that frame `frame` starts at, rounded down (`resolve::samples_at`). */
export function samplesAt(frame: number, rate: Rate, sampleRate: number): number {
  return Number(within(floorDiv(BigInt(frame) * BigInt(sampleRate) * BigInt(rate.den), BigInt(Math.max(rate.num, 1)))));
}

/** A piece of the sound of a clip: one of its segments that plays, as the engine hears it (`resolve::AudioPiece`). */
export interface SoundPiece {
  /** The frames of the clip that the segment lasts, counted from the first frame of the clip. */
  frames: [number, number];
  /** The first sample of the timeline that it sounds on, and the sample after the last. */
  from: number;
  to: number;
  /** The sample of the media, at the rate of its sound, that sample `from` is read from. */
  sourceStart: number;
  /** How fast it plays, without its direction. */
  speed: Speed;
  /** Whether it plays backward. */
  reverse: boolean;
}

/**
 * The pieces of the sound of a clip on a timeline of `rate` frames and `sampleRate` samples a
 * second, cut from media of `mediaRate` samples a second (`resolve::pieces_along`): one for each
 * segment that plays, on the samples that are of it, and none for a freeze. A clip of one speed
 * gives one piece. `window` are the samples of the timeline that the clip sounds on, where they are
 * not those of its own frames: with the frames that a crossfade plays of it, or a part of them.
 */
export function soundPieces(clip: Pick<Clip, 'start' | 'source'> & Retimable, rate: Rate, sampleRate: number, mediaRate: number, window?: [number, number]): SoundPiece[] {
  const course = clipCourse(clip);
  if (!course || clip.source.kind !== 'media') return [];
  const begins = clip.source.in;
  const [start, end] = window ?? [samplesAt(clip.start, rate, sampleRate), samplesAt(clip.start + clip.length, rate, sampleRate)];
  const sounding = course.sounding(clip.start, rate, sampleRate);
  return sounding.parts().flatMap((part, index) => {
    const from = Math.max(part.from, start);
    const to = Math.min(part.to, end);
    if (part.speed.num === 0 || from >= to) return [];
    const segment = course.segments[index];
    return [{
      frames: [segment.from, segment.to] as [number, number], from, to, sourceStart: sounding.sourceSample(begins, from, mediaRate),
      speed: { num: Math.abs(part.speed.num), den: part.speed.den }, reverse: part.speed.num < 0,
    }];
  });
}

// --- The changes of the Retime Controls that the page works out itself.

/** A speed in whole hundredths of the speed of the source, as the fraction that the engine keeps, in its lowest terms; below 0 backward, 0 a freeze. */
export function speedOfPercent(percent: number): Speed {
  if (percent === 0) return { num: 0, den: 1 };
  const shared = Number(gcd(BigInt(Math.abs(percent)), 100n));
  return { num: percent / shared, den: 100 / shared };
}

/** The slowest and the fastest that a segment plays, in hundredths of the speed of the source. */
const SLOWEST = 1;
const FASTEST = 10_000;

/**
 * `rise / (frames * unit)` as whole hundredths of the speed of the source: the speed at which a
 * segment goes `rise` units of a frame in `frames` frames, to the nearest hundredth that a segment
 * may have. A segment that goes nowhere is a freeze; one that goes somewhere plays, at 1% at the least.
 */
function percentOver(rise: bigint, frames: number, unit: number): number {
  if (rise === 0n || frames <= 0) return 0;
  const over = BigInt(frames) * BigInt(unit);
  // Rounded to the nearest whole number, a half away from nought.
  const size = Number((abs(rise) * 200n + over) / (over * 2n));
  const held = Math.min(FASTEST, Math.max(SLOWEST, size));
  return rise < 0n ? -held : held;
}

/**
 * The speed, in whole hundredths of the speed of the source and without a sign, at which a segment
 * shows what it shows in `frames` frames in place of those that it lasts: what a drag of the upper
 * handle of a speed point asks for (`set_segment_speed`). Null for a freeze, which shows nothing
 * that could last longer or shorter.
 */
export function percentToLast(segment: Segment, frames: number, unit: number): number | null {
  if (segment.step === 0n || frames < 1) return null;
  return Math.abs(percentOver(segment.step * BigInt(segment.to - segment.from), frames, unit));
}

/**
 * The speed points of a clip with the speed point that begins segment `index` moved to frame `to`
 * of the clip, the clip lasting as long as it did: what a drag of the lower handle of a speed point
 * asks for. The path keeps the place that it had at the speed point, and those where the segment
 * before it begins and where the segment itself ends, so the speeds either side change; each is
 * brought to a whole hundredth of the speed of the source, which the engine takes whatever the
 * other speeds of the clip are, so the places after the two segments may come a little sooner or
 * later than they did. Null where there is no such speed point to move (the first is at frame 0),
 * or `to` is not strictly between the speed point before it and the end of its segment.
 */
export function movedPoint(course: Course, index: number, to: number): Retime | null {
  const before = course.segments[index - 1];
  const moved = course.segments[index];
  if (index < 1 || !moved || !Number.isSafeInteger(to) || to <= before.from || to >= moved.to) return null;
  const left = percentOver(moved.at - before.at, to - before.from, course.unit);
  const right = percentOver(placeIn(moved, BigInt(moved.to)) - moved.at, moved.to - to, course.unit);
  const points: SpeedPoint[] = course.segments.map((segment, at) => (
    at === index - 1 ? { frame: segment.from, speed: speedOfPercent(left) }
      : at === index ? { frame: to, speed: speedOfPercent(right) }
        : { frame: segment.from, speed: segment.speed }));
  return { points };
}
