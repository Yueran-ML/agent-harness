// @vitest-environment jsdom
/**
 * The Gallery of the Color page and the comparisons of its viewer (T26, T27; Reference Manual 20,
 * ch. 141–142 and pp. 2999–3006; the interaction reference, §9): the albums and stills as the
 * engine lists them, listed again as it says the Gallery changed; what a grab, an apply, an append
 * and Append Node to Selected Clips send for each mode of the node editor; the menus of albums and
 * stills, a still dragged onto an album, a delete that Ctrl+Z in the Gallery undoes; the memories
 * A–H, kept in the album of the memories that the engine lists apart; and
 * the wipe and split screen of the viewer: on and off, inverted, stepped from still to still,
 * dragged on the picture by the pointer the view tells, a timeline wipe with Gang on and off, and
 * the layouts of a split screen.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineEvent, GalleryAlbum, Group, Project, Still, Target, Timeline } from '../src/studio/engine';
import { Studio, studioPages } from '../src/studio/Studio';
import {
  applyParams, applyTargets, appendNodeParams, appendNodeTargets, grabParams, pickStills, readApplySettings, readMemories, searched, stepStill, thumbnailHeight,
} from '../src/studio/color/stills';
import {
  neighbourClip, referenceClip, splitGrid, splitItems, tileAt, timelineReference, wipeAngleAt, wipeCursor, wipePositionAt, DEFAULT_WIPE,
} from '../src/studio/color/comparison';
import { calls, corrector, fakeEngine, projectOf, settle } from './colorEngine';
import { clip, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import { circularWindow } from '../src/studio/color/WindowPalette';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  localStorage.clear();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('DragEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-page')) return rect(0, 0, 1600, 900);
    if (this.dataset.test === 'viewer-box') return rect(300, 100, 960, 540);
    return rect(0, 0, 600, 300);
  });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete (window as { columbina?: unknown }).columbina; localStorage.clear(); });

const still = (id: string, album: string, name: string, label = ''): Still => ({
  id, album, name, label, created: '2026-10-31T12:00:00Z', image: { file: `${id}.png`, format: 'png', width: 1920, height: 1080 }, level: 'clip',
  grades: { clip: true, timeline: false },
  source: { timeline: { id: 't1', name: 'Main', rate: { num: 25, den: 1 } }, frame: 12, record_timecode: '00:00:00:12', clip: { id: 'c1' }, source_frame: 12, source_timecode: '00:00:00:12' },
});
const startingAlbums = (): GalleryAlbum[] => [
  { id: 'a1', name: 'Stills 1', kind: 'stills', stills: [still('s1', 'a1', '1.1.1', 'Day'), still('s2', 'a1', '1.2.1', 'Night'), still('s3', 'a1', '1.3.1')] },
  { id: 'a2', name: 'Looks', kind: 'stills', stills: [] },
  { id: 'pa1', name: 'PowerGrade 1', kind: 'powergrade', stills: [still('ps1', 'pa1', '1.1.1', 'Teal')] },
];

/** Three clips on V1, the first graded with one node; a group g1 holds the first two where `grouped`. */
function projectWith({ grouped = false }: { grouped?: boolean } = {}): Project {
  const group = grouped ? 'g1' : null;
  const clips = [
    clip('c1', 0, 100, { group, grade: { nodes: [corrector('n1')], links: [{ from: 'source', to: 'n1', kind: 'rgb', input: 0 }, { from: 'n1', to: 'output', kind: 'rgb', input: 0 }] } }),
    clip('c2', 100, 100, { group }), clip('c3', 200, 100),
  ];
  const groups: Group[] = grouped ? [{ id: 'g1', name: 'Scene', pre: { nodes: [], links: [{ from: 'source', to: 'output', kind: 'rgb', input: 0 }] }, post: { nodes: [], links: [{ from: 'source', to: 'output', kind: 'rgb', input: 0 }] } }] : [];
  return projectOf({ timelines: [timeline(clips)], groups });
}

/**
 * The studio on the Color page, on an engine that keeps a Gallery as the real one does, as far as
 * the page goes: it lists it, grabs, moves, labels and deletes stills, makes and removes albums,
 * tells `gallery.changed`, and answers wipes and split screens with what the viewer compares.
 */
async function mountGallery(project: Project = projectWith(), { albums = startingAlbums(), exported = null as unknown }: { albums?: GalleryAlbum[]; exported?: unknown } = {}) {
  const engine = fakeEngine(project);
  const listeners = new Set<(event: EngineEvent) => void>();
  const baseOn = engine.bridge.onEvent;
  engine.bridge.onEvent = (listener) => { listeners.add(listener); const off = baseOn(listener); return () => { listeners.delete(listener); off(); }; };
  let physicalButtons: string[] = [];
  const tell = (method: string, params: unknown) => act(() => {
    if (method === 'view.pointer') physicalButtons = (params as { buttons: string[] }).buttons;
    listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  });
  let gallery = albums;
  // The album of the memories, which the engine lists apart, and the stills deleted with where they were.
  let memories: GalleryAlbum | null = null;
  let recycled: Still[] = [];
  let made = 0;
  let binding = 0, sequence = 0;
  let viewport: Record<string, unknown> = { mode: 'fit' };
  const changed = () => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method: 'gallery.changed', params: {} }));
  const base = engine.bridge.call.getMockImplementation()!;
  engine.bridge.call.mockImplementation(async (method: string, params?: Record<string, any>) => {
    const ok = (result: unknown) => ({ ok: true, result });
    switch (method) {
      case 'gallery.list': return ok({ albums: gallery, memories });
      case 'gallery.thumbnail': return ok({ width: Math.round((params!.height * 16) / 9), height: params!.height, png: 'iVBORw0KGgo=' });
      case 'still.grab': {
        made += 1;
        const album = params!.album ?? gallery[0].id;
        const grabbed = still(`s${10 + made}`, album, `1.1.${made}`);
        gallery = gallery.map(each => (each.id === album ? { ...each, stills: [...each.stills, grabbed] } : each));
        const held = memories;
        if (held !== null && held.id === album) memories = { ...held, stills: [...held.stills, grabbed] };
        changed();
        return ok(grabbed);
      }
      case 'gallery.album.create': {
        if (params!.kind === 'memories') {
          memories ??= { id: 'a9', name: params!.name ?? 'Memories', kind: 'memories', stills: [] };
          changed();
          return ok({ id: memories.id, name: memories.name, kind: 'memories' });
        }
        made += 1;
        const album: GalleryAlbum = { id: params!.kind === 'powergrade' ? `pa${made + 1}` : `a${made + 2}`, name: params!.name ?? 'Stills 3', kind: params!.kind ?? 'stills', stills: [] };
        gallery = params!.kind === 'powergrade' ? [...gallery, album] : [...gallery.filter(each => each.kind === 'stills'), album, ...gallery.filter(each => each.kind === 'powergrade')];
        changed();
        return ok({ id: album.id, name: album.name, kind: album.kind });
      }
      case 'gallery.album.rename':
        gallery = gallery.map(each => (each.id === params!.album ? { ...each, name: params!.name } : each)); changed();
        return ok({ id: params!.album, name: params!.name });
      case 'gallery.album.delete':
        if (gallery.filter(each => each.kind === 'stills').length === 1 && params!.album === 'a1') {
          return { ok: false, error: { code: -32001, message: 'last', data: { reason: 'last_album', album: 'a1' } } };
        }
        gallery = gallery.filter(each => each.id !== params!.album); changed();
        return ok({ deleted: [] });
      case 'still.label':
        gallery = gallery.map(each => ({ ...each, stills: each.stills.map(one => (one.id === params!.still ? { ...one, label: params!.label } : one)) })); changed();
        return ok({ id: params!.still, label: params!.label });
      case 'still.delete':
        recycled = [...recycled, ...[...gallery, ...(memories ? [memories] : [])].flatMap(each => each.stills).filter(one => params!.stills.includes(one.id))];
        gallery = gallery.map(each => ({ ...each, stills: each.stills.filter(one => !params!.stills.includes(one.id)) }));
        if (memories) memories = { ...memories, stills: memories.stills.filter(one => !params!.stills.includes(one.id)) };
        changed();
        return ok({ deleted: params!.stills });
      case 'still.restore': {
        const back = recycled.filter(one => params!.stills.includes(one.id));
        recycled = recycled.filter(one => !params!.stills.includes(one.id));
        gallery = gallery.map(each => ({ ...each, stills: [...each.stills, ...back.filter(one => one.album === each.id)].sort((a, b) => a.id.localeCompare(b.id)) }));
        changed();
        return ok({ restored: Object.fromEntries(back.map(one => [one.id, one.album])) });
      }
      case 'still.copy': {
        made += 1;
        const source = gallery.flatMap(each => each.stills).find(one => one.id === params!.stills[0])!;
        const copy = { ...source, id: `s${10 + made}`, album: params!.album };
        const held = memories;
        if (held !== null && held.id === params!.album) memories = { ...held, stills: [...held.stills, copy] };
        changed();
        return ok({ copied: { [source.id]: copy.id } });
      }
      case 'still.move': {
        const moving = gallery.flatMap(each => each.stills).filter(one => params!.stills.includes(one.id));
        gallery = gallery.map(each => ({
          ...each, stills: each.id === params!.album ? [...each.stills, ...moving.map(one => ({ ...one, album: each.id }))] : each.stills.filter(one => !params!.stills.includes(one.id)),
        }));
        changed();
        return ok({ moved: Object.fromEntries(moving.map(one => [one.id, one.id])) });
      }
      case 'still.get': {
        const found = gallery.flatMap(each => each.stills).find(one => one.id === params!.still)!;
        return ok({ ...found, grades: { clip: null, timeline: null }, frozen: { clip: null, timeline: null },
          source: { ...found.source, media: { id: 'm1', name: 'Interview' } } });
      }
      case 'still.apply': case 'grade.append_node': return ok({ done: [], revision: 1, next_id: 10, names: {}, clamped: [] });
      case 'viewer.wipe':
        if (params!.on === false) return ok({ mode: 'off' });
        return ok({ mode: 'wipe', wipe: { ...params, reference: { ...params!.reference, ...(params!.reference.kind === 'still' ? { path: 'x.png' } : { at: whereAt(params!.reference) }) } } });
      case 'viewer.split':
        if (params!.on === false) return ok({ mode: 'off' });
        return ok({ mode: 'split', split: { on: true, items: params!.items, selected: params!.selected, outline: params!.outline } });
      case 'window.bind': return ok({ binding: ++binding, sequence: ++sequence });
      case 'window.unbind': return ok({ binding: params!.binding, phase: 'ended' });
      case 'window.input': return ok({ binding: params!.binding, capture: 1, sequence: ++sequence, phase: params!.kind === 'up' || params!.kind === 'cancel' ? 'ended' : 'arming',
        reason: null, selected: 'w2', event: params!.event ?? 20, revision: 0, keys: { pan: 0, tilt: 3, clock: 'source', frame: 10 } });
      case 'view.pointer': return ok({ monitor: 'timeline', on: params!.on, cursor: params!.cursor ?? null });
      case 'view.viewport': {
        if (params!.operation === 'zoom') viewport = { mode: 'scale', scale: 1.027, center: { x: 0.4, y: 0.6 } };
        else if (params!.operation === 'scale') viewport = { mode: 'scale', scale: params!.scale, center: { x: 0.4, y: 0.6 } };
        else if (params!.operation === 'fit') viewport = { mode: 'fit' };
        return ok({ monitor: 'timeline', ...viewport, instance: 1, geometry: { surface: [960, 540], picture: [1920, 1080] },
          ...(params!.operation === 'prepare' ? { buttons: physicalButtons } : {}) });
      }
      default: return base(method, params);
    }
  });
  // The frame that a reference of the timeline shows, as the engine answers it: at the playhead's offset into the clip named.
  const starts: Record<string, number> = { c1: 0, c2: 100, c3: 200 };
  const whereAt = (reference: Record<string, any>) => {
    if (reference.kind === 'clip') return starts[reference.clip] + (reference.frame ?? 0);
    if (reference.kind === 'neighbour') return starts[reference.current ?? 'c1'] + (reference.side === 'next' ? 100 : -100);
    return null;
  };
  (engine.bridge as unknown as Record<string, unknown>).importStills = vi.fn(async () => ({ ok: true, result: { added: [] } }));
  (engine.bridge as unknown as Record<string, unknown>).exportStills = vi.fn(async () => (exported ?? { ok: true, result: { written: [] } }));
  const view = render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
  await settle();
  fireEvent.click(screen.getByRole('tab', { name: 'Color' }));
  await settle();
  return { ...engine, ...view, tell, gallery: () => gallery, setGallery: (albums: GalleryAlbum[]) => { gallery = albums; } };
}

