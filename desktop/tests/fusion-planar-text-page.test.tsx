// @vitest-environment jsdom
/**
 * The Planar Tracker and the Text+ on the Fusion page (`src/studio/pages/FusionPage`): each added
 * from the tool bar and from Select Tool; the Inspector of each and the operations that it sends;
 * the tracking of the region of a Planar Tracker by the buttons of its Inspector; and the controls
 * of each on the picture of the viewer, dragged as one gesture: the places of the region, a place
 * put on a side and taken away, the corners of the Corner Pin, and the centre of a Text+.
 *
 * The engine of the tests shows the picture of a composition as the worked example of the protocol
 * has it: 1440 by 1080 in a timeline of 1920 by 1080, at 240, 0. The view shows the frame half as
 * large, so 8 pixels of the screen are 16 of the frame.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { en } from '../src/messages/en';
import { Studio } from '../src/studio/Studio';
import {
  applyOp, emptyGrade, ProjectCopy, type Composition, type EngineEvent, type FusionHomography, type FusionLink, type FusionNode, type FusionPicture,
  type FusionPlanarTracker, type FusionTextPlus, type FusionView, type Op, type Project, type Timeline, type TransportState, type ViewPointer,
} from '../src/studio/engine';
import { keepCopied } from '../src/studio/fusion/copied';
import { newBorder, newFollower, newNode, newOutline, newPlanarTracker, newTextPlus } from '../src/studio/fusion/edits';
import { likeABrowser } from './mounted';
import { clip, media } from './timeline-fixture';

let frames: FrameRequestCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser(); frames = [];
  keepCopied(null);
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // Every part of the page lies at the top left, so a point of the graph is a point of the window at zoom 1.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect(0, 0, 600, 300));
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; delete (window as { columbina?: unknown }).columbina; });

const AT = { timeline: 't1', clip: 'c1' };
const CANVAS = [1920, 1080];
const PICTURE: FusionPicture = { x: 240, y: 0, width: 1440, height: 1080 };
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
const STILL: FusionHomography = [1, 0, 0, 0, 1, 0, 0, 0];
const shifted = (x: number, y: number): FusionHomography => [1, 0, x, 0, 1, y, 0, 0];
/**
 * MediaIn1 through the Transform n3 into the background of the Merge n4, the Background n5 its foreground, and the Merge into
 * MediaOut1; and two nodes that nothing is linked to or from, each where the project puts it: the Planar Tracker n6 at 200,200
 * (as a new one, but for what a test gives it) and the Text+ n7 at 40,300.
 */
const comp = (planar: Partial<FusionPlanarTracker> = {}, text: Partial<FusionTextPlus> = {}): Composition => ({
  nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), newNode('transform', null, 'n3'), newNode('merge', null, 'n4'),
    newNode('background', null, 'n5'),
    { ...newNode('planar_tracker', { x: 200, y: 200 }, 'n6'), params: { ...newPlanarTracker(), ...planar } } as FusionNode,
    { ...newNode('text_plus', { x: 40, y: 300 }, 'n7'), params: { ...newTextPlus(), ...text } } as FusionNode],
  links: [link('MediaIn1', 'n3'), link('n3', 'n4', 'background'), link('n5', 'n4', 'foreground'), link('n4', 'MediaOut1')],
});
/** The middles of the cards of the Planar Tracker, of the Text+ and of the Transform. */
const C = { n6: { x: 246, y: 229 }, n7: { x: 86, y: 329 }, n3: { x: 222, y: 57 } };
/** V1: c1 (frames 0–99, with the composition; its keys count the frames of the timeline) and c2 (100–199, with none). */
const timelineOf = (first: Composition = comp()): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [clip('c1', 0, 100, { fusion: first }), clip('c2', 100, 100)] }],
});
const projectOf = (first: Composition = comp()): Project => ({ schema: 5, next_id: 10, bins: [], groups: [], media: [media], timelines: [timelineOf(first)] });

/**
 * An engine that keeps a project and applies the operations of a transaction as the copy does, giving a node added without an
 * id the next one; that has a Fusion view as the real one has; that answers `track.start` as a test says; and that tells what a
 * test says the pointer does on the picture, and the news of a track.
 */
