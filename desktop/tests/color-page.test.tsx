// @vitest-environment jsdom
/**
 * The Color page: the Interface toolbar, the gallery, the viewer of the timeline and the node
 * editor, the thumbnail timeline and the mini-timeline, and the palettes, laid out as DaVinci
 * Resolve's Color page; which clip is current, the keys of the page, the grade of the current
 * clip as the node editor draws it, and the LUTs that the engine could not read.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import type { EngineBridge } from '../src/bridge';
import { Studio, studioPages } from '../src/studio/Studio';
import { MERGE_PALETTES_PX } from '../src/studio/pages/ColorPage';
import {
  applyOp, emptyGrade, ProjectCopy, sortLinks, type Corrector, type EngineEvent, type Grade, type GradeNode, type Link, type Media, type Op,
  type Project, type Target, type Timeline, type TransportState,
} from '../src/studio/engine';
import { currentClip, numberedClips, shownClips, sourceFrameAt, stepClip } from '../src/studio/color/clips';
import { isComplete, layoutGrade, linkName, shownLabel } from '../src/studio/color/graph';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

let width: number;
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser(); width = 1600;
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, width, 900);
    if (this.classList.contains('color-mini-lanes')) return rect(30, 0, 600, 120);
    if (this.classList.contains('color-thumb-frame')) return rect(0, 0, 100, 56);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; });

/** A node of a grade, with only what the node editor reads of it. */
const corrector = (id: string, given: { label?: string; enabled?: boolean; matched?: boolean } = {}): GradeNode => ({
  id, kind: 'corrector', label: given.label ?? '', enabled: given.enabled ?? true,
  params: { shot_match: given.matched ? { gamma: { r: 1, g: 1, b: 1 }, gain: { r: 1.1, g: 1, b: 1 }, offset: { r: 0, g: 0, b: 0 }, contrast: 1, pivot: 0.435 } : null } as Corrector,
});
const parallel = (id: string, inputs = 2): GradeNode => ({ id, kind: 'parallel_mixer', label: '', enabled: true, params: { inputs } });
const link = (from: string, to: string, input = 0, kind: Link['kind'] = 'rgb'): Link => ({ from, to, kind, input });

/**
 * A grade of four nodes, given with its links out of order: 1 and 2 in series, then 2 split to 3
 * and 4 in parallel into a mixer, which goes to the output; 4 has a shot match, and 1 a label
 * longer than a node shows.
 */
const parallelGrade = (): Grade => ({
  nodes: [corrector('n1', { label: 'Skin tones and faces' }), corrector('n2'), corrector('n3'), corrector('n4', { matched: true }), parallel('n5')],
  links: [link('n5', 'output'), link('n4', 'n5', 1), link('n3', 'n5', 0), link('n2', 'n4'), link('n2', 'n3'), link('n1', 'n2'), link('source', 'n1')],
});

// Two media, a timeline of three clips on V1 and one on V2 that begins before the second of V1,
// one on a disabled V3, and a group.
const m1: Media = { ...media, id: 'm1', name: 'Interview', facts: { ...media.facts, duration: { ticks: 2500, scale: 25 } } };
const m2: Media = { ...media, id: 'm2', name: 'Street', facts: { ...media.facts, video: { ...media.facts.video!, codec: 'hevc' } } };
const graded = (): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [
      clip('c1', 0, 100, { grade: parallelGrade() }),
      clip('c2', 100, 100, { source: { kind: 'media', media: 'm2', in: { ticks: 50, scale: 25 } }, group: 'g1' }),
      clip('c3', 200, 100),
    ] },
    { id: 'v2', kind: 'video', name: 'Graphics', enabled: true, locked: false, transitions: [], clips: [
      clip('c4', 50, 100, { source: { kind: 'title', text: 'Hello', style: { font: '', size: 96, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } } }),
    ] },
    { id: 'v3', kind: 'video', name: 'V3', enabled: false, locked: false, transitions: [], clips: [clip('c5', 0, 40)] },
    { id: 'a1', kind: 'audio', name: 'A1', enabled: true, locked: false, transitions: [], clips: [clip('ca', 0, 150)] },
  ],
  grade: { nodes: [corrector('t1n')], links: [link('source', 't1n'), link('t1n', 'output')] },
});
const groupGrade = (id: string): Grade => ({ nodes: [corrector(id, { label: id })], links: [link('source', id), link(id, 'output')] });

