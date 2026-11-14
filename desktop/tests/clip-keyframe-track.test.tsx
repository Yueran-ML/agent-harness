// @vitest-environment jsdom
/**
 * The keys of a clip on the timeline of the Edit page: the diamonds along the lower edge of a clip
 * whose transform or crop has keys (`src/studio/timeline/ClipKeyMarks`), what they say and where a
 * press on one moves the playhead; the keyframe track that the button of the clip opens beneath it
 * (`src/studio/timeline/KeyframeTracks`), its rows, the keys chosen in it, a drag as one gesture,
 * Delete, copy and paste, and the keys of the keyboard that it keeps from the timeline; how it
 * follows the clip as the project changes; and that a timeline of clips without keys mounts none
 * of it.
 *
 * The page is a timeline with what the Edit page gives it: the keyframe tracks that are open
 * (`useKeyTracks`).
 */
import { memo, type FunctionComponent } from 'react';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type ClipKeyed, type Interp, type Keyframe, type Timeline as TimelineModel } from '../src/studio/engine';
import { keyOps, keyTimes, paramOf } from '../src/studio/inspector/clipKeys';
import { copyKeys, deleteKeys, moveKeys, pasteKeys } from '../src/studio/keyframes/curveGeometry';
import { HEADER_WIDTH, RULER_HEIGHT } from '../src/studio/timeline/geometry';
import { KEY_TRACK_RISE, keyTrackHeight } from '../src/studio/timeline/keyTrack';
import { Timeline } from '../src/studio/timeline/Timeline';
import { useKeyTracks } from '../src/studio/timeline/useKeyTracks';
import { clip, media } from './timeline-fixture';
import { bigTimeline, readyBridge } from './timeline-perf-fixture';
import { likeABrowser } from './mounted';

const drawn = vi.hoisted(() => ({ marks: 0, tracks: 0 }));
/** A component of a module, drawn as it is (a memo component stays one) and counted under `name` each time it is drawn. */
async function counted(original: () => Promise<Record<string, unknown>>, exported: string, name: keyof typeof drawn) {
  const real = await original();
  const component = real[exported] as FunctionComponent<object> & { type?: FunctionComponent<object> };
  const inner = component.type ?? component;
  const drawing = (props: object) => { drawn[name]++; return inner(props); };
  return { ...real, [exported]: component.type ? memo(drawing) : drawing };
}
vi.mock('../src/studio/timeline/ClipKeyMarks', original => counted(original, 'ClipKeyMarks', 'marks'));
vi.mock('../src/studio/timeline/KeyframeTracks', original => counted(original, 'KeyframeTracks', 'tracks'));

beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  // The timeline is 956 px wide beside its headers: the clips of its first frames are in the window that it draws.
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956 + HEADER_WIDTH);
  drawn.marks = 0; drawn.tracks = 0;
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); });

const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const WHOLE = { left: 0, right: 0, top: 0, bottom: 0, softness: 0 };
const ANY = { low: -Infinity, high: Infinity };
const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const rate = { num: 25, den: 1 };
const X: ClipKeyed = 'transform.x';
const SCALE: ClipKeyed = 'transform.scale';
const LEFT: ClipKeyed = 'crop.left';
const MEDIA = [media];
/** The pixels of a frame, as the timeline begins. */
const PX = 4;
/** c1 from frame 10 for 60 frames, showing its media from its first picture: the picture `n` at frame `10 + n`. */
const keyed = (keys: Keyframe[], fields: Partial<Clip> = {}) => clip('c1', 10, 60, { transform: { ...NONE, x: { keys } }, ...fields });
const three = [key(0, 0), key(20, 0.25), key(40, 0)];
const places = [key(0, 0), key(40, 1), key(80, 0)];
const zooms = [key(40, 2)];
const lefts = [key(20, 0.5)];
/** Keys of the place at the pictures 0, 40 and 80, of the zoom at 40, and of the left of the crop at 20. */
const several = clip('c1', 10, 60, { transform: { ...NONE, x: { keys: places }, scale: { keys: zooms } }, crop: { ...WHOLE, left: { keys: lefts } } });
const timelineOf = (clips: Clip[], locked = false): TimelineModel => ({
  id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});

