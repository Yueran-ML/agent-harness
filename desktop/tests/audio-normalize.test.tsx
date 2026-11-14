// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { NormalizeDialog, useNormalizeAudio } from '../src/studio/inspector/NormalizeDialog';
import { EditPage } from '../src/studio/pages/EditPage';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import type { EngineBridge } from '../src/bridge';
import type { AudioStandard, EngineAnswer, EngineEvent, Timeline } from '../src/studio/engine';
import type { MonitorControls } from '../src/studio/useMonitor';
import { Words } from '../src/app/locale';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { ja } from '../src/messages/ja';
import { overlayOpen } from '../src/app/overlay';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const standards: AudioStandard[] = [
  { id: 'server-standard', name: 'Server Standard', target: -17.5, tolerance: 1, true_peak: -1, measure: 'gated', dialogue: false, resolve: true, official: true, verified: true, source: 'server' },
  { id: 'timeline-standard', name: 'Timeline Standard', target: null, tolerance: null, true_peak: null, measure: 'ungated', dialogue: false, resolve: true, official: false, verified: false, source: 'server' },
];
function setup(page: 'Edit' | 'Fairlight' = 'Edit', words = en) {
  const selected: Timeline = { ...timeline([clip('video')]), audio: { loudness: { target: -20.5 } } as never };
  selected.tracks.push(...['sound1', 'sound2', 'locked'].map((id, i) => ({ id: `a${i}`, name: id, kind: 'audio' as const, enabled: true, locked: id === 'locked', clips: [clip(id)], transitions: [] })));
  const listeners = new Set<(event: EngineEvent) => void>();
  const monitor = { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 99, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 99, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 99 }), run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() } as MonitorControls;
  const call = vi.fn<EngineBridge['call']>(async method => ({ ok: true, result: method === 'audio.standards' ? { standards } : method === 'transport.load' ? monitor.state
    : method === 'audio.normalize' ? { job: 'mine' } : method === 'audio.meters' ? { timeline: 't1', strips: [], loudness: {}, lost: 0 } : { pending: true } }));
  const bridge = { call, onEvent: (fn: (event: EngineEvent) => void) => { listeners.add(fn); return () => { listeners.delete(fn); }; }, viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const props = { bridge, native: false, session: 1, revision: 7, pool: <div>Pool</div>, media: [media], timelines: [selected], selected, onSelect: vi.fn(), timelineMonitor: monitor, noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const draw = () => <Words.Provider value={words}><div data-test="studio">{page === 'Edit' ? <EditPage {...props} source={{ ...monitor, monitor: 'source' }} sourceMedia={null} focus="timeline" onFocus={vi.fn()} /> : <FairlightPage {...props} />}</div></Words.Provider>;
  const view = render(draw());
  const tell = (params: unknown, session = 1) => act(() => listeners.forEach(fn => fn({ kind: 'notification', method: 'audio.analysis', session, params })));
  const open = async (id = 'sound1') => { await settle(); fireEvent.contextMenu(document.querySelector(`[data-clip="${id}"]`)!, { clientX: 10, clientY: 10 }); fireEvent.click(screen.getByRole('menuitem', { name: words.label.studio.fairlightPage['Normalize Audio Levels'] })); await settle(); };
  return { props, view, draw, tell, call, monitor, open, listeners };
}
function selectClips(ids: string[]) {
  for (const [index, id] of ids.entries()) {
    const element = document.querySelector('[data-clip="' + id + '"]')!;
    fireEvent.pointerDown(element, { button: 0, ctrlKey: index > 0, clientX: 250, clientY: 30, pointerId: 1 });
    fireEvent.pointerUp(element, { button: 0, ctrlKey: index > 0, clientX: 250, clientY: 30, pointerId: 1 });
  }
}
const submit = () => fireEvent.click(screen.getByRole('button', { name: 'Normalize' }));
const done = (extra = {}) => ({ job: 'mine', state: 'done', method: 'audio.normalize', level: 'sample_peak', target: -9, mode: 'relative', measured: -15,
  clips: [{ clip: 'sound1', measured: -15, gain_db: 6, clamped: false }], applied: { revision: 8, done: [], names: {}, next_id: 20, clamped: [] }, ...extra });

it.each(['Edit', 'Fairlight'] as const)('opens shared Normalize from %s context and Audio menu with defaults and current revision', async page => {
  const app = setup(page); await settle(); selectClips(['sound1']); await app.open();
  expect(screen.getByRole('dialog', { name: 'Normalize Audio Levels' })).toBeDefined();
  expect((screen.getByLabelText('Normalization mode') as HTMLSelectElement).value).toBe('sample_peak');
  expect((screen.getByLabelText('Target (dBFS)') as HTMLInputElement).value).toBe('-9');
  expect(screen.queryByLabelText('Set Level')).toBeNull();
  app.props.revision = 12; app.view.rerender(app.draw()); submit(); submit(); await settle();
  expect(app.call.mock.calls.filter(([m]) => m === 'audio.normalize')).toHaveLength(1);
  expect(app.call).toHaveBeenCalledWith('audio.normalize', { timeline: 't1', clips: ['sound1'], level: 'sample_peak', target: -9, mode: 'relative', apply: true, revision: 12 }, { expectedSession: 1 });
  app.tell(done()); fireEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Close' })[0]);
  fireEvent.click(screen.getByRole('button', { name: 'Audio' }));
  fireEvent.click(screen.getByRole('menuitem', { name: 'Normalize Audio Levels' })); await settle();
  expect(screen.getByRole('dialog', { name: 'Normalize Audio Levels' })).toBeDefined();
});
it('reads standard targets from the server, uses timeline target for null, validates target and switches units', async () => {
  const app = setup(); await app.open();
  expect(app.call).toHaveBeenCalledWith('audio.standards', {}, { expectedSession: 1 });
  fireEvent.change(screen.getByLabelText('Normalization mode'), { target: { value: 'server-standard' } });
  expect((screen.getByLabelText('Target (LUFS)') as HTMLInputElement).value).toBe('-17.5');
  fireEvent.change(screen.getByLabelText('Normalization mode'), { target: { value: 'timeline-standard' } });
  expect((screen.getByLabelText('Target (LUFS)') as HTMLInputElement).value).toBe('-20.5');
  fireEvent.change(screen.getByLabelText('Normalization mode'), { target: { value: 'true_peak' } });
  const target = screen.getByLabelText('Target (dBFS)');
  for (const value of ['', '-61', '1']) { fireEvent.change(target, { target: { value } }); expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(true); }
  fireEvent.change(target, { target: { value: '-6.5' } }); submit(); await settle();
  expect(app.call).toHaveBeenCalledWith('audio.normalize', expect.objectContaining({ level: 'true_peak', target: -6.5 }), { expectedSession: 1 });
});
it('keeps selected multiple audio clips, skips locked/video, and right click outside selection uses only that clip', async () => {
  const app = setup(); await settle();
  app.props.selected.tracks[3].locked = false; app.view.rerender(app.draw());
  selectClips(['video', 'sound1', 'sound2', 'locked']);
  app.props.selected = { ...app.props.selected, tracks: app.props.selected.tracks.map(track => track.id === 'a2' ? { ...track, locked: true } : track) }; app.view.rerender(app.draw());
  await app.open(); expect(screen.getByLabelText('Set Level')).toBeDefined();
  expect((screen.getByLabelText('Set Level') as HTMLSelectElement).value).toBe('relative');
  fireEvent.change(screen.getByLabelText('Set Level'), { target: { value: 'independent' } }); submit(); await settle();
  expect(app.call).toHaveBeenCalledWith('audio.normalize', expect.objectContaining({ clips: ['sound1', 'sound2'], mode: 'independent' }), { expectedSession: 1 });
  app.tell(done()); fireEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Close' })[0]);
  fireEvent.pointerDown(document.querySelector('.timeline-lane')!, { button: 0 }); selectClips(['sound1']); await app.open('sound2'); submit(); await settle();
  expect(app.call).toHaveBeenLastCalledWith('audio.normalize', expect.objectContaining({ clips: ['sound2'], mode: 'relative' }), { expectedSession: 1 });
});
it('disables empty and locked selections in both menu entry paths', async () => {
  const app = setup(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Audio' }));
  expect((screen.getByRole('menuitem', { name: 'Normalize Audio Levels' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  fireEvent.contextMenu(document.querySelector('[data-clip="locked"]')!);
  expect((screen.getByRole('menuitem', { name: 'Normalize Audio Levels' }) as HTMLButtonElement).disabled).toBe(true);
  expect(app.call.mock.calls.some(([m]) => m === 'audio.normalize')).toBe(false);
});
it('accepts only its acknowledged job including an early terminal and displays actual measured gain and application', async () => {
  const app = setup(); await app.open(); let ack!: (answer: EngineAnswer) => void;
  app.call.mockImplementationOnce(() => new Promise(resolve => { ack = resolve; })); submit(); await settle();
  app.tell({ job: 'other', state: 'failed', message: 'other failure' }); app.tell(done());
  expect(screen.queryByText('Applied')).toBeNull();
  await act(async () => ack({ ok: true, result: { job: 'mine' } }));
  expect(screen.getByText('Applied')).toBeDefined();
  expect(screen.getByText('6.0 dB')).toBeDefined(); expect(screen.getAllByText('-15.0 dBFS').length).toBeGreaterThan(0);
  expect(screen.queryByText('other failure')).toBeNull();
});
it.each([
  [done({ applied: null, clips: [{ clip: 'sound1', measured: null, gain_db: null, clamped: false }], measured: null }), 'Nothing applied'],
  [done({ applied: null, refused: { code: -32001, message: 'revision changed', data: { reason: 'stale_revision' } } }), 'revision changed'],
  [done({ clips: [{ clip: 'sound1', measured: -40, gain_db: 31, clamped: true }] }), 'clamped'],
  [{ job: 'mine', state: 'failed', message: 'decode failed' }, 'decode failed'],
  [{ job: 'mine', state: 'cancelled' }, 'Cancelled'],
])('shows truthful terminal state %#', async (event, text) => {
  const app = setup(); await app.open(); submit(); await settle(); app.tell(event);
  expect(screen.getAllByText(text as string).length).toBeGreaterThan(0);
  if (text === 'clamped') expect(within(screen.getByRole('table')).getAllByText('clamped')).toHaveLength(2);
  if (text === 'Nothing applied') { expect(screen.getAllByText('No measurable level (silence or too short)').length).toBeGreaterThan(0); expect(screen.queryByText('Applied')).toBeNull(); }
});
it('isolates keys and IME, keeps cancellation pending and reports completion winning cancellation', async () => {
  const app = setup(); await app.open(); expect(overlayOpen()).toBe(true);
  for (const key of [' ', 'j', 'l', 'Delete', 'F4']) fireEvent.keyDown(screen.getByLabelText('Target (dBFS)'), { key, isComposing: key === 'l' });
  expect(app.monitor.run).not.toHaveBeenCalled(); expect(app.call.mock.calls.some(([m]) => m === 'transaction.apply')).toBe(false);
  submit(); await settle(); fireEvent.click(screen.getByRole('button', { name: 'Cancel analysis' })); await settle();
  expect(screen.queryByText('Cancelled')).toBeNull(); expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(true);
  app.tell(done()); expect(screen.getByText('Applied')).toBeDefined();
});
it.each(['close', 'session', 'timeline', 'unmount'] as const)('cancels late acknowledgement safely after %s and rejects stale events', async change => {
  const app = setup(); await app.open(); let ack!: (answer: EngineAnswer) => void;
  app.call.mockImplementationOnce(() => new Promise(resolve => { ack = resolve; })); submit(); await settle();
  if (change === 'close') fireEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Close' })[0]);
  else if (change === 'session') { app.props.session = 2; app.view.rerender(app.draw()); }
  else if (change === 'timeline') { app.props.selected = { ...app.props.selected, id: 'new-timeline' }; app.view.rerender(app.draw()); }
  else app.view.unmount();
  await settle(); await act(async () => ack({ ok: true, result: { job: 'late' } }));
  expect(app.call).toHaveBeenCalledWith('audio.cancel', { job: 'late' }, { expectedSession: 1 });
  app.tell(done()); expect(screen.queryByText('Applied')).toBeNull(); expect(screen.queryByRole('dialog')).toBeNull();
});
it.each([en, zhCN, ja])('uses shared three-language labels', async words => {
  const app = setup('Edit', words); await app.open();
  expect(screen.getByRole('dialog', { name: words.label.studio.fairlightPage['Normalize Audio Levels'] })).toBeDefined();
  expect(screen.getByLabelText(words.label.studio.fairlightPage['Normalization mode'])).toBeDefined();
});

it('shows cancellation errors without releasing the running job, permits retry, and clears the error on completion', async () => {
  const app = setup(); await app.open(); submit(); await settle();
  app.call.mockResolvedValueOnce({ ok: false, error: { code: -32000, message: 'cancel connection failure', data: null } });
  fireEvent.click(screen.getByRole('button', { name: 'Cancel analysis' })); await settle();
  // The general sentence in the page's language, with what the engine said.
  expect(screen.getByRole('alert').textContent).toBe(`${en.hint.studio.edit.failed} cancel connection failure`);
  expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Cancel analysis' })); await settle();
  expect(app.call.mock.calls.filter(([m]) => m === 'audio.cancel')).toHaveLength(2);
  app.tell(done()); expect(screen.getByText('Applied')).toBeDefined(); expect(screen.queryByRole('alert')).toBeNull();
});
it('reports a refused start, permits retry, and cancels an acknowledged running job on close', async () => {
  const app = setup(); await app.open();
  app.call.mockResolvedValueOnce({ ok: false, error: { code: -32001, message: 'stale revision', data: { reason: 'stale_revision' } } });
  submit(); await settle(); expect(screen.getByRole('alert').textContent).toBe(en.hint.studio.edit.stale_revision);
  expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(false);
  submit(); await settle(); fireEvent.click(within(screen.getByRole('dialog')).getAllByRole('button', { name: 'Close' })[0]); await settle();
  expect(app.call).toHaveBeenCalledWith('audio.cancel', { job: 'mine' }, { expectedSession: 1 });
});
it('accepts a terminal received while waiting after cancel without inventing cancellation or cancelling a completed job', async () => {
  const app = setup(); await app.open(); let ack!: (answer: EngineAnswer) => void;
  app.call.mockImplementationOnce(() => new Promise(resolve => { ack = resolve; })); submit(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel analysis' }));
  app.tell(done()); await act(async () => ack({ ok: true, result: { job: 'mine' } }));
  expect(screen.getByText('Applied')).toBeDefined(); expect(app.call.mock.calls.some(([m]) => m === 'audio.cancel')).toBe(false);
});
it('cancels an acknowledged job if cancellation was requested before acknowledgement and waits for real cancellation', async () => {
  const app = setup(); await app.open(); let ack!: (answer: EngineAnswer) => void;
  app.call.mockImplementationOnce(() => new Promise(resolve => { ack = resolve; })); submit(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel analysis' }));
  await act(async () => ack({ ok: true, result: { job: 'mine' } }));
  expect(app.call).toHaveBeenCalledWith('audio.cancel', { job: 'mine' }, { expectedSession: 1 });
  expect(screen.queryByText('Cancelled')).toBeNull(); app.tell({ job: 'mine', state: 'cancelled' }); expect(screen.getByText('Cancelled')).toBeDefined();
});
it('ignores other sessions and jobs while its job runs', async () => {
  const app = setup(); await app.open(); submit(); await settle();
  app.tell(done(), 2); app.tell(done({ job: 'other' }));
  expect(screen.queryByText('Applied')).toBeNull(); expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(true);
  app.tell(done()); expect(screen.getByText('Applied')).toBeDefined();
});

it('enforces 256 targets at the shared menu and dialog and accepts inclusive target boundaries', async () => {
  const app = setup(); app.view.unmount();
  const many = Array.from({ length: 257 }, (_, i) => clip('clip-' + i));
  const song: Timeline = { ...timeline([]), tracks: [{ id: 'a1', name: 'Audio', kind: 'audio', enabled: true, locked: false, clips: many, transitions: [] }] };
  function Menu({ ids }: { ids: string[] }) { const audio = useNormalizeAudio({ bridge: app.props.bridge, session: 1, revision: 7, timeline: song, selection: ids }); return <>{audio.menu}{audio.dialog}</>; }
  const menu = render(<Menu ids={many.map(clip => clip.id)} />);
  fireEvent.click(screen.getByRole('button', { name: 'Audio' }));
  expect((screen.getByRole('menuitem', { name: 'Normalize Audio Levels' }) as HTMLButtonElement).disabled).toBe(true);
  menu.unmount();
  const props = { bridge: app.props.bridge, session: 1, revision: 7, timeline: song, clips: many.map(clip => clip.id), onClose: vi.fn() };
  const dialog = render(<NormalizeDialog {...props} />); await settle();
  expect(screen.getByRole('alert').textContent).toBe('Select at most 256 audio clips.');
  submit(); expect(app.call.mock.calls.some(([m]) => m === 'audio.normalize')).toBe(false);
  dialog.rerender(<NormalizeDialog {...props} clips={props.clips.slice(0, 256)} />);
  for (const value of ['-60', '0']) { fireEvent.change(screen.getByLabelText('Target (dBFS)'), { target: { value } }); expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(false); }
  submit(); await settle();
  expect(app.call).toHaveBeenCalledWith('audio.normalize', expect.objectContaining({ clips: props.clips.slice(0, 256), target: 0 }), { expectedSession: 1 });
});
it('rechecks locks and clip existence at submit, deduplicates targets and disables when none remain', async () => {
  const app = setup(); app.view.unmount();
  const props = { bridge: app.props.bridge, session: 1, revision: 7, timeline: app.props.selected, clips: ['sound1', 'sound1', 'sound2', 'video', 'locked', 'missing'], onClose: vi.fn() };
  const dialog = render(<NormalizeDialog {...props} />); await settle();
  const changed = { ...props.timeline, tracks: props.timeline.tracks.filter(track => track.id !== 'a1') };
  dialog.rerender(<NormalizeDialog {...props} timeline={changed} revision={8} />); submit(); await settle();
  expect(app.call).toHaveBeenCalledWith('audio.normalize', expect.objectContaining({ clips: ['sound1'], mode: 'relative', revision: 8 }), { expectedSession: 1 });
  app.tell(done()); dialog.rerender(<NormalizeDialog {...props} timeline={{ ...changed, tracks: changed.tracks.map(track => ({ ...track, locked: true })) }} />);
  expect((screen.getByRole('button', { name: 'Normalize' }) as HTMLButtonElement).disabled).toBe(true);
});
it('reports standards lookup failure while peak normalization stays available', async () => {
  const app = setup(); await settle();
  const original = app.call.getMockImplementation()!;
  app.call.mockImplementation((method, params, context) => method === 'audio.standards' ? Promise.resolve({ ok: false, error: { code: -32000, message: 'standards unavailable', data: null } }) : original(method, params, context));
  await app.open(); expect(screen.getByRole('alert').textContent).toBe('standards unavailable');
  submit(); await settle(); expect(app.call).toHaveBeenCalledWith('audio.normalize', expect.objectContaining({ level: 'sample_peak' }), { expectedSession: 1 });
});