/** An engine that keeps a project and the timeline monitor, and tells what changed as the real one does. */
function fakeEngine(timelines: Timeline[] = [graded()]) {
  let project: Project = { schema: 3, next_id: 10, bins: [], media: [m1, m2], timelines, groups: [{ id: 'g1', name: 'Day', pre: groupGrade('pre'), post: groupGrade('post') }] };
  let revision = 0;
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
      if (method === 'transaction.apply') {
        // Add Serial after the source of a clip's grade is worked out as the engine does (T3d's
        // default node 01); other edits are not.
        const done: Op[] = [];
        for (const change of params!.ops as Record<string, unknown>[]) {
          let op: Op | null = 'op' in change ? change as unknown as Op : null;
          if (change.edit === 'add_serial' && change.node === 'source') {
            const grade = change.grade as { level: 'clip'; timeline: string; clip: string };
            const clipNow = project.timelines[0].tracks.flatMap(track => (track.kind === 'subtitle' ? [] : track.clips)).find(each => each.id === grade.clip)!;
            const id = `n${project.next_id}`;
            const fed = clipNow.grade.links.filter(each => each.from === 'source');
            op = { op: 'add_node', grade, node: corrector(id), index: clipNow.grade.nodes.length, links: [link('source', id), ...fed.map(each => ({ ...each, from: id }))] };
            project = { ...project, next_id: project.next_id + 1 };
          }
          if (op === null) continue;
          project = applyOp(project, op);
          done.push(op);
        }
        revision++;
        tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: project.next_id, done });
        return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
      }
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn> };
  const copy = new ProjectCopy(bridge);
  return {
    bridge, copy,
    /** The transport tells something of the timeline monitor. */
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
    /** Another client changes the project. */
    change: (ops: Op[]) => act(() => {
      for (const op of ops) project = applyOp(project, op);
      revision++;
      tell('project.changed', { revision, made_by: 'agent', by: 'agent', next_id: project.next_id, done: ops });
    }),
  };
}
async function mount({ words, timelines }: { words?: typeof zhCN; timelines?: Timeline[] } = {}) {
  const engine = fakeEngine(timelines);
  const tree = <Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />;
  const view = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle(); fireEvent.click(screen.getByRole('tab', { name: words ? '调色' : 'Color' })); await settle();
  return { ...engine, ...view };
}
const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
const page = () => document.querySelector<HTMLElement>('[data-test="color-page"]')!;
const key = (value: string, fields = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
const thumbs = () => [...document.querySelectorAll<HTMLElement>('[data-test="thumbnails"] > [data-clip]')];
const thumb = (id: string) => document.querySelector<HTMLElement>(`[data-test="thumbnails"] > [data-clip="${id}"]`)!;
const currentThumb = () => document.querySelector<HTMLElement>('[data-test="thumbnails"] > [data-current]')?.dataset.clip ?? null;
const click = (element: Element, fields = {}) => { fireEvent.pointerDown(element, { button: 0, clientX: 10, pointerId: 1, ...fields }); fireEvent.pointerUp(element, { clientX: 10, pointerId: 1, ...fields }); };
const graph = () => document.querySelector<SVGElement>('[data-test="grade-graph"]')!;
const numbersDrawn = () => [...graph().querySelectorAll<SVGElement>('[data-node]')].map(each => [each.dataset.node, each.querySelector('[data-test="node-number"]')!.textContent]);
const linksDrawn = () => [...graph().querySelectorAll<SVGElement>('[data-link]')].map(each => each.dataset.link);
const tabsOf = (side: string) => within(document.querySelector<HTMLElement>(`[data-test="palettes-${side}"]`)!).getAllByRole('tab').map(tab => tab.textContent);

describe('the Color page', () => {
  it('is laid out as DaVinci Resolve lays it out, the viewer playing the timeline', async () => {
    const { bridge } = await mount();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'timeline', timeline: 't1' }]);
    const toolbar = screen.getByRole('toolbar', { name: 'Interface toolbar' });
    expect(within(toolbar).getAllByRole('button').map(each => each.textContent)).toEqual(['Gallery', 'LUTs', 'Media pool', 'Clips', 'Timeline', 'Nodes']);
    expect(within(toolbar).getByRole('combobox', { name: 'Clip filter' })).toBeDefined();
    // The gallery is open at first, a placeholder until stills come.
    expect(screen.getByRole('region', { name: 'Gallery' }).textContent).toContain('Stills and albums come in a later version.');
    expect(screen.getByRole('region', { name: 'Timeline viewer' })).toBeDefined();
    expect(screen.getByRole('region', { name: 'Node editor' })).toBeDefined();
    expect(screen.getByRole('listbox', { name: 'Thumbnail timeline' })).toBeDefined();
    expect(screen.getByRole('region', { name: 'Mini-timeline' })).toBeDefined();
    // The palettes in DaVinci Resolve's order: the left, the center, and at the right the keyframes or the scopes.
    expect(tabsOf('left')).toEqual(['Primaries']);
    expect(tabsOf('center')).toEqual(['Curves', 'Qualifier', 'Window', 'Tracker', 'Blur', 'Key']);
    expect(tabsOf('right')).toEqual(['Keyframes', 'Scopes']);
    // Primaries and Curves have their controls (T17), and the right panel the docked scope (T19).
    const [primariesPanel, curvesPanel, rightPanel] = screen.getAllByRole('tabpanel');
    expect(primariesPanel.querySelector('[data-test="primaries"]')).not.toBeNull();
    expect(curvesPanel.querySelector('[data-test="curves"]')).not.toBeNull();
    expect(rightPanel.querySelector('[data-test="scopes-docked"] [data-test="scope-pane"]')).not.toBeNull();
    // The three places at the top left are one place: one opens in place of another, and pressed again it closes.
    fireEvent.click(within(toolbar).getByRole('button', { name: 'LUTs' }));
    expect(screen.queryByRole('region', { name: 'Gallery' })).toBeNull();
    expect(screen.getByRole('region', { name: 'LUT browser' }).querySelector('[data-test="lut-browser"]')).not.toBeNull();
    fireEvent.click(within(toolbar).getByRole('button', { name: 'Media pool' }));
    expect(screen.getByRole('region', { name: 'Media pool' })).toBeDefined();
    fireEvent.click(within(toolbar).getByRole('button', { name: 'Media pool' }));
    expect(document.querySelector('[data-test="color-side"]')).toBeNull();
    // Clips, Timeline and Nodes open and close the thumbnail timeline, the mini-timeline and the node editor.
    for (const [name, test] of [['Clips', 'thumbnails'], ['Timeline', 'mini-timeline'], ['Nodes', 'node-editor']] as const) {
      fireEvent.click(within(toolbar).getByRole('button', { name }));
      expect(document.querySelector(`[data-test="${test}"]`)).toBeNull();
      fireEvent.click(within(toolbar).getByRole('button', { name }));
      expect(document.querySelector(`[data-test="${test}"]`)).not.toBeNull();
    }
  });

  it('merges the left and center palettes where the page is narrow, and keeps each palette chosen', async () => {
    width = MERGE_PALETTES_PX - 1;
    await mount();
    expect(document.querySelector('[data-test="palettes-left"]')).toBeNull();
    expect(tabsOf('center')).toEqual(['Primaries', 'Curves', 'Qualifier', 'Window', 'Tracker', 'Blur', 'Key']);
    fireEvent.click(screen.getByRole('tab', { name: 'Qualifier' }));
    expect(screen.getByRole('tab', { name: 'Qualifier' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tab', { name: 'Primaries' }).getAttribute('aria-selected')).toBe('false');
    // The arrows go along the buttons of a group, and not the playhead.
    const qualifier = screen.getByRole('tab', { name: 'Qualifier' });
    qualifier.focus();
    fireEvent.keyDown(qualifier, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'Window' }).getAttribute('aria-selected')).toBe('true');
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Window' }));
  });

  it('numbers the clips of the video tracks by where they begin, whatever their track, and hides those of a disabled track', async () => {
    const { bridge } = await mount();
    expect(thumbs().map(each => [each.dataset.clip, each.dataset.number])).toEqual([['c1', '1'], ['c4', '3'], ['c2', '4'], ['c3', '5']]);
    expect(thumbs().map(each => each.querySelector('.color-thumb-track')!.textContent)).toEqual(['V1', 'V2', 'V1', 'V1']);
    expect(thumbs().map(each => each.querySelector('[data-test="thumb-number"]')!.textContent)).toEqual(['01', '03', '04', '05']);
    // The source timecode of each clip's first frame, at its media's rate; a title has none.
    expect(thumbs().map(each => each.querySelector('.color-thumb-timecode')!.textContent)).toEqual(['00:00:00:00', '', '00:00:02:00', '00:00:00:00']);
    // Its codec beneath it, until a double-click shows the names of all.
    expect(thumbs().map(each => each.querySelector('[data-test="thumb-text"]')!.textContent)).toEqual(['prores', 'Title', 'hevc', 'prores']);
    fireEvent.doubleClick(thumb('c3').querySelector('[data-test="thumb-text"]')!);
    expect(thumbs().map(each => each.querySelector('[data-test="thumb-text"]')!.textContent)).toEqual(['Interview', 'Hello', 'Street', 'Interview']);
    // The badges: a rainbow round the number of a clip that is graded, a link on one in a group, the dots of a shot match.
    expect(thumbs().filter(each => each.querySelector('[data-graded]')).map(each => each.dataset.clip)).toEqual(['c1']);
    expect(thumbs().filter(each => each.querySelector('[data-test="badge-group"]')).map(each => each.dataset.clip)).toEqual(['c2']);
    expect(thumbs().filter(each => each.querySelector('[data-test="badge-matched"]')).map(each => each.dataset.clip)).toEqual(['c1']);
    // Each picture is the first frame of its clip, in the frames of its media.
    expect(calls(bridge, 'media.thumbnail')).toEqual(expect.arrayContaining([{ media: 'm1', frame: 0, height: 90 }, { media: 'm2', frame: 50, height: 90 }]));
  });

  it('outlines the clip at the playhead, and a click takes the playhead to the first frame of a clip, as the mini-timeline does', async () => {
    const { bridge, notify } = await mount();
    expect(currentThumb()).toBe('c1');
    // Where two clips lie at the playhead, the one on the higher track is current.
    await notify({ frame: 60 });
    expect(currentThumb()).toBe('c4');
    expect(document.querySelector<HTMLElement>('.color-mini-clip[data-current]')!.dataset.clip).toBe('c4');
    bridge.call.mockClear();
    click(thumb('c3').querySelector('.color-thumb-frame')!);
    await settle();
    expect(calls(bridge, 'transport.seek')).toEqual([{ monitor: 'timeline', frame: 200, scrubbing: false }]);
    expect(currentThumb()).toBe('c3');
    expect(thumb('c3').getAttribute('aria-selected')).toBe('true');
    // A clip beneath another, chosen, is current while the playhead is on it, though the one above covers it.
    click(thumb('c1').querySelector('.color-thumb-frame')!);
    await settle();
    await notify({ frame: 70 });
    expect(currentThumb()).toBe('c1');
    // Ctrl+click and Shift+click choose more, and leave the playhead where it is.
    bridge.call.mockClear();
    click(thumb('c2').querySelector('.color-thumb-frame')!, { ctrlKey: true });
    click(thumb('c3').querySelector('.color-thumb-frame')!, { shiftKey: true });
    await settle();
    expect(calls(bridge, 'transport.seek')).toEqual([]);
    expect(thumbs().filter(each => each.getAttribute('aria-selected') === 'true').map(each => each.dataset.clip)).toEqual(['c2', 'c3']);
    // The mini-timeline: a click on a clip chooses it and takes the playhead to its first frame.
    fireEvent.click(document.querySelector('.color-mini-clip[data-clip="c2"]')!);
    await settle();
    expect(calls(bridge, 'transport.seek')).toEqual([{ monitor: 'timeline', frame: 100, scrubbing: false }]);
    expect(currentThumb()).toBe('c2');
  });

  it('follows the project as it changes, and a filter of the Clips menu', async () => {
    const { change } = await mount();
    await change([{ op: 'insert_clip', timeline: 't1', track: 'v1', clip: clip('c6', 300, 50) }]);
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c1', 'c4', 'c2', 'c3', 'c6']);
    await change([{ op: 'set_clip_group', timeline: 't1', clip: 'c3', group: 'g1' }]);
    fireEvent.change(screen.getByRole('combobox', { name: 'Clip filter' }), { target: { value: 'grouped' } });
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c2', 'c3']);
    expect(screen.getByRole('button', { name: 'Clips' }).dataset.filtered).toBeDefined();
    fireEvent.change(screen.getByRole('combobox', { name: 'Clip filter' }), { target: { value: 'ungraded' } });
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c4', 'c2', 'c3', 'c6']);
    fireEvent.change(screen.getByRole('combobox', { name: 'Clip filter' }), { target: { value: 'matched' } });
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c1']);
  });

  it('enables and disables a track from its number, and marks it red with Alt to hide its clips from the thumbnails', async () => {
    const { bridge } = await mount();
    const numbers = () => [...document.querySelectorAll<HTMLElement>('.color-mini-number')].map(each => [each.textContent, each.dataset.state, each.title]);
    expect(numbers()).toEqual([['3', 'disabled', 'V3'], ['2', 'enabled', 'Graphics'], ['1', 'enabled', 'V1']]);
    fireEvent.click(screen.getByRole('button', { name: 'Graphics' }), { altKey: true });
    expect(numbers()[1]).toEqual(['2', 'skipped', 'Graphics']);
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c1', 'c2', 'c3']);
    expect(calls(bridge, 'transaction.apply')).toEqual([]);
    // A plain click is an edit of the project, which one undo takes back.
    fireEvent.click(screen.getByRole('button', { name: 'V3' }));
    await settle();
    expect(calls(bridge, 'transaction.apply')).toEqual([{ revision: 0, ops: [{ op: 'set_track', timeline: 't1', id: 'v3', enabled: true }] }]);
    expect(numbers()[0]).toEqual(['3', 'enabled', 'V3']);
    expect(thumbs().map(each => each.dataset.clip)).toEqual(['c1', 'c5', 'c2', 'c3']);
  });

  it('draws the grade of the current clip: its nodes by number, its links in the engine order, and the shot match', async () => {
    await mount();
    expect(numbersDrawn()).toEqual([['n1', '01'], ['n2', '02'], ['n3', '03'], ['n4', '04'], ['n5', '05']]);
    expect(linksDrawn()).toEqual(sortLinks(parallelGrade().links).map(linkName));
    expect(linksDrawn()).toEqual(['source>n1:rgb:0', 'n1>n2:rgb:0', 'n2>n3:rgb:0', 'n2>n4:rgb:0', 'n3>n5:rgb:0', 'n4>n5:rgb:1', 'n5>output:rgb:0']);
    // The label above a node, cut at twelve characters; the shot match, on the node that has it.
    expect(graph().querySelector('[data-node="n1"] [data-test="node-label"]')!.textContent).toBe('Skin tones a…');
    expect([...graph().querySelectorAll<SVGElement>('[data-node]')].filter(each => each.querySelector('[data-test="node-matched"]')).map(each => each.dataset.node)).toEqual(['n4']);
    // The ports: a corrector has a picture and a key in and out, the mixer two pictures in and one out.
    expect([...graph().querySelectorAll<SVGElement>('[data-node="n5"] [data-port]')].map(each => each.dataset.port)).toEqual(['in:rgb:0', 'in:rgb:1', 'out:rgb']);
    expect([...graph().querySelectorAll<SVGElement>('[data-node="n1"] [data-port]')].map(each => each.dataset.port)).toEqual(['in:rgb:0', 'out:rgb', 'in:key:0', 'out:key']);
    expect(document.querySelector('[data-test="grade-incomplete"]')).toBeNull();
    // The first node is current at first; a press makes another current, and nothing is sent.
    expect(graph().querySelector<SVGElement>('[data-current]')!.dataset.node).toBe('n1');
    fireEvent.pointerDown(graph().querySelector('[data-node="n3"]')!, { button: 0 });
    expect(graph().querySelector<SVGElement>('[data-current]')!.dataset.node).toBe('n3');
  });

  it('switches the node editor between the grades of the clip, its group and the timeline', async () => {
    const { notify } = await mount();
    const level = () => screen.getByRole('combobox', { name: 'Node graph shown' }) as HTMLSelectElement;
    const options = () => [...level().options].map(option => option.textContent);
    expect(options()).toEqual(['Clip', 'Timeline']);
    fireEvent.change(level(), { target: { value: 'timeline' } });
    expect(numbersDrawn()).toEqual([['t1n', '01']]);
    // A clip in a group has its group's grades before and after its own, as the dots show too.
    fireEvent.change(level(), { target: { value: 'clip' } });
    await notify({ frame: 160 });
    expect(options()).toEqual(['Group Pre-Clip', 'Clip', 'Group Post-Clip', 'Timeline']);
    expect([...document.querySelectorAll<HTMLElement>('.color-level-dot')].map(each => each.getAttribute('aria-label'))).toEqual(options());
    fireEvent.click(screen.getByRole('button', { name: 'Group Pre-Clip' }));
    expect(numbersDrawn()).toEqual([['pre', '01']]);
    fireEvent.change(level(), { target: { value: 'post_clip' } });
    expect(numbersDrawn()).toEqual([['post', '01']]);
    // A clip with no nodes shows DaVinci's default node 01, which the grade does not have yet (T3d,
    // pp. 3309, 3312); it is named as the engine will name it, n and the project's next_id. The
    // playhead in a gap shows none.
    fireEvent.change(level(), { target: { value: 'clip' } });
    expect(numbersDrawn()).toEqual([['n10', '01']]);
    expect(linksDrawn()).toEqual(['source>n10:rgb:0', 'n10>output:rgb:0']);
    expect(graph().querySelector('[data-node="n10"]')!.hasAttribute('data-default')).toBe(true);
    expect(screen.queryByText('This grade has no nodes yet.')).toBeNull();
    await notify({ frame: 400 });
    expect(document.querySelector('[data-test="grade-graph"]')).toBeNull();
    expect(screen.getByText('There is no clip at the playhead.')).toBeDefined();
  });

  it('says that a grade with a node not connected is off', async () => {
    const free: Grade = { nodes: [corrector('a'), corrector('b')], links: [link('source', 'a'), link('a', 'output')] };
    const timeline = graded();
    (timeline.tracks[0] as { clips: ReturnType<typeof clip>[] }).clips[0] = clip('c1', 0, 100, { grade: free });
    await mount({ timelines: [timeline] });
    expect(document.querySelector('[data-test="grade-incomplete"]')!.textContent).toBe('The grade is off: a node is not connected.');
    expect(numbersDrawn()).toEqual([['a', '01'], ['b', '02']]);
  });

  it('takes the keys of the transport, Up and Down from clip to clip, and the keys that make the viewer larger', async () => {
    const { bridge } = await mount();
    bridge.call.mockClear();
    key('ArrowDown');
    await settle();
    expect(calls(bridge, 'transport.seek')).toEqual([{ monitor: 'timeline', frame: 50, scrubbing: false }]);
    expect(currentThumb()).toBe('c4');
    key('ArrowDown'); await settle();
    key('ArrowDown'); await settle();
    expect(currentThumb()).toBe('c3');
    key('ArrowDown'); await settle();
    expect(currentThumb()).toBe('c3');
    key('ArrowUp'); await settle();
    expect(currentThumb()).toBe('c2');
    // A track marked red is passed over (p. 3025).
    fireEvent.click(screen.getByRole('button', { name: 'Graphics' }), { altKey: true });
    key('ArrowUp'); await settle();
    expect(currentThumb()).toBe('c1');
    expect(calls(bridge, 'transport.seek').map(each => (each as { frame: number }).frame)).toEqual([50, 100, 200, 100, 0]);
    // The transport: Space plays, the arrows step, Home goes to the start.
    bridge.call.mockClear();
    key(' ');
    key('ArrowRight');
    await settle();
    expect(bridge.call.mock.calls.map(([method]) => method)).toEqual(['transport.play', 'transport.step']);
    // Alt+F, Shift+F and P, each again to go back; Esc leaves the cinema viewer too.
    expect(page().dataset.viewerMode).toBe('normal');
    key('f', { altKey: true, code: 'KeyF' });
    expect(page().dataset.viewerMode).toBe('enhanced');
    key('f', { altKey: true, code: 'KeyF' });
    expect(page().dataset.viewerMode).toBe('normal');
    key('F', { shiftKey: true, code: 'KeyF' });
    expect(page().dataset.viewerMode).toBe('full');
    key('p', { code: 'KeyP' });
    expect(page().dataset.viewerMode).toBe('cinema');
    // Over the whole window, outside the studio, and the keys still go to it.
    const cinema = document.querySelector<HTMLElement>('body > .color-cinema')!;
    expect(cinema.querySelector('[data-test="color-viewer"] .monitor')).not.toBeNull();
    expect(page().querySelector('[data-test="color-viewer"]')).toBeNull();
    const exit = screen.getByRole('button', { name: 'Exit cinema viewer' });
    exit.focus();
    bridge.call.mockClear();
    fireEvent.keyDown(exit, { key: 'ArrowLeft' });
    expect(bridge.call.mock.calls.map(([method]) => method)).toEqual(['transport.step']);
    key('Escape');
    expect(document.querySelector('.color-cinema')).toBeNull();
    expect(page().querySelector('[data-test="color-viewer"]')).not.toBeNull();
    expect(page().dataset.viewerMode).toBe('normal');
    // Not while writing in a field.
    const filter = screen.getByRole('combobox', { name: 'Clip filter' });
    filter.focus();
    fireEvent.keyDown(filter, { key: 'p', code: 'KeyP' });
    expect(page().dataset.viewerMode).toBe('normal');
  });

  it('tells of the LUTs that the engine could not read, at the foot of the viewer', async () => {
    const { notify } = await mount();
    expect(document.querySelector('[data-test="missing-luts"]')).toBeNull();
    await notify({ missing_luts: [{ name: 'film.cube', message: 'there is no LUT film.cube in the folder of the project or in the library' }] });
    const notice = document.querySelector<HTMLElement>('[data-test="missing-luts"]')!;
    expect(notice.textContent).toBe('Missing LUT: film.cube');
    expect(notice.title).toContain('there is no LUT film.cube');
    expect(notice.closest('.monitor')).not.toBeNull();
    await notify({ missing_luts: [{ name: 'a.cube', message: 'no' }, { name: 'b.cube', message: 'no' }] });
    expect(document.querySelector('[data-test="missing-luts"]')!.textContent).toBe('Multiple LUTs missing');
    await notify({ missing_luts: undefined });
    expect(document.querySelector('[data-test="missing-luts"]')).toBeNull();
  });

  it('is named as DaVinci Resolve names it in Chinese', async () => {
    await mount({ words: zhCN });
    expect(within(screen.getByRole('toolbar', { name: '界面工具栏' })).getAllByRole('button').map(each => each.textContent))
      .toEqual(['画廊', 'LUT 浏览器', '媒体池', '片段', '时间线', '节点']);
    expect(tabsOf('left')).toEqual(['一级校色']);
    expect(tabsOf('center')).toEqual(['曲线', '限定器', '窗口', '跟踪器', '模糊', '键']);
    expect(tabsOf('right')).toEqual(['关键帧', '示波器']);
  });

  it('edits the current node from the palettes, in the grade shown, and marks the palette adjusted', async () => {
    const { bridge } = await mount();
    // c1 is at the playhead, and its first node is current.
    const name = document.querySelector('[data-field="primaries.saturation"] .color-number-name')!;
    fireEvent.pointerDown(name, { button: 0, pointerId: 1, clientX: 0 });
    fireEvent.pointerMove(name, { pointerId: 1, clientX: 40 });
    fireEvent.pointerUp(name, { pointerId: 1, clientX: 40 });
    await settle();
    // The last of the drag, which ends its gesture: the drag is one step to undo. It is bound to
    // the revision that the copy has then, after the first of the drag.
    expect(calls(bridge, 'transaction.apply').at(-1)).toEqual({ revision: 1, ops: [
      { op: 'set_node', grade: { level: 'clip', timeline: 't1', clip: 'c1' }, id: 'n1', params: { primaries: { saturation: 60 } } }],
    gesture: { id: expect.any(String), end: true } });
    expect(document.querySelector('[data-palette="primaries"] [data-test="changed-dot"]')).not.toBeNull();
    expect(document.querySelector('[data-palette="curves"] [data-test="changed-dot"]')).toBeNull();
  });

  it('edits the default node 01 of a clip without nodes from the palettes, making it in the same transaction (T3d)', async () => {
    const { bridge, notify } = await mount();
    await notify({ frame: 250 });
    const c3: Target = { level: 'clip', timeline: 't1', clip: 'c3' };
    expect(numbersDrawn()).toEqual([['n10', '01']]);
    const name = document.querySelector('[data-field="primaries.saturation"] .color-number-name')!;
    expect(name).not.toBeNull();
    bridge.call.mockClear();
    fireEvent.pointerDown(name, { button: 0, pointerId: 1, clientX: 0 });
    fireEvent.pointerMove(name, { pointerId: 1, clientX: 40 });
    fireEvent.pointerUp(name, { pointerId: 1, clientX: 40 });
    await settle();
    // A gesture: its first makes the node and sets it, the others set it by the id it was made with,
    // the last ends it, so the drag is one step to undo (see node-default-gesture.test.tsx).
    const sentNow = calls(bridge, 'transaction.apply') as { ops: unknown[]; gesture: { id: string; end?: boolean } }[];
    expect(sentNow.length).toBeGreaterThanOrEqual(2);
    expect(sentNow[0].ops).toEqual([
      { edit: 'add_serial', grade: c3, node: 'source' },
      { op: 'set_node', grade: c3, id: 'n10', params: { primaries: { saturation: 60 } } }]);
    for (const each of sentNow.slice(1)) expect(each.ops).toEqual([{ op: 'set_node', grade: c3, id: 'n10', params: { primaries: { saturation: 60 } } }]);
    expect(new Set(sentNow.map(each => each.gesture.id)).size).toBe(1);
    expect(sentNow.at(-1)!.gesture.end).toBe(true);
    expect(sentNow.slice(0, -1).every(each => each.gesture.end === undefined)).toBe(true);
  });

  it('is a page of the studio on Windows only', () => {
    (window as { columbina?: unknown }).columbina = { platform: 'win32' };
    expect(studioPages()).toEqual(['media', 'cut', 'edit', 'color']);
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    expect(studioPages()).toEqual(['media', 'cut', 'edit']);
    (window as { columbina?: unknown }).columbina = { platform: 'linux' };
    expect(studioPages()).toEqual(['media', 'cut', 'edit']);
  });

  it('has no tab of its own where the studio is shown on macOS', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    const engine = fakeEngine();
    render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
    await settle();
    expect([...document.querySelectorAll('.studio-pages [role="tab"]')].map(tab => tab.textContent)).toEqual(['Media', 'Cut', 'Edit']);
    expect(document.querySelector('[data-test="color-page"]')).toBeNull();
  });
});

