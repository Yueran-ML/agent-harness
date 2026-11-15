// @vitest-environment jsdom
/**
 * The motion path in the timeline viewer of the Edit page (`src/studio/motionPath/useMotionPath`;
 * DaVinci Resolve Reference Manual 20, pp. 1106–1108): what the engine is asked to draw and when it
 * is asked to draw nothing, the pointer that the view takes meanwhile, a drag of a point as one
 * gesture of both settings, Shift, Escape, a clip on a locked track, Linear and Smooth, and the
 * on-screen Transform controls that the page remembers.
 *
 * The timeline is 1920 by 1080 and its picture is shown half as large, so 8 pixels of the screen are
 * 16 of the timeline. The clip is cut from its media 50 frames in: at frame 10 it shows picture 50.
 */
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type EngineEvent, type Interp, type Keyframe, type Param, type Timeline, type ViewPointer } from '../src/studio/engine';
import { EASE_SHARE } from '../src/studio/keyframes/value';
import { TRANSFORM_CONTROLS_KEY, useTransformControls } from '../src/studio/motionPath/TransformControls';
import { useMotionPath } from '../src/studio/motionPath/useMotionPath';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); localStorage.clear(); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });

const key = (frame: number, value: number, interp: Interp = 'linear'): Keyframe => ({ frame, value, interp });
const keys = (...given: Keyframe[]): Param => ({ keys: given });
const cut = (x: Param, y: Param, fields: Partial<Clip> = {}) => clip('c1', 10, 5, {
  source: { kind: 'media', media: 'm1', in: { ticks: 50, scale: 25 } }, transform: { x, y, scale: 1, rotation: 0, opacity: 1 }, ...fields,
});
/** From the middle to the right edge in five frames, a quarter of the height below the middle; both settings have keys at both ends. */
const X = keys(key(50, 0), key(54, 0.5));
const Y = keys(key(50, 0.25), key(54, 0.25));
const ACROSS = cut(X, Y);
const TICKS = [[960, 810], [1200, 810], [1440, 810], [1680, 810], [1920, 810]];
const timelineOf = (clips: Clip[], locked = false): Timeline => ({
  id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }],
});
type Call = ReturnType<typeof vi.fn>;
type Props = Parameters<typeof useMotionPath>[0];

function Path(props: Props) {
  const path = useMotionPath(props);
  return <div data-test="path" data-shown={path.shown || undefined}>
    {path.menu}
    {path.notice && <p data-test="notice">{path.notice}</p>}
  </div>;
}

function mount(given: Partial<Props> & { clip?: Clip; locked?: boolean } = {}) {
  const { clip: shown = ACROSS, locked = false, ...rest } = given;
  const listeners = new Set<(event: EngineEvent) => void>();
  let revision = 17;
  const call: Call = vi.fn(async (method: string) => ({ ok: true, result: method === 'transaction.apply' ? { revision: ++revision, done: [] } : {} }));
  const bridge = {
    call, onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
  } as unknown as Pick<EngineBridge, 'call' | 'onEvent'>;
  const onFocus = vi.fn();
  let props: Props = {
    bridge, session: 1, revision: 17, native: true, loaded: true, on: true, timeline: timelineOf([shown], locked), media: [media], clips: ['c1'],
    frame: 11, playing: false, onFocus, viewer: { current: document.body }, ...rest,
  };
  const view = render(<Path {...props} />);
  let event = 0;
  /** What the pointer does on the picture, at a place of the timeline: the view shows it half as large. */
  const pointer = (kind: ViewPointer['kind'], x: number, y: number, extra: Partial<ViewPointer> = {}) => {
    const told: ViewPointer = {
      monitor: 'timeline', instance: 1, event: ++event,
      presentation: { serial: 1, tag: 1, map: { surface: [960, 540], picture: [1920, 1080], dest: [0, 0, 960, 540] } },
      snapshot: { revision: 17, timeline: 't1', frame: 11, canvas: [1920, 1080], ready: true, editable: true, comparison: 'off' },
      kind, button: kind === 'down' || kind === 'up' ? 'left' : null, buttons: kind === 'up' ? [] : ['left'], x, y, inside: true,
      window: { x: Math.round(x / 2), y: Math.round(y / 2) }, shift: false, ctrl: false, alt: false, ...extra,
    };
    act(() => listeners.forEach(listener => listener({ kind: 'notification', session: 1, method: 'view.pointer', params: told })));
  };
  const tell = (told: EngineEvent) => act(() => listeners.forEach(listener => listener(told)));
  const update = (change: Partial<Props>) => { props = { ...props, ...change }; view.rerender(<Path {...props} />); };
  const sent = (method: string) => call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
  return { ...view, call, onFocus, pointer, tell, update, sent, overlays: () => sent('view.overlay').map(params => params.overlay), applied: () => sent('transaction.apply') };
}
const setKeys = (path: string, param: Param) => ({ op: 'set_clip_keys', timeline: 't1', id: 'c1', path, param });
const notice = () => document.querySelector('[data-test="notice"]')?.textContent ?? null;
const item = (id: string) => document.querySelector<HTMLButtonElement>(`[data-test="menu-${id}"]`);

