// @vitest-environment jsdom
/**
 * The page as a whole, with a workspace that the test holds in its hand: what one part of
 * the page does to another, and what the page asks of the workspace.
 *
 * The film of the opening and the music are parts of their own and are not drawn here.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../src/app/App';
import { openingStorageKey } from '../src/app/opening';
import { LEAST_ROOM } from '../src/app/room';
import type { Listed } from '../src/key/form';
import type { SessionEvent } from '../src/protocol';
import { sizeStorageKey } from '../src/size';
import { projectEvents } from '../src/state';
import { themeStorageKey } from '../src/themes';
import type { Updates } from '../src/useUpdates';
import type { PendingApproval, Workspace } from '../src/useWorkspace';
import {
  begin, end, failed, gitHubWith, header, likeABrowser, log, reply, service, standing, statusOf, user, workspaceWith,
} from './mounted';

const held = vi.hoisted(() => ({
  workspace: null as unknown, github: null as unknown, updates: null as unknown,
  /** Whether the page last said that the application is to look for newer versions by itself. */
  checking: null as boolean | null,
}));
vi.mock('../src/useWorkspace', () => ({ useWorkspace: () => held.workspace }));
vi.mock('../src/useUpdates', () => ({
  useUpdates: (checking: boolean) => {
    held.checking = checking;
    return held.updates;
  },
}));
vi.mock('../src/useGitHub', async original => ({ ...await original<object>(), useGitHub: () => held.github }));
vi.mock('../src/BackgroundMusic', () => ({
  useBackgroundMusic: () => ({ hasTrack: false, muted: false, setMuted: () => {} }),
  MusicTitlebar: () => null,
  MusicSettings: () => null,
}));
vi.mock('../src/StartupVideo', async () => {
  const { createElement } = await import('react');
  return {
    // In the place of the film stands a button that ends it.
    StartupVideo: ({ onComplete }: { onComplete: () => void }) =>
      createElement('button', { type: 'button', 'data-test': 'opening', onClick: onComplete }, 'The opening'),
  };
});

/** What the page knows of newer versions of the application: nothing, unless a test says otherwise. */
const updatesWith = (given: Partial<Updates> = {}): Updates => ({ about: null, found: null, asking: false, checkNow: vi.fn(), ...given });

beforeEach(() => {
  likeABrowser();
  held.github = gitHubWith();
  held.updates = updatesWith();
  held.checking = null;
  localStorage.clear();
  // A start that has had its opening: the page is there at once.
  localStorage.setItem(openingStorageKey, 'seen');
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.size;
});

/** Draw the page of a workspace. `now` shows the page what the workspace has become since. */
function page(workspace: Workspace) {
  held.workspace = workspace;
  const drawn = render(<App />);
  return {
    now(next: Workspace) {
      held.workspace = next;
      drawn.rerender(<App />);
    },
    unmount: drawn.unmount,
  };
}

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
/** The sheet that is on top, or null where none is open. */
const sheet = () => [...document.querySelectorAll('dialog')].at(-1) ?? null;
const box = () => one('composer') as HTMLTextAreaElement;
const write = (text: string) => { fireEvent.change(box(), { target: { value: text } }); };
const enter = async () => { await act(async () => { fireEvent.keyDown(box(), { key: 'Enter' }); }); };
const hint = () => one('composer-hint').textContent;
/** Press a key with Ctrl held down, anywhere on the page. Whether the page took the key. */
const chord = (key: string, held: KeyboardEventInit = { ctrlKey: true }) => !fireEvent.keyDown(document.body, { key, ...held });
const called = (fn: unknown) => vi.mocked(fn as (...given: unknown[]) => unknown).mock.calls;

const keysIdle = 'Enter sends. Shift+Enter starts a new line. A slash opens the commands.';
const back = 'What you had sent was not read. It is back in the field.';
const tasks = [
  { id: 's-1', modified_ms: Date.now() - 60_000, first_input: 'Fix the parser.' },
  { id: 's-2', modified_ms: Date.now() - 3 * 3_600_000, first_input: 'Write the notes.' },
];
const able = {
  protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work',
  commands: [{ name: 'compact', summary: '' }, { name: 'model', summary: '' }, { name: 'permissions', summary: '' }],
};
/** What was said in the first task, and the same with a second turn under way. */
const said = log(begin(1), user('Fix the parser.'), reply('It is fixed.'), end(1));
const working = log(begin(1), user('Fix the parser.'), reply('It is fixed.'), end(1), begin(2), user('Now the tests.'));
const atWork = statusOf({ status: 'running' });
const entry = (words: string, source: string) => ({ source, message: { role: 'user', content: [{ type: 'text', text: words }] } });

/** The first task of a project with two, at rest unless a test says otherwise. */
const first = (given: Partial<Workspace> = {}) => workspaceWith({ sessions: tasks, events: said, hello: able, ...given });
/** The first task with its second turn under way. */
const firstAtWork = (given: Partial<Workspace> = {}) => first({ events: working, status: atWork, ...given });
/** A workspace once another task is shown in it. */
const inTask = (workspace: Workspace, id: string, events: SessionEvent[] = []): Workspace => ({
  ...workspace, sessionId: id, events, projection: projectEvents(events, false), openedAt: events.length,
  status: statusOf({ session_id: id }),
});
const asked = (given: Partial<PendingApproval> = {}): PendingApproval => ({
  requestId: 7, session_id: 's-1', call_id: 'c-1', tool: 'bash', summary: 'Run a command.',
  reason: 'The command is not among those that run without a question.',
  target: 'npm test', standing: null, ...given,
});

describe('what is written and not sent', () => {
  it('is kept with its task', () => {
    const task = first();
    const { now } = page(task);
    write('Now the tests.');
    fireEvent.click(marked('task')[1]);
    expect(called(task.openSession)).toEqual([['s-2']]);
    const other = inTask(task, 's-2');
    now(other);
    expect(box().value).toBe('');
    write('Shorter, please.');
    now(task);
    expect(box().value).toBe('Now the tests.');
    expect(hint()).toBe(keysIdle);
    now(other);
    expect(box().value).toBe('Shorter, please.');
  });

  it('is forgotten once it was taken, and stays where it was not', async () => {
    const task = first();
    task.send = vi.fn(async () => false);
    const { now } = page(task);
    write('Now the tests.');
    await enter();
    expect(called(task.send)).toEqual([['Now the tests.', false]]);
    expect(box().value).toBe('Now the tests.');
    const taking = { ...task, send: vi.fn(async () => true) };
    now(taking);
    await enter();
    expect(called(taking.send)).toEqual([['Now the tests.', false]]);
    expect(box().value).toBe('');
  });

  it('is not sent while nothing can be sent', async () => {
    const task = first({ sending: true });
    page(task);
    write('Now the tests.');
    await enter();
    expect(task.send).not.toHaveBeenCalled();
  });

  it('stays as it is where more was written before the line was taken', async () => {
    let take = (_taken: boolean) => {};
    const task = first();
    task.send = vi.fn(() => new Promise<boolean>(resolve => { take = resolve; }));
    page(task);
    write('Now the tests.');
    await enter();
    write('Now the tests. And the notes.');
    await act(async () => { take(true); });
    expect(box().value).toBe('Now the tests. And the notes.');
  });

  it('is forgotten for the task that it was sent from, wherever the user is when it is taken', async () => {
    let take = (_taken: boolean) => {};
    const task = first();
    task.send = vi.fn(() => new Promise<boolean>(resolve => { take = resolve; }));
    const { now } = page(task);
    write('Now the tests.');
    await enter();
    const other = inTask(task, 's-2');
    now(other);
    write('Shorter, please.');
    await act(async () => { take(true); });
    // What is written for the task that is shown is none of this.
    expect(box().value).toBe('Shorter, please.');
    now(task);
    expect(box().value).toBe('');
  });
});

describe('leaving a task', () => {
  it('asks first where a turn is under way, and keeps with the task what was sent and never read', () => {
    const waiting = {
      ...projectEvents(working, false),
      queues: { 'next-turn': [entry('Then the docs.', 'user'), entry('A reminder that a plugin put in.', 'plugin')], 'next-step': [entry('Mind the build.', 'steer')] },
    };
    const before = firstAtWork();
    const { now } = page(before);
    const task = { ...before, projection: waiting };
    now(task);
    write('A thought.');
    fireEvent.click(marked('task')[1]);
    // The turn would be stopped. A sheet asks, and begins at the answer that changes nothing.
    expect(task.openSession).not.toHaveBeenCalled();
    expect(sheet()?.getAttribute('aria-label')).toBe('Stop the turn that is under way?');
    expect(sheet()?.querySelector('.sheet-body')?.textContent).toBe('Columbina is still at work on this task. Leaving it stops the turn.');
    expect(document.activeElement?.textContent).toBe('Stay here');
    fireEvent.click(button('Stay here'));
    expect(sheet()).toBeNull();
    expect(task.openSession).not.toHaveBeenCalled();

    fireEvent.click(marked('task')[1]);
    fireEvent.click(one('confirm-leave'));
    expect(called(task.openSession)).toEqual([['s-2']]);
    expect(sheet()).toBeNull();
    now(inTask(task, 's-2'));
    expect(box().value).toBe('');
    expect(hint()).toBe(keysIdle);
    // The service has stopped the turn of the first task and dropped what waited in it, and told of neither.
    now({ ...task, status: statusOf(), projection: projectEvents(working, false) });
    expect(box().value).toBe('Then the docs.\nMind the build.\nA thought.');
    expect(hint()).toBe(back);
    // It is still so after another look at another task.
    now(inTask(task, 's-2'));
    now({ ...task, status: statusOf(), projection: projectEvents(working, false) });
    expect(box().value).toBe('Then the docs.\nMind the build.\nA thought.');
    expect(hint()).toBe(back);
  });

  it('keeps nothing of what waits in a task at rest, which the service keeps itself', () => {
    const waiting = { ...projectEvents(said, false), queues: { 'next-turn': [entry('Then the docs.', 'user')], 'next-step': [] } };
    const task = first({ projection: waiting });
    const { now } = page(task);
    write('A thought.');
    fireEvent.click(marked('task')[1]);
    expect(called(task.openSession)).toEqual([['s-2']]);
    now(inTask(task, 's-2'));
    now(task);
    expect(box().value).toBe('A thought.');
    expect(hint()).toBe(keysIdle);
  });

  it('asks the same before a new task and before another project', async () => {
    localStorage.setItem('columbina-language', 'ja');
    const task = firstAtWork();
    task.pickFolder = vi.fn(async () => 'D:\\notes');
    page(task);
    fireEvent.click(one('new-task'));
    expect(task.openSession).not.toHaveBeenCalled();
    fireEvent.click(one('confirm-leave'));
    expect(called(task.openSession)).toEqual([[]]);

    fireEvent.click(one('open-project'));
    expect(task.pickFolder).not.toHaveBeenCalled();
    await act(async () => { fireEvent.click(one('confirm-leave')); });
    // The folder is asked for in the language of the page, and the one that was chosen is opened.
    expect(called(task.pickFolder)).toEqual([['ja']]);
    expect(called(task.openFolder)).toEqual([['D:\\notes']]);
  });

  it('opens an earlier project from the first page, by where it is', async () => {
    const none = workspaceWith({
      hasProject: false, hello: null, sessionId: null, status: null, folder: null,
      recent: [{ name: 'work', path: 'F:\\work' }, { name: 'notes', path: 'D:\\notes' }],
    });
    none.pickRecent = vi.fn(async (kept: string) => kept);
    page(none);
    await act(async () => { fireEvent.click(one('recent-projects').querySelectorAll('button')[1]); });
    expect(called(none.pickRecent)).toEqual([['D:\\notes']]);
    expect(called(none.openFolder)).toEqual([['D:\\notes']]);
  });

  it('opens no project where none was chosen', async () => {
    const task = first();
    page(task);
    await act(async () => { fireEvent.click(one('open-project')); });
    expect(task.pickFolder).toHaveBeenCalledTimes(1);
    expect(task.openFolder).not.toHaveBeenCalled();
  });

  it('begins a new task, or goes to the field where the open task is a new one already', () => {
    const task = first();
    const { now } = page(task);
    fireEvent.click(one('new-task'));
    expect(called(task.openSession)).toEqual([[]]);
    now(inTask(task, 's-3'));
    box().blur();
    fireEvent.click(one('new-task'));
    expect(task.openSession).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(box());
  });
});

