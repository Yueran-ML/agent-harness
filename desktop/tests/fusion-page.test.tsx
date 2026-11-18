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
import { zhCN } from '../src/messages/zh-CN';
import { numberedClips, shownClips } from '../src/studio/color/clips';
import { Studio } from '../src/studio/Studio';
import {
  applyOp, emptyGrade, firstComposition, ProjectCopy, type Composition, type EngineEvent, type FusionFollow, type FusionLink, type FusionNode, type FusionPoint,
  type FusionRectangle, type FusionTracker, type FusionTrackerPattern, type Media, type Op, type Project, type Timeline, type TransportState,
} from '../src/studio/engine';
import { copiedNodes, keepCopied } from '../src/studio/fusion/copied';
import { newMerge, newNode, newPattern, newPolygon, newTracker, startPoints, withFollow } from '../src/studio/fusion/edits';
import { NAME_ROOM } from '../src/studio/fusion/graph';
import { matchingTools, TOOLS } from '../src/studio/fusion/SelectTool';
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
  // What a test before copied is kept for the window, and so from one test to the next: each begins with nothing copied.
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
/** A Polygon of these points, the node `id`. */
const polygonOf = (points: FusionPoint[], id = 'n7'): FusionNode => ({ ...newNode('polygon', null, id), params: { ...newPolygon(), points } } as FusionNode);
/**
 * `comp` and three masks that nothing is linked to or from: the Rectangle n6, the Polygon n7 of the four points that the page
 * gives a new one, and the Ellipse n8. Laid out by the editor they stand in the first column under the Background n5,
 * at 28,212; 28,304 and 28,396.
 */
const maskedComp = (): Composition => {
  const made = comp();
  return { ...made, nodes: [...made.nodes, newNode('rectangle', null, 'n6'), polygonOf(startPoints()), newNode('ellipse', null, 'n8')] };
};
/**
 * Where the middles of the cards of the masks of `maskedComp` and their outputs are, and the mask inputs of the Transform and of
 * the Merge: on the lower edge of the card, a quarter of its width (23 of 92) from its left.
 */
const M = {
  n6: { x: 74, y: 241 }, n7: { x: 74, y: 333 }, n8: { x: 74, y: 425 }, n6Out: { x: 120, y: 241 }, n8Out: { x: 120, y: 425 },
  n3Mask: { x: 199, y: 86 }, n4Mask: { x: 347, y: 86 },
};
/**
 * `comp` and two effects that nothing is linked to or from: the Blur n6 and the Mosaic n7. Laid out by the editor they stand in the
 * first column under the Background n5, at 28,212 and 28,304, where the first two masks of `maskedComp` stand: `M.n6` and `M.n7`
 * are the middles of their cards.
 */
const effectComp = (): Composition => {
  const made = comp();
  return { ...made, nodes: [...made.nodes, newNode('blur', null, 'n6'), newNode('mosaic', null, 'n7')] };
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
 * the next one as the real engine does, and refuses what a test tells it to, as the real one refuses. A node added by a name of
 * the transaction (`$name`, PROTOCOL.md) is given an id as one added without, and the links of the operations after it that
 * name it are made to name that id, as the real engine makes them before it applies them: what is done carries real ids alone.
 */
function fakeEngine(given: Project) {
  let project = given;
  let revision = 0;
  let refusal: Record<string, unknown> | null = null;
  // What `track.start` answers: a job at once, as the real engine answers (the track itself is told by `news`).
  let trackAnswer: unknown = { ok: true, result: { job: 'track-1', first: 20, last: 119, frame_size: [256, 144], direction: 'forward', trackers: [0, 1] } };
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
  /** What gives the operations of one transaction their ids: the names that the transaction gave are kept from one operation to the next. */
  const naming = () => {
    const names = new Map<string, string>();
    const real = (end: string) => names.get(end) ?? end;
    return (op: Op): Op => {
      if (op.op === 'connect_fusion') return { ...op, link: { ...op.link, from: real(op.link.from), to: real(op.link.to) } };
      if (op.op !== 'add_fusion_node') return op;
      const given = op.node.id;
      if (given !== '' && !given.startsWith('$')) return op;
      const id = `n${project.next_id}`;
      project = { ...project, next_id: project.next_id + 1 };
      if (given !== '') names.set(given, id);
      const own = (end: string) => (end === '' || end === given ? id : real(end));
      return { ...op, node: { ...op.node, id } as FusionNode, ...(op.links ? { links: op.links.map(each => ({ ...each, from: own(each.from), to: own(each.to) })) } : {}) };
    };
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
        const done = (params!.ops as Op[]).map(naming());
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
    /** What `track.start` answers from now on. */
    answerTrack: (answer: unknown) => { trackAnswer = answer; },
    /** The engine tells the page a notification of its own, as of a track (`track.progress`, `track.state`). */
    news: (method: string, params: unknown) => act(() => { tell(method, params); }),
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
    // The playhead stood at frame 0, where c1 is; the clip chosen is shown all the same, and the playhead is taken to its first frame.
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
    // The ports: a Merge's background and foreground on its left, its effect mask on its lower edge and its output on its right;
    // a MediaOut has no output, a MediaIn no input, and neither has a mask.
    const ports = (id: string) => [...nodeOf(id).querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
    expect([ports('n4'), ports('MediaOut1'), ports('MediaIn1')]).toEqual([['in:background', 'in:foreground', 'in:mask', 'out'], ['in:input'], ['out']]);
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

describe('the masks in the node editor of the Fusion page', () => {
  const ports = (id: string) => [...nodeOf(id).querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
  const linkDrawn = (name: string) => graph().querySelector<SVGElement>(`[data-link="${name}"]`);

  it('shows the mask input of every kind that has one on the lower edge of its card, in the colour of masks, and none on a MediaIn or a MediaOut', async () => {
    await mount(projectOf([timelineOf(maskedComp())]));
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n6', 'n7', 'n8']);
    expect([placeOf('n6'), placeOf('n7'), placeOf('n8')]).toEqual([[28, 212], [28, 304], [28, 396]]);
    expect([...graph().querySelectorAll('[data-test="node-label"]')].map(each => each.textContent).slice(5)).toEqual(['Rectangle 1', 'Polygon 1', 'Ellipse 1']);
    expect([ports('n3'), ports('n4'), ports('n5'), ports('n6'), ports('n7'), ports('n8'), ports('MediaIn1'), ports('MediaOut1')]).toEqual([
      ['in:input', 'in:mask', 'out'], ['in:background', 'in:foreground', 'in:mask', 'out'], ['in:mask', 'out'],
      ['in:mask', 'out'], ['in:mask', 'out'], ['in:mask', 'out'], ['out'], ['in:input'],
    ]);
    // The mask input of the Merge at 324,28: a quarter of the way along its lower edge, pointing up into the card, and named as Fusion names it.
    const mask = port('n4', 'in:mask');
    expect([mask.dataset.input, mask.getAttribute('d'), mask.querySelector('title')!.textContent]).toEqual(['mask', 'M 342 87 L 347 80 L 352 87 Z', 'Effect Mask']);
    expect(port('n6', 'in:mask').getAttribute('d')).toBe('M 46 271 L 51 264 L 56 271 Z');
    // The inputs of the pictures are where they were: the foreground of the Merge two thirds down its left side, the input of the Transform half way.
    const down = 28 + (58 * 2) / 3;
    expect(port('n4', 'in:foreground').getAttribute('d')).toBe(`M 323 ${down - 5} L 330 ${down} L 323 ${down + 5} Z`);
    expect(port('n3', 'in:input').getAttribute('d')).toBe('M 175 52 L 182 57 L 175 62 Z');
    // The colour of masks is a variable of the themes: of the stripe of a mask, of a mask input and of a link into one,
    // which a link chosen or about to be replaced overrides by rules that come after.
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    expect(blockOf(readFileSync('src/ui/tokens.css', 'utf8'), ':root')).toMatch(/--fusion-mask: #[0-9a-f]{6};/);
    expect(blockOf(css, ".fusion-page .fusion-node[data-kind='polygon'] .fusion-node-stripe")).toContain('fill: var(--fusion-mask);');
    for (const kind of ['rectangle', 'ellipse', 'polygon']) expect(css).toContain(`.fusion-page .fusion-node[data-kind='${kind}'] .fusion-node-stripe`);
    expect(blockOf(css, ".fusion-page .fusion-port[data-input='mask']")).toContain('fill: var(--fusion-mask);');
    expect(blockOf(css, ".fusion-page .fusion-link[data-input='mask']")).toContain('stroke: var(--fusion-mask);');
    expect(css.indexOf(".fusion-page .fusion-link[data-input='mask'] {")).toBeLessThan(css.indexOf(".fusion-page .fusion-link[data-hover='end'] {"));
    expect(css.indexOf(".fusion-page .fusion-link[data-input='mask'] {")).toBeLessThan(css.indexOf('.fusion-page .fusion-link[data-chosen],'));
  });

  it('links any output into a mask input by a drag, takes the place of the link that was there, and takes its end to another mask input or away', async () => {
    const { bridge } = await mount(projectOf([timelineOf(maskedComp())]));
    bridge.call.mockClear();
    // The Rectangle into the mask of the Merge.
    dragFrom(port('n6', 'out'), M.n6Out, M.n4Mask);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n6', 'n4', 'mask') }]]);
    // The mask of a node comes after its pictures in the order of the engine; the link is one of a mask, and comes into the input from below.
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n6>n4:mask']);
    expect([linkDrawn('n6>n4:mask')!.dataset.input, linkDrawn('n6>n4:mask')!.getAttribute('d')]).toEqual(['mask', 'M 120 241 C 233.5 241, 347 163.5, 347 86']);
    expect(linkDrawn('n3>n4:background')!.dataset.input).toBe('background');
    // The nodes stay where they were: the Rectangle fed nothing before, and lies left of the Merge already.
    expect([placeOf('n6'), placeOf('n4')]).toEqual([[28, 212], [324, 28]]);
    // The Ellipse into the same input: one connect_fusion, and the link of the Rectangle is put out of its place.
    bridge.call.mockClear();
    dragFrom(port('n8', 'out'), M.n8Out, M.n4Mask);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n8', 'n4', 'mask') }]]);
    expect([linkDrawn('n8>n4:mask') !== null, linkDrawn('n6>n4:mask')]).toEqual([true, null]);
    // The link that is there already is not sent again.
    bridge.call.mockClear();
    dragFrom(port('n8', 'out'), M.n8Out, M.n4Mask);
    await settle();
    expect(sent(bridge)).toEqual([]);
    // A press on a mask input that has a link takes up the end of the link: let go on the mask input of the Transform, the old link goes and the new one comes.
    dragFrom(port('n4', 'in:mask'), M.n4Mask, M.n3Mask);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'n4', input: 'mask' }, { op: 'connect_fusion', ...AT, link: link('n8', 'n3', 'mask') }]]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n8>n3:mask', 'n3>n4:background', 'n5>n4:foreground']);
    // Chosen with a click, the link is deleted with Delete; and the end of a link dragged off a mask input takes the link away.
    bridge.call.mockClear();
    press(half('n8>n3:mask', 'start'), { x: 150, y: 300 });
    expect(linkDrawn('n8>n3:mask')!.getAttribute('data-chosen')).toBe('true');
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'disconnect_fusion', ...AT, to: 'n3', input: 'mask' }]]);
    bridge.call.mockClear();
    dragFrom(port('n6', 'out'), M.n6Out, M.n3Mask);
    await settle();
    dragFrom(port('n3', 'in:mask'), M.n3Mask, { x: 260, y: 200 });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n6', 'n3', 'mask') }], [{ op: 'disconnect_fusion', ...AT, to: 'n3', input: 'mask' }]]);
    expect(problem()).toBeNull();
  });

  it('adds a mask from its group of the tool bar: linked into the mask input of the node chosen where that is free, and loose otherwise', async () => {
    const { bridge } = await mount();
    // The masks are a group of their own, after the nodes of a picture and the effects on one.
    const groups = [...document.querySelectorAll<HTMLElement>('.fusion-add')];
    expect(groups.map(each => each.getAttribute('aria-label'))).toEqual(['Add Node', 'Effects', 'Masks']);
    expect(groups.map(each => [...each.querySelectorAll('button')].map(button => button.textContent))).toEqual([
      ['Merge', 'Transform', 'Background', 'MediaIn'], ['Blur', 'Mosaic', 'Delta Keyer', 'Tracker'], ['Rectangle', 'Ellipse', 'Polygon'],
    ]);
    // No node chosen: the mask is added with no links, and is the node chosen.
    bridge.call.mockClear();
    fireEvent.click(marked('add-rectangle')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('rectangle') }]]);
    expect([currentNode(), nodeOf('n10').dataset.kind]).toEqual(['n10', 'rectangle']);
    // The Merge chosen, whose mask input is free: the mask goes into it, in the one operation, which is one step to undo.
    choose('n4');
    bridge.call.mockClear();
    fireEvent.click(marked('add-ellipse')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n4', 'mask')] }]]);
    expect(linksDrawn()).toContain('n11>n4:mask');
    expect(currentNode()).toBe('n11');
    // The Ellipse is the one chosen now: a Polygon added goes into the mask that comes into it, and begins with four points.
    bridge.call.mockClear();
    fireEvent.click(marked('add-polygon')!); await settle();
    expect(sent(bridge)).toEqual([[{
      op: 'add_fusion_node', ...AT, node: { ...newNode('polygon'), params: { ...newPolygon(), points: startPoints() } }, links: [link('', 'n11', 'mask')],
    }]]);
    expect(linksDrawn()).toContain('n12>n11:mask');
    expect(fieldsShown().filter(name => name!.startsWith('points.'))).toHaveLength(8);
    // The Merge again, whose mask input has a link now: the mask is added loose, and the link stays.
    choose('n4');
    bridge.call.mockClear();
    fireEvent.click(marked('add-rectangle')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('rectangle') }]]);
    expect(linksDrawn()).toContain('n11>n4:mask');
    // A node chosen that has no mask input, the MediaIn: loose.
    choose('in1');
    bridge.call.mockClear();
    fireEvent.click(marked('add-ellipse')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('ellipse') }]]);
    // A mask is never put into the chain of the picture: the links of the pictures are as they were.
    expect(linksDrawn().filter(name => !name!.endsWith(':mask'))).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
  });

  it('adds a mask from the menu of a node into the mask input of that node, where the menu was opened', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n3'), { clientX: 230, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Ellipse' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('ellipse', { x: 230, y: 60 }), links: [link('', 'n3', 'mask')] }]]);
    expect(placeOf('n10')).toEqual([230, 60]);
    expect(linksDrawn()).toContain('n10>n3:mask');
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

