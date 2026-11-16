// @vitest-environment jsdom
/**
 * The Fusion page (DaVinci Resolve Reference Manual 20, Ch. 64, 67 and 70): which clip it takes and
 * what it says where there is none; the composition that a clip without one is shown with, which
 * goes into the project with the first change; each gesture of the node editor and the operations
 * that it sends; the Inspector's fields of each kind of node, their keys and their gestures; the
 * list of the Spline Editor; what is said of a refusal; the layout; and its place among the pages.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { en } from '../src/messages/en';
import { numberedClips, shownClips } from '../src/studio/color/clips';
import { Studio } from '../src/studio/Studio';
import {
  applyOp, emptyGrade, firstComposition, ProjectCopy, type Composition, type EngineEvent, type FusionLink, type FusionNode, type Media, type Op, type Project,
  type Timeline, type TransportState,
} from '../src/studio/engine';
import { newMerge, newNode } from '../src/studio/fusion/edits';
import { chosenPicture, fusionClip } from '../src/studio/pages/FusionPage';
import { likeABrowser } from './mounted';
import { blockOf } from './stylesheet';
import { clip, media } from './timeline-fixture';

let frames: FrameRequestCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser(); frames = [];
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // Every part of the page lies at the top left, so a point of the graph is a point of the window at zoom 1.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect(0, 0, 600, 300));
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; delete (window as { columbina?: unknown }).columbina; });

const AT = { timeline: 't1', clip: 'c1' };
const link = (from: string, to: string, input: FusionLink['input'] = 'input'): FusionLink => ({ from, to, input });
/**
 * MediaIn1 through the Transform n3 into the background of the Merge n4, the Background n5 its foreground, and the Merge into MediaOut1.
 * Laid out by the editor, the cards (92 by 58) stand at: MediaIn1 28,28; n5 28,120; n3 176,28; n4 324,28; MediaOut1 472,28.
 */
const comp = (linked = true): Composition => ({
  nodes: [newNode('media_in', null, 'MediaIn1'), newNode('media_out', null, 'MediaOut1'), newNode('transform', null, 'n3'), newNode('merge', null, 'n4'),
    newNode('background', null, 'n5')],
  links: [link('MediaIn1', 'n3'), link('n3', 'n4', 'background'), ...(linked ? [link('n5', 'n4', 'foreground')] : []), link('n4', 'MediaOut1')],
});
/** The composition with keys on the Blend of the Merge, at pictures 10 and 30 of the media. */
const keyedComp = (): Composition => {
  const made = comp();
  const merged = { ...newNode('merge', null, 'n4'), params: { ...newMerge(), blend: { keys: [{ frame: 10, value: 0.25, interp: 'linear' }, { frame: 30, value: 0.75, interp: 'linear' }] } } } as FusionNode;
  return { ...made, nodes: made.nodes.map(node => (node.id === 'n4' ? merged : node)) };
};
/** Where the ports and the middles of the cards of `comp` are, in the graph. */
const P = {
  in1: { x: 74, y: 57 }, n3: { x: 222, y: 57 }, n4: { x: 370, y: 57 }, n5: { x: 74, y: 149 }, out1: { x: 518, y: 57 },
  n3In: { x: 176, y: 57 }, n4Out: { x: 416, y: 57 }, n4Foreground: { x: 324, y: 28 + (58 * 2) / 3 }, n5Out: { x: 120, y: 149 }, out1In: { x: 472, y: 57 },
};
const m2: Media = { ...media, id: 'm2', name: 'Street' };
/** V1: c1 (frames 0–99, with a composition where one is given) and c2 (100–199); V2 above it: c3 (50–149); and, where asked for, A1 with a clip of sound. */
const timelineOf = (first: Composition | null = comp(), sound = false): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [clip('c1', 0, 100, first ? { fusion: first } : {}), clip('c2', 100, 100)] },
    { id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, transitions: [],
      clips: [clip('c3', 50, 100, { source: { kind: 'media', media: 'm2', in: { ticks: 0, scale: 25 } } })] },
    ...(sound ? [{ id: 'a1', kind: 'audio' as const, name: 'A1', enabled: true, locked: false, transitions: [], clips: [clip('s1', 0, 100)] }] : []),
  ],
});
const projectOf = (timelines: Timeline[] = [timelineOf()]): Project => ({ schema: 5, next_id: 10, bins: [], groups: [], media: [media, m2], timelines });

/**
 * An engine that keeps a project, applies the operations a transaction sends as the copy does, gives a node added without an id
 * the next one as the real engine does, and refuses what a test tells it to, as the real one refuses.
 */
