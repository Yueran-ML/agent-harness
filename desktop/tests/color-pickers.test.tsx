// @vitest-environment jsdom
/**
 * The pickers of colour of the Color page on its viewer (T17, T21), the Qualifier palette and the
 * Key palette (T21, T22): what a click or a drag on the picture makes of the colours that go into
 * the node, sent as the palettes send their settings, one step to undo.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Palettes } from '../src/studio/color/Palettes';
import { QualifierPalette, qualifierAt, qualifierEdits } from '../src/studio/color/Qualifier';
import { KeyPalette, keyChanged } from '../src/studio/color/KeyPalette';
import { PickerContext, SampleDrag, pickerInForce, usePickerHub, type Rgb, type ViewerPicker } from '../src/studio/color/pickers';
import { PICK, addRange, arcOf, measure, pickRange, qualifierAfter, softenRange, subtractRange } from '../src/studio/color/qualifierPick';
import { blackPointEdits, customPickPoints, hslPickPoints, whitePointEdits } from '../src/studio/color/pickMaths';
import { fullCorrector } from '../src/studio/color/corrector';
import { mergePatch, newQualifier, type Corrector, type GradeNode, type Qualifier, type Target, type ViewPointer, type ViewerTool } from '../src/studio/engine';
import { likeABrowser } from './mounted';

const rect = (left: number, top: number, w: number, h: number) => ({ left, top, width: w, height: h, right: left + w, bottom: top + h, x: left, y: top, toJSON() {} });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.classList.contains('color-qualifier-bar')) return rect(0, 0, 200, 14);
    return rect(0, 0, 600, 300);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const TARGET: Target = { level: 'clip', timeline: 't1', clip: 'c1' };
const close = (value: number, wanted: number, within = 1e-3) => Math.abs(value - wanted) <= within;
const byTest = (name: string) => document.querySelector<HTMLElement>(`[data-test="${name}"]`)!;

describe('what a qualifier reads of a colour', () => {
  it('reads hue, saturation and luma as the engine does, each of 100', () => {
    const red = measure([1, 0, 0]);
    expect(red.hue).toBeCloseTo(0, 6);
    expect(red.sat).toBe(100);
    expect(red.lum).toBeCloseTo(21.26, 6);
    expect(measure([0, 1, 0]).hue).toBeCloseTo(100 / 3, 4);
    expect(measure([0, 0, 1]).hue).toBeCloseTo(200 / 3, 4);
    // A grey has no hue and no saturation; luma and R, G, B are kept to 0..1.
    expect(measure([0.5, 0.5, 0.5])).toMatchObject({ hue: 0, sat: 0, lum: 50 });
    expect(measure([1.5, -0.2, 0.3])).toMatchObject({ red: 100, green: 0, blue: 30 });
  });

  it('finds the smallest arc of hue that holds them, going round', () => {
    expect(arcOf([10, 20, 15])).toEqual({ start: 10, length: 10 });
    // Either side of red: the arc goes round through 0.
    expect(arcOf([95, 3])).toEqual({ start: 95, length: 8 });
  });
});

describe('the Selection Range tools', () => {
  const orange: Rgb = [0.8, 0.4, 0.1];
  const seen = measure(orange);

  it('makes a range about the colour clicked, keeping the mode, its switches and Matte Finesse', () => {
    const base = { ...newQualifier(), invert: true, finesse: { ...newQualifier().finesse, blur_radius: 30 } };
    const q = pickRange([orange], base);
    expect(close(q.hue.center as number, seen.hue)).toBe(true);
    expect(q.hue.width).toBe(PICK.hueWidth);
    expect(q.hue.soft).toBe(PICK.hueSoft);
    expect(close(q.sat.low as number, seen.sat - PICK.margin)).toBe(true);
    expect(close(q.lum.high as number, seen.lum + PICK.margin)).toBe(true);
    expect([q.sat.low_soft, q.lum.high_soft]).toEqual([PICK.soft, PICK.soft]);
    expect(q.invert).toBe(true);
    expect(q.finesse.blur_radius).toBe(30);
    // A node with none gets a new one; a grey takes every hue.
    expect(pickRange([[0.4, 0.4, 0.4]], null).hue.width).toBe(100);
    // RGB: ranges of red, green and blue; LUM: luma alone.
    const rgb = pickRange([orange], { ...newQualifier(), mode: 'rgb' });
    expect(close(rgb.red.low as number, 80 - PICK.margin)).toBe(true);
    expect(rgb.hue).toEqual(newQualifier().hue);
    const lum = pickRange([orange], { ...newQualifier(), mode: 'lum' });
    expect(lum.sat).toEqual(newQualifier().sat);
    expect(close(lum.lum.low as number, seen.lum - PICK.margin)).toBe(true);
  });

  it('takes a drag as the colours it went over', () => {
    const q = pickRange([[0.2, 0.2, 0.2], [0.6, 0.6, 0.6]], { ...newQualifier(), mode: 'lum' });
    expect(close(q.lum.low as number, 20 - PICK.margin)).toBe(true);
    expect(close(q.lum.high as number, 60 + PICK.margin)).toBe(true);
  });

  it('grows a range to take a colour, on the side nearer it, and leaves one it holds', () => {
    const q = pickRange([orange], null);
    const yellower = measure([0.8, 0.6, 0.1]);
    const grown = addRange([[0.8, 0.6, 0.1]], q);
    const left = (grown.hue.center as number) - (grown.hue.width as number) / 2;
    const right = (grown.hue.center as number) + (grown.hue.width as number) / 2;
    expect(close(left, (q.hue.center as number) - (q.hue.width as number) / 2)).toBe(true);
    expect(close(right, yellower.hue + PICK.margin)).toBe(true);
    expect(addRange([orange], q)).toEqual(q);
  });

  it('leaves a colour out by the one edge that has least to move', () => {
    const q: Qualifier = { ...newQualifier(), mode: 'lum', lum: { on: true, low: 20, high: 80, low_soft: 5, high_soft: 5 } };
    // 70 is nearer the top: High comes down below it, beyond its softness.
    const cut = subtractRange([[0.7, 0.7, 0.7]], q);
    expect(cut.lum.low).toBe(20);
    expect(close(cut.lum.high as number, 70 - 5 - PICK.margin)).toBe(true);
    // A colour already left out changes nothing.
    expect(subtractRange([[0.05, 0.05, 0.05]], q)).toEqual(q);
  });

  it('grows or shrinks the softness on the side of a colour beyond the core', () => {
    const q: Qualifier = { ...newQualifier(), mode: 'lum', lum: { on: true, low: 40, high: 60, low_soft: 2, high_soft: 2 } };
    const soft = softenRange([[0.7, 0.7, 0.7]], q, true);
    expect(close(soft.lum.high_soft as number, 10 + PICK.margin)).toBe(true);
    expect(soft.lum.low_soft).toBe(2);
    const hard = softenRange([[0.41, 0.41, 0.41], [0.65, 0.65, 0.65]], { ...q, lum: { ...q.lum, high_soft: 20 } }, false);
    expect(close(hard.lum.high_soft as number, 5 - PICK.margin)).toBe(true);
    // Alt takes away with the Eyedropper and Add.
    expect(qualifierAfter('add', [[0.7, 0.7, 0.7]], { ...q, lum: { ...q.lum, high: 80 } }, true).lum.high).toBeLessThan(80);
  });
});

describe('the edits of a qualifier', () => {
  it('give the qualifier whole to a node that has none, and each number that changed after, into its key', () => {
    const next = { ...newQualifier(), hue: { ...newQualifier().hue, center: 10 } };
    expect(qualifierEdits(false, newQualifier(), next)).toEqual([['qualifier', { whole: next }]]);
    const keyed: Qualifier = { ...newQualifier(), hue: { ...newQualifier().hue, center: { keys: [{ frame: 0, value: 30, interp: 'linear' }, { frame: 10, value: 40, interp: 'linear' }] } } };
    const shown = qualifierAt(keyed, 12);
    expect(shown.hue.center).toBe(40);
    expect(qualifierEdits(true, shown, { ...shown, hue: { ...shown.hue, center: 45 }, mode: 'rgb' }))
      .toEqual([['qualifier.mode', { whole: 'rgb' }], ['qualifier.hue.center', 45]]);
  });
});

describe('the pickers of Primaries and Curves', () => {
  it('make the colour black by Lift and white by Gain', () => {
    const neutral = { y: 0, r: 0, g: 0, b: 0 };
    const black = Object.fromEntries(blackPointEdits([0.1, 0.2, 0.05], neutral));
    expect(black['primaries.lift.r']).toBeCloseTo(-0.1 / 0.9, 3);
    expect(black['primaries.lift.g']).toBeCloseTo(-0.25, 3);
    // Lift of x is x + lift (1 - x): 0 at the colour.
    expect(0.1 + (black['primaries.lift.r'] as number) * 0.9).toBeCloseTo(0, 2);
    const white = Object.fromEntries(whitePointEdits([0.8, 0.5, 0.25], neutral, neutral, { y: 1, r: 1, g: 1, b: 1 }));
    expect(white['primaries.gain.r']).toBeCloseTo(1.25, 3);
    expect(white['primaries.gain.g']).toBeCloseTo(2, 3);
    expect(white['primaries.gain.b']).toBeCloseTo(4, 3);
    // Lift and Gamma as they are: a Gamma of 0.5 is an exponent of 1/2.
    const bent = Object.fromEntries(whitePointEdits([0.25, 0.25, 0.25], neutral, { y: 0.5, r: 0.5, g: 0.5, b: 0.5 }, { y: 1, r: 1, g: 1, b: 1 }));
    expect(bent['primaries.gain.r']).toBeCloseTo(2, 3);
  });

  it('put a point on the custom curve at the colour, on the curve as it is', () => {
    const diagonal: [number, number][] = [[0, 0], [1, 1]];
    expect(customPickPoints(diagonal, [0.5, 0.25, 0.1], 'r')).toEqual([[0, 0], [0.5, 0.5], [1, 1]]);
    const luma = 0.2126 * 0.5 + 0.7152 * 0.25 + 0.0722 * 0.1;
    expect(customPickPoints(diagonal, [0.5, 0.25, 0.1], 'y')![1][0]).toBeCloseTo(luma, 3);
  });

  it('put three points on an HSL curve about the colours, going round for hue', () => {
    const green: Rgb = [0, 1, 0];
    const points = hslPickPoints('hue_vs_sat', [], [green]);
    expect(points.map(([x]) => x)).toEqual([0.25, 0.333, 0.417]);
    expect(points.every(([, y]) => y === 0)).toBe(true);
    const lum = hslPickPoints('lum_vs_sat', [], [[0.2, 0.2, 0.2], [0.6, 0.6, 0.6]]);
    expect(lum.map(([x]) => x)).toEqual([0.158, 0.4, 0.642]);
  });
});

describe('a drag of a picker', () => {
  it('asks one sample at a time, the latest point waiting, and ends when the last has come', async () => {
    const asked: [number, number][] = [];
    const answers: ((rgb: Rgb) => void)[] = [];
    const told: [number, boolean][] = [];
    const drag = new SampleDrag((x, y) => { asked.push([x, y]); return new Promise(done => answers.push(done)); },
      (samples, ended) => told.push([samples.length, ended]), () => {});
    drag.at(1, 1); drag.at(2, 2); drag.at(3, 3);
    expect(asked).toEqual([[1, 1]]);
    answers[0]([0.1, 0.1, 0.1]); await Promise.resolve(); await Promise.resolve();
    expect(asked).toEqual([[1, 1], [3, 3]]);
    drag.release();
    answers[1]([0.2, 0.2, 0.2]); await Promise.resolve(); await Promise.resolve();
    expect(told).toEqual([[1, false], [2, false], [2, true]]);
    expect(drag.done).toBe(true);
  });

  it('is in force when armed, else the palette picker while the viewer control is Qualifier', () => {
    const one: ViewerPicker = { cursor: 'crosshair', onPointer: () => {} };
    const two: ViewerPicker = { cursor: 'crosshair', onPointer: () => {} };
    expect(pickerInForce({ armed: null, palette: one }, 'qualifier')).toBe(one);
    expect(pickerInForce({ armed: null, palette: one }, 'window')).toBeNull();
    expect(pickerInForce({ armed: two, palette: one }, 'window')).toBe(two);
  });
});

/** A pointer event of the viewer, at a point of the picture. */
const pointer = (kind: ViewPointer['kind'], x: number, y = 10, fields: Partial<ViewPointer> = {}): ViewPointer => ({
  kind, monitor: 'timeline', button: kind === 'move' ? null : 'left', buttons: kind === 'up' ? [] : ['left'], x, y, inside: true,
  window: { x, y }, shift: false, ctrl: false, alt: false, ...fields,
});

