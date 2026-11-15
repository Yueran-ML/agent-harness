// @vitest-environment jsdom
/**
 * The keys of the style of a title, its size and the place of its anchor (`title.size`, `title.x`,
 * `title.y`): the operations that set them at the playhead with and without keys, that only a title
 * has them, where they are on the timeline and in its keyframe track, the copy of the project that
 * applies them, and the Title tab of the Inspector, which shows a keyed setting as it is at the
 * playhead with the three buttons of the Video tab beside it.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { applyOp, CLIP_KEYED, CopyMismatch, emptyGrade, titleSetting, type Clip, type Keyframe, type Project, type Timeline, type TitleStyle } from '../src/studio/engine';
import {
  CLIP_SETTINGS, addKey, hasKeys, hasSetting, keyFrameOf, keyOps, keyTimes, paramOf, putValue, removeKey, settingAt, settingsOf,
} from '../src/studio/inspector/clipKeys';
import { Inspector } from '../src/studio/inspector/Inspector';
import { copyChosen, keyId, keyMarks, keyRows, keyedSettings, pasteCopied } from '../src/studio/timeline/keyTrack';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const rate = { num: 25, den: 1 };
const key = (frame: number, value: number): Keyframe => ({ frame, value, interp: 'linear' });
const PLAIN: TitleStyle = { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' };
/** c1 from frame 10 for 60 frames, a title: its keys count its own frames, so frame `n` of them is at frame `10 + n`. */
const title = (style: Partial<TitleStyle> = {}, fields: Partial<Clip> = {}) => clip('c1', 10, 60, { source: { kind: 'title', text: 'Hi', style: { ...PLAIN, ...style } }, ...fields });
/** The title with its anchor going down the picture, from a quarter of its height to three quarters. */
const falling = (fields: Partial<Clip> = {}) => title({ y: { keys: [key(0, 0.25), key(40, 0.75)] } }, fields);
const sourceOf = (style: TitleStyle) => ({ kind: 'title', text: 'Hi', style });
const splice = (path: string, at: number, remove: number, keys: Keyframe[]) => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path, at, remove, keys });

