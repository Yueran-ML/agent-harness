// @vitest-environment jsdom
import { useState, type ComponentProps } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useAudioReadings, useMeterReading } from '../src/studio/fairlight/audioReadings';
import { Monitoring } from '../src/studio/fairlight/Monitoring';
import { IndexTracks } from '../src/studio/fairlight/IndexTracks';
import { LevelMatcher } from '../src/studio/fairlight/LevelMatcher';
import { FairlightPage } from '../src/studio/pages/FairlightPage';
import { FairlightTimeline } from '../src/studio/fairlight/FairlightTimeline';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { MEDIA_DRAG } from '../src/studio/media/drag';
import { ChannelLanes } from '../src/studio/fairlight/ChannelLanes';
import { Inspector as FairlightInspector } from '../src/studio/fairlight/Inspector';
import { Toolbar } from '../src/studio/fairlight/Toolbar';
import { EffectsLibrary, AUDIO_EFFECT_DRAG } from '../src/studio/fairlight/EffectsLibrary';
import { Waveforms } from '../src/studio/timeline/waveforms';
import { Words } from '../src/app/locale';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { ja } from '../src/messages/ja';
import { Studio } from '../src/studio/Studio';
import { ProjectCopy, applyOp, type EngineAnswer, type EngineEvent, type LoudnessReading, type Meters, type Project } from '../src/studio/engine';
import type { MonitorControls } from '../src/studio/useMonitor';
import type { EngineBridge } from '../src/bridge';
import { likeABrowser } from './mounted';
import { clip, media, timeline } from './timeline-fixture';

const loudness: LoudnessReading = { running: true, standard: 'ebu_r128', target: -23, momentary: -25, short_term: -24,
  momentary_max: -21, short_term_max: -22, integrated: -23, range: 3, true_peak: -1.4, sample_peak: -2, seconds: 12, verdict: 'within' };
const meters: Meters = { timeline: 't1', playing: true, rate: 30, scale: 'iec', strips: [{ owner: { track: 'a1' },
  pre: [], post: [{ bar: -6, peak: -2, bar_position: .85, peak_position: .95 }] }], control_room: null, loudness, lost: 0 };