type Call = ReturnType<typeof vi.fn>;
interface PageProps {
  model: TimelineModel; bridge: Pick<EngineBridge, 'call' | 'onEvent'>; onSeek: (frame: number, scrubbing: boolean) => void; frame?: number; revision?: number;
}
/** A timeline as the Edit page holds it: the page keeps which keyframe tracks are open. */
function Page({ model, bridge, onSeek, frame = 30, revision = 17 }: PageProps) {
  const keyTracks = useKeyTracks(model, 1);
  return <Timeline timeline={model} media={MEDIA} bridge={bridge} session={1} revision={revision} frame={frame} playing={false} onSeek={onSeek} keyTracks={keyTracks} />;
}
function wire(clips: Clip[], given: { frame?: number; locked?: boolean } = {}) {
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const onSeek = vi.fn();
  const props: PageProps = {
    model: timelineOf(clips, given.locked), bridge: { call, onEvent: vi.fn(() => () => {}) } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>,
    onSeek, frame: given.frame ?? 30,
  };
  const view = render(<Page {...props} />);
  return { ...view, props, call, onSeek };
}
const sent = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply');
const applied = (call: Call) => sent(call).map(([, params]) => params);
const setKeys = (path: ClipKeyed, keys: Keyframe[]) => ({ op: 'set_clip_keys', timeline: 't1', id: 'c1', path, param: { keys } });
const splice = (path: ClipKeyed, at: number, remove: number, keys: Keyframe[]) => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path, at, remove, keys });

const clipOf = (id: string) => document.querySelector<HTMLElement>(`[data-clip="${id}"]`)!;
const marksOf = (id: string) => [...clipOf(id).querySelectorAll<HTMLElement>('.clip-key')];
const framesOf = (id: string) => marksOf(id).map(mark => Number(mark.dataset.clipKey));
const toggleOf = (id: string) => clipOf(id).querySelector<HTMLButtonElement>('[data-test="clip-keys-toggle"]');
const trackOf = (id: string) => document.querySelector<HTMLElement>(`[data-key-track="${id}"]`);
const open = (id = 'c1') => { fireEvent.click(toggleOf(id)!); return trackOf(id)!; };
/** The diamonds of a row of a keyframe track: that of a setting, or `group`. */
const diamonds = (row: string, id = 'c1') => [...trackOf(id)!.querySelectorAll<HTMLElement>(`.key-track-key[data-key-row="${row}"]`)];
const at = (row: string, id = 'c1') => diamonds(row, id).map(each => Number(each.dataset.keyAt));
const chosen = (row: string, id = 'c1') => diamonds(row, id).filter(each => each.dataset.selected === 'true').map(each => Number(each.dataset.keyAt));
const pointer = (clientX: number, extra = {}) => ({ button: 0, clientX, clientY: 50, ...extra });
const click = (element: Element, extra = {}) => {
  fireEvent.pointerDown(element, pointer(100, extra));
  fireEvent.pointerUp(element, pointer(100, extra));
};
const tip = () => document.querySelector('[data-test="key-track-tip"]')?.textContent ?? null;

