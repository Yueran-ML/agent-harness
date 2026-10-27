// @vitest-environment jsdom
/**
 * The studio: its media pool, its source monitor and its keys, with an engine that the test holds
 * in its hand; how the viewer tells the shell where it lies and whether the view shows; and the
 * timecode.
 */
import { useRef, useState } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { Words } from '../src/app/locale';
import { useOverlay } from '../src/app/overlay';
import { EffortPicker } from '../src/app/EffortPicker';
import { Sheet } from '../src/app/Sheet';
import { ja } from '../src/messages/ja';
import { zhCN } from '../src/messages/zh-CN';
import { createMockEngine } from '../src/mock';
import {
  ProjectCopy, type EngineAnswer, type EngineEvent, type Media, type Project, type ProxyEvent, type TransportState,
} from '../src/studio/engine';
import { commandOf, faster } from '../src/studio/keys';
import { Studio } from '../src/studio/Studio';
import { nominalFps, rateText, timecode } from '../src/studio/timecode';
import { SCRUB_EVERY_MS } from '../src/studio/Transport';
import { useViewerRect } from '../src/studio/useViewerRect';
import { likeABrowser } from './mounted';

/** Frames that the page asks to be measured at, run when the test says a frame is drawn. */
let frames: Map<number, FrameRequestCallback>;
let nextFrame = 0;
const drawFrame = () => act(() => {
  const now = [...frames.values()];
  frames.clear();
  for (const callback of now) callback(0);
});

beforeEach(() => {
  likeABrowser();
  frames = new Map();
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { nextFrame += 1; frames.set(nextFrame, callback); return nextFrame; });
  vi.stubGlobal('cancelAnimationFrame', (id: number) => { frames.delete(id); });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  delete document.body.dataset.overlay;
});

const SCALE = 15360;
const mediaOf = (id: string, given: { path?: string; video?: Partial<NonNullable<Media['facts']['video']>> | null; seconds?: number } = {}): Media => ({
  id,
  path: given.path ?? `C:\\media\\${id}.mp4`,
  identity: { size: 1000, modified_us: 1_790_000_000_000_000, digest: '0'.repeat(64) },
  facts: {
    duration: { ticks: (given.seconds ?? 10) * SCALE, scale: SCALE },
    video: given.video === null ? null : { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'h264', bits: 8, rotation: 0, ...given.video },
    audio: { rate: 48000, channels: 2, codec: 'aac' },
  },
});
const projectOf = (...media: Media[]): Project => ({ schema: 1, next_id: 10, media, timelines: [] });
const stateOf = (given: Partial<TransportState> = {}): TransportState => ({
  media: 'm1', frame: 0, playing: false, speed: 1, exact: true, shown: 0, last: 249, sound: 'off', source: 'original', proxies: true, ...given,
});

/**
 * An engine as the shell reaches it, whose project the test sets, and which tells what the test
 * says. Every call is answered `{}`, but `project.get`, `transport.state` and what the test answers.
 */
function engineWith(project: Project, given: { state?: TransportState | null; open?: () => Promise<unknown> } = {}) {
  const listeners = new Set<(event: EngineEvent) => void>();
  let revision = 0;
  const session = 3;
  const answers = new Map<string, (params: Record<string, unknown> | undefined) => EngineAnswer>();
  const tell = (event: EngineEvent) => { for (const listener of [...listeners]) listener(event); };
  const bridge = {
    open: vi.fn(async (cwd: string) => {
      if (given.open) await given.open();
      const opened = { session, project: `${cwd}\\.columbina\\studio`, owned: true, pid: 600, revision };
      tell({ kind: 'opened', ...opened });
      return opened;
    }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>): Promise<EngineAnswer> => {
      const answer = answers.get(method);
      if (answer) return answer(params);
      if (method === 'project.get') return { ok: true, result: { revision, next_id: project.next_id, project: structuredClone(project) } };
      if (method === 'transport.state') return { ok: true, result: given.state ?? stateOf({ media: null, frame: null, shown: null, last: null, source: null }) };
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async (): Promise<EngineAnswer<null>> => ({ ok: true, result: null })),
    viewRect: vi.fn(async () => {}),
    viewShow: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  };
  return {
    bridge: bridge as unknown as EngineBridge & typeof bridge,
    answers,
    session,
    tell: (event: EngineEvent) => act(() => tell(event)),
    notify: (method: string, params: unknown) => act(() => tell({ kind: 'notification', session, method, params })),
    /** What the engine is asked, without the project and the state that the copy fetches. */
    asked: () => bridge.call.mock.calls
      .filter(([method]) => method !== 'project.get' && method !== 'transport.state')
      .map(([method, params]) => (params === undefined ? [method] : [method, params])),
    setRevision(next: number) { revision = next; },
  };
}

