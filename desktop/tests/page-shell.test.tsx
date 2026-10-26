// @vitest-environment jsdom
import { useState } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BackgroundMusic } from '../src/BackgroundMusic';
import { Inspector, type InspectorTab } from '../src/app/Inspector';
import { Settings, type SettingsPart } from '../src/app/Settings';
import { Sheet } from '../src/app/Sheet';
import { Sidebar } from '../src/app/Sidebar';
import { Recent, StartHead, StartSkills } from '../src/app/Start';
import { StatusLine } from '../src/app/StatusLine';
import type { Listed } from '../src/key/form';
import { projectEvents } from '../src/state';
import type { Workspace } from '../src/useWorkspace';
import {
  begin, call, end, gitHubWith, likeABrowser, log, modelsWith, reply, result, service, statusOf, user, workspaceWith,
  type Written,
} from './mounted';

// The assistants in Feishu and in WeCom are tried in page-assistant.test.tsx.
vi.mock('../src/useAssistant', async () => {
  const { noAssistant } = await import('./assistantAside');
  return { useAssistant: () => noAssistant };
});

beforeEach(likeABrowser);
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
/** The button that says these words, and no other. */
const button = (says: string, within: ParentNode = document) => {
  const found = [...within.querySelectorAll('button')].filter(each => each.textContent === says);
  if (found.length !== 1) throw new Error(`There is not exactly one button that says "${says}".`);
  return found[0];
};
/** How often each of these was called, in the order in which they are listed. */
const counted = (calls: Record<string, ReturnType<typeof vi.fn>>) =>
  Object.fromEntries(Object.entries(calls).map(([name, call]) => [name, call.mock.calls.length]));
const atWork = statusOf({ status: 'running' });
const fullAccess = statusOf({ sandbox: { mode: 'danger-full-access', strength: 'none' }, approval: 'never' });
const notConfined = 'Commands are not confined on this computer. A command that runs can do whatever you can do.';

describe('the status line', () => {
  const drawn = (workspace: Workspace, locale = 'en') => {
    const calls = { onPermissions: vi.fn(), onDetails: vi.fn(), onPlan: vi.fn() };
    render(<StatusLine workspace={workspace} locale={locale} {...calls} />);
    return calls;
  };
  const confined = (strength: string) => statusOf({ sandbox: { mode: 'workspace-write', strength } });
  const sign = () => document.querySelector<HTMLElement>('.status-loose');

  it('says in a sentence what the task may do, and warns where commands are not confined', () => {
    const calls = drawn(workspaceWith());
    expect(one('status').dataset.tone).toBeUndefined();
    expect(one('status-may').textContent).toBe('Can change the files of this project, and asks before it runs a command.');
    // This computer holds no command to the project. A sign beside the sentence says so to whoever asks.
    expect(sign()?.getAttribute('aria-label')).toBe(notConfined);
    expect(sign()?.getAttribute('title')).toBe(notConfined);
    fireEvent.click(one('status-may'));
    expect(counted(calls)).toEqual({ onPermissions: 1, onDetails: 0, onPlan: 0 });
    fireEvent.click(sign()!);
    expect(counted(calls)).toEqual({ onPermissions: 2, onDetails: 0, onPlan: 0 });
  });

  it('has the sign where commands are confined in part, and none where they are confined', () => {
    drawn(workspaceWith({ status: confined('partial') }));
    expect(sign()?.getAttribute('aria-label'))
      .toBe('Commands are confined only in part here. A command may reach further than these permissions allow.');
    cleanup();
    drawn(workspaceWith({ status: confined('full') }));
    expect(sign()).toBeNull();
    // Of a strength that this window does not know, nothing is claimed.
    cleanup();
    drawn(workspaceWith({ status: confined('a-new-one') }));
    expect(sign()).toBeNull();
  });

  it('says of a chat that it changes nothing, under any preset, and has no sign of commands', () => {
    const calls = drawn(workspaceWith({ chat: true }));
    expect(one('status-may').textContent).toBe('Changes no file and runs no command.');
    expect(sign()).toBeNull();
    fireEvent.click(one('status-may'));
    expect(counted(calls)).toEqual({ onPermissions: 1, onDetails: 0, onPlan: 0 });
    cleanup();
    drawn(workspaceWith({ chat: true, status: fullAccess }));
    expect(one('status-may').textContent).toBe('Changes no file and runs no command.');
    expect(one('status').dataset.tone).toBeUndefined();
    expect(one('status-may').querySelector('svg')?.getAttribute('class')).toContain('shield-check');
  });

  it('is in the tone of a warning as a whole under full access', () => {
    drawn(workspaceWith({ status: fullAccess }));
    expect(one('status').dataset.tone).toBe('warn');
    expect(one('status-may').textContent).toBe('Can read and change files anywhere on this computer, and runs commands without asking.');
    // The sentence says all there is to say: the sign beside it is not there.
    expect(sign()).toBeNull();
    // The tone is the line's own, and not one of its parts'.
    expect(one('status-may').dataset.tone).toBeUndefined();
  });

  it('goes by where files can be changed, and not by whether commands are asked about', () => {
    drawn(workspaceWith({ status: statusOf({ approval: 'never' }) }));
    expect(one('status').dataset.tone).toBeUndefined();
    expect(one('status-may').textContent).toBe('Can change the files of this project, and runs only commands that a rule allows.');
    cleanup();
    // With full access a command runs without a question, whatever the policy says.
    drawn(workspaceWith({ status: statusOf({ sandbox: { mode: 'danger-full-access', strength: 'none' }, approval: 'ask' }) }));
    expect(one('status').dataset.tone).toBe('warn');
  });

  it('says nothing of what a task may do before the service has said it, and nothing of a task without a project', () => {
    drawn(workspaceWith({ status: null }));
    expect(marked('status-may')).toEqual([]);
    expect(sign()).toBeNull();
    expect(marked('status-context')).toEqual([]);
    expect(one('status-stands').textContent).toBe('At rest');
    cleanup();
    drawn(workspaceWith({ hasProject: false, hello: null, status: null, sessionId: null }));
    expect(marked('status-stands')).toEqual([]);
  });

  it('says how the task stands', () => {
    const stands = (given: Partial<Workspace>) => {
      cleanup();
      drawn(workspaceWith(given));
      return [one('status-stands').textContent, one('status-stands').dataset.tone];
    };
    expect(stands({})).toEqual(['At rest', 'rest']);
    expect(stands({ status: atWork })).toEqual(['At work', 'live']);
    expect(stands({ status: atWork, cancelling: true })).toEqual(['Stopping', 'live']);
    expect(stands({ connected: false })).toEqual(['Not connected', 'off']);
    // A project is opened: its service process is started, and then its task is read.
    expect(stands({ loading: true, hello: null, status: null })).toEqual(['Starting', 'rest']);
    expect(stands({ loading: true, status: null })).toEqual(['Opening', 'rest']);
  });

  it('says how full the context is, in the tone of a warning from four fifths on', () => {
    const context = (tokens: number) => {
      cleanup();
      const calls = drawn(workspaceWith({
        status: statusOf({ model: { provider: 'deepseek', model: 'a-model', context_window: 1000 } }),
        projection: { ...projectEvents([], false), lastInputTokens: tokens },
      }));
      return { calls, shown: one('status-context') };
    };
    const told = (tokens: number) => {
      const { shown } = context(tokens);
      return [shown.textContent, shown.dataset.tone];
    };
    expect(told(790)).toEqual(['Context 79%', undefined]);
    expect(told(800)).toEqual(['Context 80%', 'warn']);
    expect(told(940)).toEqual(['Context 94%', 'warn']);
    expect(told(950)).toEqual(['Context 95%', 'bad']);
    // The ring is filled as far as the context is.
    const { calls, shown } = context(250);
    const [filled, whole] = shown.querySelector('.ring-fill')!.getAttribute('stroke-dasharray')!.split(' ').map(Number);
    expect(filled / whole).toBeCloseTo(0.25, 10);
    fireEvent.click(shown);
    expect(counted(calls)).toEqual({ onPermissions: 0, onDetails: 1, onPlan: 0 });
  });

  describe('how far the task has come', () => {
    /** A step of the model that read and wrote so much, and took a tenth of a second, as the log is written. */
    const step = (turn: number, number: number, input: number, output: number, cached = 0): Written[] => [
      ['step/start', { turn, step: number }],
      ['assistant/message', {
        message: { role: 'assistant', content: [{ type: 'text', text: 'Done.' }] },
        usage: { input_tokens: input, output_tokens: output, cache_read_tokens: cached, cache_write_tokens: 0 },
        finish_reason: 'stop',
      }],
    ];
    const worked = log(
      begin(1), user('Fix the parser.'), ...step(1, 1, 1_000, 30, 9_000), ...step(1, 2, 2_000, 10, 980_800), end(1),
      begin(2), user('Now the tests.'), ...step(2, 1, 200, 60, 7_000), end(2),
    );
    const stats = () => one('status-stats');

    it('says the turns and steps, how fast the model wrote, the tokens and what came from the cache', () => {
      const calls = drawn(workspaceWith({ events: worked }));
      // Each answer took a tenth of a second: 100 tokens in three tenths.
      expect(stats().querySelector('.status-stats-run')!.textContent).toBe('2 turns · 3 steps · 333 tok/s');
      // A million read and 100 written. 996,800 read from the cache is 99.68%, said as 99.6: what
      // was nearly all read from the cache is not said to be more than it was.
      expect(stats().querySelector('.status-stats-spent')!.textContent).toBe('1M tokens · 99.6% cached');
      expect(stats().getAttribute('title')).toBe(
        '2 turns and 3 steps. The model wrote 333 tokens a second on average, from when a request was made to when '
        + 'its answer was in. 1,000,100 tokens in all: 1,000,000 read, 996,800 of them from the cache, and 100 written.',
      );
      expect(stats().getAttribute('aria-label')).toBe(stats().getAttribute('title'));
      fireEvent.click(stats());
      expect(counted(calls)).toEqual({ onPermissions: 0, onDetails: 1, onPlan: 0 });
    });

    it('says the numbers as the language of the page writes them', () => {
      drawn(workspaceWith({ events: worked }), 'zh-CN');
      expect(stats().querySelector('.status-stats-spent')!.textContent).toBe('100万 tokens · 99.6% cached');
      cleanup();
      drawn(workspaceWith({ events: worked }), 'de');
      expect(stats().querySelector('.status-stats-spent')!.textContent).toBe('1 Mio. tokens · 99,6% cached');
    });

    it('leaves out what the log does not tell', () => {
      // One turn, and one step whose answer was cut short: nothing tells how fast the model wrote.
      drawn(workspaceWith({ events: log(begin(1), ['step/start', { turn: 1, step: 1 }], ['assistant/message', {
        message: { role: 'assistant', content: [] }, usage: null, finish_reason: null,
      }]) }));
      expect(stats().querySelector('.status-stats-run')!.textContent).toBe('1 turn · 1 step');
      expect(stats().querySelector('.status-stats-spent')!.textContent).toBe('0 tokens');
      expect(stats().getAttribute('title')).toBe('1 turn and 1 step. 0 tokens in all: 0 read, and 0 written.');
    });

    it('says nothing of a task that has not begun, or of no task', () => {
      drawn(workspaceWith({ events: log(user('Waiting.')) }));
      expect(marked('status-stats')).toEqual([]);
      cleanup();
      drawn(workspaceWith({ events: worked, hasProject: false }));
      expect(marked('status-stats')).toEqual([]);
    });

    it('says how fast the model wrote once, however often the line is drawn again', () => {
      // What the projection holds is read as it is: no turn is counted twice.
      const workspace = workspaceWith({ events: worked });
      const { rerender } = render(<StatusLine workspace={workspace} locale="en" onPermissions={vi.fn()} onDetails={vi.fn()} onPlan={vi.fn()} />);
      rerender(<StatusLine workspace={{ ...workspace }} locale="en" onPermissions={vi.fn()} onDetails={vi.fn()} onPlan={vi.fn()} />);
      expect(marked('status-stats').map(each => each.querySelector('.status-stats-run')!.textContent))
        .toEqual(['2 turns · 3 steps · 333 tok/s']);
    });
  });

  it('says nothing of the context of a model whose window is not known', () => {
    drawn(workspaceWith({ status: statusOf({ model: { provider: 'my-own', model: 'a-model', context_window: null } }) }));
    expect(marked('status-context')).toEqual([]);
  });

  it('says of a replay that it is one', () => {
    const replay = () => document.querySelector<HTMLElement>('.status-replay');
    drawn(workspaceWith({ mode: 'mock' }));
    expect(replay()?.textContent).toBe('Replay');
    expect(replay()?.getAttribute('title')).toBe('This is a replay of a recorded session. No model is asked for anything.');
    cleanup();
    drawn(workspaceWith({ mode: 'mock', connected: false }));
    expect(replay()).toBeNull();
    cleanup();
    drawn(workspaceWith());
    expect(replay()).toBeNull();
  });
});

