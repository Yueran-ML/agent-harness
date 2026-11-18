// @vitest-environment jsdom
/**
 * The Stroke, Drop Shadow and Background of a title, and the runs of its text styled from the bar over the text field: the
 * settings of the parts of its style as the copy of the project and the keys of the Inspector have them, the switches that give
 * a part to the titles chosen or take it away, its fields, which set it with the source whole or as keys at the playhead, and
 * the bar, which styles the characters chosen in the text and sends the runs with the text, moved with the characters.
 */
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import {
  applyOp, CopyMismatch, DEFAULT_TITLE_PARTS, emptyGrade, type Clip, type Keyframe, type Project, type Timeline, type TitleSpan, type TitleStyle,
} from '../src/studio/engine';
import { addKey, CLIP_SETTINGS, hasKeys, hasSetting, keyTimes, paramOf, putValue, settingsOf } from '../src/studio/inspector/clipKeys';
import { Inspector } from '../src/studio/inspector/Inspector';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const rate = { num: 25, den: 1 };
const key = (frame: number, value: number): Keyframe => ({ frame, value, interp: 'linear' });
const PLAIN: TitleStyle = { font: '', size: 70, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' };
const TEXT = 'Hello world';
/** A title from frame `start` for 60 frames: its keys count its own frames, so frame `n` of them is at frame `start + n`. */
const title = (style: Partial<TitleStyle> = {}, text = TEXT, id = 'c1', start = 10): Clip =>
  clip(id, start, 60, { source: { kind: 'title', text, style: { ...PLAIN, ...style } } });
const sourceOf = (style: TitleStyle, text = TEXT) => ({ kind: 'title', text, style });
const setSource = (style: TitleStyle, text = TEXT, id = 'c1') => ({ op: 'set_clip', timeline: 't1', id, source: sourceOf(style, text) });
const ALL: TitleStyle = { ...PLAIN, ...DEFAULT_TITLE_PARTS };
const timelineOf = (clips: Clip[], locked = false): Timeline => ({ id: 't1', name: 'Main', rate, width: 1920, height: 1080, sample_rate: 48000, grade: emptyGrade(),
  tracks: [{ id: 'v1', kind: 'video', name: 'V1', enabled: true, locked, clips, transitions: [] }] });

type Call = ReturnType<typeof vi.fn>;
/** The Inspector with the clips chosen, the playhead at frame 30: frame 20 of a title from frame 10. */
function mount(clips: Clip[], locked = false): Call {
  const call: Call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  render(<Inspector bridge={{ call, onEvent: vi.fn(() => () => {}) } as Pick<EngineBridge, 'call' | 'onEvent'>} session={1} revision={17}
    timeline={timelineOf(clips, locked)} media={[media]} clips={clips.map(each => each.id)} transition={null} onSpeed={vi.fn()} frame={30} onSeek={vi.fn()} />);
  return call;
}
const sent = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => (params as { ops: unknown[] }).ops);
const row = (name: string) => document.querySelector<HTMLElement>(`[data-panel="title"] [data-field="${name}"]`);
const box = (name: string) => row(name)!.querySelector<HTMLInputElement>('input.inspector-number')!;
const switchOf = (part: string) => document.querySelector<HTMLInputElement>(`[data-test="title-${part}"]`)!;
const control = (name: string) => document.querySelector<HTMLInputElement>(`[data-test="span-${name}"]`)!;
const area = () => document.querySelector<HTMLTextAreaElement>('[data-panel="title"] [data-field="text"] textarea')!;
const type = (input: HTMLElement, value: string) => { fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key: 'Enter' }); };
/** Choose the characters from `from` to the one before `to` in the text field, as a drag over them does. */
const choose = (from: number, to: number) => { area().setSelectionRange(from, to); fireEvent.select(area()); };
const PARTS = ['stroke', 'shadow', 'background'];

