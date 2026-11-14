/**
 * The frame that the palettes put a key at, as the engine reads it (PROTOCOL.md, "Frames of keys"):
 * for a clip of media, the frame of its media whose picture is shown, through its speed and its
 * direction; for a title, a colour, a still or media without a picture, the frame of the clip. The
 * cases and their frames are written by the engine (`engine/crates/model/tests/key_clocks.rs`).
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Clip, Media, Rate } from '../src/studio/engine';
import { keyClockOf, mediaFrame, ticksOfScaled } from '../src/studio/color/keyFrames';
import { placeOf } from '../src/studio/color/useNodeEdit';

interface Case { name: string; clip: Clip; media: Media | null; rate: Rate; frames: number[]; keys: number[] }
const cases: Case[] = JSON.parse(readFileSync(new URL('../../engine/crates/model/tests/data/key-clocks.json', import.meta.url), 'utf8'));

describe('the frames of keys', () => {
  it('are those the engine finds, for every case it wrote', () => {
    expect(cases.length).toBeGreaterThanOrEqual(12);
    for (const each of cases) {
      const clock = keyClockOf(each.clip, each.rate, each.media ?? undefined);
      expect(each.frames.map(clock), each.name).toEqual(each.keys);
    }
  });

  it('count the media through speed and direction, and the clip where there is no picture of its own', () => {
    const byName = (name: string) => cases.find(each => each.name === name)!;
    // Twice as fast and backward: the last frame of the clip shows its first picture.
    const fast = byName('twice as fast, backward');
    const clock = keyClockOf(fast.clip, fast.rate, fast.media!);
    const first = fast.clip.start;
    const last = first + fast.clip.length - 1;
    expect(clock(last)).toBe(25);
    expect(clock(last - 1)).toBe(27);
    expect(clock(first)).toBe(25 + 2 * (fast.clip.length - 1));
    // Half as fast: a picture is shown on two frames, with one key.
    const slow = byName('half as fast');
    const slowClock = keyClockOf(slow.clip, slow.rate, slow.media!);
    expect([0, 1, 2, 3].map(k => slowClock(slow.clip.start + k))).toEqual([6, 7, 7, 8]);
    // A title, a still: the frame of the clip, before its first too.
    for (const name of ['a title', 'a still', 'a colour', 'media without a picture']) {
      const each = byName(name);
      expect(keyClockOf(each.clip, each.rate, each.media ?? undefined)(each.clip.start + 4), name).toBe(4);
    }
    // Media other than the clip's is no media of it.
    const plain = byName('media at the timeline\'s rate');
    expect(keyClockOf(plain.clip, plain.rate, { ...plain.media!, id: 'other' })(plain.clip.start + 3)).toBe(3);
  });

  it('are worked out in integers, past what a double holds of a product', () => {
    // 2^53 / 3 frames at 7/3 of a timeline of 30000/1001, in ticks of 90000: the product needs more than 53 bits.
    const frames = 3_002_399_751_580_330;
    expect(ticksOfScaled(frames, { num: 7, den: 3 }, { num: 30000, den: 1001 }, 90000))
      .toBe((BigInt(frames) * 1001n * 90000n * 7n) / (3n * 30000n));
    expect(mediaFrame(-5n, { num: 25, den: 1 }, { ticks: 15360, scale: 15360 })).toBe(0);
    expect(mediaFrame(15360n * 9n, { num: 25, den: 1 }, { ticks: 15360, scale: 15360 })).toBe(24);
    expect(mediaFrame(613n, { num: 25, den: 1 }, { ticks: 15360, scale: 15360 })).toBe(0);
    expect(mediaFrame(614n, { num: 25, den: 1 }, { ticks: 15360, scale: 15360 })).toBe(1);
  });

  it('are where the palettes put a key: the clock of the clip for its grade and its group\'s, the timeline for its own', () => {
    const fast = cases.find(each => each.name === 'twice as fast, backward')!;
    const clock = keyClockOf(fast.clip, fast.rate, fast.media!);
    const clip = { ...fast.clip, group: 'g1' };
    const at = fast.clip.start + 3;
    expect(placeOf('clip', 't1', clip, at, clock)).toEqual({ target: { level: 'clip', timeline: 't1', clip: clip.id }, frame: clock(at) });
    expect(placeOf('pre_clip', 't1', clip, at, clock)).toEqual({ target: { level: 'pre_clip', group: 'g1' }, frame: clock(at) });
    expect(placeOf('timeline', 't1', clip, at, clock)).toEqual({ target: { level: 'timeline', timeline: 't1' }, frame: at });
    // Without a clock, the frame of the clip.
    expect(placeOf('clip', 't1', clip, at)?.frame).toBe(3);
  });
});