describe('the masks in the Inspector of the Fusion page', () => {
  const checked = (name: string) => (row(name).querySelector('input') as HTMLInputElement).checked;
  const select = (name: string) => row(name).querySelector('select')!;
  const pointFields = () => fieldsShown().filter(name => name!.startsWith('points.'));
  const pointNames = () => [...inspector().querySelectorAll<HTMLButtonElement>('.fusion-point-name')];
  const addPoint = () => marked('add-point') as HTMLButtonElement;
  const removePoint = () => marked('remove-point') as HTMLButtonElement;
  const keyedAt = (name: string) => keysOf(name)[1].dataset.keyed;
  /** The points of the Polygon n7 as the engine has them. */
  const pointsIn = (project: Project) => {
    const found = (project.timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!.nodes.find(node => node.id === 'n7')!;
    return (found.params as { points: FusionPoint[] }).points;
  };

  it('shows the settings of a mask by its kind, with the defaults of the engine, and the points of a Polygon each with its two numbers', async () => {
    await mount(projectOf([timelineOf(maskedComp())]));
    press(card('n6'), M.n6);
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n6', 'rectangle', 'Rectangle 1']);
    expect(fieldsShown()).toEqual(['label', 'level', 'soft_edge', 'border_width', 'paint_mode', 'invert', 'solid', 'center.x', 'center.y', 'width', 'height',
      'corner_radius', 'angle']);
    const numbers = ['level', 'soft_edge', 'border_width', 'center.x', 'center.y', 'width', 'height', 'corner_radius', 'angle'];
    expect(numbers.map(name => box(name).value)).toEqual(['1.000', '0.000', '0.000', '0.500', '0.500', '0.500', '0.500', '0.000', '0.0']);
    expect(numbers.map(name => row(name).querySelector('label')!.textContent))
      .toEqual(['Level', 'Soft Edge', 'Border Width', 'Center X', 'Center Y', 'Width', 'Height', 'Corner Radius', 'Angle']);
    // Every number has the three buttons of keys; the modes and the switches take none.
    expect([...inspector().querySelectorAll<HTMLElement>('[data-keys]')].map(each => each.dataset.keys)).toEqual(numbers);
    expect(keysOf('soft_edge').map(each => each.title)).toEqual(['Previous keyframe', 'Add keyframe', 'Next keyframe']);
    expect([select('paint_mode').value, [...select('paint_mode').options].map(each => each.textContent)]).toEqual(['merge', ['Merge', 'Add', 'Subtract']]);
    expect([checked('invert'), checked('solid')]).toEqual([false, true]);
    // The border of a mask may be less than nothing: its slider goes both ways.
    expect([slider('border_width').min, slider('border_width').max, slider('soft_edge').min]).toEqual(['-1', '1', '0']);
    // An Ellipse has no corners.
    press(card('n8'), M.n8);
    expect([inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['ellipse', 'Ellipse 1']);
    expect(fieldsShown()).toEqual(['label', 'level', 'soft_edge', 'border_width', 'paint_mode', 'invert', 'solid', 'center.x', 'center.y', 'width', 'height', 'angle']);
    // A Polygon: its Size and its Fill Method, and the list of its points, each with its X and its Y from the centre and their keys.
    press(card('n7'), M.n7);
    expect([inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['polygon', 'Polygon 1']);
    expect(fieldsShown()).toEqual(['label', 'level', 'soft_edge', 'border_width', 'paint_mode', 'invert', 'solid', 'center.x', 'center.y', 'size', 'angle', 'fill',
      'points.0.x', 'points.0.y', 'points.1.x', 'points.1.y', 'points.2.x', 'points.2.y', 'points.3.x', 'points.3.y']);
    expect([select('fill').value, [...select('fill').options].map(each => each.textContent)]).toEqual(['alternate', ['Alternate', 'Non Zero Winding']]);
    expect(pointFields().map(name => box(name!).value)).toEqual(['-0.200', '-0.200', '0.200', '-0.200', '0.200', '0.200', '-0.200', '0.200']);
    expect(pointNames().map(each => each.textContent)).toEqual(['Point 1', 'Point 2', 'Point 3', 'Point 4']);
    expect([row('points.2.x').querySelector('label')!.textContent, row('points.2.y').querySelector('label')!.textContent]).toEqual(['Point 3 X', 'Point 3 Y']);
    expect(keysOf('points.2.x').map(each => each.getAttribute('aria-label')))
      .toEqual(['Point 3 X: Previous keyframe', 'Point 3 X: Add keyframe', 'Point 3 X: Next keyframe']);
    // The numbers of a point have no slider, so that the list stays short.
    expect(row('points.0.x').querySelector('input[type="range"]')).toBeNull();
    expect(marked('fusion-few-points')).toBeNull();
  });

  it('sets the numbers, the switches and the modes of a mask, and a number of a point by its path', async () => {
    const { bridge } = await mount(projectOf([timelineOf(maskedComp())]));
    press(card('n6'), M.n6);
    bridge.call.mockClear();
    type('soft_edge', '0.1'); await settle();
    // Beyond the range, the value is the end of the range: a border of one width of the picture taken away, a rectangle ten pictures wide.
    type('border_width', '-2'); await settle();
    type('center.y', '0.25'); await settle();
    type('width', '99'); await settle();
    type('corner_radius', '0.5'); await settle();
    type('angle', '45'); await settle();
    fireEvent.click(row('invert').querySelector('input')!); await settle();
    fireEvent.click(row('solid').querySelector('input')!); await settle();
    fireEvent.change(select('paint_mode'), { target: { value: 'subtract' } }); await settle();
    const rectangle = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n6', params }];
    expect(sent(bridge)).toEqual([
      rectangle({ soft_edge: 0.1 }), rectangle({ border_width: -1 }), rectangle({ center: { y: 0.25 } }), rectangle({ width: 10 }), rectangle({ corner_radius: 0.5 }),
      rectangle({ angle: 45 }), rectangle({ invert: true }), rectangle({ solid: false }), rectangle({ paint_mode: 'subtract' }),
    ]);
    expect([checked('invert'), checked('solid'), select('paint_mode').value]).toEqual([true, false, 'subtract']);
    // A double click on a label puts the default back.
    bridge.call.mockClear();
    fireEvent.doubleClick(row('width').querySelector('label')!); await settle();
    expect(sent(bridge)).toEqual([rectangle({ width: 0.5 })]);
    // A Polygon: its Fill Method and its Size by a patch of the node; a number of a point by its path, since a patch cannot name one point of the list.
    press(card('n7'), M.n7);
    bridge.call.mockClear();
    fireEvent.change(select('fill'), { target: { value: 'non_zero_winding' } }); await settle();
    type('size', '2'); await settle();
    type('points.2.x', '0.3'); await settle();
    // Beyond the range of a point, 16 widths of the picture from the centre.
    type('points.0.y', '-99'); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n7', params: { fill: 'non_zero_winding' } }], [{ op: 'set_fusion_node', ...AT, id: 'n7', params: { size: 2 } }],
      [{ op: 'set_fusion_keys', ...AT, path: 'n7.points.2.x', param: 0.3 }], [{ op: 'set_fusion_keys', ...AT, path: 'n7.points.0.y', param: -16 }],
    ]);
    expect([box('points.2.x').value, box('points.0.y').value, select('fill').value]).toEqual(['0.300', '-16.000', 'non_zero_winding']);
  });

  it('puts a key on a setting of a mask and on a number of a point with the diamond, and a value typed at the key changes it', async () => {
    const { bridge } = await mount(projectOf([timelineOf(maskedComp())]));
    press(card('n7'), M.n7);
    bridge.call.mockClear();
    expect([keyedAt('soft_edge'), keyedAt('points.2.x')]).toEqual([undefined, undefined]);
    // The playhead is at frame 0, where the clip shows picture 0 of its media.
    fireEvent.click(keysOf('soft_edge')[1]); await settle();
    fireEvent.click(keysOf('points.2.x')[1]); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'splice_fusion_keys', ...AT, path: 'n7.soft_edge', at: 0, remove: 0, keys: [{ frame: 0, value: 0, interp: 'linear' }] }],
      [{ op: 'splice_fusion_keys', ...AT, path: 'n7.points.2.x', at: 0, remove: 0, keys: [{ frame: 0, value: 0.2, interp: 'linear' }] }],
    ]);
    expect([keyedAt('soft_edge'), keyedAt('points.2.x'), keyedAt('points.2.y')]).toEqual(['true', 'true', undefined]);
    expect(keysOf('points.2.x')[1].getAttribute('aria-pressed')).toBe('true');
    expect(nodeOf('n7').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    // A value typed where the key is changes the key.
    bridge.call.mockClear();
    type('points.2.x', '0.4'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n7.points.2.x', at: 0, remove: 1, keys: [{ frame: 0, value: 0.4, interp: 'linear' }] }]]);
    // The diamond takes the key away again: its last key leaves its value, as one number.
    bridge.call.mockClear();
    fireEvent.click(keysOf('points.2.x')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_keys', ...AT, path: 'n7.points.2.x', param: 0.4 }]]);
    expect(keyedAt('points.2.x')).toBeUndefined();
  });

  it('adds a point after the last or after the one chosen and takes one away, the points that stay keeping their keys', async () => {
    const { bridge, project } = await mount(projectOf([timelineOf(maskedComp())]));
    press(card('n7'), M.n7);
    expect([addPoint().textContent, removePoint().textContent, addPoint().disabled, removePoint().disabled]).toEqual(['Add Point', 'Remove Point', false, false]);
    // The second point takes a key on its X, to be told by.
    fireEvent.click(keysOf('points.1.x')[1]); await settle();
    const start = startPoints();
    const second = { x: { keys: [{ frame: 0, value: 0.2, interp: 'linear' }] }, y: -0.2 };
    const list = (points: unknown[]) => [[{ op: 'set_fusion_node', ...AT, id: 'n7', params: { points } }]];
    // No point chosen: after the last, in the middle of the side that closes the outline, and it is the point chosen then.
    bridge.call.mockClear();
    fireEvent.click(addPoint()); await settle(); await settle();
    expect(sent(bridge)).toEqual(list([start[0], second, start[2], start[3], { x: -0.2, y: 0 }]));
    expect(pointNames().map(each => each.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'false', 'false', 'true']);
    expect(inspector().querySelector<HTMLElement>('[data-point="4"]')!.dataset.chosen).toBe('true');
    // The first point chosen by its name: the new point goes after it, between it and the second, whose keys go with it to the third place.
    fireEvent.click(pointNames()[0]);
    expect(pointNames()[0].getAttribute('aria-pressed')).toBe('true');
    bridge.call.mockClear();
    fireEvent.click(addPoint()); await settle(); await settle();
    expect(sent(bridge)).toEqual(list([start[0], { x: 0, y: -0.2 }, second, start[2], start[3], { x: -0.2, y: 0 }]));
    expect(pointsIn(project())[2]).toEqual(second);
    expect([keyedAt('points.1.x'), keyedAt('points.2.x')]).toEqual([undefined, 'true']);
    expect(pointFields()).toHaveLength(12);
    // The new point is the one chosen: Remove Point takes it away, and the keys are at the second place again.
    expect(pointNames().map(each => each.getAttribute('aria-pressed'))).toEqual(['false', 'true', 'false', 'false', 'false', 'false']);
    bridge.call.mockClear();
    fireEvent.click(removePoint()); await settle();
    expect(sent(bridge)).toEqual(list([start[0], second, start[2], start[3], { x: -0.2, y: 0 }]));
    expect([keyedAt('points.1.x'), keyedAt('points.2.x')]).toEqual(['true', undefined]);
    // No point chosen: the last is taken away, down to the three that are the fewest that make a shape.
    expect(pointNames().every(each => each.getAttribute('aria-pressed') === 'false')).toBe(true);
    bridge.call.mockClear();
    fireEvent.click(removePoint()); await settle();
    fireEvent.click(removePoint()); await settle();
    expect(sent(bridge)).toEqual([...list([start[0], second, start[2], start[3]]), ...list([start[0], second, start[2]])]);
    expect([addPoint().disabled, removePoint().disabled]).toEqual([false, true]);
    expect(pointFields()).toEqual(['points.0.x', 'points.0.y', 'points.1.x', 'points.1.y', 'points.2.x', 'points.2.y']);
    expect(pointsIn(project())).toEqual([start[0], second, start[2]]);
    expect(marked('fusion-inspector-problem')).toBeNull();
  });

  it('adds no point to a polygon that is full, takes none from one of three or fewer, and says of one of fewer that it masks nothing', async () => {
    const withPolygon = (points: FusionPoint[]): Composition => ({ ...comp(), nodes: [...comp().nodes, polygonOf(points)] });
    // The Polygon is the third node of the first column, at 28,212.
    await mount(projectOf([timelineOf(withPolygon(Array.from({ length: 128 }, (_, index) => ({ x: index / 1000, y: 0 }))))]));
    press(card('n7'), M.n6);
    expect(pointFields()).toHaveLength(256);
    expect([addPoint().disabled, removePoint().disabled, marked('fusion-few-points')]).toEqual([true, false, null]);
    cleanup();
    await mount(projectOf([timelineOf(withPolygon(startPoints().slice(0, 2)))]));
    press(card('n7'), M.n6);
    expect([addPoint().disabled, removePoint().disabled]).toEqual([false, true]);
    expect(marked('fusion-few-points')!.textContent).toBe(en.hint.studio.fusionPage.fewPoints);
    // A third point makes it a shape, and the page says so no longer.
    fireEvent.click(addPoint()); await settle(); await settle();
    expect([pointFields().length, marked('fusion-few-points')]).toEqual([6, null]);
  });

  it('says nothing of a change that changes nothing, and says another refusal at its top', async () => {
    const { refuse } = await mount(projectOf([timelineOf(maskedComp())]));
    press(card('n6'), M.n6);
    // A slider let go where it was, a value typed and a switch that the engine takes for no change: none is a failure.
    refuse({ reason: 'empty' });
    const soft = slider('soft_edge');
    fireEvent.pointerDown(soft);
    fireEvent.input(soft, { target: { value: '0.2' } }); await settle();
    fireEvent.change(soft, { target: { value: '0.2' } }); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
    type('level', '0.5'); await settle();
    fireEvent.click(row('invert').querySelector('input')!); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
    refuse({ reason: 'out_of_range' });
    type('level', '0.25'); await settle();
    expect(marked('fusion-inspector-problem')!.textContent).toContain(en.hint.studio.edit.out_of_range);
  });
});

describe('the masks in the Spline Editor of the Fusion page', () => {
  const listed = () => [...document.querySelectorAll<HTMLElement>('[data-test="fusion-curves"] li[data-curve]')].map(each => each.dataset.curve);
  const named = (curve: string) => one(`[data-test="fusion-curves"] li[data-curve="${curve}"]`).textContent;
  /** `maskedComp` with keys on the Soft Edge of the Polygon and on the X of its third point. */
  const keyedMasks = (): Composition => {
    const made = maskedComp();
    const keys = (value: number) => ({ keys: [{ frame: 0, value, interp: 'linear' as const }] });
    const points: FusionPoint[] = startPoints().map((point, index) => (index === 2 ? { ...point, x: keys(0.2) } : point));
    const keyed = { ...newNode('polygon', null, 'n7'), params: { ...newPolygon(), points, soft_edge: keys(0.1) } } as FusionNode;
    return { ...made, nodes: made.nodes.map(node => (node.id === 'n7' ? keyed : node)) };
  };

  it('lists the settings of a mask as those of the other nodes, and of the points of a Polygon the numbers that have keys, named by their point', async () => {
    await mount(projectOf([timelineOf(keyedMasks())]));
    expect(nodeOf('n7').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    fireEvent.click(marked('toggle-spline')!);
    // None chosen: what has keys, of every node, each named with its node; a point is counted from 1.
    expect(listed()).toEqual(['n7.soft_edge', 'n7.points.2.x']);
    expect([named('n7.soft_edge'), named('n7.points.2.x')].map(text => text!.includes('Polygon 1 · '))).toEqual([true, true]);
    expect(named('n7.soft_edge')).toContain('Polygon 1 · Soft Edge');
    expect(named('n7.points.2.x')).toContain('Polygon 1 · Point 3 X');
    // The Polygon chosen: every setting of its kind, those with keys first, and of its points the one number that has keys.
    press(card('n7'), M.n7);
    expect(listed()).toEqual(['n7.soft_edge', 'n7.points.2.x', 'n7.level', 'n7.border_width', 'n7.center.x', 'n7.center.y', 'n7.size', 'n7.angle']);
    expect(named('n7.points.2.x')).toContain('Point 3 X');
    // A Rectangle has no keys: its settings are listed all the same, for a key to be put on one.
    press(card('n6'), M.n6);
    const settings = ['level', 'soft_edge', 'border_width', 'center.x', 'center.y', 'width', 'height', 'corner_radius', 'angle'];
    expect(listed()).toEqual(settings.map(path => `n6.${path}`));
    const names = ['Level', 'Soft Edge', 'Border Width', 'Center X', 'Center Y', 'Width', 'Height', 'Corner Radius', 'Angle'];
    expect(settings.map((path, index) => named(`n6.${path}`)!.includes(names[index]))).toEqual(settings.map(() => true));
  });
});

// A Blur and a Mosaic (PROTOCOL.md, "Blur and Mosaic"): each has a picture, `input`, and an effect mask.
describe('a Blur and a Mosaic in the node editor of the Fusion page', () => {
  const ports = (id: string) => [...nodeOf(id).querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);

  it('draws each as a card of its kind, with its picture on its left, its effect mask on its lower edge and its output on its right', async () => {
    await mount(projectOf([timelineOf(effectComp())]));
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n6', 'n7']);
    expect([nodeOf('n6').dataset.kind, nodeOf('n7').dataset.kind]).toEqual(['blur', 'mosaic']);
    expect([placeOf('n6'), placeOf('n7')]).toEqual([[28, 212], [28, 304]]);
    expect([...graph().querySelectorAll('[data-test="node-label"]')].map(each => each.textContent).slice(5)).toEqual(['Blur 1', 'Mosaic 1']);
    expect([ports('n6'), ports('n7')]).toEqual([['in:input', 'in:mask', 'out'], ['in:input', 'in:mask', 'out']]);
    // The two have one colour, a variable of the themes, on the stripe of their cards.
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    expect(blockOf(readFileSync('src/ui/tokens.css', 'utf8'), ':root')).toMatch(/--fusion-effect: #[0-9a-f]{6};/);
    expect(blockOf(css, ".fusion-page .fusion-node[data-kind='mosaic'] .fusion-node-stripe")).toContain('fill: var(--fusion-effect);');
    expect(css).toContain(".fusion-page .fusion-node[data-kind='blur'] .fusion-node-stripe");
  });

  it('adds each from their group of the tool bar: loose where no node is chosen, and after the node chosen on its picture', async () => {
    const { bridge } = await mount();
    const group = one('.fusion-add-effects');
    expect([group.getAttribute('aria-label'), [...group.querySelectorAll('button')].map(button => button.textContent)]).toEqual(['Effects', ['Blur', 'Mosaic', 'Delta Keyer', 'Tracker']]);
    expect([marked('add-blur')!.title, marked('add-mosaic')!.title]).toEqual(['Add Node: Blur', 'Add Node: Mosaic']);
    // No node chosen: the Blur is added with no links, and is the node chosen.
    bridge.call.mockClear();
    fireEvent.click(marked('add-blur')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('blur') }]]);
    expect([currentNode(), nodeOf('n10').dataset.kind]).toEqual(['n10', 'blur']);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
    // The Transform chosen, which feeds the background of the Merge: the Mosaic goes in after it, the Transform into its input
    // and it into the background, in the one operation, which is one step to undo.
    choose('n3');
    bridge.call.mockClear();
    fireEvent.click(marked('add-mosaic')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('mosaic'), links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'n3>n11:input', 'MediaIn1>n3:input', 'n11>n4:background', 'n5>n4:foreground']);
    expect([currentNode(), nodeOf('n11').dataset.kind]).toEqual(['n11', 'mosaic']);
    // The Mosaic is the one chosen now: a Blur added goes between it and the Merge.
    bridge.call.mockClear();
    fireEvent.click(marked('add-blur')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('blur'), links: [link('n11', '', 'input'), link('', 'n4', 'background')] }]]);
    expect([currentNode(), ports('n12')]).toEqual(['n12', ['in:input', 'in:mask', 'out']]);
    // A mask added with the Blur chosen goes into its effect mask, which nothing is linked into.
    bridge.call.mockClear();
    fireEvent.click(marked('add-ellipse')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n12', 'mask')] }]]);
    expect(linksDrawn()).toContain('n13>n12:mask');
    expect(problem()).toBeNull();
  });

  it('adds each from the right-click menu where the menu was opened: loose from the menu of the background, and after the node from the menu of a node', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(canvas(), { clientX: 300, clientY: 250 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Mosaic' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('mosaic', { x: 300, y: 250 }) }]]);
    expect(placeOf('n10')).toEqual([300, 250]);
    // From the menu of the Merge, the Blur goes in after it, before the MediaOut, and lies where the menu was opened.
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n4'), { clientX: 380, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Blur' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('blur', { x: 380, y: 60 }), links: [link('n4', '', 'input'), link('', 'MediaOut1', 'input')] }]]);
  });
});

