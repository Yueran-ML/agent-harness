// @vitest-environment jsdom
/**
 * Tracks of subtitles on the Edit page, as DaVinci Resolve edits them: a track added from the "+"
 * over the headers in a language; a subtitle added at the playhead by a double-click of empty space;
 * moved by its middle and made longer or shorter by its edges, held between its neighbours; its text
 * and timing set in the inspector; lifted with Backspace or ripple deleted with Delete; a track turned off for the
 * viewer and the export. Each gesture is one transaction with the revision it was worked out from;
 * what the engine refuses is said. And the copy of the project follows the cuts and shifts of
 * subtitles that a ripple brings.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import { Studio } from '../src/studio/Studio';
import { applyChange, applyOp, ProjectCopy, type Cue, type EngineEvent, type Op, type Project, type SubtitleTrack, type TransportState } from '../src/studio/engine';
import type { EngineBridge } from '../src/bridge';
import type { Change } from '../src/studio/timeline/commands';
import { HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const SCALE = 4;
const rect = (left: number, top: number, width: number, height: number): DOMRect => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} });
let frames: FrameRequestCallback[];
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const draw = () => act(() => { const pending = frames; frames = []; pending.forEach(callback => callback(0)); });
beforeEach(() => {
  likeABrowser(); frames = [];
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.push(callback); return frames.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('edit-page')) return rect(0, 0, 1500, 900);
    // The style sheet places a subtitle by its frames and the pixels of a frame of the timeline.
    if (this.dataset.cue || this.dataset.clip) {
      return rect(HEADER_WIDTH + Number(this.style.getPropertyValue('--start')) * SCALE, 34, Number(this.style.getPropertyValue('--length')) * SCALE, 64);
    }
    return rect(0, 0, 956, 400);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; });

const subtitles = (id: string, cues: Cue[], fields: Partial<SubtitleTrack> = {}): SubtitleTrack => ({ id, kind: 'subtitle', name: id, language: 'en', enabled: true, locked: false, cues, ...fields });
const cue = (id: string, start: number, length: number, text = id): Cue => ({ id, start, length, text });
const overlapping = (track: SubtitleTrack, id: string | null, start: number, length: number) =>
  track.cues.some(each => each.id !== id && each.start < start + length && start < each.start + each.length);

/**
 * An engine that keeps a project, and expands the edits of subtitles as the real one does: a track
 * added above the others with the next id, a subtitle given the next id; an overlap refused with
 * `cue_overlap`, a stale revision with `stale_revision`. It tells what changed as the real one does.
 */