function fakeEngine(given: Project) {
  let project = given;
  let revision = 0;
  let trackAnswer: unknown = { ok: true, result: { job: 'track-1', first: 0, last: 99, frame_size: [256, 144], direction: 'forward' } };
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
  const named = (op: Op): Op => {
    if (op.op !== 'add_fusion_node' || op.node.id !== '') return op;
    const id = `n${project.next_id}`;
    project = { ...project, next_id: project.next_id + 1 };
    const own = (end: string) => (end === '' ? id : end);
    return { ...op, node: { ...op.node, id } as FusionNode, ...(op.links ? { links: op.links.map(each => ({ ...each, from: own(each.from), to: own(each.to) })) } : {}) };
  };
  const bridge = {
    open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>) => {
      if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
      if (method === 'media.thumbnail') return new Promise(() => {});
      if (method === 'transport.state') return { ok: true, result: params?.monitor === 'timeline' ? state : { ...state, monitor: 'source', media: null } };
      if (method === 'transport.load') {
        state = { ...state, timeline: params!.timeline as string, frame: 0, shown: 0, last: 299 } as TransportState;
        tell('transport.state', state); return { ok: true, result: state };
      }
      if (method === 'transport.seek') {
        state = { ...state, frame: params!.frame as number, shown: params!.frame as number };
        tell('transport.state', state); return { ok: true, result: {} };
      }
      if (method === 'viewer.fusion') {
        const asked = params ?? {};
        const fusion: FusionView | undefined = typeof asked.clip === 'string'
          ? { clip: asked.clip, ...(typeof asked.node === 'string' ? { node: asked.node } : {}), shown: true, canvas: [1920, 1080], size: { width: 1440, height: 1080 }, picture: PICTURE }
          : undefined;
        state = { ...state, fusion } as TransportState;
        tell('transport.state', state); return { ok: true, result: { monitor: 'timeline', fusion: fusion ?? null } };
      }
      if (method === 'transaction.apply') {
        const done = (params!.ops as Op[]).map(named);
        for (const op of done) project = applyOp(project, op);
        revision++;
        tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: project.next_id, done });
        return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
      }
      if (method === 'history.undo' || method === 'history.redo') return { ok: true, result: { revision, done: null } };
      if (method === 'track.start') return trackAnswer;
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn> };
  let event = 0;
  return {
    bridge, copy: new ProjectCopy(bridge),
    project: () => project,
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
    /** What `track.start` answers from now on. */
    answerTrack: (answer: unknown) => { trackAnswer = answer; },
    /** The engine tells the page a notification of its own, as of a track (`track.progress`, `track.state`). */
    news: (method: string, params: unknown) => act(() => { tell(method, params); }),
    /** What the pointer does on the picture, at a place of the frame drawn: the view shows it half as large. */
    pointer: (kind: ViewPointer['kind'], x: number, y: number, extra: Partial<ViewPointer> = {}) => {
      const told: ViewPointer = {
        monitor: 'timeline', instance: 1, event: ++event,
        presentation: { serial: 1, tag: 1, map: { surface: [960, 540], picture: [1920, 1080], dest: [0, 0, 960, 540] } },
        snapshot: { revision, timeline: 't1', frame: 0, canvas: [1920, 1080], ready: true, editable: true, comparison: 'off' },
        kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x, y, inside: true,
        window: { x: Math.round(x / 2), y: Math.round(y / 2) }, shift: false, ctrl: false, alt: false, ...extra,
      };
      act(() => tell('view.pointer', told));
    },
  };
}
const tab = (name: string) => screen.getByRole('tab', { name });
const open = async (name: string) => { fireEvent.click(tab(name)); await settle(); await draw(); await settle(); };
async function mount(project: Project = projectOf()) {
  const engine = fakeEngine(project);
  render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
  await settle();
  await open('Fusion');
  return engine;
}

