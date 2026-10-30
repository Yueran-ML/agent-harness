// @vitest-environment jsdom
/**
 * The effects library, and what the timeline does with an effect dropped on it: a transition at the
 * cut it is dropped on, aligned by the side of the cut; a title or a solid colour overwriting 5
 * seconds of a video track; nothing elsewhere. And a transition chosen on the timeline, for the inspector.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { ClipTrack, Timeline as TimelineModel } from '../src/studio/engine';
import { Library } from '../src/studio/effects/Library';
import { EFFECT_DRAG, effectOfTypes, type Effect } from '../src/studio/effects/drag';
import { effectTarget } from '../src/studio/effects/drop';
import { HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { Timeline } from '../src/studio/timeline/Timeline';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, width: number, height: number): DOMRect => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('DragEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.dataset.clip) {
      // The style sheet places a clip by its frames and the pixels of a frame of the timeline.
      const scale = Number(this.closest<HTMLElement>('.timeline')?.dataset.scale ?? 4);
      return rect(HEADER_WIDTH + Number(this.style.getPropertyValue('--start')) * scale, 34, Number(this.style.getPropertyValue('--length')) * scale, 64);
    }
    if (this.dataset.track) return rect(0, 30, 956, 72);
    return rect(0, 0, 956, 400);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); });

const make = (videoTransitions: ClipTrack['transitions'] = [], locked = false): TimelineModel => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000,
  tracks: [
    { id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips: [clip('c1', 10, 60), clip('c2', 70, 60), clip('c3', 160, 40)], transitions: videoTransitions },
    { id: 'a1', kind: 'audio', name: 'A1', enabled: true, locked: false, clips: [clip('a1', 10, 60), clip('a2', 70, 60)], transitions: [] },
  ],
});
function mount(t = make()) {
  const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  const onTransition = vi.fn();
  render(<Timeline timeline={t} media={[media]} bridge={{ call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>} session={1} revision={17}
    frame={0} playing={false} onSeek={() => {}} onTransition={onTransition} />);
  return { call, onTransition };
}
const lane = (track = 'v1') => document.querySelector<HTMLElement>(`[data-track="${track}"] .timeline-lane`)!;
const dragged = (effect: Effect, session = 1) => ({
  types: [EFFECT_DRAG, `${EFFECT_DRAG}-${effect.replace('_', '-')}`], dropEffect: 'none',
  getData: (type: string) => (type === EFFECT_DRAG ? JSON.stringify({ session, effect }) : ''),
});
const at = (frame: number) => ({ clientX: HEADER_WIDTH + frame * 4, clientY: 60 });
/** Drag an effect over a track and drop it: whether it was taken while over it, and what was sent. */
async function drop(call: ReturnType<typeof vi.fn>, effect: Effect, frame: number, track = 'v1', session = 1) {
  const dataTransfer = dragged(effect, session);
  fireEvent.dragOver(lane(track), { ...at(frame), dataTransfer });
  const taken = dataTransfer.dropEffect === 'copy';
  fireEvent.drop(lane(track), { ...at(frame), dataTransfer });
  await settle();
  const sent = call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
  call.mockClear();
  return { taken, sent };
}

describe('the effects library', () => {
  it('lists the transitions, a title and a solid colour, each dragged with its kind in the types of the drag', () => {
    render(<Library session={4} />);
    expect(screen.getByRole('region', { name: 'Effects library' })).toBeDefined();
    const items = [...document.querySelectorAll<HTMLElement>('[data-effect]')];
    expect(items.map(item => [item.dataset.effect, item.textContent])).toEqual([
      ['dissolve', 'Dissolve'], ['dip_black', 'Dip to black'], ['dip_white', 'Dip to white'], ['crossfade', 'Crossfade'], ['title', 'Text'], ['color', 'Solid color'],
    ]);
    const data: Record<string, string> = {};
    const dataTransfer = { setData: (type: string, value: string) => { data[type] = value; }, effectAllowed: 'none' };
    fireEvent.dragStart(items[1], { dataTransfer });
    expect(JSON.parse(data[EFFECT_DRAG])).toEqual({ session: 4, effect: 'dip_black' });
    expect(effectOfTypes(Object.keys(data))).toBe('dip_black');
    expect(dataTransfer.effectAllowed).toBe('copy');
  });
});