describe('a Blur and a Mosaic in the Inspector of the Fusion page', () => {
  const checked = (name: string) => (row(name).querySelector('input') as HTMLInputElement).checked;
  const select = (name: string) => row(name).querySelector('select')!;
  const labelOf = (name: string) => row(name).querySelector('label')!.textContent;
  const withKeys = () => [...inspector().querySelectorAll<HTMLElement>('[data-keys]')].map(each => each.dataset.keys);
  const keyedAt = (name: string) => keysOf(name)[1].dataset.keyed;
  const toggle = (name: string) => fireEvent.click(row(name).querySelector('input')!);
  const blur = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n6', params }];
  const mosaic = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n7', params }];
  /** The settings of a node of the composition of the first clip, as the engine has them. */
  const paramsIn = (project: Project, id: string) => (project.timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!.nodes
    .find(node => node.id === id)!.params;

  it('shows the settings of a Blur, its Blur Size as one number while its X and Y are locked and as two when they are not', async () => {
    const { bridge, project } = await mount(projectOf([timelineOf(effectComp())]));
    press(card('n6'), M.n6);
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n6', 'blur', 'Blur 1']);
    expect(marked('fusion-no-settings')).toBeNull();
    // A new Blur has its X and Y locked: one Blur Size, which is the `size.x` of the node, with its keys.
    expect(fieldsShown()).toEqual(['label', 'filter', 'lock_xy', 'size.x', 'blend']);
    expect(['filter', 'lock_xy', 'size.x', 'blend'].map(labelOf)).toEqual(['Filter', 'Lock X/Y', 'Blur Size', 'Blend']);
    expect([select('filter').value, [...select('filter').options].map(each => each.textContent)]).toEqual(['gaussian', ['Gaussian', 'Box']]);
    expect(checked('lock_xy')).toBe(true);
    expect([box('size.x').value, box('blend').value]).toEqual(['5.0', '1.000']);
    // The numbers have the three buttons of keys; the Filter and the switch take none.
    expect(withKeys()).toEqual(['size.x', 'blend']);
    expect(keysOf('size.x').map(each => each.getAttribute('aria-label'))).toEqual(['Blur Size: Previous keyframe', 'Blur Size: Add keyframe', 'Blur Size: Next keyframe']);
    // The slider of a Blur Size ends at 100, as Fusion's does, though the field takes a size up to 500.
    expect([slider('size.x').min, slider('size.x').max, slider('blend').min, slider('blend').max]).toEqual(['0', '100', '0', '1']);
    // Unlocked: the switch alone is set, and the two sizes are shown apart, each with its keys.
    bridge.call.mockClear();
    toggle('lock_xy'); await settle();
    expect(sent(bridge)).toEqual([blur({ lock_xy: false })]);
    expect(checked('lock_xy')).toBe(false);
    expect(fieldsShown()).toEqual(['label', 'filter', 'lock_xy', 'size.x', 'size.y', 'blend']);
    expect(['size.x', 'size.y'].map(labelOf)).toEqual(['X Blur Size', 'Y Blur Size']);
    expect([box('size.x').value, box('size.y').value]).toEqual(['5.0', '5.0']);
    expect(withKeys()).toEqual(['size.x', 'size.y', 'blend']);
    expect(keysOf('size.y').map(each => each.getAttribute('aria-label'))).toEqual(['Y Blur Size: Previous keyframe', 'Y Blur Size: Add keyframe', 'Y Blur Size: Next keyframe']);
    bridge.call.mockClear();
    type('size.y', '12'); await settle();
    expect(sent(bridge)).toEqual([blur({ size: { y: 12 } })]);
    expect(box('size.y').value).toBe('12.0');
    // Locked again: the switch alone is set, nothing else is changed, and the size down is kept as it is.
    bridge.call.mockClear();
    toggle('lock_xy'); await settle();
    expect(sent(bridge)).toEqual([blur({ lock_xy: true })]);
    expect(fieldsShown()).toEqual(['label', 'filter', 'lock_xy', 'size.x', 'blend']);
    expect([labelOf('size.x'), box('size.x').value, checked('lock_xy')]).toEqual(['Blur Size', '5.0', true]);
    expect(paramsIn(project(), 'n6')).toEqual({ filter: 'gaussian', lock_xy: true, size: { x: 5, y: 12 }, blend: 1 });
  });

  it('sets the Filter and the numbers of a Blur within the range of the catalog, and puts the default back on a double click', async () => {
    const { bridge } = await mount(projectOf([timelineOf(effectComp())]));
    press(card('n6'), M.n6);
    bridge.call.mockClear();
    fireEvent.change(select('filter'), { target: { value: 'box' } }); await settle();
    type('size.x', '12.5'); await settle();
    // Beyond the slider, which ends at 100, the field takes a size up to the 500 of the range, and no more.
    type('size.x', '250'); await settle();
    type('size.x', '999'); await settle();
    type('blend', '0.5'); await settle();
    fireEvent.doubleClick(row('size.x').querySelector('label')!); await settle();
    expect(sent(bridge)).toEqual([
      blur({ filter: 'box' }), blur({ size: { x: 12.5 } }), blur({ size: { x: 250 } }), blur({ size: { x: 500 } }), blur({ blend: 0.5 }), blur({ size: { x: 5 } }),
    ]);
    expect([select('filter').value, box('size.x').value, box('blend').value]).toEqual(['box', '5.0', '0.500']);
  });

  it('sends the Blur Size slid as one gesture, each move the setting whole', async () => {
    const { bridge } = await mount(projectOf([timelineOf(effectComp())]));
    press(card('n6'), M.n6);
    bridge.call.mockClear();
    const size = slider('size.x');
    fireEvent.pointerDown(size);
    fireEvent.input(size, { target: { value: '20' } }); await settle();
    fireEvent.input(size, { target: { value: '30' } }); await settle();
    fireEvent.change(size, { target: { value: '30' } }); await settle();
    const moves = applied(bridge);
    // The release says the last value once more, to end the gesture.
    expect(moves.map(each => each.ops)).toEqual([blur({ size: { x: 20 } }), blur({ size: { x: 30 } }), blur({ size: { x: 30 } })]);
    expect(new Set(moves.map(each => each.gesture?.id)).size).toBe(1);
    expect(moves.map(each => each.gesture!.end)).toEqual([undefined, undefined, true]);
    expect(box('size.x').value).toBe('30.0');
  });

  it('puts a key on a number of a Blur with the diamond, by the path of the setting, and takes it away again', async () => {
    const { bridge } = await mount(projectOf([timelineOf(effectComp())]));
    press(card('n6'), M.n6);
    bridge.call.mockClear();
    expect([keyedAt('size.x'), keyedAt('blend')]).toEqual([undefined, undefined]);
    // The playhead is at frame 0, where the clip shows picture 0 of its media. Locked, the key of the Blur Size is on `size.x`.
    fireEvent.click(keysOf('size.x')[1]); await settle();
    fireEvent.click(keysOf('blend')[1]); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'splice_fusion_keys', ...AT, path: 'n6.size.x', at: 0, remove: 0, keys: [{ frame: 0, value: 5, interp: 'linear' }] }],
      [{ op: 'splice_fusion_keys', ...AT, path: 'n6.blend', at: 0, remove: 0, keys: [{ frame: 0, value: 1, interp: 'linear' }] }],
    ]);
    expect([keyedAt('size.x'), keyedAt('blend'), keysOf('size.x')[1].getAttribute('aria-pressed')]).toEqual(['true', 'true', 'true']);
    expect(nodeOf('n6').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    // A value typed where the key is changes the key.
    bridge.call.mockClear();
    type('size.x', '8'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n6.size.x', at: 0, remove: 1, keys: [{ frame: 0, value: 8, interp: 'linear' }] }]]);
    // Unlocked, the size down takes keys of its own, and the size across keeps those that it has.
    toggle('lock_xy'); await settle();
    expect([keyedAt('size.x'), keyedAt('size.y')]).toEqual(['true', undefined]);
    bridge.call.mockClear();
    fireEvent.click(keysOf('size.y')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'splice_fusion_keys', ...AT, path: 'n6.size.y', at: 0, remove: 0, keys: [{ frame: 0, value: 5, interp: 'linear' }] }]]);
    // The diamond takes the key at the playhead away again: its last key leaves its value, as one number.
    bridge.call.mockClear();
    fireEvent.click(keysOf('size.x')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_keys', ...AT, path: 'n6.size.x', param: 8 }]]);
    expect([keyedAt('size.x'), keyedAt('size.y')]).toEqual([undefined, 'true']);
  });

  it('shows the settings of a Mosaic and sets them, each number within its range, its offsets by a field alone', async () => {
    const { bridge } = await mount(projectOf([timelineOf(effectComp())]));
    press(card('n7'), M.n7);
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n7', 'mosaic', 'Mosaic 1']);
    expect(marked('fusion-no-settings')).toBeNull();
    expect(fieldsShown()).toEqual(['label', 'frequency', 'aspect', 'offset.x', 'offset.y', 'whole_cells_only']);
    const numbers = ['frequency', 'aspect', 'offset.x', 'offset.y'];
    expect([...numbers, 'whole_cells_only'].map(labelOf)).toEqual(['Pixel Frequency', 'Aspect', 'X Offset', 'Y Offset', 'Whole Cells Only']);
    expect(numbers.map(name => box(name).value)).toEqual(['32.0', '1.000', '0.0', '0.0']);
    expect(withKeys()).toEqual(numbers);
    expect(keysOf('offset.x').map(each => each.title)).toEqual(['Previous keyframe', 'Add keyframe', 'Next keyframe']);
    // The sliders are over the part of the range that is used day by day; an offset, of 16384 pixels either way, has none.
    expect([slider('frequency').min, slider('frequency').max, slider('aspect').min, slider('aspect').max]).toEqual(['1', '256', '0.25', '4']);
    expect([row('offset.x').querySelector('input[type="range"]'), row('offset.y').querySelector('input[type="range"]')]).toEqual([null, null]);
    // Whole Cells Only says where the pointer rests on it that it needs an effect mask.
    expect([checked('whole_cells_only'), row('whole_cells_only').title]).toEqual([false, en.hint.studio.fusionPage.wholeCells]);
    bridge.call.mockClear();
    type('frequency', '64'); await settle();
    // Beyond the range, the value is the end of the range: one cell across, and 16384.
    type('frequency', '0'); await settle();
    type('frequency', '99999'); await settle();
    type('aspect', '2'); await settle();
    type('aspect', '99'); await settle();
    type('offset.x', '-120.5'); await settle();
    type('offset.y', '99999'); await settle();
    toggle('whole_cells_only'); await settle();
    fireEvent.doubleClick(row('frequency').querySelector('label')!); await settle();
    expect(sent(bridge)).toEqual([
      mosaic({ frequency: 64 }), mosaic({ frequency: 1 }), mosaic({ frequency: 16384 }), mosaic({ aspect: 2 }), mosaic({ aspect: 16 }),
      mosaic({ offset: { x: -120.5 } }), mosaic({ offset: { y: 16384 } }), mosaic({ whole_cells_only: true }), mosaic({ frequency: 32 }),
    ]);
    expect([...numbers.map(name => box(name).value), checked('whole_cells_only')]).toEqual(['32.0', '16.000', '-120.5', '16384.0', true]);
    // A key on a number of a Mosaic, by the path of the setting, of the value that it has.
    bridge.call.mockClear();
    fireEvent.click(keysOf('offset.y')[1]); await settle();
    fireEvent.click(keysOf('frequency')[1]); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'splice_fusion_keys', ...AT, path: 'n7.offset.y', at: 0, remove: 0, keys: [{ frame: 0, value: 16384, interp: 'linear' }] }],
      [{ op: 'splice_fusion_keys', ...AT, path: 'n7.frequency', at: 0, remove: 0, keys: [{ frame: 0, value: 32, interp: 'linear' }] }],
    ]);
    expect([keyedAt('offset.y'), keyedAt('frequency'), keyedAt('aspect')]).toEqual(['true', 'true', undefined]);
    expect(nodeOf('n7').querySelector('[data-test="node-keyed"]')).not.toBeNull();
  });

  it('says nothing of a change that changes nothing, and says another refusal at its top', async () => {
    const { refuse } = await mount(projectOf([timelineOf(effectComp())]));
    // A value typed and a switch of a Mosaic that the engine takes for no change: neither is a failure.
    press(card('n7'), M.n7);
    refuse({ reason: 'empty' });
    type('frequency', '64'); await settle();
    toggle('whole_cells_only'); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
    // Of a Blur: its Blur Size slid and let go where it was, a value typed, its switch and its Filter.
    press(card('n6'), M.n6);
    const size = slider('size.x');
    fireEvent.pointerDown(size);
    fireEvent.input(size, { target: { value: '20' } }); await settle();
    fireEvent.change(size, { target: { value: '20' } }); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
    type('blend', '0.5'); await settle();
    toggle('lock_xy'); await settle();
    fireEvent.change(select('filter'), { target: { value: 'box' } }); await settle();
    expect(marked('fusion-inspector-problem')).toBeNull();
    refuse({ reason: 'out_of_range' });
    type('blend', '0.25'); await settle();
    expect(marked('fusion-inspector-problem')!.textContent).toContain(en.hint.studio.edit.out_of_range);
  });
});