describe('the keys of a clip on the timeline', () => {
  it('are diamonds at the frames at which any of its settings has a key, each saying which and when', () => {
    wire([several, clip('c2', 100, 50)]);
    expect(framesOf('c1')).toEqual([10, 30, 50]);
    const union = [...new Set([SCALE, X, LEFT].flatMap(path => keyTimes(several, { rate }, MEDIA, path)))].sort((a, b) => a - b);
    expect(framesOf('c1')).toEqual(union);
    // Placed by the style sheet from their frames of the clip, as the clip is from its frames of the timeline.
    expect(marksOf('c1').map(mark => Number(mark.style.getPropertyValue('--at')))).toEqual([0, 20, 40]);
    expect(marksOf('c1').map(mark => mark.title)).toEqual(['Position X · 00:00:00:10', 'Crop left · 00:00:01:05', 'Zoom, Position X · 00:00:02:00']);
    expect(toggleOf('c1')!.getAttribute('aria-pressed')).toBe('false');
    expect(toggleOf('c1')!.getAttribute('aria-label')).toBe('Keyframe Editor');
    // A clip without keys has neither diamonds nor a button.
    expect(marksOf('c2')).toEqual([]);
    expect(toggleOf('c2')).toBeNull();
    expect(drawn).toEqual({ marks: 1, tracks: 0 });
  });

  it('are found through the speed and the direction of the clip', () => {
    // Twice as fast, the picture 40 comes at frame 30, and 41, which is not shown, with the one after it.
    const fast = keyed([key(0, 0), key(40, 1), key(41, 0)], { length: 30, speed: { num: 2, den: 1 } });
    const first = wire([fast]);
    expect(framesOf('c1')).toEqual(keyTimes(fast, { rate }, MEDIA, X));
    expect(framesOf('c1')).toEqual([10, 30, 31]);
    first.unmount();
    // Backward, the first picture is shown last.
    const back = keyed([key(0, 0), key(40, 1)], { reverse: true });
    wire([back]);
    expect(framesOf('c1')).toEqual(keyTimes(back, { rate }, MEDIA, X));
    expect(framesOf('c1')).toEqual([29, 69]);
  });

  it('move the playhead to the one that is pressed, and the clip is chosen as by any press on it', () => {
    const { onSeek } = wire([keyed(three)]);
    expect(clipOf('c1').getAttribute('aria-selected')).toBe('false');
    click(marksOf('c1')[1]);
    expect(onSeek.mock.calls).toEqual([[30, false]]);
    expect(clipOf('c1').getAttribute('aria-selected')).toBe('true');
    // Another button of the pointer moves nothing.
    fireEvent.pointerDown(marksOf('c1')[2], pointer(100, { button: 2 }));
    expect(onSeek).toHaveBeenCalledTimes(1);
  });

  it('are not shown by a timeline that is not given the keyframe tracks of its page', () => {
    const bridge = { call: vi.fn().mockResolvedValue({ ok: true, result: {} }), onEvent: vi.fn(() => () => {}) } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>;
    render(<Timeline timeline={timelineOf([keyed(three)])} media={MEDIA} bridge={bridge} session={1} revision={17} frame={0} playing={false} onSeek={vi.fn()} />);
    expect(clipOf('c1')).not.toBeNull();
    expect(document.querySelector('.clip-key, .clip-keys-toggle, .key-track')).toBeNull();
  });
});