describe('the motion path in the timeline viewer', () => {
  it('has the engine draw the path of the clip chosen, with the place at the playhead, and the view take the pointer', async () => {
    const h = mount(); await settle();
    expect(h.sent('view.overlay')).toEqual([{ monitor: 'timeline', overlay: {
      canvas: [1920, 1080], lines: [TICKS],
      dots: [{ kind: 'tick', at: TICKS }, { kind: 'key', at: [[960, 810], [1920, 810]] }, { kind: 'current', at: [[1200, 810]] }],
    } }]);
    expect(h.sent('view.pointer')).toEqual([{ monitor: 'timeline', on: true, cursor: 'arrow' }]);
    // Each is asked of the engine that the path is of.
    expect(h.call.mock.calls.every(([, , context]) => context?.expectedSession === 1)).toBe(true);
    expect(document.querySelector('[data-test="path"]')?.getAttribute('data-shown')).toBe('true');
    // The playhead moves: the path is told again with its place; off the clip it has none.
    h.update({ frame: 12 }); await settle();
    expect(h.overlays().at(-1).dots.at(-1)).toEqual({ kind: 'current', at: [[1440, 810]] });
    h.update({ frame: 40 }); await settle();
    expect(h.overlays().at(-1).dots.map((each: { kind: string }) => each.kind)).toEqual(['tick', 'key']);
    expect(h.overlays()).toHaveLength(3);
    // Told the same again, it is not sent again.
    h.update({ revision: 18 }); await settle();
    expect(h.overlays()).toHaveLength(3);
    expect(h.sent('view.pointer')).toHaveLength(1);
  });

  it('leaves the place at the playhead out while the timeline plays, the path staying', async () => {
    const h = mount({ playing: true }); await settle();
    expect(h.overlays()).toEqual([{ canvas: [1920, 1080], lines: [TICKS], dots: [{ kind: 'tick', at: TICKS }, { kind: 'key', at: [[960, 810], [1920, 810]] }] }]);
    // The frames that are told while it plays change nothing of what is drawn.
    h.update({ frame: 12 }); h.update({ frame: 13 }); await settle();
    expect(h.overlays()).toHaveLength(1);
    h.update({ playing: false }); await settle();
    expect(h.overlays().at(-1).dots.at(-1)).toEqual({ kind: 'current', at: [[1680, 810]] });
  });

  it('draws the path again as the clip is edited from anywhere', async () => {
    const h = mount(); await settle();
    h.update({ revision: 18, timeline: timelineOf([cut(keys(key(50, 0), key(54, 0.25)), Y)]) }); await settle();
    expect(h.overlays().at(-1)).toEqual({
      canvas: [1920, 1080], lines: [[[960, 810], [1080, 810], [1200, 810], [1320, 810], [1440, 810]]],
      dots: [
        { kind: 'tick', at: [[960, 810], [1080, 810], [1200, 810], [1320, 810], [1440, 810]] },
        { kind: 'key', at: [[960, 810], [1440, 810]] },
        { kind: 'current', at: [[1080, 810]] },
      ],
    });
  });

  it('has the engine draw nothing, and the view let the pointer go, once no path is shown', async () => {
    // Turned off, no clip chosen, the timeline taken from the monitor, and two clips of picture chosen.
    const two = timelineOf([ACROSS, clip('c2', 20, 5)]);
    for (const change of [{ on: false }, { clips: [] }, { loaded: false }, { clips: ['c1', 'c2'] }] as Partial<Props>[]) {
      const h = mount({ timeline: two }); await settle();
      expect(h.overlays()).toHaveLength(1);
      h.update(change); await settle();
      expect(h.sent('view.overlay').at(-1), JSON.stringify(change)).toEqual({ monitor: 'timeline', overlay: null });
      expect(h.sent('view.pointer').at(-1), JSON.stringify(change)).toEqual({ monitor: 'timeline', on: false });
      expect(document.querySelector('[data-test="path"]')?.getAttribute('data-shown')).toBeNull();
      // And nothing more while there is none.
      const before = h.call.mock.calls.length;
      h.update({ frame: 13 }); await settle();
      expect(h.call.mock.calls).toHaveLength(before);
      h.unmount();
    }
    // The clip loses the keys of its position, by an edit from anywhere: its path goes too.
    const h = mount(); await settle();
    h.update({ timeline: timelineOf([cut(0.25, 0.25)]) }); await settle();
    expect(h.sent('view.overlay').at(-1)).toEqual({ monitor: 'timeline', overlay: null });
    // And as the page is left.
    const left = mount(); await settle();
    left.unmount(); await settle();
    expect(left.sent('view.overlay').at(-1)).toEqual({ monitor: 'timeline', overlay: null });
    expect(left.sent('view.pointer').at(-1)).toEqual({ monitor: 'timeline', on: false });
  });

  it('sends nothing for a clip whose position has no keys, with the controls off, in a browser, or with no timeline loaded', async () => {
    const still = cut(0.25, 0.25, { crop: { left: keys(key(50, 0), key(54, 0.5)), right: 0, top: 0, bottom: 0, softness: 0 } });
    for (const given of [{ clip: still }, { on: false }, { native: false }, { loaded: false }, { clips: [] }, { timeline: null }] as Parameters<typeof mount>[0][]) {
      const h = mount(given); await settle();
      h.update({ frame: 12 }); await settle();
      // The pointer on the picture is nothing to a page that shows no path.
      h.pointer('down', 1920, 810); h.pointer('move', 2160, 810); h.pointer('up', 2160, 810); await settle();
      expect(h.call, JSON.stringify(Object.keys(given ?? {}))).not.toHaveBeenCalled();
      expect(h.onFocus).not.toHaveBeenCalled();
      h.unmount();
    }
  });

  it('asks a view that is attached later to take the pointer too', async () => {
    const h = mount(); await settle();
    h.tell({ kind: 'attached', session: 1, monitor: 'source', adapter: null, backend: null }); await settle();
    expect(h.sent('view.pointer')).toHaveLength(1);
    h.tell({ kind: 'attached', session: 1, monitor: 'timeline', adapter: null, backend: null }); await settle();
    expect(h.sent('view.pointer')).toEqual([{ monitor: 'timeline', on: true, cursor: 'arrow' }, { monitor: 'timeline', on: true, cursor: 'arrow' }]);
  });
});