describe('a Blur and a Mosaic in the Spline Editor of the Fusion page', () => {
  const listed = () => [...document.querySelectorAll<HTMLElement>('[data-test="fusion-curves"] li[data-curve]')].map(each => each.dataset.curve);
  const named = (curve: string) => one(`[data-test="fusion-curves"] li[data-curve="${curve}"]`).textContent ?? '';

  it('lists the settings of each by the names of the Inspector, the Blur Size of a Blur as one while its X and Y are locked', async () => {
    await mount(projectOf([timelineOf(effectComp())]));
    press(card('n6'), M.n6);
    fireEvent.click(marked('toggle-spline')!);
    // Locked: the size down is kept and not read, so it is not listed, and the size across is the Blur Size.
    expect(listed()).toEqual(['n6.size.x', 'n6.blend']);
    expect([named('n6.size.x').includes('Blur Size'), named('n6.size.x').includes('X Blur Size'), named('n6.blend').includes('Blend')]).toEqual([true, false, true]);
    // Unlocked: the two sizes, named apart.
    fireEvent.click(row('lock_xy').querySelector('input')!); await settle();
    expect(listed()).toEqual(['n6.size.x', 'n6.size.y', 'n6.blend']);
    expect([named('n6.size.x').includes('X Blur Size'), named('n6.size.y').includes('Y Blur Size')]).toEqual([true, true]);
    // A key on the size down, and locked again: the keys are kept, so the setting is listed still, with those that have keys first.
    fireEvent.click(keysOf('size.y')[1]); await settle();
    fireEvent.click(row('lock_xy').querySelector('input')!); await settle();
    expect(listed()).toEqual(['n6.size.y', 'n6.size.x', 'n6.blend']);
    expect([named('n6.size.y').includes('Y Blur Size'), named('n6.size.x').includes('X Blur Size')]).toEqual([true, false]);
    // A Mosaic has no keys: its settings are listed all the same, for a key to be put on one.
    press(card('n7'), M.n7);
    const settings = ['frequency', 'aspect', 'offset.x', 'offset.y'];
    expect(listed()).toEqual(settings.map(path => `n7.${path}`));
    const names = ['Pixel Frequency', 'Aspect', 'X Offset', 'Y Offset'];
    expect(settings.map((path, index) => named(`n7.${path}`).includes(names[index]))).toEqual(settings.map(() => true));
    // None chosen: what has keys, of every node, each named with its node.
    press(canvas(), { x: 590, y: 290 });
    expect(listed()).toEqual(['n6.size.y']);
    expect(named('n6.size.y')).toContain('Blur 1 · Y Blur Size');
  });
});

/**
 * `comp` and three nodes that nothing is linked to or from, each where the project puts it, so that it stays there when it is
 * linked: the Delta Keyer n6 at 200,200, and left of it the Ellipse n7 at 40,180 and the Rectangle n8 at 40,280.
 */
const keyerComp = (): Composition => {
  const made = comp();
  return {
    ...made,
    nodes: [...made.nodes, newNode('delta_keyer', { x: 200, y: 200 }, 'n6'), newNode('ellipse', { x: 40, y: 180 }, 'n7'), newNode('rectangle', { x: 40, y: 280 }, 'n8')],
  };
};
/**
 * Where the middle of the card of the keyer of `keyerComp` and its ports are: its picture half way down its left side, its two
 * mattes on its upper edge, a quarter and three quarters of its width (23 and 69 of 92) from its left, its effect mask on its
 * lower edge and its output on its right; and the outputs of the two masks.
 */
const K = {
  n6: { x: 246, y: 229 }, n6Garbage: { x: 223, y: 200 }, n6Solid: { x: 269, y: 200 }, n6Mask: { x: 223, y: 258 },
  n7Out: { x: 132, y: 209 }, n8Out: { x: 132, y: 309 },
};
/** The numbers of a Delta Keyer, in the order of the Inspector, which is that of the catalog. */
const KEYER_NUMBERS = [
  'screen.r', 'screen.g', 'screen.b', 'gain', 'balance', 'pre_blur', 'low', 'high', 'erode_dilate', 'matte_blur', 'clean_foreground', 'clean_background',
  'replace_color.r', 'replace_color.g', 'replace_color.b', 'spill_suppression',
];

describe('a Delta Keyer in the node editor of the Fusion page', () => {
  const ports = (id: string) => [...nodeOf(id).querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
  const linkDrawn = (name: string) => graph().querySelector<SVGElement>(`[data-link="${name}"]`);

  it('presses a name too wide for its card into the room between its ports, and leaves a narrow one as it is', async () => {
    await mount(projectOf([timelineOf(keyerComp())]));
    const label = (id: string) => nodeOf(id).querySelector<SVGTextElement>('[data-test="node-label"]')!;
    expect([label('n6').textContent, label('n6').getAttribute('textLength'), label('n6').getAttribute('lengthAdjust')])
      .toEqual(['Delta Keyer …', String(NAME_ROOM), 'spacingAndGlyphs']);
    expect([label('MediaIn1').getAttribute('textLength'), label('MediaIn1').getAttribute('lengthAdjust')]).toEqual([null, null]);
  });

  it('draws its picture on its left, its two mattes on its upper edge, each in a colour of its own, its effect mask on its lower edge and its output on its right', async () => {
    await mount(projectOf([timelineOf(keyerComp())]));
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n6', 'n7', 'n8']);
    expect([nodeOf('n6').dataset.kind, placeOf('n6')]).toEqual(['delta_keyer', [200, 200]]);
    // A card shows the first twelve characters of a name: the whole of it is in its title.
    expect([...graph().querySelectorAll('[data-test="node-label"]')].map(each => each.textContent).slice(5)).toEqual(['Delta Keyer …', 'Ellipse 1', 'Rectangle 1']);
    expect(ports('n6')).toEqual(['in:input', 'in:garbage_matte', 'in:solid_matte', 'in:mask', 'out']);
    // The mattes point down into the card from its upper edge, and are named as Fusion names them.
    const [garbage, solid] = [port('n6', 'in:garbage_matte'), port('n6', 'in:solid_matte')];
    expect([garbage.dataset.input, garbage.getAttribute('d'), garbage.querySelector('title')!.textContent])
      .toEqual(['garbage_matte', 'M 218 199 L 223 206 L 228 199 Z', 'Garbage Matte']);
    expect([solid.dataset.input, solid.getAttribute('d'), solid.querySelector('title')!.textContent])
      .toEqual(['solid_matte', 'M 264 199 L 269 206 L 274 199 Z', 'Solid Matte']);
    // Its picture and its effect mask are where those of a Blur are: half way down its left side, and a quarter along its lower edge.
    expect(port('n6', 'in:input').getAttribute('d')).toBe('M 199 224 L 206 229 L 199 234 Z');
    expect(port('n6', 'in:mask').getAttribute('d')).toBe('M 218 259 L 223 252 L 228 259 Z');
    expect([port('n6', 'out').getAttribute('x'), port('n6', 'out').getAttribute('y')]).toEqual(['287', '225']);
    // No other kind has a matte input.
    expect([ports('n3'), ports('n4'), ports('n7')]).toEqual([['in:input', 'in:mask', 'out'], ['in:background', 'in:foreground', 'in:mask', 'out'], ['in:mask', 'out']]);
    // The colours of the two mattes are variables of the themes: of their inputs and of the links into them, which a link
    // chosen or about to be replaced overrides by rules that come after. The stripe of the card is in the colour of the effects.
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    const tokens = blockOf(readFileSync('src/ui/tokens.css', 'utf8'), ':root');
    const colours = ['matte-garbage', 'matte-solid'].map(name => new RegExp(`--fusion-${name}: (#[0-9a-f]{6});`).exec(tokens)?.[1]);
    expect(colours.every(colour => colour !== undefined)).toBe(true);
    expect(new Set(colours).size).toBe(2);
    for (const [input, name] of [['garbage_matte', 'matte-garbage'], ['solid_matte', 'matte-solid']]) {
      expect(blockOf(css, `.fusion-page .fusion-port[data-input='${input}']`)).toContain(`fill: var(--fusion-${name});`);
      expect(blockOf(css, `.fusion-page .fusion-link[data-input='${input}']`)).toContain(`stroke: var(--fusion-${name});`);
      expect(css.indexOf(`.fusion-page .fusion-link[data-input='${input}'] {`)).toBeLessThan(css.indexOf(".fusion-page .fusion-link[data-hover='end'] {"));
      expect(css.indexOf(`.fusion-page .fusion-link[data-input='${input}'] {`)).toBeLessThan(css.indexOf('.fusion-page .fusion-link[data-chosen],'));
    }
    expect(blockOf(css, ".fusion-page .fusion-node[data-kind='delta_keyer'] .fusion-node-stripe")).toContain('fill: var(--fusion-effect);');
  });

  it('links any output into a matte by a drag, the link drawn into it from above in its colour, and takes its end to another input or away', async () => {
    const { bridge } = await mount(projectOf([timelineOf(keyerComp())]));
    bridge.call.mockClear();
    // The Ellipse into the garbage matte.
    dragFrom(port('n7', 'out'), K.n7Out, K.n6Garbage);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n7', 'n6', 'garbage_matte') }]]);
    // The link is one of a matte: out of the output to the right, and into the input from above.
    expect([linkDrawn('n7>n6:garbage_matte')!.dataset.input, linkDrawn('n7>n6:garbage_matte')!.getAttribute('d')])
      .toEqual(['garbage_matte', 'M 132 209 C 177.5 209, 223 176, 223 200']);
    // The Rectangle into the solid matte. The mattes of a node are in the order of the engine: the garbage matte, and the solid one last.
    bridge.call.mockClear();
    dragFrom(port('n8', 'out'), K.n8Out, K.n6Solid);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect_fusion', ...AT, link: link('n8', 'n6', 'solid_matte') }]]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n7>n6:garbage_matte', 'n8>n6:solid_matte']);
    expect([linkDrawn('n8>n6:solid_matte')!.dataset.input, linkDrawn('n8>n6:solid_matte')!.getAttribute('d')])
      .toEqual(['solid_matte', 'M 132 309 C 200.5 309, 269 145.5, 269 200']);
    // The nodes stay where the project put them.
    expect([placeOf('n6'), placeOf('n7'), placeOf('n8')]).toEqual([[200, 200], [40, 180], [40, 280]]);
    // The link that is there already is not sent again.
    bridge.call.mockClear();
    dragFrom(port('n8', 'out'), K.n8Out, K.n6Solid);
    await settle();
    expect(sent(bridge)).toEqual([]);
    // A press on a matte input that has a link takes up the end of the link: let go on the effect mask of the keyer, the old
    // link goes and the new one comes; and let go on nothing, the link goes.
    dragFrom(port('n6', 'in:garbage_matte'), K.n6Garbage, K.n6Mask);
    await settle();
    dragFrom(port('n6', 'in:solid_matte'), K.n6Solid, { x: 420, y: 260 });
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'disconnect_fusion', ...AT, to: 'n6', input: 'garbage_matte' }, { op: 'connect_fusion', ...AT, link: link('n7', 'n6', 'mask') }],
      [{ op: 'disconnect_fusion', ...AT, to: 'n6', input: 'solid_matte' }],
    ]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n7>n6:mask']);
    expect(problem()).toBeNull();
  });

  it('is added from the group of the effects of the tool bar: loose where no node is chosen, and after the node chosen on its picture', async () => {
    const { bridge } = await mount();
    expect(marked('add-delta_keyer')!.title).toBe('Add Node: Delta Keyer');
    // No node chosen: the keyer is added with no links, and is the node chosen.
    bridge.call.mockClear();
    fireEvent.click(marked('add-delta_keyer')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer') }]]);
    expect([currentNode(), nodeOf('n10').dataset.kind, ports('n10')]).toEqual(['n10', 'delta_keyer', ['in:input', 'in:garbage_matte', 'in:solid_matte', 'in:mask', 'out']]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
    // The Transform chosen, which feeds the background of the Merge: the keyer goes in after it, the Transform into its
    // `input` and it into the background, in the one operation, which is one step to undo.
    choose('n3');
    bridge.call.mockClear();
    fireEvent.click(marked('add-delta_keyer')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer'), links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'n3>n11:input', 'MediaIn1>n3:input', 'n11>n4:background', 'n5>n4:foreground']);
    expect([currentNode(), nodeOf('n11').dataset.kind]).toEqual(['n11', 'delta_keyer']);
    // A mask added with the keyer chosen goes into its effect mask, which nothing is linked into, and into neither of its mattes.
    bridge.call.mockClear();
    fireEvent.click(marked('add-ellipse')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('ellipse'), links: [link('', 'n11', 'mask')] }]]);
    expect(linksDrawn()).toContain('n12>n11:mask');
    expect(linksDrawn().filter(name => name!.endsWith('_matte'))).toEqual([]);
    expect(problem()).toBeNull();
  });

  it('is added from the right-click menu of a node after that node, and from Select Tool by its name, among the effects', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n4'), { clientX: 380, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Delta Keyer' }));
    await settle();
    expect(sent(bridge)).toEqual([[{
      op: 'add_fusion_node', ...AT, node: newNode('delta_keyer', { x: 380, y: 60 }), links: [link('n4', '', 'input'), link('', 'MediaOut1', 'input')],
    }]]);
    expect(currentNode()).toBe('n10');
    // Select Tool finds it by a part of its name, and says that it is one of the effects; taken, it goes in after the keyer chosen.
    bridge.call.mockClear();
    key(' ', { code: 'Space', shiftKey: true }); await settle();
    const field = screen.getByRole('combobox', { name: 'Search tools' });
    fireEvent.change(field, { target: { value: 'keyer' } });
    const options = [...document.querySelectorAll<HTMLElement>('[data-test="select-tool-option"]')];
    expect(options.map(each => [each.dataset.kind, each.querySelector('.fusion-select-tool-name')!.textContent, each.querySelector('.fusion-select-tool-group')?.textContent]))
      .toEqual([['delta_keyer', 'Delta Keyer', 'Effects']]);
    fireEvent.keyDown(field, { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('delta_keyer'), links: [link('n10', '', 'input'), link('', 'MediaOut1', 'input')] }]]);
    // By the name that the engine gives it too, in every language.
    expect(matchingTools('delta', en.label.studio.fusionPage.kinds)).toEqual(['delta_keyer']);
    expect(matchingTools('delta_keyer', zhCN.label.studio.fusionPage.kinds)).toEqual(['delta_keyer']);
    expect(matchingTools(zhCN.label.studio.fusionPage.kinds.delta_keyer, zhCN.label.studio.fusionPage.kinds)).toEqual(['delta_keyer']);
  });

  it('takes no node dropped on a link into one of its mattes', async () => {
    // `comp`, the Blur n6 that nothing is linked to, at 28,212, the keyer n7 at 300,200 and the Ellipse n8 at 40,320, its garbage matte.
    const made = comp();
    const withMatte: Composition = {
      nodes: [...made.nodes, newNode('blur', null, 'n6'), newNode('delta_keyer', { x: 300, y: 200 }, 'n7'), newNode('ellipse', { x: 40, y: 320 }, 'n8')],
      links: [...made.links, link('n8', 'n7', 'garbage_matte')],
    };
    const { bridge } = await mount(projectOf([timelineOf(withMatte)]));
    expect([placeOf('n6'), placeOf('n7'), placeOf('n8')]).toEqual([[28, 212], [300, 200], [40, 320]]);
    bridge.call.mockClear();
    // The link into the matte runs from 132,349 over the keyer and down into 323,200: its middle lies at 263.3,246.6.
    const over = { x: 264, y: 247 };
    fireEvent.pointerDown(card('n6'), { button: 0, clientX: M.n6.x, clientY: M.n6.y });
    fireEvent.pointerMove(canvas(), { clientX: (M.n6.x + over.x) / 2, clientY: (M.n6.y + over.y) / 2 });
    fireEvent.pointerMove(canvas(), { clientX: over.x, clientY: over.y });
    expect([...graph().querySelectorAll<SVGElement>('[data-replacing]')].map(each => each.dataset.link)).toEqual([]);
    fireEvent.pointerUp(canvas(), { clientX: over.x, clientY: over.y });
    await settle();
    // The Blur is moved, and nothing is linked.
    expect(sent(bridge)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 123, y: 215 } }], [{ op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 218, y: 218 } }],
    ]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n8>n7:garbage_matte']);
  });
});

