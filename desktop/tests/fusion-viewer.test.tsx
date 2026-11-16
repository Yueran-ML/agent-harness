// @vitest-environment jsdom
/**
 * The viewer of the Fusion page (`src/studio/pages/FusionPage`, `src/studio/fusion/useFusionView` and
 * `useFusionOnscreen`; PROTOCOL.md, "The Fusion view"): the Fusion view that the page asks the engine
 * for and turns off again, the node that is viewed and how the page says which it is, the on-screen
 * controls of the node that is chosen as the engine is asked to draw them, a handle dragged as one
 * gesture, a clip on a locked track, and when the engine is asked to draw nothing.
 *
 * The engine of the tests shows the picture of a composition as the worked example of the protocol
 * has it: 1440 by 1080 in a timeline of 1920 by 1080, at 240, 0. The view shows the frame half as
 * large, so 8 pixels of the screen are 16 of the frame.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { Studio } from '../src/studio/Studio';
import {
  applyOp, emptyGrade, ProjectCopy, type Composition, type EngineEvent, type FusionLink, type FusionNode, type FusionPicture, type FusionView, type Keyframe, type Op,
  type Project, type Timeline, type TransportState, type ViewPointer,
} from '../src/studio/engine';
import { keepCopied } from '../src/studio/fusion/copied';
import { newNode, startPoints } from '../src/studio/fusion/edits';
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
/** The Fusion view of a clip as the engine of the tests tells it. */
const viewOf = (of: string, node?: string): FusionView => ({ clip: of, ...(node ? { node } : {}), shown: true, canvas: [1920, 1080], size: { width: 1440, height: 1080 }, picture: PICTURE });
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
const withParams = (node: FusionNode, params: Record<string, unknown>): FusionNode => ({ ...node, params: { ...node.params, ...params } } as unknown as FusionNode);
const k = (frame: number, value: number): Keyframe => ({ frame, value, interp: 'linear' });
/**
 * MediaIn1 through the Transform n3 into the background of the Merge n4, the Background n5 its foreground, and the Merge into
 * MediaOut1; and three masks that nothing is linked to or from: the Rectangle n6 (a new one, but for what a test gives it), the
 * Polygon n7 of the four points that the page gives a new one, half its size and a quarter turn, and the Ellipse n8.
 * Laid out by the editor, the cards (92 by 58) stand at: MediaIn1 28,28; n5 28,120; n3 176,28; n4 324,28; MediaOut1 472,28; and the
 * masks in the first column under n5, at 28,212; 28,304 and 28,396.
 */
const comp = (rectangle: Record<string, unknown> = {}): Composition => ({
  nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), newNode('transform', null, 'n3'), newNode('merge', null, 'n4'),
    newNode('background', null, 'n5'), withParams(newNode('rectangle', null, 'n6'), rectangle),
    withParams(newNode('polygon', null, 'n7'), { points: startPoints(), size: 0.5, angle: 90 }), newNode('ellipse', null, 'n8')],
  links: [link('MediaIn1', 'n3'), link('n3', 'n4', 'background'), link('n5', 'n4', 'foreground'), link('n4', 'MediaOut1')],
});
/** The middles of the cards. */
const P = { in1: { x: 74, y: 57 }, n3: { x: 222, y: 57 }, n4: { x: 370, y: 57 }, out1: { x: 518, y: 57 }, n6: { x: 74, y: 241 }, n7: { x: 74, y: 333 }, n8: { x: 74, y: 425 } };
/** V1: c1 (frames 0–99, with the composition) and c2 (100–199, with none); nothing from frame 200 on. */
const timelineOf = (first: Composition = comp(), locked = false): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, transitions: [], clips: [clip('c1', 0, 100, { fusion: first }), clip('c2', 100, 100)] }],
});
const projectOf = (timelines: Timeline[] = [timelineOf()]): Project => ({ schema: 5, next_id: 10, bins: [], groups: [], media: [media], timelines });

/**
 * An engine that keeps a project and applies the operations of a transaction as the copy does; that has a Fusion view as the real
 * one has: `viewer.fusion` turns it on and off, the state of the timeline monitor tells it, and a test has a call refused as the
 * real engine refuses it; and that tells what a test says the pointer does on the picture.
 */