const key = (value: string, fields = {}) => act(() => { fireEvent.keyDown(document.body, { key: value, ...fields }); });
const letterKey = (letter: string, fields: Record<string, boolean>) => key(letter, { code: `Key${letter.toUpperCase()}`, ...fields });
const stillTile = (id: string) => document.querySelector<HTMLElement>(`[data-test="still"][data-still="${id}"]`)!;
const stillIds = () => [...document.querySelectorAll<HTMLElement>('[data-test="still"]')].map(each => each.dataset.still);
const albumNames = () => [...document.querySelectorAll<HTMLElement>('[data-test="album"]')].map(each => each.textContent);
const albumItem = (id: string) => document.querySelector<HTMLElement>(`[data-test="album"][data-album="${id}"]`)!;
const thumb = (id: string) => document.querySelector<HTMLElement>(`[data-test="thumbnails"] > [data-clip="${id}"]`)!;
const pressThumb = (id: string, fields = {}) => {
  const frame = thumb(id).querySelector('.color-thumb-frame')!;
  fireEvent.pointerDown(frame, { button: 0, clientX: 10, pointerId: 1, ...fields }); fireEvent.pointerUp(frame, { clientX: 10, pointerId: 1, ...fields });
};
const menuItems = (name: string) => within(screen.getByRole('menu', { name })).getAllByRole('menuitem').map(each => each.getAttribute('aria-label'));
const chooseItem = (name: string) => fireEvent.click(screen.getByRole('menuitem', { name }));
const last = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => calls(bridge, method).at(-1) as Record<string, unknown>;
const C = (id: string): Target => ({ level: 'clip', timeline: 't1', clip: id });

