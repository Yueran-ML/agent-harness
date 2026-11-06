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

/**
 * The frame that an Out point marks, which is the last frame of the range and is in it, as in
 * DaVinci Resolve, where the Out point is the last frame of a clip (Reference Manual 20, pp. 760,
 * 824): a mark Out is kept as where the range ends, the frame after it (half open, as the engine
 * takes every range), and is shown as the frame before that.
 */
export function outFrame(end: number): number {
  return Math.max(0, Math.floor(end) - 1);
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

/**
 * The frame that a timecode typed into Go To Timecode asks for (Reference Manual 20, pp. 3746–3747):
 * digits fill the timecode from its frames up (`1000` is 00:00:10:00), a `.` stands for two zeros,
 * and a leading `+` or `-` moves from `frame` by that much. Colons may be typed or left out. Null
 * where the text is not one.
 */
export function enteredFrame(text: string, frame: number, rate: Rate): number | null {
  const entry = /^([+-]?)([0-9.:;]+)$/.exec(text.replace(/\s+/g, ''));
  if (!entry) return null;
  const digits = entry[2].replace(/[:;]/g, '').replace(/\./g, '00');
  if (!digits || digits.length > 8) return null;
  const [hours, minutes, seconds, frames] = digits.padStart(8, '0').match(/../g)!.map(Number);
  const fps = nominalFps(rate);
  const count = ((hours * 60 + minutes) * 60 + seconds) * fps + frames;
  const to = entry[1] === '+' ? frame + count : entry[1] === '-' ? frame - count : count;
  return Math.max(0, to);
}
