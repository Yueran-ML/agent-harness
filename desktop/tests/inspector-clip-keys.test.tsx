// @vitest-environment jsdom
/**
 * The keys of a clip's transform in the Inspector of the Edit page: the operations that set a
 * setting at the playhead with and without keys, where the keys are on the timeline, and the Video
 * tab that shows a keyed setting as it is at the playhead, with the three buttons beside it.
 */
import { readFileSync } from 'node:fs';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { applyOp, emptyGrade, type Clip, type Keyframe, type Project, type Timeline } from '../src/studio/engine';
import { addKey, keyFrameOf, keyTimes, putValue, removeKey, settingAt } from '../src/studio/inspector/clipKeys';
import { Inspector } from '../src/studio/inspector/Inspector';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const key = (frame: number, value: number): Keyframe => ({ frame, value, interp: 'linear' });
const rate = { num: 25, den: 1 };
/** c1 from frame 10 for 60 frames, showing its media from its first picture: the picture `n` at frame `10 + n`. */
const keyed = (keys: Keyframe[], fields: Partial<Clip> = {}) => clip('c1', 10, 60, { transform: { ...NONE, x: { keys } }, ...fields });
const splice = (at: number, remove: number, keys: Keyframe[]) => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', at, remove, keys });

describe('the keys of a transform', () => {
  it('are at the frame of the picture under the playhead, which is kept within the clip', () => {
    const c1 = clip('c1', 10, 60);
    expect([30, 10, 0, 69, 500].map(frame => keyFrameOf(c1, { rate }, [media], frame))).toEqual([20, 0, 0, 59, 59]);
    // A playhead between two frames is in the frame that is shown.
    expect(keyFrameOf(c1, { rate }, [media], 30.5)).toBe(20);
    // A title counts its own frames.
    const title = clip('t1', 200, 50, { source: { kind: 'title', text: 'Hi', style: { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } } });
    expect(keyFrameOf(title, { rate }, [media], 215)).toBe(15);
  });

  it('set a setting of one number with its transform whole, and one with keys by the key at the frame', () => {
    const plain = clip('c1', 10, 60);
    expect(putValue('t1', plain, 'transform.x', 0.25, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', transform: { ...NONE, x: 0.25 } }]);
    expect(putValue('t1', plain, 'transform.x', 0, 20)).toEqual([]);
    const moved = keyed([key(0, 0), key(40, 1)]);
    // The key there is changed, and keeps how it goes on.
    const held = keyed([key(0, 0), { frame: 40, value: 1, interp: 'hold' }]);
    expect(putValue('t1', held, 'transform.x', 0.5, 40)).toEqual([splice(1, 1, [{ frame: 40, value: 0.5, interp: 'hold' }])]);
    expect(putValue('t1', moved, 'transform.x', 1, 40)).toEqual([]);
    // Between keys, before the first and after the last: a key is put in its place among them.
    expect(putValue('t1', moved, 'transform.x', 0.5, 20)).toEqual([splice(1, 0, [key(20, 0.5)])]);
    expect(putValue('t1', keyed([key(10, 0), key(40, 1)]), 'transform.x', 0.5, 5)).toEqual([splice(0, 0, [key(5, 0.5)])]);
    expect(putValue('t1', moved, 'transform.x', 0.5, 50)).toEqual([splice(2, 0, [key(50, 0.5)])]);
    // The other settings of a keyed transform go along whole where a number of it is set.
    expect(putValue('t1', moved, 'transform.scale', 2, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', transform: { ...moved.transform, scale: 2 } }]);
  });

  it('add a key of the value that the setting has there, and take one away: the last leaves its value', () => {
    const plain = clip('c1', 10, 60, { transform: { ...NONE, x: 0.3 } });
    expect(addKey('t1', plain, 'transform.x', 20)).toEqual([splice(0, 0, [key(20, 0.3)])]);
    const moved = keyed([key(0, 0), key(40, 1)]);
    expect(addKey('t1', moved, 'transform.x', 20)).toEqual([splice(1, 0, [key(20, 0.5)])]);
    expect(addKey('t1', moved, 'transform.x', 40)).toEqual([]);
    expect(removeKey('t1', moved, 'transform.x', 40)).toEqual([splice(1, 1, [])]);
    expect(removeKey('t1', moved, 'transform.x', 20)).toEqual([]);
    expect(removeKey('t1', keyed([key(40, 0.75)]), 'transform.x', 40)).toEqual([{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', param: 0.75 }]);
    expect(removeKey('t1', plain, 'transform.x', 20)).toEqual([]);
    expect(settingAt(moved, 'transform.x', 30)).toBe(0.75);
  });

  it('are found on the timeline where their pictures are shown, through speed and direction', () => {
    // Pictures 0 to 59 are shown: the key at 80 is of none of them.
    expect(keyTimes(keyed([key(0, 0), key(40, 1), key(80, 0)]), { rate }, [media], 'transform.x')).toEqual([10, 50]);
    expect(keyTimes(clip('c1', 10, 60), { rate }, [media], 'transform.x')).toEqual([]);
    // Twice as fast, the picture 40 comes at frame 30, and 41, which is not shown, with the one after it.
    const fast = keyed([key(0, 0), key(40, 1), key(41, 0)], { length: 30, speed: { num: 2, den: 1 } });
    expect(keyTimes(fast, { rate }, [media], 'transform.x')).toEqual([10, 30, 31]);
    // Backward, the first picture is shown last.
    const back = keyed([key(0, 0), key(40, 1)], { reverse: true });
    expect(keyTimes(back, { rate }, [media], 'transform.x')).toEqual([29, 69]);
  });

  it('set a side of the crop with the crop whole, and by its keys once it has them', () => {
    const none = { left: 0, right: 0, top: 0, bottom: 0, softness: 0 };
    // A clip that says nothing of its crop has none.
    const plain = clip('c1', 10, 60);
    expect(putValue('t1', plain, 'crop.left', 0.25, 20)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', crop: { ...none, left: 0.25 } }]);
    expect(putValue('t1', plain, 'crop.softness', 0, 20)).toEqual([]);
    const opened = clip('c1', 10, 60, { crop: { ...none, top: 0.1, left: { keys: [key(0, 1), key(20, 0)] } } });
    expect(settingAt(opened, 'crop.left', 10)).toBe(0.5);
    expect(putValue('t1', opened, 'crop.left', 0.25, 10)).toEqual([{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'crop.left', at: 1, remove: 0, keys: [key(10, 0.25)] }]);
    expect(putValue('t1', opened, 'crop.right', 0.5, 10)).toEqual([{ op: 'set_clip', timeline: 't1', id: 'c1', crop: { ...opened.crop, right: 0.5 } }]);
    expect(keyTimes(opened, { rate }, [media], 'crop.left')).toEqual([10, 30]);
  });

  it('are applied to the copy of the project as the engine applies them', () => {
    const timeline: Timeline = { id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
      tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked: false, clips: [clip('c1', 10, 60)], transitions: [] }] };
    const project = { schema: 5, next_id: 9, bins: [], media: [media], timelines: [timeline], groups: [] } as unknown as Project;
    const x = (now: Project) => (now.timelines[0].tracks[0] as { clips: Clip[] }).clips[0].transform.x;
    const first = applyOp(project, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', at: 0, remove: 0, keys: [key(20, 0.3)] });
    expect(x(first)).toEqual({ keys: [key(20, 0.3)] });
    const second = applyOp(first, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', at: 0, remove: 1, keys: [key(5, 0), key(30, 1)] });
    expect(x(second)).toEqual({ keys: [key(5, 0), key(30, 1)] });
    expect(x(applyOp(second, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', param: 0.5 }))).toBe(0.5);
    expect(x(project)).toBe(0);
    // The first keys of a side of the crop, of a clip that said nothing of its crop.
    const cropped = applyOp(project, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'crop.left', at: 0, remove: 0, keys: [key(0, 1)] });
    expect((cropped.timelines[0].tracks[0] as { clips: Clip[] }).clips[0].crop).toEqual({ left: { keys: [key(0, 1)] }, right: 0, top: 0, bottom: 0, softness: 0 });
    const laid = applyOp(project, { op: 'set_clip', timeline: 't1', id: 'c1', composite: 'screen', crop: { left: 0.5, right: 0, top: 0, bottom: 0, softness: 0.1 } });
    expect((laid.timelines[0].tracks[0] as { clips: Clip[] }).clips[0]).toMatchObject({ composite: 'screen', crop: { left: 0.5, softness: 0.1 } });
    // A path that is none of a clip's keyed settings, or keys that are not there, is a copy gone astray.
    expect(() => applyOp(project, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'gain_db', param: 0.5 })).toThrow();
    expect(() => applyOp(project, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', at: 0, remove: 1, keys: [] })).toThrow();
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
const box = (name: string) => document.querySelector<HTMLInputElement>(`[data-field="${name}"] input.inspector-number`)!;
const buttons = (name: string) => [...document.querySelectorAll<HTMLButtonElement>(`[data-keys="${name}"] button`)];
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);

describe('the Video tab of a clip whose transform has keys', () => {
  it('shows a setting as it is at the playhead, and puts a typed value there as a key', async () => {
    // Picture 20 is under the playhead: x is halfway from 0 to a quarter of the width.
    const { call } = mount(keyed([key(0, 0), key(40, 0.25)]), 30);
    expect(box('x').value).toBe('240.0');
    expect(box('y').value).toBe('0.0');
    fireEvent.change(box('x'), { target: { value: '960' } });
    fireEvent.keyDown(box('x'), { key: 'Enter' });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [splice(1, 0, [key(20, 0.5)])] }]);
  });

  it('puts a key at the playhead with the diamond, takes away the one there, and goes from key to key', async () => {
    const between = mount(keyed([key(0, 0), key(40, 0.25)]), 30);
    const [before, diamond, after] = buttons('transform.x');
    expect(diamond.getAttribute('aria-pressed')).toBe('false');
    // Each is a button with a face that is drawn, not a letter of the font: two arrows and the diamond of a key.
    expect([before, diamond, after].map(each => [each.className, each.querySelector('svg') !== null, each.textContent])).toEqual([
      ['icon-btn', true, ''], ['icon-btn', true, ''], ['icon-btn', true, '']]);
    expect(diamond.querySelector('svg.key-diamond')).not.toBeNull();
    expect([before.title, diamond.title, after.title]).toEqual(['Previous keyframe', 'Add keyframe', 'Next keyframe']);
    // The diamond of a setting that has keys says so, though none is at the playhead.
    expect([diamond.hasAttribute('data-keyed'), diamond.hasAttribute('data-hit')]).toEqual([true, false]);
    fireEvent.click(diamond); await settle();
    expect(applied(between.call)).toEqual([{ revision: 17, ops: [splice(1, 0, [key(20, 0.125)])] }]);
    fireEvent.click(before); fireEvent.click(after);
    expect(between.onSeek.mock.calls).toEqual([[10, false], [50, false]]);
    // A setting with no keys has nowhere to go; its diamond puts its first key.
    const [none, first] = buttons('transform.rotation');
    expect(none.disabled).toBe(true);
    expect([first.hasAttribute('data-keyed'), first.hasAttribute('data-hit'), first.getAttribute('aria-pressed')]).toEqual([false, false, 'false']);
    fireEvent.click(first); await settle();
    expect(applied(between.call)[1]).toEqual({ revision: 17, ops: [{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.rotation', at: 0, remove: 0, keys: [key(20, 0)] }] });
    cleanup();
    // On a key, the diamond is lit and takes it away; there is no key after the last.
    const on = mount(keyed([key(0, 0), key(40, 0.25)]), 50);
    const [, lit, next] = buttons('transform.x');
    expect(lit.getAttribute('aria-pressed')).toBe('true');
    expect([lit.hasAttribute('data-keyed'), lit.hasAttribute('data-hit'), lit.title]).toEqual([true, true, 'Remove keyframe']);
    expect(next.disabled).toBe(true);
    // The style sheet draws the three apart: 22px to press and a border at rest, the colour of keys where the setting has keys,
    // and the diamond filled and lit where a key is at the playhead.
    const css = readFileSync('src/studio/Studio.css', 'utf8');
    expect(css).toMatch(/\.audio-param-keys \.icon-btn \{[^}]*width: 22px; height: 22px; border: 1px solid var\(--line-strong\); color: var\(--moon-1\);/);
    expect(css).toMatch(/\.audio-param-keys \.key-diamond \{ fill: none; stroke: currentColor;/);
    expect(css).toMatch(/\.audio-param-keys \.icon-btn\[data-keyed\]:not\(:disabled\) \{ color: var\(--warn\); \}/);
    expect(css).toMatch(/\.audio-param-keys \[data-hit\] \.key-diamond, \.audio-param-keys \[aria-pressed='true'\] \.key-diamond \{ fill: currentColor; \}/);
    fireEvent.click(lit); await settle();
    expect(applied(on.call)).toEqual([{ revision: 17, ops: [splice(1, 1, [])] }]);
  });
});

describe('the cropping and the composite mode of the Video tab', () => {
  it('sets a side of the crop in percent of the picture, and the composite mode as it is chosen', async () => {
    const { call } = mount(clip('c1', 10, 60), 30);
    expect(box('crop-left').value).toBe('0.0');
    fireEvent.change(box('crop-left'), { target: { value: '25' } });
    fireEvent.keyDown(box('crop-left'), { key: 'Enter' });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', crop: { left: 0.25, right: 0, top: 0, bottom: 0, softness: 0 } }] }]);
    const mode = document.querySelector<HTMLSelectElement>('[data-field="composite"] select')!;
    expect(mode.value).toBe('normal');
    expect(mode.options).toHaveLength(27);
    fireEvent.change(mode, { target: { value: 'screen' } });
    await settle();
    expect(applied(call)[1]).toEqual({ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', composite: 'screen' }] });
  });

  it('shows a side that has keys as it is at the playhead, with its diamond lit on a key', () => {
    const none = { left: 0, right: 0, top: 0, bottom: 0, softness: 0 };
    mount(clip('c1', 10, 60, { composite: 'multiply', crop: { ...none, left: { keys: [key(0, 1), key(20, 0)] } } }), 30);
    // Picture 20 is under the playhead: the last key.
    expect(box('crop-left').value).toBe('0.0');
    expect(buttons('crop.left')[1].getAttribute('aria-pressed')).toBe('true');
    expect(buttons('crop.right')[1].getAttribute('aria-pressed')).toBe('false');
    expect(document.querySelector<HTMLSelectElement>('[data-field="composite"] select')!.value).toBe('multiply');
  });
});
