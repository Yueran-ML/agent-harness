// @vitest-environment jsdom
/**
 * The node editor of the Color page as it is edited, as DaVinci Resolve's is (Reference Manual 20,
 * Chapter 143; the interaction reference, §2 and §9.4): each key and each menu sends its edit of the
 * grade shown, as one transaction; links are drawn and taken away by dragging their ends; nodes are
 * chosen one by one or with a box; nodes and grades are copied and pasted; the level chosen is the
 * grade edited; refusals are said; undo and redo go to the one history of the engine.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import type { EngineBridge } from '../src/bridge';
import { Studio } from '../src/studio/Studio';
import {
  applyOp, ProjectCopy, type Corrector, type EngineEvent, type Grade, type GradeNode, type Link, type Media, type Op, type Project, type Target, type Timeline,
  type TransportState,
} from '../src/studio/engine';
import { inputPoint, layoutGrade, linkHalves, linkNear, nodesIn, outputPoint, portAt, type Layout, type PlacedNode } from '../src/studio/color/graph';
import {
  addInput, addLoose, addNode, applyPrior, cleanupNodes, defaultNodeOf, deleteNodes, morph, moveNodes, newCorrector, nodeKey, onDefaultNode, pageKey, pasteNode,
  reset, stepNode, swapNodes, toggleAll, toggleLock, withDefaultNode, type DefaultNode, type NodeCommand,
} from '../src/studio/color/nodeEdits';
import { setNodeLut } from '../src/studio/color/luts';
import { newWindow } from '../src/studio/color/windowEdits';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  // Every part of the page lies at the top left, so a point of the graph is a point of the window at zoom 1.
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; });

const corrector = (id: string, given: { label?: string; enabled?: boolean; matched?: boolean } = {}): GradeNode => ({
  id, kind: 'corrector', label: given.label ?? '', enabled: given.enabled ?? true,
  params: { ...newCorrector(), shot_match: given.matched ? { gamma: { r: 1, g: 1, b: 1 }, gain: { r: 1.1, g: 1, b: 1 }, offset: { r: 0, g: 0, b: 0 }, contrast: 1, pivot: 0.435 } : null },
});
const parallel = (id: string, inputs = 2): GradeNode => ({ id, kind: 'parallel_mixer', label: '', enabled: true, params: { inputs } });
const layer = (id: string): GradeNode => ({ id, kind: 'layer_mixer', label: '', enabled: true, params: { inputs: 2, composite: 'normal' } });
const link = (from: string, to: string, input = 0, kind: Link['kind'] = 'rgb'): Link => ({ from, to, kind, input });

/** 1 and 2 in series, 2 split to 3 and 4 in parallel into the mixer 5, which goes to the output; 4 has a shot match. */
const parallelGrade = (): Grade => ({
  nodes: [corrector('n1', { label: 'Skin' }), corrector('n2'), corrector('n3'), corrector('n4', { matched: true }), parallel('n5')],
  links: [link('source', 'n1'), link('n1', 'n2'), link('n2', 'n3'), link('n2', 'n4'), link('n3', 'n5'), link('n4', 'n5', 1), link('n5', 'output')],
});
const groupGrade = (id: string): Grade => ({ nodes: [corrector(id)], links: [link('source', id), link(id, 'output')] });
const m2: Media = { ...media, id: 'm2', name: 'Street' };
const timelineOf = (first: Grade = parallelGrade()): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [
      clip('c1', 0, 100, { grade: first }),
      clip('c2', 100, 100, { group: 'g1', grade: groupGrade('c2n') }),
      clip('c3', 200, 100),
    ] },
  ],
  grade: groupGrade('t1n'),
});
const C1: Target = { level: 'clip', timeline: 't1', clip: 'c1' };

/**
 * An engine that keeps a project, applies the operations a transaction sends, works out `add_serial`
 * as the real one does, and refuses what a test tells it to, as the real one refuses.
 */
function fakeEngine(timeline: Timeline = timelineOf()) {
  let project: Project = { schema: 3, next_id: 10, bins: [], media: [media, m2], timelines: [timeline], groups: [{ id: 'g1', name: 'Day', pre: groupGrade('pre'), post: groupGrade('post') }] };
  let revision = 0;
  let refusal: Record<string, unknown> | null = null;
  let frozen = false;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
  const gradeAt = (target: Target): Grade => {
    if (target.level === 'timeline') return project.timelines[0].grade;
    if (target.level === 'clip') return project.timelines[0].tracks.flatMap(track => (track.kind === 'subtitle' ? [] : track.clips)).find(each => each.id === target.clip)!.grade;
    const group = project.groups.find(each => each.id === target.group)!;
    return target.level === 'pre_clip' ? group.pre : group.post;
  };
  const expand = (change: Record<string, unknown>): Op[] => {
    if ('op' in change) return [change as unknown as Op];
    if (change.edit === 'add_serial') {
      const grade = change.grade as Target;
      const after = change.node as string;
      const id = `n${project.next_id}`;
      const fed = gradeAt(grade).links.filter(each => each.from === after && each.kind === 'rgb');
      const node = corrector(id);
      if (change.windows && node.kind === 'corrector') node.params = { ...node.params, windows: change.windows as Corrector['windows'] };
      return [{ op: 'add_node', grade, node, index: gradeAt(grade).nodes.length,
        links: [link(after, id), ...fed.map(each => ({ ...each, from: id }))] }];
    }
    if (change.edit === 'add_outside') {
      // A corrector after the node, on its picture and its key, with its key input inverted (p. 3341).
      const grade = change.grade as Target;
      const after = change.node as string;
      const id = `n${project.next_id}`;
      const fed = gradeAt(grade).links.filter(each => each.from === after && each.kind === 'rgb');
      const plain = corrector(id);
      const params = plain.params as Extract<GradeNode, { kind: 'corrector' }>['params'];
      const node = { ...plain, params: { ...params, key: { ...params.key, input: { ...params.key.input, invert: true } } } } as GradeNode;
      return [{ op: 'add_node', grade, node, index: gradeAt(grade).nodes.length,
        links: [link(after, id), link(after, id, 0, 'key'), ...fed.map(each => ({ ...each, from: id }))] }];
    }
    return [];
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
        // Each change is worked out on the project as the ones before it in the transaction left it, as the engine does.
        const done: Op[] = [];
        for (const change of frozen ? [] : params!.ops as Record<string, unknown>[]) {
          for (const op of expand(change)) {
            project = applyOp(project, op); done.push(op);
            if (op.op === 'add_node') project = { ...project, next_id: project.next_id + 1 };
          }
        }
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
    refuse: (data: Record<string, unknown> | null) => { refusal = data; },
    freeze: (on: boolean) => { frozen = on; },
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
  };
}
async function mount({ words, timeline }: { words?: typeof zhCN; timeline?: Timeline } = {}) {
  const engine = fakeEngine(timeline);
  const tree = <Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />;
  const view = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle(); fireEvent.click(screen.getByRole('tab', { name: words ? '调色' : 'Color' })); await settle();
  return { ...engine, ...view };
}

