// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { defaultDynamics, defaultEq } from '../src/studio/audio/processorModel';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
import type { EngineBridge } from '../src/bridge';
import { ProcessorThumbnail } from '../src/studio/audio/ProcessorThumbnail';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
function mount(compact = false) {
  const t = timeline([]); t.tracks = [{ id: 'a', name: 'Voice', kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { eq: defaultEq(), dynamics: defaultDynamics() } as never }];
  let rev = 4;
  const call = vi.fn(async (method: string, params: any) => ({ ok: true as const, result: method === 'audio.eq_response' ? { revision: rev, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) }
    : method === 'audio.dynamics_curve' ? { revision: rev, inputs_db: params.inputs_db, outputs_db: params.inputs_db } : { revision: ++rev, done: [] } }));
  const props = { bridge: { call } as Pick<EngineBridge, 'call'>, timeline: t, revision: rev, session: 1, frame: 0, selected: ['track:a'], onSelect: vi.fn(), meters: null, compact };
  return { ...render(<Mixer {...props} />), call, props, writes: () => call.mock.calls.filter(([method]) => method === 'transaction.apply').map(([, params]) => params) };
}
it('opens EQ with a double click without toggles, and single click toggles once', async () => {
  vi.useFakeTimers(); const s = mount(); await settle(); const button = screen.getByRole('button', { name: 'EQ Voice' });
  fireEvent.click(button, { detail: 1 }); await act(async () => vi.advanceTimersByTimeAsync(400));
  fireEvent.click(button, { detail: 2 }); fireEvent.doubleClick(button); await act(async () => vi.advanceTimersByTimeAsync(550));
  expect(s.writes()).toHaveLength(0); expect(screen.getByRole('dialog', { name: 'EQ · Voice' })).toBeTruthy();
  expect(document.body.dataset.overlay).toBe('1');
  fireEvent.click(button, { detail: 1 }); await act(async () => vi.advanceTimersByTimeAsync(550)); expect(s.writes()).toHaveLength(1);
  expect(s.writes()[0].ops[0].patch.strip).toEqual({ eq: { on: true } });
});
it('provides reachable Edit processor contents, Path Settings and exact Order transaction', async () => {
  const s = mount(true); await settle(); fireEvent.click(screen.getByRole('button', { name: 'Dynamics Voice' }), { detail: 1 });
  expect(screen.getByRole('complementary', { name: 'Track processor · Voice' })).toBeTruthy(); expect(screen.getByRole('spinbutton', { name: 'Compressor Ratio value' })).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Close processor' }));
  expect(document.body.dataset.overlay).toBeUndefined();
  fireEvent.change(screen.getByRole('combobox', { name: 'Order Voice' }), { target: { value: 'eq,dynamics,fx' } }); await settle();
  expect(s.writes()[0].ops[0].patch.strip.order).toEqual(['eq', 'dynamics', 'fx']);
  s.rerender(<Mixer {...s.props} revision={5} />); fireEvent.click(screen.getByRole('button', { name: 'Path Settings Voice' }));
  expect(screen.getByRole('spinbutton', { name: 'Trim value' })).toBeTruthy();
  expect(document.body.dataset.overlay).toBe('1');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Insert' })); await settle();
  expect(s.writes().at(-1).ops[0].patch.strip).toEqual({ inserts_on: false });
});
it.each([1, 0])('opens Edit EQ immediately on mouse/keyboard activation (detail %s), without bypassing', async detail => {
  vi.useFakeTimers(); const s = mount(true); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'EQ Voice' }), { detail });
  expect(screen.getByRole('complementary', { name: 'Track processor · Voice' })).toBeTruthy();
  expect(screen.getByRole('spinbutton', { name: 'Band 2 Gain value' })).toBeTruthy();
  await act(async () => vi.advanceTimersByTimeAsync(600)); expect(s.writes()).toHaveLength(0);
});
it('copies and pastes only the clicked thumbnail module through its context menu', async () => {
  const s = mount(); await settle();
  const eq = screen.getByRole('button', { name: 'EQ Voice' });
  fireEvent.contextMenu(eq); fireEvent.click(screen.getByRole('menuitem', { name: 'Copy' }));
  fireEvent.contextMenu(eq); fireEvent.click(screen.getByRole('menuitem', { name: 'Paste' })); await settle();
  expect(s.writes()[0].ops[0].patch.strip).toEqual({ eq: defaultEq() });
});

