// @vitest-environment jsdom
/**
 * The inspector of the Edit page: what a field shows while it is dragged or typed into, and the one
 * transaction that it sends when it is let go or Enter is pressed; Escape; the fields that several
 * clips share; ranges; what the engine refuses; the panels of transitions, titles and the timeline;
 * and the dialog of speed.
 */
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { emptyGrade, type Clip, type ClipTrack, type Timeline } from '../src/studio/engine';
import { Inspector } from '../src/studio/inspector/Inspector';
import { percentOf, SpeedDialog, speedEdits, speedOf } from '../src/studio/inspector/SpeedDialog';
import { clip, media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); delete document.body.dataset.overlay; });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });

const title = (id: string, start: number, length: number, fields: Partial<Clip> = {}): Clip => clip(id, start, length, {
  source: { kind: 'title', text: 'Hello', style: { font: '', size: 70, color: '#ffffff80', x: 0.5, y: 0.5, align: 'center' } }, ...fields,
});
const track = (id: string, kind: 'video' | 'audio', clips: Clip[], transitions: ClipTrack['transitions'] = []): ClipTrack => ({ id, kind, name: id.toUpperCase(), enabled: true, locked: false, clips, transitions });
const make = (tracks: ClipTrack[]): Timeline => ({ id: 't1', name: 'Main', rate: { num: 25, den: 1 }, width: 1920, height: 1080, sample_rate: 48000, tracks, grade: emptyGrade() });
const basic = () => make([
  track('v1', 'video', [clip('c1', 10, 60, { link: 'g1' }), clip('c2', 70, 60, { transform: { x: 0, y: 0, scale: 1, rotation: 0, opacity: 0.5 } }), title('t1', 200, 50), title('t2', 260, 50, { source: { kind: 'title', text: 'Bye', style: { font: '', size: 40, color: '#00ff00ff', x: 0.25, y: 0.5, align: 'left' } } })]),
  track('a1', 'audio', [clip('a1', 10, 60, { link: 'g1', gain_db: -6 })]),
]);

type Call = ReturnType<typeof vi.fn>;
function mount(options: { timeline?: Timeline; clips?: string[]; transition?: string | null; call?: Call } = {}) {
  const call = options.call ?? vi.fn().mockResolvedValue({ ok: true, result: {} });
  const onSpeed = vi.fn();
  const props = { bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 17, timeline: options.timeline ?? basic(), media: [media],
    clips: options.clips ?? [], transition: options.transition ?? null, onSpeed };
  const view = render(<Inspector {...props} />);
  const rerender = (changed: Partial<typeof props>) => view.rerender(<Inspector {...props} {...changed} />);
  return { call, onSpeed, rerender };
}
const row = (name: string) => document.querySelector<HTMLElement>(`[data-field="${name}"]`)!;
const box = (name: string) => row(name).querySelector<HTMLInputElement>('input.inspector-number')!;
const slider = (name: string) => row(name).querySelector<HTMLInputElement>('input[type="range"]')!;
const label = (name: string) => row(name).querySelector<HTMLElement>('label')!;
const applied = (call: Call) => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params);
const type = (input: HTMLElement, value: string, key = 'Enter') => { fireEvent.change(input, { target: { value } }); fireEvent.keyDown(input, { key }); };
const transform = (fields: Partial<Clip['transform']>) => ({ x: 0, y: 0, scale: 1, rotation: 0, opacity: 1, ...fields });
const refused = (reason: string, fields = {}) => ({ ok: false, error: { code: -32001, message: `refused: ${reason}`, data: { reason, ...fields } } });

