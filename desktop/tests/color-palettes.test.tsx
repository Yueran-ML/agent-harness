// @vitest-environment jsdom
/**
 * The Primaries and Curves palettes of the Color page (T17): each control sends the setting it
 * names, in the units the engine keeps, as one transaction for each gesture; the wheels as DaVinci
 * Resolve's (drag, Shift to jump, Ctrl for the master, double click for the colour); resets; the
 * orange dots; the points of the curves; ganged and not; keyframes edited at the playhead; Auto
 * Color from the figures of the frame.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Palettes } from '../src/studio/color/Palettes';
import {
  addPoint, autoColor, chromaOf, curveAt, fullCorrector, keyAt, removePoint, rgbOfChroma, shownValue, withValue, type FrameStatistics,
} from '../src/studio/color/corrector';
import { stepAtCursor } from '../src/studio/color/controls';
import { placeOf } from '../src/studio/color/useNodeEdit';
import { mergePatch, type Corrector, type GradeNode, type Target } from '../src/studio/engine';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-ring')) return rect(0, 0, 100, 100);
    if (this.classList.contains('color-curve-area')) return this.dataset.kind === 'custom' ? rect(0, 0, 200, 200) : rect(0, 0, 300, 100);
    return rect(0, 0, 600, 300);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const STATS: FrameStatistics = { percentiles: {
  red: { 1: 0.1, 50: 0.5, 99: 0.9 }, green: { 1: 0.05, 50: 0.45, 99: 0.85 }, blue: { 1: 0, 50: 0.4, 99: 0.8 }, luma: { 1: 0.05, 50: 0.45, 99: 0.85 },
} };

/** The palettes over a corrector whose params are `params` (as a patch of a new one), at `frame` of its keys; an engine that applies `set_node`. */
function mount(params: unknown = {}, given: { frame?: number; center?: 'curves' | 'qualifier'; kind?: GradeNode['kind']; stats?: FrameStatistics } = {}) {
  let frame = given.frame ?? 0;
  let node: GradeNode = given.kind === 'parallel_mixer'
    ? { id: 'n1', kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 2 } }
    : { id: 'n1', kind: 'corrector', label: '', enabled: true, params: mergePatch(fullCorrector({}), params) as Corrector };
  let revision = 1;
  const onFailure = vi.fn();
  const bridge = {
    call: vi.fn(async (method: string, sent?: Record<string, unknown>) => {
      if (method === 'transaction.apply') {
        const [op] = sent!.ops as { params: Record<string, unknown> }[];
        node = { ...node, params: mergePatch(node.params, op.params) } as GradeNode;
        revision += 1;
        return { ok: true as const, result: { revision, done: [op], next_id: 9, names: {}, clamped: [] } };
      }
      if (method === 'scopes.statistics') return { ok: true as const, result: given.stats ?? STATS };
      return { ok: true as const, result: {} };
    }),
  };
  const tree = () => <Palettes merged={false} left="primaries" center={given.center ?? 'curves'} last="left" right="scopes"
    onLeft={() => {}} onCenter={() => {}} onRight={() => {}} edited={{ bridge, revision, target: TARGET, node, frame, onFailure }} />;
  const view = render(tree());
  return {
    bridge, onFailure,
    // The transactions that are steps: an edit made at once, or the last of a drag (those sent as
    // it goes are drawn, and kept with the last as one step; see node-edit-gesture.test.tsx).
    sent: () => bridge.call.mock.calls
      .filter(([method, each]) => {
        const gesture = (each as { gesture?: { end?: boolean } }).gesture;
        return method === 'transaction.apply' && (gesture === undefined || gesture.end === true);
      })
      .map(([, each]) => each as { revision: number; ops: Record<string, unknown>[] }),
    params: () => (node.params as Corrector),
    /** The copy takes in what the engine did, as the project's would. */
    sync: () => view.rerender(tree()),
    at: (to: number) => { frame = to; view.rerender(tree()); },
  };
}
/** The params patch of the one transaction sent, checked to be one `set_node` of the node. */
const onlyPatch = (sent: { ops: Record<string, unknown>[] }[]) => {
  expect(sent).toHaveLength(1);
  expect(sent[0].ops).toHaveLength(1);
  expect(sent[0].ops[0]).toMatchObject({ op: 'set_node', grade: TARGET, id: 'n1' });
  return sent[0].ops[0].params;
};
const field = (path: string) => document.querySelector<HTMLElement>(`[data-field="${path}"]`)!;
const box = (path: string) => field(path).querySelector<HTMLInputElement>('.color-number-box')!;
const nameOf = (path: string) => field(path).querySelector<HTMLElement>('.color-number-name')!;
const wheel = (name: string) => document.querySelector<HTMLElement>(`[data-wheel="${name}"]`)!;
const ring = (name: string) => wheel(name).querySelector<HTMLElement>('[data-test="ring"]')!;
/** A drag from `from` to `to`, through the middle, ended there. */
function drag(element: Element, from: [number, number], to: [number, number], fields: Record<string, unknown> = {}, end = true) {
  fireEvent.pointerDown(element, { button: 0, pointerId: 1, clientX: from[0], clientY: from[1], ...fields });
  fireEvent.pointerMove(element, { pointerId: 1, clientX: (from[0] + to[0]) / 2, clientY: (from[1] + to[1]) / 2, ...fields });
  fireEvent.pointerMove(element, { pointerId: 1, clientX: to[0], clientY: to[1], ...fields });
  if (end) fireEvent.pointerUp(element, { pointerId: 1, clientX: to[0], clientY: to[1], ...fields });
}