const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
/** The changes of each transaction sent. */
const sent = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'transaction.apply').map(params => (params as { ops: unknown[] }).ops);
const graph = () => document.querySelector<SVGElement>('[data-test="grade-graph"]')!;
const canvas = () => document.querySelector<HTMLElement>('[data-test="node-canvas"]')!;
const nodeOf = (id: string) => graph().querySelector<SVGElement>(`[data-node="${id}"]`)!;
const currentNode = () => graph().querySelector<SVGElement>('[data-current]')?.dataset.node ?? null;
const chosenNodes = () => [...graph().querySelectorAll<SVGElement>('[data-chosen]')].map(each => each.dataset.node);
const problem = () => document.querySelector('[data-test="node-problem"]')?.textContent ?? null;
const key = (value: string, fields: Record<string, unknown> = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
const letter = (value: string, fields: Record<string, unknown> = {}) => key(value, { code: `Key${value.toUpperCase()}`, ...fields });
/** A press of the node editor, which gives it the focus of the page (p. 2967), on a node or anywhere in it. */
const press = (element: Element, at: { x: number; y: number } = { x: 1, y: 1 }, fields: Record<string, unknown> = {}) => {
  fireEvent.pointerDown(element, { button: 0, clientX: at.x, clientY: at.y, ...fields });
  fireEvent.pointerUp(canvas(), { button: 0, clientX: at.x, clientY: at.y, ...fields });
};
/** A drag in the node editor, from an element at a point to another point. */
const dragFrom = (element: Element, from: { x: number; y: number }, to: { x: number; y: number }, fields: Record<string, unknown> = {}) => {
  fireEvent.pointerDown(element, { button: 0, clientX: from.x, clientY: from.y, ...fields });
  fireEvent.pointerMove(canvas(), { clientX: (from.x + to.x) / 2, clientY: (from.y + to.y) / 2, ...fields });
  fireEvent.pointerMove(canvas(), { clientX: to.x, clientY: to.y, ...fields });
  fireEvent.pointerUp(canvas(), { clientX: to.x, clientY: to.y, ...fields });
};
const placedOf = (layout: Layout, id: string): PlacedNode => layout.nodes.find(each => each.node.id === id)!;
const center = (layout: Layout, id: string) => { const placed = placedOf(layout, id); return { x: placed.x + 46, y: placed.y + 29 }; };
const focusNodes = () => press(canvas(), { x: 590, y: 290 });

describe('the keys of the node editor', () => {
  it('add a node as DaVinci Resolve adds one, about the current node, and the new one becomes current', async () => {
    const { bridge } = await mount();
    press(nodeOf('n3'));
    bridge.call.mockClear();
    letter('s', { altKey: true }); await settle();
    letter('s', { shiftKey: true }); await settle();
    letter('k', { altKey: true }); await settle();
    letter('p', { altKey: true }); await settle();
    letter('l', { altKey: true }); await settle();
    // Alt+O, an Outside node of the current one (pp. 3318, 3341).
    letter('o', { altKey: true }); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'add_serial', grade: C1, node: 'n3' }],
      // Alt+S made n10 after n3, and it is current now.
      [{ edit: 'add_serial_before', grade: C1, node: 'n10' }],
      [{ edit: 'append_node', grade: C1 }],
      [{ edit: 'add_parallel', grade: C1, node: 'n10' }],
      [{ edit: 'add_layer', grade: C1, node: 'n10' }],
      [{ edit: 'add_outside', grade: C1, node: 'n10' }],
    ]);
    // The Outside node, made after n10, is current now, as each node added is.
    expect(currentNode()).toBe('n11');
    expect(graph().querySelector('[data-node="n10"] [data-test="node-number"]')!.textContent).toBe('06');
  });

  it('extract, enable, disable, reset and delete, each one transaction', async () => {
    const { bridge } = await mount();
    press(nodeOf('n2'));
    bridge.call.mockClear();
    letter('e'); await settle();
    letter('d', { ctrlKey: true }); await settle();
    letter('d', { altKey: true }); await settle();
    key('Home', { shiftKey: true }); await settle();
    key('Home', { ctrlKey: true, shiftKey: true }); await settle();
    key('Home', { ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'extract_node', grade: C1, node: 'n2' }],
      [{ op: 'set_node', grade: C1, id: 'n2', enabled: false }],
      // n2 is off already: all the others go off, as Alt+D turns all off where any is on (p. 3315).
      [...['n1', 'n3', 'n4', 'n5'].map(id => ({ op: 'set_node', grade: C1, id, enabled: false }))],
      [{ edit: 'reset_node', grade: C1, node: 'n2' }],
      [...['n1', 'n2', 'n3', 'n4', 'n5'].map(id => ({ edit: 'reset_node', grade: C1, node: id }))],
      [{ op: 'set_grade', grade: C1, value: { nodes: [], links: [link('source', 'output')] } }],
    ]);
    // Shift+Home resets the node; it does not take the playhead to the first frame.
    expect(calls(bridge, 'transport.seek')).toEqual([]);
  });

  it('go back to the grade as it was when the clip was chosen with Alt+Shift+O (Original Memory)', async () => {
    const { bridge } = await mount();
    letter('s', { altKey: true }); await settle();
    expect(graph().querySelectorAll('[data-node]')).toHaveLength(6);
    bridge.call.mockClear();
    letter('o', { altKey: true, shiftKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_grade', grade: C1, value: parallelGrade() }]]);
    expect(graph().querySelectorAll('[data-node]')).toHaveLength(5);
    // As it was, there is nothing to go back to.
    letter('o', { altKey: true, shiftKey: true }); await settle();
    expect(sent(bridge)).toHaveLength(1);
  });

  it('delete the nodes chosen with Delete where the node editor has the focus, and Backspace deletes none', async () => {
    const { bridge } = await mount();
    press(nodeOf('n3'));
    press(nodeOf('n5'), undefined, { ctrlKey: true });
    bridge.call.mockClear();
    key('Backspace'); await settle();
    expect(sent(bridge)).toEqual([]);
    key('Delete'); await settle();
    // A corrector is bridged by the engine; a mixer is taken out with nothing in its place.
    expect(sent(bridge)).toEqual([[{ op: 'remove_node', grade: C1, id: 'n3' }, { op: 'remove_node', grade: C1, id: 'n5', links: [] }]]);
    // Where another part of the page has the focus, Delete is not the node editor's.
    bridge.call.mockClear();
    fireEvent.pointerDown(document.querySelector('[data-test="color-viewer"]')!, { button: 0 });
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([]);
  });

  it('go from node to node with Alt+Shift+; and Alt+Shift+\', round from the last to the first, and send nothing', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    key(';', { code: 'Semicolon', altKey: true, shiftKey: true });
    expect(currentNode()).toBe('n5');
    key("'", { code: 'Quote', altKey: true, shiftKey: true });
    key("'", { code: 'Quote', altKey: true, shiftKey: true });
    expect(currentNode()).toBe('n2');
    expect(sent(bridge)).toEqual([]);
  });

  it('send nothing while a label is being written, nor with a modifier more than its own', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    letter('s', { altKey: true, ctrlKey: true });
    letter('e', { shiftKey: true });
    await settle();
    expect(sent(bridge)).toEqual([]);
    fireEvent.contextMenu(nodeOf('n1'), { clientX: 200, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Label' }));
    const field = screen.getByRole('textbox', { name: 'Node label' }) as HTMLInputElement;
    expect(field.value).toBe('Skin');
    fireEvent.change(field, { target: { value: 'Skin tones' } });
    fireEvent.keyDown(field, { key: 'e', code: 'KeyE' });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n1', label: 'Skin tones' }]]);
    // A double-click on the label writes it again (p. 3313); Escape puts the field away and sends nothing.
    fireEvent.doubleClick(nodeOf('n1').querySelector('[data-test="node-label"]')!);
    const again = screen.getByRole('textbox', { name: 'Node label' });
    fireEvent.change(again, { target: { value: 'Faces' } });
    fireEvent.keyDown(again, { key: 'Escape' });
    await settle();
    expect(screen.queryByRole('textbox', { name: 'Node label' })).toBeNull();
    expect(sent(bridge)).toHaveLength(1);
  });
});