describe('a Delta Keyer in the Inspector of the Fusion page', () => {
  const checked = (name: string) => (row(name).querySelector('input') as HTMLInputElement).checked;
  const select = (name: string) => row(name).querySelector('select')!;
  const choices = (name: string) => [...select(name).options].map(each => each.textContent);
  const labelOf = (name: string) => row(name).querySelector('label')!.textContent;
  const withKeys = () => [...inspector().querySelectorAll<HTMLElement>('[data-keys]')].map(each => each.dataset.keys);
  const keyedAt = (name: string) => keysOf(name)[1].dataset.keyed;
  const swatch = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="color"]')!;
  const sections = () => [...inspector().querySelectorAll('h3.inspector-section')].map(each => each.textContent);
  const keyer = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n6', params }];
  const keyAt = (path: string, value: number, remove = 0) => ({ op: 'splice_fusion_keys', ...AT, path: `n6.${path}`, at: 0, remove, keys: [{ frame: 0, value, interp: 'linear' }] });
  const shown = async () => {
    const engine = await mount(projectOf([timelineOf(keyerComp())]));
    press(card('n6'), K.n6);
    return engine;
  };
  const whole = [
    'label', 'view', 'screen', 'screen.r', 'screen.g', 'screen.b', 'gain', 'balance', 'pre_blur', 'low', 'high', 'erode_dilate', 'matte_blur', 'clean_foreground',
    'clean_background', 'replace', 'replace_color', 'replace_color.r', 'replace_color.g', 'replace_color.b', 'garbage_invert', 'spill_method', 'spill_suppression',
  ];

  it('shows View Mode and then its settings in the sections of Fusion\'s tabs, with the defaults of the engine', async () => {
    await shown();
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n6', 'delta_keyer', 'Delta Keyer 1']);
    expect(marked('fusion-no-settings')).toBeNull();
    expect(fieldsShown()).toEqual(whole);
    expect(sections()).toEqual(['Key', 'Matte', 'Fringe']);
    expect(whole.slice(1).map(labelOf)).toEqual([
      'View Mode', 'Background Color', 'Background Red', 'Background Green', 'Background Blue', 'Gain', 'Balance', 'Pre-Blur', 'Threshold Low', 'Threshold High',
      'Erode/Dilate', 'Matte Blur', 'Clean Foreground', 'Clean Background', 'Replace Mode', 'Replace Color', 'Replace Red', 'Replace Green', 'Replace Blue',
      'Invert Garbage Matte', 'Spill Method', 'Spill Suppression',
    ]);
    // The modes, as the engine names them and as Fusion orders them.
    expect([select('view').value, choices('view')]).toEqual(['final_result', ['Pre Matte', 'Matte', 'Status', 'Final Result']]);
    expect([select('replace').value, choices('replace')]).toEqual(['soft_color', ['None', 'Source', 'Hard Color', 'Soft Color']]);
    expect([select('spill_method').value, choices('spill_method')]).toEqual(['medium', ['None', 'Rare', 'Medium', 'Well Done', 'Burnt']]);
    // The numbers: a pure green screen, the thresholds at their ends, a mid grey to fill with; each with the three buttons of keys.
    expect(KEYER_NUMBERS.map(name => box(name).value)).toEqual([
      '0.000', '1.000', '0.000', '1.000', '0.500', '0.0', '0.000', '1.000', '0.0', '0.0', '0.000', '0.000', '0.500', '0.500', '0.500', '1.000',
    ]);
    expect(withKeys()).toEqual(KEYER_NUMBERS);
    expect(keysOf('screen.g').map(each => each.getAttribute('aria-label')))
      .toEqual(['Background Green: Previous keyframe', 'Background Green: Add keyframe', 'Background Green: Next keyframe']);
    // The two colours as one, each a swatch.
    expect([swatch('screen').value, swatch('replace_color').value]).toEqual(['#00ff00', '#808080']);
    // The sliders are over the part of the range that is used day by day: a channel of a colour from 0 to 1 of its 65504, the
    // Gain to 4 of its 10, and the lengths to 20 pixels of their 100, the Erode/Dilate either way.
    const range = (name: string) => [slider(name).min, slider(name).max];
    expect(['screen.g', 'replace_color.r', 'gain', 'balance', 'pre_blur', 'matte_blur', 'erode_dilate', 'low'].map(range))
      .toEqual([['0', '1'], ['0', '1'], ['0', '4'], ['0', '1'], ['0', '20'], ['0', '20'], ['-20', '20'], ['0', '1']]);
    // The Invert of the garbage matte says where the pointer rests on it that it needs a garbage matte linked.
    expect([checked('garbage_invert'), row('garbage_invert').title]).toEqual([false, en.hint.studio.fusionPage.garbageInvert]);
  });

  it('sets its modes and its switch by a patch of the node, and its numbers within the range of the catalog', async () => {
    const { bridge } = await shown();
    bridge.call.mockClear();
    fireEvent.change(select('view'), { target: { value: 'matte' } }); await settle();
    fireEvent.change(select('spill_method'), { target: { value: 'burnt' } }); await settle();
    fireEvent.click(row('garbage_invert').querySelector('input')!); await settle();
    type('gain', '2.5'); await settle();
    // Beyond the slider, which ends at 4, the field takes a Gain up to the 10 of the range, and no more.
    type('gain', '99'); await settle();
    type('balance', '0.25'); await settle();
    type('pre_blur', '3'); await settle();
    type('low', '0.2'); await settle();
    type('high', '0.8'); await settle();
    // Below 0 the matte is eroded: down to the 100 pixels of the range.
    type('erode_dilate', '-250'); await settle();
    type('matte_blur', '1.5'); await settle();
    type('clean_foreground', '0.4'); await settle();
    type('spill_suppression', '0.5'); await settle();
    // A channel of a colour above 1, which its slider does not reach.
    type('screen.r', '0.25'); await settle();
    type('replace_color.b', '2'); await settle();
    fireEvent.doubleClick(row('gain').querySelector('label')!); await settle();
    expect(sent(bridge)).toEqual([
      keyer({ view: 'matte' }), keyer({ spill_method: 'burnt' }), keyer({ garbage_invert: true }), keyer({ gain: 2.5 }), keyer({ gain: 10 }),
      keyer({ balance: 0.25 }), keyer({ pre_blur: 3 }), keyer({ low: 0.2 }), keyer({ high: 0.8 }), keyer({ erode_dilate: -100 }), keyer({ matte_blur: 1.5 }),
      keyer({ clean_foreground: 0.4 }), keyer({ spill_suppression: 0.5 }), keyer({ screen: { r: 0.25 } }), keyer({ replace_color: { b: 2 } }), keyer({ gain: 1 }),
    ]);
    expect([select('view').value, select('spill_method').value, checked('garbage_invert')]).toEqual(['matte', 'burnt', true]);
    expect(['gain', 'erode_dilate', 'screen.r', 'replace_color.b'].map(name => box(name).value)).toEqual(['1.000', '-100.0', '0.250', '2.000']);
  });

  it('shows the Replace Color only while the Replace Mode fills with it', async () => {
    const { bridge } = await shown();
    const without = whole.filter(name => !name.startsWith('replace_color'));
    bridge.call.mockClear();
    fireEvent.change(select('replace'), { target: { value: 'source' } }); await settle();
    expect(fieldsShown()).toEqual(without);
    fireEvent.change(select('replace'), { target: { value: 'none' } }); await settle();
    expect(fieldsShown()).toEqual(without);
    expect(withKeys()).toEqual(KEYER_NUMBERS.filter(name => !name.startsWith('replace_color')));
    fireEvent.change(select('replace'), { target: { value: 'hard_color' } }); await settle();
    expect(fieldsShown()).toEqual(whole);
    expect(sent(bridge)).toEqual([keyer({ replace: 'source' }), keyer({ replace: 'none' }), keyer({ replace: 'hard_color' })]);
    // The colour was kept under the modes that do not read it.
    expect([swatch('replace_color').value, box('replace_color.g').value]).toEqual(['#808080', '0.500']);
  });

  it('sets a colour chosen as one by one patch of the node, and puts its keys by the paths of its settings', async () => {
    const { bridge } = await shown();
    bridge.call.mockClear();
    // From green to blue: the two channels that change go together, so that the engine never sees the grey between the two.
    fireEvent.change(swatch('screen'), { target: { value: '#0000ff' } }); await settle();
    fireEvent.change(swatch('replace_color'), { target: { value: '#ff8000' } }); await settle();
    expect(sent(bridge)).toEqual([keyer({ screen: { g: 0, b: 1 } }), keyer({ replace_color: { r: 1, g: 0.502, b: 0 } })]);
    expect(['screen.r', 'screen.g', 'screen.b', 'replace_color.g'].map(name => box(name).value)).toEqual(['0.000', '0.000', '1.000', '0.502']);
    expect([swatch('screen').value, swatch('replace_color').value]).toEqual(['#0000ff', '#ff8000']);
    // The diamond puts a key at the playhead, at frame 0, of the value that the setting has: each by its path in the node.
    bridge.call.mockClear();
    expect([keyedAt('screen.b'), keyedAt('gain')]).toEqual([undefined, undefined]);
    fireEvent.click(keysOf('screen.b')[1]); await settle();
    fireEvent.click(keysOf('gain')[1]); await settle();
    fireEvent.click(keysOf('replace_color.g')[1]); await settle();
    expect(sent(bridge)).toEqual([[keyAt('screen.b', 1)], [keyAt('gain', 1)], [keyAt('replace_color.g', 0.502)]]);
    expect([keyedAt('screen.b'), keyedAt('gain'), keyedAt('replace_color.g'), keyedAt('balance'), keysOf('gain')[1].getAttribute('aria-pressed')])
      .toEqual(['true', 'true', 'true', undefined, 'true']);
    expect(nodeOf('n6').querySelector('[data-test="node-keyed"]')).not.toBeNull();
    // A value typed where the key is changes the key.
    bridge.call.mockClear();
    type('gain', '2'); await settle();
    expect(sent(bridge)).toEqual([[keyAt('gain', 2, 1)]]);
    // A colour chosen as one: the channels without keys by the patch, and the one with keys as a key at the playhead.
    bridge.call.mockClear();
    fireEvent.change(swatch('screen'), { target: { value: '#00ff00' } }); await settle();
    expect(sent(bridge)).toEqual([[...keyer({ screen: { g: 1 } }), keyAt('screen.b', 0, 1)]]);
    // The diamond takes the key at the playhead away again: its last key leaves its value, as one number.
    bridge.call.mockClear();
    fireEvent.click(keysOf('gain')[1]); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_keys', ...AT, path: 'n6.gain', param: 2 }]]);
    expect([keyedAt('gain'), keyedAt('screen.b')]).toEqual([undefined, 'true']);
  });

  it('says in its own words that a screen colour keys nothing and that the thresholds have crossed, and any other refusal as it is said elsewhere', async () => {
    const { refuse } = await shown();
    const said = () => marked('fusion-inspector-problem')?.textContent ?? null;
    // A grey screen: no channel is above the other two.
    refuse({ reason: 'out_of_range', what: 'delta_keyer.screen' });
    type('screen.g', '0'); await settle();
    expect(said()).toContain(en.hint.studio.fusionPage.keyerScreen);
    expect(said()).not.toContain(en.hint.studio.edit.out_of_range);
    fireEvent.click(marked('fusion-inspector-problem')!.querySelector('button')!);
    expect(said()).toBeNull();
    fireEvent.change(swatch('screen'), { target: { value: '#808080' } }); await settle();
    expect(said()).toContain(en.hint.studio.fusionPage.keyerScreen);
    // The lower threshold above the upper one: typed, and slid, which is a gesture.
    refuse({ reason: 'out_of_range', what: 'delta_keyer.low' });
    type('low', '0.9'); await settle();
    expect(said()).toContain(en.hint.studio.fusionPage.keyerThreshold);
    fireEvent.click(marked('fusion-inspector-problem')!.querySelector('button')!);
    const high = slider('high');
    fireEvent.pointerDown(high);
    fireEvent.input(high, { target: { value: '0.1' } }); await settle();
    fireEvent.change(high, { target: { value: '0.1' } }); await settle();
    expect(said()).toContain(en.hint.studio.fusionPage.keyerThreshold);
    // A number out of its range, which the fields do not let through, is said in the general words; and a change that changes nothing is not said.
    refuse({ reason: 'out_of_range', what: 'delta_keyer.gain' });
    type('gain', '3'); await settle();
    expect(said()).toContain(en.hint.studio.edit.out_of_range);
    refuse({ reason: 'empty' });
    type('balance', '0.3'); await settle();
    fireEvent.change(select('view'), { target: { value: 'status' } }); await settle();
    expect(said()).toBeNull();
  });
});

describe('a Delta Keyer in the Spline Editor of the Fusion page', () => {
  const listed = () => [...document.querySelectorAll<HTMLElement>('[data-test="fusion-curves"] li[data-curve]')].map(each => each.dataset.curve);
  const named = (curve: string) => one(`[data-test="fusion-curves"] li[data-curve="${curve}"]`).textContent ?? '';

  it('lists its sixteen numbers by the names of the Inspector, and the channels of its Replace Color only while its Replace Mode reads them', async () => {
    await mount(projectOf([timelineOf(keyerComp())]));
    press(card('n6'), K.n6);
    fireEvent.click(marked('toggle-spline')!);
    expect(listed()).toEqual(KEYER_NUMBERS.map(path => `n6.${path}`));
    const names = [
      'Background Red', 'Background Green', 'Background Blue', 'Gain', 'Balance', 'Pre-Blur', 'Threshold Low', 'Threshold High', 'Erode/Dilate', 'Matte Blur',
      'Clean Foreground', 'Clean Background', 'Replace Red', 'Replace Green', 'Replace Blue', 'Spill Suppression',
    ];
    expect(KEYER_NUMBERS.map((path, index) => named(`n6.${path}`).includes(names[index]))).toEqual(KEYER_NUMBERS.map(() => true));
    // Under a Replace Mode that fills with no colour, the Replace Color is kept and not read: its channels are not listed.
    fireEvent.change(row('replace').querySelector('select')!, { target: { value: 'source' } }); await settle();
    expect(listed()).toEqual(KEYER_NUMBERS.filter(path => !path.startsWith('replace_color.')).map(path => `n6.${path}`));
    // A setting that has keys comes first; and with no node chosen it is the one listed, named with its node.
    fireEvent.click(keysOf('gain')[1]); await settle();
    expect([listed()[0], listed()]).toEqual(['n6.gain', expect.arrayContaining(['n6.screen.g', 'n6.spill_suppression'])]);
    expect(listed()).toHaveLength(13);
    press(canvas(), { x: 590, y: 290 });
    expect(listed()).toEqual(['n6.gain']);
    expect(named('n6.gain')).toContain('Delta Keyer 1 · Gain');
  });
});

// A Tracker (PROTOCOL.md, "The Tracker" and "Following a Tracker").
/** A pattern named Left, put at a quarter of the width and half the height, with a path at frames 30, 32 and 50 of the keys of the clip. */
const LEFT = (): FusionTrackerPattern => ({ ...newPattern(0.25, 0.5), name: 'Left', path: { 30: [0.25, 0.5], 32: [0.3, 0.5], 50: [0.5, 0.5] } });
/**
 * `comp` and two nodes that nothing is linked to or from, each where the project puts it: the Tracker n6 at 200,200, of the pattern
 * Left and an unnamed one at 0.75, 0.5 that has no path, its reference at frame 40 of the keys; and the Rectangle n7 at 40,300, whose
 * centre follows what `follow` says, where it is given.
 */
