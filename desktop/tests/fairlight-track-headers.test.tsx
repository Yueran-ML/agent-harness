// @vitest-environment jsdom
/**
 * Fairlight track headers (Reference Manual 20, pp. 3669–3671, 3716–3717, 3747–3748): the number,
 * format and meter of each track; Change Track Color and Lock Track Height To from the header's
 * menu; choosing tracks by Ctrl+click, Shift+click and a sweep of the headers; Alt and Ctrl+Alt
 * drags of a header's edge; and the keys that move the choice, jump to edges and fades, and zoom.
 */
import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Meters, Timeline } from '../src/studio/engine';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { useTrackChoice } from '../src/studio/fairlight/trackHeaders';
import { useFairlightView } from '../src/studio/fairlight/viewState';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const formats = ['mono', 'stereo', '5.1', 'adaptive'] as const;
const song = (): Timeline => ({
  ...timeline(),
  tracks: Array.from({ length: 4 }, (_, i) => ({
    id: `A${i + 1}`, kind: 'audio' as const, name: `Audio ${i + 1}`, enabled: true, locked: false,
    format: formats[i], channels: formats[i] === 'adaptive' ? 3 : undefined,
    clips: [clip(`c${i}`, 10 * i, 50, i === 1 ? { fade_in: 5, fade_out: 8 } : {})], transitions: [],
  })) as Timeline['tracks'],
});
type Bridge = Pick<EngineBridge, 'call' | 'onEvent'>;
const engine = () => ({ call: vi.fn<EngineBridge['call']>().mockResolvedValue({ ok: true, result: {} }), onEvent: () => () => {} });
function Page({ bridge, meters = null, onSeek = () => {}, frame = 0 }: {
  bridge: Bridge; meters?: Meters | null; onSeek?: (frame: number, scrubbing: boolean) => void; frame?: number;
}) {
  const [view, update] = useFairlightView(bridge, 1, 't1');
  const [chosen, setChosen] = useState<string[]>([]);
  return <FairlightTimeline view={view} update={update} bridge={bridge} session={1} timeline={song()} media={[media]} revision={4}
    frame={frame} playing={false} onSeek={onSeek} meters={meters} selectedTracks={chosen}
    onTrackSelect={id => setChosen([id])} onTracksSelect={setChosen} />;
}
const row = (id: string) => document.querySelector<HTMLElement>(`[data-track="${id}"]`)!;
const header = (id: string) => row(id).querySelector<HTMLElement>('.timeline-track-header')!;
const chosen = () => [...document.querySelectorAll<HTMLElement>('[data-mixer-selected]')].map(each => each.dataset.track);
const press = (target: Element, init: PointerEventInit = {}) => {
  fireEvent.pointerDown(target, { button: 0, buttons: 1, ...init });
  fireEvent.pointerUp(target, { button: 0 });
};
const timelineKey = (init: KeyboardEventInit) => fireEvent.keyDown(document.querySelector('.timeline')!, init);
beforeEach(() => {
  likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  // A view 424 pixels high: 394 below the ruler of 30.
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(424);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('shows each track number, format and its own meter in the header', () => {
  const channel = { bar: -12, peak: -10, bar_position: .8, peak_position: .83 };
  const meters = { timeline: 't1', playing: true, rate: 30, scale: 'iec', lost: 0, control_room: null,
    strips: [{ owner: { track: 'A2' }, pre: [], post: [channel, channel] }], loudness: {} } as unknown as Meters;
  render(<Page bridge={engine()} meters={meters} />);
  expect(header('A1').querySelector('[data-track-number]')!.textContent).toBe('A1');
  expect([1, 2, 3, 4].map(n => header(`A${n}`).querySelector('[data-track-format]')!.textContent)).toEqual(['Mono', 'Stereo', '5.1', 'Adaptive 3']);
  expect(screen.getAllByRole('meter', { name: /^Track meter Audio 2/ }).map(each => each.getAttribute('aria-valuenow'))).toEqual(['-12', '-12']);
  expect(screen.getAllByRole('meter', { name: /^Track meter Audio 3/ })).toHaveLength(6);
});

it('chooses tracks by click, Ctrl+click, Shift+click and a sweep down the headers', () => {
  render(<Page bridge={engine()} />);
  press(header('A1')); expect(chosen()).toEqual(['A1']);
  press(header('A3'), { ctrlKey: true }); expect(chosen()).toEqual(['A1', 'A3']);
  press(header('A1'), { ctrlKey: true }); expect(chosen()).toEqual(['A3']);
  press(header('A3')); press(header('A1'), { shiftKey: true }); expect(chosen()).toEqual(['A1', 'A2', 'A3']);
  fireEvent.pointerDown(header('A2'), { button: 0, buttons: 1 });
  fireEvent.pointerOver(header('A3'), { buttons: 1 }); fireEvent.pointerOver(header('A4'), { buttons: 1 });
  fireEvent.pointerUp(header('A4'), { button: 0 });
  expect(chosen()).toEqual(['A2', 'A3', 'A4']);
  // A pass over a header without the button down chooses nothing more.
  fireEvent.pointerOver(header('A1'), { buttons: 0 }); expect(chosen()).toEqual(['A2', 'A3', 'A4']);
  // Ctrl+Alt+Up and Down move the choice; at the top it stays.
  timelineKey({ key: 'ArrowUp', ctrlKey: true, altKey: true }); expect(chosen()).toEqual(['A1', 'A2', 'A3']);
  timelineKey({ key: 'ArrowUp', ctrlKey: true, altKey: true }); expect(chosen()).toEqual(['A1', 'A2', 'A3']);
  timelineKey({ key: 'ArrowDown', ctrlKey: true, altKey: true }); expect(chosen()).toEqual(['A2', 'A3', 'A4']);
});

it('changes the colour of the tracks chosen in one transaction from the header menu', async () => {
  const bridge = engine(); render(<Page bridge={bridge} />);
  press(header('A2')); press(header('A4'), { ctrlKey: true });
  fireEvent.contextMenu(header('A4'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Change Track Color' }));
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'Color 3' })); });
  const writes = bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply');
  expect(writes).toHaveLength(1);
  expect((writes[0][1] as { ops: unknown[] }).ops).toEqual(['A2', 'A4'].map(id => ({ op: 'set_track', timeline: 't1', id, color: '#dc8a54ff' })));
  // A header outside the choice changes its own track only.
  fireEvent.contextMenu(header('A1'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Change Track Color' }));
  await act(async () => { fireEvent.click(screen.getByRole('menuitem', { name: 'Color 1' })); });
  expect(bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply')[1][1]).toMatchObject({ ops: [{ id: 'A1', color: '#7f7f7fff' }] });
});

it('locks track heights to a preset that wheels and group drags leave alone, without a project edit', () => {
  const bridge = engine(); render(<Page bridge={bridge} />);
  press(header('A1')); press(header('A2'), { ctrlKey: true });
  fireEvent.contextMenu(header('A2'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Lock Track Height To' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Micro' }));
  expect([row('A1').style.height, row('A2').style.height]).toEqual(['24px', '24px']);
  expect(screen.getByRole('separator', { name: 'Track height: Audio 1' }).getAttribute('aria-disabled')).toBe('true');
  fireEvent.wheel(header('A3'), { deltaY: -100, shiftKey: true });
  expect([row('A1').style.height, row('A3').style.height]).toEqual(['24px', '136px']);
  // Ctrl+Alt+drag sizes every track not locked; Alt+drag the tracks chosen.
  const edge = screen.getByRole('separator', { name: 'Track height: Audio 3' });
  fireEvent.pointerDown(edge, { button: 0, clientY: 100, altKey: true, ctrlKey: true });
  fireEvent.pointerMove(edge, { clientY: 80 }); fireEvent.pointerUp(edge);
  expect(['A1', 'A2', 'A3', 'A4'].map(id => row(id).style.height)).toEqual(['24px', '24px', '116px', '116px']);
  fireEvent.contextMenu(header('A1'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Lock Track Height To' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'None' }));
  press(header('A1')); press(header('A4'), { ctrlKey: true });
  const first = screen.getByRole('separator', { name: 'Track height: Audio 1' });
  fireEvent.pointerDown(first, { button: 0, clientY: 100, altKey: true });
  fireEvent.pointerMove(first, { clientY: 150 }); fireEvent.pointerUp(first);
  expect(['A1', 'A2', 'A3', 'A4'].map(id => row(id).style.height)).toEqual(['74px', '24px', '116px', '74px']);
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});

it('writes a name at a single click on it, and not at a Ctrl+click that chooses', () => {
  render(<Page bridge={engine()} />);
  fireEvent.click(header('A2').querySelector('.timeline-track-name')!, { ctrlKey: true });
  expect(header('A2').querySelector('input')).toBeNull();
  fireEvent.click(header('A2').querySelector('.timeline-track-name')!);
  expect((header('A2').querySelector('input') as HTMLInputElement).value).toBe('Audio 2');
});

it('jumps to clip edges of the tracks chosen, to fades, and zooms the chosen tracks to the view and back', () => {
  const onSeek = vi.fn();
  render(<Page bridge={engine()} onSeek={onSeek} frame={12} />);
  timelineKey({ key: 'ArrowRight', ctrlKey: true, altKey: true }); expect(onSeek).not.toHaveBeenCalled();
  press(header('A3'));
  timelineKey({ key: 'ArrowRight', ctrlKey: true, altKey: true }); expect(onSeek).toHaveBeenLastCalledWith(20, false);
  timelineKey({ key: 'ArrowLeft', ctrlKey: true, altKey: true }); expect(onSeek).toHaveBeenCalledTimes(1);
  fireEvent.change(screen.getByLabelText('Jump to'), { target: { value: 'fade' } });
  timelineKey({ key: 'ArrowDown' }); expect(onSeek).toHaveBeenLastCalledWith(15, false);
  timelineKey({ key: 'ArrowUp' }); expect(onSeek).toHaveBeenLastCalledWith(10, false);
  press(header('A4'), { ctrlKey: true });
  timelineKey({ key: 'e', ctrlKey: true, altKey: true });
  expect(['A1', 'A2', 'A3', 'A4'].map(id => row(id).style.height)).toEqual(['128px', '128px', '197px', '197px']);
  expect(document.querySelector<HTMLElement>('.timeline-scroll')!.scrollTop).toBe(256);
  timelineKey({ key: 'e', ctrlKey: true, altKey: true });
  expect(['A1', 'A3'].map(id => row(id).style.height)).toEqual(['128px', '128px']);
});

it('hides clip names for Full Waveforms and outlines waveforms for Waveform Borders', () => {
  render(<Page bridge={engine()} />);
  const section = document.querySelector('.fairlight-timeline')!;
  expect(section.hasAttribute('data-full-waveforms')).toBe(false);
  fireEvent.click(screen.getByLabelText('Full Waveforms')); fireEvent.click(screen.getByLabelText('Waveform Borders'));
  expect(section.hasAttribute('data-full-waveforms')).toBe(true);
  expect(section.hasAttribute('data-waveform-borders')).toBe(true);
});

it('sizes every unlocked track from the vertical slider, keeping the top chosen track in place', () => {
  render(<Page bridge={engine()} />);
  press(header('A3'));
  const scroller = document.querySelector<HTMLElement>('.timeline-scroll')!;
  scroller.scrollTop = 300; fireEvent.scroll(scroller);
  // A3's middle is 128 * 2 + 64 - 300 = 20 pixels down the view; at 64 pixels a track it stays there.
  fireEvent.change(screen.getByLabelText('Track heights'), { target: { value: '64' } });
  expect(['A1', 'A2', 'A3', 'A4'].map(id => row(id).style.height)).toEqual(['64px', '64px', '64px', '64px']);
  expect(scroller.scrollTop).toBe(64 * 2 + 32 - 20);
  expect((screen.getByLabelText('Track heights') as HTMLInputElement).value).toBe('64');
});

it('shows on the slider the height of a track it sizes, not of one whose height is locked', () => {
  render(<Page bridge={engine()} />);
  fireEvent.contextMenu(header('A1'));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Lock Track Height To' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Micro' }));
  fireEvent.change(screen.getByLabelText('Track heights'), { target: { value: '64' } });
  expect(['A1', 'A2'].map(id => row(id).style.height)).toEqual(['24px', '64px']);
  expect((screen.getByLabelText('Track heights') as HTMLInputElement).value).toBe('64');
});

