import { describe, expect, it } from 'vitest';
import vectors from '../../tests/fixtures/shotmatch.json';
import { figures, mapped, neutralMatch, percentile, solve, unmapped, type Figures } from '../src/studio/color/shotmatch';

describe('the desktop and agent match shots alike', () => {
  for (const vector of vectors) it(`matches the Python reference for ${vector.name}`, () => {
    const solved = solve(vector.clip as Figures, vector.target as Figures);
    for (const key of ['gamma', 'gain', 'offset'] as const) {
      for (const channel of ['r', 'g', 'b'] as const) expect(solved.match[key][channel]).toBeCloseTo(vector.match[key][channel], 6);
    }
    expect(solved.match.contrast).toBeCloseTo(vector.match.contrast, 6);
    expect(solved.match.pivot).toBeCloseTo(vector.match.pivot, 10);
    expect(solved.before).toBeCloseTo(vector.before, 10);
    expect(solved.after).toBeCloseTo(vector.after, 8);
    expect(solved.limited).toEqual(vector.limited);
    if (vector.name.includes('neutral')) expect(solved.match).toEqual(neutralMatch());
    else expect(solved.after).toBeLessThan(solved.before);
  });

  it('reads levels from the bins and preserves the measured means', () => {
    const counts = Array<number>(1024).fill(0);
    counts[512] = 100;
    const frame = figures({ red: counts, green: counts, blue: counts }, [0.6, 0.5, 0.4]);
    expect(frame.levels[0][3]).toBeCloseTo(512 / 1023, 12);
    expect(frame.means).toEqual([0.6, 0.5, 0.4]);
    expect(percentile([0, 4, 0, 0], 1)).toBeCloseTo(0.5, 12);
    expect(percentile([0, 0, 0], 0.5)).toBe(0);
  });

  it('removes a previous match before finding its replacement', () => {
    const frame = vectors[0].target as Figures;
    const match = { ...neutralMatch(), gamma: { r: 1.1, g: 0.95, b: 1 }, gain: { r: 1.05, g: 0.9, b: 1.05 }, offset: { r: 0.01, g: 0, b: 0.01 } };
    const restored = unmapped(mapped(frame, match), match);
    for (let channel = 0; channel < 3; channel++) {
      frame.levels[channel].forEach((value, i) => expect(restored.levels[channel][i]).toBeCloseTo(value, 8));
      expect(restored.means[channel]).toBeCloseTo(frame.means[channel], 8);
    }
  });
});