const trackerComp = (follow: FusionFollow | null = null): Composition => {
  const made = comp();
  const tracker = { ...newNode('tracker', { x: 200, y: 200 }, 'n6'), params: { ...newTracker(), reference: 40, trackers: [LEFT(), newPattern(0.75, 0.5)] } } as FusionNode;
  const box = newNode('rectangle', { x: 40, y: 300 }, 'n7');
  return { ...made, nodes: [...made.nodes, tracker, follow === null ? box : withFollow(box, follow)] };
};
/** The middles of the cards of the Tracker and of the Rectangle of `trackerComp`. */
const T = { n6: { x: 246, y: 229 }, n7: { x: 86, y: 329 } };
/** The timeline of `timelineOf`, its clip c1 cut from picture 20 of its media: at a frame of the timeline its keys count 20 more. */
const trackerTimeline = (composition: Composition): Timeline => ({
  ...timelineOf(),
  tracks: [{
    id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [],
    clips: [clip('c1', 0, 100, { fusion: composition, source: { kind: 'media', media: 'm1', in: { ticks: 20, scale: 25 } } }), clip('c2', 100, 100)],
  }],
});

describe('a Tracker on the Fusion page', () => {
  const sections = () => [...inspector().querySelectorAll('h3.inspector-section')].map(each => each.textContent);
  const patternNames = () => [...inspector().querySelectorAll<HTMLButtonElement>('[data-test="pattern-name"]')];
  const pathLines = () => [...inspector().querySelectorAll<HTMLElement>('[data-test="pattern-path"]')].map(each => each.textContent);
  const switches = () => [...inspector().querySelectorAll<HTMLInputElement>('[data-test="pattern-enabled"]')];
  const select = (name: string) => row(name).querySelector('select')!;
  const checked = (name: string) => (row(name).querySelector('input') as HTMLInputElement).checked;
  const items = () => [...document.querySelectorAll<HTMLButtonElement>('[data-test="color-menu"] [role="menuitem"]')];
  /** The patterns of the Tracker set whole, and other settings of it set by a patch. */
  const list = (trackers: unknown[]) => [{ op: 'set_fusion_node', ...AT, id: 'n6', params: { trackers } }];
  const tracker = (params: Record<string, unknown>) => [{ op: 'set_fusion_node', ...AT, id: 'n6', params }];
  /** The settings of a node of the composition of the first clip, as the engine has them. */
  const paramsIn = (project: Project, id: string) => (project.timelines[0].tracks[0] as { clips: { fusion?: Composition }[] }).clips[0].fusion!.nodes
    .find(node => node.id === id)!.params;
  /** The studio on the composition of `trackerComp`, the Tracker chosen. */
  const shown = async (follow: FusionFollow | null = null) => {
    const engine = await mount(projectOf([trackerTimeline(trackerComp(follow))]));
    press(card('n6'), T.n6);
    return engine;
  };

  it('is added from the group of the effects with one pattern in the middle of the picture, and has its picture, its foreground and its mask', async () => {
    const { bridge } = await mount();
    const made = { ...newNode('tracker'), params: { ...newTracker(), trackers: [newPattern()] } };
    expect(marked('add-tracker')!.title).toBe('Add Node: Tracker');
    bridge.call.mockClear();
    fireEvent.click(marked('add-tracker')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: made }]]);
    const ports = [...nodeOf('n10').querySelectorAll<SVGElement>('[data-port]')].map(each => each.dataset.port);
    expect([currentNode(), nodeOf('n10').dataset.kind, ports]).toEqual(['n10', 'tracker', ['in:input', 'in:foreground', 'in:mask', 'out']]);
    // After the node chosen, on its picture, as a Blur goes.
    choose('n3');
    bridge.call.mockClear();
    fireEvent.click(marked('add-tracker')!); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: made, links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
    // Its card is in the colour of the effects.
    const css = readFileSync('src/studio/fusion/fusion.css', 'utf8');
    expect(blockOf(css, ".fusion-page .fusion-node[data-kind='tracker'] .fusion-node-stripe")).toContain('fill: var(--fusion-effect);');
  });

  it('shows its trackers and its Operation in the sections of Fusion\'s tabs, with the defaults of the engine and no buttons of keys', async () => {
    await shown();
    expect([inspector().dataset.node, inspector().dataset.kind, marked('fusion-node-name')!.textContent]).toEqual(['n6', 'tracker', 'Tracker 1']);
    expect(sections()).toEqual(['Trackers', 'Operation']);
    // The list: each pattern by its name or by its place, and what its path reaches, at the frames of the timeline (the keys count 20 more).
    expect(patternNames().map(each => [each.textContent, each.getAttribute('aria-pressed')])).toEqual([['Left', 'true'], ['Tracker 2', 'false']]);
    expect(pathLines()).toEqual(['Path: 3 frames, 10–30', 'No path']);
    expect(switches().map(each => [each.checked, each.getAttribute('aria-label')])).toEqual([[true, 'Enabled: Left'], [true, 'Enabled: Tracker 2']]);
    // The first is the one chosen: its name and its numbers; then the controls of the Operation tab.
    expect(fieldsShown()).toEqual([
      'label', 'tracker.name', 'pattern.x', 'pattern.y', 'pattern.width', 'pattern.height', 'search.width', 'search.height', 'offset.x', 'offset.y',
      'operation', 'merge', 'position', 'rotation', 'scaling', 'pivot', 'reference',
    ]);
    expect(['pattern.x', 'pattern.y', 'pattern.width', 'pattern.height', 'search.width', 'search.height', 'offset.x', 'offset.y'].map(name => box(name).value))
      .toEqual(['0.250', '0.500', '0.050', '0.050', '0.125', '0.125', '0.000', '0.000']);
    expect(row('tracker.name').querySelector('input')!.value).toBe('Left');
    // Nothing of it takes keys.
    expect(inspector().querySelectorAll('[data-keys]')).toHaveLength(0);
    // An Operation of None, so that the Merge is off; the boxes on; the pivot at the tracker average; the reference at frame 40 of the keys,
    // which is frame 20 of the timeline.
    expect([select('operation').value, [...select('operation').options].map(each => each.textContent)]).toEqual(['none', ['None', 'Match Move']]);
    expect([select('merge').value, select('merge').disabled, [...select('merge').options].map(each => each.textContent)])
      .toEqual(['bg_only', true, ['BG Only', 'FG Only', 'FG Over BG', 'BG Over FG']]);
    expect([checked('position'), checked('rotation'), checked('scaling')]).toEqual([true, true, true]);
    expect([select('pivot').value, [...select('pivot').options].map(each => each.textContent)]).toEqual(['tracker_average', ['Tracker Average', 'Selected Tracker', 'Manual']]);
    expect(box('reference').value).toBe('20');
    // The second chosen: its numbers and its name, and no path to clear.
    fireEvent.click(patternNames()[1]);
    expect([box('pattern.x').value, row('tracker.name').querySelector('input')!.value, (marked('clear-path') as HTMLButtonElement).disabled]).toEqual(['0.750', '', true]);
    // The Spline Editor has nothing of it.
    fireEvent.click(marked('toggle-spline')!);
    expect(marked('fusion-curves-empty')!.textContent).toBe(en.hint.studio.fusionPage.noCurves);
  });

  it('sets its patterns whole, each change of one with the others and every path as they are', async () => {
    const { bridge, project } = await shown();
    const moved = { ...LEFT(), pattern: { ...LEFT().pattern, x: 0.3 } };
    bridge.call.mockClear();
    type('pattern.x', '0.3'); await settle();
    expect(sent(bridge)).toEqual([list([moved, newPattern(0.75, 0.5)])]);
    // Add Tracker: a pattern in the middle of the picture, after the others, which is the one chosen then.
    bridge.call.mockClear();
    fireEvent.click(marked('add-pattern')!); await settle(); await settle();
    expect(sent(bridge)).toEqual([list([moved, newPattern(0.75, 0.5), newPattern()])]);
    expect(patternNames().map(each => each.getAttribute('aria-pressed'))).toEqual(['false', 'false', 'true']);
    expect(pathLines()).toEqual(['Path: 3 frames, 10–30', 'No path', 'No path']);
    // The second taken away: the first, its path with it, and the third are left.
    bridge.call.mockClear();
    fireEvent.click(inspector().querySelectorAll<HTMLButtonElement>('[data-test="remove-pattern"]')[1]); await settle();
    expect(sent(bridge)).toEqual([list([moved, newPattern()])]);
    // The first is chosen again: its path cleared, the other turned off, and the first renamed.
    expect(patternNames()[0].getAttribute('aria-pressed')).toBe('true');
    bridge.call.mockClear();
    fireEvent.click(marked('clear-path')!); await settle();
    fireEvent.click(switches()[1]); await settle();
    const name = row('tracker.name').querySelector('input')!;
    fireEvent.change(name, { target: { value: 'Eye' } }); fireEvent.keyDown(name, { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([
      list([{ ...moved, path: {} }, newPattern()]),
      list([{ ...moved, path: {} }, { ...newPattern(), enabled: false }]),
      list([{ ...moved, path: {}, name: 'Eye' }, { ...newPattern(), enabled: false }]),
    ]);
    expect([patternNames().map(each => each.textContent), pathLines()]).toEqual([['Eye', 'Tracker 2'], ['No path', 'No path']]);
    expect((paramsIn(project(), 'n6') as FusionTracker).trackers).toEqual([{ ...moved, path: {}, name: 'Eye' }, { ...newPattern(), enabled: false }]);
    expect(marked('fusion-inspector-problem')).toBeNull();
  });

  it('sets the controls of its Operation tab by a patch, and keeps its reference at the frame of the keys that the clip shows at the frame given', async () => {
    const { bridge } = await shown();
    bridge.call.mockClear();
    fireEvent.change(select('operation'), { target: { value: 'match_move' } }); await settle();
    // With Match Move the Merge is read, and set.
    expect(select('merge').disabled).toBe(false);
    fireEvent.change(select('merge'), { target: { value: 'fg_over_bg' } }); await settle();
    fireEvent.click(row('rotation').querySelector('input')!); await settle();
    fireEvent.change(select('pivot'), { target: { value: 'selected_tracker' } }); await settle();
    // The pattern that the pivot is at, by its name.
    expect([...select('pivot_tracker').options].map(each => [each.value, each.textContent])).toEqual([['0', 'Left'], ['1', 'Tracker 2']]);
    fireEvent.change(select('pivot_tracker'), { target: { value: '1' } }); await settle();
    fireEvent.change(select('pivot'), { target: { value: 'manual' } }); await settle();
    expect(fieldsShown().slice(-4)).toEqual(['pivot', 'pivot_point.x', 'pivot_point.y', 'reference']);
    type('pivot_point.x', '0.25'); await settle();
    // Frame 25 of the timeline shows frame 45 of the keys; and the playhead, at frame 0, frame 20.
    type('reference', '25'); await settle();
    expect(box('reference').value).toBe('25');
    fireEvent.click(marked('set-reference')!); await settle();
    expect(sent(bridge)).toEqual([
      tracker({ operation: 'match_move' }), tracker({ merge: 'fg_over_bg' }), tracker({ rotation: false }), tracker({ pivot: 'selected_tracker' }),
      tracker({ pivot_tracker: 1 }), tracker({ pivot: 'manual' }), tracker({ pivot_point: { x: 0.25, y: 0.5 } }), tracker({ reference: 45 }), tracker({ reference: 20 }),
    ]);
    expect([box('reference').value, checked('rotation'), select('merge').value]).toEqual(['0', false, 'fg_over_bg']);
  });

  it('is followed by the centre of a mask by Connect To, which shows its X and Y off while it follows, and lets go of it by Disconnect', async () => {
    const { bridge, project } = await mount(projectOf([trackerTimeline(trackerComp())]));
    press(card('n7'), T.n7);
    expect([inspector().dataset.kind, marked('connect-to')!.textContent, marked('fusion-follows')]).toEqual(['rectangle', 'Not connected', null]);
    // Connect To is no setting of the node: the fields are those that a mask has.
    expect(fieldsShown()).toEqual(['label', 'level', 'soft_edge', 'border_width', 'paint_mode', 'invert', 'solid', 'center.x', 'center.y', 'width', 'height',
      'corner_radius', 'angle']);
    // The menu: where each pattern of the Tracker is, its two positions, and Disconnect, off while the centre follows nothing.
    fireEvent.click(marked('connect-to')!);
    expect(items().map(each => [each.getAttribute('aria-label'), each.disabled])).toEqual([
      ['Tracker 1 › Left', false], ['Tracker 1 › Tracker 2', false], ['Tracker 1 › Steady Position', false], ['Tracker 1 › Unsteady Position', false],
      ['Disconnect', true],
    ]);
    bridge.call.mockClear();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Tracker 1 › Left' })); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: { node: 'n6', output: 'tracker', tracker: 0 } } } }]]);
    // While it follows, its X and Y are kept and not read: shown, off, with their buttons of keys; and the Inspector says what it follows.
    expect([box('center.x').disabled, box('center.y').disabled, slider('center.x').disabled, box('width').disabled]).toEqual([true, true, true, false]);
    expect([...keysOf('center.x'), ...keysOf('center.y')].map(each => each.disabled)).toEqual([true, true, true, true, true, true]);
    expect(keysOf('width')[1].disabled).toBe(false);
    expect([marked('connect-to')!.textContent, marked('fusion-follows')!.textContent]).toEqual(['Tracker 1 › Left', 'Follows Tracker 1 › Left']);
    // The menu checks what it follows; Disconnect lets go of it, and the centre is its own again.
    fireEvent.click(marked('connect-to')!);
    expect(items().filter(each => each.dataset.checked).map(each => each.getAttribute('aria-label'))).toEqual(['Tracker 1 › Left']);
    bridge.call.mockClear();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Disconnect' })); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: null } } }]]);
    expect([box('center.x').disabled, marked('fusion-follows'), marked('connect-to')!.textContent]).toEqual([false, null, 'Not connected']);
    expect((paramsIn(project(), 'n7') as FusionRectangle).center).toStrictEqual({ x: 0.5, y: 0.5 });
  });

  it('is deleted with the centres that follow it letting go of it first, in the same transaction', async () => {
    const { bridge, project } = await shown({ node: 'n6', output: 'steady_position', tracker: 0 });
    bridge.call.mockClear();
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n7', params: { center: { follow: null } } }, { op: 'remove_fusion_node', ...AT, id: 'n6' }]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n7']);
    expect((paramsIn(project(), 'n7') as FusionRectangle).center).toStrictEqual({ x: 0.5, y: 0.5 });
    expect(problem()).toBeNull();
  });
});