function fakeEngine(given: Project) {
  let project = given;
  let revision = 0;
  let refusal: Record<string, unknown> | null = null;
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
      if (method === 'transaction.apply') {
        if (refusal) return { ok: false, error: { code: -32001, message: 'refused', data: refusal } };
        const done = (params!.ops as Op[]).map(named);
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
  return {
    bridge, copy: new ProjectCopy(bridge),
    project: () => project,
    refuse: (data: Record<string, unknown> | null) => { refusal = data; },
    /** Another client changes the project: the operations are applied and told, as made by an agent. */
    outside: (ops: Op[]) => act(() => {
      for (const op of ops) project = applyOp(project, op);
      revision++;
      tell('project.changed', { revision, made_by: 'agent', by: 'agent', next_id: project.next_id, done: ops });
    }),
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
  };
}
const tab = (name: string) => screen.getByRole('tab', { name });
const open = async (name: string) => { fireEvent.click(tab(name)); await settle(); await draw(); await settle(); };
/** The studio on its engine, on the Fusion page unless the test goes there itself. */
async function mount(project: Project = projectOf(), page: 'Fusion' | null = 'Fusion') {
  const engine = fakeEngine(project);
  render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
  await settle();
  if (page) await open(page);
  return engine;
}

interface Sent { ops: unknown[]; revision?: number; gesture?: { id: string; end?: boolean } }
const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
const applied = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'transaction.apply') as Sent[];
/** The operations of each transaction sent. */
const sent = (bridge: { call: ReturnType<typeof vi.fn> }) => applied(bridge).map(each => each.ops);
const one = <T extends Element = HTMLElement>(selector: string) => document.querySelector<T>(selector)!;
const marked = (name: string) => document.querySelector<HTMLElement>(`[data-test="${name}"]`);
const graph = () => one<SVGElement>('[data-test="fusion-graph"]');
const canvas = () => one('[data-test="node-canvas"]');
const nodeOf = (id: string) => graph().querySelector<SVGElement>(`[data-node="${id}"]`)!;
const card = (id: string) => nodeOf(id).querySelector<SVGElement>('.color-node-card')!;
const port = (id: string, name: string) => nodeOf(id).querySelector<SVGElement>(`[data-port="${name}"]`)!;
const placeOf = (id: string) => [Number(card(id).getAttribute('x')), Number(card(id).getAttribute('y'))];
const nodesDrawn = () => [...graph().querySelectorAll<SVGElement>('[data-node]')].map(each => each.dataset.node);
const linksDrawn = () => [...graph().querySelectorAll<SVGElement>('[data-link]')].map(each => each.dataset.link);
const half = (name: string, which: 'start' | 'end') => graph().querySelector<SVGElement>(`[data-link-hit="${name}"][data-half="${which}"]`)!;
const currentNode = () => graph().querySelector<SVGElement>('[data-current]')?.dataset.node ?? null;
const chosenNodes = () => [...graph().querySelectorAll<SVGElement>('[data-chosen]')].map(each => each.dataset.node);
const problem = () => marked('node-problem')?.textContent ?? null;
const clipShown = () => marked('fusion-clip-name')!;
const key = (value: string, fields: Record<string, unknown> = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
type At = { x: number; y: number };
/** A press and a release in the node editor, on a node or anywhere in it. */
const press = (element: Element, at: At, fields: Record<string, unknown> = {}) => {
  fireEvent.pointerDown(element, { button: 0, clientX: at.x, clientY: at.y, ...fields });
  fireEvent.pointerUp(canvas(), { button: 0, clientX: at.x, clientY: at.y, ...fields });
};
/** A drag in the node editor, from an element at a point to another point, by way of the point between them. */
const dragFrom = (element: Element, from: At, to: At, fields: Record<string, unknown> = {}) => {
  fireEvent.pointerDown(element, { button: 0, clientX: from.x, clientY: from.y, ...fields });
  fireEvent.pointerMove(canvas(), { clientX: (from.x + to.x) / 2, clientY: (from.y + to.y) / 2, ...fields });
  fireEvent.pointerMove(canvas(), { clientX: to.x, clientY: to.y, ...fields });
  fireEvent.pointerUp(canvas(), { clientX: to.x, clientY: to.y, ...fields });
};
const choose = (id: keyof typeof P & ('in1' | 'n3' | 'n4' | 'n5' | 'out1')) => {
  const node = { in1: 'MediaIn1', n3: 'n3', n4: 'n4', n5: 'n5', out1: 'MediaOut1' }[id];
  press(card(node), P[id]);
};
const inspector = () => marked('fusion-inspector-body')!;
const fieldsShown = () => [...inspector().querySelectorAll<HTMLElement>('[data-field]')].map(each => each.dataset.field);
const row = (name: string) => inspector().querySelector<HTMLElement>(`[data-field="${name}"]`)!;
const box = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="text"]')!;
const slider = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="range"]')!;
const type = (name: string, value: string) => { fireEvent.change(box(name), { target: { value } }); fireEvent.keyDown(box(name), { key: 'Enter' }); };
const keysOf = (name: string) => [...inspector().querySelectorAll<HTMLButtonElement>(`[data-keys="${name}"] button`)];

