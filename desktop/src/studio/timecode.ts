/**
 * Frames, rates and times as the studio writes them: timecode of `HH:MM:SS:FF`, rates as an
 * editor writes them (25, 29.97, 23.976), and the facts of media in a line.
 */
import type { Media, Rate, Time } from './engine';

/** The rate that a source of sound alone is played at where the project has no timeline (see engine/PROTOCOL.md). */
export const SOUND_RATE: Rate = { num: 25, den: 1 };

/** The whole number of frames that a second of timecode counts: 30 for 29.97, 24 for 23.976. */
export function nominalFps(rate: Rate): number {
  if (!(rate.num > 0) || !(rate.den > 0)) return 25;
  return Math.max(1, Math.round(rate.num / rate.den));
}

const two = (value: number) => String(value).padStart(2, '0');

/**
 * The timecode of a frame, `HH:MM:SS:FF`, counted from frame 0. Timecode of 29.97 and 59.94 is
 * counted without dropping frames: 30 or 60 frames to a second of timecode.
 */
export function timecode(frame: number, rate: Rate): string {
  const fps = nominalFps(rate);
  const whole = Math.max(0, Math.floor(frame));
  const frames = whole % fps;
  const seconds = Math.floor(whole / fps);
  return `${two(Math.floor(seconds / 3600))}:${two(Math.floor(seconds / 60) % 60)}:${two(seconds % 60)}:${two(frames)}`;
}

/** A rate as an editor writes it: 25, 29.97, 59.94, 23.976. */
export function rateText(rate: Rate): string {
  if (!(rate.den > 0)) return '0';
  return String(Number((rate.num / rate.den).toFixed(3)));
}

/** How many frames a time lasts at a rate, rounded as the engine counts the frames of media. */
export function framesOf(time: Time, rate: Rate): number {
  if (!(time.scale > 0) || !(rate.den > 0)) return 0;
  return Math.max(0, Math.round((time.ticks / time.scale) * (rate.num / rate.den)));
}

/** The rate that the frames of media are numbered at: that of its picture, or for sound alone that of the first timeline. */
export function rateOf(media: Media | null | undefined, timelines: readonly { rate: Rate }[] = []): Rate {
  return media?.facts.video?.rate ?? timelines[0]?.rate ?? SOUND_RATE;
}

/** The name of the file of media, from its path as the system writes it. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? path;
}
