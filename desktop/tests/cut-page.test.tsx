// @vitest-environment jsdom
/**
 * The Cut page: the pool in its thumbnail view, one viewer that shows the source clip, the source
 * tape or the timeline, the buttons of its edits, and the two timelines of one timeline with one
 * playhead. Edges of clips are trimmed with ripple there, and seams roll.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useOverlay } from '../src/app/overlay';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import { Studio } from '../src/studio/Studio';
import { CUT_NARROW_PX } from '../src/studio/pages/CutPage';
import { nearestCut, nearestEditPoint } from '../src/studio/timeline/editKeys';
import { fitZoom, HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { placeOnTape, tapeIndexAt, tapeMedia, tapeOf, tapeStep } from '../src/studio/cut/SourceTape';
import { applyOp, ProjectCopy, type EngineEvent, type Media, type Op, type Project, type Timeline, type TransportState } from '../src/studio/engine';
import type { EngineBridge } from '../src/bridge';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

let width: number;
let frames: FrameRequestCallback[];
let observers: ResizeObserverCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
/** Where the row of a track begins: under the ruler, 72 pixels for each row above it. */
const row = (track: HTMLElement) => 30 + 72 * [...track.parentElement!.querySelectorAll(':scope > [data-track]')].indexOf(track);
beforeEach(() => {
  likeABrowser(); width = 1500; frames = []; observers = [];
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.stubGlobal('ResizeObserver', class { constructor(callback: ResizeObserverCallback) { observers.push(callback); } observe() {} disconnect() {} });
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('cut-page')) return rect(0, 0, width, 800);
    if (this.classList.contains('source-tape')) return rect(0, 300, 1000, 30);
    if (this.dataset.clip) {
      // The style sheet places a clip by its frames and the pixels of a frame of its timeline, less what it is scrolled.
      const timeline = this.closest<HTMLElement>('.timeline')!;
      const scale = Number(timeline.dataset.scale);
      const scrolled = timeline.querySelector<HTMLElement>('.timeline-scroll')?.scrollLeft ?? 0;
      return rect(HEADER_WIDTH + Number(this.style.getPropertyValue('--start')) * scale - scrolled, row(this.closest<HTMLElement>('[data-track]')!) + 4, Number(this.style.getPropertyValue('--length')) * scale, 64);
    }
    if (this.dataset.track) return rect(0, row(this), 956, 72);
    return rect(0, 0, 956, 400);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

// Two media in the master bin, ten and twenty seconds long at their own rates, the second marked; a third in another bin.
const m1: Media = { ...media, id: 'm1', name: 'Interview', facts: { ...media.facts, duration: { ticks: 250, scale: 25 } } };
const m2: Media = {
  ...media, id: 'm2', name: 'Street', mark_in: { ticks: 5000, scale: 1000 }, mark_out: { ticks: 15000, scale: 1000 },
  facts: { ...media.facts, duration: { ticks: 20000, scale: 1000 }, video: { ...media.facts.video!, rate: { num: 30000, den: 1001 } } },
};
const m3: Media = { ...m1, id: 'm3', name: 'Elsewhere', bin: 'b1' };
const cuts = (): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [clip('c1', 0, 100), clip('c2', 100, 100), clip('c3', 200, 100)], transitions: [] },
    { id: 'a1', kind: 'audio', name: 'A1', enabled: true, locked: false, clips: [clip('ca', 0, 150)], transitions: [] },
    { id: 'a2', kind: 'audio', name: 'A2', enabled: true, locked: true, clips: [clip('cl', 0, 130)], transitions: [] },
  ],
});

/**
 * Where a transaction's moves put the clips, and whether a `close_gap` after them is refused, as the
 * engine refuses it: with `ripple_blocked` where a clip of another track that is not locked lies
 * across the gap (unless `straddling` is 'stay'), or, with 'stay', where a clip after the gap would
 * run into one that stays. Other edits are not followed.
 */