describe('links', () => {
  it('are drawn from an output to an input of the same kind, the input and the kind as the ports say', async () => {
    const { bridge } = await mount();
    const layout = layoutGrade(parallelGrade());
    bridge.call.mockClear();
    // The key of 1 into the key of 3.
    dragFrom(nodeOf('n1').querySelector('[data-port="out:key"]')!, outputPoint(placedOf(layout, 'n1'), 'key'), inputPoint(placedOf(layout, 'n3'), 'key', 0));
    await settle();
    // The picture of 3 into the second input of the mixer, which 4 feeds: the engine puts the old link out.
    dragFrom(nodeOf('n3').querySelector('[data-port="out:rgb"]')!, outputPoint(placedOf(layout, 'n3'), 'rgb'), inputPoint(placedOf(layout, 'n5'), 'rgb', 1));
    // The picture into a key input does not take: nothing is sent.
    dragFrom(nodeOf('n1').querySelector('[data-port="out:rgb"]')!, outputPoint(placedOf(layout, 'n1'), 'rgb'), inputPoint(placedOf(layout, 'n4'), 'key', 0));
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'connect', grade: C1, link: link('n1', 'n3', 0, 'key') }],
      [{ op: 'connect', grade: C1, link: link('n3', 'n5', 1) }],
    ]);
  });

  it('are drawn from an empty input to an output, and onto a mixer for one input more', async () => {
    const free: Grade = { nodes: [corrector('a'), corrector('b'), parallel('m')], links: [link('source', 'a'), link('a', 'm'), link('m', 'output')] };
    const { bridge, freeze } = await mount({ timeline: timelineOf(free) });
    const layout = layoutGrade(free);
    freeze(true);
    bridge.call.mockClear();
    dragFrom(nodeOf('b').querySelector('[data-port="in:rgb:0"]')!, inputPoint(placedOf(layout, 'b'), 'rgb', 0), outputPoint(placedOf(layout, 'a'), 'rgb'));
    await settle();
    dragFrom(nodeOf('b').querySelector('[data-port="out:rgb"]')!, outputPoint(placedOf(layout, 'b'), 'rgb'), center(layout, 'm'));
    await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'connect', grade: C1, link: link('a', 'b') }],
      [{ op: 'set_node', grade: C1, id: 'm', params: { inputs: 3 } }, { op: 'connect', grade: C1, link: link('b', 'm', 2) }],
    ]);
  });

  it('are taken away by their end dragged off, a click on their second half, or Delete with the first half chosen', async () => {
    const { bridge, freeze } = await mount();
    const layout = layoutGrade(parallelGrade());
    // The engine leaves the grade as it is, so that each gesture is tried on the same links.
    freeze(true);
    bridge.call.mockClear();
    // The end of 4 → 5 (input 1), taken up at the input and dropped in the open.
    dragFrom(nodeOf('n5').querySelector('[data-port="in:rgb:1"]')!, inputPoint(placedOf(layout, 'n5'), 'rgb', 1), { x: 560, y: 280 });
    await settle();
    // The same end taken to the output, whose link it puts out.
    const n4n5 = graph().querySelector('[data-link-hit="n4>n5:rgb:1"][data-half="end"]')!;
    const ends = { from: outputPoint(placedOf(layout, 'n4'), 'rgb'), to: inputPoint(placedOf(layout, 'n5'), 'rgb', 1) };
    dragFrom(n4n5, ends.to, { x: layout.output.x - 9, y: layout.output.y });
    await settle();
    // The second half of 1 → 2 clicked.
    press(graph().querySelector('[data-link-hit="n1>n2:rgb:0"][data-half="end"]')!);
    await settle();
    // The first half of 2 → 3 chooses it; Delete takes it away.
    press(graph().querySelector('[data-link-hit="n2>n3:rgb:0"][data-half="start"]')!);
    expect(graph().querySelector('[data-link="n2>n3:rgb:0"]')!.hasAttribute('data-chosen')).toBe(true);
    key('Delete'); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'disconnect', grade: C1, to: 'n5', kind: 'rgb', input: 1 }],
      [{ op: 'disconnect', grade: C1, to: 'n5', kind: 'rgb', input: 1 }, { op: 'connect', grade: C1, link: link('n4', 'output') }],
      [{ op: 'disconnect', grade: C1, to: 'n2', kind: 'rgb', input: 0 }],
      [{ op: 'disconnect', grade: C1, to: 'n3', kind: 'rgb', input: 0 }],
    ]);
    expect(linkHalves(ends.from, ends.to)[1]).toContain(`${ends.to.x} ${ends.to.y}`);
  });

  it('are shown apart by kind, the picture\'s and the key\'s, and a node\'s ports as its kind has them', async () => {
    const keyed: Grade = { nodes: [corrector('a'), corrector('b')], links: [link('source', 'a'), link('a', 'b'), link('a', 'b', 0, 'key'), link('b', 'output')] };
    await mount({ timeline: timelineOf(keyed) });
    expect([...graph().querySelectorAll<SVGElement>('[data-link]')].map(each => [each.dataset.link, each.dataset.kind]))
      .toEqual([['source>a:rgb:0', 'rgb'], ['a>b:rgb:0', 'rgb'], ['a>b:key:0', 'key'], ['b>output:rgb:0', 'rgb']]);
    expect([...nodeOf('a').querySelectorAll<SVGElement>('[data-port]')].map(each => [each.dataset.port, each.dataset.kind]))
      .toEqual([['in:rgb:0', 'rgb'], ['out:rgb', 'rgb'], ['in:key:0', 'key'], ['out:key', 'key']]);
  });
});

