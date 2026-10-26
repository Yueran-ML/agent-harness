// @vitest-environment jsdom
/**
 * The tools of a task beside its conversation: what it cost and planning on the status line, what
 * the user says of a turn and going back, tasks at set times, worktrees, the groups of QQ, and
 * pictures that are pasted.
 */
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ColumbinaBridge } from '../src/bridge';
import { Conversation } from '../src/app/Conversation';
import { QqGroups } from '../src/app/QqGroups';
import { Schedules } from '../src/app/Schedules';
import { spentOf, StatusLine } from '../src/app/StatusLine';
import { Worktrees } from '../src/app/Worktrees';
import { failureOf, groupsOf, schedulesOf, taskOf, whenOf, worktreesOf, type Schedule } from '../src/app/work';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { projectEvents } from '../src/state';
import type { Workspace } from '../src/useWorkspace';
import { begin, end, likeABrowser, log, reply, statusOf, user, workspaceWith, type Written } from './mounted';

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  delete window.columbina;
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found] = marked(name);
  if (!found) throw new Error(`Nothing is marked ${name}.`);
  return found;
};

/** A shell that answers what a test says, and nothing else. */
function shellWith(calls: Partial<ColumbinaBridge>): void {
  window.columbina = calls as ColumbinaBridge;
}

const kept = (turn: number): Written => ['checkpoint', { turn, commit: 'c0ffee' }];
const judged = (turn: number, right: boolean): Written => ['verdict', { turn, right }];

describe('what the shell answers', () => {
  it('is read as far as it says what a task, a worktree or a group is', () => {
    expect(schedulesOf({
      schedules: [
        { id: 't1', name: 'News', cwd: 'F:\\work', prompt: 'Sum up.', every: 'week', weekday: 'wed', at: '18:30', missed: 'skip', enabled: false, last: 5, last_end: 'completed' },
        { id: 't2', name: 'Often', cwd: '/w', every: 'minutes', minutes: 10 },
        { id: 't3', name: 'Broken', cwd: '/w', every: 'fortnight' },
        'nothing',
      ],
    })).toEqual([
      { id: 't1', name: 'News', cwd: 'F:\\work', prompt: 'Sum up.', every: 'week', minutes: null, at: '18:30', weekday: 'wed', missed: 'skip', enabled: false, last: 5, lastEnd: 'completed' },
      { id: 't2', name: 'Often', cwd: '/w', prompt: '', every: 'minutes', minutes: 10, at: null, weekday: null, missed: 'run', enabled: true, last: null, lastEnd: null },
    ]);
    expect(schedulesOf(null)).toEqual([]);
    expect(worktreesOf({ worktrees: [{ name: 'a', path: '/p/a', branch: 'columbina/a' }, { name: 'b' }] }))
      .toEqual([{ name: 'a', path: '/p/a', branch: 'columbina/a' }]);
    expect(groupsOf({ groups: [{ id: 'G1', name: null, memory: 4, most: 4000 }, { id: 'G2', name: 'Home' }, { name: 'x' }] }))
      .toEqual([{ id: 'G1', name: null, memory: 4, most: 4000 }, { id: 'G2', name: 'Home', memory: 0, most: 0 }]);
  });

  it('says what failed without the channel that it came through', () => {
    expect(failureOf(new Error("Error invoking remote method 'columbina:worktrees': Error: git failed: no"))).toBe('git failed: no');
    expect(failureOf('plain')).toBe('plain');
  });
});

