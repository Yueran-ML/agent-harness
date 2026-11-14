// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import { EditPage } from '../src/studio/pages/EditPage';
import { CutPage } from '../src/studio/pages/CutPage';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';
import type { MonitorControls } from '../src/studio/useMonitor';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function setup() {
  const selected: Timeline = { ...timeline([]), tracks: [{ id: 'a1', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [clip('sound')], transitions: [] }],
    buses: [{ id: 'B1', name: 'First', color: '#888888ff', format: 'stereo' }, { id: 'B2', name: 'Monitored', color: '#888888ff', format: '5.1' }], audio: { monitor: 'B2' } as never };
  const monitor = { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
  const reading = { bar: -9, peak: -4, bar_position: .42, peak_position: .8 };
  const call = vi.fn(async (method: string) => ({ ok: true, result: method === 'audio.meters' ? { timeline: 't1', playing: true, rate: 30, scale: 'iec', strips: [{ owner: { bus: 'B2' }, pre: [], post: Array(6).fill(reading) }], control_room: reading, loudness: {}, lost: 0 }
    : method === 'transport.load' ? monitor.state : method === 'audio.analyze' ? { job: 'job' } : { revision: 8, done: [] } }));
  const bridge = { call, onEvent: () => () => {}, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) } as unknown as EngineBridge;
  const props = { bridge, native: false, session: 1, revision: 7, pool: <div>Pool</div>, media: [media], timelines: [selected], selected, onSelect: vi.fn(), timelineMonitor: monitor, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  return { props, call, monitor };
}
it('mounts usable Fairlight strips and polls while either Mixer or Monitoring is visible', async () => {
  const { props, call } = setup(); const view = render(<FairlightPage {...props} />); await settle();
  expect(screen.getByRole('slider', { name: 'Fader Voice' })).toBeDefined();
  expect(defaultFairlightView().mixerHeight).toBeGreaterThanOrEqual(250);
  expect(document.querySelectorAll('.mixer-region')).toHaveLength(2);
  fireEvent.click(screen.getByRole('button', { name: 'Meters' })); await settle(); const n = call.mock.calls.filter(([m]) => m === 'audio.meters').length;
  await act(async () => vi.advanceTimersByTimeAsync(100)); expect(call.mock.calls.filter(([m]) => m === 'audio.meters').length).toBeGreaterThan(n);
  fireEvent.click(screen.getByRole('button', { name: 'Mixer' })); await settle(); const stopped = call.mock.calls.length;
  await act(async () => vi.advanceTimersByTimeAsync(150)); expect(call.mock.calls.length).toBe(stopped);
  view.unmount();
});

it.each(['edit', 'cut', 'fairlight'])('uses the actual displayed frame for the %s audio inspector and seeks through the same monitor', async page => {
  const { props, monitor } = setup();
  const track = props.selected.tracks[0];
  if (track.kind !== 'subtitle') track.clips[0] = { ...track.clips[0], gain_db: { keys: [
    { at: { ticks: 0, scale: 48000 }, value: -12, interp: 'linear' },
    { at: { ticks: 48000, scale: 48000 }, value: 0, interp: 'linear' },
  ] } };
  monitor.frame = 12.5;
  if (monitor.state) monitor.state = { ...monitor.state, frame: 90, shown: 10 };
  const extra = { source: { ...monitor, monitor: 'source' as const }, sourceMedia: null, focus: 'timeline' as const, onFocus: vi.fn() };
  const element = (session = 1) => page === 'fairlight' ? <FairlightPage {...props} session={session} /> : page === 'cut' ? <CutPage {...props} {...extra} bin={null} session={session} /> : <EditPage {...props} {...extra} session={session} />;
  const view = render(element());
  await settle();
  if (page !== 'fairlight') fireEvent.click(screen.getByRole('button', { name: 'Inspector' }));
  const sound = document.querySelectorAll<HTMLElement>('[data-clip="sound"]');
  fireEvent.pointerDown(sound[sound.length - 1], { button: 0, clientX: 310, clientY: 160 });
  fireEvent.pointerUp(sound[sound.length - 1], { button: 0, clientX: 310, clientY: 160 }); await settle();
  expect(document.querySelector<HTMLInputElement>('[data-field="volume"] input.inspector-number')!.value).toBe('-6.0');
  fireEvent.click(screen.getByRole('button', { name: 'Volume: Next keyframe' }));
  expect(monitor.seek).toHaveBeenCalledWith(25, false);
  fireEvent.click(screen.getByRole('button', { name: 'Change clip speed' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Pitch Correction' }));
  view.rerender(element(2)); await settle();
  expect(screen.queryByRole('dialog')).toBeNull();
});
it('shares Edit track selection and meters the exact monitored output, never the first bus fallback', async () => {
  const { props, call, monitor } = setup();
  const extra = { source: { ...monitor, monitor: 'source' as const }, sourceMedia: null, focus: 'timeline' as const, onFocus: vi.fn() };
  const view = render(<EditPage {...props} {...extra} />); await settle();
  expect(call.mock.calls.some(([m]) => m === 'audio.meters')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'Mixer' })); await settle();
  fireEvent.click(within(screen.getByRole('region', { name: 'Mixer channels' })).getByRole('button', { name: 'Voice' }));
  expect(document.querySelector('.timeline-track[data-track="a1"]')?.getAttribute('data-mixer-selected')).toBe('true');
  fireEvent.click(screen.getByRole('button', { name: 'Meters' })); await settle();
  expect(screen.getAllByRole('meter')).toHaveLength(6); expect(screen.getByText('Monitored')).toBeDefined();
  view.rerender(<EditPage {...props} selected={{ ...props.selected, audio: { monitor: null } as never }} {...extra} />); await settle();
  expect(screen.queryAllByRole('meter')).toHaveLength(0); expect(screen.getByText('No monitored bus')).toBeDefined();
});
it('sends actual analysis standard IDs from the page without changing timeline settings enums', async () => {
  const { props, call } = setup(); render(<FairlightPage {...props} />); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound"]')!, { clientX: 10, clientY: 10 });
  fireEvent.click(screen.getByText('Analyze Audio Level')); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Analyze' })); await settle();
  expect(call).toHaveBeenCalledWith('audio.analyze', expect.objectContaining({ standard: 'ebu-r128' }), { expectedSession: 1 });
});
it.each(['timeline', 'index'])('uses the external %s selection as the next Mixer range anchor', async origin => {
  const { props } = setup(); const first = props.selected.tracks[0];
  props.selected.tracks = [1, 2, 3, 4].map(n => ({ ...first, id: `a${n}`, name: `Audio ${n}`, clips: [], transitions: [] } as typeof first));
  render(<div className="studio"><FairlightPage {...props} /></div>); await settle();
  const mixer = within(screen.getByRole('region', { name: 'Mixer channels' }));
  fireEvent.click(mixer.getByRole('button', { name: 'Audio 1' }));
  if (origin === 'timeline') fireEvent.pointerDown(document.querySelector('.timeline [data-track="a2"] .timeline-lane')!, { button: 0 });
  else { fireEvent.click(screen.getByRole('button', { name: 'Index' })); fireEvent.click(within(screen.getByRole('region', { name: 'Index Tracks' })).getByRole('button', { name: 'a2' })); }
  fireEvent.click(mixer.getByRole('button', { name: 'Audio 4' }), { shiftKey: true });
  expect([...document.querySelectorAll('.mixer-strip[data-selected]')].map(el => el.getAttribute('data-mixer-strip'))).toEqual(['track:a2', 'track:a3', 'track:a4']);
  // Local Ctrl/Shift updates must retain that anchor, rather than following the new range end.
  fireEvent.click(mixer.getByRole('button', { name: 'Audio 1' }), { ctrlKey: true });
  fireEvent.click(mixer.getByRole('button', { name: 'Audio 3' }), { shiftKey: true });
  expect([...document.querySelectorAll('.mixer-strip[data-selected]')].map(el => el.getAttribute('data-mixer-strip'))).toEqual(['track:a2', 'track:a3']);
});
it.each(['edit', 'fairlight'])('preserves transport from ordinary %s strip focus, but not text editing or IME', async page => {
  const { props, monitor, call } = setup();
  render(<div className="studio">{page === 'fairlight' ? <FairlightPage {...props} /> : <EditPage {...props}
    source={{ ...monitor, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={() => {}} />}</div>); await settle();
  if (page === 'edit') { fireEvent.click(screen.getByRole('button', { name: 'Mixer' })); await settle(); }
  const cap = screen.getByRole('slider', { name: 'Fader Voice' });
  fireEvent.pointerDown(cap, { button: 0, pointerId: 1, clientY: 100 }); fireEvent.pointerUp(cap, { pointerId: 1 });
  fireEvent.keyDown(cap, { key: ' ' }); fireEvent.keyDown(cap, { key: 'l' }); fireEvent.keyDown(cap, { key: 'k' }); fireEvent.keyUp(cap, { key: 'k' }); fireEvent.keyDown(cap, { key: 'j' });
  expect(monitor.run).toHaveBeenCalledWith({ method: 'transport.play', params: { speed: 1 } });
  expect(monitor.run).toHaveBeenCalledWith({ method: 'transport.play', params: { speed: -1 } });
  expect(monitor.run).toHaveBeenCalledWith({ method: 'transport.pause' });
  const before = (monitor.run as ReturnType<typeof vi.fn>).mock.calls.length;
  fireEvent.keyDown(cap, { key: 'j', isComposing: true, keyCode: 229 });
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Fader value Voice' }));
  const input = screen.getByRole('textbox'); fireEvent.keyDown(input, { key: ' ' }); fireEvent.keyDown(input, { key: 'l' }); fireEvent.keyDown(input, { key: 'Escape' });
  fireEvent.doubleClick(within(screen.getByRole('region', { name: 'Mixer channels' })).getByRole('button', { name: 'Voice' }));
  fireEvent.keyDown(screen.getByRole('textbox'), { key: 'j' }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' });
  expect((monitor.run as ReturnType<typeof vi.fn>).mock.calls.length).toBe(before);
  fireEvent.keyDown(cap, { key: 'Delete' }); await settle(); expect(call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});
