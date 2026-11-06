// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EngineBridge } from '../src/bridge';
import { FileWaveforms } from '../src/studio/inspector/FileWaveforms';
import { channelWaveform } from '../src/studio/timeline/ChannelWaveform';
import { clip, media, timeline } from './timeline-fixture';
import type { ClipTrack } from '../src/studio/engine';
import { likeABrowser } from './mounted';

beforeEach(() => {
  likeABrowser();
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(400);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });
const settle = () => act(async () => { for (let i = 0; i < 25; i++) await Promise.resolve(); });
const source = { ...media, facts: { ...media.facts, audio: { rate: 48000, channels: 2, codec: 'pcm', streams: [{ index: 1, channels: 2, layout: 'stereo' }, { index: 3, channels: 1, layout: 'mono' }] } } };
const track: ClipTrack = { id: 'a', name: 'Audio', kind: 'audio', format: 'stereo', enabled: true, locked: false, clips: [], transitions: [] };
const mapped = () => clip('c', 10, 100, { channels: [{ source: { stream: 1, channel: 1 }, level_db: -6 }, { source: { stream: 3, channel: 0 } }] });
function wire() {
  const listeners = new Set<Parameters<EngineBridge['onEvent']>[0]>();
  const off = vi.fn();
  const onEvent: EngineBridge['onEvent'] = listener => { listeners.add(listener); return () => { listeners.delete(listener); off(); }; };
  const call = vi.fn<EngineBridge['call']>(async (_method, params: any) => {
    const channels = params.remix ?? (params.mix || params.stream === 3 ? 1 : 2);
    return { ok: true, result: { rate: 10, channels, peaks: btoa(String.fromCharCode(...Array.from({ length: 50 * channels * 2 }, (_, i) => i % 2 ? 64 : 192))) } };
  });
  return { call, onEvent, listeners, off };
}
it('draws mapped native streams, shares their cache, scales channel trim and releases subscriptions', async () => {
  const bridge = wire(), onSelect = vi.fn();
  const view = render(<FileWaveforms bridge={bridge} session={7} clip={mapped()} track={track} media={source} timeline={timeline([])} selected={[1]} onSelect={onSelect} />);
  await settle();
  expect(bridge.call.mock.calls).toEqual([
    ['media.waveform', { media: media.id, stream: 1 }, { expectedSession: 7 }],
    ['media.waveform', { media: media.id, stream: 3 }, { expectedSession: 7 }],
  ]);
  expect(view.container.querySelectorAll('svg')).toHaveLength(2);
  expect(view.container.querySelector('path')?.getAttribute('transform')).toContain(`scale(1,${10 ** (-6 / 20)})`);
  fireEvent.click(screen.getByRole('button', { name: 'Channel 2' }), { shiftKey: true });
  expect(onSelect).toHaveBeenCalledWith(1, true);
  expect(screen.getByRole('button', { name: 'Channel 2' }).getAttribute('aria-pressed')).toBe('true');
  view.unmount();
  expect(bridge.listeners.size).toBe(0);
  expect(bridge.off).toHaveBeenCalledOnce();
});

it('fetches mixed PCM peaks separately and does not fetch silent or disabled channels', async () => {
  const bridge = wire();
  const value = mapped();
  value.channels![0].mute = true;
  value.channels![1].source = [{ stream: 1, channel: 0 }, { stream: 1, channel: 1 }];
  const view = render(<FileWaveforms bridge={bridge} session={1} clip={value} track={track} media={source} timeline={timeline([])} selected={[]} onSelect={() => {}} />);
  await settle();
  expect(bridge.call).toHaveBeenCalledExactlyOnceWith('media.waveform', { media: media.id, mix: [{ stream: 1, channel: 0 }, { stream: 1, channel: 1 }] }, { expectedSession: 1 });
  expect(screen.queryByText('Mixed waveform unavailable')).toBeNull();
  expect(view.container.querySelectorAll('svg')).toHaveLength(1);
  const quieter = { ...value, channels: value.channels!.map(channel => ({ ...channel, level_db: -12 })) };
  view.rerender(<FileWaveforms bridge={bridge} session={1} clip={quieter} track={track} media={source} timeline={timeline([])} selected={[]} onSelect={() => {}} />);
  await settle();
  expect(bridge.call).toHaveBeenCalledTimes(1);
  expect(view.container.querySelector('path')?.getAttribute('transform')).toContain(`scale(1,${10 ** (-12 / 20)})`);
  expect(channelWaveform({ ...value, channels: [{ source: null }, { source: { stream: 3, channel: 0 }, enabled: false }] }, track, source, 1).silent).toBe(true);
  expect(channelWaveform(clip('auto'), { ...track, format: 'mono' }, source, 0).remix).toBe(1);
});