describe('Circular Window controls', () => {
  it('shows wheel custom percentages and offers Fit and 100% while OSC is Off', async () => {
    const h = await mountGallery();
    const zoom = screen.getByRole('combobox', { name: 'Viewer zoom' }) as HTMLSelectElement;
    expect(zoom.value).toBe('fit');
    await h.tell('view.pointer', { monitor: 'timeline', instance: 1, event: 1, kind: 'wheel', button: null, buttons: [], delta: 30,
      x: 500, y: 200, window: { x: 250, y: 100 }, shift: false, ctrl: false, alt: false, inside: true, snapshot: null }); await settle();
    expect(zoom.value).toBe('1.027'); expect(zoom.selectedOptions[0].textContent).toBe('102.7%');
    expect(last(h.bridge, 'view.viewport')).toMatchObject({ operation: 'zoom', at: { x: 250.5, y: 100.5 } });
    expect(calls(h.bridge, 'viewer.wipe')).toHaveLength(0);
    fireEvent.change(zoom, { target: { value: '1' } }); await settle();
    expect(last(h.bridge, 'view.viewport')).toMatchObject({ operation: 'scale', scale: 1 });
    expect(zoom.value).toBe('1');
    fireEvent.change(zoom, { target: { value: 'fit' } }); await settle(); expect(zoom.value).toBe('fit');
  });

  const windowProject = () => {
    const project = projectWith();
    const track = project.timelines[0].tracks[0];
    if (track.kind === 'subtitle') throw new Error('fixture');
    const node = track.clips[0].grade.nodes[0];
    if (node.kind === 'corrector') node.params.windows = [circularWindow('w1'), circularWindow('w2')];
    return project;
  };
  const nativePoint = (kind: string, event: number, more = {}) => ({ monitor: 'timeline', instance: 1, event, kind,
    button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x: 400, y: 300,
    inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false,
    presentation: { serial: 1, tag: 1, map: null }, snapshot: { revision: 0, timeline: 't1', frame: 10, canvas: [1920, 1080],
      ready: true, editable: true, comparison: 'wipe', window: { binding: 1, selected: 'w1' } }, ...more });

  it('offers explicit OSC ownership; comparison commands keep the selected tool', async () => {
    const { bridge } = await mountGallery();
    const osc = screen.getByRole('combobox', { name: 'On-screen controls' });
    expect((osc as HTMLSelectElement).value).toBe('none');
    fireEvent.click(screen.getByRole('tab', { name: 'Window' }));
    expect((osc as HTMLSelectElement).value).toBe('window');
    fireEvent.doubleClick(stillTile('s1'));
    await settle();
    expect((osc as HTMLSelectElement).value).toBe('window');
    await letterKey('w', { ctrlKey: true });
    fireEvent.click(document.querySelector('[data-test="image-wipe"]')!);
    await settle();
    expect((osc as HTMLSelectElement).value).toBe('window');
    expect(calls(bridge, 'view.pointer').filter(p => (p as any).on).every(p => (p as any).cursor === 'arrow')).toBe(true);
    expect(screen.getByRole('button', { name: 'Add Circular Window' })).toBeDefined();
  });

  it('keeps Window ownership while wiping, accepts a server-selected row, and keeps the binding through own revisions', async () => {
    const h = await mountGallery(windowProject());
    fireEvent.click(screen.getByRole('tab', { name: 'Window' })); await settle();
    expect(calls(h.bridge, 'window.bind')).toHaveLength(1);
    const pointerCalls = calls(h.bridge, 'view.pointer');
    fireEvent.doubleClick(stillTile('s1')); await settle();
    expect(calls(h.bridge, 'view.pointer')).toEqual(pointerCalls);
    await h.tell('view.pointer', nativePoint('down', 20)); await settle();
    expect(calls(h.bridge, 'window.input')[0]).toEqual({ binding: 1, kind: 'down', event: 20 });
    expect(screen.getByRole('option', { name: 'w2 · Circular' }).getAttribute('aria-selected')).toBe('true');
    expect(document.querySelector('[data-test="window-keys"]')!.textContent).toContain('Pan: Key frame 0 · Tilt: Key frame 3');
    expect((screen.getByRole('button', { name: 'Add Circular Window' }) as HTMLButtonElement).disabled).toBe(true);
    // C2 owns its project changes; the desktop retains the binding across copies, including coalesced revisions.
    await h.outside([{ op: 'set_node', grade: C('c1'), id: 'n1', params: { windows: [circularWindow('w1'),
      { ...circularWindow('w2'), transform: { ...circularWindow('w2').transform, pan: 62 } }] } }]);
    await settle();
    await h.tell('view.pointer', nativePoint('move', 21, { x: -600, y: 3000, inside: false, snapshot: null })); await settle();
    expect(calls(h.bridge, 'window.input').at(-1)).toEqual({ binding: 1, capture: 1, kind: 'move', event: 21 });
    expect(calls(h.bridge, 'window.bind')).toHaveLength(1);
    expect(calls(h.bridge, 'viewer.wipe')).toHaveLength(1);
    await h.tell('view.pointer', nativePoint('up', 22)); await settle();
    fireEvent.click(screen.getByRole('option', { name: 'w1 · Circular' })); await settle();
    expect(calls(h.bridge, 'window.bind')).toHaveLength(2);
  });

  it('does not bind an unacknowledged created row after choosing it or switching OSC away and back', async () => {
    const h = await mountGallery(windowProject());
    fireEvent.click(screen.getByRole('tab', { name: 'Window' })); await settle();
    let release!: () => void;
    const waiting = new Promise<void>(resolve => { release = resolve; });
    const base = h.bridge.call.getMockImplementation()!;
    h.bridge.call.mockImplementation(async (...args: any[]) => {
      const answer = await base(...args);
      if (args[0] === 'transaction.apply') await waiting;
      return answer;
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add Circular Window' })); await settle();
    fireEvent.click(screen.getByRole('option', { name: 'w3 · Circular' })); await settle();
    expect(calls(h.bridge, 'window.bind').some((each: any) => each.selected === 'w3')).toBe(false);
    expect((screen.getByRole('button', { name: 'Add Circular Window' }) as HTMLButtonElement).disabled).toBe(true);
    const osc = screen.getByRole('combobox', { name: 'On-screen controls' });
    fireEvent.change(osc, { target: { value: 'none' } }); await settle();
    fireEvent.change(osc, { target: { value: 'window' } }); await settle();
    fireEvent.click(screen.getByRole('option', { name: 'w3 · Circular' })); await settle();
    expect(calls(h.bridge, 'window.bind').some((each: any) => each.selected === 'w3')).toBe(false);
    await act(async () => release()); await settle();
    expect(screen.getByRole('option', { name: 'w1 · Circular' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(screen.getByRole('option', { name: 'w3 · Circular' })); await settle();
    expect(calls(h.bridge, 'window.bind').at(-1)).toMatchObject({ selected: 'w3', revision: 1 });
  });

  it('releases Window when the real grade disconnects and rebinds after reconnection', async () => {
    const project = windowProject();
    const track = project.timelines[0].tracks[0];
    if (track.kind === 'subtitle') throw new Error('fixture');
    const grade = track.clips[0].grade;
    const h = await mountGallery(project);
    fireEvent.click(screen.getByRole('tab', { name: 'Window' })); await settle();
    expect(calls(h.bridge, 'window.bind')).toHaveLength(1);
    // C2 may already have invalidated this binding when the external disconnected graph arrived.
    const base = h.bridge.call.getMockImplementation()!;
    h.bridge.call.mockImplementation((...args: any[]) => args[0] === 'window.unbind'
      ? Promise.resolve({ ok: false, error: { code: -32001, message: 'binding', data: { reason: 'binding' } } }) : base(...args));
    await h.outside([{ op: 'set_grade', grade: C('c1'), value: { ...grade, links: grade.links.filter(each => each.to !== 'output') } }]);
    await settle();
    expect(calls(h.bridge, 'window.unbind')).toEqual([{ binding: 1 }]);
    expect(calls(h.bridge, 'window.bind')).toHaveLength(1);
    await h.outside([{ op: 'set_grade', grade: C('c1'), value: grade }]); await settle();
    expect(calls(h.bridge, 'window.bind')).toHaveLength(2);
    expect(calls(h.bridge, 'window.bind').at(-1)).toMatchObject({ selected: 'w1', revision: 2 });
  });

  it('recovers an idle retired binding when a held ProjectCopy refetch publishes only the final complete graph', async () => {
    const project = windowProject();
    const track = project.timelines[0].tracks[0];
    if (track.kind === 'subtitle') throw new Error('fixture');
    const grade = track.clips[0].grade;
    const h = await mountGallery(project);
    fireEvent.click(screen.getByRole('tab', { name: 'Window' })); await settle();
    const oldCopy = h.copy.getSnapshot();
    let reply!: (answer: unknown) => void;
    const base = h.bridge.call.getMockImplementation()!;
    h.bridge.call.mockImplementation((...args: any[]) => args[0] === 'project.get'
      ? new Promise(resolve => { reply = resolve; }) : base(...args));
    let fetching!: Promise<void>;
    act(() => { fetching = h.copy.refetch(); });
    await h.outside([{ op: 'set_grade', grade: C('c1'), value: { ...grade, links: grade.links.filter(each => each.to !== 'output') } }]);
    await h.tell('window.unbound', { binding: 1, sequence: 2, revision: 1, reason: 'project_changed' });
    await h.outside([{ op: 'set_grade', grade: C('c1'), value: grade }]);
    await settle();
    expect(h.copy.getSnapshot().revision).toBe(0);
    expect(calls(h.bridge, 'window.bind')).toHaveLength(1);
    await act(async () => { reply({ ok: true, result: { project: oldCopy.project, revision: 0, next_id: oldCopy.project!.next_id } }); await fetching; });
    await settle();
    expect(h.copy.getSnapshot().revision).toBe(2);
    expect(calls(h.bridge, 'window.bind')).toHaveLength(2);
    expect(calls(h.bridge, 'window.bind').at(-1)).toMatchObject({ revision: 2, selected: 'w1' });
    await h.tell('view.pointer', nativePoint('down', 20, { snapshot: { ...nativePoint('down', 20).snapshot, revision: 2, window: { binding: 2, selected: 'w1' } } }));
    await settle();
    expect(calls(h.bridge, 'window.input')[0]).toEqual({ binding: 2, kind: 'down', event: 20 });
  });

  it('uses the portal viewer ref for native focus, capture Escape, and right-click coordinates in cinema', async () => {
    const h = await mountGallery(windowProject());
    fireEvent.click(screen.getByRole('tab', { name: 'Window' })); await settle();
    await letterKey('p', {}); await settle();
    const viewer = document.querySelector<HTMLElement>('.color-cinema [data-test="color-viewer"]')!;
    await h.tell('view.pointer', nativePoint('down', 20)); await settle();
    expect(document.activeElement).toBe(viewer);
    fireEvent.keyDown(viewer, { key: 'Escape' }); await settle();
    expect(calls(h.bridge, 'window.input').at(-1)).toEqual({ binding: 1, capture: 1, kind: 'cancel' });
    expect(document.querySelector('.color-cinema')).not.toBeNull();
    await h.tell('view.pointer', nativePoint('down', 21, { button: 'right', buttons: ['right'] })); await settle();
    const menu = screen.getByRole('menu', { name: 'Viewer menu' });
    expect(menu.style.left).toBe('310px'); expect(menu.style.top).toBe('120px');
    expect(calls(h.bridge, 'window.input')).toHaveLength(2);
  });
});

describe('the Gallery', () => {
  it('lists the albums, the PowerGrades at the bottom, and the stills of the current one with their numbers, labels and thumbnails', async () => {
    const { bridge, tell, setGallery } = await mountGallery();
    expect(calls(bridge, 'gallery.list').length).toBeGreaterThanOrEqual(1);
    expect(albumNames()).toEqual(['Stills 1', 'Looks', 'PowerGrade 1']);
    expect(albumItem('a1').getAttribute('aria-selected')).toBe('true');
    expect(stillIds()).toEqual(['s1', 's2', 's3']);
    expect(within(stillTile('s1')).getByText('1.1.1')).toBeDefined();
    expect(within(stillTile('s1')).getByText('Day')).toBeDefined();
    // Thumbnails at the height of the Still Size slider, in steps.
    await settle();
    const heights = new Set(calls(bridge, 'gallery.thumbnail').map(each => (each as { height: number }).height));
    expect([...heights]).toEqual([thumbnailHeight(63)]);
    expect(stillTile('s1').querySelector('img')!.getAttribute('src')).toBe('data:image/png;base64,iVBORw0KGgo=');
    fireEvent.change(document.querySelector('[data-test="still-size"]')!, { target: { value: '140' } });
    await settle();
    expect(calls(bridge, 'gallery.thumbnail').some(each => (each as { height: number }).height === thumbnailHeight(140))).toBe(true);
    // Another client changes the Gallery: it is listed again.
    const asked = calls(bridge, 'gallery.list').length;
    setGallery([{ ...startingAlbums()[0], stills: [still('s9', 'a1', '2.1.1')] }, ...startingAlbums().slice(1)]);
    await tell('gallery.changed', {});
    await settle();
    expect(calls(bridge, 'gallery.list').length).toBe(asked + 1);
    expect(stillIds()).toEqual(['s9']);
    // An album clicked is the current one; the PowerGrade album shows its stills.
    fireEvent.click(albumItem('pa1'));
    expect(stillIds()).toEqual(['ps1']);
    // The Album button hides the albums; search shows the stills whose label holds what is typed.
    fireEvent.click(document.querySelector('[data-test="toggle-albums"]')!);
    expect(document.querySelector('[data-test="albums"]')).toBeNull();
  });

  it('searches the stills of the album by their labels, and says where none is', async () => {
    await mountGallery();
    fireEvent.click(document.querySelector('[data-test="toggle-search"]')!);
    fireEvent.change(document.querySelector('[data-test="still-search"]')!, { target: { value: 'nig' } });
    expect(stillIds()).toEqual(['s2']);
    fireEvent.change(document.querySelector('[data-test="still-search"]')!, { target: { value: 'dusk' } });
    expect(document.querySelector('[data-test="stills-empty"]')!.textContent).toBe('No still here has a label like this.');
    fireEvent.click(albumItem('a2'));
    fireEvent.change(document.querySelector('[data-test="still-search"]')!, { target: { value: '' } });
    expect(document.querySelector('[data-test="stills-empty"]')!.textContent).toBe('There are no stills in this album. Press Ctrl+Alt+G to grab one.');
  });

  it('grabs a still into the current album, of the current clip, at the mode of the node editor', async () => {
    const { bridge } = await mountGallery(projectWith({ grouped: true }));
    await letterKey('g', { ctrlKey: true, altKey: true });
    await settle();
    expect(last(bridge, 'still.grab')).toEqual({ view: 'timeline', album: 'a1', clip: 'c1', level: 'clip' });
    // The new still is listed, as the engine said the Gallery changed.
    expect(stillIds()).toContain('s11');
    fireEvent.click(albumItem('a2'));
    const level = screen.getByRole('combobox', { name: 'Node graph shown' }) as HTMLSelectElement;
    for (const [mode, sent] of [['pre_clip', 'pre_clip'], ['post_clip', 'post_clip'], ['timeline', 'clip']] as const) {
      fireEvent.change(level, { target: { value: mode } });
      await letterKey('g', { ctrlKey: true, altKey: true });
      await settle();
      expect(last(bridge, 'still.grab')).toEqual({ view: 'timeline', album: 'a2', clip: 'c1', level: sent });
    }
    // And from the right-click menu of the viewer.
    fireEvent.contextMenu(document.querySelector('[data-test="viewer-box"]')!, { clientX: 400, clientY: 200 });
    expect(menuItems('Viewer menu')).toEqual(['Grab Still', 'Wipe On/Off', 'Split Screen']);
    chooseItem('Grab Still');
    await settle();
    expect(calls(bridge, 'still.grab')).toHaveLength(5);
  });

  it('applies and appends a still to the clips chosen, to the timeline, or to their groups, as Apply Grades Using and Preserve say', async () => {
    const { bridge } = await mountGallery(projectWith({ grouped: true }));
    fireEvent.contextMenu(stillTile('s2'), { clientX: 20, clientY: 20 });
    expect(menuItems('Still menu')).toEqual(['Apply Grade', 'Append Node Graph', 'Delete Selected', 'Properties', 'Change Label', 'Export']);
    chooseItem('Apply Grade');
    await settle();
    expect(last(bridge, 'still.apply')).toEqual({ still: 's2', to: [C('c1')], mode: 'apply', keyframes: 'none', revision: 0 });
    // Clips chosen in the thumbnail timeline are what it goes to: the current one clicked, and another Ctrl+clicked.
    pressThumb('c1');
    pressThumb('c3', { ctrlKey: true });
    fireEvent.contextMenu(stillTile('s2'));
    chooseItem('Append Node Graph');
    await settle();
    expect(last(bridge, 'still.apply')).toEqual({ still: 's2', to: [C('c1'), C('c3')], mode: 'append', keyframes: 'none', revision: 0 });
    // The Gallery's menu: Apply Grades Using and Copy Grade: Preserve number of nodes, kept on this computer.
    fireEvent.contextMenu(document.querySelector('[data-test="stills"]')!);
    expect(menuItems('Gallery menu')).toEqual(['Apply Grades Using', 'Copy Grade: Preserve number of nodes', 'Import']);
    chooseItem('Apply Grades Using');
    expect(menuItems('Apply Grades Using')).toEqual(['No Keyframes', 'Keyframes Aligning Source Timecode', 'Keyframes Aligning Start Frames']);
    chooseItem('Keyframes Aligning Start Frames');
    fireEvent.contextMenu(document.querySelector('[data-test="stills"]')!);
    chooseItem('Copy Grade: Preserve number of nodes');
    expect(menuItems('Copy Grade: Preserve number of nodes')).toEqual(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '10']);
    chooseItem('2');
    expect(readApplySettings()).toEqual({ keyframes: 'start_frame', preserve: 2 });
    // The middle button applies the still it is pressed on (p. 3288); an append keeps no nodes.
    fireEvent.pointerDown(stillTile('s1'), { button: 1 });
    await settle();
    expect(last(bridge, 'still.apply')).toEqual({ still: 's1', to: [C('c1'), C('c3')], mode: 'apply', keyframes: 'start_frame', preserve_nodes: 2, revision: 0 });
    fireEvent.contextMenu(stillTile('s1'));
    chooseItem('Append Node Graph');
    await settle();
    expect(last(bridge, 'still.apply')).toEqual({ still: 's1', to: [C('c1'), C('c3')], mode: 'append', keyframes: 'start_frame', revision: 0 });
    // In the Timeline mode it goes to the timeline's grade; in a group's, to the groups of the clips chosen, each once.
    const level = screen.getByRole('combobox', { name: 'Node graph shown' }) as HTMLSelectElement;
    fireEvent.change(level, { target: { value: 'timeline' } });
    fireEvent.pointerDown(stillTile('s1'), { button: 1 });
    await settle();
    expect((last(bridge, 'still.apply') as { to: Target[] }).to).toEqual([{ level: 'timeline', timeline: 't1' }]);
    pressThumb('c2', { ctrlKey: true });
    fireEvent.change(level, { target: { value: 'post_clip' } });
    fireEvent.pointerDown(stillTile('s1'), { button: 1 });
    await settle();
    expect((last(bridge, 'still.apply') as { to: Target[] }).to).toEqual([{ level: 'post_clip', group: 'g1' }]);
  });

  it('says a refusal of an apply where the node editor says one', async () => {
    const { bridge } = await mountGallery();
    const base = bridge.call.getMockImplementation()!;
    bridge.call.mockImplementation(async (method: string, params?: Record<string, unknown>) => (method === 'still.apply'
      ? { ok: false, error: { code: -32001, message: 'no grade', data: { reason: 'no_grade', still: 's1', grade: 'timeline' } } } : base(method, params)));
    fireEvent.pointerDown(stillTile('s1'), { button: 1 });
    await settle();
    expect(screen.getByText('This still holds no grade for the mode of the node editor.')).toBeDefined();
  });

  it('appends the current node to the other clips chosen, from the menu of the node', async () => {
    const { bridge } = await mountGallery();
    pressThumb('c2', { ctrlKey: true });
    pressThumb('c3', { ctrlKey: true });
    const node = document.querySelector('[data-test="grade-graph"] [data-node="n1"]')!;
    fireEvent.contextMenu(node, { clientX: 5, clientY: 5 });
    chooseItem('Append Node to Selected Clips');
    await settle();
    // No Keyframes (the Gallery's default): the node as it is at the playhead.
    expect(last(bridge, 'grade.append_node')).toEqual({ from: C('c1'), node: 'n1', to: [C('c2'), C('c3')], keyframes: 'none', at: 0, revision: 0 });
    fireEvent.contextMenu(document.querySelector('[data-test="stills"]')!);
    chooseItem('Apply Grades Using');
    chooseItem('Keyframes Aligning Source Timecode');
    fireEvent.contextMenu(node, { clientX: 5, clientY: 5 });
    chooseItem('Append Node to Selected Clips');
    await settle();
    expect(last(bridge, 'grade.append_node')).toEqual({ from: C('c1'), node: 'n1', to: [C('c2'), C('c3')], keyframes: 'source_timecode', revision: 0 });
  });

  it('makes, renames and removes albums from their menu, a removal confirmed first', async () => {
    const { bridge } = await mountGallery();
    fireEvent.contextMenu(albumItem('a2'));
    expect(menuItems('Album menu')).toEqual(['Add Still Album', 'Add PowerGrade Album', 'Remove Current Album']);
    chooseItem('Add Still Album');
    await settle();
    expect(last(bridge, 'gallery.album.create')).toEqual({ kind: 'stills' });
    fireEvent.contextMenu(albumItem('a1'));
    chooseItem('Add PowerGrade Album');
    await settle();
    expect(last(bridge, 'gallery.album.create')).toEqual({ kind: 'powergrade' });
    expect(albumNames().at(-1)).toBe('Stills 3');
    // A double-click renames, Enter keeps the name.
    fireEvent.doubleClick(albumItem('a2'));
    const field = document.querySelector<HTMLInputElement>('[data-test="album-name"]')!;
    fireEvent.change(field, { target: { value: 'Day exteriors' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(last(bridge, 'gallery.album.rename')).toEqual({ album: 'a2', name: 'Day exteriors' });
    // Remove Current Album asks first; Cancel leaves it.
    fireEvent.contextMenu(albumItem('a2'));
    chooseItem('Remove Current Album');
    expect(screen.getByRole('dialog', { name: 'Remove Current Album' }).textContent).toContain('The album will be removed with all its stills. This cannot be undone.');
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    expect(calls(bridge, 'gallery.album.delete')).toEqual([]);
    fireEvent.contextMenu(albumItem('a2'));
    chooseItem('Remove Current Album');
    fireEvent.click(document.querySelector('[data-test="confirm-delete"]')!);
    await settle();
    expect(calls(bridge, 'gallery.album.delete')).toEqual([{ album: 'a2' }]);
    expect(albumNames()).not.toContain('Day exteriors');
  });

  it('labels a still, shows its properties, and exports and imports stills through the dialogs of the shell', async () => {
    const { bridge } = await mountGallery();
    fireEvent.contextMenu(stillTile('s3'));
    chooseItem('Change Label');
    const field = document.querySelector<HTMLInputElement>('[data-test="still-label"]')!;
    fireEvent.change(field, { target: { value: ' Dusk ' } });
    fireEvent.click(document.querySelector('[data-test="label-ok"]')!);
    await settle();
    expect(last(bridge, 'still.label')).toEqual({ still: 's3', label: 'Dusk' });
    expect(within(stillTile('s3')).getByText('Dusk')).toBeDefined();
    // A double-click under the number changes the label too (p. 3262).
    fireEvent.doubleClick(within(stillTile('s1')).getByText('Day'));
    expect((document.querySelector<HTMLInputElement>('[data-test="still-label"]')!).value).toBe('Day');
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    fireEvent.contextMenu(stillTile('s1'));
    chooseItem('Properties');
    await settle();
    const properties = document.querySelector('[data-test="still-properties"]')!.textContent!;
    for (const shown of ['Still ID1.1.1', 'LabelDay', 'TimelineMain', 'ClipInterview', 'Record TC00:00:00:12', 'Source TC00:00:00:12', 'Size1920 × 1080']) {
      expect(properties).toContain(shown);
    }
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Close' }));
    // Export: the stills chosen, through Save As of the shell.
    fireEvent.click(stillTile('s1'));
    fireEvent.click(stillTile('s2'), { shiftKey: true });
    fireEvent.contextMenu(stillTile('s2'));
    chooseItem('Export');
    await settle();
    expect((bridge as unknown as { exportStills: ReturnType<typeof vi.fn> }).exportStills).toHaveBeenCalledWith({ language: 'en', stills: ['s1', 's2'] });
    fireEvent.contextMenu(document.querySelector('[data-test="stills"]')!);
    chooseItem('Import');
    await settle();
    expect((bridge as unknown as { importStills: ReturnType<typeof vi.fn> }).importStills).toHaveBeenCalledWith({ language: 'en', album: 'a1' });
  });

  it('says why an export was refused', async () => {
    await mountGallery(projectWith(), { exported: { ok: false, error: { code: -32001, message: 'exists', data: { reason: 'exists', path: 'C:/x/Still_1.1.1.png' } } } });
    fireEvent.contextMenu(stillTile('s1'));
    chooseItem('Export');
    await settle();
    expect(document.querySelector('[data-test="gallery-problem"]')!.textContent).toBe('Files of these names are there already. Choose another name or place.');
  });

  it('moves stills dragged onto an album', async () => {
    const { bridge } = await mountGallery();
    fireEvent.click(stillTile('s1'));
    fireEvent.click(stillTile('s3'), { ctrlKey: true });
    fireEvent.dragStart(stillTile('s3'));
    fireEvent.dragOver(albumItem('pa1'));
    fireEvent.drop(albumItem('pa1'));
    await settle();
    expect(calls(bridge, 'still.move')).toEqual([{ stills: ['s1', 's3'], album: 'pa1' }]);
    expect(stillIds()).toEqual(['s2']);
    // A still dropped on the album it is in moves nowhere.
    fireEvent.dragStart(stillTile('s2'));
    fireEvent.drop(albumItem('a1'));
    await settle();
    expect(calls(bridge, 'still.move')).toHaveLength(1);
  });

  it('deletes stills at once, and Ctrl+Z in the Gallery brings them back, the project\'s undo elsewhere', async () => {
    const { bridge } = await mountGallery();
    fireEvent.click(stillTile('s1'));
    fireEvent.click(stillTile('s2'), { ctrlKey: true });
    fireEvent.contextMenu(stillTile('s2'));
    chooseItem('Delete Selected');
    await settle();
    // Not confirmed (§9.2): it can be undone.
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(calls(bridge, 'still.delete')).toEqual([{ stills: ['s1', 's2'] }]);
    expect(stillIds()).toEqual(['s3']);
    expect(document.querySelector('[data-test="gallery-notice"]')!.textContent).toBe('Deleted. Press Ctrl+Z in the Gallery to bring them back.');
    fireEvent.contextMenu(stillTile('s3'));
    chooseItem('Delete Selected');
    await settle();
    expect(stillIds()).toEqual([]);
    // Ctrl+Z in the Gallery: the last deletion back, then the one before; the project's history is not stepped.
    await act(() => { fireEvent.keyDown(document.querySelector('[data-test="gallery"]')!, { key: 'z', code: 'KeyZ', ctrlKey: true }); });
    await settle();
    expect(last(bridge, 'still.restore')).toEqual({ stills: ['s3'] });
    expect(stillIds()).toEqual(['s3']);
    await act(() => { fireEvent.keyDown(stillTile('s3'), { key: 'z', code: 'KeyZ', ctrlKey: true }); });
    await settle();
    expect(last(bridge, 'still.restore')).toEqual({ stills: ['s1', 's2'] });
    expect(stillIds()).toEqual(['s1', 's2', 's3']);
    expect(calls(bridge, 'history.undo')).toEqual([]);
    // With nothing left to restore, Ctrl+Z is the project's again.
    await act(() => { fireEvent.keyDown(stillTile('s3'), { key: 'z', code: 'KeyZ', ctrlKey: true }); });
    await settle();
    expect(calls(bridge, 'still.restore')).toHaveLength(2);
    expect(calls(bridge, 'history.undo')).toHaveLength(1);
  });

  it('keeps the focus in the Gallery after Delete Selected from its menu, so that Ctrl+Z there restores and does not undo a grade', async () => {
    const { bridge } = await mountGallery();
    fireEvent.click(stillTile('s2'));
    fireEvent.contextMenu(stillTile('s2'));
    // The menu takes the focus while it is open.
    expect(screen.getByRole('menu', { name: 'Still menu' }).contains(document.activeElement)).toBe(true);
    chooseItem('Delete Selected');
    await settle();
    expect(stillIds()).toEqual(['s1', 's3']);
    // Put away, it gives the focus back to the Gallery, where the deletion was asked for.
    expect(document.querySelector('[data-test="gallery"]')!.contains(document.activeElement)).toBe(true);
    await act(() => { fireEvent.keyDown(document.activeElement!, { key: 'z', code: 'KeyZ', ctrlKey: true }); });
    await settle();
    expect(calls(bridge, 'still.restore')).toEqual([{ stills: ['s2'] }]);
    expect(calls(bridge, 'history.undo')).toEqual([]);
    expect(stillIds()).toEqual(['s1', 's2', 's3']);
  });

  it('keeps grades in memories A–H: Alt+number saves, Ctrl+number and the middle button load, the menu clears, a still dragged on keeps it', async () => {
    const { bridge } = await mountGallery();
    // Hidden at first (p. 3271).
    expect(document.querySelector('[data-test="memories"]')).toBeNull();
    fireEvent.click(document.querySelector('[data-test="toggle-memories"]')!);
    expect([...document.querySelectorAll<HTMLElement>('[data-test="memory"]')].map(each => each.dataset.slot)).toEqual(['A', 'B', 'C', 'D', 'E', 'F', 'G', 'H']);
    await key('1', { code: 'Digit1', altKey: true });
    await settle();
    // The album of the memories, which the engine lists apart from the albums.
    expect(last(bridge, 'gallery.album.create')).toEqual({ kind: 'memories', name: 'Memories' });
    expect(last(bridge, 'still.grab')).toEqual({ view: 'timeline', album: 'a9', clip: 'c1', level: 'clip' });
    expect(albumNames()).toEqual(['Stills 1', 'Looks', 'PowerGrade 1']);
    expect(readMemories('F:/film')).toEqual({ album: 'a9', slots: ['s11', null, null, null, null, null, null, null] });
    expect(document.querySelector<HTMLElement>('[data-test="memory"][data-slot="A"]')!.dataset.filled).toBeDefined();
    await key('1', { code: 'Digit1', ctrlKey: true });
    await settle();
    expect(last(bridge, 'still.apply')).toEqual({ still: 's11', to: [C('c1')], mode: 'apply', keyframes: 'none', revision: 0 });
    // Saved again, the memory takes the new grade, and the still it had goes from the album of the memories.
    await key('1', { code: 'Digit1', altKey: true });
    await settle();
    expect(calls(bridge, 'gallery.album.create')).toHaveLength(1);
    expect(last(bridge, 'still.delete')).toEqual({ stills: ['s11'] });
    expect(readMemories('F:/film').slots[0]).toBe('s12');
    // A still dragged onto memory C is copied into the album of the memories; the middle button applies the copy.
    fireEvent.dragStart(stillTile('s2'));
    fireEvent.drop(document.querySelector('[data-test="memory"][data-slot="C"]')!);
    await settle();
    expect(last(bridge, 'still.copy')).toEqual({ stills: ['s2'], album: 'a9' });
    expect(readMemories('F:/film').slots[2]).toBe('s13');
    expect(stillIds()).toEqual(['s1', 's2', 's3']);
    fireEvent.pointerDown(document.querySelector('[data-test="memory"][data-slot="C"]')!, { button: 1 });
    await settle();
    expect(last(bridge, 'still.apply')).toMatchObject({ still: 's13', mode: 'apply' });
    fireEvent.contextMenu(document.querySelector('[data-test="memory"][data-slot="C"]')!);
    expect(menuItems('Memory menu')).toEqual(['Apply Grade', 'Append Node Graph', 'Clear']);
    chooseItem('Append Node Graph');
    await settle();
    expect(last(bridge, 'still.apply')).toMatchObject({ still: 's13', mode: 'append' });
    fireEvent.contextMenu(document.querySelector('[data-test="memory"][data-slot="C"]')!);
    chooseItem('Clear');
    await settle();
    expect(readMemories('F:/film').slots[2]).toBeNull();
    // Cleared, its copy goes from the album of the memories; the still of the album stays.
    expect(last(bridge, 'still.delete')).toEqual({ stills: ['s13'] });
    expect(stillIds()).toEqual(['s1', 's2', 's3']);
    // An empty memory loads nothing; the memories are the project's.
    const applied = calls(bridge, 'still.apply').length;
    await key('5', { code: 'Digit5', ctrlKey: true });
    await settle();
    expect(calls(bridge, 'still.apply')).toHaveLength(applied);
    expect(readMemories('F:/other').slots.every(each => each === null)).toBe(true);
  });
});

describe('the wipe of the viewer', () => {
  const wipes = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'viewer.wipe') as Record<string, any>[];
  const pointers = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'view.pointer') as Record<string, unknown>[];
  const point = (kind: string, x: number, y: number, more: Record<string, unknown> = {}) => ({
    monitor: 'timeline', kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'],
    x, y, inside: true, window: { x: 10, y: 20 }, shift: false, ctrl: false, alt: false, ...more,
  });

  it.each(['middle', 'left'])('keeps navigation blocked through a held %s Wipe-to-Split handoff and restores it on release', async button => {
    const h = await mountGallery();
    fireEvent.change(screen.getByRole('combobox', { name: 'On-screen controls' }), { target: { value: 'image_wipe' } });
    fireEvent.click(stillTile('s1')); await letterKey('w', { ctrlKey: true }); await settle();
    expect(pointers(h.bridge).at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'resize_ew' });
    await h.tell('view.pointer', point('down', 960, 540, { button, buttons: [button], instance: 1, event: 20,
      presentation: { serial: 1, tag: 1, map: { surface: [960, 540], picture: [1920, 1080], dest: [0, 0, 960, 540] } } })); await settle();
    await letterKey('w', { ctrlKey: true, altKey: true }); await settle();
    expect(pointers(h.bridge).slice(-2)).toEqual([{ monitor: 'timeline', on: false }, { monitor: 'timeline', on: true, cursor: 'arrow' }]);
    const viewer = document.querySelector('[data-test="viewer-box"]')!;
    const zoom = screen.getByRole('combobox', { name: 'Viewer zoom' });
    const mutations = () => calls(h.bridge, 'view.viewport').filter((p: any) => p.operation && p.operation !== 'prepare');
    fireEvent.keyDown(viewer, { key: 'Z', code: 'KeyZ', shiftKey: true });
    fireEvent.change(zoom, { target: { value: '2' } }); await settle();
    expect(mutations()).toEqual([]);
    await h.tell('view.pointer', point('up', 960, 540, { button, buttons: [], instance: 1, event: 21 })); await settle();
    fireEvent.keyDown(viewer, { key: 'Z', code: 'KeyZ', shiftKey: true }); await settle();
    fireEvent.change(zoom, { target: { value: '2' } }); await settle();
    expect(mutations()).toEqual([
      { monitor: 'timeline', instance: 1, operation: 'fit' }, { monitor: 'timeline', instance: 1, operation: 'scale', scale: 2 },
    ]);
    expect(calls(h.bridge, 'window.input')).toEqual([]);
  });

  it('turns on and off with Ctrl+W and the Image Wipe button, inverts with Alt+W, and steps from still to still', async () => {
    const { bridge } = await mountGallery();
    fireEvent.change(screen.getByRole('combobox', { name: 'On-screen controls' }), { target: { value: 'image_wipe' } });
    // No still chosen: nothing to wipe.
    await letterKey('w', { ctrlKey: true });
    expect(wipes(bridge)).toEqual([]);
    fireEvent.click(stillTile('s1'));
    await letterKey('w', { ctrlKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toEqual({ on: true, style: 'horizontal', position: 0.5, angle: 45, invert: false, reference: { kind: 'still', still: 's1' } });
    expect(pointers(bridge).at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'resize_ew' });
    expect(document.querySelector('[data-test="image-wipe"]')!.getAttribute('aria-pressed')).toBe('true');
    expect(stillTile('s1').dataset.wiped).toBeDefined();
    await letterKey('w', { altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ invert: true, reference: { kind: 'still', still: 's1' } });
    // Next and Previous Still choose the still, and the wipe shows it.
    await letterKey('n', { ctrlKey: true, altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ invert: true, reference: { kind: 'still', still: 's2' } });
    expect(stillTile('s2').dataset.current).toBeDefined();
    await letterKey('n', { ctrlKey: true, altKey: true });
    await letterKey('n', { ctrlKey: true, altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { still: 's3' } });
    await letterKey('b', { ctrlKey: true, altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { still: 's2' } });
    // The style, from the second row of the toolbar; then off, and the view lets the pointer go.
    fireEvent.change(document.querySelector('[data-test="wipe-style"]')!, { target: { value: 'vertical' } });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ style: 'vertical', reference: { still: 's2' } });
    expect(pointers(bridge).at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'resize_ns' });
    fireEvent.click(document.querySelector('[data-test="image-wipe"]')!);
    await settle();
    expect(wipes(bridge).at(-1)).toEqual({ on: false });
    expect(pointers(bridge).at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'arrow' });
    // A double-click of a still wipes it, and again puts it away.
    fireEvent.doubleClick(stillTile('s3'));
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ on: true, style: 'vertical', reference: { kind: 'still', still: 's3' } });
    fireEvent.doubleClick(stillTile('s3'));
    await settle();
    expect(wipes(bridge).at(-1)).toEqual({ on: false });
  });

  it('puts the border where the pointer drags it on the picture, sending the whole wipe each time, and turns a diagonal one with Alt', async () => {
    const { bridge, tell } = await mountGallery();
    fireEvent.change(screen.getByRole('combobox', { name: 'On-screen controls' }), { target: { value: 'image_wipe' } });
    fireEvent.click(stillTile('s1'));
    await letterKey('w', { ctrlKey: true });
    await settle();
    const sent = wipes(bridge).length;
    // A move without the button down moves nothing.
    await tell('view.pointer', point('move', 960, 540, { buttons: [] }));
    expect(wipes(bridge)).toHaveLength(sent);
    await tell('view.pointer', point('down', 480, 540));
    await settle();
    expect(wipes(bridge).at(-1)).toEqual({ on: true, style: 'horizontal', position: 0.25, angle: 45, invert: false, reference: { kind: 'still', still: 's1' } });
    await tell('view.pointer', point('move', 1440, 100));
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ position: 0.75 });
    await tell('view.pointer', point('move', 2400, 100, { inside: false }));
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ position: 1 });
    await tell('view.pointer', point('up', 960, 100));
    await tell('view.pointer', point('move', 0, 0));
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ position: 0.5 });
    // Another monitor's pointer is not the viewer's.
    await tell('view.pointer', { ...point('down', 0, 0), monitor: 'source' });
    expect(wipes(bridge).at(-1)).toMatchObject({ position: 0.5 });
    // Diagonal: the border through the pointer, and Alt turns it about the centre.
    fireEvent.change(document.querySelector('[data-test="wipe-style"]')!, { target: { value: 'diagonal' } });
    await settle();
    expect(calls(bridge, 'view.pointer').at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'move' });
    await tell('view.pointer', point('down', 1920, 0, { alt: true }));
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ style: 'diagonal', angle: wipeAngleAt(1920, 0, 1920, 1080) });
    await tell('view.pointer', point('up', 1920, 0));
    // The right button opens the viewer's menu where the pointer is.
    await tell('view.pointer', { ...point('down', 10, 10), button: 'right', buttons: ['right'] });
    expect(menuItems('Viewer menu')).toEqual(['Grab Still', 'Wipe On/Off', 'Split Screen', 'Apply Grade']);
    chooseItem('Apply Grade');
    expect(menuItems('Apply Grade')).toEqual(['No Keyframes', 'Keyframes Aligning Source Timecode', 'Keyframes Aligning Start Frames']);
    chooseItem('Keyframes Aligning Source Timecode');
    await settle();
    expect(last(bridge, 'still.apply')).toMatchObject({ still: 's1', to: [C('c1')], mode: 'apply', keyframes: 'source_timecode' });
  });

  it('wipes a clip of the timeline, outlined in blue, steps it a frame, and with Gang keeps it as far from the current clip as another becomes current', async () => {
    const { bridge, notify } = await mountGallery();
    // Wipe Timeline Clip is of a clip other than the current one.
    fireEvent.contextMenu(thumb('c1').querySelector('.color-thumb-frame')!);
    expect(menuItems('Clip menu')).not.toContain('Wipe Timeline Clip');
    fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
    fireEvent.contextMenu(thumb('c2').querySelector('.color-thumb-frame')!);
    expect(menuItems('Clip menu')[0]).toBe('Wipe Timeline Clip');
    chooseItem('Wipe Timeline Clip');
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ on: true, reference: { kind: 'clip', clip: 'c2', frame: 0 } });
    expect(thumb('c2').dataset.reference).toBeDefined();
    expect(thumb('c1').dataset.reference).toBeUndefined();
    expect((document.querySelector('[data-test="reference-mode"]') as HTMLSelectElement).value).toBe('timeline');
    // Ctrl+Alt+Right and Left step its frame; Gang off, it stays where it is as the current clip changes.
    await key('ArrowRight', { ctrlKey: true, altKey: true });
    await key('ArrowRight', { ctrlKey: true, altKey: true });
    await key('ArrowLeft', { ctrlKey: true, altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'clip', clip: 'c2', frame: 1 } });
    pressThumb('c3');
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'clip', clip: 'c2', frame: 1 } });
    pressThumb('c1');
    await settle();
    // Gang on (the viewer's options menu): ganged from where it is, a clip after the current one and a frame on.
    fireEvent.click(document.querySelector('[data-test="viewer-options"]')!);
    chooseItem('Gang Timeline Wipe With Current Clip');
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'clip', clip: 'c2', frame: 1 } });
    // Stepped back to the playhead's offset, the clip just after the current one follows the playhead.
    await key('ArrowLeft', { ctrlKey: true, altKey: true });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'neighbour', side: 'next', current: 'c1' } });
    expect(thumb('c2').dataset.reference).toBeDefined();
    // As it plays, the state of the transport tells where the reference is now (`compared`), which the outline follows.
    await notify({ compared: [250] });
    await settle();
    expect(thumb('c3').dataset.reference).toBeDefined();
    expect(thumb('c2').dataset.reference).toBeUndefined();
    await notify({ compared: undefined });
    await settle();
    pressThumb('c2');
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'neighbour', side: 'next', current: 'c2' } });
    expect(thumb('c3').dataset.reference).toBeDefined();
    // At the last clip there is none after it: nothing is sent for it.
    const sent = wipes(bridge).length;
    pressThumb('c3');
    await settle();
    expect(wipes(bridge)).toHaveLength(sent);
    // Back in the Gallery mode, the still chosen is wiped again.
    fireEvent.click(stillTile('s2'));
    fireEvent.change(document.querySelector('[data-test="reference-mode"]')!, { target: { value: 'gallery' } });
    await settle();
    expect(wipes(bridge).at(-1)).toMatchObject({ reference: { kind: 'still', still: 's2' } });
  });

  it('splits the screen with Ctrl+Alt+W in the layout chosen last, a tile clicked chosen, and the controls on the picture hidden', async () => {
    const { bridge, tell } = await mountGallery(projectWith({ grouped: true }));
    const splits = () => calls(bridge, 'viewer.split') as Record<string, any>[];
    await letterKey('w', { ctrlKey: true, altKey: true });
    await settle();
    expect(splits().at(-1)).toEqual({ on: true, items: [{ kind: 'current' }], selected: 0, outline: true });
    expect(document.querySelector('[data-test="color-page"]')!.getAttribute('data-split')).toBe('true');
    expect(calls(bridge, 'view.pointer').at(-1)).toEqual({ monitor: 'timeline', on: true, cursor: 'arrow' });
    // Selected Clips follows the clips chosen.
    pressThumb('c1');
    pressThumb('c3', { ctrlKey: true });
    await settle();
    expect(splits().at(-1)).toEqual({ on: true, items: [{ kind: 'current' }, { kind: 'clip', clip: 'c3', frame: 0 }], selected: 0, outline: true });
    // A click on the second tile chooses it, and goes to its clip (p. 3005).
    await tell('view.pointer', point('down', 1500, 500));
    await settle();
    expect(document.querySelector('[data-test="thumbnails"] > [data-current]')!.getAttribute('data-clip')).toBe('c3');
    expect(splits().at(-1)).toMatchObject({ selected: 1 });
    // The other layouts.
    const mode = () => document.querySelector('[data-test="split-mode"]') as HTMLSelectElement;
    expect([...mode().options].map(option => option.textContent)).toEqual(['Current Group', 'Neighbor Clips', 'Selected Clips', 'Selected Still Images']);
    fireEvent.change(mode(), { target: { value: 'neighbor_clips' } });
    await settle();
    // The two before above, the current and the next below (p. 3004), each following the playhead; black where there is none.
    expect(splits().at(-1)).toEqual({ on: true, items: [
      { kind: 'neighbour', side: 'previous', steps: 2, current: 'c3' }, { kind: 'neighbour', side: 'previous', current: 'c3' }, { kind: 'current' },
      { kind: 'black' },
    ], selected: 2, outline: true });
    fireEvent.click(stillTile('s1'));
    fireEvent.click(stillTile('s3'), { ctrlKey: true });
    fireEvent.change(mode(), { target: { value: 'selected_stills' } });
    await settle();
    expect(splits().at(-1)).toMatchObject({ items: [{ kind: 'current' }, { kind: 'still', still: 's1' }, { kind: 'still', still: 's3' }], selected: 0 });
    pressThumb('c1');
    fireEvent.change(mode(), { target: { value: 'current_group' } });
    await settle();
    expect(splits().at(-1)).toMatchObject({ items: [{ kind: 'current' }, { kind: 'clip', clip: 'c2', frame: 0 }], selected: 0 });
    // Show Outline, from the options menu.
    fireEvent.click(document.querySelector('[data-test="viewer-options"]')!);
    chooseItem('Split Screen');
    chooseItem('Show Outline');
    await settle();
    expect(splits().at(-1)).toMatchObject({ outline: false });
    // Off with Ctrl+Alt+W, and on again in the layout chosen last.
    await letterKey('w', { ctrlKey: true, altKey: true });
    await settle();
    expect(splits().at(-1)).toEqual({ on: false });
    expect(document.querySelector('[data-test="color-page"]')!.getAttribute('data-split')).toBeNull();
    await letterKey('w', { ctrlKey: true, altKey: true });
    await settle();
    expect(splits().at(-1)).toMatchObject({ items: [{ kind: 'current' }, { kind: 'clip', clip: 'c2', frame: 0 }] });
  });
});

