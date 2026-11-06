// @vitest-environment jsdom
/**
 * The rest of T23 on the lanes (interaction reference §11.1, §11.2, §11.4): the nested parameter
 * menu and the Arm gestures of the track header, the keyboard clipboard shared by every lane, the
 * nudge unit, and the red curve of what is being written.
 */
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { AutomationTouchProvider, useAutomationTouch } from '../src/studio/fairlight/AutomationTouch';
import { AutomationLane, parameterMenu } from '../src/studio/fairlight/AutomationLanes';
import { defaultRecord, nudgeSamples } from '../src/studio/fairlight/automation';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { AudioPlayhead } from '../src/studio/timeline/AudioOverlay';
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
it('nests the parameters of a lane by category and group', () => {
  const parameters = ['fader_db', 'mute', 'pan.x', 'pan.y', 'eq.0.freq', 'eq.0.gain_db', 'eq.on', 'send.B2.level_db', 'send.B2.on'].map(path => parameter(path));
  const labels: Record<string, string> = {
    fader_db: 'Fader', mute: 'Mute', 'pan.x': 'Pan · Left/Right', 'pan.y': 'Pan · Front/Back',
    'eq.0.freq': 'Equalizer · Band 1 · Frequency', 'eq.0.gain_db': 'Equalizer · Band 1 · Gain', 'eq.on': 'Equalizer · Enable',
    'send.B2.level_db': 'Send · Bus 2 · Level', 'send.B2.on': 'Send · Bus 2 · Enable',
  };
  const chosen: string[] = [];
  const menu = parameterMenu(parameters, path => labels[path], category => category.toUpperCase(), 'pan.x', path => chosen.push(path), 'None');
  expect(menu.map(item => [item.label, item.items?.map(sub => sub.label) ?? null])).toEqual([
    ['None', null], ['FADER', null], ['MUTE', null], ['PAN', ['Left/Right', 'Front/Back']],
    ['EQ', ['Band 1', 'Enable']], ['SENDS', ['Bus 2']],
  ]);
  const pan = menu.find(item => item.label === 'PAN')!;
  expect(pan.items![0].checked).toBe(true);
  menu.find(item => item.label === 'EQ')!.items![0].items![1].run!();
  expect(chosen).toEqual(['eq.0.gain_db']);
});

function lanes(t: Timeline, owners: { of: { track: string }; path: string; lanes: Lanes }[], call = engine([
  parameter('fader_db'), parameter('send.B2.level_db'), parameter('pan.x', { min: -100, max: 100 }),
])) {
  const onEdit = vi.fn();
  render(<AudioPlayhead.Provider value={0}>
    {owners.map(({ of, path, lanes: drawn }) => <AutomationLane key={of.track + path} bridge={{ call } as never} session={1} revision={7}
      timeline={t} of={of} lanes={drawn} scale={4} from={0} to={100} height={128} snapping={false}
      marks={() => ({ in: null, out: null })} path={path} pencil={false} onPath={() => {}} onEdit={onEdit} name={of.track}
      selection={() => ['track:a2']} />)}
  </AudioPlayhead.Provider>);
  const ops = () => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params.ops[0]);
  return { call, ops, onEdit };
}

it('pastes what was copied on one lane on another, a level as it is and another setting at the same height', async () => {
  const t = song();
  const s = lanes(t, [
    { of: { track: 'a1' }, path: 'fader_db', lanes: { fader_db: { points: [point(0, -10), point(1, 0)] } } },
    { of: { track: 'a2' }, path: 'send.B2.level_db', lanes: {} },
    { of: { track: 'a2' }, path: 'pan.x', lanes: {} },
  ]);
  await settle();
  const fader = screen.getByLabelText('a1 Fader');
  fireEvent.keyDown(fader, { key: 'a', ctrlKey: true });
  fireEvent.keyDown(fader, { key: 'c', ctrlKey: true });
  fireEvent.keyDown(screen.getAllByLabelText(/^a2 /)[0], { key: 'v', ctrlKey: true });
  await settle();
  expect(s.ops()[0]).toMatchObject({ edit: 'lane_range', action: 'paste', path: 'send.B2.level_db', points: [point(0, -10), point(1, 0)] });
  fireEvent.keyDown(screen.getAllByLabelText(/^a2 /)[1], { key: 'v', ctrlKey: true });
  await settle();
  // −10 dB is 90/110 of the fader's height: the same height of the pan.
  expect(s.ops()[1].path).toBe('pan.x');
  expect(s.ops()[1].points[0].value).toBeCloseTo(-100 + 200 * 90 / 110);
});

