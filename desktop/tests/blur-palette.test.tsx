// @vitest-environment jsdom
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Palettes } from '../src/studio/color/Palettes';
import { blurChanged, blurUses, columnEdits, neutralBlur } from '../src/studio/color/blur';
import { fullCorrector, neutralCorrector } from '../src/studio/color/corrector';
import type { EditedNode } from '../src/studio/color/useNodeEdit';
import { mergePatch, type Corrector } from '../src/studio/engine';
import { likeABrowser } from './mounted';

// React tells of children with the same key on the console; a palette that renders such fails.
let errors: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => { callback(0); return 1; });
  errors = vi.spyOn(console, 'error');
});
afterEach(() => {
  cleanup();
  const keys = errors.mock.calls.map(call => call.map(String).join(' ')).filter(said => /same key|unique "key"/.test(said));
  vi.restoreAllMocks(); vi.unstubAllGlobals();
  expect(keys).toEqual([]);
});
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
const pointer = (element: Element, kind: 'pointerDown' | 'pointerMove' | 'pointerUp', x = 0, y = 0) => fireEvent[kind](element, { pointerId: 1, button: 0, clientX: x, clientY: y });
const bar = (column: string, ch: string) => document.querySelector<HTMLElement>(`[data-test="blur-${column}-${ch}"] [role="slider"]`)!;
const byTest = (id: string) => document.querySelector<HTMLElement>(`[data-test="${id}"]`);
const box = (path: string) => document.querySelector<HTMLInputElement>(`[data-field="${path}"] input`)!;
type Sent = { revision: number; ops: { op: string; params: Record<string, unknown> }[]; gesture?: { id: string; end?: boolean } };

function mount(params: unknown = {}, extra: Partial<EditedNode> = {}) {
  const pending: ((value: unknown) => void)[] = [];
  const call = vi.fn((_method: string, _params?: Record<string, unknown>, _context?: unknown) => new Promise<any>(resolve => pending.push(resolve)));
  let context: EditedNode = { bridge: { call }, session: 1, revision: 10, target: { level: 'clip', timeline: 't1', clip: 'c1' },
    clip: 'c1', clock: 'source-clock', timelineFrame: 0, playing: false,
    node: { id: 'n1', kind: 'corrector', label: '', enabled: true, params: fullCorrector(params) }, frame: 0, onFailure: vi.fn(), ...extra };
  const tree = () => <Palettes merged={false} left="primaries" center="blur" last="center" right="scopes" onLeft={() => {}} onCenter={() => {}} onRight={() => {}} edited={context} />;
  const view = render(tree());
  return { call,
    update: (change: Partial<EditedNode>) => { context = { ...context, ...change }; view.rerender(tree()); },
    sync: (revision: number, patch: unknown) => {
      context = { ...context, revision, node: { ...context.node, params: mergePatch(context.node.params, patch) as Corrector } as EditedNode['node'] };
      view.rerender(tree());
    },
    sent: () => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, sent]) => sent as unknown as Sent),
    answer: async (revision: number) => { await act(async () => pending.shift()!({ ok: true, result: { revision } })); },
  };
}