function fakeEngine(given: Project) {
  let project = given;
  let revision = 0;
  let refused: { data: Record<string, unknown>; when: (params: Record<string, unknown>) => boolean } | null = null;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
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
        if (refused?.when(asked)) return { ok: false, error: { code: -32602, message: 'refused', data: refused.data } };
        const fusion = typeof asked.clip === 'string' ? viewOf(asked.clip, typeof asked.node === 'string' ? asked.node : undefined) : undefined;
        state = { ...state, fusion } as TransportState;
        tell('transport.state', state); return { ok: true, result: { monitor: 'timeline', fusion: fusion ?? null } };
      }
      if (method === 'transaction.apply') {
        const done = params!.ops as Op[];
        for (const op of done) project = applyOp(project, op);
        revision++;
        tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: project.next_id, done });
        return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
      }
      if (method === 'history.undo' || method === 'history.redo') return { ok: true, result: { revision, done: null } };
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn> };
  let event = 0;
  return {
    bridge, copy: new ProjectCopy(bridge),
    project: () => project,
    /** Have the engine refuse the calls of `viewer.fusion` that `when` says yes to, with this `data`; null has it refuse none. */
    refuseView: (data: Record<string, unknown> | null, when: (params: Record<string, unknown>) => boolean = () => true) => { refused = data === null ? null : { data, when }; },
    /** Another client changes the project: the operations are applied and told, as made by an agent. */
    outside: (ops: Op[]) => act(() => {
      for (const op of ops) project = applyOp(project, op);
      revision++;
      tell('project.changed', { revision, made_by: 'agent', by: 'agent', next_id: project.next_id, done: ops });
    }),
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
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
/** The studio on its engine, on the Fusion page. */
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
/** The operations of each transaction sent. */
const sent = (bridge: Calls) => applied(bridge).map(each => each.ops);
/** What `viewer.fusion` was asked, in order. */
const views = (bridge: Calls) => calls(bridge, 'viewer.fusion');
/** What the engine was asked to draw over the picture, in order: null where it was asked to draw nothing. */
const overlays = (bridge: Calls) => calls(bridge, 'view.overlay').map(params => params.overlay);
const pointers = (bridge: Calls) => calls(bridge, 'view.pointer');
const marked = (name: string) => document.querySelector<HTMLElement>(`[data-test="${name}"]`);
const graph = () => document.querySelector<SVGElement>('[data-test="fusion-graph"]')!;
const canvas = () => document.querySelector<HTMLElement>('[data-test="node-canvas"]')!;
const nodeOf = (id: string) => graph().querySelector<SVGElement>(`[data-node="${id}"]`)!;
const card = (id: string) => nodeOf(id).querySelector<SVGElement>('.color-node-card')!;
const viewedNodes = () => [...graph().querySelectorAll<SVGElement>('[data-viewed]')].map(each => each.dataset.node);
const viewedText = () => marked('fusion-viewed')?.textContent ?? null;
const problem = () => marked('node-problem')?.textContent ?? null;
const key = (value: string, fields: Record<string, unknown> = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
type At = { x: number; y: number };
/** A press and a release in the node editor, on a node or anywhere in it. */
const press = (element: Element, at: At, fields: Record<string, unknown> = {}) => {
  fireEvent.pointerDown(element, { button: 0, clientX: at.x, clientY: at.y, ...fields });
  fireEvent.pointerUp(canvas(), { button: 0, clientX: at.x, clientY: at.y, ...fields });
};
/** Choose a node of the composition by a press on its card. */
const choose = async (id: 'n3' | 'n4' | 'n6' | 'n7' | 'n8' | 'MediaIn1' | 'MediaOut1') => {
  press(card(id), id === 'MediaIn1' ? P.in1 : id === 'MediaOut1' ? P.out1 : P[id]);
  await settle();
};
const setNode = (id: string, params: Record<string, unknown>): Op => ({ op: 'set_fusion_node', ...AT, id, params });

/** A new Rectangle on the picture: 720 by 540 pixels about 960, 540, with the handles of its centre, its right side and its upper side. */
const RECTANGLE = {
  canvas: CANVAS, lines: [[[600, 810], [1320, 810], [1320, 270], [600, 270], [600, 810]]],
  dots: [{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1320, 540], [960, 270]] }],
};
const TAKEN = { monitor: 'timeline', on: true, cursor: 'arrow' };
const LET_GO = { monitor: 'timeline', on: false };
const NOTHING = { monitor: 'timeline', overlay: null };

