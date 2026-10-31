/**
 * An engine for the tests of the colour of the studio (T18): it keeps a project, applies the
 * operations of a transaction as the copy does, lists the LUTs a test gives it, answers
 * `view.display`, and refuses what a test tells it to, as the real one refuses. And the studio
 * mounted on it, on the page that a test asks for.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { vi } from 'vitest';
import { Words } from '../src/app/locale';
import type { EngineBridge } from '../src/bridge';
import type { zhCN } from '../src/messages/zh-CN';
import { Studio } from '../src/studio/Studio';
import { applyOp, ProjectCopy, type EngineEvent, type Grade, type GradeNode, type LutLibrary, type Op, type Project, type TransportState } from '../src/studio/engine';
import { newCorrector } from '../src/studio/color/nodeEdits';
import { clip, media, timeline } from './timeline-fixture';

export const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });

export const corrector = (id: string, lut: string | null = null): GradeNode => ({ id, kind: 'corrector', label: '', enabled: true, params: { ...newCorrector(), lut } });
/** n1 and n2 in series into a parallel mixer n3 with n4: n1 names `lut`. */
export const lutGrade = (lut: string | null = null): Grade => ({
  nodes: [corrector('n1', lut), corrector('n2'), { id: 'n3', kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 1 } }],
  links: [{ from: 'source', to: 'n1', kind: 'rgb', input: 0 }, { from: 'n1', to: 'n2', kind: 'rgb', input: 0 },
    { from: 'n2', to: 'n3', kind: 'rgb', input: 0 }, { from: 'n3', to: 'output', kind: 'rgb', input: 0 }],
});

/** A library of two roots: the project's with a LUT at its root and one in Looks, and the library's with Film and a shadowed copy. */
export const library = (): LutLibrary => ({
  roots: [{ place: 'project', path: 'F:\\film\\.columbina\\studio', exists: true }, { place: 'library', path: 'C:\\Users\\me\\.columbina\\luts', exists: true }],
  luts: [
    { name: 'Looks/Teal.cube', root: 0, size: 10, modified_us: 1, shadowed: false },
    { name: 'shaper.cube', root: 0, size: 10, modified_us: 1, shadowed: false },
    { name: 'Film/Kodak 2383.cube', root: 1, size: 10, modified_us: 1, shadowed: false },
    { name: 'Film/Fuji 3513.cube', root: 1, size: 10, modified_us: 1, shadowed: false },
    { name: 'shaper.cube', root: 1, size: 10, modified_us: 1, shadowed: true },
  ],
  truncated: false,
});

export function fakeEngine(project: Project, luts: LutLibrary = library()) {
  let revision = 0;
  let refusal: Record<string, unknown> | null = null;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  let state: TransportState = { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 };
  let session = 1;
  const opened = () => {
    const said = { session, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision };
    listeners.forEach(listener => listener({ kind: 'opened', ...said }));
    return said;
  };
  const bridge = {
    open: vi.fn(async () => opened()),
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
      if (method === 'luts.list') return { ok: true, result: luts };
      if (method === 'view.display') {
        const view = { hdr_on: false, hdr_shown: false, peak_nits: 400, sdr_white_nits: 80 };
        return { ok: true, result: { hdr: params?.hdr ?? true, source: view, timeline: view } };
      }
      if (method === 'transaction.apply') {
        if (refusal) return { ok: false, error: { code: -32001, message: 'refused', data: refusal } };
        const done = (params!.ops as Op[]).filter(op => 'op' in op);
        for (const op of done) project = applyOp(project, op);
        revision++;
        tell('project.changed', { revision, made_by: 'desktop', by: 'desktop', next_id: project.next_id, done });
        return { ok: true, result: { done, names: {}, revision, next_id: project.next_id, clamped: [] } };
      }
      return { ok: true, result: {} };
    }),
    importFiles: vi.fn(async () => ({ ok: true, result: null })), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn> };
  return {
    bridge, copy: new ProjectCopy(bridge),
    project: () => project,
    refuse: (data: Record<string, unknown> | null) => { refusal = data; },
    /** The engine is started again: a new session is opened. */
    restart: () => act(() => { session += 1; opened(); }),
    notify: (fields: Record<string, unknown>) => act(() => { state = { ...state, ...fields } as TransportState; tell('transport.state', state); }),
  };
}

/** A project of two media and a timeline of two clips, the first graded with `lutGrade`. */
export const projectOf = (fields: Partial<Project> = {}, grade: Grade = lutGrade()): Project => ({
  schema: 3, next_id: 10, bins: [], groups: [],
  media: [media, { ...media, id: 'm2', name: 'Street' }, { ...media, id: 'm3', name: 'Music', facts: { ...media.facts, video: null } }],
  timelines: [timeline([clip('c1', 0, 100, { grade }), clip('c2', 100)])],
  ...fields,
});

/** The studio on its engine, on a page: Media at first, or the one named. */
export async function mount(project: Project = projectOf(), { page, words, luts }: { page?: 'Color' | 'Media'; words?: typeof zhCN; luts?: LutLibrary } = {}) {
  const engine = fakeEngine(project, luts);
  const tree = <Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />;
  const view = render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle();
  if (page === 'Color') { fireEvent.click(screen.getByRole('tab', { name: words ? '调色' : 'Color' })); await settle(); }
  return { ...engine, ...view };
}

export const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
/** The operations of each transaction sent. */
export const sent = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'transaction.apply').map(params => (params as { ops: unknown[] }).ops);
