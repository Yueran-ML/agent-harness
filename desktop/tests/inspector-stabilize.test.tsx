// @vitest-environment jsdom
/**
 * Stabilization of the Video tab of the Inspector of the Edit and Cut pages (DaVinci Resolve's, Reference Manual 20,
 * pp. 633–634 and 1064–1065): the controls of a clip's stabilizer, shown as a clip is given one at first where it has none
 * and each change sent as one `set_stabilize_settings` of the control changed alone, never what was measured, however long;
 * a slider sent once, as it is let go; Camera Lock, which leaves the Cropping Ratio and
 * Smooth no say; the Stabilize button, which has the engine measure the clip (`track.stabilize`), what it says while the
 * measure goes on and how it ended, Stop, and the measure stopped as the clip or the Inspector goes; the note of a Method
 * that needs a new analysis; Reset; several clips; and the copy of the project that keeps the stabilizer as the engine does.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { en } from '../src/messages/en';
import {
  applyOp, DEFAULT_STABILIZE, emptyGrade, type Clip, type EngineEvent, type Media, type Project, type Stabilize, type StabilizeMotion,
  type StabilizeSettings, type Timeline,
} from '../src/studio/engine';
import { Inspector } from '../src/studio/inspector/Inspector';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const labels = en.label.studio.inspector;
const said = en.hint.studio.inspector.stabilize;
const rate = { num: 25, den: 1 };
const timelineOf = (clips: Clip[], locked = false): Timeline => ({
  id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});
const title = (id: string, start: number): Clip => clip(id, start, 50, {
  source: { kind: 'title', text: 'Hello', style: { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } },
});
/** A picture that does not move: one frame, held for as long as its clip lasts. */
const still: Media = { ...media, id: 'm2', name: 'Poster', facts: { duration: media.facts.duration, video: { ...media.facts.video!, still: true } } };
/** A stabilizer measured for Similarity over three frames of 1280 × 720, as the engine writes one. */
const measured = (fields: Partial<Stabilize> = {}): Stabilize => ({
  method: 'similarity', camera_lock: false, zoom: true, cropping_ratio: 0.5, smooth: 0.25, strength: 1,
  analysis: { method: 'similarity', size: [1280, 720], path: {
    0: [0, 0, 1, 0, 1, 0, 0, 0], 1: [2.5, -1.25, 1.001, 0.002, 1, 0, 0, 0], 2: [4.75, 0.5, 0.999, -0.001, 1, 0, 0, 0],
  } },
  ...fields,
});