describe('a change of what a task may do', () => {
  it('waits for a yes where it widens what may be done, and is sent after it', async () => {
    const task = first();
    page(task);
    write('/permissions full-access');
    await enter();
    expect(task.send).not.toHaveBeenCalled();
    expect(sheet()?.getAttribute('aria-label')).toBe('Allow full access?');
    expect(sheet()?.querySelector('.sheet-body')?.textContent)
      .toBe('Columbina will run commands without asking, and nothing will hold them back.F:\\work');
    expect(document.activeElement?.textContent).toBe('Cancel');
    expect(one('confirm-permission').textContent).toBe('Allow full access');
    expect(one('confirm-permission').className).toBe('btn btn-bad');
    fireEvent.click(button('Cancel'));
    expect(sheet()).toBeNull();
    // What was written is still there to be sent or changed.
    expect(box().value).toBe('/permissions full-access');

    await enter();
    await act(async () => { fireEvent.click(one('confirm-permission')); });
    expect(called(task.send)).toEqual([['/permissions full-access', false]]);
    expect(sheet()).toBeNull();
    expect(box().value).toBe('');
  });

  it('is sent at once where it narrows what may be done', async () => {
    const task = first();
    page(task);
    write('/permissions read-only');
    await enter();
    expect(called(task.send)).toEqual([['/permissions read-only', false]]);
    expect(sheet()).toBeNull();
  });

  it('asks in other words where files may be changed and commands are still asked about', async () => {
    const task = first({ status: statusOf({ sandbox: { mode: 'read-only', strength: 'none' } }) });
    page(task);
    write('/permissions workspace-write');
    await enter();
    expect(sheet()?.getAttribute('aria-label')).toBe('Let Columbina change files?');
    expect(one('confirm-permission').textContent).toBe('Allow changes');
    expect(one('confirm-permission').className).toBe('btn btn-main');
    expect(sheet()?.querySelector('.sheet-body p')?.textContent)
      .toBe('Columbina will be able to change the files of this project. It still asks before it runs a command.');
  });

  it('is sent quietly from the settings, after the same question', async () => {
    const task = first();
    page(task);
    fireEvent.click(one('status-may'));
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    await act(async () => { fireEvent.click(one('preset-read-only')); });
    // What is changed in the settings shows there. Its answer is not written into the conversation.
    expect(called(task.send)).toEqual([['/permissions read-only', true]]);
    await act(async () => { fireEvent.click(one('preset-full-access')); });
    expect(task.send).toHaveBeenCalledTimes(1);
    expect(sheet()?.getAttribute('aria-label')).toBe('Allow full access?');
    await act(async () => { fireEvent.click(one('confirm-permission')); });
    expect(called(task.send)).toEqual([['/permissions read-only', true], ['/permissions full-access', true]]);
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    // The field is no part of this: nothing was written in it, and nothing is taken out of it.
    expect(box().value).toBe('');
  });

  it('is not sent after a yes that comes too late, when the agent has gone to work', async () => {
    const task = first();
    const { now } = page(task);
    write('/permissions full-access');
    await enter();
    now({ ...task, status: atWork });
    await act(async () => { fireEvent.click(one('confirm-permission')); });
    expect(task.send).not.toHaveBeenCalled();
    expect(sheet()).toBeNull();
  });

  it('is asked about no longer once another task is shown', async () => {
    const task = first();
    const { now } = page(task);
    write('/permissions full-access');
    await enter();
    expect(sheet()).not.toBeNull();
    now(inTask(task, 's-2'));
    expect(sheet()).toBeNull();
  });
});

describe('choosing another model', () => {
  const deepseek: Listed = { use: 'deepseek', services: [service('deepseek', 'DeepSeek', { key: 'stored' }), service('openai', 'OpenAI')] };
  /** A shell that lists two models for the service in use, and stores what it is given unless a test says otherwise. */
  const shell = (chooseModel = vi.fn(async (_asked: unknown): Promise<unknown> => ({ stored: { name: 'deepseek', model: 'deepseek-pro' } }))) => {
    const listModels = vi.fn(async (service: string) => ({
      models: service === 'ollama' ? ['qwen3:8b', 'llama3.2:3b'] : ['deepseek-flash', 'deepseek-pro'],
    }));
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), listModels, chooseModel });
    return { listModels, chooseModel };
  };
  /** Open the list of models at the foot of the field, and choose the second one. */
  const choose = async () => {
    await act(async () => { fireEvent.click(one('model-button')); });
    await act(async () => { fireEvent.click([...document.querySelectorAll('[role="option"]')][1]); });
  };
  const notKept = 'The model is in use now, and could not be kept for the next start.';

  it('tells the service process at once, without a line in the conversation, and keeps the model for the next start', async () => {
    const { listModels, chooseModel } = shell();
    const task = first({ services: deepseek });
    page(task);
    await choose();
    expect(listModels.mock.calls).toEqual([['deepseek']]);
    expect(called(task.send)).toEqual([['/model deepseek-pro', true]]);
    expect(chooseModel.mock.calls).toEqual([[{ service: 'deepseek', model: 'deepseek-pro' }]]);
    // What is set up is read again, with the model that is kept now.
    expect(task.refreshServices).toHaveBeenCalledTimes(1);
    expect(task.fail).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(box());
    // Another service, and another model of it.
    cleanup();
    const local = first({
      services: { use: 'ollama', services: [service('ollama', 'Ollama', { key_env: null })] },
      status: statusOf({ model: { provider: 'ollama', model: 'qwen3:8b', context_window: null } }),
    });
    page(local);
    await choose();
    expect(listModels.mock.calls.at(-1)).toEqual(['ollama']);
    expect(called(local.send)).toEqual([['/model llama3.2:3b', true]]);
    expect(chooseModel.mock.calls.at(-1)).toEqual([{ service: 'ollama', model: 'llama3.2:3b' }]);
  });

  it('keeps nothing where another service was chosen and waits for the next start, or where the line was not taken', async () => {
    const { chooseModel } = shell();
    const waiting = first({ services: { ...deepseek, use: 'openai' } });
    page(waiting);
    await choose();
    expect(called(waiting.send)).toEqual([['/model deepseek-pro', true]]);
    cleanup();
    const refused = first({ services: deepseek, send: vi.fn(async () => false) });
    page(refused);
    await choose();
    expect(called(refused.send)).toEqual([['/model deepseek-pro', true]]);
    expect(chooseModel).not.toHaveBeenCalled();
  });

  it('says that the model is in use and not kept, where the setup program does not keep it', async () => {
    shell(vi.fn(async () => ({ error: 'what is stored cannot be read' })));
    const told = first({ services: deepseek });
    page(told);
    await choose();
    expect(called(told.fail)).toEqual([[`${notKept} What is stored cannot be read.`]]);
    expect(told.refreshServices).not.toHaveBeenCalled();
    cleanup();
    shell(vi.fn(async () => { throw new Error('The setup program did not end in time.'); }));
    const late = first({ services: deepseek });
    page(late);
    await choose();
    expect(called(late.fail)).toEqual([[`${notKept} The setup program did not end in time.`]]);
  });

  it('says in the settings that a model that is chosen there is not kept, until another is kept', async () => {
    const { chooseModel } = shell(vi.fn(async (): Promise<unknown> => ({ error: 'what is stored cannot be read' })));
    page(first({ services: deepseek }));
    chord(',');
    const settings = sheet()!;
    const chooseThere = async () => {
      await act(async () => { fireEvent.click(settings.querySelector('[data-test="model-button"]')!); });
      await act(async () => { fireEvent.click([...settings.querySelectorAll('[role="option"]')][1]); });
    };
    await chooseThere();
    const said = settings.querySelector('[data-test="model-not-kept"]')!;
    expect(said.getAttribute('role')).toBe('alert');
    expect(said.textContent).toBe(`${notKept} What is stored cannot be read.`);
    chooseModel.mockImplementation(async () => ({ stored: { name: 'deepseek', model: 'deepseek-pro' }, key_in: null }));
    await chooseThere();
    expect(settings.querySelectorAll('[data-test="model-not-kept"]')).toHaveLength(0);
  });

  it('leads from the foot of the field to the settings, at the model', async () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    shell();
    page(first({ services: deepseek }));
    await act(async () => { fireEvent.click(one('model-button')); });
    fireEvent.click(one('model-settings'));
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    expect(seen.filter(each => each.tagName === 'SECTION').map(each => each.querySelector('h3')?.textContent)).toEqual(['Model']);
    expect(one('model-service').textContent).toBe('DeepSeek');
    // The settings have the list of their own, and the field's is closed.
    expect(marked('model-button')).toHaveLength(2);
    expect(marked('model-panel')).toEqual([]);
  });
});