const settle = async () => { await act(async () => { for (let at = 0; at < 10; at += 1) await new Promise(resolve => setTimeout(resolve, 0)); }); };

async function studio(engine: ReturnType<typeof engineWith>, native = true, words = undefined as undefined | typeof zhCN) {
  const onClose = vi.fn();
  const copy = new ProjectCopy(engine.bridge);
  const tree = <Studio cwd={'F:\\film'} engine={{ bridge: engine.bridge, copy, native }} language="en" onClose={onClose} />;
  const drawn = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle();
  return { onClose, copy, ...drawn };
}

const marked = (name: string) => [...document.querySelectorAll<HTMLElement>(`[data-test="${name}"]`)];
const one = (name: string) => {
  const [found, ...more] = marked(name);
  if (!found || more.length > 0) throw new Error(`There is not exactly one "${name}" on the page.`);
  return found;
};
const item = (id: string) => document.querySelector<HTMLElement>(`[data-media="${id}"]`)!;
const names = () => marked('pool-name').map(name => name.textContent);
const press = (key: string, init: KeyboardEventInit = {}, target: Element = document.body) =>
  act(() => { fireEvent.keyDown(target, { key, ...init }); });
/** Press a key and let it go. */
const tap = (key: string) => act(() => { fireEvent.keyDown(document.body, { key }); fireEvent.keyUp(document.body, { key }); });