describe('the list of tasks', () => {
  const sessions = [
    { id: 's-1', modified_ms: Date.now() - 60_000, first_input: 'Fix the parser.' },
    { id: 's-2', modified_ms: Date.now() - 3 * 3_600_000, first_input: 'Write the notes.' },
  ];
  const drawn = (workspace: Workspace, title = 'Fix the parser.') => {
    const calls = { onNewTask: vi.fn(), onOpenTask: vi.fn(), onOpenProject: vi.fn(), onOpenChat: vi.fn(), onSettings: vi.fn(), onHide: vi.fn() };
    render(<Sidebar workspace={workspace} title={title} {...calls} />);
    return calls;
  };
  const rows = () => [...document.querySelectorAll<HTMLElement>('.tasks .task')].map(row => ({
    name: row.querySelector('.task-name')?.textContent,
    open: row.getAttribute('aria-current') === 'true',
    when: row.querySelector('.task-when')?.textContent ?? null,
    stands: row.querySelector<HTMLElement>('[data-test="task-stands"]')?.getAttribute('aria-label') ?? null,
    tone: row.querySelector<HTMLElement>('[data-test="task-stands"]')?.dataset.tone ?? null,
  }));
  const search = (text: string) => {
    fireEvent.change(document.querySelector<HTMLInputElement>('.search input')!, { target: { value: text } });
  };
  const none = () => document.querySelector('.tasks-none')?.textContent ?? null;

  it('says of the task that is open how it stands, and of no other', () => {
    drawn(workspaceWith({ sessions }));
    expect(rows()).toEqual([
      { name: 'Fix the parser.', open: true, when: '1 min ago', stands: 'At rest', tone: 'rest' },
      { name: 'Write the notes.', open: false, when: '3 h ago', stands: null, tone: null },
    ]);
    expect(one('task-stands').className).toBe('dot');
    // A line that is cut short says the whole of it to whoever rests on it.
    expect(marked('task').map(row => row.getAttribute('title'))).toEqual(['Fix the parser.', 'Write the notes.']);
  });

  it('shows the moon while the open task is at work, in the place of when it was written last', () => {
    drawn(workspaceWith({ sessions, status: atWork }));
    expect(rows()).toEqual([
      { name: 'Fix the parser.', open: true, when: null, stands: 'At work', tone: 'live' },
      { name: 'Write the notes.', open: false, when: '3 h ago', stands: null, tone: null },
    ]);
    expect(one('task-stands').className).toBe('moon');
  });

  it('says that the open task is being stopped', () => {
    drawn(workspaceWith({ sessions, status: atWork, cancelling: true }));
    expect(rows()[0]).toMatchObject({ stands: 'Stopping', tone: 'live' });
  });

  it('says that the open task is of no use where the service process is gone', () => {
    drawn(workspaceWith({ sessions, connected: false }));
    expect(rows()[0]).toMatchObject({ stands: 'Not connected', tone: 'off', when: '1 min ago' });
    expect(one('task-stands').className).toBe('dot');
  });

  it('gives a task in which nothing was said a line of its own, with how it stands', () => {
    drawn(workspaceWith({ sessions, sessionId: 's-3' }), 'New task');
    expect(rows()).toEqual([
      { name: 'New task', open: true, when: null, stands: 'At rest', tone: 'rest' },
      { name: 'Fix the parser.', open: false, when: '1 min ago', stands: null, tone: null },
      { name: 'Write the notes.', open: false, when: '3 h ago', stands: null, tone: null },
    ]);
    expect(document.querySelector('.tasks .task')?.getAttribute('title')).toBe('New task');
    // It began with nothing, so nothing that is searched for finds it.
    search('parser');
    expect(rows().map(row => row.name)).toEqual(['Fix the parser.']);
  });

  it('calls a task that began with nothing a new task, and finds it by no word', () => {
    drawn(workspaceWith({ sessions: [{ id: 's-1', modified_ms: Date.now(), first_input: null }, sessions[1]] }));
    expect(rows().map(row => [row.name, row.when])).toEqual([['New task', 'just now'], ['Write the notes.', '3 h ago']]);
    search('task');
    expect(rows()).toEqual([]);
  });

  it('says that there is no task yet, and says nothing where the open one has the list to itself', () => {
    drawn(workspaceWith({ sessionId: null }));
    expect(rows()).toEqual([]);
    expect(none()).toBe('No task yet in this project.');
    cleanup();
    drawn(workspaceWith());
    expect(rows()).toEqual([{ name: 'Fix the parser.', open: true, when: null, stands: 'At rest', tone: 'rest' }]);
    expect(none()).toBeNull();
    search('parser');
    expect(rows()).toEqual([]);
    expect(none()).toBe('No task matches.');
  });

  it('opens another task by a click on it, and does nothing for the one that is open', () => {
    const calls = drawn(workspaceWith({ sessions }));
    const [open, other] = marked('task');
    fireEvent.click(open);
    expect(calls.onOpenTask).not.toHaveBeenCalled();
    fireEvent.click(other);
    expect(calls.onOpenTask.mock.calls).toEqual([['s-2']]);
  });

  it('is searched by what the tasks began with, whatever the case of the letters', () => {
    drawn(workspaceWith({ sessions }));
    search(' NOTES ');
    expect(rows().map(row => row.name)).toEqual(['Write the notes.']);
    search('fix');
    expect(rows().map(row => row.name)).toEqual(['Fix the parser.']);
    search('the');
    expect(rows().map(row => row.name)).toEqual(['Fix the parser.', 'Write the notes.']);
    search('nothing like it');
    expect(rows()).toEqual([]);
    expect(none()).toBe('No task matches.');
  });

  it('lets no task be opened while the project is being opened or its service is gone', () => {
    const offered = (given: Partial<Workspace>) => {
      cleanup();
      drawn(workspaceWith({ sessions, ...given }));
      return [one('new-task'), ...marked('task')].map(each => !each.hasAttribute('disabled'));
    };
    expect(offered({})).toEqual([true, true, true]);
    expect(offered({ loading: true })).toEqual([false, false, false]);
    expect(offered({ connected: false })).toEqual([false, false, false]);
    // While the log of a long task is read, another task can be opened in its place.
    expect(offered({ loading: true, reading: { read: 100, of: 5000 } })).toEqual([true, true, true]);
    expect(offered({ connected: false, loading: true, reading: { read: 100, of: 5000 } })).toEqual([false, false, false]);
  });

  it('leads to a new task, to another project and to the settings', () => {
    const calls = drawn(workspaceWith({ sessions }));
    expect(one('open-project').textContent).toBe('work');
    expect(one('open-project').getAttribute('title')).toBe('F:\\work');
    fireEvent.click(one('new-task'));
    expect(counted(calls)).toEqual({ onNewTask: 1, onOpenTask: 0, onOpenProject: 0, onOpenChat: 0, onSettings: 0, onHide: 0 });
    fireEvent.click(one('open-project'));
    expect(counted(calls)).toEqual({ onNewTask: 1, onOpenTask: 0, onOpenProject: 1, onOpenChat: 0, onSettings: 0, onHide: 0 });
    fireEvent.click(one('open-settings'));
    expect(counted(calls)).toEqual({ onNewTask: 1, onOpenTask: 0, onOpenProject: 1, onOpenChat: 0, onSettings: 1, onHide: 0 });
    fireEvent.click(document.querySelector('[aria-label="Hide the list of tasks"]')!);
    expect(counted(calls)).toEqual({ onNewTask: 1, onOpenTask: 0, onOpenProject: 1, onOpenChat: 0, onSettings: 1, onHide: 1 });
  });

  it('offers a project to open where none has answered', () => {
    drawn(workspaceWith({ hello: null, sessionId: null }));
    expect(one('open-project').textContent).toBe('Open a project');
    expect(one('open-project').getAttribute('title')).toBe('Open another project');
  });

  it('leads from a project to a chat, and from a chat into a project', () => {
    const calls = drawn(workspaceWith({ sessions }));
    const chat = one('open-chat');
    expect(chat.textContent).toBe('Just chat');
    expect(chat.getAttribute('title')).toBe('A conversation that belongs to no project. Columbina changes no file in it and runs no command.');
    fireEvent.click(chat);
    expect(counted(calls)).toEqual({ onNewTask: 0, onOpenTask: 0, onOpenProject: 0, onOpenChat: 1, onSettings: 0, onHide: 0 });
    cleanup();
    // In a chat no project is open: the way into one says so, and the way into a chat is not offered.
    const inChat = drawn(workspaceWith({ chat: true, sessions, hello: { ...workspaceWith().hello!, cwd: 'C:\\Users\\me\\Columbina\\chat' } }));
    expect(one('open-project').textContent).toBe('Open a project');
    expect(one('open-project').getAttribute('title')).toBe('Open a project');
    expect(one('open-project').getAttribute('aria-label')).toBe('Open a project');
    expect(marked('open-chat')).toEqual([]);
    fireEvent.click(one('open-project'));
    expect(counted(inChat)).toEqual({ onNewTask: 0, onOpenTask: 0, onOpenProject: 1, onOpenChat: 0, onSettings: 0, onHide: 0 });
  });

  it('tells anew every minute when a task was written last', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-12T12:00:00Z'));
    drawn(workspaceWith({ sessionId: null, sessions: [{ id: 's-1', modified_ms: Date.now() - 60_000, first_input: 'Fix the parser.' }] }));
    expect(rows()[0].when).toBe('1 min ago');
    act(() => { vi.advanceTimersByTime(59_000); });
    expect(rows()[0].when).toBe('1 min ago');
    act(() => { vi.advanceTimersByTime(1_000); });
    expect(rows()[0].when).toBe('2 min ago');
  });
});

