/**
 * The value of a setting with keys, and the eases of a key, are those that the engine finds, to
 * the last bit: the cases and what the engine found are written by the engine
 * (`engine/crates/model/tests/param_values.rs`).
 */
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Keyframe, Param } from '../src/studio/engine';
import { EASES, EASE_SHARE, easeOf, eased, valueAt, type Ease } from '../src/studio/keyframes/value';

interface Valued { name: string; param: Param; frames: number[]; values: number[] }
interface Eased { name: string; keys: Keyframe[]; index: number; is: Ease | null; ease: Ease; eased: Keyframe[]; then: Ease | null }
const written: { values: Valued[]; eases: Eased[] } = JSON.parse(
  readFileSync(new URL('../../engine/crates/model/tests/data/param-values.json', import.meta.url), 'utf8'),
);

describe('the value of a setting with keys', () => {
  it('is the one that the engine finds, to the last bit, for every case it wrote', () => {
    expect(written.values.length).toBeGreaterThanOrEqual(11);
    for (const each of written.values) {
      expect(each.frames.length, each.name).toBe(each.values.length);
      each.frames.forEach((frame, n) => {
        const found = valueAt(each.param, frame);
        expect(Object.is(found, each.values[n]), `${each.name} at ${frame}: ${found}, the engine ${each.values[n]}`).toBe(true);
      });
    }
  });

  it('holds before the first key and after the last, and between two keys goes as the first says', () => {
    const keys: Keyframe[] = [
      { frame: 0, value: 2, interp: 'hold' },
      { frame: 10, value: 4, interp: 'linear' },
      { frame: 20, value: 8, interp: { bezier: { out: [EASE_SHARE, 0], in: [EASE_SHARE, 0] } } },
      { frame: 30, value: 0, interp: 'linear' },
    ];
    const at = (frame: number) => valueAt({ keys }, frame);
    expect([at(-5), at(0), at(9.99), at(10), at(15), at(20), at(30), at(99)]).toEqual([2, 2, 2, 4, 6, 8, 0, 0]);
    // Eased both ways, a stretch is half done at its middle, and slow at its ends.
    expect(at(25)).toBeCloseTo(4, 12);
    expect(8 - at(21)).toBeLessThan(0.3);
    expect(valueAt(7, 123)).toBe(7);
    expect(valueAt({ keys: [] }, 0)).toBe(0);
  });
});

describe('the ease of a key', () => {
  it('is set and read as the engine sets and reads it, for every key of every row in every way', () => {
    expect(written.eases.length).toBeGreaterThanOrEqual(36);
    for (const each of written.eases) {
      const label = `${each.name}, key ${each.index}, ${each.ease}`;
      expect(easeOf(each.keys, each.index) ?? null, label).toBe(each.is);
      const after = eased(each.keys, each.index, each.ease);
      expect(after, label).toEqual(each.eased);
      expect(easeOf(after, each.index) ?? null, label).toBe(each.then);
    }
  });

  it('leaves the keys it was given as they were, and does nothing where there is no such key', () => {
    const keys: Keyframe[] = [{ frame: 0, value: 0, interp: 'linear' }, { frame: 10, value: 3, interp: 'linear' }];
    const copy = structuredClone(keys);
    const after = eased(keys, 0, 'out');
    expect(keys).toEqual(copy);
    expect(after[0].interp).toEqual({ bezier: { out: [EASE_SHARE, 0], in: [EASE_SHARE, -3 * EASE_SHARE] } });
    expect(eased(keys, 2, 'in_out')).toEqual(copy);
    expect(eased(keys, -1, 'in_out')).toEqual(copy);
    expect(easeOf(keys, 2)).toBeUndefined();
    expect(EASES).toEqual(['linear', 'in', 'out', 'in_out']);
  });
});
