// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { Toolbar } from '../src/studio/fairlight/Toolbar';
import { useFairlightView } from '../src/studio/fairlight/viewState';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

const song = (id: string) => ({ ...timeline(), id, tracks: Array.from({ length: 8 }, (_, i) => ({
  id: `a${i}`, kind: 'audio' as const, name: `Audio ${i}`, enabled: true, locked: false,
  clips: [clip(`c${i}`, 0, 3000)], transitions: [],
})) });
type Bridge = Pick<EngineBridge, 'call' | 'onEvent'>;
function Page({ bridge, session = 1, id = 't1', frame = 0, playing = false }: {
  bridge: Bridge; session?: number; id?: string; frame?: number; playing?: boolean;
}) {
  const [view, update, reset] = useFairlightView(bridge, session, id);
  const selected = song(id);
  return <><Toolbar view={view} update={update} reset={reset} timelines={[selected]} selected={id} onSelect={() => {}} />
    <FairlightTimeline view={view} update={update} bridge={bridge} session={session} timeline={selected}
      media={[media]} revision={0} frame={frame} playing={playing} onSeek={() => {}} /></>;
}
const scroll = () => document.querySelector<HTMLElement>('.timeline-scroll')!;
const scale = () => Number(document.querySelector('.timeline')!.getAttribute('data-scale'));
const height = () => document.querySelector<HTMLElement>('[data-track="a0"]')!.style.height;
const selectMode = (value: string) => fireEvent.change(screen.getByLabelText('Timeline Scrolling'), { target: { value } });
const engine = () => ({ call: vi.fn<EngineBridge['call']>().mockResolvedValue({ ok: true, result: {} }), onEvent: () => () => {} });
beforeEach(() => {
  likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent);
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(400);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ left: 0, top: 0, width: 956, height: 400, right: 956, bottom: 400, x: 0, y: 0, toJSON() {} });
});
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

it('remembers zoom, individual heights and both scroll axes across leaving the page, without a project edit', () => {
  const bridge = engine(); const mounted = render(<Page bridge={bridge} />);
  fireEvent.keyDown(document.querySelector('.timeline')!, { key: '=', ctrlKey: true });
  fireEvent.keyDown(screen.getByRole('separator', { name: 'Track height: Audio 0' }), { key: 'ArrowDown' });
  expect(scale()).toBe(5); expect(height()).toBe('136px');
  scroll().scrollLeft = 720; scroll().scrollTop = 175; fireEvent.scroll(scroll());
  // Leave immediately, before a scheduled animation frame can save the scroll.
  mounted.unmount(); render(<Page bridge={bridge} />);
  expect(scale()).toBe(5); expect(height()).toBe('136px');
  expect(scroll().scrollLeft).toBe(720); expect(scroll().scrollTop).toBe(175);
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});

it('isolates timeline, session and bridge views and resets the current timeline only', () => {
  const bridge = engine(); const mounted = render(<Page key="1/t1" bridge={bridge} />);
  fireEvent.keyDown(document.querySelector('.timeline')!, { key: '=', ctrlKey: true });
  fireEvent.keyDown(screen.getByRole('separator', { name: 'Track height: Audio 0' }), { key: 'ArrowDown' });
  selectMode('none'); scroll().scrollLeft = 640; scroll().scrollTop = 90; fireEvent.scroll(scroll());
  mounted.rerender(<Page key="1/t2" bridge={bridge} id="t2" />);
  expect(scale()).toBe(4); expect(height()).toBe('128px'); expect(scroll().scrollLeft).toBe(0);
  fireEvent.keyDown(document.querySelector('.timeline')!, { key: '=', ctrlKey: true });
  mounted.rerender(<Page key="1/t1" bridge={bridge} />);
  expect(scale()).toBe(5); expect(height()).toBe('136px'); expect(scroll().scrollTop).toBe(90);
  expect((screen.getByLabelText('Timeline Scrolling') as HTMLSelectElement).value).toBe('none');
  fireEvent.click(screen.getByText('Reset Fairlight Layout'));
  expect(scale()).toBe(4); expect(height()).toBe('128px'); expect(scroll().scrollLeft).toBe(0); expect(scroll().scrollTop).toBe(0);
  expect((screen.getByLabelText('Timeline Scrolling') as HTMLSelectElement).value).toBe('page');
  mounted.rerender(<Page key="1/t2" bridge={bridge} id="t2" />); expect(scale()).toBe(5);
  mounted.rerender(<Page key="2/t2" bridge={bridge} session={2} id="t2" />); expect(scale()).toBe(4);
  mounted.rerender(<Page key="other" bridge={engine()} id="t2" />); expect(scale()).toBe(4);
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});

it('pages at the visible edge in both playback directions, fixes the playhead, or leaves scrolling alone', () => {
  const bridge = engine(); const mounted = render(<Page bridge={bridge} frame={0} playing />);
  expect((screen.getByLabelText('Timeline Scrolling') as HTMLSelectElement).value).toBe('page');
  mounted.rerender(<Page bridge={bridge} frame={190} playing />); expect(scroll().scrollLeft).toBe(0);
  mounted.rerender(<Page bridge={bridge} frame={201} playing />); expect(scroll().scrollLeft).toBe(800);
  mounted.rerender(<Page bridge={bridge} frame={199} playing />); expect(scroll().scrollLeft).toBe(0);
  selectMode('fixed'); mounted.rerender(<Page bridge={bridge} frame={400} playing />);
  expect(scroll().scrollLeft).toBe(1200);
  selectMode('none'); mounted.rerender(<Page bridge={bridge} frame={900} playing />);
  expect(scroll().scrollLeft).toBe(1200);
  scroll().scrollLeft = 100; fireEvent.scroll(scroll());
  mounted.rerender(<Page bridge={bridge} frame={1000} playing />); expect(scroll().scrollLeft).toBe(100);
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});

it('pages when dragging the playhead beyond the edge but does not seek while changing view mode', async () => {
  const bridge = engine(); const seek = vi.fn();
  function DragPage() {
    const [view, update] = useFairlightView(bridge, 1, 'drag');
    return <FairlightTimeline bridge={bridge} session={1} revision={0} timeline={song('drag')} media={[media]}
      view={view} update={update} frame={0} playing={false} onSeek={seek} />;
  }
  render(<DragPage />); selectMode('fixed'); selectMode('page'); expect(seek).not.toHaveBeenCalled();
  fireEvent.pointerDown(document.querySelector('.timeline-playhead')!, { button: 0, clientX: 160 });
  fireEvent.pointerMove(document.querySelector('.timeline')!, { clientX: 1000 });
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  expect(scroll().scrollLeft).toBe(800);
});
