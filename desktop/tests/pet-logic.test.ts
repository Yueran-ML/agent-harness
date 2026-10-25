/** What the pet shows of the task, and which frame of its sheet it shows for that. */
import { describe, expect, it } from 'vitest';
import { animationsOf, BUBBLE_MS, bubbleShows, cellOf, FAILED_MS, rowOf, ROWS, WAVE_MS } from '../src/pet/animate';
import { AFTER_MS, petStateOf } from '../src/pet/state';
import { begin, call, end, log, reply, result, statusOf, user, workspaceWith } from './mounted';
import type { PendingApproval } from '../src/useWorkspace';

const working = statusOf({ status: 'running' });
const asked: PendingApproval = {
  requestId: 7, session_id: 's-1', call_id: 'c-1', tool: 'shell', summary: 'Run a command.',
  reason: 'The command is not among those that run without a question.', target: 'npm test', standing: null,
};

describe('what the pet shows of a task', () => {
  it('rests where nothing happens', () => {
    expect(petStateOf(workspaceWith(), 0)).toEqual({ state: 'idle', label: '' });
  });

  it('waits with the user while the agent asks something, before anything else', () => {
    const events = log(begin(1), user('Run the tests.'), call('c-1', 'shell', { command: 'npm test' }));
    expect(petStateOf(workspaceWith({ events, status: working, approval: asked }), 0)).toEqual({ state: 'waiting', label: 'shell' });
  });

  it('reads while a tool runs, and thinks otherwise while the turn runs', () => {
    const running = log(begin(1), user('Read it.'), call('c-1', 'read', { path: 'a.md' }));
    expect(petStateOf(workspaceWith({ events: running, status: working }), 0)).toEqual({ state: 'review', label: 'read' });
    const done = log(begin(1), user('Read it.'), call('c-1', 'read', { path: 'a.md' }), result('c-1', 'text'));
    expect(petStateOf(workspaceWith({ events: done, status: working }), 0)).toEqual({ state: 'thinking', label: '' });
    expect(petStateOf(workspaceWith({ events: log(begin(1), user('Hi.')), status: working }), 0)).toEqual({ state: 'thinking', label: '' });
  });

  it('is done for a while after a turn ended well, and rests then', () => {
    // The end is the fourth event of the log, written at 1300.
    const events = log(begin(1), user('Hi.'), reply('Hello.'), end(1));
    expect(petStateOf(workspaceWith({ events }), 1300)).toEqual({ state: 'done', label: '' });
    expect(petStateOf(workspaceWith({ events }), 1300 + AFTER_MS - 1)).toEqual({ state: 'done', label: '' });
    expect(petStateOf(workspaceWith({ events }), 1300 + AFTER_MS)).toEqual({ state: 'idle', label: '' });
  });

  it('shows a failure for a while after a turn failed, was blocked or was cut off, with its code', () => {
    for (const reason of ['error', 'blocked', 'max-tokens']) {
      const ended: [string, Record<string, unknown>] = ['turn/end', { turn: 1, reason, detail: 'no', code: 'AUTH' }];
      const events = log(begin(1), user('Hi.'), ended);
      expect(petStateOf(workspaceWith({ events }), 1200), reason).toEqual({ state: 'failed', label: 'AUTH' });
    }
    const events = log(begin(1), user('Hi.'), end(1, 'error', 'no'));
    expect(petStateOf(workspaceWith({ events }), 1200)).toEqual({ state: 'failed', label: '' });
  });

  it('thinks in a new turn although a call of a turn that was stopped never had its result', () => {
    const events = log(begin(1), user('Read.'), call('c-1', 'read', {}), end(1, 'aborted', 'user'), begin(2), user('Again.'));
    expect(petStateOf(workspaceWith({ events, status: working }), 0)).toEqual({ state: 'thinking', label: '' });
  });

  it('rests at once after a turn that was stopped', () => {
    const events = log(begin(1), user('Hi.'), end(1, 'aborted', 'user'));
    expect(petStateOf(workspaceWith({ events }), 1200)).toEqual({ state: 'idle', label: '' });
  });
});

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