describe('the clip of the Fusion page', () => {
  it('is the clip at the playhead on the topmost enabled video track, named in the bar', async () => {
    const { notify, outside } = await mount();
    expect(marked('fusion-page')!.className).toBe('studio-body fusion-page');
    // At frame 0 only V1 has a clip.
    expect([clipShown().dataset.clip, clipShown().textContent]).toEqual(['c1', '01 · Interview']);
    // At frame 60 the clip of V2 lies over that of V1: the one on the highest track is the one seen.
    await notify({ frame: 60, shown: 60 });
    expect([clipShown().dataset.clip, clipShown().textContent]).toEqual(['c3', '02 · Street']);
    // A track that is disabled shows nothing: the clip under it is taken.
    await outside([{ op: 'set_track', timeline: 't1', id: 'v2', enabled: false }]);
    expect(clipShown().dataset.clip).toBe('c1');
  });

  it('goes to the clip before and after with the buttons of the bar, the playhead with it', async () => {
    const { bridge } = await mount();
    const [before, after] = [marked('fusion-previous-clip') as HTMLButtonElement, marked('fusion-next-clip') as HTMLButtonElement];
    expect([before.disabled, after.disabled]).toEqual([true, false]);
    fireEvent.click(after); await settle();
    // The clips are in the order that they begin in, whatever their track: c3 begins at frame 50.
    expect(clipShown().dataset.clip).toBe('c3');
    expect(calls(bridge, 'transport.seek').at(-1)).toEqual({ monitor: 'timeline', frame: 50, scrubbing: false });
    fireEvent.click(after); await settle();
    // At frame 100 c3 still lies over c2: the clip gone to is the one shown.
    expect(clipShown().dataset.clip).toBe('c2');
    expect(calls(bridge, 'transport.seek').at(-1)).toEqual({ monitor: 'timeline', frame: 100, scrubbing: false });
    expect(after.disabled).toBe(true);
    fireEvent.click(before); await settle();
    expect(clipShown().dataset.clip).toBe('c3');
  });

  it('is the one clip of picture chosen on the Edit page, until the playhead moves off it', async () => {
    const { notify } = await mount(projectOf(), null);
    await open('Edit');
    const picture = one('[data-clip="c3"]');
    fireEvent.pointerDown(picture, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    fireEvent.pointerUp(picture, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    await settle();
    await open('Fusion');
    // The playhead stands at frame 0, where c1 is; the clip chosen is shown all the same.
    expect(clipShown().dataset.clip).toBe('c3');
    await notify({ frame: 10, shown: 10 });
    expect(clipShown().dataset.clip).toBe('c1');
  });

  it('is worked out from the clips shown, the playhead and what was pinned', () => {
    const shown = shownClips(numberedClips(timelineOf(comp(), true)), new Set());
    const id = (found: { clip: { id: string } } | null) => found?.clip.id ?? null;
    // One clip of picture among those chosen is the one; none, several, or sound alone is none.
    expect(chosenPicture(shown, ['c2'])).toBe('c2');
    expect(chosenPicture(shown, ['c2', 's1'])).toBe('c2');
    expect(chosenPicture(shown, ['c1', 'c2'])).toBeNull();
    expect(chosenPicture(shown, ['s1'])).toBeNull();
    expect(chosenPicture(shown, [])).toBeNull();
    // Nothing pinned: the clip at the playhead, the one on the highest track where two lie there, and none in a gap.
    expect(id(fusionClip(shown, 0, null, null))).toBe('c1');
    expect(id(fusionClip(shown, 60, null, null))).toBe('c3');
    expect(id(fusionClip(shown, 250, null, null))).toBeNull();
    expect(id(fusionClip(shown, null, null, null))).toBeNull();
    // A clip pinned is shown where the playhead is on it, though another lies over it, and while the playhead stands where it stood.
    expect(id(fusionClip(shown, 120, 'c2', Number.NaN))).toBe('c2');
    expect(id(fusionClip(shown, 0, 'c2', 0))).toBe('c2');
    expect(id(fusionClip(shown, null, 'c2', null))).toBe('c2');
    // Once the playhead has moved off it, the clip at the playhead is shown; a clip that is gone is pinned no longer.
    expect(id(fusionClip(shown, 10, 'c2', Number.NaN))).toBe('c1');
    expect(id(fusionClip(shown, 0, 'gone', 0))).toBe('c1');
  });

  it('says so where there is no clip of picture at the playhead, and where there is no timeline', async () => {
    const { notify } = await mount();
    await notify({ frame: 250, shown: 250 });
    expect(marked('fusion-empty')!.textContent).toBe(en.hint.studio.fusionPage.noClip);
    expect(marked('fusion-inspector-empty')!.textContent).toBe(en.hint.studio.fusionPage.noClip);
    expect(clipShown().textContent).toBe('No clip');
    expect(marked('fusion-graph')).toBeNull();
    expect((marked('toggle-spline') as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    await mount(projectOf([]));
    expect(marked('fusion-empty')!.textContent).toBe(en.hint.studio.cutPage.noTimeline);
  });

  it('says that a clip of sound has no composition where only sound was chosen on the Edit page', async () => {
    // The picture begins at frame 100: at the playhead, frame 0, there is the clip of sound alone.
    const late: Timeline = { ...timelineOf(), tracks: [
      { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [clip('c2', 100, 100)] },
      { id: 'a1', kind: 'audio', name: 'A1', enabled: true, locked: false, transitions: [], clips: [clip('s1', 0, 100)] },
    ] };
    await mount(projectOf([late]), null);
    await open('Edit');
    const sound = one('[data-clip="s1"]');
    fireEvent.pointerDown(sound, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    fireEvent.pointerUp(sound, { button: 0, pointerId: 1, clientX: 400, clientY: 120 });
    await settle();
    await open('Fusion');
    expect(marked('fusion-empty')!.textContent).toBe(en.hint.studio.fusionPage.soundClip);
  });
});

describe('the composition that the page shows', () => {
  it('is the first one for a clip that has none, drawn without touching the project, and sent with the first change', async () => {
    const { bridge, notify, project } = await mount();
    bridge.call.mockClear();
    await notify({ frame: 160, shown: 160 });
    expect(clipShown().dataset.clip).toBe('c2');
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1']);
    expect(linksDrawn()).toEqual(['MediaIn1>MediaOut1:input']);
    expect(marked('fusion-first')!.textContent).toBe(en.hint.studio.fusionPage.first);
    // Looking at the clip, choosing a node and opening the Spline Editor change nothing.
    press(card('MediaIn1'), { x: 74, y: 57 });
    fireEvent.click(marked('toggle-spline')!);
    await settle();
    expect(sent(bridge)).toEqual([]);
    expect((project().timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[1].fusion).toBeUndefined();
    // The first change, here Pass Through: the first composition and then the change, in one transaction.
    key('p', { code: 'KeyP', ctrlKey: true }); await settle();
    const at = { timeline: 't1', clip: 'c2' };
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion', ...at, value: firstComposition() }, { op: 'set_fusion_node', ...at, id: 'MediaIn1', pass_through: true }]]);
    expect(marked('fusion-first')).toBeNull();
    expect(nodeOf('MediaIn1').dataset.passThrough).toBe('true');
    // From then on a change is sent alone.
    key('p', { code: 'KeyP', ctrlKey: true }); await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'set_fusion_node', ...at, id: 'MediaIn1', pass_through: false }]);
  });

  it('is reset to the first one, and taken away, from the menu of the background', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(canvas(), { clientX: 300, clientY: 250 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reset Fusion Composition' }));
    await settle();
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1']);
    // Reset, the composition is the first one: there is nothing more to reset, and it can still be taken away.
    fireEvent.click(marked('fusion-graph-menu')!);
    expect((screen.getByRole('menuitem', { name: 'Reset Fusion Composition' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Remove Composition' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion', ...AT, value: firstComposition() }], [{ op: 'set_fusion', ...AT, value: null }]]);
    // Without one the clip is shown with the first composition again, and has none to take away.
    expect(marked('fusion-first')).not.toBeNull();
    fireEvent.click(marked('fusion-graph-menu')!);
    expect((screen.getByRole('menuitem', { name: 'Remove Composition' }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('the node editor of the Fusion page', () => {
  it('draws each node as a card of its kind with its name, and a Merge with its two inputs', async () => {
    await mount();
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5']);
    expect([...graph().querySelectorAll<SVGElement>('[data-node]')].map(each => each.dataset.kind)).toEqual(['media_in', 'media_out', 'transform', 'merge', 'background']);
    expect([...graph().querySelectorAll('[data-test="node-label"]')].map(each => each.textContent)).toEqual(['MediaIn 1', 'MediaOut 1', 'Transform 1', 'Merge 1', 'Background 1']);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
    expect([placeOf('MediaIn1'), placeOf('n5'), placeOf('n3'), placeOf('n4'), placeOf('MediaOut1')]).toEqual([[28, 28], [28, 120], [176, 28], [324, 28], [472, 28]]);
    // The ports: a Merge's background and foreground on its left and its output on its right; a MediaOut has no output, a MediaIn no input.
    const ports = (id: string) => [...nodeOf(id).querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
    expect([ports('n4'), ports('MediaOut1'), ports('MediaIn1')]).toEqual([['in:background', 'in:foreground', 'out'], ['in:input'], ['out']]);
  });

  it('adds a node from the tool bar with no links where none is chosen, and the new node is the one chosen', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.click(marked('add-merge')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('merge') }]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n10']);
    expect(currentNode()).toBe('n10');
  });

  it('inserts a node after the one chosen, with the links that put it between that node and what it fed', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    fireEvent.click(marked('add-transform')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('transform'), links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'n3>n10:input', 'MediaIn1>n3:input', 'n10>n4:background', 'n5>n4:foreground']);
    expect(currentNode()).toBe('n10');
  });

  it('adds a node from the right-click menu where the menu was opened', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(canvas(), { clientX: 300, clientY: 250 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Background' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('background', { x: 300, y: 250 }) }]]);
    expect(placeOf('n10')).toEqual([300, 250]);
    // From the menu of a node, the new node goes in after that node, and lies where the menu was opened.
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n4'), { clientX: 380, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Transform' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('transform', { x: 380, y: 60 }), links: [link('n4', '', 'input'), link('', 'MediaOut1', 'input')] }]]);
  });

  it('moves a node as one gesture: each move sends its place whole, and the release ends it', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    dragFrom(card('n3'), P.n3, { x: P.n3.x + 40, y: P.n3.y + 40 });
    await settle();
    const moves = applied(bridge);
    expect(moves.map(each => each.ops)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 196, y: 48 } }],
      [{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 216, y: 68 } }],
    ]);
    // One gesture, so one step to undo: the same id all through, ended by the last.
    expect(new Set(moves.map(each => each.gesture?.id)).size).toBe(1);
    expect(moves[0].gesture!.id).not.toBe('');
    expect(moves.map(each => each.gesture!.end)).toEqual([undefined, true]);
    expect(placeOf('n3')).toEqual([216, 68]);
    expect(placeOf('n4')).toEqual([324, 28]);
  });

  it('moves every node chosen with the one dragged, and chooses nodes with Ctrl+click, Shift+click and a box', async () => {
    const { bridge } = await mount();
    choose('n3');
    press(card('n4'), P.n4, { ctrlKey: true });
    expect([currentNode(), chosenNodes()]).toEqual(['n3', ['n4']]);
    press(card('n5'), P.n5, { shiftKey: true });
    expect(chosenNodes()).toEqual(['n4', 'n5']);
    press(card('n5'), P.n5, { ctrlKey: true });
    expect(chosenNodes()).toEqual(['n4']);
    bridge.call.mockClear();
    dragFrom(card('n4'), P.n4, { x: P.n4.x + 10, y: P.n4.y + 20 });
    await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 186, y: 48 } }, { op: 'set_fusion_node', ...AT, id: 'n4', pos: { x: 334, y: 48 } }]);
    // A press on nothing chooses none; a box drawn round nodes chooses them.
    press(canvas(), { x: 590, y: 290 });
    expect([currentNode(), chosenNodes()]).toEqual([null, []]);
    dragFrom(canvas(), { x: 10, y: 100 }, { x: 130, y: 190 });
    expect(currentNode()).toBe('n5');
    // Ctrl+A chooses every node.
    key('a', { code: 'KeyA', ctrlKey: true });
    expect([currentNode(), chosenNodes()]).toEqual(['n5', ['MediaIn1', 'MediaOut1', 'n3', 'n4']]);
  });

  it('links an output to an input by a drag between them, and a link into an input that has one takes its place', async () => {
    const { bridge } = await mount(projectOf([timelineOf(comp(false))]));
    bridge.call.mockClear();
    // The Background, which nothing is linked from, into the foreground of the Merge.
    dragFrom(port('n5', 'out'), P.n5Out, P.n4Foreground);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n5', 'n4', 'foreground') }]]);
    expect(linksDrawn()).toContain('n5>n4:foreground');
    // Into the input of the MediaOut, which the Merge feeds: one connect_fusion, and the engine puts the old link out of its place.
    bridge.call.mockClear();
    dragFrom(port('n5', 'out'), P.n5Out, P.out1In);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n5', 'MediaOut1') }]]);
    expect(linksDrawn()).toEqual(['n5>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
    // The link that is there already is not sent again. Fed by the Background, the MediaOut is laid out one column right of it, at 176,28.
    expect(placeOf('MediaOut1')).toEqual([176, 28]);
    bridge.call.mockClear();
    dragFrom(port('n5', 'out'), P.n5Out, { x: 176, y: 57 });
    await settle();
    expect(sent(bridge)).toEqual([]);
  });

  it('unlinks by dragging the end of a link off, and by Delete or Backspace on a link chosen', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    // The second half of the link from the Merge to the MediaOut takes its end along: let go on nothing, the link is gone.
    dragFrom(half('n4>MediaOut1:input', 'end'), { x: 450, y: 57 }, { x: 450, y: 250 });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'MediaOut1', input: 'input' }]]);
    expect(linksDrawn()).not.toContain('n4>MediaOut1:input');
    // A link clicked is chosen, and Delete takes it away; Backspace does too.
    bridge.call.mockClear();
    press(half('n5>n4:foreground', 'start'), { x: 200, y: 110 });
    expect(graph().querySelector('[data-link="n5>n4:foreground"]')!.getAttribute('data-chosen')).toBe('true');
    key('Delete'); await settle();
    press(half('MediaIn1>n3:input', 'start'), { x: 148, y: 57 });
    key('Backspace'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'foreground' }], [{ op: 'disconnect_fusion', ...AT, to: 'n3', input: 'input' }]]);
  });

  it('takes the end of a link to another input: the old link goes and the new one comes, in one transaction', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    // A press on an input that has a link takes up the end of that link, here the one from the Transform into the background of the Merge.
    dragFrom(port('n4', 'in:background'), { x: 324, y: 28 + 58 / 3 }, P.out1In);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'background' }, { op: 'connect_fusion', ...AT, link: link('n3', 'MediaOut1') }]]);
    // The Merge, fed by the Background alone now, is laid out under the Transform, at 176,120: its foreground is at 176 and two thirds down its side.
    expect(placeOf('n4')).toEqual([176, 120]);
    const foreground = { x: 176, y: 120 + (58 * 2) / 3 };
    // The end of a link taken up and let go on the input that it came from: nothing is sent.
    bridge.call.mockClear();
    fireEvent.pointerDown(port('n4', 'in:foreground'), { button: 0, clientX: foreground.x, clientY: foreground.y });
    fireEvent.pointerMove(canvas(), { clientX: foreground.x - 40, clientY: foreground.y });
    expect(marked('wire')).not.toBeNull();
    fireEvent.pointerMove(canvas(), { clientX: foreground.x, clientY: foreground.y });
    fireEvent.pointerUp(canvas(), { clientX: foreground.x, clientY: foreground.y });
    await settle();
    expect(sent(bridge)).toEqual([]);
    expect(marked('wire')).toBeNull();
    // Let go on nothing, the link is taken away.
    dragFrom(port('n4', 'in:foreground'), foreground, { x: foreground.x - 60, y: foreground.y + 60 });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'foreground' }]]);
  });

  it('deletes the nodes chosen with Delete, joining what fed each to what it fed, and never the MediaOut', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    // Backspace deletes no node.
    key('Backspace'); await settle();
    expect(sent(bridge)).toEqual([]);
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4', 'background')] }]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n4', 'n5']);
    expect(linksDrawn()).toContain('MediaIn1>n4:background');
    // The MediaOut stays: Delete sends nothing for it, and its menu has Delete Node off.
    bridge.call.mockClear();
    choose('out1');
    expect(currentNode()).toBe('MediaOut1');
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([]);
    fireEvent.contextMenu(card('MediaOut1'), { clientX: P.out1.x, clientY: P.out1.y });
    expect((screen.getByRole('menuitem', { name: 'Delete Node' }) as HTMLButtonElement).disabled).toBe(true);
    // Chosen with another node, it is passed over. (The press puts the menu away.)
    choose('n5');
    press(card('MediaOut1'), P.out1, { ctrlKey: true });
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'remove_fusion_node', ...AT, id: 'n5' }]]);
  });

  it('passes the nodes chosen through with Ctrl+P, drawn dimmed, and renames one with F2', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    key('p', { code: 'KeyP', ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n3', pass_through: true }]]);
    expect(nodeOf('n3').dataset.passThrough).toBe('true');
    expect(nodeOf('n4').dataset.passThrough).toBeUndefined();
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    expect(blockOf(css, '.fusion-page .fusion-node[data-pass-through]')).toContain('opacity: 0.45;');
    // F2 opens the name of the node to be written in; Enter sends it.
    bridge.call.mockClear();
    key('F2');
    const field = screen.getByRole('textbox', { name: 'Node label' }) as HTMLInputElement;
    expect(field.value).toBe('');
    fireEvent.change(field, { target: { value: ' Shake ' } });
    // A key written into the field is no key of the editor.
    fireEvent.keyDown(field, { key: 'Delete' });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n3', label: 'Shake' }]]);
    expect(nodeOf('n3').querySelector('[data-test="node-label"]')!.textContent).toBe('Shake');
    expect(screen.queryByRole('textbox', { name: 'Node label' })).toBeNull();
  });

  it('keeps its keys to itself while another part of the page has the focus, and undoes in the one history', async () => {
    const { bridge } = await mount();
    choose('n3');
    // A press in the Inspector takes the focus from the node editor: Delete and Ctrl+P are not the editor's then.
    fireEvent.pointerDown(marked('fusion-inspector')!);
    bridge.call.mockClear();
    key('Delete'); key('p', { code: 'KeyP', ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([]);
    // Ctrl+Z and Ctrl+Shift+Z go to the history of the engine wherever the focus is.
    key('z', { code: 'KeyZ', ctrlKey: true }); await settle();
    key('z', { code: 'KeyZ', ctrlKey: true, shiftKey: true }); await settle();
    expect([calls(bridge, 'history.undo'), calls(bridge, 'history.redo')].map(each => each.length)).toEqual([1, 1]);
    // The keys of the transport stay the transport's: Space plays.
    key(' '); await settle();
    expect(calls(bridge, 'transport.play')).toHaveLength(1);
  });

  it('says a refusal of the engine above the graph, and says nothing of a change that changes nothing', async () => {
    const { bridge, refuse } = await mount();
    // A link from the Merge back into the Transform that feeds it would close a loop.
    refuse({ reason: 'loop' });
    dragFrom(port('n4', 'out'), P.n4Out, P.n3In);
    await settle();
    expect(sent(bridge).at(-1)).toEqual([{ op: 'connect_fusion', ...AT, link: link('n4', 'n3') }]);
    expect(problem()).toBe(en.hint.studio.edit.loop);
    fireEvent.click(marked('node-problem')!.querySelector('button')!);
    expect(problem()).toBeNull();
    // The refusals of compositions have sentences of their own.
    refuse({ reason: 'one_media_out', count: 2 });
    fireEvent.click(marked('add-merge')!); await settle();
    expect(problem()).toBe(en.hint.studio.edit.one_media_out);
    fireEvent.click(marked('node-problem')!.querySelector('button')!);
    // A node dragged back to where it was: the engine refuses the transaction as empty, which is no failure.
    refuse({ reason: 'empty' });
    dragFrom(card('n3'), P.n3, { x: P.n3.x + 30, y: P.n3.y });
    await settle();
    expect(problem()).toBeNull();
    expect(placeOf('n3')).toEqual([176, 28]);
  });

  it('shows a badge on a node whose settings have keys', async () => {
    await mount(projectOf([timelineOf(keyedComp())]));
    expect(nodeOf('n4').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    expect(nodeOf('n3').querySelector('[data-test="node-keyed"]')).toBeNull();
  });
});

describe('the Inspector of the Fusion page', () => {
  it('shows the settings of the one node chosen, by its kind, and says so where none or several are chosen', async () => {
    await mount();
    expect(marked('fusion-no-node')!.textContent).toBe(en.hint.studio.fusionPage.chooseNode);
    choose('n4');
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n4', 'merge', 'Merge 1']);
    expect(fieldsShown()).toEqual(['label', 'apply', 'operator', 'subtractive_additive', 'alpha_gain', 'burn_in', 'blend', 'center.x', 'center.y', 'size', 'angle']);
    // Apply Mode has the 27 composite modes of the Edit page; Operator is Over alone, and cannot be changed.
    const apply = row('apply').querySelector('select')!;
    expect([apply.value, apply.options.length]).toEqual(['normal', 27]);
    const operator = row('operator').querySelector('select')!;
    expect([operator.disabled, [...operator.options].map(each => each.textContent)]).toEqual([true, ['Over']]);
    // Every number shows its default, and has the three buttons of keys beside it.
    expect(['subtractive_additive', 'alpha_gain', 'burn_in', 'blend', 'center.x', 'center.y', 'size', 'angle'].map(name => box(name).value))
      .toEqual(['1.000', '1.000', '0.000', '1.000', '0.500', '0.500', '1.000', '0.0']);
    expect([...inspector().querySelectorAll<HTMLElement>('[data-keys]')].map(each => each.dataset.keys))
      .toEqual(['subtractive_additive', 'alpha_gain', 'burn_in', 'blend', 'center.x', 'center.y', 'size', 'angle']);
    expect(keysOf('blend').map(each => each.title)).toEqual(['Previous keyframe', 'Add keyframe', 'Next keyframe']);
    choose('n3');
    expect(fieldsShown()).toEqual(['label', 'center.x', 'center.y', 'size', 'angle', 'flip_horizontal', 'flip_vertical']);
    choose('n5');
    expect(fieldsShown()).toEqual(['label', 'color', 'color.r', 'color.g', 'color.b', 'color.a']);
    expect(['color.r', 'color.g', 'color.b', 'color.a'].map(name => box(name).value)).toEqual(['0.000', '0.000', '0.000', '1.000']);
    // A MediaIn and a MediaOut have nothing but the name.
    choose('in1');
    expect(fieldsShown()).toEqual(['label']);
    expect(marked('fusion-no-settings')!.textContent).toBe(en.hint.studio.fusionPage.noSettings);
    choose('out1');
    expect(fieldsShown()).toEqual(['label']);
    // Several chosen: no one node to show.
    press(card('n3'), P.n3, { ctrlKey: true });
    expect(marked('fusion-no-node')).not.toBeNull();
  });

  it('sets a number typed within the range of the catalog, puts the default back on a double click, and sets the switches and the modes', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    type('center.x', '0.25'); await settle();
    // Beyond the range, the value is the end of the range: 16.5 for a place.
    type('center.x', '99'); await settle();
    fireEvent.doubleClick(row('center.x').querySelector('label')!); await settle();
    type('angle', '45'); await settle();
    fireEvent.click(row('flip_horizontal').querySelector('input')!); await settle();
    const node = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n3', params }];
    expect(sent(bridge)).toEqual([
      node({ center: { x: 0.25 } }), node({ center: { x: 16.5 } }), node({ center: { x: 0.5 } }), node({ angle: 45 }), node({ flip_horizontal: true }),
    ]);
    expect((row('flip_horizontal').querySelector('input') as HTMLInputElement).checked).toBe(true);
    // The Apply Mode of a Merge.
    choose('n4');
    bridge.call.mockClear();
    fireEvent.change(row('apply').querySelector('select')!, { target: { value: 'screen' } }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n4', params: { apply: 'screen' } }]]);
    // The colour of a Background as one: its red, green and blue that change, in one transaction.
    choose('n5');
    bridge.call.mockClear();
    fireEvent.change(row('color').querySelector('input')!, { target: { value: '#ff8000' } }); await settle();
    expect(sent(bridge)).toEqual([[
      { op: 'set_fusion_node', ...AT, id: 'n5', params: { color: { r: 1 } } }, { op: 'set_fusion_node', ...AT, id: 'n5', params: { color: { g: 0.502 } } },
    ]]);
    // The name of the node.
    bridge.call.mockClear();
    const name = row('label').querySelector('input')!;
    fireEvent.change(name, { target: { value: 'Sky' } }); fireEvent.keyDown(name, { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n5', label: 'Sky' }]]);
  });

  it('sends a slider dragged as one gesture, each move the setting whole', async () => {
    const { bridge } = await mount();
    choose('n4');
    bridge.call.mockClear();
    const blend = slider('blend');
    fireEvent.pointerDown(blend);
    fireEvent.input(blend, { target: { value: '0.5' } }); await settle();
    fireEvent.input(blend, { target: { value: '0.4' } }); await settle();
    fireEvent.change(blend, { target: { value: '0.4' } }); await settle();
    const moves = applied(bridge);
    const set = (value: number) => [{ op: 'set_fusion_node', ...AT, id: 'n4', params: { blend: value } }];
    // The release says the last value once more, to end the gesture.
    expect(moves.map(each => each.ops)).toEqual([set(0.5), set(0.4), set(0.4)]);
    expect(new Set(moves.map(each => each.gesture?.id)).size).toBe(1);
    expect(moves.map(each => each.gesture!.end)).toEqual([undefined, undefined, true]);
    expect(box('blend').value).toBe('0.400');
  });

  it('puts a key with the diamond, and with a value given at another frame of a setting that has keys', async () => {
    const { bridge, notify } = await mount();
    choose('n4');
    await notify({ frame: 10, shown: 10 });
    bridge.call.mockClear();
    const [, diamond] = keysOf('blend');
    expect([diamond.getAttribute('aria-pressed'), diamond.dataset.keyed]).toEqual(['false', undefined]);
    fireEvent.click(diamond); await settle();
    // The first key of a setting is put at 0 of its keys: the number becomes the keys. The clip shows picture 10 of its media at frame 10.
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 0, remove: 0, keys: [{ frame: 10, value: 1, interp: 'linear' }] }]]);
    expect([keysOf('blend')[1].getAttribute('aria-pressed'), keysOf('blend')[1].dataset.keyed]).toEqual(['true', 'true']);
    expect(nodeOf('n4').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    // At another frame the setting has keys but none there: a value typed puts one.
    await notify({ frame: 30, shown: 30 });
    expect(keysOf('blend')[1].getAttribute('aria-pressed')).toBe('false');
    bridge.call.mockClear();
    type('blend', '0.5'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 1, remove: 0, keys: [{ frame: 30, value: 0.5, interp: 'linear' }] }]]);
    // The arrows go to the key before and after; the diamond takes the key at the playhead away again.
    expect(keysOf('blend').map(each => each.disabled)).toEqual([false, false, true]);
    fireEvent.click(keysOf('blend')[0]); await settle();
    expect(calls(bridge, 'transport.seek').at(-1)).toEqual({ monitor: 'timeline', frame: 10, scrubbing: false });
    bridge.call.mockClear();
    fireEvent.click(keysOf('blend')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n4.blend', at: 0, remove: 1, keys: [] }]]);
    // A setting without keys stays one number wherever the playhead is.
    bridge.call.mockClear();
    type('size', '2'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n4', params: { size: 2 } }]]);
  });

  it('says a refusal of the engine at its top, and nothing of a slider let go where it was', async () => {
    const { refuse } = await mount();
    choose('n4');
    refuse({ reason: 'out_of_range' });
    type('size', '3'); await settle();
    expect(marked('fusion-inspector-problem')!.textContent).toContain(en.hint.studio.edit.out_of_range);
    fireEvent.click(marked('fusion-inspector-problem')!.querySelector('button')!);
    refuse({ reason: 'empty' });
    const blend = slider('blend');
    fireEvent.pointerDown(blend);
    fireEvent.input(blend, { target: { value: '0.9' } }); await settle();
    fireEvent.change(blend, { target: { value: '0.9' } }); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
  });
});