describe('the media pool', () => {
  it('lists the media of the project with what each holds', async () => {
    const engine = engineWith(projectOf(
      mediaOf('m1', { path: 'C:\\media\\harbour.mp4', video: { width: 3840, height: 2160, rate: { num: 30000, den: 1001 }, bits: 10 }, seconds: 12 }),
      mediaOf('m2', { path: '/home/me/phone.mov', video: { width: 1920, height: 1080, rate: { num: 60000, den: 1001 }, rotation: 90 }, seconds: 3 }),
      mediaOf('m3', { path: 'C:\\media\\theme.wav', video: null, seconds: 63 }),
    ));
    await studio(engine);
    expect(engine.bridge.open).toHaveBeenCalledWith('F:\\film');
    expect(names()).toEqual(['harbour.mp4', 'phone.mov', 'theme.wav']);
    expect(marked('pool-facts').map(facts => facts.textContent)).toEqual([
      '3840×2160 · 29.97 fps · 00:00:12:00 · 10-bit',
      // Held upright: it is turned a quarter, and 59.94 is counted 60 frames to the second.
      '1080×1920 · 59.94 fps · 00:00:03:00 · 8-bit',
      // Sound alone is counted at 25 frames a second where the project has no timeline.
      'Audio only · 00:01:03:00',
    ]);
  });

  it('follows the project as the engine tells of it, whoever changed it', async () => {
    const engine = engineWith(projectOf(mediaOf('m1')));
    await studio(engine);
    expect(names()).toEqual(['m1.mp4']);
    // The agent imports a file.
    engine.notify('project.changed', { revision: 1, by: 'agent', made_by: 'agent', next_id: 11, done: [{ op: 'add_media', media: mediaOf('m10'), index: 1 }] });
    expect(names()).toEqual(['m1.mp4', 'm10.mp4']);
    engine.notify('project.changed', { revision: 2, by: 'agent', made_by: 'agent', next_id: 11, done: [{ op: 'remove_media', id: 'm1' }] });
    expect(names()).toEqual(['m10.mp4']);
  });

  it('says that there is no media yet', async () => {
    await studio(engineWith(projectOf()));
    expect(one('pool-none').textContent).toBe('There is no media yet. Import some to begin.');
  });

  it('says how the proxy of each media stands, and makes a failed one again', async () => {
    const proxy = { path: 'C:\\film\\.columbina\\studio\\cache\\proxies\\m4-000000000000.mov', width: 1920, height: 1080, codec: 'hevc', digest_of_source: '0'.repeat(64) };
    const engine = engineWith(projectOf(mediaOf('m1'), mediaOf('m2'), mediaOf('m3'), { ...mediaOf('m4'), proxy }, mediaOf('m5')));
    await studio(engine);
    const tell = (told: ProxyEvent) => engine.notify('media.proxy', told);
    tell({ media: 'm1', state: 'queued' });
    tell({ media: 'm2', state: 'running', progress: 0.426 });
    tell({ media: 'm3', state: 'failed', message: 'the encoder failed' });
    const said = (id: string) => item(id).querySelector('[data-test="pool-proxy"]')?.textContent ?? null;
    expect(said('m1')).toBe('Proxy queued');
    expect(said('m2')).toBe('Making proxy 43%');
    expect(said('m3')).toBe('Proxy failedMake again');
    // A proxy that the project has is ready, and media that needs none says nothing.
    expect(said('m4')).toBe('Proxy ready');
    expect(said('m5')).toBeNull();
    tell({ media: 'm2', state: 'done' });
    expect(said('m2')).toBe('Proxy ready');
    fireEvent.click(item('m3').querySelector('[data-test="make-again"]')!);
    await settle();
    expect(engine.asked()).toEqual([['media.make_proxy', { media: 'm3' }]]);
    tell({ media: 'm3', state: 'queued' });
    expect(said('m3')).toBe('Proxy queued');
  });

  it('imports media through the dialog of the system, and says why it could not', async () => {
    const engine = engineWith(projectOf());
    await studio(engine);
    await act(async () => { fireEvent.click(one('import')); });
    expect(engine.bridge.importFiles).toHaveBeenCalledWith({ language: 'en' });
    expect(marked('studio-problem')).toEqual([]);
    engine.bridge.importFiles.mockResolvedValueOnce({ ok: false, error: { code: -32002, message: 'C:\\a.txt is not media', data: null } } as never);
    await act(async () => { fireEvent.click(one('import')); });
    expect(one('studio-problem').textContent).toBe('The media could not be imported. C:\\a.txt is not media.');
  });

  it('shows media in the viewer at a double click, and at Enter', async () => {
    const engine = engineWith(projectOf(mediaOf('m1'), mediaOf('m2')));
    await studio(engine);
    const button = (id: string) => item(id).querySelector('button')!;
    fireEvent.click(button('m2'));
    expect(engine.asked()).toEqual([]);
    expect(button('m2').getAttribute('aria-pressed')).toBe('true');
    fireEvent.doubleClick(button('m2'));
    fireEvent.keyDown(button('m1'), { key: 'Enter' });
    await settle();
    expect(engine.asked()).toEqual([['transport.load', { media: 'm2' }], ['transport.load', { media: 'm1' }]]);
    // The viewer follows what the transport tells.
    engine.notify('transport.state', stateOf({ media: 'm1' }));
    expect(one('loaded').textContent).toBe('m1.mp4');
    expect(button('m1').getAttribute('aria-current')).toBe('true');
  });

  it('says why media could not be loaded', async () => {
    const engine = engineWith(projectOf(mediaOf('m1')));
    engine.answers.set('transport.load', () => ({ ok: false, error: { code: -32002, message: 'the first frame cannot be decoded', data: { reason: 'decode' } } }));
    await studio(engine);
    fireEvent.doubleClick(item('m1').querySelector('button')!);
    await settle();
    expect(one('studio-problem').textContent).toBe('This media could not be opened. The first frame cannot be decoded.');
  });

  it('is on the Media page only', async () => {
    await studio(engineWith(projectOf(mediaOf('m1'))));
    expect(one('page-media').getAttribute('aria-selected')).toBe('true');
    fireEvent.click(one('page-edit'));
    expect(marked('pool')).toEqual([]);
    // The monitor is the same on both.
    expect(marked('viewer')).toHaveLength(1);
    expect(marked('transport')).toHaveLength(1);
    expect(marked('studio-pages')).toEqual([]);
    expect([...document.querySelectorAll('[role="tab"]')].map(tab => tab.textContent)).toEqual(['Media', 'Edit']);
  });
});