describe('the scopes of the Color page', () => {
  it('measure the timeline monitor while the Scopes panel is shown, and the Keyframes panel lets them go', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'win32' };
    const { bridge } = await mount();
    expect(calls(bridge, 'scopes.subscribe')).toEqual([{ view: 'timeline', settings: expect.objectContaining({ kind: 'parade' }) }]);
    // The fake engine answers no scope: the pane says so, and nothing else breaks.
    expect(document.querySelector('[data-test="scope-status"]')?.textContent).toBe('The scope could not be shown.');
    fireEvent.click(screen.getByRole('tab', { name: 'Keyframes' }));
    await settle();
    expect(document.querySelector('[data-test="scope-pane"]')).toBeNull();
    fireEvent.click(screen.getByRole('tab', { name: 'Scopes' }));
    await settle();
    expect(calls(bridge, 'scopes.subscribe')).toHaveLength(2);
    // Ctrl+Shift+W opens the floating window over the page.
    key('W', { code: 'KeyW', ctrlKey: true, shiftKey: true });
    await settle();
    expect(screen.getByRole('dialog', { name: 'Video Scopes' })).toBeDefined();
    expect(calls(bridge, 'scopes.subscribe')).toHaveLength(6);
  });

  it('are not there where the studio is shown on macOS', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    const engine = fakeEngine();
    render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
    await settle();
    key('W', { code: 'KeyW', ctrlKey: true, shiftKey: true });
    await settle();
    expect(document.querySelector('[data-test="scope-pane"]')).toBeNull();
    expect(calls(engine.bridge, 'scopes.subscribe')).toEqual([]);
  });
});