describe('the page of a new task', () => {
  const skill = (name: string, user_invocable = true) => ({
    name, description: `What ${name} is for.`, user_invocable, model_invocable: true,
  });
  const head = (workspace: Workspace, github = gitHubWith()) => {
    const calls = { onOpenProject: vi.fn(), onOpenRecent: vi.fn(), onOpenChat: vi.fn(), onPermissions: vi.fn(), onSetUp: vi.fn(), onOpenPage: vi.fn() };
    const firstButton = { current: null as HTMLButtonElement | null };
    const drawn = render(<StartHead workspace={workspace} github={github} {...calls} firstButton={firstButton} />);
    return { calls, firstButton, github, drawn };
  };

  it('is the way to a project before one is open', () => {
    const recent = [{ name: 'work', path: 'F:\\work' }, { name: 'notes', path: 'D:\\notes' }];
    const { calls, firstButton } = head(workspaceWith({ hasProject: false, hello: null, status: null, sessionId: null, recent }));
    expect(one('start').querySelector('h2')?.textContent).toBe('Open a project to begin');
    // The button is handed to the page, which gives it the keyboard.
    expect(firstButton.current).toBe(one('open-project-start'));
    fireEvent.click(one('open-project-start'));
    expect(counted(calls)).toEqual({ onOpenProject: 1, onOpenRecent: 0, onOpenChat: 0, onPermissions: 0, onSetUp: 0, onOpenPage: 0 });
    const rows = [...one('recent-projects').querySelectorAll('button')];
    expect(rows.map(row => [row.querySelector('.task-name')?.textContent, row.querySelector('.task-when')?.textContent, row.title]))
      .toEqual([['work', 'F:\\work', 'F:\\work'], ['notes', 'D:\\notes', 'D:\\notes']]);
    fireEvent.click(rows[1]);
    expect(calls.onOpenRecent.mock.calls).toEqual([['D:\\notes']]);
    expect(marked('start-may')).toEqual([]);
  });

  it('offers no earlier projects where there were none', () => {
    head(workspaceWith({ hasProject: false, hello: null, status: null, sessionId: null }));
    expect(marked('recent-projects')).toEqual([]);
  });

  it('says of a chat what it is, and not where its folder is', () => {
    head(workspaceWith({ chat: true, hello: { ...workspaceWith().hello!, cwd: 'C:\\Users\\me\\Columbina\\chat' } }));
    const lead = one('start').querySelector('.start-lead');
    expect(lead?.textContent).toBe('A conversation that belongs to no project. Columbina changes no file in it and runs no command.');
    expect(lead?.className).toBe('start-lead');
    expect(lead?.hasAttribute('title')).toBe(false);
    expect(one('start-chat-lead')).toBe(lead);
    // What it may do is what the lead says: there is no line for it beside.
    expect(marked('start-may')).toEqual([]);
  });

  it('says where the task works and what it may do, and leads to where that is chosen', () => {
    const { calls } = head(workspaceWith());
    expect(one('start').querySelector('h2')?.textContent).toBe('What shall we work on?');
    expect(one('start').querySelector('.start-lead')?.textContent).toBe('F:\\work');
    expect(one('start').querySelector('.start-lead')?.getAttribute('title')).toBe('F:\\work');
    expect(marked('open-project-start')).toEqual([]);
    expect(one('start-may').textContent).toBe('Can change the files of this project, and asks before it runs a command.');
    expect(one('start-may').dataset.tone).toBeUndefined();
    fireEvent.click(one('start-may'));
    expect(counted(calls)).toEqual({ onOpenProject: 0, onOpenRecent: 0, onOpenChat: 0, onPermissions: 1, onSetUp: 0, onOpenPage: 0 });
  });

  it('says it as a warning under full access, and says nothing before the service has said it', () => {
    head(workspaceWith({ status: fullAccess }));
    expect(one('start-may').dataset.tone).toBe('warn');
    expect(one('start-may').textContent).toBe('Can read and change files anywhere on this computer, and runs commands without asking.');
    cleanup();
    head(workspaceWith({ status: statusOf({ sandbox: { mode: 'danger-full-access', strength: 'none' }, approval: 'ask' }) }));
    expect(one('start-may').dataset.tone).toBe('warn');
    cleanup();
    head(workspaceWith({ status: statusOf({ approval: 'never' }) }));
    expect(one('start-may').dataset.tone).toBeUndefined();
    expect(one('start-may').textContent).toBe('Can change the files of this project, and runs only commands that a rule allows.');
    cleanup();
    head(workspaceWith({ status: null, hello: null }));
    expect(marked('start-may')).toEqual([]);
    expect(one('start').querySelector('.start-lead')).toBeNull();
  });

  it('names the skills, and hands the name of one that was clicked to the field', () => {
    const onUse = vi.fn();
    render(<StartSkills workspace={workspaceWith({ skills: [skill('house-style', false), skill('release'), skill('review')] })} onUse={onUse} />);
    const rows = marked('skill');
    // Those that a person can hand over stand first.
    expect(rows.map(row => row.querySelector('.skill-name')?.textContent)).toEqual(['release', 'review', 'house-style']);
    expect(rows.map(row => row.tagName)).toEqual(['BUTTON', 'BUTTON', 'DIV']);
    expect(rows.map(row => row.querySelector('.skill-for')?.textContent))
      .toEqual(['What release is for.', 'What review is for.', 'What house-style is for.']);
    expect(rows.map(row => row.getAttribute('title')))
      .toEqual(['What release is for.', 'What review is for.', 'What house-style is for.']);
    // The one that only the agent loads says so, and takes no click.
    expect(rows.map(row => row.querySelector('.task-when')?.textContent ?? null)).toEqual([null, null, 'Columbina loads it itself']);
    expect(rows.map(row => row.dataset.own ?? null)).toEqual([null, null, 'agent']);
    fireEvent.click(rows[2]);
    expect(onUse).not.toHaveBeenCalled();
    fireEvent.click(rows[1]);
    expect(onUse.mock.calls).toEqual([['review']]);
    expect(document.querySelector('.skills-more')).toBeNull();
  });

  it('names six skills at most, and says how many more there are', () => {
    const many = Array.from({ length: 9 }, (_one, index) => skill(`skill-${index}`));
    render(<StartSkills workspace={workspaceWith({ skills: many })} onUse={() => {}} />);
    expect(marked('skill')).toHaveLength(6);
    expect(document.querySelector('.skills-more')?.textContent).toBe('And 3 more. /skills lists them all.');
  });

  it('has nothing to say of skills where there are none', () => {
    render(<StartSkills workspace={workspaceWith()} onUse={() => {}} />);
    expect(marked('start-skills')).toEqual([]);
  });

  it('offers the four tasks that were worked on last, without the one that is open', () => {
    const sessions = Array.from({ length: 7 }, (_one, index) => ({
      id: `s-${index + 1}`, modified_ms: Date.now() - index * 3_600_000, first_input: `Task ${index + 1}.`,
    }));
    const onOpenTask = vi.fn();
    render(<Recent workspace={workspaceWith({ sessions, sessionId: 's-2' })} onOpenTask={onOpenTask} />);
    const rows = [...one('recent').querySelectorAll('button')];
    expect(rows.map(row => [row.querySelector('.task-name')?.textContent, row.querySelector('.task-when')?.textContent]))
      .toEqual([['Task 1.', 'just now'], ['Task 3.', '2 h ago'], ['Task 4.', '3 h ago'], ['Task 5.', '4 h ago']]);
    fireEvent.click(rows[1]);
    expect(onOpenTask.mock.calls).toEqual([['s-3']]);
  });

  it('offers no earlier task where the open one is the only one', () => {
    render(<Recent workspace={workspaceWith({ sessions: [{ id: 's-1', modified_ms: Date.now(), first_input: 'Fix the parser.' }] })} onOpenTask={() => {}} />);
    expect(marked('recent')).toEqual([]);
  });
});