describe('the keys of the studio', () => {
  /** A studio with media loaded, and the transport as a test says. */
  async function loaded(state: Partial<TransportState> = {}) {
    const engine = engineWith(projectOf(mediaOf('m1')), { state: stateOf(state) });
    const drawn = await studio(engine);
    return { engine, ...drawn };
  }
  const played = (engine: ReturnType<typeof engineWith>) => engine.asked().map(([method, params]) =>
    (method === 'transport.play' ? (params as { speed: number }).speed : method === 'transport.pause' ? 'K' : JSON.stringify(params)));

  it('play faster at each L, backward faster at each J, and stop at K', async () => {
    const { engine } = await loaded();
    for (const key of ['l', 'l', 'l', 'l', 'l', 'k', 'j', 'j', 'j', 'L', 'k']) tap(key);
    await settle();
    expect(played(engine)).toEqual([1, 2, 4, 8, 8, 'K', -1, -2, -4, 1, 'K']);
  });

  it('go on from what the transport tells: L while it plays at 2× asks for 4×', async () => {
    const { engine } = await loaded({ playing: true, speed: 2 });
    press('l');
    engine.notify('transport.state', stateOf({ playing: true, speed: -4 }));
    press('j');
    engine.notify('transport.state', stateOf({ playing: false, speed: -8 }));
    press('j');
    await settle();
    expect(played(engine)).toEqual([4, -8, -1]);
  });

  it('play and stop at Space, and are taken once while a key is held', async () => {
    const { engine } = await loaded();
    press(' ');
    press(' ', { repeat: true });
    engine.notify('transport.state', stateOf({ playing: true }));
    press(' ');
    await settle();
    expect(played(engine)).toEqual([1, 'K']);
  });

  it('step a frame with K held and J or L', async () => {
    const { engine } = await loaded();
    press('k');
    press('l');
    press('j');
    act(() => { fireEvent.keyUp(document.body, { key: 'k' }); });
    press('l');
    await settle();
    expect(engine.asked()).toEqual([
      ['transport.pause'], ['transport.step', { frames: 1 }], ['transport.step', { frames: -1 }], ['transport.play', { speed: 1 }],
    ]);
  });

  it('step a frame with the arrows, a second with Shift, and go to either end with Home and End', async () => {
    const { engine } = await loaded({ last: 249 });
    press('ArrowLeft');
    press('ArrowRight');
    press('ArrowRight', { repeat: true });
    press('ArrowRight', { shiftKey: true });
    press('ArrowLeft', { shiftKey: true });
    press('Home');
    press('End');
    await settle();
    expect(engine.asked()).toEqual([
      ['transport.step', { frames: -1 }], ['transport.step', { frames: 1 }], ['transport.step', { frames: 1 }],
      ['transport.step', { frames: 25 }], ['transport.step', { frames: -25 }],
      ['transport.seek', { frame: 0, scrubbing: false }], ['transport.seek', { frame: 249, scrubbing: false }],
    ]);
  });

  it('count a second in the frames of the media: 30 at 29.97', () => {
    const context = { playing: false, speed: 1, fps: nominalFps({ num: 30000, den: 1001 }), last: null };
    expect(commandOf({ key: 'ArrowRight', shiftKey: true }, context)).toEqual({ method: 'transport.step', params: { frames: 30 } });
    expect(commandOf({ key: 'End' }, context)).toBeNull();
    expect(faster({ playing: true, speed: -8 }, false)).toBe(-8);
    expect(faster({ playing: true, speed: 4 }, false)).toBe(-1);
  });

  it('are not taken while the keyboard writes into a field, with Ctrl or Alt, or while something lies over the page', async () => {
    const { engine } = await loaded();
    const field = document.createElement('input');
    const box = document.createElement('textarea');
    const editable = document.createElement('div');
    editable.setAttribute('contenteditable', 'true');
    document.body.append(field, box, editable);
    for (const target of [field, box, editable]) {
      press('l', {}, target);
      press(' ', {}, target);
      press('ArrowLeft', {}, target);
    }
    press('l', { ctrlKey: true });
    press('j', { altKey: true });
    press('k', { metaKey: true });
    // A sheet lies over the page.
    const { unmount } = render(<Sheet title="Settings" onClose={() => {}}><p>Words</p></Sheet>);
    press('l');
    press('ArrowRight');
    unmount();
    await settle();
    expect(engine.asked()).toEqual([]);
    press('l');
    await settle();
    expect(engine.asked()).toEqual([['transport.play', { speed: 1 }]]);
    field.remove();
    box.remove();
    editable.remove();
  });

  it('are not taken where nothing is loaded, or once the studio is left', async () => {
    const engine = engineWith(projectOf(mediaOf('m1')));
    const { unmount } = await studio(engine);
    press('l');
    press('ArrowRight');
    engine.notify('transport.state', stateOf());
    unmount();
    press('l');
    await settle();
    expect(engine.asked()).toEqual([]);
  });
});