describe('the clips of the Color page', () => {
  const timeline = graded();
  const all = numberedClips(timeline);
  const shown = shownClips(all, new Set());

  it('are numbered by their first frame, the lower track first where two begin together', () => {
    const tied: Timeline = { ...timeline, tracks: [
      { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, transitions: [], clips: [clip('a', 10, 10)] },
      { id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, transitions: [], clips: [clip('b', 0, 30)] },
      { id: 'v3', kind: 'video', name: 'V3', enabled: true, locked: false, transitions: [], clips: [clip('c', 10, 5)] },
    ] };
    expect(numberedClips(tied).map(each => [each.clip.id, each.number, each.trackNumber])).toEqual([['b', 1, 2], ['a', 2, 1], ['c', 3, 3]]);
    expect(all.map(each => each.clip.id)).toEqual(['c1', 'c5', 'c4', 'c2', 'c3']);
    expect(shown.map(each => each.clip.id)).toEqual(['c1', 'c4', 'c2', 'c3']);
    expect(shownClips(all, new Set(['v2'])).map(each => each.clip.id)).toEqual(['c1', 'c2', 'c3']);
  });

  it('have one current at the playhead: the one chosen there, else the one on the highest track', () => {
    expect(currentClip(shown, null, null)).toBeNull();
    expect(currentClip(shown, 0, null)!.clip.id).toBe('c1');
    expect(currentClip(shown, 60, null)!.clip.id).toBe('c4');
    expect(currentClip(shown, 60, 'c1')!.clip.id).toBe('c1');
    expect(currentClip(shown, 160, 'c1')!.clip.id).toBe('c2');
    expect(currentClip(shown, 300, null)).toBeNull();
  });

  it('are gone through one by one with Up and Down, from the playhead where none is current', () => {
    const at = (id: string) => shown.find(each => each.clip.id === id)!;
    expect(stepClip(shown, at('c1'), 0, 1)!.clip.id).toBe('c4');
    expect(stepClip(shown, at('c1'), 0, -1)).toBeNull();
    expect(stepClip(shown, at('c3'), 250, 1)).toBeNull();
    expect(stepClip(shown, null, 310, -1)!.clip.id).toBe('c3');
    expect(stepClip(shown, null, 40, 1)!.clip.id).toBe('c4');
  });

  it('show the frame of their media that lies under a frame of the timeline, through speed and direction', () => {
    const rate = { num: 25, den: 1 };
    const fast = clip('f', 0, 100, { speed: { num: 2, den: 1 }, source: { kind: 'media', media: 'm1', in: { ticks: 10, scale: 25 } } });
    expect(sourceFrameAt(fast, 0, rate, m1)).toBe(10);
    expect(sourceFrameAt(fast, 5, rate, m1)).toBe(20);
    expect(sourceFrameAt({ ...fast, reverse: true }, 0, rate, m1)).toBe(10 + 99 * 2);
    // Media at 50 frames a second on a timeline of 25.
    const m50 = { ...m1, facts: { ...m1.facts, video: { ...m1.facts.video!, rate: { num: 50, den: 1 } } } };
    expect(sourceFrameAt(clip('g', 0, 100), 3, rate, m50)).toBe(6);
    expect(sourceFrameAt(clip('t', 0, 10, { source: { kind: 'color', color: '#000000ff' } }), 0, rate, m1)).toBeNull();
  });
});

