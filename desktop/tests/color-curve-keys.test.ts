/**
 * The points of a curve whose numbers have keyframes (PROTOCOL.md, "Grades"; T5 of phase 4): what
 * the editor of a curve shows of them at the playhead, and what an edit there makes of them.
 */
import { describe, expect, it } from 'vitest';
import type { CurveKnot, CurvePoint, Keyframe, Param } from '../src/studio/engine';
import { curvesChanged, editedPoints, neutralCorrector, pointsKeyed, shownPoints } from '../src/studio/color/corrector';

const keys = (...list: [number, number][]): Param => ({ keys: list.map(([frame, value]): Keyframe => ({ frame, value, interp: 'linear' })) });

/** Three points, the middle one going up and to the right from frame 0 to frame 10. */
const moving = (): CurveKnot[] => [[0, 0], [keys([0, 0.25], [10, 0.75]), keys([0, 0.25], [10, 0.5])], [1, 1]];

describe('the points of a curve that have keyframes', () => {
  it('are shown where the key at or before the playhead has them, and not in between', () => {
    expect(shownPoints(moving(), -5)).toEqual([[0, 0], [0.25, 0.25], [1, 1]]);
    expect(shownPoints(moving(), 0)).toEqual([[0, 0], [0.25, 0.25], [1, 1]]);
    // The controls do not follow the values between two keys (p. 3384).
    expect(shownPoints(moving(), 5)).toEqual([[0, 0], [0.25, 0.25], [1, 1]]);
    expect(shownPoints(moving(), 10)).toEqual([[0, 0], [0.75, 0.5], [1, 1]]);
    expect(shownPoints([[0, 0], [1, 1]], 99)).toEqual([[0, 0], [1, 1]]);
    expect(pointsKeyed(moving())).toBe(true);
    expect(pointsKeyed([[0, 0], [1, 1]])).toBe(false);
  });

  it('take a point that is moved into its key at the playhead, and keep the other keys', () => {
    const next: CurvePoint[] = [[0, 0], [0.3, 0.6], [1, 0.9]];
    expect(editedPoints(moving(), 4, next)).toEqual([
      [0, 0],
      [keys([0, 0.3], [10, 0.75]), keys([0, 0.6], [10, 0.5])],
      [1, 0.9],
    ]);
    expect(editedPoints(moving(), 12, next)).toEqual([
      [0, 0],
      [keys([0, 0.25], [10, 0.3]), keys([0, 0.25], [10, 0.6])],
      [1, 0.9],
    ]);
    // A curve without keys becomes the points that its editor shows.
    expect(editedPoints([[0, 0], [1, 1]], 4, next)).toEqual(next);
  });

  it('keep the keys of the points that are there when one is added or taken away', () => {
    // A point is added before the one that moves, and one after it.
    const before = shownPoints(moving(), 0);
    const addedBefore: CurvePoint[] = [before[0], [0.1, 0.05], before[1], before[2]];
    expect(editedPoints(moving(), 0, addedBefore)).toEqual([[0, 0], [0.1, 0.05], moving()[1], [1, 1]]);
    const addedAfter: CurvePoint[] = [before[0], before[1], [0.6, 0.7], before[2]];
    expect(editedPoints(moving(), 0, addedAfter)).toEqual([[0, 0], moving()[1], [0.6, 0.7], [1, 1]]);
    // The first point is taken away, and then the one that moves.
    expect(editedPoints(moving(), 0, [before[1], before[2]])).toEqual([moving()[1], [1, 1]]);
    expect(editedPoints(moving(), 0, [before[0], before[2]])).toEqual([[0, 0], [1, 1]]);
    // A reset, or the default anchors, puts the points there whole.
    expect(editedPoints(moving(), 0, [[0, 0], [0.2, 0.2], [0.4, 0.4], [0.6, 0.6], [1, 1]])).toEqual([[0, 0], [0.2, 0.2], [0.4, 0.4], [0.6, 0.6], [1, 1]]);
    expect(editedPoints(moving(), 0, [])).toEqual([]);
  });

  it('count as a change of the node, wherever the playhead is', () => {
    const corrector = neutralCorrector();
    expect(curvesChanged(corrector)).toBe(false);
    // The diagonal at frame 0, and another line later.
    corrector.curves.r.points = [[0, 0], [1, keys([0, 1], [10, 0.5])]];
    expect(shownPoints(corrector.curves.r.points, 0)).toEqual([[0, 0], [1, 1]]);
    expect(curvesChanged(corrector)).toBe(true);
  });
});