describe('the Fusion view of the timeline monitor', () => {
  it('is asked for as the page is entered with a clip, again for another clip, and turned off as the page is left: only when something changes', async () => {
    const { bridge, notify, outside } = await mount();
    expect(views(bridge)).toEqual([{ clip: 'c1' }]);
    // Each is asked of the engine that the page shows.
    expect(bridge.call.mock.calls.filter(([method]) => method === 'viewer.fusion').every(([, , context]) => context?.expectedSession === 1)).toBe(true);
    // The frames of the clip go by, a node is chosen, the composition is edited: the view is the same, and is not asked for again.
    await notify({ frame: 10, shown: 10 }); await settle();
    await choose('n6');
    await outside([setNode('n6', { angle: 10 })]); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }]);
    // The playhead comes onto another clip, which has no composition yet: it is viewed as every clip is.
    await notify({ frame: 150, shown: 150 }); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c2' }]);
    // No clip at the playhead: the view is turned off, once.
    await notify({ frame: 250, shown: 250 }); await settle();
    await notify({ frame: 260, shown: 260 }); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c2' }, { clip: null }]);
    await notify({ frame: 20, shown: 20 }); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c2' }, { clip: null }, { clip: 'c1' }]);
    // The page left: the monitor shows the timeline again.
    await open('Media');
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c2' }, { clip: null }, { clip: 'c1' }, { clip: null }]);
  });

  it('is not turned off as the page is left where it is off already', async () => {
    const { bridge, notify } = await mount();
    await notify({ frame: 250, shown: 250 }); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: null }]);
    await open('Media');
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: null }]);
  });

  it('goes back to the output where the engine has not the node, and off where it has not the clip, and says nothing of either', async () => {
    const { bridge, notify, refuseView } = await mount();
    await choose('n8');
    refuseView({ reason: 'node', node: 'n8' }, params => params.node !== undefined);
    key('1'); await settle();
    // The monitor shows what it showed before, the output: it is not asked for again.
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c1', node: 'n8' }]);
    expect([viewedText(), viewedNodes(), problem()]).toEqual(['Viewer: Output', [], null]);
    // A clip that the engine has not: the view of the clip before is turned off, and the clip is not asked for again as frames go by.
    refuseView({ reason: 'clip', clip: 'c2' }, params => params.clip === 'c2');
    await notify({ frame: 150, shown: 150 }); await settle();
    await notify({ frame: 151, shown: 151 }); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }, { clip: 'c1', node: 'n8' }, { clip: 'c2' }, { clip: null }]);
    expect(problem()).toBeNull();
    // Off already, it is not turned off again as the page is left.
    await open('Media');
    expect(views(bridge)).toHaveLength(4);
  });
});

