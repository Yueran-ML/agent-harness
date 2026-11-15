import type { AudioKey, AudioParam, Clip, Rate, Time } from '../engine';
import { sourceInterval } from './waveforms';
import { clamp } from '../audio/taper';
import { retimeCourse } from '../retime';
export const seconds = (time: Time) => time.ticks / time.scale;
export const sampleTime = (time: number, rate: number): Time => ({ ticks: Math.round(time * rate), scale: rate });
/** sourceInterval supplies the existing speed/reverse mapping, including its source-in offset, and the path of a clip with speed points. */
export function sourceAtFrame(clip: Clip, rate: Rate, frame: number): number {
  return sourceInterval(clip, rate, frame, frame)[0];
}
/**
 * The frame of the timeline at which a clip reads the time `source` of its media, which need not
 * be a whole frame. A clip with speed points may come by a time more than once, or hold it through
 * a freeze: this is the first frame at which a segment that plays reads it. A time that the clip
 * never comes by is before its first frame, where nothing of the clip is drawn.
 */
export function frameAtSource(clip: Clip, rate: Rate, source: number): number {
  const start = clip.source.kind === 'media' ? seconds(clip.source.in) : 0;
  const course = retimeCourse(clip);
  if (course) {
    const frames = course.framesAt((source - start) / (rate.den / rate.num));
    return frames.length > 0 ? clip.start + frames[0] : clip.start - 1;
  }
  const offset = (source - start) / (rate.den / rate.num * clip.speed.num / clip.speed.den);
  return clip.start + (clip.reverse ? clip.length - offset : offset);
}
/** The model floors source_span in source.in.scale before checking within_source. Quantize
 * inward with exact integer arithmetic; the continuous waveform interval can be slightly longer. */
export function sourceSampleBounds(clip: Clip, rate: Rate, sampleRate: number): [number, number] {
  const origin = clip.source.kind === 'media' ? clip.source.in : { ticks: 0, scale: 1 };
  const scale = BigInt(origin.scale), samples = BigInt(sampleRate);
  // With speed points the clip shows from the lowest place of their path, where it begins in its media, to the highest (`span::source_span`).
  const course = retimeCourse(clip);
  const span = course ? course.span(rate, origin.scale)
    : BigInt(clip.length) * BigInt(rate.den) * BigInt(clip.speed.num) * scale / (BigInt(rate.num) * BigInt(clip.speed.den));
  const low = BigInt(origin.ticks) * samples, high = (BigInt(origin.ticks) + span) * samples;
  return [Number(low / scale + (low % scale > 0n ? 1n : 0n)), Number(high / scale - (high % scale < 0n ? 1n : 0n))];
}
export function sourceSampleTime(clip: Clip, rate: Rate, value: number, sampleRate: number): Time {
  const [low, high] = sourceSampleBounds(clip, rate, sampleRate);
  return { ticks: clamp(Math.round(value * sampleRate), low, high), scale: sampleRate };
}
export function keyTime(clip: Clip, rate: Rate, frame: number, sampleRate: number, snap: boolean): Time {
  const value = sourceAtFrame(clip, rate, clamp(snap ? Math.round(frame) : frame, clip.start, clip.start + clip.length));
  return sourceSampleTime(clip, rate, value, sampleRate);
}
/** Same Hermite tangents as model/audio/param.rs; interp belongs to the key it enters/leaves. */
export function audioValue(param: AudioParam, at: number): number {
  if (typeof param === 'number') return param;
  const keys = param.keys;
  if (at <= seconds(keys[0].at)) return keys[0].value;
  const last = keys.at(-1)!;
  if (at >= seconds(last.at)) return last.value;
  const index = keys.findIndex(key => seconds(key.at) > at);
  const a = keys[index - 1], b = keys[index];
  const t = (at - seconds(a.at)) / (seconds(b.at) - seconds(a.at));
  const t2 = t * t, t3 = t2 * t, rise = b.value - a.value;
  const leaves = a.interp === 'ease_out' ? 0 : 1, arrives = b.interp === 'ease_in' ? 0 : 1;
  return (2 * t3 - 3 * t2 + 1) * a.value + (t3 - 2 * t2 + t) * leaves * rise
    + (-2 * t3 + 3 * t2) * b.value + (t3 - t2) * arrives * rise;
}
export function fadeValue(t: number, curve = 0, skew = 0): number {
  const x = clamp(t, 0, 1) ** (2 ** -skew);
  const edge = curve >= 0 ? Math.sin(Math.PI * x / 2) : 1 - Math.cos(Math.PI * x / 2);
  return x + Math.abs(curve) * (edge - x);
}
/** Paste offsets are in timeline seconds, so reverse and speed changes preserve audible timing. */
export interface CopiedAudioKey { offset: number; value: number; interp: AudioKey['interp'] }