describe('choosing nodes', () => {
  it('chooses with a box drawn round them, Ctrl+click and Shift+click, the current one always among them', async () => {
    const { bridge } = await mount();
    const layout = layoutGrade(parallelGrade());
    const n3 = placedOf(layout, 'n3');
    const n4 = placedOf(layout, 'n4');
    // A box round 3 and 4, from the open graph.
    dragFrom(graph(), { x: n3.x - 10, y: n3.y - 5 }, { x: n4.x + 100, y: n4.y + 60 });
    expect(currentNode()).toBe('n1');
    expect(chosenNodes()).toEqual(['n3', 'n4']);
    // Ctrl+click takes 4 out, and puts 2 in; Shift+click adds 5.
    press(nodeOf('n4'), center(layout, 'n4'), { ctrlKey: true });
    press(nodeOf('n2'), center(layout, 'n2'), { ctrlKey: true });
    press(nodeOf('n5'), center(layout, 'n5'), { shiftKey: true });
    expect(chosenNodes()).toEqual(['n2', 'n3', 'n5']);
    bridge.call.mockClear();
    letter('d', { ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([['n1', 'n2', 'n3', 'n5'].map(id => ({ op: 'set_node', grade: C1, id, enabled: false }))]);
    // A click in the open, or on one node, chooses that alone.
    press(graph(), { x: 5, y: 280 });
    expect(chosenNodes()).toEqual([]);
    expect(nodesIn(layout, { x: n3.x - 10, y: n3.y - 5 }, { x: n4.x + 100, y: n4.y + 60 })).toEqual(['n3', 'n4']);
  });

  it('moves nodes by dragging into the project, one step to undo for the drag, and Cleanup Node Graph puts them back in one', async () => {
    const { bridge } = await mount();
    const layout = layoutGrade(parallelGrade());
    const was = placedOf(layout, 'n3');
    const card = () => nodeOf('n3').querySelector('.color-node-card')!;
    const at = () => [Number(card().getAttribute('x')), Number(card().getAttribute('y'))];
    bridge.call.mockClear();
    // Nothing is sent while the node goes along with the pointer; one set_node of pos is sent as it is dropped (T3d, p. 3321).
    fireEvent.pointerDown(nodeOf('n3'), { button: 0, clientX: center(layout, 'n3').x, clientY: center(layout, 'n3').y });
    fireEvent.pointerMove(canvas(), { clientX: center(layout, 'n3').x + 20, clientY: center(layout, 'n3').y + 60 });
    fireEvent.pointerMove(canvas(), { clientX: center(layout, 'n3').x + 40, clientY: center(layout, 'n3').y + 120 });
    expect(sent(bridge)).toEqual([]);
    expect(at()).toEqual([was.x + 40, was.y + 120]);
    fireEvent.pointerUp(canvas(), { clientX: center(layout, 'n3').x + 40, clientY: center(layout, 'n3').y + 120 });
    // Until the engine tells of it, the node stays where it was dropped.
    expect(at()).toEqual([was.x + 40, was.y + 120]);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n3', pos: { x: was.x + 40, y: was.y + 120 } }]]);
    expect(at()).toEqual([was.x + 40, was.y + 120]);
    // Two chosen nodes dragged together: one transaction of two.
    press(nodeOf('n1'));
    press(nodeOf('n2'), { x: 1, y: 1 }, { ctrlKey: true });
    bridge.call.mockClear();
    const one = placedOf(layout, 'n1');
    dragFrom(nodeOf('n2'), center(layout, 'n2'), { x: center(layout, 'n2').x + 10, y: center(layout, 'n2').y + 200 });
    await settle();
    expect(sent(bridge)).toEqual([[
      { op: 'set_node', grade: C1, id: 'n1', pos: { x: one.x + 10, y: one.y + 200 } },
      { op: 'set_node', grade: C1, id: 'n2', pos: { x: placedOf(layout, 'n2').x + 10, y: placedOf(layout, 'n2').y + 200 } },
    ]]);
    // Cleanup Node Graph: every node where the editor lays it out, in one set_grade.
    bridge.call.mockClear();
    fireEvent.contextMenu(graph(), { clientX: 5, clientY: 280 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Cleanup Node Graph' }));
    await settle();
    const [[cleanup]] = sent(bridge) as [[{ op: string; value: Grade }]];
    expect(cleanup.op).toBe('set_grade');
    expect(cleanup.value.nodes.map(node => node.pos)).toEqual([null, null, null, null, null]);
    expect(cleanup.value.nodes.map(node => node.id)).toEqual(['n1', 'n2', 'n3', 'n4', 'n5']);
    expect(at()).toEqual([was.x, was.y]);
    // A grade laid out already sends nothing.
    bridge.call.mockClear();
    fireEvent.contextMenu(graph(), { clientX: 5, clientY: 280 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Cleanup Node Graph' }));
    await settle();
    expect(sent(bridge)).toEqual([]);
  });

  it('puts a node back where it was where the engine refuses the move', async () => {
    const { bridge, refuse } = await mount();
    const layout = layoutGrade(parallelGrade());
    const was = placedOf(layout, 'n3');
    refuse({ reason: 'out_of_range', what: 'pos' });
    dragFrom(nodeOf('n3'), center(layout, 'n3'), { x: center(layout, 'n3').x + 40, y: center(layout, 'n3').y + 120 });
    await settle();
    expect(sent(bridge).length).toBeGreaterThan(0);
    const card = nodeOf('n3').querySelector('.color-node-card')!;
    expect([Number(card.getAttribute('x')), Number(card.getAttribute('y'))]).toEqual([was.x, was.y]);
  });

  it('lays out the nodes where the project puts them', async () => {
    const grade = parallelGrade();
    grade.nodes[2] = { ...grade.nodes[2], pos: { x: 400, y: 250 } };
    await mount({ timeline: timelineOf(grade) });
    const card = nodeOf('n3').querySelector('.color-node-card')!;
    expect([Number(card.getAttribute('x')), Number(card.getAttribute('y'))]).toEqual([400, 250]);
    const auto = placedOf(layoutGrade(parallelGrade()), 'n1');
    expect(Number(nodeOf('n1').querySelector('.color-node-card')!.getAttribute('x'))).toBe(auto.x);
  });

  it('enables or disables a node by a click on its number', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    press(nodeOf('n4').querySelector('[data-number-hit]')!);
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n4', enabled: false }]]);
    expect(currentNode()).toBe('n4');
  });
});

describe('copying', () => {
  it('copies a node with Ctrl+C and pastes it onto another with Ctrl+V, in this grade or another clip\'s', async () => {
    const { bridge } = await mount();
    press(nodeOf('n1'));
    letter('c', { ctrlKey: true });
    press(nodeOf('n3'));
    bridge.call.mockClear();
    letter('v', { ctrlKey: true }); await settle();
    // To the next clip, whose node the paste goes onto.
    key('ArrowDown'); await settle();
    press(nodeOf('c2n'));
    letter('v', { ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'paste_node', from: C1, from_node: 'n1', to: C1, to_node: 'n3' }],
      [{ edit: 'paste_node', from: C1, from_node: 'n1', to: { level: 'clip', timeline: 't1', clip: 'c2' }, to_node: 'c2n' }],
    ]);
  });

  it('copies a node\'s settings with Alt+drag onto another, and swaps two with Ctrl+drag', async () => {
    const { bridge } = await mount();
    const layout = layoutGrade(parallelGrade());
    bridge.call.mockClear();
    dragFrom(nodeOf('n1'), center(layout, 'n1'), center(layout, 'n2'), { altKey: true });
    await settle();
    dragFrom(nodeOf('n3'), center(layout, 'n3'), center(layout, 'n4'), { ctrlKey: true });
    await settle();
    const ops = sent(bridge);
    expect(ops[0]).toEqual([{ edit: 'paste_node', from: C1, from_node: 'n1', to: C1, to_node: 'n2' }]);
    expect(ops[1].map(op => [(op as { id: string }).id, ((op as { params: Corrector }).params).shot_match !== null])).toEqual([['n3', true], ['n4', false]]);
    // Neither moved.
    expect(Number(nodeOf('n1').querySelector('.color-node-card')!.getAttribute('x'))).toBe(placedOf(layout, 'n1').x);
  });

  it('copies a whole grade in the thumbnail timeline, and pastes it onto the clips chosen; = and - apply the grade of a clip before', async () => {
    const { bridge, notify } = await mount();
    const thumb = (id: string) => document.querySelector<HTMLElement>(`[data-test="thumbnails"] > [data-clip="${id}"] .color-thumb-frame`)!;
    fireEvent.pointerDown(thumb('c1'), { button: 0, clientX: 10, pointerId: 1 }); fireEvent.pointerUp(thumb('c1'), { clientX: 10, pointerId: 1 });
    letter('c', { ctrlKey: true });
    fireEvent.pointerDown(thumb('c2'), { button: 0, clientX: 10, pointerId: 1 }); fireEvent.pointerUp(thumb('c2'), { clientX: 10, pointerId: 1 });
    await settle();
    fireEvent.pointerDown(thumb('c3'), { button: 0, clientX: 10, ctrlKey: true, pointerId: 1 }); fireEvent.pointerUp(thumb('c3'), { clientX: 10, ctrlKey: true, pointerId: 1 });
    await settle();
    bridge.call.mockClear();
    letter('v', { ctrlKey: true }); await settle();
    const clipOf = (id: string): Target => ({ level: 'clip', timeline: 't1', clip: id });
    expect(sent(bridge)).toEqual([[{ edit: 'copy_grade', from: C1, to: [clipOf('c2'), clipOf('c3')] }]]);
    await notify({ frame: 250 }); await settle();
    bridge.call.mockClear();
    key('='); await settle();
    key('-'); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'copy_grade', from: clipOf('c2'), to: [clipOf('c3')] }],
      [{ edit: 'copy_grade', from: C1, to: [clipOf('c3')] }],
    ]);
  });
});