describe('the Spline Editor of the Fusion page', () => {
  const listed = () => [...document.querySelectorAll<HTMLElement>('[data-test="fusion-curves"] li[data-curve]')].map(each => each.dataset.curve);

  it('lists every setting of the node chosen, those with keys first; with none chosen, the settings of all nodes that have keys', async () => {
    await mount(projectOf([timelineOf(keyedComp())]));
    expect(marked('fusion-curves')).toBeNull();
    choose('n4');
    fireEvent.click(marked('toggle-spline')!);
    expect(marked('fusion-page')!.dataset.curves).toBe('true');
    expect(listed()).toEqual(['n4.blend', 'n4.subtractive_additive', 'n4.alpha_gain', 'n4.burn_in', 'n4.center.x', 'n4.center.y', 'n4.size', 'n4.angle']);
    expect(one('[data-test="fusion-curves"] li[data-curve="n4.blend"]').textContent).toContain('Blend');
    // The Transform has no keys: its settings are listed all the same, for a key to be put on one.
    choose('n3');
    expect(listed()).toEqual(['n3.center.x', 'n3.center.y', 'n3.size', 'n3.angle']);
    // None chosen: what has keys, of every node, each named with its node.
    press(canvas(), { x: 590, y: 290 });
    expect(listed()).toEqual(['n4.blend']);
    expect(one('[data-test="fusion-curves"] li[data-curve="n4.blend"]').textContent).toContain('Merge 1 · Blend');
    fireEvent.click(marked('close-spline')!);
    expect(marked('fusion-curves')).toBeNull();
  });

  it('says so where no setting has keys and no node is chosen', async () => {
    await mount();
    fireEvent.click(marked('toggle-spline')!);
    expect(marked('fusion-curves-empty')!.textContent).toBe(en.hint.studio.fusionPage.noCurves);
  });
});

