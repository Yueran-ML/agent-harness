// @vitest-environment jsdom
/**
 * The rest of T23 (interaction reference §11.4): Punch In and Punch Out of the selected tracks, the
 * red, blue and green states of controls, the strip's Auto, a level pasted to another level and
 * the Write on Preset or Clipboard Paste preference.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AutomationTouchProvider, useAutomationTouch } from '../src/studio/fairlight/AutomationTouch';
import { AutomationToolbar } from '../src/studio/fairlight/AutomationToolbar';
import { defaultRecord } from '../src/studio/fairlight/automation';
import { levelPathsOf, pastedLanes } from '../src/studio/fairlight/automationPaste';
import { defaultFairlightView } from '../src/studio/fairlight/viewState';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { EqPanel } from '../src/studio/audio/EqPanel';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { copyProcessor } from '../src/studio/audio/ProcessorToolbar';
import { changedFields } from '../src/studio/audio/useEffectEditor';
import { SendWindow } from '../src/studio/fairlight/SendWindow';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import type { AutomationParameter, EngineAnswer, Lanes, Timeline } from '../src/studio/engine';

const settle = () => act(async () => {
  for (let i = 0; i < 15; i++)
    await Promise.resolve();
});
beforeEach(() => {
  likeABrowser();
  vi.stubGlobal('PointerEvent', MouseEvent);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  localStorage.clear();
});

const point = (seconds: number, value: number) => ({ at: { ticks: Math.round(seconds * 48000), scale: 48000 }, value });

/** A timeline of audio tracks a1 and a2 (a1 with `lanes`), a bus B2 that a1 sends to, recorded as `record` says. */
function song(record: Partial<ReturnType<typeof defaultRecord>> = {}, lanes: Lanes = {}): Timeline {
  const track = (id: string, name: string) => ({
    id, name, kind: 'audio', enabled: true, locked: false, clips: [], transitions: [],
    strip: { fader_db: 0, eq: defaultEq(), dynamics: defaultDynamics(), sends: id === 'a1' ? [{ bus: 'B2', on: true, pre: false, level_db: -6, pan: {} }] : [] },
    ...(id === 'a1' ? { lanes } : {}),
  });
  return {
    ...timeline([]),
    tracks: [track('a1', 'Voice'), track('a2', 'Music')],
    buses: [{ id: 'B1', name: 'Bus 1', color: '#fff', format: 'stereo' }, { id: 'B2', name: 'Bus 2', color: '#fff', format: 'stereo' }],
    audio: { record: { ...defaultRecord(), ...record } },
  } as unknown as Timeline;
}

const parameter = (path: string, fields: Partial<AutomationParameter> = {}): AutomationParameter => ({
  path, kind: 'continuous', scale: 'linear', value: 0, min: -100, max: 10, ...fields,
});

function engine(parameters: AutomationParameter[] = [parameter('fader_db')]) {
  return vi.fn(async (method: string, params?: any): Promise<EngineAnswer> => ({
    ok: true,
    result: method === 'automation.parameters' ? { revision: 7, of: params.of, parameters }
      : method === 'automation.punch' || method === 'automation.punch_out' ? { lanes: 3 }
        : method === 'audio.eq_response' ? { revision: 7, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) }
          : method === 'audio.dynamics_curve' ? { revision: 7, inputs_db: params.inputs_db, outputs_db: params.inputs_db }
            : { revision: 8, done: [] },
  }));
}

/** What the Provider says of a1's fader and of a1. */
function Probe() {
  const touch = useAutomationTouch()!, target = { of: { track: 'a1' }, path: 'fader_db' };
  return <>
    <output aria-label="fader state">{touch.status(target) ?? 'none'}</output>
    <output aria-label="owner state">{touch.writing({ track: 'a1' }) ?? 'none'}</output>
    <output aria-label="fader value">{touch.reading(target, -3)}</output>
    <button onClick={() => touch.start(target, -10)}>touch</button>
  </>;
}

function toolbar(t: Timeline, selected: string[], { playing = true, call = engine(), marks = { in: null as number | null, out: null as number | null } } = {}) {
  const onFailure = vi.fn();
  const props = {
    bridge: { call }, session: 1, timeline: t, view: { ...defaultFairlightView(), automation: true, automationControls: true },
    update: vi.fn(), recording: true, selected, frame: 0, onEdit: vi.fn(), onFailure, onToggle: vi.fn(),
    marks: () => marks,
  };
  const ui = (playingNow: boolean) => <AutomationTouchProvider bridge={{ call }} session={1} timeline={t} enabled recording playing={playingNow}
    show={() => {}} onFailure={error => { throw error; }}>
    <AutomationToolbar {...props} />
    <Probe />
  </AutomationTouchProvider>;
  const rendered = render(ui(playing));
  return { call, onFailure, props, ...rendered, play: (on: boolean) => rendered.rerender(ui(on)) };
}

const calls = (call: ReturnType<typeof engine>, method: string) => call.mock.calls.filter(([name]) => name === method).map(([, params]) => params);