describe('files that go with a message', () => {
  const kept = (...names: string[]) => names.map(name => `.columbina/attachments/20261016-153012/${name}`);
  const notes = { id: 'attachment-1', name: 'notes.md', size: 1200 };
  const shot = { id: 'attachment-2', name: 'shot.png', size: 3_500_000 };
  /** A shell that holds what the dialog gives, and copies what it holds into the project. */
  const shell = (chosen: unknown = { attached: [notes, shot], refused: [] }) => {
    const bridge = {
      setTheme: vi.fn(async () => {}),
      chooseAttachments: vi.fn(async (_asked: unknown): Promise<unknown> => chosen),
      attachDropped: vi.fn(async (_files: File[]): Promise<unknown> => ({ attached: [notes], refused: [] })),
      forgetAttachments: vi.fn(async (_ids: string[]) => {}),
      keepAttachments: vi.fn(async (ids: string[]): Promise<unknown> => ({
        kept: kept(...ids.map(id => (id === shot.id ? shot.name : notes.name))),
      })),
    };
    vi.stubGlobal('columbina', bridge);
    return bridge;
  };
  const attach = async () => { await act(async () => { fireEvent.click(one('attach')); }); };
  /** The files of the message, as their cards show them. */
  const files = () => marked('attachment').map(card => card.textContent);
  const notes_ = () => marked('attachments-notes').map(said => said.textContent);

  it('are chosen with the button at the foot of the field, shown by name and size, and sent with the message', async () => {
    const bridge = shell();
    const task = first();
    page(task);
    expect(one('attach').getAttribute('aria-label')).toBe('Attach files');
    await attach();
    expect(bridge.chooseAttachments).toHaveBeenCalledTimes(1);
    expect(files()).toEqual(['notes.md1 KB', 'shot.png3.3 MB']);
    expect(one('attachments').getAttribute('aria-label')).toBe('Files that go with the message');
    // Of a picture the model has the file, and not what it shows.
    expect(notes_()).toEqual(['The model gets the file, and does not see the picture.']);
    expect(document.activeElement).toBe(box());
    write('  Sum these up.  ');
    await enter();
    expect(bridge.keepAttachments.mock.calls).toEqual([[['attachment-1', 'attachment-2']]]);
    expect(called(task.send)).toEqual([[
      'Sum these up.\n\nAttached files:\n- .columbina/attachments/20261016-153012/notes.md\n'
        + '- .columbina/attachments/20261016-153012/shot.png',
      false,
    ]]);
    expect(bridge.forgetAttachments.mock.calls).toEqual([[['attachment-1', 'attachment-2']]]);
    expect(marked('attachments')).toEqual([]);
    expect(box().value).toBe('');
  });

  it('go alone where nothing is written, and not with a command, which leaves them for the next message', async () => {
    const bridge = shell({ attached: [notes], refused: [] });
    const task = first();
    page(task);
    await attach();
    // Nothing is written, and the files go all the same.
    expect((one('send') as HTMLButtonElement).disabled).toBe(false);
    // Past the name of the command its list is closed, and Enter sends.
    write('/compact ');
    expect(hint()).toBe('A command takes no files. They wait for the next message.');
    await enter();
    expect(called(task.send)).toEqual([['/compact ', false]]);
    expect(bridge.keepAttachments).not.toHaveBeenCalled();
    expect(files()).toEqual(['notes.md1 KB']);
    write('');
    await act(async () => { fireEvent.click(one('send')); });
    expect(called(task.send).at(-1)).toEqual(['Attached files:\n- .columbina/attachments/20261016-153012/notes.md', false]);
    expect(files()).toEqual([]);
  });

  it('are kept with their task, as its draft is', async () => {
    shell({ attached: [notes], refused: [{ name: 'film.mp4', why: 'tooBig' }] });
    const task = first();
    const { now } = page(task);
    await attach();
    expect(notes_()).toEqual(['film.mp4 is larger than 25 MB.']);
    now(inTask(task, 's-2'));
    expect(marked('attachments')).toEqual([]);
    // What was said of the files that were not taken was said of that moment, and is gone.
    expect(marked('attachments-notes')).toEqual([]);
    now(task);
    expect(files()).toEqual(['notes.md1 KB']);
    expect(marked('attachments-notes')).toEqual([]);
  });

  it('say why a file was not taken, and take ten at most', async () => {
    const eleven = Array.from({ length: 11 }, (_unused, index) => ({ id: `attachment-${index + 1}`, name: `part-${index + 1}.md`, size: 10 }));
    const bridge = shell({ attached: eleven, refused: [{ name: 'film.mp4', why: 'tooBig' }, { name: 'photos', why: 'notAFile' }] });
    page(first());
    await attach();
    expect(files()).toHaveLength(10);
    expect(notes_()).toEqual([
      'film.mp4 is larger than 25 MB.photos is a folder, and only files can be attached.'
        + 'part-11.md was left out: ten files at most go with one message.',
    ]);
    expect(bridge.forgetAttachments.mock.calls).toEqual([[['attachment-11']]]);
    // A file that is taken out makes room, and the notes go.
    await act(async () => { fireEvent.click(marked('attachment-remove')[0]); });
    expect(files()).toHaveLength(9);
    expect(bridge.forgetAttachments.mock.calls.at(-1)).toEqual([['attachment-1']]);
    expect(marked('attachments-notes')).toEqual([]);
  });

  it('are taken out again with their buttons, which name them', async () => {
    const bridge = shell();
    page(first());
    await attach();
    const remove = marked('attachment-remove');
    expect(remove.map(button => button.getAttribute('aria-label'))).toEqual(['Remove notes.md', 'Remove shot.png']);
    // The keyboard was on the button, which goes with its file.
    remove[1].focus();
    await act(async () => { fireEvent.click(remove[1]); });
    expect(files()).toEqual(['notes.md1 KB']);
    expect(marked('attachments-notes')).toEqual([]);
    expect(bridge.forgetAttachments.mock.calls).toEqual([[['attachment-2']]]);
    expect(document.activeElement).toBe(box());
  });

  it('send nothing where the files cannot be kept in the project, and say why', async () => {
    const bridge = shell({ attached: [notes], refused: [] });
    bridge.keepAttachments.mockRejectedValueOnce(new Error('no project is open, so the files have nowhere to be kept'));
    const task = first();
    page(task);
    await attach();
    write('Sum it up.');
    await enter();
    expect(task.send).not.toHaveBeenCalled();
    expect(called(task.fail)).toEqual([[
      'The files could not be kept in the project, so the message was not sent. No project is open, so the files have nowhere to be kept.',
    ]]);
    expect(files()).toEqual(['notes.md1 KB']);
    expect(box().value).toBe('Sum it up.');
    // An answer that says no place for the file is as good as none.
    bridge.keepAttachments.mockResolvedValueOnce({ kept: [] });
    await enter();
    expect(called(task.fail).at(-1)).toEqual(['The files could not be kept in the project, so the message was not sent.']);
    expect(task.send).not.toHaveBeenCalled();
  });

  it('stay where the message did not go, to go with it again', async () => {
    const bridge = shell({ attached: [notes], refused: [] });
    const task = first({ send: vi.fn(async () => false) });
    page(task);
    await attach();
    write('Sum it up.');
    await enter();
    expect(called(task.send)).toHaveLength(1);
    expect(bridge.forgetAttachments).not.toHaveBeenCalled();
    expect(files()).toEqual(['notes.md1 KB']);
  });

  it('are dropped on the field, which says where they go while they are dragged over it', async () => {
    const bridge = shell();
    page(first());
    const form = box().closest('form')!;
    const file = new File(['# Notes'], 'notes.md');
    const dragged = { dataTransfer: { types: ['Files'], files: [file], dropEffect: 'none' } };
    expect(fireEvent.dragEnter(form, dragged)).toBe(false);
    expect(one('composer-drop').textContent).toBe('Let go of the files to attach them.');
    expect(fireEvent.dragOver(form, dragged)).toBe(false);
    expect(dragged.dataTransfer.dropEffect).toBe('copy');
    // Inside the field the drag goes from one part to another, and the field still has it.
    fireEvent.dragEnter(box(), dragged);
    fireEvent.dragLeave(form, dragged);
    expect(marked('composer-drop')).toHaveLength(1);
    fireEvent.dragLeave(box(), dragged);
    expect(marked('composer-drop')).toEqual([]);
    fireEvent.dragEnter(form, dragged);
    await act(async () => { expect(fireEvent.drop(form, dragged)).toBe(false); });
    expect(bridge.attachDropped.mock.calls).toEqual([[[file]]]);
    expect(marked('composer-drop')).toEqual([]);
    expect(files()).toEqual(['notes.md1 KB']);
    // What is dragged and is no file is left to the page.
    const words = { dataTransfer: { types: ['text/plain'], files: [], dropEffect: 'none' } };
    expect(fireEvent.dragEnter(form, words)).toBe(true);
    expect(fireEvent.drop(form, words)).toBe(true);
    expect(bridge.attachDropped).toHaveBeenCalledTimes(1);
  });

  it('go alone only when the field could send a message', async () => {
    shell({ attached: [notes], refused: [] });
    const task = first();
    const { now } = page(task);
    await attach();
    expect((one('send') as HTMLButtonElement).disabled).toBe(false);
    now({ ...task, sending: true });
    expect((one('send') as HTMLButtonElement).disabled).toBe(true);
    now({ ...task, connected: false });
    expect((one('send') as HTMLButtonElement).disabled).toBe(true);
    now(task);
    expect((one('send') as HTMLButtonElement).disabled).toBe(false);
  });

  it('go with no message in a replay, and with none while the field cannot be used', async () => {
    const bridge = shell();
    const { now } = page(first({ mode: 'mock' }));
    expect(marked('attach')).toEqual([]);
    const form = box().closest('form')!;
    expect(fireEvent.drop(form, { dataTransfer: { types: ['Files'], files: [new File(['x'], 'x.md')] } })).toBe(true);
    expect(bridge.attachDropped).not.toHaveBeenCalled();
    now(first({ connected: false }));
    expect((one('attach') as HTMLButtonElement).disabled).toBe(true);
    expect(fireEvent.dragEnter(form, { dataTransfer: { types: ['Files'], files: [] } })).toBe(true);
    expect(marked('composer-drop')).toEqual([]);
  });

  it('say what went wrong where the shell could not be asked', async () => {
    const bridge = shell();
    bridge.chooseAttachments.mockRejectedValueOnce(new Error('The dialog could not be opened.'));
    const task = first();
    page(task);
    await attach();
    expect(called(task.fail)).toEqual([[new Error('The dialog could not be opened.')]]);
    expect(marked('attachments')).toEqual([]);
    // A dialog that was closed with nothing chosen changes nothing, not even what was said before.
    bridge.chooseAttachments.mockResolvedValueOnce({ attached: [notes], refused: [{ name: 'film.mp4', why: 'tooBig' }] });
    await attach();
    bridge.chooseAttachments.mockResolvedValueOnce(null);
    await attach();
    expect(files()).toEqual(['notes.md1 KB']);
    expect(notes_()).toEqual(['film.mp4 is larger than 25 MB.']);
    // Files that are dropped and cannot be taken are told of the same way.
    bridge.attachDropped.mockRejectedValueOnce(new Error('Those are no files that were dropped.'));
    const form = box().closest('form')!;
    await act(async () => { fireEvent.drop(form, { dataTransfer: { types: ['Files'], files: [new File(['x'], 'x.md')] } }); });
    expect(called(task.fail).at(-1)).toEqual([new Error('Those are no files that were dropped.')]);
    expect(files()).toEqual(['notes.md1 KB']);
  });
});