describe('a sheet', () => {
  it('is closed by its cross and by Escape, and gives the keyboard back to where it was', () => {
    const before = document.createElement('button');
    document.body.append(before);
    before.focus();
    const onClose = vi.fn();
    const { unmount } = render(<Sheet title="A question" onClose={onClose} foot={<button type="button" data-first="">No</button>}>
      <p>Words.</p>
    </Sheet>);
    const dialog = document.querySelector('dialog')!;
    expect(dialog.hasAttribute('open')).toBe(true);
    expect(dialog.getAttribute('aria-label')).toBe('A question');
    expect(dialog.querySelector('h2')?.textContent).toBe('A question');
    expect(dialog.getAttribute('style')).toBeNull();
    // It begins at the answer that changes nothing.
    expect(document.activeElement?.textContent).toBe('No');
    fireEvent.click(dialog.querySelector('[aria-label="Close"]')!);
    expect(onClose).toHaveBeenCalledTimes(1);
    const escape = new Event('cancel', { cancelable: true });
    act(() => { dialog.dispatchEvent(escape); });
    expect(onClose).toHaveBeenCalledTimes(2);
    // A sheet is closed by whoever opened it, and not by the browser behind its back.
    expect(escape.defaultPrevented).toBe(true);
    unmount();
    expect(document.activeElement).toBe(before);
    before.remove();
  });

  it('has no foot where it was given none', () => {
    render(<Sheet title="Settings" onClose={() => {}}><p>Words.</p></Sheet>);
    expect(document.querySelector('.sheet-foot')).toBeNull();
    expect(document.querySelector('.sheet-body')?.textContent).toBe('Words.');
  });
});