describe('the fields of the inspector', () => {
  it('shows a slider as it is dragged, and sends one change when it is let go', async () => {
    const { call } = mount({ clips: ['c1'] });
    fireEvent.pointerDown(slider('opacity'));
    fireEvent.input(slider('opacity'), { target: { value: '80' } });
    fireEvent.input(slider('opacity'), { target: { value: '60' } });
    expect(box('opacity').value).toBe('60.0');
    expect(call).not.toHaveBeenCalled();
    fireEvent.change(slider('opacity'), { target: { value: '50' } }); await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ opacity: 0.5 }) }] }]);
    // What was sent stays shown until the engine tells of it.
    expect(box('opacity').value).toBe('50.0');
  });

  it('leaves clips on locked tracks alone, and with only those chosen turns the fields off and says why', async () => {
    // Reference Manual 20, p. 762: parameters of locked clips cannot be edited in the Inspector.
    const locked = basic();
    locked.tracks[1] = { ...locked.tracks[1], locked: true } as ClipTrack;
    const { call, rerender } = mount({ timeline: locked, clips: ['c1', 'a1'] });
    // c1 and its sound a1, a1 on a locked track: the fields set c1 alone, and there is no Audio tab.
    expect(screen.queryByRole('tab', { name: 'Audio' })).toBeNull();
    expect(document.querySelector('[data-test="inspector-locked"]')).toBeNull();
    type(box('rotation'), '45'); await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ rotation: 45 }) }] }]);
    // Only a1 chosen: its fields are shown, off, with the reason.
    call.mockClear();
    rerender({ clips: ['a1'] });
    expect(document.querySelector('[data-test="inspector-locked"]')?.textContent).toBe('These clips are on locked tracks. Unlock the track to change them.');
    expect(box('volume').disabled).toBe(true);
    expect(slider('volume').disabled).toBe(true);
    expect(box('volume').value).toBe('-6.0');
    fireEvent.pointerDown(label('volume'), { button: 0, pointerId: 7, clientX: 100 });
    fireEvent.pointerMove(label('volume'), { pointerId: 7, clientX: 150 });
    fireEvent.pointerUp(label('volume'), { pointerId: 7, clientX: 150 });
    fireEvent.doubleClick(label('pan'));
    await settle();
    expect(call).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLButtonElement>('[data-test="open-speed"]')!.disabled).toBe(true);
  });

  it('puts back what was there with Escape, from a slider, a typed number or a dragged label', async () => {
    const { call } = mount({ clips: ['c1'] });
    fireEvent.pointerDown(slider('zoom'));
    fireEvent.input(slider('zoom'), { target: { value: '2.5' } });
    expect(box('zoom').value).toBe('2.500');
    fireEvent.keyDown(slider('zoom'), { key: 'Escape' });
    expect(box('zoom').value).toBe('1.000');
    // The slider, let go after Escape, sends nothing.
    fireEvent.input(slider('zoom'), { target: { value: '3' } });
    fireEvent.change(slider('zoom'), { target: { value: '3' } });
    fireEvent.change(box('rotation'), { target: { value: '45' } });
    fireEvent.keyDown(box('rotation'), { key: 'Escape' });
    expect(box('rotation').value).toBe('0.0');
    fireEvent.pointerDown(label('x'), { button: 0, pointerId: 3, clientX: 100 });
    fireEvent.pointerMove(label('x'), { pointerId: 3, clientX: 150 });
    expect(box('x').value).toBe('50.0');
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.pointerUp(label('x'), { pointerId: 3, clientX: 150 });
    expect(box('x').value).toBe('0.0');
    await settle();
    expect(call).not.toHaveBeenCalled();
  });

  it('sends a typed number at Enter or when the field is left, and a dragged label once as it is let go', async () => {
    const { call } = mount({ clips: ['c1'] });
    type(box('rotation'), '45');
    fireEvent.change(box('x'), { target: { value: '96' } }); fireEvent.blur(box('x'));
    fireEvent.pointerDown(label('y'), { button: 0, pointerId: 4, clientX: 100 });
    fireEvent.pointerMove(label('y'), { pointerId: 4, clientX: 110 });
    fireEvent.pointerMove(label('y'), { pointerId: 4, clientX: 120, shiftKey: true });
    expect(box('y').value).toBe('200.0');
    fireEvent.pointerUp(label('y'), { pointerId: 4, clientX: 120 });
    await settle();
    expect(applied(call)).toEqual([
      { revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ rotation: 45 }) }] },
      { revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ x: 0.05 }) }] },
      { revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ y: 200 / 1080 }) }] },
    ]);
  });

  it('takes a number box dragged sideways as a virtual slider, and a press that does not drag as one to write in, as DaVinci Resolve', async () => {
    // Reference Manual 20, p. 67: number fields are virtual sliders, dragged right to raise the value and left to lower it.
    const { call } = mount({ clips: ['c1'] });
    fireEvent.pointerDown(box('rotation'), { button: 0, pointerId: 5, clientX: 100 });
    fireEvent.pointerMove(box('rotation'), { pointerId: 5, clientX: 90 });
    fireEvent.pointerMove(box('rotation'), { pointerId: 5, clientX: 80 });
    expect(call).not.toHaveBeenCalled();
    const lowered = box('rotation').value;
    expect(Number(lowered)).toBeLessThan(0);
    fireEvent.pointerUp(box('rotation'), { pointerId: 5, clientX: 80 });
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ rotation: Number(lowered) }) }] }]);
    expect(document.activeElement).not.toBe(box('rotation'));
    // Escape while it is dragged puts the value back, and nothing is sent.
    fireEvent.pointerDown(box('x'), { button: 0, pointerId: 6, clientX: 100 });
    fireEvent.pointerMove(box('x'), { pointerId: 6, clientX: 160 });
    expect(box('x').value).toBe('60.0');
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.pointerUp(box('x'), { pointerId: 6, clientX: 160 });
    expect(box('x').value).toBe('0.0');
    // A press without a drag goes in to write, with the number chosen; then the box is written, not dragged.
    fireEvent.pointerDown(box('y'), { button: 0, pointerId: 7, clientX: 100 });
    fireEvent.pointerUp(box('y'), { pointerId: 7, clientX: 101 });
    expect(document.activeElement).toBe(box('y'));
    fireEvent.pointerDown(box('y'), { button: 0, pointerId: 8, clientX: 100 });
    fireEvent.pointerMove(box('y'), { pointerId: 8, clientX: 160 });
    expect(box('y').value).toBe('0.0');
    type(box('y'), '54');
    await settle();
    expect(applied(call).at(-1)).toEqual({ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ y: 54 / 1080 }) }] });
  });

  it('puts the default back at a double click on the label', async () => {
    const t = basic();
    (t.tracks[0] as ClipTrack).clips[0].transform = transform({ scale: 2 });
    const { call } = mount({ timeline: t, clips: ['c1'] });
    fireEvent.doubleClick(label('zoom'));
    fireEvent.doubleClick(label('rotation'));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ scale: 1 }) }] }]);
  });

  it('keeps numbers within what the engine takes', async () => {
    const t = basic();
    (t.tracks[0] as ClipTrack).clips[0].fade_out = 20;
    const { call } = mount({ timeline: t, clips: ['c1'] });
    type(box('zoom'), '0');
    type(box('opacity'), '250');
    type(box('fade_in'), '100');
    type(box('rotation'), 'abc');
    expect(box('rotation').value).toBe('0.0');
    await settle();
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ scale: 0.01 }) },
      // An opacity of 100% is what the clip has: nothing to send.
      { op: 'set_clip', timeline: 't1', id: 'c1', fade_in: 40 },
    ]);
  });

  it('takes the commit before into the next, waiting until the copy has it', async () => {
    let answer!: (value: unknown) => void;
    const call = vi.fn().mockImplementationOnce(() => new Promise(resolve => { answer = resolve; })).mockResolvedValue({ ok: true, result: {} });
    const { rerender } = mount({ clips: ['c1'], call });
    type(box('opacity'), '50');
    type(box('rotation'), '90');
    await settle();
    expect(applied(call)).toHaveLength(1);
    answer({ ok: true, result: { revision: 18, done: [], next_id: 1, names: {}, clamped: [] } });
    await settle();
    expect(applied(call)).toHaveLength(1);
    const t = basic();
    (t.tracks[0] as ClipTrack).clips[0].transform = transform({ opacity: 0.5 });
    rerender({ timeline: t, revision: 18 });
    await settle();
    expect(applied(call)[1]).toEqual({ revision: 18, ops: [{ op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ opacity: 0.5, rotation: 90 }) }] });
  });

  it('says why the engine refused, in a sentence', async () => {
    const call = vi.fn().mockResolvedValue(refused('out_of_range', { what: 'transform' }));
    mount({ clips: ['c1'], call });
    type(box('zoom'), '3');
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('A setting is outside its allowed range.');
    fireEvent.click(within(screen.getByRole('alert')).getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('the clips chosen', () => {
  it('show only what they share: a dash where they differ, and one transaction that sets them all', async () => {
    const { call } = mount({ clips: ['c1', 'c2'] });
    expect(screen.getByText('2 selected')).toBeDefined();
    expect(box('opacity').value).toBe('');
    expect(box('opacity').placeholder).toBe('—');
    expect(box('zoom').value).toBe('1.000');
    type(box('opacity'), '75');
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [
      { op: 'set_clip', timeline: 't1', id: 'c1', transform: transform({ opacity: 0.75 }) },
      { op: 'set_clip', timeline: 't1', id: 'c2', transform: transform({ opacity: 0.75 }) },
    ] }]);
  });

  it('show the tabs that every clip of a kind has: no title fields beside media, Title and Video for titles, Video and Audio for linked clips', () => {
    const { rerender } = mount({ clips: ['c1', 't1'] });
    expect(screen.queryByRole('tablist')).toBeNull();
    expect(document.querySelector('[data-panel="video"]')).not.toBeNull();
    expect(document.querySelector('[data-panel="title"]')).toBeNull();
    rerender({ clips: ['t1', 't2'] });
    expect(screen.getAllByRole('tab').map(tab => [tab.textContent, tab.getAttribute('aria-selected')])).toEqual([['Title', 'true'], ['Video', 'false']]);
    expect(row('text').querySelector('textarea')!.placeholder).toBe('—');
    rerender({ clips: ['c1', 'a1'] });
    expect(screen.getAllByRole('tab').map(tab => tab.textContent)).toEqual(['Video', 'Audio', 'Effects', 'File']);
    fireEvent.click(screen.getByRole('tab', { name: 'Audio' }));
    expect(box('volume').value).toBe('-6.0');
    expect(document.querySelector('[data-panel="video"]')).toBeNull();
  });

  it('set the sound of audio clips: volume in decibels, pan from -100 to 100', async () => {
    const { call } = mount({ clips: ['a1'] });
    fireEvent.input(slider('volume'), { target: { value: '-12' } });
    fireEvent.change(slider('volume'), { target: { value: '-12' } });
    await settle();
    type(box('pan'), '-40');
    type(box('volume'), '60');
    await settle();
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { op: 'set_clip', timeline: 't1', id: 'a1', gain_db: -12 },
      // Preview and end carry the same absolute value in one wire gesture / undo step.
      { op: 'set_clip', timeline: 't1', id: 'a1', gain_db: -12 },
      { op: 'set_clip', timeline: 't1', id: 'a1', pan: -0.4 },
      { op: 'set_clip', timeline: 't1', id: 'a1', gain_db: 24 },
    ]);
  });

  it('take no fade at an edge with a transition', () => {
    const t = basic();
    (t.tracks[0] as ClipTrack).transitions = [{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 10, align: 'center' }];
    mount({ timeline: t, clips: ['c1'] });
    expect(box('fade_out').disabled).toBe(true);
    expect(row('fade_out').title).toBe('A transition is at this edge, so a fade cannot be set here.');
    expect(box('fade_in').disabled).toBe(false);
  });

  it('open the dialog of speed from the inspector, for clips of media only', () => {
    const { onSpeed, rerender } = mount({ clips: ['c1', 'a1'] });
    fireEvent.click(screen.getByRole('button', { name: 'Change clip speed' }));
    expect(onSpeed).toHaveBeenCalledWith(['c1', 'a1']);
    rerender({ clips: ['t1'] });
    expect(screen.queryByRole('button', { name: 'Change clip speed' })).toBeNull();
  });
});