function rippleBlocked(project: Project, ops: readonly Record<string, unknown>[]): { track: string; clip: string } | null {
  type Lane = { id: string; kind: string; locked: boolean; clips: { id: string; start: number; end: number }[] };
  const lanes = new Map<string, Lane[]>(project.timelines.map(timeline => [timeline.id, timeline.tracks.map(track => ({
    id: track.id, kind: track.kind, locked: track.locked,
    clips: track.kind === 'subtitle' ? [] : track.clips.map(each => ({ id: each.id, start: each.start, end: each.start + each.length })),
  }))]));
  for (const op of ops) {
    const tracks = lanes.get(op.timeline as string);
    if (!tracks) continue;
    if (op.edit === 'move') {
      // Those of the kind of `to_track` go to tracks as far from theirs as it is from the track of the first clip.
      const ids = op.clips as string[];
      const laneOf = (id: string) => tracks.find(track => track.clips.some(each => each.id === id))!;
      const destination = tracks.find(track => track.id === op.to_track);
      const peers = (kind: string) => tracks.filter(track => track.kind === kind);
      const first = laneOf(ids[0]);
      const offset = destination ? peers(destination.kind).indexOf(destination) - peers(destination.kind).indexOf(first) : 0;
      const moved = ids.map(id => {
        const from = laneOf(id);
        const clip = from.clips.find(each => each.id === id)!;
        from.clips = from.clips.filter(each => each !== clip);
        const to = destination?.kind === from.kind ? peers(from.kind)[peers(from.kind).indexOf(from) + offset] : from;
        return { clip: { ...clip, start: clip.start + (op.delta as number), end: clip.end + (op.delta as number) }, to };
      });
      for (const { clip, to } of moved) {
        // Overwritten: what it covers is taken away or cut back.
        to.clips = to.clips.flatMap(each => each.end <= clip.start || each.start >= clip.end ? [each]
          : each.start < clip.start ? [{ ...each, end: clip.start }] : each.end > clip.end ? [{ ...each, start: clip.end }] : []);
        to.clips.push(clip);
      }
    }
    if (op.edit === 'close_gap') {
      const lane = tracks.find(track => track.id === op.track)!;
      const at = op.at as number;
      const from = Math.max(0, ...lane.clips.map(each => each.end).filter(end => end <= at));
      const to = Math.min(...lane.clips.map(each => each.start).filter(start => start > at));
      for (const track of tracks.filter(each => !each.locked)) {
        const across = track.clips.filter(each => each.start < to && from < each.end).sort((a, b) => b.end - a.end)[0];
        if (!across) continue;
        const first = Math.min(...track.clips.filter(each => each.start >= to).map(each => each.start));
        if (op.straddling !== 'stay' || first - (to - from) < across.end) return { track: track.id, clip: across.id };
      }
      for (const track of tracks.filter(each => !each.locked)) for (const each of track.clips) if (each.start >= to) { each.start -= to - from; each.end -= to - from; }
    }
  }
  return null;
}