describe('the keys of the style of a title', () => {
  it('are of a title only: any other clip has no such setting, and is sent nothing for one', () => {
    const shot = clip('c1', 10, 60);
    expect(CLIP_KEYED.slice(-3)).toEqual(['title.size', 'title.x', 'title.y']);
    expect(CLIP_KEYED.map(titleSetting)).toEqual([...Array<null>(10).fill(null), 'size', 'x', 'y']);
    expect(settingsOf(title())).toEqual(CLIP_SETTINGS);
    expect(settingsOf(title()).slice(-3)).toEqual(['title.size', 'title.x', 'title.y']);
    expect(settingsOf(shot)).toEqual(CLIP_SETTINGS.slice(0, 10));
    expect([hasSetting(title(), 'title.size'), hasSetting(shot, 'title.size'), hasSetting(shot, 'transform.x')]).toEqual([true, false, true]);
    // It is read as a number, which has no keys, and nothing is made for it.
    expect(typeof paramOf(shot, 'title.y')).toBe('number');
    expect(putValue('t1', shot, 'title.size', 90, 20)).toEqual([]);
    expect(addKey('t1', shot, 'title.size', 20)).toEqual([]);
    expect(removeKey('t1', shot, 'title.size', 20)).toEqual([]);
    expect(keyOps('t1', shot, 'title.y', { keys: [key(0, 0.25)] })).toEqual([]);
    expect(keyTimes(shot, { rate }, [media], 'title.y')).toEqual([]);
  });

  it('set a setting of one number with the source whole, and one with keys by the key at the frame', () => {
    const plain = title();
    expect(putValue('t1', plain, 'title.size', 90, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', source: sourceOf({ ...PLAIN, size: 90 }) }]);
    expect(putValue('t1', plain, 'title.x', 0.25, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', source: sourceOf({ ...PLAIN, x: 0.25 }) }]);
    expect(putValue('t1', plain, 'title.size', 70, 20)).toEqual([]);
    const moved = falling();
    expect(settingAt(moved, 'title.y', 20)).toBe(0.5);
    expect(putValue('t1', moved, 'title.y', 0.4, 20)).toEqual([splice('title.y', 1, 0, [key(20, 0.4)])]);
    expect(putValue('t1', moved, 'title.y', 0.5, 40)).toEqual([splice('title.y', 1, 1, [key(40, 0.5)])]);
    expect(putValue('t1', moved, 'title.y', 0.75, 40)).toEqual([]);
    // The keys of the other settings of the style go along whole where a number of it is set.
    const style = (moved.source as { style: TitleStyle }).style;
    expect(putValue('t1', moved, 'title.size', 90, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', source: sourceOf({ ...style, size: 90 }) }]);
  });

  it('add a key of the value that the setting has there, and take one away: the last leaves its value', () => {
    expect(addKey('t1', title(), 'title.size', 20)).toEqual([splice('title.size', 0, 0, [key(20, 70)])]);
    const moved = falling();
    expect(addKey('t1', moved, 'title.y', 20)).toEqual([splice('title.y', 1, 0, [key(20, 0.5)])]);
    expect(addKey('t1', moved, 'title.y', 40)).toEqual([]);
    expect(removeKey('t1', moved, 'title.y', 40)).toEqual([splice('title.y', 1, 1, [])]);
    // A size between two whole numbers stays as it is where its last key goes.
    expect(removeKey('t1', title({ size: { keys: [key(40, 83.5)] } }), 'title.size', 40)).toEqual([{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.size', param: 83.5 }]);
    expect(keyOps('t1', moved, 'title.y', 0.5)).toEqual([{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', param: 0.5 }]);
    expect(keyOps('t1', title(), 'title.x', { keys: [key(0, 0), key(30, 1)] })).toEqual([splice('title.x', 0, 0, [key(0, 0), key(30, 1)])]);
  });

  it('are on the timeline at the frames of the title itself, and listed after the settings of the Video tab', () => {
    expect(keyFrameOf(falling(), { rate }, [media], 30)).toBe(20);
    expect([hasKeys(title()), hasKeys(falling()), hasKeys(title({ size: { keys: [key(0, 70)] } }))]).toEqual([false, true, true]);
    // The title has frames 0 to 59 of its own: a key at 80 is at none of them.
    expect(keyTimes(title({ y: { keys: [key(0, 0.25), key(40, 0.75), key(80, 0.5)] } }), { rate }, [media], 'title.y')).toEqual([10, 50]);
    const both = falling({ transform: { x: { keys: [key(40, 1)] }, y: 0, scale: 1, rotation: 0, opacity: 1 } });
    expect(keyedSettings(both)).toEqual(['transform.x', 'title.y']);
    expect(keyMarks(both, { rate }, [media])).toEqual([{ at: 10, paths: ['title.y'] }, { at: 50, paths: ['transform.x', 'title.y'] }]);
    expect(keyRows(both, { rate }, [media]).map(row => [row.path, row.keys.map(each => each.at)])).toEqual([['transform.x', [50]], ['title.y', [10, 50]]]);
    // Keys copied from the style of a title are pasted into a title, and left out of a clip that is none.
    const copied = copyChosen(both, new Set([keyId('title.y', 0), keyId('transform.x', 40)]));
    expect(copied.map(each => each.path)).toEqual(['transform.x', 'title.y']);
    expect(pasteCopied(title(), copied, 5).changes.map(change => change.path)).toEqual(['transform.x', 'title.y']);
    expect(pasteCopied(clip('c2', 100, 50), copied, 5).changes.map(change => change.path)).toEqual(['transform.x']);
  });

  it('are applied to the copy of the project as the engine applies them, and refused there for a clip that is no title', () => {
    const projectOf = (c1: Clip) => {
      const timeline: Timeline = { id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
        tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [c1], transitions: [] }] };
      return { schema: 5, next_id: 9, bins: [], media: [media], timelines: [timeline], groups: [] } as unknown as Project;
    };
    const c1 = (now: Project) => (now.timelines[0].tracks[0] as { clips: Clip[] }).clips[0];
    const styleOf = (now: Project) => (c1(now).source as { style: TitleStyle }).style;
    const project = projectOf(title());
    const first = applyOp(project, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', at: 0, remove: 0, keys: [key(20, 0.3)] });
    expect(styleOf(first)).toEqual({ ...PLAIN, y: { keys: [key(20, 0.3)] } });
    const second = applyOp(first, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', at: 0, remove: 1, keys: [key(5, 0), key(30, 1)] });
    expect(styleOf(second).y).toEqual({ keys: [key(5, 0), key(30, 1)] });
    const sized = applyOp(second, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.size', param: { keys: [key(0, 70), key(30, 140)] } });
    expect(styleOf(sized)).toEqual({ ...PLAIN, size: { keys: [key(0, 70), key(30, 140)] }, y: { keys: [key(5, 0), key(30, 1)] } });
    expect(styleOf(applyOp(sized, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', param: 0.5 })).y).toBe(0.5);
    // Nothing else of the clip changes, and the project that was given is as it was.
    expect(c1(sized)).toEqual({ ...c1(project), source: { kind: 'title', text: 'Hi', style: styleOf(sized) } });
    expect(styleOf(project)).toEqual(PLAIN);
    // A source set whole brings its keys as they are.
    const keyedStyle: TitleStyle = { ...PLAIN, x: { keys: [key(0, 0), key(30, 1)] } };
    expect(styleOf(applyOp(project, { op: 'set_clip', timeline: 't1', id: 'c1', source: { kind: 'title', text: 'Hi', style: keyedStyle } }))).toEqual(keyedStyle);
    // A clip of media has no setting of a title: the copy has gone astray, as for a path that names no setting.
    const shot = projectOf(clip('c1', 10, 60));
    expect(() => applyOp(shot, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.size', param: 90 })).toThrow(CopyMismatch);
    expect(() => applyOp(shot, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', at: 0, remove: 0, keys: [key(0, 0.5)] })).toThrow(CopyMismatch);
    expect(() => applyOp(project, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.color', param: 1 })).toThrow(CopyMismatch);
    expect(() => applyOp(project, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.y', at: 0, remove: 1, keys: [] })).toThrow(CopyMismatch);
  });
});

type Call = ReturnType<typeof vi.fn>;
function mount(c1: Clip, frame: number) {
  const call: Call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  const onSeek = vi.fn();
  const timeline: Timeline = { id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
    tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [c1], transitions: [] }] };
  render(<Inspector bridge={{ call, onEvent: vi.fn(() => () => {}) } as Pick<EngineBridge, 'call' | 'onEvent'>} session={1} revision={17}
    timeline={timeline} media={[media]} clips={['c1']} transition={null} onSpeed={vi.fn()} frame={frame} onSeek={onSeek} />);
  return { call, onSeek };
}
const box = (name: string) => document.querySelector<HTMLInputElement>(`[data-panel="title"] [data-field="${name}"] input.inspector-number`)!;
const groupOf = (path: string) => document.querySelector<HTMLElement>(`[data-panel="title"] [data-keys="${path}"]`);
const buttons = (path: string) => [...groupOf(path)!.querySelectorAll<HTMLButtonElement>('button')];
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
const type = (input: HTMLElement, value: string) => { fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' }); };

describe('the Title tab of a title whose style has keys', () => {
  it('shows a setting as it is at the playhead, and puts a typed value there as a key, or as the number of a setting without keys', async () => {
    // Frame 20 of the title is under the playhead: its anchor is halfway down the picture, 540 pixels of 1080.
    const { call } = mount(falling(), 30);
    expect([box('size').value, box('x').value, box('y').value]).toEqual(['70', '960.0', '540.0']);
    type(box('y'), '270');
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [splice('title.y', 1, 0, [key(20, 0.25)])] }]);
    // The size has no keys: it is set with the source whole, which keeps the keys of the anchor.
    type(box('size'), '90');
    await settle();
    const style = (falling().source as { style: TitleStyle }).style;
    expect(applied(call)[1]).toEqual({ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', source: sourceOf({ ...style, size: 90 }) }] });
  });

  it('has the three buttons of the Video tab beside each of the three settings: a key put at the playhead, taken away, and gone to', async () => {
    const between = mount(falling(), 30);
    expect(['title.size', 'title.x', 'title.y'].map(path => [groupOf(path)!.getAttribute('aria-label'), buttons(path).length])).toEqual([['Size', 3], ['Position X', 3], ['Position Y', 3]]);
    const [before, diamond, after] = buttons('title.y');
    expect([before.title, diamond.title, after.title]).toEqual(['Previous keyframe', 'Add keyframe', 'Next keyframe']);
    expect([diamond.className, diamond.querySelector('svg.key-diamond') !== null, diamond.getAttribute('aria-pressed')]).toEqual(['icon-btn', true, 'false']);
    expect([diamond.hasAttribute('data-keyed'), diamond.hasAttribute('data-hit')]).toEqual([true, false]);
    fireEvent.click(diamond); await settle();
    expect(applied(between.call)).toEqual([{ revision: 17, ops: [splice('title.y', 1, 0, [key(20, 0.5)])] }]);
    fireEvent.click(before); fireEvent.click(after);
    expect(between.onSeek.mock.calls).toEqual([[10, false], [50, false]]);
    // A setting with no keys has nowhere to go; its diamond puts its first key, of the number that it is.
    const [none, first] = buttons('title.size');
    expect(none.disabled).toBe(true);
    expect([first.hasAttribute('data-keyed'), first.getAttribute('aria-pressed')]).toEqual([false, 'false']);
    fireEvent.click(first); await settle();
    expect(applied(between.call)[1]).toEqual({ revision: 17, ops: [splice('title.size', 0, 0, [key(20, 70)])] });
    cleanup();
    // On a key, the diamond is lit and takes it away; the last key of a setting leaves its value as one number.
    const on = mount(falling(), 50);
    const [, lit, next] = buttons('title.y');
    expect([lit.getAttribute('aria-pressed'), lit.hasAttribute('data-hit'), lit.title, next.disabled]).toEqual(['true', true, 'Remove keyframe', true]);
    fireEvent.click(lit); await settle();
    expect(applied(on.call)).toEqual([{ revision: 17, ops: [splice('title.y', 1, 1, [])] }]);
    cleanup();
    const last = mount(title({ size: { keys: [key(20, 83.5)] } }), 30);
    expect(box('size').value).toBe('84');
    fireEvent.click(buttons('title.size')[1]); await settle();
    expect(applied(last.call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.size', param: 83.5 }] }]);
  });

  it('is of a title only: a clip of media has no such buttons, and the Video tab of a title keeps those of its transform', () => {
    mount(clip('c1', 10, 60), 30);
    expect(document.querySelector('[data-keys^="title."]')).toBeNull();
    expect(document.querySelector('[data-keys="transform.x"]')).not.toBeNull();
    cleanup();
    mount(falling(), 30);
    expect(document.querySelectorAll('[data-keys^="title."]')).toHaveLength(3);
    fireEvent.click([...document.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(tab => tab.textContent === 'Video')!);
    expect(document.querySelector('[data-keys^="title."]')).toBeNull();
    expect(document.querySelectorAll('[data-keys^="transform."]')).toHaveLength(5);
  });
});
