// @vitest-environment jsdom
/**
 * The on-screen Transform controls of the Edit page's timeline viewer (`src/studio/pages/EditPage`;
 * DaVinci Resolve Reference Manual 20, pp. 1106–1108): the button in the title bar of the viewer,
 * off at first, that has the engine draw the motion path of the clip chosen, and that the page
 * remembers.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import { EditPage } from '../src/studio/pages/EditPage';
import type { MonitorControls } from '../src/studio/useMonitor';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';

beforeEach(() => { likeABrowser(); localStorage.clear(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); localStorage.clear(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const node = (id: string) => document.querySelector(`[data-clip="${id}"]`)!;
const button = () => document.querySelector<HTMLButtonElement>('[data-test="transform-controls"]')!;
/** Choose a clip on the timeline, as a press on it does. */
function choose(id: string) {
  const event = { button: 0, clientX: 250, clientY: 30, pointerId: 1 };
  fireEvent.pointerDown(node(id), event); fireEvent.pointerUp(node(id), event);
}

function setup() {
  // A clip of 100 frames that goes from the middle of the picture to its right edge.
  const moving = clip('video', 0, 100, { transform: { x: { keys: [{ frame: 0, value: 0, interp: 'linear' }, { frame: 99, value: 0.5, interp: 'linear' }] }, y: 0, scale: 1, rotation: 0, opacity: 1 } });
  const selected: Timeline = timeline([moving]);
  const monitor = { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
  const call = vi.fn<EngineBridge['call']>(async method => ({ ok: true, result: method === 'transport.load' ? monitor.state : method === 'audio.meters' ? { timeline: 't1', strips: [], loudness: {}, lost: 0 } : {} }));
  const bridge = { call, onEvent: () => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const props = { bridge, native: true, session: 1, revision: 7, pool: <div>Pool</div>, media: [media], timelines: [selected], selected, onSelect: vi.fn(), timelineMonitor: monitor, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const draw = () => <div data-test="studio"><EditPage {...props} source={{ ...monitor, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={vi.fn()} /></div>;
  const view = render(draw());
  const sent = (method: string) => call.mock.calls.filter(([each]) => each === method).map(([, params]) => params);
  return { view, draw, call, sent };
}

it('has a button for the Transform controls in the title bar of the timeline viewer, off at first, which shows the motion path of the clip chosen', async () => {
  const app = setup(); await settle();
  expect(button().closest('[data-monitor]')?.getAttribute('data-monitor')).toBe('timeline');
  expect(button().textContent).toBe('Transform controls');
  expect(button().getAttribute('aria-pressed')).toBe('false');
  expect(button().disabled).toBe(false);
  // Off, a clip whose position has keys is chosen and nothing is drawn.
  choose('video'); await settle();
  expect(app.sent('view.overlay')).toEqual([]);
  expect(app.sent('view.pointer')).toEqual([]);
  // On: the path of the clip, with the place at the playhead, and the pointer on the picture.
  fireEvent.click(button()); await settle();
  expect(button().getAttribute('aria-pressed')).toBe('true');
  const drawn = app.sent('view.overlay');
  expect(drawn).toHaveLength(1);
  const overlay = drawn[0]!.overlay as { canvas: number[]; lines: number[][][]; dots: { kind: string; at: number[][] }[] };
  expect(overlay.canvas).toEqual([1920, 1080]);
  expect(overlay.lines).toHaveLength(1);
  expect(overlay.lines[0]).toHaveLength(100);
  expect(overlay.dots.map(each => each.kind)).toEqual(['tick', 'key', 'current']);
  expect(overlay.dots[1].at).toEqual([[960, 540], [1920, 540]]);
  expect(overlay.dots[2].at).toEqual([[960, 540]]);
  expect(app.sent('view.pointer')).toEqual([{ monitor: 'timeline', on: true, cursor: 'arrow' }]);
  expect(document.querySelector('.edit-monitors')?.getAttribute('data-motion-path')).toBe('true');
  // Off again: nothing is drawn, and the pointer is the page's.
  fireEvent.click(button()); await settle();
  expect(button().getAttribute('aria-pressed')).toBe('false');
  expect(app.sent('view.overlay').at(-1)).toEqual({ monitor: 'timeline', overlay: null });
  expect(app.sent('view.pointer').at(-1)).toEqual({ monitor: 'timeline', on: false });
});

it('remembers the Transform controls from one visit of the page to the next', async () => {
  const app = setup(); await settle();
  fireEvent.click(button()); await settle();
  app.view.unmount();
  const again = setup(); await settle();
  expect(button().getAttribute('aria-pressed')).toBe('true');
  // On, with no clip chosen there is no path, and nothing is drawn.
  expect(again.sent('view.overlay')).toEqual([]);
  choose('video'); await settle();
  expect(again.sent('view.overlay')).toHaveLength(1);
});