describe('an effect dropped on the timeline', () => {
  it('adds a transition at the cut, centred on it, ending on it or starting on it by the side it is dropped on', async () => {
    const { call } = mount();
    const add = (align: string) => [{ revision: 17, ops: [{ edit: 'add_transition', timeline: 't1', left: 'c1', right: 'c2', kind: 'dissolve', length: 25, align }] }];
    expect(await drop(call, 'dissolve', 70)).toEqual({ taken: true, sent: add('center') });
    expect(await drop(call, 'dissolve', 69)).toEqual({ taken: true, sent: add('center') });
    expect(await drop(call, 'dissolve', 68)).toEqual({ taken: true, sent: add('end') });
    expect(await drop(call, 'dissolve', 72)).toEqual({ taken: true, sent: add('start') });
    expect((await drop(call, 'crossfade', 70, 'a1')).sent).toEqual([{ revision: 17, ops: [{ edit: 'add_transition', timeline: 't1', left: 'a1', right: 'a2', kind: 'crossfade', length: 25, align: 'center' }] }]);
  });

  it('changes the kind of a transition that is at the cut already, as one dropped on another replaces it', async () => {
    const { call } = mount(make([{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 10, align: 'center' }]));
    expect((await drop(call, 'dip_white', 70)).sent).toEqual([{ revision: 17, ops: [{ edit: 'set_transition', timeline: 't1', id: 'x1', kind: 'dip_white' }] }]);
  });

  it('puts a transition dropped on one of its own kind back to the standard length, and sends nothing where it has that', async () => {
    const short = mount(make([{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 10, align: 'center' }]));
    expect((await drop(short.call, 'dissolve', 70)).sent).toEqual([{ revision: 17, ops: [{ edit: 'set_transition', timeline: 't1', id: 'x1', length: 25 }] }]);
    cleanup();
    const standard = mount(make([{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 25, align: 'center' }]));
    expect((await drop(standard.call, 'dissolve', 70)).sent).toEqual([]);
  });

  it('takes no transition away from a cut, on a track of the other kind, on a locked track, or from another session', async () => {
    const { call } = mount();
    expect(await drop(call, 'dissolve', 90)).toEqual({ taken: false, sent: [] });
    // c2 and c3 do not meet: there is no cut at 130.
    expect(await drop(call, 'dissolve', 130)).toEqual({ taken: false, sent: [] });
    expect(await drop(call, 'crossfade', 70)).toEqual({ taken: false, sent: [] });
    expect(await drop(call, 'dissolve', 70, 'a1')).toEqual({ taken: false, sent: [] });
    expect((await drop(call, 'dissolve', 70, 'v1', 2)).sent).toEqual([]);
    cleanup();
    const locked = mount(make([], true));
    expect(await drop(locked.call, 'dissolve', 70)).toEqual({ taken: false, sent: [] });
  });

  it('overwrites 5 seconds of a video track with a title or a solid colour, over clips or not, in one transaction', async () => {
    const { call } = mount();
    const title = { kind: 'title', text: 'Title', style: { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } };
    expect(await drop(call, 'title', 250)).toEqual({ taken: true, sent: [{ revision: 17, ops: [
      { edit: 'overwrite', timeline: 't1', source: { ...title, length: 125 }, at: 250, video_track: 'v1', audio_track: null }] }] });
    // Before the next clip, it runs over it: the standard duration, not the room that is left.
    expect((await drop(call, 'color', 140)).sent).toEqual([{ revision: 17, ops: [
      { edit: 'overwrite', timeline: 't1', source: { kind: 'color', color: '#000000ff', length: 125 }, at: 140, video_track: 'v1', audio_track: null }] }]);
    // Dropped on clips, as in DaVinci Resolve: it overwrites them from the frame it is dropped at.
    expect(await drop(call, 'title', 30)).toEqual({ taken: true, sent: [{ revision: 17, ops: [
      { edit: 'overwrite', timeline: 't1', source: { ...title, length: 125 }, at: 30, video_track: 'v1', audio_track: null }] }] });
  });

  it('shows while it is dragged where it goes and what of the clips it overwrites, and nothing once dropped or gone', async () => {
    mount();
    const covered = () => [...document.querySelectorAll<HTMLElement>('[data-covered]')].map(each => [each.dataset.covered, each.style.left, each.style.width]);
    const footprint = () => document.querySelector<HTMLElement>('[data-preview="effect"]');
    const dataTransfer = dragged('title');
    fireEvent.dragOver(lane(), { ...at(30), dataTransfer });
    expect([footprint()!.style.left, footprint()!.style.width]).toEqual([`${HEADER_WIDTH + 30 * 4}px`, `${125 * 4}px`]);
    expect(covered()).toEqual([['c1', `${HEADER_WIDTH + 30 * 4}px`, `${40 * 4}px`], ['c2', `${HEADER_WIDTH + 70 * 4}px`, `${60 * 4}px`]]);
    // Moved on: it goes elsewhere, and overwrites only the end of c3 there.
    fireEvent.dragOver(lane(), { ...at(180), dataTransfer });
    expect(covered()).toEqual([['c3', `${HEADER_WIDTH + 180 * 4}px`, `${20 * 4}px`]]);
    fireEvent.dragLeave(lane(), { ...at(180), dataTransfer, relatedTarget: document.body });
    expect(footprint()).toBeNull();
    expect(covered()).toEqual([]);
    fireEvent.dragOver(lane(), { ...at(250), dataTransfer });
    expect(footprint()).not.toBeNull();
    expect(covered()).toEqual([]);
    // Over a track it cannot go on, nothing is shown.
    fireEvent.dragOver(lane('a1'), { ...at(30), dataTransfer });
    expect(footprint()).toBeNull();
    fireEvent.dragOver(lane(), { ...at(30), dataTransfer });
    fireEvent.drop(lane(), { ...at(30), dataTransfer });
    await settle();
    expect(footprint()).toBeNull();
    expect(covered()).toEqual([]);
  });

  it('takes no title or solid colour on an audio track, on a locked track, or from another session', async () => {
    const { call } = mount();
    expect(await drop(call, 'title', 30, 'a1')).toEqual({ taken: false, sent: [] });
    expect(await drop(call, 'color', 250, 'a1')).toEqual({ taken: false, sent: [] });
    expect((await drop(call, 'title', 30, 'v1', 2)).sent).toEqual([]);
    expect(document.querySelector('[data-preview="effect"]')).toBeNull();
    cleanup();
    const locked = mount(make([], true));
    expect(await drop(locked.call, 'title', 30)).toEqual({ taken: false, sent: [] });
    expect(await drop(locked.call, 'color', 250)).toEqual({ taken: false, sent: [] });
    expect(document.querySelectorAll('[data-covered]')).toHaveLength(0);
  });

  it('says why the engine refused a title it overwrote with', async () => {
    const { call } = mount();
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'refused: locked', data: { reason: 'locked' } } });
    await drop(call, 'title', 30);
    expect(screen.getByRole('alert').textContent).toContain('A track affected by this edit is locked.');
  });

  it('finds its place without the drop, from the types of the drag and the pointer alone', () => {
    const t = make();
    expect(effectTarget(t, 'dip_black', 'v1', 70.5, 70, 4)).toEqual({ kind: 'cut', track: 'v1', left: 'c1', right: 'c2', cut: 70, align: 'center', existing: null });
    expect(effectTarget(t, 'dip_black', 'v1', 70.5, 70, 0.5)).toEqual({ kind: 'cut', track: 'v1', left: 'c1', right: 'c2', cut: 70, align: 'center', existing: null });
    expect(effectTarget(t, 'title', 'v1', 0, 0, 4)).toEqual({ kind: 'place', track: 'v1', at: 0, length: 125, covered: [
      { id: 'c1', track: 'v1', start: 10, length: 60 }, { id: 'c2', track: 'v1', start: 70, length: 55 }] });
    expect(effectTarget(t, 'color', 'v1', 70, 70, 4)).toEqual({ kind: 'place', track: 'v1', at: 70, length: 125, covered: [
      { id: 'c2', track: 'v1', start: 70, length: 60 }, { id: 'c3', track: 'v1', start: 160, length: 35 }] });
    expect(effectTarget(t, 'title', 'v1', -3, -3, 4)).toMatchObject({ at: 0 });
    expect(effectTarget(t, 'title', 'a1', 0, 0, 4)).toBeNull();
    expect(effectTarget(make([], true), 'color', 'v1', 0, 0, 4)).toBeNull();
  });
});

describe('a transition chosen on the timeline', () => {
  it('is told to the inspector, shown chosen, and taken away with Delete', async () => {
    const { call, onTransition } = mount(make([{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 10, align: 'center' }]));
    const transition = document.querySelector<HTMLElement>('[data-transition="x1"]')!;
    fireEvent.pointerDown(transition, { button: 0, pointerId: 1, ...at(70) });
    fireEvent.pointerUp(transition, { button: 0, pointerId: 1, ...at(70) });
    expect(onTransition).toHaveBeenLastCalledWith('x1');
    expect(transition.dataset.selected).toBe('true');
    fireEvent.keyDown(screen.getByRole('region', { name: 'Timeline' }), { key: 'Delete' });
    await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'remove_transition', timeline: 't1', id: 'x1' }] });
    // A clip chosen leaves the transition.
    fireEvent.pointerDown(document.querySelector('[data-clip="c3"]')!, { button: 0, pointerId: 2, ...at(170) });
    fireEvent.pointerUp(screen.getByRole('region', { name: 'Timeline' }), { button: 0, pointerId: 2, ...at(170) });
    expect(onTransition).toHaveBeenLastCalledWith(null);
    expect(transition.dataset.selected).toBe('false');
  });
});
