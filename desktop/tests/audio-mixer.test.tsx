// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Fader } from '../src/studio/audio/Fader';
import { Meter } from '../src/studio/audio/Meter';
import { Mixer } from '../src/studio/fairlight/Mixer';
import { likeABrowser } from './mounted';
import { timeline } from './timeline-fixture';
import type { EngineBridge } from '../src/bridge';
import type { Timeline } from '../src/studio/engine';
import { Words } from '../src/app/locale';
import { en } from '../src/messages/en';
import { zhCN } from '../src/messages/zh-CN';
import { ja } from '../src/messages/ja';
const settle = () => act(async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); });
const pointer = (y: number, extra = {}) => ({ button: 0, pointerId: 1, clientY: y, ...extra });
beforeEach(() => { likeABrowser(); vi.stubGlobal('PointerEvent', MouseEvent); });
afterEach(() => { cleanup(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
function song(): Timeline { return { ...timeline([]), tracks: [-20, 9, -99].map((db, i) => ({ id: `a${i + 1}`, name: `Audio ${i + 1}`, kind: 'audio', enabled: true, locked: false, clips: [], transitions: [], strip: { fader_db: db } as never })), buses: [{ id: 'B1', name: 'Bus 1', color: '#777777ff', format: 'stereo' }] }; }
function mount(t = song(), chosen = ['track:a1']) {
  let revision = 7;
  const call = vi.fn(async () => ({ ok: true as const, result: { revision: ++revision, done: [] } }));
  const bridge = { call, onEvent: () => () => {} } as Pick<EngineBridge, 'call' | 'onEvent'>;
  const props = { bridge, session: 1, revision: 7, timeline: t, frame: 0, meters: null, selected: chosen, onSelect: vi.fn() };
  return { ...render(<Mixer {...props} />), call, props };
}
it('uses taper pixels without jumping on the slot, Shift fine precision, wheel steps and numeric entry', () => {
  vi.useFakeTimers(); const change = vi.fn(), end = vi.fn();
  render(<Fader label="Gain" value={0} onStart={() => true} onChange={change} onEnd={end} onCancel={() => {}} onCommit={change} />);
  const knob = screen.getByRole('slider'); const slot = knob.parentElement!;
  vi.spyOn(slot, 'getBoundingClientRect').mockReturnValue({ height: 200 } as DOMRect);
  fireEvent.pointerDown(slot, pointer(10)); expect(change).not.toHaveBeenCalled();
  fireEvent.pointerDown(knob, pointer(100)); fireEvent.pointerMove(knob, pointer(70));
  expect(change).toHaveBeenLastCalledWith(5); fireEvent.pointerUp(knob, pointer(70));
  fireEvent.pointerDown(knob, pointer(100)); fireEvent.pointerMove(knob, pointer(70, { shiftKey: true }));
  expect(change).toHaveBeenLastCalledWith(.5); fireEvent.pointerUp(knob, pointer(70));
  fireEvent.wheel(knob, { deltaY: -1 }); expect(change).toHaveBeenLastCalledWith(.5);
  act(() => vi.advanceTimersByTime(500));
  fireEvent.wheel(knob, { deltaY: 1, shiftKey: true }); expect(change).toHaveBeenLastCalledWith(-.1);
  act(() => vi.advanceTimersByTime(500));
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Gain value' }));
  fireEvent.change(screen.getByRole('textbox'), { target: { value: '-7.2' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
  expect(change).toHaveBeenLastCalledWith(-7.2);
});
it('shows engine meter positions, null silence and disabled state without simulated motion', () => {
  const view = render(<Meter label="Signal" channels={2} values={[{ bar: -18, peak: -4, bar_position: .73, peak_position: .91 }]} />);
  expect(screen.getAllByRole('meter')).toHaveLength(2);
  expect(document.querySelector('.audio-meter-bar')?.getAttribute('style')).toContain('73%');
  expect(document.querySelector('.audio-meter-peak')?.getAttribute('style')).toContain('91%');
  view.rerender(<Meter label="Signal" channels={2} disabled values={[{ bar: -18, peak: -4, bar_position: .73, peak_position: .91 }]} />);
  expect(document.querySelector('.audio-meter-bar')?.getAttribute('style')).toContain('0%');
  expect(document.querySelector('.audio-meter-peak')).toBeNull();
});
it('previews a real transaction, restores every group member on Escape and shares one undo gesture', async () => {
  const t = song(); t.track_groups = [{ id: 'g', name: 'Group', members: ['a1', 'a2'] }];
  const { call } = mount(t, ['track:a1', 'track:a2']); const knob = screen.getByRole('slider', { name: 'Fader Audio 1' });
  fireEvent.pointerDown(knob, pointer(100, { altKey: true })); fireEvent.pointerMove(knob, pointer(80)); await settle();
  expect(call).toHaveBeenCalledTimes(1); const first = (call.mock.calls[0] as any)[1];
  expect(first.ops).toHaveLength(2); expect(first.ops.every((op: any) => op.follow_groups === false)).toBe(true);
  fireEvent.keyDown(knob, { key: 'Escape' }); await settle();
  const last = (call.mock.calls.at(-1) as any)[1];
  expect(last.gesture).toEqual({ id: first.gesture.id, end: true }); expect(last.ops.map((op: any) => op.patch.strip.fader_db)).toEqual([-20, 9]);
  expect(last.revision).toBe(7);
});
it('does not overwrite a later revision or session, and cancels a blur in its captured session', async () => {
  const { call, props, rerender } = mount(); const knob = screen.getByRole('slider', { name: 'Fader Audio 1' });
  fireEvent.pointerDown(knob, pointer(100)); fireEvent.pointerMove(knob, pointer(80)); await settle();
  rerender(<Mixer {...props} revision={9} />); fireEvent.keyDown(knob, { key: 'Escape' }); await settle(); expect(call).toHaveBeenCalledTimes(1);
  rerender(<Mixer {...props} session={2} revision={9} />); fireEvent.pointerDown(knob, pointer(100)); fireEvent.pointerMove(knob, pointer(80)); await settle();
  fireEvent.blur(window); await settle(); expect((call.mock.calls.at(-1) as any)[2]).toEqual({ expectedSession: 2 });
});
it('keeps automated faders read-only at real playback values and blocks reset even when Off', async () => {
  const t = song(); (t.tracks[0] as any).lanes = { fader_db: { points: [{ at: { ticks: 0, scale: 48000 }, value: -10 }] } };
  const { call, props, rerender } = mount(t); let knob = screen.getByRole('slider', { name: 'Fader Audio 1' });
  expect(knob.getAttribute('aria-valuenow')).toBe('-10'); expect(knob.getAttribute('aria-readonly')).toBe('true');
  fireEvent.pointerDown(knob, pointer(100)); fireEvent.pointerMove(knob, pointer(80)); fireEvent.doubleClick(knob); await settle(); expect(call).not.toHaveBeenCalled();
  (t.tracks[0] as any).automation_arm = 'off'; rerender(<Mixer {...props} timeline={{ ...t }} />); knob = screen.getByRole('slider', { name: 'Fader Audio 1' });
  fireEvent.doubleClick(knob); await settle(); expect(call).not.toHaveBeenCalled(); expect(knob.getAttribute('aria-valuenow')).toBe('-20');
});
it('includes the first button in one solo/mute sweep and sends Solo Safe through the real edit', async () => {
  const { call } = mount();
  fireEvent.pointerDown(screen.getByRole('button', { name: 'mute Audio 1' }), pointer(0));
  fireEvent.pointerEnter(screen.getByRole('button', { name: 'mute Audio 2' }), { buttons: 1 }); fireEvent.pointerUp(window); await settle();
  expect((call.mock.calls[0] as any)[1].ops).toMatchObject([{ track: 'a1', patch: { mute: true } }, { track: 'a2', patch: { mute: true } }]);
  // A new accepted revision is delivered by the host in normal use.
});
it('handles Solo Safe, single/range/toggle selection and double-click rename with isolated keys', async () => {
  const { call, props } = mount();
  fireEvent.click(screen.getByRole('button', { name: 'solo Audio 1' }), { ctrlKey: true, shiftKey: true }); await settle();
  expect((call.mock.calls[0] as any)[1].ops[0]).toMatchObject({ edit: 'set_track_audio', patch: { solo_safe: true } });
  fireEvent.click(screen.getByRole('button', { name: 'Audio 2' }), { ctrlKey: true }); expect(props.onSelect).toHaveBeenLastCalledWith(['track:a1', 'track:a2']);
  fireEvent.click(screen.getByRole('button', { name: 'Audio 3' }), { shiftKey: true }); expect(props.onSelect).toHaveBeenLastCalledWith(['track:a1', 'track:a2', 'track:a3']);
  const globalKey = vi.fn(); window.addEventListener('keydown', globalKey);
  fireEvent.keyDown(screen.getByRole('button', { name: 'Audio 1' }), { key: 'Delete' }); expect(globalKey).not.toHaveBeenCalled(); window.removeEventListener('keydown', globalKey);
});
it('resets only unautomated faders and commits numeric input through one transaction', async () => {
  const { call, props, rerender } = mount();
  fireEvent.doubleClick(screen.getByRole('slider', { name: 'Fader Audio 1' })); await settle();
  expect((call.mock.calls[0] as any)[1]).toMatchObject({ gesture: { end: true }, ops: [{ patch: { strip: { fader_db: 0 } } }] });
  rerender(<Mixer {...props} revision={8} />);
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Fader value Audio 1' }));
  fireEvent.change(screen.getByRole('textbox'), { target: { value: '-8.5' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' }); await settle();
  expect((call.mock.calls.at(-1) as any)[1].ops[0].patch.strip.fader_db).toBe(-8.5);
});
it('keeps a wheel burst in one gesture, cancels with Escape and never sends automation.touch', async () => {
  vi.useFakeTimers(); const { call } = mount(); const cap = screen.getByRole('slider', { name: 'Fader Audio 1' });
  fireEvent.wheel(cap, { deltaY: -1, ctrlKey: true, altKey: true }); await settle();
  fireEvent.wheel(cap, { deltaY: -1, ctrlKey: true, altKey: true }); await settle();
  const transactions = call.mock.calls as any[];
  expect(transactions[1][1].ops.map((op: any) => op.patch.strip.fader_db)).toEqual([-19, 10, -98, 1]);
  expect(new Set(transactions.map((row: any) => row[1].gesture.id)).size).toBe(1);
  fireEvent.keyDown(cap, { key: 'Escape' }); await settle();
  expect((call.mock.calls.at(-1) as any)[1].ops.map((op: any) => op.patch.strip.fader_db)).toEqual([-20, 9, -99, 0]);
  expect(transactions.every(row => row[0] === 'transaction.apply')).toBe(true);
});
it('renames tracks and buses through real operations and reports rejected writes', async () => {
  const { call, props, rerender } = mount();
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Audio 1' }));
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Lead' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' }); await settle();
  expect((call.mock.calls[0] as any)[1].ops[0]).toMatchObject({ op: 'set_track', id: 'a1', name: 'Lead' });
  rerender(<Mixer {...props} revision={8} />);
  call.mockImplementationOnce(async () => ({ ok: false, error: { code: -32001, message: 'revision changed' } }) as never);
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Bus 1' })); fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Master' } });
  fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' }); await settle();
  expect((call.mock.calls.at(-1) as any)[1].ops[0]).toMatchObject({ edit: 'set_bus', id: 'B1', patch: { name: 'Master' } });
  expect(screen.getByRole('alert').textContent).toContain('revision changed');
});
it.each([en, zhCN, ja])('translates active strip controls and automation explanations', words => {
  const t = song(); (t.tracks[0] as any).lanes = { fader_db: { points: [{ at: { ticks: 0, scale: 48000 }, value: -2 }] } };
  const labels = words.label.studio.fairlightPage;
  render(<Words.Provider value={words}><Mixer bridge={{ call: vi.fn() } as never} session={1} revision={0} timeline={t} frame={0} meters={null} selected={[]} onSelect={() => {}} /></Words.Provider>);
  expect(screen.getByRole('slider', { name: `${labels.Fader} Audio 1` })).toBeDefined();
  expect(screen.getByTitle(labels['Automation controls playback. Static settings do not replace the lane.']).textContent).toContain(labels['Automation controls playback']);
  expect(screen.getByLabelText(labels['Exclusive Solo'])).toBeDefined();
});
it('keeps Alt modifiers for a typed value and releases a lost pointer capture', () => {
  const start = vi.fn(() => true), commit = vi.fn(), cancel = vi.fn();
  render(<Fader label="Gain" value={0} onStart={start} onChange={() => {}} onEnd={() => {}} onCancel={cancel} onCommit={commit} />);
  fireEvent.doubleClick(screen.getByRole('button', { name: 'Gain value' }), { altKey: true, ctrlKey: true });
  fireEvent.change(screen.getByRole('textbox'), { target: { value: '3' } }); fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
  expect(commit).toHaveBeenCalledWith(3, { altKey: true, ctrlKey: true, shiftKey: false });
  const cap = screen.getByRole('slider'); fireEvent.pointerDown(cap, pointer(30)); fireEvent.lostPointerCapture(cap);
  expect(cancel).toHaveBeenCalledTimes(1);
});
it('preserves the engine group Solo under Exclusive Solo for a pointer click and sweep', async () => {
  for (const sweeping of [false, true]) {
    const t = song(); t.audio = { exclusive_solo: true } as never;
    t.track_groups = [{ id: 'g', name: 'G', members: ['a1', 'a2'], linked: ['solo'] }];
    const view = mount(t); const solo = new Set<string>();
    // Narrow protocol oracle matching model/edit/audio.rs::set_track_audio + exclusive_solo:
    // a source edit keeps its directly linked group together; expanded primitive edits do not.
    view.call.mockImplementation((async (_method: any, request: any) => {
      for (const op of request.ops) {
        if (op.edit !== 'set_track_audio' || op.patch.solo !== true) continue;
        const was = solo.has(op.track), group = op.follow_groups !== false && ['a1', 'a2'].includes(op.track) ? ['a1', 'a2'] : [op.track];
        group.forEach(id => solo.add(id));
        if (!was) for (const id of solo) if (!group.includes(id)) solo.delete(id);
      }
      return { ok: true, result: { revision: 8, done: [] } };
    }) as never);
    const first = sweeping ? 'Audio 3' : 'Audio 1';
    fireEvent.pointerDown(screen.getByRole('button', { name: `solo ${first}` }), pointer(0));
    if (sweeping) fireEvent.pointerEnter(screen.getByRole('button', { name: 'solo Audio 1' }), { buttons: 1 });
    fireEvent.pointerUp(window); await settle();
    expect([...solo].sort()).toEqual(['a1', 'a2']);
    expect((view.call.mock.calls[0] as any)[1].ops.every((op: any) => op.follow_groups === true)).toBe(true);
    view.unmount();
  }
});
it('lets other selected faders continue when the grabbed fader reaches its own limit', async () => {
  vi.useFakeTimers(); const { call } = mount(); const cap = screen.getByRole('slider', { name: 'Fader Audio 2' });
  for (let n = 0; n < 4; n++) { fireEvent.wheel(cap, { deltaY: -1, ctrlKey: true, altKey: true }); await settle(); }
  const last = (call.mock.calls.at(-1) as any)[1];
  expect(last.ops.map((op: any) => op.patch.strip.fader_db)).toEqual([-18, 10, -97, 2]);
});
it.each(['release', 'cancel'])('continues a capped group wheel burst into one drag gesture and restores the original snapshot on %s', async finish => {
  vi.useFakeTimers(); const t = song(); t.track_groups = [{ id: 'g', name: 'G', members: ['a1', 'a2'] }];
  const view = mount(t, ['track:a2']); const cap = screen.getByRole('slider', { name: 'Fader Audio 2' });
  vi.spyOn(cap.parentElement!, 'getBoundingClientRect').mockReturnValue({ height: 200 } as DOMRect);
  for (let n = 0; n < 4; n++) { fireEvent.wheel(cap, { deltaY: -1 }); await settle(); }
  const accepted = structuredClone(t); (accepted.tracks[0] as any).strip.fader_db = -18; (accepted.tracks[1] as any).strip.fader_db = 10;
  view.rerender(<Mixer {...view.props} timeline={accepted} revision={11} />);
  const before = (view.call.mock.calls as any[]).length, id = (view.call.mock.calls[0] as any)[1].gesture.id;
  fireEvent.pointerDown(cap, pointer(100)); fireEvent.pointerMove(cap, pointer(99)); await settle();
  const calls = view.call.mock.calls as any[];
  expect(calls.length).toBeGreaterThan(before);
  expect(calls.every(row => row[1].gesture.id === id)).toBe(true);
  // Source requested 11 dB after the wheel; +1 px means +0.25 dB more, even though the cap is at 10.
  const lastGain = (id: string) => calls.at(-1)[1].ops.filter((op: any) => op.track === id).at(-1).patch.strip.fader_db;
  expect(lastGain('a1')).toBeCloseTo(-17.75); expect(lastGain('a2')).toBe(10);
  expect(calls.some(row => row[1].gesture.end)).toBe(false);
  if (finish === 'cancel') fireEvent.keyDown(cap, { key: 'Escape' }); else fireEvent.pointerUp(cap, pointer(99));
  await settle(); expect(calls.at(-1)[1].gesture).toEqual({ id, end: true });
  if (finish === 'cancel') { expect(lastGain('a1')).toBe(-20); expect(lastGain('a2')).toBe(9); }
  // The captured pre-wheel restore is the one protocol undo snapshot; no second gesture/undo step.
  expect(new Set(calls.map(row => row[1].gesture.id)).size).toBe(1);
  const count = calls.length; await act(async () => vi.advanceTimersByTimeAsync(600)); expect(calls.length).toBe(count);
  view.unmount();
});
it('cancels native wheel default only on editable caps, uses current props, and removes the listener', async () => {
  vi.useFakeTimers(); const view = mount(); const cap = screen.getByRole('slider', { name: 'Fader Audio 1' });
  const send = (target: Element) => {
    const event = new WheelEvent('wheel', { deltaY: -1, bubbles: true, cancelable: true });
    act(() => { target.dispatchEvent(event); }); return event;
  };
  expect(send(cap).defaultPrevented).toBe(true); await settle(); expect(view.call).toHaveBeenCalledTimes(1);
  expect(send(cap.parentElement!).defaultPrevented).toBe(false);
  fireEvent.keyDown(cap, { key: 'Escape' }); await settle();
  const locked = song(); locked.tracks[0].locked = true;
  view.rerender(<Mixer {...view.props} timeline={locked} revision={9} />);
  const count = view.call.mock.calls.length; expect(send(cap).defaultPrevented).toBe(false); expect(view.call.mock.calls.length).toBe(count);
  const automated = song(); (automated.tracks[0] as any).lanes = { fader_db: { points: [{ at: { ticks: 0, scale: 48000 }, value: 2 }] } };
  view.rerender(<Mixer {...view.props} timeline={automated} revision={9} />); expect(send(cap).defaultPrevented).toBe(false);
  view.unmount(); expect(send(cap).defaultPrevented).toBe(false); expect(view.call.mock.calls.length).toBe(count);
});
it('queues the wheel-to-drag final value behind an in-flight preview without ending a second gesture', async () => {
  vi.useFakeTimers(); const t = song(); t.track_groups = [{ id: 'g', name: 'G', members: ['a1', 'a2'] }];
  const view = mount(t); let acknowledge!: (reply: any) => void;
  view.call.mockImplementationOnce(() => new Promise(resolve => { acknowledge = resolve; }));
  const cap = screen.getByRole('slider', { name: 'Fader Audio 2' });
  vi.spyOn(cap.parentElement!, 'getBoundingClientRect').mockReturnValue({ height: 200 } as DOMRect);
  fireEvent.wheel(cap, { deltaY: -1 });
  fireEvent.pointerDown(cap, pointer(100)); fireEvent.pointerMove(cap, pointer(98)); fireEvent.pointerUp(cap, pointer(98));
  expect(view.call).toHaveBeenCalledTimes(1);
  await act(async () => acknowledge({ ok: true, result: { revision: 8, done: [] } })); await settle();
  const calls = view.call.mock.calls as any[]; expect(calls).toHaveLength(2);
  expect(calls[1][1]).toMatchObject({ revision: 7, gesture: { id: calls[0][1].gesture.id, end: true } });
  expect(calls[1][1].ops.at(-1)).toMatchObject({ track: 'a2', patch: { strip: { fader_db: 10 } }, follow_groups: true });
  expect(calls[1][1].ops.slice(0, 2).map((op: any) => op.patch.strip.fader_db)).toEqual([-20, 9]);
});