describe('the parts of the style of a title that take keys', () => {
  it('are settings of a title that has the part only, read as the number it is given with where it has not', () => {
    const plain = title();
    const lined = title({ stroke: { color: '#000000ff', width: 4 } });
    expect(settingsOf(plain)).toEqual(CLIP_SETTINGS.slice(0, 13));
    expect(settingsOf(lined)).toEqual([...CLIP_SETTINGS.slice(0, 13), 'title.stroke.width']);
    expect(settingsOf(title(DEFAULT_TITLE_PARTS))).toEqual(CLIP_SETTINGS);
    expect([hasSetting(plain, 'title.stroke.width'), hasSetting(lined, 'title.stroke.width'), hasSetting(lined, 'title.shadow.x'), hasSetting(clip('m', 0, 10), 'title.shadow.x')])
      .toEqual([false, true, false, false]);
    expect([paramOf(plain, 'title.stroke.width'), paramOf(plain, 'title.background.padding'), paramOf(lined, 'title.stroke.width')]).toEqual([4, 16, 4]);
    // Nothing is sent for a setting that the title has not got.
    expect(putValue('t1', plain, 'title.stroke.width', 6, 20)).toEqual([]);
    expect(addKey('t1', plain, 'title.shadow.blur', 20)).toEqual([]);
    expect(keyTimes(plain, { rate }, [media], 'title.shadow.x')).toEqual([]);
  });

  it('are set with the source whole where they are a number, the rest of the style going along, and are keys like the size', () => {
    const lined = title({ stroke: { color: '#000000ff', width: 4 }, shadow: { color: '#00000080', x: { keys: [key(0, 0), key(40, 8)] }, y: 4, blur: 4 } });
    const style = (lined.source as { style: TitleStyle }).style;
    expect(putValue('t1', lined, 'title.stroke.width', 6, 20)).toEqual([setSource({ ...style, stroke: { color: '#000000ff', width: 6 } })]);
    expect(putValue('t1', lined, 'title.shadow.x', 2, 20))
      .toEqual([{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.shadow.x', at: 1, remove: 0, keys: [key(20, 2)] }]);
    expect(addKey('t1', lined, 'title.stroke.width', 20))
      .toEqual([{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.stroke.width', at: 0, remove: 0, keys: [key(20, 4)] }]);
    expect([hasKeys(title({ stroke: { color: '#000000ff', width: 4 } })), hasKeys(lined), hasKeys(title({ background: { color: '#000000ff', padding: { keys: [key(0, 4)] }, radius: 0 } }))])
      .toEqual([false, true, true]);
  });

  it('are applied to the copy of the project as the engine applies them, and refused there for a title without the part', () => {
    const projectOf = (c1: Clip) => ({ schema: 5, next_id: 9, bins: [], media: [media], timelines: [timelineOf([c1])], groups: [] }) as unknown as Project;
    const styleOf = (now: Project) => ((now.timelines[0].tracks[0] as { clips: Clip[] }).clips[0].source as { style: TitleStyle }).style;
    const all = projectOf(title(DEFAULT_TITLE_PARTS));
    const keyed = applyOp(all, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.shadow.x', at: 0, remove: 0, keys: [key(5, 2)] });
    expect(styleOf(keyed).shadow).toEqual({ color: '#00000080', x: { keys: [key(5, 2)] }, y: 4, blur: 4 });
    const rounded = applyOp(keyed, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.background.radius', param: 12 });
    expect(styleOf(rounded).background).toEqual({ color: '#00000099', padding: 16, radius: 12 });
    const widths = { keys: [key(0, 1), key(10, 8)] };
    expect(styleOf(applyOp(rounded, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.stroke.width', param: widths })).stroke).toEqual({ color: '#000000ff', width: widths });
    // The project that was given is as it was, and nothing else of the style changed.
    expect(styleOf(all)).toEqual(ALL);
    expect({ ...styleOf(rounded), shadow: ALL.shadow, background: ALL.background }).toEqual(ALL);
    // A title without the part has not got its settings, and a part has no setting but those that take keys.
    const plain = projectOf(title());
    expect(() => applyOp(plain, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.stroke.width', param: 2 })).toThrow(CopyMismatch);
    expect(() => applyOp(plain, { op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.background.padding', at: 0, remove: 0, keys: [key(0, 1)] })).toThrow(CopyMismatch);
    expect(() => applyOp(all, { op: 'set_clip_keys', timeline: 't1', id: 'c1', path: 'title.stroke.color', param: 1 })).toThrow(CopyMismatch);
    // A source set whole brings its parts and its runs as they are.
    const spans: TitleSpan[] = [{ from: 0, to: 5, bold: true, color: '#ff0000ff' }, { from: 6, to: 11, scale: 1.5, italic: true, font: 'Georgia' }];
    const whole: TitleStyle = { ...ALL, spans };
    expect(styleOf(applyOp(plain, { op: 'set_clip', timeline: 't1', id: 'c1', source: { kind: 'title', text: TEXT, style: whole } }))).toEqual(whole);
  });
});

describe('the Stroke, Drop Shadow and Background of the Title tab', () => {
  it('are each a switch, off for a title without the part and with no fields, which gives it with its defaults', async () => {
    const call = mount([title()]);
    expect(PARTS.map(part => [switchOf(part).checked, switchOf(part).getAttribute('role'), switchOf(part).getAttribute('aria-label')]))
      .toEqual([[false, 'switch', 'Stroke'], [false, 'switch', 'Drop Shadow'], [false, 'switch', 'Background']]);
    expect(['stroke-color', 'stroke-width', 'shadow-x', 'background-radius'].map(row)).toEqual([null, null, null, null]);
    for (const part of PARTS) { fireEvent.click(switchOf(part)); await settle(); }
    expect(sent(call)).toEqual([
      [setSource({ ...PLAIN, stroke: { color: '#000000ff', width: 4 } })],
      [setSource({ ...PLAIN, shadow: { color: '#00000080', x: 4, y: 4, blur: 4 } })],
      [setSource({ ...PLAIN, background: { color: '#00000099', padding: 16, radius: 8 } })],
    ]);
  });

  it('show the fields of a part that the title has, and take the part away when switched off', async () => {
    const call = mount([title(DEFAULT_TITLE_PARTS)]);
    expect(PARTS.map(part => switchOf(part).checked)).toEqual([true, true, true]);
    expect(['stroke-width', 'shadow-x', 'shadow-y', 'shadow-blur', 'background-padding', 'background-radius'].map(name => box(name).value))
      .toEqual(['4.0', '4.0', '4.0', '4.0', '16.0', '8.0']);
    expect(['stroke-color', 'shadow-color', 'background-color'].map(name => row(name)!.querySelector('.inspector-unit')!.textContent)).toEqual(['#000000', '#000000', '#000000']);
    // The size, the anchor and the six of the parts, each with the three buttons of its keys.
    expect(document.querySelectorAll('[data-panel="title"] [data-keys^="title."]')).toHaveLength(9);
    fireEvent.click(switchOf('shadow')); await settle();
    const { shadow: _shadow, ...rest } = ALL;
    expect(sent(call)).toEqual([[setSource(rest)]]);
  });

  it('set a number of a part with the source whole, keeping its colour, and a keyed one by a key at the playhead', async () => {
    const shadow = { color: '#00000080', x: { keys: [key(0, 0), key(40, 8)] }, y: 4, blur: 4 };
    const style = { ...ALL, shadow };
    const call = mount([title(style)]);
    // Frame 20 of the title is halfway between the keys of the offset.
    expect(box('shadow-x').value).toBe('4.0');
    type(box('stroke-width'), '6'); await settle();
    type(box('shadow-x'), '10'); await settle();
    // Past the bound of the engine, the field keeps it within.
    type(box('background-radius'), '9999'); await settle();
    fireEvent.click(document.querySelector<HTMLButtonElement>('[data-keys="title.background.padding"] button[aria-pressed]')!); await settle();
    expect(sent(call)).toEqual([
      [setSource({ ...style, stroke: { color: '#000000ff', width: 6 } })],
      [{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.shadow.x', at: 1, remove: 0, keys: [key(20, 10)] }],
      [setSource({ ...style, background: { color: '#00000099', padding: 16, radius: 512 } })],
      [{ op: 'splice_clip_keys', timeline: 't1', id: 'c1', path: 'title.background.padding', at: 0, remove: 0, keys: [key(20, 16)] }],
    ]);
  });

  it('set the colour of a part keeping its alpha', async () => {
    const call = mount([title(DEFAULT_TITLE_PARTS)]);
    fireEvent.change(row('shadow-color')!.querySelector('input')!, { target: { value: '#ff0000' } }); await settle();
    fireEvent.change(row('background-color')!.querySelector('input')!, { target: { value: '#0000ff' } }); await settle();
    expect(sent(call)).toEqual([
      [setSource({ ...ALL, shadow: { ...DEFAULT_TITLE_PARTS.shadow, color: '#ff000080' } })],
      [setSource({ ...ALL, background: { ...DEFAULT_TITLE_PARTS.background, color: '#0000ff99' } })],
    ]);
  });

  it('are mixed where some titles chosen have the part: switched on, those without get it and the others keep theirs', async () => {
    const own = { color: '#112233ff', width: 9 };
    const call = mount([title({ stroke: own }), title({}, 'Bye', 'c2', 100)]);
    expect([switchOf('stroke').checked, switchOf('stroke').indeterminate, row('stroke-width')]).toEqual([false, true, null]);
    expect([switchOf('shadow').checked, switchOf('shadow').indeterminate]).toEqual([false, false]);
    fireEvent.click(switchOf('stroke')); await settle();
    expect(sent(call)).toEqual([[setSource({ ...PLAIN, stroke: { color: '#000000ff', width: 4 } }, 'Bye', 'c2')]]);
  });

  it('are off on a locked track', () => {
    mount([title()], true);
    expect(PARTS.map(part => switchOf(part).disabled)).toEqual([true, true, true]);
  });
});

describe('the font of a title', () => {
  it('is typed or chosen, sent at Enter, and empty for the one that the engine chooses', async () => {
    const call = mount([title({ font: 'Arial' })]);
    const font = row('font')!.querySelector('input')!;
    expect([font.value, font.getAttribute('list') !== null]).toEqual(['Arial', true]);
    expect(row('font')!.querySelectorAll('datalist option').length).toBeGreaterThan(20);
    type(font, 'Georgia'); await settle();
    type(font, ' '); await settle();
    expect(sent(call)).toEqual([[setSource({ ...PLAIN, font: 'Georgia' })], [setSource({ ...PLAIN, font: '' })]]);
  });
});

describe('the bar over the text of a title', () => {
  it('is off until characters are chosen in the text, and says so', () => {
    mount([title()]);
    const names = ['bold', 'italic', 'color', 'scale', 'font', 'clear'];
    expect(names.map(name => control(name).disabled)).toEqual(names.map(() => true));
    const bar = document.querySelector('[data-test="title-span-bar"]')!;
    expect([bar.getAttribute('role'), bar.getAttribute('aria-label'), bar.getAttribute('title')])
      .toEqual(['toolbar', 'Style of the selected text', 'Select characters in the text to style them.']);
    choose(6, 11);
    expect(names.map(name => control(name).disabled)).toEqual(names.map(() => false));
    expect(bar.hasAttribute('title')).toBe(false);
    // A selection of nothing is none.
    choose(3, 3);
    expect(control('bold').disabled).toBe(true);
  });

  it('makes the characters chosen bold or italic, and plain again where they all are', async () => {
    const call = mount([title({ spans: [{ from: 0, to: 5, italic: true }] })]);
    choose(6, 11);
    expect([control('bold').getAttribute('aria-pressed'), control('italic').getAttribute('aria-pressed')]).toEqual(['false', 'false']);
    fireEvent.click(control('bold')); await settle();
    choose(0, 5);
    expect(control('italic').getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(control('italic')); await settle();
    // A selection over part of a run and past it, made italic: the run and what was added to it are one.
    choose(2, 8);
    fireEvent.click(control('italic')); await settle();
    expect(sent(call)).toEqual([
      [setSource({ ...PLAIN, spans: [{ from: 0, to: 5, italic: true }, { from: 6, to: 11, bold: true }] })],
      [setSource(PLAIN)],
      [setSource({ ...PLAIN, spans: [{ from: 0, to: 8, italic: true }] })],
    ]);
  });

  it('gives the characters chosen a colour, with the alpha that they had, a share of the size and a family, and clears them', async () => {
    const call = mount([title({ color: '#ffffff80', spans: [{ from: 0, to: 11, bold: true }] })]);
    choose(0, 5);
    fireEvent.change(control('color'), { target: { value: '#00FF00' } }); await settle();
    type(control('scale'), '1.5'); await settle();
    // A share past those that a run may have is kept within them.
    type(control('scale'), '40'); await settle();
    type(control('font'), 'Georgia'); await settle();
    choose(2, 4);
    fireEvent.click(control('clear')); await settle();
    const bold = { from: 0, to: 11, bold: true };
    const style = (spans: TitleSpan[]) => ({ ...PLAIN, color: '#ffffff80', spans });
    expect(sent(call)).toEqual([
      [setSource(style([{ from: 0, to: 5, bold: true, color: '#00ff0080' }, { from: 5, to: 11, bold: true }]))],
      [setSource(style([{ from: 0, to: 5, bold: true, scale: 1.5 }, { from: 5, to: 11, bold: true }]))],
      [setSource(style([{ from: 0, to: 5, bold: true, scale: 10 }, { from: 5, to: 11, bold: true }]))],
      [setSource(style([{ from: 0, to: 5, bold: true, font: 'Georgia' }, { from: 5, to: 11, bold: true }]))],
      [setSource(style([{ ...bold, to: 2 }, { ...bold, from: 4 }]))],
    ]);
  });

  it('shows what the characters chosen say: their colour, share and family where all of them say the same', () => {
    mount([title({ spans: [{ from: 0, to: 5, color: '#ff8000ff', scale: 2, font: 'Georgia' }, { from: 5, to: 8, scale: 2 }] })]);
    choose(0, 5);
    expect([control('color').value, control('scale').value, control('font').value]).toEqual(['#ff8000', '2', 'Georgia']);
    // Over two runs: the share that both say, no family, and the colour of the title where they differ.
    choose(3, 8);
    expect([control('color').value, control('scale').value, control('font').value, control('font').placeholder]).toEqual(['#ffffff', '2', '', '—']);
    choose(8, 11);
    expect([control('scale').value, control('font').value, control('font').placeholder]).toEqual(['1', '', 'Default font']);
  });

  it('sends the text typed and not yet sent with what it does, the runs that there were moved with their characters', async () => {
    const call = mount([title({ spans: [{ from: 6, to: 11, italic: true }] })]);
    fireEvent.change(area(), { target: { value: 'Hello big world' } });
    choose(6, 9);
    fireEvent.click(control('bold')); await settle();
    expect(sent(call)).toEqual([
      [setSource({ ...PLAIN, spans: [{ from: 6, to: 9, bold: true }, { from: 10, to: 15, italic: true }] }, 'Hello big world')],
    ]);
    expect(area().value).toBe('Hello big world');
  });

  it('is not used where a title would have more runs than the engine takes, and says why', async () => {
    const many: TitleSpan[] = Array.from({ length: 256 }, (_, at) => ({ from: 2 * at, to: 2 * at + 1, bold: true }));
    const call = mount([title({ spans: many }, 'a'.repeat(600))]);
    expect(document.querySelector('[data-test="title-span-full"]')).toBeNull();
    choose(513, 514);
    fireEvent.click(control('italic')); await settle();
    expect(sent(call)).toEqual([]);
    const note = document.querySelector('[data-test="title-span-full"]')!;
    expect([note.getAttribute('role'), note.textContent]).toEqual(['status', 'A title can have at most 256 styled parts of its text.']);
    // Another selection: the note goes.
    choose(0, 1);
    expect(document.querySelector('[data-test="title-span-full"]')).toBeNull();
  });

  it('is off on a locked track, whatever is chosen', () => {
    mount([title()], true);
    choose(0, 5);
    expect(['bold', 'italic', 'color', 'scale', 'font', 'clear'].map(name => control(name).disabled)).toEqual([true, true, true, true, true, true]);
  });
});

describe('the text of a title with runs', () => {
  it('sends the runs moved and cut with the text, so that none ends past it', async () => {
    const spans = [{ from: 0, to: 5, bold: true }, { from: 6, to: 11, italic: true }];
    const call = mount([title({ spans })]);
    type(area(), 'Hello, world'); await settle();
    type(area(), 'world'); await settle();
    type(area(), 'Hi'); await settle();
    expect(sent(call)).toEqual([
      [setSource({ ...PLAIN, spans: [{ from: 0, to: 5, bold: true }, { from: 7, to: 12, italic: true }] }, 'Hello, world')],
      [setSource({ ...PLAIN, spans: [{ from: 0, to: 5, italic: true }] }, 'world')],
      [setSource({ ...PLAIN, spans: [{ from: 0, to: 1, bold: true }] }, 'Hi')],
    ]);
  });

  it('is shown drawn as its runs say, in a line under the field', () => {
    mount([title({ spans: [{ from: 0, to: 5, bold: true }, { from: 6, to: 11, italic: true }] })]);
    const line = document.querySelector<HTMLElement>('[data-test="title-text-preview"]')!;
    const parts = [...line.querySelectorAll<HTMLElement>('span')];
    expect(parts.map(part => [part.textContent, part.style.fontWeight, part.style.fontStyle])).toEqual([['Hello', '700', ''], [' ', '', ''], ['world', '', 'italic']]);
    cleanup();
    mount([title()]);
    expect(document.querySelector('[data-test="title-text-preview"]')).toBeNull();
  });
});