describe('a point of the motion path dragged in the viewer', () => {
  it('sets both settings whole at each move, as one gesture that the last move ends', async () => {
    const h = mount(); await settle();
    // A press 6 pixels of the timeline from the point at picture 54: within the 16 that 8 of the screen are.
    h.pointer('down', 1926, 810); await settle();
    expect(h.onFocus).toHaveBeenCalledWith('timeline');
    expect(h.applied()).toEqual([]);
    // The point that is held is drawn as the one dragged.
    expect(h.overlays().at(-1).dots).toEqual([
      { kind: 'tick', at: TICKS }, { kind: 'key', at: [[960, 810]] }, { kind: 'active', at: [[1920, 810]] }, { kind: 'current', at: [[1200, 810]] },
    ]);
    // 240 pixels right and 67.5 up: an eighth of the width and a sixteenth of the height.
    h.pointer('move', 2166, 742.5); await settle();
    expect(h.applied()).toEqual([{
      revision: 17, gesture: { id: expect.any(String) },
      ops: [setKeys('transform.x', keys(key(50, 0), key(54, 0.625))), setKeys('transform.y', keys(key(50, 0.25), key(54, 0.1875)))],
    }]);
    expect(h.call.mock.calls.find(([method]) => method === 'transaction.apply')![2]).toEqual({ expectedSession: 1 });
    // Each move is worked out from the clip as it was at the press, not from the move before.
    h.pointer('move', 2406, 810); await settle();
    const second = [setKeys('transform.x', keys(key(50, 0), key(54, 0.75))), setKeys('transform.y', Y)];
    expect(h.applied()[1]).toEqual({ revision: 17, gesture: { id: h.applied()[0].gesture.id }, ops: second });
    h.pointer('up', 2406, 810); await settle();
    expect(h.applied()).toHaveLength(3);
    expect(h.applied()[2]).toEqual({ revision: 17, gesture: { id: h.applied()[0].gesture.id, end: true }, ops: second });
    // Let go, it is a point as the others, and the pointer moves nothing.
    expect(h.overlays().at(-1).dots.map((each: { kind: string }) => each.kind)).toEqual(['tick', 'key', 'current']);
    h.pointer('move', 1000, 100); await settle();
    expect(h.applied()).toHaveLength(3);
    expect(notice()).toBeNull();
  });

  it('sends nothing for a press that does not move the point, or that is on no point', async () => {
    const h = mount(); await settle();
    h.pointer('down', 1920, 810); h.pointer('move', 1920, 810); h.pointer('up', 1920, 810); await settle();
    // 17 pixels of the timeline from the nearest point: beyond the 16 that 8 of the screen are.
    h.pointer('down', 1937, 810); h.pointer('move', 2100, 700); h.pointer('up', 2100, 700); await settle();
    // On a point, of the picture of another timeline; where the view shows no picture; and with the middle button.
    h.pointer('down', 1920, 810, { snapshot: { revision: 17, timeline: 't2', frame: 0, canvas: [1920, 1080], ready: true, editable: true, comparison: 'off' } });
    h.pointer('move', 2100, 700); h.pointer('up', 2100, 700); await settle();
    h.pointer('down', 1920, 810, { x: null, y: null }); h.pointer('move', 2100, 700); h.pointer('up', 2100, 700); await settle();
    h.pointer('down', 1920, 810, { button: 'middle', buttons: ['middle'] }); h.pointer('move', 2100, 700); h.pointer('up', 2100, 700, { button: 'middle' }); await settle();
    expect(h.applied()).toEqual([]);
    // Each press on the picture still makes the timeline monitor the one that the keys go to.
    expect(h.onFocus).toHaveBeenCalledTimes(5);
  });

  it('keeps the point on the axis that it has gone further along while Shift is held', async () => {
    const h = mount(); await settle();
    h.pointer('down', 1920, 810);
    h.pointer('move', 2160, 742.5, { shift: true }); await settle();
    expect(h.applied()[0].ops).toEqual([setKeys('transform.x', keys(key(50, 0), key(54, 0.625))), setKeys('transform.y', Y)]);
    // Further down than across: it goes down only.
    h.pointer('move', 1950, 1080, { shift: true }); await settle();
    expect(h.applied()[1].ops).toEqual([setKeys('transform.x', X), setKeys('transform.y', keys(key(50, 0.25), key(54, 0.5)))]);
    // Shift let go, it goes where the pointer is.
    h.pointer('move', 1950, 1080); await settle();
    expect(h.applied()[2].ops).toEqual([setKeys('transform.x', keys(key(50, 0), key(54, 0.515625))), setKeys('transform.y', keys(key(50, 0.25), key(54, 0.5)))]);
    h.pointer('up', 1950, 1080); await settle();
    expect(h.applied()).toHaveLength(4);
  });

  it('puts a key on the setting that has none at the frame of the point, in the same gesture', async () => {
    // The height has keys elsewhere, and none at picture 54, where the point of the width is.
    const apart = cut(X, keys(key(48, 0), key(56, 0.5)));
    const h = mount({ clip: apart }); await settle();
    // The point at picture 54 is where the two settings are then: (1920, 540 + 0.375 × 1080).
    expect(h.overlays()[0].dots[1]).toEqual({ kind: 'key', at: [[960, 675], [1920, 945]] });
    h.pointer('down', 1920, 945);
    h.pointer('move', 1920, 1080); await settle();
    expect(h.applied()[0].ops).toEqual([
      setKeys('transform.x', X),
      setKeys('transform.y', keys(key(48, 0), key(54, 0.5), key(56, 0.5))),
    ]);
    // Moved across only, the height gets no key.
    h.pointer('move', 2160, 945); await settle();
    expect(h.applied()[1].ops).toEqual([setKeys('transform.x', keys(key(50, 0), key(54, 0.625))), setKeys('transform.y', apart.transform.y)]);
    h.pointer('up', 2160, 945); await settle();
    expect(new Set(h.applied().map(each => each.gesture.id)).size).toBe(1);
    expect(h.applied().at(-1).gesture.end).toBe(true);
  });

  it('gives a setting that is one value a key at each point, so that the point dragged alone moves', async () => {
    const h = mount({ clip: cut(keys(key(50, 0), key(52, 0.25), key(54, 0.5)), 0.25) }); await settle();
    h.pointer('down', 1440, 810);
    h.pointer('move', 1440, 945); await settle();
    expect(h.applied()[0].ops).toEqual([
      setKeys('transform.x', keys(key(50, 0), key(52, 0.25), key(54, 0.5))),
      setKeys('transform.y', keys(key(50, 0.25), key(52, 0.375), key(54, 0.25))),
    ]);
    h.pointer('up', 1440, 945); await settle();
  });

  it('is taken back by Escape, and moves nothing until the button is let go', async () => {
    const h = mount(); await settle();
    h.pointer('down', 1920, 810);
    h.pointer('move', 2160, 742.5); await settle();
    expect(h.applied()).toHaveLength(1);
    fireEvent.keyDown(document.body, { key: 'Escape' }); await settle();
    // The two settings as they were at the press, which ends the gesture.
    expect(h.applied()).toHaveLength(2);
    expect(h.applied()[1]).toEqual({
      revision: 17, gesture: { id: h.applied()[0].gesture.id, end: true }, ops: [setKeys('transform.x', X), setKeys('transform.y', Y)],
    });
    expect(h.overlays().at(-1).dots.map((each: { kind: string }) => each.kind)).toEqual(['tick', 'key', 'current']);
    h.pointer('move', 2400, 600); h.pointer('up', 2400, 600); await settle();
    expect(h.applied()).toHaveLength(2);
    // The next press, once the page has the project as the two transactions left it, is a drag of its own.
    h.update({ revision: 19 }); await settle();
    h.pointer('down', 960, 810);
    h.pointer('move', 1200, 810); await settle();
    expect(h.applied()).toHaveLength(3);
    expect(h.applied()[2].gesture.id).not.toBe(h.applied()[0].gesture.id);
    expect(h.applied()[2].ops).toEqual([setKeys('transform.x', keys(key(50, 0.125), key(54, 0.5))), setKeys('transform.y', Y)]);
    h.pointer('up', 1200, 810); await settle();
  });

  it('sends nothing where Escape comes before the point has moved', async () => {
    const h = mount(); await settle();
    h.pointer('down', 1920, 810); await settle();
    fireEvent.keyDown(document.body, { key: 'Escape' });
    h.pointer('move', 2160, 742.5); h.pointer('up', 2160, 742.5); await settle();
    expect(h.applied()).toEqual([]);
    // Escape with no point held is left to whatever else takes it.
    const heard = vi.fn();
    window.addEventListener('keydown', heard);
    fireEvent.keyDown(document.body, { key: 'Escape' });
    window.removeEventListener('keydown', heard);
    expect(heard).toHaveBeenCalledTimes(1);
  });

  it('ends where the window loses the mouse, keeping what the last move did', async () => {
    const h = mount(); await settle();
    h.pointer('down', 1920, 810);
    h.pointer('move', 2160, 810); await settle();
    h.pointer('cancel', 2160, 810, { button: null, buttons: [] }); await settle();
    expect(h.applied()).toHaveLength(2);
    expect(h.applied()[1].gesture.end).toBe(true);
    expect(h.applied()[1].ops).toEqual(h.applied()[0].ops);
  });

  it('shows the path of a clip on a locked track, and refuses the drag as the Inspector refuses its edits', async () => {
    const h = mount({ locked: true }); await settle();
    expect(h.overlays()).toHaveLength(1);
    expect(h.sent('view.pointer')).toEqual([{ monitor: 'timeline', on: true, cursor: 'arrow' }]);
    expect(notice()).toBeNull();
    h.pointer('down', 1920, 810);
    h.pointer('move', 2160, 742.5); h.pointer('up', 2160, 742.5); await settle();
    expect(h.applied()).toEqual([]);
    expect(notice()).toBe('These clips are on locked tracks. Unlock the track to change them.');
    // No point is drawn as dragged.
    expect(h.overlays().every((each: { dots: { kind: string }[] }) => each.dots.every(dots => dots.kind !== 'active'))).toBe(true);
    // Its menu opens, with nothing in it to choose.
    h.pointer('down', 1920, 810, { button: 'right', buttons: ['right'] }); await settle();
    expect(item('path-linear')?.disabled).toBe(true);
    expect(item('path-smooth')?.disabled).toBe(true);
  });
});