const song = () => ({ ...timeline(), tracks: [timeline().tracks[0], ...['a1', 'a2', 'a3'].map((id, i) => ({
  id, name: `Audio ${i + 1}`, kind: 'audio' as const, enabled: true, locked: false, format: 'stereo' as const,
  clips: [clip(`sound${i + 1}`)], transitions: [],
}))] });
function engine() {
  const listeners = new Set<(event: EngineEvent) => void>();
  const bridge = { call: vi.fn<EngineBridge['call']>(async (method: string) => ({ ok: true as const, result: method === 'audio.meters' ? meters : method === 'audio.loudness' ? loudness : { job: 'audio-1' } })),
    onEvent: (listener: (event: EngineEvent) => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; } };
  return { ...bridge, listeners, tell: (params: unknown, session = 1) => act(() => listeners.forEach(fn => fn({ kind: 'notification', method: 'audio.analysis', session, params }))) };
}
const capturedProfile = { source: { timeline: 't1', clip: 'sound1', revision: 7 }, profile: { version: 1 as const, level_db: -22.123456, basis: 'integrated' as const }, seconds: 2 };
function Matcher(props: Omit<ComponentProps<typeof LevelMatcher>, 'profile' | 'onCapture'>) {
  const [profile, setProfile] = useState<ComponentProps<typeof LevelMatcher>['profile']>(null);
  return <LevelMatcher {...props} profile={profile} onCapture={setProfile} />;
}
const captured = (bridge: ReturnType<typeof engine>) => bridge.tell({ job: 'audio-1', state: 'done', method: 'audio.level_profile', ...capturedProfile });
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
beforeEach(() => { likeABrowser(); vi.useFakeTimers(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function Readings({ bridge, session = 1, visible = true }: { bridge: ReturnType<typeof engine>; session?: number; visible?: boolean }) {
  const audio = useAudioReadings(bridge, session, 't1', visible);
  const bar = useMeterReading(audio.readings, value => value?.strips[0]?.post[0].bar);
  return <><output data-testid="reading">{bar ?? 'none'}</output><button onClick={() => void audio.analyze(['sound1'], 'ebu_r128')}>Analyze</button><button onClick={audio.cancel}>Cancel</button><output data-testid="analysis">{audio.analysis?.state}</output></>;
}
it('does not rerender the owning page for identical stopped meters, but publishes peak resets', async () => {
  const bridge = engine();
  let next = { ...meters, playing: false, loudness: { ...loudness, running: false } };
  bridge.call.mockImplementation(async () => ({ ok: true, result: structuredClone(next) }));
  let renders = 0;
  function Host() {
    renders++;
    const audio = useAudioReadings(bridge, 1, 't1', true);
    // Whatever subscribes to the whole reading is drawn again for each new one, and only then.
    const meters = useMeterReading(audio.readings, value => value);
    return <button onClick={() => void audio.resetPeaks()}>{meters?.strips[0].post[0].peak ?? 'none'}</button>;
  }
  render(<Host />); await settle();
  await act(async () => vi.advanceTimersByTimeAsync(200));
  const stopped = renders;
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(renders).toBe(stopped);
  next = { ...next, strips: [{ ...next.strips[0], post: [{ ...next.strips[0].post[0], peak: null, peak_position: null }] }] };
  fireEvent.click(screen.getByRole('button')); await settle();
  expect(screen.getByRole('button').textContent).toBe('none');
  expect(renders).toBeGreaterThan(stopped);
});
it('polls only while visible, releases its listener and rejects a late session reply', async () => {
  const bridge = engine();
  const view = render(<Readings bridge={bridge} />); await settle();
  expect(screen.getByTestId('reading').textContent).toBe('-6');
  expect(bridge.listeners.size).toBe(1);
  view.rerender(<Readings bridge={bridge} visible={false} />); await settle();
  const calls = bridge.call.mock.calls.length;
  await act(async () => { await vi.advanceTimersByTimeAsync(500); });
  expect(bridge.call.mock.calls.length).toBe(calls);
  view.unmount(); expect(bridge.listeners.size).toBe(0);
  let finish!: (value: Awaited<ReturnType<typeof bridge.call>>) => void;
  bridge.call.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  const next = render(<Readings bridge={bridge} />); await settle();
  next.rerender(<Readings bridge={bridge} session={2} visible={false} />); await settle();
  await act(async () => finish({ ok: true, result: meters }));
  expect(screen.getByTestId('reading').textContent).toBe('none');
});

it('accepts an early analysis terminal only for the acknowledged job and cancels a late acknowledgement in its original session', async () => {
  const bridge = engine(); let finish!: (value: EngineAnswer) => void;
  bridge.call.mockImplementation(async method => method === 'audio.analyze' ? await new Promise<EngineAnswer>(resolve => { finish = resolve; }) : { ok: true, result: meters });
  const view = render(<Readings bridge={bridge} />); await settle();
  fireEvent.click(screen.getByText('Analyze')); await settle();
  bridge.tell({ job: 'unrelated', state: 'failed', message: 'someone else' });
  bridge.tell({ job: 'mine', state: 'done', method: 'audio.analyze', standard: 'ebu_r128', level: -23, results: [] });
  expect(screen.getByTestId('analysis').textContent).toBe('');
  await act(async () => finish({ ok: true, result: { job: 'mine' } }));
  expect(screen.getByTestId('analysis').textContent).toBe('done');
  fireEvent.click(screen.getByText('Analyze')); await settle();
  view.rerender(<Readings bridge={bridge} session={2} />); await settle();
  await act(async () => finish({ ok: true, result: { job: 'late' } }));
  expect(bridge.call).toHaveBeenCalledWith('audio.cancel', { job: 'late' }, { expectedSession: 1 });
  expect(screen.getByTestId('analysis').textContent).toBe('');
});

it('keeps Level Matcher pending until its terminal event, reports refusal, and cancels on leaving', async () => {
  const bridge = engine(); const view = render(<Matcher bridge={bridge} session={1} timeline={song()} revision={7} clips={['sound1', 'sound2']} />);
  fireEvent.click(screen.getByText('Capture Level Profile')); await settle(); captured(bridge); await settle(); fireEvent.click(screen.getByText('Apply Level Profile')); await settle();
  expect((screen.getByText('Apply Level Profile') as HTMLButtonElement).disabled).toBe(true);
  bridge.tell({ job: 'audio-1', state: 'done', method: 'audio.level_match', reference: { clip: 'sound1', measured: -22 }, clips: [], applied: null, refused: { code: -32001, message: 'revision changed' } });
  expect(screen.getByRole('alert').textContent).toBe('revision changed');
  fireEvent.click(screen.getByText('Apply Level Profile')); await settle(); view.unmount();
  expect(bridge.call).toHaveBeenCalledWith('audio.cancel', { job: 'audio-1' }, { expectedSession: 1 });
});

function controls(): MonitorControls {
  return { monitor: 'timeline', state: { monitor: 'timeline', timeline: 't1', frame: 0, shown: 0, playing: false, speed: 1, last: 199, exact: true, sound: 'off', source: null, proxies: true, late: 0 },
    rate: { num: 25, den: 1 }, frame: 0, last: 199, loaded: true, held: false, context: () => ({ playing: false, speed: 1, fps: 25, last: 199 }),
    run: vi.fn(), seek: vi.fn(), drag: vi.fn(), proxies: vi.fn() };
}

it('mounts the full Fairlight page with channel lanes, shared editing, view-only layout and integer transport', async () => {
  const bridge = { ...engine(), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) }; const monitor = controls();
  const selected = song();
  selected.tracks.push({ ...selected.tracks[1], id: 'surround', format: '5.1', clips: [clip('surround-clip')] } as never);
  selected.tracks.push({ ...selected.tracks[1], id: 'adaptive', format: 'adaptive', channels: 24, clips: [clip('adaptive-clip')] } as never);
  const view = render(<div data-test="studio"><FairlightPage bridge={bridge} native={false} session={1} revision={7} pool={<span>Shared pool</span>} media={[media]} timelines={[selected]}
    selected={selected} onSelect={() => {}} timelineMonitor={monitor} noticeOf={() => null} onRetry={() => {}} onProblem={() => {}} /></div>);
  await settle();
  expect(document.querySelectorAll('[data-clip="sound1"] [data-channel]')).toHaveLength(2);
  expect(document.querySelectorAll('[data-clip="surround-clip"] [data-channel]')).toHaveLength(6);
  expect(document.querySelectorAll('[data-clip="adaptive-clip"] [data-channel]')).toHaveLength(24);
  expect(document.querySelectorAll('[data-clip="adaptive-clip"] .audio-overlay')).toHaveLength(1);
  expect(document.querySelectorAll('[data-clip="surround-clip"] .audio-volume')).toHaveLength(1);
  fireEvent.click(screen.getByLabelText('Show Clip Gain Line'));
  expect(document.querySelectorAll('.audio-volume')).toHaveLength(0);
  expect(document.querySelector('[data-clip="surround-clip"] .audio-fades')).not.toBeNull();
  fireEvent.click(screen.getByLabelText('Show Clip Gain Line'));
  expect(document.querySelectorAll('[data-clip="surround-clip"] .audio-volume')).toHaveLength(1);
  expect(document.querySelector('[data-track="v1"]')).toBeNull();
  fireEvent.click(screen.getByLabelText('Video tracks')); expect(document.querySelector('[data-track="v1"]')).not.toBeNull();
  fireEvent.click(screen.getByText('Index')); fireEvent.click(screen.getByLabelText('Visible Audio 2'));
  expect(document.querySelector('.timeline [data-track="a2"]')).toBeNull();
  fireEvent.keyDown(document.body, { key: 'l' }); expect(monitor.run).toHaveBeenCalledWith({ method: 'transport.play', params: { speed: 1 } });
  fireEvent.keyDown(document.body, { key: 'F4' }); expect(document.querySelector('.fairlight-timeline')?.getAttribute('data-view')).toBe('clips');
  fireEvent.click(screen.getByText('Toggle Automation')); await settle();
  fireEvent.keyDown(document.body, { key: 'F4' }); expect(document.querySelector('.fairlight-timeline')?.getAttribute('data-view')).toBe('automation');
  expect(document.querySelector('[data-clip="sound1"] .audio-overlay')).toBeNull();
  fireEvent.keyDown(screen.getByRole('separator', { name: 'Monitoring height' }), { key: 'ArrowDown' });
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
  fireEvent.click(screen.getByText('Reset Fairlight Layout')); expect(screen.queryByText('Tracks')).toBeNull();
  expect(document.querySelector('.fairlight-timeline')?.getAttribute('data-view')).toBe('clips');
  const drop = { types: [AUDIO_EFFECT_DRAG], getData: () => 'gain' };
  fireEvent.drop(document.querySelector('[data-clip="sound1"]')!, { dataTransfer: drop }); await settle();
  expect(bridge.call).toHaveBeenCalledWith('transaction.apply', { revision: 7, ops: [{ op: 'add_insert', timeline: 't1', of: { clip: 'sound1' }, insert: { on: true, effect: { kind: 'gain' } } }] }, { expectedSession: 1 });
  view.unmount(); expect(bridge.listeners.size).toBe(0);
});

it('Studio opens Fairlight on Windows and stops polling when switching back to Edit', async () => {
  const listeners = new Set<(event: EngineEvent) => void>();
  const project: Project = { schema: 5, next_id: 20, groups: [], media: [media], bins: [], timelines: [song()] };
  const bridge = { open: vi.fn(async () => { const opened = { session: 1, project: 'F:/film', owned: true, pid: 1, revision: 0 }; listeners.forEach(fn => fn({ kind: 'opened', ...opened })); return opened; }),
    call: vi.fn(async (method: string) => ({ ok: true, result: method === 'project.get' ? { project, revision: 0, next_id: 20 } : method === 'audio.meters' ? meters : method === 'transport.state' || method === 'transport.load' ? controls().state : {} })),
    onEvent: (fn: (event: EngineEvent) => void) => { listeners.add(fn); return () => { listeners.delete(fn); }; },
    viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}), close: vi.fn(async () => {}) } as unknown as EngineBridge;
  const copy = new ProjectCopy(bridge);
  const drawn = render(<Studio cwd="F:/film" engine={{ bridge, copy, native: false }} language="en" onClose={() => {}} />); await settle();
  fireEvent.click(screen.getByRole('tab', { name: 'Fairlight' })); await settle();
  expect(document.querySelector('[data-test="fairlight-page"]')).not.toBeNull();
  const mock = bridge.call as ReturnType<typeof vi.fn>;
  const counted = () => mock.mock.calls.filter(([method]) => method === 'audio.meters').length;
  expect(counted()).toBeGreaterThan(0);
  fireEvent.click(screen.getByRole('button', { name: 'Media Pool' }));
  fireEvent.change(screen.getByLabelText('Media audio filter'), { target: { value: 'audio-only' } });
  expect(document.querySelector('.pool [data-media="m1"]')).toBeNull();
  fireEvent.change(screen.getByLabelText('Media audio filter'), { target: { value: 'with-audio' } });
  expect(document.querySelector('.pool [data-media="m1"]')).not.toBeNull();
  fireEvent.click(screen.getByRole('tab', { name: 'Edit' })); await settle(); const count = counted();
  await act(async () => { await vi.advanceTimersByTimeAsync(300); }); expect(counted()).toBe(count);
  drawn.unmount(); copy.dispose();
});

