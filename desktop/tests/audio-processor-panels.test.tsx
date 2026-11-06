// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { EqPanel } from '../src/studio/audio/EqPanel';
import { DynamicsPanel } from '../src/studio/audio/DynamicsPanel';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import { useProcessorCurve } from '../src/studio/audio/useProcessorCurve';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
function setup(module: 'eq' | 'dynamics') {
  let model: Timeline = { ...timeline([]), tracks: [{ id: 'a', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { eq: defaultEq(), dynamics: defaultDynamics() } as never }] };
  let rev = 4;
  const requests: { method: string; params: any }[] = [];
  function Host() {
    const [state, set] = useState({ timeline: model, revision: rev });
    const bridge = { call: async (method: string, params: any) => {
      requests.push({ method, params });
      if (method === 'audio.eq_response') return { ok: true, result: { revision: rev, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) } };
      if (method === 'audio.dynamics_curve') return { ok: true, result: { revision: rev, inputs_db: params.inputs_db, outputs_db: params.inputs_db } };
      if (method === 'transaction.apply') {
        // Like project.changed/applyOp, publish a new snapshot; never mutate a gesture's base.
        model = structuredClone(model);
        const strip = (model.tracks[0] as any).strip;
        const merge = (destination: any, patch: any) => { for (const [key, value] of Object.entries(patch)) {
          if (value && typeof value === 'object' && !Array.isArray(value)) merge(destination[key] ??= {}, value); else destination[key] = value;
        } };
        for (const op of params.ops) merge(strip, op.patch.strip);
        model = { ...model, tracks: [...model.tracks] }; rev++; set({ timeline: model, revision: rev });
        return { ok: true, result: { revision: rev, done: [] } };
      }
      throw new Error(method);
    } } as Pick<EngineBridge, 'call'>;
    const props = { bridge, session: 1, ...state, target: { track: 'a' } };
    return module === 'eq' ? <EqPanel {...props} /> : <DynamicsPanel {...props} />;
  }
  return { ...render(<Host />), requests, model: () => (model.tracks[0] as any).strip, writes: () => requests.filter(r => r.method === 'transaction.apply').map(r => r.params) };
}
it('requests the real EQ response, edits frequency/gain in one cancellable gesture and wheels Q', async () => {
  const s = setup('eq'); await settle();
  expect(s.requests[0]).toMatchObject({ method: 'audio.eq_response', params: { target: { track: 'a' }, timeline: 't1' } });
  const handle = screen.getByRole('slider', { name: 'EQ band 2' });
  expect(handle.getAttribute('r')).toBe('8'); expect(screen.queryByRole('slider', { name: 'EQ band 1' })).toBeNull();
  fireEvent.pointerDown(handle, { button: 0, clientX: 100, clientY: 80 });
  fireEvent.pointerMove(handle, { clientX: 130, clientY: 60 }); await settle();
  expect(s.model().eq.bands[1].freq).toBeGreaterThan(100); expect(s.model().eq.bands[1].gain_db).toBeGreaterThan(0);
  fireEvent.keyDown(handle, { key: 'Escape' }); await settle();
  expect(s.model().eq.bands[1]).toMatchObject({ freq: 100, gain_db: 0 });
  expect(new Set(s.writes().map(p => p.gesture.id)).size).toBe(1); expect(s.writes().at(-1).gesture.end).toBe(true);
  fireEvent.wheel(handle, { deltaY: -1 }); await settle(); expect(s.model().eq.bands[1].q).toBeCloseTo(1.1);
});
it('disables inapplicable pass/shelf fields and uses actual EQ range patches', async () => {
  const s = setup('eq'); await settle();
  expect((screen.getByRole('spinbutton', { name: 'Band 1 Gain value' }) as HTMLInputElement).disabled).toBe(true);
  expect((screen.getByRole('spinbutton', { name: 'Band 1 Q value' }) as HTMLInputElement).disabled).toBe(true);
  fireEvent.change(screen.getByRole('combobox', { name: 'Band 2 Range' }), { target: { value: 'medium_high' } }); await settle();
  expect(s.model().eq.bands[1]).toMatchObject({ range: 'medium_high', freq: 500 });
});
it('groups successive Q wheel notches into one cancellable gesture', async () => {
  vi.useFakeTimers(); const s = setup('eq'); await settle(); const handle = screen.getByRole('slider', { name: 'EQ band 2' });
  fireEvent.wheel(handle, { deltaY: -1 }); await settle(); fireEvent.wheel(handle, { deltaY: -1 }); await settle();
  expect(s.model().eq.bands[1].q).toBeCloseTo(1.21);
  expect(new Set(s.writes().map(p => p.gesture.id)).size).toBe(1);
  fireEvent.keyDown(handle, { key: 'Escape' }); await settle(); expect(s.model().eq.bands[1].q).toBe(1);
  await act(async () => vi.advanceTimersByTimeAsync(450)); expect(s.model().eq.bands[1].q).toBe(1);
});
it('patches Dynamics modules, precise values, sidechain and reset through actual requests', async () => {
  const s = setup('dynamics'); await settle();
  expect(s.requests.some(r => r.method === 'audio.dynamics_curve')).toBe(true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Enable' })); await settle();
  expect(s.model().dynamics.on).toBe(true); expect((screen.getByRole('checkbox', { name: 'Enable' }) as HTMLInputElement).checked).toBe(true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Gate' })); await settle();
  expect(s.model().dynamics.expander).toMatchObject({ on: true, mode: 'gate' });
  expect((screen.getByRole('spinbutton', { name: 'Expander Ratio value' }) as HTMLInputElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Expander' })); await settle(); expect(s.model().dynamics.expander.mode).toBe('expander');
  const ratio = screen.getByRole('spinbutton', { name: 'Compressor Ratio value' });
  fireEvent.change(ratio, { target: { value: '3.7' } }); fireEvent.keyDown(ratio, { key: 'Enter' }); await settle(); expect(s.model().dynamics.compressor.ratio).toBe(3.7);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Sidechain On' })); await settle(); expect(s.model().dynamics.compressor.sidechain.on).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Reset' })); await settle(); expect(s.model().dynamics).toEqual(defaultDynamics());
  expect(s.writes().every(p => p.ops.every((op: any) => Object.keys(op.patch.strip).join() === 'dynamics'))).toBe(true);
});
it('discards a curve response from an earlier revision or session', async () => {
  const pending: ((value: any) => void)[] = [];
  const call = vi.fn(() => new Promise<any>(resolve => pending.push(resolve)));
  const props = { bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 4, timeline: timeline([]), target: { track: 'a' } };
  function Curve({ revision, session }: { revision: number; session: number }) {
    const { curve } = useProcessorCurve({ ...props, revision, session }, 'eq');
    return <output>{curve?.y.join(',') ?? 'waiting'}</output>;
  }
  const view = render(<Curve revision={4} session={1} />);
  view.rerender(<Curve revision={5} session={2} />);
  pending[1]({ ok: true, result: { revision: 5, sample_rate: 48000, frequencies: [100], gain_db: [3] } }); await settle(); expect(screen.getByText('3')).toBeTruthy();
  pending[0]({ ok: true, result: { revision: 4, sample_rate: 48000, frequencies: [100], gain_db: [9] } }); await settle(); expect(screen.getByText('3')).toBeTruthy();
});
it('keeps locked/active-lane modules read-only and batch module actions skip them', async () => {
  const t = timeline([]);
  t.tracks = [0, 1, 2, 3].map(i => ({ id: `a${i}`, name: `Track ${i}`, kind: 'audio' as const, enabled: true, locked: i === 2, clips: [], transitions: [],
    strip: { eq: defaultEq() } as never, ...(i === 3 ? { lanes: { 'eq.1.gain_db': { points: [{ at: { ticks: 0, scale: 48000 }, value: 7 }] } } } : {}) }));
  const call = vi.fn(async (method: string, params: any) => ({ ok: true as const, result: method === 'audio.eq_response'
    ? { revision: 4, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) } : { revision: 5, done: [] } }));
  const props = { bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 4, timeline: t, target: { track: 'a0' }, selected: ['track:a0', 'track:a1', 'track:a2', 'track:a3'] };
  const view = render(<EqPanel {...props} />); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'Reset' }), { altKey: true }); await settle();
  expect(call.mock.calls.filter(([m]) => m === 'transaction.apply')[0][1].ops.map((op: any) => op.track)).toEqual(['a0', 'a1']);
  view.rerender(<EqPanel {...props} revision={5} />); fireEvent.click(screen.getByRole('checkbox', { name: 'Enable' }), { altKey: true }); await settle();
  expect(call.mock.calls.filter(([m]) => m === 'transaction.apply')[1][1].ops.map((op: any) => op.patch.strip)).toEqual([{ eq: { on: true } }, { eq: { on: true } }]);
  view.rerender(<EqPanel {...props} revision={5} target={{ track: 'a3' }} />); await settle();
  const gain = screen.getByRole('spinbutton', { name: 'Band 2 Gain value' }) as HTMLInputElement;
  expect(gain.disabled).toBe(true); expect(gain.value).toBe('7');
  fireEvent.click(screen.getByRole('button', { name: 'Reset' })); await settle(); expect(call.mock.calls.filter(([m]) => m === 'transaction.apply')).toHaveLength(2);
});
it('abandons an EQ preview after an external revision without restoring over the newer state', async () => {
  const t = timeline([]); t.tracks = [{ id: 'a', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { eq: defaultEq() } as never }];
  const call = vi.fn(async (method: string, params: any) => ({ ok: true as const, result: method === 'audio.eq_response'
    ? { revision: 4, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) } : { revision: 5, done: [] } }));
  const props = { bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 4, timeline: t, target: { track: 'a' } };
  const view = render(<EqPanel {...props} />); await settle(); const knob = screen.getByRole('slider', { name: 'Band 2 Gain' });
  fireEvent.pointerDown(knob, { button: 0, clientY: 100 }); fireEvent.pointerMove(knob, { clientY: 80 }); await settle();
  view.rerender(<EqPanel {...props} revision={6} />); fireEvent.pointerCancel(knob); await settle();
  expect(call.mock.calls.filter(([m]) => m === 'transaction.apply')).toHaveLength(1);
});
it.each(['knob', 'handle', 'number'] as const)('focuses the %s on pointerdown so real focus-directed Escape restores its preview', async kind => {
  const s = setup('eq'); await settle(); render(<button>Previous focus</button>); screen.getByRole('button', { name: 'Previous focus' }).focus();
  const target = kind === 'handle' ? screen.getByRole('slider', { name: 'EQ band 2' }) : kind === 'number'
    ? screen.getByRole('spinbutton', { name: 'Band 2 Gain value' }) : screen.getByRole('slider', { name: 'Band 2 Gain' });
  fireEvent.pointerDown(target, { button: 0, clientX: 100, clientY: 100 });
  expect(document.activeElement).toBe(target);
  fireEvent.pointerMove(target, { clientX: 120, clientY: 80 }); await settle(); expect(s.model().eq.bands[1].gain_db).toBeGreaterThan(0);
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' }); await settle();
  expect(s.model().eq.bands[1].gain_db).toBe(0); expect(s.writes().at(-1).gesture.end).toBe(true);
});
it.each(['knob', 'handle'] as const)('focuses a hovered %s wheel burst so Escape cancels it', async kind => {
  vi.useFakeTimers(); const s = setup('eq'); await settle(); render(<button>Previous focus</button>); screen.getByRole('button', { name: 'Previous focus' }).focus();
  const target = screen.getByRole('slider', { name: kind === 'handle' ? 'EQ band 2' : 'Band 2 Gain' });
  fireEvent.wheel(target, { deltaY: -1 }); await settle(); expect(document.activeElement).toBe(target);
  expect(s.writes()).toHaveLength(1); fireEvent.keyDown(document.activeElement!, { key: 'Escape' }); await settle();
  expect(s.model().eq.bands[1]).toMatchObject({ gain_db: 0, q: 1 });
  await act(async () => vi.advanceTimersByTimeAsync(450)); expect(s.writes()).toHaveLength(2);
});
it.each([{ width: 780, height: 250, dx: 50 }, { width: 270, height: 400, dx: 20 }])('keeps EQ dragging under the pointer for the $width × $height meet viewport', async ({ width, height, dx }) => {
  const s = setup('eq'); await settle(); const handle = screen.getByRole('slider', { name: 'EQ band 3' });
  vi.spyOn(handle.closest('svg')!, 'getBoundingClientRect').mockReturnValue({ x: 80, y: 60, left: 80, top: 60, width, height, right: 80 + width, bottom: 60 + height, toJSON() {} });
  const scale = Math.min(width / 540, height / 245), originalX = Number(handle.getAttribute('cx')), originalY = Number(handle.getAttribute('cy'));
  fireEvent.pointerDown(handle, { button: 0, clientX: 250, clientY: 180 });
  fireEvent.pointerMove(handle, { clientX: 250 + dx, clientY: 160 }); await settle();
  expect(s.model().eq.bands[2].freq).toBeCloseTo(500 * Math.pow(1000, dx / scale / 500));
  expect(s.model().eq.bands[2].gain_db).toBeCloseTo(20 / scale / 5);
  expect((Number(handle.getAttribute('cx')) - originalX) * scale).toBeCloseTo(dx);
  expect((Number(handle.getAttribute('cy')) - originalY) * scale).toBeCloseTo(-20);
  fireEvent.pointerUp(handle); await settle();
});