describe('the keyframe track of a clip', () => {
  it('opens from the button of the clip, beneath it and as wide: a group row, and a row for each setting that has keys', () => {
    wire([several, clip('c2', 100, 50)]);
    expect(trackOf('c1')).toBeNull();
    const row = document.querySelector<HTMLElement>('[data-track="v1"]')!;
    expect(row.style.height).toBe('72px');
    // The press on the button is the button's: the clip is not chosen, nor dragged.
    fireEvent.pointerDown(toggleOf('c1')!, pointer(100));
    expect(clipOf('c1').getAttribute('aria-selected')).toBe('false');
    const track = open();
    expect(toggleOf('c1')!.getAttribute('aria-pressed')).toBe('true');
    expect(track.getAttribute('aria-label')).toBe('Keyframe Editor');
    expect([...track.querySelectorAll('.key-track-row')].map(each => [each.getAttribute('data-key-track-row'), each.querySelector('.key-track-name')!.textContent]))
      .toEqual([['group', 'Keyframes'], [SCALE, 'Zoom'], [X, 'Position X'], [LEFT, 'Crop left']]);
    // The group row has a diamond at every frame with a key; each setting has its own, where `keyTimes` finds them.
    expect(at('group')).toEqual([10, 30, 50]);
    for (const path of [SCALE, X, LEFT]) expect(at(path)).toEqual(keyTimes(several, { rate }, MEDIA, path));
    expect(at(X)).toEqual([10, 50]);
    expect(diamonds('group').map(each => each.title)).toEqual(['Position X · 00:00:00:10', 'Crop left · 00:00:01:05', 'Zoom, Position X · 00:00:02:00']);
    // As wide as the clip, just under it; the track is higher by the room for it, and its clips are as high as they were.
    const height = keyTrackHeight(3);
    expect([track.style.left, track.style.width, track.style.top, track.style.height])
      .toEqual([`${HEADER_WIDTH + 10 * PX}px`, `${60 * PX}px`, `${RULER_HEIGHT + 72 - KEY_TRACK_RISE}px`, `${height}px`]);
    expect(row.style.height).toBe(`${72 + height}px`);
    expect(row.style.getPropertyValue('--keys')).toBe(`${height}px`);
    expect(row.hasAttribute('data-keys')).toBe(true);
    expect(drawn.tracks).toBeGreaterThan(0);
    // The button closes it again.
    fireEvent.click(toggleOf('c1')!);
    expect(trackOf('c1')).toBeNull();
    expect(toggleOf('c1')!.getAttribute('aria-pressed')).toBe('false');
    expect(row.style.height).toBe('72px');
    expect(row.hasAttribute('data-keys')).toBe(false);
  });

  it('may be open for several clips at once, each with what is chosen in it', () => {
    const second = clip('c3', 100, 50, { transform: { ...NONE, opacity: { keys: [key(0, 1), key(10, 0)] } } });
    wire([keyed(three), second]);
    open('c1');
    open('c3');
    expect(trackOf('c1')).not.toBeNull();
    expect(at('transform.opacity', 'c3')).toEqual([100, 110]);
    click(diamonds(X)[0]);
    click(diamonds('transform.opacity', 'c3')[1]);
    expect(chosen(X)).toEqual([10]);
    expect(chosen('transform.opacity', 'c3')).toEqual([110]);
    fireEvent.click(toggleOf('c1')!);
    expect(trackOf('c1')).toBeNull();
    expect(chosen('transform.opacity', 'c3')).toEqual([110]);
  });

  it('chooses a key with a click, more with Ctrl and Shift, and every key at a frame from the group row', () => {
    wire([several]);
    const track = open();
    click(diamonds(X)[0]);
    expect(chosen(X)).toEqual([10]);
    // The diamond of the group row at that frame stands for that key alone: it is lit with it.
    expect(chosen('group')).toEqual([10]);
    click(diamonds(X)[1], { ctrlKey: true });
    expect(chosen(X)).toEqual([10, 50]);
    // The zoom has a key at frame 50 too, which is not chosen: the group row is not lit there.
    expect(chosen('group')).toEqual([10]);
    click(diamonds(X)[1], { ctrlKey: true });
    expect(chosen(X)).toEqual([10]);
    // A diamond of the group row stands for every key at its frame: the zoom and the place.
    click(diamonds('group')[2]);
    expect([chosen(SCALE), chosen(X), chosen(LEFT), chosen('group')]).toEqual([[50], [50], [], [50]]);
    // Shift adds the keys of the row from the diamond clicked before to this one.
    click(diamonds(X)[0], { shiftKey: true });
    expect([chosen(SCALE), chosen(X), chosen(LEFT)]).toEqual([[50], [10, 50], []]);
    // From the group row, Shift takes those of every row.
    click(diamonds('group')[0]);
    click(diamonds('group')[1], { shiftKey: true });
    expect([chosen(SCALE), chosen(X), chosen(LEFT)]).toEqual([[], [10], [30]]);
    // A click where nothing is lets go of them, and so does Escape.
    click(track);
    expect([chosen(SCALE), chosen(X), chosen(LEFT)]).toEqual([[], [], []]);
    click(diamonds(LEFT)[0]);
    fireEvent.keyDown(track, { key: 'Escape' });
    expect(chosen(LEFT)).toEqual([]);
  });

  it('moves a key that is dragged: the setting whole at each move, one gesture, and the end as it is let go', async () => {
    const { call } = wire([keyed(three)]);
    const track = open();
    const from = diamonds(X)[1];
    fireEvent.pointerDown(from, pointer(200));
    // Not yet a drag.
    fireEvent.pointerMove(from, pointer(201));
    await settle();
    expect(sent(call)).toHaveLength(0);
    expect(tip()).toBeNull();
    // Five frames to the right.
    fireEvent.pointerMove(from, pointer(200 + 5 * PX));
    await settle();
    expect(sent(call)).toHaveLength(1);
    const [, first, context] = sent(call)[0];
    expect(context).toEqual({ expectedSession: 1 });
    expect(first.revision).toBe(17);
    expect(first.gesture.id).toBeTruthy();
    expect(first.gesture.end).toBeUndefined();
    expect(first.ops).toEqual([setKeys(X, moveKeys(three, [1], 5, 0, ANY))]);
    expect(first.ops[0].param.keys).toEqual([key(0, 0), key(25, 0.25), key(40, 0)]);
    // The track draws the key where the drag has it, whatever it is given meanwhile, and says its frame and how far it went.
    expect(at(X)).toEqual([10, 35, 50]);
    expect(chosen(X)).toEqual([35]);
    expect(tip()).toBe('Position X 25 (+5)');
    // A move that leaves the key at the same frame says nothing again.
    fireEvent.pointerMove(from, pointer(200 + 5 * PX + 1));
    await settle();
    expect(sent(call)).toHaveLength(1);
    // Far to the right: it stops a frame short of the key after it; far to the left, of the key before it.
    fireEvent.pointerMove(from, pointer(200 + 200 * PX));
    await settle();
    expect(sent(call)).toHaveLength(2);
    expect(sent(call)[1][1].ops).toEqual([setKeys(X, [key(0, 0), key(39, 0.25), key(40, 0)])]);
    expect(tip()).toBe('Position X 39 (+19)');
    fireEvent.pointerMove(from, pointer(200 - 200 * PX));
    await settle();
    expect(sent(call)).toHaveLength(3);
    const third = sent(call)[2][1];
    expect(third.ops).toEqual([setKeys(X, [key(0, 0), key(1, 0.25), key(40, 0)])]);
    expect(third.gesture).toEqual({ id: first.gesture.id });
    expect(at(X)).toEqual([10, 11, 50]);
    expect(tip()).toBe('Position X 1 (-19)');
    // Let go: the last move once more, as the end of the gesture, which is one step to undo.
    fireEvent.pointerUp(from, pointer(200 - 200 * PX));
    await settle();
    expect(sent(call)).toHaveLength(4);
    const last = sent(call)[3][1];
    expect(last.gesture).toEqual({ id: first.gesture.id, end: true });
    expect(last.ops).toEqual(third.ops);
    expect(last.revision).toBe(17);
    expect(tip()).toBeNull();
    // The timeline took none of it for a drag of the clip.
    expect(document.querySelector('.timeline-preview')).toBeNull();
    expect(clipOf('c1').getAttribute('aria-selected')).toBe('false');
    // A click that moves nothing sends nothing.
    click(diamonds(X)[0]);
    await settle();
    expect(sent(call)).toHaveLength(4);
    expect(track.isConnected).toBe(true);
  });

  it('moves the keys of several settings together from the group row, by what all of them have room for', async () => {
    const { call } = wire([several]);
    open();
    // The zoom and the place at frame 50, five frames to the left.
    const from = diamonds('group')[2];
    fireEvent.pointerDown(from, pointer(300));
    fireEvent.pointerMove(from, pointer(300 - 5 * PX));
    await settle();
    expect(sent(call)).toHaveLength(1);
    expect(sent(call)[0][1].ops).toEqual([setKeys(SCALE, moveKeys(zooms, [0], -5, 0, ANY)), setKeys(X, moveKeys(places, [1], -5, 0, ANY))]);
    expect([at(SCALE), at(X), at('group')]).toEqual([[45], [10, 45], [10, 30, 45]]);
    // More than one key is dragged: the track names no setting.
    expect(tip()).toBe('35 (-5)');
    // The place has a key at the picture 0: both stop a frame after it.
    fireEvent.pointerMove(from, pointer(300 - 200 * PX));
    await settle();
    expect(sent(call)[1][1].ops).toEqual([setKeys(SCALE, [key(1, 2)]), setKeys(X, [key(0, 0), key(1, 1), key(80, 0)])]);
    fireEvent.pointerUp(from, pointer(300 - 200 * PX));
    await settle();
    expect(sent(call)).toHaveLength(3);
    expect(sent(call)[2][1].gesture.end).toBe(true);
  });

  it('puts the settings back as they were where a drag is taken back', async () => {
    const { call } = wire([keyed(three)]);
    const track = open();
    const from = diamonds(X)[1];
    fireEvent.pointerDown(from, pointer(200));
    fireEvent.pointerMove(from, pointer(200 + 5 * PX));
    await settle();
    expect(at(X)).toEqual([10, 35, 50]);
    fireEvent.keyDown(track, { key: 'Escape' });
    await settle();
    expect(sent(call)).toHaveLength(2);
    expect(sent(call)[1][1]).toMatchObject({ revision: 17, gesture: { end: true }, ops: [setKeys(X, three)] });
    // The key is drawn where it was, and is chosen still.
    expect(at(X)).toEqual([10, 30, 50]);
    expect(chosen(X)).toEqual([30]);
    fireEvent.pointerUp(from, pointer(200 + 5 * PX));
    await settle();
    expect(sent(call)).toHaveLength(2);
  });

  it('takes the keys chosen away with Delete and Backspace: a splice, or the number of a setting whose last key goes', async () => {
    const c1 = keyed(three);
    const some = wire([c1]);
    open();
    click(diamonds(X)[1]);
    fireEvent.keyDown(trackOf('c1')!, { key: 'Delete' });
    await settle();
    expect(applied(some.call)).toEqual([{ revision: 17, ops: keyOps('t1', c1, X, deleteKeys({ keys: three }, [1], 20)) }]);
    expect(applied(some.call)[0].ops).toEqual([splice(X, 1, 1, [])]);
    expect(chosen(X)).toEqual([]);
    // With nothing chosen there is nothing to take away.
    fireEvent.keyDown(trackOf('c1')!, { key: 'Backspace' });
    await settle();
    expect(sent(some.call)).toHaveLength(1);
    cleanup();
    const one = wire([keyed([key(20, 0.3)])]);
    open();
    click(diamonds(X)[0]);
    fireEvent.keyDown(trackOf('c1')!, { key: 'Backspace' });
    await settle();
    expect(applied(one.call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip_keys', timeline: 't1', id: 'c1', path: X, param: 0.3 }] }]);
  });

  it('copies the keys chosen and pastes them at the playhead, each into its own setting', async () => {
    const { call, rerender, props } = wire([several]);
    open();
    // The zoom and the place at the picture 40, and the left of the crop at 20.
    click(diamonds('group')[2]);
    click(diamonds(LEFT)[0], { ctrlKey: true });
    fireEvent.keyDown(trackOf('c1')!, { key: 'c', ctrlKey: true });
    // The playhead at frame 15 of the timeline: the picture 5. The first key of all, of the crop, goes there, the others 20 after it.
    rerender(<Page {...props} frame={15} />);
    fireEvent.keyDown(trackOf('c1')!, { key: 'v', ctrlKey: true });
    await settle();
    const pasted = (path: ClipKeyed, keys: Keyframe[], which: number[], frame: number) =>
      keyOps('t1', several, path, { keys: pasteKeys(paramOf(several, path), copyKeys(keys, which), frame, ANY)!.keys });
    expect(applied(call)).toEqual([{ revision: 17, ops: [...pasted(SCALE, zooms, [0], 25), ...pasted(X, places, [1], 25), ...pasted(LEFT, lefts, [0], 5)] }]);
    expect(applied(call)[0].ops).toEqual([splice(SCALE, 0, 0, [key(25, 2)]), splice(X, 1, 0, [key(25, 1)]), splice(LEFT, 0, 0, [key(5, 0.5)])]);
    // Once the timeline has them, the keys pasted are the ones chosen.
    const after = clip('c1', 10, 60, {
      transform: { ...NONE, x: { keys: [key(0, 0), key(25, 1), key(40, 1), key(80, 0)] }, scale: { keys: [key(25, 2), key(40, 2)] } },
      crop: { ...WHOLE, left: { keys: [key(5, 0.5), key(20, 0.5)] } },
    });
    rerender(<Page {...props} frame={15} revision={18} model={timelineOf([after])} />);
    expect([chosen(SCALE), chosen(X), chosen(LEFT)]).toEqual([[35], [35], [15]]);
    // Ctrl+X copies and takes away: here the three keys just pasted.
    fireEvent.keyDown(trackOf('c1')!, { key: 'x', ctrlKey: true });
    await settle();
    expect(applied(call)[1]).toEqual({ revision: 18, ops: [splice(SCALE, 0, 1, []), splice(X, 1, 1, []), splice(LEFT, 0, 1, [])] });
    // With the playhead off the clip there is no frame of it to paste at.
    rerender(<Page {...props} frame={200} revision={18} model={timelineOf([after])} />);
    fireEvent.keyDown(trackOf('c1')!, { key: 'v', ctrlKey: true });
    await settle();
    expect(sent(call)).toHaveLength(2);
  });

  it('keeps its keys from the timeline whether or not anything is chosen, and lets the others go on', async () => {
    const { call } = wire([keyed(three)]);
    const track = open();
    // The clip is chosen: on the timeline Delete would take it away, and Ctrl+C copy it.
    click(clipOf('c1'));
    expect(clipOf('c1').getAttribute('aria-selected')).toBe('true');
    await settle();
    const asked = call.mock.calls.length;
    const page = vi.fn();
    window.addEventListener('keydown', page);
    try {
      const owned: [string, object][] = [['Delete', {}], ['Backspace', {}], ['Delete', { shiftKey: true }], ['Backspace', { shiftKey: true }], ['Escape', {}],
        ['c', { ctrlKey: true }], ['x', { ctrlKey: true }], ['v', { ctrlKey: true }], ['c', { metaKey: true }]];
      for (const [name, modifiers] of owned) fireEvent.keyDown(track, { key: name, ...modifiers });
      await settle();
      expect(page).not.toHaveBeenCalled();
      // Nothing was asked of the engine: no clip was deleted, cut or copied.
      expect(call.mock.calls).toHaveLength(asked);
      expect(clipOf('c1').getAttribute('aria-selected')).toBe('true');
      // The transport is not the track's: Space goes on to the page.
      fireEvent.keyDown(track, { key: ' ' });
      expect(page).toHaveBeenCalledTimes(1);
      // Nor is Undo: the timeline takes it, as everywhere on it.
      fireEvent.keyDown(track, { key: 'z', ctrlKey: true });
      await settle();
      expect(call.mock.calls.filter(([method]) => method === 'history.undo')).toHaveLength(1);
    } finally {
      window.removeEventListener('keydown', page);
    }
  });

  it('follows the clip as the project changes, and closes where the clip loses its last key', () => {
    const { rerender, props } = wire([keyed(three)]);
    open();
    click(diamonds(X)[1]);
    expect(chosen(X)).toEqual([30]);
    // Someone put a key before it: it is drawn, on the clip too, and the key chosen is chosen still.
    rerender(<Page {...props} revision={18} model={timelineOf([keyed([key(0, 0), key(10, 0.1), key(20, 0.25), key(40, 0)])])} />);
    expect(at(X)).toEqual([10, 20, 30, 50]);
    expect(framesOf('c1')).toEqual([10, 20, 30, 50]);
    expect(chosen(X)).toEqual([30]);
    // Undo took the key chosen away: it is chosen no longer.
    rerender(<Page {...props} revision={19} model={timelineOf([keyed([key(0, 0), key(40, 0)])])} />);
    expect(at(X)).toEqual([10, 50]);
    expect(chosen(X)).toEqual([]);
    // A second setting gets keys: it has a row, and the track of the clip is higher for it.
    rerender(<Page {...props} revision={20} model={timelineOf([keyed([key(0, 0), key(40, 0)], { crop: { ...WHOLE, left: { keys: lefts } } })])} />);
    expect(at(LEFT)).toEqual([30]);
    expect(document.querySelector<HTMLElement>('[data-track="v1"]')!.style.height).toBe(`${72 + keyTrackHeight(2)}px`);
    // The last key of the clip goes: no diamonds, no button, and its track closes.
    rerender(<Page {...props} revision={21} model={timelineOf([clip('c1', 10, 60, { transform: { ...NONE, x: 0.3 } })])} />);
    expect(trackOf('c1')).toBeNull();
    expect(marksOf('c1')).toEqual([]);
    expect(toggleOf('c1')).toBeNull();
    expect(document.querySelector<HTMLElement>('[data-track="v1"]')!.style.height).toBe('72px');
    // Where the keys come back the diamonds do, and the track stays closed until it is opened.
    rerender(<Page {...props} revision={22} model={timelineOf([keyed(three)])} />);
    expect(framesOf('c1')).toEqual([10, 30, 50]);
    expect(trackOf('c1')).toBeNull();
    expect(toggleOf('c1')!.getAttribute('aria-pressed')).toBe('false');
  });

  it('shows the keys of a clip on a locked track, which can be chosen and copied and not changed', async () => {
    const { call } = wire([keyed(three)], { locked: true });
    expect(framesOf('c1')).toEqual([10, 30, 50]);
    const track = open();
    expect(track.hasAttribute('data-locked')).toBe(true);
    const from = diamonds(X)[1];
    fireEvent.pointerDown(from, pointer(200));
    fireEvent.pointerMove(from, pointer(200 + 5 * PX));
    fireEvent.pointerUp(from, pointer(200 + 5 * PX));
    expect(chosen(X)).toEqual([30]);
    expect(at(X)).toEqual([10, 30, 50]);
    fireEvent.keyDown(track, { key: 'c', ctrlKey: true });
    fireEvent.keyDown(track, { key: 'v', ctrlKey: true });
    fireEvent.keyDown(track, { key: 'Delete' });
    await settle();
    expect(sent(call)).toHaveLength(0);
    expect(chosen(X)).toEqual([30]);
  });
});

describe('a thousand-clip timeline whose clips have no keys', () => {
  it('mounts nothing for keys, as the page gives it its keyframe tracks; one clip that gets a key mounts its own part alone', async () => {
    const bridge = readyBridge();
    const onSeek = vi.fn();
    const plain = bigTimeline();
    const tree = (model: TimelineModel, frame = 0) => <Page model={model} bridge={bridge} onSeek={onSeek} frame={frame} revision={0} />;
    const view = render(tree(plain));
    // The waveforms come from the engine at once; they are drawn once they have come.
    await act(async () => { for (let n = 0; n < 5; n++) await new Promise(resolve => setTimeout(resolve, 0)); });
    expect(document.querySelectorAll('[data-clip]').length).toBeGreaterThan(10);
    expect(drawn).toEqual({ marks: 0, tracks: 0 });
    expect(document.querySelector('.clip-key, .clip-keys-toggle, .key-track, [data-keys]')).toBeNull();
    for (let frame = 1; frame <= 10; frame++) view.rerender(tree(plain, frame));
    expect(drawn).toEqual({ marks: 0, tracks: 0 });
    // The first clip of the first track of picture gets a key: its diamonds are mounted, and nothing for any other clip.
    const first = plain.tracks[0];
    if (first.kind !== 'video') throw new Error('The first track of the fixture is one of picture.');
    const one: TimelineModel = { ...plain, tracks: [{ ...first, clips: first.clips.map((each, place) => (place === 0
      ? { ...each, transform: { ...each.transform, x: { keys: [key(0, 0)] } } } : each)) }, ...plain.tracks.slice(1)] };
    view.rerender(tree(one, 10));
    expect(document.querySelectorAll('.clip-key')).toHaveLength(1);
    expect(drawn).toEqual({ marks: 1, tracks: 0 });
    // And it is not drawn again as the playhead moves.
    for (let frame = 11; frame <= 20; frame++) view.rerender(tree(one, frame));
    expect(drawn).toEqual({ marks: 1, tracks: 0 });
  });
});