it('punches in the enabled settings of the selected tracks, red until the timeline stops', async () => {
  const s = toolbar(song({}, { fader_db: { points: [point(0, -12)] } }), ['track:a1']);
  expect(screen.getByLabelText('fader state').textContent).toBe('read');
  fireEvent.click(screen.getByText('Punch In'));
  await settle();
  expect(calls(s.call, 'automation.punch')).toEqual([{ timeline: 't1', of: [{ track: 'a1' }] }]);
  expect(screen.getByLabelText('fader state').textContent).toBe('write');
  expect(screen.getByLabelText('owner state').textContent).toBe('write');
  // Held at the value it had where it began to be written, as the engine writes it.
  expect(screen.getByLabelText('fader value').textContent).toBe('-3');
  // Write is entered again after each stop.
  s.play(false);
  await settle();
  expect(screen.getByLabelText('owner state').textContent).toBe('none');
});

it('asks for a selection without Preview, and punches in what Preview holds without one', async () => {
  const none = toolbar(song(), []);
  fireEvent.click(screen.getByText('Punch In'));
  await settle();
  expect(calls(none.call, 'automation.punch')).toEqual([]);
  expect(none.onFailure.mock.calls[0][0].message).toBe('Select the tracks to punch in first.');
  cleanup();
  const preview = toolbar(song({ preview: true }), ['track:a1']);
  fireEvent.click(screen.getByText('Punch In'));
  await settle();
  expect(calls(preview.call, 'automation.punch')).toEqual([{ timeline: 't1' }]);
});

it('punches out the selected track: what it holds is let go of here, and no release follows', async () => {
  const s = toolbar(song({ touch: 'latch' }), ['track:a1']);
  fireEvent.click(screen.getByText('touch'));
  await settle();
  expect(screen.getByLabelText('fader state').textContent).toBe('write');
  fireEvent.click(screen.getByText('Punch Out'));
  await settle();
  expect(calls(s.call, 'automation.punch_out')).toEqual([{ timeline: 't1', of: [{ track: 'a1' }] }]);
  expect(calls(s.call, 'automation.touch').map(message => message.touching)).toEqual([true]);
  expect(screen.getByLabelText('fader state').textContent).toBe('none');
  cleanup();
  // Under Preview, or with nothing selected, every track.
  const all = toolbar(song({ preview: true }), ['track:a1']);
  fireEvent.click(screen.getByText('Punch Out'));
  await settle();
  expect(calls(all.call, 'automation.punch_out')).toEqual([{ timeline: 't1' }]);
});

it('shows green only where a lane plays the control back', () => {
  const t = song({}, { fader_db: { points: [point(0, -12)] } });
  const ui = (enabled: boolean, timelineNow: Timeline) => <AutomationTouchProvider bridge={{ call: engine() }} session={1} timeline={timelineNow}
    enabled={enabled} recording={false} show={() => {}} onFailure={() => {}}><Probe /></AutomationTouchProvider>;
  const s = render(ui(true, t));
  expect(screen.getByLabelText('fader state').textContent).toBe('read');
  s.rerender(ui(false, t));
  expect(screen.getByLabelText('fader state').textContent).toBe('none');
  const off = structuredClone(t);
  (off.tracks[0] as { automation_arm?: unknown }).automation_arm = 'off';
  s.rerender(ui(true, off));
  expect(screen.getByLabelText('fader state').textContent).toBe('none');
});

it('pastes a level to a fader or a send level, from one track to the selected', () => {
  const clipboard = [
    { of: { track: 'a1' }, path: 'fader_db', points: [point(0, -6)] },
    { of: { track: 'a1' }, path: 'mute', value: 1 },
  ];
  expect(levelPathsOf(song(), ['track:a1'])).toEqual(['fader_db', 'send.B2.level_db']);
  expect(pastedLanes(clipboard, ['track:a2'], '')).toEqual(clipboard.map(lane => ({ ...lane, of: { track: 'a2' } })));
  expect(pastedLanes(clipboard, ['track:a1', 'track:a2'], 'send.B2.level_db')).toEqual([
    { of: { track: 'a1' }, path: 'send.B2.level_db', points: [point(0, -6)] },
    { of: { track: 'a2' }, path: 'send.B2.level_db', points: [point(0, -6)] },
  ]);
  expect(pastedLanes([clipboard[1]], [], 'fader_db')).toEqual([]);
});

it('sends the level pasted to another level with the edit', async () => {
  const s = toolbar(song(), ['track:a2'], {
    call: vi.fn(async (method: string): Promise<EngineAnswer> => ({
      ok: true, result: method === 'automation.copy' ? { lanes: [{ of: { track: 'a1' }, path: 'send.B2.level_db', value: -6 }] } : {},
    })),
    marks: { in: 0, out: 25 },
  });
  fireEvent.click(screen.getByText('Copy automation'));
  await settle();
  fireEvent.change(screen.getByLabelText('Paste level to'), { target: { value: 'fader_db' } });
  fireEvent.click(screen.getByText('Paste automation'));
  expect(s.props.onEdit).toHaveBeenCalledWith([{
    edit: 'automation_paste', timeline: 't1', at: { ticks: 0, scale: 48000 }, to: { ticks: 48000, scale: 48000 },
    lanes: [{ of: { track: 'a2' }, path: 'fader_db', value: -6 }],
  }]);
});