describe('a link in the conversation', () => {
  const linked = log(
    begin(1), user('Where is it?'),
    reply('See [the documentation](https://example.com/docs) or [the notes](notes.md) or [write to us](mailto:someone@example.com).'), end(1),
  );
  const links = () => [...document.querySelectorAll<HTMLButtonElement>('button.link')];

  it('shows its address before anything opens, and opens only an address of the web', () => {
    const open = vi.fn();
    vi.stubGlobal('open', open);
    page(first({ events: linked }));
    fireEvent.click(links()[0]);
    expect(sheet()?.getAttribute('aria-label')).toBe('Open this link?');
    expect(sheet()?.querySelector('pre')?.textContent).toBe('https://example.com/docs');
    expect(document.activeElement?.textContent).toBe('Cancel');
    fireEvent.click(button('Cancel'));
    expect(sheet()).toBeNull();
    expect(open).not.toHaveBeenCalled();

    fireEvent.click(links()[0]);
    fireEvent.click(button('Open it'));
    expect(open.mock.calls).toEqual([['https://example.com/docs', '_blank', 'noopener,noreferrer']]);
    expect(sheet()).toBeNull();

    for (const [index, address] of [[1, 'notes.md'], [2, 'mailto:someone@example.com']] as const) {
      fireEvent.click(links()[index]);
      expect(sheet()?.querySelector('pre')?.textContent).toBe(address);
      expect(button('Open it').disabled).toBe(true);
      fireEvent.click(button('Cancel'));
    }
    expect(open).toHaveBeenCalledTimes(1);
  });

  it('is handed to the shell where there is one, which opens it in the browser of the system', () => {
    const open = vi.fn();
    const openExternal = vi.fn(async (_address: string) => {});
    vi.stubGlobal('open', open);
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), openExternal });
    page(first({ events: linked }));
    fireEvent.click(links()[0]);
    fireEvent.click(button('Open it'));
    expect(openExternal.mock.calls).toEqual([['https://example.com/docs']]);
    expect(open).not.toHaveBeenCalled();
  });
});

describe('the keys of the page', () => {
  it('show and hide the list of tasks, begin a task, open the settings and the commands', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    const task = first();
    page(task);
    expect(marked('sidebar')).toHaveLength(1);
    expect(chord('b')).toBe(true);
    expect(marked('sidebar')).toHaveLength(0);
    expect(document.querySelector('.shell')?.getAttribute('data-sidebar')).toBe('hidden');
    // A button stands in for the list, and brings it back.
    fireEvent.click(document.querySelector('[aria-label="Show the list of tasks"]')!);
    expect(marked('sidebar')).toHaveLength(1);
    // The key is the same with the lock of the capitals on.
    expect(chord('B')).toBe(true);
    expect(chord('b', { metaKey: true })).toBe(true);
    expect(marked('sidebar')).toHaveLength(1);

    expect(chord('k')).toBe(true);
    expect(box().value).toBe('/');
    expect(marked('commands')).toHaveLength(1);
    expect(document.activeElement).toBe(box());

    expect(chord('n')).toBe(true);
    expect(called(task.openSession)).toEqual([[]]);

    expect(chord(',')).toBe(true);
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    expect(seen.map(part => part.querySelector('h3')?.textContent)).toEqual(['Appearance']);
    // With a sheet open the keys are the sheet's.
    expect(chord('b')).toBe(false);
    expect(marked('sidebar')).toHaveLength(1);
  });

  it('open the settings at their beginning from the list of tasks too', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    page(first());
    fireEvent.click(one('open-settings'));
    expect(seen.map(part => part.querySelector('h3')?.textContent)).toEqual(['Appearance']);
    fireEvent.click(button('Done'));
    expect(sheet()).toBeNull();
  });

  it('are left alone with another key held down, held down themselves, or without Ctrl', () => {
    page(first());
    for (const held of [{}, { ctrlKey: true, shiftKey: true }, { ctrlKey: true, altKey: true }, { ctrlKey: true, repeat: true }]) {
      expect(chord('b', held)).toBe(false);
      expect(chord(',', held)).toBe(false);
    }
    expect(chord('x')).toBe(false);
    expect(marked('sidebar')).toHaveLength(1);
    expect(sheet()).toBeNull();
  });

  it('begin no task and open no commands where that cannot be done', () => {
    const task = first({ connected: false, closed: '' });
    page(task);
    expect(chord('n')).toBe(false);
    expect(chord('k')).toBe(false);
    expect(task.openSession).not.toHaveBeenCalled();
    expect(box().value).toBe('');
    cleanup();
    const opening = first({ loading: true });
    page(opening);
    expect(chord('n')).toBe(false);
    expect(opening.openSession).not.toHaveBeenCalled();
  });

  it('leave the keyboard where it is when the page changes', () => {
    const task = first();
    const { now } = page(task);
    expect(document.activeElement).toBe(box());
    one('open-settings').focus();
    now(inTask(task, 's-2'));
    expect(document.activeElement).toBe(one('open-settings'));
  });

  it('hide the list from its own button too', () => {
    page(first());
    fireEvent.click(document.querySelector('[aria-label="Hide the list of tasks"]')!);
    expect(marked('sidebar')).toHaveLength(0);
  });
});

describe('a question of the agent, on the page', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-10-12T12:00:00Z'));
  });
  const settle = () => act(() => { vi.advanceTimersByTime(500); });

  it('is answered through the workspace, and the field says after a no what can be done', () => {
    const task = firstAtWork({ approval: asked() });
    const { now } = page(task);
    settle();
    fireEvent.click(one('consent-reject'));
    expect(called(task.answer)).toEqual([[7, 'rejected', false]]);
    now({ ...task, approval: null });
    expect(hint()).toBe('It was not done. You can tell Columbina how to go about it instead.');
    // Once something is written, the field says what its keys do again.
    write('Use the other script.');
    expect(hint()).toBe('What you send now is read at the next step. Esc, pressed twice, stops the turn.');
  });

  it('says nothing of a no once the turn is over', () => {
    const task = firstAtWork({ approval: asked() });
    const { now } = page(task);
    settle();
    fireEvent.click(one('consent-reject'));
    now({ ...task, approval: null, status: statusOf() });
    expect(hint()).toBe(keysIdle);
    // Nor in the turn after it.
    now({ ...task, approval: null });
    expect(hint()).toBe('What you send now is read at the next step. Esc, pressed twice, stops the turn.');
  });

  it('passes on a yes, with whether it holds for longer', () => {
    const task = firstAtWork({ approval: asked({ standing: 'this command, word for word' }) });
    page(task);
    settle();
    fireEvent.click(one('consent-always'));
    expect(called(task.answer)).toEqual([[7, 'allowed-once', true]]);
    expect(hint()).toBe('What you send now is read at the next step. Esc, pressed twice, stops the turn.');
  });

  it('has a card of its own for every question, and stops the turn from the card', () => {
    const task = firstAtWork({ approval: asked() });
    const { now } = page(task);
    settle();
    fireEvent.click(one('consent-once'));
    expect(called(task.answer)).toEqual([[7, 'allowed-once', false]]);
    now({ ...task, approval: asked({ requestId: 8, target: 'npm run build' }) });
    expect(one('consent-target').textContent).toBe('npm run build');
    // The next question waits out a first moment of its own, and is not taken for answered.
    expect((one('consent-once') as HTMLButtonElement).disabled).toBe(true);
    settle();
    fireEvent.click(one('consent-once'));
    expect(called(task.answer)).toEqual([[7, 'allowed-once', false], [8, 'allowed-once', false]]);
    fireEvent.click(one('consent-stop'));
    expect(task.cancel).toHaveBeenCalledTimes(1);
  });

  it('gives the keyboard to the field when the question is gone', () => {
    const task = firstAtWork({ approval: asked() });
    const { now } = page(task);
    box().blur();
    now({ ...task, approval: null });
    expect(document.activeElement).toBe(box());
  });
});

describe('what was sent and dropped unread', () => {
  const droppedAt = (seq: number) => ({
    ...projectEvents(working, false),
    dropped: [{ seq, source: 'steer', text: 'Mind the build.' }, { seq: seq + 1, source: 'plugin', text: 'A reminder.' }],
  });

  it('comes back into the field, before what is written there, and the field says so', () => {
    const task = firstAtWork({ openedAt: working.length });
    const { now } = page(task);
    write('A thought.');
    box().blur();
    now({ ...task, status: statusOf(), projection: droppedAt(working.length) });
    // What a plugin put in is not the user's to send again.
    expect(box().value).toBe('Mind the build.\nA thought.');
    expect(hint()).toBe(back);
    expect(document.activeElement).toBe(box());
    // It comes back once.
    now({ ...task, status: statusOf(), projection: droppedAt(working.length) });
    expect(box().value).toBe('Mind the build.\nA thought.');
    write('Mind the build, then.');
    expect(hint()).toBe(keysIdle);
  });

  it('is left alone where the log tells of it from an earlier time', () => {
    const task = first({ events: working, projection: droppedAt(3), openedAt: working.length });
    page(task);
    expect(box().value).toBe('');
    expect(hint()).toBe(keysIdle);
  });
});