function fakeEngine(tracks: SubtitleTrack[]) {
  let project: Project = { schema: 5, groups: [], next_id: 10, bins: [], media: [media], timelines: [{ ...timeline([]), tracks: [...timeline([]).tracks, ...tracks] }] };
  let revision = 0;
  let made = 0;
  const listeners = new Set<(event: EngineEvent) => void>();
  const tell = (method: string, params: unknown) => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method, params }));
  const states: Record<'source' | 'timeline', TransportState> = {
    source: { monitor: 'source', media: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null },
    timeline: { monitor: 'timeline', timeline: null, frame: null, shown: null, last: null, speed: 1, playing: false, proxies: true, exact: true, sound: 'off', source: null, late: 0 },
  } as unknown as Record<'source' | 'timeline', TransportState>;
  const refused = (reason: string) => ({ ok: false, error: { code: -32001, message: `refused: ${reason}`, data: { reason } } });
  const trackOf = (id: string) => project.timelines[0].tracks.find(each => each.id === id) as SubtitleTrack;
  const trackOfCue = (id: string) => project.timelines[0].tracks.find(each => each.kind === 'subtitle' && each.cues.some(one => one.id === id)) as SubtitleTrack;
  function expand(change: Change): Op | string {
    if ('op' in change) return change;
    switch (change.edit) {
      case 'add_track': {
        const id = `S${project.timelines[0].tracks.filter(each => each.kind === 'subtitle').length + 1}`;
        // Added hidden while another is shown: a timeline shows one subtitle track at a time.
        const shown = project.timelines[0].tracks.some(each => each.kind === 'subtitle' && each.enabled);
        return { op: 'add_track', timeline: change.timeline, track: subtitles(id, [], { language: change.language ?? '', enabled: !shown }), index: null };
      }
      case 'add_cue': {
        const track = trackOf(change.track);
        if (track.locked) return 'locked';
        if (overlapping(track, null, change.start, change.length)) return 'cue_overlap';
        made += 1;
        return { op: 'add_cue', timeline: change.timeline, track: change.track, cue: cue(`q${100 + made}`, change.start, change.length, change.text) };
      }
      case 'set_cue': {
        const track = trackOfCue(change.id);
        const was = track.cues.find(each => each.id === change.id)!;
        if (overlapping(track, change.id, change.start ?? was.start, change.length ?? was.length)) return 'cue_overlap';
        const { edit: _, ...rest } = change;
        return { op: 'set_cue', ...rest };
      }
      case 'remove_cue': return { op: 'remove_cue', timeline: change.timeline, id: change.id };
      default: throw new Error(`no ${change.edit} here`);
    }
  }
  const bridge = {
    open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film/.columbina/studio', owned: true, pid: 9, revision }; listeners.forEach(listener => listener({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string, params?: Record<string, unknown>) => {
      if (method === 'project.get') return { ok: true, result: { project, revision, next_id: project.next_id } };
      const monitor = params?.monitor as 'source' | 'timeline';
      if (method === 'transport.state') return { ok: true, result: states[monitor] };
      if (method === 'transport.load' && monitor === 'timeline') {
        states.timeline = { ...states.timeline, timeline: params!.timeline as string, frame: 0, shown: 0, last: 499 } as TransportState;
        tell('transport.state', states.timeline); return { ok: true, result: states.timeline };
      }
      if (method === 'transaction.apply') {
        if (params!.revision !== revision) return refused('stale_revision');
        const done: Op[] = [];
        let next = project;
        for (const change of params!.ops as Change[]) {
          const op = expand(change);
          if (typeof op === 'string') return refused(op);
          // As the engine: a subtitle track is shown only where no other of the timeline is.
          if (op.op === 'set_track' && op.enabled === true && next.timelines[0].tracks.some(each => each.kind === 'subtitle' && each.enabled && each.id !== op.id)
            && next.timelines[0].tracks.some(each => each.id === op.id && each.kind === 'subtitle')) return refused('subtitle_shown');
          next = applyOp(next, op); done.push(op);
          project = next;
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
  return {
    bridge,
    cues: (track: string) => trackOf(track).cues.map(({ id, start, length, text }) => [id, start, length, text]),
    playhead: (frame: number) => act(() => { states.timeline = { ...states.timeline, frame, shown: frame } as TransportState; tell('transport.state', states.timeline); }),
  };
}
async function mount(tracks: SubtitleTrack[], words?: typeof zhCN) {
  const engine = fakeEngine(tracks);
  const tree = <Studio cwd="F:/film" engine={{ bridge: engine.bridge, copy: new ProjectCopy(engine.bridge), native: true }} language="en" onClose={() => {}} />;
  render(words ? <Words.Provider value={words}>{tree}</Words.Provider> : tree);
  await settle(); fireEvent.click(screen.getByRole('tab', { name: words ? '剪辑' : 'Edit' })); await settle(); draw(); await settle();
  return engine;
}
const sent = (bridge: { call: ReturnType<typeof vi.fn> }) => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
const timelineRegion = () => screen.getByRole('region', { name: 'Timeline' });
const lane = (track: string) => document.querySelector<HTMLElement>(`[data-track="${track}"] .timeline-lane`)!;
const cueElement = (id: string) => document.querySelector<HTMLElement>(`[data-cue="${id}"]`)!;
const x = (frame: number) => HEADER_WIDTH + frame * SCALE;
/** Drag a subtitle from `from` to `to` (frames) and let it go: Escape on the way gives it up. */
async function drag(id: string, from: number, to: number, giveUp = false) {
  fireEvent.pointerDown(cueElement(id), { button: 0, clientX: x(from), clientY: 60 });
  fireEvent.pointerMove(timelineRegion(), { clientX: x((from + to) / 2), clientY: 60 });
  fireEvent.pointerMove(timelineRegion(), { clientX: x(to), clientY: 60 });
  const preview = document.querySelector<HTMLElement>(`[data-preview="${id}"]`);
  const shown = preview && { left: preview.style.left, width: preview.style.width, clamped: preview.dataset.clamped };
  if (giveUp) fireEvent.keyDown(timelineRegion(), { key: 'Escape' });
  fireEvent.pointerUp(timelineRegion(), { clientX: x(to), clientY: 60 });
  await settle();
  return shown;
}

describe('a track of subtitles', () => {
  it('is added from the "+" over the headers, in the language chosen, as one transaction', async () => {
    const engine = await mount([]);
    fireEvent.click(screen.getByRole('button', { name: 'Add track' }));
    const menu = screen.getByRole('dialog', { name: 'Add subtitle track' });
    // Escape puts it away, and nothing is sent.
    fireEvent.keyDown(within(menu).getByRole('combobox'), { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'Add subtitle track' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Add track' }));
    const language = screen.getByRole('combobox', { name: 'Language' }) as HTMLSelectElement;
    // The language of the page is offered first.
    expect(language.value).toBe('en');
    fireEvent.change(language, { target: { value: 'ja' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add subtitle track' }));
    await settle();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'add_track', timeline: 't1', kind: 'subtitle', language: 'ja' }] }]);
    expect(screen.queryByRole('dialog', { name: 'Add subtitle track' })).toBeNull();
    expect(document.querySelector('[data-track="S1"] [data-test="track-language"]')!.textContent).toBe(' · ja');
    expect(lane('S1').title).toBe('Double-click empty space on a subtitle track to add a subtitle at the playhead, or right-click it and choose Add subtitle to add one where you clicked.');
  });

  it('is added from the menu of a right-click on a track header, and a subtitle from that of a subtitle track, where it was clicked', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 200, 50)]), subtitles('S2', [], { locked: true, enabled: false })]);
    // Reference Manual 20, p. 1088: right-click a track header and choose Add Subtitle Track.
    fireEvent.contextMenu(document.querySelector('[data-track="v1"] .timeline-track-header')!, { clientX: 40, clientY: 120 });
    const menu = screen.getByRole('menu', { name: 'Track menu' });
    expect(within(menu).getAllByRole('menuitem').map(each => each.textContent)).toEqual(['Add subtitle track']);
    expect(document.activeElement).toBe(within(menu).getByRole('menuitem'));
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Add subtitle track' }));
    await settle();
    expect(screen.queryByRole('menu')).toBeNull();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'add_track', timeline: 't1', kind: 'subtitle', language: 'en' }] }]);
    // P. 1089: right-click a subtitle track and choose Add Subtitle. Resolve starts it at the playhead; here it
    // starts where the track was clicked, as the author chose.
    engine.playhead(100);
    fireEvent.contextMenu(lane('S1'), { clientX: x(400), clientY: 60 });
    fireEvent.click(screen.getByRole('menuitem', { name: 'Add subtitle' }));
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 1, ops: [{ edit: 'add_cue', timeline: 't1', track: 'S1', start: 400, length: 75, text: 'Subtitle' }] });
    // On a locked track it cannot be chosen; Escape puts the menu away; a track of picture offers none of it.
    fireEvent.contextMenu(lane('S2'), { clientX: x(400), clientY: 60 });
    expect((screen.getByRole('menuitem', { name: 'Add subtitle' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    // A track of picture offers its Edit menu (cut, copy, paste), and nothing of subtitles.
    fireEvent.contextMenu(lane('v1'), { clientX: x(400), clientY: 60 });
    expect(screen.getByRole('menu', { name: 'Edit' })).toBeTruthy();
    expect(screen.queryByRole('menuitem', { name: /subtitle/i })).toBeNull();
  });

  it('is shown one at a time from its header, as DaVinci Resolve shows subtitle tracks', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 100, 50)]), subtitles('S2', [], { enabled: false })]);
    const shown = () => [...document.querySelectorAll<HTMLElement>('[data-kind="subtitle"]')].filter(each => each.dataset.enabled === 'true').map(each => each.dataset.track);
    // Reference Manual 20, p. 1084: enabling one subtitle track disables the others, in one transaction (the shown one first, as the engine takes them).
    fireEvent.click(screen.getByRole('button', { name: 'Show in the viewer, in place of the subtitle track shown: S2' }));
    await settle();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [
      { op: 'set_track', timeline: 't1', id: 'S1', enabled: false }, { op: 'set_track', timeline: 't1', id: 'S2', enabled: true },
    ] }]);
    expect(shown()).toEqual(['S2']);
    // Hidden, none is shown; shown again, nothing else is to hide.
    fireEvent.click(screen.getByRole('button', { name: 'Hide in the viewer: S2' }));
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 1, ops: [{ op: 'set_track', timeline: 't1', id: 'S2', enabled: false }] });
    expect(shown()).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: 'Show in the viewer, in place of the subtitle track shown: S1' }));
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 2, ops: [{ op: 'set_track', timeline: 't1', id: 'S1', enabled: true }] });
    // A track added while one is shown comes hidden.
    fireEvent.click(screen.getByRole('button', { name: 'Add track' }));
    fireEvent.click(screen.getByRole('button', { name: 'Add subtitle track' }));
    await settle();
    expect(shown()).toEqual(['S1']);
    expect(document.querySelector<HTMLElement>('[data-track="S3"]')!.dataset.enabled).toBe('false');
    // What the engine refuses so (another client showed one meanwhile) is said in a sentence of its own.
    engine.bridge.call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32001, message: 'subtitle track S1 is shown, and a timeline shows one subtitle track at a time', data: { reason: 'subtitle_shown', track: 'S1', index: 0 } } }));
    fireEvent.click(screen.getByRole('button', { name: 'Show in the viewer, in place of the subtitle track shown: S3' }));
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('Another subtitle track is shown. A timeline shows one subtitle track at a time.');
  });
});