describe('the node that the viewer of the Fusion page shows', () => {
  it('is the node chosen for the key 1, with a dot on its card and its name in the bar, and the output again for the same again or the key 0', async () => {
    const { bridge } = await mount();
    expect([viewedText(), viewedNodes()]).toEqual(['Viewer: Output', []]);
    // With no node chosen the key shows nothing else.
    key('1'); await settle();
    expect(views(bridge)).toEqual([{ clip: 'c1' }]);
    await choose('n8');
    key('1'); await settle();
    expect(views(bridge).at(-1)).toEqual({ clip: 'c1', node: 'n8' });
    expect(viewedNodes()).toEqual(['n8']);
    expect(nodeOf('n8').querySelector('[data-test="node-viewed"]')).not.toBeNull();
    expect(nodeOf('n6').querySelector('[data-test="node-viewed"]')).toBeNull();
    expect([viewedText(), marked('fusion-viewed')!.dataset.node]).toEqual(['Viewer: Ellipse 1', 'n8']);
    // Another node chosen is not the node viewed, until the key says so.
    await choose('n6');
    expect([views(bridge).length, viewedNodes()]).toEqual([2, ['n8']]);
    key('1', { code: 'Digit1' }); await settle();
    expect([views(bridge).at(-1), viewedNodes(), viewedText()]).toEqual([{ clip: 'c1', node: 'n6' }, ['n6'], 'Viewer: Rectangle 1']);
    // The same again goes back to what comes out of the composition.
    key('1'); await settle();
    expect([views(bridge).at(-1), viewedNodes(), viewedText(), marked('fusion-viewed')!.dataset.node]).toEqual([{ clip: 'c1' }, [], 'Viewer: Output', undefined]);
    // And so does the key 0, which does nothing where the output is shown already.
    key('1'); await settle();
    key('0'); await settle();
    key('0'); await settle();
    expect(views(bridge).slice(3)).toEqual([{ clip: 'c1' }, { clip: 'c1', node: 'n6' }, { clip: 'c1' }]);
    // The MediaOut is the output itself: nothing else is asked for.
    await choose('MediaOut1');
    key('1'); await settle();
    expect([views(bridge).length, viewedNodes()]).toEqual([6, []]);
    // The key is the node editor's: while the Inspector has the focus it shows nothing.
    await choose('n8');
    fireEvent.pointerDown(marked('fusion-inspector')!);
    key('1'); await settle();
    expect([views(bridge).length, viewedNodes()]).toEqual([6, []]);
  });

  it('is chosen from the menu of a node, and the output again from the menus', async () => {
    const { bridge } = await mount();
    fireEvent.contextMenu(card('n6'), { clientX: P.n6.x, clientY: P.n6.y });
    expect((screen.getByRole('menuitem', { name: 'View Output' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('menuitem', { name: 'View' }).dataset.checked).toBeUndefined();
    fireEvent.click(screen.getByRole('menuitem', { name: 'View' }));
    await settle();
    expect([views(bridge).at(-1), viewedNodes()]).toEqual([{ clip: 'c1', node: 'n6' }, ['n6']]);
    // The menu of the node that is viewed has View checked, and View Output on.
    fireEvent.contextMenu(card('n6'), { clientX: P.n6.x, clientY: P.n6.y });
    expect(screen.getByRole('menuitem', { name: 'View' }).dataset.checked).toBe('true');
    fireEvent.click(screen.getByRole('menuitem', { name: 'View Output' }));
    await settle();
    expect([views(bridge).at(-1), viewedNodes()]).toEqual([{ clip: 'c1' }, []]);
    // The menu of the background has View Output too.
    fireEvent.contextMenu(card('n8'), { clientX: P.n8.x, clientY: P.n8.y });
    fireEvent.click(screen.getByRole('menuitem', { name: 'View' }));
    await settle();
    fireEvent.contextMenu(canvas(), { clientX: 590, clientY: 290 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'View Output' }));
    await settle();
    expect(views(bridge).slice(-2)).toEqual([{ clip: 'c1', node: 'n8' }, { clip: 'c1' }]);
  });

  it('is the output again once the node is taken away, and the output of another clip', async () => {
    const { bridge, notify, outside } = await mount();
    await choose('n6');
    key('1'); await settle();
    expect(views(bridge).at(-1)).toEqual({ clip: 'c1', node: 'n6' });
    await outside([{ op: 'remove_fusion_node', ...AT, id: 'n6' }]); await settle();
    expect([views(bridge).at(-1), viewedText(), viewedNodes()]).toEqual([{ clip: 'c1' }, 'Viewer: Output', []]);
    // Another clip begins with what comes out of its composition, whatever was viewed of the clip before.
    await choose('n8');
    key('1'); await settle();
    await notify({ frame: 150, shown: 150 }); await settle();
    expect([views(bridge).at(-1), viewedText(), viewedNodes()]).toEqual([{ clip: 'c2' }, 'Viewer: Output', []]);
  });

  it('is looked at on a locked track too: the viewer is the page\'s, and nothing of the project is changed', async () => {
    const { bridge } = await mount(projectOf([timelineOf(comp(), true)]));
    await choose('n8');
    key('1'); await settle();
    expect([views(bridge).at(-1), viewedNodes()]).toEqual([{ clip: 'c1', node: 'n8' }, ['n8']]);
    expect(sent(bridge)).toEqual([]);
  });
});

describe('the on-screen controls of the node chosen on the Fusion page', () => {
  it('are drawn in the pixels of the picture for a Rectangle, as it is turned, an Ellipse, a Polygon, a Merge and a Transform, and the view takes the pointer', async () => {
    const { bridge, outside } = await mount();
    // With no node chosen there is nothing to draw, and the mouse goes through the view to the page.
    expect([overlays(bridge), pointers(bridge)]).toEqual([[], []]);
    await choose('n6');
    expect(calls(bridge, 'view.overlay')).toEqual([{ monitor: 'timeline', overlay: RECTANGLE }]);
    expect(pointers(bridge)).toEqual([TAKEN]);
    expect(bridge.call.mock.calls.filter(([method]) => method === 'view.overlay' || method === 'view.pointer').every(([, , context]) => context?.expectedSession === 1)).toBe(true);
    // A quarter turn, counter-clockwise as it is seen: what was its right side is its upper side.
    await outside([setNode('n6', { angle: 90 })]); await settle();
    expect(overlays(bridge).at(-1)).toEqual({
      canvas: CANVAS, lines: [[[1230, 900], [1230, 180], [690, 180], [690, 900], [1230, 900]]],
      dots: [{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[960, 180], [690, 540]] }],
    });
    // A new Ellipse: a circle of 360 pixels about 960, 540, drawn through 64 places of it.
    await choose('n8');
    const circle = overlays(bridge).at(-1);
    expect([circle.canvas, circle.lines.length, circle.lines[0].length]).toEqual([CANVAS, 1, 65]);
    expect([0, 16, 32, 48, 64].map(step => circle.lines[0][step])).toEqual([[1320, 540], [960, 180], [600, 540], [960, 900], [1320, 540]]);
    expect(circle.lines[0][8]).toEqual([1214.558, 285.442]);
    expect(circle.dots).toEqual([{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1320, 540], [960, 180]] }]);
    // The Polygon, half its size and a quarter turn: 0.2 of the width is 144 pixels, and its lower left point is at the lower right.
    await choose('n7');
    expect(overlays(bridge).at(-1)).toEqual({
      canvas: CANVAS, lines: [[[1104, 684], [1104, 396], [816, 396], [816, 684], [1104, 684]]],
      dots: [{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1104, 684], [1104, 396], [816, 396], [816, 684]] }],
    });
    // A Merge and a Transform have the centre alone.
    await choose('n4');
    expect(overlays(bridge).at(-1)).toEqual({ canvas: CANVAS, lines: [], dots: [{ kind: 'current', at: [[960, 540]] }] });
    await outside([setNode('n3', { center: { x: 0.25, y: 0.75 } })]); await settle();
    await choose('n3');
    expect(overlays(bridge).at(-1)).toEqual({ canvas: CANVAS, lines: [], dots: [{ kind: 'current', at: [[600, 270]] }] });
    // From one node to the next the engine was never asked to draw nothing, and the view kept the pointer.
    expect(overlays(bridge)).toHaveLength(6);
    expect(overlays(bridge).every(each => each !== null)).toBe(true);
    expect(pointers(bridge)).toEqual([TAKEN]);
  });

  it('are drawn whichever node is viewed: a shape is in the coordinates of the composition either way', async () => {
    const { bridge } = await mount();
    await choose('n6');
    key('1'); await settle();
    expect(views(bridge).at(-1)).toEqual({ clip: 'c1', node: 'n6' });
    // The same controls, which are not sent again.
    expect(overlays(bridge)).toEqual([RECTANGLE]);
  });

  it('are told again only when they change, and follow a setting that has keys as the playhead moves', async () => {
    const { bridge, notify, outside } = await mount();
    await choose('n6');
    // Frames go by, and another node is edited: the same controls are not sent again.
    await notify({ frame: 5, shown: 5 }); await settle();
    await outside([{ op: 'set_fusion_node', ...AT, id: 'n3', pass_through: true }]); await settle();
    expect(overlays(bridge)).toEqual([RECTANGLE]);
    // The node is edited from anywhere: its controls are where it is.
    await outside([setNode('n6', { center: { x: 0.25 } })]); await settle();
    expect(overlays(bridge)).toHaveLength(2);
    expect(overlays(bridge).at(-1)).toEqual({
      canvas: CANVAS, lines: [[[240, 810], [960, 810], [960, 270], [240, 270], [240, 810]]],
      dots: [{ kind: 'current', at: [[600, 540]] }, { kind: 'key', at: [[960, 540], [600, 270]] }],
    });
    // Its centre given keys, from a quarter of the width at picture 0 to three quarters at picture 50: at picture 5 it is 0.3 of the width, at 25 half.
    await outside([{ op: 'set_fusion_keys', ...AT, path: 'n6.center.x', param: { keys: [k(0, 0.25), k(50, 0.75)] } }]); await settle();
    expect(overlays(bridge).at(-1).dots[0]).toEqual({ kind: 'current', at: [[672, 540]] });
    await notify({ frame: 25, shown: 25 }); await settle();
    expect(overlays(bridge).at(-1).dots[0]).toEqual({ kind: 'current', at: [[960, 540]] });
    expect(overlays(bridge)).toHaveLength(4);
  });

  it('are drawn no longer when the picture is not shown, when no node that has any is chosen, and as the page is left', async () => {
    const { bridge, notify } = await mount();
    await choose('n6');
    expect(overlays(bridge)).toEqual([RECTANGLE]);
    // The playhead leaves the clip, says the view: nothing is drawn, and the view lets the pointer go.
    await notify({ fusion: { ...viewOf('c1'), shown: false } }); await settle();
    expect(calls(bridge, 'view.overlay').at(-1)).toEqual(NOTHING);
    expect(pointers(bridge)).toEqual([TAKEN, LET_GO]);
    // Shown again, they are drawn again.
    await notify({ fusion: viewOf('c1') }); await settle();
    expect(overlays(bridge)).toEqual([RECTANGLE, null, RECTANGLE]);
    expect(pointers(bridge)).toEqual([TAKEN, LET_GO, TAKEN]);
    // A view that does not say where the picture lies has nothing to draw them on.
    await notify({ fusion: { clip: 'c1', shown: true, canvas: [1920, 1080] } }); await settle();
    expect(overlays(bridge)).toEqual([RECTANGLE, null, RECTANGLE, null]);
    await notify({ fusion: viewOf('c1') }); await settle();
    // A node that has no controls; no node; and several nodes.
    await choose('MediaIn1');
    expect(overlays(bridge)).toEqual([RECTANGLE, null, RECTANGLE, null, RECTANGLE, null]);
    await choose('n6');
    press(canvas(), { x: 590, y: 290 }); await settle();
    expect(overlays(bridge).slice(6)).toEqual([RECTANGLE, null]);
    await choose('n6');
    press(card('n8'), P.n8, { ctrlKey: true }); await settle();
    expect(overlays(bridge).slice(8)).toEqual([RECTANGLE, null]);
    // Nothing more is asked while there is nothing to draw.
    const before = calls(bridge, 'view.overlay').length + pointers(bridge).length;
    await notify({ frame: 3, shown: 3 }); await settle();
    expect(calls(bridge, 'view.overlay').length + pointers(bridge).length).toBe(before);
    // And as the page is left: the marks are cleared, the pointer is let go, and the monitor shows the timeline again.
    press(canvas(), { x: 590, y: 290 });
    await choose('n6');
    expect(overlays(bridge).at(-1)).toEqual(RECTANGLE);
    await open('Media');
    expect(calls(bridge, 'view.overlay').at(-1)).toEqual(NOTHING);
    expect(pointers(bridge).at(-1)).toEqual(LET_GO);
    expect(views(bridge).at(-1)).toEqual({ clip: null });
  });
});