describe('a service process that cannot be used', () => {
  it('is said to have ended, with what it wrote last, and a new start has the keyboard', () => {
    const task = first({ connected: false, closed: 'It ended with the code 3.', lastWords: ['Traceback', 'ValueError: there is no such profile'] });
    const { now } = page(task);
    expect(one('closed-what').textContent).toBe('The service process has ended.');
    expect(one('last-words').textContent).toBe('Traceback\nValueError: there is no such profile');
    expect(one('banner-closed').querySelector('details p')?.textContent).toBe('It ended with the code 3.');
    expect(document.activeElement).toBe(one('restart'));
    fireEvent.click(one('restart'));
    expect(task.reconnect).toHaveBeenCalledTimes(1);
    // The log is the shell's to show.
    expect(marked('show-log')).toEqual([]);
    // What was said before is still to be read.
    expect(one('title').textContent).toBe('Fix the parser.');
    // While it starts again, it is not asked to start once more.
    now({ ...task, loading: true });
    expect((one('restart') as HTMLButtonElement).disabled).toBe(true);
  });

  it('is said not to have started where it never answered, and says no more than is known', () => {
    page(workspaceWith({ hello: null, sessionId: null, status: null, connected: false, closed: '', folder: 'F:\\work' }));
    expect(one('closed-what').textContent).toBe('The service process could not be started.');
    expect(marked('last-words')).toEqual([]);
    expect(one('banner-closed').querySelector('details')).toBeNull();
    // There is nothing to welcome anyone to, and no field to write in.
    expect(marked('start')).toEqual([]);
    expect(document.querySelector('.dock')?.hasAttribute('hidden')).toBe(true);
  });

  it('shows its log where the shell can show it', () => {
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), notify: vi.fn(async () => {}) });
    const task = first({ connected: false, closed: 'It ended with the code 3.' });
    page(task);
    fireEvent.click(one('show-log'));
    expect(task.showLog).toHaveBeenCalledTimes(1);
  });

  it('says of a task that has stopped that it takes nothing more', () => {
    const task = first({ status: statusOf({ status: 'stopped', detail: 'The log could not be written.' }) });
    const { now } = page(task);
    expect(one('banner-stopped').querySelector('p')?.textContent)
      .toBe('This task has stopped and takes nothing more. Start Columbina again to go on with it.');
    expect(one('banner-stopped').querySelector('details p')?.textContent).toBe('The log could not be written.');
    expect(document.activeElement).toBe(one('restart'));
    fireEvent.click(one('restart'));
    expect(task.reconnect).toHaveBeenCalledTimes(1);
    expect(marked('banner-closed')).toEqual([]);
    now({ ...task, loading: true });
    expect((one('restart') as HTMLButtonElement).disabled).toBe(true);
    // Where the service process is gone as well, that is what is said.
    now({ ...task, connected: false, closed: 'It ended with the code 3.' });
    expect(marked('banner-stopped')).toEqual([]);
    expect(marked('banner-closed')).toHaveLength(1);
  });

  it('says of a request that failed what failed, until that is put away', () => {
    const task = first({ error: 'The service did not know the session.' });
    page(task);
    expect([...one('banner-error').querySelectorAll('p')].map(said => said.textContent))
      .toEqual(['That did not go through.', 'The service did not know the session.']);
    fireEvent.click(one('banner-error').querySelector('[aria-label="Close"]')!);
    expect(task.clearError).toHaveBeenCalledTimes(1);
  });

  it('says of what failed because the service is gone only that the service is gone', () => {
    page(first({ connected: false, closed: 'It ended with the code 3.', error: 'The service process has ended.' }));
    expect(marked('banner-error')).toEqual([]);
    expect(marked('banner-closed')).toHaveLength(1);
  });

  it('says that a start takes long, and lets it be given up then', () => {
    vi.useFakeTimers();
    const task = workspaceWith({ hello: null, sessionId: null, status: null, loading: true, folder: 'F:\\work' });
    page(task);
    expect(one('starting').querySelector('[role="status"]')?.textContent).toBe('Starting Columbina in F:\\work.');
    expect(marked('cancel-start')).toEqual([]);
    act(() => { vi.advanceTimersByTime(2999); });
    expect(marked('still-starting')).toEqual([]);
    act(() => { vi.advanceTimersByTime(1); });
    expect(one('still-starting').textContent).toBe('It is still starting.');
    fireEvent.click(one('cancel-start'));
    expect(task.cancelStart).toHaveBeenCalledTimes(1);
    // The field is not there to be written in before there is a task.
    expect(document.querySelector('.dock')?.hasAttribute('hidden')).toBe(true);
  });

  it('counts the events of a long log as they are read, and shows the conversation when all are there', () => {
    const task = first({ loading: true, reading: { read: 200, of: 5000 } });
    const { now } = page(task);
    expect(one('starting').querySelector('[role="status"]')?.textContent).toBe('Read 200 of 5000 events.');
    expect(marked('turn')).toEqual([]);
    expect(document.querySelector('.dock')?.hasAttribute('hidden')).toBe(true);
    now({ ...task, loading: false, reading: null });
    expect(marked('starting')).toEqual([]);
    expect(marked('turn')).toHaveLength(1);
    expect(document.querySelector('.dock')?.hasAttribute('hidden')).toBe(false);
    expect(one('title').textContent).toBe('Fix the parser.');
  });
});

describe('telling of what waits while the window is in the background', () => {
  const finished = log(...working.map(event => [event.type, event.data] as [string, Record<string, unknown>]), reply('The tests pass.'), end(2));
  const broken = log(...working.map(event => [event.type, event.data] as [string, Record<string, unknown>]), end(2, 'cancelled'));
  const bridge = () => {
    const notify = vi.fn(async (_notice: { title: string; body: string }) => {});
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), notify });
    return notify;
  };

  it('tells of a question once, and of how a turn ended', () => {
    const notify = bridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const task = firstAtWork();
    const { now } = page(task);
    expect(notify).not.toHaveBeenCalled();
    const asking = { ...task, approval: asked() };
    now(asking);
    expect(notify.mock.calls).toEqual([[{ title: 'Columbina needs your consent', body: 'npm test' }]]);
    now({ ...asking });
    expect(notify).toHaveBeenCalledTimes(1);
    // A call that is about no one thing is told of as the service sums it up.
    now({ ...task, approval: asked({ requestId: 8, target: null }) });
    expect(notify.mock.calls[1]).toEqual([{ title: 'Columbina needs your consent', body: 'Run a command.' }]);
    now({ ...task, status: statusOf(), events: finished, projection: projectEvents(finished, false) });
    expect(notify.mock.calls.slice(2)).toEqual([[{ title: 'Columbina has finished', body: 'Fix the parser.' }]]);
  });

  it('says that the agent has stopped where the turn did not end well, and where the service process ended', () => {
    const notify = bridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const task = firstAtWork();
    const { now } = page(task);
    now({ ...task, status: statusOf(), events: broken, projection: projectEvents(broken, false) });
    expect(notify.mock.calls).toEqual([[{ title: 'Columbina has stopped', body: 'Fix the parser.' }]]);
    now({ ...task, status: statusOf(), events: broken, projection: projectEvents(broken, false), connected: false, closed: 'It ended with the code 3.' });
    expect(notify.mock.calls.slice(1)).toEqual([[{ title: 'Columbina has stopped', body: 'The service process has ended.' }]]);
  });

  it('says of a service process that never answered that it could not be started', () => {
    const notify = bridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    const starting = workspaceWith({ hello: null, sessionId: null, status: null, loading: true });
    const { now } = page(starting);
    now({ ...starting, loading: false, connected: false, closed: '' });
    expect(notify.mock.calls).toEqual([[{ title: 'Columbina has stopped', body: 'The service process could not be started.' }]]);
  });

  it('tells of nothing while the window has the keyboard', () => {
    const notify = bridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    const task = firstAtWork();
    const { now } = page(task);
    now({ ...task, approval: asked() });
    now({ ...task, status: statusOf(), events: finished, projection: projectEvents(finished, false) });
    now({ ...task, status: statusOf(), connected: false, closed: '' });
    expect(notify).not.toHaveBeenCalled();
  });

  it('tells of nothing where the user chose not to be told', () => {
    const notify = bridge();
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    localStorage.setItem('columbina-tell-me', 'false');
    const task = firstAtWork();
    const { now } = page(task);
    now({ ...task, approval: asked() });
    now({ ...task, status: statusOf(), events: finished, projection: projectEvents(finished, false) });
    now({ ...task, status: statusOf(), events: finished, projection: projectEvents(finished, false), connected: false, closed: '' });
    expect(notify).not.toHaveBeenCalled();
    // The settings show the choice, and take it back.
    chord(',');
    const tell = one('tell-me') as HTMLInputElement;
    expect(tell.checked).toBe(false);
    fireEvent.click(tell);
    expect(localStorage.getItem('columbina-tell-me')).toBe('true');
  });
});

describe('what the page is called and what stands at its top', () => {
  it('names the window after the folder and the page after what the task began with', () => {
    page(first());
    expect(document.title).toBe('work · Columbina');
    expect(document.querySelector('.titlebar-project')?.textContent).toBe('work');
    expect(document.querySelector('.titlebar-project')?.getAttribute('title')).toBe('F:\\work');
    expect(one('title').textContent).toBe('Fix the parser.');
    expect(document.querySelector('.window')?.hasAttribute('data-native')).toBe(false);
  });

  it('names the window by itself before there is a folder', () => {
    page(workspaceWith({ hasProject: false, hello: null, sessionId: null, status: null, folder: null }));
    expect(document.title).toBe('Columbina');
    expect(one('title').textContent).toBe('');
    // Without a project there is no list of tasks and no field, and the settings have a button of their own.
    expect(marked('sidebar')).toEqual([]);
    expect(document.querySelector('.dock')?.hasAttribute('hidden')).toBe(true);
    expect(document.activeElement).toBe(one('open-project-start'));
    fireEvent.click(one('open-settings'));
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
  });

  it('counts on its button the calls that changed a file', () => {
    const changed = log(
      begin(1), user('Fix the parser.'),
      ['tool/call', { call_id: 'c-1', name: 'edit', arguments: JSON.stringify({ path: 'parser.py', old: 'teh', new: 'the' }) }],
      ['tool/result', { message: { role: 'tool', call_id: 'c-1', content: [{ type: 'text', text: 'Edited parser.py.' }] }, error: null }],
      reply('It is fixed.'), end(1),
    );
    page(first({ events: changed }));
    expect(one('show-changes').textContent).toBe('Changes1');
  });

  it('opens the panel at what changed or at the details, and closes it from the panel', () => {
    page(first());
    expect(one('show-changes').textContent).toBe('Changes');
    fireEvent.click(one('show-changes'));
    expect(one('inspector').getAttribute('aria-label')).toBe('Changes');
    // The panel has the two ways into it itself.
    expect(marked('show-changes')).toEqual([]);
    fireEvent.click(document.querySelector('.inspector-head > [aria-label="Close"]')!);
    expect(marked('inspector')).toEqual([]);
    fireEvent.click(one('show-details'));
    expect(one('inspector').getAttribute('aria-label')).toBe('Details');
  });

  it('opens the details from how full the context is, and the settings from what the task may do', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    page(first({ projection: { ...projectEvents(said, false), lastInputTokens: 2000 } }));
    fireEvent.click(one('status-context'));
    expect(one('inspector').getAttribute('aria-label')).toBe('Details');
    fireEvent.click(one('status-may'));
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    expect(seen.map(part => part.querySelector('h3')?.textContent)).toEqual(['Permissions']);
  });

  it('shows the page of a new task for a task in which nothing was said, with the field in it', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    const skills = [{ name: 'review', description: 'Look a change over.', user_invocable: true, model_invocable: true }];
    const task = inTask(first({ skills }), 's-3');
    page(task);
    expect(one('title').textContent).toBe('');
    expect(marked('show-changes')).toEqual([]);
    expect(document.querySelector('.stage')?.hasAttribute('data-empty')).toBe(true);
    expect(one('start').querySelector('h2')?.textContent).toBe('What shall we work on?');
    // The tasks that were worked on last, and the skills that there are.
    expect([...one('recent').querySelectorAll('.task-name')].map(name => name.textContent)).toEqual(['Fix the parser.', 'Write the notes.']);
    fireEvent.click(one('recent').querySelectorAll('button')[1]);
    expect(called(task.openSession)).toEqual([['s-2']]);
    box().blur();
    fireEvent.click(one('skill'));
    expect(box().value).toBe('/skill review ');
    expect(document.activeElement).toBe(box());
    fireEvent.click(one('start-may'));
    expect(seen.map(part => part.querySelector('h3')?.textContent)).toEqual(['Permissions']);
  });
});