it.each([en, zhCN, ja])('uses the catalogue for Fairlight controls, effects, inspector and monitoring', words => {
  const t = words.label.studio.fairlightPage; const bridge = engine();
  render(<Words.Provider value={words}><Toolbar view={defaultFairlightView()} update={() => {}} reset={() => {}} timelines={[song()]} selected="t1" onSelect={() => {}} />
    <EffectsLibrary /><FairlightInspector bridge={bridge} session={1} revision={0} timeline={song()} media={[media]} clips={[]} track={null} onEdit={() => {}} />
    <Monitoring timeline={song()} meters={meters} onAction={() => {}} onResetPeaks={() => {}} onPatch={() => {}} /></Words.Provider>);
  expect(screen.getByRole('button', { name: t['Reset Fairlight Layout'] })).toBeDefined();
  expect(screen.getByRole('button', { name: `${t.Favorite} ${t.Gain}` })).toBeDefined();
  expect(screen.getByRole('tab', { name: t.Audio })).toBeDefined();
  expect(screen.getByLabelText(t['Loudness standard'])).toBeDefined();
  expect(screen.getByText(t['Select audio clips'])).toBeDefined();
});

it('keeps Fairlight Audio on audio controls for linked video/audio clips and shows no timeline settings when empty', () => {
  const bridge = engine(); const selected = song();
  const view = render(<FairlightInspector bridge={bridge} session={1} revision={0} timeline={selected} media={[media]} clips={['c1', 'sound1']} track="a1" onEdit={() => {}} />);
  expect(document.querySelector('[data-panel="audio"]')).not.toBeNull();
  expect(document.querySelector('[data-panel="video"]')).toBeNull();
  view.rerender(<FairlightInspector bridge={bridge} session={1} revision={0} timeline={selected} media={[media]} clips={[]} track={null} onEdit={() => {}} />);
  expect(screen.getByText('Select audio clips')).toBeDefined();
  expect(document.querySelector('.inspector-panel')).toBeNull();
});