describe('the Blur palette as its tabs show it (pp. 3454–3456)', () => {
  it('is a corrector of the engine as at first, and an older node without it reads so', () => {
    expect(neutralCorrector().blur).toEqual(neutralBlur());
    const old = neutralCorrector() as Partial<Corrector>;
    delete old.blur;
    expect(fullCorrector(old).blur).toEqual(neutralBlur());
    expect(blurChanged(neutralCorrector())).toBe(false);
    expect(blurChanged(fullCorrector({ blur: { mode: 'mist', linked: { radius: false } } }))).toBe(false);
    expect(blurChanged(fullCorrector({ blur: { scaling: { g: 0.3 } } }))).toBe(true);
    expect(blurChanged(fullCorrector({ blur: { mix: { keys: [{ frame: 0, value: 100 }, { frame: 5, value: 40 }] } } }))).toBe(true);
  });

  it('greys what each tab does not use and keeps it', () => {
    expect(['radius', 'ratio', 'scaling', 'coring_softness', 'level', 'mix'].map(each => blurUses('blur', each as never))).toEqual([true, true, false, false, false, false]);
    expect(['radius', 'ratio', 'scaling', 'coring_softness', 'level', 'mix'].map(each => blurUses('sharpen', each as never))).toEqual([true, true, true, true, true, false]);
    expect(['radius', 'ratio', 'scaling', 'coring_softness', 'level', 'mix'].map(each => blurUses('mist', each as never))).toEqual([true, true, true, false, false, true]);
    mount({ blur: { mode: 'sharpen', scaling: { r: 0.6, g: 0.6, b: 0.6 }, level: 0.2 } });
    expect(document.querySelector('[data-column="scaling"]')!.hasAttribute('data-unused')).toBe(false);
    expect(box('blur.level').disabled).toBe(false);
    expect(box('blur.mix').disabled).toBe(true);
    expect(box('blur.scaling.r').value).toBe('0.60');
    expect(document.querySelector('[data-blur-mode="sharpen"]')!.getAttribute('aria-checked')).toBe('true');
  });

  it('shows Blur at first: its Scaling and the settings beneath grey, its values as the pages show them', () => {
    mount();
    expect(byTest('blur-palette')!.getAttribute('data-mode')).toBe('blur');
    expect(box('blur.radius.r').value).toBe('0.50');
    expect(box('blur.ratio.b').value).toBe('0.50');
    expect(box('blur.scaling.g').value).toBe('0.25');
    expect(box('blur.mix').value).toBe('100.00');
    expect(document.querySelector('[data-column="scaling"]')!.hasAttribute('data-unused')).toBe(true);
    expect(box('blur.scaling.g').disabled).toBe(true);
    expect([box('blur.coring_softness'), box('blur.level'), box('blur.mix')].every(each => each.disabled)).toBe(true);
    expect(document.querySelector('[data-palette="blur"] [data-test="changed-dot"]')).toBeNull();
  });

  it('chooses a tab as one step, the settings kept', async () => {
    const edit = mount({ blur: { radius: { r: 0.3, g: 0.3, b: 0.3 } } });
    fireEvent.click(document.querySelector('[data-blur-mode="mist"]')!);
    expect(edit.sent()).toHaveLength(1);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { mode: 'mist' } });
    expect(edit.sent()[0].gesture).toBeUndefined();
    await edit.answer(11); edit.sync(11, { blur: { mode: 'mist' } });
    expect(box('blur.mix').disabled).toBe(false);
    expect(box('blur.radius.g').value).toBe('0.30');
    // The tab chosen again sends nothing.
    fireEvent.click(document.querySelector('[data-blur-mode="mist"]')!);
    expect(edit.sent()).toHaveLength(1);
  });

  it('shows the orange dot where a setting of it is changed', () => {
    mount({ blur: { radius: { g: 0.7 } } });
    expect(document.querySelector('[data-palette="blur"] [data-test="changed-dot"]')).not.toBeNull();
  });

  it('says there is no corrector where the node is a mixer', () => {
    mount({}, { node: { id: 'm', kind: 'parallel_mixer', label: '', enabled: true, params: { inputs: 2 } } as EditedNode['node'] });
    expect(byTest('no-corrector')!).toBeTruthy();
    expect(byTest('blur-palette')).toBeNull();
  });
});

describe('ganged columns', () => {
  it('moves the three by as much as the one dragged, as one gesture to undo', async () => {
    // Ganged again after the green was set apart: it keeps its offset.
    const edit = mount({ blur: { radius: { r: 0.5, g: 0.6, b: 0.4 } } });
    const red = bar('radius', 'r');
    pointer(red, 'pointerDown', 0, 100); pointer(red, 'pointerMove', 0, 80);
    expect(edit.sent()).toHaveLength(1);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { radius: { r: 0.6, g: 0.7, b: 0.5 } } });
    expect(edit.sent()[0].gesture?.id).toMatch(/^palette-/);
    expect(box('blur.radius.g').value).toBe('0.70');
    pointer(red, 'pointerMove', 0, 60); pointer(red, 'pointerUp', 0, 60);
    await edit.answer(11);
    expect(edit.sent()).toHaveLength(2);
    expect(edit.sent()[1]).toMatchObject({ revision: 10, gesture: { id: edit.sent()[0].gesture!.id, end: true },
      ops: [{ params: { blur: { radius: { r: 0.7, g: 0.8, b: 0.6 } } } }] });
  });

  it('keeps each within its bounds', () => {
    const base = fullCorrector({ blur: { ratio: { r: 0.9, g: 0.5, b: 0.95 } } });
    expect(columnEdits(base, 0, 'ratio', 'g', 0.6)).toEqual([['blur.ratio.r', 1], ['blur.ratio.g', 0.6], ['blur.ratio.b', 1]]);
    expect(columnEdits(fullCorrector({ blur: { linked: { ratio: false } } }), 0, 'ratio', 'g', 0.6)).toEqual([['blur.ratio.g', 0.6]]);
  });

  it('ungangs at the button, and then a bar moves its own channel alone', async () => {
    const edit = mount();
    fireEvent.click(byTest('blur-gang-scaling')!);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { linked: { scaling: false } } });
    await edit.answer(11); edit.sync(11, { blur: { linked: { scaling: false }, mode: 'sharpen' } });
    expect(byTest('blur-gang-scaling')!.getAttribute('aria-pressed')).toBe('false');
    const blue = bar('scaling', 'b');
    pointer(blue, 'pointerDown', 0, 100); pointer(blue, 'pointerMove', 0, 50); pointer(blue, 'pointerUp', 0, 50);
    expect(edit.sent()[1].ops[0].params).toEqual({ blur: { scaling: { b: 0.5 } } });
  });

  it('takes a number typed into a box of a ganged column as the bar takes it', () => {
    const edit = mount({ blur: { mode: 'sharpen' } });
    const field = box('blur.scaling.r');
    fireEvent.doubleClick(field); fireEvent.change(field, { target: { value: '0.5' } }); fireEvent.keyDown(field, { key: 'Enter' });
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { scaling: { r: 0.5, g: 0.5, b: 0.5 } } });
  });

  it('resets a column, and the palette, to the values the pages show', async () => {
    const edit = mount({ blur: { mode: 'sharpen', radius: { r: 0.2, g: 0.3, b: 0.4 }, level: 0.5, mix: 20, linked: { radius: false } } });
    fireEvent.click(byTest('blur-reset-radius')!);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { radius: { r: 0.5, g: 0.5, b: 0.5 } } });
    await edit.answer(11); edit.sync(11, { blur: { radius: { r: 0.5, g: 0.5, b: 0.5 } } });
    fireEvent.click(byTest('blur-reset')!);
    expect(edit.sent()[1].ops[0].params).toEqual({ blur: {
      radius: { r: 0.5, g: 0.5, b: 0.5 }, ratio: { r: 0.5, g: 0.5, b: 0.5 }, scaling: { r: 0.25, g: 0.25, b: 0.25 },
      coring_softness: 0, level: 0, mix: 100,
    } });
  });
});

