import { describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import {
  applyOp, callEngine, deviceRect, EngineError, ProjectCopy,
  type Clip, type EngineAnswer, type EngineEvent, type Media, type Op, type Project, type ProjectChanged, type Proxy,
} from '../src/studio/engine';

const SCALE = 15360;
const empty = (): Project => ({ schema: 1, next_id: 1, media: [], timelines: [] });
const media = (id: string, audio = true): Media => ({
  id, path: `C:\\media\\${id}.mp4`,
  identity: { size: 1000, modified_us: 1_790_000_000_000_000, digest: '0'.repeat(64) },
  facts: {
    duration: { ticks: 10 * SCALE, scale: SCALE },
    video: { width: 1920, height: 1080, rate: { num: 25, den: 1 }, codec: 'h264', bits: 8, rotation: 0 },
    audio: audio ? { rate: 48000, channels: 2, codec: 'aac' } : null,
  },
});
const clip = (id: string, start: number, length: number, from = 'm1', ticks = 0): Clip => ({ id, media: from, source_in: { ticks, scale: SCALE }, start, length });
const proxy: Proxy = { path: 'C:\\film\\.columbina\\studio\\cache\\proxies\\m1-000000000000.mov', width: 1920, height: 1080, codec: 'hevc', digest_of_source: '0'.repeat(64) };

/** A bridge of the engine whose engine has the project that a test sets, at its revision. */
function engineOf(start: { revision: number; project: Project }) {
  let current = start;
  const listeners = new Set<(event: EngineEvent) => void>();
  let gate: Promise<void> | null = null;
  const call = vi.fn(async (method: string, _params?: Record<string, unknown>): Promise<EngineAnswer> => {
    if (method !== 'project.get') return { ok: false, error: { code: -32601, message: 'no such method', data: null } };
    // What the engine has when it is asked, answered once the test lets it.
    const answer = { revision: current.revision, next_id: current.project.next_id, project: structuredClone(current.project) };
    if (gate) await gate;
    return { ok: true, result: answer };
  });
  const bridge: Pick<EngineBridge, 'call' | 'onEvent'> = {
    call,
    onEvent: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  };
  let session = 1;
  return {
    bridge, call, listeners,
    /** Set what the engine has now. */
    have(revision: number, project: Project) { current = { revision, project }; },
    /** Hold the answers to `project.get` until the promise that is returned is kept. */
    hold() { let open!: () => void; gate = new Promise(resolve => { open = resolve; }); return () => { gate = null; open(); }; },
    tell(event: EngineEvent) { for (const listener of [...listeners]) listener(event); },
    opened(next = session) { session = next; this.tell({ kind: 'opened', session, project: 'C:\\film\\.columbina\\studio', owned: true, pid: 600, revision: current.revision }); },
    changed(change: ProjectChanged) { this.tell({ kind: 'notification', session, method: 'project.changed', params: change }); },
  };
}

const settle = async () => { for (let at = 0; at < 10; at += 1) await new Promise(resolve => setImmediate(resolve)); };
const change = (revision: number, next_id: number, ...done: Op[]): ProjectChanged => ({ revision, by: 'agent', made_by: 'agent', done, next_id });

describe('the copy of the project', () => {
  it('applies each operation as the engine did it, and comes to the project that the engine has', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    expect(copy.getSnapshot()).toMatchObject({ status: 'connected', revision: 0, project: empty() });
    const script: ProjectChanged[] = [
      // Media imported: two files, one transaction.
      change(1, 3, { op: 'add_media', media: media('m1'), index: 0 }, { op: 'add_media', media: media('m2', false), index: 1 }),
      // A timeline with a track of picture, whose clips come in the order of their start.
      change(2, 6, { op: 'add_timeline', index: 0, timeline: { id: 't3', name: 'Cut', rate: { num: 25, den: 1 }, tracks: [
        { id: 'V1', kind: 'video', clips: [clip('c5', 100, 20), clip('c4', 0, 50)] },
      ] } }),
      change(3, 6, { op: 'add_track', timeline: 't3', index: 1, track: { id: 'A1', kind: 'audio', clips: [] } }),
      // A clip put between two others by its start.
      change(4, 8, { op: 'insert_clip', timeline: 't3', track: 'V1', clip: clip('c6', 200, 25) },
        { op: 'insert_clip', timeline: 't3', track: 'V1', clip: clip('c7', 60, 10) }),
      // Moved past the others: the order follows the start.
      change(5, 8, { op: 'move_clip', timeline: 't3', id: 'c7', track: 'V1', start: 300 }),
      change(6, 8, { op: 'trim_clip', timeline: 't3', id: 'c4', source_in: { ticks: SCALE, scale: SCALE }, start: 10, length: 40 }),
      // A proxy that the engine made, which is no step of the history.
      { ...change(7, 8, { op: 'set_proxy', media: 'm1', proxy }), by: 'engine', made_by: 'engine' },
      change(8, 8, { op: 'remove_clip', timeline: 't3', id: 'c6' }),
      change(9, 9, { op: 'insert_clip', timeline: 't3', track: 'A1', clip: clip('c8', 0, 25) }),
      // A timeline put before the first, and a track that is taken away with its clips.
      change(10, 10, { op: 'add_timeline', index: 0, timeline: { id: 't9', name: 'Other', rate: { num: 30000, den: 1001 }, tracks: [] } },
        { op: 'remove_track', timeline: 't3', id: 'A1' }),
      // A clip moved onto a track of its kind on which it starts first.
      change(11, 10, { op: 'add_track', timeline: 't3', track: { id: 'V2', kind: 'video', clips: [clip('c9', 20, 5, 'm2')] } },
        { op: 'move_clip', timeline: 't3', id: 'c7', track: 'V2', start: 0 }),
      change(12, 10, { op: 'set_proxy', media: 'm1', proxy: null }, { op: 'remove_clip', timeline: 't3', id: 'c9' },
        { op: 'remove_media', id: 'm2' }, { op: 'remove_timeline', id: 't9' }),
      // An undo: the media comes back at its place.
      change(13, 10, { op: 'add_media', media: media('m2', false), index: 1 }),
    ];
    for (const each of script) engine.changed(each);
    const expected: Project = {
      schema: 1, next_id: 10,
      media: [media('m1'), media('m2', false)],
      timelines: [{
        id: 't3', name: 'Cut', rate: { num: 25, den: 1 }, tracks: [
          { id: 'V1', kind: 'video', clips: [clip('c4', 10, 40, 'm1', SCALE), clip('c5', 100, 20)] },
          { id: 'V2', kind: 'video', clips: [clip('c7', 0, 10)] },
        ],
      }],
    };
    // Strictly: a proxy taken away is left out, as the engine writes it.
    expect(copy.getSnapshot().project).toStrictEqual(expected);
    expect(copy.getSnapshot().revision).toBe(13);
    // And it is what the engine gives when it is asked anew.
    engine.have(13, expected);
    await copy.refetch();
    expect(copy.getSnapshot().project).toStrictEqual(expected);
    expect(engine.call).toHaveBeenCalledTimes(2);
  });

  it('holds the proxy of media while it is set, and the order of the clips of a track by their start', () => {
    let project: Project = { ...empty(), media: [media('m1')], timelines: [{ id: 't1', name: 'T', rate: { num: 25, den: 1 }, tracks: [{ id: 'V1', kind: 'video', clips: [] }] }] };
    project = applyOp(project, { op: 'set_proxy', media: 'm1', proxy });
    expect(project.media[0].proxy).toEqual(proxy);
    for (const [id, start] of [['c1', 50], ['c2', 10], ['c3', 30], ['c4', 0]] as const) {
      project = applyOp(project, { op: 'insert_clip', timeline: 't1', track: 'V1', clip: clip(id, start, 5) });
    }
    expect(project.timelines[0].tracks[0].clips.map(each => each.id)).toEqual(['c4', 'c2', 'c3', 'c1']);
    // What did not change is the same object: a view of it need not be drawn again.
    const before = project;
    const after = applyOp(project, { op: 'add_timeline', timeline: { id: 't2', name: 'U', rate: { num: 25, den: 1 }, tracks: [] } });
    expect(after.timelines[0]).toBe(before.timelines[0]);
    expect(after.media).toBe(before.media);
    expect(before.timelines).toHaveLength(1);
  });

  it('refuses what names a thing that the copy does not hold', () => {
    for (const op of [
      { op: 'remove_media', id: 'm9' }, { op: 'remove_timeline', id: 't9' }, { op: 'remove_clip', timeline: 't9', id: 'c1' },
      { op: 'move_clip', timeline: 't1', id: 'c9', track: 'V1', start: 0 }, { op: 'add_media', media: media('m2'), index: 5 },
      { op: 'set_proxy', media: 'm9', proxy: null }, { op: 'remove_track', timeline: 't1', id: 'A1' }, { op: 'paint' },
    ] as Op[]) {
      expect(() => applyOp({ ...empty(), timelines: [{ id: 't1', name: 'T', rate: { num: 25, den: 1 }, tracks: [{ id: 'V1', kind: 'video', clips: [] }] }] }, op)).toThrow();
    }
  });

  it('fetches the project anew where a revision is skipped, and passes over one that it has', async () => {
    const engine = engineOf({ revision: 4, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    const seen = copy.getSnapshot();
    engine.changed(change(3, 1, { op: 'add_media', media: media('m1') }));
    engine.changed(change(4, 1, { op: 'add_media', media: media('m1') }));
    expect(copy.getSnapshot()).toBe(seen);
    const later = { ...empty(), next_id: 3, media: [media('m1'), media('m2')] };
    engine.have(6, later);
    engine.changed(change(6, 3, { op: 'add_media', media: media('m2') }));
    await settle();
    expect(engine.call).toHaveBeenCalledTimes(2);
    expect(copy.getSnapshot()).toMatchObject({ status: 'connected', revision: 6, project: later });
  });

  it('fetches the project anew where a change does not fit the copy', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    engine.have(1, { ...empty(), next_id: 2 });
    engine.changed(change(1, 2, { op: 'remove_media', id: 'm1' }));
    await settle();
    expect(engine.call).toHaveBeenCalledTimes(2);
    expect(copy.getSnapshot()).toMatchObject({ revision: 1, project: { next_id: 2 } });
  });

  it('applies what it is told while it fetches after the fetch, from the revision that the fetch answered', async () => {
    const engine = engineOf({ revision: 1, project: { ...empty(), next_id: 2, media: [media('m1')] } });
    const copy = new ProjectCopy(engine.bridge);
    const release = engine.hold();
    engine.opened();
    // The change of revision 1 is in the answer; the one of revision 2 came after it.
    engine.changed(change(1, 2, { op: 'add_media', media: media('m1') }));
    engine.changed(change(2, 3, { op: 'add_media', media: media('m2') }));
    expect(copy.getSnapshot().status).toBe('disconnected');
    release();
    await settle();
    expect(copy.getSnapshot()).toMatchObject({ status: 'connected', revision: 2, project: { next_id: 3, media: [media('m1'), media('m2')] } });
    expect(engine.call).toHaveBeenCalledOnce();
  });

  it('is disconnected once the engine has ended, keeps the project as it was, and fetches it when an engine is opened again', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    engine.tell({ kind: 'notification', session: 1, method: 'transport.state', params: { media: 'm1', frame: 0 } });
    engine.tell({ kind: 'attached', session: 1, adapter: 'A', backend: 'Dx12' });
    engine.tell({ kind: 'exited', session: 1, expected: false, code: 1, signal: null });
    expect(copy.getSnapshot()).toMatchObject({ status: 'disconnected', revision: 0, project: empty(), transport: null, view: { state: 'none' } });
    // What is told now is passed over: a new engine is fetched from.
    engine.changed(change(1, 2, { op: 'add_media', media: media('m1') }));
    expect(copy.getSnapshot().revision).toBe(0);
    engine.have(0, { ...empty(), next_id: 2, media: [media('m1')] });
    engine.opened(2);
    await settle();
    expect(copy.getSnapshot()).toMatchObject({ status: 'connected', session: 2, revision: 0, project: { media: [media('m1')] } });
    // What an engine that is no longer open tells is passed over.
    engine.tell({ kind: 'exited', session: 1, expected: true, code: 0, signal: null });
    expect(copy.getSnapshot().status).toBe('connected');
  });

  it('lets go of a fetch from an engine that ended meanwhile', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    const release = engine.hold();
    engine.opened();
    engine.tell({ kind: 'exited', session: 1, expected: false, code: 1, signal: null });
    release();
    await settle();
    expect(copy.getSnapshot()).toMatchObject({ status: 'disconnected', project: null });
  });

  it('keeps what the transport, the proxies, the view and the errors of the engine told last', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    const state = { media: 'm1', frame: 37, playing: false, speed: 1, exact: true, shown: 37, last: 99, sound: 'off', source: 'proxy', proxies: true };
    engine.tell({ kind: 'notification', session: 1, method: 'transport.state', params: state });
    engine.tell({ kind: 'notification', session: 1, method: 'media.proxy', params: { media: 'm1', state: 'running', progress: 0.5 } });
    engine.tell({ kind: 'notification', session: 1, method: 'media.proxy', params: { media: 'm2', state: 'queued' } });
    engine.tell({ kind: 'notification', session: 1, method: 'media.proxy', params: { media: 'm1', state: 'done' } });
    engine.tell({ kind: 'notification', session: 1, method: 'engine.error', params: { message: 'the project could not be saved', kind: 'save' } });
    engine.tell({ kind: 'unsupported', session: 1, reason: 'headless', message: 'no such method' });
    expect(copy.getSnapshot()).toMatchObject({
      transport: state, proxies: { m1: { media: 'm1', state: 'done' }, m2: { media: 'm2', state: 'queued' } },
      error: 'the project could not be saved', view: { state: 'unsupported', reason: 'headless' },
    });
    engine.tell({ kind: 'attached', session: 1, adapter: 'A', backend: 'Dx12' });
    expect(copy.getSnapshot().view).toEqual({ state: 'attached', adapter: 'A', backend: 'Dx12' });
    engine.tell({ kind: 'error', session: 1, message: 'the view could not be made' });
    expect(copy.getSnapshot().error).toBe('the view could not be made');
  });

  it('tells whoever subscribed of each change, with a new snapshot, and stops when it is asked to', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    const heard = vi.fn();
    const stop = copy.subscribe(heard);
    engine.opened();
    await settle();
    const first = copy.getSnapshot();
    expect(heard).toHaveBeenCalled();
    // Nothing changed is the same snapshot, as useSyncExternalStore needs.
    expect(copy.getSnapshot()).toBe(first);
    heard.mockClear();
    engine.changed(change(1, 2, { op: 'add_media', media: media('m1') }));
    expect(heard).toHaveBeenCalledOnce();
    expect(copy.getSnapshot()).not.toBe(first);
    expect(first.revision).toBe(0);
    stop();
    engine.changed(change(2, 3, { op: 'add_media', media: media('m2') }));
    expect(heard).toHaveBeenCalledOnce();
    expect(copy.getSnapshot().revision).toBe(2);
    // Disposed, it hears the engine no more.
    copy.dispose();
    expect(engine.listeners.size).toBe(0);
  });

  it('takes a proxy away where the operation leaves it out', () => {
    const project: Project = { ...empty(), media: [{ ...media('m1'), proxy }] };
    const changed = applyOp(project, { op: 'set_proxy', media: 'm1' } as Op);
    expect(changed.media[0]).toStrictEqual(media('m1'));
    expect('proxy' in changed.media[0]).toBe(false);
  });

  it('tries a fetch that failed again, a little later each time, while the engine is open', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    try {
      const engine = engineOf({ revision: 2, project: empty() });
      const busy = { ok: false as const, error: { code: null, message: 'The connection to the engine was closed.', data: null } };
      engine.call.mockResolvedValueOnce(busy).mockResolvedValueOnce(busy).mockResolvedValueOnce(busy);
      const copy = new ProjectCopy(engine.bridge);
      engine.opened();
      await vi.advanceTimersByTimeAsync(0);
      expect(copy.getSnapshot()).toMatchObject({ status: 'disconnected', error: 'The connection to the engine was closed.' });
      expect(engine.call).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(250);
      expect(engine.call).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(499);
      expect(engine.call).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1);
      expect(engine.call).toHaveBeenCalledTimes(3);
      await vi.advanceTimersByTimeAsync(1000);
      expect(copy.getSnapshot()).toMatchObject({ status: 'connected', revision: 2, error: null });
      expect(engine.call).toHaveBeenCalledTimes(4);
      // Once the engine has ended, nothing more is tried.
      engine.call.mockResolvedValue(busy);
      engine.changed(change(5, 1));
      await vi.advanceTimersByTimeAsync(0);
      engine.tell({ kind: 'exited', session: 1, expected: false, code: 1, signal: null });
      const asked = engine.call.mock.calls.length;
      await vi.advanceTimersByTimeAsync(60_000);
      expect(engine.call.mock.calls.length).toBe(asked);
    } finally {
      vi.useRealTimers();
    }
  });

  it('hears why an engine of a later session could not be opened, and that one is being taken over', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    const copy = new ProjectCopy(engine.bridge);
    engine.opened();
    await settle();
    engine.tell({ kind: 'taking-over', session: 2, pid: 31 });
    expect(copy.getSnapshot()).toMatchObject({ session: 1, takingOver: true });
    // What the agent's engine tells as it yields is passed over.
    engine.tell({ kind: 'notification', session: 2, method: 'engine.yielding', params: { to: 'desktop' } });
    engine.tell({ kind: 'error', session: 2, message: 'No engine for the project could be reached.' });
    expect(copy.getSnapshot()).toMatchObject({ session: 1, takingOver: false, error: 'No engine for the project could be reached.' });
    engine.tell({ kind: 'taking-over', session: 3, pid: 31 });
    engine.opened(3);
    await settle();
    expect(copy.getSnapshot()).toMatchObject({ session: 3, status: 'connected', takingOver: false });
    // And no longer what the sessions before it tell.
    engine.tell({ kind: 'error', session: 2, message: 'late' });
    expect(copy.getSnapshot().error).toBeNull();
    engine.tell({ kind: 'measure', session: null });
    expect(copy.getSnapshot()).toMatchObject({ session: 3, status: 'connected' });
  });

  it('says why the project could not be fetched', async () => {
    const engine = engineOf({ revision: 0, project: empty() });
    engine.call.mockResolvedValueOnce({ ok: false, error: { code: null, message: 'No project is open in the studio.', data: null } });
    const copy = new ProjectCopy(engine.bridge);
    await copy.refetch();
    expect(copy.getSnapshot()).toMatchObject({ status: 'disconnected', error: 'No project is open in the studio.' });
  });
});

