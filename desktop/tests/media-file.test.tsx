// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { MediaFile } from '../src/studio/media/MediaFile';
import { sourceLayout } from '../src/studio/media/audioLayout';
import type { EngineBridge } from '../src/bridge';
import type { Media, Op } from '../src/studio/engine';
import { media } from './timeline-fixture';
import { likeABrowser } from './mounted';

beforeEach(() => {
  likeABrowser();
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(400);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); });
function source(id = 'm1', stream = 1): Media {
  const item = { ...media, id, name: `Take ${id}`, facts: { ...media.facts, audio: {
    rate: 48000, channels: 2, codec: 'pcm', streams: [{ index: stream, channels: 2, layout: 'stereo' }],
  } } };
  return { ...item, audio_config: sourceLayout(item)[0].channels.map((channel, index) => ({
    name: `Mic ${index + 1}`, format: 'mono', channels: [{ ...channel, level_db: index ? -3 : 0 }],
  })) };
}
function setup(items = [source()]) {
  let current = items;
  const history: Media[][] = [];
  const apply = vi.fn<(ops: Op[]) => Promise<boolean>>();
  const off = vi.fn();
  const call = vi.fn<EngineBridge['call']>(async () => ({ ok: true, result: { rate: 100, channels: 2,
    peaks: btoa(String.fromCharCode(...Array.from({ length: 1600 }, (_, i) => i % 2 ? 64 : 192))),
  } }));
  const bridge = { call, onEvent: () => off };
  function Harness() {
    const [chosen, setChosen] = useState(items);
    apply.mockImplementation(async ops => {
      history.push(chosen);
      const updated = chosen.map(item => {
        const op = ops.find(op => op.op === 'set_media_audio' && op.id === item.id);
        return op?.op === 'set_media_audio' ? { ...item, audio_config: op.config ?? undefined } : item;
      });
      current = updated; setChosen(updated); return true;
    });
    return <><button onClick={() => { current = history.pop()!; setChosen(current); }}>Undo</button>
      <MediaFile media={chosen} bridge={bridge} session={1} onApply={apply} /></>;
  }
  const view = render(<Harness />);
  return { ...view, bridge, apply, off, history, current: () => current };
}

it('draws the last selected source with shared native peaks and releases its subscription', async () => {
  const s = setup([source('m1', 1), source('m2', 3)]); await settle();
  expect(s.bridge.call).toHaveBeenCalledExactlyOnceWith('media.waveform', { media: 'm2', stream: 3 }, { expectedSession: 1 });
  expect(s.container.querySelectorAll('svg')).toHaveLength(2);
  expect(screen.getByRole('heading', { name: 'Multiple Clips' })).toBeTruthy();
  expect(s.container.querySelectorAll('path')[1].getAttribute('transform')).toContain(`scale(1,${10 ** (-3 / 20)})`);
  s.unmount(); expect(s.off).toHaveBeenCalledOnce();
});

it('changes active and mute across files without replacing their layouts or unrelated trims', async () => {
  const first = source('m1', 1), second = source('m2', 3);
  first.audio_config![0].name = 'Boom'; first.audio_config![1].channels[0].level_db = -8;
  const s = setup([first, second]); await settle();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 1 Active' })); await settle();
  expect(s.apply).toHaveBeenCalledTimes(1);
  expect(s.history).toHaveLength(1);
  for (const item of s.current()) expect(item.audio_config![0].channels[0].enabled).toBe(false);
  expect(s.current()[0].audio_config![0].name).toBe('Boom');
  expect(s.current()[0].audio_config![1].channels[0].level_db).toBe(-8);
  expect(s.current()[0].audio_config![0].channels[0].source).toEqual({ stream: 1, channel: 0 });
  fireEvent.click(screen.getByRole('button', { name: 'Undo' })); await settle();
  expect(s.current()).toEqual([first, second]);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 2 Mute' })); await settle();
  for (const item of s.current()) expect(item.audio_config![1].channels[0].mute).toBe(true);
});