describe('the wheel over Radius (p. 3454)', () => {
  it('moves Radius by 0.01 a notch, ganged, each notch a step; no other bar takes it', async () => {
    const edit = mount();
    const red = document.querySelector('[data-test="blur-radius-r"]')!;
    const event = new WheelEvent('wheel', { deltaY: -100, cancelable: true, bubbles: true });
    red.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(true);
    expect(edit.sent()).toHaveLength(1);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { radius: { r: 0.51, g: 0.51, b: 0.51 } } });
    expect(edit.sent()[0].gesture).toBeUndefined();
    await edit.answer(11); edit.sync(11, { blur: { radius: { r: 0.51, g: 0.51, b: 0.51 } } });
    red.dispatchEvent(new WheelEvent('wheel', { deltaY: 100, cancelable: true, bubbles: true }));
    expect(edit.sent()[1].ops[0].params).toEqual({ blur: { radius: { r: 0.5, g: 0.5, b: 0.5 } } });
    const ratio = new WheelEvent('wheel', { deltaY: -100, cancelable: true, bubbles: true });
    document.querySelector('[data-test="blur-ratio-r"]')!.dispatchEvent(ratio);
    expect(ratio.defaultPrevented).toBe(false);
    expect(edit.sent()).toHaveLength(2);
  });
});

describe('keys and the controls that own them', () => {
  const keyed = { keys: [{ frame: 0, value: 0.3, interp: 'linear' }, { frame: 10, value: 0.7, interp: 'linear' }] };

  it('puts an edit into the key at the playhead, and the key is shown', () => {
    const edit = mount({ blur: { linked: { radius: false }, radius: { r: keyed } } }, { frame: 12, timelineFrame: 12 });
    expect(box('blur.radius.r').value).toBe('0.70');
    expect(document.querySelector('[data-field="blur.radius.r"] [data-test="keyed"]')).not.toBeNull();
    const red = bar('radius', 'r');
    pointer(red, 'pointerDown', 0, 100); pointer(red, 'pointerMove', 0, 120); pointer(red, 'pointerUp', 0, 120);
    expect(edit.sent()[0].ops[0].params).toEqual({ blur: { radius: { r: { keys: [{ frame: 0, value: 0.3, interp: 'linear' }, { frame: 10, value: 0.6, interp: 'linear' }] } } } });
  });

  it('ends a ganged drag where a key of a channel it moves is crossed, and not an unganged one', async () => {
    const ganged = mount({ blur: { radius: { g: keyed } } }, { frame: 5, timelineFrame: 5, playing: true });
    let red = bar('radius', 'r');
    pointer(red, 'pointerDown', 0, 100);
    ganged.update({ frame: 11, timelineFrame: 11 });
    red = bar('radius', 'r');
    pointer(red, 'pointerMove', 0, 80); pointer(red, 'pointerUp', 0, 80);
    await settle();
    expect(ganged.sent()).toHaveLength(0);
    cleanup();
    const apart = mount({ blur: { linked: { radius: false }, radius: { g: keyed } } }, { frame: 5, timelineFrame: 5, playing: true });
    red = bar('radius', 'r');
    pointer(red, 'pointerDown', 0, 100);
    apart.update({ frame: 11, timelineFrame: 11 });
    expect(bar('radius', 'r')).toBe(red);
    pointer(red, 'pointerMove', 0, 80); pointer(red, 'pointerUp', 0, 80);
    expect(apart.sent()).toHaveLength(1);
    expect(apart.sent()[0].ops[0].params).toEqual({ blur: { radius: { r: 0.6 } } });
  });
});