describe('when a task runs', () => {
  const task = (changes: Partial<Schedule>): Schedule => ({
    id: 't', name: 'n', cwd: '/w', prompt: 'p', every: 'day', minutes: null, at: '09:00', weekday: null,
    missed: 'run', enabled: true, last: null, lastEnd: null, ...changes,
  });

  it('is said in the words of the page', () => {
    expect(whenOf(en, task({ every: 'minutes', minutes: 30, at: null }))).toBe('every 30 minutes');
    expect(whenOf(en, task({ every: 'hour', at: ':05' }))).toBe('every hour at minute 05');
    expect(whenOf(en, task({}))).toBe('every day at 09:00');
    expect(whenOf(en, task({ every: 'week', weekday: 'wed', at: '18:30' }))).toBe('every Wednesday at 18:30');
    expect(whenOf(zhCN, task({ every: 'week', weekday: 'sun', at: '08:00' }))).toBe('每周日 08:00');
  });

  it('is asked for only where the form holds the time that its kind needs', () => {
    const form = {
      name: ' News ', cwd: '/w', prompt: ' Sum up. ', every: 'day' as const, minutes: '30', time: '09:00', minute: '5',
      weekday: 'fri' as const, missed: 'skip' as const,
    };
    expect(taskOf(form)).toEqual({ name: 'News', cwd: '/w', prompt: 'Sum up.', every: 'day', at: '09:00', missed: 'skip' });
    expect(taskOf({ ...form, every: 'week' })).toMatchObject({ at: '09:00', weekday: 'fri' });
    expect(taskOf({ ...form, every: 'hour' })).toMatchObject({ at: ':05' });
    expect(taskOf({ ...form, every: 'minutes' })).toMatchObject({ minutes: 30 });
    expect(taskOf({ ...form, every: 'minutes', minutes: '4' })).toBeNull();
    expect(taskOf({ ...form, every: 'hour', minute: '60' })).toBeNull();
    expect(taskOf({ ...form, time: '24:00' })).toBeNull();
    expect(taskOf({ ...form, name: ' ' })).toBeNull();
    expect(taskOf({ ...form, cwd: '' })).toBeNull();
  });
});

describe('the status line', () => {
  it('says what the task cost, in each currency, small amounts to two digits', () => {
    expect(spentOf({ USD: 0.012345 }, 'en')).toBe('$0.012');
    expect(spentOf({ USD: 1.5, CNY: 0.1 }, 'en')).toBe('$1.50 + CN¥0.1');
    expect(spentOf({ credits: 0.5 }, 'en')).toBe('0.5 credits');
    expect(spentOf({ USD: 0 }, 'en')).toBeNull();
    expect(spentOf(null, 'en')).toBeNull();
  });

  it('shows the cost, and plan mode, which a press leaves', () => {
    const onPlan = vi.fn();
    const workspace = workspaceWith({ status: statusOf({ spent: { USD: 0.25 }, plan: true }) });
    render(<StatusLine workspace={workspace} locale="en" onPermissions={vi.fn()} onDetails={vi.fn()} onPlan={onPlan} />);
    expect(one('status-money').textContent).toBe('~$0.25');
    expect(one('status-plan').textContent).toBe('Plan mode');
    fireEvent.click(one('status-plan'));
    expect(onPlan).toHaveBeenCalledTimes(1);
  });

  it('says neither where nothing was spent and the task does not plan', () => {
    render(<StatusLine workspace={workspaceWith({ status: statusOf({ spent: {} }) })} locale="en" onPermissions={vi.fn()} onDetails={vi.fn()} onPlan={vi.fn()} />);
    expect(marked('status-money')).toEqual([]);
    expect(marked('status-plan')).toEqual([]);
  });
});

