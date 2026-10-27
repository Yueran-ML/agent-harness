// @vitest-environment jsdom
/**
 * The studio as a part of the page: its way in among the tasks, the conversation that stays
 * beside it, and the view of the engine, which is hidden as the studio is left and while the
 * settings lie over it.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App, { conversationKey, STUDIO_ROOMY_PX } from '../src/app/App';
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

const film = {
  id: 'm1', path: 'F:/work/film.mp4', identity: { size: 1, modified_us: 0, digest: '0'.repeat(64) },
  facts: { duration: { ticks: 10, scale: 1 }, video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'h264', bits: 8, rotation: 0 }, audio: null },
};
/** What the page asked the transport, but for what it fetches by itself. */
const asked = () => engine.call.mock.calls.map(([method]) => method).filter(method => method !== 'project.get' && method !== 'transport.state');

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
      ? { ok: true, result: { revision: 0, next_id: 2, project: { schema: 1, next_id: 2, media: [film], timelines: [] } } }
      : method === 'transport.state'
        ? { ok: true, result: { media: 'm1', frame: 0, playing: false, speed: 1, exact: true, shown: 0, last: 249, sound: 'off', source: 'original', proxies: true } }
        : { ok: true, result: {} })),
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
  // A window with room for the conversation beside the studio.
  vi.stubGlobal('innerWidth', 1440);
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

  it('is left, and its engine closed, as the app goes to another folder', async () => {
    const drawn = page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    const other = { protocol: 1, server: { name: 'columbina', version: '0.1.0' }, cwd: 'F:\\other', commands: [] };
    // The service of the other folder starts: for a moment it has said nothing.
    drawn.now(task({ hello: null, folder: 'F:\\other', loading: true, connected: false }));
    drawn.now(task({ hello: other, folder: 'F:\\other' }));
    await settle();
    expect(marked('studio')).toEqual([]);
    expect(one('open-studio').getAttribute('aria-pressed')).toBe('false');
    expect(engine.close).toHaveBeenCalledOnce();
    // The studio of the other folder opens its project in an engine of its own.
    fireEvent.click(one('open-studio'));
    await settle();
    expect(engine.open.mock.calls.map(([cwd]) => cwd)).toEqual(['F:\\work', 'F:\\other']);
    drawn.now(task({ hello: other, folder: 'F:\\other' }));
    await settle();
    expect(engine.close).toHaveBeenCalledOnce();
  });

  it('stays while the service starts again in the same folder', async () => {
    const drawn = page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    // Another model of another service, or a restart: the service is started again.
    drawn.now(task({ hello: null, loading: true, connected: false }));
    expect(marked('studio')).toHaveLength(1);
    expect(one('open-studio').getAttribute('aria-pressed')).toBe('true');
    drawn.now(task());
    await settle();
    expect(marked('studio')).toHaveLength(1);
    expect(engine.open).toHaveBeenCalledOnce();
    expect(engine.close).not.toHaveBeenCalled();
  });

  it('is left in a chat, which offers none, and its engine is kept for the folder', async () => {
    const drawn = page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    drawn.now(task({ chat: true, hello: null }));
    expect(marked('studio')).toEqual([]);
    expect(marked('open-studio')).toEqual([]);
    drawn.now(task());
    await settle();
    expect(marked('studio')).toEqual([]);
    expect(engine.close).not.toHaveBeenCalled();
  });

  it('puts the conversation away and brings it back, by its button and by Ctrl+Shift+B, and remembers it', async () => {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    expect(shell().dataset.conversation).toBeUndefined();
    expect(one('toggle-conversation').getAttribute('aria-label')).toBe('Hide the conversation');
    expect(one('toggle-conversation').title).toBe('Hide the conversation (Ctrl+Shift+B)');
    fireEvent.click(one('toggle-conversation'));
    expect(shell().dataset.conversation).toBe('hidden');
    expect(localStorage.getItem(conversationKey)).toBe('hidden');
    expect(one('toggle-conversation').getAttribute('aria-label')).toBe('Show the conversation');
    expect(fireEvent.keyDown(document.body, { key: 'B', ctrlKey: true, shiftKey: true })).toBe(false);
    expect(shell().dataset.conversation).toBeUndefined();
    expect(localStorage.getItem(conversationKey)).toBe('shown');
    // Ctrl+B still puts the tasks away.
    fireEvent.keyDown(document.body, { key: 'b', ctrlKey: true });
    expect(marked('sidebar')).toEqual([]);
    expect(shell().dataset.conversation).toBeUndefined();
    fireEvent.keyDown(document.body, { key: 'b', ctrlKey: true });
    // Put away, the choice holds the next time the studio opens.
    fireEvent.click(one('toggle-conversation'));
    fireEvent.click(one('close-studio'));
    expect(shell().dataset.conversation).toBeUndefined();
    fireEvent.click(one('open-studio'));
    await settle();
    expect(shell().dataset.conversation).toBe('hidden');
  });

  it('puts the conversation away as it opens in a narrow window, whatever was chosen', async () => {
    vi.stubGlobal('innerWidth', STUDIO_ROOMY_PX - 1);
    localStorage.setItem(conversationKey, 'shown');
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    expect(shell().dataset.conversation).toBe('hidden');
    // Ctrl+Shift+B does nothing where the studio is not shown.
    fireEvent.click(one('close-studio'));
    expect(fireEvent.keyDown(document.body, { key: 'B', ctrlKey: true, shiftKey: true })).toBe(true);
  });

  it('gives the field the keyboard again as the studio is left', async () => {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    expect(document.activeElement).toBe(one('studio'));
    // Left from among the tasks, where the button keeps the keyboard otherwise.
    one('open-studio').focus();
    fireEvent.click(one('open-studio'));
    expect(document.activeElement).toBe(one('composer'));
  });

  it('takes its keys while the panel over the page in a narrow window is closed, and none while it is open', async () => {
    vi.stubGlobal('matchMedia', (media: string) => ({ matches: media === '(max-width: 1120px)', media, addEventListener() {}, removeEventListener() {} }));
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    fireEvent.keyDown(document.body, { key: 'l' });
    await settle();
    expect(asked()).toEqual(['transport.play']);
    fireEvent.click(one('show-details'));
    expect(document.body.dataset.overlay).toBe('1');
    fireEvent.keyDown(document.body, { key: 'k' });
    fireEvent.click(one('inspector').querySelector('[aria-label="Close"]')!);
    expect(document.body.dataset.overlay).toBeUndefined();
    fireEvent.keyDown(document.body, { key: 'k' });
    await settle();
    expect(asked()).toEqual(['transport.play', 'transport.pause']);
  });

  it('leaves the keys of the conversation to it', async () => {
    page(task({ sessions: [{ id: 's-1', modified_ms: Date.now(), first_input: 'Cut the film.' }, { id: 's-2', modified_ms: Date.now(), first_input: 'Other.' }] }));
    fireEvent.click(one('open-studio'));
    await settle();
    const [, other] = marked('task');
    other.focus();
    fireEvent.keyDown(other, { key: ' ' });
    fireEvent.keyDown(one('show-details'), { key: 'End' });
    fireEvent.keyDown(one('composer'), { key: 'l' });
    await settle();
    expect(asked()).toEqual([]);
  });
});