describe('the Primaries palette', () => {
  it('drags a setting by its name as one step, shown at once and sent when let go, in the units the engine keeps', async () => {
    const palettes = mount();
    drag(nameOf('primaries.saturation'), [100, 0], [140, 0], {}, false);
    // 40 pixels at a quarter each, shown while it is dragged, and nothing sent.
    expect(box('primaries.saturation').value).toBe('60.00');
    expect(palettes.sent()).toHaveLength(0);
    fireEvent.pointerUp(nameOf('primaries.saturation'), { pointerId: 1, clientX: 140 });
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { saturation: 60 } });
    expect(palettes.sent()[0].revision).toBe(1);
    // Shown as sent until the copy has it, and from the copy then.
    expect(box('primaries.saturation').value).toBe('60.00');
    palettes.sync();
    expect(box('primaries.saturation').value).toBe('60.00');
  });

  it('sends each shared setting, and each number of a wheel, at its own path and scale', async () => {
    const palettes = mount();
    const cases: [string, number, Record<string, unknown>][] = [
      ['primaries.temperature', 30, { primaries: { temperature: 300 } }],
      ['primaries.tint', -20, { primaries: { tint: -10 } }],
      ['primaries.contrast', 20, { primaries: { contrast: 1.1 } }],
      ['primaries.pivot', -50, { primaries: { pivot: 0.335 } }],
      ['primaries.midtone_detail', 10, { primaries: { midtone_detail: 5 } }],
      ['primaries.color_boost', 10, { primaries: { color_boost: 5 } }],
      ['primaries.shadows', -10, { primaries: { shadows: -5 } }],
      ['primaries.highlights', 10, { primaries: { highlights: 5 } }],
      ['primaries.hue', 40, { primaries: { hue: 60 } }],
      ['primaries.lum_mix', -100, { primaries: { lum_mix: 50 } }],
      ['primaries.lift.r', 50, { primaries: { lift: { r: 0.1 } } }],
      ['primaries.gamma.y', -50, { primaries: { gamma: { y: -0.1 } } }],
      ['primaries.gain.b', 50, { primaries: { gain: { b: 1.2 } } }],
      ['primaries.offset.g', 50, { primaries: { offset: { g: 35 } } }],
    ];
    for (const [path, by, patch] of cases) {
      const before = palettes.sent().length;
      drag(box(path), [200, 0], [200 + by, 0]);
      await settle();
      expect(palettes.sent()).toHaveLength(before + 1);
      expect(palettes.sent().at(-1)!.ops[0].params).toEqual(patch);
      palettes.sync();
    }
    // A setting is held to its range: Lum Mix no further than 0.
    drag(box('primaries.lum_mix'), [200, 0], [-400, 0]);
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { lum_mix: 0 } });
  });

  it('moves the colour balance by as much as the pointer, wherever the drag starts, and keeps the luma', async () => {
    const palettes = mount();
    // 40 pixels right on a ring of 50, at half the pointer's way, toward blue.
    drag(ring('lift'), [90, 90], [130, 90]);
    await settle();
    const lift = (onlyPatch(palettes.sent()) as { primaries: { lift: Record<string, number> } }).primaries.lift;
    expect(Object.keys(lift).sort()).toEqual(['b', 'g', 'r']);
    expect(lift.b).toBeCloseTo(0.2, 2);
    expect(lift.r).toBeCloseTo(0, 3);
    expect(0.2126 * lift.r + 0.7152 * lift.g + 0.0722 * lift.b).toBeCloseTo(0, 2);
  });

  it('jumps the indicator to the pointer with Shift, even with a click that does not drag', async () => {
    const palettes = mount();
    fireEvent.pointerDown(ring('lift'), { button: 0, pointerId: 1, clientX: 50, clientY: 0, shiftKey: true });
    fireEvent.pointerUp(ring('lift'), { pointerId: 1, clientX: 50, clientY: 0, shiftKey: true });
    await settle();
    // The top of the ring is +Cr: red up, its edge where red moves by the reach of Lift less the share of blue's scale.
    const lift = (onlyPatch(palettes.sent()) as { primaries: { lift: Record<string, number> } }).primaries.lift;
    expect(lift.r).toBeCloseTo(0.424, 2);
    expect(lift.b).toBeCloseTo(0, 3);
    expect(lift.g).toBeLessThan(0);
    palettes.sync();
    const indicator = ring('lift').querySelector<HTMLElement>('[data-test="indicator"]')!;
    expect(parseFloat(indicator.style.left)).toBeCloseTo(50, 0);
    expect(parseFloat(indicator.style.top)).toBeCloseTo(0, 0);
  });

  it('turns the master with Ctrl in the ring, and with its own wheel: Y, R, G and B together', async () => {
    const palettes = mount({ primaries: { lift: { r: 0.05 } } });
    drag(ring('lift'), [50, 50], [100, 50], { ctrlKey: true });
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { lift: { y: 0.1, r: 0.15, g: 0.1, b: 0.1 } } });
    palettes.sync();
    drag(wheel('gain').querySelector('[data-test="master-wheel"]')!, [100, 0], [50, 0]);
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { gain: { y: 0.8, r: 0.8, g: 0.8, b: 0.8 } } });
    // Offset has no Y (p. 3067): its master moves R, G and B.
    palettes.sync();
    drag(wheel('offset').querySelector('[data-test="master-wheel"]')!, [100, 0], [150, 0]);
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { offset: { r: 35, g: 35, b: 35 } } });
    // Up and down on a master wheel do nothing, and nothing is sent.
    palettes.sync();
    const before = palettes.sent().length;
    drag(wheel('gamma').querySelector('[data-test="master-wheel"]')!, [100, 100], [100, 20]);
    await settle();
    expect(palettes.sent()).toHaveLength(before);
  });

  it('resets the colour of a ring with a double click, keeping its master; the wheel and the palette with their buttons', async () => {
    const palettes = mount({ primaries: { lift: { y: 0.3, r: 0.2, g: 0.1, b: 0 }, gain: { r: 1.2 }, saturation: 70 } });
    fireEvent.doubleClick(ring('lift'));
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { lift: { r: 0.114, g: 0.114, b: 0.114 } } });
    palettes.sync();
    fireEvent.click(wheel('lift').querySelector('[data-test="reset-wheel"]')!);
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { lift: { y: 0, r: 0, g: 0, b: 0 } } });
    palettes.sync();
    // A double click on the labels beneath the numbers resets the four (p. 3064), on a name its setting.
    fireEvent.doubleClick(wheel('gain').querySelector('[data-test="wheel-letters"]')!);
    await settle();
    palettes.sync();
    fireEvent.doubleClick(nameOf('primaries.saturation'));
    await settle();
    expect(palettes.sent().slice(-2).map(each => each.ops[0].params)).toEqual([
      { primaries: { gain: { y: 1, r: 1, g: 1, b: 1 } } }, { primaries: { saturation: 50 } },
    ]);
    palettes.sync();
    drag(nameOf('primaries.contrast'), [0, 0], [40, 0]);
    await settle();
    palettes.sync();
    fireEvent.click(screen.getAllByRole('button', { name: /Reset$/ })[0]);
    await settle();
    const reset = palettes.sent().at(-1)!.ops[0].params as { primaries: Record<string, unknown>; log: Record<string, unknown> };
    expect(reset.primaries).toMatchObject({ lift: { y: 0, r: 0, g: 0, b: 0 }, gain: { y: 1 }, offset: { r: 25 }, saturation: 50, contrast: 1, pivot: 0.435, lum_mix: 100 });
    expect(reset.log).toMatchObject({ shadow: { r: 0 }, low_range: 0.333, high_range: 0.55 });
  });

  it('writes into a number, steps a digit with Up and Down, and keeps what is written to the range', async () => {
    const palettes = mount();
    const lift = box('primaries.lift.r');
    fireEvent.doubleClick(lift);
    expect(lift.readOnly).toBe(false);
    fireEvent.change(lift, { target: { value: '0.25' } });
    lift.setSelectionRange(4, 4);
    fireEvent.keyDown(lift, { key: 'ArrowUp' });
    expect(lift.value).toBe('0.26');
    fireEvent.keyDown(lift, { key: 'Enter' });
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { lift: { r: 0.26 } } });
    palettes.sync();
    fireEvent.doubleClick(lift);
    fireEvent.change(lift, { target: { value: '5' } });
    fireEvent.keyDown(lift, { key: 'Enter' });
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { lift: { r: 1 } } });
    // Escape while writing, or while dragging, puts back what was there and sends nothing.
    palettes.sync();
    fireEvent.doubleClick(lift);
    fireEvent.change(lift, { target: { value: '0.5' } });
    fireEvent.keyDown(lift, { key: 'Escape' });
    drag(lift, [0, 0], [80, 0], {}, false);
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.pointerUp(lift, { pointerId: 1, clientX: 80 });
    await settle();
    expect(palettes.sent()).toHaveLength(2);
    expect(lift.value).toBe('1.00');
  });

  it('has the Bars and Log modes, Alt+Z between Wheels and Log, and Low and High Range in Log', async () => {
    const palettes = mount();
    fireEvent.click(screen.getByRole('radio', { name: 'Color Bars' }));
    drag(wheel('lift').querySelector('[data-test="bar-y"]')!, [0, 100], [0, 80]);
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { lift: { y: 0.2 } } });
    palettes.sync();
    fireEvent.keyDown(document.body, { key: 'z', code: 'KeyZ', altKey: true });
    expect(screen.getByRole('radio', { name: 'Log Wheels' }).getAttribute('aria-checked')).toBe('true');
    expect(['shadow', 'midtone', 'highlight', 'offset'].every(name => wheel(name) !== null)).toBe(true);
    expect(document.querySelector('[data-wheel="lift"]')).toBeNull();
    drag(box('log.low_range'), [0, 0], [20, 0]);
    await settle();
    palettes.sync();
    drag(ring('shadow'), [50, 50], [75, 50], { ctrlKey: true });
    await settle();
    expect(palettes.sent().slice(-2).map(each => each.ops[0].params)).toEqual([
      { log: { low_range: 0.373 } }, { log: { shadow: { r: 0.05, g: 0.05, b: 0.05 } } },
    ]);
    fireEvent.keyDown(document.body, { key: 'z', code: 'KeyZ', altKey: true });
    expect(screen.getByRole('radio', { name: 'Color Wheels' }).getAttribute('aria-checked')).toBe('true');
  });

  it('edits the keyframe at the playhead or at its left, and shows its value', async () => {
    const keys = { keys: [{ frame: 0, value: 40, interp: 'linear' }, { frame: 50, value: 60, interp: 'linear' }] };
    const palettes = mount({ primaries: { saturation: keys } }, { frame: 30 });
    // Between two keys: the one at the left, whose value the control shows (p. 3384), not the value in between.
    expect(box('primaries.saturation').value).toBe('40.00');
    expect(field('primaries.saturation').querySelector('[data-test="keyed"]')).not.toBeNull();
    drag(nameOf('primaries.saturation'), [0, 0], [40, 0]);
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: { saturation: { keys: [
      { frame: 0, value: 50, interp: 'linear' }, { frame: 50, value: 60, interp: 'linear' }] } } });
    palettes.sync();
    // On the last key, or beyond it: the last.
    palettes.at(80);
    expect(box('primaries.saturation').value).toBe('60.00');
    fireEvent.doubleClick(nameOf('primaries.saturation'));
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ primaries: { saturation: { keys: [
      { frame: 0, value: 50, interp: 'linear' }, { frame: 50, value: 50, interp: 'linear' }] } } });
    // A wheel of keys and values: each number goes where it is.
    palettes.sync();
    expect(field('primaries.contrast').querySelector('[data-test="keyed"]')).toBeNull();
  });

  it('sets Auto Color into Lift and Gain from the figures of the frame shown, and says when the frame is too flat', async () => {
    const palettes = mount();
    fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    await settle();
    expect(palettes.bridge.call).toHaveBeenCalledWith('scopes.statistics', { view: 'timeline' });
    const expected = autoColor(STATS, { lift: [0, 0, 0], gain: [1, 1, 1] })!;
    expect(onlyPatch(palettes.sent())).toEqual({ primaries: {
      lift: { r: expected.lift[0], g: expected.lift[1], b: expected.lift[2] }, gain: { r: expected.gain[0], g: expected.gain[1], b: expected.gain[2] },
    } });
    // Alt+Shift+C does the same.
    palettes.sync();
    fireEvent.keyDown(document.body, { key: 'C', code: 'KeyC', altKey: true, shiftKey: true });
    await settle();
    expect(palettes.bridge.call.mock.calls.filter(([method]) => method === 'scopes.statistics')).toHaveLength(2);
    cleanup();
    const flat = mount({}, { stats: { percentiles: { ...STATS.percentiles, red: { 1: 0.5, 50: 0.5, 99: 0.505 } } } });
    fireEvent.click(screen.getByRole('button', { name: 'Auto Color' }));
    await settle();
    expect(flat.sent()).toHaveLength(0);
    expect(screen.getByRole('status').textContent).toBe('Auto Color found too little range in this frame to balance it.');
  });

  it('keeps the pickers of the viewer off where no viewer hands them its pointer, and says how they are used', () => {
    // The Color page gives them the viewer (color-pickers.test.tsx); the palettes alone have none.
    mount();
    for (const test of ['picker-black', 'picker-white', 'curve-picker']) {
      const button = document.querySelector<HTMLButtonElement>(`[data-test="${test}"]`)!;
      expect(button.disabled).toBe(true);
      expect(button.title).toContain('Click on the viewer');
    }
  });

  it('shows an orange dot on the palettes in which the current node is adjusted, and none for another kind of node', async () => {
    const plain = mount();
    expect(document.querySelectorAll('[data-test="changed-dot"]')).toHaveLength(0);
    drag(nameOf('primaries.saturation'), [0, 0], [40, 0]);
    await settle();
    plain.sync();
    const dotted = () => [...document.querySelectorAll('[data-test="changed-dot"]')].map(dot => dot.closest<HTMLElement>('[data-palette]')!.dataset.palette);
    expect(dotted()).toEqual(['primaries']);
    cleanup();
    mount({ log: { high_range: 0.6 }, hsl: { hue_vs_sat: [[0.5, 0.2]] } });
    expect(dotted()).toEqual(['primaries', 'curves']);
    cleanup();
    mount({ soft_clip: { high: { r: 90 } } });
    expect(dotted()).toEqual(['curves']);
    cleanup();
    mount({}, { kind: 'parallel_mixer' });
    expect(dotted()).toEqual([]);
    expect(document.querySelectorAll('[data-test="no-corrector"]')).toHaveLength(2);
  });
});