// The tracking of the patterns of a Tracker (PROTOCOL.md, "The patterns of a Tracker"), driven as the Color page drives a window's.
describe('tracking the patterns of a Tracker on the Fusion page', () => {
  const track = en.hint.studio.fusionPage.track;
  const button = (name: string) => marked(`tracker-${name}`) as HTMLButtonElement;
  const status = () => marked('fusion-track-status')?.textContent ?? null;
  const offs = () => [
    (marked('add-pattern') as HTMLButtonElement).disabled,
    inspector().querySelector<HTMLInputElement>('[data-test="pattern-enabled"]')!.disabled,
    box('pattern.x').disabled,
    (marked('clear-path') as HTMLButtonElement).disabled,
  ];
  /** The fusion target of a track of the Tracker n6 of the clip c1, at the revision of the project. */
  const target = { fusion: { timeline: 't1', clip: 'c1', node: 'n6' }, revision: expect.any(Number) };
  /** How a track ended, as the engine tells it. */
  const ended = (job: string, phase: string, reason: string | null, trackers: unknown[] = []) => ({
    job, phase, reason, frames: 5, lost: 0, frame: 55, revision: 4, tracked: [50, 55], trackers,
  });
  /**
   * The studio on the composition of `trackerComp`, its clip c1 cut from picture 20 of its media: the playhead at frame 30 of the
   * timeline, which is frame 50 of the keys, and the Tracker chosen.
   */
  const tracking = async () => {
    const engine = await mount(projectOf([trackerTimeline(trackerComp())]));
    await engine.notify({ frame: 30, shown: 30 });
    press(card('n6'), T.n6);
    return engine;
  };

  it('tracks the enabled patterns from the frame of the keys at the playhead, follows it in the viewer, and stops with Stop', async () => {
    const { bridge, news, outside } = await tracking();
    // The buttons in the order of Fusion's, and Track Forward and Back; Stop off while nothing goes on.
    expect([...marked('fusion-track')!.querySelectorAll<HTMLButtonElement>('button')].map(each => [each.getAttribute('aria-label'), each.disabled])).toEqual([
      ['Track Reverse', false], ['Track One Frame Reverse', false], ['Stop Tracking', true], ['Track One Frame Forward', false], ['Track Forward', false],
      ['Track Forward and Back', false],
    ]);
    expect(offs()).toEqual([false, false, false, false]);
    fireEvent.click(button('forward')); await settle();
    // Every enabled pattern (no `trackers`), from frame 50 of the keys, which the playhead at frame 30 of the timeline shows.
    expect(calls(bridge, 'track.start')).toEqual([{ ...target, frame: 50, direction: 'forward' }]);
    // While it goes, Stop alone is on, and nothing of the Tracker is changed.
    expect([button('forward').disabled, button('reverse').disabled, button('one_forward').disabled, button('stop').disabled]).toEqual([true, true, true, false]);
    expect(offs()).toEqual([true, true, true, true]);
    // The frame reached is shown in the viewer, and said with the patterns lost so far.
    await news('track.progress', { job: 'track-1', frame: 55, timeline_frame: 35, done: 5, total: 70, lost: 1, revision: 3, following: [0] });
    await settle();
    expect(calls(bridge, 'transport.seek').at(-1)).toEqual({ monitor: 'timeline', frame: 35, scrubbing: false });
    expect(status()).toBe('Tracking 00:00:01:10: frame 5 of 70. Patterns lost: 1.');
    expect(marked('fusion-clip-name')!.dataset.clip).toBe('c1');
    // The news of another job is not this one's.
    await news('track.progress', { job: 'track-9', frame: 90, timeline_frame: 70, done: 1, total: 2, lost: 0, revision: 3, following: [] });
    await news('track.state', ended('track-9', 'done', null));
    expect([button('stop').disabled, status()]).toEqual([false, 'Tracking 00:00:01:10: frame 5 of 70. Patterns lost: 1.']);
    fireEvent.click(button('stop')); await settle();
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-1' }]);
    // How it ended, and where each pattern was lost, at the frame of the timeline that shows frame 53 of the keys.
    await news('track.state', ended('track-1', 'stopped', 'stop', [{ tracker: 0, frames: 5, lost: [] }, { tracker: 1, frames: 3, lost: [53] }]));
    expect(status()).toBe(`${track.stopped} Tracker 2 was lost at 00:00:01:08.`);
    expect([button('forward').disabled, button('stop').disabled]).toEqual([false, true]);
    expect(offs()).toEqual([false, false, false, false]);
    // Each other button tracks its own way.
    for (const direction of ['reverse', 'one_reverse', 'one_forward', 'forward_and_back']) {
      fireEvent.click(button(direction)); await settle();
      await news('track.state', ended('track-1', 'done', null));
    }
    expect(calls(bridge, 'track.start').map(each => (each as { direction: string }).direction)).toEqual(['forward', 'reverse', 'one_reverse', 'one_forward', 'forward_and_back']);
    expect(status()).toBe(track.done.replace('{frames}', '5'));
    expect(marked('fusion-inspector-problem')).toBeNull();
    // How it ended is said until the project changes after the revision that its last write made (4), as an undo of it does.
    for (let at = 0; at < 4; at++) await outside([]);
    expect(status()).toBe(track.done.replace('{frames}', '5'));
    await outside([]);
    expect(status() ?? '').toBe('');
  });

  it('says why the engine would not track, and tracks nothing then', async () => {
    const { bridge, answerTrack, news } = await tracking();
    for (const reason of ['untrackable', 'busy', 'no_trackers', 'stale_revision', 'frame', 'target'] as const) {
      answerTrack({ ok: false, error: { code: -32001, message: 'refused', data: { reason } } });
      fireEvent.click(button('forward')); await settle();
      expect([status(), button('forward').disabled, button('stop').disabled]).toEqual([track.failed[reason], false, true]);
    }
    expect(calls(bridge, 'track.start')).toHaveLength(6);
    expect(calls(bridge, 'track.stop')).toEqual([]);
    // A track that the engine could not finish says why too.
    answerTrack({ ok: true, result: { job: 'track-2', first: 20, last: 119, frame_size: [256, 144], direction: 'forward', trackers: [0, 1] } });
    fireEvent.click(button('forward')); await settle();
    expect(button('stop').disabled).toBe(false);
    await news('track.state', { ...ended('track-2', 'failed', 'cannot_read'), message: 'no decoder', tracked: null });
    expect([status(), button('forward').disabled, button('stop').disabled]).toEqual([track.failed.cannot_read, false, true]);
  });

  it('takes the keys of the Tracker menu of the Color page while a Tracker is chosen', async () => {
    const { bridge, news } = await tracking();
    const directions = () => calls(bridge, 'track.start').map(each => (each as { direction: string }).direction);
    key('t', { code: 'KeyT', altKey: true }); await settle();
    expect(directions()).toEqual(['reverse']);
    key('t', { code: 'KeyT', ctrlKey: true, altKey: true }); await settle();
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-1' }]);
    await news('track.state', ended('track-1', 'stopped', 'stop'));
    expect(status()).toBe(track.stopped);
    key('ArrowRight', { altKey: true }); await settle();
    await news('track.state', ended('track-1', 'done', null));
    key('ArrowLeft', { altKey: true }); await settle();
    await news('track.state', ended('track-1', 'done', null));
    key('t', { code: 'KeyT', ctrlKey: true }); await settle();
    await news('track.state', ended('track-1', 'done', null));
    expect(directions()).toEqual(['reverse', 'one_forward', 'one_reverse', 'forward']);
    // With no track going on Stop is no key of the page; with a node of another kind chosen, nothing is tracked.
    const stop = new KeyboardEvent('keydown', { key: 't', code: 'KeyT', ctrlKey: true, altKey: true, bubbles: true, cancelable: true });
    document.body.dispatchEvent(stop);
    press(card('n7'), T.n7);
    const forward = new KeyboardEvent('keydown', { key: 't', code: 'KeyT', ctrlKey: true, bubbles: true, cancelable: true });
    document.body.dispatchEvent(forward);
    await settle();
    expect([stop.defaultPrevented, forward.defaultPrevented, directions().length, calls(bridge, 'track.stop').length]).toEqual([false, false, 4, 1]);
  });

  it('stops the track as another node is chosen and as the page is left', async () => {
    const { bridge, news } = await tracking();
    fireEvent.click(button('forward')); await settle();
    press(card('n7'), T.n7); await settle();
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-1' }]);
    expect(marked('fusion-track')).toBeNull();
    await news('track.state', ended('track-1', 'stopped', 'stop'));
    // The Tracker chosen again says how its track ended.
    press(card('n6'), T.n6);
    expect(status()).toBe(track.stopped);
    fireEvent.click(button('forward')); await settle();
    expect(button('stop').disabled).toBe(false);
    await open('Edit');
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-1' }, { job: 'track-1' }]);
  });

  it('takes over a track of the patterns of a Tracker that it did not start, and not a track of a window', async () => {
    const { bridge, news } = await tracking();
    await news('track.progress', { job: 'track-5', frame: 12, timeline_frame: 40, done: 1, total: 9, lost: 0, revision: 2 });
    expect([button('stop').disabled, status()]).toEqual([true, null]);
    await news('track.progress', { job: 'track-6', frame: 60, timeline_frame: 40, done: 2, total: 9, lost: 0, revision: 2, following: [0, 1] });
    expect([button('stop').disabled, button('forward').disabled, (marked('add-pattern') as HTMLButtonElement).disabled]).toEqual([false, true, true]);
    expect(status()).toBe('Tracking 00:00:01:15: frame 2 of 9. Patterns lost: 0.');
    fireEvent.click(button('stop')); await settle();
    expect(calls(bridge, 'track.stop')).toEqual([{ job: 'track-6' }]);
    await news('track.state', ended('track-6', 'stopped', 'stale_revision'));
    expect([status(), button('forward').disabled]).toEqual([track.changed, false]);
  });
});

describe('copying, cutting, pasting and duplicating nodes on the Fusion page', () => {
  /** A letter with Ctrl, or with what is given in its place. */
  const chord = (letter: string, fields: Record<string, unknown> = { ctrlKey: true }) => key(letter, { code: `Key${letter.toUpperCase()}`, ...fields });
  /** A node as it is pasted: one of its kind with every setting at its default, at its place, named by the transaction. */
  const pasted = (kind: Parameters<typeof newNode>[0], at: At, index = 0) => ({ ...newNode(kind, at), id: `$p${index}` });
  const off = (name: string) => (screen.getByRole('menuitem', { name }) as HTMLButtonElement).disabled;

  it('copies the nodes chosen with Ctrl+C and pastes them with Ctrl+V: new nodes down and right, linked among themselves alone, and chosen', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    // Nothing was copied yet: Ctrl+V sends nothing.
    chord('v'); await settle();
    expect(sent(bridge)).toEqual([]);
    choose('n3');
    press(card('n4'), P.n4, { ctrlKey: true });
    // Copying changes nothing of the project.
    chord('c'); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(copiedNodes()!.nodes.map(node => [node.id, node.pos])).toEqual([['n3', { x: 176, y: 28 }], ['n4', { x: 324, y: 28 }]]);
    chord('v'); await settle();
    // One transaction, which is one step to undo: the link from the Transform into the Merge goes with the later of the two.
    expect(sent(bridge)).toEqual([[
      { op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 200, y: 52 }, 0) },
      { op: 'add_fusion_node', ...AT, node: pasted('merge', { x: 348, y: 52 }, 1), links: [link('$p0', '$p1', 'background')] },
    ]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5', 'n10', 'n11']);
    expect([placeOf('n10'), placeOf('n11')]).toEqual([[200, 52], [348, 52]]);
    // Nothing is linked to the rest of the graph, and the nodes pasted are the ones chosen.
    expect(linksDrawn().filter(name => /n1[01]/.test(name!))).toEqual(['n10>n11:background']);
    expect(linksDrawn()).toHaveLength(5);
    expect([currentNode(), chosenNodes()]).toEqual(['n11', ['n10']]);
    // Pasted again, the same nodes lie further down and right, clear of the ones pasted before.
    bridge.call.mockClear();
    chord('v'); await settle();
    expect(sent(bridge)).toEqual([[
      { op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 224, y: 76 }, 0) },
      { op: 'add_fusion_node', ...AT, node: pasted('merge', { x: 372, y: 76 }, 1), links: [link('$p0', '$p1', 'background')] },
    ]]);
    expect([currentNode(), chosenNodes()]).toEqual(['n13', ['n12']]);
    expect(problem()).toBeNull();
  });

  it('cuts with Ctrl+X as Delete takes nodes away, pastes with Cmd+V too, and duplicates with Ctrl+D without touching what was copied', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    chord('x'); await settle();
    // The Transform is taken away, and what fed it is joined to what it fed, in one transaction.
    expect(sent(bridge)).toEqual([[{ op: 'remove_fusion_node', ...AT, id: 'n3', links: [link('MediaIn1', 'n4', 'background')] }]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n4', 'n5']);
    // It is kept: pasted, it is a new node down and right of where it lay, with no links.
    bridge.call.mockClear();
    chord('v', { metaKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 200, y: 52 }) }]]);
    expect([currentNode(), placeOf('n10')]).toEqual(['n10', [200, 52]]);
    // Duplicate: the node chosen once more, down and right of itself.
    bridge.call.mockClear();
    chord('d'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 224, y: 76 }) }]]);
    expect(currentNode()).toBe('n11');
    // What was cut is still what Paste pastes: from where it lay, clear of the two that lie there now.
    expect(copiedNodes()!.nodes.map(node => node.id)).toEqual(['n3']);
    bridge.call.mockClear();
    chord('v'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 248, y: 100 }) }]]);
    // The MediaOut is neither cut nor duplicated.
    choose('out1');
    bridge.call.mockClear();
    chord('x'); chord('d'); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(copiedNodes()!.nodes.map(node => node.id)).toEqual(['n3']);
  });

  it('has Cut, Copy, Paste and Duplicate in the menus: of the nodes chosen, and pasted where the menu was opened', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n3'), { clientX: P.n3.x, clientY: P.n3.y });
    // Nothing was copied yet, so there is nothing to paste.
    expect([off('Cut'), off('Copy'), off('Paste'), off('Duplicate')]).toEqual([false, false, true, false]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
    await settle();
    expect(sent(bridge)).toEqual([]);
    // From the menu of the background: where the menu was opened.
    fireEvent.contextMenu(canvas(), { clientX: 300, clientY: 250 });
    expect(off('Paste')).toBe(false);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Paste' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 300, y: 250 }) }]]);
    expect([currentNode(), placeOf('n10')]).toEqual(['n10', [300, 250]]);
    // Duplicate and Cut, from the menu of a node.
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n5'), { clientX: P.n5.x, clientY: P.n5.y });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Duplicate' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('background', { x: 52, y: 144 }) }]]);
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n5'), { clientX: P.n5.x, clientY: P.n5.y });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Cut' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'remove_fusion_node', ...AT, id: 'n5' }]]);
    expect(copiedNodes()!.nodes.map(node => node.kind)).toEqual(['background']);
    // The MediaOut is not copied: a composition has exactly one.
    fireEvent.contextMenu(card('MediaOut1'), { clientX: P.out1.x, clientY: P.out1.y });
    expect([off('Cut'), off('Copy'), off('Paste'), off('Duplicate')]).toEqual([true, true, false, true]);
    // A node right-clicked that is one of several chosen leaves them chosen, and the menu is about them all. (The press puts the menu away.)
    choose('n3');
    press(card('n4'), P.n4, { ctrlKey: true });
    fireEvent.contextMenu(card('n4'), { clientX: P.n4.x, clientY: P.n4.y });
    expect([currentNode(), chosenNodes()]).toEqual(['n4', ['n3']]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
    expect([copiedNodes()!.nodes.map(node => node.id), copiedNodes()!.links]).toEqual([['n3', 'n4'], [link('n3', 'n4', 'background')]]);
  });

  it('pastes into the composition of another clip what was copied in one, the first composition before it where the clip has none', async () => {
    const { bridge, notify } = await mount();
    choose('n3');
    press(card('n4'), P.n4, { ctrlKey: true });
    chord('c');
    await notify({ frame: 160, shown: 160 });
    expect(clipShown().dataset.clip).toBe('c2');
    bridge.call.mockClear();
    chord('v'); await settle();
    const at = { timeline: 't1', clip: 'c2' };
    expect(sent(bridge)).toEqual([[
      { op: 'set_fusion', ...at, value: firstComposition() },
      { op: 'add_fusion_node', ...at, node: pasted('transform', { x: 200, y: 52 }, 0) },
      { op: 'add_fusion_node', ...at, node: pasted('merge', { x: 348, y: 52 }, 1), links: [link('$p0', '$p1', 'background')] },
    ]]);
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n10', 'n11']);
    expect(linksDrawn()).toEqual(['MediaIn1>MediaOut1:input', 'n10>n11:background']);
    expect([currentNode(), chosenNodes()]).toEqual(['n11', ['n10']]);
    // Back on the first clip, the same nodes are pasted there too.
    await notify({ frame: 0, shown: 0 });
    expect(clipShown().dataset.clip).toBe('c1');
    bridge.call.mockClear();
    chord('v'); await settle();
    expect(sent(bridge)).toEqual([[
      { op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 200, y: 52 }, 0) },
      { op: 'add_fusion_node', ...AT, node: pasted('merge', { x: 348, y: 52 }, 1), links: [link('$p0', '$p1', 'background')] },
    ]]);
  });

  it('pastes nothing into a composition that has no room for it, and says why', async () => {
    // 255 nodes of the 256 that a composition may have.
    const made = comp();
    const many: Composition = { ...made, nodes: [...made.nodes, ...Array.from({ length: 250 }, (_, index) => newNode('transform', null, `t${index}`))] };
    const { bridge } = await mount(projectOf([timelineOf(many)]));
    expect(nodesDrawn()).toHaveLength(255);
    choose('n3');
    press(card('n4'), P.n4, { ctrlKey: true });
    bridge.call.mockClear();
    chord('c'); chord('v'); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(problem()).toBe(en.hint.studio.fusionPage.tooManyNodes);
    // Duplicate of the two is refused in the same way; one node more has room.
    chord('d'); await settle();
    expect(sent(bridge)).toEqual([]);
    choose('n3');
    chord('d'); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: pasted('transform', { x: 200, y: 52 }) }]]);
    expect(problem()).toBeNull();
    expect(nodesDrawn()).toHaveLength(256);
  });
});

