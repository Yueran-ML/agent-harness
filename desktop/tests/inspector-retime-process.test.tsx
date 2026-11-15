// @vitest-environment jsdom
/**
 * The Retime Process of the Video tab of the Inspector (Retime and Scaling in DaVinci Resolve): the
 * select that shows what the chosen clips of media show between two pictures of their media, the
 * one transaction that it sends for them, and the copy of the project that keeps the setting as
 * the engine writes it.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { en } from '../src/messages/en';
import { applyOp, emptyGrade, RETIME_PROCESSES, type Clip, type Media, type Project, type Timeline } from '../src/studio/engine';
import { Inspector } from '../src/studio/inspector/Inspector';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const rate = { num: 25, den: 1 };
const title = (id: string, start: number): Clip => clip(id, start, 50, {
  source: { kind: 'title', text: 'Hello', style: { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } },
});
/** A picture that does not move: one frame, held for as long as its clip lasts. */
const still: Media = { ...media, id: 'm2', name: 'Poster', facts: { duration: media.facts.duration, video: { ...media.facts.video!, still: true } } };
const timelineOf = (clips: Clip[]): Timeline => ({
  id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips, transitions: [] }],
});

type Call = ReturnType<typeof vi.fn>;
function mount(clips: Clip[], chosen: string[] = clips.map(each => each.id)) {
  const call: Call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  render(<Inspector bridge={{ call, onEvent: vi.fn(() => () => {}) } as Pick<EngineBridge, 'call' | 'onEvent'>} session={1} revision={17}
    timeline={timelineOf(clips)} media={[media, still]} clips={chosen} transition={null} onSpeed={vi.fn()} />);
  return { call };
}
const select = () => document.querySelector<HTMLSelectElement>('[data-field="retime-process"] select');
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
const set = (id: string, retime_process: string) => ({ op: 'set_clip', timeline: 't1', id, retime_process });

describe('the Retime Process of the Video tab', () => {
  it('shows Nearest for a clip that says nothing of it, and sets Frame Blend as it is chosen', async () => {
    const { call } = mount([clip('c1', 10, 60)]);
    const process = select()!;
    expect(process.value).toBe('nearest');
    expect([...process.options].map(option => [option.value, option.textContent])).toEqual([['nearest', 'Nearest'], ['frame_blend', 'Frame Blend']]);
    expect([...process.options].map(option => option.textContent)).toEqual(RETIME_PROCESSES.map(each => en.label.studio.inspector.retimeProcesses[each]));
    // Under the heading that DaVinci Resolve gives it.
    const headings = [...document.querySelectorAll('h3.inspector-section')].map(heading => heading.textContent);
    expect(headings).toContain(en.label.studio.inspector.retimeAndScaling);
    fireEvent.change(process, { target: { value: 'frame_blend' } });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [set('c1', 'frame_blend')] }]);
  });

  it('shows what a clip is set to, and sets Nearest by its name', async () => {
    const { call } = mount([clip('c1', 10, 60, { retime_process: 'frame_blend' })]);
    expect(select()!.value).toBe('frame_blend');
    fireEvent.change(select()!, { target: { value: 'nearest' } });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [set('c1', 'nearest')] }]);
  });

  it('sets every chosen clip of media that is not so already, in one transaction, and no title', async () => {
    const clips = [clip('c1', 10, 60), clip('c2', 70, 60, { retime_process: 'frame_blend' }), clip('c3', 130, 60), title('t1', 200)];
    const { call } = mount(clips);
    // The clips of media differ: the select shows that, and offers both.
    const process = select()!;
    expect(process.value).toBe('');
    expect(process.closest('[data-field]')!.getAttribute('data-mixed')).toBe('true');
    fireEvent.change(process, { target: { value: 'frame_blend' } });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [set('c1', 'frame_blend'), set('c3', 'frame_blend')] }]);
    cleanup();
    const back = mount(clips);
    fireEvent.change(select()!, { target: { value: 'nearest' } });
    await settle();
    expect(applied(back.call)).toEqual([{ revision: 17, ops: [set('c2', 'nearest')] }]);
  });

  it('is not shown for a title or a still, which show one picture however they are played', async () => {
    mount([title('t1', 200)]);
    // A title opens on its own tab: its picture is set on the Video tab beside it.
    const video = [...document.querySelectorAll<HTMLElement>('[role="tab"]')].find(tab => tab.textContent === en.label.studio.inspector.video)!;
    fireEvent.click(video);
    expect(document.querySelector('[data-field="zoom"]')).not.toBeNull();
    expect(select()).toBeNull();
    cleanup();
    const poster = clip('c5', 10, 60, { source: { kind: 'media', media: 'm2', in: { ticks: 0, scale: 25 } } });
    mount([poster]);
    expect(document.querySelector('[data-field="zoom"]')).not.toBeNull();
    expect(select()).toBeNull();
    cleanup();
    // Beside a clip of media that moves, it is of that clip alone.
    const beside = mount([poster, clip('c1', 100, 60)]);
    expect(select()!.value).toBe('nearest');
    fireEvent.change(select()!, { target: { value: 'frame_blend' } });
    await settle();
    expect(applied(beside.call)).toEqual([{ revision: 17, ops: [set('c1', 'frame_blend')] }]);
  });

  it('is off for clips on a locked track', () => {
    const locked = timelineOf([clip('c1', 10, 60)]);
    locked.tracks[0].locked = true;
    render(<Inspector bridge={{ call: vi.fn(), onEvent: vi.fn(() => () => {}) } as Pick<EngineBridge, 'call' | 'onEvent'>} session={1} revision={17}
      timeline={locked} media={[media]} clips={['c1']} transition={null} onSpeed={vi.fn()} />);
    expect(select()!.disabled).toBe(true);
  });
});

describe('the Retime Process in the copy of the project', () => {
  it('is kept as the engine writes it: Frame Blend by its name, and Nearest by no field at all', () => {
    const project = { schema: 5, next_id: 9, bins: [], media: [media], timelines: [timelineOf([clip('c1', 10, 60)])], groups: [] } as unknown as Project;
    const first = (now: Project) => (now.timelines[0].tracks[0] as { clips: Clip[] }).clips[0];
    const blended = applyOp(project, { op: 'set_clip', timeline: 't1', id: 'c1', retime_process: 'frame_blend' });
    expect(first(blended).retime_process).toBe('frame_blend');
    expect(first(project)).not.toHaveProperty('retime_process');
    const nearest = applyOp(blended, { op: 'set_clip', timeline: 't1', id: 'c1', retime_process: 'nearest' });
    expect(first(nearest)).not.toHaveProperty('retime_process');
    expect(nearest).toStrictEqual(project);
    // An operation that sets something else leaves it as it is.
    const faded = applyOp(blended, { op: 'set_clip', timeline: 't1', id: 'c1', fade_in: 5 });
    expect(first(faded)).toMatchObject({ retime_process: 'frame_blend', fade_in: 5 });
  });
});
