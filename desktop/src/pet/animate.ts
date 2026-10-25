/**
 * Which frame of its sheet a pet shows: the row for what it does, and the frame of that row for the
 * time that has gone by. Worked out apart from the window, so that every case can be tried.
 */
import type { PetState } from './state';

/** The rows of a sheet, as the format names them. */
export type RowName = 'idle' | 'runRight' | 'runLeft' | 'wave' | 'jump' | 'failed' | 'waiting' | 'thinking' | 'review';
export const ROWS: RowName[] = ['idle', 'runRight', 'runLeft', 'wave', 'jump', 'failed', 'waiting', 'thinking', 'review'];
export const COLUMNS = 8;

export interface Animation { row: number; frames: number; fps: number }
export type Animations = Partial<Record<RowName, Animation>>;

/** How fast each row goes where a package does not say. */
const SPEED: Record<RowName, number> = {
  idle: 5, runRight: 10, runLeft: 10, wave: 6, jump: 8, failed: 6, waiting: 4, thinking: 5, review: 4,
};

/** How long a pet waves when a turn is done, and shows a failure, before it rests. */
export const WAVE_MS = 2_400;
export const FAILED_MS = 8_000;
/** How long a bubble stays that says something has happened. One that asks for the user stays while it does. */
export const BUBBLE_MS = 6_000;

/** The row for what a pet does, some time after it began to do it. */
export function rowOf(state: PetState, elapsed: number): RowName {
  switch (state) {
    case 'done': return elapsed < WAVE_MS ? 'wave' : 'idle';
    case 'failed': return elapsed < FAILED_MS ? 'failed' : 'idle';
    case 'thinking': return 'thinking';
    case 'review': return 'review';
    case 'waiting': return 'waiting';
    default: return 'idle';
  }
}

/** Whether the bubble of a state still shows, some time after the state began. */
export function bubbleShows(state: PetState, elapsed: number): boolean {
  if (state === 'waiting') return true;
  return (state === 'done' || state === 'failed') && elapsed < BUBBLE_MS;
}

/**
 * The rows of a sheet: those that its package names, and the others counted from the picture. A
 * row has as many frames as there are cells with something in them before the first empty one,
 * and one at least.
 *
 * Args:
 *   named: What the package names, or null.
 *   filled: Whether a cell of the picture has something in it, by its column and row.
 */
export function animationsOf(named: Animations | null, filled: (column: number, row: number) => boolean): Record<RowName, Animation> {
  const all = {} as Record<RowName, Animation>;
  ROWS.forEach((name, row) => {
    const given = named?.[name];
    if (given) { all[name] = given; return; }
    let frames = 0;
    while (frames < COLUMNS && filled(frames, row)) frames += 1;
    all[name] = { row, frames: Math.max(1, frames), fps: SPEED[name] };
  });
  return all;
}

/** The cell to show of a row, some time after it began, and the first one where nothing is to move. */
export function cellOf(animation: Animation, elapsed: number, still: boolean): { column: number; row: number } {
  const column = still ? 0 : Math.floor((Math.max(0, elapsed) / 1000) * animation.fps) % animation.frames;
  return { column, row: animation.row };
}