describe('the menu of a point of the motion path', () => {
  const LINE = [key(50, 0), key(52, 0.25), key(54, 0.5)];
  const SMOOTH = [
    key(50, 0, { bezier: { out: [EASE_SHARE, 0.25 * EASE_SHARE], in: [EASE_SHARE, 0] } }),
    key(52, 0.25, { bezier: { out: [EASE_SHARE, 0], in: [EASE_SHARE, -0.25 * EASE_SHARE] } }),
  ];
  const right = { button: 'right', buttons: ['right'] } as Partial<ViewPointer>;

  it('opens at a point for the right button, with Linear checked for a point on lines, and makes it Smooth in one transaction', async () => {
    const h = mount({ clip: cut({ keys: LINE }, 0.25) }); await settle();
    // Off every point, the right button opens nothing.
    h.pointer('down', 1200, 810, right); await settle();
    expect(item('path-linear')).toBeNull();
    h.pointer('down', 1440, 810, right); await settle();
    expect(document.querySelector('[role="menu"]')?.getAttribute('aria-label')).toBe('Motion path point');
    expect(item('path-linear')?.dataset.checked).toBe('true');
    expect(item('path-smooth')?.dataset.checked).toBeUndefined();
    expect(item('path-smooth')?.getAttribute('aria-label')).toBe('Smooth');
    fireEvent.click(item('path-smooth')!); await settle();
    // The keys that differ, of the setting that has a key there: the one that is a number is not touched.
    expect(h.applied()).toEqual([{ revision: 17, ops: [{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'transform.x', at: 0, remove: 2, keys: SMOOTH }] }]);
    expect(item('path-smooth')).toBeNull();
    // No drag was begun by it.
    h.pointer('move', 1600, 900); await settle();
    expect(h.applied()).toHaveLength(1);
  });

  it('shows Smooth checked for a point that is, and makes it Linear on both settings', async () => {
    const smooth = { keys: [...SMOOTH, key(54, 0.5)] };
    const h = mount({ clip: cut(smooth, smooth) }); await settle();
    // The point at picture 52: a quarter of the width and of the height from the middle.
    h.pointer('down', 1440, 810, right); await settle();
    expect(item('path-smooth')?.dataset.checked).toBe('true');
    expect(item('path-linear')?.dataset.checked).toBeUndefined();
    fireEvent.click(item('path-linear')!); await settle();
    const linear = (path: string) => ({ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path, at: 0, remove: 2, keys: LINE.slice(0, 2) });
    expect(h.applied()).toEqual([{ revision: 17, ops: [linear('transform.x'), linear('transform.y')] }]);
  });

  it('sends nothing for the way that the point goes already', async () => {
    const h = mount({ clip: cut({ keys: LINE }, 0.25) }); await settle();
    h.pointer('down', 1440, 810, right); await settle();
    fireEvent.click(item('path-linear')!); await settle();
    expect(h.applied()).toEqual([]);
  });
});

describe('the on-screen Transform controls of the Edit page', () => {
  it('are off at first, and remembered by the page once they are turned', () => {
    const first = renderHook(() => useTransformControls());
    expect(first.result.current.on).toBe(false);
    act(() => first.result.current.toggle());
    expect(first.result.current.on).toBe(true);
    expect(localStorage.getItem(TRANSFORM_CONTROLS_KEY)).toBe('true');
    first.unmount();
    // The page left and come back to: they are as they were left.
    const again = renderHook(() => useTransformControls());
    expect(again.result.current.on).toBe(true);
    act(() => again.result.current.toggle());
    expect(again.result.current.on).toBe(false);
    expect(localStorage.getItem(TRANSFORM_CONTROLS_KEY)).toBe('false');
    again.unmount();
    expect(renderHook(() => useTransformControls()).result.current.on).toBe(false);
  });
});
