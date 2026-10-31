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
import type { EngineAnswer, EngineEvent, MigrationReport } from '../src/studio/engine';
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
  id: 'm1', name: 'film.mp4', bin: null, mark_in: null, mark_out: null, path: 'F:/work/film.mp4', identity: { size: 1, modified_us: 0, digest: '0'.repeat(64) },
  facts: { duration: { ticks: 10, scale: 1 }, video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'h264', bits: 8, rotation: 0 }, audio: null },
};
/** What the page asked the transport, but for what it fetches by itself. */
// The choice of HDR on the viewers is sent at each connect (T18), as the project and the state are fetched.
const asked = () => engine.call.mock.calls.map(([method]) => method).filter(method => method !== 'project.get' && method !== 'transport.state' && method !== 'view.display');

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
      ? { ok: true, result: { revision: 0, next_id: 2, project: { schema: 3, groups: [], bins: [], next_id: 2, media: [film], timelines: [] } } }
      : method === 'transport.state'
        ? { ok: true, result: { monitor: 'source', media: 'm1', frame: 0, playing: false, speed: 1, exact: true, shown: 0, last: 249, sound: 'off', source: 'original', proxies: true } }
        : { ok: true, result: {} })),
    importFiles: vi.fn(async () => ({ ok: true, result: null })),
    viewRect: vi.fn(async () => {}),
    viewShow: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    /** Tell the page something of the engine, as the shell does. */
    tell: (event: EngineEvent) => act(() => { for (const listener of [...listeners]) listener(event); }),
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
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true, 'source']);
  });

  it('is greyed outside Windows, says why when chosen by mouse or keyboard, and opens nothing', async () => {
    const said = 'The editing studio runs only on Windows for now. macOS will come later.';
    for (const platform of ['darwin', 'linux']) {
      vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), onFullScreen: () => () => {}, engine, platform });
      page(task());
      const way = one('open-studio');
      expect(way.textContent).toBe('Studio');
      expect(way.getAttribute('aria-disabled')).toBe('true');
      expect(way.getAttribute('aria-pressed')).toBeNull();
      expect(way.title).toBe(said);
      expect(marked('studio-windows-only')).toEqual([]);
      fireEvent.click(way);
      await settle();
      expect(one('studio-windows-only').textContent).toBe(said);
      expect(way.getAttribute('aria-describedby')).toBe('studio-windows-only');
      expect(marked('studio')).toEqual([]);
      expect(shell().dataset.studio).toBeUndefined();
      expect(engine.open).not.toHaveBeenCalled();
      expect(engine.viewShow).not.toHaveBeenCalled();
      // The keyboard reaches it, and is told the same; leaving it puts the words away.
      fireEvent.blur(way);
      expect(marked('studio-windows-only')).toEqual([]);
      way.focus();
      expect(document.activeElement).toBe(way);
      fireEvent.keyDown(way, { key: 'Enter' });
      fireEvent.click(way);
      expect(one('studio-windows-only').textContent).toBe(said);
      expect(engine.open).not.toHaveBeenCalled();
      cleanup();
    }
    // On Windows the way in is the studio's, as ever.
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), engine, platform: 'win32' });
    page(task());
    expect(one('open-studio').getAttribute('aria-disabled')).toBeNull();
    expect(one('open-studio').title).toBe('');
    fireEvent.click(one('open-studio'));
    await settle();
    expect(engine.open).toHaveBeenCalledWith('F:\\work');
    expect(marked('studio')).toHaveLength(1);
  });

  it('says outside Windows, in Chinese too, that the studio comes to macOS later', async () => {
    vi.stubGlobal('columbina', { setTheme: vi.fn(async () => {}), onFullScreen: () => () => {}, engine, platform: 'darwin' });
    localStorage.setItem('columbina-language', 'zh');
    page(task());
    fireEvent.click(one('open-studio'));
    expect(one('studio-windows-only').textContent).toBe('剪辑工作室目前只支持 Windows，macOS 之后会加入。');
  });

  it('hides the view while the settings lie over it, and as it is left, and keeps the engine open', async () => {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    drawFrame();
    fireEvent.click(one('open-settings'));
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false, 'source']);
    fireEvent(document.querySelector('dialog')!, new Event('cancel'));
    expect(marked('studio')).toHaveLength(1);
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true, 'source']);
    fireEvent.click(one('close-studio'));
    expect(marked('studio')).toEqual([]);
    expect(shell().dataset.studio).toBeUndefined();
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false, 'source']);
    expect(engine.close).not.toHaveBeenCalled();
    // Opened again: the shell keeps the engine, and is told where the viewer lies before it shows.
    fireEvent.click(one('open-studio'));
    await settle();
    expect(engine.open).toHaveBeenCalledTimes(2);
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([false, 'source']);
    drawFrame();
    expect(engine.viewShow.mock.calls.at(-1)).toEqual([true, 'source']);
  });

  it('hides both views of the Edit page while the settings lie over them, and as the studio is left', async () => {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
    fireEvent.click(one('page-edit'));
    await settle();
    drawFrame();
    const shown = () => Object.fromEntries((['source', 'timeline'] as const).map(monitor => [monitor, (engine.viewShow.mock.calls as unknown as [boolean, string][]).filter(([, which]) => which === monitor).at(-1)?.[0]]));
    expect(shown()).toEqual({ source: true, timeline: true });
    fireEvent.click(one('open-settings'));
    expect(shown()).toEqual({ source: false, timeline: false });
    fireEvent(document.querySelector('dialog')!, new Event('cancel'));
    expect(shown()).toEqual({ source: true, timeline: true });
    fireEvent.click(one('close-studio'));
    expect(shown()).toEqual({ source: false, timeline: false });
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
    expect(last()).toEqual([false, 'source']);
    fireEvent.change(one('composer'), { target: { value: '' } });
    expect(last()).toEqual([true, 'source']);
    // The panel beside the conversation lies over the page only in a narrow window.
    const closePanel = () => fireEvent.click(one('inspector').querySelector('[aria-label="Close"]')!);
    fireEvent.click(one('show-details'));
    expect(marked('inspector')).toHaveLength(1);
    expect(last()).toEqual([true, 'source']);
    closePanel();
    vi.stubGlobal('matchMedia', (media: string) => ({ matches: media === '(max-width: 1120px)', media, addEventListener() {}, removeEventListener() {} }));
    fireEvent.click(one('show-details'));
    expect(last()).toEqual([false, 'source']);
    closePanel();
    expect(marked('inspector')).toEqual([]);
    expect(last()).toEqual([true, 'source']);
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

describe('the old projects of the folder, as the studio opens', () => {
  const migrated = (migrations: MigrationReport[], error: string | null = null, session = 1): EngineEvent => ({ kind: 'migrated', session, migrations, error });
  const reports: MigrationReport[] = [
    { source: 'F:/work/film', timeline: 't3', dropped: ['speech ducking comes with Fairlight in phase 3'], warnings: [] },
    { source: 'F:/work/trip', timeline: 't4', dropped: [], warnings: ['migration committed; analysis/story: no transcript'] },
    { source: 'F:/work/talk', error: 'cannot migrate F:/work/talk: legacy source a1 changed since import' },
  ];
  async function opened() {
    page(task());
    fireEvent.click(one('open-studio'));
    await settle();
  }

  it('says once which became timelines and that their files are kept, and what each left out in its details', async () => {
    await opened();
    expect(marked('migration-banner')).toEqual([]);
    engine.tell(migrated(reports));
    expect(one('migration-done').textContent).toBe('2 old projects became timelines: film, trip. Their files are kept as they were.');
    expect(one('migration-failed').textContent).toBe('An old project could not be migrated. Its files are untouched.');
    expect(marked('migration-details')).toEqual([]);
    fireEvent.click(one('migration-toggle'));
    const details = one('migration-details');
    const film = details.querySelector<HTMLElement>('[data-source="F:/work/film"]')!;
    expect(film.textContent).toContain('film');
    expect(film.textContent).toContain('Left out');
    expect(film.textContent).toContain('speech ducking comes with Fairlight in phase 3');
    const trip = details.querySelector<HTMLElement>('[data-source="F:/work/trip"]')!;
    expect(trip.textContent).not.toContain('Left out');
    expect(trip.textContent).toContain('migration committed; analysis/story: no transcript');
    const talk = details.querySelector<HTMLElement>('[data-failed="true"]')!;
    expect(talk.textContent).toContain('Not migrated');
    expect(talk.textContent).toContain('Cannot migrate F:/work/talk: legacy source a1 changed since import.');
    expect(one('migration-toggle').getAttribute('aria-expanded')).toBe('true');
    // It lies over nothing: the view of the source monitor stays shown.
    expect(document.body.dataset.overlay).toBeUndefined();
    // Put away, it is not said again, also when the studio is left and opened again.
    fireEvent.click(one('migration-close'));
    expect(marked('migration-banner')).toEqual([]);
    fireEvent.click(one('close-studio'));
    fireEvent.click(one('open-studio'));
    await settle();
    expect(marked('migration-banner')).toEqual([]);
  });

  it('names a timeline as the project has it, and is kept while the studio is left before it is told', async () => {
    engine.call.mockImplementation(async (method: string): Promise<EngineAnswer> => (method === 'project.get'
      ? { ok: true, result: { revision: 1, next_id: 5, project: { schema: 3, groups: [], bins: [], next_id: 5, media: [film], timelines: [
        { id: 't3', name: 'Film (old)', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, tracks: [],
          grade: { nodes: [], links: [{ from: 'source', to: 'output', kind: 'rgb', input: 0 }] } },
      ] } } }
      : { ok: true, result: {} }));
    await opened();
    fireEvent.click(one('close-studio'));
    engine.tell(migrated([reports[0]]));
    fireEvent.click(one('open-studio'));
    await settle();
    expect(one('migration-done').textContent).toBe('An old project became a timeline: Film (old). Its files are kept as they were.');
    expect(marked('migration-failed')).toEqual([]);
  });

  it('names three timelines in its sentence and counts the rest, and tells apart two reports of one folder', async () => {
    const failing = vi.spyOn(console, 'error');
    await opened();
    const many = ['a', 'b', 'c', 'd', 'e'].map(name => ({ source: `F:/work/${name}`, dropped: [], warnings: [] }));
    engine.tell(migrated([...many, { source: 'F:/work/x', error: 'first' }, { source: 'F:/work/x', error: 'again' }]));
    expect(one('migration-done').textContent).toBe('5 old projects became timelines: a, b, c, and 2 more. Their files are kept as they were.');
    fireEvent.click(one('migration-toggle'));
    expect(one('migration-details').querySelectorAll('li[data-source]')).toHaveLength(7);
    expect(failing.mock.calls.filter(([said]) => String(said).includes('same key'))).toEqual([]);
  });

  it('says nothing where there was nothing to say, or of an engine that was open before', async () => {
    await opened();
    engine.tell(migrated([]));
    expect(marked('migration-banner')).toEqual([]);
    engine.tell(migrated(reports, null, 0));
    expect(marked('migration-banner')).toEqual([]);
  });

  it('says where the old projects could not be looked at, and the studio works on', async () => {
    await opened();
    engine.tell(migrated([], 'another client is migrating this Studio project; try again later'));
    expect(one('migration-error').textContent).toBe('The old projects in this folder could not be looked at.');
    expect(one('migration-banner').textContent).toContain('Another client is migrating this Studio project; try again later.');
    expect(marked('migration-toggle')).toEqual([]);
    fireEvent.keyDown(document.body, { key: 'l' });
    await settle();
    expect(asked()).toEqual(['transport.play']);
  });

  it('is said in Chinese', async () => {
    localStorage.setItem('columbina-language', 'zh');
    await opened();
    engine.tell(migrated(reports));
    expect(one('migration-done').textContent).toBe('已把 2 个旧工程转成时间线：film、trip。旧文件原样保留。');
    expect(one('migration-failed').textContent).toBe('有 1 个旧工程没能迁移，它们的文件没有改动。');
    expect(one('migration-toggle').textContent).toBe('详情');
  });
});