/**
 * The palettes of the Color page as it gives them the viewer: a node whose params are `params`;
 * `view.sample` answers the colour `colourAt(x)` for the node's input.
 */
function mount(params: unknown, given: { center?: 'curves' | 'qualifier' | 'key'; node?: GradeNode; colourAt?: (x: number) => Rgb; tool?: ViewerTool } = {}) {
  let node: GradeNode = given.node ?? { id: 'n1', kind: 'corrector', label: '', enabled: true, params: mergePatch(fullCorrector({}), params) as Corrector };
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
      if (method === 'view.sample') {
        const rgb = (given.colourAt ?? (() => [0.8, 0.4, 0.1] as Rgb))(sent!.x as number);
        return { ok: true as const, result: { frame: 0, x: sent!.x, y: sent!.y, size: 3, width: 100, height: 100, region: {}, rgb,
          node: { grade: TARGET, node: 'n1', input: rgb, output: rgb, key: 1 } } };
      }
      return { ok: true as const, result: {} };
    }),
  };
  const hold: { active: ViewerPicker | null; tool: ViewerTool } = { active: null, tool: given.tool ?? 'qualifier' };
  function Hub({ children }: { children: ReactNode }) {
    const [tool, setTool] = useState<ViewerTool>(given.tool ?? 'qualifier');
    const edited = { bridge, revision, target: TARGET, node, frame: 0, onFailure };
    const { hub, active } = usePickerHub(tool, setTool, edited, onFailure);
    hold.active = active; hold.tool = tool;
    return <PickerContext.Provider value={hub}>{children}</PickerContext.Provider>;
  }
  const tree = () => {
    const edited = { bridge, revision, target: TARGET, node, frame: 0, onFailure };
    return <Hub><Palettes merged={false} left="primaries" center={given.center ?? 'qualifier'} last="left" right="scopes"
      onLeft={() => {}} onCenter={() => {}} onRight={() => {}} edited={edited}
      bodies={{ qualifier: edit => <QualifierPalette key={edit.interaction} edit={edit} />, key: edit => <KeyPalette edit={edit} edited={edited} /> }} /></Hub>;
  };
  const view = render(tree());
  return {
    bridge, onFailure, hold,
    sent: () => bridge.call.mock.calls
      .filter(([method, each]) => {
        const gesture = (each as { gesture?: { end?: boolean } }).gesture;
        return method === 'transaction.apply' && (gesture === undefined || gesture.end === true);
      })
      .map(([, each]) => each as { revision: number; ops: { op: string; params: Record<string, unknown> }[] }),
    previews: () => bridge.call.mock.calls.filter(([method, each]) => method === 'transaction.apply' && (each as { gesture?: { end?: boolean } }).gesture?.end !== true
      && (each as { gesture?: unknown }).gesture !== undefined),
    params: () => node.params as Corrector,
    sync: () => view.rerender(tree()),
    /** The viewer hands the picker in force a pointer event. */
    point: async (event: ViewPointer) => { await act(async () => { hold.active?.onPointer(event); }); await settle(); },
  };
}

