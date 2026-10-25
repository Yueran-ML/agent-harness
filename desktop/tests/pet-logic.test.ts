/** Which frame of its sheet a pet shows for what the task is doing. */
import { describe, expect, it } from 'vitest';
import { animationsOf, BUBBLE_MS, bubbleShows, cellOf, FAILED_MS, rowOf, ROWS, WAVE_MS } from '../src/pet/animate';

describe('the row that a pet shows', () => {
  it('is that of its state, and waves for a while when it is done, and shows a failure for a while', () => {
    expect(['idle', 'thinking', 'review', 'waiting'].map(state => rowOf(state as never, 1e9))).toEqual(['idle', 'thinking', 'review', 'waiting']);
    expect([rowOf('done', 0), rowOf('done', WAVE_MS - 1), rowOf('done', WAVE_MS)]).toEqual(['wave', 'wave', 'idle']);
    expect([rowOf('failed', 0), rowOf('failed', FAILED_MS - 1), rowOf('failed', FAILED_MS)]).toEqual(['failed', 'failed', 'idle']);
  });

  it('has a bubble while the pet waits, and for a while when something has happened', () => {
    expect([bubbleShows('waiting', 1e9), bubbleShows('idle', 0), bubbleShows('thinking', 0), bubbleShows('review', 0)]).toEqual([true, false, false, false]);
    for (const state of ['done', 'failed'] as const) {
      expect([bubbleShows(state, BUBBLE_MS - 1), bubbleShows(state, BUBBLE_MS)]).toEqual([true, false]);
    }
  });
});

describe('the rows of a sheet', () => {
  it('are those that its package names, and the others counted from the picture', () => {
    // Every row has as many frames as its number, the first one at least one, and the last one all.
    const filled = (column: number, row: number) => column < Math.max(1, row);
    const rows = animationsOf({ wave: { row: 3, frames: 2, fps: 12 } }, filled);
    expect(Object.keys(rows)).toEqual(ROWS);
    expect(rows.wave).toEqual({ row: 3, frames: 2, fps: 12 });
    expect(rows.idle).toEqual({ row: 0, frames: 1, fps: 5 });
    expect(rows.runRight).toEqual({ row: 1, frames: 1, fps: 10 });
    expect(rows.jump).toEqual({ row: 4, frames: 4, fps: 8 });
    expect(rows.review).toEqual({ row: 8, frames: 8, fps: 4 });
    // A row that is full has eight frames, and one with nothing in it one all the same.
    expect(animationsOf(null, () => true).idle.frames).toBe(8);
    expect(animationsOf(null, () => false).thinking).toEqual({ row: 7, frames: 1, fps: 5 });
  });

  it('go through their frames at their speed, and stand on the first where nothing is to move', () => {
    const wave = { row: 3, frames: 4, fps: 8 };
    expect([0, 124, 125, 250, 499, 500].map(at => cellOf(wave, at, false).column)).toEqual([0, 0, 1, 2, 3, 0]);
    expect(cellOf(wave, 375, true)).toEqual({ column: 0, row: 3 });
    expect(cellOf(wave, -50, false)).toEqual({ column: 0, row: 3 });
  });
});
