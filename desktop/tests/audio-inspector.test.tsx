// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Inspector } from '../src/studio/inspector/Inspector';
import { Inspector as FairlightInspector } from '../src/studio/fairlight/Inspector';
import { SpeedDialog } from '../src/studio/inspector/SpeedDialog';
import { applyOp, type AudioParam, type Clip, type ClipTrack, type Op, type Project } from '../src/studio/engine';
import { Words } from '../src/app/locale';
import { zhCN } from '../src/messages/zh-CN';
import { ja } from '../src/messages/ja';
import { clip, media, timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
const curve = (values = [-12, 0]): AudioParam => ({ keys: values.map((value, i) => ({ at: { ticks: i * 48000, scale: 48000 }, value, interp: 'linear' })) });
const box = (name: string) => document.querySelector<HTMLInputElement>(`[data-field="${name}"] input.inspector-number`)!;
const label = (name: string) => document.querySelector<HTMLElement>(`[data-field="${name}"] label`)!;
const type = (name: string, value: string) => { fireEvent.change(box(name), { target: { value } }); fireEvent.keyDown(box(name), { key: 'Enter' }); };
const pointer = (x: number, extra = {}) => ({ button: 0, pointerId: 1, clientX: x, ...extra });
function mount(clips: Clip[] = [clip('a', 0, 25, { gain_db: curve(), pan: curve([-.5, .5]) })], fairlight = false) {
  let revision = 17;
  const call = vi.fn(async () => ({ ok: true as const, result: { revision: ++revision, done: [] } }));
  const t = { ...timeline(), rate: { num: 25, den: 1 }, tracks: [{ id: 'audio', kind: 'audio', name: 'Audio', enabled: true, locked: false, clips, transitions: [] } as ClipTrack] };
  const props = { bridge: { call, onEvent: vi.fn(() => () => {}) }, session: 1, revision: 17, timeline: t, clips: clips.map(c => c.id), media: [media], transition: null, frame: 12.5, onSeek: vi.fn(), onSpeed: vi.fn() };
  const element = (extra = {}) => fairlight ? <FairlightInspector {...props} track={null} onEdit={() => {}} {...extra} /> : <Inspector {...props} {...extra} />;
  const view = render(element());
  return { ...view, props, call, update: (extra: object) => view.rerender(element(extra)), requests: () => call.mock.calls.map(args => (args as unknown[])[1] as any) };
}

it('shows both interpolated values at the actual displayed frame and types a point without erasing animation', async () => {
  const v = mount();
  expect(box('volume').value).toBe('-6.0'); expect(box('pan').value).toBe('0');
  type('volume', '-3'); await settle();
  expect(v.requests()[0].ops).toEqual([{ edit: 'clip_gain_keys', timeline: 't1', clip: 'a', param: 'gain_db', action: 'add', at: { ticks: 24000, scale: 48000 }, value: -3 }]);
});

it('has parameter-local key controls, interpolation, bracket navigation and a reset to static zero', async () => {
  const v = mount();
  fireEvent.click(screen.getByRole('button', { name: 'Volume: Add keyframe' })); await settle();
  expect(v.requests()[0].ops[0]).toMatchObject({ param: 'gain_db', action: 'add', value: -6 });
  const next = screen.getByRole('button', { name: 'Pan: Next keyframe' });
  fireEvent.keyDown(next, { key: ']' }); expect(v.props.onSeek).toHaveBeenCalledWith(25, false);
  v.update({ frame: 0, revision: 18 });
  expect(screen.getByRole('button', { name: 'Volume: Remove keyframe' }).getAttribute('aria-pressed')).toBe('true');
  fireEvent.change(screen.getByLabelText('Volume: Keyframe interpolation'), { target: { value: 'ease_out' } }); await settle();
  expect(v.requests().at(-1).ops[0]).toMatchObject({ action: 'set_interp', interp: 'ease_out' });
  v.update({ revision: 19 }); fireEvent.click(screen.getByRole('button', { name: 'Volume: Reset' })); await settle();
  expect(v.requests().at(-1).ops).toEqual([{ op: 'set_clip', timeline: 't1', id: 'a', gain_db: 0 }]);
});

it('previews relative multi-gain from the captured curves, continues past one member limit and cancels in one gesture', async () => {
  const v = mount([clip('a', 0, 25, { gain_db: 23 }), clip('b', 0, 25, { gain_db: curve() })]);
  fireEvent.pointerDown(label('volume'), pointer(0)); fireEvent.pointerMove(label('volume'), pointer(30)); await settle();
  expect(v.requests()[0].ops.at(-1)).toMatchObject({ edit: 'nudge_clip_gain', clips: ['a', 'b'], delta_db: 3, linked: false });
  expect(box('volume').value).toBe('');
  expect(box('volume').placeholder).toBe('—');
  fireEvent.pointerMove(label('volume'), pointer(60)); await settle();
  expect(v.requests().at(-1).ops.at(-1).delta_db).toBe(6);
  fireEvent.keyDown(window, { key: 'Escape' }); await settle();
  expect(v.requests().at(-1).ops).toEqual([{ op: 'set_clip', timeline: 't1', id: 'a', gain_db: 23 }, { op: 'set_clip', timeline: 't1', id: 'b', gain_db: curve() }]);
  expect(new Set(v.requests().map(r => r.gesture.id)).size).toBe(1);
  expect(v.requests().at(-1).gesture.end).toBe(true);
});

it.each([false, true])('uses the page fine adjustment convention (Fairlight=%s) and anchors animated points during playback', async fairlight => {
  const v = mount(undefined, fairlight);
  fireEvent.pointerDown(label('volume'), pointer(0));
  fireEvent.pointerMove(label('volume'), pointer(10, { shiftKey: true })); await settle();
  expect(v.requests()[0].ops.at(-1).value).toBe(fairlight ? -5.9 : 4);
  v.update({ frame: 20, revision: 18 });
  fireEvent.pointerMove(label('volume'), pointer(20, { shiftKey: true })); await settle();
  expect(v.requests().at(-1).ops.at(-1).at).toEqual({ ticks: 24000, scale: 48000 });
  fireEvent.pointerUp(label('volume'), pointer(20)); await settle();
  expect(v.requests().at(-1).gesture.end).toBe(true);
});

it('offers integral semitones and cents, preserving the other pitch field for each selected clip', async () => {
  const v = mount([clip('a', 0, 25, { pitch: { semitones: 3, cents: 20 } }), clip('b', 0, 25, { pitch: { semitones: -2, cents: -30 } })]);
  type('semitones', '5.7'); await settle();
  expect(v.requests()[0].ops.map((op: any) => op.pitch)).toEqual([{ semitones: 6, cents: 20 }, { semitones: 6, cents: -30 }]);
});

it('sends pitch correction alone without set_speed, ripple or reverse changes', async () => {
  const v = mount(); v.unmount();
  render(<SpeedDialog {...v.props} onClose={() => {}} />);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Pitch Correction' }));
  fireEvent.click(screen.getByRole('button', { name: 'Change' })); await settle();
  expect(v.requests()[0].ops).toEqual([{ op: 'set_clip', timeline: 't1', id: 'a', pitch_correction: true }]);
});

it.each([false, true])('bounds source-sample points on a 2x clip at 44.1 kHz (reverse=%s)', async reverse => {
  const v = mount([clip('a', 10, 61, { speed: { num: 2, den: 1 }, reverse, gain_db: curve() })]);
  const now = { ...v.props.timeline, rate: { num: 24, den: 1 } };
  v.update({ timeline: now, frame: reverse ? -50 : 200, media: [{ ...media, facts: { ...media.facts, audio: { ...media.facts.audio!, rate: 44100 } } }] });
  type('volume', '-7'); await settle();
  // Source span is floored at source.in.scale=25, then quantized inward to 44.1 kHz.
  expect(v.requests()[0].ops[0].at).toEqual({ ticks: 224028, scale: 44100 });
});

it('does not mark an out-of-clip clamped endpoint as an actual keyframe hit', () => {
  const v = mount(); v.update({ frame: -10 });
  expect(screen.getByRole('button', { name: 'Volume: Add keyframe' }).getAttribute('data-hit')).toBeNull();
});

it('keeps pan key times/interpolation during relative drag, clamps each independently and reads the model back', async () => {
  const pan: AudioParam = { keys: [{ at: { ticks: 0, scale: 25 }, value: -.7, interp: 'ease_out' }, { at: { ticks: 25, scale: 25 }, value: .9, interp: 'ease_in' }] };
  const v = mount([clip('a', 0, 25, { pan }), clip('b', 0, 25, { pan: .8 })]);
  let project: Project = { schema: 4, next_id: 3, bins: [], media: [media], groups: [], timelines: [v.props.timeline] };
  fireEvent.pointerDown(label('pan'), pointer(0)); fireEvent.pointerMove(label('pan'), pointer(80)); await settle();
  const ops = v.requests()[0].ops as Op[];
  for (const op of ops) project = applyOp(project, op);
  const stored = (project.timelines[0].tracks[0] as ClipTrack).clips;
  expect(stored[0].pan).toEqual({ keys: [{ at: { ticks: 0, scale: 25 }, value: -.7 + .4, interp: 'ease_out' }, { at: { ticks: 25, scale: 25 }, value: 1, interp: 'ease_in' }] });
  expect(stored[1].pan).toBe(1);
  v.update({ timeline: project.timelines[0], revision: 18 });
  expect(document.querySelector('[data-field="pan"]')?.getAttribute('data-mixed')).toBe('true');
  fireEvent.pointerUp(label('pan'), pointer(80)); await settle();
  expect(v.requests().at(-1).ops).toEqual(ops);
});

it('types a uniform absolute value into mixed static/animated clips, preserving existing keys on readback', async () => {
  const v = mount([clip('a', 0, 25, { gain_db: curve() }), clip('b', 0, 25, { gain_db: 7 })]);
  type('volume', '-3'); await settle();
  expect(v.requests()[0].ops).toEqual([
    { edit: 'clip_gain_keys', timeline: 't1', clip: 'a', param: 'gain_db', action: 'add', at: { ticks: 24000, scale: 48000 }, value: -3 },
    { op: 'set_clip', timeline: 't1', id: 'b', gain_db: -3 },
  ]);
  // The server expands the edit to this set_clip; exercise the real project copy's readback.
  const old = curve() as Exclude<AudioParam, number>;
  const accepted: Op = { op: 'set_clip', timeline: 't1', id: 'a', gain_db: { keys: [old.keys[0], { at: { ticks: 24000, scale: 48000 }, value: -3, interp: 'linear' }, old.keys[1]] } };
  let project: Project = { schema: 4, next_id: 3, bins: [], media: [media], groups: [], timelines: [v.props.timeline] };
  project = applyOp(applyOp(project, accepted), v.requests()[0].ops[1]);
  v.update({ timeline: project.timelines[0], revision: 18 });
  expect(box('volume').value).toBe('-3.0');
  const reread = JSON.parse(JSON.stringify(project)) as Project;
  expect((reread.timelines[0].tracks[0] as ClipTrack).clips[0].gain_db).toEqual(accepted.gain_db);
});

it.each(['pointer', 'blur', 'selection', 'revision', 'session'])('terminates a numeric gesture on %s without editing a replacement selection/session', async cancel => {
  const v = mount();
  fireEvent.pointerDown(label('volume'), pointer(0)); fireEvent.pointerMove(label('volume'), pointer(20)); await settle();
  if (cancel === 'pointer') fireEvent.pointerCancel(label('volume'), pointer(20));
  if (cancel === 'blur') fireEvent.blur(window);
  if (cancel === 'selection') v.update({ clips: [] });
  if (cancel === 'revision') v.update({ revision: 25 });
  if (cancel === 'session') v.update({ session: 2 });
  await settle();
  if (cancel === 'revision') expect(v.requests()).toHaveLength(1);
  else expect(v.requests().at(-1)).toMatchObject({ revision: 17, gesture: { end: true }, ops: [{ op: 'set_clip', gain_db: curve() }] });
  const calls = v.requests().length;
  if (label('volume')) { fireEvent.pointerMove(label('volume'), pointer(40)); fireEvent.pointerUp(label('volume'), pointer(40)); }
  await settle(); expect(v.requests()).toHaveLength(calls);
});

it('cancels a live slider on window blur and ignores its late native change event', async () => {
  const v = mount(); const slider = document.querySelector<HTMLInputElement>('[data-field="volume"] input[type="range"]')!;
  fireEvent.pointerDown(slider); fireEvent.input(slider, { target: { value: '-4' } }); await settle();
  fireEvent.blur(window); await settle(); const count = v.requests().length;
  fireEvent.change(slider, { target: { value: '-4' } }); await settle();
  expect(v.requests()).toHaveLength(count);
});

it('anchors the frame at pointer down, even when the first motion arrives after playback advances', async () => {
  const v = mount(); fireEvent.pointerDown(label('volume'), pointer(0)); v.update({ frame: 20 });
  fireEvent.pointerMove(label('volume'), pointer(10)); await settle();
  expect(v.requests()[0].ops[0].at).toEqual({ ticks: 24000, scale: 48000 });
});

it('uses Alt fine pan values in Fairlight without rounding them to whole percent', async () => {
  const v = mount([clip('a')], true);
  fireEvent.pointerDown(label('pan'), pointer(0)); fireEvent.pointerMove(label('pan'), pointer(10, { altKey: true })); await settle();
  expect(v.requests()[0].ops[0].pan).toBe(.005);
});

it('keeps same-frame subkeys navigable in timeline order for reverse clips', () => {
  const v = mount([clip('a', 0, 25, { reverse: true, gain_db: { keys: [.5, .51].map(time => ({ at: { ticks: time * 48000, scale: 48000 }, value: 0, interp: 'linear' })) } })]);
  v.update({ frame: 0 }); const button = screen.getByRole('button', { name: 'Volume: Next keyframe' });
  fireEvent.keyDown(button, { key: ']' }); v.update({ frame: 12 }); fireEvent.keyDown(button, { key: ']' });
  expect(v.props.onSeek.mock.calls).toEqual([[12, false], [13, false]]);
});

it('isolates parameter keys from clip deletion and clipboard commands, and ignores IME Enter', async () => {
  const v = mount(); const outer = vi.fn(); window.addEventListener('keydown', outer);
  const button = screen.getByRole('button', { name: 'Volume: Add keyframe' });
  for (const key of ['Delete', 'Backspace', 'x', 'c', 'v']) fireEvent.keyDown(button, { key, ctrlKey: true });
  expect(outer).not.toHaveBeenCalled(); window.removeEventListener('keydown', outer);
  fireEvent.change(box('volume'), { target: { value: '-5' } }); fireEvent.keyDown(box('volume'), { key: 'Enter', isComposing: true }); await settle();
  expect(v.requests()).toHaveLength(0);
});

it('disables all changes on locked tracks and has no audio targets for a video-only selection', async () => {
  const v = mount();
  v.update({ timeline: { ...v.props.timeline, tracks: [{ ...v.props.timeline.tracks[0], locked: true }] } });
  expect(box('volume').disabled).toBe(true); expect(box('cents').disabled).toBe(true);
  expect(screen.getByRole('button', { name: 'Volume: Add keyframe' }).hasAttribute('disabled')).toBe(true);
  v.update({ timeline: timeline([clip('video')]), clips: ['video'] });
  expect(box('volume')).toBeNull(); expect(box('semitones')).toBeNull(); await settle(); expect(v.requests()).toHaveLength(0);
});

it('drags pitch relatively, clamps per member, preserves cents, then restores on cancel', async () => {
  const v = mount([clip('a', 0, 25, { pitch: { semitones: 11, cents: 20 } }), clip('b', 0, 25, { pitch: { semitones: -2, cents: -30 } })]);
  fireEvent.pointerDown(label('semitones'), pointer(0)); fireEvent.pointerMove(label('semitones'), pointer(30)); await settle();
  expect(v.requests()[0].ops.map((op: any) => op.pitch)).toEqual([{ semitones: 12, cents: 20 }, { semitones: 1, cents: -30 }]);
  fireEvent.keyDown(window, { key: 'Escape' }); await settle();
  expect(v.requests().at(-1).ops.map((op: any) => op.pitch)).toEqual([{ semitones: 11, cents: 20 }, { semitones: -2, cents: -30 }]);
});

it('combines speed, reverse and pitch correction in one transaction and warns beyond 0.5–2x', async () => {
  const v = mount(); v.unmount(); render(<SpeedDialog {...v.props} onClose={() => {}} />);
  fireEvent.change(document.querySelector('#speed-percent')!, { target: { value: '250' } });
  fireEvent.click(screen.getByRole('checkbox', { name: 'Reverse speed' }));
  fireEvent.click(screen.getByRole('checkbox', { name: 'Pitch Correction' }));
  expect(screen.getByRole('status').textContent).toContain('50% to 200%');
  fireEvent.click(screen.getByRole('button', { name: 'Change' })); await settle();
  expect(v.requests()).toHaveLength(1);
  expect(v.requests()[0].ops).toEqual([{ edit: 'set_speed', timeline: 't1', clip: 'a', speed: { num: 5, den: 2 }, reverse: true, ripple: true, linked: false }, { op: 'set_clip', timeline: 't1', id: 'a', pitch_correction: true }]);
});

it('allows correction-only changes on mixed speeds without filling the speed field', async () => {
  const v = mount([clip('a', 0, 25, { speed: { num: 3, den: 1 } }), clip('b')]); v.unmount();
  render(<SpeedDialog {...v.props} onClose={() => {}} />);
  expect((document.querySelector('#speed-percent') as HTMLInputElement).value).toBe('');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Pitch Correction' })); fireEvent.click(screen.getByRole('button', { name: 'Change' })); await settle();
  expect(v.requests()[0].ops).toEqual(['a', 'b'].map(id => ({ op: 'set_clip', timeline: 't1', id, pitch_correction: true })));
});

it.each([[zhCN, '半音', '音调校正'], [ja, '半音', 'ピッチ補正']] as const)('localizes new audio fields and correction controls', (words, pitch, correction) => {
  const v = mount(); v.unmount();
  const view = render(<Words.Provider value={words}><Inspector {...v.props} /></Words.Provider>);
  expect(screen.getByLabelText(pitch)).toBeTruthy(); view.unmount();
  render(<Words.Provider value={words}><SpeedDialog {...v.props} onClose={() => {}} /></Words.Provider>);
  expect(screen.getByRole('checkbox', { name: correction })).toBeTruthy();
});

it('uses Fairlight fine modifiers in the existing fade fields while Edit keeps Shift tenfold', async () => {
  for (const fairlight of [true, false]) {
    const v = mount([clip('a', 0, 100)], fairlight);
    fireEvent.pointerDown(label('fade_in'), pointer(0)); fireEvent.pointerMove(label('fade_in'), pointer(20, { shiftKey: true }));
    fireEvent.pointerUp(label('fade_in'), pointer(20)); await settle();
    expect(v.requests()[0].ops[0].fade_in).toBe(fairlight ? 1 : 100); v.unmount();
  }
});

it('preserves reverse when only speed is edited, including a newer model snapshot', async () => {
  const v = mount(); v.unmount();
  const view = render(<SpeedDialog {...v.props} onClose={() => {}} />);
  fireEvent.change(document.querySelector('#speed-percent')!, { target: { value: '50' } });
  const changed = { ...v.props.timeline, tracks: [{ ...v.props.timeline.tracks[0], clips: [clip('a', 0, 25, { reverse: true })] }] };
  view.rerender(<SpeedDialog {...v.props} timeline={changed} revision={18} onClose={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Change' })); await settle();
  expect(v.requests()[0].ops[0]).not.toHaveProperty('reverse');
});

it('does not interleave key edits with an active held numeric gesture', async () => {
  const v = mount(); fireEvent.pointerDown(label('volume'), pointer(0));
  fireEvent.click(screen.getByRole('button', { name: 'Volume: Add keyframe' })); await settle();
  expect(v.requests()).toHaveLength(0);
});

it('keeps fractional cents and the existing semitone setting', async () => {
  const v = mount([clip('a', 0, 25, { pitch: { semitones: -5, cents: 0 } })]);
  type('cents', '-20.5'); await settle();
  expect(v.requests()[0].ops[0].pitch).toEqual({ semitones: -5, cents: -20.5 });
});