describe('the menus', () => {
  it('of a node add nodes, label, delete, reset, and take away a shot match', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    const open = (id: string) => fireEvent.contextMenu(nodeOf(id), { clientX: 200, clientY: 60 });
    open('n4');
    expect(currentNode()).toBe('n4');
    const menu = screen.getByRole('menu', { name: 'Node menu' });
    expect(within(menu).getAllByRole('menuitem').map(each => [each.getAttribute('aria-label'), each.getAttribute('aria-keyshortcuts')])).toEqual([
      ['Add Node', null], ['Add Outside Node', 'Alt+O'], ['Change Label', null], ['Delete Node', 'Delete'], ['Reset Node Grade', 'Shift+Home'],
      // Lock Node (T3d, p. 3382).
      ['Lock Node', null],
      // The LUT of the node (T18, p. 3412).
      ['LUT', null], ['Reveal Selected LUT', null],
      ['Reset Shot Match', null],
      // Append Node to Selected Clips (T26, p. 3297), which no other clip chosen leaves off.
      ['Append Node to Selected Clips', null],
    ]);
    expect((within(menu).getByRole('menuitem', { name: 'Append Node to Selected Clips' }) as HTMLButtonElement).disabled).toBe(true);
    // Add Outside Node, of a corrector (p. 3341: "Right-click a node choose Add Outside Node").
    expect((within(menu).getByRole('menuitem', { name: 'Add Outside Node' }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Add Node' }));
    expect(within(screen.getByRole('menu', { name: 'Add Node' })).getAllByRole('menuitem').map(each => [each.getAttribute('aria-label'), each.getAttribute('aria-keyshortcuts')]))
      .toEqual([['Add Serial', 'Alt+S'], ['Add Serial Node + CPW', 'Alt+C'], ['Add Serial Node + LPW', 'Alt+Q'], ['Add Serial Node + PPW', 'Alt+G'],
        ['Add Serial Node + PCW', 'Alt+B'], ['Add Before Current', 'Shift+S'], ['Add Parallel', 'Alt+P'], ['Add Layer', 'Alt+L'], ['Add Outside Node', 'Alt+O']]);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Parallel' }));
    await settle();
    open('n4'); fireEvent.click(screen.getByRole('menuitem', { name: 'Add Outside Node' })); await settle();
    open('n4'); fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(within(screen.getByRole('menu', { name: 'Add Node' })).getByRole('menuitem', { name: 'Add Outside Node' })); await settle();
    open('n4'); fireEvent.click(screen.getByRole('menuitem', { name: 'Reset Shot Match' })); await settle();
    open('n4'); fireEvent.click(screen.getByRole('menuitem', { name: 'Delete Node' })); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'add_parallel', grade: C1, node: 'n4' }],
      [{ edit: 'add_outside', grade: C1, node: 'n4' }],
      [{ edit: 'add_outside', grade: C1, node: 'n4' }],
      [{ op: 'set_node', grade: C1, id: 'n4', params: { shot_match: null } }],
      [{ op: 'remove_node', grade: C1, id: 'n4' }],
    ]);
  });

  it('of a mixer add an input, morph it, and choose a composite mode', async () => {
    const mixed: Grade = { nodes: [corrector('a'), corrector('b'), layer('m')], links: [link('source', 'a'), link('source', 'b'), link('a', 'm'), link('b', 'm', 1), link('m', 'output')] };
    const { bridge } = await mount({ timeline: timelineOf(mixed) });
    bridge.call.mockClear();
    const open = () => fireEvent.contextMenu(nodeOf('m'), { clientX: 200, clientY: 60 });
    open(); fireEvent.click(screen.getByRole('menuitem', { name: 'Add One Input' })); await settle();
    open(); fireEvent.click(screen.getByRole('menuitem', { name: 'Composite Mode' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Screen' })); await settle();
    open(); fireEvent.click(screen.getByRole('menuitem', { name: 'Morph Into Parallel Node' })); await settle();
    const ops = sent(bridge);
    expect(ops[0]).toEqual([{ op: 'set_node', grade: C1, id: 'm', params: { inputs: 3 } }]);
    expect(ops[1]).toEqual([{ op: 'set_node', grade: C1, id: 'm', params: { composite: 'screen' } }]);
    expect(ops[2]).toEqual([{ op: 'set_grade', grade: C1, value: { ...mixed, nodes: [mixed.nodes[0], mixed.nodes[1], { ...parallel('m', 3), locked: false, pos: null }] } }]);
  });

  it('of the background add a loose corrector where it was asked for, and reset all; of a link, delete it', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(graph(), { clientX: 300, clientY: 250 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    fireEvent.click(screen.getByRole('menuitem', { name: 'Corrector' }));
    await settle();
    fireEvent.contextMenu(graph(), { clientX: 5, clientY: 280 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Reset All Grades and Nodes' }));
    await settle();
    const ops = sent(bridge);
    // It lies where the menu was opened, which the project keeps (T3d).
    expect(ops[0]).toEqual([{ op: 'add_node', grade: C1, node: { id: '', kind: 'corrector', label: '', enabled: true, locked: false, pos: { x: 300, y: 250 }, params: newCorrector() } }]);
    expect(ops[1]).toEqual([{ op: 'set_grade', grade: C1, value: { nodes: [], links: [link('source', 'output')] } }]);
  });

  it('of a thumbnail collapse the group grades of the clips chosen that are in a group', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    fireEvent.contextMenu(document.querySelector('[data-test="thumbnails"] > [data-clip="c1"]')!, { clientX: 50, clientY: 400 });
    expect((screen.getByRole('menuitem', { name: 'Collapse Group Grades' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Collapse Group Grades' }), { key: 'Escape' });
    fireEvent.contextMenu(document.querySelector('[data-test="thumbnails"] > [data-clip="c2"]')!, { clientX: 50, clientY: 400 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Collapse Group Grades' }));
    await settle();
    expect(sent(bridge)).toEqual([[{ edit: 'collapse_group_grades', timeline: 't1', clips: ['c2'] }]]);
  });

  it('put a loose node dropped on a link into it', async () => {
    const loose: Grade = { nodes: [corrector('a'), corrector('b'), corrector('x')], links: [link('source', 'a'), link('a', 'b'), link('b', 'output')] };
    const { bridge } = await mount({ timeline: timelineOf(loose) });
    const layout = layoutGrade(loose);
    const ab = linkHalves(outputPoint(placedOf(layout, 'a'), 'rgb'), inputPoint(placedOf(layout, 'b'), 'rgb', 0));
    const middle = ab[1].split(' ').slice(1, 3).map(Number);
    expect(linkNear(layout, { x: middle[0], y: middle[1] })).toEqual(link('a', 'b'));
    bridge.call.mockClear();
    dragFrom(nodeOf('x'), center(layout, 'x'), { x: middle[0], y: middle[1] });
    await settle();
    // Where it was dropped and its links: one transaction, one step to undo.
    const x = placedOf(layout, 'x');
    const moved = { x: Math.round(x.x + middle[0] - center(layout, 'x').x), y: Math.round(x.y + middle[1] - center(layout, 'x').y) };
    expect(sent(bridge)).toEqual([[
      { op: 'set_node', grade: C1, id: 'x', pos: moved },
      { op: 'connect', grade: C1, link: link('a', 'x') }, { op: 'connect', grade: C1, link: link('x', 'b') },
    ]]);
  });
});

describe('the level chosen', () => {
  it('is the grade that the keys and the links edit: the clip, the timeline, and the group before and after the clip', async () => {
    const { bridge, notify } = await mount();
    const level = () => screen.getByRole('combobox', { name: 'Node graph shown' }) as HTMLSelectElement;
    bridge.call.mockClear();
    fireEvent.change(level(), { target: { value: 'timeline' } });
    letter('s', { altKey: true }); await settle();
    await notify({ frame: 150 }); await settle();
    fireEvent.change(level(), { target: { value: 'pre_clip' } });
    letter('s', { altKey: true }); await settle();
    fireEvent.change(level(), { target: { value: 'post_clip' } });
    letter('k', { altKey: true }); await settle();
    fireEvent.change(level(), { target: { value: 'clip' } });
    letter('e'); await settle();
    expect(sent(bridge)).toEqual([
      [{ edit: 'add_serial', grade: { level: 'timeline', timeline: 't1' }, node: 't1n' }],
      [{ edit: 'add_serial', grade: { level: 'pre_clip', group: 'g1' }, node: 'pre' }],
      [{ edit: 'append_node', grade: { level: 'post_clip', group: 'g1' } }],
      [{ edit: 'extract_node', grade: { level: 'clip', timeline: 't1', clip: 'c2' }, node: 'c2n' }],
    ]);
  });
});

describe('refusals and the history', () => {
  it('say why the engine refused, in the words of the page and its language', async () => {
    const { bridge, refuse } = await mount();
    const layout = layoutGrade(parallelGrade());
    refuse({ reason: 'loop', node: 'n1' });
    dragFrom(nodeOf('n3').querySelector('[data-port="out:rgb"]')!, outputPoint(placedOf(layout, 'n3'), 'rgb'), inputPoint(placedOf(layout, 'n1'), 'rgb', 0));
    await settle();
    expect(sent(bridge)).toEqual([[{ op: 'connect', grade: C1, link: link('n3', 'n1') }]]);
    expect(problem()).toBe('This link would make the grade loop back on itself.');
    refuse({ reason: 'wrong_node', node: 'n5', kind: 'parallel_mixer' });
    press(nodeOf('n5'));
    letter('p', { altKey: true }); await settle();
    expect(problem()).toBe('This cannot be done with the chosen node.');
    fireEvent.click(within(document.querySelector('[data-test="node-problem"]')!).getByRole('button'));
    expect(problem()).toBeNull();
  });

  it('are said by the editor itself where it knows the engine would refuse, and nothing is sent', async () => {
    const mixers: Grade = { nodes: [parallel('m1', 1), parallel('m2', 1)], links: [link('source', 'm1'), link('m1', 'm2'), link('m2', 'output')] };
    const { bridge } = await mount({ timeline: timelineOf(mixers), words: zhCN });
    const layout = layoutGrade(mixers);
    bridge.call.mockClear();
    // Two mixers swapped with Ctrl+drag: only correctors swap what they do.
    dragFrom(nodeOf('m1'), center(layout, 'm1'), center(layout, 'm2'), { ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(problem()).toBe('所选节点不能做这个操作。');
  });

  it('undo and redo in the one history of the engine with Ctrl+Z and Ctrl+Shift+Z', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    letter('z', { ctrlKey: true }); await settle();
    letter('z', { ctrlKey: true, shiftKey: true }); await settle();
    expect(bridge.call.mock.calls.map(([method]) => method).filter(method => method.startsWith('history.'))).toEqual(['history.undo', 'history.redo']);
  });
});

describe('the Color page on other systems', () => {
  it('is not there on macOS, so no key of the node editor sends anything', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    const engine = fakeEngine();
    render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
    await settle();
    expect(document.querySelector('[data-test="node-editor"]')).toBeNull();
    letter('s', { altKey: true }); letter('z', { ctrlKey: true }); await settle();
    expect(calls(engine.bridge, 'transaction.apply')).toEqual([]);
    expect(calls(engine.bridge, 'history.undo')).toEqual([]);
  });
});

describe('the rules of the node editor', () => {
  const k = (value: string, fields: Partial<{ shift: boolean; ctrl: boolean; alt: boolean; code: string; repeat: boolean }> = {}) => ({
    key: value, shift: false, ctrl: false, alt: false, meta: false, repeat: false, composing: false, ...fields,
  });
  it('read each key of p. 3318 and its neighbours by where it lies', () => {
    const table: [ReturnType<typeof k>, NodeCommand | null][] = [
      [k('ß', { alt: true, code: 'KeyS' }), { kind: 'add', how: 'serial' }],
      [k('S', { shift: true, code: 'KeyS' }), { kind: 'add', how: 'before' }],
      [k('k', { alt: true, code: 'KeyK' }), { kind: 'add', how: 'append' }],
      [k('p', { alt: true, code: 'KeyP' }), { kind: 'add', how: 'parallel' }],
      [k('l', { alt: true, code: 'KeyL' }), { kind: 'add', how: 'layer' }],
      [k('o', { alt: true, code: 'KeyO' }), { kind: 'add', how: 'outside' }],
      [k('ø', { alt: true, code: 'KeyO' }), { kind: 'add', how: 'outside' }],
      [k('e', { code: 'KeyE' }), { kind: 'extract' }],
      [k('d', { ctrl: true, code: 'KeyD' }), { kind: 'enable' }],
      [k('d', { alt: true, code: 'KeyD' }), { kind: 'enableAll' }],
      [k('Home', { shift: true }), { kind: 'reset', how: 'node' }],
      [k('Home', { ctrl: true, shift: true }), { kind: 'reset', how: 'keepNodes' }],
      [k('Home', { ctrl: true }), { kind: 'reset', how: 'all' }],
      [k('Home'), null],
      [k(':', { alt: true, shift: true, code: 'Semicolon' }), { kind: 'step', direction: -1 }],
      [k('"', { alt: true, shift: true, code: 'Quote' }), { kind: 'step', direction: 1 }],
      [k('O', { alt: true, shift: true, code: 'KeyO' }), { kind: 'original' }],
      [k('Delete'), { kind: 'delete', key: 'Delete' }],
      [k('Backspace'), { kind: 'delete', key: 'Backspace' }],
      [k('c', { ctrl: true, code: 'KeyC' }), { kind: 'copy' }],
      [k('v', { ctrl: true, code: 'KeyV' }), { kind: 'paste' }],
      [k('h', { code: 'KeyH' }), { kind: 'tool' }],
      [k('s', { alt: true, code: 'KeyS', repeat: true }), null],
    ];
    expect(table.map(([pressed]) => nodeKey(pressed))).toEqual(table.map(([, command]) => command));
    expect(pageKey(k('z', { ctrl: true, code: 'KeyZ' }))).toEqual({ kind: 'history', method: 'history.undo' });
    expect(pageKey(k('Z', { ctrl: true, shift: true, code: 'KeyZ' }))).toEqual({ kind: 'history', method: 'history.redo' });
    expect(pageKey(k('='))).toEqual({ kind: 'prior', back: 1 });
    expect(pageKey(k('-'))).toEqual({ kind: 'prior', back: 2 });
  });

  it('work each command out from the grade', () => {
    const grade = parallelGrade();
    expect(addNode('serial', C1, null)).toEqual([{ edit: 'add_serial', grade: C1, node: 'source' }]);
    expect(addNode('before', C1, null)).toEqual([{ edit: 'add_serial_before', grade: C1, node: 'output' }]);
    expect(addNode('parallel', C1, null)).toEqual({ refused: 'wrong_node' });
    expect(addNode('outside', C1, null)).toEqual({ refused: 'wrong_node' });
    expect(addNode('outside', C1, 'n4')).toEqual([{ edit: 'add_outside', grade: C1, node: 'n4' }]);
    expect(deleteNodes(C1, grade, ['zz'])).toBeNull();
    expect(toggleAll(C1, { ...grade, nodes: grade.nodes.map(node => ({ ...node, enabled: false })) }))
      .toEqual(grade.nodes.map(node => ({ op: 'set_node', grade: C1, id: node.id, enabled: true })));
    // Some off: all go off, and the next press puts every one on (p. 3315).
    const some = { ...grade, nodes: grade.nodes.map((node, at) => ({ ...node, enabled: at !== 1 })) };
    expect(toggleAll(C1, some)).toEqual(['n1', 'n3', 'n4', 'n5'].map(id => ({ op: 'set_node', grade: C1, id, enabled: false })));
    expect(reset('all', C1, { nodes: [], links: [link('source', 'output')] }, null)).toBeNull();
    expect(swapNodes(C1, grade, 'n1', 'n5')).toEqual({ refused: 'wrong_node' });
    expect(stepNode(grade, 'n5', 1)).toBe('n1');
    expect(stepNode(grade, 'n1', -1)).toBe('n5');
    const clips = timelineOf().tracks[0].kind === 'video' ? (timelineOf().tracks[0] as { clips: ReturnType<typeof clip>[] }).clips : [];
    expect(applyPrior('t1', clips, 'c2', 2)).toBeNull();
    expect(applyPrior('t1', clips, 'c2', 1)).toEqual([{ edit: 'copy_grade', from: C1, to: [{ level: 'clip', timeline: 't1', clip: 'c2' }] }]);
    const layout = layoutGrade(grade);
    expect(portAt(layout, inputPoint(placedOf(layout, 'n5'), 'rgb', 1))).toEqual({ node: 'n5', side: 'in', kind: 'rgb', input: 1 });
    expect(portAt(layout, { x: layout.source.x + 9, y: layout.source.y })).toEqual({ node: 'source', side: 'out', kind: 'rgb', input: 0 });
    // A node dragged stands where it was put, and the output stays right of it.
    const moved = layoutGrade(grade, { n5: { x: 2000, y: 400 } });
    expect([placedOf(moved, 'n5').x, placedOf(moved, 'n5').y]).toEqual([2000, 400]);
    expect(moved.output.x).toBeGreaterThan(2000 + 92);
    expect(moved.height).toBeGreaterThanOrEqual(400 + 58);
  });
});

describe('Lock Node (T3d, p. 3382)', () => {
  const lockedGrade = (): Grade => {
    const grade = parallelGrade();
    grade.nodes[1] = { ...grade.nodes[1], locked: true };
    return grade;
  };

  it('draws a lock on a locked node, and locks and unlocks a node from its menu, one transaction each', async () => {
    const { bridge } = await mount({ timeline: timelineOf(lockedGrade()) });
    expect(nodeOf('n2').hasAttribute('data-locked')).toBe(true);
    expect(nodeOf('n2').querySelector('[data-test="node-locked"]')).not.toBeNull();
    expect(nodeOf('n2').querySelector('title')!.textContent).toContain('Locked');
    expect(nodeOf('n1').querySelector('[data-test="node-locked"]')).toBeNull();
    bridge.call.mockClear();
    fireEvent.contextMenu(nodeOf('n2'), { clientX: 200, clientY: 60 });
    const item = screen.getByRole('menuitem', { name: 'Lock Node' });
    expect(item.dataset.checked).toBe('true');
    fireEvent.click(item); await settle();
    fireEvent.contextMenu(nodeOf('n1'), { clientX: 200, clientY: 60 });
    expect(screen.getByRole('menuitem', { name: 'Lock Node' }).dataset.checked).toBeUndefined();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Lock Node' })); await settle();
    expect(sent(bridge)).toEqual([
      [{ op: 'set_node', grade: C1, id: 'n2', locked: false }],
      [{ op: 'set_node', grade: C1, id: 'n1', locked: true }],
    ]);
    expect(nodeOf('n2').hasAttribute('data-locked')).toBe(false);
    expect(nodeOf('n1').hasAttribute('data-locked')).toBe(true);
  });

  it('keeps the settings of a locked node: a reset of it is refused by the editor, and the reset of every node passes it over', async () => {
    const { bridge } = await mount({ timeline: timelineOf(lockedGrade()), words: zhCN });
    press(nodeOf('n2'));
    bridge.call.mockClear();
    key('Home', { shiftKey: true }); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(problem()).toBe('这个节点已锁定，解锁后才能修改它的设置。');
    key('Home', { shiftKey: true, ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([['n1', 'n3', 'n4', 'n5'].map(node => ({ edit: 'reset_node', grade: C1, node }))]);
    // Pasted onto, or swapped with, it is refused too.
    press(nodeOf('n1'));
    letter('c', { ctrlKey: true });
    press(nodeOf('n2'));
    bridge.call.mockClear();
    focusNodes(); press(nodeOf('n2'));
    letter('v', { ctrlKey: true }); await settle();
    const layout = layoutGrade(lockedGrade());
    dragFrom(nodeOf('n3'), center(layout, 'n3'), center(layout, 'n2'), { ctrlKey: true }); await settle();
    expect(sent(bridge)).toEqual([]);
    expect(problem()).toBe('这个节点已锁定，解锁后才能修改它的设置。');
  });

  it('says the refusal of the engine of a locked node in the words of the page', async () => {
    const { bridge, refuse } = await mount();
    refuse({ reason: 'node_locked', node: 'n1' });
    press(nodeOf('n1'));
    bridge.call.mockClear();
    key('Home', { shiftKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ edit: 'reset_node', grade: C1, node: 'n1' }]]);
    expect(problem()).toBe('This node is locked. Unlock it to change its settings.');
  });
});

describe('the default node 01 (T3d, pp. 3309, 3312)', () => {
  const empty = (): Grade => ({ nodes: [], links: [link('source', 'output')] });
  const make = { edit: 'add_serial', grade: C1, node: 'source' };

  it('shows a grade without nodes with an ungraded corrector 01, named as the engine will name it, which nothing that would leave it so makes', async () => {
    const { bridge } = await mount({ timeline: timelineOf(empty()) });
    expect([...graph().querySelectorAll<SVGElement>('[data-node]')].map(each => [each.dataset.node, each.querySelector('[data-test="node-number"]')!.textContent]))
      .toEqual([['n10', '01']]);
    expect(nodeOf('n10').hasAttribute('data-default')).toBe(true);
    expect(currentNode()).toBe('n10');
    expect(document.querySelector('[data-test="no-nodes"]')).toBeNull();
    // Its thumbnail is not marked graded: the grade has no node.
    expect(document.querySelector('[data-test="thumbnails"] > [data-clip="c1"] [data-test="thumb-number"]')!.hasAttribute('data-graded')).toBe(false);
    focusNodes(); press(nodeOf('n10'));
    bridge.call.mockClear();
    key('Delete'); await settle();
    key('Home', { shiftKey: true }); await settle();
    key('Home', { shiftKey: true, ctrlKey: true }); await settle();
    key('Home', { ctrlKey: true }); await settle();
    fireEvent.contextMenu(graph(), { clientX: 5, clientY: 280 });
    expect((screen.getByRole('menuitem', { name: 'Reset All Grades and Nodes' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Cleanup Node Graph' })); await settle();
    expect(sent(bridge)).toEqual([]);
  });

  it('makes the node with the first change to it, in the transaction of the change: a drag, a label, a lock', async () => {
    const { bridge } = await mount({ timeline: timelineOf(empty()) });
    const layout = layoutGrade({ nodes: [corrector('n10')], links: [link('source', 'n10'), link('n10', 'output')] });
    const was = placedOf(layout, 'n10');
    bridge.call.mockClear();
    dragFrom(nodeOf('n10'), center(layout, 'n10'), { x: center(layout, 'n10').x + 30, y: center(layout, 'n10').y + 50 });
    await settle();
    expect(sent(bridge)).toEqual([[make, { op: 'set_node', grade: C1, id: 'n10', pos: { x: was.x + 30, y: was.y + 50 } }]]);
    // It is a node of the grade now: the next change is to it alone.
    expect(nodeOf('n10').hasAttribute('data-default')).toBe(false);
    bridge.call.mockClear();
    fireEvent.contextMenu(nodeOf('n10'), { clientX: 200, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Lock Node' })); await settle();
    expect(sent(bridge)).toEqual([[{ op: 'set_node', grade: C1, id: 'n10', locked: true }]]);
  });

  it('makes the node before a label, and before a node added after it', async () => {
    const first = await mount({ timeline: timelineOf(empty()) });
    first.bridge.call.mockClear();
    fireEvent.contextMenu(nodeOf('n10'), { clientX: 200, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Change Label' }));
    const field = screen.getByRole('textbox', { name: 'Node label' }) as HTMLInputElement;
    fireEvent.change(field, { target: { value: 'Base' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(sent(first.bridge)).toEqual([[make, { op: 'set_node', grade: C1, id: 'n10', label: 'Base' }]]);
    cleanup();
    const second = await mount({ timeline: timelineOf(empty()) });
    second.bridge.call.mockClear();
    // Alt+S on node 01 adds 02 after it, as in DaVinci: 01 is made first.
    letter('s', { altKey: true }); await settle();
    expect(sent(second.bridge)).toEqual([[make, { edit: 'add_serial', grade: C1, node: 'n10' }]]);
  });
});

describe('the default node 01 and Lock Node, as rules', () => {
  const empty: Grade = { nodes: [], links: [link('source', 'output')] };
  const shown: DefaultNode = { target: C1, id: 'n10' };
  const make = { edit: 'add_serial', grade: C1, node: 'source' };

  it('take a grade without nodes, whose source goes to its output, for one with the node the engine will add', () => {
    expect(defaultNodeOf(empty, C1, 10)).toEqual(shown);
    expect(defaultNodeOf(empty, C1, undefined)).toBeNull();
    expect(defaultNodeOf(parallelGrade(), C1, 10)).toBeNull();
    expect(defaultNodeOf({ nodes: [], links: [] }, C1, 10)).toBeNull();
    expect(withDefaultNode(empty, 'n10')).toEqual({ nodes: [{ ...corrector('n10'), locked: false, pos: null }], links: [link('source', 'n10'), link('n10', 'output')] });
    expect(withDefaultNode(parallelGrade(), 'n10')).toEqual(parallelGrade());
  });

  it('make the node first where a change is to the grade shown, and nothing where it is to the node as it is', () => {
    const lut = setNodeLut(C1, withDefaultNode(empty, 'n10').nodes[0], 'film.cube');
    expect(onDefaultNode(lut, shown)).toEqual([make, { op: 'set_node', grade: C1, id: 'n10', params: { lut: 'film.cube' } }]);
    expect(onDefaultNode([{ op: 'disconnect', grade: C1, to: 'output', kind: 'rgb', input: 0 }], shown))
      .toEqual([make, { op: 'disconnect', grade: C1, to: 'output', kind: 'rgb', input: 0 }]);
    expect(onDefaultNode([{ edit: 'append_node', grade: C1 }], shown)).toEqual([make, { edit: 'append_node', grade: C1 }]);
    expect(onDefaultNode([{ op: 'remove_node', grade: C1, id: 'n10' }], shown)).toBeNull();
    expect(onDefaultNode([{ edit: 'reset_node', grade: C1, node: 'n10' }], shown)).toBeNull();
    // A whole grade put in place, or another grade, is sent as it is.
    const whole = [{ op: 'set_grade' as const, grade: C1, value: empty }];
    expect(onDefaultNode(whole, shown)).toEqual(whole);
    const other: Target = { level: 'clip', timeline: 't1', clip: 'c2' };
    expect(onDefaultNode([{ op: 'set_node', grade: other, id: 'x', enabled: false }], shown)).toEqual([{ op: 'set_node', grade: other, id: 'x', enabled: false }]);
    // A node pasted onto it is pasted once it is made.
    const paste = [{ edit: 'paste_node' as const, from: other, from_node: 'x', to: C1, to_node: 'n10' }];
    expect(onDefaultNode(paste, shown)).toEqual([make, ...paste]);
    expect(onDefaultNode({ refused: 'wrong_node' }, shown)).toEqual({ refused: 'wrong_node' });
    expect(onDefaultNode(lut, null)).toEqual(lut);
  });

  it('lay out the first column and the step between columns as the engine moves appended nodes by (FIRST_COLUMN, NODE_STEP of grade.rs)', () => {
    const layout = layoutGrade({ nodes: [corrector('a'), corrector('b')], links: [link('source', 'a'), link('a', 'b'), link('b', 'output')] });
    expect(placedOf(layout, 'a').x).toBe(102);
    expect(placedOf(layout, 'b').x - placedOf(layout, 'a').x).toBe(148);
  });

  it('move nodes, clean them up and lock them by set_node and set_grade, and refuse the settings of a locked node', () => {
    const grade = parallelGrade();
    grade.nodes[0] = { ...grade.nodes[0], pos: { x: 10, y: 20 } };
    expect(moveNodes(C1, grade, { n1: { x: 10.2, y: 19.6 }, n2: { x: 300.4, y: 40 } })).toEqual([{ op: 'set_node', grade: C1, id: 'n2', pos: { x: 300, y: 40 } }]);
    const [clean] = cleanupNodes(C1, grade) as unknown as [{ op: 'set_grade'; value: Grade }];
    expect(clean.value.nodes.every(node => node.pos === null)).toBe(true);
    expect(cleanupNodes(C1, parallelGrade())).toBeNull();
    expect(toggleLock(C1, grade, 'n1')).toEqual([{ op: 'set_node', grade: C1, id: 'n1', locked: true }]);
    grade.nodes[4] = { ...grade.nodes[4], locked: true };
    expect(addInput(C1, grade, 'n5')).toEqual({ refused: 'node_locked' });
    expect(morph(C1, grade, 'n5')).toEqual({ refused: 'node_locked' });
    expect(pasteNode(C1, 'n1', C1, 'n5', grade)).toEqual({ refused: 'node_locked' });
    expect(reset('node', C1, grade, 'n5')).toEqual({ refused: 'node_locked' });
    expect(addLoose(C1, { x: 12.4, y: 7.6 })).toEqual([{ op: 'add_node', grade: C1, node: { id: '', kind: 'corrector', label: '', enabled: true, locked: false, pos: { x: 12, y: 8 }, params: newCorrector() } }]);
  });
});

describe('a node added with a window (Add Serial Node + CPW, LPW, PPW, PCW; p. 3187)', () => {
  const empty = (): Grade => ({ nodes: [], links: [link('source', 'output')] });
  const centerShown = () => document.querySelector<HTMLElement>('[data-test="palettes-center"] [data-palette-shown]')?.dataset.paletteShown;
  const osc = () => (document.querySelector('[data-test="viewer-osc"]') as HTMLSelectElement).value;

  it('Alt+C, Alt+Q, Alt+G and Alt+B ask for a serial node with a circular, linear, polygonal or Curve window', () => {
    const k = (code: string) => ({ key: code.slice(3).toLowerCase(), code, ctrl: false, alt: true, shift: false, meta: false, repeat: false, composing: false });
    expect(['KeyC', 'KeyQ', 'KeyG', 'KeyB'].map(code => nodeKey(k(code)))).toEqual(['circular', 'linear', 'polygon', 'curve']
      .map(window => ({ kind: 'add', how: 'serial', window })));
    // By where the key lies, as Alt changes what it types on some layouts; with Shift or Ctrl they are other keys.
    expect(nodeKey({ ...k('KeyC'), key: 'ç' })).toEqual({ kind: 'add', how: 'serial', window: 'circular' });
    expect(nodeKey({ ...k('KeyC'), shift: true })).toBeNull();
    expect(nodeKey({ ...k('KeyG'), ctrl: true })).toBeNull();
    expect(addNode('serial', C1, 'n3', 'polygon')).toEqual([{ edit: 'add_serial', grade: C1, node: 'n3', windows: [newWindow('w1', 'polygon')] }]);
    expect(addNode('serial', C1, null, 'curve')).toEqual([{ edit: 'add_serial', grade: C1, node: 'source',
      windows: [{ id: 'w1', label: '', enabled: true, invert: false, combine: 'add', opacity: 100,
        transform: { size: 50, aspect: 50, pan: 50, tilt: 50, rotate: 0 }, shape: { kind: 'curve', points: [], closed: false, inside: 0, outside: 0 } }] }]);
  });

  it('adds the node with its window in one transaction, makes it current and opens the Window palette on that window', async () => {
    const { bridge } = await mount();
    press(nodeOf('n3'));
    expect(centerShown()).not.toBe('window');
    bridge.call.mockClear();
    letter('c', { altKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ edit: 'add_serial', grade: C1, node: 'n3', windows: [newWindow('w1', 'circular')] }]]);
    expect(currentNode()).toBe('n10');
    // The Window palette opens up, ready for editing, with the viewer's controls on the window (pp. 3187, 3189).
    expect(centerShown()).toBe('window');
    expect(osc()).toBe('window');
    expect(screen.getByRole('option', { name: 'w1 · Circular' }).getAttribute('aria-selected')).toBe('true');
    for (const [code, shape, label] of [['q', 'linear', 'Linear'], ['g', 'polygon', 'Polygon'], ['b', 'curve', 'Curve']] as const) {
      const after = currentNode()!;
      bridge.call.mockClear();
      letter(code, { altKey: true }); await settle();
      expect(sent(bridge)).toEqual([[{ edit: 'add_serial', grade: C1, node: after, windows: [newWindow('w1', shape)] }]]);
      expect(currentNode()).not.toBe(after);
      expect(screen.getByRole('option', { name: `w1 · ${label}` }).getAttribute('aria-selected')).toBe('true');
    }
    expect(calls(bridge, 'history.undo')).toEqual([]);
  });

  it('on a grade shown with its default node 01, makes 01 first in the same transaction, so one undo takes both away', async () => {
    const { bridge } = await mount({ timeline: timelineOf(empty()) });
    bridge.call.mockClear();
    letter('b', { altKey: true }); await settle();
    expect(sent(bridge)).toEqual([[{ edit: 'add_serial', grade: C1, node: 'source' },
      { edit: 'add_serial', grade: C1, node: 'n10', windows: [newWindow('w1', 'curve')] }]]);
    expect(currentNode()).toBe('n11');
    expect(graph().querySelector('[data-node="n11"] [data-test="node-number"]')!.textContent).toBe('02');
    expect(centerShown()).toBe('window');
    // The Curve is an empty draft, chosen, for clicks in the viewer to draw (p. 3193).
    expect(screen.getByRole('option', { name: 'w1 · Curve' }).getAttribute('aria-selected')).toBe('true');
  });

  it('are in the Add Node menu of a node with their keys, and a plain Alt+S opens no palette', async () => {
    const { bridge } = await mount();
    press(nodeOf('n3'));
    bridge.call.mockClear();
    letter('s', { altKey: true }); await settle();
    expect(centerShown()).not.toBe('window');
    fireEvent.contextMenu(nodeOf('n3'), { clientX: 200, clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add Node' }));
    const item = screen.getByRole('menuitem', { name: /Add Serial Node \+ LPW/ });
    expect(item.textContent).toContain('Alt+Q');
    for (const [name, keys] of [['CPW', 'Alt+C'], ['PPW', 'Alt+G'], ['PCW', 'Alt+B']]) {
      expect(screen.getByRole('menuitem', { name: new RegExp(`Add Serial Node \\+ ${name}`) }).textContent).toContain(keys);
    }
    bridge.call.mockClear();
    fireEvent.click(item); await settle();
    expect(sent(bridge)).toEqual([[{ edit: 'add_serial', grade: C1, node: 'n3', windows: [newWindow('w1', 'linear')] }]]);
    expect(centerShown()).toBe('window');
  });
});