describe('calling the engine from the page', () => {
  it('gives the result, or an error with the code and data that the engine gave', async () => {
    const call = vi.fn(async (): Promise<EngineAnswer> => ({ ok: true, result: { revision: 3, done: [], next_id: 4, names: { $c: 'c3' } } }));
    await expect(callEngine({ call }, 'transaction.apply', { ops: [] })).resolves.toEqual({ revision: 3, done: [], next_id: 4, names: { $c: 'c3' } });
    expect(call).toHaveBeenCalledWith('transaction.apply', { ops: [] });
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'overlap', data: { reason: 'overlap', with: 'c1' } } });
    const refused = await callEngine({ call }, 'transaction.apply', { ops: [] }).catch(error => error);
    expect(refused).toBeInstanceOf(EngineError);
    expect(refused).toMatchObject({ message: 'overlap', code: -32001, data: { reason: 'overlap', with: 'c1' } });
  });

  it('measures the viewer in device pixels, so that neighbours share their edges', () => {
    expect(deviceRect({ left: 10.3, top: 20.5, right: 330.3, bottom: 200.7 }, 1.5)).toEqual({ x: 15, y: 31, width: 480, height: 270 });
    expect(deviceRect({ left: 0, top: 0, right: 100, bottom: 50 }, 1)).toEqual({ x: 0, y: 0, width: 100, height: 50 });
    // A viewer that lies partly outside the window is measured from its corner.
    expect(deviceRect({ left: -4, top: -4, right: 10, bottom: 10 }, 1)).toEqual({ x: 0, y: 0, width: 10, height: 10 });
  });
});