describe('a node dropped onto a link on the Fusion page', () => {
  const replacing = () => [...graph().querySelectorAll<SVGElement>('[data-replacing]')].map(each => each.dataset.link);
  /** Press an element at a point and drag to another by way of the point between them, without letting go. */
  const dragTo = (element: Element, from: At, to: At) => {
    fireEvent.pointerDown(element, { button: 0, clientX: from.x, clientY: from.y });
    fireEvent.pointerMove(canvas(), { clientX: (from.x + to.x) / 2, clientY: (from.y + to.y) / 2 });
    fireEvent.pointerMove(canvas(), { clientX: to.x, clientY: to.y });
  };
  const letGo = (at: At) => fireEvent.pointerUp(canvas(), { clientX: at.x, clientY: at.y });

  it('goes into the link: what fed the link feeds its first picture and it feeds what the link fed, in the gesture of the move', async () => {
    const { bridge } = await mount(projectOf([timelineOf(effectComp())]));
    bridge.call.mockClear();
    // The link from the Transform into the background of the Merge runs from 268,57 to 324,47.3: its middle lies at 296,52.2.
    const over = { x: 296, y: 53 };
    dragTo(card('n6'), M.n6, over);
    // While the Blur is over it, the link is drawn as one about to be replaced.
    expect(replacing()).toEqual(['n3>n4:background']);
    letGo(over);
    expect(replacing()).toEqual([]);
    await settle();
    const moves = applied(bridge);
    expect(moves.map(each => each.ops)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 139, y: 118 } }],
      [
        { op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 250, y: 24 } },
        { op: 'connect_fusion', ...AT, link: link('n3', 'n6') }, { op: 'connect_fusion', ...AT, link: link('n6', 'n4', 'background') },
      ],
    ]);
    // One gesture, ended by the transaction that has the links: one step to undo.
    expect(new Set(moves.map(each => each.gesture?.id)).size).toBe(1);
    expect(moves.map(each => each.gesture!.end)).toEqual([undefined, true]);
    expect(linksDrawn()).toEqual(expect.arrayContaining(['n3>n6:input', 'n6>n4:background']));
    expect(linksDrawn()).not.toContain('n3>n4:background');
    expect(linksDrawn()).toHaveLength(5);
    expect(placeOf('n6')).toEqual([250, 24]);
    expect(problem()).toBeNull();
  });

  it('does not go into a link into a mask input', async () => {
    // `comp`, the Blur n6 that nothing is linked to, at 28,212, and the Rectangle n7, at 28,304, into the mask of the Merge.
    const made = comp();
    const withMask: Composition = { nodes: [...made.nodes, newNode('blur', null, 'n6'), newNode('rectangle', null, 'n7')], links: [...made.links, link('n7', 'n4', 'mask')] };
    const { bridge } = await mount(projectOf([timelineOf(withMask)]));
    expect([placeOf('n6'), placeOf('n7')]).toEqual([[28, 212], [28, 304]]);
    bridge.call.mockClear();
    // The link into the mask runs from 120,333 up into 347,86: its middle lies at 276.1,255.8.
    const over = { x: 276, y: 255 };
    dragTo(card('n6'), M.n6, over);
    expect(replacing()).toEqual([]);
    letGo(over);
    await settle();
    // The Blur is moved, and nothing is linked.
    expect(sent(bridge)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 129, y: 219 } }], [{ op: 'set_fusion_node', ...AT, id: 'n6', pos: { x: 230, y: 226 } }],
    ]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground', 'n7>n4:mask']);
  });

  it('does not go into a link where it has links itself', async () => {
    const { bridge } = await mount(projectOf([timelineOf(comp(false))]));
    bridge.call.mockClear();
    // The Transform, which is linked, over the link from the Merge into the MediaOut, from 416,57 to 472,57.
    const over = { x: 444, y: 57 };
    dragTo(card('n3'), P.n3, over);
    expect(replacing()).toEqual([]);
    letGo(over);
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 287, y: 28 } }], [{ op: 'set_fusion_node', ...AT, id: 'n3', pos: { x: 398, y: 28 } }],
    ]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background']);
  });
});

describe('Select Tool on the Fusion page', () => {
  const dialog = () => screen.queryByRole('dialog', { name: 'Select Tool' });
  const field = () => screen.getByRole('combobox', { name: 'Search tools' }) as HTMLInputElement;
  const options = () => [...document.querySelectorAll<HTMLElement>('[data-test="select-tool-option"]')];
  const kinds = () => options().map(each => each.dataset.kind);
  const markedKind = () => options().find(each => each.getAttribute('aria-selected') === 'true')?.dataset.kind ?? null;
  const shiftSpace = () => key(' ', { code: 'Space', shiftKey: true });

  it('opens on Shift+Space while the node editor has the focus, narrows as it is typed in, and adds the kind marked on Enter', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    shiftSpace(); await settle();
    // A labelled dialog, the keyboard in its field, which owns the list and says which kind is marked.
    expect(dialog()).not.toBeNull();
    expect(document.activeElement).toBe(field());
    expect(screen.getByRole('listbox', { name: 'Select Tool' }).id).toBe(field().getAttribute('aria-controls'));
    expect(kinds()).toEqual(['merge', 'transform', 'background', 'media_in', 'blur', 'mosaic', 'delta_keyer', 'tracker', 'rectangle', 'ellipse', 'polygon']);
    expect(options().map(each => each.querySelector('.fusion-select-tool-name')!.textContent))
      .toEqual(['Merge', 'Transform', 'Background', 'MediaIn', 'Blur', 'Mosaic', 'Delta Keyer', 'Tracker', 'Rectangle', 'Ellipse', 'Polygon']);
    expect(options().every(each => each.getAttribute('role') === 'option')).toBe(true);
    expect([markedKind(), field().getAttribute('aria-activedescendant')]).toEqual(['merge', options()[0].id]);
    // Shift+Space was the editor's: the transport, which plays on Space, was not asked to.
    expect(calls(bridge, 'transport.play')).toHaveLength(0);
    // What is typed narrows the list: the kinds whose name begins with it, then those that have it further on.
    fireEvent.change(field(), { target: { value: 'm' } });
    expect(kinds()).toEqual(['merge', 'media_in', 'mosaic', 'transform']);
    // The arrows move through the list, and round its ends.
    fireEvent.keyDown(field(), { key: 'ArrowDown' });
    expect([markedKind(), field().getAttribute('aria-activedescendant')]).toEqual(['media_in', options()[1].id]);
    fireEvent.keyDown(field(), { key: 'ArrowUp' });
    fireEvent.keyDown(field(), { key: 'ArrowUp' });
    expect(markedKind()).toBe('transform');
    fireEvent.change(field(), { target: { value: 'mos' } });
    expect([kinds(), markedKind()]).toEqual([['mosaic'], 'mosaic']);
    // Enter adds the kind marked as its button of the tool bar does: after the Transform chosen, before the Merge that it fed.
    fireEvent.keyDown(field(), { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('mosaic'), links: [link('n3', '', 'input'), link('', 'n4', 'background')] }]]);
    expect(dialog()).toBeNull();
    expect([currentNode(), nodeOf('n10').dataset.kind]).toEqual(['n10', 'mosaic']);
    // A mask taken is linked into the mask of the node chosen, the Mosaic.
    bridge.call.mockClear();
    shiftSpace();
    fireEvent.change(field(), { target: { value: 'e' } });
    expect(kinds()).toEqual(['ellipse', 'merge', 'media_in', 'delta_keyer', 'tracker', 'rectangle']);
    fireEvent.keyDown(field(), { key: 'ArrowUp' });
    expect(markedKind()).toBe('rectangle');
    fireEvent.keyDown(field(), { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('rectangle'), links: [link('', 'n10', 'mask')] }]]);
    expect(linksDrawn()).toContain('n11>n10:mask');
  });

  it('opens on the release of Shift+Space where an input method took its press, as a Chinese or Japanese one does', async () => {
    const { bridge } = await mount();
    choose('n3');
    bridge.call.mockClear();
    // Only the release comes through: the press of the Space was the input method's, which turns full width on and off.
    fireEvent.keyUp(document.body, { key: ' ', code: 'Space', shiftKey: true }); await settle();
    expect(dialog()).not.toBeNull();
    expect(calls(bridge, 'transport.play')).toHaveLength(0);
    fireEvent.keyDown(field(), { key: 'Escape' }); await settle();
    expect(dialog()).toBeNull();
    // A press that came through opens it, and its release does not open it again.
    shiftSpace(); await settle();
    expect(dialog()).not.toBeNull();
    fireEvent.keyDown(field(), { key: 'Escape' }); await settle();
    fireEvent.keyUp(document.body, { key: ' ', code: 'Space', shiftKey: true }); await settle();
    expect(dialog()).toBeNull();
    // The release of a Space without Shift opens nothing.
    fireEvent.keyUp(document.body, { key: ' ', code: 'Space' }); await settle();
    expect(dialog()).toBeNull();
  });

  it('closes on Escape and adds nothing, says so where nothing is named as what was typed, and is not opened from another part of the page', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    shiftSpace();
    fireEvent.change(field(), { target: { value: 'zz' } });
    expect(kinds()).toEqual([]);
    expect(marked('select-tool-none')!.textContent).toBe(en.hint.studio.fusionPage.noTool);
    expect(field().getAttribute('aria-activedescendant')).toBeNull();
    // Enter with nothing marked adds nothing, and the dialog stays.
    fireEvent.keyDown(field(), { key: 'Enter' }); await settle();
    expect(dialog()).not.toBeNull();
    fireEvent.keyDown(field(), { key: 'Escape' }); await settle();
    expect(dialog()).toBeNull();
    expect(sent(bridge)).toEqual([]);
    // A press elsewhere closes it too.
    shiftSpace();
    expect(dialog()).not.toBeNull();
    fireEvent.pointerDown(marked('fusion-inspector')!);
    expect(dialog()).toBeNull();
    // The Inspector has the focus now: Shift+Space is the transport's Space, and opens nothing.
    shiftSpace(); await settle();
    expect(dialog()).toBeNull();
    expect(calls(bridge, 'transport.play')).toHaveLength(1);
    expect(sent(bridge)).toEqual([]);
  });

  it('opens from the menus, and puts the node taken where the menu was opened', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(canvas(), { clientX: 300, clientY: 250 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Select Tool…' }));
    expect(dialog()).not.toBeNull();
    expect(document.activeElement).toBe(field());
    fireEvent.change(field(), { target: { value: 'back' } });
    fireEvent.click(options()[0]);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('background', { x: 300, y: 250 }) }]]);
    expect(dialog()).toBeNull();
    // From the menu of a node, the kind taken goes in after that node.
    bridge.call.mockClear();
    fireEvent.contextMenu(card('n4'), { clientX: 380, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Select Tool…' }));
    fireEvent.change(field(), { target: { value: 'blur' } });
    fireEvent.keyDown(field(), { key: 'Enter' }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'add_fusion_node', ...AT, node: newNode('blur', { x: 380, y: 60 }), links: [link('n4', '', 'input'), link('', 'MediaOut1', 'input')] }]]);
  });

  it('finds a kind by the name that the page shows it by and by the name that the engine gives it', () => {
    const names = en.label.studio.fusionPage.kinds;
    expect(matchingTools('', names)).toEqual([...TOOLS]);
    expect(matchingTools('  ', names)).toEqual([...TOOLS]);
    expect(matchingTools('BL', names)).toEqual(['blur']);
    expect(matchingTools('media in', names)).toEqual(['media_in']);
    expect(matchingTools('media_in', names)).toEqual(['media_in']);
    expect(matchingTools('gon', names)).toEqual(['polygon']);
    expect(matchingTools('zz', names)).toEqual([]);
    // In another language, by its name there, and by the name of the engine all the same.
    const chinese = zhCN.label.studio.fusionPage.kinds;
    expect(matchingTools(chinese.blur, chinese)).toEqual(['blur']);
    expect(matchingTools('blur', chinese)).toEqual(['blur']);
  });
});

describe('a clip on a locked track on the Fusion page', () => {
  const lock = (locked: boolean): Op[] => [{ op: 'set_track', timeline: 't1', id: 'v1', locked }];

  it('is shown, and nothing of its composition is changed: the node editor and the Inspector are off, and say why', async () => {
    const { bridge, outside } = await mount();
    choose('n4');
    key('c', { code: 'KeyC', ctrlKey: true });
    expect(marked('fusion-locked')).toBeNull();
    await outside(lock(true));
    bridge.call.mockClear();
    // The page says why, in the node editor and in the Inspector, in the words of the Inspector of the Edit page.
    expect(marked('fusion-locked')!.textContent).toBe(en.hint.studio.inspector.locked);
    expect(marked('fusion-inspector-locked')!.textContent).toBe(en.hint.studio.inspector.locked);
    // The composition is shown as it is, and its settings too.
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5']);
    expect([inspector().dataset.node, box('blend').value]).toEqual(['n4', '1.000']);
    // The buttons of the tool bar are off, and so is every field, switch and button of the Inspector that changes something.
    expect([...document.querySelectorAll<HTMLButtonElement>('.fusion-add button')].map(each => each.disabled)).toEqual(Array.from({ length: 11 }, () => true));
    expect([box('blend').disabled, slider('blend').disabled, row('apply').querySelector('select')!.disabled, row('label').querySelector('input')!.disabled])
      .toEqual([true, true, true, true]);
    expect(keysOf('blend')[1].disabled).toBe(true);
    // The keys that change something do nothing, and open neither the field of a name nor Select Tool.
    key('Delete'); key('p', { code: 'KeyP', ctrlKey: true }); key('x', { code: 'KeyX', ctrlKey: true }); key('v', { code: 'KeyV', ctrlKey: true });
    key('d', { code: 'KeyD', ctrlKey: true }); key('F2'); key(' ', { code: 'Space', shiftKey: true });
    await settle();
    expect(sent(bridge)).toEqual([]);
    expect(screen.queryByRole('textbox', { name: 'Node label' })).toBeNull();
    expect(screen.queryByRole('dialog', { name: 'Select Tool' })).toBeNull();
    expect(nodesDrawn()).toEqual(['MediaIn1', 'MediaOut1', 'n3', 'n4', 'n5']);
    // A drag moves no node, draws no link and takes up the end of none; a press still chooses.
    dragFrom(card('n3'), P.n3, { x: P.n3.x + 40, y: P.n3.y + 40 });
    expect([currentNode(), placeOf('n3')]).toEqual(['n3', [176, 28]]);
    dragFrom(port('n5', 'out'), P.n5Out, P.out1In);
    expect([currentNode(), marked('wire')]).toEqual(['n5', null]);
    dragFrom(half('n4>MediaOut1:input', 'end'), { x: 450, y: 57 }, { x: 450, y: 250 });
    await settle();
    expect(sent(bridge)).toEqual([]);
    expect(linksDrawn()).toEqual(['n4>MediaOut1:input', 'MediaIn1>n3:input', 'n3>n4:background', 'n5>n4:foreground']);
    // Ctrl+A chooses every node still; and of the commands of a menu, Copy alone is left.
    key('a', { code: 'KeyA', ctrlKey: true });
    expect(chosenNodes()).toHaveLength(4);
    fireEvent.contextMenu(card('n4'), { clientX: P.n4.x, clientY: P.n4.y });
    const off = (name: string) => (screen.getByRole('menuitem', { name }) as HTMLButtonElement).disabled;
    expect(['Add Node', 'Select Tool…', 'Cut', 'Paste', 'Duplicate', 'Rename', 'Pass Through', 'Delete Node'].map(off)).toEqual(Array.from({ length: 8 }, () => true));
    expect(off('Copy')).toBe(false);
    // Unlocked again, the page changes the composition as before. (The press puts the menu away.)
    await outside(lock(false));
    expect([marked('fusion-locked'), marked('fusion-inspector-locked')]).toEqual([null, null]);
    choose('n4');
    expect([(marked('add-merge') as HTMLButtonElement).disabled, box('blend').disabled]).toEqual([false, false]);
    key('p', { code: 'KeyP', ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_fusion_node', ...AT, id: 'n4', pass_through: true }]]);
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