function mount(clips: Clip[], given: { chosen?: string[]; locked?: boolean } = {}) {
  const listeners: ((event: EngineEvent) => void)[] = [];
  // `track.stabilize` is answered at once with its job, as the engine answers it: the measure itself is told by `tell`.
  const call = vi.fn(async (method: string, params?: Record<string, unknown>, _context?: unknown) => {
    if (method === 'track.stabilize') return { ok: true, result: { job: 'track-1', first: 0, last: 59, frame_size: [1280, 720], method: params?.method } };
    return { ok: true, result: {} };
  });
  const bridge = { call, onEvent: (listener: (event: EngineEvent) => void) => { listeners.push(listener); return () => { listeners.splice(listeners.indexOf(listener), 1); }; } };
  const props = {
    bridge: bridge as unknown as Pick<EngineBridge, 'call' | 'onEvent'>, session: 1, revision: 17, timeline: timelineOf(clips, given.locked),
    media: [media, still], clips: given.chosen ?? clips.map(each => each.id), transition: null, onSpeed: vi.fn(),
  };
  const view = render(<Inspector {...props} />);
  const rerender = (changed: Partial<typeof props>) => view.rerender(<Inspector {...props} {...changed} />);
  const tell = (method: string, params: unknown, session = 1) => act(() => { for (const each of [...listeners]) each({ kind: 'notification', session, method, params }); });
  const sent = (method: string) => call.mock.calls.filter(([asked]) => asked === method).map(([, params]) => params);
  return { call, view, rerender, tell, sent, applied: () => sent('transaction.apply') };
}
const row = (name: string) => document.querySelector<HTMLElement>(`[data-field="${name}"]`)!;
const box = (name: string) => row(name).querySelector<HTMLInputElement>('input.inspector-number')!;
const slider = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="range"]')!;
const check = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="checkbox"]')!;
const methodOf = () => row('stabilize-method').querySelector<HTMLSelectElement>('select')!;
const byTest = (id: string) => document.querySelector<HTMLElement>(`[data-test="${id}"]`);
const button = (id: string) => byTest(id) as HTMLButtonElement;
const status = () => byTest('stabilize-status')?.textContent ?? null;
const section = () => document.querySelector('[data-section="stabilization"]');
const type = (input: HTMLInputElement, value: string) => { fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' }); };
const set = (value: Stabilize | null, id = 'c1') => ({ op: 'set_stabilize', timeline: 't1', clip: id, value });
const settings = (fields: StabilizeSettings, id = 'c1') => ({ op: 'set_stabilize_settings', timeline: 't1', clip: id, settings: fields });
const ended = (fields: Record<string, unknown>) => ({ job: 'track-1', phase: 'done', reason: null, frames: 60, lost: 0, frame: 59, revision: 18, analysed: [0, 59], method: 'perspective', ...fields });

describe('Stabilization of the Video tab', () => {
  it('shows a clip that has no stabilizer as a clip is given one at first, after the cropping as DaVinci Resolve has it', () => {
    mount([clip('c1', 10, 60)]);
    const headings = [...document.querySelectorAll('h3.inspector-section')].map(heading => heading.textContent);
    expect(headings.indexOf(labels.stabilization)).toBe(headings.indexOf(labels.cropping) + 1);
    expect(headings.indexOf(labels.retimeAndScaling)).toBe(headings.indexOf(labels.stabilization) + 1);
    expect(methodOf().value).toBe('perspective');
    expect([...methodOf().options].map(option => [option.value, option.textContent]))
      .toEqual([['perspective', 'Perspective'], ['similarity', 'Similarity'], ['translation', 'Translation']]);
    expect(check('stabilize-camera-lock').checked).toBe(false);
    expect(check('stabilize-zoom').checked).toBe(true);
    expect(['stabilize-cropping-ratio', 'stabilize-smooth', 'stabilize-strength'].map(name => box(name).value)).toEqual(['0.500', '0.250', '1.000']);
    expect([slider('stabilize-strength').min, slider('stabilize-strength').max]).toEqual(['-1', '1']);
    // Nothing to take away, nothing measured for another Method, and nothing measuring.
    expect(button('stabilize-reset').disabled).toBe(true);
    expect(button('stabilize-reset').getAttribute('aria-label')).toBe(labels.resetStabilization);
    expect(button('stabilize-run').disabled).toBe(false);
    expect(button('stabilize-run').textContent).toBe(labels.stabilize);
    expect([byTest('stabilize-again'), byTest('stabilize-stop'), byTest('stabilize-status'), byTest('stabilize-one')]).toEqual([null, null, null, null]);
  });

  it('sends the control changed alone of a clip that has none, which the engine gives the stabilizer of a clip at first, each change one transaction', async () => {
    const { applied } = mount([clip('c1', 10, 60)]);
    type(box('stabilize-smooth'), '0.6');
    await settle();
    expect(applied()).toEqual([{ revision: 17, ops: [settings({ smooth: 0.6 })] }]);
    fireEvent.click(check('stabilize-camera-lock'));
    await settle();
    fireEvent.change(methodOf(), { target: { value: 'translation' } });
    await settle();
    expect(applied().slice(1)).toEqual([
      { revision: 17, ops: [settings({ camera_lock: true })] },
      { revision: 17, ops: [settings({ method: 'translation' })] },
    ]);
  });

  it('keeps the analysis of a measured clip as a control changes, and says that another Method is to be measured again', async () => {
    const { applied, rerender } = mount([clip('c1', 10, 60, { stabilize: measured() })]);
    expect(methodOf().value).toBe('similarity');
    expect(button('stabilize-reset').disabled).toBe(false);
    expect(byTest('stabilize-again')).toBeNull();
    type(box('stabilize-strength'), '-0.5');
    await settle();
    expect(applied()).toEqual([{ revision: 17, ops: [settings({ strength: -0.5 })] }]);
    fireEvent.change(methodOf(), { target: { value: 'translation' } });
    await settle();
    // The engine keeps the analysis measured for Similarity: the Method alone is sent.
    expect(applied()[1]).toEqual({ revision: 17, ops: [settings({ method: 'translation' })] });
    // The copy takes it in: the analysis measured for Similarity is drawn until the clip is measured again.
    rerender({ revision: 18, timeline: timelineOf([clip('c1', 10, 60, { stabilize: measured({ method: 'translation' }) })]) });
    expect(byTest('stabilize-again')!.textContent).toBe('Stabilize again to use Translation.');
    expect(byTest('stabilize-again')!.textContent).toBe(said.again.replace('{method}', labels.stabilizeMethods.translation));
  });

  it('turns the Cropping Ratio and Smooth off while Camera Lock is on', () => {
    mount([clip('c1', 10, 60, { stabilize: measured({ camera_lock: true }) })]);
    expect(check('stabilize-camera-lock').checked).toBe(true);
    for (const name of ['stabilize-cropping-ratio', 'stabilize-smooth']) {
      expect([box(name).disabled, slider(name).disabled], name).toEqual([true, true]);
    }
    expect([box('stabilize-strength').disabled, check('stabilize-zoom').disabled, methodOf().disabled]).toEqual([false, false, false]);
  });

  it('measures the clip for its Method at the revision of the project, says how far it went, and stops it with Stop', async () => {
    const { call, sent, tell } = mount([clip('c1', 10, 60, { stabilize: measured({ method: 'translation' }) })]);
    fireEvent.click(button('stabilize-run'));
    await settle();
    expect(sent('track.stabilize')).toEqual([{ timeline: 't1', clip: 'c1', revision: 17, method: 'translation' }]);
    // Of the engine of the session that the page is on.
    expect(call.mock.calls.find(([asked]) => asked === 'track.stabilize')![2]).toEqual({ expectedSession: 1 });
    expect(status()).toBe('Analyzing: 0 of 60 frames.');
    // While it measures, the rest of the section is off: a change of the project would end the measure.
    expect(button('stabilize-run').disabled).toBe(true);
    expect([methodOf().disabled, check('stabilize-zoom').disabled, check('stabilize-camera-lock').disabled, box('stabilize-strength').disabled,
      button('stabilize-reset').disabled]).toEqual([true, true, true, true, true]);
    await tell('track.progress', { job: 'track-1', frame: 12, timeline_frame: 22, done: 13, total: 60, lost: 0, revision: 17 });
    expect(status()).toBe('Analyzing: 13 of 60 frames.');
    // The news of another job is not this one's.
    await tell('track.state', ended({ job: 'track-9', frames: 1, analysed: [0, 0] }));
    expect(byTest('stabilize-stop')).not.toBeNull();
    fireEvent.click(button('stabilize-stop'));
    await settle();
    expect(sent('track.stop')).toEqual([{ job: 'track-1' }]);
    await tell('track.state', ended({ phase: 'stopped', reason: 'stop', frames: 13, frame: 12, revision: 17, analysed: null, method: 'translation' }));
    expect(status()).toBe(said.stopped);
    expect(byTest('stabilize-stop')).toBeNull();
    expect([button('stabilize-run').disabled, methodOf().disabled, box('stabilize-strength').disabled]).toEqual([false, false, false]);
  });

  it('says how a measure ended: the frames analysed and where the picture was lost, a change of the project, a failure', async () => {
    const { sent, tell } = mount([clip('c1', 10, 60)]);
    const run = async () => { fireEvent.click(button('stabilize-run')); await settle(); };
    await run();
    // A clip that has no stabilizer is measured for the Method that the Inspector shows.
    expect(sent('track.stabilize')).toEqual([{ timeline: 't1', clip: 'c1', revision: 17, method: 'perspective' }]);
    await tell('track.state', ended({ lost: 2 }));
    expect(status()).toBe('Stabilized. Frames analyzed: 60. The picture was lost at 2 of them, which keep the motion of the frame before.');
    await run();
    await tell('track.state', ended({}));
    expect(status()).toBe('Stabilized. Frames analyzed: 60.');
    await run();
    await tell('track.state', ended({ phase: 'stopped', reason: 'project_changed', frames: 20, analysed: null }));
    expect(status()).toBe(said.changed);
    await run();
    await tell('track.state', ended({ phase: 'failed', reason: 'cannot_read', frames: 0, analysed: null, message: 'no such file' }));
    expect(status()).toBe(said.failed.cannot_read);
    await run();
    await tell('track.state', ended({ phase: 'failed', reason: 'missing_frame', frames: 30, analysed: null }));
    expect(status()).toBe(said.failed.missing_frame);
    await run();
    await tell('track.state', ended({ phase: 'failed', reason: 'failed', frames: 30, analysed: null }));
    expect(status()).toBe(said.failed.other);
    expect(sent('track.stabilize')).toHaveLength(6);
  });

  it('says in words why the engine would not measure', async () => {
    const { call } = mount([clip('c1', 10, 60)]);
    for (const reason of ['untrackable', 'busy', 'stale_revision', 'target'] as const) {
      call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32001, message: 'refused', data: { reason } } }) as never);
      fireEvent.click(button('stabilize-run'));
      await settle();
      expect(status(), reason).toBe(said.failed[reason]);
      expect(button('stabilize-run').disabled, reason).toBe(false);
      expect(byTest('stabilize-stop'), reason).toBeNull();
    }
    expect(said.failed.untrackable).toBe('A title, a color, a still or a clip whose media is missing cannot be stabilized.');
    // A refusal that names no reason is said as the Inspector says a failure, with what the engine said.
    call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32602, message: 'unknown variant', data: null } }) as never);
    fireEvent.click(button('stabilize-run'));
    await settle();
    expect(status()).toBe(`${en.hint.studio.edit.failed} unknown variant`);
  });

  it('stops the measure as another clip is chosen, as the Inspector goes, and once it is answered after the Inspector went', async () => {
    const first = mount([clip('c1', 10, 60), clip('c2', 100, 60)], { chosen: ['c1'] });
    fireEvent.click(button('stabilize-run'));
    await settle();
    first.rerender({ clips: ['c2'] });
    await settle();
    expect(first.sent('track.stop')).toEqual([{ job: 'track-1' }]);
    // What is told of it is not said of the clip shown now.
    await first.tell('track.state', ended({ phase: 'stopped', reason: 'stop', frames: 3, analysed: null }));
    expect(status()).toBeNull();
    cleanup();

    const second = mount([clip('c1', 10, 60)]);
    fireEvent.click(button('stabilize-run'));
    await settle();
    second.view.unmount();
    expect(second.sent('track.stop')).toEqual([{ job: 'track-1' }]);
    cleanup();

    let answer: (value: unknown) => void = () => {};
    const third = mount([clip('c1', 10, 60)]);
    third.call.mockImplementationOnce(() => new Promise(resolve => { answer = resolve; }) as never);
    fireEvent.click(button('stabilize-run'));
    await settle();
    // Asked and not answered: pressed again, it asks nothing more.
    expect(button('stabilize-run').disabled).toBe(true);
    third.view.unmount();
    expect(third.sent('track.stop')).toEqual([]);
    await act(async () => { answer({ ok: true, result: { job: 'track-2', first: 0, last: 59, frame_size: [1280, 720], method: 'perspective' } }); });
    await settle();
    expect(third.sent('track.stop')).toEqual([{ job: 'track-2' }]);
    expect(third.sent('track.stabilize')).toHaveLength(1);
  });

  it('takes the stabilizer away with Reset, from the clips that have one', async () => {
    const { applied } = mount([clip('c1', 10, 60, { stabilize: measured() }), clip('c2', 100, 60)]);
    fireEvent.click(button('stabilize-reset'));
    await settle();
    expect(applied()).toEqual([{ revision: 17, ops: [set(null)] }]);
  });

  it('shows what several clips share, sets each of them, and measures one clip at a time', async () => {
    const { applied, sent } = mount([clip('c1', 10, 60, { stabilize: measured({ camera_lock: true }) }), clip('c2', 100, 60)]);
    // They differ in Method and Camera Lock: a dash, and a mark.
    expect(methodOf().value).toBe('');
    expect(row('stabilize-method').getAttribute('data-mixed')).toBe('true');
    expect([check('stabilize-camera-lock').indeterminate, row('stabilize-camera-lock').getAttribute('data-mixed')]).toEqual([true, 'true']);
    // Camera Lock is not on for all of them: the Cropping Ratio and Smooth stay on.
    expect(box('stabilize-smooth').disabled).toBe(false);
    expect([check('stabilize-zoom').checked, box('stabilize-smooth').value]).toEqual([true, '0.250']);
    expect(button('stabilize-run').disabled).toBe(true);
    expect(byTest('stabilize-one')!.textContent).toBe(said.one);
    fireEvent.click(button('stabilize-run'));
    await settle();
    expect(sent('track.stabilize')).toEqual([]);
    type(box('stabilize-smooth'), '0.5');
    await settle();
    expect(applied()).toEqual([{ revision: 17, ops: [settings({ smooth: 0.5 }), settings({ smooth: 0.5 }, 'c2')] }]);
    // Camera Lock on for all of them: a clip that has it on already is left out.
    fireEvent.click(check('stabilize-camera-lock'));
    await settle();
    expect(applied()[1]).toEqual({ revision: 17, ops: [settings({ camera_lock: true }, 'c2')] });
  });

  it('never sends what was measured, however long, and sends a slider once, as it is let go, one change for each clip', async () => {
    // A stabilizer measured over 2000 frames: its analysis is tens of thousands of bytes of JSON.
    const path: Record<string, StabilizeMotion> = {};
    for (let frame = 0; frame < 2000; frame++) path[frame] = [frame * 0.5, frame * -0.25, 1.001, 0.002, 1, 0, 0, 0];
    const long = measured({ analysis: { method: 'similarity', size: [1920, 1080], path } });
    expect(JSON.stringify(long).length).toBeGreaterThan(50_000);
    const { applied } = mount([clip('c1', 10, 60, { stabilize: long }), clip('c2', 100, 60)]);
    type(box('stabilize-smooth'), '0.6');
    await settle();
    fireEvent.change(methodOf(), { target: { value: 'translation' } });
    await settle();
    fireEvent.click(check('stabilize-zoom'));
    await settle();
    // The Strength slider dragged: shown as it moves, and nothing sent until it is let go.
    fireEvent.pointerDown(slider('stabilize-strength'));
    fireEvent.input(slider('stabilize-strength'), { target: { value: '0.25' } });
    fireEvent.input(slider('stabilize-strength'), { target: { value: '-0.25' } });
    expect(box('stabilize-strength').value).toBe('-0.250');
    await settle();
    expect(applied()).toHaveLength(3);
    fireEvent.change(slider('stabilize-strength'), { target: { value: '-0.5' } });
    await settle();
    expect(applied()).toEqual([
      { revision: 17, ops: [settings({ smooth: 0.6 }), settings({ smooth: 0.6 }, 'c2')] },
      { revision: 17, ops: [settings({ method: 'translation' }), settings({ method: 'translation' }, 'c2')] },
      { revision: 17, ops: [settings({ zoom: false }), settings({ zoom: false }, 'c2')] },
      { revision: 17, ops: [settings({ strength: -0.5 }), settings({ strength: -0.5 }, 'c2')] },
    ]);
    for (const sent of applied()) expect(JSON.stringify(sent).length).toBeLessThan(300);
    expect(JSON.stringify(applied())).not.toContain('analysis');
  });

  it('is not shown for a title or a still, which have no motion to measure, and is off on a locked track', () => {
    mount([title('t1', 200)]);
    const video = [...document.querySelectorAll<HTMLElement>('[role="tab"]')].find(tab => tab.textContent === labels.video)!;
    fireEvent.click(video);
    expect(document.querySelector('[data-field="zoom"]')).not.toBeNull();
    expect(section()).toBeNull();
    cleanup();
    mount([clip('c5', 10, 60, { source: { kind: 'media', media: 'm2', in: { ticks: 0, scale: 25 } } })]);
    expect(document.querySelector('[data-field="zoom"]')).not.toBeNull();
    expect(section()).toBeNull();
    cleanup();
    mount([clip('c1', 10, 60, { stabilize: measured() })], { locked: true });
    expect(section()).not.toBeNull();
    expect([methodOf().disabled, check('stabilize-zoom').disabled, check('stabilize-camera-lock').disabled, box('stabilize-smooth').disabled,
      button('stabilize-run').disabled, button('stabilize-reset').disabled]).toEqual([true, true, true, true, true, true]);
  });
});