describe('a subtitle', () => {
  it('is added at the playhead by a double-click of empty space, for three seconds or until the next, and opened in the inspector', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 200, 50)]), subtitles('S2', [], { locked: true })]);
    expect(document.querySelector('[data-test="inspector-slot"]')).toBeNull();
    engine.playhead(100);
    fireEvent.doubleClick(lane('S1'), { clientX: x(400), clientY: 60 });
    await settle();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'add_cue', timeline: 't1', track: 'S1', start: 100, length: 75, text: 'Subtitle' }] }]);
    expect(engine.cues('S1')).toEqual([['q101', 100, 75, 'Subtitle'], ['q1', 200, 50, 'q1']]);
    // The new one is chosen, and the inspector opens on it, to be written in.
    expect(cueElement('q101').dataset.selected).toBe('true');
    const inspector = document.querySelector<HTMLElement>('[data-test="inspector"]')!;
    expect(within(inspector).getByText('Subtitle', { selector: '.inspector-what' })).toBeDefined();
    expect((inspector.querySelector('[data-field="cue_text"] textarea') as HTMLTextAreaElement).value).toBe('Subtitle');
    // Just before the next, it runs up to it.
    engine.playhead(180);
    fireEvent.doubleClick(lane('S1'), { clientX: x(10), clientY: 60 });
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 1, ops: [{ edit: 'add_cue', timeline: 't1', track: 'S1', start: 180, length: 20, text: 'Subtitle' }] });
    // Where a subtitle is shown at the playhead, or the track is locked, it says so and sends nothing.
    const count = sent(engine.bridge).length;
    engine.playhead(210);
    fireEvent.doubleClick(lane('S1'));
    expect(screen.getByRole('alert').textContent).toContain('Another subtitle occupies these frames.');
    fireEvent.doubleClick(lane('S2'));
    expect(screen.getByRole('alert').textContent).toContain('A track affected by this edit is locked.');
    // A double-click on a clip track adds nothing.
    fireEvent.doubleClick(lane('v1'));
    await settle();
    expect(sent(engine.bridge)).toHaveLength(count);
  });

  it('is moved by its middle and made longer or shorter by its edges, held at its neighbours, one change each', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 100, 50), cue('q2', 200, 50)])]);
    expect(await drag('q1', 120, 150)).toEqual({ left: `${x(130)}px`, width: `${50 * SCALE}px`, clamped: 'false' });
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q1', start: 130 }] }]);
    // Its end, dragged past the next one, stops at it: the preview says it is held there.
    expect(await drag('q1', 179, 300)).toEqual({ left: `${x(130)}px`, width: `${70 * SCALE}px`, clamped: 'true' });
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 1, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q1', length: 70 }] });
    expect(engine.cues('S1')).toEqual([['q1', 130, 70, 'q1'], ['q2', 200, 50, 'q2']]);
    // The start of the next one, dragged later, makes it shorter; it is never shorter than a frame.
    expect(await drag('q2', 200, 400)).toEqual({ left: `${x(249)}px`, width: `${1 * SCALE}px`, clamped: 'true' });
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 2, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q2', start: 249, length: 1 }] });
    // Moved back past the first, it stops at its end.
    await drag('q2', 249.5, 0);
    expect(engine.cues('S1')).toEqual([['q1', 130, 70, 'q1'], ['q2', 200, 1, 'q2']]);
    // Escape gives the drag up; a click without a move only chooses it.
    const count = sent(engine.bridge).length;
    expect(await drag('q1', 160, 170, true)).not.toBeNull();
    fireEvent.pointerDown(cueElement('q1'), { button: 0, clientX: x(160), clientY: 60 });
    fireEvent.pointerUp(timelineRegion(), { clientX: x(160), clientY: 60 });
    await settle();
    expect(sent(engine.bridge)).toHaveLength(count);
    expect(cueElement('q1').dataset.selected).toBe('true');
    expect(document.querySelector('[data-preview]')).toBeNull();
  });

  it('is not dragged on a locked track, nor with the blade', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 100, 50)], { locked: true }), subtitles('S2', [cue('q2', 100, 50)])]);
    await drag('q1', 120, 160);
    fireEvent.keyDown(timelineRegion(), { key: 'b' });
    await drag('q2', 120, 160);
    expect(sent(engine.bridge)).toEqual([]);
  });

  it('has its text, start and length set in the inspector, and says why the engine refused an overlap', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 100, 50, 'Hello'), cue('q2', 200, 50)])]);
    fireEvent.doubleClick(cueElement('q1'));
    await settle();
    const field = document.querySelector<HTMLTextAreaElement>('[data-field="cue_text"] textarea')!;
    expect(field.value).toBe('Hello');
    fireEvent.change(field, { target: { value: 'Hello\nworld' } });
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q1', text: 'Hello\nworld' }] }]);
    expect(cueElement('q1').textContent).toBe('Hello\nworld');
    const start = document.querySelector<HTMLInputElement>('[data-field="cue_start"] input')!;
    fireEvent.change(start, { target: { value: '120' } });
    fireEvent.keyDown(start, { key: 'Enter' });
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 1, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q1', start: 120 }] });
    // Longer than the room before the next: the engine refuses, and the inspector says why.
    const length = document.querySelector<HTMLInputElement>('[data-field="cue_length"] input')!;
    fireEvent.change(length, { target: { value: '100' } });
    fireEvent.keyDown(length, { key: 'Enter' });
    await settle();
    expect(within(document.querySelector<HTMLElement>('[data-test="inspector"]')!).getByRole('alert').textContent).toContain('Another subtitle occupies these frames.');
    expect(engine.cues('S1')).toEqual([['q1', 120, 50, 'Hello\nworld'], ['q2', 200, 50, 'q2']]);
  });

  it('takes no key while an input method composes its text, nor while its text is written', async () => {
    const engine = await mount([subtitles('S1', [cue('q1', 100, 50, 'Hello')])]);
    fireEvent.doubleClick(cueElement('q1'));
    await settle();
    const field = document.querySelector<HTMLTextAreaElement>('[data-field="cue_text"] textarea')!;
    field.focus();
    fireEvent.change(field, { target: { value: '你好' } });
    // Enter that ends a composition, and the keys of editing on the way, do nothing to the timeline.
    for (const key of ['Enter', 'Delete', 'Backspace', 'b', 'n']) fireEvent.keyDown(field, { key, isComposing: true, keyCode: 229 });
    // Delete and Backspace while writing are the text's.
    fireEvent.keyDown(field, { key: 'Backspace' });
    fireEvent.keyDown(field, { key: 'Delete' });
    await settle();
    expect(sent(engine.bridge)).toEqual([]);
    expect(timelineRegion().dataset.tool).toBe('select');
    fireEvent.keyDown(field, { key: 'Enter' });
    await settle();
    expect(sent(engine.bridge)).toEqual([{ revision: 0, ops: [{ edit: 'set_cue', timeline: 't1', id: 'q1', text: '你好' }] }]);
  });

  it('is lifted with Backspace, ripple deleted with Delete (the later ones of its track moving back), or taken from the inspector', async () => {
    // Within the window that the timeline draws, 300 frames wide here.
    const engine = await mount([subtitles('S1', [cue('q1', 100, 40), cue('q2', 160, 40), cue('q3', 220, 40)])]);
    fireEvent.pointerDown(cueElement('q1'), { button: 0, clientX: x(120), clientY: 60 });
    fireEvent.pointerUp(timelineRegion(), { clientX: x(120), clientY: 60 });
    fireEvent.keyDown(timelineRegion(), { key: 'Backspace' });
    await settle();
    fireEvent.pointerDown(cueElement('q2'), { button: 0, clientX: x(180), clientY: 60 });
    fireEvent.pointerUp(timelineRegion(), { clientX: x(180), clientY: 60 });
    fireEvent.keyDown(timelineRegion(), { key: 'Delete' });
    await settle();
    // As DaVinci Resolve: Backspace leaves the gap; Delete closes it on the subtitle's track (Reference Manual 20, pp. 765, 1092).
    expect(sent(engine.bridge)).toEqual([
      { revision: 0, ops: [{ edit: 'remove_cue', timeline: 't1', id: 'q1' }] },
      { revision: 1, ops: [{ edit: 'remove_cue', timeline: 't1', id: 'q2' }, { op: 'shift_cues', timeline: 't1', track: 'S1', cues: ['q3'], by: -40 }] },
    ]);
    expect(engine.cues('S1')).toEqual([['q3', 180, 40, 'q3']]);
    fireEvent.doubleClick(cueElement('q3'));
    await settle();
    fireEvent.click(document.querySelector('[data-test="remove-cue"]')!);
    await settle();
    expect(sent(engine.bridge).at(-1)).toEqual({ revision: 2, ops: [{ edit: 'remove_cue', timeline: 't1', id: 'q3' }] });
    expect(engine.cues('S1')).toEqual([]);
    // Nothing is chosen any more: the inspector shows the timeline.
    expect(screen.getByText('Select a clip on the timeline.')).toBeDefined();
  });

  it('is said in Chinese as DaVinci Resolve says it', async () => {
    await mount([subtitles('S1', [cue('q1', 100, 50)])], zhCN);
    expect(screen.getByRole('button', { name: '在检视器中隐藏: S1' })).toBeDefined();
    fireEvent.click(screen.getByRole('button', { name: '添加轨道' }));
    expect(screen.getByRole('dialog', { name: '添加字幕轨道' })).toBeDefined();
    expect(lane('S1').title).toBe('在字幕轨道的空处双击，会在播放头处添加一条字幕；右键选择“添加字幕”，会在点击的位置添加。');
  });
});

