/**
 * A large timeline for the performance tests: six video tracks and four audio tracks of a hundred
 * clips each, a thousand in all, whose sound has a waveform that is ready. The waveform is answered
 * at once, as the engine answers one that it has already worked out.
 */
import { vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Media, Timeline } from '../src/studio/engine';
import { clip, media } from './timeline-fixture';

export const VIDEO_TRACKS = 6;
export const AUDIO_TRACKS = 4;
export const PER_TRACK = 100;
/** Peaks a second, in the waveform that the engine gives. */
const PEAK_RATE = 20;

export function bigTimeline(): Timeline {
  const track = (kind: 'video' | 'audio', row: number) => ({
    id: `${kind[0]}${row}`, kind, name: `${kind === 'video' ? 'V' : 'A'}${row + 1}`, enabled: true, locked: false, transitions: [],
    clips: Array.from({ length: PER_TRACK }, (_, i) => clip(`${kind[0]}${row}_${i}`, i * 200, 180, { link: `l${row % AUDIO_TRACKS}_${i}` })),
  });
  return {
    id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
    tracks: [
      ...Array.from({ length: VIDEO_TRACKS }, (_, row) => track('video', row)),
      ...Array.from({ length: AUDIO_TRACKS }, (_, row) => track('audio', row)),
    ],
  } as Timeline;
}

/** Peaks for the whole of `media`: two channels, a wave whose height changes along it. */
function peaks(item: Media): string {
  const seconds = item.facts.duration.ticks / item.facts.duration.scale;
  const bins = Math.ceil(seconds * PEAK_RATE);
  const bytes = new Uint8Array(bins * 4);
  for (let bin = 0; bin < bins; bin++) {
    const height = Math.round(Math.abs(Math.sin(bin / 7)) * 120);
    for (let channel = 0; channel < 2; channel++) {
      bytes[(bin * 2 + channel) * 2] = (256 - height) & 255;
      bytes[(bin * 2 + channel) * 2 + 1] = height;
    }
  }
  let text = '';
  for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  return btoa(text);
}

/** A bridge whose waveforms are ready: the answer comes at once. */
export function readyBridge(): Pick<EngineBridge, 'call' | 'onEvent'> {
  const encoded = peaks(media);
  return {
    call: vi.fn(async (method: string) => method === 'media.waveform'
      ? { ok: true, result: { rate: PEAK_RATE, channels: 2, peaks: encoded } }
      : { ok: true, result: null }),
    onEvent: () => () => {},
  } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>;
}

/** The middle of a list of times, its 95th percentile and its largest. */
export function spread(times: readonly number[]): { p50: number; p95: number; max: number } {
  const sorted = [...times].sort((a, b) => a - b);
  const at = (part: number) => sorted[Math.min(sorted.length - 1, Math.ceil(part * sorted.length) - 1)];
  return { p50: at(0.5), p95: at(0.95), max: sorted[sorted.length - 1] };
}
