/**
 * The frame that the keys of a clip's grades count at a frame of its timeline, as the engine finds
 * it (PROTOCOL.md, "Frames of keys"; `KeyClock` of engine/crates/model/src/keyframes.rs): a clip of
 * media with a picture that is not a still counts the frames of its media, at the media's own rate,
 * the number of the picture it shows there, through its speed and its direction, or along its speed
 * points where it has them (`../retime`); a title, a colour, a still and media without a picture
 * count the frames of the clip from its first, 0. DaVinci
 * Resolve ties the keys of the Color page to the source of a clip so (Reference Manual 20,
 * pp. 3383, 3393), so a trim, a slip or a change of speed leaves a key on its picture.
 *
 * Worked out in integers as the engine does (`span::source_tick` and `span::media_frame`), so that a
 * key the palettes set is at the frame the engine reads it at, not one beside it.
 */
import type { Clip, Media, Rate } from '../engine';
import { retimeCourse } from '../retime';

/** The frame of the keys of a clip's grades at a frame of its timeline. */
export type KeyClock = (frame: number) => number;

/** `a / b` rounded down, as Rust's `div_euclid` for a `b` above 0. */
function floorDiv(a: bigint, b: bigint): bigint {
  const q = a / b;
  return a % b < 0n ? q - 1n : q;
}

/**
 * The ticks of `scale` that `frames` frames of `rate` last at `speed` (`span::ticks_of_scaled`):
 * `frames * rate.den * scale * speed.num / (speed.den * rate.num)`, rounded down.
 */
export function ticksOfScaled(frames: number, speed: { num: number; den: number }, rate: Rate, scale: number): bigint {
  const whole = BigInt(frames) * BigInt(rate.den) * BigInt(scale);
  const num = BigInt(speed.num);
  const den = BigInt(speed.den) * BigInt(rate.num);
  const q = floorDiv(whole, den);
  const r = whole - q * den;
  // Truncated, as Rust's `/` of i128 truncates.
  return q * num + (r * num) / den;
}

/**
 * The frame of a picture of `rate` that the place `tick` is in, for media `duration` long
 * (`span::media_frame`): `ceil((tick + 1) * num / (scale * den)) - 1`, the place kept within the
 * media.
 */
export function mediaFrame(tick: bigint, rate: Rate, duration: { ticks: number; scale: number }): number {
  const last = BigInt(Math.max(duration.ticks - 1, 0));
  const kept = tick < 0n ? 0n : tick > last ? last : tick;
  const under = BigInt(duration.scale) * BigInt(rate.den);
  if (under === 0n) return 0;
  return Number(((kept + 1n) * BigInt(rate.num) - 1n) / under);
}

/** What of a clip the clock of its keys is counted from. */
export type ClockedClip = Pick<Clip, 'start' | 'length' | 'source' | 'speed' | 'reverse' | 'retime'>;

/** The clock of the keys of `clip`, on a timeline of `rate`, cut from `media` (the media it names, where it is one). */
export function keyClockOf(clip: ClockedClip, rate: Rate, media: Media | undefined): KeyClock {
  const video = clip.source.kind === 'media' && media !== undefined && media.id === clip.source.media ? media.facts.video : null;
  if (!video || video.still || clip.source.kind !== 'media') return frame => frame - clip.start;
  const begins = BigInt(clip.source.in.ticks);
  const { duration } = media!.facts;
  // A clip with speed points shows the places of their path: one picture all through a freeze, and earlier pictures frame by frame where a segment plays backward.
  const course = retimeCourse(clip);
  if (course) return frame => mediaFrame(course.tick(begins, frame - clip.start, rate, duration.scale), video.rate, duration);
  return (frame) => {
    const k = frame - clip.start;
    const shown = clip.reverse ? clip.length - 1 - k : k;
    return mediaFrame(begins + ticksOfScaled(shown, clip.speed, rate, duration.scale), video.rate, duration);
  };
}

/**
 * Frames of a timeline, from `first` to `last`, both among them, over which the clock of a clip
 * goes one way: up, down, or not at all. The clock of a clip of one speed goes one way along the
 * whole clip; that of a clip with speed points goes one way along each of its segments.
 */
export interface KeyRun { first: number; last: number }

/** The runs of a clip with speed points, one for each segment in the order of the timeline; null for a clip of one speed, whose clock goes one way from end to end. */
export function keyRunsOf(clip: ClockedClip): KeyRun[] | null {
  const course = retimeCourse(clip);
  return course ? course.segments.map(segment => ({ first: clip.start + segment.from, last: clip.start + segment.to - 1 })) : null;
}

/** The least and the greatest frame of keys that a clock shows over runs: those at their ends, since the clock goes one way along each. */
export function keyRangeOf(clock: KeyClock, runs: readonly KeyRun[]): [number, number] {
  const ends = runs.flatMap(run => [clock(run.first), clock(run.last)]);
  return [Math.min(...ends), Math.max(...ends)];
}

/** The least and the greatest frame of keys that a clip shows, wherever in the clip: for a clip of one speed, those of its first frame and its last. */
export function shownKeyRange(clip: ClockedClip, clock: KeyClock): [number, number] {
  return keyRangeOf(clock, keyRunsOf(clip) ?? [{ first: clip.start, last: clip.start + Math.max(clip.length, 1) - 1 }]);
}

/**
 * The first frame of the timeline at which a clock that goes one way along each of `runs` shows
 * the frame of keys `keyFrame`: in the first run that comes by it, the first frame that shows it,
 * or the first that shows what is past it where the clip plays too fast to show it (as `keyTimes`
 * of `inspector/clipKeys` finds the frames of a clip of one speed). A frame of keys that is skipped
 * at a speed point, between what the last frame of a run shows and what the first frame of the next
 * shows, is at the first frame of the next. Null where no run comes by it.
 *
 * A clip that turns comes by a picture a second time, and one that slows shows it on several
 * frames: this is the first of them all, as `Course.frameOf` gives the first frame for a place.
 */
export function firstShowing(clock: KeyClock, runs: readonly KeyRun[], keyFrame: number): number | null {
  for (const [index, run] of runs.entries()) {
    const begins = clock(run.first);
    const ends = clock(run.last);
    if (index > 0) {
      const before = clock(runs[index - 1].last);
      if ((before < keyFrame && keyFrame < begins) || (before > keyFrame && keyFrame > begins)) return run.first;
    }
    if (keyFrame < Math.min(begins, ends) || keyFrame > Math.max(begins, ends)) continue;
    const rising = ends >= begins;
    let from = run.first;
    let to = run.last;
    while (from < to) {
      const middle = Math.floor((from + to) / 2);
      const reached = rising ? clock(middle) >= keyFrame : clock(middle) <= keyFrame;
      if (reached) to = middle;
      else from = middle + 1;
    }
    return from;
  }
  return null;
}