/** An engine that keeps a project and the two transports, and tells what changed as the real one does. */
function fakeEngine(timelines: Timeline[] = [cuts()], pool: { media: Media[]; bins: Project['bins'] } = { media: [m1, m2, m3], bins: [{ id: 'b1', name: 'B-roll', parent: null }] }) {
  let project: Project = { schema: 2, next_id: 10, bins: pool.bins, media: pool.media, timelines };
  let revision = 0;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  const states: Record<'source' | 'timeline', TransportState> = {
    source: { monitor: 'source', media: 'm1', frame: 10, shown: 10, last: 249, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: 'original' },
    timeline: { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 },
  };
  const bridge = {
    open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>) => {
      if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
      if (method === 'media.thumbnail') return new Promise(() => {});
      const monitor = params?.monitor as 'source' | 'timeline';
      if (method === 'transport.state') return { ok: true, result: states[monitor] };
      if (method === 'transport.load') {
        const state = (monitor === 'timeline'
          ? { ...states.timeline, timeline: params!.timeline as string, frame: 0, shown: 0, last: 299 }
          : { ...states.source, media: params!.media as string, frame: 0, shown: 0 }) as TransportState;
        states[monitor] = state; tell('transport.state', state); return { ok: true, result: state };
      }
      if (method === 'transport.seek') {
        states[monitor] = { ...states[monitor], frame: params!.frame as number, shown: params!.frame as number } as TransportState;
        tell('transport.state', states[monitor]); return { ok: true, result: states[monitor] };
      }
      if (method === 'transaction.apply') {
        // Editing commands are expanded by the engine alone; here they change nothing but the revision,
        // but a gap closed through a clip of another track is refused, as the engine refuses it.
        const blocked = rippleBlocked(project, params!.ops as Record<string, unknown>[]);
        if (blocked) return { ok: false, error: { code: -32001, message: 'refused: ripple_blocked', data: { reason: 'ripple_blocked', ...blocked } } };
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
  } as unknown as EngineBridge & { call: ReturnType<typeof vi.fn>; viewShow: ReturnType<typeof vi.fn> };
  const copy = new ProjectCopy(bridge);
  return {
    bridge, copy,
    notify: (monitor: 'source' | 'timeline', fields: Partial<TransportState>) => act(() => { states[monitor] = { ...states[monitor], ...fields } as TransportState; tell('transport.state', states[monitor]); }),
  };
}
function Lying() { useOverlay(); return null; }
async function mount({ words, timelines, pool }: { words?: typeof zhCN; timelines?: Timeline[]; pool?: { media: Media[]; bins: Project['bins'] } } = {}) {
  const engine = fakeEngine(timelines, pool);
  const tree = (lying: boolean) => <>{<Studio cwd="F:/film" engine={{ ...engine, native: true }} language="en" onClose={() => {}} />}{lying && <Lying />}</>;
  const view = render(words ? <Words.Provider value={words}>{tree(false)}</Words.Provider> : tree(false));
  await settle(); fireEvent.click(screen.getByRole('tab', { name: words ? '快编' : 'Cut' })); await settle(); draw(); await settle();
  const overlay = (lying: boolean) => view.rerender(words ? <Words.Provider value={words}>{tree(lying)}</Words.Provider> : tree(lying));
  return { ...engine, ...view, overlay };
}
const calls = (bridge: { call: ReturnType<typeof vi.fn> }, method: string) => bridge.call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
const sent = (bridge: { call: ReturnType<typeof vi.fn> }) => calls(bridge, 'transaction.apply') as { revision: number; ops: unknown[] }[];
const key = (value: string, fields = {}) => fireEvent.keyDown(document.body, { key: value, ...fields });
const page = () => document.querySelector<HTMLElement>('[data-test="cut-page"]')!;
const overview = () => screen.getByRole('region', { name: 'Whole timeline' });
const detail = () => screen.getByRole('region', { name: 'Timeline around the playhead' });
const shownMonitors = () => [...document.querySelectorAll<HTMLElement>('.cut-viewer > .monitor')].filter(each => !each.hidden).map(each => each.dataset.monitor);
const playhead = (region: HTMLElement) => region.querySelector<HTMLElement>('.timeline-playhead')!.style.left;
const lastShow = (bridge: { viewShow: ReturnType<typeof vi.fn> }, monitor: string) => bridge.viewShow.mock.calls.filter(([, name]) => name === monitor).at(-1)?.[0];

describe('the Cut page', () => {
  it('has the pool in its thumbnail view, one viewer, the edit buttons beneath it and two timelines', async () => {
    const { bridge } = await mount();
    const pool = screen.getByRole('region', { name: 'Media pool' });
    expect(within(pool).getByRole('button', { name: 'Thumbnail view' }).getAttribute('aria-pressed')).toBe('true');
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'timeline', timeline: 't1' }]);
    // The viewer shows the timeline to begin with; the source monitor is there, hidden, with its view.
    expect(page().dataset.mode).toBe('timeline');
    expect(shownMonitors()).toEqual(['timeline']);
    expect(lastShow(bridge, 'timeline')).toBe(true);
    expect(lastShow(bridge, 'source')).toBe(false);
    const edits = screen.getByRole('toolbar', { name: 'Edits' });
    expect(within(edits).getAllByRole('button').map(each => each.getAttribute('aria-label')))
      .toEqual(['Smart insert', 'Append', 'Ripple overwrite', 'Place on top', 'Source overwrite']);
    expect(within(screen.getByRole('toolbar', { name: 'Transitions' })).getAllByRole('button').map(each => each.getAttribute('aria-label')))
      .toEqual(['Dissolve', 'Dip to black']);
    expect(overview().dataset.timeline).toBe('t1');
    expect(detail().dataset.timeline).toBe('t1');
  });

  it('switches its one viewer between the source clip, the source tape and the timeline, the keys going to the one shown', async () => {
    const { bridge } = await mount();
    const mode = (name: string) => fireEvent.click(screen.getByRole('button', { name }));
    mode('Source clip'); await settle(); draw(); await settle();
    expect(page().dataset.mode).toBe('source');
    expect(shownMonitors()).toEqual(['source']);
    expect(lastShow(bridge, 'source')).toBe(true);
    expect(lastShow(bridge, 'timeline')).toBe(false);
    expect(document.querySelector('[data-test="source-tape"]')).toBeNull();
    bridge.call.mockClear();
    key('l');
    expect(calls(bridge, 'transport.play')).toEqual([{ speed: 1, monitor: 'source' }]);
    mode('Source tape'); await settle();
    expect(page().dataset.mode).toBe('tape');
    expect(shownMonitors()).toEqual(['source']);
    expect(document.querySelector('[data-test="source-tape"]')).not.toBeNull();
    // The medium that the viewer has is on the tape: it is not loaded again.
    expect(calls(bridge, 'transport.load')).toEqual([]);
    mode('Timeline'); await settle();
    expect(page().dataset.mode).toBe('timeline');
    expect(shownMonitors()).toEqual(['timeline']);
    bridge.call.mockClear();
    key('l');
    expect(calls(bridge, 'transport.play')).toEqual([{ speed: 1, monitor: 'timeline' }]);
    // A press on a timeline shows the timeline; going back to the source finds the tape as it was left.
    mode('Source tape'); await settle();
    fireEvent.pointerDown(detail().querySelector('.timeline-lane')!, { button: 0, clientX: 300, clientY: 40 });
    expect(page().dataset.mode).toBe('timeline');
    fireEvent.click(screen.getByRole('button', { name: 'Source tape' }));
    expect(page().dataset.mode).toBe('tape');
  });

  it('plays the media of the bin as one tape, each medium with its own frames and marks, across their boundaries', async () => {
    const { bridge } = await mount({ pool: { media: [m1, m2], bins: [] } });
    fireEvent.click(screen.getByRole('button', { name: 'Source tape' })); await settle();
    const tape = document.querySelector<HTMLElement>('[data-test="source-tape"]')!;
    // The master bin's two media, in their order.
    expect([...tape.querySelectorAll<HTMLElement>('[data-tape-media]')].map(each => [each.dataset.tapeMedia, each.style.left, each.style.width]))
      .toEqual([['m1', '0%', `${10 / 30 * 100}%`], ['m2', `${10 / 30 * 100}%`, `${20 / 30 * 100}%`]]);
    // The marks of the second are drawn in it, at a quarter and three quarters of it; the first has none.
    expect([...tape.querySelectorAll<HTMLElement>('[data-tape-mark]')].map(each => [each.dataset.media, each.dataset.tapeMark, each.style.left]))
      .toEqual([['m2', 'in', '25%'], ['m2', 'out', '75%']]);
    bridge.call.mockClear();
    // Halfway along the tape is 15 seconds in: five seconds into the second medium, at its own 29.97 fps.
    fireEvent.pointerDown(tape, { button: 0, clientX: 500 });
    fireEvent.pointerUp(tape, { button: 0, clientX: 500 });
    await settle();
    expect(bridge.call.mock.calls.map(([method, params]) => [method, params])).toEqual([
      ['transport.load', { monitor: 'source', media: 'm2' }],
      ['transport.seek', { monitor: 'source', frame: 149, scrubbing: true }],
      ['transport.seek', { monitor: 'source', frame: 149, scrubbing: false }],
    ]);
    expect(tape.querySelector('[data-tape-media="m2"]')!.hasAttribute('data-current')).toBe(true);
    expect(document.querySelector('[data-test="tape-head"]')).not.toBeNull();
    // A mark is set on the medium that the tape is in, at its own frame.
    key('i'); await settle();
    expect(sent(bridge).at(-1)).toEqual({ revision: 0, ops: [{ op: 'set_media', id: 'm2', mark_in: { ticks: 4971, scale: 1000 } }] });
    // Either side of the boundary is a frame of one medium: the last of the first, the first of the second.
    bridge.call.mockClear();
    fireEvent.pointerDown(tape, { button: 0, clientX: 333 }); fireEvent.pointerUp(tape, { button: 0, clientX: 333 }); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'source', media: 'm1' }]);
    expect(calls(bridge, 'transport.seek').at(-1)).toEqual({ monitor: 'source', frame: 249, scrubbing: false });
    bridge.call.mockClear();
    fireEvent.pointerDown(tape, { button: 0, clientX: 334 }); fireEvent.pointerUp(tape, { button: 0, clientX: 334 }); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'source', media: 'm2' }]);
    expect(calls(bridge, 'transport.seek')).toEqual([{ monitor: 'source', frame: 0, scrubbing: false }]);
    // The arrows go from medium to medium; past the last there is none.
    bridge.call.mockClear();
    key('ArrowDown'); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([]);
    key('ArrowUp'); await settle();
    expect(calls(bridge, 'transport.load')).toEqual([{ monitor: 'source', media: 'm1' }]);
    expect(calls(bridge, 'transaction.apply')).toEqual([]);
  });

  it('strings out the media of the bin and of the bins in it, and chooses in the pool the medium under the playhead of the tape', async () => {
    // Reference Manual 20, p. 546: every clip of the open bin and of its subfolders is on the tape, and the one the playhead is in is selected in the Media Pool.
    const m4: Media = { ...m1, id: 'm4', name: 'Inner', bin: 'b2' };
    const bins = [{ id: 'b1', name: 'B-roll', parent: null }, { id: 'b2', name: 'Inner', parent: 'b1' }];
    expect(tapeMedia([m1, m4, m3, m2], bins, 'b1').map(each => each.id)).toEqual(['m3', 'm4']);
    expect(tapeMedia([m1, m4, m3, m2], bins, null).map(each => each.id)).toEqual(['m1', 'm2', 'm3', 'm4']);
    expect(tapeMedia([m1, m4, m3, m2], bins, 'b2').map(each => each.id)).toEqual(['m4']);
    const { notify } = await mount({ pool: { media: [m1, m2, m3, m4], bins } });
    fireEvent.click(screen.getByRole('button', { name: 'Source tape' })); await settle();
    // The master bin's tape is all the media.
    expect([...document.querySelectorAll<HTMLElement>('[data-tape-media]')].map(each => each.dataset.tapeMedia)).toEqual(['m1', 'm2', 'm3', 'm4']);
    const chosen = () => [...document.querySelectorAll<HTMLElement>('[data-test="pool-item"][aria-selected="true"]')].map(each => each.dataset.media);
    notify('source', { media: 'm2', frame: 3 }); await settle();
    expect(chosen()).toEqual(['m2']);
    // A bin's tape holds the media of the bins in it.
    fireEvent.click(document.querySelector('[data-bin="b1"]')!); await settle();
    expect([...document.querySelectorAll<HTMLElement>('[data-tape-media]')].map(each => each.dataset.tapeMedia)).toEqual(['m3', 'm4']);
    notify('source', { media: 'm3', frame: 0 }); await settle();
    expect(chosen()).toEqual(['m3']);
    // Off the tape, the pool is chosen in by hand alone.
    fireEvent.click(screen.getByRole('button', { name: 'Source clip' })); await settle();
    notify('source', { media: 'm4', frame: 0 }); await settle();
    expect(chosen()).toEqual(['m3']);
  });

  it('works out the tape as pure functions: frames of one medium, never a range across two', () => {
    const tape = tapeOf([m1, m2]);
    expect(tape.segments.map(each => [each.media.id, each.offset, each.frames, each.start, each.seconds])).toEqual([['m1', 0, 250, 0, 10], ['m2', 250, 599, 10, 20]]);
    expect(placeOnTape(tape, 249)).toEqual({ media: 'm1', frame: 249 });
    expect(placeOnTape(tape, 250)).toEqual({ media: 'm2', frame: 0 });
    expect(placeOnTape(tape, 10_000)).toEqual({ media: 'm2', frame: 598 });
    expect(tapeIndexAt(tape, 30)).toBe(250 + 598);
    expect(tapeStep(tape, 'm1', 1)).toBe('m2');
    expect(tapeStep(tape, 'm2', 1)).toBeNull();
    expect(tapeStep(tape, 'm3', -1)).toBe('m1');
    expect(placeOnTape(tapeOf([]), 0)).toBeNull();
  });

  it('puts the source on the timeline with each button, one transaction each, and keeps the source tape where it was', async () => {
    const { bridge, notify } = await mount();
    fireEvent.click(screen.getByRole('button', { name: 'Source tape' })); await settle();
    const tape = document.querySelector<HTMLElement>('[data-test="source-tape"]')!;
    fireEvent.pointerDown(tape, { button: 0, clientX: 500 }); fireEvent.pointerUp(tape, { button: 0, clientX: 500 }); await settle();
    notify('timeline', { frame: 140 }); await settle();
    // The edit point nearest 140 among the clips of the tracks not locked: 150, the end of the clip on A1.
    const indicator = detail().querySelector<HTMLElement>('[data-test="smart-indicator"]')!;
    expect(indicator.dataset.frame).toBe('150');
    const press = async (name: string) => { bridge.call.mockClear(); fireEvent.click(screen.getByRole('button', { name })); await settle(); return sent(bridge); };
    const whole = { kind: 'media', media: 'm2' };
    const targets = { video_track: 'v1', audio_track: 'a1' };
    expect(await press('Smart insert')).toEqual([{ revision: 0, ops: [{ edit: 'smart_insert', timeline: 't1', source: whole, at: 150, ...targets }] }]);
    // Of two edit points as near, the earlier: 125 is 25 frames from 100 and from 150 (the end at 130 is on a locked track).
    notify('timeline', { frame: 125 }); await settle();
    expect(detail().querySelector<HTMLElement>('[data-test="smart-indicator"]')!.dataset.frame).toBe('100');
    expect(await press('Smart insert')).toEqual([{ revision: 1, ops: [{ edit: 'smart_insert', timeline: 't1', source: whole, at: 100, ...targets }] }]);
    expect(await press('Append')).toEqual([{ revision: 2, ops: [{ edit: 'append', timeline: 't1', source: whole, ...targets }] }]);
    expect(await press('Ripple overwrite')).toEqual([{ revision: 3, ops: [{ edit: 'ripple_overwrite', timeline: 't1', source: whole, at: 125, ...targets }] }]);
    expect(await press('Place on top')).toEqual([{ revision: 4, ops: [{ edit: 'place_on_top', timeline: 't1', source: whole, at: 125 }] }]);
    // Source Overwrite syncs by timecode, which the engine does not read: as DaVinci Resolve without
    // overlapping timecode, it does nothing (Reference Manual 20, p. 563), so it is disabled and says why.
    const sourceOverwrite = screen.getByRole('button', { name: 'Source overwrite' }) as HTMLButtonElement;
    expect(sourceOverwrite.disabled).toBe(true);
    expect(document.querySelector<HTMLElement>('[data-test="cut-sourceOverwrite-why"]')!.title).toMatch(/does not read the timecode.*does nothing/);
    expect(await press('Source overwrite')).toEqual([]);
    // The viewer still shows the tape, in the same medium.
    expect(page().dataset.mode).toBe('tape');
    expect(calls(bridge, 'transport.load')).toEqual([]);
    expect(tape.querySelector('[data-tape-media="m2"]')!.hasAttribute('data-current')).toBe(true);
    // The keys of the Cut page do the same: F9 Smart Insert, F10 Overwrite (Reference Manual 20, p. 565), Shift+F10 Ripple Overwrite, F12, Shift+F12.
    bridge.call.mockClear();
    for (const [value, fields] of [['F9', {}], ['F10', {}], ['F10', { shiftKey: true }], ['F12', {}], ['F12', { shiftKey: true }]] as const) { key(value, fields); await settle(); }
    expect(sent(bridge).map(each => (each.ops[0] as { edit: string }).edit)).toEqual(['smart_insert', 'overwrite', 'ripple_overwrite', 'place_on_top', 'append']);
    expect(sent(bridge)[1].ops[0]).toEqual({ edit: 'overwrite', timeline: 't1', source: whole, at: 125, ...targets });
  });

  it('finds the edit point that Smart Insert goes in at as the engine does', () => {
    const t = cuts();
    expect([140, 125, 128, 0, 299, 400].map(frame => nearestCut(t, frame))).toEqual([150, 100, 150, 0, 300, 300]);
    expect(nearestCut({ ...t, tracks: [] }, 80)).toBe(0);
  });

  it('shows one timeline above and the part of it around the playhead beneath, with one playhead', async () => {
    const { bridge, notify } = await mount();
    const fit = fitZoom(300, 800);
    expect(Number(overview().dataset.scale)).toBeCloseTo(fit);
    expect(Number(detail().dataset.scale)).toBe(4);
    notify('timeline', { frame: 150 }); await settle();
    expect(playhead(overview())).toBe(`${HEADER_WIDTH + 150 * fit}px`);
    expect(playhead(detail())).toBe(`${HEADER_WIDTH + 150 * 4}px`);
    // The lower timeline keeps the playhead in its middle; the upper one marks the part that it shows.
    expect(detail().querySelector<HTMLElement>('.timeline-scroll')!.scrollLeft).toBe(150 * 4 - 400);
    const box = () => overview().querySelector<HTMLElement>('[data-test="detail-window"]')!;
    expect([box().dataset.from, box().dataset.to]).toEqual(['50', '250']);
    // A zoom of the lower one changes the part marked above, and not the upper one.
    fireEvent.keyDown(detail(), { key: '=', ctrlKey: true }); await settle();
    expect(Number(detail().dataset.scale)).toBe(5);
    expect(Number(overview().dataset.scale)).toBeCloseTo(fit);
    expect([box().dataset.from, box().dataset.to]).toEqual(['70', '230']);
    // The upper one does not zoom.
    fireEvent.keyDown(overview(), { key: '=', ctrlKey: true }); await settle();
    expect(Number(overview().dataset.scale)).toBeCloseTo(fit);
    expect(overview().querySelector('[data-test="smart-indicator"]')).toBeNull();
    // A press on a track of the upper one moves the playhead there, which both follow.
    bridge.call.mockClear();
    const lane = overview().querySelector<HTMLElement>('[data-track="v1"] .timeline-lane')!;
    fireEvent.pointerDown(lane, { button: 0, clientX: HEADER_WIDTH + 200 * fit, clientY: 40 });
    fireEvent.pointerUp(overview(), { button: 0, clientX: HEADER_WIDTH + 200 * fit, clientY: 40 });
    await settle();
    expect(calls(bridge, 'transport.seek')).toEqual([{ frame: 200, scrubbing: true, monitor: 'timeline' }, { frame: 200, scrubbing: false, monitor: 'timeline' }]);
    expect(playhead(detail())).toBe(`${HEADER_WIDTH + 200 * 5}px`);
    expect(calls(bridge, 'transaction.apply')).toEqual([]);
  });

  it('trims an edge of Track 1 with ripple and rolls a seam, in either timeline, and moves clips in the upper one, closing the gap they leave', async () => {
    const { bridge } = await mount();
    const at = (region: HTMLElement, frame: number) => HEADER_WIDTH + frame * Number(region.dataset.scale);
    const drag = async (region: HTMLElement, id: string, from: number, to: number) => {
      bridge.call.mockClear();
      fireEvent.pointerDown(region.querySelector(`[data-clip="${id}"]`)!, { button: 0, clientX: from, clientY: 45, pointerId: 1 });
      fireEvent.pointerMove(region, { clientX: to, clientY: 45, pointerId: 1 });
      fireEvent.pointerUp(region, { clientX: to, clientY: 45, pointerId: 1 });
      await settle();
      return sent(bridge).map(each => each.ops);
    };
    const lower = detail();
    // Four pixels in from the end of c1: its edge, trimmed with ripple as the Cut page does on Track 1.
    expect(await drag(lower, 'c1', at(lower, 100) - 4, at(lower, 100) - 4 + 40))
      .toEqual([[{ edit: 'trim', timeline: 't1', delta: 10, linked: true, clip: 'c1', edge: 'out', ripple: true }]]);
    expect(await drag(lower, 'c3', at(lower, 300) - 4, at(lower, 300) - 4 - 20))
      .toEqual([[{ edit: 'trim', timeline: 't1', delta: -5, linked: true, clip: 'c3', edge: 'out', ripple: true }]]);
    // On the seam between c1 and c2: a roll.
    expect(await drag(lower, 'c1', at(lower, 100) - 1, at(lower, 100) - 1 + 40))
      .toEqual([[{ edit: 'roll', timeline: 't1', delta: 10, linked: true, left: 'c1', right: 'c2' }]]);
    const upper = overview();
    const scale = Number(upper.dataset.scale);
    expect(await drag(upper, 'c3', at(upper, 300) - 4, at(upper, 300) - 4 - 10 * scale))
      .toEqual([[{ edit: 'trim', timeline: 't1', delta: -10, linked: true, clip: 'c3', edge: 'out', ripple: true }]]);
    // Moved on Track 1, c2 overwrites the start of c3, and the gap that it leaves at 100 is closed (Reference Manual 20, pp. 585–586).
    expect(await drag(upper, 'c2', at(upper, 150), at(upper, 150) + 10 * scale))
      .toEqual([[{ edit: 'move', timeline: 't1', delta: 10, linked: true, clips: ['c2'], overwrite: true }, { edit: 'close_gap', timeline: 't1', track: 'v1', at: 100, straddling: 'stay' }]]);
  });

  it('keeps Track 1 magnetic: a clip dropped on an edit point goes in there, one deleted leaves no gap, and the tracks above only resize', async () => {
    const withAbove = cuts();
    withAbove.tracks.splice(1, 0, { id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, clips: [clip('b1', 20, 40), clip('b2', 60, 40)], transitions: [] });
    const { bridge } = await mount({ timelines: [withAbove] });
    const lower = detail();
    const at = (frame: number) => HEADER_WIDTH + frame * Number(lower.dataset.scale);
    // The pointer on the row of the clip pressed: V2 is drawn above V1.
    const y = (id: string) => lower.querySelector<HTMLElement>(`[data-clip="${id}"]`)!.getBoundingClientRect().top + 10;
    const drag = async (id: string, from: number, to: number) => {
      bridge.call.mockClear();
      const clientY = y(id);
      fireEvent.pointerDown(lower.querySelector(`[data-clip="${id}"]`)!, { button: 0, clientX: from, clientY, pointerId: 1 });
      fireEvent.pointerMove(lower, { clientX: (from + to) / 2, clientY, pointerId: 1 });
      fireEvent.pointerMove(lower, { clientX: to, clientY, pointerId: 1 });
      const swap = lower.querySelector<HTMLElement>('.timeline-snap');
      fireEvent.pointerUp(lower, { clientX: to, clientY, pointerId: 1 });
      await settle();
      return { ops: sent(bridge).map(each => each.ops), marked: swap?.style.left ?? null };
    };
    // c1 dropped with the pointer on the edit point between c2 and c3: put in there, the rest moving aside (p. 586, "Swapping Clips").
    const swapped = await drag('c1', at(50), at(201));
    expect(swapped.ops).toEqual([[{ edit: 'insert_move', timeline: 't1', clips: ['c1'], at: 200, linked: true }]]);
    expect(swapped.marked).toBe(`${at(200)}px`);
    // An edge of a clip above Track 1 only resizes that clip, stopping at the one beside it (pp. 509, 590).
    expect((await drag('b1', at(60) - 4, at(60) - 4 + 40)).ops).toEqual([]);
    expect((await drag('b2', at(100) - 4, at(100) - 4 + 40)).ops).toEqual([[{ edit: 'trim', timeline: 't1', delta: 10, linked: true, clip: 'b2', edge: 'out', ripple: false }]]);
    // A clip above Track 1, moved, leaves its gap.
    expect((await drag('b2', at(80), at(80) + 40)).ops).toEqual([[{ edit: 'move', timeline: 't1', delta: 10, linked: true, clips: ['b2'], overwrite: true }]]);
    // Deleted with Backspace, a clip of Track 1 ripples (p. 588); one above it is lifted.
    const choose = (id: string, frame: number) => { fireEvent.pointerDown(lower.querySelector(`[data-clip="${id}"]`)!, { button: 0, clientX: at(frame), clientY: y(id), pointerId: 1 }); fireEvent.pointerUp(lower, { clientX: at(frame), clientY: y(id), pointerId: 1 }); };
    bridge.call.mockClear();
    choose('c2', 150); key('Backspace'); await settle();
    choose('b1', 40); key('Backspace'); await settle();
    choose('c2', 150); fireEvent.pointerDown(lower.querySelector('[data-clip="b1"]')!, { button: 0, clientX: at(40), clientY: y('b1'), pointerId: 1, ctrlKey: true });
    fireEvent.pointerUp(lower, { clientX: at(40), clientY: y('b1'), pointerId: 1, ctrlKey: true });
    key('Backspace'); await settle();
    expect(sent(bridge).map(each => each.ops)).toEqual([
      [{ edit: 'ripple_delete', timeline: 't1', clips: ['c2'], linked: true }],
      [{ edit: 'lift', timeline: 't1', clips: ['b1'], linked: true }],
      [{ edit: 'lift', timeline: 't1', clips: ['b1'], linked: false }, { edit: 'ripple_delete', timeline: 't1', clips: ['c2'], linked: true }],
    ]);
  });

  it('closes up Track 1 alone: a bed of music across the gap and a clip moved straight up off it stay where they are', async () => {
    const withBed = cuts();
    withBed.tracks.splice(1, 0, { id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, clips: [clip('b1', 20, 40)], transitions: [] });
    // The bed of music on A2 is not locked: it lies across every gap that Track 1 leaves before 130, as 'ca' on A1 does before 150.
    withBed.tracks.find(track => track.id === 'a2')!.locked = false;
    const { bridge } = await mount({ timelines: [withBed] });
    const lower = detail();
    const at = (frame: number) => HEADER_WIDTH + frame * Number(lower.dataset.scale);
    const y = (id: string) => lower.querySelector<HTMLElement>(`[data-clip="${id}"]`)!.getBoundingClientRect().top + 10;
    const answered = async () => {
      const indices = bridge.call.mock.calls.map(([method], index) => method === 'transaction.apply' ? index : -1).filter(index => index >= 0);
      return indices.length === 0 ? null : await bridge.call.mock.results[indices.at(-1)!].value;
    };
    const drag = async (id: string, from: number, to: number, fromY: number, toY: number) => {
      bridge.call.mockClear();
      fireEvent.pointerDown(lower.querySelector(`[data-clip="${id}"]`)!, { button: 0, clientX: from, clientY: fromY, pointerId: 1 });
      fireEvent.pointerMove(lower, { clientX: (from + to) / 2, clientY: (fromY + toY) / 2, pointerId: 1 });
      fireEvent.pointerMove(lower, { clientX: to, clientY: toY, pointerId: 1 });
      fireEvent.pointerUp(lower, { clientX: to, clientY: toY, pointerId: 1 });
      await settle();
      return sent(bridge).map(each => each.ops);
    };
    // The engine refuses to close a gap that the bed lies across, unless what lies across it stays.
    expect(await bridge.call('transaction.apply', { revision: 0, ops: [
      { edit: 'move', timeline: 't1', delta: 10, linked: true, clips: ['c2'], overwrite: true },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 100 },
    ] })).toMatchObject({ ok: false, error: { data: { reason: 'ripple_blocked', track: 'a1', clip: 'ca' } } });
    // c2 moved on Track 1 over the start of c3: the gap at 100 is closed, the bed and 'ca' staying.
    expect(await drag('c2', at(150), at(150) + 40, y('c2'), y('c2'))).toEqual([[
      { edit: 'move', timeline: 't1', delta: 10, linked: true, clips: ['c2'], overwrite: true },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 100, straddling: 'stay' },
    ]]);
    expect(await answered()).toMatchObject({ ok: true });
    // c2 moved straight up to V2: it stays above the gap that it leaves, which is closed under it.
    expect(await drag('c2', at(150), at(150), y('c2'), y('b1'))).toEqual([[
      { edit: 'move', timeline: 't1', delta: 0, linked: true, clips: ['c2'], to_track: 'v2', overwrite: true },
      { edit: 'close_gap', timeline: 't1', track: 'v1', at: 100, straddling: 'stay' },
    ]]);
    expect(await answered()).toMatchObject({ ok: true });
    expect(document.querySelector('[data-test="studio-problem"]')).toBeNull();
  });

  it('puts a dissolve or a dip to black at the edit point nearest the playhead, the earlier of two as near', async () => {
    const withTransition = cuts();
    (withTransition.tracks[0] as Extract<Timeline['tracks'][number], { kind: 'video' | 'audio' }>).transitions = [{ id: 'x1', kind: 'dissolve', left: 'c2', right: 'c3', length: 25, align: 'center' }];
    const { bridge, notify } = await mount({ timelines: [withTransition] });
    const press = async (name: string) => { bridge.call.mockClear(); fireEvent.click(screen.getByRole('button', { name })); await settle(); return sent(bridge).map(each => each.ops); };
    notify('timeline', { frame: 130 }); await settle();
    expect(await press('Dissolve')).toEqual([[{ edit: 'add_transition', timeline: 't1', left: 'c1', right: 'c2', kind: 'dissolve', length: 25, align: 'center' }]]);
    // 150 is as near to the cut at 100 as to that at 200: the earlier.
    notify('timeline', { frame: 150 }); await settle();
    expect(await press('Dip to black')).toEqual([[{ edit: 'add_transition', timeline: 't1', left: 'c1', right: 'c2', kind: 'dip_black', length: 25, align: 'center' }]]);
    // At a cut that has a transition, it becomes of the kind asked for.
    notify('timeline', { frame: 190 }); await settle();
    expect(await press('Dip to black')).toEqual([[{ edit: 'set_transition', timeline: 't1', id: 'x1', kind: 'dip_black' }]]);
    // Ctrl+T on the Cut page is a dissolve there too.
    bridge.call.mockClear(); key('t', { ctrlKey: true }); await settle();
    expect(sent(bridge).map(each => each.ops)).toEqual([[{ edit: 'set_transition', timeline: 't1', id: 'x1', kind: 'dissolve' }]]);
  });

  it('says so where no two clips meet on a track of picture, and sends nothing', async () => {
    const apart = cuts();
    apart.tracks = [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [clip('c1', 0, 100), clip('c2', 120, 100)], transitions: [] }];
    expect(nearestEditPoint(apart, 110)).toBeNull();
    const { bridge } = await mount({ timelines: [apart] });
    fireEvent.click(screen.getByRole('button', { name: 'Dissolve' })); await settle();
    expect(within(detail()).getByRole('alert').textContent).toContain('No two clips meet on a video track near the playhead.');
    expect(sent(bridge)).toEqual([]);
  });

  it('calls nothing Close Up, which in DaVinci Resolve frames a face, and uses the words of its Chinese Cut page', async () => {
    await mount({ words: zhCN });
    const names = [...page().querySelectorAll<HTMLElement>('button')].map(each => `${each.getAttribute('aria-label') ?? ''} ${each.textContent ?? ''} ${each.title}`);
    expect(names.some(name => /close ?up|拼接|特写/i.test(name))).toBe(false);
    expect(within(screen.getByRole('toolbar', { name: '编辑' })).getAllByRole('button').map(each => each.getAttribute('aria-label')))
      .toEqual(['智能插入', '追加', '波纹覆盖', '放到上层', '源覆盖']);
    expect(within(screen.getByRole('toolbar', { name: '转场' })).getAllByRole('button').map(each => each.getAttribute('aria-label')))
      .toEqual(['叠化', '淡入淡出至黑场']);
    expect(within(screen.getByRole('group', { name: '检视器模式' })).getAllByRole('button').map(each => each.textContent)).toEqual(['源片段', '源磁带', '时间线']);
    expect(screen.getByRole('button', { name: '源覆盖' }).title).toContain('这个编辑不做任何事');
  });

  it('narrows to buttons without names, and hides its native views under what lies over the page', async () => {
    const { bridge, overlay } = await mount();
    expect(page().dataset.narrow).toBeUndefined();
    act(() => { width = CUT_NARROW_PX - 1; observers.forEach(callback => callback([], {} as ResizeObserver)); });
    expect(page().dataset.narrow).toBe('true');
    expect(screen.getByRole('button', { name: 'Smart insert' })).toBeDefined();
    expect(lastShow(bridge, 'timeline')).toBe(true);
    overlay(true); await settle();
    expect(lastShow(bridge, 'timeline')).toBe(false);
    expect(lastShow(bridge, 'source')).toBe(false);
    overlay(false); await settle();
    expect(lastShow(bridge, 'timeline')).toBe(true);
    expect(lastShow(bridge, 'source')).toBe(false);
  });

  it('turns the buttons of the source off where the viewer has no source, and of all where there is no timeline', async () => {
    await mount({ timelines: [] });
    expect((screen.getByRole('button', { name: 'Smart insert' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: 'Dissolve' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText('Create a timeline on the Edit page to begin.')).toBeDefined();
  });
});