describe('a turn that failed for its key, on the page', () => {
  const noKey = log(
    begin(1), user('/what is the mean of 1, 2 and 3?'), header('deepseek'),
    failed('AUTH', 'there is no API key for deepseek', null), end(1, 'error', 'AUTH: there is no API key for deepseek'),
  );
  const withKey = { use: 'deepseek', services: [service('deepseek', 'DeepSeek', { key: 'stored' })] };

  it('is given to the agent again as it was said, whatever it begins with', async () => {
    const task = first({ events: noKey, services: withKey });
    page(task);
    await act(async () => { fireEvent.click(one('ending-again')); });
    expect(called(task.prompt)).toEqual([['/what is the mean of 1, 2 and 3?']]);
    expect(task.send).not.toHaveBeenCalled();
  });

  it('sums up earlier messages where the context was too long for the model', async () => {
    const tooLong = log(
      begin(1), user('Go on.'), header('deepseek'),
      failed('CONTEXT_WINDOW_EXCEEDED', 'the request is longer than the context of the model', 400),
      end(1, 'error', 'CONTEXT_WINDOW_EXCEEDED: the request is longer than the context of the model'),
    );
    const task = first({ events: tooLong });
    page(task);
    expect(one('ending-action').textContent).toBe('Sum up earlier messages');
    await act(async () => {
      fireEvent.click(one('ending-action'));
      // One press does it once.
      fireEvent.click(one('ending-action'));
    });
    expect(called(task.send)).toEqual([['/compact']]);
  });

  it('leads to the part of the settings where the key is set up', () => {
    const seen: Element[] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) { seen.push(this); };
    page(first({ events: noKey, services: withKey }));
    fireEvent.click(one('ending-action'));
    expect(sheet()?.getAttribute('aria-label')).toBe('Settings');
    expect(seen.map(part => part.querySelector('h3')?.textContent)).toEqual(['Model']);
  });

  it('is not given again while something is being sent', async () => {
    const task = first({ events: noKey, services: withKey, sending: true });
    page(task);
    await act(async () => { fireEvent.click(one('ending-again')); });
    expect(task.prompt).not.toHaveBeenCalled();
  });
});

describe('the opening', () => {
  it('is shown at the first start, with the page behind it out of reach, and not at the next', () => {
    localStorage.clear();
    const task = first();
    const { unmount } = page(task);
    expect(marked('opening')).toHaveLength(1);
    const shell = document.querySelector('.shell')!;
    expect([shell.hasAttribute('inert'), shell.getAttribute('aria-hidden')]).toEqual([true, 'true']);
    // No key of the page does anything behind it.
    expect(chord(',')).toBe(false);
    expect(sheet()).toBeNull();
    // That the opening was on the screen is kept from the first moment on.
    expect(localStorage.getItem(openingStorageKey)).toBe('seen');
    // A browser lets no keyboard into a part that is out of reach. The DOM of the tests does,
    // so the field is made to let go of it, as it would not have had it.
    box().blur();
    fireEvent.click(one('opening'));
    expect(marked('opening')).toEqual([]);
    expect([shell.hasAttribute('inert'), shell.getAttribute('aria-hidden')]).toEqual([false, null]);
    expect(document.activeElement).toBe(box());
    unmount();
    page(task);
    expect(marked('opening')).toEqual([]);
  });

  it('does not keep a question of the agent waiting behind it', () => {
    localStorage.clear();
    const task = firstAtWork();
    const { now } = page(task);
    expect(marked('opening')).toHaveLength(1);
    now({ ...task, approval: asked() });
    expect(marked('opening')).toEqual([]);
    expect(marked('consent')).toHaveLength(1);
  });

  it('is shown at every start where that was chosen, and at once where that is asked for', () => {
    page(first());
    chord(',');
    const every = one('play-opening') as HTMLInputElement;
    expect(every.checked).toBe(false);
    fireEvent.click(every);
    expect(localStorage.getItem(openingStorageKey)).toBe('always');
    fireEvent.click(button('Play the opening now'));
    // The settings make way for it.
    expect(sheet()).toBeNull();
    expect(marked('opening')).toHaveLength(1);
    cleanup();
    page(first());
    expect(marked('opening')).toHaveLength(1);
    fireEvent.click(one('opening'));
    chord(',');
    expect((one('play-opening') as HTMLInputElement).checked).toBe(true);
  });
});

describe('what the window looks like', () => {
  it('is chosen in the settings, shown at once and kept', () => {
    page(first());
    expect(document.documentElement.dataset.theme).toBe('moonveil');
    expect(document.documentElement.dataset.size).toBe('standard');
    chord(',');
    fireEvent.click(one('theme-moonwhite'));
    expect(document.documentElement.dataset.theme).toBe('moonwhite');
    expect(localStorage.getItem(themeStorageKey)).toBe('moonwhite');
    fireEvent.click(one('size-large'));
    expect(document.documentElement.dataset.size).toBe('large');
    expect(localStorage.getItem(sizeStorageKey)).toBe('large');
    cleanup();
    page(first());
    expect(document.documentElement.dataset.theme).toBe('moonwhite');
    expect(document.documentElement.dataset.size).toBe('large');
  });

  it('tells the shell of the theme, so that the frame of the window goes with it', () => {
    const setTheme = vi.fn(async (_theme: string) => {});
    vi.stubGlobal('columbina', { setTheme });
    page(first());
    expect(setTheme.mock.calls).toEqual([['moonveil']]);
    expect(document.querySelector('.window')?.hasAttribute('data-native')).toBe(true);
    chord(',');
    fireEvent.click(one('theme-moonwhite'));
    expect(setTheme.mock.calls).toEqual([['moonveil'], ['moonwhite']]);
  });

  it('leaves room for the buttons of macOS at the left end of the bar, and takes it back in full screen', () => {
    let tell = (_full: boolean) => {};
    const stop = vi.fn();
    const onFullScreen = vi.fn((listener: (full: boolean) => void) => { tell = listener; return stop; });
    vi.stubGlobal('columbina', { platform: 'darwin', setTheme: vi.fn(async () => {}), onFullScreen });
    const { unmount } = page(first());
    const frame = document.querySelector('.window')!;
    expect(frame.getAttribute('data-native')).toBe('darwin');
    expect(frame.hasAttribute('data-full-screen')).toBe(false);
    act(() => tell(true));
    expect(frame.hasAttribute('data-full-screen')).toBe(true);
    act(() => tell(false));
    expect(frame.hasAttribute('data-full-screen')).toBe(false);
    expect(onFullScreen).toHaveBeenCalledOnce();
    unmount();
    expect(stop).toHaveBeenCalledOnce();
  });

  it('names the system of the window, and hears of full screen on macOS only', () => {
    const onFullScreen = vi.fn(() => () => {});
    vi.stubGlobal('columbina', { platform: 'win32', setTheme: vi.fn(async () => {}), onFullScreen });
    page(first());
    expect(document.querySelector('.window')?.getAttribute('data-native')).toBe('win32');
    expect(onFullScreen).not.toHaveBeenCalled();
  });

  it('writes the shortcut of the list of tasks with ⌘ on macOS, and with Ctrl elsewhere', () => {
    const titled = (label: string) => document.querySelector(`button[aria-label="${label}"]`)?.getAttribute('title');
    for (const [platform, written] of [['darwin', '⌘B'], ['win32', 'Ctrl+B'], [undefined, 'Ctrl+B']]) {
      cleanup();
      // Without a platform, the page is shown without the application, as in a browser.
      vi.stubGlobal('columbina', platform ? { platform, setTheme: vi.fn(async () => {}), onFullScreen: () => () => {} } : undefined);
      page(first());
      expect(titled('Hide the list of tasks'), platform).toBe(`Hide the list of tasks (${written})`);
      chord('b');
      expect(titled('Show the list of tasks'), platform).toBe(`Show the list of tasks (${written})`);
      // The key is the same on every system: ⌘ is taken where Ctrl is.
      chord('b', { metaKey: true });
      expect(titled('Hide the list of tasks'), platform).toBe(`Hide the list of tasks (${written})`);
    }
  });

  it('counts as the language counts, and asks for the window of the key in the language of its words', async () => {
    localStorage.setItem('columbina-language', 'zh');
    const task = first({
      projection: { ...projectEvents(said, false), lastInputTokens: 12_000 },
      services: { use: 'deepseek', services: [service('deepseek', 'DeepSeek')] },
    });
    page(task);
    fireEvent.click(one('status-context'));
    expect(one('inspector').textContent).toContain('1.2万');
    chord(',');
    await act(async () => { fireEvent.click(one('set-up-service')); });
    expect(called(task.setUpService)).toEqual([['deepseek', 'zh-CN']]);
  });

  it('speaks the language that was chosen, and says to the page which one it is', () => {
    page(first());
    expect(document.documentElement.lang).toBe('en');
    chord(',');
    fireEvent.change(one('language'), { target: { value: 'ja' } });
    expect(document.documentElement.lang).toBe('ja');
    expect(sheet()?.getAttribute('aria-label')).toBe('設定');
    expect(localStorage.getItem('columbina-language')).toBe('ja');
    fireEvent.change(one('language'), { target: { value: 'zh' } });
    expect(document.documentElement.lang).toBe('zh-CN');
    // Going by the system again, nothing is kept: the next start asks the system anew.
    fireEvent.change(one('language'), { target: { value: 'system' } });
    expect(localStorage.getItem('columbina-language')).toBeNull();
    expect(document.documentElement.lang).toBe('en');
  });
});