describe('a turn that has ended', () => {
  const turns = log(begin(1), kept(1), user('Fix it.'), reply('Fixed.'), end(1), judged(1, false), judged(1, true), begin(2), user('More.'), reply('Done.'), end(2));

  it('is said to have done what was asked, or not, and the last word holds', () => {
    const workspace = workspaceWith({ events: turns });
    expect(projectEvents(turns).verdicts).toEqual({ 1: true });
    render(<Conversation workspace={workspace} onLink={() => {}} onAction={vi.fn()} />);
    const [first, second] = marked('turn-tools');
    expect(first.querySelector('[data-test="verdict-right"]')!.getAttribute('aria-pressed')).toBe('true');
    expect(second.querySelector('[data-test="verdict-right"]')!.getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(second.querySelector('[data-test="verdict-wrong"]')!);
    expect(workspace.send).toHaveBeenCalledWith('/verdict wrong #2', true);
  });

  it('puts the files back as they were before it, once that is confirmed, where they were kept', () => {
    const workspace = workspaceWith({ events: turns });
    expect(projectEvents(turns).kept).toEqual([1]);
    render(<Conversation workspace={workspace} onLink={() => {}} onAction={vi.fn()} />);
    // Only before the first turn were the files kept.
    expect(marked('rewind')).toHaveLength(1);
    fireEvent.click(one('rewind'));
    expect(one('rewind-ask').textContent).toContain('before turn 1');
    expect(workspace.send).not.toHaveBeenCalled();
    fireEvent.click(one('rewind-yes'));
    expect(workspace.send).toHaveBeenCalledWith('/rewind 1 files');
    // Kept as it was, the turn offers it again.
    fireEvent.click(one('rewind'));
    fireEvent.click([...one('rewind-ask').querySelectorAll('button')].at(-1)!);
    expect(marked('rewind-ask')).toEqual([]);
    expect(workspace.send).toHaveBeenCalledTimes(1);
  });

  it('can be judged and gone back from only while the agent is at rest, and not in a chat', () => {
    render(<Conversation workspace={workspaceWith({ events: turns, chat: true, status: statusOf({ status: 'running' }) })} onLink={() => {}} onAction={vi.fn()} />);
    expect(marked('rewind')).toEqual([]);
    // What was said of the first turn still shows; the second has nothing to show.
    expect(marked('turn-tools')).toHaveLength(1);
    expect((one('verdict-right') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('the tasks that run at set times', () => {
  const listed = {
    schedules: [{ id: 't1', name: 'News', cwd: 'F:\\work', prompt: 'Sum up.', every: 'day', at: '09:00', enabled: true, last: Date.UTC(2026, 9, 26, 1), last_end: 'completed' }],
  };

  it('are listed, switched off and removed, and one is added in the folder of the project', async () => {
    const schedules = vi.fn(async (asked: Parameters<ColumbinaBridge['schedules']>[0]) => (asked.what === 'list' ? listed : {}));
    shellWith({ schedules });
    const workspace = workspaceWith();
    render(<Schedules workspace={workspace} language="en" />);
    await waitFor(() => expect(marked('schedule')).toHaveLength(1));
    expect(one('schedule').textContent).toContain('every day at 09:00');
    expect(one('schedule').textContent).toContain('completed');

    await act(async () => { fireEvent.click(one('schedule-on')); });
    expect(schedules).toHaveBeenCalledWith({ what: 'disable', id: 't1' });
    await act(async () => { fireEvent.click(one('schedule-remove')); });
    expect(schedules).toHaveBeenCalledWith({ what: 'remove', id: 't1' });

    expect(one('schedule-cwd').textContent).toBe('F:\\work');
    expect((one('schedule-add') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(one('schedule-name'), { target: { value: 'Weekly' } });
    fireEvent.change(one('schedule-prompt'), { target: { value: 'Look back.' } });
    fireEvent.change(one('schedule-every'), { target: { value: 'week' } });
    fireEvent.change(one('schedule-weekday'), { target: { value: 'fri' } });
    fireEvent.change(one('schedule-time'), { target: { value: '17:00' } });
    await act(async () => { fireEvent.click(one('schedule-add')); });
    expect(schedules).toHaveBeenCalledWith({
      what: 'add',
      task: { name: 'Weekly', cwd: 'F:\\work', prompt: 'Look back.', every: 'week', at: '17:00', weekday: 'fri', missed: 'run' },
    });
    expect((one('schedule-name') as HTMLInputElement).value).toBe('');
  });

  it('run in another folder that is chosen, and say what could not be done', async () => {
    const schedules = vi.fn(async (asked: Parameters<ColumbinaBridge['schedules']>[0]) => {
      if (asked.what === 'list') return { schedules: [] };
      throw new Error("Error invoking remote method 'columbina:schedules': Error: the disk is full");
    });
    shellWith({ schedules });
    const workspace = workspaceWith({ chat: true, pickFolder: vi.fn(async () => 'G:\\elsewhere') });
    render(<Schedules workspace={workspace} language="en" />);
    await waitFor(() => expect(marked('schedules-none')).toHaveLength(1));
    expect(one('schedule-cwd').textContent).toBe('');
    await act(async () => { fireEvent.click(one('schedule-choose')); });
    expect(one('schedule-cwd').textContent).toBe('G:\\elsewhere');
    fireEvent.change(one('schedule-name'), { target: { value: 'N' } });
    fireEvent.change(one('schedule-prompt'), { target: { value: 'P' } });
    fireEvent.change(one('schedule-every'), { target: { value: 'minutes' } });
    fireEvent.change(one('schedule-minutes'), { target: { value: '15' } });
    await act(async () => { fireEvent.click(one('schedule-add')); });
    expect(one('schedules-failed').textContent).toBe('The tasks could not be changed. the disk is full');
  });

  it('are not offered where the program cannot be asked', async () => {
    const schedules = vi.fn(async () => { throw new Error('no program'); });
    shellWith({ schedules });
    render(<Schedules workspace={workspaceWith()} language="en" />);
    await act(async () => {});
    expect(marked('schedules')).toEqual([]);
  });
});

describe('the worktrees of a project', () => {
  const found = { worktrees: [{ name: 'fix', path: 'F:\\work\\.columbina\\worktrees\\fix', branch: 'columbina/fix' }] };

  it('are made for a task, which is opened there, and merged after a yes', async () => {
    const worktrees = vi.fn(async (asked: Parameters<ColumbinaBridge['worktrees']>[0]) => (
      asked.what === 'add' ? { name: 'new-task', path: 'F:\\work\\.columbina\\worktrees\\new-task' } : found));
    shellWith({ worktrees });
    const workspace = workspaceWith();
    render(<Worktrees workspace={workspace} />);
    await waitFor(() => expect(marked('worktree')).toHaveLength(1));
    expect(worktrees).toHaveBeenCalledWith({ what: 'list', cwd: 'F:\\work', name: undefined, force: false });

    fireEvent.change(one('worktree-name'), { target: { value: 'New task' } });
    await act(async () => { fireEvent.click(one('worktree-make')); });
    expect(worktrees).toHaveBeenCalledWith({ what: 'add', cwd: 'F:\\work', name: 'New task', force: false });
    expect(workspace.openFolder).toHaveBeenCalledWith('F:\\work\\.columbina\\worktrees\\new-task');

    fireEvent.click(one('worktree-merge'));
    expect(one('worktree-ask').textContent).toContain('Merge the work of fix');
    await act(async () => { fireEvent.click(one('worktree-yes')); });
    expect(worktrees).toHaveBeenCalledWith({ what: 'merge', cwd: 'F:\\work', name: 'fix', force: false });
  });

  it('are removed with their work, but not the one that is open, and say what failed', async () => {
    const worktrees = vi.fn(async (asked: Parameters<ColumbinaBridge['worktrees']>[0]) => {
      if (asked.what === 'remove') throw new Error('git failed: it is locked');
      return { worktrees: [...found.worktrees, { name: 'open', path: 'f:/work/.columbina/worktrees/open/', branch: 'columbina/open' }] };
    });
    shellWith({ worktrees });
    render(<Worktrees workspace={workspaceWith({ folder: 'F:\\work\\.columbina\\worktrees\\open' })} />);
    await waitFor(() => expect(marked('worktree')).toHaveLength(2));
    // The worktree that is open is shown as such, and cannot be ended from inside it.
    expect(marked('worktree-remove')).toHaveLength(1);
    fireEvent.click(one('worktree-remove'));
    await act(async () => { fireEvent.click(one('worktree-yes')); });
    expect(worktrees).toHaveBeenCalledWith({ what: 'remove', cwd: 'F:\\work\\.columbina\\worktrees\\open', name: 'fix', force: true });
    expect(one('worktrees-failed').textContent).toBe('The worktree could not be changed. git failed: it is locked');
  });

  it('are nothing to a conversation of no project, or a project that is in no repository', async () => {
    const worktrees = vi.fn(async () => { throw new Error('not in git'); });
    shellWith({ worktrees });
    const { unmount } = render(<Worktrees workspace={workspaceWith({ chat: true })} />);
    expect(marked('worktrees')).toEqual([]);
    unmount();
    render(<Worktrees workspace={workspaceWith()} />);
    await act(async () => {});
    expect(marked('worktree')).toEqual([]);
    expect(marked('worktrees-failed')).toEqual([]);
  });
});

describe('the groups of the assistant in QQ', () => {
  it('are named, and their memories are read and forgotten after a yes', async () => {
    let groups = [{ id: 'G1', name: null, memory: 4, most: 4000 }];
    const qqGroups = vi.fn(async (asked: Parameters<ColumbinaBridge['qqGroups']>[0]) => {
      if (asked.what === 'memory') return { id: 'G1', memory: 'cat\n' };
      if (asked.what === 'forget') groups = [{ ...groups[0], memory: 0 }];
      if (asked.what === 'name') groups = [{ ...groups[0], name: asked.name! as unknown as null }];
      return { groups };
    });
    shellWith({ qqGroups });
    render(<QqGroups />);
    await waitFor(() => expect(marked('qq-group')).toHaveLength(1));
    expect(one('qq-group').textContent).toContain('Memory: 4 of 4000 characters');

    expect((one('qq-group-save') as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(one('qq-group-name'), { target: { value: ' Home ' } });
    await act(async () => { fireEvent.click(one('qq-group-save')); });
    expect(qqGroups).toHaveBeenCalledWith({ what: 'name', id: 'G1', name: 'Home' });

    await act(async () => { fireEvent.click(one('qq-group-show')); });
    expect(one('qq-group-memory').textContent).toBe('cat\n');
    fireEvent.click(one('qq-group-forget'));
    await act(async () => { fireEvent.click(one('qq-group-forget-yes')); });
    expect(qqGroups).toHaveBeenCalledWith({ what: 'forget', id: 'G1' });
    expect(one('qq-group-memory').textContent).toBe('The memory is empty.');
    await act(async () => { fireEvent.click(one('qq-group-show')); });
    expect(marked('qq-group-memory')).toEqual([]);
  });

  it('say what failed, and are nothing where the program cannot be asked', async () => {
    const qqGroups = vi.fn(async (asked: Parameters<ColumbinaBridge['qqGroups']>[0]) => {
      if (asked.what === 'list') return { groups: [{ id: 'G1', name: 'Home', memory: 0, most: 4000 }] };
      throw new Error('no bot of QQ is set up');
    });
    shellWith({ qqGroups });
    const { unmount } = render(<QqGroups />);
    await waitFor(() => expect(marked('qq-group')).toHaveLength(1));
    await act(async () => { fireEvent.click(one('qq-group-show')); });
    expect(one('qq-groups-failed').textContent).toBe('The groups could not be changed. no bot of QQ is set up');
    unmount();

    shellWith({ qqGroups: vi.fn(async () => { throw new Error('no program'); }) });
    render(<QqGroups />);
    await act(async () => {});
    expect(marked('qq-groups')).toEqual([]);
  });
});

describe('the parts that need the shell', () => {
  it('are nothing in a replay', () => {
    const workspace: Workspace = workspaceWith();
    render(<><Schedules workspace={workspace} language="en" /><Worktrees workspace={workspace} /><QqGroups /></>);
    expect(marked('schedules')).toEqual([]);
    expect(marked('worktrees')).toEqual([]);
    expect(marked('qq-groups')).toEqual([]);
  });
});