describe('the Gallery and comparisons on other systems', () => {
  it('are on the Color page, which is the studio\'s on Windows only', async () => {
    (window as { columbina?: unknown }).columbina = { platform: 'darwin' };
    expect(studioPages()).not.toContain('color');
    const engine = fakeEngine(projectWith());
    render(<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />);
    await settle();
    expect(document.querySelector('[data-test="gallery"]')).toBeNull();
    expect(calls(engine.bridge, 'gallery.list')).toEqual([]);
    (window as { columbina?: unknown }).columbina = { platform: 'win32' };
    expect(studioPages()).toContain('color');
  });
});

describe('the rules of the Gallery', () => {
  const tl = { id: 't1' } as Timeline;
  const clips = [clip('c1', 0, 100, { group: 'g1' }), clip('c2', 100, 100, { group: 'g1' }), clip('c3', 200, 50, { group: 'g2' }), clip('c4', 250, 50)];
  const groups = [{ id: 'g1' }, { id: 'g2' }] as Group[];

  it('grab at the mode of the node editor, a clip of no group at its own grade', () => {
    expect(grabParams('a1', clips[0], 'pre_clip')).toEqual({ view: 'timeline', album: 'a1', clip: 'c1', level: 'pre_clip' });
    expect(grabParams(null, clips[3], 'post_clip')).toEqual({ view: 'timeline', clip: 'c4', level: 'clip' });
    expect(grabParams('a2', null, 'timeline')).toEqual({ view: 'timeline', album: 'a2', level: 'clip' });
  });

  it('apply to the clips chosen, else the current one, to the timeline, or to each group once', () => {
    expect(applyTargets('clip', tl, clips, new Set(), clips[1], groups)).toEqual([C('c2')]);
    expect(applyTargets('clip', tl, clips, new Set(['c4', 'c1']), clips[1], groups)).toEqual([C('c1'), C('c4')]);
    expect(applyTargets('timeline', tl, clips, new Set(['c1']), clips[0], groups)).toEqual([{ level: 'timeline', timeline: 't1' }]);
    expect(applyTargets('pre_clip', tl, clips, new Set(['c1', 'c2', 'c3', 'c4']), clips[0], groups)).toEqual([{ level: 'pre_clip', group: 'g1' }, { level: 'pre_clip', group: 'g2' }]);
    expect(applyTargets('clip', null, clips, new Set(), clips[0], groups)).toEqual([]);
    expect(applyTargets('clip', tl, clips, new Set(), null, groups)).toEqual([]);
    expect(applyParams('s1', [C('c1')], 'append', { keyframes: 'none', preserve: 3 }, 7)).toEqual({ still: 's1', to: [C('c1')], mode: 'append', keyframes: 'none', revision: 7 });
    expect(appendNodeTargets(tl, clips, new Set(['c1', 'c3']), clips[0])).toEqual([C('c3')]);
    expect(appendNodeParams(C('c1'), 'n2', [C('c3')], { keyframes: 'start_frame', preserve: 0 }, 2, 40)).toEqual({ from: C('c1'), node: 'n2', to: [C('c3')], keyframes: 'start_frame', revision: 2 });
    // No Keyframes: the node as it is at the playhead.
    expect(appendNodeParams(C('c1'), 'n2', [C('c3')], { keyframes: 'none', preserve: 0 }, 2, 40)).toEqual({ from: C('c1'), node: 'n2', to: [C('c3')], keyframes: 'none', at: 40, revision: 2 });
  });

  it('choose stills by click, Ctrl and Shift, step through them to the ends, and search their labels', () => {
    const stills = startingAlbums()[0].stills;
    expect(pickStills(stills, ['s1'], 's1', 's3', { ctrl: false, shift: true })).toEqual(['s1', 's2', 's3']);
    expect(pickStills(stills, ['s1', 's2'], 's1', 's2', { ctrl: true, shift: false })).toEqual(['s1']);
    expect(pickStills(stills, ['s1'], 's1', 's3', { ctrl: false, shift: false })).toEqual(['s3']);
    expect(stepStill(stills, null, 1)?.id).toBe('s1');
    expect(stepStill(stills, null, -1)?.id).toBe('s3');
    expect(stepStill(stills, 's3', 1)?.id).toBe('s3');
    expect(stepStill([], 's3', 1)).toBeNull();
    expect(searched(stills, ' DAY ').map(each => each.id)).toEqual(['s1']);
    expect(thumbnailHeight(63)).toBe(72);
    expect(thumbnailHeight(160, 2)).toBe(270);
  });

  it('read the settings and memories that are kept, and nothing that is not theirs', () => {
    localStorage.setItem('columbina.studio.gallery.apply', JSON.stringify({ keyframes: 'sideways', preserve: 11 }));
    expect(readApplySettings()).toEqual({ keyframes: 'none', preserve: 0 });
    localStorage.setItem('columbina.studio.memories:F:/film', JSON.stringify({ album: 3, slots: ['s1', 7, '', 's4'] }));
    expect(readMemories('F:/film')).toEqual({ album: null, slots: ['s1', null, null, 's4', null, null, null, null] });
    localStorage.setItem('columbina.studio.memories:F:/film', '{');
    expect(readMemories('F:/film').slots).toHaveLength(8);
  });
});