describe('the transport', () => {
  it('shows the timecode of the playhead, and follows what the transport tells', async () => {
    const engine = engineWith(projectOf(mediaOf('m1', { video: { rate: { num: 30000, den: 1001 } } })), { state: stateOf({ frame: 0, last: 299 }) });
    await studio(engine);
    expect(one('timecode').textContent).toBe('00:00:00:00');
    engine.notify('transport.state', stateOf({ frame: 1829, shown: 1800, exact: false, last: 299 }));
    expect(one('timecode').textContent).toBe('00:01:00:29');
    expect(one('timecode').dataset.exact).toBe('no');
    expect(one('timecode').title).toBe('A frame near the playhead is shown until the frame itself is ready.');
    engine.notify('transport.state', stateOf({ frame: 1829, shown: 1829, exact: true, playing: true, speed: -4, sound: 'unavailable', source: 'proxy' }));
    expect(one('timecode').dataset.exact).toBeUndefined();
    expect(one('speed').textContent).toBe('−4×');
    expect(one('play').getAttribute('aria-label')).toBe('Pause');
    expect(one('source').textContent).toBe('Proxy');
    expect(one('sound').getAttribute('aria-label')).toBe('Sound unavailable');
    expect(one('sound').title).toBe('No sound device can be used, so the picture plays alone.');
  });

  it('sends what its buttons ask for', async () => {
    const engine = engineWith(projectOf(mediaOf('m1')), { state: stateOf({ proxies: true }) });
    await studio(engine);
    fireEvent.click(one('step-back'));
    fireEvent.click(one('play'));
    fireEvent.click(one('step-forward'));
    fireEvent.click(one('use-proxies'));
    await settle();
    expect(engine.asked()).toEqual([
      ['transport.step', { frames: -1 }], ['transport.play', { speed: 1 }], ['transport.step', { frames: 1 }], ['transport.proxies', { on: false }],
    ]);
  });

  it('can do nothing where nothing is loaded', async () => {
    await studio(engineWith(projectOf(mediaOf('m1'))));
    expect((one('play') as HTMLButtonElement).disabled).toBe(true);
    expect(one('scrubber').getAttribute('aria-disabled')).toBe('true');
    expect(one('loaded').textContent).toBe('Nothing loaded');
    expect(one('load-hint').textContent).toBe('Double-click media on the Media page to see it here.');
  });

  it('sends a seek that scrubs at most every 1/30 s while the playhead is dragged, and one that does not as it is let go', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const engine = engineWith(projectOf(mediaOf('m1')), { state: stateOf({ last: 1000 }) });
    const copy = new ProjectCopy(engine.bridge);
    render(<Studio cwd={'F:\\film'} engine={{ bridge: engine.bridge, copy, native: true }} language="en" onClose={() => {}} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(10); });
    const bar = one('scrubber');
    bar.getBoundingClientRect = () => ({ left: 100, right: 1100, top: 0, bottom: 18, width: 1000, height: 18, x: 100, y: 0, toJSON() {} }) as DOMRect;
    const seeks = () => engine.asked().filter(([method]) => method === 'transport.seek').map(([, params]) => params);
    act(() => { fireEvent.pointerDown(bar, { clientX: 200, button: 0, pointerId: 1 }); });
    expect(seeks()).toEqual([{ frame: 100, scrubbing: true }]);
    // Three moves within the next thirtieth of a second: only the last is sent, once it is over.
    for (const x of [210, 220, 250]) {
      act(() => { vi.advanceTimersByTime(5); fireEvent.pointerMove(bar, { clientX: x, pointerId: 1 }); });
    }
    expect(seeks()).toHaveLength(1);
    expect(one('timecode').textContent).toBe('00:00:06:00');
    act(() => { vi.advanceTimersByTime(Math.ceil(SCRUB_EVERY_MS)); });
    expect(seeks()).toEqual([{ frame: 100, scrubbing: true }, { frame: 150, scrubbing: true }]);
    // A move long after is sent at once.
    act(() => { vi.advanceTimersByTime(100); fireEvent.pointerMove(bar, { clientX: 400, pointerId: 1 }); });
    act(() => { vi.advanceTimersByTime(10); fireEvent.pointerMove(bar, { clientX: 1300, pointerId: 1 }); });
    expect(seeks()).toHaveLength(3);
    // Let go before the one that waits is sent: it is not sent, and the frame itself is.
    act(() => { fireEvent.pointerUp(bar, { clientX: 1300, pointerId: 1 }); });
    act(() => { vi.advanceTimersByTime(100); });
    expect(seeks()).toEqual([
      { frame: 100, scrubbing: true }, { frame: 150, scrubbing: true }, { frame: 300, scrubbing: true }, { frame: 1000, scrubbing: false },
    ]);
    // A move that is no drag sends nothing.
    act(() => { fireEvent.pointerMove(bar, { clientX: 300, pointerId: 1 }); });
    expect(seeks()).toHaveLength(4);
  });
});