describe('the Curves palette', () => {
  const editor = () => document.querySelector<HTMLElement>('[data-test="curve-editor"]')!;
  const click = (x: number, y: number, fields = {}) => {
    fireEvent.pointerDown(editor(), { button: 0, pointerId: 1, clientX: x, clientY: y, ...fields });
    fireEvent.pointerUp(editor(), { pointerId: 1, clientX: x, clientY: y, ...fields });
  };
  const ganged = (points: number[][]) => ({ curves: { y: { points }, r: { points }, g: { points }, b: { points } } });

  it('adds a point where it is clicked, ganged on the four curves, and one dragged on is one step', async () => {
    const palettes = mount();
    // (100, 60) of 200 × 200: input 0.5, output 0.7.
    click(100, 60);
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual(ganged([[0, 0], [0.5, 0.7], [1, 1]]));
    palettes.sync();
    drag(editor(), [50, 100], [50, 120]);
    await settle();
    expect(palettes.sent()).toHaveLength(2);
    expect(palettes.sent()[1].ops[0].params).toEqual(ganged([[0, 0], [0.25, 0.4], [0.5, 0.7], [1, 1]]));
  });

  it('adds a point on the curve with Shift, moves one, puts it on the diagonal with Alt, and takes one away with a right click', async () => {
    const palettes = mount({ curves: { y: { points: [[0, 0], [0.5, 0.6], [1, 1]] }, r: { points: [[0, 0], [0.5, 0.6], [1, 1]] },
      g: { points: [[0, 0], [0.5, 0.6], [1, 1]] }, b: { points: [[0, 0], [0.5, 0.6], [1, 1]] } } });
    click(50, 20, { shiftKey: true });
    await settle();
    const y = curveAt([[0, 0], [0.5, 0.6], [1, 1]], 0.25, 'custom');
    expect(onlyPatch(palettes.sent())).toEqual(ganged([[0, 0], [0.25, Math.round(y * 1000) / 1000], [0.5, 0.6], [1, 1]]));
    palettes.sync();
    // The point at (0.5, 0.6) is at (100, 80); dragged with Alt it sits on the diagonal, which is drawn while it does.
    drag(editor(), [100, 80], [110, 40], { altKey: true }, false);
    expect(document.querySelector('[data-test="diagonal-guide"]')).not.toBeNull();
    fireEvent.pointerUp(editor(), { pointerId: 1, clientX: 110, clientY: 40 });
    await settle();
    expect(palettes.sent()[1].ops[0].params).toEqual(ganged([[0, 0], [0.25, Math.round(y * 1000) / 1000], [0.55, 0.55], [1, 1]]));
    palettes.sync();
    fireEvent.contextMenu(editor(), { clientX: 110, clientY: 90 });
    // The black point stays.
    fireEvent.contextMenu(editor(), { clientX: 0, clientY: 200 });
    await settle();
    expect(palettes.sent()).toHaveLength(3);
    expect(palettes.sent()[2].ops[0].params).toEqual(ganged([[0, 0], [0.25, Math.round(y * 1000) / 1000], [1, 1]]));
  });

  it('ungangs when a channel is chosen, edits that curve alone, and gangs again with the Gang button', async () => {
    const palettes = mount();
    fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ curves: { ganged: false } });
    palettes.sync();
    click(100, 100);
    await settle();
    expect(palettes.sent()[1].ops[0].params).toEqual({ curves: { r: { points: [[0, 0], [0.5, 0.5], [1, 1]] } } });
    palettes.sync();
    // The four are drawn, the one chosen on top.
    expect([...document.querySelectorAll('.color-curve-line')].map(line => (line as SVGElement).dataset.curve)).toEqual(['y', 'g', 'b', 'r']);
    drag(box('curves.g.intensity'), [0, 0], [-40, 0]);
    await settle();
    palettes.sync();
    fireEvent.click(screen.getByRole('button', { name: 'Gang Custom Curves' }));
    await settle();
    expect(palettes.sent().slice(2).map(each => each.ops[0].params)).toEqual([
      { curves: { g: { intensity: 80 } } }, { curves: { ganged: true } },
    ]);
  });

  it('resets a channel beside its intensity, and every curve with Reset Custom Curve', async () => {
    const palettes = mount({ curves: { ganged: false, b: { points: [[0, 0], [0.4, 0.6], [1, 1]], intensity: 70 } } });
    fireEvent.click(document.querySelector('[data-test="reset-b"]')!);
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ curves: { b: { points: [[0, 0], [1, 1]], intensity: 100 } } });
    palettes.sync();
    click(100, 50);
    await settle();
    palettes.sync();
    fireEvent.click(screen.getByRole('button', { name: /Reset Custom Curve/ }));
    await settle();
    const all = palettes.sent()[2].ops[0].params as { curves: Record<string, unknown> };
    expect(Object.keys(all.curves).sort()).toEqual(['b', 'g', 'r', 'y']);
    expect(all.curves.y).toEqual({ points: [[0, 0], [1, 1]], intensity: 100 });
  });

  it('puts Default Anchors on the curve, and copies a curve to another channel', async () => {
    const palettes = mount({ curves: { ganged: false } });
    fireEvent.click(screen.getByRole('button', { name: 'Green' }));
    fireEvent.change(document.querySelector('[data-test="curve-options"]')!, { target: { value: 'anchors' } });
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ curves: { g: { points: [[0, 0], [0.25, 0.25], [0.5, 0.5], [0.75, 0.75], [1, 1]] } } });
    palettes.sync();
    fireEvent.change(document.querySelector('[data-test="curve-options"]')!, { target: { value: 'b' } });
    await settle();
    expect(palettes.sent()[1].ops[0].params).toEqual({ curves: { b: { points: [[0, 0], [0.25, 0.25], [0.5, 0.5], [0.75, 0.75], [1, 1]] } } });
  });

  it('sets Soft Clip ganged on the 10-bit scale, and each channel apart once unganged', async () => {
    const palettes = mount();
    expect(box('soft_clip.high.r').value).toBe('1023');
    fireEvent.doubleClick(box('soft_clip.high.r'));
    fireEvent.change(box('soft_clip.high.r'), { target: { value: '1000' } });
    fireEvent.keyDown(box('soft_clip.high.r'), { key: 'Enter' });
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ soft_clip: { high: { r: 97.752, g: 97.752, b: 97.752 } } });
    palettes.sync();
    fireEvent.click(document.querySelector('[data-test="gang-soft-clip"]')!);
    drag(box('soft_clip.low_soft.g'), [0, 0], [40, 0]);
    await settle();
    expect(palettes.sent()[1].ops[0].params).toEqual({ soft_clip: { low_soft: { g: 10 } } });
  });

  it('edits the HSL curves: a point, the six vectors of the curves of hue, and the Input and Output of the point chosen', async () => {
    const palettes = mount();
    fireEvent.click(screen.getByRole('radio', { name: 'Hue vs Sat' }));
    // (100, 50) of 300 × 100: a third of the hues, out 0.
    click(100, 50);
    await settle();
    expect(onlyPatch(palettes.sent())).toEqual({ hsl: { hue_vs_sat: [[0.333, 0]] } });
    palettes.sync();
    // The point chosen: its hue in degrees, its saturation as a gain.
    expect(box('hsl.hue_vs_sat.input').value).toBe('119.88');
    expect(box('hsl.hue_vs_sat.output').value).toBe('1.00');
    drag(box('hsl.hue_vs_sat.output'), [0, 0], [50, 0]);
    await settle();
    expect(palettes.sent()[1].ops[0].params).toEqual({ hsl: { hue_vs_sat: [[0.333, 0.2]] } });
    palettes.sync();
    fireEvent.click(screen.getByRole('button', { name: 'Red' }));
    await settle();
    const red = palettes.sent()[2].ops[0].params as { hsl: { hue_vs_sat: number[][] } };
    expect(red.hsl.hue_vs_sat.map(([x]) => x)).toEqual([0, 0.083, 0.333, 0.917]);
    palettes.sync();
    // Right click takes a point away; a curve of hue keeps none of its own.
    fireEvent.contextMenu(editor(), { clientX: 100, clientY: 40 });
    await settle();
    expect(palettes.sent()[3].ops[0].params).toEqual({ hsl: { hue_vs_sat: red.hsl.hue_vs_sat.filter(([x]) => x !== 0.333) } });
    // Hue vs Hue shows its Output as a rotation in degrees; Lum vs Sat has no vector patches.
    fireEvent.click(screen.getByRole('radio', { name: 'Lum vs Sat' }));
    expect(document.querySelector('.color-vector')).toBeNull();
    fireEvent.click(screen.getByRole('radio', { name: 'Hue vs Hue' }));
    click(150, 25);
    await settle();
    palettes.sync();
    expect(box('hsl.hue_vs_hue.output').value).toBe('90.00');
  });
});

