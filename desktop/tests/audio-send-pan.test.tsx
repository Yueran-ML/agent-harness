// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PanControl, panCapabilities } from '../src/studio/audio/PanControl';
import { defaultPan } from '../src/studio/audio/routingModel';
import { SendWindow } from '../src/studio/fairlight/SendWindow';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { Monitoring } from '../src/studio/fairlight/Monitoring';
import { AutomationTouchProvider } from '../src/studio/fairlight/AutomationTouch';
import { defaultRecord } from '../src/studio/fairlight/automation';
import type { AutomationParameter, EngineAnswer, Timeline } from '../src/studio/engine';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 15; i++) await Promise.resolve(); });
function setup() {
  const song = { ...timeline([]), tracks: [{ id: 'A1', name: 'Voice', kind: 'audio', format: 'stereo', enabled: true, locked: false, clips: [], transitions: [], automation_arm: true,
    strip: { pan: defaultPan(), outputs: [{ bus: 'B1', on: true }], sends: [
      { bus: 'B2', on: true, pre: false, level_db: 0, pan: defaultPan() }, { bus: 'B3', on: true, pre: false, level_db: -8, pan: { ...defaultPan(), x: .9 } },
    ] } }], buses: [{ id: 'B1', name: 'Main', format: '5.1', color: '#aaa' }, { id: 'B2', name: 'Reverb', format: '5.1', color: '#aaa' }, { id: 'B3', name: 'Delay', format: 'stereo', color: '#aaa' }],
    audio: { record: { ...defaultRecord(), preview: true } } } as unknown as Timeline;
  const parameters: AutomationParameter[] = ['pan.x', 'pan.y', 'pan.divergence', 'send.B2.on', 'send.B2.level_db', 'send.B2.pan.x', 'send.B2.pan.y', 'send.B3.on', 'send.B3.level_db', 'send.B3.pan.x', 'send.B3.pan.y', 'output.B1.on'].map(path => ({ path, kind: path.endsWith('.on') ? 'switch' : 'continuous', scale: 'linear', min: path.endsWith('level_db') ? -100 : -1, max: path.endsWith('level_db') ? 10 : 1, value: 0 }));
  const call = vi.fn(async(method: string): Promise<EngineAnswer> => ({ ok: true, result: method === 'automation.parameters' ? { revision: 7, of: { track: 'A1' }, parameters } : { revision: 8, done: [] } }));
  const context = { bridge: { call }, session: 1, revision: 7, timeline: song, target: { track: 'A1' }, seconds: 0 };
  return { song, call, context, writes: () => call.mock.calls.filter(c => c[0] === 'transaction.apply') as any[], touches: () => call.mock.calls.filter(c => c[0] === 'automation.touch').map(c => (c as any[])[1]) };
}
it('drags two-dimensional Pan with one wire gesture and Escape restores the original values', async () => {
  const s = setup(); render(<PanControl context={s.context} stripKey="track:A1" />);
  const pan = screen.getByRole('slider', { name: 'Pan Voice' });
  fireEvent.pointerDown(pan, { button: 0, clientX: 100, clientY: 100 }); fireEvent.pointerMove(pan, { clientX: 140, clientY: 60 }); await settle();
  expect(s.writes()[0][1].ops[0]).toMatchObject({ edit: 'set_track_audio', track: 'A1' });
  expect(s.writes()[0][1].ops[0].patch.strip.pan.x).toBeCloseTo(40 / 75); expect(s.writes()[0][1].ops[0].patch.strip.pan.y).toBeCloseTo(40 / 75);
  fireEvent.keyDown(pan, { key: 'Escape' }); await settle();
  expect(s.writes().at(-1)[1].ops[0].patch.strip.pan).toEqual({ x: 0, y: 0 });
  expect(s.writes().at(-1)[1].gesture).toEqual({ id: s.writes()[0][1].gesture.id, end: true });
});
it('distinguishes a tiny click from drag and opens the detailed window without a double-click bypass', async () => {
  vi.useFakeTimers(); const s = setup(); const v = render(<PanControl context={s.context} stripKey="track:A1" />);
  const pan = screen.getByRole('slider', { name: 'Pan Voice' });
  fireEvent.pointerDown(pan, { button: 0, clientX: 100, clientY: 100 }); fireEvent.pointerMove(pan, { clientX: 102, clientY: 100 }); fireEvent.pointerUp(pan);
  expect(s.writes()).toHaveLength(0); await act(async () => vi.advanceTimersByTimeAsync(501));
  expect(s.writes()[0][1].ops[0].patch.strip.pan).toEqual({ on: false });
  v.unmount(); s.call.mockClear(); render(<PanControl context={s.context} stripKey="track:A1" />);
  const next = screen.getByRole('slider', { name: 'Pan Voice' });
  for (let i = 0; i < 2; i++) { fireEvent.pointerDown(next, { button: 0 }); fireEvent.pointerUp(next); await act(async () => vi.advanceTimersByTimeAsync(100)); }
  fireEvent.doubleClick(next); await act(async () => vi.advanceTimersByTimeAsync(600));
  expect(screen.getByRole('dialog', { name: 'Pan · Voice' })).toBeDefined(); expect(s.writes()).toHaveLength(0);
});
it.each(['width', 'axis'] as const)('supports Ctrl width and Shift axis lock: %s', async mode => {
  const s = setup(); render(<PanControl context={s.context} stripKey="track:A1" />); const pan = screen.getByRole('slider', { name: 'Pan Voice' });
  fireEvent.pointerDown(pan, { button: 0, clientX: 100, clientY: 100, ctrlKey: mode === 'width' });
  fireEvent.pointerMove(pan, { clientX: 80, clientY: 60, shiftKey: mode === 'axis' }); fireEvent.pointerUp(pan); await settle();
  const panValue = s.writes().at(-1)[1].ops[0].patch.strip.pan;
  if (mode === 'width') expect(panValue).toEqual({ spread: 89.2 }); else { expect(panValue.x).toBe(0); expect(panValue.y).toBeCloseTo(40 / 75); }
});
it('uses target layouts for rear/LFE controls and keeps sends independent in static transactions', async () => {
  const s = setup(); render(<SendWindow context={s.context} stripKey="track:A1" onClose={() => {}} />);
  const reverb = within(screen.getByText('Reverb').closest('article')!), delay = within(screen.getByText('Delay').closest('article')!);
  expect((delay.getByRole('spinbutton', { name: 'Front/Back value' }) as HTMLInputElement).disabled).toBe(true);
  expect((delay.getByRole('checkbox', { name: 'Boom On' }) as HTMLInputElement).disabled).toBe(true);
  expect((reverb.getByRole('spinbutton', { name: 'Front/Back value' }) as HTMLInputElement).disabled).toBe(false);
  fireEvent.click(reverb.getByRole('checkbox', { name: 'Pre' })); await settle();
  const sends = s.writes()[0][1].ops[0].patch.strip.sends;
  expect(sends[0]).toMatchObject({ bus: 'B2', pre: true, level_db: 0 }); expect(sends[1]).toMatchObject({ bus: 'B3', pre: false, level_db: -8, pan: { x: .9 } });
  expect(panCapabilities(['3.0'])).toEqual({ rear: false, lfe: false }); expect(panCapabilities(['7.1'])).toEqual({ rear: true, lfe: true });
});
it('records both Pan axes, releases each and refuses unsupported metadata paths without static writes', async () => {
  const s = setup(); const view = render(<AutomationTouchProvider {...s.context} enabled recording show={() => {}} onFailure={e => { throw e; }}><PanControl context={s.context} stripKey="track:A1" /></AutomationTouchProvider>); await settle();
  const pan = screen.getByRole('slider', { name: 'Pan Voice' }); fireEvent.pointerDown(pan, { button: 0, clientX: 100, clientY: 100 }); fireEvent.pointerMove(pan, { clientX: 140, clientY: 60 }); fireEvent.pointerUp(pan); await settle();
  for (const path of ['pan.x', 'pan.y']) { const last = s.touches().filter(m => m.path === path).at(-1); expect(last.touching).toBe(false); expect(last.value).toBeCloseTo(40 / 75); }
  fireEvent.doubleClick(pan); expect((screen.getByRole('checkbox', { name: 'Pan Enable' }) as HTMLInputElement).disabled).toBe(true); expect((screen.getByRole('spinbutton', { name: 'Rotate value' }) as HTMLInputElement).disabled).toBe(true);
  view.unmount(); expect(s.writes()).toHaveLength(0);
});
it('records send level and target-specific Pan and releases on window teardown', async () => {
  const s = setup(); const v = render(<AutomationTouchProvider {...s.context} enabled recording show={() => {}} onFailure={e => { throw e; }}><SendWindow context={s.context} stripKey="track:A1" onClose={() => {}} /></AutomationTouchProvider>); await settle();
  const row = within(screen.getByText('Reverb').closest('article')!); expect((row.getByRole('checkbox', { name: 'Pre' }) as HTMLInputElement).disabled).toBe(true);
  const cap = row.getByRole('slider', { name: 'Send Level Reverb' }); fireEvent.wheel(cap, { deltaY: -1 }); await settle();
  const pan = row.getByRole('slider', { name: 'Pan Voice' }); fireEvent.pointerDown(pan, { button: 0, clientX: 100, clientY: 100 }); fireEvent.pointerMove(pan, { clientX: 120, clientY: 80 }); v.unmount(); await settle();
  for (const path of ['send.B2.level_db', 'send.B2.pan.x', 'send.B2.pan.y']) expect(s.touches().filter(m => m.path === path).at(-1)?.touching).toBe(false);
  expect(s.touches().some(m => m.path.startsWith('send.B3.'))).toBe(false); expect(s.writes()).toHaveLength(0);
});
it('does not let an old Pan knob wheel timer release a later surface drag of the same path', async () => {
  vi.useFakeTimers(); const s = setup(); s.song.buses![0].format = 'stereo';
  render(<AutomationTouchProvider {...s.context} enabled recording show={() => {}} onFailure={e => { throw e; }}><PanControl context={s.context} stripKey="track:A1" /></AutomationTouchProvider>); await settle();
  fireEvent.doubleClick(screen.getByRole('slider', { name: 'Pan Voice' })); const dialog = within(screen.getByRole('dialog'));
  fireEvent.wheel(dialog.getByRole('slider', { name: 'Left/Right' }), { deltaY: -1 }); await act(async () => vi.advanceTimersByTimeAsync(100));
  const surface = dialog.getByRole('slider', { name: 'Pan Voice' }); fireEvent.pointerDown(surface, { button: 0, clientX: 100, clientY: 100 }); fireEvent.pointerMove(surface, { clientX: 120, clientY: 100 });
  await act(async () => vi.advanceTimersByTimeAsync(600)); expect(s.touches().at(-1)).toMatchObject({ path: 'pan.x', touching: true });
  fireEvent.pointerUp(surface); await settle(); expect(s.touches().at(-1)).toMatchObject({ path: 'pan.x', touching: false }); expect(s.writes()).toHaveLength(0);
});
it('reads active lanes and opens the real Send window from the shared mixer', async () => {
  const s = setup(); (s.song.tracks[0] as any).lanes = { 'pan.x': { points: [{ at: { ticks: 0, scale: 48000 }, value: -.5 }] }, 'send.B2.level_db': { points: [{ at: { ticks: 0, scale: 48000 }, value: -12 }] } };
  render(<Mixer {...s.context} frame={0} meters={null} selected={[]} onSelect={() => {}} />);
  expect(screen.getByRole('slider', { name: 'Pan Voice' }).getAttribute('aria-valuenow')).toBe('-50');
  fireEvent.click(screen.getByText('Reverb -12.0')); expect(screen.getByRole('dialog', { name: 'Bus Sends · Voice' })).toBeDefined();
  const level = screen.getByRole('slider', { name: 'Send Level Reverb' }); expect(level.getAttribute('aria-disabled')).toBe('true');
});
it('keeps null monitoring as None and identifies Main by its own lack of outputs', () => {
  const s = setup(), patch = vi.fn(); s.song.audio!.monitor = null; s.song.buses![0].strip = { outputs: [{ bus: 'B2', on: true }] } as never;
  render(<Monitoring timeline={s.song} meters={null} onPatch={patch} onAction={() => {}} onResetPeaks={() => {}} />);
  const select = screen.getByRole('combobox', { name: 'Monitor bus' }) as HTMLSelectElement; expect(select.value).toBe('');
  expect(within(screen.getByRole('group', { name: 'Main outputs' })).getAllByRole('option').map(o => o.textContent)).toEqual(['Reverb', 'Delay']);
  fireEvent.change(select, { target: { value: 'B2' } }); expect(patch).toHaveBeenLastCalledWith({ monitor: 'B2' }); fireEvent.change(select, { target: { value: '' } }); expect(patch).toHaveBeenLastCalledWith({ monitor: null });
});
it.each([60, 240])('moves the Pan marker exactly with the pointer on a real %d px surface', async size => {
  const s = setup(); render(<PanControl context={s.context} stripKey="track:A1" />);
  if (size === 240) fireEvent.doubleClick(screen.getByRole('slider', { name: 'Pan Voice' }));
  const pan = size === 240 ? within(screen.getByRole('dialog')).getByRole('slider', { name: 'Pan Voice' }) : screen.getByRole('slider', { name: 'Pan Voice' });
  vi.spyOn(pan, 'getBoundingClientRect').mockReturnValue({ left: 20, top: 40, width: size, height: size, right: 20 + size, bottom: 40 + size, x: 20, y: 40, toJSON() {} });
  const position = () => pan.querySelector('g')!.getAttribute('transform')!.match(/translate\(([-\d.]+) ([-\d.]+)\)/)!.slice(1).map(Number).map(n => n / 200 * size);
  const before = position(), dx = size / 6, dy = -size / 8;
  fireEvent.pointerDown(pan, { button: 0, clientX: 20 + size / 2, clientY: 40 + size / 2 });
  fireEvent.pointerMove(pan, { clientX: 20 + size / 2 + dx, clientY: 40 + size / 2 + dy });
  const after = position(); expect(after[0] - before[0]).toBeCloseTo(dx); expect(after[1] - before[1]).toBeCloseTo(dy);
  fireEvent.pointerUp(pan); await settle();
  expect(new Set(s.writes().map(call => call[1].gesture.id)).size).toBe(1); expect(s.writes().at(-1)[1].gesture.end).toBe(true);
});