describe('a handle dragged on the picture of the Fusion page', () => {
  it('moves the centre as one gesture: each move sets its two settings whole, kept within their range, and the release ends it', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n6');
    bridge.call.mockClear();
    // A press 6 pixels of the frame from the centre: within the 16 that 8 of the screen are.
    engine.pointer('down', 966, 540); await settle();
    expect(applied(bridge)).toEqual([]);
    // The handle that is held is drawn as the one dragged, and the viewer has the focus of the page.
    expect(overlays(bridge).at(-1).dots).toEqual([{ kind: 'key', at: [[1320, 540], [960, 270]] }, { kind: 'active', at: [[960, 540]] }]);
    expect(marked('fusion-viewer')!.dataset.focused).toBe('true');
    // 144 pixels right and 108 up: a tenth of the width and of the height of the picture.
    engine.pointer('move', 1110, 432); await settle();
    expect(sent(bridge)).toEqual([[setNode('n6', { center: { x: 0.6 } }), setNode('n6', { center: { y: 0.6 } })]]);
    expect(bridge.call.mock.calls.find(([method]) => method === 'transaction.apply')![2]).toEqual({ expectedSession: 1 });
    // The controls are drawn where the node is now.
    expect(overlays(bridge).at(-1)).toEqual({
      canvas: CANVAS, lines: [[[744, 702], [1464, 702], [1464, 162], [744, 162], [744, 702]]],
      dots: [{ kind: 'key', at: [[1464, 432], [1104, 162]] }, { kind: 'active', at: [[1104, 432]] }],
    });
    // Twenty widths further right, worked out from the node as it was at the press: the centre of a mask goes no further than 10.
    engine.pointer('move', 966 + 1440 * 20, 540); await settle();
    expect(sent(bridge).at(-1)).toEqual([setNode('n6', { center: { x: 10 } }), setNode('n6', { center: { y: 0.5 } })]);
    engine.pointer('up', 966 + 1440 * 20, 540); await settle();
    // One gesture, so one step to undo: the same id all through, ended by the last, which is the last move again.
    const moves = applied(bridge);
    expect(moves).toHaveLength(3);
    expect(moves[2].ops).toEqual(moves[1].ops);
    expect(new Set(moves.map(each => each.gesture?.id)).size).toBe(1);
    expect(moves[0].gesture!.id).not.toBe('');
    expect(moves.map(each => each.gesture!.end)).toEqual([undefined, undefined, true]);
    const moved = (engine.project().timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!.nodes.find(node => node.id === 'n6')!;
    expect((moved.params as { center: unknown }).center).toEqual({ x: 10, y: 0.5 });
    // Let go, the handle is one as the others, and the pointer moves nothing.
    expect(overlays(bridge).at(-1).dots.map((each: { kind: string }) => each.kind)).toEqual(['current', 'key']);
    engine.pointer('move', 500, 500); await settle();
    expect(applied(bridge)).toHaveLength(3);
    expect(problem()).toBeNull();
  });

  it('sends nothing for a press that does not move the handle, or that is on none', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n6');
    engine.pointer('down', 960, 540); engine.pointer('move', 960, 540); engine.pointer('up', 960, 540); await settle();
    // 17 pixels of the frame from the nearest handle: beyond the 16 that 8 of the screen are.
    engine.pointer('down', 977, 540); engine.pointer('move', 1100, 400); engine.pointer('up', 1100, 400); await settle();
    // On a handle, of the picture of another timeline; where the view shows no picture; and with the right button.
    engine.pointer('down', 960, 540, { snapshot: { revision: 0, timeline: 't2', frame: 0, canvas: [1920, 1080], ready: true, editable: true, comparison: 'off' } });
    engine.pointer('move', 1100, 400); engine.pointer('up', 1100, 400); await settle();
    engine.pointer('down', 960, 540, { x: null, y: null }); engine.pointer('move', 1100, 400); engine.pointer('up', 1100, 400); await settle();
    engine.pointer('down', 960, 540, { button: 'right', buttons: ['right'] }); engine.pointer('move', 1100, 400); engine.pointer('up', 1100, 400, { button: 'right' }); await settle();
    expect(applied(bridge)).toEqual([]);
  });

  it('moves a point of a Polygon where the pointer takes it, the mapping inverted: its numbers are set as numbers of a point are', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n7');
    bridge.call.mockClear();
    // The point at 816, 396 is the third, 0.2 and 0.2 from the centre before the polygon is sized and turned: taken 72 right and 144 down.
    engine.pointer('down', 816, 396);
    engine.pointer('move', 888, 540); await settle();
    const number = (path: string, param: number): Op => ({ op: 'set_fusion_keys', ...AT, path, param });
    expect(sent(bridge)).toEqual([[number('n7.points.2.x', 0), number('n7.points.2.y', 0.1)]]);
    engine.pointer('up', 888, 540); await settle();
    expect(applied(bridge).map(each => each.gesture!.end)).toEqual([undefined, true]);
    // The point is drawn where the pointer took it, and the others where they were.
    expect(overlays(bridge).at(-1)).toEqual({
      canvas: CANVAS, lines: [[[1104, 684], [1104, 396], [888, 540], [816, 684], [1104, 684]]],
      dots: [{ kind: 'current', at: [[960, 540]] }, { kind: 'key', at: [[1104, 684], [1104, 396], [888, 540], [816, 684]] }],
    });
  });

  it('sets a setting that has keys as a key at the playhead: the key there changed, or one put there', async () => {
    const keyed = { center: { x: { keys: [k(0, 0.25), k(50, 0.75)] }, y: 0.5 } };
    const engine = await mount(projectOf([timelineOf(comp(keyed))]));
    const { bridge } = engine;
    await choose('n6');
    // At picture 0 the centre is a quarter of the width from the left of the picture.
    expect(overlays(bridge).at(-1).dots[0]).toEqual({ kind: 'current', at: [[600, 540]] });
    bridge.call.mockClear();
    engine.pointer('down', 600, 540);
    engine.pointer('move', 744, 540); await settle();
    // The key at picture 0 has the new value; the setting without keys is set by a patch of the node.
    expect(sent(bridge)).toEqual([[
      { op: 'set_fusion_keys', ...AT, path: 'n6.center.x', param: { keys: [k(0, 0.35), k(50, 0.75)] } }, setNode('n6', { center: { y: 0.5 } }),
    ]]);
    engine.pointer('up', 744, 540); await settle();
    // Between two keys, at picture 25, the centre is 0.55 of the width now: dragged a tenth of the width left, a key is put there.
    await engine.notify({ frame: 25, shown: 25 }); await settle();
    expect(overlays(bridge).at(-1).dots[0]).toEqual({ kind: 'current', at: [[1032, 540]] });
    bridge.call.mockClear();
    engine.pointer('down', 1032, 540);
    engine.pointer('move', 888, 540); await settle();
    expect(sent(bridge)).toEqual([[
      { op: 'set_fusion_keys', ...AT, path: 'n6.center.x', param: { keys: [k(0, 0.35), k(25, 0.45), k(50, 0.75)] } }, setNode('n6', { center: { y: 0.5 } }),
    ]]);
    engine.pointer('up', 888, 540); await settle();
  });

  it('is taken back by Escape, and moves nothing until the button is let go', async () => {
    const engine = await mount();
    const { bridge } = engine;
    await choose('n6');
    bridge.call.mockClear();
    engine.pointer('down', 960, 540);
    engine.pointer('move', 1104, 432); await settle();
    expect(applied(bridge)).toHaveLength(1);
    fireEvent.keyDown(document.body, { key: 'Escape' }); await settle();
    // The two settings as they were at the press, which ends the gesture.
    expect(applied(bridge)).toHaveLength(2);
    expect(applied(bridge)[1].ops).toEqual([setNode('n6', { center: { x: 0.5 } }), setNode('n6', { center: { y: 0.5 } })]);
    expect(applied(bridge)[1].gesture).toEqual({ id: applied(bridge)[0].gesture!.id, end: true });
    expect(overlays(bridge).at(-1)).toEqual(RECTANGLE);
    engine.pointer('move', 1300, 300); engine.pointer('up', 1300, 300); await settle();
    expect(applied(bridge)).toHaveLength(2);
  });

  it('is not begun on a locked track: the controls are drawn, and no handle is taken hold of', async () => {
    const engine = await mount(projectOf([timelineOf(comp(), true)]));
    const { bridge } = engine;
    await choose('n6');
    expect(overlays(bridge)).toEqual([RECTANGLE]);
    expect(pointers(bridge)).toEqual([TAKEN]);
    engine.pointer('down', 960, 540);
    engine.pointer('move', 1104, 432); engine.pointer('up', 1104, 432); await settle();
    expect(applied(bridge)).toEqual([]);
    // No handle is drawn as dragged.
    expect(overlays(bridge)).toEqual([RECTANGLE]);
    // Unlocked, the same drag moves the centre.
    await engine.outside([{ op: 'set_track', timeline: 't1', id: 'v1', locked: false }]); await settle();
    engine.pointer('down', 960, 540);
    engine.pointer('move', 1104, 432); await settle();
    engine.pointer('up', 1104, 432); await settle();
    expect(sent(bridge)[0]).toEqual([setNode('n6', { center: { x: 0.6 } }), setNode('n6', { center: { y: 0.6 } })]);
  });
});