describe('the viewer', () => {
  it('says why there is no picture, or what it waits for', async () => {
    let failing: Error | null = new Error("Error invoking remote method 'columbina:engine-open': Error: There is no engine program. Build engine/ of the repository, or name the program with COLUMBINA_ENGINE by its whole path.");
    const engine = engineWith(projectOf(), { open: async () => { if (failing) throw failing; } });
    await studio(engine);
    expect(one('viewer-words').textContent).toBe('The engine of the studio is not installed.');
    expect(one('viewer-detail').textContent).toBe('There is no engine program. Build engine/ of the repository, or name the program with COLUMBINA_ENGINE by its whole path.');
    failing = new Error('The application is closing.');
    await act(async () => { fireEvent.click(one('retry')); });
    await settle();
    expect(one('viewer-words').textContent).toBe('The studio could not be opened.');
    failing = null;
    await act(async () => { fireEvent.click(one('retry')); });
    await settle();
    expect(engine.bridge.open).toHaveBeenCalledTimes(3);
    expect(marked('viewer-notice')).toEqual([]);

    engine.tell({ kind: 'taking-over', session: engine.session, pid: 77 });
    expect(one('viewer-words').textContent).toBe('The project is being taken over from the agent.');
    engine.tell({ kind: 'opened', session: engine.session, project: 'F:\\film\\.columbina\\studio', owned: true, pid: 600, revision: 0 });
    expect(marked('viewer-notice')).toEqual([]);
    engine.tell({ kind: 'unsupported', session: engine.session, reason: 'unsupported', message: 'views are not made on this system' });
    expect(one('viewer-words').textContent).toBe('The preview cannot be shown on this system yet.');
    engine.tell({ kind: 'unsupported', session: engine.session, reason: 'headless', message: 'no such method' });
    expect(one('viewer-words').textContent).toBe('The project is open in an engine without a window, so the preview cannot be shown.');
    engine.tell({ kind: 'exited', session: engine.session, expected: false, code: 1, signal: null });
    expect(one('viewer-words').textContent).toBe('The engine of the studio has stopped.');
    await act(async () => { fireEvent.click(one('retry')); });
    await settle();
    expect(engine.bridge.open).toHaveBeenCalledTimes(4);
    expect(marked('viewer-notice')).toEqual([]);
  });

  it('says while the project is opened', async () => {
    let open!: () => void;
    const engine = engineWith(projectOf(), { open: () => new Promise<void>((resolve) => { open = resolve; }) });
    await studio(engine);
    expect(one('viewer-words').textContent).toBe('The project is being opened in the studio.');
    await act(async () => { open(); });
    await settle();
    expect(marked('viewer-notice')).toEqual([]);
  });

  it('is a grey box in a browser, which says where the preview appears, in every language', async () => {
    const engine = engineWith(projectOf());
    await studio(engine, false);
    expect(one('viewer').dataset.browser).toBe('');
    expect(one('viewer-words').textContent).toBe('The preview appears in the desktop app.');
    // Nothing is told to a shell that is not there.
    expect(engine.bridge.viewRect).not.toHaveBeenCalled();
    expect(engine.bridge.viewShow).not.toHaveBeenCalled();
    cleanup();
    await studio(engineWith(projectOf()), false, zhCN);
    expect(one('viewer-words').textContent).toBe('预览窗只在桌面应用里显示。');
    cleanup();
    await studio(engineWith(projectOf()), false, ja);
    expect(one('viewer-words').textContent).toBe('プレビューはデスクトップアプリで表示されます。');
  });

  it('works with the engine of the replay', async () => {
    const bridge = createMockEngine();
    render(<Studio cwd={'F:\\work'} engine={{ bridge, copy: new ProjectCopy(bridge), native: false }} language="en" onClose={() => {}} />);
    await settle();
    expect(names()).toEqual(['harbour-4k.mp4', 'interview.mov', 'IMG_0412.MOV', 'theme.wav']);
    expect(one('loaded').textContent).toBe('harbour-4k.mp4');
    expect(one('timecode').textContent).toBe('00:00:01:07');
    press('l');
    press('l');
    await settle();
    expect(one('speed').textContent).toBe('2×');
    fireEvent.doubleClick(item('m4').querySelector('button')!);
    await settle();
    expect(one('loaded').textContent).toBe('theme.wav');
    expect(one('sound').dataset.sound).toBe('off');
  });
});