describe('the panels', () => {
  it('set a title: its text at Enter, its colour keeping its alpha, its alignment, its size', async () => {
    const { call } = mount({ clips: ['t1'] });
    const text = row('text').querySelector('textarea')!;
    fireEvent.change(text, { target: { value: 'Opening' } });
    fireEvent.keyDown(text, { key: 'Enter', shiftKey: true });
    fireEvent.keyDown(text, { key: 'Enter' });
    fireEvent.change(row('color').querySelector('input')!, { target: { value: '#ff0000' } });
    fireEvent.click(screen.getByRole('button', { name: 'Align left' }));
    type(box('size'), '5000');
    await settle();
    const style = { font: '', size: 70, color: '#ffffff80', x: 0.5, y: 0.5, align: 'center' };
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { op: 'set_clip', timeline: 't1', id: 't1', source: { kind: 'title', text: 'Opening', style } },
      { op: 'set_clip', timeline: 't1', id: 't1', source: { kind: 'title', text: 'Hello', style: { ...style, color: '#ff000080' } } },
      { op: 'set_clip', timeline: 't1', id: 't1', source: { kind: 'title', text: 'Hello', style: { ...style, align: 'left' } } },
      { op: 'set_clip', timeline: 't1', id: 't1', source: { kind: 'title', text: 'Hello', style: { ...style, size: 4096 } } },
    ]);
  });

  it('set a transition, and offer the longest that fits where the engine says it has too few frames', async () => {
    const t = basic();
    (t.tracks[0] as ClipTrack).transitions = [{ id: 'x1', kind: 'dissolve', left: 'c1', right: 'c2', length: 10, align: 'center' }];
    const call = vi.fn().mockResolvedValueOnce({ ok: true, result: {} }).mockResolvedValueOnce(refused('no_handles', { clip: 'c2', need: 20, have: 9, longest: 9 }))
      .mockResolvedValue({ ok: true, result: {} });
    mount({ timeline: t, transition: 'x1', call });
    expect(screen.getByText('Transition · Dissolve')).toBeDefined();
    fireEvent.change(row('kind').querySelector('select')!, { target: { value: 'dip_black' } });
    type(box('length'), '40');
    await settle();
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toContain('The source does not have enough frames for this edit.');
    expect(alert.textContent).toContain('The longest that fits is 9 frames.');
    fireEvent.click(within(alert).getByRole('button', { name: 'Use 9 frames' }));
    fireEvent.change(row('align').querySelector('select')!, { target: { value: 'end' } });
    fireEvent.click(screen.getByRole('button', { name: 'Delete transition' }));
    await settle();
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { edit: 'set_transition', timeline: 't1', id: 'x1', kind: 'dip_black' },
      { edit: 'set_transition', timeline: 't1', id: 'x1', length: 40 },
      { edit: 'set_transition', timeline: 't1', id: 'x1', length: 9 },
      { edit: 'set_transition', timeline: 't1', id: 'x1', align: 'end' },
      { edit: 'remove_transition', timeline: 't1', id: 'x1' },
    ]);
  });

  it('offers only a crossfade on an audio track', () => {
    const t = basic();
    (t.tracks[1] as ClipTrack).clips.push(clip('a2', 70, 60));
    (t.tracks[1] as ClipTrack).transitions = [{ id: 'x2', kind: 'crossfade', left: 'a1', right: 'a2', length: 10, align: 'center' }];
    mount({ timeline: t, transition: 'x2' });
    const kind = row('kind').querySelector('select')!;
    expect(kind.disabled).toBe(true);
    expect([...kind.options].map(option => option.value)).toEqual(['crossfade']);
  });

  it('set the timeline where nothing is chosen, with its frame rate locked while it has content', async () => {
    const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
    mount({ call });
    expect(screen.getByText('Select a clip on the timeline.')).toBeDefined();
    const rate = row('rate').querySelector('select')!;
    expect(rate.disabled).toBe(true);
    const note = screen.getByText('The frame rate is locked because the timeline has content. A new timeline can use another rate.');
    expect(rate.getAttribute('aria-describedby')).toBe(note.id);
    // The other settings stay open.
    for (const name of ['resolution', 'sample_rate']) expect(row(name).querySelector('select')!.disabled).toBe(false);
    expect(box('width').disabled).toBe(false);
    type(box('width'), '1281');
    fireEvent.change(row('resolution').querySelector('select')!, { target: { value: '3840x2160' } });
    fireEvent.change(row('sample_rate').querySelector('select')!, { target: { value: '96000' } });
    const name = row('name').querySelector('input')!;
    type(name, '  ', 'Enter');
    type(name, 'Final cut');
    await settle();
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { op: 'set_timeline', id: 't1', width: 1280 },
      { op: 'set_timeline', id: 't1', width: 3840, height: 2160 },
      { op: 'set_timeline', id: 't1', sample_rate: 96000 },
      { op: 'set_timeline', id: 't1', name: 'Final cut' },
    ]);
  });

  it('lock the frame rate for a subtitle alone, and free it on a timeline with empty tracks', async () => {
    const subtitles = (cues: { id: string; start: number; length: number; text: string }[]) =>
      ({ id: 's1', kind: 'subtitle' as const, name: 'ST1', language: 'en', enabled: true, locked: false, cues });
    const empty = make([track('v1', 'video', []), track('a1', 'audio', [])]);
    const withCue = { ...empty, tracks: [...empty.tracks, subtitles([{ id: 'q1', start: 0, length: 50, text: 'Hi' }])] };
    const call = vi.fn().mockResolvedValueOnce(refused('out_of_source', { media: 'm1', why: 'past_end' })).mockResolvedValue({ ok: true, result: {} });
    const { rerender } = mount({ call, timeline: withCue as Timeline });
    expect(row('rate').querySelector('select')!.disabled).toBe(true);
    rerender({ timeline: { ...empty, tracks: [...empty.tracks, subtitles([])] } as Timeline });
    const rate = row('rate').querySelector('select')!;
    expect(rate.disabled).toBe(false);
    expect(rate.getAttribute('aria-describedby')).toBeNull();
    expect(screen.queryByText(/The frame rate is locked/)).toBeNull();
    // Content that came meanwhile: the engine refuses, and says so in a sentence of its own.
    fireEvent.change(rate, { target: { value: '30000/1001' } });
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('At this frame rate, a clip would run past the end of its media.');
    fireEvent.change(rate, { target: { value: '50/1' } });
    await settle();
    expect(applied(call).map(each => (each as { ops: unknown[] }).ops[0])).toEqual([
      { op: 'set_timeline', id: 't1', rate: { num: 30000, den: 1001 } },
      { op: 'set_timeline', id: 't1', rate: { num: 50, den: 1 } },
    ]);
  });
});

