import { expect, it } from 'vitest';
import { dbToPosition, positionToDb, CLIP_GAIN, FADER_GAIN } from '../src/studio/audio/taper';
import { audioValue, sourceAtFrame, frameAtSource, sampleTime, fadeValue, keyTime } from '../src/studio/timeline/audioKeys';
import { clip } from './timeline-fixture';

it('shares invertible taper anchors while allowing the full clip range', () => {
  expect(dbToPosition(0, FADER_GAIN)).toBe(.75);
  expect(positionToDb(1, CLIP_GAIN)).toBe(24);
  expect(dbToPosition(0, CLIP_GAIN)).toBeCloseTo((.75 - .004) / (1.28 - .004));
  for (const range of [FADER_GAIN, CLIP_GAIN]) for (const value of [range[0], -60, -20, 0, 5, 10, range[1]])
    expect(positionToDb(dbToPosition(value, range), range)).toBeCloseTo(value, 10);
});
it('maps sped and reversed clips through source time and samples, never timeline frames as samples', () => {
  const c = clip('x', 10, 60, { source: { kind: 'media', media: 'm1', in: { ticks: 48000, scale: 48000 } }, speed: { num: 2, den: 1 }, reverse: true });
  const rate = { num: 30, den: 1 };
  expect(sourceAtFrame(c, rate, 25)).toBe(4);
  expect(frameAtSource(c, rate, 4)).toBe(25);
  expect(sampleTime(1 + 1 / 44100, 44100)).toEqual({ ticks: 44101, scale: 44100 });
});
it('matches engine Hermite interpolation and endpoint holds', () => {
  const param = { keys: [{ at: { ticks: 1, scale: 1 }, value: -10, interp: 'ease_out' as const }, { at: { ticks: 3, scale: 1 }, value: 0, interp: 'linear' as const }] };
  expect(audioValue(param, 0)).toBe(-10);
  expect(audioValue(param, 2)).toBe(-6.25);
  expect(audioValue(param, 4)).toBe(0);
  expect(audioValue({ keys: [param.keys[0], { ...param.keys[1], interp: 'ease_in' }] }, 1.5)).toBe(-8.4375);
  expect(fadeValue(.5, 0, 0)).toBe(.5);
  expect(fadeValue(.5, 1, 0)).toBeCloseTo(Math.SQRT1_2);
});
it.each([false, true])('quantizes keys inside source_span truncated in the source scale, including speed (reverse=%s)', reverse => {
  const c = clip('x', 10, 61, { reverse, speed: { num: 3, den: 2 }, source: { kind: 'media', media: 'm1', in: { ticks: 1, scale: 1000 } } });
  // 61/24 * 1.5 = 3.8125 s, but source_span is 3812 ticks in the source's 1000 Hz clock.
  const at = keyTime(c, { num: 24, den: 1 }, reverse ? 10 : 71, 44100, false);
  expect(BigInt(at.ticks) * 1000n <= 3813n * BigInt(at.scale)).toBe(true);
  expect(at).toEqual({ ticks: 168153, scale: 44100 });
});