describe('the copy of the project', () => {
  it('follows a subtitle cut in two and shifted by a ripple, as the engine expands it', () => {
    const t = { ...timeline([]), tracks: [...timeline([]).tracks, subtitles('S1', [cue('q1', 100, 100, 'Both halves'), cue('q2', 300, 20)])] };
    const project: Project = { schema: 5, groups: [], next_id: 10, bins: [], media: [media], timelines: [t] };
    // An insert of 40 frames at 150: the subtitle across it is cut there, and what follows moves on.
    const done: Op[] = [
      { op: 'set_cue', timeline: 't1', id: 'q1', length: 50 },
      { op: 'add_cue', timeline: 't1', track: 'S1', cue: cue('q11', 150, 50, 'Both halves') },
      { op: 'shift_cues', timeline: 't1', track: 'S1', cues: ['q11', 'q2'], by: 40 },
    ];
    const after = applyChange(project, { done, next_id: 12 });
    const track = after.timelines[0].tracks.find(each => each.id === 'S1') as SubtitleTrack;
    expect(track.cues.map(({ id, start, length, text }) => [id, start, length, text])).toEqual([
      ['q1', 100, 50, 'Both halves'], ['q11', 190, 50, 'Both halves'], ['q2', 340, 20, 'q2'],
    ]);
    expect(after.next_id).toBe(12);
  });
});