describe('where the viewer lies', () => {
  /** A viewer that lies where `box` says, told to a shell that the test holds. */
  function Viewer({ bridge, active, box }: { bridge: Pick<EngineBridge, 'viewRect' | 'viewShow' | 'onEvent'>; active: boolean; box: () => DOMRect }) {
    const element = useRef<HTMLDivElement>(null);
    useViewerRect(element, bridge, active);
    return <div ref={(node) => { element.current = node; if (node) node.getBoundingClientRect = box; }} />;
  }
  const at = (left: number, top: number, right: number, bottom: number) =>
    ({ left, top, right, bottom, width: right - left, height: bottom - top, x: left, y: top, toJSON() {} }) as DOMRect;
  function shell() {
    const listeners = new Set<(event: EngineEvent) => void>();
    return {
      viewRect: vi.fn(async () => {}),
      viewShow: vi.fn(async () => {}),
      onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
      tell: (event: EngineEvent) => act(() => { for (const listener of [...listeners]) listener(event); }),
    };
  }
  /** Something that lies over the page while it is open. */
  function Lying({ open }: { open: boolean }) {
    useOverlay(open);
    return null;
  }

  it('is told in pixels of the screen, at most once a frame and only when it changed', async () => {
    vi.stubGlobal('devicePixelRatio', 1.5);
    const bridge = shell();
    let box = at(264.4, 44.2, 900.6, 600.3);
    render(<Viewer bridge={bridge} active box={() => box} />);
    expect(bridge.viewRect).not.toHaveBeenCalled();
    drawFrame();
    // Left and top rounded, then right and bottom, and the size between them.
    expect(bridge.viewRect.mock.calls).toEqual([[{ x: 397, y: 66, width: 954, height: 834 }]]);
    // Asked many times in a frame: measured once. Nothing changed: nothing is told.
    for (let times = 0; times < 5; times += 1) act(() => { window.dispatchEvent(new Event('resize')); });
    expect(frames.size).toBe(1);
    drawFrame();
    expect(bridge.viewRect).toHaveBeenCalledTimes(1);
    box = at(0, 44, 640, 400);
    act(() => { window.dispatchEvent(new Event('resize')); window.dispatchEvent(new Event('resize')); });
    drawFrame();
    expect(bridge.viewRect.mock.calls.at(-1)).toEqual([{ x: 0, y: 66, width: 960, height: 534 }]);
    expect(bridge.viewRect).toHaveBeenCalledTimes(2);
  });

  it('is told again when the shell asks, and when the ratio of pixels changes', async () => {
    vi.stubGlobal('devicePixelRatio', 1);
    const queries: { media: string; listener: (() => void) | null }[] = [];
    vi.stubGlobal('matchMedia', (media: string) => {
      const query = { media, listener: null as (() => void) | null };
      queries.push(query);
      return { matches: true, media, addEventListener: (_: string, listener: () => void) => { query.listener = listener; }, removeEventListener: () => { query.listener = null; } };
    });
    const bridge = shell();
    render(<Viewer bridge={bridge} active box={() => at(10, 10, 110, 60)} />);
    drawFrame();
    expect(bridge.viewRect).toHaveBeenCalledTimes(1);
    // The page was zoomed: the shell says so. The same rectangle is told again.
    bridge.tell({ kind: 'measure', session: 3 });
    drawFrame();
    expect(bridge.viewRect.mock.calls).toEqual([[{ x: 10, y: 10, width: 100, height: 50 }], [{ x: 10, y: 10, width: 100, height: 50 }]]);
    // The ratio changes, as the page hears it.
    expect(queries.at(-1)!.media).toBe('(resolution: 1dppx)');
    vi.stubGlobal('devicePixelRatio', 1.25);
    act(() => { queries.at(-1)!.listener!(); });
    drawFrame();
    expect(bridge.viewRect.mock.calls.at(-1)).toEqual([{ x: 13, y: 13, width: 125, height: 62 }]);
    expect(queries.at(-1)!.media).toBe('(resolution: 1.25dppx)');
  });

  it('is shown once it is told, hidden while something lies over the page, and hidden when it is no longer shown', async () => {
    const bridge = shell();
    function Page() {
      const [lying, setLying] = useState(false);
      const [active, setActive] = useState(true);
      return <>
        <Viewer bridge={bridge} active={active} box={() => at(0, 0, 100, 100)} />
        <Lying open={lying} />
        <button type="button" data-test="lie" onClick={() => setLying(open => !open)}>lie</button>
        <button type="button" data-test="leave" onClick={() => setActive(shown => !shown)}>leave</button>
      </>;
    }
    render(<Page />);
    // Hidden until the shell knows where it lies.
    expect(bridge.viewShow.mock.calls).toEqual([[false]]);
    drawFrame();
    expect(bridge.viewShow.mock.calls).toEqual([[false], [true]]);
    expect(bridge.viewRect.mock.invocationCallOrder[0]).toBeLessThan(bridge.viewShow.mock.invocationCallOrder[1]);
    fireEvent.click(one('lie'));
    expect(document.body.dataset.overlay).toBe('1');
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([false]);
    fireEvent.click(one('lie'));
    expect(document.body.dataset.overlay).toBeUndefined();
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
    // The page is hidden, and shown again.
    const hidden = vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden');
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([false]);
    hidden.mockReturnValue('visible');
    act(() => { document.dispatchEvent(new Event('visibilitychange')); });
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
    const shows = bridge.viewShow.mock.calls.length;
    // The studio is left.
    fireEvent.click(one('leave'));
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([false]);
    expect(bridge.viewShow.mock.calls.length).toBe(shows + 1);
    // Something that opens while it is not shown asks nothing of the shell.
    fireEvent.click(one('lie'));
    expect(bridge.viewShow.mock.calls.length).toBe(shows + 1);
    fireEvent.click(one('lie'));
    // Shown again: it is told where it lies before it is shown.
    fireEvent.click(one('leave'));
    drawFrame();
    expect(bridge.viewShow.mock.calls.slice(shows + 1)).toEqual([[false], [true]]);
  });

  it('is hidden while a sheet, a picker or the list of commands is open', async () => {
    const bridge = shell();
    render(<Viewer bridge={bridge} active box={() => at(0, 0, 100, 100)} />);
    drawFrame();
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
    const { unmount } = render(<Sheet title="A question" onClose={() => {}}><p>Words</p></Sheet>);
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([false]);
    unmount();
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
    render(<EffortPicker effort="high" efforts={['low', 'high']} choice="free" placement="up" onChoose={() => {}} />);
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
    fireEvent.click(one('effort-button'));
    expect(marked('effort-panel')).toHaveLength(1);
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([false]);
    fireEvent.click(one('effort-button'));
    expect(marked('effort-panel')).toEqual([]);
    expect(bridge.viewShow.mock.calls.at(-1)).toEqual([true]);
  });
});