it('Alt-click isolates active or audible channels across all configured tracks', async () => {
  const s = setup(); await settle();
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 2 Active' }), { altKey: true }); await settle();
  expect(s.current()[0].audio_config!.map(row => row.channels[0].enabled)).toEqual([false, true]);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 1 Mute' }), { altKey: true }); await settle();
  expect(s.current()[0].audio_config!.map(row => row.channels[0].mute)).toEqual([false, true]);
});

it('remaps a chosen embedded source to each file own stream IDs and preserves every other channel', async () => {
  const s = setup([source('m1', 1), source('m2', 3)]); await settle();
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: '1' } }); await settle();
  expect(s.current().map(item => item.audio_config![0].channels[0].source)).toEqual([{ stream: 1, channel: 1 }, { stream: 3, channel: 1 }]);
  expect(s.current().map(item => item.audio_config![1].channels[0].level_db)).toEqual([-3, -3]);
  fireEvent.change(screen.getByRole('combobox', { name: 'Channel 1 Source' }), { target: { value: 'none' } }); await settle();
  expect(s.current().map(item => item.audio_config![0].channels[0].source)).toEqual([null, null]);
});

it('sets one absolute level for Ctrl/Shift-selected channels with one undo step per completed slider drag', async () => {
  const s = setup([source('m1'), source('m2', 3)]); await settle();
  const slider = screen.getByRole('slider', { name: 'Channel Level' });
  expect((slider as HTMLInputElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'Channel 1' }));
  fireEvent.click(screen.getByRole('button', { name: 'Channel 2' }), { ctrlKey: true });
  fireEvent.input(slider, { target: { value: '-5' } });
  fireEvent.input(slider, { target: { value: '-12' } });
  expect(s.apply).not.toHaveBeenCalled();
  fireEvent.change(slider, { target: { value: '-12' } }); await settle();
  expect(s.apply).toHaveBeenCalledTimes(1);
  expect(s.current().flatMap(item => item.audio_config!.flatMap(row => row.channels.map(channel => channel.level_db)))).toEqual([-12, -12, -12, -12]);
  expect(s.bridge.call).toHaveBeenCalledTimes(1);
  fireEvent.click(screen.getByRole('button', { name: 'Undo' })); await settle();
  expect(s.current().flatMap(item => item.audio_config!.flatMap(row => row.channels.map(channel => channel.level_db)))).toEqual([0, -3, 0, -3]);
  fireEvent.click(screen.getByRole('button', { name: 'Channel 2' }), { shiftKey: true });
  expect(screen.getByRole('button', { name: 'Channel 2' }).getAttribute('aria-pressed')).toBe('false');
});

it('rejects mismatched channel counts and keeps failed edits retryable', async () => {
  const a = source(), b = source('m2'); b.audio_config!.pop();
  const apply = vi.fn().mockResolvedValue(false), bridge = { call: vi.fn().mockResolvedValue({ ok: true, result: { pending: true } }), onEvent: () => () => {} };
  const view = render(<MediaFile media={[a, b]} bridge={bridge} session={1} onApply={apply} />);
  expect(screen.getByRole('checkbox', { name: 'Channel 1 Active' }).matches(':disabled')).toBe(true);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 1 Active' })); await settle();
  expect(apply).not.toHaveBeenCalled();
  view.rerender(<MediaFile media={[a]} bridge={bridge} session={1} onApply={apply} />);
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 1 Mute' })); await settle();
  expect(screen.getByRole('alert').textContent).toContain('Could not update');
  fireEvent.click(screen.getByRole('checkbox', { name: 'Channel 1 Mute' })); await settle();
  expect(apply).toHaveBeenCalledTimes(2);
});

it('blocks repeated pending edits and does not submit old controls after unmount', async () => {
  let finish!: (ok: boolean) => void;
  const apply = vi.fn().mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  const bridge = { call: vi.fn().mockResolvedValue({ ok: true, result: { pending: true } }), onEvent: () => () => {} };
  const view = render(<MediaFile media={[source()]} bridge={bridge} session={1} onApply={apply} />);
  const control = screen.getByRole('checkbox', { name: 'Channel 1 Mute' });
  fireEvent.click(control); fireEvent.click(control);
  expect(apply).toHaveBeenCalledTimes(1);
  view.unmount(); finish(true); await settle();
  fireEvent.click(control); expect(apply).toHaveBeenCalledTimes(1);
});