describe('newer versions of the application', () => {
  const releaseOf = (version: string) => `https://github.com/Yueran-ML/agent-harness/releases/tag/desktop-v${version}`;
  const packaged = { version: '0.1.2', checks: true };
  const newer = (version = '0.1.3') => ({ newer: { version, page: releaseOf(version) }, failure: null });
  /** A shell that opens pages, as the page asks it to. */
  const opening = () => {
    const openExternal = vi.fn(async (_address: string) => {});
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), openExternal });
    return openExternal;
  };
  /** What the settings say of the last question. */
  const state = () => one('update-state').textContent;
  const told = 'Columbina 0.1.3 is out. Open the page to download it';

  it('are pointed to at the end of the bar of the window, and the button opens the page of the release', async () => {
    const openExternal = opening();
    held.updates = updatesWith({ about: packaged, found: newer() });
    page(first());
    const button_ = one('update-told');
    expect(button_.closest('.titlebar')?.lastElementChild).toBe(button_);
    expect(button_.textContent).toBe('Update 0.1.3');
    expect([button_.getAttribute('aria-label'), button_.getAttribute('title')]).toEqual([told, told]);
    await act(async () => { fireEvent.click(button_); });
    expect(openExternal.mock.calls).toEqual([[releaseOf('0.1.3')]]);
  });

  it('are pointed to only where one was found and not skipped', () => {
    held.updates = updatesWith({ about: packaged, found: { newer: null, failure: null } });
    const { now } = page(first());
    expect(marked('update-told')).toEqual([]);
    held.updates = updatesWith({ about: packaged, found: { newer: null, failure: 'network' } });
    now(first());
    expect(marked('update-told')).toEqual([]);
    held.updates = updatesWith({ about: packaged, found: newer('0.1.4') });
    now(first());
    expect(one('update-told').textContent).toBe('Update 0.1.4');
    cleanup();
    localStorage.setItem('columbina-update-skipped', '0.1.4');
    const again = page(first());
    expect(marked('update-told')).toEqual([]);
    held.updates = updatesWith({ about: packaged, found: newer('0.1.5') });
    again.now(first());
    expect(one('update-told').textContent).toBe('Update 0.1.5');
  });

  it('are not pointed to over the opening', () => {
    localStorage.removeItem(openingStorageKey);
    held.updates = updatesWith({ about: packaged, found: newer() });
    page(first());
    expect(marked('opening')).toHaveLength(1);
    expect(marked('update-told')).toEqual([]);
    fireEvent.click(one('opening'));
    expect(one('update-told').textContent).toBe('Update 0.1.3');
  });

  it('are looked for by the application itself, unless the user says no in the settings', () => {
    held.updates = updatesWith({ about: packaged });
    page(first());
    expect(held.checking).toBe(true);
    chord(',');
    const checking = one('check-updates') as HTMLInputElement;
    expect(checking.checked).toBe(true);
    fireEvent.click(checking);
    expect([held.checking, checking.checked]).toEqual([false, false]);
    expect(localStorage.getItem('columbina-check-updates')).toBe('false');
    // The next start goes by what was chosen.
    cleanup();
    page(first());
    expect(held.checking).toBe(false);
    chord(',');
    fireEvent.click(one('check-updates'));
    expect(held.checking).toBe(true);
    expect(localStorage.getItem('columbina-check-updates')).toBe('true');
  });

  it('show in the settings the version that runs, and that one from source looks for none', () => {
    held.updates = updatesWith({ about: { version: '0.1.2', checks: false } });
    page(first());
    chord(',');
    expect(one('desktop-version').textContent).toBe('Desktop application 0.1.2.');
    expect(one('updates').textContent).toContain('Run from source, the application does not look for newer versions.');
    expect([marked('check-updates'), marked('check-now'), marked('update-state')]).toEqual([[], [], []]);
  });

  it('show nothing in the settings where the shell said nothing of the application', () => {
    page(first());
    chord(',');
    expect(one('updates').textContent).toBe('Updates');
  });

  it('show in the settings what the last question found, and ask now where the user asks', async () => {
    const openExternal = opening();
    const asking = updatesWith({ about: packaged, asking: true, found: newer() });
    held.updates = asking;
    const { now } = page(first());
    chord(',');
    expect(state()).toBe('Asking GitHub for the newest version.');
    expect((one('check-now') as HTMLButtonElement).disabled).toBe(true);
    // While it asks, nothing of an earlier answer is offered.
    expect([marked('update-open'), marked('update-skip')]).toEqual([[], []]);
    const shown = (given: Partial<Updates>) => {
      held.updates = updatesWith({ about: packaged, ...given });
      now(first());
      return state();
    };
    expect(shown({ found: null })).toBe('');
    expect(shown({ found: { newer: null, failure: null } })).toBe('This is the newest version.');
    expect(shown({ found: { newer: null, failure: 'network' } })).toBe('GitHub could not be reached.');
    expect(shown({ found: { newer: null, failure: 'busy' } })).toBe('GitHub turned the question away for now. Try again in a while.');
    expect(shown({ found: { newer: null, failure: 'unreadable' } })).toBe('No list of releases came from GitHub.');
    // A failure with no version that was found is said once, in the line itself.
    expect([marked('update-open'), marked('update-skip'), marked('update-failed')]).toEqual([[], [], []]);
    expect(shown({ found: newer() })).toBe('Columbina 0.1.3 is out.');
    expect(marked('update-failed')).toEqual([]);
    expect((one('check-now') as HTMLButtonElement).disabled).toBe(false);
    await act(async () => { fireEvent.click(one('update-open')); });
    expect(openExternal.mock.calls).toEqual([[releaseOf('0.1.3')]]);
    const shownUpdates = held.updates as Updates;
    fireEvent.click(one('check-now'));
    expect(shownUpdates.checkNow).toHaveBeenCalledTimes(1);
    expect(asking.checkNow).not.toHaveBeenCalled();
    expect(one('updates').textContent).toContain('The check asks GitHub for the list of releases of Columbina, and sends nothing else.');
    expect(one('desktop-version').textContent).toBe('Desktop application 0.1.2.');
    expect(one('updates').textContent).not.toContain('Run from source');
  });

  it('stay offered when a later look fails, which the settings say beneath', () => {
    const failed = { newer: { version: '0.1.3', page: releaseOf('0.1.3') }, failure: 'busy' as const };
    held.updates = updatesWith({ about: packaged, found: failed });
    const { now } = page(first());
    expect(one('update-told').textContent).toBe('Update 0.1.3');
    chord(',');
    expect(state()).toBe('Columbina 0.1.3 is out.');
    expect(one('update-failed').textContent).toBe('GitHub turned the question away for now. Try again in a while.');
    expect([marked('update-open'), marked('update-skip')].map(found => found.length)).toEqual([1, 1]);
    // While it asks again, what the last look said is put away.
    held.updates = updatesWith({ about: packaged, found: failed, asking: true });
    now(first());
    expect([marked('update-failed'), marked('update-open')]).toEqual([[], []]);
  });

  it('can be skipped one version at a time', () => {
    held.updates = updatesWith({ about: packaged, found: newer() });
    const { now } = page(first());
    chord(',');
    fireEvent.click(one('update-skip'));
    expect(localStorage.getItem('columbina-update-skipped')).toBe('0.1.3');
    expect([marked('update-told'), marked('update-skip')]).toEqual([[], []]);
    // The page of the release is still offered in the settings.
    expect(marked('update-open')).toHaveLength(1);
    expect(state()).toBe('Columbina 0.1.3 is out.');
    // A version after the one that was skipped is pointed to again.
    held.updates = updatesWith({ about: packaged, found: newer('0.1.4') });
    now(first());
    expect(one('update-told').textContent).toBe('Update 0.1.4');
    expect(marked('update-skip')).toHaveLength(1);
  });
});