it('preserves both delayed toggles when the first transaction publishes a new model and revision', async () => {
  vi.useFakeTimers();
  const s = mount(); await settle();
  let current = s.props;
  s.call.mockImplementation(async (method: string, params: any) => {
    if (method !== 'transaction.apply') return { ok: true as const, result: method === 'audio.eq_response'
      ? { revision: current.revision, sample_rate: 48000, frequencies: params.frequencies, gain_db: params.frequencies.map(() => 0) }
      : { revision: current.revision, inputs_db: params.inputs_db, outputs_db: params.inputs_db } };
    const next = structuredClone(current.timeline), patch = params.ops[0].patch.strip;
    const track = next.tracks[0];
    if (track.kind !== 'audio') throw new Error('Expected the audio track fixture');
    const strip = track.strip!;
    if (patch.eq) strip.eq = { ...strip.eq!, ...patch.eq };
    if (patch.dynamics) strip.dynamics = { ...strip.dynamics!, ...patch.dynamics };
    current = { ...current, timeline: next, revision: current.revision + 1 };
    s.rerender(<Mixer {...current} />);
    return { ok: true as const, result: { revision: current.revision, done: [] } };
  });
  fireEvent.click(screen.getByRole('button', { name: 'EQ Voice' }), { detail: 1 });
  await act(async () => vi.advanceTimersByTimeAsync(200));
  fireEvent.click(screen.getByRole('button', { name: 'Dynamics Voice' }), { detail: 1 });
  await act(async () => vi.advanceTimersByTimeAsync(300)); await settle();
  expect(current.revision).toBe(5); expect(current.timeline.tracks[0]).toMatchObject({ strip: { eq: { on: true } } });
  await act(async () => vi.advanceTimersByTimeAsync(200)); await settle();
  expect(s.writes()).toHaveLength(2);
  expect(current.revision).toBe(6); expect(current.timeline.tracks[0]).toMatchObject({ strip: { eq: { on: true }, dynamics: { on: true } } });
  expect(s.writes().map(write => write.revision)).toEqual([4, 5]);
  expect(s.writes()[1].ops[0].patch.strip).toEqual({ dynamics: { on: true } });
});

function thumbnail() {
  const context = { bridge: { call: vi.fn(async () => ({ ok: false, error: { message: 'No fixture' } })) } as unknown as Pick<EngineBridge, 'call'>,
    session: 1, revision: 4, timeline: timeline([]), target: { track: 'a' } };
  const props = { name: 'Voice', module: 'eq' as 'eq' | 'dynamics', on: false, disabled: false, context,
    settings: defaultEq(), onToggle: vi.fn(), onOpen: vi.fn(), onPaste: vi.fn() };
  return { ...render(<ProcessorThumbnail {...props} />), props };
}
it('uses the current callback and model for a pending click on the same owner', async () => {
  vi.useFakeTimers(); const s = thumbnail(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'EQ Voice' }), { detail: 1 });
  const next = vi.fn();
  s.rerender(<ProcessorThumbnail {...s.props} on onToggle={next} context={{ ...s.props.context, revision: 5 }} />);
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(s.props.onToggle).not.toHaveBeenCalled(); expect(next).toHaveBeenCalledTimes(1);
});
it.each(['session', 'timeline', 'owner', 'module'] as const)('cancels a pending thumbnail click when %s changes', async change => {
  vi.useFakeTimers(); const s = thumbnail(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'EQ Voice' }), { detail: 1 });
  const context = { ...s.props.context };
  if (change === 'session') context.session++;
  if (change === 'timeline') context.timeline = { ...context.timeline, id: 'other' };
  if (change === 'owner') context.target = { track: 'b' };
  const next = vi.fn();
  s.rerender(<ProcessorThumbnail {...s.props} module={change === 'module' ? 'dynamics' : 'eq'} context={context} onToggle={next} />);
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(s.props.onToggle).not.toHaveBeenCalled(); expect(next).not.toHaveBeenCalled();
});
it('does not apply a pending click after its source becomes readonly', async () => {
  vi.useFakeTimers(); const s = thumbnail(); await settle();
  fireEvent.click(screen.getByRole('button', { name: 'EQ Voice' }), { detail: 1 });
  s.rerender(<ProcessorThumbnail {...s.props} disabled />);
  await act(async () => vi.advanceTimersByTimeAsync(500));
  expect(s.props.onToggle).not.toHaveBeenCalled();
});