describe('the settings', () => {
  const music = {
    hasTrack: false, name: undefined, localTrack: false, canRestoreLocal: false, busy: false, muted: false, setMuted: vi.fn(),
    playback: 'empty', problem: null, unkept: null, stands: 'none', play: vi.fn(), chooseFile: vi.fn(), remove: vi.fn(),
    restoreLocal: vi.fn(), fileInput: null,
  } as unknown as BackgroundMusic;
  const listed = (use: string | null, given: { key?: string; model?: string } = {}): Listed => ({
    use,
    services: [
      { ...service('deepseek', 'DeepSeek', { key: given.key }), model: given.model ?? 'deepseek-flash' },
      service('openai', 'OpenAI'),
      service('my-own', 'http://10.0.0.5:8000/v1', { own: true, key: 'stored' }),
    ],
  });
  /** A service process that knows both of the commands that the settings send. */
  const able = {
    protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work',
    commands: [{ name: 'permissions', summary: '' }, { name: 'model', summary: '' }],
  };
  const calls = {
    onTheme: vi.fn(), onSize: vi.fn(), onLanguage: vi.fn(), onPlayOpening: vi.fn(), onPlayNow: vi.fn(), onTellMe: vi.fn(),
    onCommand: vi.fn(), onClose: vi.fn(), onOpen: vi.fn(), onChooseModel: vi.fn(), onCheckingUpdates: vi.fn(), onSkip: vi.fn(),
    onChooseEffort: vi.fn(), onReasoning: vi.fn(),
  };
  // Nothing is known of newer versions: these settings are about other things.
  const updates = { about: null, found: null, asking: false, checkNow: vi.fn() };
  // The service lists two models, of which the first is in use.
  const deepseekModels = () => modelsWith({ deepseek: { kind: 'listed', models: ['deepseek-flash', 'deepseek-pro'], at: 0 } });
  // The words of this window are English here, whatever language the window for the key is asked for in.
  const sheet = (workspace: Workspace, part: SettingsPart = 'model', models = deepseekModels()) => <Settings
    workspace={workspace} part={part} theme="moonveil" size="standard" language="system" shownLanguage="ja" reasoning="none"
    playOpening={false} tellMe music={music} github={gitHubWith()} models={models} updates={updates} checkingUpdates
    skipped={null} notKept={null} {...calls}
  />;
  const services = () => [...one('services').querySelectorAll('li')].map(row =>
    [...row.children].map(part => part.textContent).join(' | '));
  const modelButton = () => one('model-button') as HTMLButtonElement;
  /** Whether a model can be chosen in the settings: now, once Columbina is at rest, or not there at all. */
  const modelChoice = () => {
    if (marked('model-button').length === 0) return 'none';
    if (modelButton().disabled) return 'off';
    fireEvent.click(modelButton());
    const option = one('model-panel').querySelector('[role="option"]')!;
    const choice = option.getAttribute('aria-disabled') === 'true' ? 'busy' : 'free';
    fireEvent.click(modelButton());
    return choice;
  };
  const whenIdle = 'The model and the permissions can be changed when Columbina is at rest.';
  const unavailable = 'Services are set up here when the application runs the service itself, not a replay.';
  beforeEach(() => { for (const call of Object.values(calls)) call.mockClear(); });

  it('offer to start the application with the system, in the tray, where the shell can', async () => {
    const loginItem = vi.fn(async (): Promise<unknown> => ({ offered: true, on: false }));
    const setLoginItem = vi.fn(async (on: boolean): Promise<unknown> => ({ offered: true, on }));
    vi.stubGlobal('columbina', { loginItem, setLoginItem });
    render(sheet(workspaceWith({ hello: able })));
    await act(async () => {});
    const box = () => one('login-item-on') as HTMLInputElement;
    expect(box().checked).toBe(false);
    expect(one('login-item').textContent).toContain('Start Columbina in the tray when I sign in to Windows');
    expect(one('login-item').textContent).toContain('A click on the icon in the tray opens the window.');
    await act(async () => { fireEvent.click(box()); });
    expect(setLoginItem.mock.calls).toEqual([[true]]);
    expect(box().checked).toBe(true);
    vi.unstubAllGlobals();
  });

  it('offer no start with the system where the shell offers none or says nothing of it', async () => {
    for (const shell of [
      { loginItem: vi.fn(async () => ({ offered: false, on: false })) },
      { loginItem: vi.fn(async () => { throw new Error('no such call'); }) },
      undefined,
    ]) {
      vi.stubGlobal('columbina', shell);
      render(sheet(workspaceWith({ hello: able })));
      await act(async () => {});
      expect(marked('login-item')).toEqual([]);
      cleanup();
    }
    vi.unstubAllGlobals();
  });

  it('say where the start with the system could not be changed, and leave it as it was', async () => {
    const fail = vi.fn();
    vi.stubGlobal('columbina', {
      loginItem: vi.fn(async () => ({ offered: true, on: true })),
      setLoginItem: vi.fn(async () => { throw new Error('The system said no.'); }),
    });
    render(sheet(workspaceWith({ hello: able, fail })));
    await act(async () => {});
    await act(async () => { fireEvent.click(one('login-item-on')); });
    expect(String(fail.mock.calls[0][0])).toContain('The system said no.');
    expect((one('login-item-on') as HTMLInputElement).checked).toBe(true);
    vi.unstubAllGlobals();
  });

  it('open at the part that was asked for', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    const opened = (part: SettingsPart) => {
      seen.length = 0;
      cleanup();
      render(sheet(workspaceWith(), part));
      return seen.map(section => section.querySelector('h3')?.textContent);
    };
    expect(opened('appearance')).toEqual(['Appearance']);
    expect(opened('model')).toEqual(['Model']);
    expect(opened('account')).toEqual(['GitHub account']);
    expect(opened('permissions')).toEqual(['Permissions']);
  });

  it('are closed by the one button at their foot', () => {
    render(sheet(workspaceWith()));
    expect([...document.querySelectorAll('.sheet-foot > *')].map(part => part.textContent)).toEqual(['Done']);
    fireEvent.click(button('Done'));
    expect(calls.onClose).toHaveBeenCalledTimes(1);
    expect(document.querySelector('dialog')?.getAttribute('aria-label')).toBe('Settings');
  });

  it('offer the themes, the languages and three sizes of type, and say which hold', () => {
    render(sheet(workspaceWith(), 'appearance'));
    const themes = ['moonveil', 'moonwhite'].map(theme => one(`theme-${theme}`));
    expect(themes.map(each => [each.textContent, each.getAttribute('aria-pressed')]))
      .toEqual([['Moonveil', 'true'], ['Moonwhite', 'false']]);
    fireEvent.click(themes[1]);
    expect(calls.onTheme.mock.calls).toEqual([['moonwhite']]);

    const language = one('language') as HTMLSelectElement;
    expect(language.value).toBe('system');
    expect([...language.options].map(option => [option.value, option.textContent]))
      .toEqual([['system', 'As the system has it'], ['zh', '中文'], ['en', 'English'], ['ja', '日本語']]);
    fireEvent.change(language, { target: { value: 'ja' } });
    expect(calls.onLanguage.mock.calls).toEqual([['ja']]);

    const sizes = ['small', 'standard', 'large'].map(size => one(`size-${size}`));
    expect(sizes.map(each => [each.textContent, each.getAttribute('aria-pressed')]))
      .toEqual([['Small', 'false'], ['Standard', 'true'], ['Large', 'false']]);
    fireEvent.click(sizes[2]);
    expect(calls.onSize.mock.calls).toEqual([['large']]);
  });

  it('offer the models of the service in use, and hand on another one that is chosen', () => {
    const models = deepseekModels();
    const workspace = workspaceWith({ hello: able, services: listed('deepseek') });
    const { rerender } = render(sheet(workspace, 'model', models));
    // The list opens below the button, which stands where a field would.
    expect(modelButton().textContent).toBe('deepseek-flash');
    expect(modelButton().closest('section')?.querySelector('h3')?.textContent).toBe('Model');
    fireEvent.click(modelButton());
    expect(one('model-panel').dataset.placement).toBe('down');
    expect(models.freshen).toHaveBeenCalledWith('deepseek');
    // The settings are where it leads, so it leads nowhere further.
    expect(marked('model-settings')).toEqual([]);
    const options = () => [...one('model-panel').querySelectorAll<HTMLElement>('[role="option"]')];
    expect(options().map(option => option.textContent)).toEqual(['deepseek-flash', 'deepseek-pro']);
    // The model in use changes nothing, and closes the list.
    fireEvent.click(options()[0]);
    expect(calls.onChooseModel).not.toHaveBeenCalled();
    expect(marked('model-panel')).toEqual([]);
    expect(document.activeElement).toBe(modelButton());
    fireEvent.click(modelButton());
    fireEvent.click(options()[1]);
    expect(calls.onChooseModel.mock.calls).toEqual([['deepseek-pro']]);
    expect(calls.onCommand).not.toHaveBeenCalled();
    // The button follows the model that the service says is in use.
    rerender(sheet({ ...workspace, status: statusOf({ model: { provider: 'deepseek', model: 'deepseek-pro', context_window: 1000 } }) }, 'model', models));
    expect(modelButton().textContent).toBe('deepseek-pro');
    // Escape closes the list, and the settings stay open.
    fireEvent.click(modelButton());
    expect(fireEvent.keyDown(one('models-filter'), { key: 'Escape' })).toBe(false);
    expect(marked('model-panel')).toEqual([]);
    expect(calls.onClose).not.toHaveBeenCalled();
  });

  it('send a change of what the task may do as a command, and none for what holds already', () => {
    render(sheet(workspaceWith({ hello: able })));
    const presets = ['read-only', 'workspace-write', 'full-access'].map(id => one(`preset-${id}`));
    expect(presets.map(each => [each.querySelector('strong')?.textContent, each.getAttribute('aria-pressed'), each.dataset.tone]))
      .toEqual([['Read only', 'false', undefined], ['Change files of the project', 'true', undefined], ['Full access', 'false', 'warn']]);
    expect(presets.map(each => each.querySelector('.choice-words > span')?.textContent)).toEqual([
      'Can read the files of this project, and asks before it runs a command.',
      'Can change the files of this project, and asks before it runs a command.',
      'Can read and change files anywhere on this computer, and runs commands without asking.',
    ]);
    fireEvent.click(presets[1]);
    expect(calls.onCommand).not.toHaveBeenCalled();
    fireEvent.click(presets[0]);
    fireEvent.click(presets[2]);
    expect(calls.onCommand.mock.calls).toEqual([['/permissions read-only'], ['/permissions full-access']]);
    // How well this computer holds a command to that is said under the three.
    const part = presets[0].closest('section')!;
    expect(part.querySelector('.choices + .caption')?.textContent).toBe(notConfined);
    expect(part.querySelector('.setting-now')).toBeNull();
  });

  it('say what holds where it is none of the three that are offered', () => {
    render(sheet(workspaceWith({ hello: able, status: statusOf({ sandbox: { mode: 'read-only', strength: 'full' }, approval: 'never' }) })));
    const part = one('preset-read-only').closest('section')!;
    expect(part.querySelector('.setting-now')?.textContent).toBe('Can read the files of this project, and runs only commands that a rule allows.');
    expect([...part.querySelectorAll('.choice')].map(each => each.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'false']);
    expect(part.querySelector('.choices + .caption')?.textContent).toBe('Commands are confined to what these permissions allow.');
  });

  it('say in a replay that no list of models is there', () => {
    const models = deepseekModels();
    render(sheet(workspaceWith({ hello: able, mode: 'mock' }), 'model', models));
    fireEvent.click(modelButton());
    expect(one('models-notes').textContent)
      .toBe('The models of a service are listed where the application runs the service itself, not in a replay.');
    expect(marked('models-filter')).toEqual([]);
    expect(models.freshen).not.toHaveBeenCalled();
  });

  it('offer no change while the agent is at work or cannot be asked', () => {
    const offered = (given: Partial<Workspace>) => {
      cleanup();
      render(sheet(workspaceWith({ hello: able, ...given })));
      const said = document.body.textContent!.split(whenIdle).length - 1;
      return [modelChoice(), !one('preset-read-only').hasAttribute('disabled'), said];
    };
    expect(offered({})).toEqual(['free', true, 0]);
    // Both parts say why nothing can be changed. The models can be looked at all the same.
    expect(offered({ status: atWork })).toEqual(['busy', false, 2]);
    expect(offered({ sending: true })).toEqual(['busy', false, 0]);
    expect(offered({ status: statusOf({ status: 'stopped' }) })).toEqual(['off', false, 0]);
    expect(offered({ loading: true })).toEqual(['off', false, 0]);
    expect(offered({ connected: false })).toEqual(['off', false, 0]);
    // A service process that knows one of the two commands is sent that one.
    expect(offered({ hello: { ...able, commands: [able.commands[1]] } })).toEqual(['free', false, 0]);
    expect(offered({ hello: { ...able, commands: [able.commands[0]] } })).toEqual(['off', true, 0]);
    expect(offered({ hello: null })).toEqual(['off', false, 0]);
    // Before the service has said how the task stands there is no model to show.
    expect(offered({ status: null })).toEqual(['none', true, 0]);
  });

  it('tell of the service in use, of those that the user added and of those with a key', () => {
    render(sheet(workspaceWith({ services: listed('deepseek') })));
    // A service of the user is called by the name that the user gave it, with where it is beside it.
    expect(services()).toEqual(['DeepSeek | In use | No key', 'my-own | http://10.0.0.5:8000/v1 | Key stored']);
    expect(one('services').querySelector('.service-at')?.getAttribute('title')).toBe('http://10.0.0.5:8000/v1');
    expect(one('model-service').textContent).toBe('DeepSeek');
    expect(one('set-up-service').hasAttribute('disabled')).toBe(false);
    expect(document.body.textContent).not.toContain(unavailable);
  });

  it('say of each key where it is from', () => {
    const from = (key: string, key_env: string | null = 'A_KEY') => {
      cleanup();
      render(sheet(workspaceWith({ services: { use: 'deepseek', services: [service('deepseek', 'DeepSeek', { key, key_env })] } })));
      const said = document.querySelector<HTMLElement>('.service-key')!;
      return [said.textContent, said.dataset.key];
    };
    expect(from('none')).toEqual(['No key', 'none']);
    expect(from('stored')).toEqual(['Key stored', 'stored']);
    expect(from('environment')).toEqual(['Key from a variable', 'environment']);
    expect(from('stored-for-another-address')).toEqual(['Key for another address', 'stored-for-another-address']);
    expect(from('none', null)).toEqual(['No key needed', 'none']);
  });

  it('tell of the service that the process talks to where none was chosen', () => {
    render(sheet(workspaceWith({ services: listed(null) })));
    expect(services()).toEqual(['DeepSeek | In use | No key', 'my-own | http://10.0.0.5:8000/v1 | Key stored']);
    expect(marked('stored-note')).toEqual([]);
  });

  it('say where services are set up, in a window that can set up none', () => {
    render(sheet(workspaceWith()));
    expect(marked('services')).toEqual([]);
    expect(one('set-up-service').hasAttribute('disabled')).toBe(true);
    expect(document.body.textContent).toContain(unavailable);
    // No list says what the service is called: it is named as the service process names it.
    expect(one('model-service').textContent).toBe('deepseek');
    expect(marked('stored-note')).toEqual([]);
  });

  it('open the window for the key for the service in use, in the language of the words, and once at a time', async () => {
    const workspace = workspaceWith({ services: listed('deepseek') });
    let closeIt = (_stored: null) => {};
    workspace.setUpService = vi.fn(() => new Promise<null>(resolve => { closeIt = resolve; }));
    render(sheet(workspace));
    await act(async () => { fireEvent.click(one('set-up-service')); });
    expect(vi.mocked(workspace.setUpService).mock.calls).toEqual([['deepseek', 'ja']]);
    expect(one('set-up-service').hasAttribute('disabled')).toBe(true);
    // The window was closed and nothing was stored: nothing is asked for.
    await act(async () => { closeIt(null); });
    expect(one('set-up-service').hasAttribute('disabled')).toBe(false);
    expect(marked('stored-note')).toEqual([]);
  });

  it('ask for a new start for as long as another service is chosen than the process talks to', () => {
    const workspace = workspaceWith({ services: listed('my-own') });
    const { rerender } = render(sheet(workspace));
    expect(one('stored-note').querySelector('p')?.textContent).toBe('What was stored is used once Columbina is started again.');
    expect(services()).toContain('my-own | In use | http://10.0.0.5:8000/v1 | Key stored');
    // The model is still that of the service that the process talks to.
    expect(one('model-service').textContent).toBe('DeepSeek');
    fireEvent.click(button('Start it again', one('stored-note')));
    expect(workspace.reconnect).toHaveBeenCalledTimes(1);
    // A turn is not cut off by a new start.
    rerender(sheet({ ...workspace, status: atWork }));
    expect(button('Start it again', one('stored-note')).disabled).toBe(true);
    rerender(sheet({ ...workspace, loading: true }));
    expect(button('Start it again', one('stored-note')).disabled).toBe(true);
    // The process was started again and talks to that service.
    rerender(sheet({ ...workspace, status: statusOf({ model: { provider: 'my-own', model: 'a-model', context_window: 32_768 } }) }));
    expect(marked('stored-note')).toEqual([]);
    expect(one('model-service').textContent).toBe('my-own');
  });

  it('ask for no new start after a key alone was stored', async () => {
    const before = workspaceWith({ services: listed('deepseek') });
    const after = listed('deepseek', { key: 'stored' });
    before.setUpService = vi.fn(async () => ({ listed: after }));
    const { rerender } = render(sheet(before));
    await act(async () => { fireEvent.click(one('set-up-service')); });
    rerender(sheet({ ...before, services: after }));
    expect(services()[0]).toBe('DeepSeek | In use | Key stored');
    expect(marked('stored-note')).toEqual([]);
    // The process looks for the key with every request: it is not started again for one.
    expect(before.reconnect).not.toHaveBeenCalled();
  });

  it('start the process again at once after another model was stored, where nothing is under way', async () => {
    const before = workspaceWith({ services: listed('deepseek') });
    const after = listed('deepseek', { key: 'stored', model: 'deepseek-other' });
    before.setUpService = vi.fn(async () => ({ listed: after }));
    const { rerender } = render(sheet(before));
    await act(async () => { fireEvent.click(one('set-up-service')); });
    expect(before.reconnect).toHaveBeenCalledTimes(1);
    rerender(sheet({ ...before, services: after }));
    expect(marked('stored-note')).toEqual([]);
  });

  it('ask for a new start after another model was stored while a turn was under way, until the process is started again', async () => {
    const before = workspaceWith({ services: listed('deepseek') });
    const after = listed('deepseek', { key: 'stored', model: 'deepseek-other' });
    let closeIt = (_stored: { listed: Listed | null }) => {};
    before.setUpService = vi.fn(() => new Promise<{ listed: Listed | null }>(resolve => { closeIt = resolve; }));
    const { rerender } = render(sheet(before));
    await act(async () => { fireEvent.click(one('set-up-service')); });
    // A turn began while the window of the key was open. What counts is what holds when it is closed.
    const atWorkNow = { ...before, status: atWork };
    rerender(sheet(atWorkNow));
    await act(async () => { closeIt({ listed: after }); });
    rerender(sheet({ ...atWorkNow, services: after }));
    expect(before.reconnect).not.toHaveBeenCalled();
    expect(one('stored-note').textContent).toContain('started again');
    // A process that was started later greets anew, and knows what was stored.
    rerender(sheet({ ...before, services: after, hello: { ...before.hello! } }));
    expect(marked('stored-note')).toEqual([]);
  });

  it('leave a new start for later where something is under way or about to be, and start nothing without a project', async () => {
    const after = listed('deepseek', { key: 'stored', model: 'deepseek-other' });
    for (const given of [{ loading: true }, { sending: true }, { approval: {} as Workspace['approval'] }]) {
      cleanup();
      const busy = workspaceWith({ services: listed('deepseek'), ...given });
      busy.setUpService = vi.fn(async () => ({ listed: after }));
      render(sheet(busy));
      await act(async () => { fireEvent.click(one('set-up-service')); });
      expect(busy.reconnect).not.toHaveBeenCalled();
    }
    cleanup();
    const none = workspaceWith({ services: listed('deepseek'), hasProject: false, hello: null, status: null, sessionId: null });
    none.setUpService = vi.fn(async () => ({ listed: after }));
    render(sheet(none));
    await act(async () => { fireEvent.click(one('set-up-service')); });
    expect(none.reconnect).not.toHaveBeenCalled();
    expect(marked('stored-note')).toEqual([]);
  });

  it('let the opening be played at every start or at once, and notices be sent or not', () => {
    render(sheet(workspaceWith()));
    const opening = one('play-opening') as HTMLInputElement;
    expect(opening.checked).toBe(false);
    fireEvent.click(opening);
    expect(calls.onPlayOpening.mock.calls).toEqual([[true]]);
    fireEvent.click(button('Play the opening now'));
    expect(calls.onPlayNow).toHaveBeenCalledTimes(1);
    const tell = one('tell-me') as HTMLInputElement;
    expect(tell.checked).toBe(true);
    fireEvent.click(tell);
    expect(calls.onTellMe.mock.calls).toEqual([[false]]);
  });

  it('play no opening over a task at work or over a question that waits', () => {
    const playable = (given: Partial<Workspace>) => {
      cleanup();
      render(sheet(workspaceWith(given)));
      return !button('Play the opening now').disabled;
    };
    expect(playable({})).toBe(true);
    expect(playable({ status: atWork })).toBe(false);
    expect(playable({ loading: true })).toBe(false);
    expect(playable({ approval: {} as Workspace['approval'] })).toBe(false);
  });

  it('end with what the application is and how it is connected', () => {
    const workspace = workspaceWith();
    const { rerender } = render(sheet(workspace));
    const about = one('about');
    expect([...about.children].map(part => part.textContent)).toEqual([
      'About',
      'Columbina 0.1.0, protocol 1.',
      'Connected to the service process that runs in this folder.',
      'F:\\work',
      'Start it again',
    ]);
    // The log is the shell's to show. Without a shell there is no button for it.
    expect(marked('about-log')).toEqual([]);
    fireEvent.click(button('Start it again', about));
    expect(workspace.reconnect).toHaveBeenCalledTimes(1);
    rerender(sheet({ ...workspace, status: atWork }));
    expect(button('Start it again', about).disabled).toBe(true);
    rerender(sheet({ ...workspace, loading: true }));
    expect(button('Start it again', about).disabled).toBe(true);
  });

  it('say in the end that the service process has ended, or that this is a replay', () => {
    const said = (given: Partial<Workspace>) => {
      cleanup();
      render(sheet(workspaceWith(given)));
      return [...one('about').querySelectorAll('p')].map(part => part.textContent);
    };
    expect(said({ connected: false })).toEqual(['Columbina 0.1.0, protocol 1.', 'The service process has ended.', 'F:\\work']);
    expect(said({ mode: 'mock' }))
      .toEqual(['Columbina 0.1.0, protocol 1.', 'This is a replay of a recorded session. No model is asked for anything.', 'F:\\work']);
    expect(said({ mode: 'mock', connected: false })[1]).toBe('The service process has ended.');
    // Before any service process has answered there is no version and no folder to name.
    expect(said({ hello: null, connected: false })).toEqual(['The service process has ended.']);
  });

  it('show the log of the service process where the shell can show it', () => {
    vi.stubGlobal('columbina', {});
    const workspace = workspaceWith();
    render(sheet(workspace));
    expect(one('about-log').textContent).toBe('Show the log');
    fireEvent.click(one('about-log'));
    expect(workspace.showLog).toHaveBeenCalledTimes(1);
    expect(one('about').lastElementChild?.textContent).toBe('What goes wrong in the service process is written to a log on this computer.');
  });
});

