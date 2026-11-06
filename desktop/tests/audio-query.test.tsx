// @vitest-environment jsdom
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { useProcessorCurve } from '../src/studio/audio/useProcessorCurve';
import { useAutomationParameters } from '../src/studio/audio/useAutomationParameters';
import { timeline } from './timeline-fixture';

afterEach(cleanup);
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
function model(): Timeline {
  return { ...timeline([]), tracks: ['a', 'b'].map(id => ({ id, name: id, kind: 'audio',
    enabled: true, locked: false, clips: [], transitions: [],
    strip: { eq: defaultEq(), dynamics: defaultDynamics(), fader_db: -3 } as never })) };
}
function setup() {
  let revision = 4, song = model();
  const call = vi.fn(async (method: string, params: any) => {
    const strip = (song.tracks.find(row => row.id === (params.target?.track ?? params.of?.track)) as any)?.strip;
    return { ok: true as const, result: method === 'automation.parameters'
      ? { revision, of: params.of, parameters: [{ path: 'fader_db', value: strip?.fader_db ?? 0, kind: 'continuous', scale: 'linear', min: -100, max: 10 }] }
      : method === 'audio.eq_response' ? { revision, frequencies: [100], gain_db: [strip?.eq.gain_db ?? 0] }
        : { revision, inputs_db: [-20], outputs_db: [-20] } };
  });
  const bridge = { call } as Pick<EngineBridge, 'call'>;
  function Curves({ id, target = 'a', session = 1 }: { id: string; target?: string; session?: number }) {
    const props = { bridge, session, revision, timeline: song, target: { track: target } };
    const eq = useProcessorCurve(props, 'eq'), dynamics = useProcessorCurve(props, 'dynamics');
    return <output data-testid={id}>{eq.curve?.y[0] ?? 'waiting'}/{dynamics.curve?.y[0] ?? 'waiting'}</output>;
  }
  function Parameters({ id, target = 'a', enabled = true, session = 1 }: { id: string; target?: string; enabled?: boolean; session?: number }) {
    const result = useAutomationParameters({ bridge, session, revision, timeline: song }, { track: target }, enabled);
    return <output data-testid={id}>{result.parameters[0]?.value ?? 'waiting'}</output>;
  }
  return { call, Curves, Parameters, update: (edit: (next: Timeline) => void) => { song = structuredClone(song); edit(song); revision++; } };
}

it('shares curves between a panel and thumbnail and ignores unrelated project edits', async () => {
  const s = setup(), { Curves } = s;
  const ui = () => <><Curves id="panel"/><Curves id="thumbnail"/><Curves id="other" target="b"/></>;
  const view = render(ui()); await settle(); expect(s.call).toHaveBeenCalledTimes(4);
  s.update(next => { next.name = 'Renamed'; next.tracks[0].name = 'Voice'; });
  view.rerender(ui()); expect(screen.getByTestId('panel').textContent).toBe('0/-20');
  await settle(); expect(s.call).toHaveBeenCalledTimes(4);
  s.update(next => { (next.tracks[0] as any).strip.eq.gain_db = 4; });
  view.rerender(ui()); await settle();
  expect(s.call).toHaveBeenCalledTimes(5);
  expect(screen.getByTestId('panel').textContent).toBe('4/-20');
  expect(screen.getByTestId('thumbnail').textContent).toBe('4/-20');
  expect(screen.getByTestId('other').textContent).toBe('0/-20');
});

it('shares owner metadata, keeps it on unrelated edits, and refreshes current values and structure', async () => {
  const s = setup(), { Parameters } = s;
  const ui = () => <><Parameters id="fx"/><Parameters id="pan"/><Parameters id="lane"/><Parameters id="other" target="b"/></>;
  const view = render(ui()); await settle(); expect(s.call).toHaveBeenCalledTimes(2);
  s.update(next => { next.tracks[0].name = 'Voice'; });
  view.rerender(ui()); expect(screen.getByTestId('lane').textContent).toBe('-3');
  await settle(); expect(s.call).toHaveBeenCalledTimes(2);
  s.update(next => { (next.tracks[0] as any).strip.fader_db = -9; });
  view.rerender(ui()); await settle(); expect(s.call).toHaveBeenCalledTimes(3);
  expect(screen.getByTestId('lane').textContent).toBe('-9');
  expect(screen.getByTestId('other').textContent).toBe('-3');
  s.update(next => { (next.tracks[0] as any).strip.sends = [{ bus: 'B2', level_db: -5 }]; });
  view.rerender(ui()); await settle(); expect(s.call).toHaveBeenCalledTimes(4);
  s.update(next => { (next.tracks[0] as any).strip.sends = []; });
  view.rerender(ui()); await settle(); expect(s.call).toHaveBeenCalledTimes(5);
});

it('refreshes curves for sample-rate changes and clears results across sessions', async () => {
  const s = setup(), { Curves } = s;
  const view = render(<Curves id="panel"/>); await settle();
  s.update(next => { next.sample_rate = 44100; });
  view.rerender(<Curves id="panel"/>); await settle(); expect(s.call).toHaveBeenCalledTimes(4);
  view.rerender(<Curves id="panel" session={2}/>);
  expect(screen.getByTestId('panel').textContent).toBe('waiting/waiting');
  await settle(); expect(s.call).toHaveBeenCalledTimes(6);
});

it('does not retain cached parameters after all consumers leave or expose disabled metadata', async () => {
  const s = setup(), { Parameters } = s;
  const view = render(<Parameters id="lane"/>); await settle();
  view.rerender(<Parameters id="lane" enabled={false}/>);
  expect(screen.getByTestId('lane').textContent).toBe('waiting'); await settle();
  view.rerender(<Parameters id="lane"/>); await settle(); expect(s.call).toHaveBeenCalledTimes(2);
  view.unmount(); await settle(); render(<Parameters id="new"/>); await settle();
  expect(s.call).toHaveBeenCalledTimes(3);
});

it('accepts a pending curve across an unrelated edit but discards it after settings change', async () => {
  const s = setup(), { Curves } = s;
  const pending: ((value: any) => void)[] = [];
  s.call.mockImplementation(() => new Promise(resolve => pending.push(resolve)));
  const view = render(<Curves id="panel"/>);
  s.update(next => { next.name = 'Renamed'; }); view.rerender(<Curves id="panel"/>);
  expect(s.call).toHaveBeenCalledTimes(2);
  await act(async () => { pending[0]({ ok: true, result: { revision: 4, frequencies: [100], gain_db: [2] } }); });
  expect(screen.getByTestId('panel').textContent).toBe('2/waiting');
  s.update(next => { (next.tracks[0] as any).strip.dynamics.makeup_db = 5; }); view.rerender(<Curves id="panel"/>);
  await act(async () => { pending[2]({ ok: true, result: { revision: 6, inputs_db: [-20], outputs_db: [-15] } }); });
  await act(async () => { pending[1]({ ok: true, result: { revision: 4, inputs_db: [-20], outputs_db: [-20] } }); });
  expect(screen.getByTestId('panel').textContent).toBe('2/-15');
});