it('puts every enabled parameter of a strip into Preview with its Auto button', async () => {
  const t = song({ preview: true, categories: { ...defaultRecord().categories, eq: false } });
  const call = engine([parameter('fader_db'), parameter('mute', { kind: 'switch', min: 0, max: 1 }), parameter('eq.on', { kind: 'switch', min: 0, max: 1 })]);
  render(<AutomationTouchProvider bridge={{ call }} session={1} timeline={t} enabled recording playing={false} show={() => {}}
    onFailure={error => { throw error; }}>
    <Mixer bridge={{ call }} session={1} revision={7} timeline={t} frame={0} meters={null} selected={[]} onSelect={() => {}} />
  </AutomationTouchProvider>);
  fireEvent.click(screen.getByRole('button', { name: 'Auto Voice' }));
  await settle();
  const touches = calls(call, 'automation.touch');
  expect(touches.filter(message => message.touching).map(message => message.path)).toEqual(['fader_db', 'mute']);
  expect(touches.filter(message => !message.touching)).toHaveLength(2);
});

it('writes a preset or a paste as automation only under Write on Preset or Clipboard Paste', async () => {
  const t = song({ touch: 'latch' });
  const call = engine([parameter('eq.on', { kind: 'switch', min: 0, max: 1 }), parameter('eq.gain_db', { min: -20, max: 20 })]);
  const s = render(<AutomationTouchProvider bridge={{ call }} session={1} timeline={t} enabled recording playing
    show={() => {}} onFailure={error => { throw error; }}>
    <EqPanel bridge={{ call }} session={1} revision={7} timeline={t} target={{ track: 'a1' }} />
  </AutomationTouchProvider>);
  await settle();
  copyProcessor('eq', { ...defaultEq(), gain_db: 4 });
  fireEvent.click(screen.getByRole('button', { name: 'Paste' }));
  await settle();
  expect(calls(call, 'automation.touch')).toEqual([]);
  expect(calls(call, 'transaction.apply')).toHaveLength(1);
  localStorage.setItem('fairlight.write_on_paste', 'true');
  s.unmount();
  render(<AutomationTouchProvider bridge={{ call }} session={1} timeline={t} enabled recording playing
    show={() => {}} onFailure={error => { throw error; }}>
    <EqPanel bridge={{ call }} session={1} revision={7} timeline={t} target={{ track: 'a1' }} />
  </AutomationTouchProvider>);
  await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Paste' }));
  await settle();
  expect(calls(call, 'automation.touch').map(message => [message.path, message.value, message.touching]))
    .toEqual([['eq.gain_db', 4, true], ['eq.gain_db', 4, false]]);
  expect(calls(call, 'transaction.apply')).toHaveLength(1);
});

it('shows a send switch green where a lane plays it, and its automation on a right-click', async () => {
  const t = song({}, { 'send.B2.on': { kind: 'switch', points: [point(0, 1)] } });
  const show = vi.fn(), call = engine([parameter('send.B2.on', { kind: 'switch', min: 0, max: 1 })]);
  render(<AutomationTouchProvider bridge={{ call }} session={1} timeline={t} enabled recording={false} show={show}
    onFailure={error => { throw error; }}>
    <SendWindow context={{ bridge: { call }, session: 1, revision: 7, timeline: t, target: { track: 'a1' } }} stripKey="track:a1" onClose={() => {}} />
  </AutomationTouchProvider>);
  await settle();
  const on = screen.getByRole('checkbox', { name: 'On' }).closest('label')!;
  expect(on.getAttribute('data-automation-state')).toBe('read');
  fireEvent.contextMenu(on);
  expect(show).toHaveBeenCalledWith({ of: { track: 'a1' }, path: 'send.B2.on' });
});

it('writes only the settings that a pasted insert setting changes', () => {
  expect(changedFields({ kind: 'gain', gain_db: 0, invert: false, band: { q: 1, on: true } },
    { kind: 'gain', gain_db: 3, invert: false, band: { q: 2, on: false } })).toEqual([['gain_db', 3], ['band.q', 2], ['band.on', 0]]);
});

it('shows nothing in Write where the engine put nothing into Write', async () => {
  const call = vi.fn(async (method: string): Promise<EngineAnswer> => ({ ok: true, result: method === 'automation.punch' ? { lanes: 0 } : {} }));
  toolbar(song(), ['track:a1'], { call });
  fireEvent.click(screen.getByText('Punch In'));
  await settle();
  expect(screen.getByLabelText('owner state').textContent).toBe('none');
});