describe('the dialog of speed', () => {
  const dialog = (clips: string[], call: Call = vi.fn().mockResolvedValue({ ok: true, result: {} })) => {
    const onClose = vi.fn();
    render(<SpeedDialog clips={clips} timeline={basic()} bridge={{ call }} session={1} revision={17} onClose={onClose} />);
    return { call, onClose, percent: () => screen.getByLabelText('Speed') as HTMLInputElement };
  };

  it('keeps a speed as a fraction in its lowest terms', () => {
    expect(speedOf(50)).toEqual({ num: 1, den: 2 });
    expect(speedOf(250)).toEqual({ num: 5, den: 2 });
    expect(speedOf(33.33)).toEqual({ num: 3333, den: 10000 });
    expect(percentOf({ num: 3333, den: 10000 })).toBe(33.33);
  });

  it('changes a group of linked clips as one, a clip without its partner alone, and leaves titles out', () => {
    const t = basic();
    expect(speedEdits(t, ['c1', 'a1', 't1'], { num: 1, den: 2 }, undefined, true)).toEqual([
      { edit: 'set_speed', timeline: 't1', clip: 'c1', speed: { num: 1, den: 2 }, ripple: true, linked: true },
    ]);
    expect(speedEdits(t, ['a1', 'c2'], { num: 2, den: 1 }, true, false)).toEqual([
      { edit: 'set_speed', timeline: 't1', clip: 'a1', speed: { num: 2, den: 1 }, reverse: true, ripple: false, linked: false },
      { edit: 'set_speed', timeline: 't1', clip: 'c2', speed: { num: 2, den: 1 }, reverse: true, ripple: false, linked: false },
    ]);
  });

  it('sends the speed, backward and rippled as asked, in one transaction, and closes', async () => {
    const { call, onClose, percent } = dialog(['c1']);
    expect(percent().value).toBe('100');
    fireEvent.change(percent(), { target: { value: '50' } });
    expect(screen.getByText('Duration 00:00:04:20')).toBeDefined();
    fireEvent.click(screen.getByLabelText('Reverse speed'));
    fireEvent.click(screen.getByLabelText('Ripple sequence'));
    fireEvent.click(screen.getByRole('button', { name: 'Change' }));
    await settle();
    expect(applied(call)).toEqual([{ revision: 17, ops: [{ edit: 'set_speed', timeline: 't1', clip: 'c1', speed: { num: 1, den: 2 }, reverse: true, ripple: false, linked: false }] }]);
    expect(onClose).toHaveBeenCalled();
  });

  it('takes no speed out of its range, and says why the engine refused one, staying open', async () => {
    const { call, onClose, percent } = dialog(['c1'], vi.fn().mockResolvedValue(refused('locked', { track: 'v1' })));
    fireEvent.change(percent(), { target: { value: '0' } });
    expect(screen.getByText('The speed is from 1% to 10000%.')).toBeDefined();
    expect((screen.getByRole('button', { name: 'Change' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(percent(), { target: { value: '200' } });
    fireEvent.submit(percent().closest('form')!);
    await settle();
    expect(call).toHaveBeenCalledTimes(1);
    expect(screen.getByText('A track affected by this edit is locked.')).toBeDefined();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('says that titles have no speed', () => {
    dialog(['t1']);
    expect(screen.getByText('Only clips of media can change speed.')).toBeDefined();
    expect((screen.getByRole('button', { name: 'Change' }) as HTMLButtonElement).disabled).toBe(true);
  });
});