describe('the Qualifier palette', () => {
  it('takes the viewer as its own, and keys the colour clicked: the qualifier whole to a node that had none, one step', async () => {
    const palettes = mount({});
    expect(palettes.hold.active).not.toBeNull();
    expect(byTest('qualifier-empty')).toBeTruthy();
    await palettes.point(pointer('down', 40));
    await palettes.point(pointer('up', 40));
    const sent = palettes.sent();
    expect(sent).toHaveLength(1);
    const qualifier = sent[0].ops[0].params.qualifier as Qualifier;
    expect(qualifier).toEqual(pickRange([[0.8, 0.4, 0.1]], null));
    const sampled = palettes.bridge.call.mock.calls.filter(([method]) => method === 'view.sample').map(([, each]) => each);
    expect(sampled[0]).toEqual({ monitor: 'timeline', x: 40, y: 10, size: 3, grade: TARGET, node: 'n1' });
  });

  it('grows the range over the colours of a drag, shown as it goes and one step when let go', async () => {
    const palettes = mount({ qualifier: { ...newQualifier(), mode: 'lum' } }, { colourAt: x => [x / 100, x / 100, x / 100] });
    await palettes.point(pointer('down', 20));
    await palettes.point(pointer('move', 50));
    expect(palettes.previews().length).toBeGreaterThan(0);
    await palettes.point(pointer('up', 60));
    const sent = palettes.sent();
    expect(sent).toHaveLength(1);
    // A qualifier there already: the numbers that changed, at their paths.
    expect((sent[0].ops[0].params.qualifier as Qualifier).lum).toMatchObject({ low: 20 - PICK.margin, high: 60 + PICK.margin });
    expect(Object.keys(sent[0].ops[0].params.qualifier as object)).toEqual(['lum']);
  });

  it('subtracts with the tool chosen, or with Alt', async () => {
    const lum = { on: true, low: 20, high: 80, low_soft: 0, high_soft: 0 };
    const palettes = mount({ qualifier: { ...newQualifier(), mode: 'lum', lum } }, { colourAt: x => (x === 1 ? [0.75, 0.75, 0.75] : [0.5, 0.5, 0.5]) });
    fireEvent.click(screen.getByRole('radio', { name: 'Subtract Color Range' }));
    await palettes.point(pointer('down', 1)); await palettes.point(pointer('up', 1));
    expect((palettes.sent()[0].ops[0].params.qualifier as Qualifier).lum.high).toBeCloseTo(75 - PICK.margin, 3);
    palettes.sync();
    // The Eyedropper with Alt: 50 left out by bringing High down, which has less to move than Low.
    fireEvent.click(screen.getByRole('radio', { name: 'Sample Eyedropper' }));
    await palettes.point(pointer('down', 2, 10, { alt: true })); await palettes.point(pointer('up', 2, 10, { alt: true }));
    expect(palettes.sent()).toHaveLength(2);
    expect((palettes.sent()[1].ops[0].params.qualifier as Qualifier).lum.high).toBeCloseTo(50 - PICK.margin, 3);
  });

  it('switches its mode, a component and Invert, sets Matte Finesse, and Reset takes the qualifier away', async () => {
    const palettes = mount({ qualifier: newQualifier() });
    fireEvent.click(screen.getByRole('radio', { name: 'RGB' })); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { mode: 'rgb' } });
    palettes.sync();
    expect(document.querySelector('[data-component="red"]')).not.toBeNull();
    fireEvent.click(document.querySelector<HTMLElement>('[data-test="qualifier-on-red"]')!); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { red: { on: false } } });
    palettes.sync();
    fireEvent.click(byTest('qualifier-invert')); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { invert: true } });
    palettes.sync();
    fireEvent.change(byTest('qualifier-morph'), { target: { value: 'closing' } }); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { finesse: { morph: 'closing' } } });
    palettes.sync();
    // The palette's button has the orange dot while there is a qualifier.
    expect(document.querySelector('[data-palette="qualifier"]')!.hasAttribute('data-changed')).toBe(true);
    fireEvent.click(document.querySelector<HTMLElement>('[data-test="qualifier"] [data-test="reset-palette"]')!); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: null });
    palettes.sync();
    expect(document.querySelector('[data-palette="qualifier"]')!.hasAttribute('data-changed')).toBe(false);
  });

  it('drags the edge of a range bar to set its High, with Alt its softness', async () => {
    const palettes = mount({ qualifier: { ...newQualifier(), mode: 'lum' } });
    const bar = byTest('qualifier-bar-lum');
    // The bar is 200 pixels: the High edge at 200, dragged 40 pixels in.
    fireEvent.pointerDown(bar, { button: 0, pointerId: 1, clientX: 199, clientY: 5 });
    fireEvent.pointerMove(bar, { pointerId: 1, clientX: 180, clientY: 5 });
    fireEvent.pointerMove(bar, { pointerId: 1, clientX: 159, clientY: 5 });
    fireEvent.pointerUp(bar, { pointerId: 1, clientX: 159, clientY: 5 });
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { lum: { high: 80 } } });
    palettes.sync();
    fireEvent.pointerDown(bar, { button: 0, pointerId: 2, clientX: 160, clientY: 5, altKey: true });
    fireEvent.pointerMove(bar, { pointerId: 2, clientX: 170, clientY: 5, altKey: true });
    fireEvent.pointerMove(bar, { pointerId: 2, clientX: 180, clientY: 5, altKey: true });
    fireEvent.pointerUp(bar, { pointerId: 2, clientX: 180, clientY: 5, altKey: true });
    await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ qualifier: { lum: { high_soft: 10 } } });
  });
});