describe('the timecode', () => {
  it('counts the frames of a second at 25, 29.97, 60 and 23.976', () => {
    expect(timecode(0, { num: 25, den: 1 })).toBe('00:00:00:00');
    expect(timecode(24, { num: 25, den: 1 })).toBe('00:00:00:24');
    expect(timecode(25, { num: 25, den: 1 })).toBe('00:00:01:00');
    expect(timecode(25 * 3661 + 7, { num: 25, den: 1 })).toBe('01:01:01:07');
    // 29.97 is counted without dropping frames: 30 to a second, from 0 to 29.
    expect(timecode(29, { num: 30000, den: 1001 })).toBe('00:00:00:29');
    expect(timecode(30, { num: 30000, den: 1001 })).toBe('00:00:01:00');
    expect(timecode(30 * 600 + 15, { num: 30000, den: 1001 })).toBe('00:10:00:15');
    expect(timecode(59, { num: 60, den: 1 })).toBe('00:00:00:59');
    expect(timecode(60 * 59 + 1, { num: 60, den: 1 })).toBe('00:00:59:01');
    expect(timecode(23, { num: 24000, den: 1001 })).toBe('00:00:00:23');
    expect(timecode(24 * 3600 * 10, { num: 24000, den: 1001 })).toBe('10:00:00:00');
    expect(timecode(-3, { num: 25, den: 1 })).toBe('00:00:00:00');
  });

  it('writes a rate as an editor does', () => {
    expect([[25, 1], [30000, 1001], [60000, 1001], [24000, 1001], [60, 1], [50, 1]].map(([num, den]) => rateText({ num, den })))
      .toEqual(['25', '29.97', '59.94', '23.976', '60', '50']);
    expect(nominalFps({ num: 30000, den: 1001 })).toBe(30);
    expect(nominalFps({ num: 0, den: 1 })).toBe(25);
  });
});