describe('the rules of comparisons', () => {
  it('put the border of each style where the pointer is, as the engine draws it', () => {
    const at = (style: typeof DEFAULT_WIPE.style, x: number, y: number, angle = 45) => wipePositionAt({ ...DEFAULT_WIPE, style, angle }, x, y, 1920, 1080);
    expect(at('horizontal', 480, 0)).toBe(0.25);
    expect(at('horizontal', -50, 0)).toBe(0);
    expect(at('vertical', 0, 270)).toBe(0.25);
    expect(at('mix', 1920, 0)).toBe(1);
    expect(at('checkerboard', 960, 0)).toBe(0.5);
    expect(at('venetian_blind', 0, 1080)).toBe(1);
    expect(at('box', 960 + 480, 540)).toBe(0.5);
    expect(at('box', 960, 540 + 540)).toBe(1);
    expect(at('difference', 100, 100)).toBeNull();
    // A diagonal at 90° is a level border: its position goes down the frame.
    expect(at('diagonal', 0, 270, 90)).toBeCloseTo(0.25, 6);
    expect(at('diagonal', 480, 0, 0)).toBeCloseTo(0.25, 6);
    expect(wipeAngleAt(1920, 540, 1920, 1080)).toBe(90);
    expect(wipeAngleAt(960, 0, 1920, 1080)).toBe(0);
    expect(wipeAngleAt(960, 540, 1920, 1080)).toBeNull();
    expect([wipeCursor('horizontal'), wipeCursor('venetian_blind'), wipeCursor('box'), wipeCursor('difference')]).toEqual(['resize_ew', 'resize_ns', 'move', null]);
  });

  it('lay a split screen as the engine lays it, and find the tile of a point', () => {
    expect([1, 2, 3, 4, 5, 7, 10, 16].map(count => splitGrid(count))).toEqual([
      { columns: 1, rows: 1 }, { columns: 2, rows: 1 }, { columns: 2, rows: 2 }, { columns: 2, rows: 2 }, { columns: 3, rows: 2 },
      { columns: 3, rows: 3 }, { columns: 4, rows: 3 }, { columns: 4, rows: 4 },
    ]);
    expect(tileAt(3, 100, 100, 1920, 1080)).toBe(0);
    expect(tileAt(3, 1000, 100, 1920, 1080)).toBe(1);
    expect(tileAt(3, 100, 600, 1920, 1080)).toBe(2);
    // The cell that is not there, and a point beyond the frame.
    expect(tileAt(3, 1000, 600, 1920, 1080)).toBeNull();
    expect(tileAt(2, 1920, 10, 1920, 1080)).toBeNull();
    // Cells are cut at whole pixels: of 5 pixels in 2 columns the first has 2.
    expect([0, 1, 2, 3, 4].map(x => tileAt(2, x, 0, 5, 1))).toEqual([0, 0, 1, 1, 1]);
  });

  it('make the items of each layout, and the reference of a timeline wipe with Gang off and on', () => {
    const clips = [clip('c1', 0, 100, { group: 'g' }), clip('c2', 100, 100), clip('c3', 200, 100, { group: 'g' }), clip('c4', 300, 10)];
    const base = { clips, chosen: new Set<string>(), current: clips[2], stills: ['s1'], frame: 230 };
    expect(splitItems('neighbor_clips', base)).toEqual({ items: [
      { kind: 'neighbour', side: 'previous', steps: 2, current: 'c3' }, { kind: 'neighbour', side: 'previous', current: 'c3' }, { kind: 'current' },
      { kind: 'neighbour', side: 'next', current: 'c3' },
    ], selected: 2, clips: ['c1', 'c2', 'c3', 'c4'] });
    // At the first clip the cells before it are black, each in its place.
    expect(splitItems('neighbor_clips', { ...base, current: clips[0], frame: 5 })).toEqual({ items: [
      { kind: 'black' }, { kind: 'black' }, { kind: 'current' }, { kind: 'neighbour', side: 'next', current: 'c1' },
    ], selected: 2, clips: [null, null, 'c1', 'c2'] });
    expect(splitItems('current_group', base).items).toEqual([{ kind: 'clip', clip: 'c1', frame: 0 }, { kind: 'current' }]);
    expect(splitItems('current_group', { ...base, current: clips[1] }).items).toEqual([{ kind: 'current' }]);
    expect(splitItems('selected_stills', { ...base, stills: Array.from({ length: 20 }, (_, at) => `s${at}`) }).items).toHaveLength(16);
    expect(splitItems('selected_clips', { ...base, chosen: new Set(['c4', 'c3']) })).toMatchObject({ items: [{ kind: 'current' }, { kind: 'clip', clip: 'c4', frame: 0 }], selected: 0 });
    // A timeline wipe: Gang off holds the clip at its frame, within it.
    expect(timelineReference({ clip: 'c4', frame: 40 }, null, clips, clips[0], 0)).toEqual({ kind: 'clip', clip: 'c4', frame: 9 });
    expect(timelineReference({ clip: 'gone', frame: 0 }, null, clips, clips[0], 0)).toBeNull();
    // Gang on: as far from the current clip as it was, at the playhead's offset moved as far; the next one unmoved is a neighbour.
    expect(timelineReference({ clip: 'c2', frame: 0 }, { distance: 1, shift: 0 }, clips, clips[1], 120)).toEqual({ kind: 'neighbour', side: 'next', current: 'c2' });
    expect(timelineReference({ clip: 'c2', frame: 0 }, { distance: 2, shift: 3 }, clips, clips[0], 20)).toEqual({ kind: 'clip', clip: 'c3', frame: 23 });
    expect(timelineReference({ clip: 'c2', frame: 0 }, { distance: -1, shift: 0 }, clips, clips[0], 20)).toBeNull();
    // Two away and unmoved, a neighbour of two steps, which follows the playhead too.
    expect(timelineReference({ clip: 'c3', frame: 0 }, { distance: 2, shift: 0 }, clips, clips[0], 20)).toEqual({ kind: 'neighbour', side: 'next', steps: 2, current: 'c1' });
    expect(neighbourClip(clips, clips[3], { kind: 'neighbour', side: 'previous', steps: 2 })?.id).toBe('c2');
    expect(neighbourClip(clips, clips[0], { kind: 'neighbour', side: 'previous' })).toBeNull();
    expect(referenceClip(clips, 150)).toBe('c2');
    expect(referenceClip(clips, null)).toBeNull();
  });
});