it('requests an explicit non-default stream and never draws an unrelated waveform for a source mix', async () => {
  const bridge = engine(); bridge.call.mockResolvedValue({ ok: true, result: { pending: true } });
  const waves = new Waveforms(bridge, 1); waves.connect();
  const sound = clip('mapped', 0, 100, { channels: [{ source: { stream: 3, channel: 0 } }, { source: [{ stream: 1, channel: 0 }, { stream: 3, channel: 1 }] }] });
  render(<ChannelLanes clip={sound} track={song().tracks[1]} media={{ ...media, facts: { ...media.facts, audio: { ...media.facts.audio!, streams: [{ index: 3, channels: 2, layout: 'stereo' }] } } }} rate={{ num: 25, den: 1 }} scale={4} from={0} to={100} waves={waves} gain={1} />);
  await settle();
  expect(bridge.call).toHaveBeenCalledWith('media.waveform', { media: 'm1', stream: 3 }, { expectedSession: 1 });
  expect(bridge.call).toHaveBeenCalledTimes(2);
  expect(bridge.call).toHaveBeenCalledWith('media.waveform', { media: 'm1', mix: [{ stream: 1, channel: 0 }, { stream: 3, channel: 1 }] }, { expectedSession: 1 });
  expect(screen.queryByText('Mixed waveform unavailable')).toBeNull();
  expect(document.querySelector('[data-channel="1"] svg')).toBeNull(); waves.disconnect();
});

it('replays moves and color without replacing any track contents', () => {
  const project: Project = { schema: 5, next_id: 20, groups: [], media: [media], bins: [], timelines: [song()] };
  const original = project.timelines[0].tracks[3];
  const moved = applyOp(project, { op: 'move_track', timeline: 't1', id: 'a3', index: 1 });
  expect(moved.timelines[0].tracks[1]).toBe(original);
  const colored = applyOp(moved, { op: 'set_track', timeline: 't1', id: 'a3', color: '#5aa0e0ff' });
  expect(colored.timelines[0].tracks[1].color).toBe('#5aa0e0ff');
});