describe('how hard the model thinks', () => {
  const levels = ['default', 'off', 'low', 'high', 'max', 'ultra'];
  /** A service process that takes the command, and its status with the level in use and those on offer. */
  const knowing = { ...able, commands: [...able.commands, { name: 'effort', summary: '' }] };
  const thinking = (effort = 'high', efforts = levels, given: Partial<ReturnType<typeof statusOf>> = {}) => statusOf({
    model: { provider: 'deepseek', model: 'deepseek-flash', context_window: null, effort, efforts }, ...given,
  });
  const task = (given: Partial<Workspace> = {}) => first({ hello: knowing, status: thinking(), ...given });
  /** A shell that keeps the level that it is given, unless a test says otherwise. */
  const shell = (chooseEffort = vi.fn(async (_asked: unknown): Promise<unknown> => ({ effort: { name: 'deepseek', effort: 'max' } }))) => {
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), chooseEffort });
    return chooseEffort;
  };
  const options = () => marked('effort-option');
  const shown = () => options().map(option => [
    option.querySelector('.effort-name')!.textContent, option.querySelector('.effort-note')!.textContent,
  ]);
  const open = async (within: ParentNode = document) => {
    await act(async () => { fireEvent.click(within.querySelector('[data-test="effort-button"]')!); });
  };
  const pick = async (level: string) => {
    await act(async () => { fireEvent.click(options().find(option => option.dataset.level === level)!); });
  };
  const notKept = 'The level of thinking is in use now, and could not be kept for the next start.';

  it('stands beside the model and offers the levels of the service, each with what it does', async () => {
    shell();
    page(task());
    const button_ = one('effort-button');
    expect(button_.textContent).toBe('Thinking: High');
    expect(one('model-button').closest('.model-picker')!.nextElementSibling).toBe(button_.closest('.model-picker'));
    // It opens above the field, where the list of commands would stand, and puts that away.
    write('/');
    expect(marked('commands')).toHaveLength(1);
    await open();
    expect(marked('commands')).toEqual([]);
    expect(one('effort-panel').dataset.placement).toBe('up');
    // It is no higher than the room that it has, which the DOM of the tests makes the least.
    expect(one('effort-panel').style.maxHeight).toBe(`${LEAST_ROOM}px`);
    expect(shown()).toEqual([
      ['Default', 'As the service decides.'],
      ['Off', 'No thinking: the fastest answers.'],
      ['Low', 'A little thinking.'],
      ['High', 'Thinks the task through.'],
      ['Max', 'The most thinking that the model does.'],
      ['Ultra', 'The most thinking, and subagents that share the work and review it. The slowest and the dearest, and only for this run.'],
    ]);
    // The level in use is marked, and the keyboard is in the list, on it.
    expect(options().filter(option => option.getAttribute('aria-current') === 'true').map(option => option.dataset.level)).toEqual(['high']);
    const list = document.activeElement as HTMLElement;
    expect(list.getAttribute('role')).toBe('listbox');
    expect(document.getElementById(list.getAttribute('aria-activedescendant')!)?.dataset.level).toBe('high');
  });

  it('tells the service process without a line in the conversation, and keeps the level for the next start', async () => {
    const chooseEffort = shell();
    const it_ = task();
    page(it_);
    await open();
    await pick('max');
    expect(called(it_.send)).toEqual([['/effort max', true]]);
    expect(chooseEffort.mock.calls).toEqual([[{ service: 'deepseek', effort: 'max' }]]);
    expect(it_.fail).not.toHaveBeenCalled();
    expect(marked('effort-panel')).toEqual([]);
    expect(document.activeElement).toBe(box());
    cleanup();
    // The level is kept for the service in use, whichever it is.
    page(task({ status: statusOf({ model: { provider: 'mine', model: 'mine-1', context_window: null, effort: 'high', efforts: levels } }) }));
    await open();
    await pick('off');
    expect(chooseEffort.mock.calls.at(-1)).toEqual([{ service: 'mine', effort: 'off' }]);
  });

  it('keeps ultra for this run alone, and nothing where the line was not taken or the level is the one in use', async () => {
    const chooseEffort = shell();
    const ultra = task();
    page(ultra);
    await open();
    await pick('ultra');
    expect(called(ultra.send)).toEqual([['/effort ultra', true]]);
    cleanup();
    const refused = task({ send: vi.fn(async () => false) });
    page(refused);
    await open();
    await pick('low');
    expect(called(refused.send)).toEqual([['/effort low', true]]);
    cleanup();
    const same = task();
    page(same);
    await open();
    await pick('high');
    expect(called(same.send)).toEqual([]);
    expect(chooseEffort).not.toHaveBeenCalled();
  });

  it('says that the level is in use and not kept, where the setup program does not keep it', async () => {
    shell(vi.fn(async () => ({ error: 'what is stored cannot be read' })));
    const told = task();
    page(told);
    await open();
    await pick('off');
    expect(called(told.fail)).toEqual([[`${notKept} What is stored cannot be read.`]]);
    cleanup();
    shell(vi.fn(async () => { throw new Error('That service is not among those that are set up.'); }));
    const late = task();
    page(late);
    await open();
    await pick('off');
    expect(called(late.fail)).toEqual([[`${notKept} That service is not among those that are set up.`]]);
  });

  it('is chosen with the keyboard, and Esc gives the keyboard back to the button', async () => {
    shell();
    const keyed = task({ status: thinking('default') });
    page(keyed);
    await open();
    const list = document.activeElement as HTMLElement;
    const at = () => document.getElementById(list.getAttribute('aria-activedescendant')!)?.dataset.level;
    fireEvent.keyDown(list, { key: 'ArrowUp' });
    expect(at()).toBe('ultra');
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    expect(at()).toBe('off');
    await act(async () => { fireEvent.keyDown(list, { key: 'Enter' }); });
    expect(called(keyed.send)).toEqual([['/effort off', true]]);
    // Opened again, the keyboard is on the level in use: the service has not told of another yet.
    await open();
    const again = document.activeElement as HTMLElement;
    expect(document.getElementById(again.getAttribute('aria-activedescendant')!)?.dataset.level).toBe('default');
    fireEvent.keyDown(again, { key: 'Escape' });
    expect(marked('effort-panel')).toEqual([]);
    expect(document.activeElement).toBe(one('effort-button'));
  });

  it('opens below the field where it has too little room above and more below', async () => {
    shell();
    page(task());
    // A panel of 400 has 92 above its button, and more below it within the window.
    const putBack = standing(element => (element.dataset.test === 'effort-panel' ? [-300, 100]
      : element.classList.contains('effort-picker') ? [108, 240] : null));
    try {
      await open();
      expect(one('effort-panel').dataset.placement).toBe('down');
      expect(one('effort-panel').style.maxHeight).toBe(`${window.innerHeight - 240 - 8 - 8}px`);
    } finally {
      putBack();
    }
  });

  it('keeps the level that the keyboard is on in sight, once the panel has its height', async () => {
    // The level that was brought into sight, and how high the panel was at that moment.
    const seen: [string | undefined, string | undefined][] = [];
    Element.prototype.scrollIntoView = function scrollIntoView(this: Element) {
      seen.push([(this as HTMLElement).dataset.level, this.closest<HTMLElement>('[data-test="effort-panel"]')?.style.maxHeight]);
    };
    shell();
    page(task());
    await open();
    expect(seen.at(-1)).toEqual(['high', `${LEAST_ROOM}px`]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' });
    expect(seen.at(-1)).toEqual(['max', `${LEAST_ROOM}px`]);
  });

  it('can be looked at and not chosen while Columbina is at work', async () => {
    shell();
    const busy = task({ status: thinking('high', levels, { status: 'running' }), events: working });
    page(busy);
    await open();
    expect(one('effort-panel').textContent).toContain('How hard the model thinks can be changed when Columbina is at rest.');
    expect(options().every(option => option.getAttribute('aria-disabled') === 'true')).toBe(true);
    await pick('max');
    expect(called(busy.send)).toEqual([]);
    // A level that is out of reach does nothing: the panel stands, and the button says the level in use.
    expect(marked('effort-panel')).toHaveLength(1);
    expect(one('effort-button').textContent).toBe('Thinking: High');
  });

  it('can be used as the field can, at the foot of the field and in the settings', async () => {
    shell();
    const whenIdle = 'How hard the model thinks can be changed when Columbina is at rest.';
    const choice = async (within: ParentNode) => {
      const opener = within.querySelector<HTMLButtonElement>('[data-test="effort-button"]')!;
      if (opener.disabled) return 'off';
      await open(within);
      const said = within.querySelector('[data-test="effort-panel"]')!.textContent!.includes(whenIdle) ? 'busy' : 'free';
      await open(within);
      return said;
    };
    for (const [given, expected] of [
      [{}, 'free'],
      [{ status: thinking('high', levels, { status: 'running' }), events: working }, 'busy'],
      [{ sending: true }, 'busy'],
      [{ loading: true }, 'off'],
      [{ connected: false }, 'off'],
      [{ status: thinking('high', levels, { status: 'stopped' }) }, 'off'],
    ] as [Partial<Workspace>, string][]) {
      page(task(given));
      expect(await choice(document.querySelector('.composer')!), JSON.stringify(given)).toBe(expected);
      chord(',');
      expect(await choice(sheet()!), `settings ${JSON.stringify(given)}`).toBe(expected);
      cleanup();
    }
    // Where there is no project, the field cannot be used.
    page(task({ hasProject: false }));
    expect(await choice(document.querySelector('.composer')!)).toBe('off');
  });

  it('offers the work of a team alone where the service is not told how much to think', async () => {
    shell();
    page(task({ status: thinking('default', ['default', 'ultra']) }));
    await open();
    expect(shown()).toEqual([
      ['Default', 'As the service decides.'],
      ['Ultra', 'Subagents that share the work and review it. The service is not asked to think harder. Only for this run.'],
    ]);
  });

  it('is offered by no program that does not know it, and not in a replay', () => {
    page(first({ status: thinking() }));
    expect(marked('effort-button')).toEqual([]);
    chord(',');
    expect(sheet()!.querySelectorAll('[data-test="effort-button"]')).toHaveLength(0);
    cleanup();
    page(task({ status: statusOf() }));
    expect(marked('effort-button')).toEqual([]);
    cleanup();
    page(task({ status: thinking('loud') }));
    expect(marked('effort-button')).toEqual([]);
    cleanup();
    page(task({ mode: 'mock' }));
    expect(marked('effort-button')).toEqual([]);
    chord(',');
    expect(sheet()!.querySelectorAll('[data-test="effort-button"]')).toHaveLength(0);
  });

  it('stands in the settings beside the model', async () => {
    shell();
    const set = task();
    page(set);
    chord(',');
    const settings = sheet()!;
    expect(settings.querySelectorAll('[data-test="effort-button"]')).toHaveLength(1);
    await open(settings);
    await pick('low');
    expect(called(set.send)).toEqual([['/effort low', true]]);
  });

  it('says in the settings that a level that is chosen there is not kept, until they are closed', async () => {
    const chooseEffort = shell(vi.fn(async () => ({ error: 'what is stored cannot be read' })));
    page(task());
    chord(',');
    await open(sheet()!);
    await pick('off');
    const said = sheet()!.querySelector('[data-test="model-not-kept"]')!;
    expect(said.getAttribute('role')).toBe('alert');
    expect(said.textContent).toBe(`${notKept} What is stored cannot be read.`);
    // A level that is kept after it says nothing more of the one before.
    chooseEffort.mockImplementation(async () => ({ effort: { name: 'deepseek', effort: 'max' } }));
    await open(sheet()!);
    await pick('max');
    expect(sheet()!.querySelectorAll('[data-test="model-not-kept"]')).toHaveLength(0);
    chooseEffort.mockImplementation(async () => ({ error: 'what is stored cannot be read' }));
    await open(sheet()!);
    await pick('low');
    expect(sheet()!.querySelectorAll('[data-test="model-not-kept"]')).toHaveLength(1);
    // Closed and opened again, the settings say nothing of it.
    fireEvent.click(button('Done', sheet()!));
    chord(',');
    expect(sheet()!.querySelectorAll('[data-test="model-not-kept"]')).toHaveLength(0);
  });
});

describe('the trace of the task that is open', () => {
  const view = () => (marked('trace').length > 0 ? 'trace' : marked('talk').length > 0 ? 'conversation' : 'neither');

  it('stands in the place of the conversation, from the title bar and with Ctrl+J', () => {
    page(first());
    expect(one('views').getAttribute('aria-label')).toBe('View of the task');
    expect(one('view-conversation').getAttribute('aria-pressed')).toBe('true');
    expect(one('view-trace').getAttribute('title')).toMatch(/^Trace \(.+J\)$/);
    expect(view()).toBe('conversation');
    fireEvent.click(one('view-trace'));
    expect(view()).toBe('trace');
    expect(one('view-trace').getAttribute('aria-pressed')).toBe('true');
    // The field stays where it is.
    expect(marked('composer')).toHaveLength(1);
    expect(chord('j')).toBe(true);
    expect(view()).toBe('conversation');
    chord('j');
    expect(view()).toBe('trace');
  });

  it('is remembered for each task, and a task that was not looked at shows its conversation', () => {
    const shown = page(first());
    fireEvent.click(one('view-trace'));
    shown.now(first({ sessionId: 's-2' }));
    expect(view()).toBe('conversation');
    shown.now(first({ sessionId: 's-1' }));
    expect(view()).toBe('trace');
  });

  it('is not offered where no task is open, or where the task has nothing in it yet', () => {
    page(first({ hasProject: false }));
    expect(marked('views')).toEqual([]);
    expect(chord('j')).toBe(false);
    cleanup();
    page(first({ events: [] }));
    expect(marked('views')).toEqual([]);
    expect(chord('j')).toBe(false);
  });
});
