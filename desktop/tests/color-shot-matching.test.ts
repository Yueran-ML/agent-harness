import { afterEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { applyOp, emptyGrade, type EngineEvent, type Grade, type Op, type ScopeStatistics, type Target } from '../src/studio/engine';
import { newCorrectorNode } from '../src/studio/color/nodeEdits';
import { neutralMatch } from '../src/studio/color/shotmatch';
import { matchShots, type MatchRequest } from '../src/studio/color/shotMatching';
import { clip, timeline } from './timeline-fixture';
import { projectOf } from './colorEngine';

const graded = (...ids: string[]): Grade => {
  const path = ['source', ...ids, 'output'];
  return { nodes: ids.map(id => newCorrectorNode(id)), links: path.slice(1).map((to, i) => ({ from: path[i], to, kind: 'rgb', input: 0 })) };
};
const target = (id: string): Target => ({ level: 'clip', timeline: 't1', clip: id });
const atClip = (id: string, grade = graded(`n${id}`)) => clip(id, Number(id.slice(1)) * 100, 100, { grade });
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

function studio(grades = [atClip('c1'), atClip('c2'), atClip('c3')]) {
  let project = projectOf({ timelines: [timeline(grades)] });
  let revision = 0;
  let session = 1;
  let active: { id: string; base: number; touched: number } | null = null;
  const history: typeof project[] = [];
  const listeners = new Set<(event: EngineEvent) => void>();
  let measured: (params: Record<string, unknown>) => Promise<void> = async () => {};
  const tell = (event: EngineEvent) => listeners.forEach(listener => listener(event));
  const call = vi.fn(async (method: string, params?: Record<string, unknown>) => {
    if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
    if (method === 'scopes.statistics') {
      await measured(params!);
      const histogram = Array<number>(1024).fill(0);
      for (let i = 100; i < 850; i++) histogram[i] = 1;
      return { ok: true, result: { histogram: { bins: 1024, red: histogram, green: histogram, blue: histogram, luma: histogram },
        red: { mean: 0.5 }, green: { mean: 0.5 }, blue: { mean: 0.5 } } as ScopeStatistics };
    }
    if (method === 'transaction.apply') {
      const gesture = params!.gesture as { id: string; end?: boolean } | undefined;
      if (active && Date.now() - active.touched >= 10000) active = null;
      const same = active !== null && active.id === gesture?.id;
      const asked = params!.revision as number;
      if (asked !== revision && !(same && asked >= active!.base)) return { ok: false, error: { code: -32001, message: 'stale', data: { reason: 'stale_revision' } } };
      const before = structuredClone(project);
      const done: Op[] = [];
      for (const input of params!.ops as (Op | { edit: 'add_serial'; grade: Target; node: string })[]) {
        let op: Op;
        if ('edit' in input) {
          const id = `n${project.next_id++}`;
          op = { op: 'add_node', grade: input.grade, node: newCorrectorNode(id), links: [
            { from: 'source', to: id, kind: 'rgb', input: 0 }, { from: id, to: 'output', kind: 'rgb', input: 0 },
          ] };
        } else op = input;
        project = applyOp(project, op); done.push(op);
      }
      if (!same) history.push(before);
      if (gesture) active = { id: gesture.id, base: same ? active!.base : revision, touched: Date.now() };
      else active = null;
      revision++;
      if (gesture?.end) {
        const original = history.at(-1)!;
        if (JSON.stringify(original.timelines) === JSON.stringify(project.timelines)) history.pop();
        active = null;
      }
      return { ok: true, result: { revision, done, next_id: project.next_id, names: {}, clamped: [] } };
    }
    return { ok: true, result: {} };
  });
  const bridge = { call, onEvent: (listen: (event: EngineEvent) => void) => { listeners.add(listen); return () => { listeners.delete(listen); }; } } as Pick<EngineBridge, 'call' | 'onEvent'>;
  return {
    call, bridge, request: (more: Partial<MatchRequest> = {}): MatchRequest => ({ bridge, revision, session, timeline: 't1', reference: grades[0], clips: grades.slice(1), nodes: {}, ...more }),
    grade: (id: string) => project.timelines[0].tracks.flatMap(t => t.kind === 'subtitle' ? [] : t.clips).find(c => c.id === id)!.grade,
    history, measure: (next: typeof measured) => { measured = next; },
    restart: () => { session++; tell({ kind: 'opened', session, project: 'other', owned: true, pid: 10, revision: 0 }); },
    outside: (ops: Op[]) => { active = null; for (const op of ops) project = applyOp(project, op); revision++; },
    undo: () => { active = null; project = history.pop()!; revision++; },
  };
}

afterEach(() => vi.useRealTimers());

describe('matching selected clips', () => {
  it('measures each selected node at the clip middle and writes all matches in one transaction', async () => {
    const first = atClip('c1'); const second = atClip('c2', graded('n2a', 'n2b')); const third = atClip('c3', graded('n3a', 'n3b'));
    const engine = studio([first, second, third]);
    await matchShots(engine.request({ clips: [first, second, third], nodes: { 'clip:c2': 'n2b', 'clip:c3': 'n3a' } }));
    const measured = engine.call.mock.calls.filter(([method]) => method === 'scopes.statistics').map(([, params]) => params);
    expect(measured).toEqual([
      { timeline: 't1', frame: 150, grade: target('c1'), node: 'nc1', bins: 1024 },
      { timeline: 't1', frame: 250, grade: target('c2'), node: 'n2b', bins: 1024 },
      { timeline: 't1', frame: 350, grade: target('c3'), node: 'n3a', bins: 1024 },
    ]);
    const writes = engine.call.mock.calls.filter(([method]) => method === 'transaction.apply');
    expect(writes).toHaveLength(1);
    expect(writes[0][1]).toMatchObject({ revision: 0, ops: [
      { op: 'set_node', grade: target('c2'), id: 'n2b', params: { shot_match: neutralMatch() } },
      { op: 'set_node', grade: target('c3'), id: 'n3a', params: { shot_match: neutralMatch() } },
    ] });
    expect(engine.grade('c1')).toEqual(first.grade);
    expect(engine.history).toHaveLength(1);
  });

  it('creates empty grades inside one gesture and removes the temporary reference node', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    await matchShots(engine.request());
    const writes = engine.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params!);
    expect(writes).toHaveLength(2);
    expect(writes[1].gesture).toEqual({ ...writes[0].gesture as object, end: true });
    expect(engine.grade('c1')).toEqual(emptyGrade());
    expect(engine.grade('c2').nodes).toHaveLength(1);
    expect(engine.history).toHaveLength(1);
    expect(engine.history[0].timelines[0]).toEqual(timeline([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]));
  });

  it('restores added nodes without leaving history when measuring fails', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    engine.measure(async () => { throw new Error('cannot decode'); });
    await expect(matchShots(engine.request())).rejects.toThrow('cannot decode');
    expect(engine.grade('c1')).toEqual(emptyGrade()); expect(engine.grade('c2')).toEqual(emptyGrade());
    expect(engine.history).toHaveLength(0);
  });

  it('refuses a stale result instead of overwriting a concurrent edit', async () => {
    const engine = studio();
    engine.measure(async () => { engine.outside([{ op: 'set_node', grade: target('c2'), id: 'nc2', label: 'Agent edit' }]); });
    await expect(matchShots(engine.request())).rejects.toThrow('stale');
    expect(engine.grade('c2').nodes[0].label).toBe('Agent edit');
    expect(engine.grade('c2').nodes[0]).toMatchObject({ params: { shot_match: null } });
  });

  it('does not write or clean up in a replacement project', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    engine.measure(async () => engine.restart());
    await expect(matchShots(engine.request())).rejects.toThrow();
    expect(engine.call.mock.calls.filter(([m]) => m === 'transaction.apply')).toHaveLength(1);
  });

  it('restores temporary nodes if the page is left while waiting for a measurement', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    const abort = new AbortController();
    engine.measure(async () => abort.abort());
    await expect(matchShots(engine.request({ signal: abort.signal }))).rejects.toThrow();
    expect(engine.grade('c1')).toEqual(emptyGrade()); expect(engine.grade('c2')).toEqual(emptyGrade());
    expect(engine.history).toHaveLength(0);
  });

  it.each(['result', 'error'] as const)('restores immediately on abort, before a measurement gives a late %s', async (late) => {
    vi.useFakeTimers();
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    const abort = new AbortController();
    let release!: () => void;
    let reject!: (error: Error) => void;
    const held = new Promise<void>((resolve, fail) => { release = resolve; reject = fail; });
    engine.measure(async () => held);
    const outcome = matchShots(engine.request({ signal: abort.signal })).then(() => null, error => error as Error);
    await flush();
    expect(engine.history).toHaveLength(1);
    abort.abort(); await flush();
    // The read is still pending; rollback must use the free main connection now, not after
    // that read lets the engine's ten-second gesture window expire.
    expect(engine.grade('c1')).toEqual(emptyGrade()); expect(engine.grade('c2')).toEqual(emptyGrade());
    expect(engine.history).toHaveLength(0);
    expect(await outcome).toMatchObject({ reason: 'cancelled' });
    const writes = engine.call.mock.calls.filter(([method]) => method === 'transaction.apply').length;
    await vi.advanceTimersByTimeAsync(15000);
    if (late === 'result') release(); else reject(new Error('late decode failure'));
    await flush();
    expect(engine.call.mock.calls.filter(([method]) => method === 'transaction.apply')).toHaveLength(writes);
    expect(engine.history).toHaveLength(0);
  });

  it('does not roll back into a replacement session when aborting a pending measurement', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    const abort = new AbortController();
    let release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    engine.measure(async () => held);
    const outcome = matchShots(engine.request({ signal: abort.signal })).then(() => null, error => error as Error);
    await flush();
    engine.restart(); abort.abort(); await flush();
    release(); await outcome;
    expect(engine.call.mock.calls.filter(([method]) => method === 'transaction.apply')).toHaveLength(1);
  });

  it('matches the reference as it looks, including its existing match', async () => {
    const reference = atClip('c1');
    const node = reference.grade.nodes[0];
    if (node.kind === 'corrector') node.params.shot_match = { ...neutralMatch(), gain: { r: 0.8, g: 1, b: 1.1 } };
    const engine = studio([reference, atClip('c2')]);
    await matchShots(engine.request());
    expect(engine.grade('c1')).toEqual(reference.grade);
    expect(engine.grade('c2').nodes[0]).toMatchObject({ params: { shot_match: neutralMatch() } });
  });

  it('removes the receiving node old match before solving its replacement', async () => {
    const receiving = atClip('c2');
    const node = receiving.grade.nodes[0];
    if (node.kind === 'corrector') node.params.shot_match = { ...neutralMatch(), gain: { r: 0.9, g: 1, b: 1 } };
    const engine = studio([atClip('c1'), receiving]);
    await matchShots(engine.request());
    const matched = engine.grade('c2').nodes[0];
    expect(matched).toMatchObject({ kind: 'corrector' });
    if (matched.kind === 'corrector') expect(matched.params.shot_match!.gain.r).toBeCloseTo(0.9, 2);
  });

  it('leaves a concurrent edit intact when temporary grades cannot safely be restored', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    engine.measure(async () => { engine.outside([{ op: 'set_node', grade: target('c2'), id: engine.grade('c2').nodes[0].id, label: 'Keep this' }]); });
    await expect(matchShots(engine.request())).rejects.toThrow('cleanup');
    expect(engine.grade('c2').nodes[0]).toMatchObject({ label: 'Keep this', params: { shot_match: null } });
    expect(engine.call.mock.calls.filter(([m]) => m === 'transaction.apply').every(([, p]) => p!.revision === 0)).toBe(true);
  });

  it('refuses after a gesture expires instead of creating a second undo step', async () => {
    vi.useFakeTimers();
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    engine.measure(async () => { vi.setSystemTime(Date.now() + 11000); });
    await expect(matchShots(engine.request())).rejects.toThrow('cleanup');
    expect(engine.history).toHaveLength(1);
    expect(engine.grade('c2').nodes[0]).toMatchObject({ params: { shot_match: null } });
  });

  it('does not re-create nodes when another client undoes their temporary creation', async () => {
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    let undone = false;
    engine.measure(async () => { if (!undone) { engine.undo(); undone = true; } });
    await expect(matchShots(engine.request())).rejects.toThrow('cleanup');
    expect(engine.grade('c1')).toEqual(emptyGrade()); expect(engine.grade('c2')).toEqual(emptyGrade());
    expect(engine.history).toHaveLength(0);
  });

  it.each(['locked', 'mixer', 'animated'] as const)('checks a %s selected node before creating any other nodes', async (kind) => {
    const node = newCorrectorNode('n2');
    if (kind === 'locked') node.locked = true;
    if (kind === 'animated' && node.kind === 'corrector') node.params.shot_match = { ...neutralMatch(), contrast: { keys: [{ frame: 0, value: 1, interp: 'linear' }] } };
    const grade: Grade = { ...graded('n2'), nodes: [kind === 'mixer' ? { id: 'n2', label: '', enabled: true, kind: 'parallel_mixer', params: { inputs: 1 } } : node] };
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', grade)]);
    await expect(matchShots(engine.request())).rejects.toThrow();
    expect(engine.call).not.toHaveBeenCalled();
  });

  it('keeps a slow set of measurements in a single gesture', async () => {
    vi.useFakeTimers();
    const engine = studio([atClip('c1', emptyGrade()), atClip('c2', emptyGrade())]);
    let release!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    engine.measure(async () => held);
    const work = matchShots(engine.request());
    void work.catch(() => {});
    await flush();
    await vi.advanceTimersByTimeAsync(15000);
    release(); await work;
    expect(engine.history).toHaveLength(1);
    expect(engine.grade('c1')).toEqual(emptyGrade());
  });
});
