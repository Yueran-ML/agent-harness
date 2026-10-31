/**
 * The frame that the keys of a clip's grades count at a frame of its timeline, as the engine finds
 * it (PROTOCOL.md, "Frames of keys"; `KeyClock` of engine/crates/model/src/keyframes.rs): a clip of
 * media with a picture that is not a still counts the frames of its media, at the media's own rate,
 * the number of the picture it shows there, through its speed and its direction; a title, a colour,
 * a still and media without a picture count the frames of the clip from its first, 0. DaVinci
 * Resolve ties the keys of the Color page to the source of a clip so (Reference Manual 20,
 * pp. 3383, 3393), so a trim, a slip or a change of speed leaves a key on its picture.
 *
 * Worked out in integers as the engine does (`span::source_tick` and `span::media_frame`), so that a
 * key the palettes set is at the frame the engine reads it at, not one beside it.
 */
import type { Clip, Media, Rate } from '../engine';

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

/** The clock of the keys of `clip`, on a timeline of `rate`, cut from `media` (the media it names, where it is one). */
export function keyClockOf(clip: Pick<Clip, 'start' | 'length' | 'source' | 'speed' | 'reverse'>, rate: Rate, media: Media | undefined): KeyClock {
  const video = clip.source.kind === 'media' && media !== undefined && media.id === clip.source.media ? media.facts.video : null;
  if (!video || video.still || clip.source.kind !== 'media') return frame => frame - clip.start;
  const begins = BigInt(clip.source.in.ticks);
  const { duration } = media!.facts;
  return (frame) => {
    const k = frame - clip.start;
    const shown = clip.reverse ? clip.length - 1 - k : k;
    return mediaFrame(begins + ticksOfScaled(shown, clip.speed, rate, duration.scale), video.rate, duration);
  };
}
