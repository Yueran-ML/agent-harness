// @vitest-environment jsdom
/**
 * The studio as a part of the page: its way in among the tasks, the conversation that stays
 * beside it, and the view of the engine, which is hidden as the studio is left and while the
 * settings lie over it.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../src/app/App';
import { openingStorageKey } from '../src/app/opening';
import type { EngineAnswer, EngineEvent } from '../src/studio/engine';
import type { Updates } from '../src/useUpdates';
import type { Workspace } from '../src/useWorkspace';
import { gitHubWith, likeABrowser, log, reply, user, workspaceWith } from './mounted';

const held = vi.hoisted(() => ({ workspace: null as unknown, github: null as unknown, updates: null as unknown }));
vi.mock('../src/useWorkspace', () => ({ useWorkspace: () => held.workspace }));
vi.mock('../src/useUpdates', () => ({ useUpdates: () => held.updates }));
vi.mock('../src/useGitHub', async original => ({ ...await original<object>(), useGitHub: () => held.github }));
vi.mock('../src/useAssistant', async () => {
  const { noAssistant } = await import('./assistantAside');
  return { useAssistant: () => noAssistant };
});
vi.mock('../src/BackgroundMusic', () => ({
  useBackgroundMusic: () => ({ hasTrack: false, muted: false, setMuted: () => {} }),
  MusicTitlebar: () => null,
  MusicSettings: () => null,
}));

let frames: FrameRequestCallback[] = [];
const drawFrame = () => act(() => { const now = frames; frames = []; for (const callback of now) callback(0); });

/** The engine of the studio, as the shell lets the page reach it. */
function shellEngine() {
  const listeners = new Set<(event: EngineEvent) => void>();
  return {
    open: vi.fn(async (cwd: string) => {
      const opened = { session: 1, project: `${cwd}\\.columbina\\studio`, owned: true, pid: 600, revision: 0 };
      for (const listener of [...listeners]) listener({ kind: 'opened', ...opened });
      return opened;
    }),
    call: vi.fn(async (method: string): Promise<EngineAnswer> => (method === 'project.get'
      ? { ok: true, result: { revision: 0, next_id: 1, project: { schema: 1, next_id: 1, media: [], timelines: [] } } }
      : { ok: true, result: { media: null, frame: null, playing: false, speed: 1, exact: true, shown: null, last: null, sound: 'none', source: null, proxies: true } })),
    importFiles: vi.fn(async () => ({ ok: true, result: null })),
    viewRect: vi.fn(async () => {}),
    viewShow: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  };
}

let engine: ReturnType<typeof shellEngine>;
beforeEach(() => {
  likeABrowser();
  frames = [];
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  held.github = gitHubWith();
  held.updates = { about: null, found: null, asking: false, checkNow: vi.fn() } satisfies Updates;
  localStorage.clear();
  localStorage.setItem(openingStorageKey, 'seen');
  engine = shellEngine();
  vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), engine });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.size;
});

const settle = async () => { await act(async () => { for (let at = 0; at < 10; at += 1) await new Promise(resolve => setTimeout(resolve, 0)); }); };
const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const shell = () => document.querySelector<HTMLElement>('.shell')!;
const task = (given: Partial<Workspace> = {}) => workspaceWith({ events: log(user('Cut the film.'), reply('Gladly.')), ...given });

function page(workspace: Workspace) {
  held.workspace = workspace;
  const drawn = render(<App />);
  return { now(next: Workspace) { held.workspace = next; drawn.rerender(<App />); } };
}

describe('the studio in the page', () => {
  it('is opened among the tasks for the folder of the project, beside the conversation', async () => {
    page(task());
    expect(one('open-studio').textContent).toBe('Studio');
    expect(one('open-studio').getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(one('open-studio'));
    await settle();
    expect(engine.open).toHaveBeenCalledWith('F:\\work');
    expect(marked('studio')).toHaveLength(1);
    expect(shell().dataset.studio).toBe('');
    expect(one('open-studio').getAttribute('aria-pressed')).toBe('true');
    // The conversation stays, and can be written to.
    expect(marked('composer')).toHaveLength(1);
    // The keyboard is the studio's, for its keys.
    expect(document.activeElement).toBe(one('studio'));
    drawFrame();
    expect(engine.viewRect).toHaveBeenCalledTimes(1);
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true]);
  });

  it('hides the view while the settings lie over it, and as it is left, and keeps the engine open', async () => {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    drawFrame();
    fireEvent.click(one('open-settings'));
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false]);
    fireEvent(document.querySelector('dialog')!, new Event('cancel'));
    expect(marked('studio')).toHaveLength(1);
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true]);
    fireEvent.click(one('close-studio'));
    expect(marked('studio')).toEqual([]);
    expect(shell().dataset.studio).toBeUndefined();
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false]);
    expect(engine.close).not.toHaveBeenCalled();
    // Opened again: the shell keeps the engine, and is told where the viewer lies before it shows.
    fireEvent.click(one('open-studio'));
    await settle();
    expect(engine.open).toHaveBeenCalledTimes(2);
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false]);
    drawFrame();
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true]);
  });

  it('hides the view while the list of commands or a panel over the page is open', async () => {
    const hello = { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\work', commands: [{ name: 'compact', summary: '' }] };
    page(task({ hello }));
    fireEvent.click(one('open-studio'));
    await settle();
    drawFrame();
    const last = () => engine.viewShow.mock.calls.at(-1);
    fireEvent.change(one('composer'), { target: { value: '/' } });
    expect(marked('commands')).toHaveLength(1);
    expect(last()).toEqual([false]);
    fireEvent.change(one('composer'), { target: { value: '' } });
    expect(last()).toEqual([true]);
    // The panel beside the conversation lies over the page only in a narrow window.
    const closePanel = () => fireEvent.click(one('inspector').querySelector('[aria-label="Close"]')!);
    fireEvent.click(one('show-details'));
    expect(marked('inspector')).toHaveLength(1);
    expect(last()).toEqual([true]);
    closePanel();
    vi.stubGlobal('matchMedia', (media: string) => ({ matches: media === '(max-width: 1120px)', media, addEventListener() {}, removeEventListener() {} }));
    fireEvent.click(one('show-details'));
    expect(last()).toEqual([false]);
    closePanel();
    expect(marked('inspector')).toEqual([]);
    expect(last()).toEqual([true]);
  });

  it('is left as the app goes to another folder, and is not offered in a chat', async () => {
    const drawn = page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    drawn.now(task({ hello: { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\other', commands: [] }, folder: 'F:\\other' }));
    expect(marked('studio')).toEqual([]);
    expect(one('open-studio').getAttribute('aria-pressed')).toBe('false');
    drawn.now(task({ chat: true }));
    expect(marked('open-studio')).toEqual([]);
  });
});