interface Sent { ops: unknown[]; revision?: number; gesture?: { id: string; end?: boolean } }
type Calls = { call: ReturnType<typeof vi.fn> };
const calls = (bridge: Calls, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
const applied = (bridge: Calls) => calls(bridge, 'transaction.apply') as Sent[];
const sent = (bridge: Calls) => applied(bridge).map(each => each.ops);
const overlays = (bridge: Calls) => calls(bridge, 'view.overlay').map(params => params.overlay);
const marked = (name: string) => document.querySelector<HTMLElement>(`[data-test="${name}"]`);
const graph = () => document.querySelector<SVGElement>('[data-test="fusion-graph"]')!;
const canvas = () => document.querySelector<HTMLElement>('[data-test="node-canvas"]')!;
const nodeOf = (id: string) => graph().querySelector<SVGElement>(`[data-node="${id}"]`)!;
const card = (id: string) => nodeOf(id).querySelector<SVGElement>('.color-node-card')!;
const currentNode = () => graph().querySelector<SVGElement>('[data-current]')?.dataset.node ?? null;
const key = (value: string, fields: Record<string, unknown> = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
/** Choose a node by a press on its card. */
const choose = async (id: 'n3' | 'n6' | 'n7') => {
  fireEvent.pointerDown(card(id), { button: 0, clientX: C[id].x, clientY: C[id].y });
  fireEvent.pointerUp(canvas(), { button: 0, clientX: C[id].x, clientY: C[id].y });
  await settle();
};
const inspector = () => marked('fusion-inspector-body')!;
const fieldsShown = () => [...inspector().querySelectorAll<HTMLElement>('[data-field]')].map(each => each.dataset.field);
const sections = () => [...inspector().querySelectorAll('h3.inspector-section')].map(each => each.textContent);
const row = (name: string) => inspector().querySelector<HTMLElement>(`[data-field="${name}"]`)!;
const box = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="text"]')!;
const select = (name: string) => row(name).querySelector('select')!;
const type = (name: string, value: string) => { fireEvent.change(box(name), { target: { value } }); fireEvent.keyDown(box(name), { key: 'Enter' }); };
const keysOf = (name: string) => [...inspector().querySelectorAll<HTMLButtonElement>(`[data-keys="${name}"] button`)];
const withKeys = () => [...inspector().querySelectorAll<HTMLElement>('[data-keys]')].map(each => each.dataset.keys);
const setNode = (id: string, params: Record<string, unknown>): Op => ({ op: 'set_fusion_node', ...AT, id, params });
/** The settings of a node of the composition of the first clip, as the engine has them. */
const paramsIn = (project: Project, id: string) => (project.timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!.nodes
  .find(node => node.id === id)!.params;

describe('a Planar Tracker on the Fusion page', () => {
  it('is added from the group of the effects and from Select Tool, after the node chosen, with its picture, its foreground and its mask', async () => {
    const { bridge } = await mount();
    expect(marked('add-planar_tracker')!.title).toBe('Add Node: Planar Tracker');
    bridge.call.mockClear();
    fireEvent.click(marked('add-planar_tracker')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('planar_tracker') }]]);
    const ports = [...nodeOf('n10').querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
    expect([currentNode(), nodeOf('n10').dataset.kind, ports]).toEqual(['n10', 'planar_tracker', ['in:input', 'in:foreground', 'in:mask', 'out']]);
    // Select Tool finds it by its name, among the effects; taken, it goes in after the Transform chosen.
    await choose('n3');
    bridge.call.mockClear();
    key(' ', { code: 'Space', shiftKey: true }); await settle();
    const field = screen.getByRole('combobox', { name: 'Search tools' });
    fireEvent.change(field, { target: { value: 'planar' } });
    const options = [...document.querySelectorAll<HTMLElement>('[data-test="select-tool-option"]')];
    expect(options.map(each => [each.dataset.kind, each.querySelector('.fusion-select-tool-group')?.textContent])).toEqual([['planar_tracker', 'Effects']]);
    fireEvent.keyDown(field, { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('planar_tracker'), links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
  });

  it('shows its controls with the defaults of the engine and no buttons of keys, and sets each by a patch', async () => {
    const { bridge, notify, project } = await mount(projectOf(comp({ path: { 30: STILL, 40: shifted(0.1, 0) } })));
    await notify({ frame: 30, shown: 30 });
    await choose('n6');
    expect([inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['planar_tracker', 'Planar Tracker 1']);
    expect(sections()).toEqual(['Controls']);
    expect(fieldsShown()).toEqual(['label', 'operation', 'motion', 'reference']);
    expect([select('operation').value, [...select('operation').options].map(each => each.textContent)]).toEqual(['none', ['None', 'Steady', 'Corner Pin']]);
    expect([select('motion').value, [...select('motion').options].map(each => each.textContent)])
      .toEqual(['perspective', ['Translation', 'Translation/Rotation', 'Translation/Rotation/Scale', 'Perspective']]);
    expect([box('reference').value, marked('planar-path')!.textContent, withKeys()]).toEqual(['0', 'Track: 2 frames, 30–40', []]);
    expect(marked('planar-region')!.textContent).toBe(en.hint.studio.fusionPage.planarRegion);
    bridge.call.mockClear();
    fireEvent.change(select('motion'), { target: { value: 'translation' } }); await settle();
    type('reference', '25'); await settle();
    // Set takes the frame of the keys at the playhead.
    fireEvent.click(marked('set-reference')!); await settle();
    fireEvent.change(select('operation'), { target: { value: 'corner_pin' } }); await settle();
    // With the Corner Pin, its corners, which are set whole.
    expect(sections()).toEqual(['Controls', 'Corner Pin']);
    expect(fieldsShown().slice(4)).toEqual(['corners.tl.x', 'corners.tl.y', 'corners.tr.x', 'corners.tr.y', 'corners.br.x', 'corners.br.y', 'corners.bl.x', 'corners.bl.y']);
    expect(box('corners.br.y').value).toBe('0.250');
    type('corners.tl.x', '0.2'); await settle();
    const corners = newPlanarTracker().corners;
    expect(sent(bridge)).toEqual([
      [setNode('n6', { motion: 'translation' })], [setNode('n6', { reference: 25 })], [setNode('n6', { reference: 30 })],
      [setNode('n6', { operation: 'corner_pin' })], [setNode('n6', { corners: { ...corners, tl: { x: 0.2, y: 0.75 } } })],
    ]);
    // Clear Track takes the path away, each frame given null; then there is nothing to clear.
    bridge.call.mockClear();
    fireEvent.click(marked('clear-track')!); await settle();
    expect(sent(bridge)).toEqual([[setNode('n6', { path: { 30: null, 40: null } })]]);
    expect([(paramsIn(project(), 'n6') as FusionPlanarTracker).path, marked('planar-path')!.textContent, (marked('clear-track') as HTMLButtonElement).disabled])
      .toEqual([{}, 'Not tracked', true]);
    expect(marked('fusion-inspector-problem')).toBeNull();
  });

  it('tracks its region from the frame of the keys at the playhead, says the frames lost and how it ended, and why the engine would not track', async () => {
    const { bridge, notify, news, answerTrack } = await mount();
    const button = (name: string) => marked(`tracker-${name}`) as HTMLButtonElement;
    const status = () => marked('fusion-track-status')?.textContent ?? null;
    const planarTrack = en.hint.studio.fusionPage.planarTrack;
    await notify({ frame: 30, shown: 30 });
    await choose('n6');
    expect([...marked('fusion-track')!.querySelectorAll<HTMLButtonElement>('button')].map(each => [each.getAttribute('aria-label'), each.disabled])).toEqual([
      ['Track Reverse', false], ['Track One Frame Reverse', false], ['Stop Tracking', true], ['Track One Frame Forward', false], ['Track Forward', false],
      ['Track Forward and Back', false],
    ]);
    fireEvent.click(button('forward')); await settle();
    expect(calls(bridge, 'track.start')).toEqual([{ planar: { timeline: 't1', clip: 'c1', node: 'n6' }, revision: expect.any(Number), frame: 30, direction: 'forward' }]);
    // While it goes, Stop alone is on, and nothing of the Planar Tracker is changed.
    expect([button('forward').disabled, button('stop').disabled, select('operation').disabled]).toEqual([true, false, true]);
    await news('track.progress', { job: 'track-1', frame: 35, timeline_frame: 35, done: 5, total: 70, lost: 2, revision: 3 });
    expect(status()).toBe('Tracking 00:00:01:10: frame 5 of 70. Frames lost: 2.');
    fireEvent.click(button('stop')); await settle();
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-1' }]);
    await news('track.state', { job: 'track-1', phase: 'stopped', reason: 'stop', frames: 5, lost: 2, frame: 35, revision: 4, tracked: [30, 35] });
    expect([status(), select('operation').disabled]).toEqual([en.hint.studio.fusionPage.track.stopped, false]);
    // Done, with the frames lost; and a region with nothing to follow.
    fireEvent.click(button('reverse')); await settle();
    await news('track.state', { job: 'track-1', phase: 'done', reason: null, frames: 12, lost: 1, frame: 18, revision: 4, tracked: [18, 35] });
    expect(status()).toBe('Frames tracked: 12. Frames lost: 1.');
    fireEvent.click(button('one_forward')); await settle();
    await news('track.state', { job: 'track-1', phase: 'failed', reason: 'no_features', frames: 0, lost: 0, frame: 35, revision: 4, tracked: null, message: 'none' });
    expect(status()).toBe(planarTrack.failed.no_features);
    expect(calls(bridge, 'track.start').map(each => (each as { direction: string }).direction)).toEqual(['forward', 'reverse', 'one_forward']);
    // The keys of the Tracker menu of the Color page track it too.
    key('t', { code: 'KeyT', altKey: true }); await settle();
    expect(calls(bridge, 'track.start').at(-1)).toEqual(expect.objectContaining({ planar: { timeline: 't1', clip: 'c1', node: 'n6' }, direction: 'reverse' }));
    await news('track.state', { job: 'track-1', phase: 'done', reason: null, frames: 1, lost: 0, frame: 29, revision: 4, tracked: [29, 30] });
    // Refusals, in the words of the Planar Tracker where it has its own.
    for (const reason of ['region', 'target', 'busy', 'untrackable', 'stale_revision'] as const) {
      answerTrack({ ok: false, error: { code: -32001, message: 'refused', data: { reason } } });
      fireEvent.click(button('forward')); await settle();
      const said = reason === 'region' || reason === 'target' ? planarTrack.failed[reason] : en.hint.studio.fusionPage.track.failed[reason];
      expect([status(), button('forward').disabled, button('stop').disabled], reason).toEqual([said, false, true]);
    }
    expect(marked('fusion-inspector-problem')).toBeNull();
  });
});

/** The region of a new Planar Tracker on the picture: its four places, from the lower left one round. */
const REGION = [[600, 810], [1320, 810], [1320, 270], [600, 270]];

describe('the region of a Planar Tracker on the picture of the Fusion page', () => {
  const region = newPlanarTracker().region;
  /** The region set whole, with the corners of the box about it. */
  const regionOf = (places: { x: number; y: number }[], corners = newPlanarTracker().corners) => setNode('n6', { region: places, corners });

  it('is drawn, and a place of it dragged as one gesture: the region set whole at each move, with the corners of the box about it', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n6');
    expect(overlays(bridge).at(-1)).toEqual({ canvas: CANVAS, lines: [[...REGION, REGION[0]]], dots: [{ kind: 'key', at: REGION }] });
    bridge.call.mockClear();
    // The upper right place, taken a tenth of the width right and a tenth of the height up.
    engine.pointer('down', 1322, 272);
    engine.pointer('move', 1466, 164); await settle();
    const moved = [region[0], region[1], { x: 0.85, y: 0.85 }, region[3]];
    const about = { tl: { x: 0.25, y: 0.85 }, tr: { x: 0.85, y: 0.85 }, br: { x: 0.85, y: 0.25 }, bl: { x: 0.25, y: 0.25 } };
    expect(sent(bridge)).toEqual([[regionOf(moved, about)]]);
    engine.pointer('up', 1466, 164); await settle();
    // One gesture, one step to undo.
    expect(applied(bridge).map(each => each.gesture!.end)).toEqual([undefined, true]);
    expect(new Set(applied(bridge).map(each => each.gesture!.id)).size).toBe(1);
    expect((paramsIn(engine.project(), 'n6') as FusionPlanarTracker).region).toEqual(moved);
    expect(overlays(bridge).at(-1).lines[0][2]).toEqual([1464, 162]);
  });

  it('takes a place where a side is pressed, which the drag that follows moves, all one step; and a place away by Alt or the middle button', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n6');
    bridge.call.mockClear();
    // A press on the lower side, off its handles: a place there, at once, and the release ends the gesture.
    engine.pointer('down', 960, 812); await settle();
    const grown = [region[0], { x: 0.5, y: 0.25 }, ...region.slice(1)];
    expect(sent(bridge)).toEqual([[regionOf(grown)]]);
    engine.pointer('up', 960, 812); await settle();
    expect(applied(bridge).map(each => each.gesture!.end)).toEqual([undefined, true]);
    expect((paramsIn(engine.project(), 'n6') as FusionPlanarTracker).region).toEqual(grown);
    // Pressed and dragged: the place put goes with the pointer, in the same gesture.
    bridge.call.mockClear();
    engine.pointer('down', 1322, 540);
    engine.pointer('move', 1466, 540); await settle();
    engine.pointer('up', 1466, 540); await settle();
    const right = [...grown.slice(0, 3), { x: 0.75, y: 0.5 }, ...grown.slice(3)];
    const further = [...grown.slice(0, 3), { x: 0.85, y: 0.5 }, ...grown.slice(3)];
    const wider = { tl: { x: 0.25, y: 0.75 }, tr: { x: 0.85, y: 0.75 }, br: { x: 0.85, y: 0.25 }, bl: { x: 0.25, y: 0.25 } };
    expect(sent(bridge)).toEqual([[regionOf(right)], [regionOf(further, wider)], [regionOf(further, wider)]]);
    expect(new Set(applied(bridge).map(each => each.gesture!.id)).size).toBe(1);
    // Alt and a press on a place takes it away, one transaction; so does the middle button.
    bridge.call.mockClear();
    engine.pointer('down', 1464, 540, { alt: true }); await settle();
    engine.pointer('up', 1464, 540, { alt: true }); await settle();
    expect(sent(bridge)).toEqual([[regionOf(grown)]]);
    expect(applied(bridge)[0].gesture!.end).toBe(true);
    engine.pointer('down', 960, 810, { button: 'middle', buttons: ['middle'] }); await settle();
    engine.pointer('up', 960, 810, { button: 'middle', buttons: [] }); await settle();
    expect(sent(bridge).at(-1)).toEqual([regionOf(region)]);
    // Three places are the fewest: one is taken away down to them, and none below them.
    engine.pointer('down', 600, 810, { alt: true }); engine.pointer('up', 600, 810, { alt: true }); await settle();
    expect(sent(bridge)).toHaveLength(3);
    expect(sent(bridge).at(-1)).toEqual([regionOf(region.slice(1))]);
    const before = applied(bridge).length;
    engine.pointer('down', 1320, 270, { alt: true }); engine.pointer('up', 1320, 270, { alt: true }); await settle();
    expect(applied(bridge)).toHaveLength(before);
    expect((paramsIn(engine.project(), 'n6') as FusionPlanarTracker).region).toEqual(region.slice(1));
  });

  it('has its corners dragged with the Corner Pin, the corners set whole and the region left as it is', async () => {
    const engine = await mount(projectOf(comp({ operation: 'corner_pin' })));
    const { bridge } = engine;
    await choose('n6');
    expect(overlays(bridge).at(-1).lines[1]).toEqual([[600, 270], [1320, 270], [1320, 810], [600, 810], [600, 270]]);
    bridge.call.mockClear();
    // The upper left corner lies on the upper left place of the region: the corner is the one pressed.
    engine.pointer('down', 600, 270);
    engine.pointer('move', 456, 270); await settle();
    engine.pointer('up', 456, 270); await settle();
    const corners = { ...newPlanarTracker().corners, tl: { x: 0.15, y: 0.75 } };
    expect(sent(bridge)).toEqual([[setNode('n6', { corners })], [setNode('n6', { corners })]]);
    expect((paramsIn(engine.project(), 'n6') as FusionPlanarTracker).region).toEqual(region);
  });
});

describe('a Text+ on the Fusion page', () => {
  it('is added from the tool bar and from Select Tool, with no links, whatever is chosen', async () => {
    const { bridge } = await mount();
    expect(marked('add-text_plus')!.title).toBe('Add Node: Text+');
    await choose('n3');
    bridge.call.mockClear();
    fireEvent.click(marked('add-text_plus')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('text_plus') }]]);
    const ports = [...nodeOf('n10').querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
    expect([currentNode(), nodeOf('n10').dataset.kind, ports]).toEqual(['n10', 'text_plus', ['in:mask', 'out']]);
    bridge.call.mockClear();
    key(' ', { code: 'Space', shiftKey: true }); await settle();
    const field = screen.getByRole('combobox', { name: 'Search tools' });
    fireEvent.change(field, { target: { value: 'text' } });
    expect([...document.querySelectorAll<HTMLElement>('[data-test="select-tool-option"]')].map(each => each.dataset.kind)).toEqual(['text_plus']);
    fireEvent.keyDown(field, { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('text_plus') }]]);
  });

  it('shows its settings in the sections of Fusion\'s tabs, each number with its buttons of keys, and sets what takes none by a patch', async () => {
    const { bridge } = await mount();
    await choose('n7');
    expect([inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['text_plus', 'Text+ 1']);
    expect(sections()).toEqual(['Text', 'Layout', 'Transform', 'Shading', 'Follower']);
    expect(fieldsShown()).toEqual([
      'label', 'text', 'font', 'size', 'tracking', 'line_spacing', 'justify', 'vertical', 'angle', 'write_on.start', 'write_on.end', 'center.x', 'center.y',
      'transform.select', 'transform.offset.x', 'transform.offset.y', 'transform.angle', 'transform.scale', 'fill.color', 'fill.color.r', 'fill.color.g',
      'fill.color.b', 'fill.color.a', 'outline', 'border', 'follower',
    ]);
    expect(withKeys()).toEqual([
      'size', 'tracking', 'line_spacing', 'angle', 'write_on.start', 'write_on.end', 'center.x', 'center.y', 'transform.offset.x', 'transform.offset.y',
      'transform.angle', 'transform.scale', 'fill.color.r', 'fill.color.g', 'fill.color.b', 'fill.color.a',
    ]);
    expect([box('size').value, box('line_spacing').value, select('justify').value, select('vertical').value, select('transform.select').value])
      .toEqual(['0.080', '1.000', 'center', 'center', 'characters']);
    bridge.call.mockClear();
    const textArea = row('text').querySelector('textarea')!;
    fireEvent.change(textArea, { target: { value: 'Hello\nWorld' } }); fireEvent.keyDown(textArea, { key: 'Enter' }); await settle();
    const font = row('font').querySelector('input')!;
    fireEvent.change(font, { target: { value: 'Arial' } }); fireEvent.keyDown(font, { key: 'Enter' }); await settle();
    fireEvent.change(select('justify'), { target: { value: 'left' } }); await settle();
    fireEvent.change(select('vertical'), { target: { value: 'top' } }); await settle();
    fireEvent.change(select('transform.select'), { target: { value: 'words' } }); await settle();
    type('size', '0.1'); await settle();
    type('transform.offset.x', '0.25'); await settle();
    fireEvent.change(row('fill.color').querySelector('input[type="color"]')!, { target: { value: '#ff0000' } }); await settle();
    expect(sent(bridge)).toEqual([
      [setNode('n7', { text: 'Hello\nWorld' })], [setNode('n7', { font: 'Arial' })], [setNode('n7', { justify: 'left' })], [setNode('n7', { vertical: 'top' })],
      [setNode('n7', { transform: { select: 'words' } })], [setNode('n7', { size: 0.1 })], [setNode('n7', { transform: { offset: { x: 0.25 } } })],
      [setNode('n7', { fill: { color: { g: 0, b: 0 } } })],
    ]);
    // A key put at the playhead, of the value that the setting has.
    bridge.call.mockClear();
    fireEvent.click(keysOf('size')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n7.size', at: 0, remove: 0, keys: [{ frame: 0, value: 0.1, interp: 'linear' }] }]]);
    expect(marked('fusion-inspector-problem')).toBeNull();
  });

  it('turns its parts on as the engine makes them and off, one step each, offering keys of a part only while it has it', async () => {
    const { bridge, project } = await mount();
    await choose('n7');
    const part = (name: string) => marked(`text-${name}`) as HTMLInputElement;
    expect([part('outline').checked, part('border').checked, part('follower').checked]).toEqual([false, false, false]);
    expect([keysOf('outline.thickness'), keysOf('border.extend'), keysOf('follower.opacity')]).toEqual([[], [], []]);
    bridge.call.mockClear();
    fireEvent.click(part('outline')); await settle();
    expect(sent(bridge)).toEqual([[setNode('n7', { outline: newOutline() })]]);
    expect(part('outline').checked).toBe(true);
    expect(withKeys().slice(16)).toEqual(['outline.color.r', 'outline.color.g', 'outline.color.b', 'outline.color.a', 'outline.thickness']);
    expect(keysOf('outline.thickness')).toHaveLength(3);
    type('outline.thickness', '0.05'); await settle();
    fireEvent.click(part('border')); await settle();
    fireEvent.click(part('follower')); await settle();
    expect(fieldsShown().slice(-9)).toEqual([
      'follower', 'follower.order', 'follower.delay', 'follower.delay_type', 'follower.opacity', 'follower.offset.x', 'follower.offset.y', 'follower.angle',
      'follower.scale',
    ]);
    fireEvent.change(select('follower.order'), { target: { value: 'inside_out' } }); await settle();
    type('follower.delay', '5'); await settle();
    fireEvent.change(select('follower.delay_type'), { target: { value: 'first_to_last' } }); await settle();
    // Its Delay takes no keys.
    expect(keysOf('follower.delay')).toEqual([]);
    expect(sent(bridge).slice(1)).toEqual([
      [setNode('n7', { outline: { thickness: 0.05 } })], [setNode('n7', { border: newBorder() })], [setNode('n7', { follower: newFollower() })],
      [setNode('n7', { follower: { order: 'inside_out' } })], [setNode('n7', { follower: { delay: 5 } })], [setNode('n7', { follower: { delay_type: 'first_to_last' } })],
    ]);
    // Off: the part is taken away, and with it its settings and their keys.
    bridge.call.mockClear();
    fireEvent.click(part('outline')); await settle();
    expect(sent(bridge)).toEqual([[setNode('n7', { outline: null })]]);
    expect(Object.keys(paramsIn(project(), 'n7'))).not.toContain('outline');
    expect([part('outline').checked, keysOf('outline.thickness'), row('outline.color.r')]).toEqual([false, [], null]);
    expect((paramsIn(project(), 'n7') as FusionTextPlus).follower).toEqual({ ...newFollower(), order: 'inside_out', delay: 5, delay_type: 'first_to_last' });
  });

  it('has the handle of its centre on the picture, dragged as one gesture', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n7');
    expect(overlays(bridge).at(-1)).toEqual({ canvas: CANVAS, lines: [], dots: [{ kind: 'current', at: [[960, 540]] }] });
    bridge.call.mockClear();
    engine.pointer('down', 966, 540);
    engine.pointer('move', 1110, 432); await settle();
    expect(sent(bridge)).toEqual([[setNode('n7', { center: { x: 0.6 } }), setNode('n7', { center: { y: 0.6 } })]]);
    engine.pointer('up', 1110, 432); await settle();
    expect(applied(bridge).map(each => each.gesture!.end)).toEqual([undefined, true]);
    expect((paramsIn(engine.project(), 'n7') as FusionTextPlus).center).toEqual({ x: 0.6, y: 0.6 });
  });
});