it('nudges by the unit chosen in the Automation Controls', async () => {
  localStorage.setItem('fairlight.automation_nudge', 'millisecond');
  const t = song();
  expect(nudgeSamples('frame', t)).toBe(1920);
  expect(nudgeSamples('subframe', t)).toBe(192);
  // Two lanes, as an edit of a lane waits for the project to take the one before.
  const s = lanes(t, ['a1', 'a2'].map(track => ({
    of: { track }, path: 'fader_db', lanes: { fader_db: { points: [point(1, -10)] } },
  })));
  await settle();
  const one = screen.getByLabelText('a1 Fader'), other = screen.getByLabelText('a2 Fader');
  fireEvent.keyDown(one, { key: 'a', ctrlKey: true });
  fireEvent.keyDown(one, { key: '.', code: 'Period' });
  fireEvent.keyDown(other, { key: 'a', ctrlKey: true });
  fireEvent.keyDown(other, { key: '>', code: 'Period', shiftKey: true });
  await settle();
  expect(s.ops().map(op => op.by.ticks)).toEqual([48, 240]);
});

it('arms the selected tracks with Alt+click on a track header, and every track with Ctrl+Alt', async () => {
  const s = lanes(song(), [{ of: { track: 'a1' }, path: 'fader_db', lanes: {} }]);
  await settle();
  const arm = screen.getByRole('button', { name: 'Arm a1' });
  fireEvent.click(arm, { altKey: true });
  expect(s.onEdit.mock.calls[0][0].map((op: { track?: string; id?: string }) => op.track ?? op.id)).toEqual(['a1', 'a2']);
  fireEvent.click(arm, { altKey: true, ctrlKey: true });
  expect(s.onEdit.mock.calls[1][0].map((op: { track?: string; id?: string }) => op.track ?? op.id)).toEqual(['a1', 'a2', 'B1', 'B2']);
  fireEvent.contextMenu(arm);
  fireEvent.click(screen.getByRole('menuitem', { name: 'Safe' }));
  expect(s.onEdit.mock.calls[2][0]).toEqual([expect.objectContaining({ track: 'a1', patch: { automation_arm: 'safe' } })]);
});

it('draws the new curve red from where it began to be written to the playhead', async () => {
  const t = song({ touch: 'latch' });
  const call = engine(), bridge = { call };
  function Touch() {
    const touch = useAutomationTouch()!;
    return <button onClick={() => touch.start({ of: { track: 'a1' }, path: 'fader_db' }, -20)}>touch</button>;
  }
  const ui = (frame: number) => <AutomationTouchProvider bridge={bridge} session={1} timeline={t} enabled recording playing
    show={() => {}} onFailure={error => { throw error; }}>
    <Touch />
    <AudioPlayhead.Provider value={frame}>
      <AutomationLane bridge={bridge as never} session={1} revision={7} timeline={t} of={{ track: 'a1' }} lanes={{}} scale={4}
        from={0} to={100} height={128} snapping={false} marks={() => ({ in: null, out: null })} path="fader_db" pencil={false}
        onPath={() => {}} onEdit={() => {}} name="a1" />
    </AudioPlayhead.Provider>
  </AutomationTouchProvider>;
  const s = render(ui(0));
  await settle();
  expect(document.querySelector('.automation-trail')).toBeNull();
  fireEvent.click(screen.getByText('touch'));
  await settle();
  s.rerender(ui(10));
  s.rerender(ui(20));
  const trail = document.querySelector('.automation-trail')!;
  expect(trail.getAttribute('stroke')).toBe('#f66');
  expect(trail.getAttribute('points')!.split(' ').length).toBeGreaterThanOrEqual(3);
});