it('uses rendered rows for shared clip selection, vertical dragging and audio-only pool drops', async () => {
  const bridge = engine(); const selected = song(); const selection = vi.fn();
  bridge.call.mockImplementation(async method => ({ ok: true, result: method === 'media.waveform' ? { pending: true } : { revision: 8, done: [], names: {}, next_id: 20, clamped: [] } } as never));
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(956);
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function(this: HTMLElement) {
    const row = this.closest('.timeline-track'); const index = [...document.querySelectorAll('.timeline-track')].indexOf(row!);
    const top = index < 0 ? 0 : 30 + index * 128; const left = this.hasAttribute('data-clip') ? 156 : 0;
    const width = this.hasAttribute('data-clip') ? 400 : 956;
    return { x: left, y: top, left, top, width, height: 128, bottom: top + 128, right: left + width, toJSON() {} };
  });
  render(<div data-test="studio"><FairlightTimeline timeline={selected} media={[media]} bridge={bridge} session={1} revision={7} frame={0} playing={false} onSeek={() => {}}
    onSelection={selection} view={{ ...defaultFairlightView(), hidden: ['a2'] }} update={() => {}} /></div>); await settle();
  const first = document.querySelector<HTMLElement>('[data-clip="sound1"]')!;
  fireEvent.pointerDown(first, { button: 0, clientX: 300, clientY: 80, pointerId: 1 });
  fireEvent.pointerUp(first, { button: 0, clientX: 300, clientY: 80, pointerId: 1 });
  expect(selection).toHaveBeenLastCalledWith(['sound1']);
  fireEvent.pointerDown(first, { button: 0, clientX: 300, clientY: 80, pointerId: 1 });
  fireEvent.pointerMove(first, { buttons: 1, clientX: 300, clientY: 200, shiftKey: true, pointerId: 1 });
  fireEvent.pointerUp(first, { button: 0, clientX: 300, clientY: 200, shiftKey: true, pointerId: 1 }); await settle();
  const transactions = () => bridge.call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params as { ops: Record<string, unknown>[] });
  expect(transactions().at(-1)?.ops[0]).toMatchObject({ edit: 'move', clips: ['sound1'], to_track: 'a3', delta: 0 });
  const lane = document.querySelector<HTMLElement>('[data-track="a3"] .timeline-lane')!;
  const dataTransfer = { types: [MEDIA_DRAG], getData: () => JSON.stringify({ session: 1, media: ['m1'] }), dropEffect: '' };
  fireEvent.drop(lane, { clientX: 556, clientY: 200, dataTransfer }); await settle();
  expect(transactions().at(-1)?.ops[0]).toMatchObject({ edit: 'overwrite', audio_track: 'a3', video_track: null });
});
it('shows engine dB directly and standard verdict colors, relative loudness and controls', () => {
  const action = vi.fn(); const patch = vi.fn();
  render(<Monitoring timeline={song()} meters={meters} onAction={action} onPatch={patch} onResetPeaks={() => {}} />);
  expect(screen.getByText('-6.0 dBFS')).toBeDefined();
  expect(screen.getByTestId('loudness-integrated').textContent).toContain('0.0 LU');
  expect(screen.getByTestId('loudness-integrated').dataset.verdict).toBe('within');
  fireEvent.click(screen.getByRole('button', { name: 'Pause' })); expect(action).toHaveBeenCalledWith('pause');
  fireEvent.change(screen.getByLabelText('Loudness standard'), { target: { value: 'atsc_a85' } });
  expect(patch).toHaveBeenCalledWith({ loudness: { standard: 'atsc_a85' } });
});
it('maps a visible audio track move to the final full track index and uses ADC audio patch', () => {
  const edit = vi.fn();
  render(<IndexTracks timeline={song()} hidden={[]} onHidden={() => {}} onSelect={() => {}} onEdit={edit} />);
  fireEvent.click(screen.getByRole('button', { name: 'Move Audio 3 up' }));
  expect(edit).toHaveBeenCalledWith([{ op: 'move_track', timeline: 't1', id: 'a3', index: 2 }]);
  fireEvent.dragStart(document.querySelector('[data-index-track="a3"]')!, { dataTransfer: { setData: vi.fn() } });
  fireEvent.drop(document.querySelector('[data-index-track="a1"]')!);
  expect(edit).toHaveBeenCalledWith([{ op: 'move_track', timeline: 't1', id: 'a3', index: 1 }]);
  fireEvent.click(screen.getByLabelText('ADC Audio 1'));
  expect(edit).toHaveBeenCalledWith([{ op: 'set_track_audio', timeline: 't1', track: 'a1', patch: { adc: false } }]);
});
it('captures a reference in page state and applies Level Matcher through the real method', async () => {
  const bridge = engine();
  render(<Matcher bridge={bridge} session={1} timeline={song()} revision={7} clips={['sound1', 'sound2']} />);
  fireEvent.click(screen.getByRole('button', { name: 'Capture Level Profile' })); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.level_profile', { timeline: 't1', clip: 'sound1', revision: 7 }, { expectedSession: 1 });
  captured(bridge); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Apply Level Profile' })); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.level_match', { timeline: 't1', profile: capturedProfile.profile, clips: ['sound1', 'sound2'], apply: true, revision: 7 }, { expectedSession: 1 });
});


