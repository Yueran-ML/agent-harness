// @vitest-environment jsdom
/**
 * What the desktop reads of a clip through its speed and its direction, on a clip with speed
 * points: a segment that plays forward, a freeze, a segment that plays backward, and one that
 * plays at twice the speed. The frames of keys, the pictures shown, where a key is drawn, the
 * sound and its waveform, the room for a trim or a slip, what the clip says of its speed, and what
 * Change Clip Speed sends for it.
 *
 * The clip is that of the engine's test `a_clip_with_speed_points_shows_the_places_of_their_path_
 * and_its_keys_there` (`engine/crates/model/src/resolve.rs`): 50 frames from frame 100, which play
 * 10 frames of the media, hold a picture for 10, go back over the 10, and then on at twice the speed.
 */
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { sourceFrameAt } from '../src/studio/color/clips';
import { keyClockOf, keyRunsOf, shownKeyRange } from '../src/studio/color/keyFrames';
import { keyRange, timelineFrameOf, timesOf, type KeySpan } from '../src/studio/color/keyTracks';
import type { Clip, Keyframe, Media, Retime } from '../src/studio/engine';
import { frameOfKey, keyTimes } from '../src/studio/inspector/clipKeys';
import { speedEdits } from '../src/studio/inspector/SpeedDialog';
import { frameAtSource, keyTime, sourceAtFrame, sourceSampleBounds } from '../src/studio/timeline/audioKeys';
import { HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { beginDrag, dragTo, type Hit } from '../src/studio/timeline/interact';
import { keyRows } from '../src/studio/timeline/keyTrack';
import { Timeline } from '../src/studio/timeline/Timeline';
import { Waveform } from '../src/studio/timeline/Waveform';
import { sourceInterval, Waveforms } from '../src/studio/timeline/waveforms';
import { clip, media, timeline } from './timeline-fixture';
import { readyBridge } from './timeline-perf-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956 + HEADER_WIDTH);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const rate = { num: 25, den: 1 };
const MEDIA = [media];
const retime = (...points: [number, number, number][]): Retime => ({ points: points.map(([frame, num, den]) => ({ frame, speed: { num, den } })) });
const key = (frame: number, value = 0): Keyframe => ({ frame, value, interp: 'linear' });
const NONE = { x: 0, y: 0, scale: 1, rotation: 0, opacity: 1 };
const POINTS = retime([0, 1, 1], [10, 0, 1], [20, -1, 1], [30, 2, 1]);
/** c3 on frames [100, 150), from the first picture of its media. */
const c3 = (fields: Partial<Clip> = {}) => clip('c3', 100, 50, { retime: POINTS, ...fields });
const range = (count: number) => Array.from({ length: count }, (_, n) => n);
/** A time or a frame that is worked out in floating numbers, to nine places. */
const near = (value: number) => Number(value.toFixed(9));
/** The picture of the media that each of its frames shows: one all through the freeze, and on the way back those of the way up, the last first. */
const PICTURES = [...range(10), ...range(10).map(() => 10), ...range(10).reverse(), ...range(20).map(n => 2 * n)];

describe('the pictures and the keys of a clip with speed points', () => {
  it('count the frame of the media that each frame shows along the speed points', () => {
    const clock = keyClockOf(c3(), rate, media);
    expect(range(50).map(k => clock(100 + k))).toEqual(PICTURES);
    // The thumbnails of the Color page show the same pictures.
    expect(range(50).map(k => sourceFrameAt(c3(), k, rate, media))).toEqual(PICTURES);
    // From the lowest picture of the path to the highest, wherever in the clip.
    expect(shownKeyRange(c3(), clock)).toEqual([0, 38]);
    // A clip of one speed is read as it was: backward, its last frame shows its first picture.
    const back = clip('c', 10, 30, { reverse: true });
    expect(keyRunsOf(back)).toBeNull();
    expect(shownKeyRange(back, keyClockOf(back, rate, media))).toEqual([0, 29]);
  });

  it('show a key at the first frame that comes by its picture, and not at all where none does', () => {
    // The picture 3 is shown on the way up and again on the way back; 10 all through the freeze; 11 only passed at twice the speed; 39 never.
    const keyed = c3({ transform: { ...NONE, x: { keys: [3, 10, 11, 20, 38, 39].map(frame => key(frame)) } } });
    expect(keyTimes(keyed, { rate }, MEDIA, 'transform.x')).toEqual([103, 110, 136, 140, 149]);
    expect([3, 9, 10, 11, 38].map(frame => frameOfKey(keyed, { rate }, MEDIA, frame))).toEqual([103, 109, 110, 136, 149]);
    // A picture that the clip does not show is at the first frame that shows the nearest that it does.
    expect([frameOfKey(keyed, { rate }, MEDIA, 100), frameOfKey(keyed, { rate }, MEDIA, -5)]).toEqual([149, 100]);
    // The rows of its keyframe track leave out the key that is not shown.
    expect(keyRows(keyed, { rate }, MEDIA)[0].keys.map(each => [each.frame, each.at])).toEqual([[3, 103], [10, 110], [11, 136], [20, 140], [38, 149]]);
    // A picture skipped at a speed point is at the first frame after it.
    const skipping = clip('c4', 0, 10, { retime: retime([0, 3, 1], [4, 1, 1]), transform: { ...NONE, x: { keys: [7, 10, 12].map(frame => key(frame)) } } });
    expect(range(10).map(keyClockOf(skipping, rate, media))).toEqual([0, 3, 6, 9, 12, 13, 14, 15, 16, 17]);
    expect([7, 10, 12].map(frame => frameOfKey(skipping, { rate }, MEDIA, frame))).toEqual([3, 4, 4]);
  });

  it('are shown so by the Keyframe Editor of the Color page', () => {
    const clock = keyClockOf(c3(), rate, media);
    const span: KeySpan = { first: 100, last: 149, clock, runs: keyRunsOf(c3())! };
    expect(span.runs).toEqual([{ first: 100, last: 109 }, { first: 110, last: 119 }, { first: 120, last: 129 }, { first: 130, last: 149 }]);
    expect(keyRange(span)).toEqual([0, 38]);
    expect([3, 10, 99, -1].map(frame => timelineFrameOf(span, frame))).toEqual([103, 110, 149, 100]);
    expect(timesOf(span, [3, 39, 20])).toEqual([103, 140]);
  });
});

describe('the sound of a clip with speed points', () => {
  it('reads the time of its source along the speed points', () => {
    const sound = c3();
    // Up, held, half of the way back, and half of the way on at twice the speed.
    expect([105, 115, 125, 140].map(frame => near(sourceAtFrame(sound, rate, frame)))).toEqual([0.2, 0.4, 0.2, 0.8]);
    expect(sourceInterval(sound, rate, 100, 150).map(near)).toEqual([0, 1.6]);
    // Over a part of it, from the lowest place that the part comes by to the highest, which is at a speed point.
    expect(sourceInterval(sound, rate, 105, 125).map(near)).toEqual([0.2, 0.4]);
    // The keys of its gain are at times of its source: the model takes them within what the clip shows.
    expect(sourceSampleBounds(sound, rate, 48000)).toEqual([0, 76_800]);
    expect(keyTime(sound, rate, 115, 48000, true)).toEqual({ ticks: 19_200, scale: 48000 });
  });

  it('draws a key of its gain at the first frame that reads its time', () => {
    const sound = c3();
    // 0.2 seconds is read on the way up and on the way back: the way up comes first. The time held is read as the way back begins.
    expect([0.2, 0.4, 1].map(seconds => near(frameAtSource(sound, rate, seconds)))).toEqual([105, 120, 142.5]);
    // After what the clip reads, the last segment goes on, as a clip of one speed goes on.
    expect(near(frameAtSource(sound, rate, 2))).toBe(155);
    // A time that the clip never comes by is before its first frame, where nothing is drawn.
    const dipping = clip('c5', 40, 20, { source: { kind: 'media', media: 'm1', in: { ticks: 50, scale: 25 } }, retime: retime([0, -1, 1], [10, 1, 1]) });
    expect(near(sourceAtFrame(dipping, rate, 50))).toBe(2);
    expect(frameAtSource(dipping, rate, 1)).toBe(39);
  });

  it('draws its waveform a piece for each segment that plays, and nothing where it holds a picture', async () => {
    const waves = new Waveforms(readyBridge() as Pick<EngineBridge, 'call' | 'onEvent'>, 1);
    waves.connect();
    const marks = (d: string | null) => (d ?? '').split('M').length - 1;
    const drawn = async (sound: Clip) => {
      const view = render(<Waveform cache={waves} media="m1" clip={sound} rate={rate} from={0} to={40} scale={8} />);
      // The peaks come from the engine at once, and the tiles are worked out after the clip is drawn.
      await act(async () => { for (let at = 0; at < 5; at++) await new Promise(resolve => setTimeout(resolve, 0)); });
      const paths = [...view.container.querySelectorAll('path')].map(path => marks(path.getAttribute('d')));
      view.unmount();
      return paths;
    };
    // 320 pixels, a column of two pixels to each quarter of a frame: 160 columns for each of two channels.
    expect(await drawn(clip('a1', 0, 40))).toEqual([160, 160]);
    // 20 frames that play and 20 that hold a picture: the columns of the first half only.
    expect(await drawn(clip('a2', 0, 40, { retime: retime([0, 1, 1], [20, 0, 1]) }))).toEqual([80, 80]);
    // Forward, held and back: the freeze in the middle is left empty.
    expect(await drawn(clip('a3', 0, 40, { retime: retime([0, 1, 1], [10, 0, 1], [30, -1, 1]) }))).toEqual([80, 80]);
    // A freeze frame from end to end has no sound to draw.
    expect(await drawn(clip('a4', 0, 40, { retime: retime([0, 0, 1]) }))).toEqual([0, 0]);
    waves.disconnect();
  });
});

describe('a trim, a roll and a slip of a clip with speed points', () => {
  // 60 frames of media. The clip begins 5 frames into it, plays 10 frames and then 20 more at twice the speed: 30 frames of the media, to its frame 35.
  const short: Media = { ...media, facts: { ...media.facts, duration: { ticks: 60, scale: 25 } } };
  const pointed = clip('c1', 10, 20, { source: { kind: 'media', media: 'm1', in: { ticks: 5, scale: 25 } }, retime: retime([0, 1, 1], [10, 2, 1]) });
  const dragged = (action: Hit, origin: number, by: number) => dragTo(beginDrag(timeline([pointed]), 1, ['c1'], 'c1', action, origin, false)!, origin + by, { media: [short] }).delta;

  it('are held to what its media has along the segments at its ends', () => {
    // Its tail goes on at twice the speed: 25 frames of media are 12 frames of the clip.
    expect(dragged('trim_out', 30, 100)).toBe(12);
    // Its head goes on before it at the speed of the source: 5 frames.
    expect(dragged('trim_in', 10, -100)).toBe(-5);
    // Inward, an edge stops a frame from the other, as for any clip.
    expect(dragged('trim_out', 30, -100)).toBe(-19);
  });

  it('slips by frames of its source: those before the lowest place of its path, and those after the highest', () => {
    expect(dragged('slip', 20, 100)).toBe(25);
    expect(dragged('slip', 20, -100)).toBe(-5);
  });
});

describe('what the page says and sends of a clip with speed points', () => {
  it('says the slowest and the fastest of its speeds on the clip', () => {
    const bridge = { call: vi.fn().mockResolvedValue({ ok: true, result: {} }), onEvent: vi.fn(() => () => {}) } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>;
    render(<Timeline timeline={timeline([clip('c1', 0, 40, { speed: { num: 2, den: 1 }, reverse: true }), c3(), clip('c9', 200, 40)])} media={MEDIA} bridge={bridge}
      session={1} revision={17} frame={0} playing={false} onSeek={vi.fn()} />);
    const said = (id: string) => document.querySelector(`[data-clip="${id}"] .timeline-clip-speed`)?.textContent ?? null;
    expect(said('c3')).toBe('−100–200%');
    // A clip of one speed says it as it did, and one at its own speed nothing.
    expect(said('c1')).toBe('−2×');
    expect(said('c9')).toBeNull();
    // Nothing of the Retime Controls is on a timeline that is not given them.
    expect(document.querySelector('.retime-bar')).toBeNull();
  });

  it('puts the clip back to one speed before Change Clip Speed gives it a speed', () => {
    const model = timeline([c3(), clip('c9', 200, 40)]);
    expect(speedEdits(model, ['c3', 'c9'], { num: 2, den: 1 }, undefined, true)).toEqual([
      { edit: 'reset_speed', timeline: 't1', clip: 'c3', ripple: true, linked: false },
      { edit: 'set_speed', timeline: 't1', clip: 'c3', speed: { num: 2, den: 1 }, ripple: true, linked: false },
      { edit: 'set_speed', timeline: 't1', clip: 'c9', speed: { num: 2, den: 1 }, ripple: true, linked: false },
    ]);
    // A group of linked clips all chosen is reset as one, where any of them has speed points.
    const linked = timeline([c3({ link: 'g1' }), clip('c9', 200, 50, { link: 'g1' })]);
    expect(speedEdits(linked, ['c9', 'c3'], { num: 1, den: 2 }, true, false)).toEqual([
      { edit: 'reset_speed', timeline: 't1', clip: 'c9', ripple: false, linked: true },
      { edit: 'set_speed', timeline: 't1', clip: 'c9', speed: { num: 1, den: 2 }, reverse: true, ripple: false, linked: true },
    ]);
  });
});
