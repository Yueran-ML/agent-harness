// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { Timeline } from '../src/studio/timeline/Timeline';
import type { EngineEvent } from '../src/studio/engine';
import { HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

let draws: FrameRequestCallback[];
beforeEach(() => {
  likeABrowser();
  draws = [];
  vi.stubGlobal('requestAnimationFrame', (f: FrameRequestCallback) => { draws.push(f); return draws.length; });
  vi.stubGlobal('cancelAnimationFrame', () => {});
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(400);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0, width: 956, height: 400, right: 956, bottom: 400, x: 0, y: 0, toJSON() {} });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); });
const draw = () => act(() => { const pending = draws; draws = []; pending.forEach(f => f(0)); });
const settle = () => act(async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); });
function mount(t = timeline()) {
  const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  const bridge = { call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
  const onSeek = vi.fn();
  const props = { timeline: t, media: [media], bridge, session: 1, revision: 0, frame: 0, playing: false, onSeek };
  const view = render(<Timeline {...props} />);
  return { ...view, props, call, onSeek };
}

describe('the timeline display', () => {
  it('virtualizes a thousand clips and exposes their names and frame ranges', () => {
    mount(timeline(Array.from({ length: 1000 }, (_, i) => clip(`c${i}`, i * 100, 80))));
    expect(document.querySelectorAll('[data-clip]').length).toBeLessThan(5);
    expect(document.querySelector('[data-clip="c0"]')?.textContent).toContain('Interview');
    const scroll = document.querySelector<HTMLElement>('.timeline-scroll')!;
    scroll.scrollLeft = 20000;
    fireEvent.scroll(scroll); draw();
    expect(document.querySelector('[data-clip="c0"]')).toBeNull();
    expect(document.querySelector('[data-clip="c50"]')).not.toBeNull();
    expect(document.querySelectorAll('[data-clip]').length).toBeLessThan(5);
  });
  it('shows types, speed, disabled clips, transition span and subtitle text', () => {
    const t = timeline([clip('c1', 0, 100, { enabled: false, speed: { num: 2, den: 1 }, reverse: true }), clip('c2', 100)]);
    if (t.tracks[0].kind !== 'subtitle') t.tracks[0].transitions.push({ id: 'x1', left: 'c1', right: 'c2', length: 5, kind: 'dissolve', align: 'center' });
    t.tracks.push({ id: 'v2', name: 'Titles', kind: 'video', enabled: true, locked: false, transitions: [], clips: [clip('title', 0, 100, { source: { kind: 'title', text: 'A new day', style: { font: 'Arial', size: 48, color: '#ffffffff', x: 0.5, y: 0.5, align: 'center' } } })] });
    t.tracks.push({ id: 's1', name: 'English', kind: 'subtitle', enabled: true, locked: false, language: 'en', cues: [{ id: 'q1', start: 10, length: 30, text: 'Hello world' }] });
    mount(t);
    expect(document.querySelector('[data-clip="c1"]')?.getAttribute('data-enabled')).toBe('false');
    expect(document.querySelector('[data-clip="c1"]')?.textContent).toContain('−2×');
    expect(document.querySelector('[data-clip="title"]')?.getAttribute('data-kind')).toBe('title');
    expect(document.querySelector('[data-cue="q1"]')?.textContent).toContain('Hello world');
    const x = document.querySelector<HTMLElement>('[data-transition="x1"]')!;
    // Frames 98 to 103: the style sheet places it at 4 px a frame, the scale that the content gives.
    expect([x.style.getPropertyValue('--start'), x.style.getPropertyValue('--length')]).toEqual(['98', '5']);
    expect(document.querySelector<HTMLElement>('.timeline-content')!.style.getPropertyValue('--scale')).toBe('4');
  });
  it('throttles scrubbing and always sends the exact release frame', async () => {
    vi.useFakeTimers();
    const { onSeek } = mount();
    const ruler = screen.getByRole('slider', { name: 'Timeline position' });
    fireEvent.pointerDown(ruler, { button: 0, clientX: HEADER_WIDTH + 100 * 4, pointerId: 1 });
    const root = screen.getByRole('region', { name: 'Timeline' });
    for (let i = 101; i <= 110; i++) fireEvent.pointerMove(root, { clientX: HEADER_WIDTH + i * 4, pointerId: 1 });
    expect(onSeek.mock.calls).toEqual([[100, true]]);
    act(() => vi.advanceTimersByTime(34));
    expect(onSeek).toHaveBeenLastCalledWith(110, true);
    fireEvent.pointerUp(root, { clientX: HEADER_WIDTH + 120 * 4, pointerId: 1 });
    expect(onSeek).toHaveBeenLastCalledWith(120, false);
    act(() => vi.advanceTimersByTime(100));
    expect(onSeek).toHaveBeenCalledTimes(3);
    expect(ruler.getAttribute('aria-valuenow')).toBe('120');
  });
  it('locks, mutes and renames through engine transactions; track height is only view state', async () => {
    const t = timeline();
    t.tracks.push({ id: 'a1', name: 'Dialogue', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] });
    const { call } = mount(t);
    fireEvent.click(screen.getByRole('button', { name: 'Lock: V1' }));
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 0, ops: [{ op: 'set_track', timeline: 't1', id: 'v1', locked: true }] });
    fireEvent.click(screen.getByRole('button', { name: 'Mute: Dialogue' }));
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 0, ops: [{ op: 'set_track', timeline: 't1', id: 'a1', enabled: false }] });
    fireEvent.doubleClick(screen.getByText('V1'));
    const name = screen.getByRole('textbox', { name: 'Name' });
    fireEvent.change(name, { target: { value: 'Wide shot' } });
    fireEvent.keyDown(name, { key: 'Enter' });
    await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 0, ops: [{ op: 'set_track', timeline: 't1', id: 'v1', name: 'Wide shot' }] });
    const before = call.mock.calls.length;
    fireEvent.keyDown(screen.getByRole('separator', { name: 'Track height: V1' }), { key: 'ArrowDown' });
    expect(document.querySelector<HTMLElement>('[data-track="v1"]')?.style.height).toBe('80px');
    expect(call).toHaveBeenCalledTimes(before);
  });
  it('takes the wheel and the keys of zoom as DaVinci Resolve, around the playhead, and follows playback', () => {
    const t = timeline(Array.from({ length: 20 }, (_, i) => clip(`c${i}`, i * 100)));
    t.tracks.push({ id: 'a1', name: 'A1', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] });
    const { props, rerender } = mount(t);
    rerender(<Timeline {...props} frame={150} />);
    const root = screen.getByRole('region', { name: 'Timeline' });
    const scroll = document.querySelector<HTMLElement>('.timeline-scroll')!;
    const scale = () => Number(root.dataset.scale);
    scroll.scrollLeft = 400; fireEvent.scroll(scroll); draw();
    // Reference Manual 20, p. 712: Alt with the wheel zooms, centred on the playhead (p. 711), which stays where it is.
    fireEvent.wheel(scroll, { altKey: true, deltaY: -100, clientX: HEADER_WIDTH + 20 });
    expect(scale()).toBeGreaterThan(4);
    expect((scroll.scrollLeft + 200) / scale()).toBeCloseTo(150);
    // Ctrl with the wheel goes forward and back in time, at the same zoom.
    const zoomed = scale(); const left = scroll.scrollLeft;
    fireEvent.wheel(scroll, { ctrlKey: true, deltaY: 120 });
    expect(scroll.scrollLeft).toBe(left + 120);
    expect(scale()).toBe(zoomed);
    // Shift with the wheel makes the tracks of the part under the pointer higher or lower; told as sideways too.
    const height = (id: string) => document.querySelector<HTMLElement>(`[data-track="${id}"]`)!.style.height;
    fireEvent.wheel(document.querySelector('[data-track="a1"] .timeline-lane')!, { shiftKey: true, deltaY: -100 });
    expect([height('v1'), height('a1')]).toEqual(['72px', '80px']);
    fireEvent.wheel(document.querySelector('[data-track="v1"] .timeline-lane')!, { shiftKey: true, deltaX: 100 });
    expect([height('v1'), height('a1')]).toEqual(['64px', '80px']);
    // Shift+Z fits the whole timeline, and pressed again goes back to the zoom before, around the playhead.
    fireEvent.keyDown(root, { key: 'z', shiftKey: true });
    expect(scale()).toBeCloseTo(760 / 2000);
    rerender(<Timeline {...props} frame={1000} />);
    fireEvent.keyDown(root, { key: 'z', shiftKey: true });
    expect(scale()).toBe(zoomed);
    expect(scroll.scrollLeft).toBe(Math.round(1000 * zoomed - 400));
    // Ctrl+= zooms in around the playhead too.
    fireEvent.keyDown(root, { key: '=', ctrlKey: true });
    expect(scale()).toBeCloseTo(zoomed * 1.25);
    expect((scroll.scrollLeft + 400) / scale()).toBeCloseTo(1000, 0);
    rerender(<Timeline {...props} frame={1900} playing />);
    expect(scroll.scrollLeft).toBeGreaterThan(0);
  });
  it('shows an engine refusal without pretending the track changed', async () => {
    const { call } = mount();
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'track is missing', data: { reason: 'missing' } } });
    fireEvent.click(screen.getByRole('button', { name: 'Lock: V1' }));
    await settle();
    expect(screen.getByRole('alert').textContent).toContain('no longer in the project');
    expect(screen.getByRole('button', { name: 'Lock: V1' }).getAttribute('aria-pressed')).toBe('false');
  });

  it('draws source waveforms only for visible audio clips and shares analysis across tracks', async () => {
    const t = timeline([]);
    const a = clip('a1', 0, 100);
    const b = clip('a2', 5000, 100, { source: { kind: 'media', media: 'm2', in: { ticks: 0, scale: 25 } } });
    t.tracks = [
      { id: 'a1', name: 'A1', kind: 'audio', enabled: true, locked: false, clips: [a, b], transitions: [] },
      { id: 'a2', name: 'A2', kind: 'audio', enabled: true, locked: false, clips: [{ ...a, id: 'a3' }], transitions: [] },
    ];
    let tell!: (event: EngineEvent) => void;
    const peaks = Buffer.from(new Int8Array(Array.from({ length: 800 }, (_, i) => i % 2 ? 64 : -64)).buffer).toString('base64');
    const call = vi.fn().mockResolvedValueOnce({ ok: true, result: { pending: true } }).mockResolvedValue({ ok: true, result: { rate: 100, channels: 1, peaks } });
    const bridge = { call, onEvent: listener => { tell = listener; return () => {}; } } as Pick<EngineBridge, 'call' | 'onEvent'>;
    render(<Timeline timeline={t} media={[media, { ...media, id: 'm2' }]} bridge={bridge} session={1} revision={0} frame={0} playing={false} onSeek={() => {}} />);
    await settle();
    expect(call.mock.calls).toEqual([['media.waveform', { media: 'm1' }]]);
    expect(screen.getAllByText('Preparing waveform…')).toHaveLength(2);
    act(() => tell({ kind: 'notification', session: 1, method: 'media.waveform', params: { media: 'm1' } }));
    await settle();
    const shapes = [...document.querySelectorAll('[data-waveform="m1"] path')];
    expect(shapes).toHaveLength(2);
    expect(shapes.every(shape => shape.getAttribute('d')?.startsWith('M1,16.9V47.1'))).toBe(true);
    expect(call).toHaveBeenCalledTimes(2);
  });

  it('cancels queued seeks when switching timelines or unmounting, and finalizes a cancelled pointer at its last position', () => {
    vi.useFakeTimers();
    const { props, rerender, unmount, onSeek } = mount();
    const root = screen.getByRole('region', { name: 'Timeline' });
    const ruler = screen.getByRole('slider', { name: 'Timeline position' });
    fireEvent.pointerDown(ruler, { button: 0, clientX: HEADER_WIDTH + 80, pointerId: 1 });
    fireEvent.pointerMove(root, { clientX: HEADER_WIDTH + 120, pointerId: 1 });
    fireEvent.pointerCancel(root, { pointerId: 1 });
    expect(onSeek).toHaveBeenLastCalledWith(30, false);
    act(() => vi.advanceTimersByTime(100));
    expect(onSeek).toHaveBeenCalledTimes(2);
    fireEvent.pointerDown(ruler, { button: 0, clientX: HEADER_WIDTH + 160, pointerId: 2 });
    fireEvent.pointerMove(root, { clientX: HEADER_WIDTH + 200, pointerId: 2 });
    rerender(<Timeline {...props} timeline={{ ...props.timeline, id: 't2' }} />);
    act(() => vi.advanceTimersByTime(100));
    expect(onSeek).toHaveBeenCalledTimes(3);
    fireEvent.pointerDown(screen.getByRole('slider'), { button: 0, clientX: HEADER_WIDTH + 240, pointerId: 3 });
    fireEvent.pointerMove(root, { clientX: HEADER_WIDTH + 280, pointerId: 3 });
    unmount();
    act(() => vi.advanceTimersByTime(100));
    expect(onSeek).toHaveBeenCalledTimes(4);
  });
});