it('forgets a Ctrl+click that a lane kept from the timeline, so the next click chooses', () => {
  const onTrack = vi.fn();
  function Rows() {
    const choice = useTrackChoice({ selected: [], onTracks: () => {}, onTrack });
    return <div className="fairlight-timeline" onPointerDownCapture={choice.pointerDown} onPointerUpCapture={choice.pointerUp}>
      {['A1', 'A2'].map(id => <div key={id} className="timeline-track" data-track={id}>
        <div className="timeline-track-header" onPointerDown={() => choice.trackSelect(id)}>
          <span data-test={`lane-${id}`} onPointerDown={event => event.stopPropagation()} />
        </div>
      </div>)}
    </div>;
  }
  render(<Rows />);
  press(document.querySelector('[data-test="lane-A1"]')!, { ctrlKey: true });
  expect(onTrack).not.toHaveBeenCalled();
  press(header('A2'));
  expect(onTrack).toHaveBeenCalledWith('A2');
});

it('marks the playhead on the scroll bar while it is off the view, and brings it back as playing starts without scrolling', () => {
  const bridge = engine();
  function Playing({ frame, playing }: { frame: number; playing: boolean }) {
    const [view, update] = useFairlightView(bridge, 1, 't1');
    return <FairlightTimeline view={view} update={update} bridge={bridge} session={1} timeline={song()} media={[media]} revision={4}
      frame={frame} playing={playing} onSeek={() => {}} />;
  }
  const mounted = render(<Playing frame={0} playing={false} />);
  fireEvent.change(screen.getByLabelText('Timeline Scrolling'), { target: { value: 'none' } });
  mounted.rerender(<Playing frame={1} playing={false} />);
  expect(screen.queryByRole('img', { name: 'Playhead out of view' })).toBeNull();
  // 800 pixels of view at 4 a frame: frame 300 is off it.
  mounted.rerender(<Playing frame={300} playing={false} />);
  expect(screen.getByRole('img', { name: 'Playhead out of view' })).toBeTruthy();
  expect(document.querySelector<HTMLElement>('.timeline-scroll')!.scrollLeft).toBe(0);
  mounted.rerender(<Playing frame={300} playing />);
  expect(document.querySelector<HTMLElement>('.timeline-scroll')!.scrollLeft).toBe(1200 - 80);
  mounted.rerender(<Playing frame={301} playing />);
  expect(screen.queryByRole('img', { name: 'Playhead out of view' })).toBeNull();
});