it('commits an Index control sweep once, including the start and repeated crossings, and discards cancellation', () => {
  const edit = vi.fn();
  const view = render(<IndexTracks timeline={song()} hidden={[]} onHidden={() => {}} onSelect={() => {}} onEdit={edit} />);
  const first = screen.getByRole('button', { name: 'Mute Audio 1' });
  const second = screen.getByRole('button', { name: 'Mute Audio 2' });
  const third = screen.getByRole('button', { name: 'Mute Audio 3' });
  // The mixer's switch edits: one per member, as a mixer strip's mute writes them.
  const mute = (track: string) => ({ edit: 'set_track_audio', timeline: 't1', track, patch: { mute: true }, follow_groups: false });
  fireEvent.pointerDown(first, { button: 0, buttons: 1 });
  fireEvent.pointerEnter(second, { buttons: 1 });
  fireEvent.pointerEnter(first, { buttons: 1 });
  fireEvent.pointerEnter(third, { buttons: 1 });
  fireEvent.pointerEnter(second, { buttons: 1 });
  expect(edit).not.toHaveBeenCalled();
  fireEvent.pointerUp(third, { button: 0 });
  fireEvent.click(third, { detail: 1 });
  expect(edit.mock.calls).toEqual([[[mute('a1'), mute('a2'), mute('a3')]]]);
  edit.mockClear();
  fireEvent.pointerDown(first, { button: 0, buttons: 1 });
  fireEvent.pointerUp(first, { button: 0 });
  fireEvent.click(first, { detail: 1 });
  expect(edit.mock.calls).toEqual([[[mute('a1')]]]);
  edit.mockClear();
  fireEvent.click(first, { detail: 0 }); // Keyboard activation still has one transaction.
  expect(edit.mock.calls).toEqual([[[mute('a1')]]]);
  edit.mockClear();
  for (const cancel of [() => fireEvent.pointerCancel(window), () => fireEvent.keyDown(window, { key: 'Escape' }), () => fireEvent.blur(window)]) {
    fireEvent.pointerDown(first, { button: 0, buttons: 1 });
    fireEvent.pointerEnter(second, { buttons: 1 });
    cancel(); fireEvent.pointerUp(second); fireEvent.click(second, { detail: 1 });
    expect(edit).not.toHaveBeenCalled();
  }
  fireEvent.pointerDown(first, { button: 0, buttons: 1 });
  fireEvent.pointerEnter(second, { buttons: 1 });
  fireEvent.pointerUp(document.body);
  expect(edit.mock.calls).toEqual([[[mute('a1'), mute('a2')]]]);
  edit.mockClear();
  fireEvent.pointerDown(first, { button: 0, buttons: 1 });
  view.unmount(); fireEvent.pointerUp(window);
  expect(edit).not.toHaveBeenCalled();
});