describe('the pickers of Primaries and Curves on the viewer', () => {
  it('arms Pick Black Point for one click, which makes the colour black by Lift', async () => {
    const palettes = mount({}, { center: 'curves', tool: 'none', colourAt: () => [0.1, 0.2, 0.05] });
    expect(palettes.hold.active).toBeNull();
    const button = byTest('picker-black');
    expect((button as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(button); await settle();
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(palettes.hold.active).not.toBeNull();
    await palettes.point(pointer('down', 5));
    const patch = palettes.sent()[0].ops[0].params as { primaries: { lift: Record<string, number> } };
    expect(patch.primaries.lift.r).toBeCloseTo(-0.1 / 0.9, 3);
    expect(palettes.hold.active).toBeNull();
    expect(button.getAttribute('aria-pressed')).toBe('false');
    // Escape lets an armed picker go.
    fireEvent.click(byTest('picker-white')); await settle();
    expect(palettes.hold.active).not.toBeNull();
    fireEvent.keyDown(document.body, { key: 'Escape' }); await settle();
    expect(palettes.hold.active).toBeNull();
  });

  it('switches the viewer to Qualifier from the Curves picker, and puts a point on the curve open', async () => {
    const palettes = mount({}, { center: 'curves', tool: 'none', colourAt: () => [0.5, 0.5, 0.5] });
    fireEvent.click(byTest('curve-picker')); await settle();
    expect(palettes.hold.tool).toBe('qualifier');
    expect(byTest('curve-picker').getAttribute('aria-pressed')).toBe('true');
    await palettes.point(pointer('down', 5)); await palettes.point(pointer('up', 5));
    expect(palettes.sent()[0].ops[0].params).toEqual({ curves: {
      y: { points: [[0, 0], [0.5, 0.5], [1, 1]] }, r: { points: [[0, 0], [0.5, 0.5], [1, 1]] },
      g: { points: [[0, 0], [0.5, 0.5], [1, 1]] }, b: { points: [[0, 0], [0.5, 0.5], [1, 1]] } } });
  });
});

describe('the Key palette', () => {
  it('sends the Key Input, Key Output and Qualifier settings of a corrector at their paths', async () => {
    const palettes = mount({}, { center: 'key' });
    fireEvent.click(byTest('key-input-invert')); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ key: { input: { invert: true } } });
    palettes.sync();
    fireEvent.click(within(byTest('key-qualifier-matte')).getByRole('radio', { name: 'Mask' })); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ key: { qualifier: { matte: false } } });
    palettes.sync();
    // Output Gain, typed: 0 to 2.
    const gain = document.querySelector<HTMLElement>('[data-field="key.output.gain"] .color-number-box') as HTMLInputElement;
    fireEvent.doubleClick(gain); fireEvent.change(gain, { target: { value: '0.5' } }); fireEvent.keyDown(gain, { key: 'Enter' }); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ key: { output: { gain: 0.5 } } });
    palettes.sync();
    expect(keyChanged(palettes.params())).toBe(true);
    expect(keyChanged(fullCorrector({}))).toBe(false);
    expect(document.querySelector('[data-palette="key"]')!.hasAttribute('data-changed')).toBe(true);
  });

  it('sends the rows of a Key Mixer whole, one set_node each', async () => {
    const node: GradeNode = { id: 'n1', kind: 'key_mixer', label: '', enabled: true, params: {
      inputs: [{ invert: false, matte: false, gain: 1, offset: 0 }, { invert: false, matte: false, gain: 1, offset: 0 }],
      output: { invert: false, gain: 1, offset: 0 } } };
    const palettes = mount({}, { center: 'key', node });
    expect(byTest('key-mixer')).toBeTruthy();
    fireEvent.click(byTest('mixer-invert-1')); await settle();
    expect(palettes.sent().at(-1)!.ops[0]).toMatchObject({ op: 'set_node', id: 'n1', params: { inputs: [
      { invert: false, matte: false, gain: 1, offset: 0 }, { invert: true, matte: false, gain: 1, offset: 0 }] } });
    fireEvent.click(byTest('mixer-output-invert')); await settle();
    expect(palettes.sent().at(-1)!.ops[0].params).toEqual({ output: { invert: true, gain: 1, offset: 0 } });
  });
});

function within(element: HTMLElement) {
  return { getByRole: (role: string, { name }: { name: string }) => [...element.querySelectorAll<HTMLElement>(`[role="${role}"]`)].find(each => each.textContent === name)! };
}
