// @vitest-environment jsdom
/**
 * The Edit page: the pool, the source and timeline monitors side by side above the timeline, each
 * monitor with its own transport and view; which monitor the keys go to; one monitor where the page
 * is narrow; the views hidden while a dialog lies over the page; choosing and making timelines; and
 * the keys of editing, which take what the source monitor has to the playhead of the timeline.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import { Studio } from '../src/studio/Studio';
import { TWO_MONITORS_PX } from '../src/studio/pages/EditPage';
import { freshName, timelineDefaults } from '../src/studio/pages/NewTimeline';
import { applyOp, ProjectCopy, type EngineEvent, type Media, type Op, type Project, type TransportState } from '../src/studio/engine';
import type { EngineBridge } from '../src/bridge';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

let width: number;
let frames: FrameRequestCallback[];
let observers: ResizeObserverCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
beforeEach(() => {
  likeABrowser(); width = 1500; frames = []; observers = [];
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.stubGlobal('ResizeObserver', class { constructor(callback: ResizeObserverCallback) { observers.push(callback); } observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  // The page is as wide as the test says; the source monitor lies at the left, the timeline monitor at the right.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    const w = this.classList.contains('edit-page') ? width : 450;
    const left = this.closest('[data-monitor="timeline"]') ? 800 : 330;
    return { left, top: 100, width: w, height: 300, right: left + w, bottom: 400, x: left, y: 100, toJSON() {} };
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; });

const second: Media = { ...media, id: 'm2', name: 'Street', facts: { ...media.facts, video: { ...media.facts.video!, width: 1080, height: 1920, rate: { num: 30000, den: 1001 } } } };

/** An engine that keeps a project and the two transports, and tells what changed as the real one does. */
function fakeEngine(empty = false) {
  let project: Project = { schema: 2, next_id: 10, bins: [], media: [media, second], timelines: empty ? [] : [timeline(), { ...timeline([clip('c3')]), id: 't2', name: 'Alternate' }] };
  let revision = 0;
  let refuse: string | null = null;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  const states: Record<'source' | 'timeline', TransportState> = {
    source: { monitor: 'source', media: 'm1', frame: 10, shown: 10, last: 249, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: 'original' },
    timeline: { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 },
  };
  const bridge = {
    open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>) => {
      if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
      const monitor = params?.monitor as 'source' | 'timeline';
      if (method === 'transport.state') return { ok: true, result: states[monitor] };
      if (method === 'transport.load') {
        const state = (monitor === 'timeline'
          ? { ...states.timeline, timeline: params!.timeline as string, frame: 0, shown: 0, last: params!.timeline === 't2' ? 99 : 199 }
          : { ...states.source, media: params!.media as string, frame: 0, shown: 0 }) as TransportState;
        states[monitor] = state; tell('transport.state', state); return { ok: true, result: state };
      }
      if (method === 'transaction.apply') {
        if (refuse) return { ok: false, error: { code: -32001, message: refuse, data: { reason: 'out_of_range' } } };
        const names: Record<string, string> = {};
        // Editing commands are expanded by the engine alone; here they change nothing but the revision.
        const done = (params!.ops as Op[]).filter(op => 'op' in op).map((op) => {
          if (op.op !== 'add_timeline') return op;
          names[op.timeline.id] = 't10';
          return { ...op, timeline: { ...op.timeline, id: 't10', tracks: op.timeline.tracks.map(track => ({ ...track, id: track.kind === 'video' ? 'V1' : 'A1' })) } };
        });
        for (const op of done) project = applyOp(project, op);
        revision++; project = { ...project, next_id: 20 };
        tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: 20, done });
        return { ok: true, result: { done, names, revision, next_id: 20, clamped: [] } };
      }
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn>; viewShow: ReturnType<typeof vi.fn>; viewRect: ReturnType<typeof vi.fn> };
  const copy = new ProjectCopy(bridge);
  return {
    bridge, copy,
    refuse: (message: string | null) => { refuse = message; },
    notify: (monitor: 'source' | 'timeline', fields: Partial<TransportState>) => act(() => { states[monitor] = { ...states[monitor], ...fields } as TransportState; tell('transport.state', states[monitor]); }),
  };
}
async function mount(empty = false, words?: typeof zhCN) {
  const engine = fakeEngine(empty);
  const tree = <Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />;
  const view = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle(); fireEvent.click(screen.getByRole('tab', { name: words ? '剪辑' : 'Edit' })); await settle(); draw(); await settle();
  return { ...engine, ...view };
}
const monitor = (name: 'Source viewer' | 'Timeline viewer') => screen.getByRole('region', { name });
const key = (value: string, fields = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
const narrowTo = async (to: number) => { act(() => { width = to; observers.forEach(callback => callback([], {} as ResizeObserver)); }); draw(); await settle(); };

describe('the Edit page', () => {
  it('lays out the pool, two native monitors and the timeline, and loads the first timeline once', async () => {
    const { bridge } = await mount();
    expect(screen.getByRole('region', { name: 'Media pool' })).toBeDefined();
    expect(monitor('Source viewer').dataset.monitor).toBe('source');
    expect(monitor('Timeline viewer').dataset.monitor).toBe('timeline');
    expect(document.querySelector('[data-clip="c1"]')).not.toBeNull();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'timeline', timeline: 't1' }]);
    expect(bridge.viewShow).toHaveBeenCalledWith(true, 'source'); expect(bridge.viewShow).toHaveBeenCalledWith(true, 'timeline');
    expect(bridge.viewRect.mock.calls.some(([box, name]) => name === 'source' && box.x === 330)).toBe(true);
    expect(bridge.viewRect.mock.calls.some(([box, name]) => name === 'timeline' && box.x === 800)).toBe(true);
    // Each monitor has its name, what it has loaded, and a transport of its own.
    expect(within(monitor('Source viewer')).getByText('Interview')).toBeDefined();
    expect(within(monitor('Timeline viewer')).getByText('Main')).toBeDefined();
    expect(document.querySelectorAll('[data-test="transport"]')).toHaveLength(2);
    // The timeline monitor has the keys until another is pressed.
    expect(monitor('Timeline viewer').dataset.focused).toBe('true');
    expect(monitor('Source viewer').dataset.focused).toBe('false');
  });

  it('has its pages as DaVinci Resolve orders them, the arrows going round all three', async () => {
    await mount();
    const tabs = () => screen.getAllByRole('tab').map(tab => [tab.textContent, tab.tabIndex]);
    expect(tabs()).toEqual([['Media', -1], ['Cut', -1], ['Edit', 0]]);
    const edit = screen.getByRole('tab', { name: 'Edit' });
    edit.focus();
    fireEvent.keyDown(edit, { key: 'ArrowRight' });
    expect(tabs()).toEqual([['Media', 0], ['Cut', -1], ['Edit', -1]]);
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Cut' }));
    expect(document.querySelector('[data-test="cut-page"]')).not.toBeNull();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' });
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' });
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Edit' }));
  });

  it('puts away and brings back the pool and the inspector', async () => {
    await mount();
    expect(screen.queryByRole('complementary', { name: 'Inspector' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Media pool' }));
    expect(screen.queryByRole('region', { name: 'Media pool' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Media pool' }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(screen.getByRole('button', { name: 'Media pool' }));
    expect(screen.getByRole('region', { name: 'Media pool' })).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: 'Inspector' }));
    const inspector = screen.getByRole('complementary', { name: 'Inspector' });
    expect(inspector.textContent).toContain('Select a clip on the timeline.');
    // The clips chosen on the timeline are handed to it.
    fireEvent.pointerDown(document.querySelector('[data-clip="c1"]')!, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    fireEvent.pointerUp(document.querySelector('[data-clip="c1"]')!, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    await settle();
    expect(inspector.textContent).toContain('1 selected');
  });

  it('routes the buttons and the keys of the transport to their own monitor, and the keys to the one pressed last', async () => {
    const { bridge } = await mount(); bridge.call.mockClear();
    fireEvent.click(within(monitor('Source viewer')).getByRole('button', { name: 'Play' }));
    fireEvent.click(within(monitor('Timeline viewer')).getByRole('button', { name: 'Next frame' }));
    fireEvent.click(within(monitor('Timeline viewer')).getByRole('button', { name: 'Use proxies' }));
    fireEvent.pointerDown(monitor('Source viewer'));
    expect(monitor('Source viewer').dataset.focused).toBe('true');
    key('l');
    fireEvent.pointerDown(monitor('Timeline viewer')); key('l'); key('l'); key('ArrowLeft');
    // The timeline, pressed, gives the keys to the timeline monitor.
    fireEvent.pointerDown(monitor('Source viewer')); fireEvent.pointerDown(screen.getByRole('region', { name: 'Timeline' })); key('k');
    await settle();
    expect(bridge.call.mock.calls).toEqual([
      ['transport.play', { monitor: 'source', speed: 1 }], ['transport.step', { monitor: 'timeline', frames: 1 }], ['transport.proxies', { monitor: 'timeline', on: false }],
      ['transport.play', { monitor: 'source', speed: 2 }], ['transport.play', { monitor: 'timeline', speed: 1 }], ['transport.play', { monitor: 'timeline', speed: 2 }],
      ['transport.step', { monitor: 'timeline', frames: -1 }], ['transport.pause', { monitor: 'timeline' }],
    ]);
  });

  it('keeps the timecode, the scrubber and the original or proxy of each monitor apart', async () => {
    const engine = await mount();
    engine.notify('source', { frame: 25, shown: 25, source: 'proxy' });
    engine.notify('timeline', { frame: 75, shown: 75, source: 'mixed' });
    expect(monitor('Source viewer').querySelector('[data-test="timecode"]')?.textContent).toBe('00:00:01:00');
    expect(monitor('Timeline viewer').querySelector('[data-test="timecode"]')?.textContent).toBe('00:00:03:00');
    expect(monitor('Source viewer').querySelector('[data-test="source"]')?.textContent).toBe('Proxy');
    expect(monitor('Timeline viewer').querySelector('[data-test="source"]')?.textContent).toBe('Mixed');
    expect(monitor('Timeline viewer').querySelector('[data-test="scrubber"]')?.getAttribute('aria-valuemax')).toBe('199');
    // The playhead of the timeline is that of the timeline monitor.
    const playhead = document.querySelector<HTMLElement>('.timeline-playhead')!;
    expect(playhead.style.left).toBe(`${156 + 75 * 4}px`);
  });

  it('switches timelines without touching the source, and does not load again at an edit', async () => {
    const { bridge } = await mount(); bridge.call.mockClear();
    fireEvent.change(screen.getByRole('combobox', { name: 'Timeline' }), { target: { value: 't2' } }); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'timeline', timeline: 't2' }]);
    expect(document.querySelector('[data-timeline="t2"]')).not.toBeNull();
    expect(within(monitor('Timeline viewer')).getByText('Alternate')).toBeDefined();
    bridge.call.mockClear();
    fireEvent.click(screen.getByRole('button', { name: 'Lock: V1' })); await settle();
    expect(calls(bridge, 'transaction.apply')).toHaveLength(1);
    expect(calls(bridge, 'transport.load')).toEqual([]);
    // Back on the Media page and on the Edit page again, the monitor already has it.
    fireEvent.click(screen.getByRole('tab', { name: 'Media' })); await settle();
    fireEvent.click(screen.getByRole('tab', { name: 'Edit' })); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([]);
    expect((screen.getByRole('combobox', { name: 'Timeline' }) as HTMLSelectElement).value).toBe('t2');
  });

  it(`has one monitor narrower than ${TWO_MONITORS_PX} px, switched without either losing its place`, async () => {
    const engine = await mount(); const { bridge } = engine;
    engine.notify('source', { frame: 25, shown: 25 }); engine.notify('timeline', { frame: 75, shown: 75 });
    bridge.call.mockClear(); bridge.viewShow.mockClear();
    await narrowTo(TWO_MONITORS_PX - 1);
    expect(screen.queryByRole('region', { name: 'Source viewer' })).toBeNull();
    expect(monitor('Timeline viewer')).toBeDefined(); expect(bridge.viewShow).toHaveBeenCalledWith(false, 'source');
    const switcher = screen.getByRole('group', { name: 'Viewer shown' });
    expect(within(switcher).getByRole('button', { name: 'Timeline' }).getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(within(switcher).getByRole('button', { name: 'Source' })); draw(); await settle();
    expect(monitor('Source viewer').querySelector('[data-test="timecode"]')?.textContent).toBe('00:00:01:00');
    expect(screen.queryByRole('region', { name: 'Timeline viewer' })).toBeNull();
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'timeline'); expect(bridge.viewShow).toHaveBeenLastCalledWith(true, 'source');
    // The keys go to the monitor shown.
    key(' ');
    fireEvent.click(within(screen.getByRole('group', { name: 'Viewer shown' })).getByRole('button', { name: 'Timeline' })); draw(); await settle();
    expect(monitor('Timeline viewer').querySelector('[data-test="timecode"]')?.textContent).toBe('00:00:03:00');
    expect(calls(bridge, 'transport.load')).toEqual([]);
    expect(calls(bridge, 'transport.seek')).toEqual([]);
    expect(calls(bridge, 'transport.play')).toEqual([{ monitor: 'source', speed: 1 }]);
    // Wide again, both are there, with their names in place of the switch.
    await narrowTo(TWO_MONITORS_PX);
    expect(monitor('Source viewer')).toBeDefined(); expect(monitor('Timeline viewer')).toBeDefined();
    expect(screen.queryByRole('group', { name: 'Viewer shown' })).toBeNull();
  });

  it('opens media double-clicked in the pool in the source monitor, which then has the keys', async () => {
    const { bridge } = await mount(); bridge.call.mockClear();
    fireEvent.doubleClick(document.querySelector('[data-media="m2"]')!); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([{ media: 'm2', monitor: 'source' }]);
    expect(monitor('Source viewer').dataset.focused).toBe('true');
    expect(within(monitor('Source viewer')).getByText('Street')).toBeDefined();
    key('i'); await settle();
    expect(calls(bridge, 'transaction.apply')).toEqual([{ revision: 0, ops: [{ op: 'set_media', id: 'm2', mark_in: { ticks: 0, scale: 25 } }] }]);
  });

  it('hides both native views under a dialog, makes a timeline in one transaction, and selects it', async () => {
    const { bridge } = await mount(); bridge.call.mockClear(); bridge.viewShow.mockClear();
    fireEvent.click(screen.getByRole('button', { name: 'New timeline' })); await settle();
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'source'); expect(bridge.viewShow).toHaveBeenCalledWith(false, 'timeline');
    key('l'); key('F9'); expect(bridge.call).not.toHaveBeenCalled();
    const dialog = screen.getByRole('dialog', { name: 'New timeline' });
    expect((within(dialog).getByRole('textbox', { name: 'Name' }) as HTMLInputElement).value).toBe('Timeline 3');
    expect(within(dialog).getByText('1920 × 1080 · 25 fps · 48 kHz')).toBeDefined();
    fireEvent.change(within(dialog).getByRole('textbox', { name: 'Name' }), { target: { value: 'My film' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Create' })); await settle(); draw(); await settle();
    expect(calls(bridge, 'transaction.apply')).toEqual([{ ops: [{ op: 'add_timeline', timeline: {
      id: '$timeline', name: 'My film', width: 1920, height: 1080, rate: { num: 25, den: 1 }, sample_rate: 48000,
      tracks: [
        { id: '', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [], transitions: [] },
        { id: '', kind: 'audio', name: 'A1', enabled: true, locked: false, clips: [], transitions: [] },
      ] } }] }]);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect((screen.getByRole('combobox', { name: 'Timeline' }) as HTMLSelectElement).value).toBe('t10');
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'timeline', timeline: 't10' }]);
    expect(bridge.viewShow).toHaveBeenLastCalledWith(true, expect.anything());
    expect(bridge.viewShow).toHaveBeenCalledWith(true, 'source'); expect(bridge.viewShow).toHaveBeenCalledWith(true, 'timeline');
  });

  it('keeps the dialog open and says why where the engine refuses the timeline', async () => {
    const engine = await mount();
    engine.refuse('width out of range');
    fireEvent.click(screen.getByRole('button', { name: 'New timeline' })); await settle();
    fireEvent.click(screen.getByRole('button', { name: 'Create' })); await settle();
    const dialog = screen.getByRole('dialog', { name: 'New timeline' });
    expect(within(dialog).getByRole('alert').textContent).toBe('The timeline could not be created.width out of range');
    expect((screen.getByRole('combobox', { name: 'Timeline' }) as HTMLSelectElement).value).toBe('t1');
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('marks the source on its media and the timeline in the page, and places the source at the timeline with F9 and F10', async () => {
    const engine = await mount(); const { bridge } = engine; bridge.call.mockClear();
    fireEvent.pointerDown(monitor('Source viewer')); key('i'); await settle();
    expect(calls(bridge, 'transaction.apply')).toEqual([{ revision: 0, ops: [{ op: 'set_media', id: 'm1', mark_in: { ticks: 10, scale: 25 } }] }]);
    // Marks on the timeline monitor stay in the page.
    bridge.call.mockClear(); fireEvent.pointerDown(monitor('Timeline viewer'));
    engine.notify('timeline', { frame: 20, shown: 20 }); key('i');
    engine.notify('timeline', { frame: 29, shown: 29 }); key('o');
    expect(bridge.call).not.toHaveBeenCalled();
    expect(document.querySelector('.timeline-mark[data-edge="in"]')).not.toBeNull();
    // F9 inserts what the source has between the marks of the timeline, whichever monitor has the keys.
    fireEvent.pointerDown(monitor('Source viewer'));
    key('F9'); await settle();
    expect(calls(bridge, 'transaction.apply').at(-1)).toEqual({ revision: 1, ops: [{ edit: 'insert', timeline: 't1', at: 20, source: { kind: 'media', media: 'm1', length: 10 }, video_track: 'v1', audio_track: null }] });
    // Without marks, F10 overwrites at the playhead of the timeline monitor.
    fireEvent.pointerDown(monitor('Timeline viewer')); key('x', { altKey: true });
    engine.notify('timeline', { frame: 150, shown: 150 });
    key('F10'); await settle();
    expect(calls(bridge, 'transaction.apply').at(-1)).toEqual({ revision: 2, ops: [{ edit: 'overwrite', timeline: 't1', at: 150, source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: null }] });
  });

  it('keeps F11, F12 and Ctrl with plus or minus from the shell, zooming the timeline with them, and leaves Ctrl+0', async () => {
    const { bridge } = await mount(); bridge.call.mockClear();
    const taken = (value: string, fields = {}) => !fireEvent.keyDown(document.body, { key: value, ...fields });
    const scale = () => Number(document.querySelector<HTMLElement>('.timeline')!.dataset.scale);
    const before = scale();
    expect(taken('=', { ctrlKey: true })).toBe(true);
    expect(scale()).toBeCloseTo(before * 1.25);
    expect(taken('+', { ctrlKey: true, shiftKey: true })).toBe(true);
    expect(taken('-', { ctrlKey: true })).toBe(true);
    expect(scale()).toBeCloseTo(before * 1.25);
    // The timeline itself takes them as well where it has the keyboard.
    const timeline = document.querySelector<HTMLElement>('.timeline')!;
    expect(!fireEvent.keyDown(timeline, { key: '-', ctrlKey: true })).toBe(true);
    expect(scale()).toBeCloseTo(before);
    // F11 replaces and F12 places on top, as in DaVinci Resolve: neither goes to full screen or the tools.
    expect(taken('F11')).toBe(true);
    await settle();
    expect(taken('F12')).toBe(true);
    await settle();
    expect(calls(bridge, 'transaction.apply').map(each => (each as { ops: { edit: string }[] }).ops[0].edit)).toEqual(['replace', 'place_on_top']);
    // Ctrl+0 is not the studio's: the page goes back to its own size.
    expect(taken('0', { ctrlKey: true })).toBe(false);
  });

  it('hands the keyboard from the list of timelines to the timeline once one is chosen', async () => {
    const { bridge } = await mount(); bridge.call.mockClear();
    const picker = screen.getByRole('combobox', { name: 'Timeline' });
    picker.focus();
    fireEvent.change(picker, { target: { value: 't2' } }); await settle();
    // The list keeps the keyboard no longer: I marks the timeline, as it would with the timeline pressed.
    expect(document.activeElement).toBe(document.querySelector('.timeline'));
    fireEvent.keyDown(document.activeElement!, { key: 'i' });
    expect(document.querySelector('.timeline-mark[data-edge="in"]')).not.toBeNull();
    expect(calls(bridge, 'transaction.apply')).toEqual([]);
  });

  it('names a new timeline after a number that no timeline has', () => {
    expect(freshName('Timeline', ['Main', 'Alternate'])).toBe('Timeline 3');
    expect(freshName('Timeline', ['Timeline 2', 'Timeline 3'])).toBe('Timeline 4');
    expect(freshName('时间线', ['时间线 1'])).toBe('时间线 2');
    expect(freshName('Timeline', [])).toBe('Timeline 1');
  });

  it('takes no keys of editing from a field, nor of the transport where the focused monitor has nothing', async () => {
    const { bridge } = await mount(true); bridge.call.mockClear();
    key('l'); key('F9');
    fireEvent.keyDown(screen.getByRole('combobox', { name: 'Timeline' }), { key: 'i' });
    await settle();
    expect(bridge.call).not.toHaveBeenCalled();
  });

  it('says what to do before the first timeline, with the timeline transport off', async () => {
    await mount(true);
    expect(within(monitor('Timeline viewer')).getByRole('button', { name: 'Play' }).hasAttribute('disabled')).toBe(true);
    expect(within(monitor('Timeline viewer')).getByText('Choose or create a timeline to begin editing.', { selector: '[data-test="load-hint"]' })).toBeDefined();
    expect((screen.getByRole('combobox', { name: 'Timeline' }) as HTMLSelectElement).disabled).toBe(true);
    expect(screen.getByRole('button', { name: 'New timeline' })).toBeDefined();
    expect(document.querySelector('[data-clip]')).toBeNull();
  });

  it('hides the timeline view as the Media page is shown, and keeps only the source monitor there', async () => {
    const { bridge } = await mount();
    bridge.viewShow.mockClear(); fireEvent.click(screen.getByRole('tab', { name: 'Media' })); draw(); await settle();
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'timeline');
    expect(bridge.viewShow).not.toHaveBeenCalledWith(true, 'timeline');
    expect(document.querySelectorAll('[data-test="viewer"]')).toHaveLength(1);
    expect(document.querySelector('[data-monitor="timeline"]')).toBeNull();
  });

  it('fills the inspector with what is chosen, opens the dialog of speed with R over both views, and puts the effects library beside the pool', async () => {
    const { bridge } = await mount();
    fireEvent.click(screen.getByRole('button', { name: 'Inspector' }));
    const inspector = screen.getByRole('complementary', { name: 'Inspector' });
    // Nothing chosen: the settings of the timeline.
    expect(within(inspector).getByText('Timeline settings')).toBeDefined();
    fireEvent.pointerDown(document.querySelector('[data-clip="c1"]')!, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    fireEvent.pointerUp(document.querySelector('[data-clip="c1"]')!, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    await settle();
    expect(within(inspector).getByLabelText('Zoom', { selector: 'input[type="text"]' })).toBeDefined();
    bridge.call.mockClear(); bridge.viewShow.mockClear();
    key('r'); await settle();
    const dialog = screen.getByRole('dialog', { name: 'Change clip speed' });
    expect(bridge.viewShow).toHaveBeenCalledWith(false, 'source'); expect(bridge.viewShow).toHaveBeenCalledWith(false, 'timeline');
    fireEvent.change(within(dialog).getByLabelText('Speed'), { target: { value: '200' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'Change' }));
    await settle();
    expect(calls(bridge, 'transaction.apply')).toEqual([{ revision: 0, ops: [{ edit: 'set_speed', timeline: 't1', clip: 'c1', speed: { num: 2, den: 1 }, reverse: false, ripple: true, linked: false }] }]);
    expect(screen.queryByRole('dialog', { name: 'Change clip speed' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Effects library' }));
    expect(screen.getByRole('region', { name: 'Effects library' })).toBeDefined();
    expect(screen.getByRole('region', { name: 'Media pool' })).toBeDefined();
  });

  it('names the viewers as DaVinci Resolve does in Chinese', async () => {
    await mount(false, zhCN);
    expect(screen.getByRole('region', { name: '源检视器' })).toBeDefined();
    expect(screen.getByRole('region', { name: '时间线检视器' })).toBeDefined();
    expect(screen.getAllByRole('tab').map(tab => tab.textContent)).toEqual(['媒体', '快编', '剪辑']);
  });
});

describe('the settings of a new timeline', () => {
  const video = (fields: Partial<NonNullable<Media['facts']['video']>>, id = 'v'): Media => ({ ...media, id, facts: { ...media.facts, video: { ...media.facts.video!, ...fields } } });
  it('follow the first video of the pool, as it is shown', () => {
    expect(timelineDefaults([media, second])).toEqual({ width: 1920, height: 1080, rate: { num: 25, den: 1 }, sample_rate: 48000 });
    expect(timelineDefaults([second, media])).toEqual({ width: 1080, height: 1920, rate: { num: 30000, den: 1001 }, sample_rate: 48000 });
    // A phone's file that is turned to be shown upright, and one of an odd size.
    expect(timelineDefaults([video({ rotation: 90 })])).toMatchObject({ width: 1080, height: 1920 });
    expect(timelineDefaults([video({ width: 1279, height: 721 })])).toMatchObject({ width: 1278, height: 720 });
  });
  it('pass over sound alone and stills, and are 1920 × 1080 at 25 fps where there is no video', () => {
    const sound: Media = { ...media, id: 'a', facts: { ...media.facts, video: null } };
    expect(timelineDefaults([sound, video({ still: true, width: 640, height: 480 }), video({ rate: { num: 50, den: 1 } })])).toMatchObject({ rate: { num: 50, den: 1 } });
    expect(timelineDefaults([])).toEqual({ width: 1920, height: 1080, rate: { num: 25, den: 1 }, sample_rate: 48000 });
    expect(timelineDefaults([sound])).toEqual({ width: 1920, height: 1080, rate: { num: 25, den: 1 }, sample_rate: 48000 });
  });
});