describe('the panel beside the conversation', () => {
  /** The panel with its tab kept by whoever shows it, as the page does. */
  function Kept({ workspace, first, locale = 'en', onClose = () => {} }: {
    workspace: Workspace;
    first: InspectorTab;
    locale?: string;
    onClose?: () => void;
  }) {
    const [tab, setTab] = useState(first);
    return <Inspector workspace={workspace} tab={tab} onTab={setTab} onClose={onClose} locale={locale} />;
  }
  const tabs = () => [...document.querySelectorAll<HTMLElement>('[role="tab"]')];
  const tabsShown = () => tabs().map(tab => [tab.textContent, tab.getAttribute('aria-selected'), tab.tabIndex]);
  const panel = () => document.querySelector<HTMLElement>('[role="tabpanel"]')!;
  /** What each part of the details says, child by child. */
  const parts = () => [...panel().querySelectorAll('section')].map(part => [...part.children].map(child => child.textContent));
  const used = {
    ...projectEvents([], false),
    usage: { input_tokens: 9000, output_tokens: 1500, cache_read_tokens: 2000, cache_write_tokens: 1000 },
    lastInputTokens: 26_500,
    tools: [{ name: 'read', description: 'Read a file of the project.', parameters: {} }],
  };
  const window32 = statusOf({ model: { provider: 'deepseek', model: 'deepseek-flash', context_window: 32_000 } });

  it('has two tabs that the arrows go between, and one of them is a stop for Tab', () => {
    const onClose = vi.fn();
    render(<Kept workspace={workspaceWith()} first="changes" onClose={onClose} />);
    expect(one('inspector').getAttribute('aria-label')).toBe('Changes');
    expect(tabsShown()).toEqual([['Changes', 'true', 0], ['Details', 'false', -1]]);
    expect(panel().getAttribute('aria-labelledby')).toBe('inspector-tab-changes');
    expect(tabs().map(tab => tab.getAttribute('aria-controls'))).toEqual([panel().id, panel().id]);
    const row = document.querySelector('[role="tablist"]')!;
    const key = (name: string, held: KeyboardEventInit = {}) => !fireEvent.keyDown(row, { key: name, ...held });

    expect(key('ArrowRight')).toBe(true);
    expect(tabsShown()).toEqual([['Changes', 'false', -1], ['Details', 'true', 0]]);
    expect(document.activeElement?.id).toBe('inspector-tab-details');
    expect(one('inspector').getAttribute('aria-label')).toBe('Details');
    expect(panel().getAttribute('aria-labelledby')).toBe('inspector-tab-details');
    // The row goes round, and Home and End go to its ends.
    key('ArrowRight');
    expect(document.activeElement?.id).toBe('inspector-tab-changes');
    key('End');
    expect(document.activeElement?.id).toBe('inspector-tab-details');
    key('Home');
    expect(document.activeElement?.id).toBe('inspector-tab-changes');
    // A key with another one held down is meant for something else, and so is any other key.
    for (const held of [{ ctrlKey: true }, { altKey: true }, { metaKey: true }, { shiftKey: true }]) {
      expect(key('ArrowRight', held)).toBe(false);
    }
    expect(key('a')).toBe(false);
    expect(tabsShown()).toEqual([['Changes', 'true', 0], ['Details', 'false', -1]]);

    fireEvent.click(tabs()[1]);
    expect(tabsShown()).toEqual([['Changes', 'false', -1], ['Details', 'true', 0]]);
    fireEvent.click(document.querySelector('.inspector-head > [aria-label="Close"]')!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('lists the calls that changed a file, and counts them on their tab', () => {
    const events = log(
      begin(1), user('Tidy the notes.'),
      call('c-1', 'read', { path: 'notes.md' }), result('c-1', 'The notes.'),
      call('c-2', 'edit', { path: 'notes.md', old: 'teh', new: 'the' }), result('c-2', 'Edited notes.md.'),
      call('c-3', 'bash', { command: 'git status' }), result('c-3', 'Nothing to commit.'),
      call('c-4', 'write', { path: 'todo.md', content: 'Nothing.' }), result('c-4', 'Wrote todo.md.'),
      reply('Done.'), end(1),
    );
    render(<Kept workspace={workspaceWith({ events })} first="changes" />);
    expect(tabsShown().map(([says]) => says)).toEqual(['Changes2', 'Details']);
    expect([...panel().querySelectorAll('[data-test="tool"] .tool-object')].map(row => row.textContent)).toEqual(['notes.md', 'todo.md']);
    cleanup();
    render(<Kept workspace={workspaceWith()} first="changes" />);
    expect(tabsShown().map(([says]) => says)).toEqual(['Changes', 'Details']);
    expect(panel().textContent).toBe('No file was changed in this task.');
  });

  it('tells of the model, of how full its context is and of what the task has used', () => {
    const workspace = workspaceWith({ projection: used, status: window32, services: { use: null, services: [service('deepseek', 'DeepSeek')] } });
    render(<Kept workspace={workspace} first="details" />);
    expect(parts()).toEqual([
      ['Model', 'deepseek-flash', 'DeepSeek'],
      ['Context', '', '26.5K of 32K tokens in the last request.'],
      // What was read is what was sent anew together with what the service had kept.
      ['Tokens of this task', 'Read12KWritten1.5KCached17%'],
      ['Tools', 'readRead a file of the project.'],
      ['Task', 's-1'],
    ]);
    const meter = panel().querySelector<HTMLElement>('.meter')!;
    expect([meter.getAttribute('role'), meter.getAttribute('aria-label')]).toEqual(['img', 'Context 83%']);
    expect([meter.querySelector('span')!.style.width, meter.querySelector('span')!.dataset.tone]).toEqual(['83%', 'warn']);
  });

  it('shows the context in the tone that the status line shows it in', () => {
    const tone = (tokens: number) => {
      cleanup();
      render(<Kept workspace={workspaceWith({ projection: { ...used, lastInputTokens: tokens }, status: window32 })} first="details" />);
      return panel().querySelector<HTMLElement>('.meter span')!.dataset.tone;
    };
    expect([tone(25_200), tone(25_600), tone(30_200), tone(30_400)]).toEqual([undefined, 'warn', 'warn', 'bad']);
  });

  it('counts as the language of the page counts', () => {
    render(<Kept workspace={workspaceWith({ projection: { ...used, lastInputTokens: 12_000 }, status: window32 })} first="details" locale="zh" />);
    expect(parts()[1][2]).toBe('1.2万 of 3.2万 tokens in the last request.');
    expect(parts()[2][1]).toBe('Read1.2万Written1500Cached17%');
  });

  it('says what it does not know of: the window of a model, the tools, a task', () => {
    const unknown = statusOf({ model: { provider: 'my-own', model: 'a-model', context_window: null } });
    render(<Kept workspace={workspaceWith({ sessionId: null, status: unknown })} first="details" />);
    expect(parts()).toEqual([
      ['Model', 'a-model', 'my-own'],
      ['Context', '0 tokens in the last request.'],
      ['Tokens of this task', 'Read0Written0Cached0%'],
      ['Tools', 'The tools are known once the first request is made.'],
    ]);
    cleanup();
    render(<Kept workspace={workspaceWith({ sessionId: null, status: null })} first="details" />);
    expect(parts().map(([title]) => title)).toEqual(['Context', 'Tokens of this task', 'Tools']);
  });

  it('copies the name of the task, and says for a moment that it did', async () => {
    const writeText = vi.fn(async (_text: string) => {});
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    try {
      vi.useFakeTimers();
      render(<Kept workspace={workspaceWith()} first="details" />);
      const copy = panel().querySelector<HTMLButtonElement>('.task-id button')!;
      expect([copy.getAttribute('aria-label'), copy.title]).toEqual(['Copy', 'Copy']);
      await act(async () => { fireEvent.click(copy); });
      expect(writeText.mock.calls).toEqual([['s-1']]);
      expect([copy.getAttribute('aria-label'), copy.title]).toEqual(['Copied', 'Copied']);
      act(() => { vi.advanceTimersByTime(1599); });
      expect(copy.getAttribute('aria-label')).toBe('Copied');
      act(() => { vi.advanceTimersByTime(1); });
      expect(copy.getAttribute('aria-label')).toBe('Copy');
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('does not say that it copied what could not be copied', async () => {
    const writeText = vi.fn(async (_text: string) => { throw new Error('The clipboard is not allowed.'); });
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    try {
      render(<Kept workspace={workspaceWith()} first="details" />);
      const copy = panel().querySelector<HTMLButtonElement>('.task-id button')!;
      await act(async () => { fireEvent.click(copy); });
      expect(writeText).toHaveBeenCalledTimes(1);
      expect(copy.getAttribute('aria-label')).toBe('Copy');
    } finally {
      Reflect.deleteProperty(navigator, 'clipboard');
    }
  });
});
