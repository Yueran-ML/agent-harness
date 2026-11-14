// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { BusFormat } from '../src/studio/fairlight/BusFormat';
import { RouteButtons } from '../src/studio/fairlight/RouteButtons';
import { BusAssign } from '../src/studio/fairlight/BusAssign';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { mixerStrips } from '../src/studio/audio/mixerModel';
import { targetOf } from '../src/studio/audio/processorModel';
import { timeline } from './timeline-fixture';
import { likeABrowser } from './mounted';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
const settle = () => act(async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); });
function setup() {
  const song: Timeline = { ...timeline([]), tracks: ['A1', 'A2'].map(id => ({ id, name: id, kind: 'audio', enabled: true, locked: false, clips: [], transitions: [] })),
    buses: [{ id: 'B1', name: 'Bus 1', color: '#5aa0e0ff', format: 'stereo' }, { id: 'B2', name: 'Bus 2', color: '#ffaa00ff', format: 'stereo' }] };
  const call = vi.fn(async () => ({ ok: true as const, result: { revision: 8, done: [] } }));
  return { song, call, context: { timeline: song, bridge: { call } as Pick<EngineBridge, 'call'>, session: 1, revision: 7 }, close: vi.fn() };
}
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
it('keeps Bus Format edits local until OK, then uses one transaction; Cancel writes nothing', async () => {
  const { context, call, close } = setup();
  const view = render(<BusFormat {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Add Bus' }));
  fireEvent.change(screen.getByRole('textbox', { name: 'Name B3' }), { target: { value: 'Music' } });
  fireEvent.change(screen.getByRole('combobox', { name: 'Format B3' }), { target: { value: '5.1' } });
  expect(call).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Cancel' })); expect(close).toHaveBeenCalledOnce(); expect(call).not.toHaveBeenCalled();
  view.unmount(); render(<BusFormat {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Add Bus' })); fireEvent.click(screen.getByRole('button', { name: 'OK' })); await settle();
  expect(call).toHaveBeenCalledTimes(1); expect(call.mock.calls[0]).toMatchObject(['transaction.apply', { revision: 7, ops: [{ op: 'add_bus', bus: { id: 'B3', format: 'stereo' } }] }, { expectedSession: 1 }]);
});
it('lists in_use references and refuses a stale Bus Format draft instead of overwriting new changes', async () => {
  const { context, call, close } = setup(); const view = render(<BusFormat {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('radio', { name: 'Bus 1' })); fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
  expect(screen.getByRole('alert').textContent).toContain('A1'); expect(call).not.toHaveBeenCalled();
  view.rerender(<BusFormat {...context} revision={8} onClose={close} />);
  expect((screen.getByRole('button', { name: 'OK' }) as HTMLButtonElement).disabled).toBe(true);
});
it('routes Alt selection in one transaction, excludes locked owners, and keeps Ctrl menu open', async () => {
  const { context, song, call } = setup(); song.tracks[1].locked = true;
  const strip = mixerStrips(song)[0];
  render(<RouteButtons context={{ ...context, target: targetOf(strip), selected: ['track:A1', 'track:A2'] }} strip={strip} field="outputs" onOpen={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'Add output A1' }));
  fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Bus 2' }), { altKey: true, ctrlKey: true }); await settle();
  // Ctrl+Alt covers all unlocked strips; B2 itself is excluded by the cycle check.
  const ops = (call.mock.calls[0] as any)[1].ops;
  expect(ops.map((op: any) => op.track ?? op.id)).toEqual(['A1', 'B1']);
  expect(screen.getByRole('menu')).toBeTruthy();
  expect(ops[0].patch.strip.outputs).toEqual([{ bus: 'B1', on: true }, { bus: 'B2', on: true }]);
});
it('Bus Assign drag paints one transaction and keeps self-routing unavailable', async () => {
  const { context, call, close } = setup(); render(<BusAssign {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Out Bus 2' }));
  const list = within(screen.getByRole('group', { name: 'Assignment targets' }));
  expect((list.getByRole('button', { name: 'Bus 2' }) as HTMLButtonElement).disabled).toBe(true);
  fireEvent.pointerDown(list.getByRole('button', { name: 'A1' }), { button: 0, pointerId: 1 });
  fireEvent.pointerEnter(list.getByRole('button', { name: 'A2' }), { buttons: 1, pointerId: 1 });
  expect(call).not.toHaveBeenCalled(); fireEvent.pointerUp(window); await settle();
  expect(call).toHaveBeenCalledOnce(); expect((call.mock.calls[0] as any)[1].ops.map((o: any) => o.track)).toEqual(['A1', 'A2']);
});
it('replaces a bus at the 64-bus limit by freeing its slot before adding the new draft bus', async () => {
  const { context, song, call, close } = setup();
  song.buses = Array.from({ length: 64 }, (_, index) => ({ id: `B${index + 1}`, name: `Bus ${index + 1}`, color: '#777777ff', format: 'stereo' }));
  render(<BusFormat {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('radio', { name: 'Bus 64' })); fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
  fireEvent.click(screen.getByRole('button', { name: 'Add Bus' })); fireEvent.click(screen.getByRole('button', { name: 'OK' })); await settle();
  expect((call.mock.calls[0] as any)[1].ops).toMatchObject([{ op: 'remove_bus', id: 'B64' }, { op: 'add_bus', bus: { id: 'B65' } }]);
});
it('uses global undo/redo inside the overlay, waiting for a pending assignment reply', async () => {
  const { context, call, close } = setup();
  let finish!: (value: { ok: true; result: { revision: number; done: never[] } }) => void;
  call.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  render(<BusAssign {...context} onClose={close} />);
  fireEvent.click(screen.getByRole('button', { name: 'Out Bus 2' }));
  fireEvent.click(within(screen.getByRole('group', { name: 'Assignment targets' })).getByRole('button', { name: 'A1' })); await settle();
  const window = screen.getByRole('dialog', { name: 'Bus Assign' });
  fireEvent.keyDown(window, { key: 'z', code: 'KeyZ', ctrlKey: true }); await settle();
  expect(call).toHaveBeenCalledTimes(1);
  finish({ ok: true, result: { revision: 8, done: [] } }); await settle();
  expect(call).toHaveBeenNthCalledWith(2, 'history.undo', {}, { expectedSession: 1 });
  fireEvent.keyDown(window, { key: 'Z', code: 'KeyZ', ctrlKey: true, shiftKey: true }); await settle();
  expect(call).toHaveBeenNthCalledWith(3, 'history.redo', {}, { expectedSession: 1 });
});
it('moves focus into each real mixer window and restores it when Escape closes the window', () => {
  const { context } = setup();
  render(<Mixer {...context} frame={0} meters={null} selected={[]} onSelect={() => {}} />);
  const assign = screen.getByRole('button', { name: 'Bus Assign' }); assign.focus(); fireEvent.click(assign);
  expect(document.activeElement).toBe(screen.getByRole('dialog', { name: 'Bus Assign' }));
  fireEvent.keyDown(document.activeElement!, { key: 'Escape' });
  expect(screen.queryByRole('dialog', { name: 'Bus Assign' })).toBeNull(); expect(document.activeElement).toBe(assign);
  const format = screen.getByRole('button', { name: 'Bus Format' }); format.focus(); fireEvent.click(format);
  const dialog = screen.getByRole('dialog', { name: 'Bus Format' });
  expect(dialog.tagName).toBe('DIALOG'); expect(dialog.hasAttribute('open')).toBe(true);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Cancel' }));
  fireEvent(dialog, new Event('cancel', { cancelable: true })); expect(screen.queryByRole('dialog', { name: 'Bus Format' })).toBeNull(); expect(document.activeElement).toBe(format);
});