it('draws automatic remixes and changes the request when the track format changes', async () => {
  const bridge = wire();
  const props = { bridge, session: 1, clip: clip('auto'), media: source, timeline: timeline([]), selected: [], onSelect: () => {} };
  const view = render(<FileWaveforms {...props} track={{ ...track, format: 'mono' }} />);
  await settle();
  expect(bridge.call.mock.calls[0][1]).toEqual({ media: media.id, remix: 1 });
  expect(view.container.querySelectorAll('svg')).toHaveLength(1);
  view.rerender(<FileWaveforms {...props} track={{ ...track, format: '5.1', order: 'film' }} />);
  await settle();
  expect(bridge.call.mock.calls[1][1]).toEqual({ media: media.id, remix: 6 });
  expect(bridge.call).toHaveBeenCalledTimes(2);
  expect(view.container.querySelectorAll('svg')).toHaveLength(6);
});

it('maps automatic SMPTE samples to Film lanes without reordering explicit source selections', () => {
  const film: ClipTrack = { ...track, format: '5.1', order: 'film' };
  const six = { ...source, facts: { ...source.facts, audio: { ...source.facts.audio, channels: 6 } } };
  const lanes = Array.from({ length: 6 }, (_, i) => channelWaveform(clip('auto'), film, six, i));
  expect(lanes.map(lane => lane.channel)).toEqual([0, 2, 1, 4, 5, 3]);
  expect(lanes.every(lane => lane.remix === undefined)).toBe(true);
  expect(channelWaveform(mapped(), film, source, 1)).toMatchObject({ channel: 0, stream: 3, remix: undefined });
});

it('isolates sessions and shares one default-stream request for automatic stereo lanes', async () => {
  const bridge = wire();
  const props = { bridge, clip: clip('auto'), track, media: source, timeline: timeline([]), selected: [], onSelect: () => {} };
  const view = render(<FileWaveforms {...props} session={1} />); await settle();
  expect(bridge.call).toHaveBeenCalledTimes(1);
  expect(bridge.call.mock.calls[0]).toEqual(['media.waveform', { media: media.id }, { expectedSession: 1 }]);
  view.rerender(<FileWaveforms {...props} session={2} />); await settle();
  expect(bridge.call).toHaveBeenCalledTimes(2);
  expect(bridge.call.mock.calls[1][2]).toEqual({ expectedSession: 2 });
  expect(bridge.off).toHaveBeenCalledTimes(1);
  expect(bridge.listeners.size).toBe(1);
});


it('draws retained automatic outputs from the original conversion alongside native choices', async () => {
  const bridge = wire(), value = mapped();
  value.channels = [{ source: [], remix: { channels: 6, channel: 4 }, level_db: -6 }, { source: { stream: 3, channel: 0 } }];
  const view = render(<FileWaveforms bridge={bridge} session={1} clip={value} track={track} media={source} timeline={timeline([])} selected={[]} onSelect={() => {}} />);
  await settle();
  expect(bridge.call.mock.calls.map(call => call[1])).toEqual([{ media: media.id, remix: 6 }, { media: media.id, stream: 3 }]);
  expect(view.container.querySelectorAll('svg')).toHaveLength(2);
  expect(channelWaveform(value, track, source, 0)).toMatchObject({ remix: 6, channel: 4, mix: undefined, silent: false, gain: 10 ** (-6 / 20) });
});