describe('the rules of the palettes', () => {
  it('put an edit into the keyframe at the playhead or at its left (p. 3391)', () => {
    const keys = [{ frame: 10, value: 1, interp: 'linear' as const }, { frame: 20, value: 2, interp: 'hold' as const }, { frame: 30, value: 3, interp: 'linear' as const }];
    expect([0, 10, 15, 20, 29, 30, 99].map(frame => keyAt(keys, frame))).toEqual([0, 0, 0, 1, 1, 2, 2]);
    expect(shownValue({ keys }, 25)).toBe(2);
    expect(shownValue(7, 25)).toBe(7);
    expect(withValue(7, 25, 9)).toBe(9);
    expect(withValue({ keys }, 25, 9)).toEqual({ keys: [keys[0], { ...keys[1], value: 9 }, keys[2]] });
  });

  it('draw a curve through its points, go on along its ends where custom, flat where HSL, round where of hue', () => {
    const points: [number, number][] = [[0, 0], [0.3, 0.5], [1, 1]];
    for (const [x, y] of points) expect(curveAt(points, x, 'custom')).toBeCloseTo(y, 6);
    expect(curveAt([[0.2, 0.1], [0.8, 0.4]], 0.9, 'flat')).toBeCloseTo(0.4, 6);
    expect(curveAt([[0.2, 0.1], [0.8, 0.4]], 0.9, 'custom')).toBeCloseTo(0.45, 6);
    expect(curveAt([], 0.4, 'round')).toBe(0);
    // A point near red raises both ends together.
    expect(curveAt([[0.02, 0.5], [0.5, 0]], 0.99, 'round')).toBeGreaterThan(0.4);
    expect(addPoint(points, [0.3, 0.2], true)).toBeNull();
    expect(addPoint([[0.1, 0], [0.9, 1]], [0.05, 0.2], true)).toBeNull();
    expect(removePoint(points, 0, true)).toBeNull();
    expect(removePoint(points, 1, true)).toEqual([[0, 0], [1, 1]]);
    expect(removePoint([[0.5, 0.2]], 0, false)).toEqual([]);
  });

  it('move the indicator of a ring as the vectorscope lays the colours, keeping the luma', () => {
    const [r, g, b] = rgbOfChroma(0.1, -0.05);
    expect(0.2126 * r + 0.7152 * g + 0.0722 * b).toBeCloseTo(0, 10);
    const [cb, cr] = chromaOf(r, g, b);
    expect(cb).toBeCloseTo(0.1, 10);
    expect(cr).toBeCloseTo(-0.05, 10);
    // Red is up and a little left, at 103°.
    const [rb, rr] = chromaOf(1, 0, 0);
    expect((Math.atan2(rr, rb) * 180) / Math.PI).toBeCloseTo(103.4, 0);
  });

  it('balance the blacks and the whites with Auto Color, after what the node does already', () => {
    const balanced = autoColor(STATS, { lift: [0, 0, 0], gain: [1, 1, 1] })!;
    // Each channel's 1% and 99% go to the luma-weighted mean of the three.
    const black = 0.2126 * 0.1 + 0.7152 * 0.05 + 0.0722 * 0;
    const white = 0.2126 * 0.9 + 0.7152 * 0.85 + 0.0722 * 0.8;
    const through = (x: number, i: number) => balanced.gain[i] * (x + balanced.lift[i] * (1 - x));
    expect(through(0.1, 0)).toBeCloseTo(black, 2);
    expect(through(0.85, 1)).toBeCloseTo(white, 2);
    expect(through(0.8, 2)).toBeCloseTo(white, 2);
    // Put after a gain the node has: the result is that gain, then the balance.
    const after = autoColor(STATS, { lift: [0, 0, 0], gain: [2, 1, 1] })!;
    const [a, shift] = [balanced.gain[0] * (1 - balanced.lift[0]), balanced.gain[0] * balanced.lift[0]];
    expect(after.gain[0]).toBeCloseTo(2 * a + shift, 2);
    expect(after.lift[0]).toBeCloseTo(shift / (2 * a + shift), 2);
    expect(autoColor({ percentiles: { ...STATS.percentiles, blue: { 1: 0.3, 50: 0.3, 99: 0.31 } } }, { lift: [0, 0, 0], gain: [1, 1, 1] })).toBeNull();
  });

  it('step the digit at the left of the cursor, or the last where all is chosen', () => {
    expect(stepAtCursor('12.345', 2, false, 3)).toBe(1);
    expect(stepAtCursor('12.345', 1, false, 3)).toBe(10);
    expect(stepAtCursor('12.345', 4, false, 3)).toBe(0.1);
    expect(stepAtCursor('12.345', 6, false, 3)).toBe(0.001);
    expect(stepAtCursor('12.345', 0, true, 3)).toBe(0.001);
  });

  it('name the grade of each level and count its keys from the clip, or from the timeline for its own', () => {
    const clip = { id: 'c1', start: 100, group: 'k1' };
    expect(placeOf('clip', 't1', clip, 130)).toEqual({ target: { level: 'clip', timeline: 't1', clip: 'c1' }, frame: 30 });
    expect(placeOf('pre_clip', 't1', clip, 130)).toEqual({ target: { level: 'pre_clip', group: 'k1' }, frame: 30 });
    expect(placeOf('timeline', 't1', clip, 130)).toEqual({ target: { level: 'timeline', timeline: 't1' }, frame: 130 });
    expect(placeOf('post_clip', 't1', { ...clip, group: null }, 130)).toBeNull();
  });
});