describe('the stabilizer in the copy of the project', () => {
  it('is set whole and taken away as the engine writes it, and kept by a copy of its clip', () => {
    const project = { schema: 5, next_id: 9, bins: [], media: [media], timelines: [timelineOf([clip('c1', 10, 60)])], groups: [] } as unknown as Project;
    const clipsOf = (now: Project) => (now.timelines[0].tracks[0] as { clips: Clip[] }).clips;
    const stabilized = applyOp(project, { op: 'set_stabilize', timeline: 't1', clip: 'c1', value: measured() });
    expect(clipsOf(stabilized)[0].stabilize).toStrictEqual(measured());
    expect(clipsOf(project)[0]).not.toHaveProperty('stabilize');
    // Set whole: a stabilizer without an analysis takes the place of the one with it.
    const plain = applyOp(stabilized, { op: 'set_stabilize', timeline: 't1', clip: 'c1', value: { ...DEFAULT_STABILIZE } });
    expect(clipsOf(plain)[0].stabilize).toStrictEqual({ ...DEFAULT_STABILIZE });
    const gone = applyOp(stabilized, { op: 'set_stabilize', timeline: 't1', clip: 'c1', value: null });
    expect(clipsOf(gone)[0]).not.toHaveProperty('stabilize');
    expect(gone).toStrictEqual(project);
    // A copy of the clip keeps it, its analysis with it, as the engine keeps it.
    const copied = applyOp(stabilized, { op: 'insert_clip', timeline: 't1', track: 'v1', clip: { ...clipsOf(stabilized)[0], id: 'c9', start: 100 } });
    expect(clipsOf(copied).find(each => each.id === 'c9')!.stabilize).toStrictEqual(measured());
  });
});