function mountLayout(native = false) {
  const bridge = { ...engine(), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const selected = song(); const monitor = controls();
  const view = render(<div data-test="studio"><FairlightPage bridge={bridge} native={native} session={1} revision={7} pool={null} media={[media]} timelines={[selected]}
    selected={selected} onSelect={() => {}} timelineMonitor={monitor} noticeOf={() => null} onRetry={() => {}} onProblem={() => {}} /></div>);
  return { ...view, bridge, monitor };
}
it('resizes each panel in the direction of its dragged boundary without adding history', async () => {
  const { bridge } = mountLayout(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Index' }));
  const move = (name: string, x: number, y: number, expected: number) => {
    const handle = screen.getByRole('separator', { name });
    fireEvent.pointerDown(handle, { button: 0, clientX: 100, clientY: 100 });
    fireEvent.pointerMove(handle, { clientX: 100 + x, clientY: 100 + y });
    fireEvent.pointerUp(handle);
    expect(handle.getAttribute('aria-valuenow')).toBe(String(expected));
  };
  move('Inspector width', 20, 0, 260);
  move('Mixer height', 0, 20, defaultFairlightView().mixerHeight - 20);
  move('Left panel width', 20, 0, 360);
  move('Monitoring height', 0, 20, 260);
  expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
});
it('shows the native cinema viewer from every entry when the embedded viewer is off and restores its setting', async () => {
  const { bridge } = mountLayout(true); await settle();
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  fireEvent.click(screen.getByRole('button', { name: /^Viewer$/ }));
  expect(document.querySelector('.fairlight-small-viewer')).toBeNull();
  for (const enter of [() => fireEvent.click(screen.getByRole('button', { name: 'Cinema viewer' })),
    () => fireEvent.keyDown(document.body, { key: 'p' }), () => fireEvent.keyDown(document.body, { key: 'f', ctrlKey: true })]) {
    bridge.viewShow.mockClear(); enter();
    await act(async () => { await vi.advanceTimersByTimeAsync(20); });
    expect(document.querySelector('.fairlight-cinema [data-test="viewer"]')).not.toBeNull();
    expect(bridge.viewShow).toHaveBeenLastCalledWith(true, 'timeline');
    fireEvent.keyDown(document.body, { key: 'Escape' });
    expect(document.querySelector('.fairlight-cinema')).toBeNull();
    expect(document.querySelector('.fairlight-small-viewer')).toBeNull();
    expect(screen.getByRole('button', { name: /^Viewer$/ }).getAttribute('aria-pressed')).toBe('false');
    expect(bridge.viewShow).toHaveBeenLastCalledWith(false, 'timeline');
  }
});


it.each([en, zhCN, ja])(
  'requests automatic remixes and keeps default 5.1 samples for Film display', async words => {
    for (const [channels, format, order] of [[1, 'stereo', 'smpte'], [2, 'mono', 'smpte'], [6, '5.1', 'film']] as const) {
      const bridge = engine(); bridge.call.mockResolvedValue({ ok: true, result: { pending: true } });
      const waves = new Waveforms(bridge, 1); waves.connect();
      const track = { ...song().tracks[1], format, order } as never;
      const view = render(<Words.Provider value={words}><ChannelLanes clip={clip('auto')} track={track}
        media={{ ...media, facts: { ...media.facts, audio: { ...media.facts.audio!, channels } } }}
        rate={{ num: 25, den: 1 }} scale={4} from={0} to={100} waves={waves} gain={1} /></Words.Provider>);
      await settle();
      expect(bridge.call).toHaveBeenCalledExactlyOnceWith('media.waveform', {
        media: media.id, ...(format === '5.1' ? {} : { remix: format === 'mono' ? 1 : 2 }),
      }, { expectedSession: 1 });
      view.unmount(); waves.disconnect();
    }
  });
it('keeps directly corresponding automatic mono, stereo and SMPTE waveforms', async () => {
  for (const [channels, format] of [[1, 'mono'], [2, 'stereo'], [6, '5.1']] as const) {
    const bridge = engine(); bridge.call.mockResolvedValue({ ok: true, result: { rate: 100, channels,
      peaks: Buffer.from(new Int8Array(Array.from({ length: 800 * channels }, (_, i) => i % 2 ? 64 : -64)).buffer).toString('base64') } });
    const waves = new Waveforms(bridge, 1); waves.connect();
    const view = render(<ChannelLanes clip={clip('auto')} track={{ ...song().tracks[1], format, order: 'smpte' } as never}
      media={{ ...media, facts: { ...media.facts, audio: { ...media.facts.audio!, channels } } }}
      rate={{ num: 25, den: 1 }} scale={4} from={0} to={100} waves={waves} gain={1} />);
    await settle();
    expect(bridge.call).toHaveBeenCalledWith('media.waveform', { media: 'm1' }, { expectedSession: 1 });
    expect(document.querySelectorAll('.timeline-waveform')).toHaveLength(channels);
    expect([...document.querySelectorAll('.timeline-waveform')].map(node => node.getAttribute('viewBox')))
      .toEqual(Array.from({ length: channels }, (_, i) => `0 ${i * 60 / channels + 2} 400 ${60 / channels}`));
    view.unmount(); waves.disconnect();
  }
});
it('registers audio menus and analysis sheets as overlays, hides native views and isolates page shortcuts', async () => {
  const { bridge, monitor } = mountLayout(true); await settle();
  await act(async () => { await vi.advanceTimersByTimeAsync(20); });
  const selectedClip = document.querySelector('[data-clip="sound1"]')!;
  fireEvent.pointerDown(selectedClip, { button: 0, clientX: 300, clientY: 80 });
  fireEvent.pointerUp(selectedClip, { button: 0, clientX: 300, clientY: 80 });
  expect(selectedClip.getAttribute('aria-selected')).toBe('true');
  const openMenu = () => fireEvent.contextMenu(selectedClip, { clientX: 100, clientY: 100 });
  const isolated = () => {
    (monitor.run as ReturnType<typeof vi.fn>).mockClear();
    for (const key of ['j', 'k', 'l', 'F4', 'b', 'Delete', 'Backspace']) fireEvent.keyDown(document.activeElement!, { key });
    expect(monitor.run).not.toHaveBeenCalled();
    expect(document.querySelector('.fairlight-timeline')?.getAttribute('data-view')).toBe('clips');
    expect(document.querySelector('.timeline')?.getAttribute('data-tool')).toBe('select');
    expect(bridge.call.mock.calls.some(([method]) => method === 'transaction.apply')).toBe(false);
  };
  openMenu();
  expect(document.body.dataset.overlay).toBe('1');
  expect(bridge.viewShow).toHaveBeenLastCalledWith(false, 'timeline'); isolated();
  fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Analyze Audio Level' }), { key: 'Escape' });
  expect(screen.queryByRole('menu')).toBeNull();
  expect(bridge.viewShow).toHaveBeenLastCalledWith(true, 'timeline');
  openMenu(); fireEvent.pointerDown(document.body);
  expect(screen.queryByRole('menu')).toBeNull();
  expect(document.body.dataset.overlay).toBeUndefined();
  openMenu(); fireEvent.click(screen.getByRole('menuitem', { name: 'Analyze Audio Level' }));
  const dialog = screen.getByRole('dialog', { name: 'Analyze Audio Level' });
  expect(dialog.hasAttribute('open')).toBe(true);
  expect(document.body.dataset.overlay).toBe('1');
  expect(bridge.viewShow).toHaveBeenLastCalledWith(false, 'timeline'); isolated();
  // Sheet uses the native dialog cancel event for Escape; backdrop presses do not dismiss it.
  fireEvent.pointerDown(document.body); expect(dialog.hasAttribute('open')).toBe(true);
  fireEvent(dialog, new Event('cancel', { bubbles: true, cancelable: true }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(document.body.dataset.overlay).toBeUndefined();
  expect(bridge.viewShow).toHaveBeenLastCalledWith(true, 'timeline');
});

it('keeps cancellation pending until the engine terminal and admits completion winning the race', async () => {
  const bridge = engine(); render(<Readings bridge={bridge} />); await settle();
  fireEvent.click(screen.getByText('Analyze')); await settle();
  fireEvent.click(screen.getByText('Cancel')); await settle();
  expect(screen.getByTestId('analysis').textContent).toBe('running');
  fireEvent.click(screen.getByText('Analyze')); await settle();
  expect(bridge.call.mock.calls.filter(([method]) => method === 'audio.analyze')).toHaveLength(1);
  bridge.tell({ job: 'audio-1', state: 'done', method: 'audio.analyze', standard: 'ebu-r128', level: -23, results: [] });
  expect(screen.getByTestId('analysis').textContent).toBe('done');
});
it('retains a measured profile across timelines and source deletion, but isolates engine sessions', async () => {
  const bridge = { ...engine(), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const first = song(); const second = { ...song(), id: 't2', tracks: song().tracks.filter(track => track.id !== 'a1') };
  const props = { bridge, native: false, revision: 7, pool: null, media: [media], timelines: [first, second], onSelect: vi.fn(), timelineMonitor: controls(), noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const view = render(<FairlightPage {...props} session={1} selected={first} />); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound1"]')!, { clientX: 100, clientY: 100 });
  expect((screen.getByRole('menuitem', { name: 'Apply Level Profile' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Capture Level Profile' })); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.level_profile', { timeline: 't1', clip: 'sound1', revision: 7 }, { expectedSession: 1 });
  captured(bridge); await settle();
  view.rerender(<FairlightPage {...props} session={1} selected={second} revision={8} />); await settle();
  expect(screen.getByText(/Main \/ Interview/)).toBeTruthy();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound2"]')!, { clientX: 100, clientY: 100 });
  expect((screen.getByRole('menuitem', { name: 'Apply Level Profile' }) as HTMLButtonElement).disabled).not.toBe(true);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Apply Level Profile' })); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.level_match', { timeline: 't2', profile: capturedProfile.profile, clips: ['sound2'], apply: true, revision: 8 }, { expectedSession: 1 });
  view.rerender(<FairlightPage {...props} session={2} selected={second} />); await settle();
  bridge.tell({ job: 'audio-1', state: 'done', method: 'audio.level_profile', ...capturedProfile }, 1); await settle();
  expect(screen.queryByText(/Main \/ Interview/)).toBeNull();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound2"]')!, { clientX: 100, clientY: 100 });
  expect((screen.getByRole('menuitem', { name: 'Apply Level Profile' }) as HTMLButtonElement).disabled).toBe(true);
});

it.each(['failed', 'cancelled', 'silent', 'cancel-before-done'] as const)('keeps the old profile when another capture is %s', async failure => {
  const bridge = engine(); let sequence = 0;
  bridge.call.mockImplementation(async method => ({ ok: true, result: { job: method === 'audio.cancel' ? 'unused' : `audio-${++sequence}` } }));
  render(<Matcher bridge={bridge} session={1} timeline={song()} revision={7} clips={['sound1']} />);
  fireEvent.click(screen.getByRole('button', { name: 'Capture Level Profile' })); await settle(); captured(bridge); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Capture Level Profile' })); await settle();
  expect((screen.getByRole('button', { name: 'Apply Level Profile' }) as HTMLButtonElement).disabled).toBe(true);
  if (failure === 'cancel-before-done') fireEvent.click(screen.getByRole('button', { name: 'Cancel Level Matcher' }));
  if (failure === 'silent' || failure === 'cancel-before-done') bridge.tell({ job: 'audio-2', state: 'done', method: 'audio.level_profile', ...capturedProfile, profile: { version: 1, level_db: failure === 'silent' ? null : -10, basis: failure === 'silent' ? null : 'mean' } });
  else bridge.tell({ job: 'audio-2', state: failure, message: 'missing source' });
  await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Apply Level Profile' })); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.level_match', { timeline: 't1', profile: capturedProfile.profile, clips: ['sound1'], apply: true, revision: 7 }, { expectedSession: 1 });
});

it('does not accept a pending capture after switching timelines, and displays the short-clip mean', async () => {
  const bridge = { ...engine(), viewRect: vi.fn(async () => {}), viewShow: vi.fn(async () => {}) };
  const first = song(); const second = { ...song(), id: 't2' };
  const props = { bridge, native: false, session: 1, revision: 7, pool: null, media: [media], timelines: [first, second], onSelect: vi.fn(), timelineMonitor: controls(), noticeOf: () => null, onRetry: vi.fn(), onProblem: vi.fn() };
  const view = render(<FairlightPage {...props} selected={first} />); await settle();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound1"]')!, { clientX: 100, clientY: 100 });
  fireEvent.click(screen.getByRole('menuitem', { name: 'Capture Level Profile' })); await settle();
  view.rerender(<FairlightPage {...props} selected={second} />); await settle();
  expect(bridge.call).toHaveBeenCalledWith('audio.cancel', { job: 'audio-1' }, { expectedSession: 1 });
  captured(bridge); await settle(); expect(screen.queryByText(/Main \/ Interview/)).toBeNull();
  fireEvent.contextMenu(document.querySelector('[data-clip="sound1"]')!, { clientX: 100, clientY: 100 });
  fireEvent.click(screen.getByRole('menuitem', { name: 'Capture Level Profile' })); await settle();
  bridge.tell({ job: 'audio-1', state: 'done', method: 'audio.level_profile', ...capturedProfile, source: { timeline: 't2', clip: 'sound1', revision: 7 }, profile: { version: 1, level_db: -24.123, basis: 'mean' } }); await settle();
  expect(screen.getByText(/Mean level profile/)).toBeTruthy();
});