describe('the Fusion page among the pages', () => {
  it('lies between Edit and Color in the bar of pages, with an icon as the others have', async () => {
    await mount(projectOf(), null);
    const tabs = [...document.querySelectorAll<HTMLElement>('.studio-pages [role="tab"]')];
    expect(tabs.map(each => each.textContent)).toEqual(['Media', 'Cut', 'Edit', 'Fusion', 'Color', 'Fairlight']);
    expect(tabs.every(each => each.querySelector('svg') !== null)).toBe(true);
    // Nothing of the page is mounted while another page is shown.
    expect(marked('fusion-page')).toBeNull();
    await open('Fusion');
    expect(marked('fusion-page')).not.toBeNull();
    await open('Media');
    expect(marked('fusion-page')).toBeNull();
  });

  it('is a grid of the viewer above the node editor with the Inspector at the right, whichever stylesheet is loaded last', async () => {
    await mount();
    // The parts of the page, in the order that they are laid out in.
    expect([...marked('fusion-page')!.children].map(each => each.className)).toEqual(['fusion-bar', 'fusion-viewer', 'fusion-nodes', 'fusion-inspector']);
    expect(marked('fusion-viewer')!.querySelector('[data-monitor="timeline"] [data-test="transport"]')).not.toBeNull();
    // The DOM of the tests lays nothing out: the rules that lay the page out are read.
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    const studio = readFileSync('src/studio/Studio.css', 'utf8');
    const color = readFileSync('src/studio/color/color.css', 'utf8');
    // The page is the body of the studio too, which Studio.css makes a row of panes, and Studio.css is loaded after the
    // stylesheet of the page: the rule that makes the page a grid names both classes, and so wins whichever is loaded last.
    expect(blockOf(studio, '.studio-body')).toContain('display: flex;');
    expect(blockOf(css, '.studio-body.fusion-page')).toContain('display: grid;');
    expect(css).not.toMatch(/^\.fusion-page \{[^}]*display:/m);
    const page = blockOf(css, '.fusion-page').replace(/\s+/g, ' ');
    expect(page).toContain('"bar bar" "viewer inspector" "nodes inspector"');
    // The regions keep a least size: at a studio 1000px wide the Inspector has 300px and the viewer and the node editor the rest,
    // and the viewer and the node editor are 220px high at the least each, which with the bar fits a studio 700px high.
    const [, low, share, high] = /grid-template-columns: minmax\(0, 1fr\) clamp\((\d+)px, (\d+)cqi, (\d+)px\);/.exec(page)!.map(Number);
    const inspectorAt = (width: number) => Math.min(high, Math.max(low, (width * share) / 100));
    expect([inspectorAt(1000), 1000 - 1 - inspectorAt(1000)]).toEqual([300, 699]);
    expect(page).toContain('grid-template-rows: auto minmax(220px, 1fr) minmax(220px, 1fr);');
    // What does not fit is scrolled: the page, the graph of the node editor, and the Inspector.
    expect(page).toContain('overflow: auto;');
    expect(blockOf(css, '.fusion-inspector')).toContain('overflow: auto;');
    expect(blockOf(color, '.color-nodes-canvas')).toContain('overflow: auto;');
    // Narrower than the Inspector beside a viewer of 500px, the Inspector goes beneath.
    const stacked = Number(/@container studio \(max-width: (\d+)px\)/.exec(css)![1]);
    expect(stacked + 1 - inspectorAt(stacked + 1)).toBeGreaterThanOrEqual(500);
    expect(css.slice(css.indexOf('@container studio'))).toContain('grid-template-areas: "bar" "viewer" "nodes" "inspector";');
    // The colours of the kinds of nodes are variables of the themes, defined beside the others.
    const tokens = readFileSync('src/ui/tokens.css', 'utf8');
    for (const name of ['media-in', 'media-out', 'merge', 'transform', 'background', 'input-background', 'input-foreground']) {
      expect(blockOf(tokens, ':root')).toMatch(new RegExp(`--fusion-${name}: #[0-9a-f]{6};`));
      expect(css).toContain(`var(--fusion-${name})`);
    }
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });
});
