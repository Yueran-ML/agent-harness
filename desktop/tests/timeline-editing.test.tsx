// @vitest-environment jsdom
import { createRef } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Timeline, type TimelineHandle } from '../src/studio/timeline/Timeline';
import type { EngineBridge } from '../src/bridge';
import { HEADER_WIDTH } from '../src/studio/timeline/geometry';
import { keyOf } from '../src/app/keys';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import { MEDIA_DRAG } from '../src/studio/media/drag';

const rect = (left: number, top: number, width: number, height: number): DOMRect => ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top, toJSON() {} });
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
  vi.stubGlobal('DragEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (this.dataset.clip) {
      // The style sheet places a clip by its frames and the pixels of a frame of the timeline.
      const scale = Number(this.closest<HTMLElement>('.timeline')?.dataset.scale ?? 4);
      return rect(HEADER_WIDTH + Number(this.style.getPropertyValue('--start')) * scale, 34, Number(this.style.getPropertyValue('--length')) * scale, 64);
    }
    if (this.dataset.track) return rect(0, 30, 956, 72);
    return rect(0, 0, 956, 400);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); });
const node = (id = 'c1') => document.querySelector<HTMLElement>(`[data-clip="${id}"]`)!;
const root = () => screen.getByRole('region', { name: 'Timeline' });
const pointer = (frame: number, extra = {}) => ({ button: 0, clientX: HEADER_WIDTH + frame * 4, clientY: 45, pointerId: 1, ...extra });
function mount(t = timeline([clip('c1', 10, 60), clip('c2', 100, 60)])) {
  const call = vi.fn().mockResolvedValue({ ok: true, result: {} });
  const bridge = { call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
  const onSeek = vi.fn();
  const onSpeed = vi.fn();
  const onSelection = vi.fn();
  const ref = createRef<TimelineHandle>();
  const props = { timeline: t, media: [media], bridge, session: 1, revision: 17, frame: 20, playing: false, onSeek, onSpeed, onSelection, ref };
  return { ...render(<Timeline {...props} />), props, call, onSeek, onSpeed, onSelection, ref };
}
const click = (id = 'c1', extra = {}) => { const at = id === 'c1' ? 30 : 120; fireEvent.pointerDown(node(id), pointer(at, extra)); fireEvent.pointerUp(root(), pointer(at, extra)); };

describe('timeline editing by pointer and keys', () => {
  it('selects linked clips, toggles linked selection, and preserves a multiple selection while dragging', async () => {
    const t = timeline([clip('c1', 10, 60, { link: 'g1' }), clip('c2', 100, 60, { link: 'g1' })]);
    const { call, onSelection } = mount(t);
    click();
    expect(node().getAttribute('aria-selected')).toBe('true');
    expect(node('c2').getAttribute('aria-selected')).toBe('true');
    expect(onSelection).toHaveBeenLastCalledWith(['c1', 'c2']);
    expect(call).not.toHaveBeenCalled();
    fireEvent.keyDown(root(), { key: 'l', ctrlKey: true, shiftKey: true });
    click('c2', { ctrlKey: true });
    expect(node('c2').getAttribute('aria-selected')).toBe('false');
    fireEvent.keyDown(root(), { key: 'Backspace' }); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'lift', timeline: 't1', clips: ['c1'], linked: false }] });
  });
  it('previews and snaps a move without changing clips or calling the engine, then commits once on release', async () => {
    const { call } = mount();
    fireEvent.pointerDown(node(), pointer(30));
    fireEvent.pointerMove(root(), pointer(59));
    expect(document.querySelector<HTMLElement>('[data-preview="c1"]')?.style.left).toBe(`${HEADER_WIDTH + 160}px`);
    expect(document.querySelector<HTMLElement>('.timeline-snap')?.style.left).toBe(`${HEADER_WIDTH + 400}px`);
    expect(node().style.getPropertyValue('--start')).toBe('10');
    expect(call).not.toHaveBeenCalled();
    fireEvent.pointerUp(root(), pointer(59)); await settle();
    fireEvent.pointerUp(root(), pointer(59));
    expect(call.mock.calls).toEqual([['transaction.apply', { revision: 17, ops: [{ edit: 'move', timeline: 't1', delta: 30, linked: true, clips: ['c1'], overwrite: true }] }]]);
    expect(document.querySelector('[data-preview]')).toBeNull();
  });
  it('can disable snapping with N during a drag', async () => {
    const { call } = mount();
    fireEvent.pointerDown(node(), pointer(30)); fireEvent.pointerMove(root(), pointer(59));
    fireEvent.keyDown(root(), { key: 'n' });
    expect(screen.getByRole('button', { name: 'Snapping' }).getAttribute('aria-pressed')).toBe('false');
    fireEvent.pointerMove(root(), pointer(59)); fireEvent.pointerUp(root(), pointer(59)); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'move', timeline: 't1', delta: 29, linked: true, clips: ['c1'], overwrite: true }] });
  });
  it.each(['Escape', 'pointerCancel', 'blur'])('cancels a drag with %s without editing', async how => {
    const { call } = mount();
    fireEvent.pointerDown(node(), pointer(30)); fireEvent.pointerMove(root(), pointer(50));
    expect(document.querySelector('[data-preview]')).not.toBeNull();
    if (how === 'Escape') fireEvent.keyDown(root(), { key: 'Escape' });
    else if (how === 'blur') fireEvent.blur(window);
    else fireEvent.pointerCancel(root(), pointer(50));
    fireEvent.pointerUp(root(), pointer(50)); await settle();
    expect(call).not.toHaveBeenCalled();
    expect(document.querySelector('[data-preview]')).toBeNull();
  });
  it('cancels an old drag if the agent edits the project before release', async () => {
    const { props, rerender, call } = mount();
    fireEvent.pointerDown(node(), pointer(30)); fireEvent.pointerMove(root(), pointer(50));
    rerender(<Timeline {...props} revision={18} />);
    fireEvent.pointerUp(root(), pointer(50)); await settle();
    expect(call).not.toHaveBeenCalled();
    expect(screen.getByRole('alert').textContent).toContain('changed');
  });
  it('uses ripple trim in trim mode and respects the source handle', async () => {
    const { call } = mount();
    fireEvent.keyDown(root(), { key: 't' });
    fireEvent.pointerDown(node(), pointer(11));
    fireEvent.pointerMove(root(), pointer(16));
    fireEvent.pointerUp(root(), pointer(16)); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'trim', timeline: 't1', delta: 5, linked: true, clip: 'c1', edge: 'in', ripple: true }] });
  });
  it('moves an already selected clip with Ctrl held without toggling it off, overwriting as a plain drag does', async () => {
    const { call } = mount(); click();
    fireEvent.pointerDown(node(), pointer(30, { ctrlKey: true }));
    fireEvent.pointerMove(root(), pointer(80, { ctrlKey: true }));
    fireEvent.pointerUp(root(), pointer(80, { ctrlKey: true })); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'move', timeline: 't1', delta: 50, clips: ['c1'], linked: true, overwrite: true }] });
    expect(node().getAttribute('aria-selected')).toBe('true');
  });
  it.each([
    ['slip', 45], ['slide', 85],
  ] as const)('uses the %s half of a clip in trim mode', async (edit, clientY) => {
    const { call } = mount();
    fireEvent.keyDown(root(), { key: 't' });
    fireEvent.pointerDown(node(), pointer(30, { clientY }));
    fireEvent.pointerMove(root(), pointer(40, { clientY })); fireEvent.pointerUp(root(), pointer(40, { clientY })); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [{ edit, timeline: 't1', delta: 10, linked: true, clip: 'c1' }] });
  });
  it('rolls a shared cut, shuffles in with Ctrl+Shift-drag, shows what a drag overwrites, and changes only tracks with Shift', async () => {
    const { call } = mount(timeline([clip('c1', 10, 60), clip('c2', 70, 60)]));
    fireEvent.pointerDown(node(), pointer(69.75));
    fireEvent.pointerMove(root(), pointer(79.75)); fireEvent.pointerUp(root(), pointer(79.75)); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'roll', timeline: 't1', left: 'c1', right: 'c2', delta: 10, linked: true }] });
    const shuffle = { shiftKey: true, ctrlKey: true };
    fireEvent.pointerDown(node(), pointer(30, shuffle));
    fireEvent.pointerMove(root(), pointer(90, shuffle));
    expect(document.querySelector('[data-covered]')).toBeNull();
    fireEvent.pointerUp(root(), pointer(90, shuffle)); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'insert_move', timeline: 't1', clips: ['c1'], at: 70, linked: true }] });
    // A plain drag onto c2 shows the part of it that it would overwrite.
    fireEvent.pointerDown(node(), pointer(30));
    fireEvent.pointerMove(root(), pointer(60));
    const covered = document.querySelector<HTMLElement>('[data-covered="c2"]')!;
    expect([covered.style.left, covered.style.width]).toEqual([`${HEADER_WIDTH + 70 * 4}px`, `${30 * 4}px`]);
    fireEvent.keyDown(root(), { key: 'Escape' });
    expect(document.querySelector('[data-covered]')).toBeNull();
    // Shift alone keeps the clip where it is in time: there is no other track here, so nothing changes.
    call.mockClear();
    fireEvent.pointerDown(node(), pointer(30, { shiftKey: true }));
    fireEvent.pointerMove(root(), pointer(90, { shiftKey: true }));
    expect(document.querySelector<HTMLElement>('[data-preview="c1"]')?.style.left).toBe(`${HEADER_WIDTH + 40}px`);
    fireEvent.pointerUp(root(), pointer(90, { shiftKey: true })); await settle();
    expect(call).not.toHaveBeenCalled();
  });
  it('retains a multi-selection and routes a captured pointer to the track under its release position', async () => {
    const t = timeline(); t.tracks.push({ id: 'v2', kind: 'video', name: 'V2', enabled: true, locked: false, clips: [], transitions: [] });
    const { call } = mount(t);
    const v1 = document.querySelector<HTMLElement>('[data-track="v1"]')!;
    const v2 = document.querySelector<HTMLElement>('[data-track="v2"]')!;
    vi.spyOn(v1, 'getBoundingClientRect').mockReturnValue(rect(0, 102, 956, 72));
    vi.spyOn(v2, 'getBoundingClientRect').mockReturnValue(rect(0, 30, 956, 72));
    click(); click('c2', { ctrlKey: true });
    fireEvent.pointerDown(node(), pointer(30));
    fireEvent.pointerMove(root(), pointer(40)); fireEvent.pointerUp(root(), pointer(40)); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'move', timeline: 't1', clips: ['c1', 'c2'], delta: 10, to_track: 'v2', overwrite: true, linked: true }] });
  });
  it('blades the clicked track or all tracks with Shift without a drag', async () => {
    const { call } = mount();
    fireEvent.keyDown(root(), { key: 'b' });
    fireEvent.pointerDown(node(), pointer(30)); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'blade', timeline: 't1', at: 30, tracks: ['v1'], linked: true }] });
    fireEvent.pointerDown(node(), pointer(40, { shiftKey: true })); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'blade', timeline: 't1', at: 40, linked: true }] });
  });
  it('gives Ctrl+Shift+Z to redo, keeps marks in the UI, and requests the speed dialog', async () => {
    const { call, onSpeed } = mount();
    fireEvent.keyDown(root(), { key: 'z', ctrlKey: true, shiftKey: true }); await settle();
    expect(call).toHaveBeenLastCalledWith('history.redo', undefined);
    expect(root().dataset.scale).toBe('4');
    call.mockClear();
    fireEvent.keyDown(root(), { key: 'i' });
    fireEvent.keyDown(root(), { key: 'o' });
    expect(document.querySelector<HTMLElement>('.timeline-mark-range')?.style.width).toBe('4px');
    expect(call).not.toHaveBeenCalled();
    click(); fireEvent.keyDown(root(), { key: 'r' });
    expect(onSpeed).toHaveBeenCalledWith(['c1']);
  });
  it('ignores editing shortcuts in text fields, IME, repeated keys and modal overlays', () => {
    const { call } = mount(); click();
    fireEvent.keyDown(root(), { key: 'Delete', isComposing: true });
    fireEvent.keyDown(root(), { key: 'Delete', keyCode: 229 });
    fireEvent.keyDown(root(), { key: 'Delete', repeat: true });
    fireEvent.doubleClick(screen.getByText('V1'));
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Name' }), { key: 'Delete' });
    const dialog = document.createElement('dialog'); dialog.open = true; document.body.append(dialog);
    fireEvent.keyDown(root(), { key: 'Delete' }); dialog.remove();
    expect(call).not.toHaveBeenCalled();
  });
  it('shows a localized refusal and does not move the accepted project', async () => {
    const { call } = mount();
    call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'raw technical detail', data: { reason: 'no_room' } } });
    fireEvent.pointerDown(node(), pointer(30)); fireEvent.pointerMove(root(), pointer(80)); fireEvent.pointerUp(root(), pointer(80)); await settle();
    expect(screen.getByRole('alert').textContent).toContain('not enough room');
    expect(screen.getByRole('alert').textContent).not.toContain('raw technical detail');
    expect(node().style.getPropertyValue('--start')).toBe('10');
  });
  it('exposes source mark and placement keys for the dual-monitor host, with explicit target suppression', async () => {
    const { props, ref, rerender, call } = mount();
    rerender(<Timeline {...props} focus="source" source={{ media, frame: 45, rate: { num: 25, den: 1 } }} />);
    act(() => { ref.current!.keyDown(keyOf(new KeyboardEvent('keydown', { key: 'i' }))); }); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ op: 'set_media', id: 'm1', mark_in: { ticks: 45, scale: 25 } }] });
    fireEvent.click(screen.getByRole('button', { name: 'Target track: V1' }));
    act(() => { ref.current!.keyDown(keyOf(new KeyboardEvent('keydown', { key: 'F9' }))); }); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'insert', timeline: 't1', source: { kind: 'media', media: 'm1' }, at: 20, video_track: null, audio_track: null }] });
  });
  it('does not start editing a locked clip or submit another transaction while one is pending', async () => {
    const { props, rerender, call } = mount();
    rerender(<Timeline {...props} timeline={{ ...props.timeline, tracks: props.timeline.tracks.map(t => ({ ...t, locked: true })) }} />);
    click(); fireEvent.keyDown(root(), { key: 'Delete' });
    expect(call).not.toHaveBeenCalled();
    rerender(<Timeline {...props} />);
    call.mockImplementation(() => new Promise(() => {}));
    click(); fireEvent.keyDown(root(), { key: 'Delete' }); fireEvent.keyDown(root(), { key: 'Delete' });
    expect(call).toHaveBeenCalledTimes(1);
  });
  it('drops multiple media in pool order as one overwrite transaction, using source marks for their spacing', async () => {
    const { props, rerender, call } = mount();
    const items = [{ ...media, mark_in: { ticks: 25, scale: 25 }, mark_out: { ticks: 75, scale: 25 } }, { ...media, id: 'm2' }];
    rerender(<Timeline {...props} media={items} />);
    const dataTransfer = { types: [MEDIA_DRAG], dropEffect: 'none', getData: () => JSON.stringify({ session: 1, media: ['m1', 'm2'] }) };
    const lane = document.querySelector('.timeline-lane')!;
    fireEvent.dragOver(lane, { clientX: HEADER_WIDTH + 280, clientY: 45, dataTransfer });
    expect(dataTransfer.dropEffect).toBe('copy'); expect(call).not.toHaveBeenCalled();
    fireEvent.drop(lane, { clientX: HEADER_WIDTH + 280, clientY: 45, dataTransfer }); await settle();
    expect(call).toHaveBeenCalledWith('transaction.apply', { revision: 17, ops: [
      { edit: 'overwrite', timeline: 't1', at: 70, source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: null },
      { edit: 'overwrite', timeline: 't1', at: 120, source: { kind: 'media', media: 'm2' }, video_track: 'v1', audio_track: null },
    ] });
  });
  it('inserts on Shift-drop and ignores media drags from a different project session or onto locked tracks', async () => {
    const { props, rerender, call } = mount();
    const lane = document.querySelector('.timeline-lane')!;
    const dataTransfer = { types: [MEDIA_DRAG], getData: () => JSON.stringify({ session: 2, media: ['m1'] }) };
    fireEvent.drop(lane, { clientX: HEADER_WIDTH + 80, clientY: 45, dataTransfer }); await settle();
    expect(call).not.toHaveBeenCalled();
    dataTransfer.getData = () => JSON.stringify({ session: 1, media: ['m1'] });
    fireEvent.drop(lane, { clientX: HEADER_WIDTH + 80, clientY: 45, dataTransfer, shiftKey: true }); await settle();
    expect(call).toHaveBeenLastCalledWith('transaction.apply', { revision: 17, ops: [{ edit: 'insert', timeline: 't1', at: 20, source: { kind: 'media', media: 'm1' }, video_track: 'v1', audio_track: null }] });
    call.mockClear();
    rerender(<Timeline {...props} timeline={{ ...props.timeline, tracks: props.timeline.tracks.map(t => ({ ...t, locked: true })) }} />);
    fireEvent.drop(lane, { clientX: HEADER_WIDTH + 80, clientY: 45, dataTransfer }); await settle();
    expect(call).not.toHaveBeenCalled();
  });
});