describe('the grade as the node editor lays it out', () => {
  it('puts each node a column right of the furthest that feeds it, and parallel ones one above the other', () => {
    const layout = layoutGrade(parallelGrade());
    expect(layout.nodes.map(each => [each.node.id, each.number, each.column, each.row])).toEqual([
      ['n1', 1, 1, 0], ['n2', 2, 2, 0], ['n3', 3, 3, 0], ['n4', 4, 3, 1], ['n5', 5, 4, 0],
    ]);
    expect(layout.output.x).toBeGreaterThan(Math.max(...layout.nodes.map(each => each.x)));
    expect(layout.source.x).toBeLessThan(Math.min(...layout.nodes.map(each => each.x)));
    // A node that nothing feeds stands in the first column.
    const free = layoutGrade({ nodes: [corrector('a'), corrector('b')], links: [link('source', 'a'), link('a', 'output')] });
    expect(free.nodes.map(each => [each.column, each.row])).toEqual([[1, 0], [1, 1]]);
    expect(layoutGrade(emptyGrade()).nodes).toEqual([]);
  });

  it('is complete as the engine judges it: the output fed, every input of the picture fed, every node feeding another', () => {
    expect(isComplete(parallelGrade())).toBe(true);
    expect(isComplete(emptyGrade())).toBe(true);
    expect(isComplete({ nodes: [], links: [] })).toBe(false);
    const grade = parallelGrade();
    expect(isComplete({ ...grade, links: grade.links.filter(each => !(each.to === 'n5' && each.input === 1)) })).toBe(false);
    expect(isComplete({ nodes: [corrector('a'), corrector('b')], links: [link('source', 'a'), link('a', 'output')] })).toBe(false);
    // A key link does not feed the picture's input.
    expect(isComplete({ nodes: [corrector('a')], links: [link('source', 'a', 0, 'key'), link('a', 'output')] })).toBe(false);
  });

  it('cuts a label at twelve characters', () => {
    expect(shownLabel('Skin')).toBe('Skin');
    expect(shownLabel('123456789012')).toBe('123456789012');
    expect(shownLabel('1234567890123')).toBe('123456789012…');
  });
});
